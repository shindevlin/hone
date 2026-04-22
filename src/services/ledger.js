"use strict";

/**
 * Ledger Service — permanent on-chain state management.
 *
 * Phase D: Mongo is no longer used for chain state writes. Every recordX
 * function builds a plain ledger entry object, applies it to the in-memory
 * stateStore (which updates balances/accounts/tokens/etc.), and pushes it
 * into pendingEntries. The miner flushes pendingEntries into the next block's
 * payload at finalization. Block files on disk are the canonical source of
 * truth — stateStore is the working cache, rebuilt by replay on startup.
 *
 * Read paths (getBalance, getTokenBalances, getAccountRecord, getAllAccounts,
 * getCurrentEpoch) all query stateStore. O(1) lookups, no aggregations.
 */

const stateStore = require('../chain/stateStore');
const epochBandwidth = require('./epochBandwidth');
const fs = require('fs');
const path = require('path');

// Pending entries — collected during an epoch, written into the next block.
//
// IMPORTANT: Entries originate in MULTIPLE processes on the same machine —
// the API server (src/index.js) creates most of them (account creation,
// transfers, stakes, escrow, commerce, etc.) and the miner (bin/btcpc-mine)
// creates MINING_REWARD entries during epoch finalization.
//
// Each process has its own in-memory `pendingEntries` array. To get entries
// from the API server into the miner's block payloads, we ALSO append every
// entry to a shared on-disk queue at data/pending-entries.jsonl. When the
// miner flushes, it reads and clears the shared file, unioning those entries
// with its own in-memory pending list.
//
// This replaces the broken pre-v2.13.1 behavior where API server entries
// accumulated forever in a private array and never reached any block.
const pendingEntries = [];

// Data directory is overridable via BTCPC_DATA_DIR (used in tests to isolate
// the shared pending-entries file between parallel jest workers, and in Docker
// to point at /app/data inside the container).
function _dataDir() {
  return process.env.BTCPC_DATA_DIR
    ? path.resolve(process.env.BTCPC_DATA_DIR)
    : path.resolve(__dirname, '..', '..', 'data');
}
function _pendingFile() {
  return path.join(_dataDir(), 'pending-entries.jsonl');
}

function _ensurePendingDir() {
  try {
    const d = _dataDir();
    if (!fs.existsSync(d)) fs.mkdirSync(d, { recursive: true });
  } catch (_) {}
}

// ─────────────────────────────────────────────────────────────────
// Mempool gossip — broadcast ledger entries to peers (v2.13.3)
// ─────────────────────────────────────────────────────────────────

// Track entries we've already gossiped so we don't double-broadcast.
// Populated by _gossipEntry (originator path) and appendForeignEntry
// (receiver path) — the latter prevents re-gossiping entries that
// arrived from a peer.
var gossipedHashes = new Set();
var GOSSIP_HASH_CAP = 50000;

/**
 * Broadcast a freshly-persisted ledger entry to all P2P peers.
 * Uses a lazy require to break the ledger ↔ p2p circular dependency.
 * All errors are swallowed — the entry is already in the local queue.
 */
var BLOCK_ONLY_GOSSIP_SKIP = new Set([
  'MINING_REWARD', 'CLOCK_REWARD', 'STORAGE_REWARD',
  'SERVICE_REWARD', 'IOT_REWARD', 'FAUCET',
]);

function _gossipEntry(entry) {
  try {
    if (BLOCK_ONLY_GOSSIP_SKIP.has(entry.type)) return;
    var key = JSON.stringify(entry);
    if (gossipedHashes.has(key)) return;
    gossipedHashes.add(key);
    // Bounded cache: evict oldest 1000 entries when cap is reached
    if (gossipedHashes.size > GOSSIP_HASH_CAP) {
      var iter = gossipedHashes.values();
      for (var i = 0; i < 1000; i++) gossipedHashes.delete(iter.next().value);
    }
    var p2p = require('../p2p/network');
    var protocol = require('../p2p/protocol');
    var msg = protocol.createMessage(
      'MEMPOOL_ENTRY',
      { entry: entry },
      p2p.NODE_ID || 'ledger'
    );
    if (typeof p2p.broadcast === 'function') {
      p2p.broadcast(msg);
    }
  } catch (_) {
    // silent: entry is still in the local queue, gossip is best-effort
  }
}

function _appendPendingToDisk(entry) {
  try {
    _ensurePendingDir();
    // O_APPEND on POSIX makes writes up to PIPE_BUF (4096 bytes) atomic
    // across processes. Ledger entries are well under that.
    fs.appendFileSync(_pendingFile(), JSON.stringify(entry) + '\n');
  } catch (err) {
    // Non-fatal: entry is still in stateStore + in-memory pendingEntries.
    // The miner will just miss it in the next block. Log once per minute to
    // avoid flooding.
    if (!_appendPendingToDisk._lastWarn || Date.now() - _appendPendingToDisk._lastWarn > 60000) {
      console.error('[BTCPC ledger] pending-entries.jsonl append failed: ' + err.message);
      _appendPendingToDisk._lastWarn = Date.now();
    }
  }
}

// If a process crashed between renameSync and unlinkSync it leaves a
// .draining-<pid>-<ts> file on disk. Recover those entries so they're
// not silently lost.
function _recoverStaleDrainFiles() {
  var entries = [];
  try {
    var dir = _dataDir();
    if (!fs.existsSync(dir)) return entries;
    var files = fs.readdirSync(dir).filter(function (f) {
      return f.startsWith('pending-entries.jsonl.draining-');
    });
    for (var i = 0; i < files.length; i++) {
      var p = path.join(dir, files[i]);
      try {
        var raw = fs.readFileSync(p, 'utf8');
        fs.unlinkSync(p);
        raw.split('\n').forEach(function (line) {
          line = line.trim();
          if (!line) return;
          try { entries.push(JSON.parse(line)); } catch (_) {}
        });
      } catch (e) {
        console.warn('[BTCPC ledger] Could not recover stale drain file ' + files[i] + ': ' + e.message);
      }
    }
  } catch (e) {
    console.warn('[BTCPC ledger] Stale drain file scan failed: ' + e.message);
  }
  return entries;
}

function _readAndClearPendingFile() {
  // Recover entries from .draining-* files left by any previously crashed process.
  var entries = _recoverStaleDrainFiles();
  try {
    var pendingFile = _pendingFile();
    if (!fs.existsSync(pendingFile)) return entries;

    // Rename-then-read pattern: any concurrent _appendPendingToDisk will
    // create a fresh file while we drain the old one. No writes lost.
    var draining = pendingFile + '.draining-' + process.pid + '-' + Date.now();
    try {
      fs.renameSync(pendingFile, draining);
    } catch (err) {
      // File may have vanished between exists and rename (another miner?).
      if (err.code === 'ENOENT') return entries;
      throw err;
    }

    var raw = fs.readFileSync(draining, 'utf8');
    fs.unlinkSync(draining);

    var lines = raw.split('\n');
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i].trim();
      if (!line) continue;
      try {
        entries.push(JSON.parse(line));
      } catch (_) {
        // Skip corrupt lines silently — don't let one bad entry block a block.
      }
    }
  } catch (err) {
    console.error('[BTCPC ledger] pending-entries.jsonl drain failed: ' + err.message);
  }
  return entries;
}

// Build a plain ledger entry object with default timestamp.
// Replaces `new LedgerEntry({...})` — no Mongoose, no save, no _id.
function _entry(data) {
  const e = Object.assign({
    type: null,
    from: null,
    to: null,
    token: 'BTCPC',
    amount: 0,
    epoch: 0,
    signature: null,
    signed_by: null,
    memo: null,
    timestamp: Date.now(),
  }, data || {});
  return e;
}

// Persist a ledger entry without touching Mongo.
// Applies to stateStore (updates balances/accounts/tokens/stakes/escrows/etc.),
// pushes into the in-process pendingEntries array, appends to the shared
// on-disk queue so entries originating in the API server eventually land in
// blocks written by the miner process, AND gossips to P2P peers so that the
// current network broadcaster (which may be on another machine) sees the entry.
//
// Epoch Bandwidth is consumed here for user-originated entries. System reward
// entries (MINING_REWARD, CLOCK_REWARD, etc.) have cost 0 and always pass.
function _persist(entry) {
  const payer = entry.from || null;
  const ebResult = epochBandwidth.checkAndDeduct(payer, entry.type);
  if (!ebResult.allowed) {
    const err = new Error(
      'Insufficient epoch bandwidth — ' +
      Math.floor(ebResult.eb_remaining) + ' EB available, ' +
      ebResult.cost + ' required. Stake more BTCPC or wait for next epoch.'
    );
    err.code = 'INSUFFICIENT_EB';
    err.eb_remaining = ebResult.eb_remaining;
    err.eb_cost = ebResult.cost;
    err.max_eb = ebResult.max_eb;
    throw err;
  }
  stateStore.applyEntry(entry);
  pendingEntries.push(entry);
  _appendPendingToDisk(entry);
  _gossipEntry(entry);
  return entry;
}

/**
 * Get the current epoch number. Read from stateStore chain height.
 */
async function getCurrentEpoch() {
  const h = stateStore.getChainHeight();
  return h >= 0 ? h : 0;
}

/**
 * Phase D: stateStore.applyEntry() handles balance updates via the entry
 * dispatcher (TRANSFER, MINING_REWARD, STAKE, etc.), so there's no separate
 * wallet cache to update. Kept as a no-op for backward compatibility with
 * external callers (p2p/protocol.js, mining/miner.js, services/escrow.js)
 * that still invoke it — Phase E will clean up those call sites.
 */
async function updateWalletCache(_username, _token, _delta) {
  // no-op: stateStore is the cache, and applyEntry already updates balances
}

async function updateWalletCacheByUserId(_userId, _token, _delta) {
  // no-op: see updateWalletCache
}

/**
 * Record an account creation on the ledger.
 */
async function recordAccountCreate(username, publicKeys, chainAddresses, epoch) {
  const entry = _entry({
    type: 'ACCOUNT_CREATE',
    to: username,
    epoch: epoch || 0,
    account_data: {
      username,
      public_keys: publicKeys || {},
      chain_addresses: chainAddresses || {},
    },
  });
  return _persist(entry);
}

/**
 * Record creation of a child wallet under a parent.
 * Authorization: parent's posting key must sign the request.
 * Child account name will be "parent/childName".
 *
 * @param {string} parent        — parent account name
 * @param {string} childName     — short name (no "/" allowed)
 * @param {string} postingKeySig — signature from parent's posting key
 * @param {number} epoch         — current epoch
 */
async function recordWalletCreateChild(parent, childName, postingKeySig, epoch) {
  if (!parent) throw new Error('Parent account required');
  if (!childName) throw new Error('Child name required');
  if (String(childName).indexOf('/') !== -1) throw new Error('Child name must not contain "/"');

  const childAccount = parent + '/' + childName;

  // Validate parent exists
  if (!stateStore.hasAccount(parent)) throw new Error('Parent account not found: ' + parent);
  // Reject if child already exists
  if (stateStore.hasAccount(childAccount)) throw new Error('Child account already exists: ' + childAccount);

  const entry = _entry({
    type: 'WALLET_CREATE_CHILD',
    from: parent,
    to: childAccount,
    epoch: epoch || 0,
    signature: postingKeySig || null,
    signed_by: 'posting',
    wallet_data: {
      parent,
      child_name: childName,
    },
  });
  return _persist(entry);
}

/**
 * Promote a nested wallet such as "shindevlin/bob" into native account "bob".
 * The native account is created with the recipient's public keys; private keys
 * and mnemonic stay with the recipient.
 */
async function recordWalletUnnest(parent, nestedName, nativeName, newPublicKeys, chainAddresses, ownerKeySig, epoch) {
  if (!parent) throw new Error('Parent account required');
  if (!nestedName) throw new Error('Nested wallet name required');
  if (!nativeName) throw new Error('Native account name required');
  if (String(nestedName).indexOf('/') !== -1) throw new Error('Nested wallet name must not contain "/"');
  if (String(nativeName).indexOf('/') !== -1) throw new Error('Native account name must not contain "/"');
  if (!/^[a-z0-9][a-z0-9._-]{0,19}$/.test(String(nativeName))) throw new Error('Invalid native account name');
  if (!newPublicKeys || typeof newPublicKeys !== 'object') throw new Error('Recipient public keys required');

  const nestedWallet = parent + '/' + nestedName;
  if (!stateStore.hasAccount(parent)) throw new Error('Parent account not found: ' + parent);
  if (!stateStore.hasAccount(nestedWallet)) throw new Error('Nested wallet not found: ' + nestedWallet);
  if (stateStore.getParent(nestedWallet) !== parent) throw new Error('Nested wallet parent mismatch');
  if (stateStore.hasAccount(nativeName)) throw new Error('Native account already exists: ' + nativeName);

  return _persist(_entry({
    type: 'WALLET_UNNEST',
    from: parent,
    to: nativeName,
    epoch: epoch || 0,
    signature: ownerKeySig || null,
    signed_by: 'owner',
    wallet_data: {
      parent,
      nested_name: nestedName,
      nested_wallet: nestedWallet,
      native_name: nativeName,
      public_keys: newPublicKeys,
      chain_addresses: chainAddresses || {},
    },
  }));
}

