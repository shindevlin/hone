"use strict";

/**
 * BTCPC Block Proposal Builder
 * Shin Devlin
 *
 * Pure aggregator: takes gossiped attestations (work, verifications, clock
 * heartbeats) and produces a deterministic block proposal. Multiple clocks
 * running this function over the same gossip stream will produce identical
 * proposals → consensus.
 *
 * No MongoDB. No local state. Just gossip in → proposal out.
 *
 * Roles:
 *   miners → broadcast INFERENCE_REVEAL with work_value (recorded in protocol.minerWorkByEpoch)
 *   verifiers → broadcast VERIFY_RESPONSE (recorded in protocol.verifiersByEpoch)
 *   clocks → broadcast CLOCK_HEARTBEAT (recorded in protocol.clockUptimeByEpoch)
 *           AND build BLOCK_PROPOSAL by aggregating the above
 */

var crypto = require("crypto");

// Reward split (whitepaper canonical)
var MINER_PCT = 0.85;
var VERIFIER_PCT = 0.10;
var CLOCK_PCT = 0.05;
var IDLE_VERIFIER_PCT = 0.01;
var IDLE_CLOCK_PCT = 0.01;

// Account name validation — same as everywhere else
var ACCOUNT_RE = /^[a-z0-9][a-z0-9-]{2,19}$/;

function isValidAccount(name) {
  return typeof name === "string" && ACCOUNT_RE.test(name);
}

function roundAmount(n) {
  return parseFloat(Number(n).toFixed(10));
}

/**
 * Build a deterministic block proposal for an epoch.
 *
 * @param {object} options
 * @param {number} options.epochNumber — the epoch this proposal is for
 * @param {number} options.blockReward — total emission for this epoch
 * @param {string} options.proposerAccount — clock node building this proposal
 * @param {object} options.protocol — protocol module (for getMinerWorkForEpoch, getActiveVerifiers, getActiveClockNodes)
 * @returns {object} proposal — can be broadcast as BLOCK_PROPOSAL
 */
