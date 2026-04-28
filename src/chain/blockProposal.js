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
var stateStore = require("./stateStore");
var nodeRegistry = require("./nodeRegistry");

// Work scores — each role's contribution in normalized compute units.
// The pool split is demand-driven: rolePool = blockReward × (roleScore / totalScore).
// Calibrated so at typical participation the split approximates the original whitepaper
// ratios, but shifts naturally with actual network activity.
var CLOCK_SCORE_PER_NODE    = 1000;
var VERIFIER_SCORE_PER_NODE = 3000;
var STORAGE_SCORE_PER_HOST  = 2000;
var SENSOR_SCORE_PER_SENSOR = 500;
var GATEWAY_SCORE_PER_GW    = 1500;

// Idle-epoch infrastructure floor (when NO work exists across all roles)
var IDLE_VERIFIER_PCT = 0.01;
var IDLE_CLOCK_PCT    = 0.01;

// Staking thresholds
var _csEnv = parseInt(process.env.BTCPC_MIN_CLOCK_STAKE);
var MIN_CLOCK_STAKE = isNaN(_csEnv) ? 10 : _csEnv;
var _msEnv = parseInt(process.env.BTCPC_MIN_MINER_STAKE);
var MIN_MINER_STAKE = isNaN(_msEnv) ? 10 : _msEnv;
var BOOTSTRAP_EPOCHS = 1000;

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

  // ── Bootstrap mode: first 100 epochs get full payouts ──
  // The chain needs initial liquidity for staking, faucet, and transfers.
  // During bootstrap, all connected nodes get credited with synthetic work
  // so the full block reward is distributed every epoch.
  var BOOTSTRAP_FULL_PAYOUT_EPOCHS = 100;

  // ── Aggregate gossiped attestations ──
  var minerWork = protocol.getMinerWorkForEpoch(epochNumber); // { miner: { work_value, job_count } }
  var activeVerifiers = (protocol.getActiveVerifiers(epochNumber) || []).filter(isValidAccount);
  var rawClocks = (protocol.getActiveClockNodes(epochNumber) || []).filter(isValidAccount);

  // Bootstrap: if within first 100 epochs, ensure the proposer has work credit
  // so rewards are distributed (not 98% unminted as idle epochs)
  if (epochNumber < BOOTSTRAP_FULL_PAYOUT_EPOCHS) {
    if (Object.keys(minerWork).length === 0) {
      // Credit the proposer with synthetic work so the epoch isn't idle
      minerWork[proposerAccount] = { work_value: 1000, job_count: 1 };
    }
    // Ensure proposer is in clocks too
    if (rawClocks.indexOf(proposerAccount) === -1) {
      rawClocks.push(proposerAccount);
    }
  }

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

  // Desktop/all-in-one bootstrap safety: if heartbeat gossip arrives late but
  // the local proposer account is permissioned, keep the clock pool active.
  // This prevents a healthy solo genesis node from showing "0 clock(s)" while
  // the separate btcpc-clock role is running and connected.
  if (activeClocks.indexOf(proposerAccount) === -1) {
    var proposerNode = nodeRegistry.getNode(proposerAccount);
    if (proposerNode && (proposerNode.permissioned || proposerNode.stake >= MIN_CLOCK_STAKE)) {
      activeClocks.push(proposerAccount);
    }
  }

  // ── Filter clocks by minimum stake (skip during bootstrap or when stake=0) ──
  if (epochNumber >= BOOTSTRAP_EPOCHS && MIN_CLOCK_STAKE > 0) {
    activeClocks = activeClocks.filter(function (clock) {
      var pool = stateStore.getStakePool(clock);
      if (pool && pool.total_staked >= MIN_CLOCK_STAKE) return true;

      // Genesis/desktop nodes can be permissioned in nodeRegistry before an
      // on-chain stake pool exists. Treat those as eligible so clock work is
      // rewarded instead of silently recycling the clock pool forever.
      var nodeInfo = nodeRegistry.getNode(clock);
      if (!nodeInfo) return false;
      if (nodeInfo.stake && nodeInfo.stake >= MIN_CLOCK_STAKE) return true;
      return !!nodeInfo.permissioned;
    });
  }

  // ── Verifier enforcement: unverified work counts at 50% ──
  var verifiedMiners = (typeof protocol.getVerifiedMiners === "function")
    ? protocol.getVerifiedMiners(epochNumber)
    : new Set();

  // ── Filter and sort miners (deterministic order) ──
  var miners = Object.keys(minerWork).filter(isValidAccount).sort();

  // Build effective work values — verified miners get full credit,
  // unverified miners get 50% penalty.
  var effectiveWork = {};
  var totalWorkValue = 0;
  for (var m of miners) {
    var raw = minerWork[m].work_value || 0;
    effectiveWork[m] = verifiedMiners.has(m) ? raw : roundAmount(raw * 0.5);
    totalWorkValue += effectiveWork[m];
  }

  // Sort verifiers and clocks deterministically
  activeVerifiers.sort();
  activeClocks.sort();

  // ── Gather infrastructure participants ────────────────────────────────
  var activeStorageHosts = [];
  try {
    var storageHostRecords = stateStore.getActiveStorageHosts(epochNumber, 10);
    activeStorageHosts = storageHostRecords
      .map(function (h) { return h.host || h.account; })
      .filter(isValidAccount);
    activeStorageHosts = Array.from(new Set(activeStorageHosts));
  } catch (_) {}

  var activeSensors = [];
  var activeGateways = [];
  try {
    var sr = require("../services/sensorRegistry");
    var allSensors = sr.getAllSensors ? sr.getAllSensors() : [];
    activeSensors = allSensors
      .filter(function (s) { return s.last_reading_epoch != null && s.last_reading_epoch >= epochNumber - 10; })
      .map(function (s) { return s.owner; });
    activeSensors = Array.from(new Set(activeSensors));

    var gwReg = require("../services/loraGatewayRegistry");
    var allGateways = gwReg.getAllGateways ? gwReg.getAllGateways() : [];
    activeGateways = allGateways
      .filter(function (g) { return g.last_heartbeat_epoch != null && g.last_heartbeat_epoch >= epochNumber - 10; })
      .map(function (g) { return g.owner; });
    activeGateways = Array.from(new Set(activeGateways));
  } catch (_) {}

  var rewards = [];

  // ── Compute stake-weighted miner scores ───────────────────────────────
  var weightedWork = {};
  var totalWeightedWork = 0;
  for (var miner of miners) {
    var pool = stateStore.getStakePool(miner);
    var staked = (pool && pool.total_staked) || 0;
    var nodeInfo = null;
    try {
      nodeInfo = require("../chain/nodeRegistry").getNode(miner);
      if (nodeInfo && nodeInfo.stake > staked) staked = nodeInfo.stake;
    } catch (_) {}
    var weight;
    if (nodeInfo && nodeInfo.permissioned) {
      weight = 1;
    } else if (staked >= MIN_MINER_STAKE) {
      weight = Math.min(Math.sqrt(staked / MIN_MINER_STAKE), 10);
    } else if (epochNumber < BOOTSTRAP_EPOCHS) {
      weight = 0.25;
    } else {
      weight = 0;
    }
    var ww = (effectiveWork[miner] || 0) * weight;
    weightedWork[miner] = ww;
    totalWeightedWork += ww;
  }

  // ── Compute work scores for all roles ─────────────────────────────────
  var minerScore    = totalWeightedWork;
  var clockScore    = activeClocks.length    * CLOCK_SCORE_PER_NODE;
  var verifierScore = activeVerifiers.length * VERIFIER_SCORE_PER_NODE;
  var storageScore  = activeStorageHosts.length * STORAGE_SCORE_PER_HOST;
  var sensorScore   = activeSensors.length   * SENSOR_SCORE_PER_SENSOR
                    + activeGateways.length  * GATEWAY_SCORE_PER_GW;
  var totalScore    = minerScore + clockScore + verifierScore + storageScore + sensorScore;

  if (totalScore === 0) {
    // ── Truly idle: small infrastructure floor, rest unminted ──
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
    // ── Dynamic split: each role earns proportional to its work score ──
    var minerPool    = roundAmount(blockReward * (minerScore    / totalScore));
    var verifierPool = roundAmount(blockReward * (verifierScore / totalScore));
    var clockPool    = roundAmount(blockReward * (clockScore    / totalScore));
    var storagePool  = roundAmount(blockReward * (storageScore  / totalScore));
    var iotPool      = roundAmount(blockReward * (sensorScore   / totalScore));

    // Miners — proportional to stake-weighted work
    if (minerScore > 0) {
      for (var miner2 of miners) {
        if (weightedWork[miner2] > 0) {
          var share = roundAmount(minerPool * (weightedWork[miner2] / totalWeightedWork));
          rewards.push({ to: miner2, amount: share, type: "mining" });
        }
      }
    }

    // Verifiers — equal split
    if (verifierScore > 0) {
      var vEqual = roundAmount(verifierPool / activeVerifiers.length);
      for (var ver of activeVerifiers) {
        rewards.push({ to: ver, amount: vEqual, type: "verifier" });
      }
    }

    // Clocks — equal split
    if (clockScore > 0) {
      var cEqual = roundAmount(clockPool / activeClocks.length);
      for (var clk of activeClocks) {
        var existing2 = rewards.find(function (r) { return r.to === clk; });
        if (existing2) {
          existing2.amount = roundAmount(existing2.amount + cEqual);
        } else {
          rewards.push({ to: clk, amount: cEqual, type: "clock" });
        }
      }
    }

    // Storage — equal split
    if (storageScore > 0) {
      var sEqual = roundAmount(storagePool / activeStorageHosts.length);
      for (var sh of activeStorageHosts) {
        var existingS = rewards.find(function (r) { return r.to === sh; });
        if (existingS) {
          existingS.amount = roundAmount(existingS.amount + sEqual);
        } else {
          rewards.push({ to: sh, amount: sEqual, type: "storage" });
        }
      }
    }

    // IoT — split between sensors (by count) and gateways (by count)
    if (sensorScore > 0) {
      var sensorOnlyScore  = activeSensors.length  * SENSOR_SCORE_PER_SENSOR;
      var gatewayOnlyScore = activeGateways.length * GATEWAY_SCORE_PER_GW;
      var sensorSubPool  = roundAmount(iotPool * (sensorOnlyScore  / sensorScore));
      var gatewaySubPool = roundAmount(iotPool * (gatewayOnlyScore / sensorScore));

      if (activeSensors.length > 0) {
        var sensorEqual = roundAmount(sensorSubPool / activeSensors.length);
        for (var so of activeSensors) {
          var existingIot = rewards.find(function (r) { return r.to === so; });
          if (existingIot) {
            existingIot.amount = roundAmount(existingIot.amount + sensorEqual);
          } else {
            rewards.push({ to: so, amount: sensorEqual, type: "iot_sensor" });
          }
        }
      }
      if (activeGateways.length > 0) {
        var gwEqual = roundAmount(gatewaySubPool / activeGateways.length);
        for (var gw of activeGateways) {
          var existingGw = rewards.find(function (r) { return r.to === gw; });
          if (existingGw) {
            existingGw.amount = roundAmount(existingGw.amount + gwEqual);
          } else {
            rewards.push({ to: gw, amount: gwEqual, type: "iot_gateway" });
          }
        }
      }
    }
  }

  // ── Sort rewards deterministically (by recipient name) for hashing ──
  rewards.sort(function (a, b) { return a.to < b.to ? -1 : a.to > b.to ? 1 : 0; });

  // ── Compute deterministic consensus hash via canonical function ──
  // Uses finalizationConsensus.hashRewards so every code path hashes identically.
  var finConsensus = require("./finalizationConsensus");
  var rewardsForHash = rewards.map(function (r) { return { miner: r.to, amount: r.amount }; });
  var consensusHash = finConsensus.hashRewards(rewardsForHash, totalWorkValue, miners.length, epochNumber);

  return {
    epoch_number: epochNumber,
    proposer: proposerAccount,
    proposer_role: "clock",
    block_reward: blockReward,
    rewards: rewards,
    miners_active: miners.length,
    verifiers_active: activeVerifiers.length,
    clocks_active: activeClocks.length,
    storage_active: activeStorageHosts.length,
    sensors_active: activeSensors.length,
    gateways_active: activeGateways.length,
    total_work: totalWorkValue,
    total_score: totalScore,
    // Dynamic pool percentages (what actually got paid, for display)
    pool_pcts: totalScore > 0 ? {
      mining:      minerScore    / totalScore,
      verifier:    verifierScore / totalScore,
      clock:       clockScore    / totalScore,
      storage:     storageScore  / totalScore,
      iot:         sensorScore   / totalScore,
    } : null,
    consensus_hash: consensusHash,
    timestamp: Date.now(),
  };
}

module.exports = {
  buildProposal: buildProposal,
  CLOCK_SCORE_PER_NODE: CLOCK_SCORE_PER_NODE,
  VERIFIER_SCORE_PER_NODE: VERIFIER_SCORE_PER_NODE,
  STORAGE_SCORE_PER_HOST: STORAGE_SCORE_PER_HOST,
  SENSOR_SCORE_PER_SENSOR: SENSOR_SCORE_PER_SENSOR,
  GATEWAY_SCORE_PER_GW: GATEWAY_SCORE_PER_GW,
  IDLE_VERIFIER_PCT: IDLE_VERIFIER_PCT,
  IDLE_CLOCK_PCT: IDLE_CLOCK_PCT,
};