/**
 * Assign a reserved nested wallet name as an alias to an existing account.
 * Example: shindevlin/joshua -> josh, so transfers to either name resolve to josh.
 */
async function recordAccountAliasAssign(parent, nestedName, targetAccount, ownerKeySig, epoch) {
  if (!parent) throw new Error('Parent account required');
  if (!nestedName) throw new Error('Nested wallet name required');
  if (!targetAccount) throw new Error('Target account required');
  if (String(nestedName).indexOf('/') !== -1) throw new Error('Nested wallet name must not contain "/"');

  const nestedWallet = parent + '/' + nestedName;
  if (!stateStore.hasAccount(parent)) throw new Error('Parent account not found: ' + parent);
  if (!stateStore.hasAccount(nestedWallet)) throw new Error('Nested wallet not found: ' + nestedWallet);
  if (stateStore.getParent(nestedWallet) !== parent) throw new Error('Nested wallet parent mismatch');
  if (!stateStore.hasAccount(targetAccount)) throw new Error('Target account not found: ' + targetAccount);
  if (stateStore.hasAccount(nestedName)) throw new Error('Alias is already a native account: ' + nestedName);
  if (stateStore.getAliasTarget && stateStore.getAliasTarget(nestedName)) throw new Error('Alias already assigned: ' + nestedName);

  return _persist(_entry({
    type: 'ACCOUNT_ALIAS_ASSIGN',
    from: parent,
    to: nestedName,
    epoch: epoch || 0,
    signature: ownerKeySig || null,
    signed_by: 'owner',
    alias_data: {
      parent,
      nested_name: nestedName,
      nested_wallet: nestedWallet,
      alias: nestedName,
      target_account: targetAccount,
      transferable: true,
    },
  }));
}

async function recordAccountAliasTransfer(alias, fromAccount, toAccount, ownerKeySig, epoch) {
  if (!alias) throw new Error('Alias required');
  if (!fromAccount) throw new Error('From account required');
  if (!toAccount) throw new Error('To account required');
  if (!stateStore.getAliasTarget || stateStore.getAliasTarget(alias) !== fromAccount) {
    throw new Error('Alias is not assigned to from account');
  }
  if (!stateStore.hasAccount(toAccount)) throw new Error('To account not found: ' + toAccount);

  return _persist(_entry({
    type: 'ACCOUNT_ALIAS_TRANSFER',
    from: alias,
    to: toAccount,
    epoch: epoch || 0,
    signature: ownerKeySig || null,
    signed_by: 'owner',
    alias_data: {
      alias,
      from_account: fromAccount,
      to_account: toAccount,
    },
  }));
}

/**
 * Record a transfer on the ledger. ALL transfers go through here.
 * Validates via mempool (double-spend protection), then applies to stateStore.
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
    signature: signature || null,
  };
  const mResult = mempool.submit(tx);
  if (!mResult.accepted && mResult.reason !== 'duplicate') {
    throw new Error('Transfer rejected: ' + mResult.reason);
  }

  const entry = _entry({
    type: 'TRANSFER',
    from,
    to,
    token: token || 'BTCPC',
    amount,
    epoch,
    signature,
    signed_by: 'active',
    memo,
  });
  return _persist(entry);
}

/**
 * Record a mining reward on the ledger.
 * @param {string} rewardSource — optional: 'mining', 'clock', 'storage', 'iot', 'verifier', 'service', 'inference_split'
 */
async function recordMiningReward(miner, amount, epoch, _token, _memo, rewardSource) {
  const entry = _entry({
    type: 'MINING_REWARD',
    to: miner,
    token: 'BTCPC',
    amount,
    epoch,
    reward_source: rewardSource || 'mining',
  });
  return _persist(entry);
  // Cross-chain credits (0.1 BTCPC per chain) are accrued automatically by
  // stateStore.applyEntry on every MINING_REWARD — both for new blocks and
  // when replaying historical blocks, giving all miners retroactive credit.
}

/**
 * Record a project revenue split configuration on the ledger.
 */
async function recordProjectRevenueSplit(setter, project, splits, epoch) {
  const entry = _entry({
    type: 'PROJECT_REVENUE_SPLIT',
    from: setter,
    token: 'BTCPC',
    amount: 0,
    epoch,
    split_data: { project, splits },
  });
  return _persist(entry);
}

/**
 * Register a community model upload on-chain.
 * Caller must have already stored all blob files and deducted stake from balance.
 */
async function recordModelUpload(uploader, modelId, modelData, epoch) {
  const entry = _entry({
    type: 'MODEL_UPLOAD',
    from: uploader,
    epoch,
    model_data: {
      model_id: modelId,
      name: modelData.name || modelId,
      description: modelData.description || '',
      format: modelData.format || 'onnx',
      size_mb: modelData.size_mb || 0,
      files: modelData.files || {},
      royalty_percent: modelData.royalty_percent || 5,
      stake_amount: modelData.stake_amount || 0,
    },
  });
  return _persist(entry);
}

/**
 * Store a private encrypted file manifest on-chain (v3.1.110+).
 * The manifest is AES-256-GCM encrypted with a per-file DEK.
 * The DEK is ECIES-wrapped for the owner's memo public key.
 * Storage nodes never see the plaintext manifest — only the owner
 * (or grantees) can reconstruct chunk locations.
 */
async function recordFileStore(owner, storageId, encryptedManifest, wrappedDekOwner, initialGrants, totalSize, epoch) {
  const entry = _entry({
    type: 'FILE_STORE',
    from: owner,
    epoch,
    file_data: {
      storage_id: storageId,
      encrypted_manifest: encryptedManifest,
      wrapped_dek_owner: wrappedDekOwner,
      grants: initialGrants || [],
      total_size: totalSize || 0,
    },
  });
  return _persist(entry);
}

/**
 * Owner adds a new access grant to an existing stored file.
 * The caller (owner) must supply wrapped_dek already encrypted for the grantee
 * using the grantee's memo public key — the server never holds the plaintext DEK.
 */
async function recordFileGrant(owner, storageId, grantEntry, epoch) {
  const entry = _entry({
    type: 'FILE_GRANT',
    from: owner,
    epoch,
    grant_data: {
      storage_id: storageId,
      grantee: grantEntry.grantee,
      wrapped_dek: grantEntry.wrapped_dek,
      expires_at: grantEntry.expires_at || null,
    },
  });
  return _persist(entry);
}

/**
 * Record one half of a quantum-split file on chain (v3.1.111+).
 * Each FILE_SHARD entry carries one encrypted pointer: blob_cid + DEK share,
 * wrapped for a specific key (posting for shard A, memo for shard B).
 * Two entries per file, written separately — nothing on chain links them.
 * The pair_id that ties them together is returned to the client only.
 */
async function recordFileShard(owner, shardId, encryptedPtr, epoch) {
  const entry = _entry({
    type: 'FILE_SHARD',
    from: owner,
    epoch,
    shard_data: {
      shard_id: shardId,
      encrypted_ptr: encryptedPtr,
    },
  });
  return _persist(entry);
}

/**
 * Owner revokes a grantee's access. The grantee's wrapped_dek is removed
 * from chain state; they can no longer recover chunk locations.
 */
async function recordFileRevoke(owner, storageId, grantee, epoch) {
  const entry = _entry({
    type: 'FILE_REVOKE',
    from: owner,
    epoch,
    revoke_data: { storage_id: storageId, grantee },
  });
  return _persist(entry);
}

/**
 * Record a faucet distribution.
 *
 * New faucet grants must be delegated network-use balance, never spendable
 * wallet balance. Kept for compatibility with older callers, but it now emits a
 * DELEGATE entry from btcpc_faucet instead of a FAUCET credit.
 */
async function recordFaucet(to, amount, epoch) {
  return recordDelegate('btcpc_faucet', to, amount, 'faucet', epoch);
}

/**
 * Token creation fee tiers.
 * Standard supply (42M) is cheapest. Custom supply costs more.
 */
const TOKEN_FEE_TIERS = {
  micro:    { maxSupply: 1000000,       fee: 21,  label: 'Micro (up to 1M)' },
  standard: { maxSupply: 42000000,      fee: 42,  label: 'Standard (up to 42M)' },
  mega:     { maxSupply: 1000000000,    fee: 84,  label: 'Mega (up to 1B)' },
  custom:   { maxSupply: Infinity,      fee: 168, label: 'Custom (any amount)' },
};

const NFT_CREATION_FEE = 10; // BTCPC per NFT collection

function getTokenFee(supply) {
  if (supply <= TOKEN_FEE_TIERS.micro.maxSupply) return TOKEN_FEE_TIERS.micro.fee;
  if (supply <= TOKEN_FEE_TIERS.standard.maxSupply) return TOKEN_FEE_TIERS.standard.fee;
  if (supply <= TOKEN_FEE_TIERS.mega.maxSupply) return TOKEN_FEE_TIERS.mega.fee;
  return TOKEN_FEE_TIERS.custom.fee;
}

async function recordTokenCreate(creator, tokenData, fee, epoch) {
  if (!fee && fee !== 0) {
    fee = getTokenFee(tokenData.supply || 42000000);
  }

  // Fee payment — goes to protocol treasury
  if (fee > 0) {
    await recordTransfer(creator, 'btcpc_treasury', fee, 'BTCPC', null, epoch, 'Token creation fee: ' + tokenData.symbol);
  }

  const entry = _entry({
    type: 'TOKEN_CREATE',
    from: creator,
    token: tokenData.symbol,
    epoch,
    token_data: {
      name: tokenData.name,
      symbol: tokenData.symbol,
      supply: tokenData.supply,
      decimals: tokenData.decimals || 8,
      type: tokenData.type || 'fungible',
    },
  });
  _persist(entry);

  // Mint initial supply to creator (fungible tokens only)
  if (tokenData.type !== 'nft') {
    const mintEntry = _entry({
      type: 'FAUCET',
      from: 'btcpc_mint',
      to: creator,
      token: tokenData.symbol,
      amount: tokenData.supply,
      epoch,
      memo: 'Initial supply: ' + tokenData.name,
    });
    _persist(mintEntry);
  }

  return entry;
}

/**
 * Record staking on the ledger.
 */
async function recordStake(account, amount, purpose, epoch) {
  const entry = _entry({
    type: 'STAKE',
    from: account,
    to: 'btcpc_staking_pool',
    token: 'BTCPC',
    amount,
    epoch,
    delegation_data: { purpose },
  });
  return _persist(entry);
}

/**
 * Record unstake (withdrawal from staking pool) on the ledger.
 */
async function recordUnstake(account, amount, epoch, memo) {
  const entry = _entry({
    type: 'UNSTAKE',
    from: 'btcpc_staking_pool',
    to: account,
    token: 'BTCPC',
    amount,
    epoch,
    memo,
  });
  return _persist(entry);
}

/**
 * Record delegation on the ledger.
 */
async function recordDelegate(from, to, amount, purpose, epoch) {
  const entry = _entry({
    type: 'DELEGATE',
    from,
    to,
    token: 'BTCPC',
    amount,
    epoch,
    delegation_data: { purpose },
  });
  return _persist(entry);
}

/**
 * Record an inference charge — deducts from requester's delegated balance first,
 * then owned balance. Payment goes to miner/pool (to).
 */
async function recordInferenceCharge(from, to, amount, epoch, requestId) {
  const entry = _entry({
    type: 'INFERENCE_CHARGE',
    from,
    to,
    token: 'BTCPC',
    amount,
    epoch,
    memo: requestId ? 'inference:' + requestId : undefined,
  });
  return _persist(entry);
}

/**
 * Record undelegation on the ledger.
 */
async function recordUndelegate(from, to, amount, epoch, memo) {
  const entry = _entry({
    type: 'UNDELEGATE',
    from,
    to,
    token: 'BTCPC',
    amount,
    epoch,
    memo,
  });
  return _persist(entry);
}

/**
 * Record escrow lock on the ledger.
 */
async function recordEscrowLock(payer, requestId, amount, epoch) {
  const entry = _entry({
    type: 'ESCROW_LOCK',
    from: payer,
    to: 'btcpc_escrow',
    token: 'BTCPC',
    amount,
    epoch,
    memo: 'escrow:' + requestId,
  });
  return _persist(entry);
}

/**
 * Record escrow release (payment to node) on the ledger.
 */
async function recordEscrowRelease(recipient, requestId, amount, epoch, memo) {
  const entry = _entry({
    type: 'ESCROW_RELEASE',
    from: 'btcpc_escrow',
    to: recipient,
    token: 'BTCPC',
    amount,
    epoch,
    memo: 'escrow:' + requestId,
    escrow_id: requestId,
    settlement_note: memo || null,
  });
  return _persist(entry);
}

/**
 * Record escrow refund on the ledger.
 */
async function recordEscrowRefund(payer, requestId, amount, epoch) {
  const entry = _entry({
    type: 'ESCROW_REFUND',
    from: 'btcpc_escrow',
    to: payer,
    token: 'BTCPC',
    amount,
    epoch,
    memo: 'escrow:' + requestId,
  });
  return _persist(entry);
}

