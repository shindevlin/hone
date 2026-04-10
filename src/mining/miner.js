"use strict";

const Epoch = require('../models/Epoch');
const Node = require('../models/Node');
const User = require('../models/User');
const Wallet = require('../models/Wallet');
const WorkProof = require('../models/WorkProof');
const { getBlockReward } = require('../services/emissionSchedule');
const { finalizeEpoch, EPOCH_DURATION_MS } = require('../services/epochManager');
const { generateWork, getEpochMetadata } = require('./workGenerator');
const { computeStateHash } = require('./stateHash');
const { createGenesisBlock, GENESIS_MINER } = require('./genesisBlock');
const MINER_ACCOUNT = process.env.BTCPC_MINER || GENESIS_MINER;
const GenesisDream = require('../models/GenesisDream');
const MiningProof = require('../models/MiningProof');
const { filterInscription } = require('../services/contentFilter');
const { generateAllClaimProofs } = require('../claims/claimProofGenerator');
const CrossChainClaim = require('../models/CrossChainClaim');
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
const mempool = require('../p2p/mempool');

const FINALITY_INTERVAL = parseInt(process.env.BTCPC_FINALITY_INTERVAL) || 100;
const WORK_ITEMS_PER_EPOCH = parseInt(process.env.BTCPC_WORK_PER_EPOCH) || 3;
const resourceManager = require('../services/resourceManager');
const { notifyUpdate, notifyMining } = require('../services/systemNotify');
const MODEL = process.env.BTCPC_MODEL || 'qwen3.5:27b';
const http = require('http');

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
  const epoch = await Epoch.findOne({ epoch_number: epochNumber });
  if (!epoch) return null;
  if (epoch.status === 'finalized') return epoch;

  if (pendingFinalizations.has(epochNumber)) {
    return waitForFinalization(epochNumber);
  }

  pendingFinalizations.add(epochNumber);

  // Count active miners on the network
  const { getIdleMiners } = require('../p2p/protocol');
  const activeMiners = await Node.countDocuments({ status: 'active' });
  console.log(`[BTCPC] Epoch ${epochNumber}: waiting for proofs from ${activeMiners} active miner(s)...`);

  // Poll until all active miners have submitted proofs OR announced idle
  const startTime = Date.now();
  while (Date.now() - startTime < MAX_FINALIZATION_WAIT_MS) {
    const proofCount = await MiningProof.countDocuments({ block_number: epochNumber });
    const idleCount = getIdleMiners(epochNumber).size;
    const accountedFor = proofCount + idleCount;

    if (accountedFor >= activeMiners) {
      console.log(`[BTCPC] Epoch ${epochNumber}: all miners accounted for (${proofCount} proofs, ${idleCount} idle). Finalizing.`);
      break;
    }

    // Check if epoch was finalized by another node
    const check = await Epoch.findOne({ epoch_number: epochNumber });
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
    const epoch = await Epoch.findOne({ epoch_number: epochNumber });
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
  const InferenceJob = require('../models/InferenceJob');
  const finConsensus = require('../chain/finalizationConsensus');
  const nodeRegistry = require('../chain/nodeRegistry');

  const rewardedEpochs = await Epoch.countDocuments({ status: 'finalized', settled_jobs: { $gt: 0 } });
  const rewardNumber = rewardedEpochs;
  const blockReward = getBlockReward(rewardNumber);
  const epochsDeferred = epochNumber - rewardNumber;

  // Sweep: settle verified jobs
  const candidateJobs = await InferenceJob.find({
    status: { $in: ['completed', 'verifying'] },
    settlement_epoch: null
  });

  let sweptCount = 0;
  for (const job of candidateJobs) {
    const required = job.required_verifications || 1;
    const verified = (job.verifications || []).length;
    const effectiveVerified = verified > 0 ? verified : (job.node_name ? 1 : 0);
    if (effectiveVerified >= required) {
      job.settlement_epoch = epochNumber;
      job.settled_at = new Date();
      if (job.status !== 'settled') job.status = 'settled';
      await job.save();
      sweptCount++;
    }
  }

  if (sweptCount > 0) {
    console.log(`[BTCPC] Epoch ${epochNumber}: ${sweptCount} job(s) settled`);
  }

  // Release escrows for settled jobs — pay the miner, refund overpayment
  const escrow = require('../services/escrow');
  const settledJobs = await InferenceJob.find({ settlement_epoch: epochNumber });
  for (const job of settledJobs) {
    const miner = job.node_name || (job.verifications && job.verifications[0] && job.verifications[0].miner);
    if (miner && job.cost > 0) {
      try {
        await escrow.releaseForJob(job.job_id, miner, job.cost);
      } catch (_) {} // non-fatal — escrow may not exist for pre-escrow jobs
    }
  }

  // Sweep any stale escrows (safety net — auto-refund after 10 min)
  await escrow.sweepEscrows(600000).catch(() => {});
  const syntheticProofs = await MiningProof.find({ block_number: epochNumber });

  // Build per-miner work values
  const minerWork = {};
  const { verifyModelParams } = require('./workGenerator');

  for (const job of settledJobs) {
    if (job.verifications && job.verifications.length > 0) {
      for (const v of job.verifications) {
        if (!v.miner || !v.result_hash) continue;
        if (v.model_hash) {
          const registryHash = await getRegistryModelHash(job.model);
          if (registryHash && v.model_hash !== registryHash) continue;
        }
        if (!minerWork[v.miner]) minerWork[v.miner] = { work_value: 0, models: new Set() };
        minerWork[v.miner].work_value += (v.work_value || 0);
        minerWork[v.miner].models.add(job.model);
      }
    } else if (job.node_name) {
      const params = await verifyModelParams(job.model || 'qwen3:4b');
      const wv = (job.tokens_generated || 0) * params;
      if (!minerWork[job.node_name]) minerWork[job.node_name] = { work_value: 0, models: new Set() };
      minerWork[job.node_name].work_value += wv;
      minerWork[job.node_name].models.add(job.model);
    }
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

  // ── Reward split ──
  // WITH WORK:  85% miners | 10% verifiers | 5% clocks
  // NO WORK:    0% miners  |  1% verifiers  | 1% clocks | 98% unminted
  const MINER_PCT = 0.85;
  const VERIFIER_PCT = 0.10;
  const CLOCK_PCT = 0.05;
  const IDLE_VERIFIER_PCT = 0.01;
  const IDLE_CLOCK_PCT = 0.01;

  const rewards = [];

  const { getActiveClockNodes } = require('../p2p/protocol');
  // Clock nodes: any account that heartbeat this epoch.
  // Filter out raw nodeIds (hex strings) and nodes prefixed clock- (anonymous).
  // Do NOT require nodeRegistry — clock nodes are open participation, no stake.
  const activeClocks = getActiveClockNodes(epochNumber).filter(a =>
    a && !a.startsWith('clock-') && !/^[a-f0-9]{32,}$/i.test(a) && /^[a-z0-9][a-z0-9-]{2,19}$/.test(a)
  );

  // Get active verifiers for this epoch (nodes that actually verified inference)
  const verifier = require('../inference/verifier');
  const { getActiveVerifiers } = require('../p2p/protocol');
  // Verifiers: any account that submitted a VERIFY_RESPONSE this epoch.
  // Same open-participation filter as clocks.
  const activeVerifiers = getActiveVerifiers(epochNumber).filter(a =>
    a && /^[a-z0-9][a-z0-9-]{2,19}$/.test(a)
  );
  // Use real verifiers if any responded, otherwise fall back to clock nodes
  const verifierPool = activeVerifiers.length > 0 ? activeVerifiers : activeClocks;

  if (miners.length === 0 || totalWorkValue === 0) {
    // Empty epoch — minimal rewards to keep nodes online
    // 98% NOT MINTED (emission slot preserved)
    const idleReward = blockReward; // full reward available but mostly unspent

    if (verifierPool.length > 0) {
      const vShare = parseFloat((idleReward * IDLE_VERIFIER_PCT / verifierPool.length).toFixed(10));
      for (const v of verifierPool) {
        rewards.push({ miner: v, amount: vShare, type: 'verifier' });
      }
    }

    if (activeClocks.length > 0) {
      const cShare = parseFloat((idleReward * IDLE_CLOCK_PCT / activeClocks.length).toFixed(10));
      for (const c of activeClocks) {
        // Don't double-pay if already got verifier reward
        const existing = rewards.find(r => r.miner === c);
        if (existing) {
          existing.amount = parseFloat((existing.amount + cShare).toFixed(10));
        } else {
          rewards.push({ miner: c, amount: cShare, type: 'clock' });
        }
      }
    }
  } else {
    // Active epoch — full reward distribution
    const minerPool = parseFloat((blockReward * MINER_PCT).toFixed(10));
    const verifierRewardPool = parseFloat((blockReward * VERIFIER_PCT).toFixed(10));
    const clockRewardPool = parseFloat((blockReward * CLOCK_PCT).toFixed(10));

    // Miners: 85% by work_value
    for (const miner of miners) {
      const share = parseFloat((minerPool * (minerWork[miner].work_value / totalWorkValue)).toFixed(10));
      rewards.push({ miner, amount: share, type: 'mining' });
    }

    // Verifiers: 10% split among active verifiers (capped per job, not per network)
    if (verifierPool.length > 0) {
      const vCount = Math.min(verifierPool.length, verifier.getVerifierCount(verifierPool.length + miners.length));
      const vShare = parseFloat((verifierRewardPool / vCount).toFixed(10));
      // Select the verifiers for this epoch deterministically
      const selectedVerifiers = verifier.selectVerifiers(
        '0'.repeat(64), String(epochNumber), '', verifierPool, vCount
      );
      for (const v of selectedVerifiers) {
        rewards.push({ miner: v, amount: vShare, type: 'verifier' });
      }
      // Unselected verifier reward goes back to miners
      if (selectedVerifiers.length < verifierPool.length) {
        const unused = verifierRewardPool - (vShare * selectedVerifiers.length);
        if (unused > 0 && miners.length > 0) {
          const extra = parseFloat((unused / miners.length).toFixed(10));
          for (const r of rewards) {
            if (r.type === 'mining') r.amount = parseFloat((r.amount + extra).toFixed(10));
          }
        }
      }
    } else {
      // No verifiers — redistribute to miners
      const extra = parseFloat((verifierRewardPool / miners.length).toFixed(10));
      for (const r of rewards) {
        if (r.type === 'mining') r.amount = parseFloat((r.amount + extra).toFixed(10));
      }
    }

    // Clocks: 5% split among ALL active clocks
    if (activeClocks.length > 0) {
      const cShare = parseFloat((clockRewardPool / activeClocks.length).toFixed(10));
      for (const c of activeClocks) {
        const existing = rewards.find(r => r.miner === c);
        if (existing) {
          existing.amount = parseFloat((existing.amount + cShare).toFixed(10));
        } else {
          rewards.push({ miner: c, amount: cShare, type: 'clock' });
        }
      }
    } else if (miners.length > 0) {
      const extra = parseFloat((clockRewardPool / miners.length).toFixed(10));
      for (const r of rewards) {
        if (r.type === 'mining') r.amount = parseFloat((r.amount + extra).toFixed(10));
      }
    }
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
  let epoch = await Epoch.findOne({ epoch_number: epochNumber });
  if (!epoch) {
    // No epoch document yet — create one (the unified BLOCK_PROPOSAL flow
    // doesn't pre-create epoch records via EPOCH_START anymore)
    try {
      epoch = await Epoch.create({
        epoch_number: epochNumber,
        status: 'active',
        started_at: new Date(),
        block_reward: proposal.block_reward || 0,
      });
    } catch (e) {
      // Race: another node created it first
      epoch = await Epoch.findOne({ epoch_number: epochNumber });
      if (!epoch) return null;
    }
  }
  if (epoch.status === 'finalized') return epoch; // already done

  const rewards = proposal.rewards || [];

  // Write rewards to permanent ledger
  for (const r of rewards) {
    await ledger.recordMiningReward(r.miner, r.amount, epochNumber);
    await ledger.updateWalletCache(r.miner, 'BTCPC', r.amount);

    // Update mining proof
    const proof = await MiningProof.findOne({ block_number: epochNumber, miner: r.miner });
    if (proof) {
      proof.reward_earned = r.amount;
      await proof.save();
    }

    console.log(`[BTCPC]   ${r.miner}: ${r.amount.toFixed(4)} BTCPC (${r.type || 'mining'})`);
  }

  // Finalize epoch record
  epoch.consensus_hash = proposal.consensus_hash || '0'.repeat(64);
  epoch.total_work = proposal.total_work || 0;
  epoch.rewards_distributed = rewards.map(r => ({ node_id: r.miner, amount: r.amount }));
  epoch.block_reward = proposal.block_reward || 0;
  epoch.reward_number = proposal.reward_number;
  epoch.epochs_deferred = proposal.epochs_deferred || 0;
  epoch.ended_at = new Date();
  epoch.status = 'finalized';
  epoch.settled_jobs = proposal.settled_jobs || 0;
  await epoch.save();

  console.log(`[BTCPC] Epoch ${epochNumber} finalized | reward #${proposal.reward_number} | ${rewards.length} reward(s) | ${(proposal.block_reward || 0).toFixed(4)} BTCPC`);

  // ── Write block to disk — source of truth ──
  try {
    // Get ledger entries for this epoch (already flushed to pending)
    const epochLedgerEntries = ledger.flushPendingEntries();

    // Apply entries to SMT for state root
    stateManager.applyLedgerEntries(epochLedgerEntries);
    const stateRoot = stateManager.getStateRoot();

    // Compute Merkle roots
    const txHashes = epochLedgerEntries.map(e => blockStore.hashLedgerEntry(e));
    const txMerkleRoot = Block.computeMerkleRoot(txHashes);

    // Collect compute proofs for this epoch
    const epochProofs = await WorkProof.find({ epoch_number: epochNumber }).lean();
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

    const miningProofs = await MiningProof.find({ block_number: epochNumber }).lean();

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

  // Get miner references
  const user = await User.findOne({ username: MINER_ACCOUNT });
  if (!user) {
    console.error(`[BTCPC] Miner account '${MINER_ACCOUNT}' not found.`);
    return;
  }

  const node = await Node.findOne({ account: user._id });
  if (!node) {
    console.error(`[BTCPC] Mining node for '${MINER_ACCOUNT}' not found.`);
    return;
  }

  const wallet = await Wallet.findOne({ userId: user._id });
  if (!wallet) {
    console.error('[BTCPC] Genesis wallet not found. Run genesis first.');
    return;
  }

  // Ensure epoch record exists
  let epoch = await Epoch.findOne({ epoch_number: epochNumber });
  if (!epoch) {
    const reward = getBlockReward(epochNumber);
    epoch = new Epoch({
      epoch_number: epochNumber,
      started_at: new Date(),
      block_reward: reward,
      status: 'active'
    });
    await epoch.save();
  }

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

  // Only skip synthetic work if THIS miner has RECENT active inference jobs
  // Stale claims (>10 min) are expired back to pending so other miners can take them
  const InferenceJob = require('../models/InferenceJob');
  const STALE_CLAIM_MS = 600000; // 10 min
  const staleThreshold = new Date(Date.now() - STALE_CLAIM_MS);

  // Expire stale claims from this miner
  const staleJobs = await InferenceJob.updateMany(
    { claimed_by: MINER_ACCOUNT, status: { $in: ['claimed', 'processing'] }, claimed_at: { $lt: staleThreshold } },
    { $set: { status: 'pending', claimed_by: null, claimed_at: null } }
  );
  if (staleJobs.modifiedCount > 0) {
    console.log(`[BTCPC]   Expired ${staleJobs.modifiedCount} stale claim(s) back to pending`);
  }

  // No synthetic work — miners only earn from real inference jobs
  const syntheticCount = 0;

  // Verify mining model against Ollama registry before doing any work
  const modelCheck = await verifyModel(MODEL);
  if (!modelCheck.verified) {
    console.error(`[BTCPC] REFUSING TO MINE: model ${MODEL} failed verification — ${modelCheck.reason}`);
    const idleMsg = createMessage('MINER_IDLE', {
      block_number: epochNumber,
      miner: MINER_ACCOUNT,
      reason: 'model_verification_failed'
    }, p2p.NODE_ID);
    p2p.broadcast(idleMsg);
    return;
  }
  const modelHash = modelCheck.localHash; // store on proofs for verification

  for (let i = 0; i < syntheticCount; i++) {
    try {
      const isGenesisFirstWork = (epochNumber === 0 && i === 0);
      if (isGenesisFirstWork) {
        console.log(`[BTCPC]   GENESIS INFERENCE -- the first dream computed into reality`);
      } else {
        console.log(`[BTCPC]   Work item ${i + 1}/${syntheticCount} -- sending to Ollama (${MODEL})...`);
      }
      const work = await generateWork(MODEL, isGenesisFirstWork ? GENESIS_PROMPT : undefined);

      const proof = new WorkProof({
        epoch_number: epochNumber,
        node_id: MINER_ACCOUNT,
        prompt_hash: work.prompt_hash,
        result_hash: work.result_hash,
        model: work.model,
        tokens_generated: work.tokens_generated,
        model_weight_factor: work.model_weight_factor,
        work_value: work.work_value
      });
      await proof.save();

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
    // Check for inference jobs completed since the previous epoch
    // (jobs completed in the last EPOCH_DURATION window belong to this epoch)
    const lookback = new Date(Date.now() - EPOCH_DURATION_MS);
    const recentJobs = await InferenceJob.find({
      node_name: MINER_ACCOUNT,
      status: 'completed',
      settlement_epoch: null, // not yet assigned to an epoch
      completed_at: { $gte: lookback }
    });

    if (recentJobs.length > 0) {
      // Calculate work value from inference jobs processed
      const { verifyModelParams } = require('./workGenerator');
      for (const job of recentJobs) {
        const params = await verifyModelParams(job.model || MODEL);
        const tokens = job.tokens_generated || 0;
        totalTokens += tokens;
        totalWorkValue += tokens * params;

        // Tag job with settlement epoch
        if (!job.settlement_epoch) {
          job.settlement_epoch = epochNumber;
          job.settled_at = new Date();
          await job.save();
        }
      }
      console.log(`[BTCPC]   ${recentJobs.length} inference job(s) completed this epoch: ${totalTokens} tokens, work_value=${totalWorkValue}`);
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
  const previousEpoch = await Epoch.findOne({ epoch_number: epochNumber - 1 });
  const previousHash = previousEpoch ? previousEpoch.consensus_hash : '0'.repeat(64);
  const stateHash = await computeStateHash(epochNumber, previousHash);

  // Step 3: Submit epoch commitment
  epoch.commitments.push({
    node_id: node._id,
    state_hash: stateHash,
    tx_count: 0,
    inference_count: workProofs.length,
    submitted_at: new Date()
  });
  epoch.consensus_hash = stateHash;
  await epoch.save();

  // Step 4: Create genesis dream for this block (mandatory)
  const metadata = getEpochMetadata(epochNumber);
  const existingDream = await GenesisDream.findOne({ block_number: epochNumber });
  if (!existingDream) {
    const workHash = workProofs.length > 0 ? workProofs[0].result_hash : '0'.repeat(64);

    // Apply content filter to inscription text
    const tagResult = filterInscription(metadata.tag);
    const projectResult = filterInscription(metadata.project);
    const filteredTag = tagResult.filtered_text;
    const filteredProject = projectResult.filtered_text;
    if (tagResult.was_redacted || projectResult.was_redacted) {
      console.log(`[BTCPC]   Content filter: inscription text redacted`);
    }

    const dream = new GenesisDream({
      block_number: epochNumber,
      original_miner: MINER_ACCOUNT,
      current_owner: MINER_ACCOUNT,
      inscription: {
        project: filteredProject,
        tag: filteredTag,
        custom_data: { epoch: epochNumber, model: MODEL, work_items: workProofs.length }
      },
      proof: {
        state_hash: stateHash,
        work_hash: workHash,
        tokens_computed: totalTokens,
        model: MODEL
      }
    });
    await dream.save();
    console.log(`[BTCPC]   Dream #${epochNumber}: "${filteredTag}" [${filteredProject}]`);
  }

  // Step 4b: Create soulbound mining proof (reward set to 0 until finalization splits it)
  // Save to DB — if duplicate, that's fine, we still broadcast
  try {
    const existingProof = await MiningProof.findOne({ block_number: epochNumber, miner: MINER_ACCOUNT });
    if (!existingProof) {
      const miningProof = new MiningProof({
        block_number: epochNumber,
        miner: MINER_ACCOUNT,
        reward_earned: 0,
        model: MODEL,
        model_hash: modelHash,
        tokens_computed: totalTokens,
        work_value: totalWorkValue,
        state_hash: stateHash
      });
      await miningProof.save();
      console.log(`[BTCPC]   Mining Proof #${epochNumber}: submitted by ${MINER_ACCOUNT} (work_value: ${totalWorkValue})`);
    } else {
      console.log(`[BTCPC]   Mining Proof #${epochNumber}: already exists for ${MINER_ACCOUNT}`);
    }
  } catch (err) {
    console.log(`[BTCPC]   Mining Proof #${epochNumber}: save error (${err.message}) — broadcasting anyway`);
  }

  // ALWAYS broadcast proof via P2P — even if DB save failed or proof already existed
  // Other nodes need this to finalize the epoch
  {
    const proofMsg = createMessage('MINING_PROOF', {
      block_number: epochNumber,
      miner: MINER_ACCOUNT,
      model: MODEL,
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
    // Use the miner's actual reward share, not the full block reward
    const myProof = await MiningProof.findOne({ block_number: epochNumber, miner: MINER_ACCOUNT });
    const myReward = myProof ? myProof.reward_earned : 0;

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

        for (const proof of claimProofs) {
          const existing = await CrossChainClaim.findOne({
            miner: proof.miner,
            chain: proof.chain,
            epoch: proof.epoch
          });
          if (!existing) {
            const claim = new CrossChainClaim({
              miner: proof.miner,
              chain: proof.chain,
              target_wallet: proof.target_wallet,
              epoch: proof.epoch,
              native_reward: finalized.block_reward,
              claim_amount: proof.amount,
              period: proof.period,
              cross_chain_ratio: proof.cross_chain_ratio,
              proof_signature: proof.proof_signature,
              proof_recovery: proof.proof_recovery
            });
            await claim.save();
          }
        }

        if (claimProofs.length > 0) {
          console.log('[BTCPC]   Cross-chain proofs: ' + claimProofs.map(function (p) { return p.chain; }).join(', '));
        }
      } catch (err) {
        console.error('[BTCPC]   Cross-chain proof generation error: ' + err.message);
      }
    }
  }

  // Step 5c: Update node tracking
  node.last_epoch_commitment = epochNumber;
  await node.save();

  // Step 6: Log results
  const balanceBTCPC = wallet.balance.get('BTCPC') || 0;
  // Re-read wallet to get updated balance after reward distribution
  const updatedWallet = await Wallet.findOne({ userId: user._id });
  const currentBalance = updatedWallet ? (updatedWallet.balance.get('BTCPC') || 0) : balanceBTCPC;

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
  console.log(`[BTCPC] Model:      ${MODEL}`);
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
    // Register SIK hash on Node document
    const sikUser = await User.findOne({ username: MINER_ACCOUNT });
    if (sikUser) {
      const sikNode = await Node.findOne({ account: sikUser._id });
      if (sikNode && sikNode.sik_hash !== sik.sik_hash) {
        sikNode.sik_hash = sik.sik_hash;
        sikNode.sik_type = sik.software_only ? 'software' : 'silicon';
        await sikNode.save();
        console.log(`[BTCPC] SIK registered on node: ${sik.sik_hash.slice(0, 16)}... (${sikNode.sik_type})`);
      }
    }
  } catch (err) {
    console.warn('[BTCPC] SIK probe skipped:', err.message);
  }

  // Sync local Ollama models to node record
  try {
    const { syncLocalModels } = require('../services/modelRegistry');
    const user = await User.findOne({ username: MINER_ACCOUNT });
    const node = user ? await Node.findOne({ account: user._id }) : null;
    const models = await syncLocalModels(node?._id);
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

  // Ensure this miner's account exists (auto-register on first run)
  // Uses BTCPC_MNEMONIC from .env if set — preserves saved keys across chain resets
  let minerUser = await User.findOne({ username: MINER_ACCOUNT });
  if (!minerUser) {
    const { createAccount } = require('../wallet/accountManager');
    const savedMnemonic = process.env.BTCPC_MNEMONIC || null;
    try {
      const account = await createAccount(MINER_ACCOUNT, savedMnemonic, `${MINER_ACCOUNT}-miner`);
      minerUser = await User.findOne({ username: MINER_ACCOUNT });
      console.log(`[BTCPC] Miner account created: ${MINER_ACCOUNT} (${account.address})`);
      if (savedMnemonic) {
        console.log(`[BTCPC] Using saved mnemonic from BTCPC_MNEMONIC env`);
      }
      console.log(`[BTCPC] Wallets: ${JSON.stringify(account.chainWallets)}`);

      // Record account creation on the permanent ledger
      await ledger.recordAccountCreate(MINER_ACCOUNT, account.publicKeys, account.chainWallets, 0);
      console.log(`[BTCPC] Account announced to ledger (permanent)`);

      // Broadcast to all nodes so they have the account too
      const announceMsg = createMessage('ACCOUNT_ANNOUNCE', {
        username: MINER_ACCOUNT,
        public_keys: account.publicKeys,
        chain_addresses: account.chainWallets,
        epoch: 0
      }, p2p.NODE_ID);
      p2p.broadcast(announceMsg);
      console.log(`[BTCPC] Account broadcast to P2P network`);

      // Check for unclaimed tokens sent to this username before it existed
      const crypto = require('crypto');
      const unclaimedAddr = 'BTCPC' + crypto.createHash('sha256').update('btcpc-username:' + MINER_ACCOUNT).digest('hex').slice(0, 40);
      const unclaimedWallet = await Wallet.findOne({ address: unclaimedAddr, userId: null, chain: 'btcpc' });
      if (unclaimedWallet) {
        const unclaimedBalance = unclaimedWallet.balance.get('BTCPC') || 0;
        if (unclaimedBalance > 0) {
          // Transfer unclaimed tokens to the new account
          const myWallet = await Wallet.findOne({ userId: minerUser._id, chain: 'btcpc' });
          if (myWallet) {
            const myBal = myWallet.balance.get('BTCPC') || 0;
            myWallet.balance.set('BTCPC', myBal + unclaimedBalance);
            await myWallet.save();
            unclaimedWallet.balance.set('BTCPC', 0);
            unclaimedWallet.userId = minerUser._id; // link to real account
            await unclaimedWallet.save();
            console.log(`[BTCPC] Claimed ${unclaimedBalance.toFixed(4)} BTCPC from unclaimed address`);
          }
        }
      }
    } catch (err) {
      console.error(`[BTCPC] Failed to create miner account: ${err.message}`);
    }
  }

  // Ensure mining node exists
  if (minerUser) {
    let minerNode = await Node.findOne({ account: minerUser._id });
    if (!minerNode) {
      minerNode = new Node({
        account: minerUser._id,
        endpoint: process.env.OLLAMA_URL || 'http://localhost:11434',
        models: [MODEL],
        stake_amount: 1000,
        status: 'active',
        inference_engine: 'ollama',
        reputation: 100
      });
      await minerNode.save();
      console.log(`[BTCPC] Mining node registered for ${MINER_ACCOUNT}`);
    }
  }

  running = true;

  // Determine the starting epoch number — use highest of:
  // 1. Time-based calculation from genesis
  // 2. Highest epoch in MongoDB
  // 3. P2P chain height (blocks synced from other miners)
  let currentEpoch;
  if (genesis.alreadyExisted) {
    const genesisTime = genesis.epoch.started_at.getTime();
    const timeBased = Math.floor((Date.now() - genesisTime) / EPOCH_DURATION_MS);

    const highestInDB = await Epoch.findOne().sort({ epoch_number: -1 }).lean();
    const dbBased = highestInDB ? highestInDB.epoch_number + 1 : 0;

    const { getChainHeight } = require('../p2p/chainSync');
    const p2pHeight = getChainHeight() + 1; // next epoch after highest synced block

    currentEpoch = Math.max(timeBased, dbBased, p2pHeight);
    if (currentEpoch < 1) currentEpoch = 1;

    console.log(`[BTCPC] Epoch sync: time=${timeBased}, db=${dbBased}, p2p=${p2pHeight} → starting at ${currentEpoch}`);
  } else {
    currentEpoch = 0;
  }

  // ── Epoch consensus ──
  // Any eligible node can be the epoch authority.
  // Eligibility: permissioned nodes (approved by genesis) or staked nodes.
  // Genesis miner is always eligible as a fallback.
  const genesisTime = genesis.epoch.started_at.getTime();
  const epochConsensus = require('../chain/authorityRotation');
  const nodeRegistry = require('../chain/nodeRegistry');

  // Register this miner in the node registry (permissioned if genesis miner)
  nodeRegistry.registerNode(MINER_ACCOUNT, 'miner', 1000, null, 0, MINER_ACCOUNT === GENESIS_MINER);

  // Load node registry from block files
  nodeRegistry.loadFromBlocks();

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
          const _user = await User.findOne({ username: MINER_ACCOUNT });
          const _node = _user ? await Node.findOne({ account: _user._id }) : null;
          syncLocalModels(_node?._id).catch(() => {});

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

    // Update protocol's epoch cache so handlers know what epoch we're in
    p2p.setCurrentEpoch ? p2p.setCurrentEpoch(targetEpoch) : null;
    lastProposedEpoch = targetEpoch;

    try {
      const reward = getBlockReward(targetEpoch);
      const proposal = blockProposal.buildProposal({
        epochNumber: targetEpoch,
        blockReward: reward,
        proposerAccount: MINER_ACCOUNT,
        protocol: p2p,
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
