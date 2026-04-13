"use strict";

const User = require('../models/User');
const { getBlockReward } = require('../services/emissionSchedule');
const { finalizeEpoch, EPOCH_DURATION_MS } = require('../services/epochManager');
const { generateWork, getEpochMetadata } = require('./workGenerator');
const { computeStateHash } = require('./stateHash');
const { createGenesisBlock, GENESIS_MINER } = require('./genesisBlock');
const MINER_ACCOUNT = process.env.BTCPC_MINER || GENESIS_MINER;
const { filterInscription } = require('../services/contentFilter');
const { generateAllClaimProofs } = require('../claims/claimProofGenerator');
const axios = require('axios');
const p2p = require('../p2p/network');
const { createBlockMessage, createMessage } = require('../p2p/protocol');
const { loadFromDatabase: loadChainFromDB, cacheBlock } = require('../p2p/chainSync');
const silicon = require('../silicon');
const { startInferenceHandler } = require('../inference/handler');
const { startAutoUpdater } = require('../services/autoUpdater');
const { verifyAllModels, verifyModel } = require('../services/modelVerifier');
const { startModelManager } = require('../services/modelManager');
const ledger = require('../services/ledger');
const Block = require('../chain/block');
const blockStore = require('../chain/blockStore');
const blockchain = require('../chain/blockchain');
const stateManager = require('../chain/stateManager');
const stateStore = require('../chain/stateStore');
const nodeRegistry = require('../chain/nodeRegistry');
const mempool = require('../p2p/mempool');

const FINALITY_INTERVAL = parseInt(process.env.BTCPC_FINALITY_INTERVAL) || 100;
const WORK_ITEMS_PER_EPOCH = parseInt(process.env.BTCPC_WORK_PER_EPOCH) || 3;
const resourceManager = require('../services/resourceManager');
const { notifyUpdate, notifyMining } = require('../services/systemNotify');
// BTCPC_MODEL may be unset — resolveWorkingModel will auto-pick the best local model
const MODEL = process.env.BTCPC_MODEL || null;
const http = require('http');
const { resolveWorkingModel } = require('./modelHealer');

// ── Alertbot heartbeat ──
function sendAlert(severity, message, details) {
  const url = process.env.ALERTBOT_URL;
  const key = process.env.ALERTBOT_API_KEY;
  if (!url || !key) return;
  const body = JSON.stringify({ project: 'btcpc', severity, message, service: 'miner', details });
  const parsed = new URL('/alert', url);
  const req = http.request({
    hostname: parsed.hostname, port: parsed.port, path: '/alert',
    method: 'POST', timeout: 3000,
    headers: { 'Content-Type': 'application/json', 'X-API-Key': key, 'Content-Length': Buffer.byteLength(body) },
  });
  req.on('error', () => {});
  req.write(body);
  req.end();
}

let running = false;
let miningInterval = null;

// ── Finalization scheduling ──
// Finalization waits until ALL active miners have submitted proofs for the epoch.
// The LAST miner to submit triggers finalization (they see all proofs).
// Max wait prevents hanging if a miner goes offline.
const MAX_FINALIZATION_WAIT_MS = parseInt(process.env.BTCPC_MAX_FINALIZATION_WAIT_MS) || 180000; // 3 min max
const PROOF_POLL_INTERVAL_MS = 10000; // check every 10s
const pendingFinalizations = new Set();

async function scheduleFinalization(epochNumber) {
  // Phase D: epoch state lives in stateStore — no Mongo fallback.
  let epoch = stateStore.getEpoch(epochNumber);
  if (!epoch) return null;
  if (epoch.status === 'finalized') return epoch;

  if (pendingFinalizations.has(epochNumber)) {
    return waitForFinalization(epochNumber);
  }

  pendingFinalizations.add(epochNumber);

  // Count active miners on the network (from chain state, not Mongo)
  const { getIdleMiners } = require('../p2p/protocol');
  const activeMiners = nodeRegistry.getRegisteredNodes()
    .filter(n => n.type === 'miner').length;
  console.log(`[BTCPC] Epoch ${epochNumber}: waiting for proofs from ${activeMiners} active miner(s)...`);

  // Poll until all active miners have submitted proofs OR announced idle
  const startTime = Date.now();
  while (Date.now() - startTime < MAX_FINALIZATION_WAIT_MS) {
    const proofCount = stateStore.getMiningProofs(epochNumber).length;
    const idleCount = getIdleMiners(epochNumber).size;
    const accountedFor = proofCount + idleCount;

    if (accountedFor >= activeMiners) {
      console.log(`[BTCPC] Epoch ${epochNumber}: all miners accounted for (${proofCount} proofs, ${idleCount} idle). Finalizing.`);
      break;
    }

    // Check if epoch was finalized by another node (applied to stateStore
    // via EPOCH_FINALIZED → applyRemoteEntries → block replay).
    const check = stateStore.getEpoch(epochNumber);
    if (check && check.status === 'finalized') {
      pendingFinalizations.delete(epochNumber);
      return check;
    }

    const elapsed = Math.round((Date.now() - startTime) / 1000);
    if (elapsed % 30 === 0 && elapsed > 0) {
      console.log(`[BTCPC] Epoch ${epochNumber}: ${proofCount} proofs + ${idleCount} idle = ${accountedFor}/${activeMiners} after ${elapsed}s...`);
    }

    await new Promise(r => setTimeout(r, PROOF_POLL_INTERVAL_MS));
  }

  // Finalize with whatever proofs we have
  try {
    const result = await finalizeAndSplitRewards(epochNumber);
    return result;
  } catch (err) {
    console.error(`[BTCPC] Finalization error for epoch ${epochNumber}:`, err.message);
    return null;
  } finally {
    pendingFinalizations.delete(epochNumber);
  }
}

async function waitForFinalization(epochNumber) {
  const start = Date.now();
  while (Date.now() - start < MAX_FINALIZATION_WAIT_MS + 30000) {
    const epoch = stateStore.getEpoch(epochNumber);
    if (epoch && epoch.status === 'finalized') return epoch;
    await new Promise(r => setTimeout(r, 5000));
  }
  return null;
}

/**
 * Finalize epoch — distribute rewards based on jobs that SETTLED in this epoch.
 *
 * A job settles when all required verifications are in.
 * Miners get paid in the epoch the job settles, not the epoch they did the work.
 * Reward per miner = their total work_value from settled jobs / total work_value.
 *
 * Also handles synthetic mining proofs (epochs with no inference jobs).
 */
/**
 * computeFinalization — compute the reward split WITHOUT writing to DB.
 * Returns a proposal object that can be broadcast for consensus.
 * Every miner computes this independently from their local proofs.
 */