/**
 * Register a node on the permanent ledger.
 *
 * v2.10.2: nodeType can be a single string (legacy) or an array of
 * strings (multi-capability). Additional optional fields allow nodes
 * to advertise storage/service/LoRa capacity for their respective
 * capabilities. A single account can declare multiple roles.
 *
 * @param {string} username
 * @param {string|string[]} nodeType — single role or array of roles:
 *        "miner", "verifier", "clock", "storage_host", "service_host",
 *        "gateway_op", "sensor_bridge"
 * @param {string} p2pAddress
 * @param {boolean} permissioned
 * @param {number} epoch
 * @param {object} [extra] — optional { storage_capacity_gb, service_capacity, lora_region }
 */
async function recordNodeRegister(username, nodeType, p2pAddress, permissioned, epoch, extra) {
  var types;
  if (Array.isArray(nodeType)) {
    types = nodeType;
  } else {
    types = [nodeType || 'clock'];
  }
  // Normalize to lowercase + dedupe
  var normalized = {};
  for (var i = 0; i < types.length; i++) {
    var t = String(types[i] || '').trim().toLowerCase();
    if (t) normalized[t] = true;
  }
  types = Object.keys(normalized);

  var accountData = {
    username: username,
    node_types: types,
    node_type: types[0] || 'clock', // legacy field for backward compat
    p2p_address: p2pAddress || null,
    permissioned: !!permissioned,
  };

  if (extra && typeof extra === 'object') {
    if (extra.storage_capacity_gb !== undefined) {
      accountData.storage_capacity_gb = extra.storage_capacity_gb;
    }
    if (extra.service_capacity) {
      accountData.service_capacity = extra.service_capacity;
    }
    if (extra.lora_region) {
      accountData.lora_region = extra.lora_region;
    }
  }

  const entry = _entry({
    type: 'NODE_REGISTER',
    from: username,
    epoch: epoch || 0,
    memo: types.join(','),
    account_data: accountData,
  });
  return _persist(entry);
}

/**
 * Create an NFT collection on the ledger.
 */
async function recordNFTCreate(creator, collectionData, epoch) {
  const fee = NFT_CREATION_FEE;

  if (fee > 0) {
    await recordTransfer(creator, 'btcpc_treasury', fee, 'BTCPC', null, epoch, 'NFT collection fee: ' + collectionData.symbol);
  }

  const entry = _entry({
    type: 'TOKEN_CREATE',
    from: creator,
    token: collectionData.symbol,
    epoch,
    token_data: {
      name: collectionData.name,
      symbol: collectionData.symbol,
      supply: collectionData.maxSupply || 0, // 0 = unlimited minting
      decimals: 0, // NFTs are indivisible
      type: 'nft',
    },
    memo: collectionData.description || null,
  });
  return _persist(entry);
}

/**
 * Mint an NFT within a collection.
 */
async function recordNFTMint(collection, to, tokenId, metadata, epoch) {
  const entry = _entry({
    type: 'FAUCET',
    from: 'btcpc_mint',
    to,
    token: collection,
    amount: 1,
    epoch,
    memo: 'nft:' + tokenId + ':' + JSON.stringify(metadata || {}),
  });
  return _persist(entry);
}

/**
 * Transfer an NFT. Rejects soulbound and time-locked tokens.
 */
async function recordNFTTransfer(from, to, collection, tokenId, epoch, signature) {
  // Amber Pill NFTs are non-transferable by design — check by ID prefix
  // before even looking up the stateStore record (covers replay + fresh mints).
  if (tokenId && tokenId.startsWith('amber-pill-')) {
    throw new Error('Amber Pill NFTs are non-transferable (soulbound geo-pioneer badges)');
  }

  // Check soulbound / time-lock flags via stateStore NFT map.
  const nft = stateStore.getNFT(collection, tokenId);
  if (nft) {
    if (nft.soulbound) {
      throw new Error('This NFT is soulbound and cannot be transferred');
    }
    if (nft.time_locked) {
      const currentEpoch = await getCurrentEpoch();
      if (currentEpoch < nft.unlock_epoch) {
        throw new Error('This NFT is time-locked until epoch ' + nft.unlock_epoch);
      }
    }
  }

  const entry = _entry({
    type: 'TRANSFER',
    from,
    to,
    token: collection,
    amount: 1,
    epoch,
    signature,
    signed_by: 'active',
    memo: 'nft:' + tokenId,
  });
  return _persist(entry);
}

/**
 * Mint a soulbound NFT — permanently bound to the recipient, never transferable.
 */
async function recordSoulboundMint(collection, to, tokenId, metadata, epoch) {
  metadata = metadata || {};
  metadata.soulbound = true;
  metadata.bound_to = to;
  metadata.bound_at = Date.now();

  return recordNFTMint(collection, to, tokenId, metadata, epoch);
}

/**
 * Mint a revenue-sharing NFT — holders earn a % of inference fees.
 */
async function recordRevenueShareMint(collection, to, tokenId, revenueConfig, metadata, epoch) {
  metadata = metadata || {};
  metadata.revenue_share = true;
  metadata.model = revenueConfig.model;
  metadata.rev_share_percent = revenueConfig.revSharePercent || 5;
  metadata.project = revenueConfig.project || null;
  metadata.creator = to;
  metadata.created_at = Date.now();

  return recordNFTMint(collection, to, tokenId, metadata, epoch);
}

/**
 * Distribute revenue share to NFT holders for a completed inference job.
 * Iterates stateStore's NFT map and pays out holders whose NFT matches
 * the model that served the inference.
 */
async function distributeRevenueShare(model, inferenceRevenue, epoch) {
  const payouts = [];
  const allNFTs = stateStore.getAllNFTs ? stateStore.getAllNFTs() : [];

  for (const nft of allNFTs) {
    const meta = nft && nft.metadata;
    if (!meta || !meta.revenue_share || meta.model !== model) continue;

    const percent = meta.rev_share_percent || 5;
    const holder = nft.owner;
    const amount = parseFloat((inferenceRevenue * percent / 100).toFixed(10));

    if (holder && amount > 0.000001) {
      await recordMiningReward(holder, amount, epoch);
      payouts.push({ to: holder, amount, percent });
    }
  }

  return payouts;
}

/**
 * Mint a time-locked NFT — cannot be transferred until a specific epoch.
 */
async function recordTimeLockedMint(collection, to, tokenId, unlockEpoch, metadata, epoch) {
  metadata = metadata || {};
  metadata.time_locked = true;
  metadata.unlock_epoch = unlockEpoch;

  return recordNFTMint(collection, to, tokenId, metadata, epoch);
}

/**
 * Mint a rental NFT — owner retains ownership, renter gets temporary access.
 */
async function recordRentalMint(collection, to, tokenId, rentalConfig, metadata, epoch) {
  metadata = metadata || {};
  metadata.rental = true;
  metadata.owner = to;
  metadata.max_rental_epochs = rentalConfig.maxEpochs || 8640;
  metadata.rental_price = rentalConfig.price || 0;

  return recordNFTMint(collection, to, tokenId, metadata, epoch);
}

/**
 * Rent an NFT — creates a temporary access record.
 */
async function recordNFTRental(collection, tokenId, renter, ownerUsername, durationEpochs, price, epoch) {
  if (price > 0) {
    await recordTransfer(renter, ownerUsername, price, 'BTCPC', null, epoch, 'NFT rental: ' + collection + ':' + tokenId);
  }

  const entry = _entry({
    type: 'TRANSFER',
    from: renter,
    to: renter, // access grant, not a transfer
    token: collection,
    amount: 0,
    epoch,
    memo: 'nft-rental:' + tokenId + ':' + durationEpochs + ':' + ownerUsername,
  });
  return _persist(entry);
}

/**
 * Mint a composable NFT — can contain other NFTs/tokens as a bundle.
 */
async function recordComposableMint(collection, to, tokenId, contents, metadata, epoch) {
  metadata = metadata || {};
  metadata.composable = true;
  metadata.contents = contents || [];

  return recordNFTMint(collection, to, tokenId, metadata, epoch);
}

/**
 * Add an item to a composable NFT.
 */
async function recordComposableAdd(parentCollection, parentTokenId, childCollection, childTokenId, owner, epoch) {
  const entry = _entry({
    type: 'TRANSFER',
    from: owner,
    to: 'composable:' + parentCollection + ':' + parentTokenId,
    token: childCollection,
    amount: 1,
    epoch,
    memo: 'compose:' + childTokenId,
  });
  return _persist(entry);
}

/**
 * Mint an evolving NFT — metadata changes based on on-chain activity.
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
  const entry = _entry({
    type: 'TRANSFER',
    from: owner,
    to: owner,
    token: collection,
    amount: 0,
    epoch,
    memo: 'evolve:' + tokenId + ':' + newMetricValue,
  });
  return _persist(entry);
}

/**
 * Record a heartbeat — proves the account holder is alive.
 */
async function recordHeartbeat(username, epoch) {
  const entry = _entry({
    type: 'HEARTBEAT',
    from: username,
    to: username,
    token: 'BTCPC',
    amount: 0,
    epoch: epoch || 0,
    memo: 'alive',
  });
  return _persist(entry);
}

// ─────────────────────────────────────────────────────────────────
// Commerce + reputation (v2.10)
// ─────────────────────────────────────────────────────────────────

const bondingCurve = require('./stakeBondingCurve');

/**
 * Open a storefront for `seller`. Caller must have paid the stable fee +
 * locked the BTCPC stake before calling this; those happen via recordTransfer
 * to treasury + recordStake (handled by the route layer).
 *
 * storeData: { name, banner_cid, description_cid, categories }
 * capacity: starting number of product slots (bought via bonding curve)
 * stakeAmount: BTCPC locked as collateral
 * stablePaidUsd: USD paid via wrapped stable
 */
async function recordStoreOpen(seller, storeData, capacity, stakeAmount, stablePaidUsd, epoch) {
  if (!seller) throw new Error('seller required');
  const existing = stateStore.getStore(seller);
  if (existing && existing.status === 'active') throw new Error('store already open');

  const entry = _entry({
    type: 'STORE_OPEN',
    from: seller,
    epoch,
    store_data: {
      action: 'open',
      name: (storeData && storeData.name) || seller,
      banner_cid: (storeData && storeData.banner_cid) || null,
      description_cid: (storeData && storeData.description_cid) || null,
      categories: (storeData && storeData.categories) || [],
      capacity: capacity || 0,
      stake_amount: stakeAmount || 0,
      stake_paid_usd: stablePaidUsd || 0,
    },
  });
  return _persist(entry);
}

async function recordStoreUpdate(seller, updates, epoch) {
  if (!seller) throw new Error('seller required');
  const entry = _entry({
    type: 'STORE_UPDATE',
    from: seller,
    epoch,
    store_data: Object.assign({ action: 'update' }, updates || {}),
  });
  return _persist(entry);
}

async function recordStoreClose(seller, epoch) {
  if (!seller) throw new Error('seller required');
  const entry = _entry({
    type: 'STORE_CLOSE',
    from: seller,
    epoch,
    store_data: { action: 'close' },
  });
  return _persist(entry);
}

/**
 * Expand a store's product capacity via the bonding curve. Caller pays in a
 * wrapped stable (wUSDC/wUSDT/wDAI) to the treasury before this is recorded.
 * Additional BTCPC stake is also locked proportional to the new capacity.
 */
async function recordStakePurchase(seller, additionalCapacity, stableToken, stablePaidUsd, additionalStakeBtcpc, epoch) {
  if (!seller) throw new Error('seller required');
  if (additionalCapacity <= 0) throw new Error('capacity must be positive');

  const entry = _entry({
    type: 'STAKE_PURCHASE',
    from: seller,
    token: stableToken || 'wUSDC',
    amount: stablePaidUsd || 0,
    epoch,
    store_data: {
      action: 'stake_purchase',
      capacity_delta: additionalCapacity,
      stake_amount: additionalStakeBtcpc || 0,
      stake_paid_usd: stablePaidUsd || 0,
      stable_token: stableToken || 'wUSDC',
    },
  });
  return _persist(entry);
}

/**
 * Create a product listing. Requires an active store with remaining capacity;
 * the stateStore dispatcher enforces that as a chain invariant.
 *
 * productData: { product_id, title, description_snippet, content_cid, category, price, token, stock }
 */
async function recordProductCreate(seller, productData, epoch) {
  if (!seller) throw new Error('seller required');
  if (!productData || !productData.product_id) throw new Error('product_id required');

  const entry = _entry({
    type: 'PRODUCT_CREATE',
    from: seller,
    epoch,
    product_data: productData,
  });
  return _persist(entry);
}

async function recordProductUpdate(seller, productId, updates, epoch) {
  if (!seller || !productId) throw new Error('seller + productId required');
  const entry = _entry({
    type: 'PRODUCT_UPDATE',
    from: seller,
    epoch,
    product_data: Object.assign({ product_id: productId }, updates || {}),
  });
  return _persist(entry);
}

async function recordProductDelist(seller, productId, epoch) {
  if (!seller || !productId) throw new Error('seller + productId required');
  const entry = _entry({
    type: 'PRODUCT_DELIST',
    from: seller,
    epoch,
    product_data: { product_id: productId },
  });
  return _persist(entry);
}

