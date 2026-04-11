"use strict";

/**
 * BTCPC Block Emission — five-pool reward distribution (v2.13.4)
 * Shin Devlin
 *
 * Pure function — no I/O, no DB, no P2P. Accepts a participants object and
 * the block reward amount; returns an array of reward entries plus an
 * optional recycle entry.
 *
 * Pool split:
 *   Miner pool    60%  — proportional to work_value (real inference jobs only)
 *   Verifier pool 10%  — equal split among active verifiers
 *   Clock pool     5%  — equal split among active clocks (ALWAYS paid if any active)
 *   Storage pool  15%  — equal split among storage hosts that heartbeated this epoch
 *   Service pool  10%  — equal split among service hosts that heartbeated this epoch
 *                ----
 *                100%
 *
 * Empty pools flow to btcpc_recycle. Nothing is ever burnt.
 * See feedback_no_burn_all_recycle.md.
 *
 * Participants shape:
 *   {
 *     miners:       string[]           // account names with work_value > 0
 *     minerWork:    { [account]: number }  // raw work_value per miner
 *     verifiers:    string[]
 *     clocks:       string[]
 *     storageHosts: string[]
 *     serviceHosts: string[]
 *   }
 */

const MINER_POOL_PCT    = 0.60;
const VERIFIER_POOL_PCT = 0.10;
const CLOCK_POOL_PCT    = 0.05;
const STORAGE_POOL_PCT  = 0.15;
const SERVICE_POOL_PCT  = 0.10;

function distributeBlockReward(blockReward, participants) {
  const rewards = [];
  let recycledAmount = 0;

  const miners       = participants.miners       || [];
  const minerWork    = participants.minerWork    || {};
  const verifiers    = participants.verifiers    || [];
  const clocks       = participants.clocks       || [];
  const storageHosts = participants.storageHosts || [];
  const serviceHosts = participants.serviceHosts || [];

  // ── Miner pool: 60%, proportional to work_value ──
  const minerPool = blockReward * MINER_POOL_PCT;
  const totalWork = miners.reduce((s, m) => s + (minerWork[m] || 0), 0);
  if (miners.length === 0 || totalWork === 0) {
    recycledAmount += minerPool;
  } else {
    for (const m of miners) {
      const share = parseFloat((minerPool * ((minerWork[m] || 0) / totalWork)).toFixed(10));
      rewards.push({ miner: m, amount: share, type: 'miner' });
    }
  }

  // ── Verifier pool: 10%, equal split ──
  const verifierPool = blockReward * VERIFIER_POOL_PCT;
  if (verifiers.length === 0) {
    recycledAmount += verifierPool;
  } else {
    const share = parseFloat((verifierPool / verifiers.length).toFixed(10));
    for (const v of verifiers) {
      rewards.push({ miner: v, amount: share, type: 'verifier' });
    }
  }

  // ── Clock pool: 5%, equal split, ALWAYS paid if any clocks active ──
  const clockPool = blockReward * CLOCK_POOL_PCT;
  if (clocks.length === 0) {
    recycledAmount += clockPool;
  } else {
    const share = parseFloat((clockPool / clocks.length).toFixed(10));
    for (const c of clocks) {
      rewards.push({ miner: c, amount: share, type: 'clock' });
    }
  }

  // ── Storage pool: 15%, equal split ──
  const storagePool = blockReward * STORAGE_POOL_PCT;
  if (storageHosts.length === 0) {
    recycledAmount += storagePool;
  } else {
    const share = parseFloat((storagePool / storageHosts.length).toFixed(10));
    for (const h of storageHosts) {
      rewards.push({ miner: h, amount: share, type: 'storage' });
    }
  }

  // ── Service pool: 10%, equal split ──
  const servicePool = blockReward * SERVICE_POOL_PCT;
  if (serviceHosts.length === 0) {
    recycledAmount += servicePool;
  } else {
    const share = parseFloat((servicePool / serviceHosts.length).toFixed(10));
    for (const h of serviceHosts) {
      rewards.push({ miner: h, amount: share, type: 'service' });
    }
  }

  // ── Recycle unclaimed pools — never burnt ──
  if (recycledAmount > 0) {
    rewards.push({
      miner: 'btcpc_recycle',
      amount: parseFloat(recycledAmount.toFixed(10)),
      type: 'recycle',
    });
  }

  return rewards;
}

module.exports = {
  distributeBlockReward,
  MINER_POOL_PCT,
  VERIFIER_POOL_PCT,
  CLOCK_POOL_PCT,
  STORAGE_POOL_PCT,
  SERVICE_POOL_PCT,
};