async function computeFinalization(epochNumber) {
  const { getRegistryModelHash } = require('../services/modelVerifier');
  const finConsensus = require('../chain/finalizationConsensus');
  const nodeRegistry = require('../chain/nodeRegistry');

  // Phase D: count finalized+settled epochs from stateStore
  let rewardedEpochs = 0;
  for (let e = 0; e <= epochNumber; e++) {
    const meta = stateStore.getEpoch(e);
    if (meta && meta.status === 'finalized' && (meta.settled_jobs || 0) > 0) rewardedEpochs++;
  }
  const rewardNumber = rewardedEpochs;
  const blockReward = getBlockReward(rewardNumber);
  const epochsDeferred = epochNumber - rewardNumber;

  // Phase E: InferenceJob model deleted. Settlement is based entirely on
  // compute proofs stored in stateStore. Escrow sweep still runs.
  const escrow = require('../services/escrow');
  await escrow.sweepEscrows(600000).catch(() => {});

  // Phase E: settled jobs count comes from compute proofs (each proof = 1 settled job)
  const settledJobsCount = stateStore.getComputeProofs(epochNumber).length;
  if (settledJobsCount > 0) {
    console.log(`[BTCPC] Epoch ${epochNumber}: ${settledJobsCount} compute proof(s) will be rewarded`);
  }

  // Phase D: synthetic mining proofs live exclusively in stateStore
  // (addMiningProof during mining, setMiningProofs on replay).
  const syntheticProofs = stateStore.getMiningProofs(epochNumber).slice();

  // Build per-miner work values from stateStore compute proofs (Phase E)
  const minerWork = {};
  const { verifyModelParams } = require('./workGenerator');
  const computeProofs = stateStore.getComputeProofs(epochNumber);

  for (const proof of computeProofs) {
    const miner = proof.node_id;
    if (!miner) continue;
    if (proof.model_hash) {
      const registryHash = await getRegistryModelHash(proof.model);
      if (registryHash && proof.model_hash !== registryHash) continue;
    }
    if (!minerWork[miner]) minerWork[miner] = { work_value: 0, models: new Set() };
    minerWork[miner].work_value += (proof.work_value || 0);
    if (proof.model) minerWork[miner].models.add(proof.model);
  }

  // Placeholder for settledJobs (used below for logging/consensus hash)
  const settledJobs = computeProofs.map(p => ({ job_id: p.prompt_hash, model: p.model, node_name: p.node_id }));

  for (const _job of settledJobs) {
    // work already counted above — this loop kept for structure compatibility
  }

  for (const proof of syntheticProofs) {
    if (proof.model_hash) {
      const registryHash = await getRegistryModelHash(proof.model);
      if (registryHash && proof.model_hash !== registryHash) continue;
    }
    if (!minerWork[proof.miner]) minerWork[proof.miner] = { work_value: 0, models: new Set() };
    minerWork[proof.miner].work_value += (proof.work_value || 0);
    minerWork[proof.miner].models.add(proof.model);
  }

  // ── Gossiped attestations from P2P (cross-machine consensus source) ──
  // Every node receives INFERENCE_REVEAL/RESULT messages via the relay and
  // records work_value in protocol.minerWorkByEpoch. This makes consensus
  // deterministic across miners with separate MongoDB instances — they all
  // see the same attestations and compute the same rewards.
  try {
    const { getMinerWorkForEpoch } = require('../p2p/protocol');
    const gossipedWork = getMinerWorkForEpoch(epochNumber);
    for (const minerName of Object.keys(gossipedWork)) {
      if (!minerWork[minerName]) {
        minerWork[minerName] = { work_value: 0, models: new Set() };
      }
      // Take the MAX of local and gossiped to avoid double-counting jobs
      // that appear in both (own jobs that we processed AND broadcast)
      const gossipedValue = gossipedWork[minerName].work_value || 0;
      if (gossipedValue > minerWork[minerName].work_value) {
        minerWork[minerName].work_value = gossipedValue;
      }
    }
  } catch (e) {
    console.error('[BTCPC] Could not read gossiped work:', e.message);
  }

  const miners = Object.keys(minerWork);
  const totalWorkValue = miners.reduce((sum, m) => sum + minerWork[m].work_value, 0);

  // ── Reward split (v3.0-pre) ──
  // Six-pool model. Each pool has eligible recipients; empty pools flow to
  // btcpc_recycle (never burnt — see feedback_no_burn_all_recycle.md).
  //
  //   Miner pool    55%  — proportional to work_value; only real inference jobs
  //   Verifier pool 10%  — equal split among active verifiers this epoch
  //   Clock pool     5%  — equal split among active clocks this epoch (ALWAYS paid)
  //   Storage pool  12%  — equal split among hosts with STORAGE_HEARTBEAT this epoch
  //   Service pool   8%  — equal split among service hosts with SERVICE_HEARTBEAT
  //   IoT pool      10%  — 60/40 split between sensors and gateways via nanoRewards
  //                ----
  //                100%  — leftover per-pool goes to btcpc_recycle
  //
  // Worst case (only clocks active): 5% to clocks, 95% to recycle.
  // Genesis (miners + clocks + IoT): 70% claimed, 30% to recycle.
  // All active: 100% claimed, 0% recycled.
  const MINER_POOL_PCT    = 0.55;
  const VERIFIER_POOL_PCT = 0.10;
  const CLOCK_POOL_PCT    = 0.05;
  const STORAGE_POOL_PCT  = 0.12;
  const SERVICE_POOL_PCT  = 0.08;
  const IOT_POOL_PCT      = 0.10;

  const rewards = [];
  let recycledAmount = 0;

  const { getActiveClockNodes, getActiveVerifiers } = require('../p2p/protocol');
  // Clock nodes: any account that heartbeated this epoch.
  // Filter out raw nodeIds (hex strings) and anonymous clock- prefixed nodes.
  // Open participation — no stake required.
  const activeClocks = getActiveClockNodes(epochNumber).filter(a =>
    a && !a.startsWith('clock-') && !/^[a-f0-9]{32,}$/i.test(a) && /^[a-z0-9][a-z0-9-]{2,19}$/.test(a)
  );

  // Verifiers: any account that submitted a VERIFY_RESPONSE this epoch.
  const activeVerifiers = getActiveVerifiers(epochNumber).filter(a =>
    a && /^[a-z0-9][a-z0-9-]{2,19}$/.test(a)
  );

  // Storage hosts: any host that sent STORAGE_HEARTBEAT for exactly this epoch.
  const storageHostsThisEpoch = stateStore.getStorageHostsForEpoch(epochNumber);

  // Service hosts: any host that sent SERVICE_HEARTBEAT for exactly this epoch.
  // Uses serviceRegistry in-memory heartbeats (v2.13-alpha).
  let serviceHostsThisEpoch = [];
  try {
    const serviceRegistry = require('../services/serviceRegistry');
    const allHeartbeats = serviceRegistry._getHeartbeatsForEpoch
      ? serviceRegistry._getHeartbeatsForEpoch(epochNumber)
      : [];
    const serviceHostSet = new Set(allHeartbeats.map(h => h.host).filter(Boolean));
    serviceHostsThisEpoch = Array.from(serviceHostSet).filter(
      h => /^[a-z0-9][a-z0-9-]{2,19}$/.test(h)
    );
  } catch (_) {
    // serviceRegistry not available — service pool goes to recycle
  }

  // ── Miner pool: 60%, proportional to work_value ──
  const minerPool = blockReward * MINER_POOL_PCT;
  if (miners.length === 0 || totalWorkValue === 0) {
    recycledAmount += minerPool;
  } else {
    for (const miner of miners) {
      const share = parseFloat((minerPool * (minerWork[miner].work_value / totalWorkValue)).toFixed(10));
      rewards.push({ miner, amount: share, type: 'mining' });
    }
  }

  // ── Verifier pool: 10%, equal split ──
  const verifierRewardPool = blockReward * VERIFIER_POOL_PCT;
  if (activeVerifiers.length === 0) {
    recycledAmount += verifierRewardPool;
  } else {
    const vShare = parseFloat((verifierRewardPool / activeVerifiers.length).toFixed(10));
    for (const v of activeVerifiers) {
      rewards.push({ miner: v, amount: vShare, type: 'verifier' });
    }
  }

  // ── Clock pool: 5%, equal split, ALWAYS paid if any clocks active ──
  const clockRewardPool = blockReward * CLOCK_POOL_PCT;
  if (activeClocks.length === 0) {
    recycledAmount += clockRewardPool;
  } else {
    const cShare = parseFloat((clockRewardPool / activeClocks.length).toFixed(10));
    for (const c of activeClocks) {
      rewards.push({ miner: c, amount: cShare, type: 'clock' });
    }
  }

  // ── Storage pool: 15%, equal split among hosts that heartbeated this epoch ──
  const storageRewardPool = blockReward * STORAGE_POOL_PCT;
  if (storageHostsThisEpoch.length === 0) {
    recycledAmount += storageRewardPool;
  } else {
    const sShare = parseFloat((storageRewardPool / storageHostsThisEpoch.length).toFixed(10));
    for (const h of storageHostsThisEpoch) {
      rewards.push({ miner: h, amount: sShare, type: 'storage' });
    }
  }

  // ── Service pool: 10%, equal split among service hosts that heartbeated ──
  const serviceRewardPool = blockReward * SERVICE_POOL_PCT;
  if (serviceHostsThisEpoch.length === 0) {
    recycledAmount += serviceRewardPool;
  } else {
    const svShare = parseFloat((serviceRewardPool / serviceHostsThisEpoch.length).toFixed(10));
    for (const h of serviceHostsThisEpoch) {
      rewards.push({ miner: h, amount: svShare, type: 'service' });
    }
  }

  // ── IoT pool: 10%, split 60/40 sensors/gateways via nanoRewards ──
  const iotRewardPool = blockReward * IOT_POOL_PCT;
  try {
    const nanoRewards = require('../services/nanoRewards');
    const activeGatewaysForEpoch = stateStore.getGatewaysForEpoch(epochNumber);
    const activeSensorsForEpoch = stateStore.getSensorsForEpoch(epochNumber);

    if (activeGatewaysForEpoch.length === 0 && activeSensorsForEpoch.length === 0) {
      recycledAmount += iotRewardPool;
    } else {
      const iotRewards = nanoRewards.computeIoTRewards(
        epochNumber,
        iotRewardPool,
        activeGatewaysForEpoch,
        activeSensorsForEpoch
      );
      for (const r of iotRewards) {
        rewards.push({ miner: r.account, amount: parseFloat(r.amount.toFixed(10)), type: r.type });
      }
      // Any leftover (rounding) goes to recycle
      const totalIoTPaid = iotRewards.reduce((sum, r) => sum + r.amount, 0);
      const iotRemainder = iotRewardPool - totalIoTPaid;
      if (iotRemainder > 0.000000001) {
        recycledAmount += iotRemainder;
      }
    }
  } catch (iotErr) {
    // nanoRewards not available — IoT pool to recycle
    recycledAmount += iotRewardPool;
    console.error('[BTCPC] IoT reward computation failed:', iotErr.message);
  }

  // ── Recycle unclaimed pools — never burnt ──
  if (recycledAmount > 0) {
    rewards.push({
      miner: 'btcpc_recycle',
      amount: parseFloat(recycledAmount.toFixed(10)),
      type: 'recycle',
    });
  }

  // ── Clock slashing: detect drift and offline clocks ──
  try {
    const slashing = require('../services/slashing');
    const registeredNodes = nodeRegistry.getRegisteredNodes();
    const registeredClocks = registeredNodes.filter(n => n.type === 'clock');

    for (const clock of registeredClocks) {
      // Check offline: clock registered but not active for > 10 consecutive epochs
      let offlineCount = 0;
      for (let e = epochNumber; e > Math.max(0, epochNumber - 10); e--) {
        const active = getActiveClockNodes(e);
        if (active.indexOf(clock.username) === -1) {
          offlineCount++;
        } else {
          break; // consecutive streak broken
        }
      }
      if (offlineCount >= 10) {
        await slashing.recordOffense(clock.username, 'CLOCK_OFFLINE', {
          epoch: epochNumber,
          consecutive_offline_epochs: offlineCount
        });
      }
    }
  } catch (slashErr) {
    console.error('[BTCPC] Clock slashing check failed:', slashErr.message);
  }

  const consensusHash = finConsensus.hashRewards(rewards, totalWorkValue, settledJobs.length);

  return {
    epoch_number: epochNumber,
    proposer: MINER_ACCOUNT,
    rewards,
    total_work: totalWorkValue,
    settled_jobs: settledJobs.length,
    block_reward: blockReward,
    reward_number: rewardNumber,
    epochs_deferred: epochsDeferred,
    consensus_hash: consensusHash,
    timestamp: Date.now()
  };
}

