"use strict";

/**
 * Fine-tuning marketplace routes.
 * Shin Devlin
 *
 * Buyers post training jobs: dataset CID + base model + hyperparameters.
 * Miners claim, run LoRA fine-tuning via Ollama (or custom trainer),
 * upload the adapter as a BTCPC-FS blob, and submit the adapter CID.
 * Protocol takes 10%. Long TTL (2000 epochs ≈ 16 hours).
 *
 * Routes:
 *   POST  /api/finetune               — post a fine-tuning job
 *   GET   /api/finetune               — list open jobs (public)
 *   GET   /api/finetune/mine          — buyer's jobs (auth)
 *   GET   /api/finetune/training      — miner's active training jobs (auth)
 *   GET   /api/finetune/:id           — job detail
 *   POST  /api/finetune/:id/claim     — miner claims job
 *   POST  /api/finetune/:id/progress  — miner reports training progress
 *   POST  /api/finetune/:id/complete  — miner submits completed adapter CID
 *   POST  /api/finetune/:id/refund    — buyer refunds expired job
 */

const express = require("express");
const router = express.Router();
const crypto = require("crypto");
const { authenticateToken } = require("../middlewares/auth");
const { sanitizeString, sanitizeAmount } = require("../middlewares/validate");
const stateStore = require("../chain/stateStore");
const ledger = require("../services/ledger");

const PROTOCOL_FEE_ACCOUNT = "btcpc_fees";
const PROTOCOL_FEE_PCT = 0.10;
const MIN_FINETUNE_FEE = 1.0;     // 1 BTCPC minimum (training is expensive)
const DEFAULT_TTL_EPOCHS = 2000;  // ~16 hours at 30s/epoch
const MAX_TTL_EPOCHS = 10000;

function _ftId() {
  return "ft_" + crypto.randomBytes(8).toString("hex");
}