/**
 * Place an order. Caller is expected to have locked escrow via
 * recordEscrowLock BEFORE calling this — escrow_id is passed through.
 */
async function recordOrderPlace(buyer, seller, orderId, productId, quantity, unitPrice, token, escrowId, epoch) {
  if (!buyer || !seller || !orderId || !productId) throw new Error('buyer, seller, orderId, productId required');
  const total = parseFloat((unitPrice * quantity).toFixed(10));
  const entry = _entry({
    type: 'ORDER_PLACE',
    from: buyer,
    to: seller,
    token: token || 'BTCPC',
    amount: total,
    epoch,
    order_data: {
      order_id: orderId,
      product_id: productId,
      quantity: quantity,
      unit_price: unitPrice,
      total: total,
      token: token || 'BTCPC',
      escrow_id: escrowId || null,
    },
  });
  return _persist(entry);
}

async function recordOrderFulfill(seller, orderId, fulfillmentCid, epoch) {
  if (!seller || !orderId) throw new Error('seller + orderId required');
  const entry = _entry({
    type: 'ORDER_FULFILL',
    from: seller,
    epoch,
    order_data: {
      order_id: orderId,
      fulfillment_cid: fulfillmentCid || null,
    },
  });
  return _persist(entry);
}

async function recordOrderDelivered(buyer, orderId, epoch) {
  if (!buyer || !orderId) throw new Error('buyer + orderId required');
  const entry = _entry({
    type: 'ORDER_DELIVERED',
    from: buyer,
    epoch,
    order_data: { order_id: orderId },
  });
  return _persist(entry);
}

async function recordOrderCancel(party, orderId, epoch) {
  if (!party || !orderId) throw new Error('party + orderId required');
  const entry = _entry({
    type: 'ORDER_CANCEL',
    from: party,
    epoch,
    order_data: { order_id: orderId },
  });
  return _persist(entry);
}

async function recordOrderDispute(buyer, orderId, memo, epoch) {
  if (!buyer || !orderId) throw new Error('buyer + orderId required');
  const entry = _entry({
    type: 'ORDER_DISPUTE',
    from: buyer,
    epoch,
    order_data: {
      order_id: orderId,
      memo: memo || null,
    },
  });
  return _persist(entry);
}

/**
 * Cast a reputation vote on a store, miner, or product.
 *
 * voter: the voting account
 * targetType: "store" | "miner" | "product"
 * targetId: the account / product_id being voted on
 * vote: +1 or -1
 * weight: 1-100, determined by caller (stake/completed txn count)
 * memo: optional free-text review (can be a content_cid)
 */
async function recordReputationVote(voter, targetType, targetId, vote, weight, memo, epoch) {
  if (!voter || !targetType || !targetId) throw new Error('voter, targetType, targetId required');
  if (!['store', 'miner', 'product'].includes(targetType)) throw new Error('invalid target_type');

  const entry = _entry({
    type: 'REPUTATION_VOTE',
    from: voter,
    epoch,
    vote_data: {
      target_type: targetType,
      target_id: targetId,
      vote: vote > 0 ? 1 : -1,
      weight: Math.max(1, Math.min(100, weight || 1)),
      memo: memo || null,
    },
  });
  return _persist(entry);
}

/**
 * Record a BTCPC-FS blob storage commitment on chain.
 *
 * v2.11.0 signature: (uploader, cid, size, hosts, durationEpochs, paymentBtcpc, epoch)
 *   - Simple hosts list, single-tier, no region constraints
 *   - Preserved for backward compatibility
 *
 * v2.11.2 signature: (uploader, cid, size, options, epoch) where options is:
 *   {
 *     hosts: [...],              // legacy unified list (auto-populated from active+cold)
 *     active_hosts: [...],       // v2.11.2+: hosts committed to active serving
 *     cold_hosts: [...],         // v2.11.2+: hosts committed to durability only
 *     duration_epochs: number,
 *     payment_btcpc: number,
 *     region_constraints: [...], // v2.11.2+: ISO region codes, empty = any
 *     target_active: number,     // v2.11.2+: uploader-requested active count
 *     target_cold: number,       // v2.11.2+: uploader-requested cold count
 *   }
 *
 * Callers detect the signature by checking if the 4th argument is an
 * array (legacy) or an object (v2.11.2+).
 */
async function recordBlobStoreCommit(uploader, cid, size, hostsOrOptions, durationEpochsOrEpoch, paymentBtcpc, epoch) {
  if (!uploader) throw new Error('uploader required');
  if (!cid || !/^[a-f0-9]{64}$/.test(cid)) throw new Error('cid must be 64-char hex sha256');
  if (!Number.isFinite(size) || size <= 0) throw new Error('size must be a positive number');

  let blobData;
  let effectiveEpoch;

  if (Array.isArray(hostsOrOptions)) {
    // Legacy v2.11.0 positional signature
    blobData = {
      cid: cid,
      size: size,
      hosts: hostsOrOptions,
      duration_epochs: durationEpochsOrEpoch || 0,
      payment_btcpc: paymentBtcpc || 0,
    };
    effectiveEpoch = epoch;
  } else if (hostsOrOptions && typeof hostsOrOptions === 'object') {
    // v2.11.2+ named options signature
    const opts = hostsOrOptions;
    const active = Array.isArray(opts.active_hosts) ? opts.active_hosts : [];
    const cold = Array.isArray(opts.cold_hosts) ? opts.cold_hosts : [];
    const legacyHosts = Array.isArray(opts.hosts) ? opts.hosts : active.concat(cold);

    blobData = {
      cid: cid,
      size: size,
      hosts: legacyHosts,
      active_hosts: active,
      cold_hosts: cold,
      duration_epochs: opts.duration_epochs || 0,
      payment_btcpc: opts.payment_btcpc || 0,
      region_constraints: Array.isArray(opts.region_constraints) ? opts.region_constraints : [],
      target_active: opts.target_active || active.length,
      target_cold: opts.target_cold || cold.length,
    };
    effectiveEpoch = durationEpochsOrEpoch; // 5th arg is epoch in this signature
  } else {
    throw new Error('hostsOrOptions must be an array (legacy) or object (v2.11.2+)');
  }

  const entry = _entry({
    type: 'BLOB_STORE_COMMIT',
    from: uploader,
    epoch: effectiveEpoch,
    blob_data: blobData,
  });
  return _persist(entry);
}

/**
 * Issue a blob challenge (v2.11.2+).
 *
 * A verifier picks a random host + CID + byte range and asks the host
 * to return the sha256 of those bytes. Host has ~2 epochs to respond.
 *
 * IMPORTANT: failures do NOT trigger slashing. See
 * feedback_storage_no_slash.md — storage is pay-for-delivery, not
 * slash-for-absence. Failed challenges just reduce the host's payout
 * share for this specific commit and dip reputation.
 *
 * @param {string} challenger — verifier account issuing the challenge
 * @param {string} challengeId — unique id, caller generates
 * @param {string} host — storage host being challenged
 * @param {string} cid — blob being verified
 * @param {number} byteStart
 * @param {number} byteLength
 * @param {string} [expectedHash] — optional: if challenger pre-publishes
 *        the expected hash, the chain can auto-resolve the challenge
 *        on response without a follow-up CHALLENGE_RESULT entry
 * @param {number} epoch
 */
async function recordBlobChallenge(challenger, challengeId, host, cid, byteStart, byteLength, expectedHash, epoch) {
  if (!challenger) throw new Error('challenger required');
  if (!challengeId) throw new Error('challengeId required');
  if (!host) throw new Error('host required');
  if (!cid || !/^[a-f0-9]{64}$/.test(cid)) throw new Error('cid must be 64-char hex sha256');
  if (!Number.isFinite(byteStart) || byteStart < 0) throw new Error('byteStart must be non-negative');
  if (!Number.isFinite(byteLength) || byteLength <= 0) throw new Error('byteLength must be positive');

  const entry = _entry({
    type: 'BLOB_CHALLENGE',
    from: challenger,
    epoch,
    challenge_data: {
      challenge_id: challengeId,
      host: host,
      cid: cid,
      byte_start: byteStart,
      byte_length: byteLength,
      expected_hash: expectedHash || null,
    },
  });
  return _persist(entry);
}

/**
 * Host responds to a challenge with the computed hash.
 * If the challenger pre-published expected_hash, the chain resolves
 * status immediately on this entry. Otherwise status transitions to
 * "responded" and waits for BLOB_CHALLENGE_RESULT.
 */
async function recordBlobChallengeResponse(host, challengeId, responseHash, epoch) {
  if (!host) throw new Error('host required');
  if (!challengeId) throw new Error('challengeId required');
  if (!responseHash || !/^[a-f0-9]{64}$/.test(responseHash)) {
    throw new Error('responseHash must be 64-char hex sha256');
  }

  const entry = _entry({
    type: 'BLOB_CHALLENGE_RESPONSE',
    from: host,
    epoch,
    challenge_data: {
      challenge_id: challengeId,
      response_hash: responseHash,
    },
  });
  return _persist(entry);
}

/**
 * Verifier records the outcome of a responded challenge after auditing
 * the response hash against their own computation.
 */
async function recordBlobChallengeResult(verifier, challengeId, passed, epoch) {
  if (!verifier) throw new Error('verifier required');
  if (!challengeId) throw new Error('challengeId required');
  const entry = _entry({
    type: 'BLOB_CHALLENGE_RESULT',
    from: verifier,
    epoch,
    challenge_data: {
      challenge_id: challengeId,
      passed: !!passed,
    },
  });
  return _persist(entry);
}

/**
 * Verifier records a challenge timeout — host failed to respond within
 * the response window (default 2 epochs). Counts as a failure but
 * importantly does NOT slash.
 */
async function recordBlobChallengeTimeout(verifier, challengeId, epoch) {
  if (!verifier) throw new Error('verifier required');
  if (!challengeId) throw new Error('challengeId required');
  const entry = _entry({
    type: 'BLOB_CHALLENGE_TIMEOUT',
    from: verifier,
    epoch,
    challenge_data: {
      challenge_id: challengeId,
    },
  });
  return _persist(entry);
}

/**
 * Record a storage heartbeat on chain (v2.11.2+).
 *
 * Home-user-friendly durability signal. A storage host broadcasts that
 * it's still alive and still holds a list of CIDs. Used by:
 *   1. Uptime-weighted payout formulas
 *   2. Verifier challenge selection (pick a recent heartbeat, challenge
 *      the host for a random byte range from one of its declared CIDs)
 *   3. Auto-replacement when a host goes dark
 *
 * Designed for flaky home internet: heartbeats are cheap, intermittent
 * drops are tolerable, and a host that misses a few heartbeats but
 * comes back still has a usable uptime_factor > 0. Stake slashing only
 * kicks in for extended silence or failed challenges.
 *
 * @param {string} host — the reporting storage host account name
 * @param {string[]} cids — list of CIDs currently held on disk
 * @param {number} capacityUsedGb — optional, total disk used by blobs
 * @param {number} epoch
 */
async function recordStorageHeartbeat(host, cids, capacityUsedGb, epoch) {
  if (!host) throw new Error('host required');
  if (!Array.isArray(cids)) throw new Error('cids must be an array');

  // Filter to valid CIDs only (defense in depth — stateStore also filters)
  const cleanCids = cids.filter((c) => typeof c === 'string' && /^[a-f0-9]{64}$/.test(c));

  const entry = _entry({
    type: 'STORAGE_HEARTBEAT',
    from: host,
    epoch,
    blob_data: {
      cids: cleanCids,
      capacity_used_gb: Number(capacityUsedGb) || 0,
    },
  });
  return _persist(entry);
}

/**
 * Record a BTCPC-FS blob serve proof on chain (v2.11.1+).
 *
 * A storage host reports how many bytes of a CID it served in the current
 * epoch. Chain invariants (enforced in stateStore dispatcher):
 *   - the CID must have a prior BLOB_STORE_COMMIT
 *   - the reporting host must be in the committed hosts list
 *   - bytes_served must be non-negative
 *
 * v2.11.1 records and aggregates. v2.11.2+ will add verifier spot-checks
 * (sample a byte range, challenge the host to return it, slash on mismatch)
 * to make fraudulent inflated proofs economically irrational.
 *
 * @param {string} host — the reporting storage host account name
 * @param {string} cid — 64-char hex sha256
 * @param {number} bytesServed — bytes served since last proof, non-negative
 * @param {number} requestCount — optional, number of distinct requests served
 * @param {string} accessLogMerkleRoot — optional, root of host's access log
 *                 merkle tree for this epoch (used by verifier spot-checks)
 * @param {number} epoch
 */
async function recordBlobServeProof(host, cid, bytesServed, requestCount, accessLogMerkleRoot, epoch) {
  if (!host) throw new Error('host required');
  if (!cid || !/^[a-f0-9]{64}$/.test(cid)) throw new Error('cid must be 64-char hex sha256');
  if (!Number.isFinite(bytesServed) || bytesServed < 0) {
    throw new Error('bytes_served must be a non-negative number');
  }

  const entry = _entry({
    type: 'BLOB_SERVE_PROOF',
    from: host,
    epoch,
    blob_data: {
      cid: cid,
      bytes_served: bytesServed,
      request_count: requestCount || 0,
      access_log_merkle_root: accessLogMerkleRoot || null,
    },
  });
  return _persist(entry);
}

