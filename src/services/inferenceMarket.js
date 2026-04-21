"use strict";

/**
 * Inference Marketplace — Verasens / BTCPC Network
 * Shin Devlin
 *
 * Connects compute buyers to miners. Buyers post jobs with a BTCPC escrow.
 * Miners claim, run inference via Ollama, submit a result hash. Verifier
 * nodes confirm. Protocol takes 10% on settlement.
 *
 * Job lifecycle:
 *   open → claimed → submitted → settled
 *                 ↘ expired → refunded
 *
 * All money moves through the existing escrow system. Job metadata is
 * tracked via INFERENCE_JOB_* ledger entries and the inferenceJobs Map
 * in stateStore (rebuilt on replay, same as commerce orders).
 */

const crypto = require("crypto");
const stateStore = require("../chain/stateStore");
const ledger = require("./ledger");

const PROTOCOL_FEE_ACCOUNT = "btcpc_fees";
const PROTOCOL_FEE_PCT = 0.10;
const DEFAULT_TTL_EPOCHS = 20; // 10 minutes at 30s/epoch
const MIN_JOB_FEE = 0.01; // 0.01 BTCPC minimum

function _jobId() {
  return "job_" + crypto.randomBytes(8).toString("hex");
}

/**
 * Buyer opens a marketplace job. Escrow is locked immediately.
 * Returns the jobId that miners can claim.
 *
 * @param {string} buyer - BTCPC account name
 * @param {string} prompt - The inference prompt
 * @param {number} maxFee - BTCPC to escrow (miner earns 90%, protocol 10%)
 * @param {object} opts - { model, ttlEpochs, systemPrompt, streaming }
 */
async function openJob(buyer, prompt, maxFee, opts) {
  opts = opts || {};
  if (!buyer || !prompt) throw new Error("buyer and prompt required");
  if (!maxFee || maxFee < MIN_JOB_FEE) {
    throw new Error(`maxFee must be at least ${MIN_JOB_FEE} BTCPC`);
  }

  const balance = stateStore.getBalance(buyer, "BTCPC");
  if (balance < maxFee) {
    throw new Error(`Insufficient balance: have ${balance} BTCPC, need ${maxFee}`);
  }

  const jobId = _jobId();
  const epoch = await ledger.getCurrentEpoch();
  const ttlEpochs = opts.ttlEpochs || DEFAULT_TTL_EPOCHS;

  // Lock escrow first
  await ledger.recordEscrowLock(buyer, jobId, maxFee, epoch);

  // Record job opening on chain
  await ledger.recordInferenceJobOpen(buyer, jobId, {
    prompt,
    max_fee: maxFee,
    model: opts.model || null,
    system_prompt: opts.systemPrompt || null,
    ttl_epochs: ttlEpochs,
    expires_epoch: epoch + ttlEpochs,
  }, epoch);

  return { job_id: jobId, buyer, max_fee: maxFee, status: "open", epoch };
}

/**
 * Miner claims an open job. Job transitions to 'claimed'.
 * Only one miner can claim a job at a time.
 *
 * @param {string} jobId
 * @param {string} miner - BTCPC account name
 */
async function claimJob(jobId, miner) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (job.status !== "open") throw new Error("Job not open (status: " + job.status + ")");

  const epoch = await ledger.getCurrentEpoch();
  if (job.expires_epoch && epoch > job.expires_epoch) {
    // Auto-expire
    await _expireJob(jobId, job, epoch);
    throw new Error("Job expired");
  }

  await ledger.recordInferenceJobClaim(jobId, miner, epoch);
  return { job_id: jobId, miner, status: "claimed", epoch };
}

/**
 * Miner submits inference result. Includes the response text and a hash
 * for verification. Job transitions to 'submitted'.
 *
 * @param {string} jobId
 * @param {string} miner
 * @param {string} result - The actual response text
 * @param {string} proofHash - sha256(prompt + result + miner) for light verification
 */
async function submitJob(jobId, miner, result, proofHash) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (job.status !== "claimed") throw new Error("Job not claimed (status: " + job.status + ")");
  if (job.miner !== miner) throw new Error("Not your job");
  if (!result) throw new Error("result required");

  const epoch = await ledger.getCurrentEpoch();

  // Compute canonical proof hash if not provided
  if (!proofHash) {
    proofHash = crypto
      .createHash("sha256")
      .update(job.prompt + result + miner)
      .digest("hex");
  }

  await ledger.recordInferenceJobSubmit(jobId, miner, proofHash, epoch);
  return { job_id: jobId, miner, status: "submitted", proof_hash: proofHash, epoch };
}

/**
 * Settle a submitted job. Verifier (or auto-settle after timeout) calls this.
 * Releases 90% escrow to miner, 10% to btcpc_fees, refunds overpayment.
 *
 * @param {string} jobId
 * @param {number} actualCost - How much BTCPC the job actually cost (≤ maxFee)
 * @param {string} settledBy - Account settling (verifier or 'auto')
 */
