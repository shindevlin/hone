"use strict";

const crypto = require('crypto');
const User = require('../models/User');
const { getBlockReward } = require('../services/emissionSchedule');

const path = require('path');
const fs = require('fs');

const Block = require('../chain/block');
const blockStore = require('../chain/blockStore');
const stateManager = require('../chain/stateManager');
const stateStore = require('../chain/stateStore');
const nodeRegistry = require('../chain/nodeRegistry');
const { getReservedNamesForShin } = require('../services/reservedNames');

const GENESIS_MESSAGE = "The Answer to the Ultimate Question of Life, the Universe, and Everything";
const GENESIS_MINER = "shindevlin";
const GENESIS_STATE_HASH = '0'.repeat(64);
const WHITEPAPER_PATH = path.resolve(__dirname, '../../docs/BTCPC_WHITEPAPER.md');

/**
 * Create the genesis block (epoch 0) and the genesis miner account.
 * Returns the genesis block data if created, or existing data if already present.
 */
async function createGenesisBlock() {
  // Phase E: Epoch/Wallet/Node models deleted — check block files + stateStore
  const stateStore = require('../chain/stateStore');
  const existingEpoch = stateStore.getEpoch(0);

  // Also check finality snapshots — genesis (block-00000000.bin) may have been
  // compacted into a finality snapshot, so the individual file is gone.
  const fs = require('fs');
  const path = require('path');
  const blocksDir = path.resolve(process.cwd(), 'data', 'blocks');
  var hasFinalitySnap = false;
  try {
    if (fs.existsSync(blocksDir)) {
      hasFinalitySnap = fs.readdirSync(blocksDir).some(f => f.startsWith('finality-'));
    }
  } catch (_) {}

  if (existingEpoch || blockStore.hasBlock(0) || hasFinalitySnap) {
    console.log('[BTCPC] Genesis block already exists');
    const epochData = existingEpoch || {
      epoch_number: 0,
      started_at: new Date(0),
      block_reward: getBlockReward(0),
      status: 'finalized'
    };
    return {
      epoch: epochData,
      user: null,
      wallet: null,
      node: null,
      alreadyExisted: true
    };
  }

  console.log('[BTCPC] Creating genesis block...');
  console.log(`[BTCPC] Genesis message: "${GENESIS_MESSAGE}"`);

  // Create the genesis miner account — stateStore is source of truth; Mongo is optional
  const mongoEnabled = process.env.BTCPC_MONGO_MODE === 'enabled';
  let user = null;
  if (mongoEnabled) {
    user = await User.findOne({ username: GENESIS_MINER });
  }
  if (!user && !stateStore.hasAccount(GENESIS_MINER)) {
    const { createAccount } = require('../wallet/accountManager');
    const savedMnemonic = process.env.BTCPC_MNEMONIC || null;
    try {
      const account = await createAccount(GENESIS_MINER, savedMnemonic, `${GENESIS_MINER}-genesis`);
      if (mongoEnabled) user = await User.findOne({ username: GENESIS_MINER });
      console.log(`[BTCPC] Genesis miner account created: ${GENESIS_MINER} (${account.address})`);
      if (savedMnemonic) console.log(`[BTCPC] Using saved mnemonic from BTCPC_MNEMONIC`);
      console.log(`[BTCPC] Wallets: ${JSON.stringify(account.chainWallets)}`);

      // Announce genesis miner on the permanent ledger
      const ledger = require('../services/ledger');
      await ledger.recordAccountCreate(GENESIS_MINER, account.publicKeys, account.chainWallets, 0);
      console.log(`[BTCPC] Genesis miner announced to permanent ledger`);
    } catch (err) {
      console.error(`[BTCPC] Failed to create genesis account: ${err.message}`);
      if (mongoEnabled) {
        // Fallback: create minimal User doc only when Mongo is available
        try {
          const passwordHash = crypto.createHash('sha256').update(`${GENESIS_MINER}-genesis-${Date.now()}`).digest('hex');
          user = new User({ username: GENESIS_MINER, email: `${GENESIS_MINER}@btcpc.network`, password: passwordHash, isActive: true });
          await user.save();
        } catch (_) {}
      }
    }
  }

  // Register the genesis mining node in nodeRegistry (in-memory, no Mongo)
  if (!nodeRegistry.isRegistered(GENESIS_MINER)) {
    nodeRegistry.registerNode(
      GENESIS_MINER,
      'miner',
      1000,
      process.env.OLLAMA_URL || 'http://100.122.145.60:11434',
      0,
      true  // permissioned genesis node
    );
    console.log('[BTCPC] Genesis mining node registered in nodeRegistry');
  }

  // Create epoch 0 in stateStore — no Mongo
  const genesisReward = getBlockReward(0);
  const genesisStartedAt = new Date();
  stateStore.setEpoch(0, {
    epoch_number: 0,
    started_at: genesisStartedAt,
    block_reward: genesisReward,
    status: 'active',
    consensus_hash: GENESIS_STATE_HASH
  });

  // Genesis Dream #0 — inscribed with the complete whitepaper
  // Stored as a ledger entry in the genesis block payload (no GenesisDream Mongo model)
  let genesisDreamData = null;
  try {
    const whitepaper = fs.readFileSync(WHITEPAPER_PATH, 'utf8');
    genesisDreamData = {
      block_number: 0,
      original_miner: GENESIS_MINER,
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
    };
    console.log(`[BTCPC] Genesis Dream #0 inscribed — ${whitepaper.length} chars of whitepaper`);
    console.log(`[BTCPC]   "The chain dreamed itself into existence"`);
  } catch (err) {
    console.error(`[BTCPC] Failed to read whitepaper for genesis dream: ${err.message}`);
  }

  // Reserve premium names as nested zero-balance wallets under Shin.
  // They can later be promoted to native top-level accounts via WALLET_UNNEST
  // using the recipient's public keys.
  let reservedCount = 0;
  try {
    const reservedNames = getReservedNamesForShin();
    const ledger = require('../services/ledger');
    for (const name of reservedNames) {
      const nestedAccount = GENESIS_MINER + '/' + name;
      if (!stateStore.hasAccount(nestedAccount)) {
        await ledger.recordWalletCreateChild(GENESIS_MINER, name, null, 0);
        reservedCount++;
      }

      if (mongoEnabled) {
        const existing = await User.findOne({ username: name });
        if (!existing && name !== GENESIS_MINER) {
          const rUser = new User({
            username: name,
            email: `${name}@reserved.btcpc.network`,
            password: crypto.createHash('sha256').update(`reserved-${name}-genesis`).digest('hex'),
            isActive: false  // inactive until claimed/sold
          });
          await rUser.save();
        }
      }
    }
    console.log(`[BTCPC] Reserved ${reservedCount} premium account names for shindevlin`);
  } catch (err) {
    console.log(`[BTCPC] Could not load reserved names: ${err.message}`);
  }

  // ── Write genesis block to disk — the source of truth ──
  try {
    blockStore.ensureBlockDir();

    // Flush pending ledger entries (from recordAccountCreate above) as genesis block payload
    const ledger = require('../services/ledger');
    const pendingEntries = ledger.flushPendingEntries ? ledger.flushPendingEntries() : [];
    const cleanEntries = pendingEntries.filter(e => (e.epoch === 0 || e.epoch === undefined));

    // If genesis dream was created, add it as a special entry
    if (genesisDreamData) {
      cleanEntries.push({
        type: 'GENESIS_DREAM',
        from: GENESIS_MINER,
        to: GENESIS_MINER,
        token: 'BTCPC',
        amount: 0,
        epoch: 0,
        signed_by: GENESIS_MINER,
        memo: 'Genesis Dream #0',
        timestamp: genesisStartedAt.toISOString(),
        account_data: genesisDreamData
      });
    }

    // Apply genesis entries to SMT
    stateManager.applyLedgerEntries(cleanEntries);
    const stateRoot = stateManager.getStateRoot();

    // Compute Merkle roots
    const txHashes = cleanEntries.map(e => blockStore.hashLedgerEntry(e));
    const txMerkleRoot = Block.computeMerkleRoot(txHashes);

    const genesisBlock = new Block({
      version: 1,
      epoch_number: 0,
      previous_block_hash: '0'.repeat(64),
      merkle_root_transactions: txMerkleRoot,
      merkle_root_compute_proofs: '0'.repeat(64),
      state_root: stateRoot,
      timestamp: genesisStartedAt.getTime(),
      difficulty: 1,
      miner_id: GENESIS_MINER
    });

    const payload = {
      ledger_entries: cleanEntries,
      rewards: [],
      compute_proofs: [],
      mining_proofs: []
    };

    blockStore.writeBlock(genesisBlock, payload);
    const blockHash = genesisBlock.computeHash();
    console.log(`[BTCPC] Genesis block written to disk: ${blockHash.slice(0, 16)}...`);
    console.log(`[BTCPC]   State root: ${stateRoot.slice(0, 16)}...`);
    console.log(`[BTCPC]   Tx Merkle root: ${txMerkleRoot.slice(0, 16)}...`);
  } catch (err) {
    console.error(`[BTCPC] Failed to write genesis block to disk: ${err.message}`);
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

  const genesisEpochData = stateStore.getEpoch(0) || {
    epoch_number: 0,
    started_at: genesisStartedAt,
    block_reward: genesisReward,
    status: 'active',
    consensus_hash: GENESIS_STATE_HASH
  };

  return {
    epoch: genesisEpochData,
    user,
    wallet: null,
    node: null,
    alreadyExisted: false
  };
}

module.exports = {
  createGenesisBlock,
  GENESIS_MESSAGE,
  GENESIS_MINER,
  GENESIS_STATE_HASH
};
