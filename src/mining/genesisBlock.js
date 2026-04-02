"use strict";

const crypto = require('crypto');
const User = require('../models/User');
const Wallet = require('../models/Wallet');
const Node = require('../models/Node');
const Epoch = require('../models/Epoch');
const { getBlockReward } = require('../services/emissionSchedule');

const path = require('path');
const fs = require('fs');
const GenesisDream = require('../models/GenesisDream');

const GENESIS_MESSAGE = "The Answer to the Ultimate Question of Life, the Universe, and Everything";
const GENESIS_MINER = "shindevlin";
const GENESIS_STATE_HASH = '0'.repeat(64);
const RESERVED_NAMES_PATH = path.resolve(__dirname, '../../data/reserved-names.json');
const WHITEPAPER_PATH = path.resolve(__dirname, '../../docs/BTCPC_WHITEPAPER.md');

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

  // Create the genesis miner account using saved mnemonic if available
  let user = await User.findOne({ username: GENESIS_MINER });
  let wallet;
  if (!user) {
    const { createAccount } = require('../wallet/accountManager');
    const savedMnemonic = process.env.BTCPC_MNEMONIC || null;
    try {
      const account = await createAccount(GENESIS_MINER, savedMnemonic, `${GENESIS_MINER}-genesis`);
      user = await User.findOne({ username: GENESIS_MINER });
      wallet = await Wallet.findOne({ userId: user._id, chain: 'btcpc' });
      console.log(`[BTCPC] Genesis miner account created: ${GENESIS_MINER} (${account.address})`);
      if (savedMnemonic) console.log(`[BTCPC] Using saved mnemonic from BTCPC_MNEMONIC`);
      console.log(`[BTCPC] Wallets: ${JSON.stringify(account.chainWallets)}`);
    } catch (err) {
      console.error(`[BTCPC] Failed to create genesis account: ${err.message}`);
      // Fallback to simple account
      const passwordHash = crypto.createHash('sha256').update(`${GENESIS_MINER}-genesis-${Date.now()}`).digest('hex');
      user = new User({ username: GENESIS_MINER, email: `${GENESIS_MINER}@btcpc.network`, password: passwordHash, isActive: true });
      await user.save();
      wallet = new Wallet({ userId: user._id, chain: 'btcpc', address: GENESIS_MINER, balance: new Map([['BTCPC', 0]]) });
      await wallet.save();
    }
  } else {
    wallet = await Wallet.findOne({ userId: user._id, chain: 'btcpc' });
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
      inference_engine: 'ollama',
      // Cross-chain wallets (env override or defaults)
      hive_account: process.env.BTCPC_HIVE_ACCOUNT || 'shindevlin',
      base_wallet: process.env.BTCPC_BASE_WALLET || '0xD3675710dADF62a7a7bd321b17cA79A1Cd7CF699',
      arbitrum_wallet: process.env.BTCPC_ARBITRUM_WALLET || '0xD3675710dADF62a7a7bd321b17cA79A1Cd7CF699',
      optimism_wallet: process.env.BTCPC_OPTIMISM_WALLET || '0xD3675710dADF62a7a7bd321b17cA79A1Cd7CF699',
      solana_wallet: process.env.BTCPC_SOLANA_WALLET || '7B7pqWCYTgSSysmyMk2iwhCECB8SFphDSrVQJvn9M2bB',
      ton_wallet: process.env.BTCPC_TON_WALLET || 'UQCx_w46JZwxw8_VUoahnnwHWeOZY8_3sW1fmPtBYCPvA7cJ',
      bitcoin_wallet: process.env.BTCPC_BITCOIN_WALLET || 'bc1p2yeza4mezdjmphwqkgcshfmnzjmnthnpr6medmvzcldna0encyjs4x8fep'
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

  // Create Genesis Dream #0 — inscribed with the complete whitepaper
  try {
    const whitepaper = fs.readFileSync(WHITEPAPER_PATH, 'utf8');
    const genesisDream = new GenesisDream({
      block_number: 0,
      original_miner: GENESIS_MINER,
      current_owner: GENESIS_MINER,
      inscription: {
        project: 'btcpc',
        tag: 'Genesis — The chain dreamed itself into existence',
        custom_data: {
          title: 'Bitcoin Proof of Compute — Whitepaper',
          author: 'Shin Devlin',
          version: '0.3',
          message: GENESIS_MESSAGE,
          content: whitepaper
        }
      },
      proof: {
        state_hash: GENESIS_STATE_HASH,
        work_hash: GENESIS_STATE_HASH,
        tokens_computed: 0,
        model: 'genesis'
      }
    });
    await genesisDream.save();
    console.log(`[BTCPC] Genesis Dream #0 inscribed — ${whitepaper.length} chars of whitepaper`);
    console.log(`[BTCPC]   "The chain dreamed itself into existence"`);
  } catch (err) {
    console.error(`[BTCPC] Failed to create genesis dream: ${err.message}`);
  }

  // Reserve top names — owned by shindevlin, sellable later
  let reservedCount = 0;
  try {
    const reservedNames = JSON.parse(fs.readFileSync(RESERVED_NAMES_PATH, 'utf8'));
    for (const name of reservedNames) {
      const existing = await User.findOne({ username: name });
      if (!existing && name !== GENESIS_MINER) {
        const rUser = new User({
          username: name,
          email: `${name}@reserved.btcpc.network`,
          password: crypto.createHash('sha256').update(`reserved-${name}-genesis`).digest('hex'),
          isActive: false  // inactive until claimed/sold
        });
        await rUser.save();
        reservedCount++;
      }
    }
    console.log(`[BTCPC] Reserved ${reservedCount} premium account names for shindevlin`);
  } catch (err) {
    console.log(`[BTCPC] Could not load reserved names: ${err.message}`);
  }

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
