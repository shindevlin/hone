"use strict";

/**
 * Computer use (browser automation) marketplace routes.
 * Shin Devlin
 *
 * Buyers post browser jobs: a goal + starting URL. Each agentic turn the
 * miner takes a screenshot (stored as HONE-FS blob), sends available actions,
 * and the buyer (or an agent) submits the next action. Miner executes via
 * Playwright, loops until goal is reached or turn limit hit.
 *
 * Job lifecycle mirrors the agentic inference loop:
 *   open → claimed → action_pending → claimed → … → submitted → settled
 *
 * Routes:
 *   POST  /api/browser               — buyer posts a browser job
 *   GET   /api/browser               — list open browser jobs
 *   GET   /api/browser/mine          — buyer's jobs
 *   GET   /api/browser/:id           — job detail + turn history
 *   POST  /api/browser/:id/claim     — miner claims, returns first screenshot
 *   POST  /api/browser/:id/action    — buyer submits next action
 *   POST  /api/browser/:id/complete  — miner reports goal reached
 *   POST  /api/browser/:id/refund    — buyer refunds
 */

const express = require("express");
const router = express.Router();
const crypto = require("crypto");
const { authenticateToken } = require("../middlewares/auth");
const { sanitizeString, sanitizeAmount } = require("../middlewares/validate");
const stateStore = require("../chain/stateStore");
const ledger = require("../services/ledger");
const { closeSession, deleteSessionBlobs } = require("../services/browserRunner");

const PROTOCOL_FEE_ACCOUNT = "hone_fees";
const PROTOCOL_FEE_PCT = 0.10;
const MIN_BROWSER_FEE = 0.1;      // Higher minimum — browser jobs are resource-intensive
const DEFAULT_MAX_TURNS = 20;
const DEFAULT_TTL_EPOCHS = 120;   // 1 hour

function _bId() {
  return "browser_" + crypto.randomBytes(6).toString("hex");
}