/**
 * applyFinalization — write the winning proposal to DB, ledger, and disk.
 * Called by the consensus winner OR when receiving EPOCH_FINALIZED.
 */
async function applyFinalization(epochNumber, proposal) {
  // Phase D: epoch metadata lives in stateStore + block payload, not Mongo.
  let epoch = stateStore.getEpoch(epochNumber);
  if (!epoch) {
    epoch = {
      epoch_number: epochNumber,
      status: 'active',
      started_at: new Date(),
      block_reward: proposal.block_reward || 0,
      commitments: [],
    };
  }
  if (epoch.status === 'finalized') return epoch; // already done

  const rewards = proposal.rewards || [];

  // Write rewards to permanent ledger — stateStore balances update via
  // the MINING_REWARD entry dispatcher, and the mining proof reward_earned
  // is updated in-place on the in-memory proof entry.
  const epochProofs = stateStore.getMiningProofs(epochNumber).slice();
  for (const r of rewards) {
    await ledger.recordMiningReward(r.miner, r.amount, epochNumber);

    // Update reward_earned on the matching proof
    for (const proof of epochProofs) {
      if (proof.miner === r.miner) {
        proof.reward_earned = r.amount;
        break;
      }
    }

    console.log(`[BTCPC]   ${r.miner}: ${r.amount.toFixed(4)} BTCPC (${r.type || 'mining'})`);
  }
  stateStore.setMiningProofs(epochNumber, epochProofs);

  // Finalize epoch record in stateStore
  epoch.consensus_hash = proposal.consensus_hash || '0'.repeat(64);
  epoch.total_work = proposal.total_work || 0;
  epoch.rewards_distributed = rewards.map(r => ({ node_id: r.miner, amount: r.amount }));
  epoch.block_reward = proposal.block_reward || 0;
  epoch.reward_number = proposal.reward_number;
  epoch.epochs_deferred = proposal.epochs_deferred || 0;
  epoch.ended_at = new Date();
  epoch.status = 'finalized';
  epoch.settled_jobs = proposal.settled_jobs || 0;
  stateStore.setEpoch(epochNumber, epoch);

  console.log(`[BTCPC] Epoch ${epochNumber} finalized | reward #${proposal.reward_number} | ${rewards.length} reward(s) | ${(proposal.block_reward || 0).toFixed(4)} BTCPC`);

  // ── Write block to disk — source of truth ──
  try {
    // Get ledger entries for this epoch (already flushed to pending)
    const epochLedgerEntries = ledger.flushPendingEntries();

    // Apply entries to SMT for state root
    stateManager.applyLedgerEntries(epochLedgerEntries);
    // Phase B: also apply to stateStore so the in-memory cache tracks live writes
    try {
      const stateStore = require('../chain/stateStore');
      stateStore.applyEntries(epochLedgerEntries);
    } catch (_) {}
    const stateRoot = stateManager.getStateRoot();

    // Compute Merkle roots
    const txHashes = epochLedgerEntries.map(e => blockStore.hashLedgerEntry(e));
    const txMerkleRoot = Block.computeMerkleRoot(txHashes);

    // Phase D: compute proofs live exclusively in stateStore (added live
    // via addComputeProof in the mining loop, hydrated on replay).
    const epochProofs = stateStore.getComputeProofs(epochNumber);
    const proofHashes = epochProofs.map(p => blockStore.hashComputeProof(p));
    const cpMerkleRoot = Block.computeMerkleRoot(proofHashes);

    // Get previous block hash
    let prevHash = '0'.repeat(64);
    if (epochNumber > 0) {
      const prevHeader = blockStore.readBlockHeader(epochNumber - 1);
      if (prevHeader) {
        prevHash = prevHeader.computeHash();
      }
    }

    const block = new Block({
      version: 1,
      epoch_number: epochNumber,
      previous_block_hash: prevHash,
      merkle_root_transactions: txMerkleRoot,
      merkle_root_compute_proofs: cpMerkleRoot,
      state_root: stateRoot,
      timestamp: epoch.ended_at.getTime(),
      difficulty: epoch.difficulty || 1,
      miner_id: MINER_ACCOUNT
    });

    // Phase D: mining proofs come exclusively from stateStore
    const miningProofs = stateStore.getMiningProofs(epochNumber);

    const payload = {
      ledger_entries: epochLedgerEntries,
      rewards: rewards.map(r => ({ miner: r.node_id, amount: r.amount })),
      compute_proofs: epochProofs.map(p => ({
        node_id: p.node_id, prompt_hash: p.prompt_hash,
        result_hash: p.result_hash, model: p.model,
        tokens_generated: p.tokens_generated, work_value: p.work_value
      })),
      mining_proofs: miningProofs.map(p => ({
        miner: p.miner, reward_earned: p.reward_earned,
        model: p.model, tokens_computed: p.tokens_computed,
        work_value: p.work_value, state_hash: p.state_hash
      }))
    };

    blockStore.writeBlock(block, payload);
    blockchain.addBlock(block);

    const blockHash = block.computeHash();
    console.log(`[BTCPC] Block ${epochNumber} written to disk: ${blockHash.slice(0, 16)}... | state: ${stateRoot.slice(0, 16)}...`);

    // ── Finality block every N epochs ──
    if (epochNumber > 0 && epochNumber % FINALITY_INTERVAL === 0) {
      const snapshot = stateManager.generateFinalitySnapshot();
      // Rolling commitment: SHA256(prev_finality_hash + current_state_root)
      const prevFinalityEpoch = epochNumber - FINALITY_INTERVAL;
      let prevFinalityHash = '0'.repeat(64);
      if (prevFinalityEpoch >= 0 && blockStore.hasFinality(prevFinalityEpoch)) {
        const prevFin = blockStore.readFinality(prevFinalityEpoch);
        if (prevFin && prevFin.snapshot.rolling_commitment) {
          prevFinalityHash = prevFin.snapshot.rolling_commitment;
        }
      }
      const crypto = require('crypto');
      snapshot.rolling_commitment = crypto.createHash('sha256')
        .update(prevFinalityHash + stateRoot)
        .digest('hex');
      snapshot.finality_epoch = epochNumber;
      snapshot.block_hash = blockHash;

      blockStore.writeFinality(block, snapshot);
      console.log(`[BTCPC] Finality block ${epochNumber} written | ${snapshot.account_count} accounts | commitment: ${snapshot.rolling_commitment.slice(0, 16)}...`);

      // Lucid Pruning — remove block files before this finality block
      const pruned = blockStore.pruneBeforeFinality(epochNumber);
      if (pruned > 0) {
        console.log(`[BTCPC] Lucid Pruning: ${pruned} block files pruned (before epoch ${epochNumber})`);
      }
    }

    // Clear mempool — transactions are now in the block
    const mempoolTxs = mempool.getTransactions();
    const clearedHashes = mempoolTxs.map(t => t.txHash).filter(Boolean);
    if (clearedHashes.length > 0) {
      mempool.removeTransactions(clearedHashes);
      console.log(`[BTCPC] Mempool: ${clearedHashes.length} transactions included in block ${epochNumber}`);
    }

    // Attach block data to epoch for broadcast
    epoch._blockData = {
      header_hex: block.serialize().toString('hex'),
      block_hash: blockHash,
      state_root: stateRoot,
      ledger: epochLedgerEntries,
      is_finality: epochNumber > 0 && epochNumber % FINALITY_INTERVAL === 0
    };
  } catch (err) {
    console.error(`[BTCPC] Failed to write block to disk: ${err.message}`);
    // Non-fatal: chain continues, block can be reconstructed later
    // Still flush ledger entries so they make it to P2P broadcast
    epoch._blockData = { ledger: ledger.flushPendingEntries() };
  }

  return epoch;
}

