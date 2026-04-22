"use strict";

/**
 * Inference Marketplace HTTP Routes
 * Shin Devlin
 *
 * REST API for the BTCPC inference marketplace. Buyers post jobs with
 * BTCPC escrow. Miners claim and run inference (one-shot or agentic loop).
 * Protocol takes 10%.
 *
 * Routes:
 *   POST   /api/jobs                     — buyer posts a job
 *   GET    /api/jobs                     — list open jobs (public)
 *   GET    /api/jobs/mine                — buyer's own jobs (auth)
 *   GET    /api/jobs/claimed             — miner's claimed jobs (auth)
 *   GET    /api/jobs/tool-pending        — buyer's jobs awaiting tool results (auth)
 *   GET    /api/jobs/:id                 — job detail (public)
 *   POST   /api/jobs/:id/claim           — miner claims a job
 *   POST   /api/jobs/:id/tool-calls      — miner submits tool calls (agentic)
 *   POST   /api/jobs/:id/tool-result     — buyer submits tool execution results (agentic)
 *   POST   /api/jobs/:id/submit          — miner submits final result
 *   POST   /api/jobs/:id/settle          — settle (auto or verifier)
 *   POST   /api/jobs/:id/refund          — refund expired job
 */

const express = require("express");
const router = express.Router();
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const { authenticateToken } = require("../middlewares/auth");
const market = require("../services/inferenceMarket");
const stateStore = require("../chain/stateStore");
const { sanitizeString, sanitizeAmount } = require("../middlewares/validate");
const { PROTOCOL_TOOL_SCHEMAS } = require("../services/protocolTools");

