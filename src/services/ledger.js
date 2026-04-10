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

// Pending entries — collected during an epoch, written into the next block
const pendingEntries = [];

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
// Applies to stateStore (updates balances/accounts/tokens/stakes/escrows/etc.)
// and pushes into pendingEntries for inclusion in the next block.
function _persist(entry) {
  stateStore.applyEntry(entry);
  pendingEntries.push(entry);
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
 */
async function recordMiningReward(miner, amount, epoch) {
  const entry = _entry({
    type: 'MINING_REWARD',
    to: miner,
    token: 'BTCPC',
    amount,
    epoch,
  });
  return _persist(entry);
}

/**
 * Record a faucet distribution.
 */
async function recordFaucet(to, amount, epoch) {
  const entry = _entry({
    type: 'FAUCET',
    from: 'btcpc_genesis',
    to,
    token: 'BTCPC',
    amount,
    epoch,
  });
  return _persist(entry);
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
    memo: memo || 'escrow:' + requestId,
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
 */
async function recordNodeRegister(username, nodeType, p2pAddress, permissioned, epoch) {
  const entry = _entry({
    type: 'NODE_REGISTER',
    from: username,
    epoch: epoch || 0,
    memo: nodeType || 'clock',
    account_data: {
      username,
      node_type: nodeType || 'clock',
      p2p_address: p2pAddress || null,
      permissioned: !!permissioned,
    },
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
 */
function flushPendingEntries() {
  const entries = [...pendingEntries];
  pendingEntries.length = 0;
  return entries;
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
  applyRemoteEntries,
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
};
