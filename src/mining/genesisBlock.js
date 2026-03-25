"use strict";

const crypto = require('crypto');
const User = require('../models/User');
const Wallet = require('../models/Wallet');
const Node = require('../models/Node');
const Epoch = require('../models/Epoch');
const { getBlockReward } = require('../services/emissionSchedule');

const GENESIS_MESSAGE = "The Answer to the Ultimate Question of Life, the Universe, and Everything";
const GENESIS_MINER = "shindevlin";
const GENESIS_STATE_HASH = '0'.repeat(64);

/**
 * Create the genesis block (epoch 0) and the genesis miner account.
 * Returns the genesis block data if created, or existing data if already present.
 */
async function createGenesisBlock() {
  // Check if genesis epoch already exists
  const existingEpoch = await Epoch.findOne({ epoch_number: 0 });
  if (existingEpoch) {
    console.log('[BTCPC] Genesis block already exists');
    const user = await User.findOne({ username: GENESIS_MINER });
    const wallet = user ? await Wallet.findOne({ userId: user._id }) : null;
    const node = user ? await Node.findOne({ account: user._id }) : null;
    return {
      epoch: existingEpoch,
      user,
      wallet,
      node,
      alreadyExisted: true
    };
  }

  console.log('[BTCPC] Creating genesis block...');
  console.log(`[BTCPC] Genesis message: "${GENESIS_MESSAGE}"`);

  // Create the genesis miner user account
  let user = await User.findOne({ username: GENESIS_MINER });
  if (!user) {
    const passwordHash = crypto.createHash('sha256')
      .update(`${GENESIS_MINER}-genesis-${Date.now()}`)
      .digest('hex');

    user = new User({
      username: GENESIS_MINER,
      email: `${GENESIS_MINER}@btcpc.network`,
      password: passwordHash,
      isActive: true
    });
    await user.save();
    console.log(`[BTCPC] Genesis miner account created: ${GENESIS_MINER}`);
  }

  // Create the genesis miner wallet
  let wallet = await Wallet.findOne({ userId: user._id });
  if (!wallet) {
    wallet = new Wallet({
      userId: user._id,
      chain: 'hive',
      address: GENESIS_MINER,
      balance: new Map([['BTCPC', 0]])
    });
    await wallet.save();
    console.log(`[BTCPC] Genesis wallet created: ${GENESIS_MINER}`);
  }

  // Create the genesis mining node (exempt from stake minimum for genesis)
  let node = await Node.findOne({ account: user._id });
  if (!node) {
    node = new Node({
      account: user._id,
      endpoint: process.env.OLLAMA_URL || 'http://100.122.145.60:11434',
      models: ['qwen3.5:27b'],
      hardware: {
        gpu: 'Genesis Miner',
        vram_gb: 0,
        cpu_cores: 0,
        ram_gb: 0
      },
      stake_amount: 1000,
      status: 'active',
      hive_account: 'thisthatjosh'
    });
    await node.save();
    console.log('[BTCPC] Genesis mining node registered');
  }

  // Create epoch 0 -- the genesis epoch
  const genesisReward = getBlockReward(0);
  const genesisEpoch = new Epoch({
    epoch_number: 0,
    started_at: new Date(),
    block_reward: genesisReward,
    status: 'active',
    consensus_hash: GENESIS_STATE_HASH
  });
  await genesisEpoch.save();

  console.log('[BTCPC] ================================================');
  console.log('[BTCPC]          GENESIS BLOCK CREATED');
  console.log('[BTCPC] ================================================');
  console.log(`[BTCPC] Epoch:        0`);
  console.log(`[BTCPC] Miner:        ${GENESIS_MINER}`);
  console.log(`[BTCPC] Model:        qwen3.5:27b`);
  console.log(`[BTCPC] State Hash:   ${GENESIS_STATE_HASH}`);
  console.log(`[BTCPC] Message:      "${GENESIS_MESSAGE}"`);
  console.log(`[BTCPC] Block Reward: ${genesisReward} BTCPC`);
  console.log('[BTCPC] ================================================');

  return {
    epoch: genesisEpoch,
    user,
    wallet,
    node,
    alreadyExisted: false
  };
}

module.exports = {
  createGenesisBlock,
  GENESIS_MESSAGE,
  GENESIS_MINER,
  GENESIS_STATE_HASH
};