async function settleJob(jobId, actualCost, settledBy) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (job.status !== "submitted") throw new Error("Job not submitted (status: " + job.status + ")");

  const epoch = await ledger.getCurrentEpoch();

  // Clamp actual cost to max_fee
  actualCost = Math.min(actualCost || job.max_fee, job.max_fee);

  const protocolFee = parseFloat((actualCost * PROTOCOL_FEE_PCT).toFixed(10));
  const minerPayout = parseFloat((actualCost - protocolFee).toFixed(10));
  const overpayment = parseFloat((job.max_fee - actualCost).toFixed(10));

  // Release miner payout from escrow
  if (minerPayout > 0) {
    await ledger.recordEscrowRelease(job.miner, jobId, minerPayout, epoch, "Inference job settlement");
  }

  // Protocol fee — separate ESCROW_RELEASE to protocol account
  if (protocolFee > 0) {
    await ledger.recordEscrowRelease(PROTOCOL_FEE_ACCOUNT, jobId, protocolFee, epoch, "Inference protocol fee 10%");
  }

  // Refund overpayment to buyer
  if (overpayment > 0.000001) {
    await ledger.recordEscrowRefund(job.buyer, jobId, overpayment, epoch);
  }

  // Record settlement on chain for explorer + reputation
  await ledger.recordInferenceJobSettle(jobId, job.miner, job.buyer, {
    actual_cost: actualCost,
    miner_payout: minerPayout,
    protocol_fee: protocolFee,
    overpayment,
    settled_by: settledBy || "auto",
  }, epoch);

  // Update miner reputation
  try {
    await ledger.recordNodeReputationUpdate(job.miner, "inference", true, epoch);
  } catch (_) {}

  return {
    job_id: jobId,
    status: "settled",
    miner: job.miner,
    buyer: job.buyer,
    miner_payout: minerPayout,
    protocol_fee: protocolFee,
    overpayment,
  };
}

/**
 * Refund an expired or cancelled job back to the buyer.
 */
async function refundJob(jobId) {
  const job = stateStore.getInferenceJob(jobId);
  if (!job) throw new Error("Job not found: " + jobId);
  if (job.status === "settled" || job.status === "refunded") {
    throw new Error("Job already " + job.status);
  }

  const epoch = await ledger.getCurrentEpoch();
  await _expireJob(jobId, job, epoch);
  return { job_id: jobId, status: "refunded", buyer: job.buyer, amount: job.max_fee };
}

async function _expireJob(jobId, job, epoch) {
  await ledger.recordEscrowRefund(job.buyer, jobId, job.max_fee, epoch);
  await ledger.recordInferenceJobRefund(jobId, job.buyer, "expired", epoch);
}

/**
 * Sweep expired jobs — auto-refund any open/claimed jobs past their TTL.
 * Called periodically by the epoch manager.
 */
async function sweepExpiredJobs() {
  const epoch = await ledger.getCurrentEpoch();
  const open = stateStore.getOpenInferenceJobs();
  let swept = 0;

  for (const job of open) {
    if (job.expires_epoch && epoch > job.expires_epoch) {
      try {
        await _expireJob(job.job_id, job, epoch);
        swept++;
      } catch (_) {}
    }
  }

  if (swept > 0) {
    console.log(`[InferenceMarket] Swept ${swept} expired jobs at epoch ${epoch}`);
  }
  return { swept, epoch };
}

/**
 * Get paginated list of open jobs, optionally filtered by model.
 */
function getOpenJobs(opts) {
  opts = opts || {};
  const limit = Math.min(opts.limit || 50, 200);
  const model = opts.model || null;
  const offset = opts.offset || 0;

  let jobs = stateStore.getOpenInferenceJobs();
  if (model) jobs = jobs.filter((j) => j.model === model);
  return {
    jobs: jobs.slice(offset, offset + limit),
    total: jobs.length,
    limit,
    offset,
  };
}

/**
 * Get jobs posted by a specific buyer.
 */
function getBuyerJobs(buyer, opts) {
  opts = opts || {};
  const limit = Math.min(opts.limit || 50, 200);
  return stateStore.getInferenceJobsByBuyer(buyer).slice(0, limit);
}

/**
 * Get jobs claimed by a specific miner.
 */
function getMinerJobs(miner, opts) {
  opts = opts || {};
  const limit = Math.min(opts.limit || 50, 200);
  return stateStore.getInferenceJobsByMiner(miner).slice(0, limit);
}

module.exports = {
  openJob,
  claimJob,
  submitJob,
  settleJob,
  refundJob,
  sweepExpiredJobs,
  getOpenJobs,
  getBuyerJobs,
  getMinerJobs,
  PROTOCOL_FEE_PCT,
  MIN_JOB_FEE,
};
