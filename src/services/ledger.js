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
async function recordTokenCreate(creator, tokenData, fee, epoch) {
  // Fee payment to genesis operator
  if (fee > 0) {
    await recordTransfer(creator, 'shindevlin', fee, 'BTCPC', null, epoch, 'Token creation fee');
  }

  const entry = new LedgerEntry({
    type: 'TOKEN_CREATE',
    from: creator,
    token: tokenData.symbol,
    epoch,
    token_data: tokenData
  });
  await entry.save();
  pendingEntries.push(entry.toObject());

  // Mint initial supply to creator
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

  return parseFloat((inTotal - outTotal).toFixed(10));
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