// ─────────────────────────────────────────────────────────────────
// IoT sensor + gateway ledger entries (v2.15-beta)
// ─────────────────────────────────────────────────────────────────

/**
 * Record a sensor registration on chain.
 * Entry type: SENSOR_REGISTER
 *
 * @param {string} owner    — BTCPC account name
 * @param {string} sensorId — "<owner>/<device-name>"
 * @param {object} spec     — { type, unit, decimals, region, lora_gateway?, hardware_model?, firmware_version? }
 * @param {number} epoch
 */
function recordSensorRegister(owner, sensorId, spec, epoch) {
  if (!owner) throw new Error('owner required');
  if (!sensorId) throw new Error('sensorId required');
  if (!spec || typeof spec !== 'object') throw new Error('spec required');

  const entry = _entry({
    type: 'SENSOR_REGISTER',
    from: owner,
    epoch: epoch || 0,
    sensor_data: {
      sensor_id: sensorId,
      owner: owner,
      type: spec.type || null,
      unit: spec.unit || null,
      decimals: spec.decimals !== undefined ? spec.decimals : 2,
      region: spec.region || null,
      lora_gateway: spec.lora_gateway || null,
      hardware_model: spec.hardware_model || null,
      firmware_version: spec.firmware_version || null,
    },
  });
  return _persist(entry);
}

/**
 * Record a sensor reading on chain.
 * Entry type: SENSOR_READING
 *
 * @param {string} sensorId — "<owner>/<device-name>"
 * @param {number} value    — the measurement
 * @param {object} metadata — { latitude?, longitude?, altitude?, battery_pct?, signal_strength_dbm?, gateway_id? }
 * @param {number} epoch
 */
function recordSensorReading(sensorId, value, metadata, epoch) {
  if (!sensorId) throw new Error('sensorId required');
  if (!Number.isFinite(Number(value))) throw new Error('value must be a finite number');

  const entry = _entry({
    type: 'SENSOR_READING',
    from: sensorId,
    epoch: epoch || 0,
    sensor_data: {
      sensor_id: sensorId,
      value: Number(value),
      metadata: metadata || {},
    },
  });
  return _persist(entry);
}

/**
 * Record a sensor data commit — finalized epoch readings persisted as a blob.
 * Entry type: SENSOR_DATA_COMMIT
 *
 * @param {string} sensorId     — "<owner>/<device-name>"
 * @param {string} cid          — 64-char hex sha256 of the serialized readings blob
 * @param {number} epoch
 * @param {number} readingCount — number of readings in this epoch
 * @param {number} medianValue  — finalized median for this epoch
 */
function recordSensorDataCommit(sensorId, cid, epoch, readingCount, medianValue) {
  if (!sensorId) throw new Error('sensorId required');
  if (!cid || !/^[a-f0-9]{64}$/.test(cid)) throw new Error('cid must be 64-char hex sha256');

  const entry = _entry({
    type: 'SENSOR_DATA_COMMIT',
    from: sensorId,
    epoch: epoch || 0,
    sensor_data: {
      sensor_id: sensorId,
      cid: cid,
      reading_count: readingCount || 0,
      median_value: medianValue !== undefined ? medianValue : null,
    },
  });
  return _persist(entry);
}

/**
 * Record a gateway registration on chain.
 * Entry type: GATEWAY_REGISTER
 *
 * @param {string} owner     — BTCPC account name
 * @param {string} gatewayId — "<owner>/<gateway-name>"
 * @param {object} spec      — { region, latitude, longitude, antenna_gain_dbi?, hardware_model?, firmware_version?, max_sensors? }
 * @param {number} epoch
 */
function recordGatewayRegister(owner, gatewayId, spec, epoch) {
  if (!owner) throw new Error('owner required');
  if (!gatewayId) throw new Error('gatewayId required');
  if (!spec || typeof spec !== 'object') throw new Error('spec required');

  const entry = _entry({
    type: 'GATEWAY_REGISTER',
    from: owner,
    epoch: epoch || 0,
    gateway_data: {
      gateway_id: gatewayId,
      owner: owner,
      region: spec.region || null,
      latitude: spec.latitude !== undefined ? Number(spec.latitude) : null,
      longitude: spec.longitude !== undefined ? Number(spec.longitude) : null,
      antenna_gain_dbi: spec.antenna_gain_dbi !== undefined ? Number(spec.antenna_gain_dbi) : null,
      hardware_model: spec.hardware_model || null,
      firmware_version: spec.firmware_version || null,
      max_sensors: spec.max_sensors !== undefined ? parseInt(spec.max_sensors, 10) : 50,
    },
  });
  return _persist(entry);
}

/**
 * Record a gateway heartbeat on chain.
 * Entry type: GATEWAY_HEARTBEAT
 *
 * @param {string} gatewayId — "<owner>/<gateway-name>"
 * @param {object} stats     — { sensors_connected, packets_relayed_this_epoch, uptime_seconds?, battery_pct? }
 * @param {number} epoch
 */
function recordGatewayHeartbeat(gatewayId, stats, epoch) {
  if (!gatewayId) throw new Error('gatewayId required');

  const entry = _entry({
    type: 'GATEWAY_HEARTBEAT',
    from: gatewayId,
    epoch: epoch || 0,
    gateway_data: {
      gateway_id: gatewayId,
      stats: stats || {},
    },
  });
  return _persist(entry);
}

// ─────────────────────────────────────────────────────────────────
// Bridge ledger entries (v2.16-alpha)
// ─────────────────────────────────────────────────────────────────

/**
 * Record a bridge wrap: user locks BTCPC native, wBTCPC released on dest chain.
 * Entry type: BRIDGE_WRAP
 *
 * @param {string} user    — BTCPC account name
 * @param {string} chainId — destination chain id (e.g., "base", "ethereum")
 * @param {number} amount  — BTCPC to wrap
 * @param {number} fee     — wrap fee in BTCPC (routed to btcpc_recycle)
 * @param {number} epoch
 */
async function recordBridgeWrap(user, chainId, amount, fee, epoch) {
  if (!user) throw new Error('user required');
  if (!chainId) throw new Error('chainId required');
  if (!Number.isFinite(amount) || amount <= 0) throw new Error('amount must be positive');

  const netAmount = amount;
  const feeAmount = fee || 0;

  // Debit user BTCPC
  const entry = _entry({
    type: 'BRIDGE_WRAP',
    from: user,
    to: 'btcpc_recycle',
    token: 'BTCPC',
    amount: netAmount,
    epoch: epoch || 0,
    bridge_data: {
      chain_id: chainId,
      amount: netAmount,
      fee: feeAmount,
      direction: 'wrap',
    },
  });
  return _persist(entry);
}

/**
 * Record a bridge unwrap: user returns wBTCPC, BTCPC native released.
 * Entry type: BRIDGE_UNWRAP
 *
 * @param {string} user    — BTCPC account name
 * @param {string} chainId — source chain id
 * @param {number} amount  — BTCPC to release to user
 * @param {number} fee     — unwrap fee routed to btcpc_recycle
 * @param {number} epoch
 */
async function recordBridgeUnwrap(user, chainId, amount, fee, epoch) {
  if (!user) throw new Error('user required');
  if (!chainId) throw new Error('chainId required');
  if (!Number.isFinite(amount) || amount <= 0) throw new Error('amount must be positive');

  const entry = _entry({
    type: 'BRIDGE_UNWRAP',
    from: 'btcpc_recycle',
    to: user,
    token: 'BTCPC',
    amount: amount,
    epoch: epoch || 0,
    bridge_data: {
      chain_id: chainId,
      amount: amount,
      fee: fee || 0,
      direction: 'unwrap',
    },
  });
  return _persist(entry);
}

/**
 * Record a bridge LP fund: funder deposits BTCPC liquidity with a lock period.
 * Entry type: BRIDGE_FUND
 *
 * @param {string} funder   — BTCPC account name
 * @param {string} chainId  — destination chain being funded
 * @param {number} amount   — BTCPC deposited
 * @param {number} lockDays — lock duration in days (30-1460)
 * @param {number} epoch
 */
async function recordBridgeFund(funder, chainId, amount, lockDays, epoch) {
  if (!funder) throw new Error('funder required');
  if (!chainId) throw new Error('chainId required');
  if (!Number.isFinite(amount) || amount <= 0) throw new Error('amount must be positive');

  const entry = _entry({
    type: 'BRIDGE_FUND',
    from: funder,
    to: 'btcpc_recycle',
    token: 'BTCPC',
    amount: amount,
    epoch: epoch || 0,
    bridge_data: {
      chain_id: chainId,
      amount: amount,
      lock_days: lockDays || 30,
      direction: 'fund',
    },
  });
  return _persist(entry);
}

/**
 * Record a bridge LP unlock: funder's lock period expired, queued for withdrawal.
 * Entry type: BRIDGE_UNLOCK
 *
 * @param {string} funder  — BTCPC account name
 * @param {string} chainId — chain where the LP was locked
 * @param {number} epoch
 */
async function recordBridgeUnlock(funder, chainId, epoch) {
  if (!funder) throw new Error('funder required');
  if (!chainId) throw new Error('chainId required');

  const entry = _entry({
    type: 'BRIDGE_UNLOCK',
    from: funder,
    epoch: epoch || 0,
    bridge_data: {
      chain_id: chainId,
      direction: 'unlock',
    },
  });
  return _persist(entry);
}

// ─────────────────────────────────────────────────────────────────
// Stateful compute ledger entries (v2.14-beta)
// ─────────────────────────────────────────────────────────────────

/**
 * Record a stateful service deployment on the ledger.
 * Entry type: SERVICE_DEPLOY_STATEFUL
 *
 * @param {string} deployer        — BTCPC account name
 * @param {string} slug            — "<deployer>/<name>"
 * @param {object} runtimeSpec     — runtime config object
 * @param {object} snapshotConfig  — { snapshot_interval_epochs, replication_factor }
 * @param {number} epoch
 */
async function recordStatefulServiceDeploy(deployer, slug, runtimeSpec, snapshotConfig, epoch) {
  if (!deployer) throw new Error('deployer required');
  if (!slug) throw new Error('slug required');
  if (!runtimeSpec || typeof runtimeSpec !== 'object') throw new Error('runtimeSpec required');
  snapshotConfig = snapshotConfig || {};

  const entry = _entry({
    type: 'SERVICE_DEPLOY_STATEFUL',
    from: deployer,
    epoch: epoch || 0,
    service_data: {
      slug,
      deployer,
      runtime_spec: runtimeSpec,
      stateful: true,
      snapshot_interval_epochs: snapshotConfig.snapshot_interval_epochs || null,
      replication_factor: typeof snapshotConfig.replication_factor === 'number'
        ? snapshotConfig.replication_factor : 3,
    },
  });
  return _persist(entry);
}

/**
 * Record a snapshot commit on the ledger.
 * Entry type: SNAPSHOT_COMMIT
 *
 * @param {string}   slug          — service slug
 * @param {string}   cid           — 64-char hex sha256 of the snapshot blob
 * @param {string[]} replicaHosts  — hosts that hold the replica
 * @param {number}   epoch
 */
async function recordSnapshotCommit(slug, cid, replicaHosts, epoch) {
  if (!slug) throw new Error('slug required');
  if (!cid || !/^[a-f0-9]{64}$/.test(cid)) throw new Error('cid must be 64-char hex sha256');

  const entry = _entry({
    type: 'SNAPSHOT_COMMIT',
    from: slug,
    epoch: epoch || 0,
    snapshot_data: {
      slug,
      cid,
      replica_hosts: Array.isArray(replicaHosts) ? replicaHosts.slice() : [],
    },
  });
  return _persist(entry);
}

/**
 * Record a snapshot restore on the ledger.
 * Entry type: SNAPSHOT_RESTORE
 *
 * @param {string} slug   — service slug
 * @param {string} cid    — snapshot CID being restored
 * @param {string} host   — host performing the restore
 * @param {number} epoch
 */
async function recordSnapshotRestore(slug, cid, host, epoch) {
  if (!slug) throw new Error('slug required');

  const entry = _entry({
    type: 'SNAPSHOT_RESTORE',
    from: host || slug,
    epoch: epoch || 0,
    snapshot_data: {
      slug,
      cid: cid || null,
      host: host || null,
    },
  });
  return _persist(entry);
}

// Expose bonding curve helpers at the ledger module for callers/tests
const commerce = {
  costForCapacity: bondingCurve.costForCapacity,
  capacityForPayment: bondingCurve.capacityForPayment,
  stakeForCapacity: bondingCurve.stakeForCapacity,
};

/**
 * Get balance for an account. Phase C: reads from stateStore.
 */
async function getBalance(username, token) {
  return stateStore.getBalance(username, token || 'BTCPC');
}

/**
 * Get all tokens held by an account.
 */
async function getTokenBalances(username) {
  return stateStore.getTokenBalances(username);
}

/**
 * Get the full account record from stateStore (public info only).
 */