function buildProposal(options) {
  var epochNumber = options.epochNumber;
  var blockReward = options.blockReward;
  var proposerAccount = options.proposerAccount;
  var protocol = options.protocol;

  // ── Aggregate gossiped attestations ──
  var minerWork = protocol.getMinerWorkForEpoch(epochNumber); // { miner: { work_value, job_count } }
  var activeVerifiers = (protocol.getActiveVerifiers(epochNumber) || []).filter(isValidAccount);
  var rawClocks = (protocol.getActiveClockNodes(epochNumber) || []).filter(isValidAccount);

  // Anti-self-credit: the proposer's own heartbeats only count if at least one
  // OTHER node witnessed them. Without this, a solo node could self-report
  // heartbeats and claim clock rewards with no external validation.
  var activeClocks = rawClocks;
  if (typeof protocol.getHeartbeatWitnesses === "function") {
    activeClocks = rawClocks.filter(function (clock) {
      if (clock !== proposerAccount) return true; // not the proposer — keep
      var witnesses = protocol.getHeartbeatWitnesses(clock, epochNumber);
      // The proposer's own nodeId doesn't count. We need at least one witness
      // that is NOT the proposer's own relay of its heartbeat. Since nodeIds
      // are opaque hex, we check if there are >= 2 distinct witnesses (self +
      // at least one other) or if the set is non-empty from P2P (heartbeats
      // only get recorded in handleClockHeartbeat which runs on receipt from
      // a PEER, not on local send — so any witness entry means another node
      // relayed it).
      return witnesses.size > 0;
    });
  }

  // ── Filter and sort miners (deterministic order) ──
  var miners = Object.keys(minerWork).filter(isValidAccount).sort();
  var totalWorkValue = 0;
  for (var m of miners) totalWorkValue += (minerWork[m].work_value || 0);

  // Sort verifiers and clocks deterministically
  activeVerifiers.sort();
  activeClocks.sort();

  var rewards = [];

  if (miners.length === 0 || totalWorkValue === 0) {
    // ── Idle epoch: 98% unminted, 1% to verifiers, 1% to clocks ──
    if (activeVerifiers.length > 0) {
      var vShare = roundAmount(blockReward * IDLE_VERIFIER_PCT / activeVerifiers.length);
      for (var v of activeVerifiers) {
        rewards.push({ to: v, amount: vShare, type: "verifier" });
      }
    }
    if (activeClocks.length > 0) {
      var cShare = roundAmount(blockReward * IDLE_CLOCK_PCT / activeClocks.length);
      for (var c of activeClocks) {
        var existing = rewards.find(function (r) { return r.to === c; });
        if (existing) {
          existing.amount = roundAmount(existing.amount + cShare);
        } else {
          rewards.push({ to: c, amount: cShare, type: "clock" });
        }
      }
    }
  } else {
    // ── Active epoch: 85% miners (by work), 10% verifiers (split), 5% clocks (split) ──
    var minerPool = roundAmount(blockReward * MINER_PCT);
    var verifierPool = roundAmount(blockReward * VERIFIER_PCT);
    var clockPool = roundAmount(blockReward * CLOCK_PCT);

    // Miners — proportional to work
    for (var miner of miners) {
      var share = roundAmount(minerPool * (minerWork[miner].work_value / totalWorkValue));
      rewards.push({ to: miner, amount: share, type: "mining" });
    }

    // Verifiers — split equally (or redistribute to miners if none)
    if (activeVerifiers.length > 0) {
      var vEqual = roundAmount(verifierPool / activeVerifiers.length);
      for (var ver of activeVerifiers) {
        rewards.push({ to: ver, amount: vEqual, type: "verifier" });
      }
    } else {
      // No verifiers — redistribute to miners
      var extraPerMiner = roundAmount(verifierPool / miners.length);
      for (var r of rewards) {
        if (r.type === "mining") r.amount = roundAmount(r.amount + extraPerMiner);
      }
    }

    // Clocks — split equally (or redistribute to miners if none)
    if (activeClocks.length > 0) {
      var cEqual = roundAmount(clockPool / activeClocks.length);
      for (var clk of activeClocks) {
        var existing2 = rewards.find(function (r) { return r.to === clk; });
        if (existing2) {
          existing2.amount = roundAmount(existing2.amount + cEqual);
        } else {
          rewards.push({ to: clk, amount: cEqual, type: "clock" });
        }
      }
    } else {
      var extraClock = roundAmount(clockPool / miners.length);
      for (var r2 of rewards) {
        if (r2.type === "mining") r2.amount = roundAmount(r2.amount + extraClock);
      }
    }
  }

  // ── Sort rewards deterministically (by recipient name) for hashing ──
  rewards.sort(function (a, b) { return a.to < b.to ? -1 : a.to > b.to ? 1 : 0; });

  // ── Compute deterministic consensus hash ──
  // Same gossip → same hash on every clock running this function
  var consensusInput = JSON.stringify({
    epoch: epochNumber,
    block_reward: blockReward,
    total_work: totalWorkValue,
    miners: miners,
    verifiers: activeVerifiers,
    clocks: activeClocks,
    rewards: rewards.map(function (r) { return [r.to, r.amount, r.type]; }),
  });
  var consensusHash = crypto.createHash("sha256").update(consensusInput).digest("hex");

  return {
    epoch_number: epochNumber,
    proposer: proposerAccount,
    proposer_role: "clock",
    block_reward: blockReward,
    rewards: rewards,
    miners_active: miners.length,
    verifiers_active: activeVerifiers.length,
    clocks_active: activeClocks.length,
    total_work: totalWorkValue,
    consensus_hash: consensusHash,
    timestamp: Date.now(),
  };
}

module.exports = {
  buildProposal: buildProposal,
  MINER_PCT: MINER_PCT,
  VERIFIER_PCT: VERIFIER_PCT,
  CLOCK_PCT: CLOCK_PCT,
  IDLE_VERIFIER_PCT: IDLE_VERIFIER_PCT,
  IDLE_CLOCK_PCT: IDLE_CLOCK_PCT,
};
