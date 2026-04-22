"use strict";

/**
 * Unified marketplace TTL sweep — runs every epoch.
 * Shin Devlin
 *
 * Handles all three marketplace types:
 *   - Inference jobs (open/claimed past expires_epoch → auto-refund)
 *   - Browser jobs  (open/claimed/action_pending past expires_epoch → close session + refund)
 *   - Fine-tuning jobs (open/training past expires_epoch → refund)
 *
 * Called from the miner's epoch finalization loop alongside escrow.sweepEscrows().
 * Safe to call multiple times per epoch (idempotent on terminal states).
 */

const stateStore = require("../chain/stateStore");
const ledger = require("./ledger");
const { sweepExpiredJobs } = require("./inferenceMarket");

async function _sweepBrowserJobs(epoch) {
  const active = stateStore.getOpenBrowserJobs ? stateStore.getOpenBrowserJobs() : [];
  let swept = 0;

  for (const job of active) {
    if (!job.expires_epoch || epoch <= job.expires_epoch) continue;
    if (["settled", "refunded"].includes(job.status)) continue;

    try {
      // Close any live Playwright session
      try {
        const { closeSession, deleteSessionBlobs } = require("./browserRunner");
        const screenshotCids = (job.turns || []).map((t) => t.screenshot_cid).filter(Boolean);
        await closeSession(job.job_id);
        deleteSessionBlobs(screenshotCids);
      } catch (_) {}

      // Refund escrow to buyer
      await ledger.recordEscrowRefund(job.buyer, job.job_id, job.max_fee, epoch);
      await ledger.recordBrowserJobRefund(job.job_id, job.buyer, "ttl_expired", epoch);
      swept++;
      console.log(`[MarketplaceSweep] Browser job ${job.job_id} expired at epoch ${epoch} — refunded ${job.max_fee} BTCPC to ${job.buyer}`);
    } catch (err) {
      console.warn(`[MarketplaceSweep] Failed to sweep browser job ${job.job_id}: ${err.message}`);
    }
  }
  return swept;
}

async function _sweepFinetuneJobs(epoch) {
  const active = stateStore.getOpenFinetuneJobs ? stateStore.getOpenFinetuneJobs() : [];
  let swept = 0;

  for (const job of active) {
    if (!job.expires_epoch || epoch <= job.expires_epoch) continue;
    if (["settled", "refunded"].includes(job.status)) continue;

    try {
      await ledger.recordEscrowRefund(job.buyer, job.finetune_id, job.fee, epoch);
      await ledger.recordFinetuneJobRefund(job.finetune_id, job.buyer, "ttl_expired", epoch);
      swept++;
      console.log(`[MarketplaceSweep] Finetune job ${job.finetune_id} expired at epoch ${epoch} — refunded ${job.fee} BTCPC to ${job.buyer}`);
    } catch (err) {
      console.warn(`[MarketplaceSweep] Failed to sweep finetune job ${job.finetune_id}: ${err.message}`);
    }
  }
  return swept;
}

/**
 * Sweep all marketplace types for expired TTLs. Call once per epoch.
 */
async function sweepAllMarketplaces() {
  const epoch = await ledger.getCurrentEpoch();
  const results = await Promise.allSettled([
    sweepExpiredJobs(),
    _sweepBrowserJobs(epoch),
    _sweepFinetuneJobs(epoch),
  ]);

  const counts = results.map((r) => (r.status === "fulfilled" ? r.value || 0 : 0));
  const total = counts.reduce((a, b) => a + b, 0);
  if (total > 0) {
    console.log(`[MarketplaceSweep] Swept ${counts[0]} inference, ${counts[1]} browser, ${counts[2]} finetune expired jobs`);
  }
}

module.exports = { sweepAllMarketplaces };