async function getAccountRecord(username) {
  const account = stateStore.getAccount(username);
  if (!account) return null;
  return {
    username: account.username,
    public_keys: account.public_keys,
    chain_addresses: account.chain_addresses,
    created_epoch: account.created_epoch,
  };
}

/**
 * Get all accounts registered on the ledger.
 */
async function getAllAccounts() {
  return stateStore.getAllAccounts().map(a => ({
    account_data: {
      username: a.username,
      public_keys: a.public_keys,
      chain_addresses: a.chain_addresses,
    },
    epoch: a.created_epoch,
  }));
}

/**
 * Flush pending entries — returns them for inclusion in the next block.
 * Drains BOTH the in-process pendingEntries array AND the shared on-disk
 * queue at data/pending-entries.jsonl, so entries created by the API server
 * process end up in the miner's block payloads.
 *
 * Dedupe note: if the same entry somehow appears in both the in-memory
 * array and the on-disk queue (e.g. the miner creates one and also writes
 * it to the file via _persist), stateStore.applyEntry already auto-dedupes
 * via its internal seenEntries Set, so the block payload can carry a
 * duplicate safely — replay will ignore the second copy.
 */
function flushPendingEntries() {
  const memoryEntries = pendingEntries.splice(0, pendingEntries.length);
  const diskEntries = _readAndClearPendingFile();

  // Dedupe by canonical JSON — if the same _persist() call produced an
  // in-memory entry AND a disk entry (normal for single-process dev
  // setups) we only emit one copy. Cross-process scenarios (API server
  // appends, miner drains) have no overlap so this is a no-op.
  const seen = new Set();
  const out = [];
  const all = memoryEntries.concat(diskEntries);
  for (const e of all) {
    var key;
    try {
      key = JSON.stringify(e);
    } catch (_) {
      out.push(e);
      continue;
    }
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(e);
  }
  return out;
}

/**
 * Apply ledger entries received from a remote node (EPOCH_FINALIZED /
 * BLOCK_PROPOSAL gossip). Phase D: dedupe + apply via stateStore only.
 * stateStore.applyEntry already auto-dedupes via its internal seenEntries Set.
 * Remote entries are NOT pushed into pendingEntries — those are for entries
 * this node originates and will include in its own next block.
 */
async function applyRemoteEntries(entries) {
  if (!Array.isArray(entries)) return 0;
  let applied = 0;
  for (const entry of entries) {
    stateStore.applyEntry(entry);
    applied++;
  }
  return applied;
}

/**
 * Record cross-chain activity observed by the chain monitor daemon.
 * Creates a CROSS_CHAIN_ACTIVITY ledger entry that flows into the next block
 * and is stored in stateStore for read paths.
 *
 * @param {string} username     - BTCPC account that owns the external address
 * @param {string} chain        - external chain name (base|arbitrum|ethereum|solana|bitcoin)
 * @param {string} address      - external chain address that was monitored
 * @param {string} activityType - activity descriptor (balance_change|transaction)
 * @param {object} data         - chain-specific activity data (old/new balance, etc.)
 * @param {number} epoch        - current BTCPC epoch
 */
async function recordCrossChainActivity(username, chain, address, activityType, data, epoch) {
  const entry = _entry({
    type: 'CROSS_CHAIN_ACTIVITY',
    to: username,
    epoch: epoch || 0,
    cross_chain_data: Object.assign({
      chain: chain,
      address: address,
      activity_type: activityType || 'balance_change',
    }, data || {}),
  });
  return _persist(entry);
}

/**
 * Apply a ledger entry received from a P2P peer's MEMPOOL_ENTRY gossip
 * message WITHOUT triggering re-gossip (to prevent infinite loops).
 *
 * The entry is:
 *   1. Applied to stateStore so read paths (balances, accounts, etc.) are
 *      immediately up-to-date on this node.
 *   2. Appended to the shared on-disk pending-entries.jsonl queue so that
 *      whenever THIS node becomes the broadcaster, it will include the entry
 *      in its block payload.
 *   3. Recorded in gossipedHashes so _gossipEntry silently skips it —
 *      preventing re-broadcast of a peer-originated entry back into the mesh.
 *
 * Called exclusively by src/p2p/protocol.js:handleMempoolEntry.
 */
function appendForeignEntry(entry) {
  if (!entry || !entry.type) return;
  // Apply to stateStore (idempotent — seenEntries dedupes)
  stateStore.applyEntry(entry);
  // Append to disk queue for the next local flush
  _appendPendingToDisk(entry);
  // Prevent this entry from being re-gossiped by _gossipEntry
  try {
    gossipedHashes.add(JSON.stringify(entry));
  } catch (_) {}
}

/**
 * Record an Amber Pill NFT mint on the ledger (v2.18).
 *
 * Called automatically by the stateStore AMBER_PILL_MINT dispatcher when a
 * geo-pioneer is detected. Creates a permanent, soulbound on-chain record of
 * the first node of a given role to operate in a geographic region.
 *
 * @param {string} account   — pioneer's BTCPC account
 * @param {string} role      — MINER | STORAGE | CLOCK | SENSOR | GATEWAY
 * @param {string} region    — metro-area region code (e.g. "sv-san-salvador")
 * @param {string} nftId     — "amber-pill-<role>-<region>"
 * @param {number} epoch
 */
async function recordAmberPillMint(account, role, region, nftId, epoch) {
  if (!account) throw new Error('account required');
  if (!role) throw new Error('role required');
  if (!region) throw new Error('region required');
  if (!nftId) throw new Error('nftId required');

  return _persist(_entry({
    type: 'AMBER_PILL_MINT',
    to: account,
    epoch: epoch || 0,
    amber_pill_data: {
      role: role,
      region: region,
      nft_id: nftId,
      expires_epoch: (epoch || 0) + 1051200,
    },
  }));
}

/**
 * Record a key rotation on the ledger.
 *
 * @param {string} username         — account whose keys are being rotated
 * @param {object} newPublicKeys    — { active?, posting?, memo?, owner? } — only
 *                                    the keys being replaced need to be present
 * @param {string} authorizedBy     — 'password' | 'owner_signature'
 * @param {number} epoch
 */
async function recordKeyRotate(username, newPublicKeys, authorizedBy, epoch) {
  if (!username) throw new Error('username required');
  if (!newPublicKeys || typeof newPublicKeys !== 'object') throw new Error('newPublicKeys required');
  if (!authorizedBy) throw new Error('authorizedBy required');

  return _persist(_entry({
    type: 'KEY_ROTATE',
    to: username,
    epoch: epoch || 0,
    key_rotate_data: {
      new_public_keys: newPublicKeys,
      authorized_by: authorizedBy,
      timestamp: Date.now(),
    },
  }));
}

/**
 * Register a device public key as an authorized signer for an account.
 * Called automatically on first device startup — natoshi never has to do this manually.
 *
 * @param {string} ownerAccount   — e.g. 'natoshisakamoto'
 * @param {string} devicePubkey   — hex-encoded compressed secp256k1 public key of this device
 * @param {string} hardwareHash   — hardware fingerprint hash (best-effort anti-Sybil)
 * @param {string} scope          — 'mine' | 'clock' | 'all'
 * @param {number} epoch
 */
async function recordDeviceAuthorize(ownerAccount, devicePubkey, hardwareHash, scope, epoch) {
  if (!ownerAccount) throw new Error('ownerAccount required');
  if (!devicePubkey || typeof devicePubkey !== 'string') throw new Error('devicePubkey required');

  return _persist(_entry({
    type: 'DEVICE_AUTHORIZE',
    to: ownerAccount,
    epoch: epoch || 0,
    device_data: {
      device_pubkey: devicePubkey,
      scope: scope || 'mine',
      hardware_hash: hardwareHash || null,
      timestamp: Date.now(),
    },
  }));
}

/**
 * Revoke a device key (e.g. device lost/stolen).
 */
async function recordDeviceRevoke(ownerAccount, devicePubkey, epoch) {
  if (!ownerAccount || !devicePubkey) throw new Error('ownerAccount and devicePubkey required');
  return _persist(_entry({
    type: 'DEVICE_REVOKE',
    to: ownerAccount,
    epoch: epoch || 0,
    device_data: { device_pubkey: devicePubkey, timestamp: Date.now() },
  }));
}

/**
 * Register an IoT device key on chain (v3.2).
 * Authorized by the owner's posting key — posting cannot move funds.
 *
 * @param {string} owner               — BTCPC account name
 * @param {string} deviceId            — "<owner>/<device-name>" (e.g. "josh/flipper-abc123")
 * @param {string} devicePubkey        — hex-encoded compressed secp256k1 public key of the device
 * @param {string} postingKeySignature — owner's posting-key signature over deviceId + devicePubkey
 * @param {number} epoch
 */
function recordDeviceKeyRegister(owner, deviceId, devicePubkey, postingKeySignature, epoch) {
  if (!owner) throw new Error('owner required');
  if (!deviceId) throw new Error('deviceId required');
  if (!devicePubkey) throw new Error('devicePubkey required');
  if (!postingKeySignature) throw new Error('postingKeySignature required');

  const entry = _entry({
    type: 'DEVICE_KEY_REGISTER',
    from: owner,
    epoch: epoch || 0,
    device_key_data: {
      device_id: deviceId,
      owner: owner,
      device_pubkey: devicePubkey,
      posting_key_sig: postingKeySignature,
      registered_epoch: epoch || 0,
    },
  });
  return _persist(entry);
}

/**
 * Record a completed scientific compute result on the ledger.
 * Entry type: SCIENTIFIC_RESULT
 *
 * Called automatically by the scientific compute engine for open-source jobs.
 * The on-chain entry permanently links requester, model, input hash, result hash,
 * and (for large results) the BTCPC-FS CID.
 *
 * @param {string}  jobId
 * @param {string}  requester      — BTCPC account name
 * @param {string}  model          — Ollama model used
 * @param {string}  inputHash      — SHA-256 of input_data
 * @param {string}  resultHash     — SHA-256 of result bytes
 * @param {string}  resultBlobCid  — BTCPC-FS CID if result was too large for inline (or null)
 * @param {boolean} openSource     — always true when called from completeJob
 * @param {number}  fee            — actual fee paid after discount
 * @param {number}  epoch
 * @param {string}  title          — human-readable job title
 * @param {string}  jobType        — 'protein_folding' | 'drug_discovery' | etc.
 */
async function recordScientificResult(jobId, requester, model, inputHash, resultHash, resultBlobCid, openSource, fee, epoch, title, jobType) {
  const entry = _entry({
    type: 'SCIENTIFIC_RESULT',
    from: requester,
    to: 'btcpc_science',
    token: 'BTCPC',
    amount: fee || 0,
    epoch: epoch || 0,
    signed_by: requester,
    memo: title || jobId,
    timestamp: Date.now(),
    scientific_data: {
      job_id: jobId,
      model,
      type: jobType || 'general',
      input_hash: inputHash,
      result_hash: resultHash,
      result_blob_cid: resultBlobCid || null,
      open_source: openSource || false,
    },
  });
  return _persist(entry);
}

/**
 * Register tool capabilities for a node.
 * Entry type: TOOL_CAPABILITY_REGISTER
 *
 * tools: [{ name, kind: 'builtin'|'cli'|'mcp', description, command?, mcp_url?, deterministic? }]
 */
async function recordToolCapabilityRegister(nodeId, tools, epoch) {
  if (!nodeId) throw new Error('nodeId required');
  return _persist(_entry({
    type: 'TOOL_CAPABILITY_REGISTER',
    from: nodeId,
    epoch: epoch || 0,
    tool_data: { tools: tools || [], registered_at: Date.now() },
  }));
}

/**
 * Commit a tool trace to BTCPC-FS for non-deterministic tool results.
 * Entry type: TOOL_TRACE_COMMIT
 *
 * proof_id:        the compute proof this trace belongs to
 * tool_trace_hash: sha256 of the tool call log
 * trace_cids:      [{ tool, cid }] — each non-deterministic call committed separately
 */
async function recordToolTraceCommit(account, proofId, toolTraceHash, traceCids, epoch) {
  if (!account || !proofId) throw new Error('account and proofId required');
  return _persist(_entry({
    type: 'TOOL_TRACE_COMMIT',
    from: account,
    epoch: epoch || 0,
    tool_data: {
      proof_id: proofId,
      tool_trace_hash: toolTraceHash,
      trace_cids: traceCids || [],
      committed_at: Date.now(),
    },
  }));
}

// ─────────────────────────────────────────────────────────────────
// Name Auctions (v3.6)
// ─────────────────────────────────────────────────────────────────

const DEFAULT_AUCTION_DURATION_EPOCHS = 20160; // ~7 days at 30s epochs

async function recordNameAuctionOpen(seller, name, startPriceUsd, minBidIncrementUsd, auctionEndEpoch, epoch) {
  if (!seller) throw new Error('seller required');
  if (!name) throw new Error('name required');
  if (typeof startPriceUsd !== 'number' || startPriceUsd < 0) throw new Error('startPriceUsd must be a non-negative number');
  if (typeof minBidIncrementUsd !== 'number' || minBidIncrementUsd < 0) throw new Error('minBidIncrementUsd must be a non-negative number');
  const ep = epoch || 0;
  const endEpoch = auctionEndEpoch || (ep + DEFAULT_AUCTION_DURATION_EPOCHS);
  return _persist(_entry({
    type: 'NAME_AUCTION_OPEN',
    from: seller,
    epoch: ep,
    auction_data: {
      name,
      seller,
      start_price_usd: startPriceUsd,
      min_bid_increment_usd: minBidIncrementUsd,
      auction_end_epoch: endEpoch,
    },
  }));
}