const BLOB_DIR = process.env.BTCPC_BLOB_DIR || path.resolve(__dirname, "../../data/blobs");
const memoryService = require("../services/memoryService");

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
    const maxTurns = Math.min(parseInt(req.body.max_turns) || 1, 20);

    // tools: array of tool name strings (protocol tools) or full schema objects (custom)
    let tools = [];
    if (Array.isArray(req.body.tools)) {
      tools = req.body.tools.slice(0, 20); // max 20 tools per job
    }

    if (!prompt) return res.status(400).json({ error: "prompt required" });
    if (!maxFee || maxFee <= 0)
      return res.status(400).json({ error: "max_fee required" });

    const outputSchema = req.body.output_schema || null;
    const tier = req.body.tier || "standard";
    const ragCids = Array.isArray(req.body.rag_cids) ? req.body.rag_cids.slice(0, 10) : [];
    const sessionId = req.body.session_id ? sanitizeString(req.body.session_id, 64) : null;
    const autoMemory = !!req.body.auto_memory;
    const memoryProject = req.body.memory_project ? sanitizeString(req.body.memory_project, 64) : null;
    const memoryIds = Array.isArray(req.body.memory_ids) ? req.body.memory_ids.slice(0, 20) : [];

    // Images: accept pre-uploaded CIDs or inline base64 — stored as blobs, chain only sees CIDs
    let imageCids = Array.isArray(req.body.image_cids) ? req.body.image_cids.slice(0, 5) : [];
    if (Array.isArray(req.body.images) && req.body.images.length > 0) {
      if (!fs.existsSync(BLOB_DIR)) fs.mkdirSync(BLOB_DIR, { recursive: true });
      for (const b64 of req.body.images.slice(0, 5)) {
        try {
          const buf = Buffer.from(b64, "base64");
          if (buf.length > 4 * 1024 * 1024) continue; // >4MB: use POST /api/blobs instead
          const cid = crypto.createHash("sha256").update(buf).digest("hex");
          fs.writeFileSync(path.join(BLOB_DIR, cid), buf);
          imageCids.push(cid);
        } catch (_) {}
      }
    }

    // Audio: accept pre-uploaded CID or inline base64
    let audioCid = req.body.audio_cid ? sanitizeString(req.body.audio_cid, 64) : null;
    if (!audioCid && req.body.audio) {
      try {
        const buf = Buffer.from(req.body.audio, "base64");
        if (buf.length <= 10 * 1024 * 1024) { // >10MB: use POST /api/blobs instead
          if (!fs.existsSync(BLOB_DIR)) fs.mkdirSync(BLOB_DIR, { recursive: true });
          audioCid = crypto.createHash("sha256").update(buf).digest("hex");
          fs.writeFileSync(path.join(BLOB_DIR, audioCid), buf);
        }
      } catch (_) {}
    }

    // Inject relevant memories into system prompt if requested
    let effectiveSystemPrompt = systemPrompt;
    if (autoMemory || memoryIds.length > 0) {
      const memCtx = memoryService.loadMemoriesForJob(buyer, memoryIds, memoryProject);
      if (memCtx) {
        effectiveSystemPrompt = effectiveSystemPrompt
          ? `${effectiveSystemPrompt}\n\n${memCtx}`
          : memCtx;
      }
    }

    const result = await market.openJob(buyer, prompt, maxFee, {
      model,
      systemPrompt: effectiveSystemPrompt,
      ttlEpochs,
      tools,
      maxTurns,
      outputSchema,
      tier,
      ragCids,
      imageCids,
      audioCid,
      sessionId,
      autoMemory,
      memoryProject,
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

// ── GET /api/jobs/tool-pending ────────────────────────────────────────────────
// Buyer's jobs awaiting tool execution results (tool_pending status).
// Buyer reads these, executes the tool calls, then POST /tool-result.
router.get("/tool-pending", authenticateToken, (req, res) => {
  try {
    const jobs = stateStore.getJobsAwaitingToolResults(req.user.username);
    res.json({ jobs });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── GET /api/jobs/protocol-tools ─────────────────────────────────────────────
// List all protocol-native tool schemas buyers can include in jobs. Public.
router.get("/protocol-tools", (req, res) => {
  res.json({ tools: PROTOCOL_TOOL_SCHEMAS });
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

  // Turns: full history to buyer/miner; strip sensitive content from public
  const turns = isOwner ? (job.turns || []) : [];

  // When job is tool_pending and viewer is buyer, include the pending tool calls
  let pendingToolCalls = null;
  if (job.status === "tool_pending" && isOwner && job.buyer === (req.user && req.user.username)) {
    const lastTurn = (job.turns || []).filter((t) => t.role === "assistant").pop();
    pendingToolCalls = lastTurn ? lastTurn.tool_calls : null;
  }

  res.json({
    job_id: job.job_id,
    buyer: job.buyer,
    max_fee: job.max_fee,
    model: job.model,
    prompt: isOwner ? job.prompt : job.prompt.slice(0, 100) + (job.prompt.length > 100 ? "…" : ""),
    system_prompt: isOwner ? job.system_prompt : null,
    tools: job.tools || [],
    max_turns: job.max_turns || 1,
    current_turn: job.current_turn || 0,
    turns,
    pending_tool_calls: pendingToolCalls,
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

    // Return full job context to the miner who claimed
    const job = stateStore.getInferenceJob(req.params.id);
    res.json({
      ...result,
      prompt: job ? job.prompt : null,
      system_prompt: job ? job.system_prompt : null,
      model: job ? job.model : null,
      tools: job ? (job.tools || []) : [],
      max_turns: job ? (job.max_turns || 1) : 1,
      turns: job ? (job.turns || []) : [],
      image_cids: job ? (job.image_cids || []) : [],
      audio_cid: job ? (job.audio_cid || null) : null,
    });
  } catch (err) {
    const status =
      err.message.includes("not found") ? 404 :
      err.message.includes("not open") ? 409 :
      err.message.includes("expired") ? 410 : 400;
    res.status(status).json({ error: err.message });
  }
});

// ── POST /api/jobs/:id/tool-calls ─────────────────────────────────────────────
// Miner detected tool calls in Ollama response. Protocol-native tools are
// executed inline. External tools are forwarded to the buyer (tool_pending).
router.post("/:id/tool-calls", authenticateToken, async (req, res) => {
  try {
    const miner = req.user.username;
    const toolCalls = req.body.tool_calls;
    if (!Array.isArray(toolCalls) || toolCalls.length === 0) {
      return res.status(400).json({ error: "tool_calls array required" });
    }
    const result = await market.submitToolCalls(req.params.id, miner, toolCalls);
    res.json(result);
  } catch (err) {
    const status =
      err.message.includes("not found") ? 404 :
      err.message.includes("not claimed") ? 409 :
      err.message.includes("Not your job") ? 403 :
      err.message.includes("max_turns") ? 422 : 400;
    res.status(status).json({ error: err.message });
  }
});

// ── POST /api/jobs/:id/tool-result ────────────────────────────────────────────
// Buyer submits tool execution results for external tools. Job returns to
// 'claimed' so the miner can run the next inference turn.
router.post("/:id/tool-result", authenticateToken, async (req, res) => {
  try {
    const buyer = req.user.username;
    const toolResults = req.body.tool_results;
    if (!Array.isArray(toolResults) || toolResults.length === 0) {
      return res.status(400).json({ error: "tool_results array required" });
    }
    const result = await market.submitToolResults(req.params.id, buyer, toolResults);
    res.json(result);
  } catch (err) {
    const status =
      err.message.includes("not found") ? 404 :
      err.message.includes("not awaiting") ? 409 :
      err.message.includes("Not your job") ? 403 : 400;
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

// ── POST /api/jobs/batch ──────────────────────────────────────────────────────
// Open up to 20 jobs in one request. Shared config merges with per-job overrides.
// Returns batch_id + array of job results.
router.post("/batch", authenticateToken, async (req, res) => {
  try {
    const buyer = req.user.username;
    const jobDefs = req.body.jobs;
    if (!Array.isArray(jobDefs) || jobDefs.length === 0) {
      return res.status(400).json({ error: "jobs array required" });
    }
    if (jobDefs.length > 20) {
      return res.status(400).json({ error: "max 20 jobs per batch" });
    }
    const shared = req.body.shared || {};
    const batchId = "batch_" + require("crypto").randomBytes(6).toString("hex");

    const results = [];
    const errors = [];

    for (let i = 0; i < jobDefs.length; i++) {
      const def = jobDefs[i];
      const prompt = sanitizeString(def.prompt || shared.prompt, MAX_PROMPT_LENGTH);
      const maxFee = sanitizeAmount(def.max_fee != null ? def.max_fee : shared.max_fee);
      if (!prompt || !maxFee) {
        errors.push({ index: i, error: "prompt and max_fee required" });
        continue;
      }
      try {
        const job = await market.openJob(buyer, prompt, maxFee, {
          model: def.model || shared.model || null,
          systemPrompt: def.system_prompt || shared.system_prompt || null,
          ttlEpochs: Math.min(parseInt(def.ttl_epochs || shared.ttl_epochs) || 20, 480),
          tools: Array.isArray(def.tools) ? def.tools : (shared.tools || []),
          maxTurns: Math.min(parseInt(def.max_turns || shared.max_turns) || 1, 20),
          outputSchema: def.output_schema || shared.output_schema || null,
          tier: def.tier || shared.tier || "standard",
          ragCids: Array.isArray(def.rag_cids) ? def.rag_cids : (shared.rag_cids || []),
          sessionId: def.session_id || shared.session_id || null,
          batchId,
        });
        results.push({ index: i, ...job });
      } catch (err) {
        errors.push({ index: i, error: err.message });
      }
    }

    res.json({
      batch_id: batchId,
      submitted: results.length,
      failed: errors.length,
      jobs: results,
      errors: errors.length ? errors : undefined,
    });
  } catch (err) {
    const status = err.message && err.message.includes("Insufficient") ? 402 : 400;
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
