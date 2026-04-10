"use strict";

/**
 * Ledger Service — permanent on-chain state management.
 *
 * The ledger never prunes. All entries from genesis forward are permanent.
 * Balance is always computed from the ledger, never stored directly.
 *
 * Every write creates a LedgerEntry AND is included in the next
 * EPOCH_FINALIZED broadcast so all nodes have the same ledger.
 */

const LedgerEntry = require('../models/LedgerEntry');
const Wallet = require('../models/Wallet');
const User = require('../models/User');
const Epoch = require('../models/Epoch');

/**
 * Get the current epoch number for ledger entries created via API.
 */
async function getCurrentEpoch() {
  const latest = await Epoch.findOne().sort({ epoch_number: -1 }).lean();
  return latest ? latest.epoch_number : 0;
}

/**
 * Update wallet balance cache after a ledger write.
 * Wallet.balance is a CACHE — the ledger is the source of truth.
 */
async function updateWalletCache(username, token, delta) {
  token = token || 'BTCPC';
  const user = await User.findOne({ username });
  if (!user) return;
  const wallet = await Wallet.findOne({ userId: user._id, chain: 'btcpc' });
  if (!wallet) return;
  const current = wallet.balance.get(token) || 0;
  wallet.balance.set(token, parseFloat((current + delta).toFixed(10)));
  await wallet.save();
}

/**
 * Update wallet balance cache by userId (for controllers that have userId, not username).
 */
async function updateWalletCacheByUserId(userId, token, delta) {
  token = token || 'BTCPC';
  const wallet = await Wallet.findOne({ userId, chain: 'btcpc' });
  if (!wallet) return;
  const current = wallet.balance.get(token) || 0;
  wallet.balance.set(token, parseFloat((current + delta).toFixed(10)));
  await wallet.save();
}

// Pending entries — collected during an epoch, written at EPOCH_END
const pendingEntries = [];

/**
 * Record an account creation on the ledger.
 */
