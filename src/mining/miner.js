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
async function finalizeAndSplitRewards(epochNumber) {
  // Atomic claim: only one miner can finalize an epoch
  const epoch = await Epoch.findOneAndUpdate(
    { epoch_number: epochNumber, status: { $ne: 'finalized' } },
    { $set: { status: 'finalizing' } },
    { new: true }
  );
  if (!epoch) return await Epoch.findOne({ epoch_number: epochNumber }); // already finalized by another miner

  const { getRegistryModelHash } = require('../services/modelVerifier');
  const InferenceJob = require('../models/InferenceJob');

  // ── Reward number tracks emission schedule, not epoch number ──
  // Empty epochs (no settled jobs) don't consume a reward slot.
  // The reward number only advances when work is done.
  // This means the emission schedule stretches — no rewards are skipped or stacked.
  // Reward number = count of epochs that actually had work.
  // Empty epochs don't consume emission slots — the schedule stretches.
  const rewardedEpochs = await Epoch.countDocuments({ status: 'finalized', settled_jobs: { $gt: 0 } });
  const rewardNumber = rewardedEpochs;
  const blockReward = getBlockReward(rewardNumber);
  const epochsDeferred = epochNumber - rewardNumber; // how far behind the reward schedule is

  // ── Sweep: find jobs that are fully verified and ready to settle ──
  // A job settles when it has enough verifications (3 in consensus, 1 in genesis).
  // Authority tags them at EPOCH_END — miners don't get paid until all verifications are in.
  const candidateJobs = await InferenceJob.find({
    status: { $in: ['completed', 'verifying'] },
    settlement_epoch: null
  });

  let sweptCount = 0;
  for (const job of candidateJobs) {
    const required = job.required_verifications || 1;
    const verified = (job.verifications || []).length;

    // In genesis/old mode: status 'completed' with node_name counts as 1 verification
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
    console.log(`[BTCPC] Epoch ${epochNumber}: ${sweptCount} job(s) settled (all verifications complete)`);
  }

  // ── Collect all jobs that settled in this epoch ──
  const settledJobs = await InferenceJob.find({ settlement_epoch: epochNumber });

  // ── Also collect synthetic mining proofs ──
  const syntheticProofs = await MiningProof.find({ block_number: epochNumber });

  // Build per-miner work_value totals
  const minerWork = {}; // { minerName: { work_value, model_hashes: Set, models: Set } }

  // From settled inference jobs
  const { verifyModelParams } = require('./workGenerator');
  for (const job of settledJobs) {
    // New model: verifications array
    if (job.verifications && job.verifications.length > 0) {
      for (const v of job.verifications) {
        if (!v.miner || !v.result_hash) continue;
        if (v.model_hash) {
          const registryHash = await getRegistryModelHash(job.model);
          if (registryHash && v.model_hash !== registryHash) {
            console.error(`[BTCPC] REJECTED verification from ${v.miner}: model hash mismatch`);
            continue;
          }
        }
        if (!minerWork[v.miner]) minerWork[v.miner] = { work_value: 0, models: new Set() };
        minerWork[v.miner].work_value += (v.work_value || 0);
        minerWork[v.miner].models.add(job.model);
      }
    } else if (job.node_name) {
      // Genesis/legacy mode: single miner, no verifications array
      const params = await verifyModelParams(job.model || 'qwen3:4b');
      const tokens = job.tokens_generated || 0;
      const wv = tokens * params;
      if (!minerWork[job.node_name]) minerWork[job.node_name] = { work_value: 0, models: new Set() };
      minerWork[job.node_name].work_value += wv;
      minerWork[job.node_name].models.add(job.model);
    }
  }

  // From synthetic proofs (no inference jobs that epoch)
  for (const proof of syntheticProofs) {
    // Verify model hash
    if (proof.model_hash) {
      const registryHash = await getRegistryModelHash(proof.model);
      if (registryHash && proof.model_hash !== registryHash) {
        console.error(`[BTCPC] REJECTED proof from ${proof.miner}: model hash mismatch`);
        continue;
      }
    }

    if (!minerWork[proof.miner]) minerWork[proof.miner] = { work_value: 0, models: new Set() };
    minerWork[proof.miner].work_value += (proof.work_value || 0);
    minerWork[proof.miner].models.add(proof.model);
  }

  const miners = Object.keys(minerWork);
  const totalWorkValue = miners.reduce((sum, m) => sum + minerWork[m].work_value, 0);

  console.log(`[BTCPC] Finalizing epoch ${epochNumber}: ${settledJobs.length} settled jobs, ${syntheticProofs.length} synthetic proofs, ${miners.length} miner(s), total work_value: ${totalWorkValue}`);

  // ── Empty epoch: no inference work done ──
  // Clock nodes still get paid (they kept the chain alive).
  // Mining reward is deferred — doesn't consume emission slot.
  if (miners.length === 0 || totalWorkValue === 0) {
    const rewards = [];

    // Clock nodes still earn their 2% even in empty epochs
    const { getActiveClockNodes } = require('../p2p/protocol');
    const activeClocks = getActiveClockNodes(epochNumber).filter(account => {
      if (!account || account.startsWith('clock-')) return false;
      return nodeRegistry.isRegistered(account);
    });

    // Use a small clock-only reward from the block reward pool
    if (activeClocks.length > 0) {
      const clockOnlyReward = parseFloat((blockReward * 0.02).toFixed(10));
      const clockShare = parseFloat((clockOnlyReward / activeClocks.length).toFixed(10));
      for (const clockNode of activeClocks) {
        await ledger.recordMiningReward(clockNode, clockShare, epochNumber);
        await ledger.updateWalletCache(clockNode, 'BTCPC', clockShare);
        console.log(`[BTCPC]   ${clockNode}: ${clockShare.toFixed(4)} BTCPC (clock — empty epoch)`);
        rewards.push({ node_id: clockNode, amount: clockShare, type: 'clock' });
      }
    }

    epoch.total_work = 0;
    epoch.rewards_distributed = rewards;
    epoch.block_reward = rewards.length > 0 ? rewards.reduce((s, r) => s + r.amount, 0) : 0;
    epoch.ended_at = new Date();
    epoch.status = 'finalized';
    epoch.settled_jobs = 0;
    await epoch.save();
    console.log(`[BTCPC] Epoch ${epochNumber} finalized | EMPTY — no inference work, mining reward deferred${activeClocks.length > 0 ? ', ' + activeClocks.length + ' clock(s) paid' : ''}`);
    return epoch;
  }

  // ── Split rewards: 98% miners, 2% clock nodes ──
  const CLOCK_POOL_PCT = 0.02;
  const minerPoolReward = parseFloat((blockReward * (1 - CLOCK_POOL_PCT)).toFixed(10));
  const clockPoolReward = parseFloat((blockReward * CLOCK_POOL_PCT).toFixed(10));

  const rewards = [];

  // ── Miner rewards (98%) ──
  for (const miner of miners) {
    let share;
    if (totalWorkValue === 0) {
      share = minerPoolReward / miners.length;
    } else {
      share = minerPoolReward * (minerWork[miner].work_value / totalWorkValue);
    }
    share = parseFloat(share.toFixed(10));

    // Record mining reward on permanent ledger — this IS the chain
    await ledger.recordMiningReward(miner, share, epochNumber);

    // Update wallet cache (ledger is source of truth)
    await ledger.updateWalletCache(miner, 'BTCPC', share);

    // Update mining proof with earned reward
    const proof = await MiningProof.findOne({ block_number: epochNumber, miner });
    if (proof) {
      proof.reward_earned = share;
      await proof.save();
    }

    const pct = totalWorkValue > 0 ? ((minerWork[miner].work_value / totalWorkValue) * 100).toFixed(1) : (100 / miners.length).toFixed(1);
    const modelList = [...minerWork[miner].models].join(', ');
    console.log(`[BTCPC]   ${miner}: ${share.toFixed(4)} BTCPC (${pct}%, mining)`);
    rewards.push({ node_id: miner, amount: share, type: 'mining' });
  }

  // ── Clock node rewards (2%) — split among active REGISTERED clock nodes ──
  const { getActiveClockNodes } = require('../p2p/protocol');
  const activeClocks = getActiveClockNodes(epochNumber).filter(account => {
    // Only pay registered nodes — no random clock-XXXX accounts
    if (!account || account.length < 2) return false;
    if (account.startsWith('clock-')) return false; // unregistered auto-generated
    return nodeRegistry.isRegistered(account);
  });

  if (activeClocks.length > 0 && clockPoolReward > 0) {
    const clockShare = parseFloat((clockPoolReward / activeClocks.length).toFixed(10));
    for (const clockNode of activeClocks) {

      await ledger.recordMiningReward(clockNode, clockShare, epochNumber);
      await ledger.updateWalletCache(clockNode, 'BTCPC', clockShare);

      console.log(`[BTCPC]   ${clockNode}: ${clockShare.toFixed(4)} BTCPC (clock)`);
      rewards.push({ node_id: clockNode, amount: clockShare, type: 'clock' });
    }
    console.log(`[BTCPC]   Clock pool: ${clockPoolReward.toFixed(4)} BTCPC → ${activeClocks.length} node(s)`);
  } else if (clockPoolReward > 0) {
    // No active clocks — give the 2% to miners instead
    const extraPerMiner = parseFloat((clockPoolReward / miners.length).toFixed(10));
    for (const miner of miners) {
      await ledger.recordMiningReward(miner, extraPerMiner, epochNumber);
      await ledger.updateWalletCache(miner, 'BTCPC', extraPerMiner);
      // Find the existing reward entry and add to it
      const existing = rewards.find(r => r.node_id === miner);
      if (existing) existing.amount = parseFloat((existing.amount + extraPerMiner).toFixed(10));
    }
    console.log(`[BTCPC]   No active clocks — ${clockPoolReward.toFixed(4)} BTCPC redistributed to miners`);
  }

  // Finalize epoch record
  epoch.consensus_hash = epoch.commitments?.length > 0 ? epoch.commitments[0].state_hash : '0'.repeat(64);
  epoch.total_work = totalWorkValue;
  epoch.rewards_distributed = rewards;
  epoch.block_reward = blockReward;
  epoch.reward_number = rewardNumber;
  epoch.epochs_deferred = epochsDeferred; // how many empty epochs pushed this reward out
  epoch.ended_at = new Date();
  epoch.status = 'finalized';
  epoch.settled_jobs = settledJobs.length;
  await epoch.save();

  console.log(`[BTCPC] Epoch ${epochNumber} finalized | reward #${rewardNumber} (deferred ${epochsDeferred}) | ${miners.length} miner(s) | ${settledJobs.length} settled jobs | ${blockReward.toFixed(4)} BTCPC split`);

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

    // ── EPOCH_END from a clock node — finalize and broadcast block ──
    if (msg.type === 'EPOCH_END') {
      const data = msg.data || {};
      const endedEpoch = data.epoch_number;
      if (!endedEpoch) return;

      console.log(`[BTCPC] Epoch ${endedEpoch} ENDED (from ${data.authority || 'clock'})`);

      // Finalize — any miner with proofs can do this
      setImmediate(async () => {
        try {
          const finalized = await finalizeAndSplitRewards(endedEpoch);
          if (finalized) {
            const bd = finalized._blockData || {};

            const blockMsg = createMessage('EPOCH_FINALIZED', {
              epoch_number: endedEpoch,
              block_reward: finalized.block_reward,
              reward_number: finalized.reward_number,
              epochs_deferred: finalized.epochs_deferred,
              settled_jobs: finalized.settled_jobs || 0,
              rewards: (finalized.rewards_distributed || []).map(r => ({
                miner: r.node_id,
                amount: r.amount
              })),
              total_work: finalized.total_work,
              consensus_hash: finalized.consensus_hash,
              authority: MINER_ACCOUNT,
              ledger: bd.ledger || [],
              header_hex: bd.header_hex || null,
              block_hash: bd.block_hash || null,
              state_root: bd.state_root || null,
              is_finality: bd.is_finality || false
            }, p2p.NODE_ID);
            p2p.broadcast(blockMsg);
            console.log(`[BTCPC] Block ${endedEpoch} broadcast to network`);
          }
        } catch (err) {
          console.error(`[BTCPC] Finalization error for epoch ${endedEpoch}:`, err.message);
        }
      });
    }
  });

  // Fallback: if no clock sends EPOCH_START for 2 epoch durations, mine anyway
  miningInterval = setInterval(async () => {
    if (!running) return;
    const clockEpoch = getCurrentEpochNumber();
    if (clockEpoch > currentEpoch + 1) {
      console.log(`[BTCPC] No clock heard — fallback mining epoch ${clockEpoch}`);
      currentEpoch = clockEpoch;
      try {
        await mineEpoch(currentEpoch);
        await finalizeAndSplitRewards(currentEpoch);
      } catch (err) {
        console.error(`[BTCPC] Fallback epoch ${currentEpoch} error:`, err.message);
      }
    }
  }, EPOCH_DURATION_MS);

  console.log(`[BTCPC] Mining loop active -- next epoch in ${EPOCH_DURATION_MS / 1000}s`);

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
