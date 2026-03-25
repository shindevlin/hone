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
const GenesisDream = require('../models/GenesisDream');

const WORK_ITEMS_PER_EPOCH = parseInt(process.env.BTCPC_WORK_PER_EPOCH) || 3;
const MODEL = process.env.BTCPC_MODEL || 'qwen3.5:27b';

let running = false;
let miningInterval = null;

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

  // Get genesis miner references
  const user = await User.findOne({ username: GENESIS_MINER });
  if (!user) {
    console.error('[BTCPC] Genesis miner account not found. Run genesis first.');
    return;
  }

  const node = await Node.findOne({ account: user._id });
  if (!node) {
    console.error('[BTCPC] Genesis mining node not found. Run genesis first.');
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

  for (let i = 0; i < WORK_ITEMS_PER_EPOCH; i++) {
    try {
      const isGenesisFirstWork = (epochNumber === 0 && i === 0);
      if (isGenesisFirstWork) {
        console.log(`[BTCPC]   GENESIS INFERENCE -- the first dream computed into reality`);
      } else {
        console.log(`[BTCPC]   Work item ${i + 1}/${WORK_ITEMS_PER_EPOCH} -- sending to Ollama (${MODEL})...`);
      }
      const work = await generateWork(MODEL, isGenesisFirstWork ? GENESIS_PROMPT : undefined);

      const proof = new WorkProof({
        epoch_number: epochNumber,
        node_id: GENESIS_MINER,
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

  if (workProofs.length === 0) {
    console.error('[BTCPC] No work completed this epoch. Ollama may be down.');
    return;
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
    const dream = new GenesisDream({
      block_number: epochNumber,
      original_miner: GENESIS_MINER,
      current_owner: GENESIS_MINER,
      inscription: {
        project: metadata.project,
        tag: metadata.tag,
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
    console.log(`[BTCPC]   Dream #${epochNumber}: "${metadata.tag}" [${metadata.project}]`);
  }

  // Step 5: Finalize epoch and distribute rewards
  const finalized = await finalizeEpoch(epochNumber);

  // Step 5: Update node tracking
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
  console.log(`[BTCPC]   Balance:      ${currentBalance.toFixed(8)} BTCPC`);
  console.log(`[BTCPC]   Work items:   ${workProofs.length}`);
  console.log(`[BTCPC]   Tokens:       ${totalTokens}`);
  console.log(`[BTCPC]   Work value:   ${totalWorkValue.toFixed(1)}`);
  console.log(`[BTCPC]   State hash:   ${stateHash.slice(0, 16)}...`);
  console.log(`[BTCPC]   Duration:     ${elapsed}s`);
  console.log('[BTCPC] ------------------------------------------------');
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

  // Create genesis block if needed
  const genesis = await createGenesisBlock();

  running = true;

  // Determine the starting epoch number
  let currentEpoch;
  if (genesis.alreadyExisted) {
    // Calculate current epoch based on genesis time
    const genesisTime = genesis.epoch.started_at.getTime();
    currentEpoch = Math.floor((Date.now() - genesisTime) / EPOCH_DURATION_MS);
    if (currentEpoch < 1) currentEpoch = 1;
  } else {
    // Fresh genesis -- start with epoch 0 mining
    currentEpoch = 0;
  }

  // Mine the first epoch immediately
  try {
    await mineEpoch(currentEpoch);
  } catch (err) {
    console.error(`[BTCPC] Epoch ${currentEpoch} mining error:`, err.message);
  }

  // Schedule subsequent epochs
  miningInterval = setInterval(async () => {
    if (!running) return;

    currentEpoch++;
    try {
      await mineEpoch(currentEpoch);
    } catch (err) {
      console.error(`[BTCPC] Epoch ${currentEpoch} mining error:`, err.message);
    }
  }, EPOCH_DURATION_MS);

  console.log(`[BTCPC] Mining loop active -- next epoch in ${EPOCH_DURATION_MS / 1000}s`);
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