// ── POST /api/finetune ────────────────────────────────────────────────────────
router.post("/", authenticateToken, async (req, res) => {
  try {
    const buyer = req.user.username;
    const datasetCid = sanitizeString(req.body.dataset_cid, 64);
    const baseModel = sanitizeString(req.body.base_model, 100);
    const maxFee = sanitizeAmount(req.body.max_fee);
    const epochs = Math.min(parseInt(req.body.epochs) || 3, 100);
    const loraRank = Math.min(parseInt(req.body.lora_rank) || 16, 256);
    const ttlEpochs = Math.min(parseInt(req.body.ttl_epochs) || DEFAULT_TTL_EPOCHS, MAX_TTL_EPOCHS);
    const sessionId = req.body.session_id ? sanitizeString(req.body.session_id, 64) : null;

    if (!datasetCid || !/^[a-f0-9]{64}$/.test(datasetCid)) {
      return res.status(400).json({ error: "dataset_cid must be a valid 64-char blob CID" });
    }
    if (!baseModel) return res.status(400).json({ error: "base_model required" });
    if (!maxFee || maxFee < MIN_FINETUNE_FEE) {
      return res.status(400).json({ error: `max_fee must be at least ${MIN_FINETUNE_FEE} BTCPC` });
    }

    const balance = stateStore.getBalance(buyer, "BTCPC");
    if (balance < maxFee) {
      return res.status(402).json({ error: `Insufficient balance: have ${balance}, need ${maxFee}` });
    }

    const finetuneId = _ftId();
    const epoch = await ledger.getCurrentEpoch();

    await ledger.recordEscrowLock(buyer, finetuneId, maxFee, epoch);
    await ledger.recordFinetuneJobOpen(buyer, finetuneId, {
      dataset_cid: datasetCid,
      base_model: baseModel,
      epochs,
      lora_rank: loraRank,
      max_fee: maxFee,
      ttl_epochs: ttlEpochs,
      expires_epoch: epoch + ttlEpochs,
      session_id: sessionId,
    }, epoch);

    res.json({
      finetune_id: finetuneId,
      buyer,
      dataset_cid: datasetCid,
      base_model: baseModel,
      epochs,
      lora_rank: loraRank,
      max_fee: maxFee,
      status: "open",
      ttl_epochs: ttlEpochs,
    });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── GET /api/finetune ─────────────────────────────────────────────────────────
router.get("/", (req, res) => {
  try {
    const jobs = stateStore.getOpenFinetuneJobs();
    res.json({ jobs, total: jobs.length });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── GET /api/finetune/mine ────────────────────────────────────────────────────
router.get("/mine", authenticateToken, (req, res) => {
  try {
    const jobs = stateStore.getFinetuneJobsByBuyer(req.user.username);
    res.json({ jobs });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── GET /api/finetune/training ────────────────────────────────────────────────
router.get("/training", authenticateToken, (req, res) => {
  try {
    const jobs = stateStore.getFinetuneJobsByMiner(req.user.username);
    res.json({ jobs });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ── GET /api/finetune/:id ─────────────────────────────────────────────────────
router.get("/:id", (req, res) => {
  const job = stateStore.getFinetuneJob(req.params.id);
  if (!job) return res.status(404).json({ error: "Fine-tuning job not found" });
  res.json(job);
});

// ── POST /api/finetune/:id/claim ──────────────────────────────────────────────
router.post("/:id/claim", authenticateToken, async (req, res) => {
  try {
    const miner = req.user.username;
    const job = stateStore.getFinetuneJob(req.params.id);
    if (!job) return res.status(404).json({ error: "Job not found" });
    if (job.status !== "open") return res.status(409).json({ error: "Job not open" });

    const epoch = await ledger.getCurrentEpoch();
    if (job.expires_epoch && epoch > job.expires_epoch) {
      return res.status(410).json({ error: "Job expired" });
    }

    await ledger.recordFinetuneJobClaim(req.params.id, miner, epoch);
    res.json({
      finetune_id: req.params.id,
      miner,
      status: "training",
      dataset_cid: job.dataset_cid,
      base_model: job.base_model,
      epochs: job.epochs,
      lora_rank: job.lora_rank,
    });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── POST /api/finetune/:id/progress ──────────────────────────────────────────
router.post("/:id/progress", authenticateToken, async (req, res) => {
  try {
    const miner = req.user.username;
    const job = stateStore.getFinetuneJob(req.params.id);
    if (!job) return res.status(404).json({ error: "Job not found" });
    if (job.miner !== miner) return res.status(403).json({ error: "Not your job" });

    const progressPct = Math.min(Math.max(parseFloat(req.body.progress_pct) || 0, 0), 100);
    const currentEpochNum = parseInt(req.body.current_epoch_num) || 0;
    const loss = req.body.loss != null ? parseFloat(req.body.loss) : null;

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordFinetuneJobProgress(req.params.id, miner, {
      progress_pct: progressPct, current_epoch_num: currentEpochNum, loss,
    }, epoch);

    res.json({ finetune_id: req.params.id, progress_pct: progressPct, loss });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── POST /api/finetune/:id/complete ──────────────────────────────────────────
router.post("/:id/complete", authenticateToken, async (req, res) => {
  try {
    const miner = req.user.username;
    const job = stateStore.getFinetuneJob(req.params.id);
    if (!job) return res.status(404).json({ error: "Job not found" });
    if (job.miner !== miner) return res.status(403).json({ error: "Not your job" });
    if (job.status !== "training") return res.status(409).json({ error: "Job not in training" });

    const adapterCid = sanitizeString(req.body.adapter_cid, 64);
    if (!adapterCid || !/^[a-f0-9]{64}$/.test(adapterCid)) {
      return res.status(400).json({ error: "adapter_cid must be a valid 64-char blob CID" });
    }

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordFinetuneJobComplete(req.params.id, miner, adapterCid, epoch);

    // Auto-settle: release escrow to miner (90%) and protocol (10%)
    const actualCost = job.max_fee;
    const protocolFee = parseFloat((actualCost * PROTOCOL_FEE_PCT).toFixed(10));
    const minerPayout = parseFloat((actualCost - protocolFee).toFixed(10));

    if (minerPayout > 0) {
      await ledger.recordEscrowRelease(miner, req.params.id, minerPayout, epoch, "Fine-tuning settlement");
    }
    if (protocolFee > 0) {
      await ledger.recordEscrowRelease(PROTOCOL_FEE_ACCOUNT, req.params.id, protocolFee, epoch, "Fine-tuning protocol fee");
    }

    await ledger.recordFinetuneJobSettle(req.params.id, miner, job.buyer, {
      actual_cost: actualCost,
      miner_payout: minerPayout,
      protocol_fee: protocolFee,
    }, epoch);

    res.json({
      finetune_id: req.params.id,
      status: "settled",
      adapter_cid: adapterCid,
      miner_payout: minerPayout,
      protocol_fee: protocolFee,
    });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// ── POST /api/finetune/:id/refund ─────────────────────────────────────────────
router.post("/:id/refund", authenticateToken, async (req, res) => {
  try {
    const job = stateStore.getFinetuneJob(req.params.id);
    if (!job) return res.status(404).json({ error: "Job not found" });
    if (job.buyer !== req.user.username) {
      return res.status(403).json({ error: "Only buyer can refund" });
    }
    if (job.status === "training") {
      return res.status(409).json({ error: "Job is being trained — wait for TTL expiry" });
    }
    if (job.status === "settled" || job.status === "refunded") {
      return res.status(409).json({ error: "Job already " + job.status });
    }

    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordEscrowRefund(job.buyer, req.params.id, job.max_fee, epoch);
    await ledger.recordFinetuneJobRefund(req.params.id, job.buyer, "manual", epoch);

    res.json({ finetune_id: req.params.id, status: "refunded", refunded: job.max_fee });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

module.exports = router;