async function recordNameAuctionBid(name, bidder, bidUsd, chain, txHash, btcpcAccount, btcpcPubkeys, epoch) {
  if (!name) throw new Error('name required');
  if (!bidder) throw new Error('bidder required');
  if (typeof bidUsd !== 'number' || bidUsd <= 0) throw new Error('bidUsd must be a positive number');
  if (!chain) throw new Error('chain required');
  if (!txHash) throw new Error('txHash required');
  return _persist(_entry({
    type: 'NAME_AUCTION_BID',
    from: bidder,
    epoch: epoch || 0,
    auction_data: {
      name,
      bidder,
      bid_usd: bidUsd,
      chain,
      tx_hash: txHash,
      btcpc_account: btcpcAccount || bidder,
      btcpc_pubkeys: btcpcPubkeys || {},
    },
  }));
}

async function recordNameAuctionSettle(name, sellerSig, epoch) {
  if (!name) throw new Error('name required');
  return _persist(_entry({
    type: 'NAME_AUCTION_SETTLE',
    from: 'shindevlin',
    epoch: epoch || 0,
    auction_data: {
      name,
      seller_sig: sellerSig || null,
    },
  }));
}

async function recordNameAuctionCancel(name, sellerSig, epoch) {
  if (!name) throw new Error('name required');
  return _persist(_entry({
    type: 'NAME_AUCTION_CANCEL',
    from: 'shindevlin',
    epoch: epoch || 0,
    auction_data: {
      name,
      seller_sig: sellerSig || null,
    },
  }));
}

// Flat mint fee charged on the BTCPC chain when a claim payload is generated.
// Paid here regardless of destination chain — ETH gas is separate and paid by the claimer.
const CROSS_CHAIN_CLAIM_FEE = 0.001;

/**
 * Record a storage migration — data moved from a failing host to a new host.
 * The old host takes a reputation ding; the new host starts earning for the shard.
 */
async function recordStorageMigration(cid, fromHost, toHost, reason, epoch) {
  if (!cid) throw new Error('cid required');
  if (!fromHost) throw new Error('fromHost required');
  if (!toHost) throw new Error('toHost required');
  return _persist(_entry({
    type: 'STORAGE_MIGRATION',
    from: fromHost,
    to: toHost,
    epoch: epoch || 0,
    migration_data: { cid, from_host: fromHost, to_host: toHost, reason: reason || 'challenge_failure' },
  }));
}

/**
 * Record a node reputation event — oracle or verifier-driven.
 * @param {string} account   — account whose reputation is updated
 * @param {string} category  — 'storage'|'mining'|'sensor'|'clock'|'inference'
 * @param {boolean} passed   — whether the event was a pass or fail
 * @param {number} epoch
 */
async function recordNodeReputationUpdate(account, category, passed, epoch) {
  if (!account) throw new Error('account required');
  if (!category) throw new Error('category required');
  return _persist(_entry({
    type: 'NODE_REPUTATION_UPDATE',
    from: account,
    epoch: epoch || 0,
    reputation_data: { account, category, passed: !!passed },
  }));
}

/**
 * Record a cross-chain claim — consumes accumulated wBTCPC credit on a chain.
 * Charges CROSS_CHAIN_CLAIM_FEE BTCPC to btcpc_recycle on the BTCPC side.
 */
async function recordCrossChainClaim(account, chain, amount, claimAddress, signature, epoch) {
  if (!account) throw new Error('account required');
  if (!chain) throw new Error('chain required');
  if (!amount || amount <= 0) throw new Error('amount must be > 0');
  if (!claimAddress) throw new Error('claim address required');

  const bal = stateStore.getBalance ? stateStore.getBalance(account, 'BTCPC') : 0;
  if (bal < CROSS_CHAIN_CLAIM_FEE) {
    throw new Error(`Insufficient balance for mint fee — need ${CROSS_CHAIN_CLAIM_FEE} BTCPC`);
  }

  // Deduct mint fee → recycle pool
  _persist(_entry({
    type: 'TRANSFER',
    from: account,
    to: 'btcpc_recycle',
    token: 'BTCPC',
    amount: CROSS_CHAIN_CLAIM_FEE,
    epoch: epoch || 0,
    memo: 'cross-chain mint fee',
  }));

  return _persist(_entry({
    type: 'CROSS_CHAIN_CLAIM',
    from: account,
    to: account,
    token: 'BTCPC',
    amount,
    chain,
    epoch: epoch || 0,
    signature: signature || null,
    signed_by: 'posting',
    claim_data: { chain, amount, claim_address: claimAddress, mint_fee: CROSS_CHAIN_CLAIM_FEE },
  }));
}

/**
 * Record a cross-chain identity link on-chain.
 * Creates a permanent, signed attestation that a BTCPC account owns specific
 * addresses on external chains. This is the canonical record btcpcscan and
 * oracle watchers use to route cross-chain payments.
 *
 * @param {string} account        — BTCPC account name
 * @param {object} chainAddresses — { eth, solana, ton, bitcoin, ... }
 * @param {string} ownerKeySig    — signature from account's owner key
 * @param {number} epoch
 */
async function recordIdentityLink(account, chainAddresses, ownerKeySig, epoch) {
  if (!account) throw new Error('account required');
  if (!chainAddresses || typeof chainAddresses !== 'object') throw new Error('chain_addresses required');
  if (!ownerKeySig) throw new Error('owner key signature required');
  if (!stateStore.hasAccount(account)) throw new Error('account not found: ' + account);
  return _persist(_entry({
    type: 'IDENTITY_LINK',
    from: account,
    to: account,
    epoch: epoch || 0,
    signature: ownerKeySig,
    signed_by: 'owner',
    identity_data: { chain_addresses: chainAddresses },
  }));
}

// ─── Storage settlement lag (v3.1.119+) ──────────────────────────────────────
// Hosts earn storage rewards at the end of each epoch, but payout is held for
// STORAGE_HOLD_EPOCHS epochs. If a blob challenge fails during the hold window,
// the held reward is forfeited. This gives buyers a complaint window without
// slashing the host's stake — consistent with the no-slash rule.

const STORAGE_HOLD_EPOCHS = 10; // ~5 minutes

/**
 * Record that a storage reward is being held pending challenge window.
 * Called by blobPayouts.settlePayouts instead of paying directly.
 */
async function recordStoragePayoutHold(host, cid, amount, holdEpoch) {
  if (!host || !cid || !amount || amount <= 0) return null;
  const holdId = `sph_${host}_${cid}_${holdEpoch}`;
  return _persist(_entry({
    type: 'STORAGE_PAYOUT_HOLD',
    from: 'btcpc_storage_escrow',
    to: host,
    token: 'BTCPC',
    amount,
    epoch: holdEpoch || 0,
    storage_hold_data: {
      hold_id: holdId,
      host,
      cid,
      amount,
      hold_epoch: holdEpoch,
      release_epoch: (holdEpoch || 0) + STORAGE_HOLD_EPOCHS,
    },
  }));
}

/**
 * Release a held storage payout to the host after the challenge window passes.
 */
async function recordStoragePayoutRelease(holdId, host, amount, epoch) {
  if (!holdId || !host || !amount) return null;
  return _persist(_entry({
    type: 'STORAGE_PAYOUT_RELEASE',
    from: 'btcpc_storage_escrow',
    to: host,
    token: 'BTCPC',
    amount,
    epoch: epoch || 0,
    storage_hold_data: { hold_id: holdId, host, amount },
  }));
}

/**
 * Forfeit a held storage payout — challenge failed during the hold window.
 * The held amount routes to btcpc_recycle (not to the challenger — no slash).
 */
async function recordStoragePayoutForfeit(holdId, host, amount, cid, epoch) {
  if (!holdId || !host || !amount) return null;
  return _persist(_entry({
    type: 'STORAGE_PAYOUT_FORFEIT',
    from: 'btcpc_storage_escrow',
    to: 'btcpc_recycle',
    token: 'BTCPC',
    amount,
    epoch: epoch || 0,
    storage_hold_data: { hold_id: holdId, host, cid, amount },
  }));
}

/**
 * Scan all pending storage holds and release those whose challenge window has closed.
 * Called from epochManager on every epoch tick.
 */
async function releaseMaturedStorageHolds(currentEpoch) {
  const holds = stateStore.getPendingStorageHolds
    ? stateStore.getPendingStorageHolds()
    : [];

  let released = 0;
  let totalReleased = 0;

  for (const hold of holds) {
    if (currentEpoch < hold.release_epoch) continue;
    try {
      await recordStoragePayoutRelease(hold.hold_id, hold.host, hold.amount, currentEpoch);
      released++;
      totalReleased += hold.amount;
    } catch (_) {}
  }

  return { released, totalReleased: parseFloat(totalReleased.toFixed(10)) };
}

// ─── btcpc_recycle redistribution (v3.1.119+) ────────────────────────────────

const RECYCLE_DISTRIBUTION_INTERVAL_EPOCHS = 240; // ~2 hours
const RECYCLE_DISTRIBUTION_RATE = 0.05; // distribute 5% of pool per interval

/**
 * Distribute a portion of the btcpc_recycle pool to active stakers,
 * proportional to their stake weight. Called every 240 epochs.
 */
async function distributeRecyclePool(epoch) {
  // Only distribute once the 42M supply is fully minted (Phase 2).
  // In Phase 1 the recycle pool accumulates fees as future reward endowment.
  if (!stateStore.isPhase2()) return { distributed: 0, recipients: 0, reason: 'phase1' };

  const recycleBalance = stateStore.getBalance('btcpc_recycle', 'BTCPC');
  if (!recycleBalance || recycleBalance < 0.01) return { distributed: 0, recipients: 0 };

  const stakers = stateStore.getAllStakePools().filter(s =>
    s.total_staked > 0 &&
    s.username !== 'btcpc_recycle' &&
    s.username !== 'btcpc' &&
    s.username !== 'btcpc_escrow' &&
    s.username !== 'btcpc_fees'
  );
  if (stakers.length === 0) return { distributed: 0, recipients: 0 };

  const totalStake = stakers.reduce((sum, s) => sum + s.total_staked, 0);
  if (totalStake <= 0) return { distributed: 0, recipients: 0 };

  const poolToDistribute = parseFloat((recycleBalance * RECYCLE_DISTRIBUTION_RATE).toFixed(10));
  if (poolToDistribute < 0.000001) return { distributed: 0, recipients: 0 };

  let totalDistributed = 0;
  let recipients = 0;

  for (const staker of stakers) {
    const share = staker.total_staked / totalStake;
    const amount = parseFloat((poolToDistribute * share).toFixed(10));
    if (amount < 0.000001) continue;
    await _persist(_entry({
      type: 'RECYCLE_DISTRIBUTION',
      from: 'btcpc_recycle',
      to: staker.username,
      token: 'BTCPC',
      amount,
      epoch: epoch || 0,
      memo: `recycle:${epoch}:${staker.total_staked.toFixed(2)}stake`,
    }));
    totalDistributed += amount;
    recipients++;
  }

  return { distributed: parseFloat(totalDistributed.toFixed(10)), recipients };
}

async function recordNameDelegate(name, delegated, epoch) {
  if (!name) throw new Error('name required');
  return _persist(_entry({
    type: 'NAME_DELEGATE',
    from: 'shindevlin',
    epoch: epoch || 0,
    delegate_data: { name, delegated: !!delegated },
  }));
}

// ─── Inference Marketplace (v3.1.119+) ───────────────────────────────────────

async function recordInferenceJobOpen(buyer, jobId, jobData, epoch) {
  if (!buyer) throw new Error('buyer required');
  if (!jobId) throw new Error('jobId required');
  return _persist(_entry({
    type: 'INFERENCE_JOB_OPEN',
    from: buyer,
    epoch: epoch || 0,
    job_data: {
      job_id: jobId,
      buyer,
      prompt: jobData.prompt,
      max_fee: jobData.max_fee,
      model: jobData.model || null,
      system_prompt: jobData.system_prompt || null,
      ttl_epochs: jobData.ttl_epochs || 20,
      expires_epoch: jobData.expires_epoch || 0,
      tools: jobData.tools || [],
      max_turns: jobData.max_turns || 1,
      output_schema: jobData.output_schema || null,
      tier: jobData.tier || 'standard',
      rag_cids: jobData.rag_cids || [],
      image_cids: jobData.image_cids || [],
      audio_cid: jobData.audio_cid || null,
      batch_id: jobData.batch_id || null,
      session_id: jobData.session_id || null,
      status: 'open',
    },
  }));
}

async function recordInferenceJobClaim(jobId, miner, epoch) {
  if (!jobId) throw new Error('jobId required');
  if (!miner) throw new Error('miner required');
  return _persist(_entry({
    type: 'INFERENCE_JOB_CLAIM',
    from: miner,
    epoch: epoch || 0,
    job_data: { job_id: jobId, miner, status: 'claimed' },
  }));
}