async function recordAccountCreate(username, publicKeys, chainAddresses, epoch) {
  const entry = new LedgerEntry({
    type: 'ACCOUNT_CREATE',
    to: username,
    epoch: epoch || 0,
    account_data: {
      username,
      public_keys: publicKeys || {},
      chain_addresses: chainAddresses || {}
    }
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Record a transfer on the ledger. ALL transfers go through here.
 * Validates via mempool (double-spend protection), writes to ledger,
 * updates wallet caches. Nothing bypasses this.
 */
async function recordTransfer(from, to, amount, token, signature, epoch, memo) {
  if (amount <= 0) throw new Error('Amount must be positive');
  if (!from) throw new Error('Sender required');
  if (!to) throw new Error('Recipient required');
  if (from === to) throw new Error('Cannot transfer to self');

  // Mempool validation — reject double-spends, enforce nonces
  const mempool = require('../p2p/mempool');
  const tx = {
    type: 'TRANSFER',
    from,
    to,
    amount,
    token: token || 'BTCPC',
    nonce: Date.now(),
    timestamp: Date.now(),
    memo: memo || null,
    signature: signature || null
  };
  const mResult = mempool.submit(tx);
  // Allow 'duplicate' — caller may have already submitted to mempool
  if (!mResult.accepted && mResult.reason !== 'duplicate') {
    throw new Error('Transfer rejected: ' + mResult.reason);
  }

  const entry = new LedgerEntry({
    type: 'TRANSFER',
    from,
    to,
    token: token || 'BTCPC',
    amount,
    epoch,
    signature,
    signed_by: 'active',
    memo
  });
  await entry.save();
  pendingEntries.push(entry.toObject());

  // Update wallet caches — balance reflects immediately
  await updateWalletCache(from, token || 'BTCPC', -amount);
  await updateWalletCache(to, token || 'BTCPC', amount);

  return entry;
}

/**
 * Record a mining reward on the ledger.
 */
async function recordMiningReward(miner, amount, epoch) {
  const entry = new LedgerEntry({
    type: 'MINING_REWARD',
    to: miner,
    token: 'BTCPC',
    amount,
    epoch
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Record a faucet distribution.
 */
async function recordFaucet(to, amount, epoch) {
  const entry = new LedgerEntry({
    type: 'FAUCET',
    from: 'btcpc_genesis',
    to,
    token: 'BTCPC',
    amount,
    epoch
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Record a token creation on the ledger.
 */
/**
 * Token creation fee tiers.
 * Standard supply (42M) is cheapest. Custom supply costs more.
 */
const TOKEN_FEE_TIERS = {
  micro:    { maxSupply: 1000000,       fee: 21,  label: 'Micro (up to 1M)' },
  standard: { maxSupply: 42000000,      fee: 42,  label: 'Standard (up to 42M)' },
  mega:     { maxSupply: 1000000000,    fee: 84,  label: 'Mega (up to 1B)' },
  custom:   { maxSupply: Infinity,      fee: 168, label: 'Custom (any amount)' }
};

const NFT_CREATION_FEE = 10; // BTCPC per NFT collection

function getTokenFee(supply) {
  if (supply <= TOKEN_FEE_TIERS.micro.maxSupply) return TOKEN_FEE_TIERS.micro.fee;
  if (supply <= TOKEN_FEE_TIERS.standard.maxSupply) return TOKEN_FEE_TIERS.standard.fee;
  if (supply <= TOKEN_FEE_TIERS.mega.maxSupply) return TOKEN_FEE_TIERS.mega.fee;
  return TOKEN_FEE_TIERS.custom.fee;
}

async function recordTokenCreate(creator, tokenData, fee, epoch) {
  // Calculate fee from tier if not provided
  if (!fee && fee !== 0) {
    fee = getTokenFee(tokenData.supply || 42000000);
  }

  // Fee payment — goes to protocol treasury (genesis operator during genesis phase)
  if (fee > 0) {
    await recordTransfer(creator, 'btcpc_treasury', fee, 'BTCPC', null, epoch, 'Token creation fee: ' + tokenData.symbol);
  }

  const entry = new LedgerEntry({
    type: 'TOKEN_CREATE',
    from: creator,
    token: tokenData.symbol,
    epoch,
    token_data: {
      name: tokenData.name,
      symbol: tokenData.symbol,
      supply: tokenData.supply,
      decimals: tokenData.decimals || 8,
      type: tokenData.type || 'fungible' // 'fungible' or 'nft'
    }
  });
  await entry.save();
  pendingEntries.push(entry.toObject());

  // Mint initial supply to creator (fungible tokens only)
  if (tokenData.type !== 'nft') {
    const mintEntry = new LedgerEntry({
      type: 'FAUCET',
      from: 'btcpc_mint',
      to: creator,
      token: tokenData.symbol,
      amount: tokenData.supply,
      epoch,
      memo: 'Initial supply: ' + tokenData.name
    });
    await mintEntry.save();
    pendingEntries.push(mintEntry.toObject());
  }

  return entry;
}

/**
 * Record staking on the ledger.
 */
async function recordStake(account, amount, purpose, epoch) {
  const entry = new LedgerEntry({
    type: 'STAKE',
    from: account,
    to: 'btcpc_staking_pool',
    token: 'BTCPC',
    amount,
    epoch,
    delegation_data: { purpose }
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Record unstake (withdrawal from staking pool) on the ledger.
 */
async function recordUnstake(account, amount, epoch, memo) {
  const entry = new LedgerEntry({
    type: 'UNSTAKE',
    from: 'btcpc_staking_pool',
    to: account,
    token: 'BTCPC',
    amount,
    epoch,
    memo
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Record delegation on the ledger.
 */
async function recordDelegate(from, to, amount, purpose, epoch) {
  const entry = new LedgerEntry({
    type: 'DELEGATE',
    from,
    to,
    token: 'BTCPC',
    amount,
    epoch,
    delegation_data: { purpose }
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Record undelegation on the ledger.
 */
async function recordUndelegate(from, to, amount, epoch, memo) {
  const entry = new LedgerEntry({
    type: 'UNDELEGATE',
    from,
    to,
    token: 'BTCPC',
    amount,
    epoch,
    memo
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Record escrow lock on the ledger.
 */
async function recordEscrowLock(payer, requestId, amount, epoch) {
  const entry = new LedgerEntry({
    type: 'ESCROW_LOCK',
    from: payer,
    to: 'btcpc_escrow',
    token: 'BTCPC',
    amount,
    epoch,
    memo: 'escrow:' + requestId
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Record escrow release (payment to node) on the ledger.
 */
async function recordEscrowRelease(recipient, requestId, amount, epoch, memo) {
  const entry = new LedgerEntry({
    type: 'ESCROW_RELEASE',
    from: 'btcpc_escrow',
    to: recipient,
    token: 'BTCPC',
    amount,
    epoch,
    memo: memo || 'escrow:' + requestId
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Record escrow refund on the ledger.
 */
async function recordEscrowRefund(payer, requestId, amount, epoch) {
  const entry = new LedgerEntry({
    type: 'ESCROW_REFUND',
    from: 'btcpc_escrow',
    to: payer,
    token: 'BTCPC',
    amount,
    epoch,
    memo: 'escrow:' + requestId
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Register a node on the permanent ledger.
 * Permissioned nodes are approved by the genesis operator.
 * Permissionless nodes need to stake to participate in epoch consensus.
 */
async function recordNodeRegister(username, nodeType, p2pAddress, permissioned, epoch) {
  const entry = new LedgerEntry({
    type: 'NODE_REGISTER',
    from: username,
    epoch: epoch || 0,
    memo: nodeType || 'clock',
    account_data: {
      username,
      node_type: nodeType || 'clock',
      p2p_address: p2pAddress || null,
      permissioned: !!permissioned
    }
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Create an NFT collection on the ledger.
 */
async function recordNFTCreate(creator, collectionData, epoch) {
  const fee = NFT_CREATION_FEE;

  if (fee > 0) {
    await recordTransfer(creator, 'btcpc_treasury', fee, 'BTCPC', null, epoch, 'NFT collection fee: ' + collectionData.symbol);
  }

  const entry = new LedgerEntry({
    type: 'TOKEN_CREATE',
    from: creator,
    token: collectionData.symbol,
    epoch,
    token_data: {
      name: collectionData.name,
      symbol: collectionData.symbol,
      supply: collectionData.maxSupply || 0, // 0 = unlimited minting
      decimals: 0, // NFTs are indivisible
      type: 'nft'
    },
    memo: collectionData.description || null
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Mint an NFT within a collection.
 */
async function recordNFTMint(collection, to, tokenId, metadata, epoch) {
  const entry = new LedgerEntry({
    type: 'FAUCET',
    from: 'btcpc_mint',
    to,
    token: collection,
    amount: 1,
    epoch,
    memo: 'nft:' + tokenId + ':' + JSON.stringify(metadata || {})
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Transfer an NFT. Rejects soulbound tokens.
 */
async function recordNFTTransfer(from, to, collection, tokenId, epoch, signature) {
  // Check if this NFT is soulbound — find the mint entry
  const mintEntry = await LedgerEntry.findOne({
    type: 'FAUCET',
    token: collection,
    memo: { $regex: new RegExp('^nft:' + tokenId + ':') }
  }).lean();

  if (mintEntry) {
    try {
      var meta = JSON.parse(mintEntry.memo.split(':').slice(2).join(':'));
      if (meta.soulbound) {
        throw new Error('This NFT is soulbound and cannot be transferred');
      }
      if (meta.time_locked) {
        var currentEpoch = await getCurrentEpoch();
        if (currentEpoch < meta.unlock_epoch) {
          throw new Error('This NFT is time-locked until epoch ' + meta.unlock_epoch);
        }
      }
    } catch (e) {
      if (e.message.includes('soulbound') || e.message.includes('time-locked')) throw e;
    }
  }

  const entry = new LedgerEntry({
    type: 'TRANSFER',
    from,
    to,
    token: collection,
    amount: 1,
    epoch,
    signature,
    signed_by: 'active',
    memo: 'nft:' + tokenId
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Mint a soulbound NFT — permanently bound to the recipient, can never be transferred.
 * Use cases: achievements, certifications, mining proofs, reputation badges.
 */
async function recordSoulboundMint(collection, to, tokenId, metadata, epoch) {
  metadata = metadata || {};
  metadata.soulbound = true;
  metadata.bound_to = to;
  metadata.bound_at = Date.now();

  return recordNFTMint(collection, to, tokenId, metadata, epoch);
}

/**
 * Mint a revenue-sharing NFT — holders earn a % of inference fees from a model/project.
 * Model creators mint these to earn passive income when their model serves inference.
 *
 * @param {string} collection — NFT collection symbol
 * @param {string} to — recipient (model creator)
 * @param {string} tokenId — unique NFT id
 * @param {object} revenueConfig — { model, revSharePercent, project }
 * @param {object} metadata — additional metadata
 * @param {number} epoch
 */
async function recordRevenueShareMint(collection, to, tokenId, revenueConfig, metadata, epoch) {
  metadata = metadata || {};
  metadata.revenue_share = true;
  metadata.model = revenueConfig.model;
  metadata.rev_share_percent = revenueConfig.revSharePercent || 5; // default 5%
  metadata.project = revenueConfig.project || null;
  metadata.creator = to;
  metadata.created_at = Date.now();

  return recordNFTMint(collection, to, tokenId, metadata, epoch);
}

/**
 * Distribute revenue share to NFT holders for a completed inference job.
 * Called during escrow release — skims a percentage for the model creator.
 *
 * @param {string} model — the model that served the inference
 * @param {number} inferenceRevenue — total BTCPC paid for this inference
 * @param {number} epoch
 * @returns {Array<{to, amount}>} payouts made to NFT holders
 */
async function distributeRevenueShare(model, inferenceRevenue, epoch) {
  // Find all revenue-share NFTs for this model
  const revShareEntries = await LedgerEntry.find({
    type: 'FAUCET',
    from: 'btcpc_mint',
    memo: { $regex: /revenue_share.*true/ }
  }).lean();

  const payouts = [];

  for (const entry of revShareEntries) {
    try {
      var memoData = JSON.parse(entry.memo.split(':').slice(2).join(':'));
      if (!memoData.revenue_share || memoData.model !== model) continue;

      var percent = memoData.rev_share_percent || 5;
      var holder = entry.to;
      var amount = parseFloat((inferenceRevenue * percent / 100).toFixed(10));

      if (amount > 0.000001) {
        await recordMiningReward(holder, amount, epoch);
        await updateWalletCache(holder, 'BTCPC', amount);
        payouts.push({ to: holder, amount: amount, percent: percent });
      }
    } catch (_) {}
  }

  return payouts;
}

/**
 * Mint a time-locked NFT — cannot be transferred until a specific epoch.
 * Use cases: vesting tokens, earned rewards that mature, timed exclusives.
 */
async function recordTimeLockedMint(collection, to, tokenId, unlockEpoch, metadata, epoch) {
  metadata = metadata || {};
  metadata.time_locked = true;
  metadata.unlock_epoch = unlockEpoch;

  return recordNFTMint(collection, to, tokenId, metadata, epoch);
}

/**
 * Mint a rental NFT — owner retains ownership, renter gets temporary access.
 * Use cases: model access passes, inference credit bundles, time-limited permissions.
 */
async function recordRentalMint(collection, to, tokenId, rentalConfig, metadata, epoch) {
  metadata = metadata || {};
  metadata.rental = true;
  metadata.owner = to;
  metadata.max_rental_epochs = rentalConfig.maxEpochs || 8640; // default 30 days
  metadata.rental_price = rentalConfig.price || 0;

  return recordNFTMint(collection, to, tokenId, metadata, epoch);
}

/**
 * Rent an NFT — creates a temporary access record.
 */
async function recordNFTRental(collection, tokenId, renter, ownerUsername, durationEpochs, price, epoch) {
  // Pay the owner
  if (price > 0) {
    await recordTransfer(renter, ownerUsername, price, 'BTCPC', null, epoch, 'NFT rental: ' + collection + ':' + tokenId);
  }

  const entry = new LedgerEntry({
    type: 'TRANSFER',
    from: renter,
    to: renter, // renter is both from and to — it's an access grant, not a transfer
    token: collection,
    amount: 0,
    epoch,
    memo: 'nft-rental:' + tokenId + ':' + durationEpochs + ':' + ownerUsername
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Mint a composable NFT — can contain other NFTs/tokens as a bundle.
 * Use cases: portfolio NFTs, loot boxes, curated collections.
 */
async function recordComposableMint(collection, to, tokenId, contents, metadata, epoch) {
  metadata = metadata || {};
  metadata.composable = true;
  metadata.contents = contents || []; // [{ collection, tokenId }, ...]

  return recordNFTMint(collection, to, tokenId, metadata, epoch);
}

/**
 * Add an item to a composable NFT.
 */
async function recordComposableAdd(parentCollection, parentTokenId, childCollection, childTokenId, owner, epoch) {
  const entry = new LedgerEntry({
    type: 'TRANSFER',
    from: owner,
    to: 'composable:' + parentCollection + ':' + parentTokenId,
    token: childCollection,
    amount: 1,
    epoch,
    memo: 'compose:' + childTokenId
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Mint an evolving NFT — metadata changes based on on-chain activity.
 * Use cases: miner reputation badges, project usage trackers, achievement progression.
 *
 * The evolution rules define how the NFT changes:
 *   { metric: "total_compute", thresholds: [1000, 10000, 100000], labels: ["Bronze", "Silver", "Gold"] }
 */
async function recordEvolvingMint(collection, to, tokenId, evolutionRules, metadata, epoch) {
  metadata = metadata || {};
  metadata.evolving = true;
  metadata.evolution_rules = evolutionRules;
  metadata.current_level = 0;
  metadata.metric_value = 0;

  return recordNFTMint(collection, to, tokenId, metadata, epoch);
}

/**
 * Update an evolving NFT's metric and check for level-up.
 */
async function recordEvolvingUpdate(collection, tokenId, owner, newMetricValue, epoch) {
  const entry = new LedgerEntry({
    type: 'TRANSFER',
    from: owner,
    to: owner,
    token: collection,
    amount: 0,
    epoch,
    memo: 'evolve:' + tokenId + ':' + newMetricValue
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Record a heartbeat — proves the account holder is alive.
 * Zero cost. Resets the dormancy clock. One tap every 5 years.
 */
async function recordHeartbeat(username, epoch) {
  const entry = new LedgerEntry({
    type: 'HEARTBEAT',
    from: username,
    to: username,
    token: 'BTCPC',
    amount: 0,
    epoch: epoch || 0,
    memo: 'alive'
  });
  await entry.save();
  pendingEntries.push(entry.toObject());
  return entry;
}

/**
 * Compute balance for an account by replaying the ledger.
 * This is the source of truth — not the Wallet model.
 *
 * @param {string} username
 * @param {string} [token='BTCPC']
 * @returns {Promise<number>}
 */
async function getBalance(username, token) {
  token = token || 'BTCPC';

  const incoming = await LedgerEntry.aggregate([
    { $match: { to: username, token } },
    { $group: { _id: null, total: { $sum: '$amount' } } }
  ]);

  const outgoing = await LedgerEntry.aggregate([
    { $match: { from: username, token } },
    { $group: { _id: null, total: { $sum: '$amount' } } }
  ]);

  const inTotal = incoming.length > 0 ? incoming[0].total : 0;
  const outTotal = outgoing.length > 0 ? outgoing[0].total : 0;

  const mongoBalance = parseFloat((inTotal - outTotal).toFixed(10));

  // Phase B: shadow-read from stateStore and log any divergence.
  // stateStore is the future source of truth — in Phase C we'll delete the Mongo path.
  try {
    const stateStore = require('../chain/stateStore');
    const storeBalance = stateStore.getBalance(username, token);
    if (Math.abs(storeBalance - mongoBalance) > 0.0000001) {
      console.log('[BTCPC STATE DIVERGENCE] user=' + username + ' token=' + token +
        ' mongo=' + mongoBalance + ' store=' + storeBalance);
    }
  } catch (_) { /* stateStore not yet populated on this process — skip */ }

  return mongoBalance;
}

/**
 * Get all tokens held by an account.
 */
async function getTokenBalances(username) {
  const tokens = await LedgerEntry.distinct('token', {
    $or: [{ from: username }, { to: username }]
  });

  const balances = {};
  for (const token of tokens) {
    balances[token] = await getBalance(username, token);
  }
  return balances;
}

/**
 * Get the full account record from the ledger (public info only).
 */
async function getAccountRecord(username) {
  const entry = await LedgerEntry.findOne({
    type: 'ACCOUNT_CREATE',
    'account_data.username': username
  }).lean();
  return entry ? entry.account_data : null;
}

/**
 * Get all accounts registered on the ledger.
 */
async function getAllAccounts() {
  return LedgerEntry.find({ type: 'ACCOUNT_CREATE' })
    .select('account_data timestamp epoch')
    .lean();
}

/**
 * Flush pending entries — returns them for inclusion in EPOCH_FINALIZED.
 */
function flushPendingEntries() {
  const entries = [...pendingEntries];
  pendingEntries.length = 0;
  return entries;
}

/**
 * Apply ledger entries received from EPOCH_FINALIZED broadcast.
 * Used by follower nodes to sync their ledger.
 */
async function applyRemoteEntries(entries) {
  let applied = 0;
  for (const entry of entries) {
    // Check for duplicate (by type + from + to + amount + epoch + timestamp)
    const exists = await LedgerEntry.findOne({
      type: entry.type,
      from: entry.from,
      to: entry.to,
      amount: entry.amount,
      epoch: entry.epoch,
      token: entry.token
    });
    if (exists) continue;

    // Strip MongoDB _id to avoid duplicate key errors from remote entries
    const clean = { ...entry };
    delete clean._id;
    delete clean.__v;
    await LedgerEntry.create(clean);
    applied++;
  }
  return applied;
}

module.exports = {
  recordAccountCreate,
  recordTransfer,
  recordMiningReward,
  recordFaucet,
  recordTokenCreate,
  recordStake,
  recordUnstake,
  recordDelegate,
  recordUndelegate,
  recordEscrowLock,
  recordEscrowRelease,
  recordEscrowRefund,
  recordNodeRegister,
  recordHeartbeat,
  recordNFTCreate,
  recordNFTMint,
  recordNFTTransfer,
  recordSoulboundMint,
  recordRevenueShareMint,
  distributeRevenueShare,
  recordTimeLockedMint,
  recordRentalMint,
  recordNFTRental,
  recordComposableMint,
  recordComposableAdd,
  recordEvolvingMint,
  recordEvolvingUpdate,
  getTokenFee,
  TOKEN_FEE_TIERS,
  NFT_CREATION_FEE,
  getCurrentEpoch,
  updateWalletCache,
  updateWalletCacheByUserId,
  getBalance,
  getTokenBalances,
  getAccountRecord,
  getAllAccounts,
  flushPendingEntries,
  applyRemoteEntries
};