// ── POST /api/browser ─────────────────────────────────────────────────────────
router.post("/", authenticateToken, async (req, res) => {
  try {
    const buyer = req.user.username;
    const goal = sanitizeString(req.body.goal, 2000);
    const startUrl = sanitizeString(req.body.start_url, 512);
    const maxFee = sanitizeAmount(req.body.max_fee);
    const maxTurns = Math.min(parseInt(req.body.max_turns) || DEFAULT_MAX_TURNS, 100);
    const ttlEpochs = Math.min(parseInt(req.body.ttl_epochs) || DEFAULT_TTL_EPOCHS, 480);
    const viewport = req.body.viewport || { width: 1280, height: 800 };

    if (!goal) return res.status(400).json({ error: "goal required" });
    if (!startUrl || !/^https?:\/\//i.test(startUrl)) {
      return res.status(400).json({ error: "start_url must be a valid http/https URL" });
    }
    if (!maxFee || maxFee < MIN_BROWSER_FEE) {
      return res.status(400).json({ error: `max_fee must be at least ${MIN_BROWSER_FEE} HONE` });
    }

    const balance = stateStore.getBalance(buyer, "HONE");
    if (balance < maxFee) {
      return res.status(402).json({ error: `Insufficient balance: have ${balance}, need ${maxFee}` });
    }

    const jobId = _bId();
    const epoch = await ledger.getCurrentEpoch();

    await ledger.recordEscrowLock(buyer, jobId, maxFee, epoch);
    await ledger.recordBrowserJobOpen(buyer, jobId, {
      goal,
      start_url: startUrl,
      max_fee: maxFee,
      max_turns: maxTurns,
      ttl_epochs: ttlEpochs,
      expires_epoch: epoch + ttlEpochs,
      viewport,
    }, epoch);

    res.json({
      job_id: jobId,
      buyer,
      goal,
      start_url: startUrl,
      max_fee: maxFee,
      max_turns: maxTurns,
      status: "open",
    });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── GET /api/browser ──────────────────────────────────────────────────────────
router.get("/", (req, res) => {
  try {
    const jobs = stateStore.getOpenBrowserJobs ? stateStore.getOpenBrowserJobs() : [];
    res.json({ jobs, total: jobs.length });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── GET /api/browser/mine ─────────────────────────────────────────────────────
router.get("/mine", authenticateToken, (req, res) => {
  try {
    const jobs = stateStore.getBrowserJobsByBuyer ? stateStore.getBrowserJobsByBuyer(req.user.username) : [];
    res.json({ jobs });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── GET /api/browser/:id ──────────────────────────────────────────────────────
router.get("/:id", (req, res) => {
  const job = stateStore.getBrowserJob ? stateStore.getBrowserJob(req.params.id) : null;
  if (!job) return res.status(404).json({ error: "Browser job not found" });

  const isOwner = req.user && (req.user.username === job.buyer || req.user.username === job.miner);
  res.json({
    ...job,
    // Redact full turn history from non-owners (screenshots can be large CIDs)
    turns: isOwner ? job.turns : (job.turns || []).map((t) => ({
      turn: t.turn,
      role: t.role,
      action_type: t.action_type,
      screenshot_cid: t.screenshot_cid,
    })),
  });
});

// ── POST /api/browser/:id/claim ───────────────────────────────────────────────
// Miner claims the job. Should immediately navigate to start_url, take a
// screenshot, store it as a blob, and return the screenshot CID + available actions.
router.post("/:id/claim", authenticateToken, async (req, res) => {
  try {
    const miner = req.user.username;
    const job = stateStore.getBrowserJob ? stateStore.getBrowserJob(req.params.id) : null;
    if (!job) return res.status(404).json({ error: "Job not found" });
    if (job.status !== "open") return res.status(409).json({ error: "Job not open" });

    const epoch = await ledger.getCurrentEpoch();
    if (job.expires_epoch && epoch > job.expires_epoch) {
      return res.status(410).json({ error: "Job expired" });
    }

    await ledger.recordBrowserJobClaim(req.params.id, miner, epoch);

    res.json({
      job_id: req.params.id,
      miner,
      status: "claimed",
      goal: job.goal,
      start_url: job.start_url,
      viewport: job.viewport,
      max_turns: job.max_turns,
    });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── POST /api/browser/:id/screenshot ─────────────────────────────────────────
// Miner posts a screenshot turn. Includes screenshot_cid + available actions.
// Job transitions to action_pending so buyer can respond.
router.post("/:id/screenshot", authenticateToken, async (req, res) => {
  try {
    const miner = req.user.username;
    const job = stateStore.getBrowserJob ? stateStore.getBrowserJob(req.params.id) : null;
    if (!job) return res.status(404).json({ error: "Job not found" });
    if (job.miner !== miner) return res.status(403).json({ error: "Not your job" });

    const screenshotCid = sanitizeString(req.body.screenshot_cid, 64);
    const availableActions = req.body.available_actions || [];
    const currentUrl = sanitizeString(req.body.current_url, 512);
    const pageTitle = sanitizeString(req.body.page_title, 200);
    const accessibilityTree = req.body.accessibility_tree || null; // optional DOM summary

    if (!screenshotCid || !/^[a-f0-9]{64}$/.test(screenshotCid)) {
      return res.status(400).json({ error: "screenshot_cid must be a valid 64-char blob CID" });
    }

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordBrowserJobScreenshot(req.params.id, miner, {
      screenshot_cid: screenshotCid,
      available_actions: availableActions,
      current_url: currentUrl,
      page_title: pageTitle,
      accessibility_tree: accessibilityTree,
      turn: (job.current_turn || 0) + 1,
    }, epoch);

    res.json({
      job_id: req.params.id,
      status: "action_pending",
      screenshot_cid: screenshotCid,
      current_url: currentUrl,
      available_actions: availableActions,
    });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── POST /api/browser/:id/action ──────────────────────────────────────────────
// Buyer (or autonomous agent) submits the next browser action.
// action_type: click | type | navigate | scroll | key | wait | done
router.post("/:id/action", authenticateToken, async (req, res) => {
  try {
    const buyer = req.user.username;
    const job = stateStore.getBrowserJob ? stateStore.getBrowserJob(req.params.id) : null;
    if (!job) return res.status(404).json({ error: "Job not found" });
    if (job.buyer !== buyer) return res.status(403).json({ error: "Not your job" });
    if (job.status !== "action_pending") {
      return res.status(409).json({ error: "Job not awaiting action (status: " + job.status + ")" });
    }

    const actionType = sanitizeString(req.body.action_type, 32);
    const validActions = ["click", "type", "navigate", "scroll", "key", "wait", "done"];
    if (!validActions.includes(actionType)) {
      return res.status(400).json({ error: `action_type must be one of: ${validActions.join(", ")}` });
    }

    const action = {
      action_type: actionType,
      coordinate: req.body.coordinate || null,         // { x, y } for click/scroll
      text: sanitizeString(req.body.text, 1000) || null, // for type/navigate/key
      selector: sanitizeString(req.body.selector, 200) || null,
      scroll_direction: req.body.scroll_direction || null,
      scroll_amount: req.body.scroll_amount || null,
      wait_ms: req.body.wait_ms || null,
    };

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordBrowserJobAction(req.params.id, buyer, {
      ...action,
      turn: job.current_turn || 0,
    }, epoch);

    if (actionType === "done") {
      // Buyer signals goal is complete — settle
      const actualCost = job.max_fee;
      const protocolFee = parseFloat((actualCost * PROTOCOL_FEE_PCT).toFixed(10));
      const minerPayout = parseFloat((actualCost - protocolFee).toFixed(10));
      if (minerPayout > 0) await ledger.recordEscrowRelease(job.miner, req.params.id, minerPayout, epoch, "Browser job settlement");
      if (protocolFee > 0) await ledger.recordEscrowRelease(PROTOCOL_FEE_ACCOUNT, req.params.id, protocolFee, epoch, "Browser job protocol fee");
      await ledger.recordBrowserJobSettle(req.params.id, job.miner, buyer, { actual_cost: actualCost, miner_payout: minerPayout, protocol_fee: protocolFee }, epoch);
      // Delete local screenshot blobs — session data belongs to the buyer only
      const screenshotCids = (job.turns || []).map((t) => t.screenshot_cid).filter(Boolean);
      await closeSession(req.params.id);
      deleteSessionBlobs(screenshotCids);
      return res.json({ job_id: req.params.id, status: "settled", action });
    }

    res.json({ job_id: req.params.id, status: "claimed", action, turn: job.current_turn });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── POST /api/browser/:id/complete ────────────────────────────────────────────
// Miner signals goal is reached autonomously (without buyer action).
router.post("/:id/complete", authenticateToken, async (req, res) => {
  try {
    const miner = req.user.username;
    const job = stateStore.getBrowserJob ? stateStore.getBrowserJob(req.params.id) : null;
    if (!job) return res.status(404).json({ error: "Job not found" });
    if (job.miner !== miner) return res.status(403).json({ error: "Not your job" });

    const resultSummary = sanitizeString(req.body.result_summary, 2000);
    const finalScreenshotCid = sanitizeString(req.body.final_screenshot_cid, 64);

    const epoch = await ledger.getCurrentEpoch();
    const actualCost = job.max_fee;
    const protocolFee = parseFloat((actualCost * PROTOCOL_FEE_PCT).toFixed(10));
    const minerPayout = parseFloat((actualCost - protocolFee).toFixed(10));

    if (minerPayout > 0) await ledger.recordEscrowRelease(miner, req.params.id, minerPayout, epoch, "Browser job settlement");
    if (protocolFee > 0) await ledger.recordEscrowRelease(PROTOCOL_FEE_ACCOUNT, req.params.id, protocolFee, epoch, "Browser job protocol fee");
    await ledger.recordBrowserJobSettle(req.params.id, miner, job.buyer, {
      actual_cost: actualCost, miner_payout: minerPayout, protocol_fee: protocolFee,
      result_summary: resultSummary, final_screenshot_cid: finalScreenshotCid,
    }, epoch);

    // Delete local screenshot blobs — session data belongs to the buyer only
    const screenshotCids = (job.turns || []).map((t) => t.screenshot_cid).filter(Boolean);
    if (finalScreenshotCid) screenshotCids.push(finalScreenshotCid);
    await closeSession(req.params.id);
    deleteSessionBlobs(screenshotCids);

    res.json({ job_id: req.params.id, status: "settled", miner_payout: minerPayout });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── POST /api/browser/:id/refund ──────────────────────────────────────────────
router.post("/:id/refund", authenticateToken, async (req, res) => {
  try {
    const job = stateStore.getBrowserJob ? stateStore.getBrowserJob(req.params.id) : null;
    if (!job) return res.status(404).json({ error: "Job not found" });
    if (job.buyer !== req.user.username) return res.status(403).json({ error: "Not your job" });
    if (["settled", "refunded"].includes(job.status)) {
      return res.status(409).json({ error: "Job already " + job.status });
    }
    if (job.status === "claimed" || job.status === "action_pending") {
      return res.status(409).json({ error: "Active browser session — wait for TTL" });
    }

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordEscrowRefund(job.buyer, req.params.id, job.max_fee, epoch);
    await ledger.recordBrowserJobRefund(req.params.id, job.buyer, "manual", epoch);

    // Clean up any local screenshots — job is abandoned
    const screenshotCids = (job.turns || []).map((t) => t.screenshot_cid).filter(Boolean);
    await closeSession(req.params.id);
    deleteSessionBlobs(screenshotCids);

    res.json({ job_id: req.params.id, status: "refunded", refunded: job.max_fee });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

module.exports = router;