async function recordInferenceJobSubmit(jobId, miner, proofHash, epoch) {
  if (!jobId) throw new Error('jobId required');
  if (!miner) throw new Error('miner required');
  return _persist(_entry({
    type: 'INFERENCE_JOB_SUBMIT',
    from: miner,
    epoch: epoch || 0,
    job_data: { job_id: jobId, miner, proof_hash: proofHash, status: 'submitted' },
  }));
}

async function recordInferenceJobSettle(jobId, miner, buyer, settlementData, epoch) {
  if (!jobId) throw new Error('jobId required');
  return _persist(_entry({
    type: 'INFERENCE_JOB_SETTLE',
    from: miner,
    to: buyer,
    epoch: epoch || 0,
    job_data: {
      job_id: jobId,
      miner,
      buyer,
      actual_cost: settlementData.actual_cost,
      miner_payout: settlementData.miner_payout,
      protocol_fee: settlementData.protocol_fee,
      overpayment: settlementData.overpayment,
      settled_by: settlementData.settled_by || 'auto',
      status: 'settled',
    },
  }));
}

async function recordInferenceJobRefund(jobId, buyer, reason, epoch) {
  if (!jobId) throw new Error('jobId required');
  if (!buyer) throw new Error('buyer required');
  return _persist(_entry({
    type: 'INFERENCE_JOB_REFUND',
    from: buyer,
    epoch: epoch || 0,
    job_data: { job_id: jobId, buyer, reason: reason || 'expired', status: 'refunded' },
  }));
}

// ── Browser / computer-use jobs (v3.1.121+) ───────────────────────────────────

async function recordBrowserJobOpen(buyer, jobId, jobData, epoch) {
  return _persist(_entry({ type: 'BROWSER_JOB_OPEN', from: buyer, epoch: epoch || 0,
    browser_data: { ...jobData, job_id: jobId, buyer } }));
}
async function recordBrowserJobClaim(jobId, miner, epoch) {
  return _persist(_entry({ type: 'BROWSER_JOB_CLAIM', from: miner, epoch: epoch || 0,
    browser_data: { job_id: jobId, miner } }));
}
async function recordBrowserJobScreenshot(jobId, miner, ssData, epoch) {
  return _persist(_entry({ type: 'BROWSER_JOB_SCREENSHOT', from: miner, epoch: epoch || 0,
    browser_data: { job_id: jobId, miner, ...ssData } }));
}
async function recordBrowserJobAction(jobId, buyer, actionData, epoch) {
  return _persist(_entry({ type: 'BROWSER_JOB_ACTION', from: buyer, epoch: epoch || 0,
    browser_data: { job_id: jobId, buyer, ...actionData } }));
}
async function recordBrowserJobSettle(jobId, miner, buyer, settlementData, epoch) {
  return _persist(_entry({ type: 'BROWSER_JOB_SETTLE', from: miner, to: buyer, epoch: epoch || 0,
    browser_data: { job_id: jobId, miner, buyer, ...settlementData } }));
}
async function recordBrowserJobRefund(jobId, buyer, reason, epoch) {
  return _persist(_entry({ type: 'BROWSER_JOB_REFUND', from: buyer, epoch: epoch || 0,
    browser_data: { job_id: jobId, buyer, reason: reason || 'expired' } }));
}

// ── Fine-tuning jobs (v3.1.121+) ─────────────────────────────────────────────

async function recordFinetuneJobOpen(buyer, finetuneId, ftData, epoch) {
  if (!buyer) throw new Error('buyer required');
  if (!finetuneId) throw new Error('finetuneId required');
  return _persist(_entry({
    type: 'FINETUNE_JOB_OPEN',
    from: buyer,
    epoch: epoch || 0,
    ft_data: { ...ftData, finetune_id: finetuneId, buyer },
  }));
}

async function recordFinetuneJobClaim(finetuneId, miner, epoch) {
  return _persist(_entry({
    type: 'FINETUNE_JOB_CLAIM',
    from: miner,
    epoch: epoch || 0,
    ft_data: { finetune_id: finetuneId, miner },
  }));
}

async function recordFinetuneJobProgress(finetuneId, miner, progressData, epoch) {
  return _persist(_entry({
    type: 'FINETUNE_JOB_PROGRESS',
    from: miner,
    epoch: epoch || 0,
    ft_data: { finetune_id: finetuneId, miner, ...progressData },
  }));
}

async function recordFinetuneJobComplete(finetuneId, miner, adapterCid, epoch) {
  return _persist(_entry({
    type: 'FINETUNE_JOB_COMPLETE',
    from: miner,
    epoch: epoch || 0,
    ft_data: { finetune_id: finetuneId, miner, adapter_cid: adapterCid },
  }));
}

async function recordFinetuneJobSettle(finetuneId, miner, buyer, settlementData, epoch) {
  return _persist(_entry({
    type: 'FINETUNE_JOB_SETTLE',
    from: miner,
    to: buyer,
    epoch: epoch || 0,
    ft_data: { finetune_id: finetuneId, miner, buyer, ...settlementData },
  }));
}

async function recordFinetuneJobRefund(finetuneId, buyer, reason, epoch) {
  return _persist(_entry({
    type: 'FINETUNE_JOB_REFUND',
    from: buyer,
    epoch: epoch || 0,
    ft_data: { finetune_id: finetuneId, buyer, reason: reason || 'expired' },
  }));
}

// ── Sessions (v3.1.121+) ──────────────────────────────────────────────────────

async function recordSessionCreate(buyer, sessionId, opts, epoch) {
  if (!buyer) throw new Error('buyer required');
  if (!sessionId) throw new Error('sessionId required');
  opts = opts || {};
  return _persist(_entry({
    type: 'SESSION_CREATE',
    from: buyer,
    epoch: epoch || 0,
    session_data: { session_id: sessionId, buyer, name: opts.name || null, initial_summary: opts.initialSummary || null },
  }));
}

async function recordSessionAddJob(sessionId, jobId, epoch) {
  if (!sessionId) throw new Error('sessionId required');
  if (!jobId) throw new Error('jobId required');
  return _persist(_entry({
    type: 'SESSION_ADD_JOB',
    epoch: epoch || 0,
    session_data: { session_id: sessionId, job_id: jobId },
  }));
}

async function recordSessionUpdateSummary(sessionId, summary, epoch) {
  if (!sessionId) throw new Error('sessionId required');
  return _persist(_entry({
    type: 'SESSION_UPDATE_SUMMARY',
    epoch: epoch || 0,
    session_data: { session_id: sessionId, summary },
  }));
}

// ── MCP Tool Registry (v3.1.121+) ─────────────────────────────────────────────

async function recordToolRegister(owner, toolData, epoch) {
  if (!owner) throw new Error('owner required');
  if (!toolData || !toolData.name) throw new Error('tool name required');
  return _persist(_entry({
    type: 'TOOL_REGISTER',
    from: owner,
    epoch: epoch || 0,
    tool_data: { ...toolData, owner },
  }));
}

async function recordToolUnregister(owner, toolName, epoch) {
  if (!toolName) throw new Error('toolName required');
  return _persist(_entry({
    type: 'TOOL_UNREGISTER',
    from: owner,
    epoch: epoch || 0,
    tool_data: { name: toolName, owner },
  }));
}

async function recordMinerCapability(miner, capabilities, epoch) {
  if (!miner) throw new Error('miner required');
  return _persist(_entry({
    type: 'MINER_CAPABILITY',
    from: miner,
    epoch: epoch || 0,
    capabilities,
  }));
}

async function recordInferenceJobToolCall(jobId, miner, toolCalls, turn, epoch) {
  if (!jobId) throw new Error('jobId required');
  if (!miner) throw new Error('miner required');
  if (!Array.isArray(toolCalls)) throw new Error('toolCalls must be an array');
  return _persist(_entry({
    type: 'INFERENCE_JOB_TOOL_CALL',
    from: miner,
    epoch: epoch || 0,
    job_data: { job_id: jobId, miner, tool_calls: toolCalls, turn: turn || 0, status: 'tool_pending' },
  }));
}

async function recordInferenceJobToolResult(jobId, buyer, toolResults, turn, epoch) {
  if (!jobId) throw new Error('jobId required');
  if (!buyer) throw new Error('buyer required');
  if (!Array.isArray(toolResults)) throw new Error('toolResults must be an array');
  return _persist(_entry({
    type: 'INFERENCE_JOB_TOOL_RESULT',
    from: buyer,
    epoch: epoch || 0,
    job_data: { job_id: jobId, buyer, tool_results: toolResults, turn: turn || 0, status: 'claimed' },
  }));
}

module.exports = {
  recordAccountCreate,
  recordTransfer,
  recordMiningReward,
  recordProjectRevenueSplit,
  recordModelUpload,
  recordFaucet,
  // Nested wallets (v3.3)
  recordWalletCreateChild,
  recordWalletUnnest,
  recordAccountAliasAssign,
  recordAccountAliasTransfer,
  recordTokenCreate,
  recordStake,
  recordUnstake,
  recordDelegate,
  recordUndelegate,
  recordInferenceCharge,
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
  applyRemoteEntries,
  appendForeignEntry,
  // Commerce (v2.10)
  recordStoreOpen,
  recordStoreUpdate,
  recordStoreClose,
  recordStakePurchase,
  recordProductCreate,
  recordProductUpdate,
  recordProductDelist,
  recordOrderPlace,
  recordOrderFulfill,
  recordOrderDelivered,
  recordOrderCancel,
  recordOrderDispute,
  recordReputationVote,
  commerce,
  // Private encrypted file storage (v3.1.110+)
  recordFileStore,
  recordFileGrant,
  recordFileRevoke,
  recordFileShard,
  // BTCPC-FS (v2.11+)
  recordBlobStoreCommit,
  recordBlobServeProof,
  recordStorageHeartbeat,
  // BTCPC-FS challenge-response (v2.11.2+, pay-for-delivery)
  recordBlobChallenge,
  recordBlobChallengeResponse,
  recordBlobChallengeResult,
  recordBlobChallengeTimeout,
  // Stateful compute (v2.14-beta)
  recordStatefulServiceDeploy,
  recordSnapshotCommit,
  recordSnapshotRestore,
  // IoT sensor + gateway (v2.15-beta)
  recordSensorRegister,
  recordSensorReading,
  recordSensorDataCommit,
  recordGatewayRegister,
  recordGatewayHeartbeat,
  // Bridge (v2.16-alpha)
  recordBridgeWrap,
  recordBridgeUnwrap,
  recordBridgeFund,
  recordBridgeUnlock,
  // Cross-chain monitor (v2.17)
  recordCrossChainActivity,
  // Key rotation (security)
  recordKeyRotate,
  // Device key delegation (v3.2) — auto-called on first device startup
  recordDeviceAuthorize,
  recordDeviceRevoke,
  // IoT device key registration (v3.2) — posting-key-authorized
  recordDeviceKeyRegister,
  // Amber Pill geo-pioneer NFT (v2.18)
  recordAmberPillMint,
  // Native tool use + MCP (v3.2)
  recordToolCapabilityRegister,
  recordToolTraceCommit,
  // Scientific compute (v3.2)
  recordScientificResult,
  // Name Auctions (v3.6)
  recordNameAuctionOpen,
  recordNameAuctionBid,
  recordNameAuctionSettle,
  recordNameAuctionCancel,
  recordNameDelegate,
  DEFAULT_AUCTION_DURATION_EPOCHS,
  recordIdentityLink,
  recordCrossChainClaim,
  recordStorageMigration,
  recordNodeReputationUpdate,
  // Storage settlement lag (v3.1.119+)
  recordStoragePayoutHold,
  recordStoragePayoutRelease,
  recordStoragePayoutForfeit,
  releaseMaturedStorageHolds,
  STORAGE_HOLD_EPOCHS,
  // btcpc_recycle redistribution (v3.1.119+)
  distributeRecyclePool,
  RECYCLE_DISTRIBUTION_INTERVAL_EPOCHS,
  RECYCLE_DISTRIBUTION_RATE,
  // Inference Marketplace (v3.1.119+)
  recordInferenceJobOpen,
  recordInferenceJobClaim,
  recordInferenceJobSubmit,
  recordInferenceJobSettle,
  recordInferenceJobRefund,
  recordInferenceJobToolCall,
  recordInferenceJobToolResult,
  // Browser / computer-use (v3.1.121+)
  recordBrowserJobOpen,
  recordBrowserJobClaim,
  recordBrowserJobScreenshot,
  recordBrowserJobAction,
  recordBrowserJobSettle,
  recordBrowserJobRefund,
  // Fine-tuning (v3.1.121+)
  recordFinetuneJobOpen,
  recordFinetuneJobClaim,
  recordFinetuneJobProgress,
  recordFinetuneJobComplete,
  recordFinetuneJobSettle,
  recordFinetuneJobRefund,
  // Sessions (v3.1.121+)
  recordSessionCreate,
  recordSessionAddJob,
  recordSessionUpdateSummary,
  // MCP Tool Registry (v3.1.121+)
  recordToolRegister,
  recordToolUnregister,
  recordMinerCapability,
};
