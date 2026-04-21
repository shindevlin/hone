"use strict";

/**
 * Session routes — persistent conversation context stored on-chain.
 * Shin Devlin
 *
 * Sessions link multiple inference jobs together with shared context.
 * The summary is stored on-chain and prepended to every job in the session.
 *
 * Routes:
 *   POST  /api/sessions            — create a session
 *   GET   /api/sessions            — buyer's sessions (auth)
 *   GET   /api/sessions/:id        — session detail + job list
 *   PATCH /api/sessions/:id/summary — update rolling summary (miner/buyer)
 *   DELETE /api/sessions/:id       — buyer closes session (no on-chain delete, just marks)
 */

const express = require("express");
const router = express.Router();
const crypto = require("crypto");
const { authenticateToken } = require("../middlewares/auth");
const { sanitizeString } = require("../middlewares/validate");
const stateStore = require("../chain/stateStore");
const ledger = require("../services/ledger");

const MAX_SUMMARY_LENGTH = 4000;
const MAX_NAME_LENGTH = 80;

// ── POST /api/sessions ────────────────────────────────────────────────────────
router.post("/", authenticateToken, async (req, res) => {
  try {
    const buyer = req.user.username;
    const sessionId = "sess_" + crypto.randomBytes(8).toString("hex");
    const name = req.body.name ? sanitizeString(req.body.name, MAX_NAME_LENGTH) : null;
    const initialSummary = req.body.initial_summary
      ? sanitizeString(req.body.initial_summary, MAX_SUMMARY_LENGTH)
      : null;

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordSessionCreate(buyer, sessionId, { name, initialSummary }, epoch);

    res.json({
      session_id: sessionId,
      buyer,
      name,
      summary: initialSummary,
      job_ids: [],
      created_epoch: epoch,
    });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── GET /api/sessions ─────────────────────────────────────────────────────────
router.get("/", authenticateToken, (req, res) => {
  try {
    const sessions = stateStore.getSessionsByBuyer(req.user.username);
    res.json({ sessions });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── GET /api/sessions/:id ─────────────────────────────────────────────────────
router.get("/:id", authenticateToken, (req, res) => {
  const session = stateStore.getSession(req.params.id);
  if (!session) return res.status(404).json({ error: "Session not found" });
  if (session.buyer !== req.user.username) {
    return res.status(403).json({ error: "Not your session" });
  }

  // Attach lightweight job summaries
  const jobs = (session.job_ids || []).map((jid) => {
    const job = stateStore.getInferenceJob(jid);
    if (!job) return { job_id: jid, status: "unknown" };
    return {
      job_id: jid,
      status: job.status,
      model: job.model,
      tier: job.tier,
      open_epoch: job.open_epoch,
      settled_epoch: job.settled_epoch,
      max_fee: job.max_fee,
      actual_cost: job.actual_cost,
    };
  });

  res.json({ ...session, jobs });
});

// ── PATCH /api/sessions/:id/summary ──────────────────────────────────────────
// Update the rolling session summary. Typically called after a job settles.
router.patch("/:id/summary", authenticateToken, async (req, res) => {
  try {
    const session = stateStore.getSession(req.params.id);
    if (!session) return res.status(404).json({ error: "Session not found" });
    if (session.buyer !== req.user.username) {
      return res.status(403).json({ error: "Not your session" });
    }

    const summary = sanitizeString(req.body.summary, MAX_SUMMARY_LENGTH);
    if (!summary) return res.status(400).json({ error: "summary required" });

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordSessionUpdateSummary(req.params.id, summary, epoch);

    res.json({ session_id: req.params.id, summary, updated_epoch: epoch });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

module.exports = router;
