"use strict";

/**
 * HONE Block Proposal Builder
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

// Scarcity: below this node count, per-node score is multiplied to attract participation
var SCARCITY_THRESHOLD  = 3;
var SCARCITY_MULTIPLIER = 2.5;

// Testnet carveout — 0.1% off the top for nodes running the public test network
var TESTNET_FRACTION = 0.001;

function scarcityScore(count, basePerNode) {
  if (count === 0) return 0;
  return count * (count < SCARCITY_THRESHOLD ? basePerNode * SCARCITY_MULTIPLIER : basePerNode);
}

// Staking thresholds
var _csEnv = parseInt(process.env.HONE_MIN_CLOCK_STAKE);
var MIN_CLOCK_STAKE = isNaN(_csEnv) ? 10 : _csEnv;
var _msEnv = parseInt(process.env.HONE_MIN_MINER_STAKE);
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
  // the separate hone-clock role is running and connected.
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

  // ── Testnet nodes ─────────────────────────────────────────────────────
  var activeTestnetNodes = [];
  try {
    activeTestnetNodes = protocol.getActiveTestnetNodes(epochNumber).filter(isValidAccount);
  } catch (_) {}

  // ── Service hosts (HONE hosted services) ─────────────────────────────
  var activeServiceHosts = [];
  var serviceWork = {};
  try {
    var svcReg = require("../services/serviceRegistry");
    var allServices = svcReg.getAllServices ? svcReg.getAllServices() : [];
    // Collect unique host accounts that served requests in the recent window
    var svcHostSet = new Set();
    for (var svc of allServices) {
      var hosts = svcReg.getActiveHostsForService
        ? svcReg.getActiveHostsForService(svc.slug, epochNumber, 10)
        : [];
      for (var hst of hosts) {
        var acct = typeof hst === "string" ? hst : (hst.host || hst.account);
        if (isValidAccount(acct)) svcHostSet.add(acct);
      }
    }
    activeServiceHosts = Array.from(svcHostSet);
    if (svcReg.getServiceWorkByEpoch) {
      serviceWork = svcReg.getServiceWorkByEpoch(epochNumber);
    }
  } catch (_) {}

  // ── Gather infrastructure participants ────────────────────────────────
  var activeStorageHosts = [];
  var storageWork = {};
  try {
    var storageHostRecords = stateStore.getActiveStorageHosts(epochNumber, 10);
    activeStorageHosts = storageHostRecords
      .map(function (h) { return h.host || h.account; })
      .filter(isValidAccount);
    activeStorageHosts = Array.from(new Set(activeStorageHosts));
    if (stateStore.getStorageWorkByEpoch) {
      storageWork = stateStore.getStorageWorkByEpoch(epochNumber);
    }
  } catch (_) {}

  var activeSensors = [];
  var sensorWork = {};
  var sensorRevenue = {};
  var activeGateways = [];
  var gatewayWork = {};
  try {
    var sr = require("../services/sensorRegistry");
    var allSensors = sr.getAllSensors ? sr.getAllSensors() : [];
    activeSensors = allSensors
      .filter(function (s) { return s.last_reading_epoch != null && s.last_reading_epoch >= epochNumber - 10; })
      .map(function (s) { return s.owner; });
    activeSensors = Array.from(new Set(activeSensors));
    if (sr.getSensorWorkByEpoch) {
      sensorWork = sr.getSensorWorkByEpoch(epochNumber);
    }
    if (sr.getSensorRevenueByEpoch) {
      sensorRevenue = sr.getSensorRevenueByEpoch(epochNumber);
    }

    var gwReg = require("../services/loraGatewayRegistry");
    var allGateways = gwReg.getAllGateways ? gwReg.getAllGateways() : [];
    activeGateways = allGateways
      .filter(function (g) { return g.last_heartbeat_epoch != null && g.last_heartbeat_epoch >= epochNumber - 10; })
      .map(function (g) { return g.owner; });
    activeGateways = Array.from(new Set(activeGateways));
    if (gwReg.getGatewayWorkByEpoch) {
      gatewayWork = gwReg.getGatewayWorkByEpoch(epochNumber);
    }
  } catch (_) {}

  // ── Gather verifier work map ──────────────────────────────────────────
  var verifierWork = {};
  try {
    if (typeof protocol.getVerifierWorkByEpoch === "function") {
      verifierWork = protocol.getVerifierWorkByEpoch(epochNumber);
    }
  } catch (_) {}

  var rewards = [];

  // Per-unit work score constants (same calibration as rewardDistribution.js)
  var VERIFIER_SCORE_PER_CHECK  = 300;
  var STORAGE_SCORE_PER_MB      = 2;
  var SENSOR_SCORE_PER_READING  = 50;
  var GATEWAY_SCORE_PER_PACKET  = 15;
  var SERVICE_SCORE_PER_REQUEST = 25;

  // Helper: real work-based score, falls back to count-based proxy with scarcity
  function _roleScore(participants, workMap, perUnit, perNode, unitScale) {
    if (!participants || participants.length === 0) return 0;
    unitScale = unitScale || 1;
    if (workMap) {
      var tw = 0;
      for (var _p of participants) tw += (workMap[_p] || 0);
      if (tw > 0) return (tw / unitScale) * perUnit;
    }
    return scarcityScore(participants.length, perNode);
  }

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

  // ── Activity-gated emission ───────────────────────────────────────────
  // Actual emission scales with network activity. Full emission when the
  // network is busy; as low as 1% when activity is minimal; 0 if totally
  // idle. Unissued tokens simply aren't minted — no recycle, no inflation.
  //
  // HONE_FULL_ACTIVITY_SCORE: the totalScore that earns 100% emission.
  // Default calibrated so a small but live network earns ~20-30%.
  // Tune upward as the network grows.
  var FULL_ACTIVITY_SCORE = parseInt(process.env.HONE_FULL_ACTIVITY_SCORE) || 70000;
  var MIN_EMISSION_RATIO  = 0.01; // 1% floor when ANY activity exists

  // Compute scores now (needed for emission gate before testnet split)
  var minerScore    = totalWeightedWork;
  var clockScore    = scarcityScore(activeClocks.length, CLOCK_SCORE_PER_NODE);
  var verifierScore = _roleScore(activeVerifiers, verifierWork, VERIFIER_SCORE_PER_CHECK, VERIFIER_SCORE_PER_NODE, 1);
  var storageScore  = _roleScore(activeStorageHosts, storageWork, STORAGE_SCORE_PER_MB, STORAGE_SCORE_PER_HOST, 1048576);
  // Sensor hybrid score: base (liveness) + revenue (demand) per active sensor
  var _sensorHasRevenue = Object.keys(sensorRevenue).length > 0;
  var _sensorHasWork    = Object.keys(sensorWork).length > 0;
  var _sensorBaseTotal  = 0;
  for (var _sni of activeSensors) {
    if (_sensorHasWork && !(sensorWork[_sni] > 0)) continue; // skip dead sensors
    _sensorBaseTotal += 100 + ((_sensorHasRevenue ? (sensorRevenue[_sni] || 0) : 0) * 5000);
  }
  var sensorRawScore  = _sensorBaseTotal > 0
    ? _sensorBaseTotal
    : _roleScore(activeSensors, sensorWork, SENSOR_SCORE_PER_READING, SENSOR_SCORE_PER_SENSOR, 1);
  var gatewayRawScore = _roleScore(activeGateways, gatewayWork, GATEWAY_SCORE_PER_PACKET, GATEWAY_SCORE_PER_GW,    1);
  var sensorScore   = sensorRawScore + gatewayRawScore;
  var serviceScore  = _roleScore(activeServiceHosts, serviceWork, SERVICE_SCORE_PER_REQUEST, SERVICE_SCORE_PER_HOST, 1);
  var totalScore    = minerScore + clockScore + verifierScore
                    + storageScore + sensorScore + serviceScore;

  var activityRatio = totalScore === 0
    ? 0
    : Math.min(1.0, Math.max(MIN_EMISSION_RATIO, totalScore / FULL_ACTIVITY_SCORE));

  var scheduledReward = blockReward;
  blockReward = roundAmount(scheduledReward * activityRatio);

  // ── 1. Testnet carveout (0.1%) ─────────────────────────────────────────
  var testnetPool   = roundAmount(blockReward * TESTNET_FRACTION);
  var distributable = roundAmount(blockReward - testnetPool);

  if (activeTestnetNodes.length > 0) {
    var tnShare = roundAmount(testnetPool / activeTestnetNodes.length);
    for (var tn of activeTestnetNodes) {
      rewards.push({ to: tn, amount: tnShare, type: "testnet" });
    }
  } else {
    rewards.push({ to: "hone_recycle", amount: testnetPool, type: "recycle" });
  }

  // ── 2. Distribute the activity-scaled reward ──────────────────────────
  if (totalScore === 0) {
    rewards.push({ to: "hone_recycle", amount: distributable, type: "recycle" });
  } else {
    // ── 3. Pure proportional split ────────────────────────────────────
    var minerPool    = roundAmount(distributable * (minerScore    / totalScore));
    var verifierPool = roundAmount(distributable * (verifierScore / totalScore));
    var clockPool    = roundAmount(distributable * (clockScore    / totalScore));
    var storagePool  = roundAmount(distributable * (storageScore  / totalScore));
    var iotPool      = roundAmount(distributable * (sensorScore   / totalScore));
    var servicePool  = roundAmount(distributable * (serviceScore  / totalScore));

    // Miners — proportional to stake-weighted work
    if (minerScore > 0) {
      for (var miner2 of miners) {
        if (weightedWork[miner2] > 0) {
          var mShare = roundAmount(minerPool * (weightedWork[miner2] / totalWeightedWork));
          rewards.push({ to: miner2, amount: mShare, type: "mining" });
        }
      }
    }

    // Helper: split a pool among participants proportional to work map, or equal.
    function _splitPool(participants, workMap, pool, type) {
      if (!participants || participants.length === 0) return;
      var totalWork = 0;
      if (workMap) {
        for (var _pp of participants) totalWork += (workMap[_pp] || 0);
      }
      for (var _rp of participants) {
        var amt;
        if (totalWork > 0) {
          var _w = workMap[_rp] || 0;
          if (_w === 0) continue;
          amt = roundAmount(pool * (_w / totalWork));
        } else {
          amt = roundAmount(pool / participants.length);
        }
        var _ex = rewards.find(function (r) { return r.to === _rp; });
        if (_ex) _ex.amount = roundAmount(_ex.amount + amt);
        else rewards.push({ to: _rp, amount: amt, type: type });
      }
    }

    // Verifiers — proportional to verifications performed (equal fallback)
    if (verifierScore > 0) {
      _splitPool(activeVerifiers, verifierWork, verifierPool, "verifier");
    }

    // Clocks — equal split (uptime = work)
    if (clockScore > 0) {
      _splitPool(activeClocks, null, clockPool, "clock");
    }

    // Storage — proportional to bytes delivered (equal fallback)
    if (storageScore > 0) {
      _splitPool(activeStorageHosts, storageWork, storagePool, "storage");
    }

    // IoT — sub-split by sensor vs gateway score, then proportional within each
    if (sensorScore > 0) {
      var sensorSubPool  = roundAmount(iotPool * (sensorRawScore  / sensorScore));
      var gatewaySubPool = roundAmount(iotPool * (gatewayRawScore / sensorScore));

      if (sensorRawScore > 0 && activeSensors.length > 0) {
        // Build combined score map for within-pool split (base + revenue × multiplier)
        var _snCombined = {};
        for (var _sni2 of activeSensors) {
          if (_sensorHasWork && !(sensorWork[_sni2] > 0)) continue;
          _snCombined[_sni2] = 100 + ((_sensorHasRevenue ? (sensorRevenue[_sni2] || 0) : 0) * 5000);
        }
        var _activeSn = Object.keys(_snCombined).length > 0 ? Object.keys(_snCombined) : activeSensors;
        var _snMap    = Object.keys(_snCombined).length > 0 ? _snCombined : sensorWork;
        _splitPool(_activeSn, _snMap, sensorSubPool, "iot_sensor");
      }
      if (gatewayRawScore > 0 && activeGateways.length > 0) {
        _splitPool(activeGateways, gatewayWork, gatewaySubPool, "iot_gateway");
      }
    }

    // Services — proportional to requests served (equal fallback)
    if (serviceScore > 0) {
      _splitPool(activeServiceHosts, serviceWork, servicePool, "service");
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
    scheduled_reward: scheduledReward,
    activity_ratio: activityRatio,
    block_reward: blockReward,
    rewards: rewards,
    miners_active: miners.length,
    verifiers_active: activeVerifiers.length,
    clocks_active: activeClocks.length,
    storage_active: activeStorageHosts.length,
    sensors_active: activeSensors.length,
    gateways_active: activeGateways.length,
    services_active: activeServiceHosts.length,
    total_work: totalWorkValue,
    total_score: totalScore,
    pool_pcts: totalScore > 0 ? {
      mining:   minerScore    / totalScore,
      verifier: verifierScore / totalScore,
      clock:    clockScore    / totalScore,
      storage:  storageScore  / totalScore,
      iot:      sensorScore   / totalScore,
      service:  serviceScore  / totalScore,
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
};