/**
 * Run one full mining cycle for the given epoch.
 *
 * 1. Generate synthetic inference work via Ollama
 * 2. Store work proofs
 * 3. Compute state hash
 * 4. Submit epoch commitment
 * 5. Finalize epoch and distribute rewards
 * 6. Log results
 */
async function mineEpoch(epochNumber) {
  const startTime = Date.now();
  const ts = new Date().toISOString();

  console.log(`\n[BTCPC] ${ts} -- Epoch ${epochNumber} mining started`);

  // Get miner references — Phase F: stateStore first, Mongo fallback.
  // On a Mongo-disabled node the User model returns null; that's fine
  // because the miner account is tracked via stateStore + nodeRegistry.
  let user = null;
  try {
    user = await User.findOne({ username: MINER_ACCOUNT });
  } catch (_) {
    // Mongo disabled — that's ok
  }
  // If neither Mongo nor stateStore knows the account, the miner was
  // just registered in startMiner() via ledger.recordAccountCreate.
  // The account exists in stateStore even if User.findOne returns null.

  // Phase E: Node/Wallet models deleted — use nodeRegistry and stateStore
  const nodeEntry = nodeRegistry.getNode(MINER_ACCOUNT);
  const node = {
    _id: MINER_ACCOUNT,
    account: user ? user._id : MINER_ACCOUNT,
    hive_account: process.env.BTCPC_HIVE_ACCOUNT,
    base_wallet: process.env.BTCPC_BASE_WALLET,
    arbitrum_wallet: process.env.BTCPC_ARBITRUM_WALLET,
    optimism_wallet: process.env.BTCPC_OPTIMISM_WALLET,
    solana_wallet: process.env.BTCPC_SOLANA_WALLET,
    ton_wallet: process.env.BTCPC_TON_WALLET,
    bitcoin_wallet: process.env.BTCPC_BITCOIN_WALLET,
    last_epoch_commitment: nodeEntry ? nodeEntry.registeredEpoch : 0,
  };

  // Account existence check via stateStore.
  if (!stateStore.hasAccount(MINER_ACCOUNT)) {
    console.warn('[BTCPC] Miner account not yet in chain state (pre-genesis replay is normal)');
  }

  // Phase D: epoch metadata lives in stateStore + block payload, not Mongo.
  let epoch = stateStore.getEpoch(epochNumber);
  if (!epoch) {
    const reward = getBlockReward(epochNumber);
    epoch = {
      epoch_number: epochNumber,
      started_at: new Date(),
      block_reward: reward,
      status: 'active',
      commitments: [],
      consensus_hash: null,
    };
    stateStore.setEpoch(epochNumber, epoch);
  }
  if (!epoch.commitments) epoch.commitments = [];

  if (epoch.status === 'finalized') {
    console.log(`[BTCPC] Epoch ${epochNumber} already finalized, skipping`);
    return;
  }

  // Step 1: Generate inference work
  const workProofs = [];
  let totalTokens = 0;
  let totalWorkValue = 0;

  // Genesis epoch gets a special first prompt
  const GENESIS_PROMPT = "What is the meaning of computation? If a machine dreams an answer into existence through pure mathematical reasoning, is that dream less real than a human thought? Describe a future where every unit of energy spent computing produces something useful — where proof of work means proof of value created, not value destroyed. The answer, as always, is 42.";

  // Phase E: InferenceJob model deleted. Stale claim expiry handled by handler.js
  // which tracks claimedJobs in memory. No Mongo sweep needed here.
  // No synthetic work — miners only earn from real inference jobs
  const syntheticCount = 0;

  // Resolve working model — auto-heals via pull + fallback chain; never crashes
  const workingModel = await resolveWorkingModel(MODEL);
  if (!workingModel) {
    console.error(`[BTCPC] No verifiable model available — broadcasting MINER_IDLE, staying alive`);
    const idleMsg = createMessage('MINER_IDLE', {
      block_number: epochNumber,
      miner: MINER_ACCOUNT,
      reason: 'no_verifiable_model'
    }, p2p.NODE_ID);
    p2p.broadcast(idleMsg);
    // Do NOT return early in a way that kills the loop — fall through to no-work path
    return;
  }
  // Re-verify to get the local hash for proofs (cache hit — no extra HTTP call)
  const modelCheck = await verifyModel(workingModel);
  const modelHash = modelCheck.localHash; // store on proofs for verification

  for (let i = 0; i < syntheticCount; i++) {
    try {
      const isGenesisFirstWork = (epochNumber === 0 && i === 0);
      if (isGenesisFirstWork) {
        console.log(`[BTCPC]   GENESIS INFERENCE -- the first dream computed into reality`);
      } else {
        console.log(`[BTCPC]   Work item ${i + 1}/${syntheticCount} -- sending to Ollama (${workingModel})...`);
      }
      const work = await generateWork(workingModel, isGenesisFirstWork ? GENESIS_PROMPT : undefined);

      // Phase D: WorkProofs live in stateStore + next block payload, not Mongo.
      const proof = {
        epoch_number: epochNumber,
        node_id: MINER_ACCOUNT,
        prompt_hash: work.prompt_hash,
        result_hash: work.result_hash,
        model: work.model,
        tokens_generated: work.tokens_generated,
        model_weight_factor: work.model_weight_factor,
        work_value: work.work_value,
      };
      stateStore.addComputeProof(epochNumber, proof);

      workProofs.push(proof);
      totalTokens += work.tokens_generated;
      totalWorkValue += work.work_value;

      console.log(`[BTCPC]   Work item ${i + 1} complete: ${work.tokens_generated} tokens, value=${work.work_value.toFixed(1)}`);
    } catch (err) {
      console.error(`[BTCPC]   Work item ${i + 1} failed: ${err.message}`);
    }
  }

  // If no synthetic work, check if we completed any inference jobs this epoch
  if (workProofs.length === 0) {
    // Phase E: InferenceJob model deleted. Completed inference jobs are now
    // tracked as compute proofs in stateStore (added by handler.js via
    // stateStore.addComputeProof). Read them from stateStore.
    const recentProofs = stateStore.getComputeProofs(epochNumber);
    const myProofs = recentProofs.filter(p => p.node_id === MINER_ACCOUNT);

    if (myProofs.length > 0) {
      for (const proof of myProofs) {
        totalTokens += proof.tokens_generated || 0;
        totalWorkValue += proof.work_value || 0;
      }
      console.log(`[BTCPC]   ${myProofs.length} inference proof(s) this epoch: ${totalTokens} tokens, work_value=${totalWorkValue}`);
    }

    if (totalWorkValue === 0) {
      console.log(`[BTCPC] No work this epoch — announcing idle to network`);
      const idleMsg = createMessage('MINER_IDLE', {
        block_number: epochNumber,
        miner: MINER_ACCOUNT,
        reason: 'no_work_completed'
      }, p2p.NODE_ID);
      p2p.broadcast(idleMsg);
      return;
    }
  }

  // Step 2: Compute state hash
  const previousEpoch = stateStore.getEpoch(epochNumber - 1);
  const previousHash = previousEpoch ? previousEpoch.consensus_hash : '0'.repeat(64);
  const stateHash = await computeStateHash(epochNumber, previousHash);

  // Step 3: Submit epoch commitment to stateStore (no Mongo write)
  epoch.commitments.push({
    node_id: node._id,
    state_hash: stateHash,
    tx_count: 0,
    inference_count: workProofs.length,
    submitted_at: new Date(),
  });
  epoch.consensus_hash = stateHash;
  stateStore.setEpoch(epochNumber, epoch);

  // Step 4: Create genesis dream for this block (mandatory)
  const metadata = getEpochMetadata(epochNumber);
  // Phase E: GenesisDream model deleted — dreams recorded as ledger entries
  // in the block payload. The dream metadata is inscribed in the EPOCH_FINALIZED
  // block and readable via block file replay.
  {
    const workHash = workProofs.length > 0 ? workProofs[0].result_hash : '0'.repeat(64);
    const tagResult = filterInscription(metadata.tag);
    const projectResult = filterInscription(metadata.project);
    const filteredTag = tagResult.filtered_text;
    const filteredProject = projectResult.filtered_text;
    if (tagResult.was_redacted || projectResult.was_redacted) {
      console.log(`[BTCPC]   Content filter: inscription text redacted`);
    }
    // Dream metadata is stored on the block payload (not in Mongo)
    console.log(`[BTCPC]   Dream #${epochNumber}: "${filteredTag}" [${filteredProject}] work_hash=${workHash.slice(0, 16)}...`);
  }

  // Step 4b: Record this miner's proof for the epoch.
  // Phase D: proofs live in stateStore and the next block payload, not Mongo.
  // Dedupe by miner — addMiningProof replaces any existing entry for the
  // same miner in the same epoch.
  stateStore.addMiningProof(epochNumber, {
    block_number: epochNumber,
    miner: MINER_ACCOUNT,
    reward_earned: 0,
    model: workingModel,
    model_hash: modelHash,
    tokens_computed: totalTokens,
    work_value: totalWorkValue,
    state_hash: stateHash,
  });
  console.log(`[BTCPC]   Mining Proof #${epochNumber}: submitted by ${MINER_ACCOUNT} (work_value: ${totalWorkValue})`);

  // ALWAYS broadcast proof via P2P — even if DB save failed or proof already existed
  // Other nodes need this to finalize the epoch
  {
    const proofMsg = createMessage('MINING_PROOF', {
      block_number: epochNumber,
      miner: MINER_ACCOUNT,
      model: workingModel,
      model_hash: modelHash,
      tokens_computed: totalTokens,
      work_value: totalWorkValue,
      state_hash: stateHash
    }, p2p.NODE_ID);
    p2p.broadcast(proofMsg);
    console.log(`[BTCPC]   Proof broadcast to P2P network`);
  }

  // Step 5: Finalization — ONLY the epoch authority finalizes
  // Followers wait for EPOCH_FINALIZED broadcast from authority
  const isAuthority = (MINER_ACCOUNT === GENESIS_MINER);
  let finalized = null;
  if (!isAuthority) {
    console.log(`[BTCPC] Proof submitted. Waiting for authority to finalize epoch ${epochNumber}.`);
    // Follower does not finalize — authority handles it at EPOCH_END
  }

  // Step 5b: Generate cross-chain claim proofs for linked wallets
  const linkedChains = {};
  if (node.hive_account) linkedChains.hive = node.hive_account;
  if (node.base_wallet) linkedChains.base = node.base_wallet;
  if (node.arbitrum_wallet) linkedChains.arbitrum = node.arbitrum_wallet;
  if (node.optimism_wallet) linkedChains.optimism = node.optimism_wallet;
  if (node.solana_wallet) linkedChains.solana = node.solana_wallet;
  if (node.ton_wallet) linkedChains.ton = node.ton_wallet;
  if (node.bitcoin_wallet) linkedChains.bitcoin = node.bitcoin_wallet;

  let claimProofs = [];
  if (Object.keys(linkedChains).length > 0 && finalized) {
    // Phase D: read the miner's reward share from stateStore
    const myProofs = stateStore.getMiningProofs(epochNumber);
    const myProof = myProofs.find(p => p.miner === MINER_ACCOUNT);
    const myReward = myProof ? (myProof.reward_earned || 0) : 0;

    const postingKey = process.env.BTCPC_POSTING_KEY;
    if (postingKey && myReward > 0) {
      try {
        claimProofs = generateAllClaimProofs(
          MINER_ACCOUNT,
          epochNumber,
          myReward,
          linkedChains,
          postingKey
        );
        // Phase D: cross-chain claim proofs are not persisted to Mongo.
        // They're signed and gossiped as needed; claim state lives on the
        // target chain, not on BTCPC.
        if (claimProofs.length > 0) {
          console.log('[BTCPC]   Cross-chain proofs: ' + claimProofs.map(function (p) { return p.chain; }).join(', '));
        }
      } catch (err) {
        console.error('[BTCPC]   Cross-chain proof generation error: ' + err.message);
      }
    }
  }

  // Step 5c: Update node tracking — in-memory only (nodeRegistry already
  // tracks last-seen from ledger entries). The legacy Node.save() was a
  // redundant cache.
  node.last_epoch_commitment = epochNumber;

  // Step 6: Log results — read updated balance from chain state (stateStore)
  const currentBalance = stateStore.getBalance(MINER_ACCOUNT, 'BTCPC');

  const elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
  const reward = finalized ? finalized.block_reward : 0;

  console.log('[BTCPC] ------------------------------------------------');
  console.log(`[BTCPC] Epoch ${epochNumber} complete`);
  console.log(`[BTCPC]   Reward:       +${reward} BTCPC`);
  console.log(`[BTCPC]   Balance:      ${currentBalance.toFixed(10)} BTCPC`);
  console.log(`[BTCPC]   Work items:   ${workProofs.length}`);
  console.log(`[BTCPC]   Tokens:       ${totalTokens}`);
  console.log(`[BTCPC]   Work value:   ${totalWorkValue.toFixed(1)}`);
  console.log(`[BTCPC]   State hash:   ${stateHash.slice(0, 16)}...`);
  console.log(`[BTCPC]   Claims:       ${claimProofs.length} chain(s)`);
  console.log(`[BTCPC]   Duration:     ${elapsed}s`);
  console.log('[BTCPC] ------------------------------------------------');

  sendAlert('ok', `Epoch ${epochNumber} mined: +${reward} BTCPC (${totalTokens} tokens, ${elapsed}s)`);

  // Broadcast finalized epoch to P2P network
  try {
    const blockData = {
      epoch_number: epochNumber,
      block_reward: reward,
      total_work: totalWorkValue,
      consensus_hash: stateHash,
      status: 'finalized',
      started_at: epoch.started_at,
      ended_at: new Date(),
    };
    cacheBlock(blockData);
    const blockMsg = createBlockMessage(blockData, p2p.NODE_ID);
    p2p.broadcast(blockMsg);
  } catch (err) {
    console.error('[BTCPC] P2P broadcast error:', err.message);
  }
}

