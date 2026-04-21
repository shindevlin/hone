"use strict";

/**
 * Inference Marketplace HTTP Routes — v3.1.119
 * Shin Devlin
 *
 * REST API for the BTCPC inference marketplace. Buyers post jobs with
 * BTCPC escrow. Miners claim and run inference. Verifier nodes confirm.
 * Protocol takes 10%.
 *
 * Routes:
 *   POST   /api/jobs                  — buyer posts a job
 *   GET    /api/jobs                  — list open jobs (public)
 *   GET    /api/jobs/mine             — buyer's own jobs (auth)
 *   GET    /api/jobs/claimed          — miner's claimed jobs (auth)
 *   GET    /api/jobs/:id              — job detail (public)
 *   POST   /api/jobs/:id/claim        — miner claims a job
 *   POST   /api/jobs/:id/submit       — miner submits result
 *   POST   /api/jobs/:id/settle       — settle (auto or verifier)
 *   POST   /api/jobs/:id/refund       — refund expired job
 */

const express = require("express");
const router = express.Router();
const { authenticateToken } = require("../middlewares/auth");
const market = require("../services/inferenceMarket");
const stateStore = require("../chain/stateStore");
const { sanitizeString, sanitizeAmount } = require("../middlewares/validate");

const MAX_PROMPT_LENGTH = 8000;
const MAX_RESULT_LENGTH = 32000;

// ── POST /api/jobs ────────────────────────────────────────────────────────────
// Buyer opens a new inference job. Escrow locked immediately.
router.post("/", authenticateToken, async (req, res) => {
  try {
    const buyer = req.user.username;
    const prompt = sanitizeString(req.body.prompt, MAX_PROMPT_LENGTH);
    const maxFee = sanitizeAmount(req.body.max_fee);
    const model = req.body.model ? sanitizeString(req.body.model, 100) : null;
    const systemPrompt = req.body.system_prompt
      ? sanitizeString(req.body.system_prompt, 2000)
      : null;
    const ttlEpochs = Math.min(
      parseInt(req.body.ttl_epochs) || 20,
      480 // max 4 hours
    );

    if (!prompt) return res.status(400).json({ error: "prompt required" });
    if (!maxFee || maxFee <= 0)
      return res.status(400).json({ error: "max_fee required" });

    const result = await market.openJob(buyer, prompt, maxFee, {
      model,
      systemPrompt,
      ttlEpochs,
    });
    res.json(result);
  } catch (err) {
    const status = err.message && err.message.includes("Insufficient") ? 402 : 400;
    res.status(status).json({ error: err.message });
  }
});

// ── GET /api/jobs ─────────────────────────────────────────────────────────────
// List open (and recently claimed) jobs. Public.
router.get("/", (req, res) => {
  try {
    const model = req.query.model ? sanitizeString(req.query.model, 100) : null;
    const limit = Math.min(parseInt(req.query.limit) || 50, 200);
    const offset = Math.max(parseInt(req.query.offset) || 0, 0);

    const result = market.getOpenJobs({ model, limit, offset });

    // Strip prompt from public listing (privacy — full prompt only for claimer)
    const sanitized = result.jobs.map((j) => ({
      job_id: j.job_id,
      buyer: j.buyer,
      max_fee: j.max_fee,
      model: j.model,
      prompt_preview: j.prompt ? j.prompt.slice(0, 100) + (j.prompt.length > 100 ? "…" : "") : null,
      ttl_epochs: j.ttl_epochs,
      expires_epoch: j.expires_epoch,
      status: j.status,
      open_epoch: j.open_epoch,
      miner: j.miner || null,
    }));

    res.json({ jobs: sanitized, total: result.total, limit, offset });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── GET /api/jobs/mine ────────────────────────────────────────────────────────
// Buyer's own jobs (full prompt visible).
router.get("/mine", authenticateToken, (req, res) => {
  try {
    const jobs = market.getBuyerJobs(req.user.username, {
      limit: Math.min(parseInt(req.query.limit) || 50, 200),
    });
    res.json({ jobs });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── GET /api/jobs/claimed ─────────────────────────────────────────────────────
// Miner's claimed/submitted/settled jobs.
router.get("/claimed", authenticateToken, (req, res) => {
  try {
    const jobs = market.getMinerJobs(req.user.username, {
      limit: Math.min(parseInt(req.query.limit) || 50, 200),
    });
    res.json({ jobs });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── GET /api/jobs/:id ─────────────────────────────────────────────────────────
// Full job detail. Full prompt only visible to buyer or claimer.
router.get("/:id", (req, res) => {
  const job = stateStore.getInferenceJob(req.params.id);
  if (!job) return res.status(404).json({ error: "Job not found" });

  // For public viewers: redact full prompt
  const isOwner =
    req.headers.authorization && req.user &&
    (req.user.username === job.buyer || req.user.username === job.miner);

  res.json({
    job_id: job.job_id,
    buyer: job.buyer,
    max_fee: job.max_fee,
    model: job.model,
    prompt: isOwner ? job.prompt : job.prompt.slice(0, 100) + (job.prompt.length > 100 ? "…" : ""),
    system_prompt: isOwner ? job.system_prompt : null,
    status: job.status,
    miner: job.miner,
    proof_hash: job.proof_hash,
    actual_cost: job.actual_cost,
    miner_payout: job.miner_payout,
    protocol_fee: job.protocol_fee,
    ttl_epochs: job.ttl_epochs,
    expires_epoch: job.expires_epoch,
    open_epoch: job.open_epoch,
    claimed_epoch: job.claimed_epoch,
    submitted_epoch: job.submitted_epoch,
    settled_epoch: job.settled_epoch,
  });
});

// ── POST /api/jobs/:id/claim ──────────────────────────────────────────────────
// Miner claims an open job. Returns the full prompt for inference.
router.post("/:id/claim", authenticateToken, async (req, res) => {
  try {
    const miner = req.user.username;
    const result = await market.claimJob(req.params.id, miner);

    // Return full prompt to the miner who claimed
    const job = stateStore.getInferenceJob(req.params.id);
    res.json({
      ...result,
      prompt: job ? job.prompt : null,
      system_prompt: job ? job.system_prompt : null,
      model: job ? job.model : null,
    });
  } catch (err) {
    const status =
      err.message.includes("not found") ? 404 :
      err.message.includes("not open") ? 409 :
      err.message.includes("expired") ? 410 : 400;
    res.status(status).json({ error: err.message });
  }
});

// ── POST /api/jobs/:id/submit ─────────────────────────────────────────────────
// Miner submits the inference result. Triggers auto-settle.
router.post("/:id/submit", authenticateToken, async (req, res) => {
  try {
    const miner = req.user.username;
    const result_text = sanitizeString(req.body.result, MAX_RESULT_LENGTH);
    const proof_hash = req.body.proof_hash
      ? sanitizeString(req.body.proof_hash, 64)
      : null;

    if (!result_text)
      return res.status(400).json({ error: "result required" });

    const submitted = await market.submitJob(
      req.params.id,
      miner,
      result_text,
      proof_hash
    );

    // Auto-settle immediately (no separate verifier step for v1 marketplace)
    // Full verifier quorum is a v2 feature. For now, miner self-reports cost.
    const job = stateStore.getInferenceJob(req.params.id);
    const actualCost = req.body.actual_cost
      ? Math.min(parseFloat(req.body.actual_cost), job ? job.max_fee : 0)
      : job ? job.max_fee : 0;

    const settlement = await market.settleJob(req.params.id, actualCost, "auto");

    res.json({
      submitted,
      settlement,
      result_preview: result_text.slice(0, 200),
    });
  } catch (err) {
    const status =
      err.message.includes("not found") ? 404 :
      err.message.includes("not claimed") ? 409 :
      err.message.includes("Not your job") ? 403 : 400;
    res.status(status).json({ error: err.message });
  }
});

// ── POST /api/jobs/:id/settle ─────────────────────────────────────────────────
// Manual settle — for verifier nodes or admin.
router.post("/:id/settle", authenticateToken, async (req, res) => {
  try {
    const settledBy = req.user.username;
    const job = stateStore.getInferenceJob(req.params.id);
    if (!job) return res.status(404).json({ error: "Job not found" });

    const actualCost = req.body.actual_cost
      ? Math.min(parseFloat(req.body.actual_cost), job.max_fee)
      : job.max_fee;

    const result = await market.settleJob(req.params.id, actualCost, settledBy);
    res.json(result);
  } catch (err) {
    const status = err.message.includes("not found") ? 404 :
      err.message.includes("not submitted") ? 409 : 400;
    res.status(status).json({ error: err.message });
  }
});

// ── POST /api/jobs/:id/refund ─────────────────────────────────────────────────
// Buyer or system refunds an expired/abandoned job.
router.post("/:id/refund", authenticateToken, async (req, res) => {
  try {
    const job = stateStore.getInferenceJob(req.params.id);
    if (!job) return res.status(404).json({ error: "Job not found" });

    // Only buyer can manually refund; system sweeps handle the rest
    if (job.buyer !== req.user.username) {
      return res.status(403).json({ error: "Only the buyer can refund this job" });
    }
    if (job.status === "claimed" || job.status === "submitted") {
      return res.status(409).json({
        error: "Job has an active miner — wait for TTL expiry",
      });
    }

    const result = await market.refundJob(req.params.id);
    res.json(result);
  } catch (err) {
    const status = err.message.includes("not found") ? 404 :
      err.message.includes("already") ? 409 : 400;
    res.status(status).json({ error: err.message });
  }
});

// ── GET /api/jobs/stats ───────────────────────────────────────────────────────
// Network-wide marketplace stats. Public.
router.get("/stats/overview", (req, res) => {
  try {
    const open = stateStore.getOpenInferenceJobs();
    const openCount = open.filter((j) => j.status === "open").length;
    const claimedCount = open.filter((j) => j.status === "claimed").length;
    const totalOpenFees = open.reduce((s, j) => s + (j.max_fee || 0), 0);

    res.json({
      open_jobs: openCount,
      claimed_jobs: claimedCount,
      total_escrow_btcpc: parseFloat(totalOpenFees.toFixed(6)),
      protocol_fee_pct: market.PROTOCOL_FEE_PCT * 100,
      min_job_fee_btcpc: market.MIN_JOB_FEE,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