/**
 * Start the mining daemon.
 * Creates genesis if needed, then enters the epoch mining loop.
 */
async function startMiner() {
  if (running) {
    console.log('[BTCPC] Miner already running');
    return;
  }

  console.log('[BTCPC] ================================================');
  console.log('[BTCPC]    BTCPC Mining Daemon Starting');
  console.log('[BTCPC] ================================================');
  console.log(`[BTCPC] Ollama:     ${process.env.OLLAMA_URL || 'http://100.122.145.60:11434'}`);
  console.log(`[BTCPC] Model:      ${MODEL || '(auto-select on first epoch)'}`);
  console.log(`[BTCPC] Work/epoch: ${WORK_ITEMS_PER_EPOCH}`);
  console.log(`[BTCPC] Epoch:      ${EPOCH_DURATION_MS / 1000}s`);
  console.log('[BTCPC] ================================================');

  // Start P2P network
  try {
    await loadChainFromDB();
    p2p.startServer();
    p2p.connectToSeeds();
    console.log(`[BTCPC] P2P network started on port ${process.env.P2P_PORT || 6942}`);
    console.log(`[BTCPC] Node ID: ${p2p.NODE_ID}`);

    // Connect to public relay (works through any NAT)
    const relayUrl = process.env.BTCPC_RELAY_URL || 'wss://btcpc-relay.shindevlin.workers.dev/ws';
    p2p.connectToPeer(relayUrl);
    console.log(`[BTCPC] Connecting to relay: ${relayUrl}`);

    // Start inference handler — listen for jobs on P2P
    startInferenceHandler();

    // Auto-discover peers from bot registry
    const peerRegistryUrl = process.env.PEER_REGISTRY_URL;
    if (peerRegistryUrl) {
      try {
        const { data } = await axios.get(`${peerRegistryUrl}/peers`, { timeout: 5000 });
        const discoveredPeers = (data.peers || []).map(p => p.address).filter(Boolean);
        for (const addr of discoveredPeers) {
          p2p.connectToPeer(addr);
        }
        if (discoveredPeers.length > 0) {
          console.log(`[BTCPC] Discovered ${discoveredPeers.length} peer(s) from registry`);
        }

        // Register ourselves
        const myPort = process.env.P2P_PORT || 6942;
        const myAddr = `ws://${process.env.P2P_ADVERTISE_IP || 'localhost'}:${myPort}`;
        await axios.post(`${peerRegistryUrl}/peers/register`, {
          address: myAddr,
          username: MINER_ACCOUNT,
          gpu: null,
        }, { timeout: 5000 }).catch(() => {});
      } catch (err) {
        console.warn('[BTCPC] Peer registry unreachable:', err.message);
      }
    }
  } catch (err) {
    console.error('[BTCPC] P2P startup error (mining continues):', err.message);
  }

  // Probe GPU silicon fingerprint
  try {
    const sik = await silicon.getFingerprint();
    console.log(`[BTCPC] Silicon ID: ${sik.sik_hash.slice(0, 16)}...`);
    console.log(`[BTCPC] GPU: ${sik.gpu} (${sik.vram_mb} MB)`);
    if (sik.software_only) {
      console.log('[BTCPC] WARNING: Software-only fingerprint. Compile CUDA probe for silicon-bound identity.');
    }
    // Phase E: Node model deleted — SIK hash stored in nodeRegistry in-memory
    const sikEntry = nodeRegistry.getNode(MINER_ACCOUNT);
    if (sikEntry && sikEntry.sik_hash !== sik.sik_hash) {
      sikEntry.sik_hash = sik.sik_hash;
      sikEntry.sik_type = sik.software_only ? 'software' : 'silicon';
      console.log(`[BTCPC] SIK registered: ${sik.sik_hash.slice(0, 16)}... (${sikEntry.sik_type})`);
    }
  } catch (err) {
    console.warn('[BTCPC] SIK probe skipped:', err.message);
  }

  // Sync local Ollama models (Phase E: Node model deleted, pass null nodeId)
  try {
    const { syncLocalModels } = require('../services/modelRegistry');
    const models = await syncLocalModels(null);
    console.log(`[BTCPC] Models synced: ${models.join(', ') || 'none'}`);

    // Verify all models against Ollama registry
    const verResults = await verifyAllModels();
    for (const v of verResults) {
      if (v.verified) {
        console.log(`[BTCPC] Model ${v.model}: VERIFIED`);
      } else {
        console.error(`[BTCPC] Model ${v.model}: REJECTED — ${v.reason}`);
      }
    }
  } catch (err) {
    console.warn('[BTCPC] Model sync skipped:', err.message);
  }

  // Create genesis block if needed
  const genesis = await createGenesisBlock();

  // Ensure this miner's account exists (auto-register on first run).
  // Phase F: Mongo is optional. Check stateStore first (the canonical
  // source of truth), then Mongo as fallback, then create from scratch
  // via ledger.recordAccountCreate (which goes through the cross-process
  // queue + P2P mempool gossip — no Mongo needed).
  let minerAccountExists = stateStore.hasAccount
    ? stateStore.hasAccount(MINER_ACCOUNT)
    : !!stateStore.getAccount(MINER_ACCOUNT);

  // Fallback to Mongo if stateStore doesn't know the account yet
  // (e.g., replay hasn't caught up). Wrapped in try/catch because
  // Mongo may be disabled (Phase F).
  let minerUser = null;
  if (!minerAccountExists) {
    try {
      minerUser = await User.findOne({ username: MINER_ACCOUNT });
      if (minerUser) minerAccountExists = true;
    } catch (_) {
      // Mongo disabled or unreachable — that's fine, stateStore is primary
    }
  }

  if (!minerAccountExists) {
    console.log(`[BTCPC] Account '${MINER_ACCOUNT}' not found — creating...`);
    try {
      // Try the full wallet creation path (generates mnemonic + keys).
      // This calls accountManager.createAccount which writes to Mongo IF
      // available, but we don't depend on it — the ledger entry below is
      // what actually puts the account on chain.
      const { createAccount } = require('../wallet/accountManager');
      const savedMnemonic = process.env.BTCPC_MNEMONIC || process.env[`BTCPC_MNEMONIC_${MINER_ACCOUNT.toUpperCase().replace(/-/g, '_')}`] || null;
      let account;
      try {
        account = await createAccount(MINER_ACCOUNT, savedMnemonic, `${MINER_ACCOUNT}-miner`);
        console.log(`[BTCPC] Miner account created: ${MINER_ACCOUNT}`);
        if (savedMnemonic) console.log(`[BTCPC] Using saved mnemonic`);
      } catch (createErr) {
        // createAccount may fail if Mongo is down (it tries to create a User doc).
        // Fall back to a minimal on-chain-only creation via ledger entry.
        console.warn(`[BTCPC] Full account creation failed (${createErr.message}), using chain-only path`);
        account = null;
      }

      // Record on the permanent ledger — this is the canonical path that
      // works with or without Mongo. The entry flows through the cross-
      // process queue and P2P mempool gossip to reach the broadcaster.
      const publicKeys = account ? account.publicKeys : {};
      const chainWallets = account ? account.chainWallets : {};
      await ledger.recordAccountCreate(MINER_ACCOUNT, publicKeys, chainWallets, 0);
      console.log(`[BTCPC] Account announced to ledger (permanent)`);

      // Broadcast to all nodes so they have the account immediately
      // (ACCOUNT_ANNOUNCE updates stateStore in memory on every peer)
      try {
        const announceMsg = createMessage('ACCOUNT_ANNOUNCE', {
          username: MINER_ACCOUNT,
          public_keys: publicKeys,
          chain_addresses: chainWallets,
          epoch: 0
        }, p2p.NODE_ID);
        p2p.broadcast(announceMsg);
        console.log(`[BTCPC] Account broadcast to P2P network`);
      } catch (_) {}

      minerAccountExists = true;
    } catch (err) {
      console.error(`[BTCPC] Failed to create miner account: ${err.message}`);
      // Continue anyway — the miner can still participate as a clock/verifier
      // even without a fully-created account. The account will auto-create
      // on the next MINING_REWARD that credits this username.
      minerAccountExists = true; // don't block startup
    }
  } else {
    console.log(`[BTCPC] Miner account '${MINER_ACCOUNT}' found on chain`);
  }

  // Phase E: Node model deleted — register in nodeRegistry in-memory
  if (!nodeRegistry.isRegistered(MINER_ACCOUNT)) {
    nodeRegistry.registerNode(MINER_ACCOUNT, 'miner', 1000, process.env.OLLAMA_URL || 'http://localhost:11434', 0, MINER_ACCOUNT === GENESIS_MINER);
    console.log(`[BTCPC] Mining node registered for ${MINER_ACCOUNT} (nodeRegistry)`);
  }

  running = true;

  // Seed the protocol's epoch cache so heartbeats arriving before the first
  // proposal fires get filed under the right epoch
  try {
    const protocolMod = require('../p2p/protocol');
    const genesisEpoch = stateStore.getEpoch(0);
    if (genesisEpoch && genesisEpoch.started_at) {
      const startedAt = genesisEpoch.started_at instanceof Date
        ? genesisEpoch.started_at
        : new Date(genesisEpoch.started_at);
      const initWall = Math.floor((Date.now() - startedAt.getTime()) / EPOCH_DURATION_MS);
      if (initWall > 0 && protocolMod.setCurrentEpoch) {
        protocolMod.setCurrentEpoch(initWall);
      }
    }
  } catch (_) {}

  // Determine the starting epoch number — use highest of:
  // 1. Time-based calculation from genesis
  // 2. Highest epoch in stateStore / block files
  // 3. P2P chain height (blocks synced from other miners)
  let currentEpoch;
  if (genesis.alreadyExisted) {
    // Genesis: April 12, 2026 10:30 PM Mountain Time (04:30 UTC April 13)
    const genesisTime = 1776054600000;
    const timeBased = Math.floor((Date.now() - genesisTime) / EPOCH_DURATION_MS);

    const chainHeight = stateStore.getChainHeight();
    const dbBased = chainHeight >= 0 ? chainHeight + 1 : 0;

    const { getChainHeight } = require('../p2p/chainSync');
    const p2pHeight = getChainHeight() + 1; // next epoch after highest synced block

    currentEpoch = Math.max(timeBased, dbBased, p2pHeight);
    if (currentEpoch < 1) currentEpoch = 1;

    console.log(`[BTCPC] Epoch sync: time=${timeBased}, store=${dbBased}, p2p=${p2pHeight} → starting at ${currentEpoch}`);
  } else {
    currentEpoch = 0;
  }

  // ── Epoch consensus ──
  // Any eligible node can be the epoch authority.
  // Eligibility: permissioned nodes (approved by genesis) or staked nodes.
  // Genesis miner is always eligible as a fallback.
  const genesisTime = genesis.epoch.started_at.getTime();
  const epochConsensus = require('../chain/authorityRotation');
  // nodeRegistry is already imported at module scope (line 27) — do NOT
  // re-declare here or it creates a TDZ that crashes line 1030.

  // Register this miner in the node registry (permissioned if genesis miner)
  nodeRegistry.registerNode(MINER_ACCOUNT, 'miner', 1000, null, 0, MINER_ACCOUNT === GENESIS_MINER);

  // Load node registry from block files
  nodeRegistry.loadFromBlocks();

  // Phase B: replay blocks into stateStore at miner startup too
  try {
    const replay = require('../chain/replay');
    const replayResult = await replay.replayFromDisk({ verbose: true });
    console.log('[BTCPC] stateStore replay (miner): ' + replayResult.replayed + ' blocks, ' +
      replayResult.accounts + ' accounts, ' + replayResult.durationMs + 'ms');
  } catch (err) {
    console.error('[BTCPC] stateStore replay error:', err.message);
  }

  function getCurrentEpochNumber() {
    return Math.floor((Date.now() - genesisTime) / EPOCH_DURATION_MS);
  }

  function isEpochAuthority() {
    const nodeInfo = nodeRegistry.getNode(MINER_ACCOUNT);
    return epochConsensus.isEpochEligible(MINER_ACCOUNT, nodeInfo, nodeRegistry.PERMISSIONLESS_MIN_STAKE).eligible;
  }

  // ── Miner is NEVER the clock — clocks drive timing, miners do work ──
  // Miner listens for EPOCH_START from clock nodes, mines, and finalizes.
  // Clock nodes (btcpc-clock) handle EPOCH_START/END timing.

  console.log(`[BTCPC] Miner ${MINER_ACCOUNT} — waiting for EPOCH_START from clock nodes...`);

  let lastEpoch = -1;

  p2p.onMessage(async (msg) => {
    // ── EPOCH_START from a clock node — start mining ──
    if (msg.type === 'EPOCH_START') {
      const data = msg.data || {};
      if (!data.epoch_number || data.epoch_number <= lastEpoch) return;

      lastEpoch = data.epoch_number;
      currentEpoch = data.epoch_number;
      console.log(`[BTCPC] Epoch ${currentEpoch} STARTED (from ${data.authority || 'clock'})`);

      const epochToMine = currentEpoch;
      setImmediate(async () => {
        try {
          const { syncLocalModels } = require('../services/modelRegistry');
          syncLocalModels(null).catch(() => {});

          await mineEpoch(epochToMine);
        } catch (err) {
          console.error(`[BTCPC] Epoch ${epochToMine} mining error:`, err.message);
        }
      });
    }

    // EPOCH_END is a no-op — block proposals are now driven by the wall clock
    // loop, not by EPOCH_END messages. Kept here only for log compat.
    // The unified BLOCK_PROPOSAL flow handles all consensus.
  });

  // ── Consensus resolution callback — when the network agrees, apply and broadcast ──
  const finConsensus = require('../chain/finalizationConsensus');
  finConsensus.onResolved(async (epochNumber, winner) => {
    try {
      // Only the designated broadcaster applies and broadcasts
      if (!finConsensus.amIBroadcaster(epochNumber, MINER_ACCOUNT)) {
        console.log(`[BTCPC] Epoch ${epochNumber} consensus reached — ${winner.proposer} will broadcast`);
        return;
      }

      console.log(`[BTCPC] Epoch ${epochNumber} consensus reached — I am the broadcaster`);

      // Apply the winning proposal to DB + ledger
      const epoch = await applyFinalization(epochNumber, winner);
      if (!epoch) return;

      const bd = epoch._blockData || {};

      const blockMsg = createMessage('EPOCH_FINALIZED', {
        epoch_number: epochNumber,
        block_reward: winner.block_reward,
        reward_number: winner.reward_number,
        epochs_deferred: winner.epochs_deferred,
        settled_jobs: winner.settled_jobs || 0,
        rewards: (winner.rewards || []).map(r => ({
          miner: r.miner,
          amount: r.amount
        })),
        total_work: winner.total_work,
        consensus_hash: winner.consensus_hash,
        authority: MINER_ACCOUNT,
        ledger: bd.ledger || [],
        header_hex: bd.header_hex || null,
        block_hash: bd.block_hash || null,
        state_root: bd.state_root || null,
        is_finality: bd.is_finality || false
      }, p2p.NODE_ID);
      p2p.broadcast(blockMsg);
      console.log(`[BTCPC] Block ${epochNumber} broadcast to network (consensus)`);

      // Auto-submit cross-chain claims for this miner's rewards
      try {
        const myReward = (winner.rewards || []).find(r => r.miner === MINER_ACCOUNT);
        if (myReward && myReward.amount > 0) {
          const { submitAllClaims } = require('../claims/evmClaimSubmitter');
          const postingKey = process.env.BTCPC_SHIN_POSTING_KEY || process.env.BTCPC_POSTING_KEY;
          if (postingKey) {
            const linkedChains = { evm: process.env.BTCPC_EVM_ADDRESS };
            submitAllClaims(MINER_ACCOUNT, epochNumber, myReward.amount, linkedChains, postingKey)
              .then(results => {
                if (results.length > 0) console.log(`[BTCPC] Cross-chain claims: ${results.length} submitted`);
              })
              .catch(() => {});
          }
        }
      } catch (_) {}
    } catch (err) {
      console.error(`[BTCPC] Consensus apply error for epoch ${epochNumber}:`, err.message);
    }
  });

  // ── Unified clock loop: any node with this code is also a clock ──
  // Wall clock advances → build BLOCK_PROPOSAL from gossiped attestations
  // → broadcast → consensus picks winner. Single message, no ceremony.
  //
  // Miners do work. Verifiers check work. Clocks aggregate and propose blocks.
  // This same loop runs in btcpc-clock too — both nodes are clocks.
  const blockProposal = require('../chain/blockProposal');
  let lastProposedEpoch = -1;
  // The chain's "current epoch" is the latest finalized epoch. We propose
  // a block for currentEpoch + 1 once wall clock reaches that boundary.
  let highestFinalizedEpoch = currentEpoch - 1; // currentEpoch was max + 1

  miningInterval = setInterval(() => {
    if (!running) return;
    const wallEpoch = getCurrentEpochNumber();
    // Target: the next epoch to propose is highestFinalized + 1
    const targetEpoch = highestFinalizedEpoch + 1;
    // Only propose once wall clock has reached this epoch
    if (wallEpoch < targetEpoch) return;
    if (targetEpoch <= lastProposedEpoch) return;

    lastProposedEpoch = targetEpoch;

    try {
      const reward = getBlockReward(targetEpoch);
      const protocolModule = require('../p2p/protocol');
      // Update protocol's epoch cache so subsequent heartbeats file correctly
      if (protocolModule.setCurrentEpoch) protocolModule.setCurrentEpoch(targetEpoch);
      const proposal = blockProposal.buildProposal({
        epochNumber: targetEpoch,
        blockReward: reward,
        proposerAccount: MINER_ACCOUNT,
        protocol: protocolModule,
      });

      const msg = createMessage('BLOCK_PROPOSAL', proposal, p2p.NODE_ID);
      p2p.broadcast(msg);
      console.log(`[BTCPC] Block proposal for epoch ${targetEpoch}: ${proposal.miners_active} miner(s), ${proposal.verifiers_active} verifier(s), ${proposal.clocks_active} clock(s), work=${proposal.total_work}`);

      // Also submit to local consensus tracker
      const finConsensus = require('../chain/finalizationConsensus');
      finConsensus.submitProposal(targetEpoch, {
        proposer: MINER_ACCOUNT,
        rewards: proposal.rewards.map(r => ({ miner: r.to, amount: r.amount, type: r.type })),
        total_work: proposal.total_work,
        consensus_hash: proposal.consensus_hash,
        settled_jobs: proposal.miners_active,
        block_reward: reward,
        timestamp: proposal.timestamp,
      });
    } catch (err) {
      console.error(`[BTCPC] Block proposal error for epoch ${targetEpoch}:`, err.message);
    }
  }, 5000); // check every 5s

  // When an epoch is resolved and applied, advance highestFinalizedEpoch
  // so the loop proposes the next epoch when wall clock reaches it
  const finConsensusForTracking = require('../chain/finalizationConsensus');
  finConsensusForTracking.onResolved((epochNumber) => {
    if (epochNumber > highestFinalizedEpoch) {
      highestFinalizedEpoch = epochNumber;
    }
  });

  console.log(`[BTCPC] Block proposal loop active — checking every 5s, epoch duration ${EPOCH_DURATION_MS / 1000}s`);

  // Start auto-updater (checks GitHub every 15min, stages + notifies)
  startAutoUpdater();

  // Start model manager (auto-pulls in-demand models within disk budget)
  startModelManager();
}

/**
 * Stop the mining daemon gracefully.
 */
function stopMiner() {
  running = false;
  if (miningInterval) {
    clearInterval(miningInterval);
    miningInterval = null;
  }
  console.log('[BTCPC] Mining daemon stopped');
}

module.exports = {
  startMiner,
  stopMiner,
  mineEpoch
};
