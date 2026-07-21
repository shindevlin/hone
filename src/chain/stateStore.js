"use strict";

const HoneNativeExchange = require('./HoneNativeExchange');

/**
 * HONE State Store — in-memory chain state cache.

 * Shin Devlin
 *
 * Pure in-memory cache. No I/O. No Mongoose. The blockchain (block files on
 * disk) is the canonical source of truth. This module is the working view,
 * rebuilt by `replay.js` at startup and kept current by `ledger.recordX`
 * calls as new entries flow in.
 *
 * All reads by controllers, routes, miner, and explorer go through here.
 * Balance queries are O(1) Map lookups, not O(N) Mongo aggregations.
 *
 * Mutation is always via applyEntry(entry). Entries are the universal
 * event type — the same shape whether they come from replay, from the
 * local recordX, or from applyRemoteEntries gossip sync.
 *
 * Deterministic: same gossip → same state → same snapshot hash.
 */

// Balances: "username|token" → number. Track every token separately.
var balances = new Map();

// Account metadata: username → { created_epoch, public_keys, chain_addresses, heartbeat_epoch }
// Balance/staked/delegated/nonce live in stateManager SMT; we mirror them here for fast reads.
var accounts = new Map();

// Device key registry: device_pubkey → { owner, scope, hardware_hash, authorized_epoch, last_ip }
// Allows many physical devices to sign proposals on behalf of one account.
var deviceKeys = new Map();

// IoT device key registry (v3.2): device_id → { device_id, owner, device_pubkey, registered_epoch }
// device_id = "<owner>/<device-name>" (e.g. "josh/flipper-abc123")
// Authorized by the owner's posting key (cannot move funds).
var iotDeviceKeys = new Map();

// Token metadata: symbol → { name, symbol, supply, decimals, type, creator, created_epoch }
var tokens = new Map();

// NFTs: "collection|tokenId" → { owner, metadata, minted_epoch, transferable, soulbound, time_locked, unlock_epoch, evolving, metrics }
var nfts = new Map();

// Staking pool state: username → { total_staked, purpose, first_stake_epoch }
var stakes = new Map();

// Delegations: "from|to" → { amount, purpose, epoch, source: 'owned'|'delegated' }
var delegations = new Map();

// Delegated-received balances: "username|token" → units
// Tracks how much delegated network-use balance an account holds.
// Separate from balances — cannot be transferred or used in escrow.
var delegatedReceived = new Map();

// Escrows: requestId → { payer, amount, status, locked_epoch, released_to }
var escrows = new Map();

// Epoch metadata: epochNumber → { started_at, ended_at, block_reward, total_work, consensus_hash, status, rewards_distributed }
var epochs = new Map();

// Projects (PROJECT_CREATE ledger entries): name → { owner, repo_url, wallet_address, created_epoch }
// Note: API keys are NOT here — they live in ~/.hone/secrets.json
var projects = new Map();
var projectRevenueSplits = new Map(); // projectName → { splits: [{ account, percent }] }
var earnings = new Map(); // account → { mining, clock, storage, iot, verifier, total }
var seenEntries = new Set(); // deduplicate replayed entries
// Community model registry (v3.1.97+): model_id → { model_id, name, description, format,
//   size_mb, uploader, royalty_percent, stake_amount, files, status, upload_epoch }
var communityModels = new Map();
// Sponsored stakes: beneficiary → { sponsor, amount, sharePercent, epoch }
var sponsoredStakes = new Map();
// Mutable network policy (admin-settable, replayed from ledger)
var networkPolicy = { stakeFreeUntilStakers: 1000 };

// ─────────────────────────────────────────────────────────────────
// Commerce state (v2.10)
// ─────────────────────────────────────────────────────────────────
// Stores: seller → { name, banner_cid, description_cid, categories, stake_amount, stake_paid_usd, capacity, rep_score, rep_votes_up, rep_votes_down, total_sales, total_fulfilled, total_disputed, created_epoch, status }
// "capacity" is how many products the store can hold, bought via bonding curve.
var stores = new Map();
// Native Exchange Queue
var exchangeQueue = [];
var exchangeStats = { total_volume: 0, total_fees: 0 };


// Products: product_id → { store, seller, title, description_snippet, content_cid, category, price, token, stock, rep_score, rep_votes_up, rep_votes_down, total_sold, created_epoch, updated_epoch, status }
var products = new Map();

// Orders: order_id → { buyer, seller, product_id, quantity, unit_price, total, token, escrow_id, status, placed_epoch, fulfilled_epoch, delivered_epoch }
var orders = new Map();

// Inference marketplace jobs (v3.1.119+): job_id → { job_id, buyer, prompt, max_fee, model, system_prompt, ttl_epochs, expires_epoch, miner, proof_hash, status, open_epoch, claimed_epoch, settled_epoch }
var inferenceJobs = new Map();

// Sessions (v3.1.121+): session_id → { session_id, buyer, created_epoch, job_ids, summary, last_updated_epoch }
var sessions = new Map();

// Fine-tuning jobs (v3.1.121+): finetune_id → { finetune_id, buyer, dataset_cid, base_model, epochs, lora_rank, status, ... }
var finetuneJobs = new Map();

// Browser/computer-use jobs (v3.1.121+): job_id → { job_id, buyer, goal, start_url, status, turns, ... }
var browserJobs = new Map();

// Registered MCP tools (v3.1.121+): tool_name → { name, description, schema, webhook_url, owner, registered_epoch }
var toolRegistry = new Map();

// Miner capabilities (v3.1.125+): miner → { miner, models, vision, vision_model, audio, code_exec, browser, finetune, tiers, version, updated_epoch }
var minerCapabilities = new Map();

// Inference memory (v3.1.131+):
// userMemories: account → Map<memory_id, { memory_id, cid, tags, project_id, linked_memory_ids, created_epoch, size_bytes }>
// memoryProjects: account → Map<project_id, { project_id, name, memory_ids, created_epoch }>
var userMemories = new Map();
var memoryProjects = new Map();

// Storage payout holds (v3.1.119+): hold_id → { hold_id, host, cid, amount, hold_epoch, release_epoch, status }
var storagePayoutHolds = new Map();

// Reputation aggregates: "store|<id>" or "miner|<id>" or "product|<id>" → { score, votes_up, votes_down, completed, disputed, last_updated_epoch }
var reputation = new Map();

// Reputation votes (dedupe + recall): "voter|target_type|target_id" → { vote: +1|-1, weight, epoch, memo }
var reputationVotes = new Map();

// HONE-FS blobs (v2.11+): cid → { size, uploader, hosts[], committed_epoch, expires_epoch, payment_hone }
// Tracks which CIDs have been committed on chain. The actual bytes live in
// src/services/blobStore.js — this Map is the chain-level metadata only.
var blobs = new Map();

// Private encrypted file storage (v3.1.110+):
// storage_id → { owner, encrypted_manifest, wrapped_dek_owner, grants[], total_size, stored_epoch }
// Manifests are encrypted client-side; the chain only stores opaque ciphertext.
// grants[]: [{ grantee, wrapped_dek, expires_at }]
var storedFiles = new Map();

// Quantum-resistant split storage (v3.1.111+):
// shard_id → { owner, encrypted_ptr, created_epoch }
// Each entry holds ONE opaque ECIES-wrapped pointer (blob_cid + DEK share).
// Two entries per file (posting key wraps shard A, memo key wraps shard B).
// No on-chain field links the two — only the client's locally-held pair_id does.
var shardPtrs = new Map();

// HONE-FS storage heartbeats (v2.11.2+): host → {
//   last_heartbeat_epoch,
//   heartbeats: [{ epoch, cids: [...], capacity_used_gb }, ...],  // rolling window
//   total_heartbeats,
//   uptime_window_start,
// }
// Home-user-friendly durability signal: storage hosts send a small proof
// of liveness every N epochs, listing the CIDs they currently hold. This
// is the foundation for uptime-weighted payouts (v2.11.2+) and for the
// challenge protocol (verifiers pick a random host + CID from recent
// heartbeats to challenge).
var storageHeartbeats = new Map();

// Storage delivery tracking (v2.16+): epoch → Map<host, bytes_delivered>
// Set by blob-serving routes; used to score storage hosts proportional to delivery.
var storageDeliveryByEpoch = new Map();
var _currentStorageDeliveryEpoch = -1;

// Keep at most this many recent heartbeats per host to bound memory.
// 1000 epochs ≈ 8.3 hours at 30 sec epochs — enough history for uptime
// calculation windows without unbounded growth.
var STORAGE_HEARTBEAT_RETENTION = 1000;

// ─────────────────────────────────────────────────────────────────
// Node reputation (v3.1.118): unified reputation per account across all service types.
// account → {
//   storage:   { score, passed, failed, migrations, last_epoch }
//   mining:    { score, passed, failed, last_epoch }
//   sensor:    { score, passed, failed, last_epoch }
//   clock:     { score, passed, failed, last_epoch }
//   inference: { score, passed, failed, last_epoch }
//   review:    { score, passed, failed, last_epoch }
//   overall:   number (0–100, weighted average)
// }
// Score = exponential moving average of pass rate, 0–100.
// New accounts start at 50 (neutral). Score rises with good events, falls with bad.
var nodeReputation = new Map();

var REPUTATION_WEIGHTS = { storage: 0.3, mining: 0.3, sensor: 0.2, clock: 0.1, inference: 0.1 };

function _repScore(category) {
  return { score: 50, passed: 0, failed: 0, migrations: 0, last_epoch: 0 };
}

function _updateRepScore(current, passed) {
  // EMA: new_score = 0.9 * old_score + 0.1 * (100 if pass, 0 if fail)
  var updated = Object.assign({}, current);
  updated.score = Math.round(0.9 * (current.score || 50) + (passed ? 10 : 0));
  if (passed) updated.passed = (current.passed || 0) + 1;
  else updated.failed = (current.failed || 0) + 1;
  return updated;
}

function _getNodeRep(account) {
  return nodeReputation.get(account) || {
    storage: _repScore('storage'),
    mining: _repScore('mining'),
    sensor: _repScore('sensor'),
    clock: _repScore('clock'),
    inference: _repScore('inference'),
    review: _repScore('review'),
    overall: 50,
  };
}

function _computeOverall(rep) {
  var w = REPUTATION_WEIGHTS;
  return Math.round(
    rep.storage.score   * w.storage +
    rep.mining.score    * w.mining +
    rep.sensor.score    * w.sensor +
    rep.clock.score     * w.clock +
    rep.inference.score * w.inference
  );
}

function _applyRepEvent(account, category, passed, epoch, extra) {
  if (!account) return;
  var rep = _getNodeRep(account);
  if (!rep[category]) rep[category] = _repScore(category);
  rep[category] = _updateRepScore(rep[category], passed);
  rep[category].last_epoch = epoch || 0;
  if (extra) Object.assign(rep[category], extra);
  rep.overall = _computeOverall(rep);
  nodeReputation.set(account, rep);
}

// HONE-FS blob challenges (v2.11.2+): challenge_id → {
//   challenger, host, cid, byte_start, byte_length, expected_hash?,
//   issued_epoch, response_epoch, response_hash, status
// }
// Verifier-driven spot-checks of storage hosts. Records on chain for
// audit trail. See feedback_storage_no_slash.md:
// FAILURES ARE NOT SLASHED. Failed challenges reduce payout share for
// THIS commit and dip reputation. Stake is never touched.
var blobChallenges = new Map();

// Per-host challenge tally: host → {
//   total_issued, total_passed, total_failed, last_challenge_epoch
// }
// Drives the selector + reputation weighting. Recent failures push a
// host down the auto-selector ranking but don't slash anything.
var blobChallengeStats = new Map();

// ─────────────────────────────────────────────────────────────────
// IoT sensor + gateway state (v2.15-beta)
// ─────────────────────────────────────────────────────────────────
// sensors: sensor_id → { sensor_id, owner, type, unit, decimals, region, status, created_epoch, ... }
var sensors = new Map();

// sensorReadings: "<sensor_id>|<epoch>" → array of { value, metadata, submitted_at }
// Buffered for median consensus at finalization.
var sensorReadings = new Map();

// gateways: gateway_id → { gateway_id, owner, region, latitude, longitude, status, last_heartbeat_epoch, ... }
var gateways = new Map();

// gatewayHeartbeats: gateway_id → { epochs: Set<number>, last_heartbeat_epoch, total_heartbeats }
var gatewayHeartbeats = new Map();

// witnessMap: "<sensorId>|<epoch>" → Set of relay_account strings
// Tracks unique relay nodes that submitted a reading for a given sensor+epoch.
var witnessMap = new Map();

// sensorVouches: target_id → [{ voucher, target_id, target_type, stake_amount, epoch }]
// Tracks who vouches for which sensor/account and how much they have at stake.
var sensorVouches = new Map();

// ─────────────────────────────────────────────────────────────────
// Name Auctions (v3.6)
// nameAuctions: name → AuctionRecord
// AuctionRecord: {
//   name, seller, start_price_usd, min_bid_increment_usd,
//   auction_end_epoch, open_epoch,
//   bids: [ { bidder, bid_usd, chain, tx_hash, hone_account, hone_pubkeys, bid_epoch } ],
//   status: "open" | "settled" | "cancelled",
//   winner: null | bidder,
// }
// ─────────────────────────────────────────────────────────────────
var nameAuctions = new Map();
// nameDelegate: name → { delegated: bool, epoch }
var nameDelegate = new Map();

// Tier gates: index 0 = tier 1 (4+ char), ..., index 4 = tier 5 (1 char)
var AUCTION_TIER_GATES = [100, 250, 1000, 2500, 25000];

function _nameTier(name) {
  var len = String(name || "").length;
  if (len === 1) return 5;
  if (len === 2) return 4;
  if (len === 3) return 3;
  return 1; // 4+ char names: tier 1 (lowest gate)
}

// ─────────────────────────────────────────────────────────────────
// Device Stake & Rent (v3.5) — pooled staker model
// deviceStakerPools: device_id → Map<staker_account, StakerRecord>
// StakerRecord: { staker, stake_amount, original_stake, tribute_paid,
//                 slot, rent_mode, entry_epoch, rent_paid_total }
// Up to 10 stakers per device, ranked by stake_amount descending (slot 1 = highest).
// Yield split per epoch per device: 70% owner, 20% staker pool, 10% recycle.
// Slot multipliers weight each staker's share of the 20% pool.
// deviceAutoStake: device_id → auto_stake_pct (0-50)
// ─────────────────────────────────────────────────────────────────
var deviceStakerPools = new Map(); // device_id → Map<staker_account, StakerRecord>
var deviceAutoStake = new Map();   // device_id → pct (0-50)
var deviceStakerIndex = new Map(); // staker_account → Set<device_id>

var SLOT_MULTIPLIERS = [1.85, 1.60, 1.38, 1.19, 1.03, 0.89, 0.77, 0.67, 0.58, 0.50];
var MAX_STAKERS = 10;


// ─────────────────────────────────────────────────────────────────
// Bridge state (v2.16-alpha)
// ─────────────────────────────────────────────────────────────────
// bridgeWraps: "user|chainId|epoch" → { user, chainId, amount, fee }
// bridgeUnwraps: "user|chainId|epoch" → { user, chainId, amount, fee }
// bridgeFunders: "funder|chainId" → { funder, chainId, amount, lock_days, locked_epoch, status: 'locked'|'queued' }
var bridgeWraps = new Map();
var bridgeUnwraps = new Map();
var bridgeFunders = new Map();

// ─────────────────────────────────────────────────────────────────
// Cross-chain activity state (v2.17)
// ─────────────────────────────────────────────────────────────────
// crossChainActivity: username → [{ chain, address, activity_type, epoch, timestamp, ...data }]
// Populated by CROSS_CHAIN_ACTIVITY entries from the chain-monitor daemon.
// Bounded per-user to 1000 entries to prevent unbounded growth.
var crossChainActivity = new Map();
var CROSS_CHAIN_ACTIVITY_RETENTION = 1000;

// ─────────────────────────────────────────────────────────────────
// Cross-chain credits (v3.1.114): "account|chain" → unclaimed amount
// Miners accrue 0.1 HONE per supported chain per HONE earned on-chain.
// Consumed by CROSS_CHAIN_CLAIM entries when user claims wHONE on a chain.
var crossChainCredits = new Map();

// Canonical list of chains where wHONE exists. One credit per chain per reward.
var CROSS_CHAIN_SUPPORTED = [
  'ethereum', 'base', 'arbitrum', 'optimism',  // EVM (same address, separate contracts)
  'solana', 'ton', 'bitcoin',                   // non-EVM
  'hive', 'bsc', 'polygon',                     // additional
];

// ─────────────────────────────────────────────────────────────────
// Stateful compute state (v2.14-beta)
// ─────────────────────────────────────────────────────────────────
// services: slug → deployment record (stateful services tracked here)
// snapshots: slug → [{cid, epoch, replicas, timestamp}]  ordered oldest→newest
var services = new Map();
var snapshots = new Map();

// Mining proofs indexed by epoch
var miningProofsByEpoch = new Map();

// Compute proofs indexed by epoch
var computeProofsByEpoch = new Map();

// Resolved finalization consensus results indexed by epoch.
// Populated from FINALIZATION_CONSENSUS ledger entries so resolved epochs
// survive process restarts (consensus state used to live only in memory).
var finalizedConsensusByEpoch = new Map();

// ─────────────────────────────────────────────────────────────────
// Four-tier finality anchor log (v3.2)
// ─────────────────────────────────────────────────────────────────
// anchorLog: ring buffer of the last ANCHOR_LOG_CAP L2/ETH/BTC anchor
// entries written by finalityAnchoring.js.
// Entry shape: { type, epoch, state_root, anchor_level, anchored_at, ... }
var anchorLog = [];
var ANCHOR_LOG_CAP = 100;

// Slashing records: username → [ { epoch, offenseType, tier, amount, evidence } ]
var slashRecords = new Map();

// Inference review votes: job_id → [ { reviewer, verdict, epoch, review_mode, review_stage } ]
var inferenceReviewVotes = new Map();

// ─────────────────────────────────────────────────────────────────
// Nested wallet state (v3.3)
// parent/child namespace using "/" separator.
// childrenMap: parent → Set of child account names ("parent/child")
// parentMap:   child account name → parent account name
// ─────────────────────────────────────────────────────────────────
var childrenMap = new Map(); // parent → Set<childAccountName>
var parentMap = new Map();   // childAccountName → parent
var aliasMap = new Map();    // alias account name → canonical account name
var accountAliases = new Map(); // canonical account name → Set<alias account names>

// ─────────────────────────────────────────────────────────────────
// hone_recycle Phase 2 endowment state (v3.3)
// Activated automatically when total distributed supply >= 42,000,000 HONE.
// ─────────────────────────────────────────────────────────────────
var recycleRate = null;          // r — computed once at Phase 2 activation
var phase2Activated = false;     // true once Phase 2 is live
var phase2ActivationEpoch = null; // epoch at which Phase 2 first fired
var totalSupplyDistributed = 0;  // running sum of all non-recycle rewards issued

// Chain height: highest known finalized epoch
var chainHeight = -1;

// Current dynamic block cap (v3.0). Updated each epoch via setCurrentBlockCap.
// Initialized to DEFAULT_BLOCK_CAP (1 MB). Stored here so all nodes agree.
var currentBlockCap = 1 * 1024 * 1024; // 1 MB default

// Dedupe: entries we've already applied (by hash of canonical fields)
var seenEntries = new Set();
var SEEN_ENTRIES_CAP = 100000;

// ─────────────────────────────────────────────────────────────────
// Integer arithmetic helpers (10 decimal places = 1e10 units per HONE)
// ─────────────────────────────────────────────────────────────────
var UNITS_PER_HONE = 1e10; // 10 decimal places
function toUnits(hone) { return Math.round(hone * UNITS_PER_HONE); }
function fromUnits(units) { return units / UNITS_PER_HONE; }

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

function _balanceKey(username, token) {
  return username + "|" + (token || "HONE");
}

function _round(n) {
  return parseFloat(Number(n).toFixed(10));
}

function _coerceAmount(amount) {
  var parsed = typeof amount === "number" ? amount : Number(amount);
  if (!Number.isFinite(parsed) || parsed <= 0) return null;
  return parsed;
}

function _credit(username, token, amount) {
  var parsed = _coerceAmount(amount);
  if (!username || parsed === null) return;
  var key = _balanceKey(username, token);
  var currentUnits = balances.get(key) || 0;
  var addUnits = toUnits(parsed);
  if (!Number.isFinite(addUnits) || addUnits <= 0) return;
  balances.set(key, currentUnits + addUnits);
}

function _debit(username, token, amount) {
  var parsed = _coerceAmount(amount);
  if (!username || parsed === null) return false;
  var key = _balanceKey(username, token);
  var currentUnits = balances.get(key) || 0;
  var debitUnits = toUnits(parsed);
  if (!Number.isFinite(debitUnits) || debitUnits <= 0) return false;
  // System accounts may go negative; regular accounts cannot.
  if (currentUnits < debitUnits && !_isSystemAccount(username)) {
    return false; // insufficient balance — reject silently (Vuln 6 fix)
  }
  balances.set(key, currentUnits - debitUnits);
  return true;
}

function _creditDelegated(username, token, amount) {
  var parsed = _coerceAmount(amount);
  if (!username || parsed === null) return;
  var key = _balanceKey(username, token);
  var current = delegatedReceived.get(key) || 0;
  var creditUnits = toUnits(parsed);
  if (!Number.isFinite(creditUnits) || creditUnits <= 0) return;
  delegatedReceived.set(key, current + creditUnits);
}

function _debitDelegated(username, token, amount) {
  var parsed = _coerceAmount(amount);
  if (!username || parsed === null) return false;
  var key = _balanceKey(username, token);
  var current = delegatedReceived.get(key) || 0;
  var units = toUnits(parsed);
  if (!Number.isFinite(units) || units <= 0) return false;
  if (current < units) return false;
  delegatedReceived.set(key, current - units);
  return true;
}

function _ensureAccount(username, metadata) {
  if (!username) return;
  if (!accounts.has(username)) {
    accounts.set(username, {
      created_epoch: (metadata && metadata.epoch) || 0,
      public_keys: (metadata && metadata.public_keys) || {},
      chain_addresses: (metadata && metadata.chain_addresses) || {},
      heartbeat_epoch: 0,
    });
  }
}

function _bumpChallengeStat(host, outcome) {
  if (!host) return;
  var stats = blobChallengeStats.get(host) || {
    total_issued: 0,
    total_passed: 0,
    total_failed: 0,
    last_challenge_epoch: 0,
  };
  if (outcome === "passed") stats.total_passed += 1;
  else if (outcome === "failed") stats.total_failed += 1;
  blobChallengeStats.set(host, stats);
}

function _isSystemAccount(username) {
  if (!username) return false;
  return username === "hone_staking_pool" ||
         username === "hone_escrow" ||
         username === "hone_genesis" ||
         username === "hone_mint" ||
         username === "hone_recycle" ||
         username === "hone_treasury" ||
         username === "hone" ||
         username.startsWith("project:") ||
         username.startsWith("escrow:");
}

function _assertNonNegativeAmount(value, label) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    throw new Error("Corrupt negative value in " + label + ": " + value);
  }
}

function assertBalanceIntegrity(context) {
  var label = context || "stateStore";
  balances.forEach(function (amount, key) {
    _assertNonNegativeAmount(amount, label + " balance " + key);
  });
  delegatedReceived.forEach(function (amount, key) {
    _assertNonNegativeAmount(amount, label + " delegated balance " + key);
  });
  stakes.forEach(function (pool, username) {
    if (!pool) return;
    _assertNonNegativeAmount(pool.total_staked || 0, label + " stake pool " + username);
  });
}

/**
 * Hard guard: reject any attempt to set public_keys on hone_recycle.
 * This is a keyless protocol contract — no owner, no keys, ever.
 */
function _isRecycleKeyAttempt(entry) {
  var target = entry.to || entry.from;
  if (target !== "hone_recycle") return false;
  // Reject if the entry carries any key-setting payload
  var data = entry.account_data || entry.key_rotate_data || {};
  var keys = data.public_keys || data.new_public_keys || {};
  return Object.keys(keys).length > 0;
}

// Canonical dedupe hash for an entry
function _entryKey(entry) {
  // Domain-specific identifiers so multiple entries of the same type in the
  // same epoch/sender don't collide: product_id, order_id, vote target, etc.
  var domainId = "";
  if (entry.product_data && entry.product_data.product_id) {
    domainId = "p:" + entry.product_data.product_id;
  } else if (entry.order_data && entry.order_data.order_id) {
    domainId = "o:" + entry.order_data.order_id;
  } else if (entry.vote_data && entry.vote_data.target_type && entry.vote_data.target_id) {
    domainId = "v:" + entry.vote_data.target_type + ":" + entry.vote_data.target_id;
  } else if (entry.store_data && entry.store_data.action) {
    domainId = "s:" + entry.store_data.action;
  } else if (entry.type === "STORAGE_HEARTBEAT") {
    // Dedupe per host+epoch only — exclude timestamp so same-epoch duplicates collapse.
    return ["STORAGE_HEARTBEAT", entry.from || "", entry.epoch || 0].join("|");
  } else if (entry.type === "WALLET_CREATE_CHILD") {
    var wccParentId = (entry.wallet_data && entry.wallet_data.parent) || entry.from || "";
    var wccChildId = (entry.wallet_data && entry.wallet_data.child_name) || "";
    domainId = "wcc:" + wccParentId + "/" + wccChildId;
  } else if (entry.type === "DEVICE_YIELD_STAKE") {
    domainId = "dys:" + (entry.yield_stake_data && entry.yield_stake_data.device_id || "") + ":" + (entry.from || "") + ":" + (entry.epoch || 0) + ":" + (entry.amount || 0);
  } else if (entry.type === "DEVICE_YIELD_UNSTAKE") {
    domainId = "dyu:" + (entry.yield_stake_data && entry.yield_stake_data.device_id || "") + ":" + (entry.from || "") + ":" + (entry.epoch || 0);
  } else if (entry.type === "DEVICE_KEY_REGISTER") {
    domainId = "dkr:" + (entry.device_key_data && entry.device_key_data.device_id || "");
  } else if (entry.type === "SENSOR_REGISTER") {
    domainId = "sr:" + (entry.sensor_data && entry.sensor_data.sensor_id || "");
  } else if (entry.type === "SENSOR_KEY_REGISTER") {
    domainId = "skr:" + (entry.sensor_data && entry.sensor_data.sensor_id || "") + ":" + (entry.epoch || 0);
  } else if (entry.type === "SENSOR_VOUCH") {
    domainId = "sv:" + (entry.vouch_data && entry.vouch_data.voucher || "") + ":" + (entry.vouch_data && entry.vouch_data.target_id || "") + ":" + (entry.epoch || 0);
  } else if (entry.type === "SENSOR_READING") {
    domainId = "srd:" + (entry.sensor_data && entry.sensor_data.sensor_id || "") + ":" + (entry.epoch || 0) + ":" + (entry.sensor_data && entry.sensor_data.value !== undefined ? entry.sensor_data.value : "") + ":" + (entry.timestamp || 0);
  } else if (entry.type === "SENSOR_DATA_COMMIT") {
    domainId = "sdc:" + (entry.sensor_data && entry.sensor_data.cid || "");
  } else if (entry.type === "GATEWAY_REGISTER") {
    domainId = "gr:" + (entry.gateway_data && entry.gateway_data.gateway_id || "");
  } else if (entry.type === "GATEWAY_HEARTBEAT") {
    domainId = "gh:" + (entry.gateway_data && entry.gateway_data.gateway_id || "") + ":" + (entry.epoch || 0);
  } else if (entry.type === "CROSS_CHAIN_ACTIVITY" && entry.cross_chain_data) {
    // Each external chain observation is unique per (user, chain, address, timestamp)
    domainId = "cca:" +
      (entry.to || "") + ":" +
      (entry.cross_chain_data.chain || "") + ":" +
      (entry.cross_chain_data.address || "") + ":" +
      (entry.timestamp || 0);
  } else if (entry.type === "BRIDGE_WRAP" || entry.type === "BRIDGE_UNWRAP") {
    domainId = "bw:" + entry.type + ":" + (entry.from || "") + ":" + (entry.bridge_data && entry.bridge_data.chain_id || "") + ":" + (entry.epoch || 0) + ":" + (entry.timestamp || 0);
  } else if (entry.type === "BRIDGE_FUND") {
    domainId = "bf:" + (entry.from || "") + ":" + (entry.bridge_data && entry.bridge_data.chain_id || "") + ":" + (entry.epoch || 0);
  } else if (entry.type === "BRIDGE_UNLOCK") {
    domainId = "bu:" + (entry.from || "") + ":" + (entry.bridge_data && entry.bridge_data.chain_id || "") + ":" + (entry.epoch || 0);
  } else if (
    entry.challenge_data &&
    entry.challenge_data.challenge_id &&
    (entry.type === "BLOB_CHALLENGE" ||
     entry.type === "BLOB_CHALLENGE_RESPONSE" ||
     entry.type === "BLOB_CHALLENGE_RESULT" ||
     entry.type === "BLOB_CHALLENGE_TIMEOUT")
  ) {
    domainId = "bc:" + entry.type + ":" + entry.challenge_data.challenge_id;
  } else if (
    entry.auction_data && entry.auction_data.name &&
    (entry.type === "NAME_AUCTION_OPEN" ||
     entry.type === "NAME_AUCTION_BID" ||
     entry.type === "NAME_AUCTION_SETTLE" ||
     entry.type === "NAME_AUCTION_CANCEL")
  ) {
    // Auctions: unique per name + type + bidder (for bids) + epoch
    var auctionBidder = (entry.type === "NAME_AUCTION_BID" && entry.auction_data.bidder) ? ":" + entry.auction_data.bidder + ":" + (entry.auction_data.bid_usd || 0) : "";
    domainId = "na:" + entry.type + ":" + entry.auction_data.name + auctionBidder + ":" + (entry.epoch || 0);
  } else if (entry.blob_data && entry.blob_data.cid) {
    // Serve proofs can repeat per-epoch per-host; include bytes_served
    // + timestamp so multiple proofs in one epoch aren't deduped.
    if (entry.type === "BLOB_SERVE_PROOF") {
      domainId =
        "bsp:" +
        entry.blob_data.cid +
        ":" +
        (entry.blob_data.bytes_served || 0) +
        ":" +
        (entry.blob_data.access_log_merkle_root || "");
    } else {
      domainId = "b:" + entry.blob_data.cid;
    }
  }
  return [
    entry.type || "",
    entry.from || "",
    entry.to || "",
    entry.amount || 0,
    entry.epoch || 0,
    entry.token || "",
    entry.memo || "",
    entry.timestamp || 0,
    domainId,
  ].join("|");
}

// ─────────────────────────────────────────────────────────────────
// Core mutator: applyEntry
// ─────────────────────────────────────────────────────────────────

/**
 * Apply a single ledger entry to the state.
 * Dispatches on entry.type. Idempotent: same entry applied twice is a no-op.
 *
 * This is the ONLY way state gets mutated. Both replay (from disk) and
 * live writes (recordX from ledger.js) flow through here.
 */
function applyEntry(entry) {
  if (!entry || !entry.type) return;

  // Dedupe: skip if we've already applied this exact entry
  var key = _entryKey(entry);
  if (seenEntries.has(key)) return;
  seenEntries.add(key);
  if (seenEntries.size > SEEN_ENTRIES_CAP) {
    // Cheap bounded cache: drop oldest ~1/4 when full
    var toDelete = Math.floor(SEEN_ENTRIES_CAP / 4);
    var iter = seenEntries.values();
    for (var i = 0; i < toDelete; i++) seenEntries.delete(iter.next().value);
  }

  // Hard guard: never let anything set keys on hone_recycle
  if (_isRecycleKeyAttempt(entry)) return;

  var from = entry.from;
  var to = entry.to;
  var amount = entry.amount || 0;
  var parsedAmount = _coerceAmount(amount);
  var token = entry.token || "HONE";

  switch (entry.type) {
    case "ACCOUNT_CREATE":
      _ensureAccount(to, {
        epoch: entry.epoch,
        public_keys: entry.account_data && entry.account_data.public_keys,
        chain_addresses: entry.account_data && entry.account_data.chain_addresses,
      });
      break;

    // ── Nested wallet (v3.3) ──────────────────────────────────────────
    case "WALLET_CREATE_CHILD": {
      var wccData = entry.wallet_data || {};
      var wccParent = wccData.parent || from;
      var wccChildName = wccData.child_name;
      if (!wccParent || !wccChildName) break;
      // Reject if child_name contains a "/" (no grandchildren yet)
      if (String(wccChildName).indexOf("/") !== -1) break;
      var wccChild = wccParent + "/" + wccChildName;
      // Idempotent: if already exists, skip
      if (accounts.has(wccChild)) break;
      _ensureAccount(wccChild, { epoch: entry.epoch, public_keys: {}, chain_addresses: {} });
      // Register parent–child links
      parentMap.set(wccChild, wccParent);
      var wccSiblings = childrenMap.get(wccParent) || new Set();
      wccSiblings.add(wccChild);
      childrenMap.set(wccParent, wccSiblings);
      break;
    }

    case "WALLET_UNNEST": {
      var wuData = entry.wallet_data || {};
      var wuParent = wuData.parent || from;
      var wuNestedName = wuData.nested_name || wuData.child_name;
      var wuNested = wuData.nested_wallet || (wuParent && wuNestedName ? wuParent + "/" + wuNestedName : "");
      var wuNative = wuData.native_name || to;
      if (!wuParent || !wuNested || !wuNative) break;
      if (String(wuNative).indexOf("/") !== -1) break;
      if (!/^[a-z0-9][a-z0-9._-]{0,19}$/.test(String(wuNative))) break;
      if (!accounts.has(wuNested)) break;
      if (accounts.has(wuNative)) break;
      if (parentMap.get(wuNested) !== wuParent) break;

      _ensureAccount(wuNative, {
        epoch: entry.epoch,
        public_keys: wuData.public_keys || {},
        chain_addresses: wuData.chain_addresses || {},
      });

      for (var wuBalEntry of Array.from(balances.entries())) {
        if (wuBalEntry[0].indexOf(wuNested + "|") === 0) {
          var wuToken = wuBalEntry[0].slice((wuNested + "|").length);
          var wuNativeKey = _balanceKey(wuNative, wuToken);
          balances.set(wuNativeKey, (balances.get(wuNativeKey) || 0) + wuBalEntry[1]);
          balances.delete(wuBalEntry[0]);
        }
      }
      for (var wuDelEntry of Array.from(delegatedReceived.entries())) {
        if (wuDelEntry[0].indexOf(wuNested + "|") === 0) {
          var wuDelToken = wuDelEntry[0].slice((wuNested + "|").length);
          var wuDelNativeKey = _balanceKey(wuNative, wuDelToken);
          delegatedReceived.set(wuDelNativeKey, (delegatedReceived.get(wuDelNativeKey) || 0) + wuDelEntry[1]);
          delegatedReceived.delete(wuDelEntry[0]);
        }
      }
      if (stakes.has(wuNested)) {
        stakes.set(wuNative, stakes.get(wuNested));
        stakes.delete(wuNested);
      }

      accounts.delete(wuNested);
      parentMap.delete(wuNested);
      var wuSiblings = childrenMap.get(wuParent);
      if (wuSiblings) {
        wuSiblings.delete(wuNested);
        childrenMap.set(wuParent, wuSiblings);
      }
      break;
    }

    case "IDENTITY_LINK": {
      // Formal on-chain attestation that a HONE account owns specific addresses
      // on external chains. Signed with the owner key. Updates chain_addresses in
      // place and records the epoch of the attestation for audit purposes.
      var ilAccount = from || to;
      if (!ilAccount || !accounts.has(ilAccount)) break;
      var ilData = entry.identity_data || {};
      var ilAddresses = ilData.chain_addresses || {};
      var ilAcc = accounts.get(ilAccount);
      // Merge new addresses — existing addresses are overwritten per key, others preserved
      var merged = Object.assign({}, ilAcc.chain_addresses || {}, ilAddresses);
      ilAcc.chain_addresses = merged;
      ilAcc.identity_linked_epoch = entry.epoch;
      accounts.set(ilAccount, ilAcc);
      break;
    }

    case "CROSS_CHAIN_CREDIT": {
      // Accrues claimable wHONE on each supported chain alongside mining rewards.
      var cccAccount = to || from;
      var cccChain = entry.chain;
      var cccAmount = parsedAmount;
      if (!cccAccount || !cccChain || cccAmount === null) break;
      var cccKey = cccAccount + '|' + cccChain;
      crossChainCredits.set(cccKey, (crossChainCredits.get(cccKey) || 0) + cccAmount);
      break;
    }

    case "CROSS_CHAIN_CLAIM": {
      // Consumes accumulated credit when user claims wHONE on a chain.
      var clAccount = from || to;
      var clChain = entry.chain;
      var clAmount = parsedAmount;
      if (!clAccount || !clChain || clAmount === null) break;
      var clKey = clAccount + '|' + clChain;
      var clBal = crossChainCredits.get(clKey) || 0;
      crossChainCredits.set(clKey, Math.max(0, clBal - clAmount));
      break;
    }

    case "ACCOUNT_ALIAS_ASSIGN": {
      var aaData = entry.alias_data || {};
      var aaParent = aaData.parent || from;
      var aaAlias = aaData.alias || to;
      var aaTarget = aaData.target_account;
      var aaNested = aaData.nested_wallet || (aaParent && aaAlias ? aaParent + "/" + aaAlias : "");
      if (!aaParent || !aaAlias || !aaTarget || !aaNested) break;
      if (String(aaAlias).indexOf("/") !== -1) break;
      if (!/^[a-z0-9][a-z0-9._-]{0,19}$/.test(String(aaAlias))) break;
      if (!accounts.has(aaTarget)) break;
      if (accounts.has(aaAlias)) break;
      if (aliasMap.has(aaAlias)) break;
      if (!accounts.has(aaNested)) break;
      if (parentMap.get(aaNested) !== aaParent) break;

      aliasMap.set(aaAlias, aaTarget);
      var aaAliases = accountAliases.get(aaTarget) || new Set();
      aaAliases.add(aaAlias);
      accountAliases.set(aaTarget, aaAliases);

      accounts.delete(aaNested);
      parentMap.delete(aaNested);
      var aaSiblings = childrenMap.get(aaParent);
      if (aaSiblings) {
        aaSiblings.delete(aaNested);
        childrenMap.set(aaParent, aaSiblings);
      }
      break;
    }

    case "ACCOUNT_ALIAS_TRANSFER": {
      var atData = entry.alias_data || {};
      var atAlias = atData.alias || from;
      var atFrom = atData.from_account || aliasMap.get(atAlias);
      var atTo = atData.to_account || to;
      if (!atAlias || !atFrom || !atTo) break;
      if (!aliasMap.has(atAlias)) break;
      if (aliasMap.get(atAlias) !== atFrom) break;
      if (!accounts.has(atTo)) break;

      aliasMap.set(atAlias, atTo);
      var atFromAliases = accountAliases.get(atFrom);
      if (atFromAliases) {
        atFromAliases.delete(atAlias);
        accountAliases.set(atFrom, atFromAliases);
      }
      var atToAliases = accountAliases.get(atTo) || new Set();
      atToAliases.add(atAlias);
      accountAliases.set(atTo, atToAliases);
      break;
    }

    case "TRANSFER":
      // Integer-unit validation: amount must convert to a positive integer
      if (parsedAmount === null) break;
      if (!_debit(from, token, parsedAmount)) break; // insufficient balance — delegated balance NOT eligible
      _credit(to, token, parsedAmount);
      break;

    case "SLASH":
      if (parsedAmount === null) break;
      if (!from || !to) break;
      if (!_debitStake(from, parsedAmount)) break;
      _credit(to, token || "HONE", parsedAmount);
      if (entry.slash_data && entry.slash_data.account) {
        addSlashRecord(entry.slash_data.account, {
          account: entry.slash_data.account,
          role: entry.slash_data.role || "verifier",
          offenseType: entry.slash_data.offenseType || "REVIEW_DISSENT",
          tier: entry.slash_data.tier || 0,
          amount: parsedAmount,
          evidence: entry.slash_data.evidence || null,
          slashTxId: entry._id ? entry._id.toString() : null,
          deregistered: !!entry.slash_data.deregistered,
          epoch: entry.epoch || 0,
          timestamp: entry.timestamp || new Date().toISOString(),
          appeal: entry.slash_data.appeal || { deadline: (entry.epoch || 0) + 100 },
        });
      }
      break;

    case "INFERENCE_CHARGE":
      // Deduct network service fee from delegated balance first, then owned balance.
      // to = miner/pool that receives payment; from = requester
      if (parsedAmount === null) break;
      var chargeToken = token || "HONE";
      var chargedFromDelegated = _debitDelegated(from, chargeToken, parsedAmount);
      if (!chargedFromDelegated) {
        // fall back to owned balance
        if (!_debit(from, chargeToken, parsedAmount)) break;
      }
      _credit(to, chargeToken, parsedAmount);
      break;

    case "SPONSOR_STAKE": {
      var spAmt = _coerceAmount(amount);
      if (spAmt === null || !from || !to) break;
      if (!_debit(from, "HONE", spAmt)) break;
      sponsoredStakes.set(to, { sponsor: from, amount: spAmt, sharePercent: entry.share_percent || 15, epoch: entry.epoch });
      break;
    }

    case "FINALIZATION_CONSENSUS": {
      var cd = entry.consensus_data;
      if (!cd || typeof cd.epoch_number !== "number") break;
      finalizedConsensusByEpoch.set(cd.epoch_number, {
        proposer: cd.proposer || null,
        consensus_hash: cd.consensus_hash || null,
        total_work: cd.total_work || 0,
        consensus_nodes: cd.consensus_nodes || 0,
        consensus_proposals: cd.consensus_proposals || 0,
        rewards: Array.isArray(cd.rewards) ? cd.rewards : [],
        timestamp: entry.timestamp || 0
      });
      break;
    }

    case "NETWORK_POLICY": {
      if (typeof entry.stake_free_until_stakers === 'number') {
        networkPolicy.stakeFreeUntilStakers = entry.stake_free_until_stakers;
      }
      break;
    }

    case "MINING_REWARD":
    case "FAUCET":
      _credit(to, token, amount);
      // Route sponsor cut before tracking own earnings
      if (entry.type === "MINING_REWARD" && to && token === "HONE") {
        var spRec = sponsoredStakes.get(to);
        if (spRec && spRec.sponsor && spRec.sharePercent > 0) {
          var spCut = _round(amount * spRec.sharePercent / 100);
          if (spCut > 0 && _debit(to, "HONE", spCut)) {
            _credit(spRec.sponsor, "HONE", spCut);
            var spE = earnings.get(spRec.sponsor) || { mining: 0, clock: 0, storage: 0, iot: 0, verifier: 0, service: 0, inference_split: 0, total: 0 };
            spE.service = _round(spE.service + spCut);
            spE.total   = _round(spE.total   + spCut);
            earnings.set(spRec.sponsor, spE);
          }
        }
      }
      // Track earnings by source for the earnings breakdown endpoint
      if (entry.type === "MINING_REWARD" && to) {
        var earningRecord = earnings.get(to) || { mining: 0, clock: 0, storage: 0, iot: 0, verifier: 0, service: 0, inference_split: 0, total: 0 };
        var rewardSource = entry.reward_source || "mining";
        if (earningRecord[rewardSource] !== undefined) {
          earningRecord[rewardSource] = _round(earningRecord[rewardSource] + amount);
        } else {
          earningRecord.mining = _round(earningRecord.mining + amount);
        }
        earningRecord.total = _round(earningRecord.total + amount);
        earnings.set(to, earningRecord);
        // Each accepted mining reward is a passed reputation event
        _applyRepEvent(to, 'mining', true, entry.epoch);
        // Track total supply distributed (excludes recycle account)
        if (to !== "hone_recycle") {
          totalSupplyDistributed = _round(totalSupplyDistributed + amount);
        }
        // Auto-accrue cross-chain credits (0.1 HONE per chain per HONE earned).
        // Runs for both new blocks (no companion CROSS_CHAIN_CREDIT entry yet) and
        // historical blocks replayed from disk, giving all miners retroactive credit.
        var ccCreditAmount = _round(amount * 0.1);
        if (ccCreditAmount > 0) {
          var ccChains = CROSS_CHAIN_SUPPORTED;
          for (var ci = 0; ci < ccChains.length; ci++) {
            var ccKey = to + '|' + ccChains[ci];
            crossChainCredits.set(ccKey, _round((crossChainCredits.get(ccKey) || 0) + ccCreditAmount));
          }
        }
      }
      break;

    case "TOKEN_CREATE":
      if (entry.token_data) {
        var td = entry.token_data;
        tokens.set(td.symbol, {
          name: td.name,
          symbol: td.symbol,
          supply: td.supply || 0,
          decimals: td.decimals || 0,
          type: td.type || "fungible",
          creator: from,
          created_epoch: entry.epoch,
        });
        // Credit full supply to creator if fungible
        if (td.type === "fungible" && td.supply > 0 && from) {
          _credit(from, td.symbol, td.supply);
        }
      }
      break;

    case "STAKE":
      if (_coerceAmount(amount) === null) break;
      if (!_debit(from, "HONE", amount)) break;
      if (from) {
        var s = stakes.get(from) || { total_staked: 0, purpose: entry.memo, first_stake_epoch: entry.epoch };
        s.total_staked = _round(s.total_staked + amount);
        stakes.set(from, s);
      }
      break;

    case "UNSTAKE":
      if (to) {
        var us = stakes.get(to);
        if (!us || parsedAmount === null || toUnits(us.total_staked) < toUnits(parsedAmount)) break;
        _credit(to, "HONE", parsedAmount);
        us.total_staked = _round(us.total_staked - parsedAmount);
        if (us.total_staked <= 0) stakes.delete(to);
        else stakes.set(to, us);
      }
      break;

    case "DELEGATE": {
      // Multi-layer delegation: draw from owned balance first, then delegated balance.
      // recipient always receives into their delegatedReceived pool (network-use only).
      var delegSource = 'owned';
      if (parsedAmount === null) break;
      var ownedOk = _debit(from, "HONE", parsedAmount);
      if (!ownedOk) {
        // from doesn't have enough owned — try re-delegating from their received pool
        var delegOk = _debitDelegated(from, "HONE", parsedAmount);
        if (delegOk) delegSource = 'delegated';
        // if neither succeeds, the entry is a no-op (silently rejected)
      }
      if (from && to && (ownedOk || delegSource === 'delegated')) {
        _creditDelegated(to, "HONE", parsedAmount);
        var dkey = from + "|" + to;
        var d = delegations.get(dkey) || { amount: 0, purpose: (entry.delegation_data && entry.delegation_data.purpose) || entry.memo, epoch: entry.epoch, source: delegSource };
        d.amount = _round(d.amount + parsedAmount);
        d.source = delegSource;
        delegations.set(dkey, d);
      }
      break;
    }

    case "UNDELEGATE": {
      // Return tokens: if delegation was sourced from owned balance → restore to owned.
      // If sourced from delegated pool (re-delegation) → restore to delegator's delegatedReceived.
      var ddata = entry.delegation_data || {};
      var delegator2 = ddata.delegator || from;
      var miner2 = ddata.miner || to;
      var returnTo = ddata.return_to || to; // who gets tokens back
      if (from && to && parsedAmount !== null) {
        var dkey2 = delegator2 + "|" + miner2;
        var d2 = delegations.get(dkey2);
        var delegSrc2 = (d2 && d2.source) || 'owned';
        if (delegSrc2 === 'delegated') {
          _creditDelegated(returnTo, "HONE", parsedAmount);
        } else {
          _credit(returnTo, "HONE", parsedAmount);
        }
        // Debit the undelegated amount from recipient's delegatedReceived
        _debitDelegated(miner2, "HONE", parsedAmount);
        if (d2) {
          d2.amount = _round(d2.amount - parsedAmount);
          if (d2.amount <= 0) delegations.delete(dkey2);
          else delegations.set(dkey2, d2);
        }
      }
      break;
    }

    case "ESCROW_LOCK": {
      var escrowSource = "owned";
      if (parsedAmount === null) break;
      var escrowPaid = _debitDelegated(from, "HONE", parsedAmount);
      if (escrowPaid) {
        escrowSource = "delegated";
      } else {
        escrowPaid = _debit(from, "HONE", parsedAmount);
      }
      if (!escrowPaid) break;
      if (entry.memo) {
        // memo is usually "escrow:request_id"
        var rid = entry.memo.startsWith("escrow:") ? entry.memo.slice(7) : entry.memo;
        escrows.set(rid, {
          payer: from,
          amount: parsedAmount,
          source: escrowSource,
          status: "locked",
          locked_epoch: entry.epoch,
          released_to: null,
        });
      }
      break;
    }

    case "ESCROW_RELEASE":
      if (entry.escrow_id || entry.memo) {
        var rid2 = entry.escrow_id || (entry.memo.startsWith("escrow:") ? entry.memo.slice(7) : entry.memo);
        var e2 = escrows.get(rid2);
        if (e2) {
          var alreadyReleased = e2.released_amount || 0;
          if (parsedAmount === null) break;
          if (toUnits(alreadyReleased + parsedAmount) > toUnits(e2.amount)) break;
          _credit(to, "HONE", parsedAmount);
          e2.status = "released";
          e2.released_to = to;
          e2.released_amount = _round(alreadyReleased + parsedAmount);
          escrows.set(rid2, e2);
        } else if (!entry.escrow_id && entry.memo && !entry.memo.startsWith("escrow:")) {
          // Legacy release entries sometimes used memo for a human payout note
          // instead of escrow identity. Preserve historical replay behavior.
          _credit(to, "HONE", parsedAmount);
        }
      } else {
        _credit(to, "HONE", parsedAmount);
      }
      break;

    case "ESCROW_REFUND":
      if (entry.memo) {
        var rid3 = entry.memo.startsWith("escrow:") ? entry.memo.slice(7) : entry.memo;
        var e3 = escrows.get(rid3);
        if (e3) {
          if (e3.source === "delegated") _creditDelegated(to, "HONE", parsedAmount);
          else _credit(to, "HONE", parsedAmount);
          e3.status = "refunded";
          escrows.set(rid3, e3);
        } else {
          _credit(to, "HONE", parsedAmount);
        }
      } else {
        _credit(to, "HONE", parsedAmount);
      }
      break;

    case "NODE_REGISTER":
      // nodeRegistry handles its own state, but we track existence here too.
      // v2.10.2: accept both legacy account_data.node_type (string) and the
      // new account_data.node_types (array) for multi-capability nodes.
      // A single account can declare multiple roles: miner + verifier +
      // storage_host + gateway_op + sensor_bridge, etc. Capability-specific
      // reward pools pay each role independently.
      _ensureAccount(from || to);
      if (from && accounts.has(from) && entry.account_data) {
        var acct = accounts.get(from);
        var declaredTypes = entry.account_data.node_types;
        if (!Array.isArray(declaredTypes) && entry.account_data.node_type) {
          declaredTypes = [entry.account_data.node_type];
        }
        if (Array.isArray(declaredTypes) && declaredTypes.length > 0) {
          // Normalize + dedupe
          var unique = {};
          for (var ni = 0; ni < declaredTypes.length; ni++) {
            var t = String(declaredTypes[ni] || "").trim().toLowerCase();
            if (t) unique[t] = true;
          }
          acct.node_types = Object.keys(unique);
        }
        // Optional capacity fields (advertised by the node for specific roles)
        if (entry.account_data.storage_capacity_gb !== undefined) {
          acct.storage_capacity_gb = Number(entry.account_data.storage_capacity_gb) || 0;
        }
        if (entry.account_data.service_capacity) {
          acct.service_capacity = entry.account_data.service_capacity;
        }
        if (entry.account_data.lora_region) {
          acct.lora_region = String(entry.account_data.lora_region).toUpperCase();
        }
        if (entry.account_data.p2p_address) {
          acct.p2p_address = entry.account_data.p2p_address;
        }
        if (entry.account_data.permissioned !== undefined) {
          acct.permissioned = !!entry.account_data.permissioned;
        }
        acct.last_registered_epoch = entry.epoch;
        accounts.set(from, acct);
      }
      break;

    case "NODE_ANNOUNCE":
      if (from && accounts.has(from) && entry.account_data) {
        var naAcct = accounts.get(from);
        if (entry.account_data.p2p_address) {
          naAcct.p2p_address = entry.account_data.p2p_address;
        }
        if (Array.isArray(entry.account_data.node_types) && entry.account_data.node_types.length > 0) {
          var naUnique = {};
          for (var nai = 0; nai < entry.account_data.node_types.length; nai++) {
            var naT = String(entry.account_data.node_types[nai] || "").trim().toLowerCase();
            if (naT) naUnique[naT] = true;
          }
          naAcct.node_types = Object.keys(naUnique);
        }
        if (entry.account_data.permissioned !== undefined) {
          naAcct.permissioned = !!entry.account_data.permissioned;
        }
        accounts.set(from, naAcct);
      }
      break;

    case "HEARTBEAT":
      if (from && accounts.has(from)) {
        var acc = accounts.get(from);
        acc.heartbeat_epoch = entry.epoch;
        accounts.set(from, acc);
      }
      break;

    case "PROJECT_CREATE":
      if (entry.account_data && entry.account_data.name) {
        projects.set(entry.account_data.name, {
          owner: entry.account_data.owner || from,
          repo_url: entry.account_data.repo_url || "",
          wallet_address: entry.account_data.wallet_address || from,
          created_epoch: entry.epoch,
        });
      }
      break;

    case "PROJECT_REVENUE_SPLIT":
      // Set revenue split configuration for a project.
      // split_data: { project: "name", splits: [{ account, percent }] }
      if (entry.split_data && entry.split_data.project) {
        var spd = entry.split_data;
        projectRevenueSplits.set(spd.project, {
          splits: Array.isArray(spd.splits) ? spd.splits.slice() : [],
          set_epoch: entry.epoch,
          set_by: from,
        });
      }
      break;

    // ── Commerce (v2.10) ───────────────────────────────────────────
    case "STORE_OPEN":
      // Opens a storefront for `from`. Requires stake_amount locked via
      // STAKE entry in the same block (enforced at recordX level, not here).
      if (entry.store_data && from) {
        var sd = entry.store_data;
        stores.set(from, {
          seller: from,
          name: sd.name || from,
          banner_cid: sd.banner_cid || null,
          description_cid: sd.description_cid || null,
          categories: sd.categories || [],
          stake_amount: _round(sd.stake_amount || 0),
          stake_paid_usd: _round(sd.stake_paid_usd || 0),
          capacity: sd.capacity || 0,
          rep_score: 0,
          rep_votes_up: 0,
          rep_votes_down: 0,
          total_sales: 0,
          total_fulfilled: 0,
          total_disputed: 0,
          created_epoch: entry.epoch,
          status: "active",
        });
      }
      break;

    case "STORE_UPDATE":
      if (entry.store_data && from) {
        var existingStore = stores.get(from);
        if (existingStore) {
          var su = entry.store_data;
          if (su.name !== undefined) existingStore.name = su.name;
          if (su.banner_cid !== undefined) existingStore.banner_cid = su.banner_cid;
          if (su.description_cid !== undefined) existingStore.description_cid = su.description_cid;
          if (su.categories !== undefined) existingStore.categories = su.categories;
          // Capacity increase via STAKE_PURCHASE only, not here.
          stores.set(from, existingStore);
        }
      }
      break;

    case "STORE_CLOSE":
      if (from && stores.has(from)) {
        var closingStore = stores.get(from);
        closingStore.status = "closed";
        closingStore.closed_epoch = entry.epoch;
        stores.set(from, closingStore);
        // Delist all products from this seller
        for (var prodEntry of products) {
          if (prodEntry[1].seller === from && prodEntry[1].status === "active") {
            prodEntry[1].status = "delisted";
            products.set(prodEntry[0], prodEntry[1]);
          }
        }
      }
      break;

    case "STAKE_PURCHASE":
      // Buyer pays in stable token (wUSDC/wUSDT/wDAI) → store capacity increases.
      // Requires active store. The stable token debit happens via a paired TRANSFER
      // entry at record time. This entry records the capacity delta.
      if (entry.store_data && from) {
        var pStore = stores.get(from);
        if (pStore) {
          var capDelta = entry.store_data.capacity_delta || 0;
          var usdDelta = entry.store_data.stake_paid_usd || 0;
          var stakeDelta = entry.store_data.stake_amount || 0;
          pStore.capacity = (pStore.capacity || 0) + capDelta;
          pStore.stake_amount = _round((pStore.stake_amount || 0) + stakeDelta);
          pStore.stake_paid_usd = _round((pStore.stake_paid_usd || 0) + usdDelta);
          stores.set(from, pStore);
        }
      }
      break;

    // ── Native Exchange (v3.4) ───────────────────────────────────────
    case "EXCHANGE_DEPOSIT":
      if (from && parsedAmount > 0 && token === "HONE") {
        // Lock HONE in exchange system account
        if (!_debit(from, "HONE", parsedAmount)) break;
        _credit("hone_exchange", "HONE", parsedAmount);
        // Add to FIFO queue
        exchangeQueue.push({
          seller: from,
          amount: parsedAmount,
          epoch: entry.epoch
        });
      }
      break;

    case "EXCHANGE_BUY":
      // stablecoin amount and token in entry.stable_data
      if (from && entry.stable_data && entry.stable_data.amount > 0) {
        HoneNativeExchange.applyExchangeBuy({
          buyer: from,
          token: entry.stable_data.token,
          amount: entry.stable_data.amount,
          epoch: entry.epoch
        }, {
          balances,
          tokenBalances,
          exchangeQueue,
          exchangeStats,
          oracleFeeds: require('../services/oracleFeeds'),
          _round,
          _debit,
          _credit
        });
      }
      break;



    case "PRODUCT_CREATE":
      if (entry.product_data && entry.product_data.product_id && from) {
        var pd = entry.product_data;
        var sellerStore = stores.get(from);
        // Chain invariant: seller must have an active store with remaining capacity
        var activeCount = 0;
        for (var pe of products) {
          if (pe[1].seller === from && pe[1].status === "active") activeCount++;
        }
        if (!sellerStore || sellerStore.status !== "active") break;
        if (activeCount >= (sellerStore.capacity || 0)) break;

        products.set(pd.product_id, {
          product_id: pd.product_id,
          store: from,
          seller: from,
          title: pd.title || "",
          description_snippet: String(pd.description_snippet || "").slice(0, 256),
          content_cid: pd.content_cid || null,
          category: pd.category || "uncategorized",
          price: _round(pd.price || 0),
          token: pd.token || "HONE",
          stock: pd.stock || 0,
          rep_score: 0,
          rep_votes_up: 0,
          rep_votes_down: 0,
          total_sold: 0,
          created_epoch: entry.epoch,
          updated_epoch: entry.epoch,
          status: "active",
        });
      }
      break;

    case "PRODUCT_UPDATE":
      if (entry.product_data && entry.product_data.product_id) {
        var pdu = entry.product_data;
        var existingProduct = products.get(pdu.product_id);
        if (existingProduct && existingProduct.seller === from) {
          if (pdu.title !== undefined) existingProduct.title = pdu.title;
          if (pdu.description_snippet !== undefined) existingProduct.description_snippet = String(pdu.description_snippet).slice(0, 256);
          if (pdu.content_cid !== undefined) existingProduct.content_cid = pdu.content_cid;
          if (pdu.category !== undefined) existingProduct.category = pdu.category;
          if (pdu.price !== undefined) existingProduct.price = _round(pdu.price);
          if (pdu.stock !== undefined) existingProduct.stock = pdu.stock;
          existingProduct.updated_epoch = entry.epoch;
          products.set(pdu.product_id, existingProduct);
        }
      }
      break;

    case "PRODUCT_DELIST":
      if (entry.product_data && entry.product_data.product_id) {
        var pdd = products.get(entry.product_data.product_id);
        if (pdd && pdd.seller === from) {
          pdd.status = "delisted";
          pdd.updated_epoch = entry.epoch;
          products.set(entry.product_data.product_id, pdd);
        }
      }
      break;

    case "ORDER_PLACE":
      if (entry.order_data && entry.order_data.order_id && from && to) {
        var od = entry.order_data;
        var orderedProduct = products.get(od.product_id);
        // Decrement stock (seller's commitment)
        if (orderedProduct && orderedProduct.status === "active") {
          orderedProduct.stock = Math.max(0, (orderedProduct.stock || 0) - (od.quantity || 1));
          products.set(od.product_id, orderedProduct);
        }
        orders.set(od.order_id, {
          order_id: od.order_id,
          buyer: from,
          seller: to,
          product_id: od.product_id,
          quantity: od.quantity || 1,
          unit_price: _round(od.unit_price || 0),
          total: _round(od.total || 0),
          token: od.token || "HONE",
          escrow_id: od.escrow_id || null,
          status: "placed",
          placed_epoch: entry.epoch,
          fulfilled_epoch: null,
          delivered_epoch: null,
        });
        // Increment store total_sales counter
        var sellerStoreRef = stores.get(to);
        if (sellerStoreRef) {
          sellerStoreRef.total_sales = (sellerStoreRef.total_sales || 0) + 1;
          stores.set(to, sellerStoreRef);
        }
      }
      break;

    case "ORDER_FULFILL":
      if (entry.order_data && entry.order_data.order_id) {
        var ofo = orders.get(entry.order_data.order_id);
        if (ofo && ofo.seller === from) {
          ofo.status = "fulfilled";
          ofo.fulfilled_epoch = entry.epoch;
          if (entry.order_data.fulfillment_cid) {
            ofo.fulfillment_cid = entry.order_data.fulfillment_cid;
          }
          orders.set(entry.order_data.order_id, ofo);
        }
      }
      break;

    case "ORDER_DELIVERED":
      if (entry.order_data && entry.order_data.order_id) {
        var odo = orders.get(entry.order_data.order_id);
        // Only the buyer confirms delivery
        if (odo && odo.buyer === from) {
          odo.status = "delivered";
          odo.delivered_epoch = entry.epoch;
          orders.set(entry.order_data.order_id, odo);
          // Increment store fulfilled counter
          var fulfillStore = stores.get(odo.seller);
          if (fulfillStore) {
            fulfillStore.total_fulfilled = (fulfillStore.total_fulfilled || 0) + 1;
            stores.set(odo.seller, fulfillStore);
          }
          // Bump product sold count
          var soldProduct = products.get(odo.product_id);
          if (soldProduct) {
            soldProduct.total_sold = (soldProduct.total_sold || 0) + (odo.quantity || 1);
            products.set(odo.product_id, soldProduct);
          }
        }
      }
      break;

    case "ORDER_CANCEL":
      if (entry.order_data && entry.order_data.order_id) {
        var oco = orders.get(entry.order_data.order_id);
        if (oco && (oco.buyer === from || oco.seller === from)) {
          oco.status = "cancelled";
          oco.cancelled_epoch = entry.epoch;
          orders.set(entry.order_data.order_id, oco);
          // Restore stock
          var cancelledProduct = products.get(oco.product_id);
          if (cancelledProduct) {
            cancelledProduct.stock = (cancelledProduct.stock || 0) + (oco.quantity || 1);
            products.set(oco.product_id, cancelledProduct);
          }
        }
      }
      break;

    case "ORDER_DISPUTE":
      if (entry.order_data && entry.order_data.order_id) {
        var disputeOrder = orders.get(entry.order_data.order_id);
        if (disputeOrder && disputeOrder.buyer === from) {
          disputeOrder.status = "disputed";
          disputeOrder.disputed_epoch = entry.epoch;
          disputeOrder.dispute_memo = entry.order_data.memo || null;
          orders.set(entry.order_data.order_id, disputeOrder);
          // Increment store disputed counter
          var disputedStore = stores.get(disputeOrder.seller);
          if (disputedStore) {
            disputedStore.total_disputed = (disputedStore.total_disputed || 0) + 1;
            stores.set(disputeOrder.seller, disputedStore);
          }
        }
      }
      break;

    // ── Storage settlement lag (v3.1.119+) ──────────────────────────
    case "STORAGE_PAYOUT_HOLD":
      if (entry.storage_hold_data && entry.storage_hold_data.hold_id) {
        var sphData = entry.storage_hold_data;
        // Debit hone_storage_escrow account (create it if needed)
        _ensureAccount("hone_storage_escrow");
        storagePayoutHolds.set(sphData.hold_id, {
          hold_id: sphData.hold_id,
          host: sphData.host,
          cid: sphData.cid,
          amount: sphData.amount,
          hold_epoch: sphData.hold_epoch,
          release_epoch: sphData.release_epoch,
          status: "pending",
        });
      }
      break;

    case "STORAGE_PAYOUT_RELEASE":
      if (entry.storage_hold_data && entry.storage_hold_data.hold_id) {
        var sphRelease = storagePayoutHolds.get(entry.storage_hold_data.hold_id);
        if (sphRelease && sphRelease.status === "pending") {
          _credit(to, "HONE", amount);
          sphRelease.status = "released";
          sphRelease.released_epoch = entry.epoch;
          storagePayoutHolds.set(entry.storage_hold_data.hold_id, sphRelease);
        }
      }
      break;

    case "STORAGE_PAYOUT_FORFEIT":
      if (entry.storage_hold_data && entry.storage_hold_data.hold_id) {
        var sphForfeit = storagePayoutHolds.get(entry.storage_hold_data.hold_id);
        if (sphForfeit && sphForfeit.status === "pending") {
          _credit("hone_recycle", "HONE", amount);
          sphForfeit.status = "forfeited";
          sphForfeit.forfeited_epoch = entry.epoch;
          storagePayoutHolds.set(entry.storage_hold_data.hold_id, sphForfeit);
        }
      }
      break;

    // ── hone_recycle pool redistribution (v3.1.119+) ───────────────
    case "RECYCLE_DISTRIBUTION":
      // Standard debit/credit — recycle pool pays stakers proportionally.
      // The distribution calculation happens in ledger.distributeRecyclePool();
      // here we just apply the balance change.
      _debit(from, "HONE", amount);
      _credit(to, "HONE", amount);
      break;

    // ── Inference Marketplace (v3.1.119+) ───────────────────────────
    case "INFERENCE_JOB_OPEN":
      if (entry.job_data && entry.job_data.job_id) {
        var ijOpen = entry.job_data;
        inferenceJobs.set(ijOpen.job_id, {
          job_id: ijOpen.job_id,
          buyer: ijOpen.buyer,
          prompt: ijOpen.prompt,
          max_fee: ijOpen.max_fee,
          model: ijOpen.model || null,
          system_prompt: ijOpen.system_prompt || null,
          ttl_epochs: ijOpen.ttl_epochs || 20,
          expires_epoch: ijOpen.expires_epoch || 0,
          tools: ijOpen.tools || [],
          max_turns: ijOpen.max_turns || 1,
          turns: [],
          current_turn: 0,
          output_schema: ijOpen.output_schema || null,
          tier: ijOpen.tier || "standard",
          rag_cids: ijOpen.rag_cids || [],
          image_cids: ijOpen.image_cids || [],
          audio_cid: ijOpen.audio_cid || null,
          batch_id: ijOpen.batch_id || null,
          session_id: ijOpen.session_id || null,
          auto_memory: !!ijOpen.auto_memory,
          memory_project: ijOpen.memory_project || null,
          escrow_amount: ijOpen.escrow_amount || 0,
          review_mode: ijOpen.review_mode || "computer",
          review_fee: ijOpen.review_fee || 0,
          challenge_fee: ijOpen.challenge_fee || 0,
          challenge_window_epochs: ijOpen.challenge_window_epochs || 24,
          challenge_deadline_epoch: ijOpen.challenge_deadline_epoch || 0,
          review_stage: ijOpen.review_stage || "initial",
          review_epoch: null,
          assigned_reviewers: ijOpen.assigned_reviewers || [],
          review_committee_hash: ijOpen.review_committee_hash || null,
          miner: null,
          proof_hash: null,
          actual_cost: null,
          review_status: null,
          reviewer: null,
          review_verdict: null,
          challenged_by: null,
          challenge_reason: null,
          challenge_status: null,
          challenge_escrow_id: null,
          challenge_epoch: null,
          review_votes: [],
          review_vote_count: 0,
          review_winner: null,
          review_dissenters: [],
          review_resolved_epoch: null,
          finality_epoch: null,
          finality_hash: null,
          finality_outcome: null,
          status: "open",
          open_epoch: entry.epoch,
          claimed_epoch: null,
          submitted_epoch: null,
          settled_epoch: null,
        });
      }
      break;

    case "INFERENCE_JOB_CLAIM":
      if (entry.job_data && entry.job_data.job_id) {
        var ijClaim = inferenceJobs.get(entry.job_data.job_id);
        if (ijClaim && ijClaim.status === "open") {
          ijClaim.miner = entry.job_data.miner;
          ijClaim.status = "claimed";
          ijClaim.claimed_epoch = entry.epoch;
          inferenceJobs.set(entry.job_data.job_id, ijClaim);
        }
      }
      break;

    case "INFERENCE_JOB_SUBMIT":
      if (entry.job_data && entry.job_data.job_id) {
        var ijSubmit = inferenceJobs.get(entry.job_data.job_id);
        if (ijSubmit && ijSubmit.status === "claimed") {
          ijSubmit.proof_hash = entry.job_data.proof_hash;
          ijSubmit.actual_cost = entry.job_data.actual_cost != null ? entry.job_data.actual_cost : ijSubmit.actual_cost;
          ijSubmit.status = "submitted";
          ijSubmit.submitted_epoch = entry.epoch;
          inferenceJobs.set(entry.job_data.job_id, ijSubmit);
        }
      }
      break;

    case "INFERENCE_JOB_SETTLE":
      if (entry.job_data && entry.job_data.job_id) {
        var ijSettle = inferenceJobs.get(entry.job_data.job_id);
        if (ijSettle) {
          ijSettle.status = "settled";
          ijSettle.actual_cost = entry.job_data.actual_cost;
          ijSettle.miner_payout = entry.job_data.miner_payout;
          ijSettle.protocol_fee = entry.job_data.protocol_fee;
          ijSettle.settled_epoch = entry.epoch;
          ijSettle.settled_by = entry.job_data.settled_by || "auto";
          inferenceJobs.set(entry.job_data.job_id, ijSettle);
        }
      }
      break;

    case "INFERENCE_JOB_REVIEW":
      if (entry.job_data && entry.job_data.job_id) {
        var ijReview = inferenceJobs.get(entry.job_data.job_id);
        if (ijReview) {
          ijReview.review_status = entry.job_data.status || "awaiting_challenge";
          ijReview.review_verdict = entry.job_data.verdict || "accepted";
          ijReview.reviewer = entry.job_data.reviewer || entry.from || null;
          ijReview.review_mode = entry.job_data.review_mode || ijReview.review_mode || "computer";
          ijReview.review_fee = entry.job_data.review_fee || ijReview.review_fee || 0;
          ijReview.challenge_window_epochs = entry.job_data.challenge_window_epochs || ijReview.challenge_window_epochs || 24;
          ijReview.challenge_deadline_epoch = entry.job_data.challenge_deadline_epoch || ijReview.challenge_deadline_epoch || 0;
          ijReview.challenge_escrow_id = entry.job_data.challenge_escrow_id || ijReview.challenge_escrow_id || null;
          ijReview.review_stage = entry.job_data.review_stage || ijReview.review_stage || "initial";
          ijReview.challenge_status = entry.job_data.challenge_status || ijReview.challenge_status || null;
          if (Array.isArray(entry.job_data.review_votes)) {
            ijReview.review_votes = entry.job_data.review_votes.slice();
            ijReview.review_vote_count = ijReview.review_votes.length;
          }
          if (entry.job_data.review_winner) {
            ijReview.review_winner = entry.job_data.review_winner;
          }
          if (Array.isArray(entry.job_data.review_dissenters)) {
            ijReview.review_dissenters = entry.job_data.review_dissenters.slice();
          }
          ijReview.review_epoch = entry.epoch;
          ijReview.status = entry.job_data.status || (entry.job_data.verdict === "accepted" ? "awaiting_challenge" : "review_rejected");
          inferenceJobs.set(entry.job_data.job_id, ijReview);
        }
      }
      break;

    case "INFERENCE_JOB_REVIEW_ASSIGNMENT":
      if (entry.job_data && entry.job_data.job_id) {
        var ijAssign = inferenceJobs.get(entry.job_data.job_id);
        if (ijAssign) {
          ijAssign.review_mode = entry.job_data.review_mode || ijAssign.review_mode || "computer";
          ijAssign.review_stage = entry.job_data.review_stage || ijAssign.review_stage || "initial";
          ijAssign.assigned_reviewers = Array.isArray(entry.job_data.assigned_reviewers) ? entry.job_data.assigned_reviewers.slice() : [];
          ijAssign.review_committee_hash = entry.job_data.review_committee_hash || ijAssign.review_committee_hash || null;
          ijAssign.review_status = entry.job_data.status || "review_assigned";
          inferenceJobs.set(entry.job_data.job_id, ijAssign);
        }
      }
      break;

    case "INFERENCE_JOB_CHALLENGE":
      if (entry.job_data && entry.job_data.job_id) {
        var ijChallenge = inferenceJobs.get(entry.job_data.job_id);
        if (ijChallenge) {
          ijChallenge.challenged_by = entry.job_data.challenger || entry.from || null;
          ijChallenge.challenge_reason = entry.job_data.reason || "quality_dispute";
          ijChallenge.challenge_fee = entry.job_data.challenge_fee || ijChallenge.challenge_fee || 0;
          ijChallenge.challenge_bond = entry.job_data.challenge_bond || 0;
          ijChallenge.challenge_escrow_id = entry.job_data.challenge_escrow_id || ijChallenge.challenge_escrow_id || null;
          ijChallenge.challenge_epoch = entry.epoch;
          ijChallenge.challenge_deadline_epoch = entry.job_data.challenge_deadline_epoch || ijChallenge.challenge_deadline_epoch || 0;
          ijChallenge.assigned_reviewers = Array.isArray(entry.job_data.assigned_reviewers) ? entry.job_data.assigned_reviewers.slice() : (ijChallenge.assigned_reviewers || []);
          ijChallenge.review_committee_hash = entry.job_data.review_committee_hash || ijChallenge.review_committee_hash || null;
          ijChallenge.review_stage = entry.job_data.review_stage || ijChallenge.review_stage || "challenge";
          if (Array.isArray(entry.job_data.review_votes)) {
            ijChallenge.review_votes = entry.job_data.review_votes.slice();
            ijChallenge.review_vote_count = ijChallenge.review_votes.length;
          }
          ijChallenge.challenge_status = "challenged";
          ijChallenge.status = "challenged";
          inferenceJobs.set(entry.job_data.job_id, ijChallenge);
        }
      }
      break;

    case "INFERENCE_JOB_FINALITY":
      if (entry.job_data && entry.job_data.job_id) {
        var ijFinal = inferenceJobs.get(entry.job_data.job_id);
        if (ijFinal) {
          ijFinal.finality_epoch = entry.job_data.finality_epoch || entry.epoch || 0;
          ijFinal.finality_hash = entry.job_data.finality_hash || null;
          ijFinal.finality_outcome = entry.job_data.finality_outcome || ijFinal.finality_outcome || null;
          ijFinal.review_stage = entry.job_data.review_stage || ijFinal.review_stage || null;
          ijFinal.challenge_status = entry.job_data.challenge_status || ijFinal.challenge_status || null;
          if (Array.isArray(entry.job_data.review_dissenters)) {
            ijFinal.review_dissenters = entry.job_data.review_dissenters.slice();
          }
          if (entry.job_data.review_winner) {
            ijFinal.review_winner = entry.job_data.review_winner;
          }
          ijFinal.status = entry.job_data.status || "finalized";
          inferenceJobs.set(entry.job_data.job_id, ijFinal);
        }
      }
      break;

    case "INFERENCE_JOB_REVIEW_VOTE":
      if (entry.job_data && entry.job_data.job_id) {
        var ijVote = inferenceJobs.get(entry.job_data.job_id);
        if (ijVote) {
          var voteKey = entry.job_data.job_id;
          var reviewVoteList = inferenceReviewVotes.get(voteKey) || [];
          if (!Array.isArray(ijVote.review_votes)) ijVote.review_votes = [];
          var voteRecord = {
            reviewer: entry.job_data.reviewer || entry.from || null,
            verdict: entry.job_data.verdict || "accepted",
            epoch: entry.epoch,
            review_mode: entry.job_data.review_mode || ijVote.review_mode || "computer",
            review_stage: entry.job_data.review_stage || ijVote.review_stage || "initial",
            status: entry.job_data.status || "review_voting",
          };
          if (entry.job_data.review_committee_hash) {
            voteRecord.review_committee_hash = entry.job_data.review_committee_hash;
          }
          var existingVoteIndex = ijVote.review_votes.findIndex(function (vote) {
            return vote.reviewer === voteRecord.reviewer && vote.review_stage === voteRecord.review_stage;
          });
          if (existingVoteIndex >= 0) {
            ijVote.review_votes[existingVoteIndex] = voteRecord;
          } else {
            ijVote.review_votes.push(voteRecord);
          }
          var existingReviewVoteIndex = reviewVoteList.findIndex(function (vote) {
            return vote.reviewer === voteRecord.reviewer && vote.review_stage === voteRecord.review_stage;
          });
          if (existingReviewVoteIndex >= 0) {
            reviewVoteList[existingReviewVoteIndex] = voteRecord;
          } else {
            reviewVoteList.push(voteRecord);
          }
          inferenceReviewVotes.set(voteKey, reviewVoteList);
          ijVote.review_vote_count = ijVote.review_votes.length;
          ijVote.review_status = entry.job_data.status || ijVote.review_status || "review_voting";
          inferenceJobs.set(entry.job_data.job_id, ijVote);
        }
      }
      break;

    case "INFERENCE_JOB_REVIEW_OUTCOME":
      if (entry.job_data && entry.job_data.job_id) {
        var ijOutcome = inferenceJobs.get(entry.job_data.job_id);
        if (ijOutcome) {
          ijOutcome.review_verdict = entry.job_data.review_verdict || ijOutcome.review_verdict || null;
          ijOutcome.challenge_status = entry.job_data.challenge_status || ijOutcome.challenge_status || null;
          ijOutcome.review_status = entry.job_data.status || ijOutcome.review_status || "review_resolved";
          ijOutcome.review_winner = entry.job_data.review_winner || ijOutcome.review_winner || null;
          ijOutcome.review_dissenters = Array.isArray(entry.job_data.review_dissenters)
            ? entry.job_data.review_dissenters.slice()
            : (ijOutcome.review_dissenters || []);
          ijOutcome.review_vote_count = entry.job_data.review_vote_count != null
            ? entry.job_data.review_vote_count
            : (Array.isArray(ijOutcome.review_votes) ? ijOutcome.review_votes.length : 0);
          ijOutcome.review_resolved_epoch = entry.epoch;
          if (entry.job_data.status) {
            ijOutcome.status = entry.job_data.status;
          }
          if (Array.isArray(ijOutcome.review_votes)) {
            inferenceReviewVotes.set(entry.job_data.job_id, ijOutcome.review_votes.slice());
          }
          inferenceJobs.set(entry.job_data.job_id, ijOutcome);
        }
      }
      break;

    case "INFERENCE_JOB_REFUND":
      if (entry.job_data && entry.job_data.job_id) {
        var ijRefund = inferenceJobs.get(entry.job_data.job_id);
        if (ijRefund) {
          ijRefund.status = "refunded";
          ijRefund.refund_reason = entry.job_data.reason || "expired";
          ijRefund.refunded_epoch = entry.epoch;
          inferenceJobs.set(entry.job_data.job_id, ijRefund);
        }
      }
      break;

    case "INFERENCE_JOB_TOOL_CALL":
      if (entry.job_data && entry.job_data.job_id) {
        var ijTC = inferenceJobs.get(entry.job_data.job_id);
        if (ijTC && (ijTC.status === "claimed" || ijTC.status === "tool_pending")) {
          ijTC.turns = ijTC.turns || [];
          ijTC.turns.push({
            role: "assistant",
            tool_calls: entry.job_data.tool_calls,
            turn: entry.job_data.turn,
            epoch: entry.epoch,
          });
          ijTC.status = "tool_pending";
          ijTC.current_turn = entry.job_data.turn;
          inferenceJobs.set(entry.job_data.job_id, ijTC);
        }
      }
      break;

    case "INFERENCE_JOB_TOOL_RESULT":
      if (entry.job_data && entry.job_data.job_id) {
        var ijTR = inferenceJobs.get(entry.job_data.job_id);
        if (ijTR && ijTR.status === "tool_pending") {
          ijTR.turns = ijTR.turns || [];
          ijTR.turns.push({
            role: "tool",
            tool_results: entry.job_data.tool_results,
            turn: entry.job_data.turn,
            epoch: entry.epoch,
          });
          // Re-open for next inference turn
          ijTR.status = "claimed";
          ijTR.current_turn = entry.job_data.turn;
          inferenceJobs.set(entry.job_data.job_id, ijTR);
        }
      }
      break;

    // ── Browser / Computer-use jobs (v3.1.121+) ─────────────────────
    case "BROWSER_JOB_OPEN":
      if (entry.browser_data && entry.browser_data.job_id) {
        var bjOpen = entry.browser_data;
        browserJobs.set(bjOpen.job_id, {
          job_id: bjOpen.job_id,
          buyer: bjOpen.buyer,
          goal: bjOpen.goal,
          start_url: bjOpen.start_url,
          max_fee: bjOpen.max_fee,
          max_turns: bjOpen.max_turns || 20,
          ttl_epochs: bjOpen.ttl_epochs || 120,
          expires_epoch: bjOpen.expires_epoch || 0,
          viewport: bjOpen.viewport || { width: 1280, height: 800 },
          status: "open",
          miner: null,
          turns: [],
          current_turn: 0,
          open_epoch: entry.epoch,
          claimed_epoch: null,
          settled_epoch: null,
        });
      }
      break;

    case "BROWSER_JOB_CLAIM":
      if (entry.browser_data && entry.browser_data.job_id) {
        var bjClaim = browserJobs.get(entry.browser_data.job_id);
        if (bjClaim && bjClaim.status === "open") {
          bjClaim.miner = entry.browser_data.miner;
          bjClaim.status = "claimed";
          bjClaim.claimed_epoch = entry.epoch;
          browserJobs.set(entry.browser_data.job_id, bjClaim);
        }
      }
      break;

    case "BROWSER_JOB_SCREENSHOT":
      if (entry.browser_data && entry.browser_data.job_id) {
        var bjSS = browserJobs.get(entry.browser_data.job_id);
        if (bjSS && (bjSS.status === "claimed" || bjSS.status === "action_pending")) {
          bjSS.turns = bjSS.turns || [];
          bjSS.turns.push({
            role: "miner",
            screenshot_cid: entry.browser_data.screenshot_cid,
            current_url: entry.browser_data.current_url,
            page_title: entry.browser_data.page_title,
            available_actions: entry.browser_data.available_actions,
            turn: entry.browser_data.turn,
            epoch: entry.epoch,
          });
          bjSS.current_turn = entry.browser_data.turn;
          bjSS.status = "action_pending";
          browserJobs.set(entry.browser_data.job_id, bjSS);
        }
      }
      break;

    case "BROWSER_JOB_ACTION":
      if (entry.browser_data && entry.browser_data.job_id) {
        var bjAct = browserJobs.get(entry.browser_data.job_id);
        if (bjAct && bjAct.status === "action_pending") {
          bjAct.turns = bjAct.turns || [];
          bjAct.turns.push({
            role: "buyer",
            action_type: entry.browser_data.action_type,
            coordinate: entry.browser_data.coordinate,
            text: entry.browser_data.text,
            selector: entry.browser_data.selector,
            turn: entry.browser_data.turn,
            epoch: entry.epoch,
          });
          bjAct.status = entry.browser_data.action_type === "done" ? "submitted" : "claimed";
          browserJobs.set(entry.browser_data.job_id, bjAct);
        }
      }
      break;

    case "BROWSER_JOB_SETTLE":
      if (entry.browser_data && entry.browser_data.job_id) {
        var bjSettle = browserJobs.get(entry.browser_data.job_id);
        if (bjSettle) {
          bjSettle.status = "settled";
          bjSettle.actual_cost = entry.browser_data.actual_cost;
          bjSettle.miner_payout = entry.browser_data.miner_payout;
          bjSettle.result_summary = entry.browser_data.result_summary || null;
          bjSettle.final_screenshot_cid = entry.browser_data.final_screenshot_cid || null;
          bjSettle.settled_epoch = entry.epoch;
          browserJobs.set(entry.browser_data.job_id, bjSettle);
        }
      }
      break;

    case "BROWSER_JOB_REFUND":
      if (entry.browser_data && entry.browser_data.job_id) {
        var bjRefund = browserJobs.get(entry.browser_data.job_id);
        if (bjRefund) {
          bjRefund.status = "refunded";
          bjRefund.refund_reason = entry.browser_data.reason || "expired";
          bjRefund.refunded_epoch = entry.epoch;
          browserJobs.set(entry.browser_data.job_id, bjRefund);
        }
      }
      break;

    // ── Fine-tuning jobs (v3.1.121+) ────────────────────────────────
    case "FINETUNE_JOB_OPEN":
      if (entry.ft_data && entry.ft_data.finetune_id) {
        var ftOpen = entry.ft_data;
        finetuneJobs.set(ftOpen.finetune_id, {
          finetune_id: ftOpen.finetune_id,
          buyer: ftOpen.buyer,
          dataset_cid: ftOpen.dataset_cid,
          base_model: ftOpen.base_model,
          epochs: ftOpen.epochs || 3,
          lora_rank: ftOpen.lora_rank || 16,
          max_fee: ftOpen.max_fee,
          ttl_epochs: ftOpen.ttl_epochs || 2000,
          expires_epoch: ftOpen.expires_epoch || 0,
          status: "open",
          miner: null,
          adapter_cid: null,
          progress_pct: 0,
          open_epoch: entry.epoch,
          claimed_epoch: null,
          completed_epoch: null,
          settled_epoch: null,
        });
      }
      break;

    case "FINETUNE_JOB_CLAIM":
      if (entry.ft_data && entry.ft_data.finetune_id) {
        var ftClaim = finetuneJobs.get(entry.ft_data.finetune_id);
        if (ftClaim && ftClaim.status === "open") {
          ftClaim.miner = entry.ft_data.miner;
          ftClaim.status = "training";
          ftClaim.claimed_epoch = entry.epoch;
          finetuneJobs.set(entry.ft_data.finetune_id, ftClaim);
        }
      }
      break;

    case "FINETUNE_JOB_PROGRESS":
      if (entry.ft_data && entry.ft_data.finetune_id) {
        var ftProg = finetuneJobs.get(entry.ft_data.finetune_id);
        if (ftProg) {
          ftProg.progress_pct = entry.ft_data.progress_pct || 0;
          ftProg.current_epoch_num = entry.ft_data.current_epoch_num || 0;
          ftProg.loss = entry.ft_data.loss || null;
          finetuneJobs.set(entry.ft_data.finetune_id, ftProg);
        }
      }
      break;

    case "FINETUNE_JOB_COMPLETE":
      if (entry.ft_data && entry.ft_data.finetune_id) {
        var ftComp = finetuneJobs.get(entry.ft_data.finetune_id);
        if (ftComp) {
          ftComp.status = "completed";
          ftComp.adapter_cid = entry.ft_data.adapter_cid;
          ftComp.completed_epoch = entry.epoch;
          finetuneJobs.set(entry.ft_data.finetune_id, ftComp);
        }
      }
      break;

    case "FINETUNE_JOB_SETTLE":
      if (entry.ft_data && entry.ft_data.finetune_id) {
        var ftSettle = finetuneJobs.get(entry.ft_data.finetune_id);
        if (ftSettle) {
          ftSettle.status = "settled";
          ftSettle.actual_cost = entry.ft_data.actual_cost;
          ftSettle.miner_payout = entry.ft_data.miner_payout;
          ftSettle.settled_epoch = entry.epoch;
          finetuneJobs.set(entry.ft_data.finetune_id, ftSettle);
        }
      }
      break;

    case "FINETUNE_JOB_REFUND":
      if (entry.ft_data && entry.ft_data.finetune_id) {
        var ftRefund = finetuneJobs.get(entry.ft_data.finetune_id);
        if (ftRefund) {
          ftRefund.status = "refunded";
          ftRefund.refund_reason = entry.ft_data.reason || "expired";
          ftRefund.refunded_epoch = entry.epoch;
          finetuneJobs.set(entry.ft_data.finetune_id, ftRefund);
        }
      }
      break;

    // ── Sessions (v3.1.121+) ────────────────────────────────────────
    case "SESSION_CREATE":
      if (entry.session_data && entry.session_data.session_id) {
        var sd = entry.session_data;
        sessions.set(sd.session_id, {
          session_id: sd.session_id,
          buyer: sd.buyer,
          name: sd.name || null,
          created_epoch: entry.epoch,
          last_updated_epoch: entry.epoch,
          job_ids: [],
          summary: sd.initial_summary || null,
        });
      }
      break;

    case "SESSION_ADD_JOB":
      if (entry.session_data && entry.session_data.session_id) {
        var sadj = sessions.get(entry.session_data.session_id);
        if (sadj) {
          sadj.job_ids = sadj.job_ids || [];
          if (!sadj.job_ids.includes(entry.session_data.job_id)) {
            sadj.job_ids.push(entry.session_data.job_id);
          }
          sadj.last_updated_epoch = entry.epoch;
          sessions.set(entry.session_data.session_id, sadj);
        }
      }
      break;

    case "SESSION_UPDATE_SUMMARY":
      if (entry.session_data && entry.session_data.session_id) {
        var sus = sessions.get(entry.session_data.session_id);
        if (sus) {
          sus.summary = entry.session_data.summary;
          sus.last_updated_epoch = entry.epoch;
          sessions.set(entry.session_data.session_id, sus);
        }
      }
      break;

    // ── MCP Tool Registry (v3.1.121+) ───────────────────────────────
    case "TOOL_REGISTER":
      if (entry.tool_data && entry.tool_data.name) {
        var td = entry.tool_data;
        toolRegistry.set(td.name, {
          name: td.name,
          description: td.description || "",
          schema: td.schema || null,
          webhook_url: td.webhook_url,
          owner: td.owner || from,
          registered_epoch: entry.epoch,
        });
      }
      break;

    case "TOOL_UNREGISTER":
      if (entry.tool_data && entry.tool_data.name) {
        toolRegistry.delete(entry.tool_data.name);
      }
      break;

    // ── Inference memory (v3.1.131+) ────────────────────────────────
    case "INFERENCE_MEMORY_SAVE":
      if (entry.memory_data && entry.memory_data.memory_id && from) {
        var md = entry.memory_data;
        if (!/^[a-f0-9]{64}$/.test(md.cid || "")) break;
        if (!userMemories.has(from)) userMemories.set(from, new Map());
        userMemories.get(from).set(md.memory_id, {
          memory_id: md.memory_id,
          cid: md.cid,
          tags: Array.isArray(md.tags) ? md.tags : [],
          project_id: md.project_id || null,
          linked_memory_ids: Array.isArray(md.linked_memory_ids) ? md.linked_memory_ids : [],
          created_epoch: entry.epoch,
          size_bytes: md.size_bytes || 0,
        });
        // Add memory to project's memory_ids list
        if (md.project_id) {
          if (!memoryProjects.has(from)) memoryProjects.set(from, new Map());
          var projMap = memoryProjects.get(from);
          var existingProj = projMap.get(md.project_id);
          if (existingProj && !existingProj.memory_ids.includes(md.memory_id)) {
            existingProj.memory_ids.push(md.memory_id);
          }
        }
      }
      break;

    case "INFERENCE_MEMORY_DELETE":
      if (entry.memory_data && entry.memory_data.memory_id && from) {
        var delMap = userMemories.get(from);
        if (delMap) delMap.delete(entry.memory_data.memory_id);
        // Remove from project
        var delProjId = entry.memory_data.project_id;
        if (delProjId) {
          var delProjMap = memoryProjects.get(from);
          if (delProjMap) {
            var delProj = delProjMap.get(delProjId);
            if (delProj) {
              delProj.memory_ids = delProj.memory_ids.filter(function(id) { return id !== entry.memory_data.memory_id; });
            }
          }
        }
      }
      break;

    case "MEMORY_PROJECT_CREATE":
      if (entry.memory_data && entry.memory_data.project_id && from) {
        if (!memoryProjects.has(from)) memoryProjects.set(from, new Map());
        var projCreateMap = memoryProjects.get(from);
        if (!projCreateMap.has(entry.memory_data.project_id)) {
          projCreateMap.set(entry.memory_data.project_id, {
            project_id: entry.memory_data.project_id,
            name: entry.memory_data.name || entry.memory_data.project_id,
            memory_ids: [],
            created_epoch: entry.epoch,
          });
        }
      }
      break;

    // ── Miner capability advertisement (v3.1.125+) ──────────────────
    case "MINER_CAPABILITY":
      if (entry.capabilities && from) {
        var caps = entry.capabilities;
        minerCapabilities.set(from, {
          miner: from,
          models: Array.isArray(caps.models) ? caps.models : [],
          vision: !!caps.vision,
          vision_model: caps.vision_model || null,
          audio: !!caps.audio,
          code_exec: !!caps.code_exec,
          browser: !!caps.browser,
          finetune: !!caps.finetune,
          tiers: Array.isArray(caps.tiers) ? caps.tiers : ["standard"],
          version: caps.version || null,
          updated_epoch: entry.epoch,
        });
      }
      break;

    // ── HONE-FS blob commits (v2.11+) ─────────────────────────────
    case "BLOB_STORE_COMMIT":
      if (entry.blob_data && entry.blob_data.cid && from) {
        var bd = entry.blob_data;
        if (!/^[a-f0-9]{64}$/.test(bd.cid)) break;
        var existingBlob = blobs.get(bd.cid) || {
          cid: bd.cid,
          size: bd.size || 0,
          uploader: from,
          hosts: [],
          active_hosts: [],      // v2.11.2+: hosts committed to active serving
          cold_hosts: [],        // v2.11.2+: hosts committed to durability only
          region_constraints: [], // v2.11.2+: ISO region list, empty = any
          target_active: 0,      // v2.11.2+: uploader-requested active count
          target_cold: 0,        // v2.11.2+: uploader-requested cold count
          under_replicated: false, // v2.11.2+: true when actuals < targets
          committed_epoch: entry.epoch,
          expires_epoch: entry.epoch + (bd.duration_epochs || 0),
          payment_hone: 0,
          bytes_served_total: 0,
          bytes_served_by_host: {},
          serve_proof_count: 0,
        };
        // Legacy hosts field: kept as the union of active + cold for
        // backward compat with v2.11.0/v2.11.1 code paths.
        var hostSet = {};
        existingBlob.hosts.forEach(function (h) { hostSet[h] = true; });
        (bd.hosts || []).forEach(function (h) {
          if (typeof h === "string" && h.length > 0) hostSet[h] = true;
        });
        existingBlob.hosts = Object.keys(hostSet);

        // v2.11.2+: track active vs cold separately
        if (Array.isArray(bd.active_hosts)) {
          var activeSet = {};
          existingBlob.active_hosts.forEach(function (h) { activeSet[h] = true; });
          bd.active_hosts.forEach(function (h) {
            if (typeof h === "string" && h.length > 0) activeSet[h] = true;
          });
          existingBlob.active_hosts = Object.keys(activeSet);
          // Also add to the unified hosts list
          existingBlob.active_hosts.forEach(function (h) { hostSet[h] = true; });
        }
        if (Array.isArray(bd.cold_hosts)) {
          var coldSet = {};
          existingBlob.cold_hosts.forEach(function (h) { coldSet[h] = true; });
          bd.cold_hosts.forEach(function (h) {
            if (typeof h === "string" && h.length > 0) coldSet[h] = true;
          });
          existingBlob.cold_hosts = Object.keys(coldSet);
          existingBlob.cold_hosts.forEach(function (h) { hostSet[h] = true; });
        }
        existingBlob.hosts = Object.keys(hostSet);

        // Region constraints (chain invariant for all future selections)
        if (Array.isArray(bd.region_constraints) && bd.region_constraints.length > 0) {
          existingBlob.region_constraints = bd.region_constraints
            .filter(function (r) { return typeof r === "string" && r.length > 0; })
            .map(function (r) { return r.toUpperCase(); });
        }

        // Uploader targets (what they wanted; actuals depend on pool size)
        if (Number.isFinite(bd.target_active)) existingBlob.target_active = bd.target_active;
        if (Number.isFinite(bd.target_cold)) existingBlob.target_cold = bd.target_cold;

        // Under-replicated flag: true when either actual is below target
        existingBlob.under_replicated =
          existingBlob.active_hosts.length < existingBlob.target_active ||
          existingBlob.cold_hosts.length < existingBlob.target_cold;

        var newExpiry = entry.epoch + (bd.duration_epochs || 0);
        if (newExpiry > existingBlob.expires_epoch) {
          existingBlob.expires_epoch = newExpiry;
        }
        existingBlob.payment_hone = _round(existingBlob.payment_hone + (bd.payment_hone || 0));
        if (bd.size && !existingBlob.size) existingBlob.size = bd.size;
        blobs.set(bd.cid, existingBlob);
      }
      break;

    // ── Community model registry (v3.1.97+) ───────────────────────────────────
    // Anyone can upload a model to the chain and earn royalties on inference.
    case "MODEL_UPLOAD":
      if (entry.model_data && entry.model_data.model_id && from) {
        var md = entry.model_data;
        var modelId = md.model_id;
        if (!/^[a-z0-9][a-z0-9._-]{1,63}$/.test(modelId)) break;
        // First write wins — uploader cannot be changed after registration
        if (!communityModels.has(modelId)) {
          communityModels.set(modelId, {
            model_id: modelId,
            name: md.name || modelId,
            description: md.description || '',
            format: md.format || 'onnx',
            size_mb: md.size_mb || 0,
            uploader: from,
            royalty_percent: Math.min(Math.max(Number(md.royalty_percent) || 5, 0), 10),
            stake_amount: Number(md.stake_amount) || 0,
            files: md.files || {},
            status: Object.keys(md.files || {}).length > 0 ? 'active' : 'pending',
            upload_epoch: entry.epoch,
          });
        } else {
          // Uploader can update files/metadata but not royalty or ownership
          var existing = communityModels.get(modelId);
          if (existing.uploader === from) {
            if (md.files) existing.files = Object.assign({}, existing.files, md.files);
            if (md.name) existing.name = md.name;
            if (md.description) existing.description = md.description;
            if (md.size_mb) existing.size_mb = md.size_mb;
            existing.status = Object.keys(existing.files).length > 0 ? 'active' : 'pending';
            communityModels.set(modelId, existing);
          }
        }
      }
      break;

    // ── Private encrypted file storage (v3.1.110+) ─────────────────
    case "FILE_STORE":
      if (entry.file_data && entry.file_data.storage_id && from) {
        var fd = entry.file_data;
        var sid = fd.storage_id;
        if (typeof sid === 'string' && sid.length > 0 && !storedFiles.has(sid)) {
          storedFiles.set(sid, {
            owner: from,
            encrypted_manifest: fd.encrypted_manifest || null,
            wrapped_dek_owner: fd.wrapped_dek_owner || null,
            grants: Array.isArray(fd.grants) ? fd.grants.slice() : [],
            total_size: Number(fd.total_size) || 0,
            stored_epoch: entry.epoch || 0,
          });
        }
      }
      break;

    case "FILE_GRANT":
      if (entry.grant_data && entry.grant_data.storage_id && from) {
        var gd = entry.grant_data;
        var gFile = storedFiles.get(gd.storage_id);
        if (gFile && gFile.owner === from && gd.grantee && gd.wrapped_dek) {
          var existingIdx = gFile.grants.findIndex(function(g) { return g.grantee === gd.grantee; });
          var newGrant = { grantee: gd.grantee, wrapped_dek: gd.wrapped_dek, expires_at: gd.expires_at || null };
          if (existingIdx >= 0) {
            gFile.grants[existingIdx] = newGrant;
          } else {
            gFile.grants.push(newGrant);
          }
        }
      }
      break;

    case "FILE_REVOKE":
      if (entry.revoke_data && entry.revoke_data.storage_id && from) {
        var rd = entry.revoke_data;
        var rFile = storedFiles.get(rd.storage_id);
        if (rFile && rFile.owner === from && rd.grantee) {
          rFile.grants = rFile.grants.filter(function(g) { return g.grantee !== rd.grantee; });
        }
      }
      break;

    // ── Quantum-resistant split storage (v3.1.111+) ─────────────────
    // One opaque shard pointer per entry. Two entries constitute a file;
    // nothing here links them — that association lives client-side only.
    case "FILE_SHARD":
      if (entry.shard_data && entry.shard_data.shard_id && from) {
        var shd = entry.shard_data;
        if (typeof shd.shard_id === 'string' && shd.shard_id.length > 0 && !shardPtrs.has(shd.shard_id)) {
          shardPtrs.set(shd.shard_id, {
            owner: from,
            encrypted_ptr: shd.encrypted_ptr || null,
            created_epoch: entry.epoch || 0,
          });
        }
      }
      break;

    // ── HONE-FS storage heartbeats (v2.11.2+) ─────────────────────
    // A storage host proves it's alive and listing the CIDs it has on
    // disk. Home-user-friendly durability signal. Used by:
    //   1. Uptime-weighted payout formulas (blobPayouts v2.11.2+)
    //   2. Verifier challenge selection (pick host + CID from recent
    //      heartbeats for challenge-response audits)
    //   3. Auto-replacement when a host goes dark (no heartbeats for
    //      extended window → removed from commit hosts list, replaced
    //      from the pool)
    //
    // No chain-state mutation of the blob itself — just host liveness.
    // Home users with slow disks or flaky ISPs can heartbeat every few
    // epochs without needing to serve any bytes.
    case "STORAGE_HEARTBEAT":
      if (from) {
        var hbRecord = storageHeartbeats.get(from) || {
          host: from,
          heartbeats: [],
          total_heartbeats: 0,
          first_heartbeat_epoch: entry.epoch,
        };
        var hbEntry = {
          epoch: entry.epoch,
          cids: Array.isArray(entry.blob_data && entry.blob_data.cids)
            ? entry.blob_data.cids.filter(function (c) {
                return typeof c === "string" && /^[a-f0-9]{64}$/.test(c);
              })
            : [],
          capacity_used_gb:
            (entry.blob_data && Number(entry.blob_data.capacity_used_gb)) || 0,
        };
        hbRecord.heartbeats.push(hbEntry);
        if (hbRecord.heartbeats.length > STORAGE_HEARTBEAT_RETENTION) {
          // Drop oldest entries to bound memory
          hbRecord.heartbeats.splice(
            0,
            hbRecord.heartbeats.length - STORAGE_HEARTBEAT_RETENTION
          );
        }
        hbRecord.total_heartbeats = (hbRecord.total_heartbeats || 0) + 1;
        hbRecord.last_heartbeat_epoch = entry.epoch;
        storageHeartbeats.set(from, hbRecord);
      }
      break;

    // ── HONE-FS challenge-response (v2.11.2+) ─────────────────────
    // A verifier issues a spot-check: "return the sha256 of bytes
    // [start..start+length] of CID X". Host has 2 epochs to respond.
    // No slashing — just payout weight + reputation.
    case "BLOB_CHALLENGE":
      if (entry.challenge_data && entry.challenge_data.challenge_id && from) {
        var cd = entry.challenge_data;
        var challengeId = cd.challenge_id;
        if (blobChallenges.has(challengeId)) break; // dedupe
        blobChallenges.set(challengeId, {
          challenge_id: challengeId,
          challenger: from,
          host: cd.host,
          cid: cd.cid,
          byte_start: cd.byte_start || 0,
          byte_length: cd.byte_length || 0,
          issued_epoch: entry.epoch,
          response_epoch: null,
          response_hash: null,
          expected_hash: cd.expected_hash || null,
          status: "pending",
        });
        // Initialize host's stats if first challenge
        if (cd.host) {
          var stats = blobChallengeStats.get(cd.host) || {
            total_issued: 0,
            total_passed: 0,
            total_failed: 0,
            last_challenge_epoch: 0,
          };
          stats.total_issued += 1;
          stats.last_challenge_epoch = entry.epoch;
          blobChallengeStats.set(cd.host, stats);
        }
      }
      break;

    case "BLOB_CHALLENGE_RESPONSE":
      if (entry.challenge_data && entry.challenge_data.challenge_id && from) {
        var crd = entry.challenge_data;
        var challenge = blobChallenges.get(crd.challenge_id);
        if (!challenge) break;
        if (challenge.status !== "pending") break;
        if (challenge.host !== from) break; // only the challenged host can respond
        challenge.response_epoch = entry.epoch;
        challenge.response_hash = crd.response_hash || null;
        // If the challenger pre-published the expected hash, we can
        // resolve status immediately. Otherwise the challenger records
        // BLOB_CHALLENGE_RESULT in a follow-up entry once they verify.
        if (challenge.expected_hash) {
          if (challenge.response_hash === challenge.expected_hash) {
            challenge.status = "passed";
            _bumpChallengeStat(from, "passed");
            _applyRepEvent(from, 'storage', true, entry.epoch);
          } else {
            challenge.status = "failed_mismatch";
            _bumpChallengeStat(from, "failed");
            _applyRepEvent(from, 'storage', false, entry.epoch);
          }
        } else {
          challenge.status = "responded"; // awaiting verifier ruling
        }
        blobChallenges.set(crd.challenge_id, challenge);
      }
      break;

    case "BLOB_CHALLENGE_RESULT":
      // Verifier records the outcome of a responded challenge.
      // Separate from CHALLENGE_RESPONSE so the verifier can audit the
      // response hash against their own computed hash before committing.
      if (entry.challenge_data && entry.challenge_data.challenge_id && from) {
        var rd = entry.challenge_data;
        var c2 = blobChallenges.get(rd.challenge_id);
        if (!c2) break;
        if (c2.challenger !== from) break; // only the original challenger
        if (c2.status !== "responded") break;
        if (rd.passed) {
          c2.status = "passed";
          _bumpChallengeStat(c2.host, "passed");
          _applyRepEvent(c2.host, 'storage', true, entry.epoch);
        } else {
          c2.status = "failed_mismatch";
          _bumpChallengeStat(c2.host, "failed");
          _applyRepEvent(c2.host, 'storage', false, entry.epoch);
          // Mark any pending storage payout holds for this CID as forfeit_pending.
          // The ledger will write STORAGE_PAYOUT_FORFEIT entries on next epoch sweep.
          forfeitStorageHoldsForCid(c2.cid);
        }
        blobChallenges.set(rd.challenge_id, c2);
      }
      break;

    case "BLOB_CHALLENGE_TIMEOUT":
      // Verifier records that a challenge exceeded its response window
      // without a CHALLENGE_RESPONSE entry. Counts as a failure but
      // does NOT slash — see feedback_storage_no_slash.md.
      if (entry.challenge_data && entry.challenge_data.challenge_id && from) {
        var td = entry.challenge_data;
        var c3 = blobChallenges.get(td.challenge_id);
        if (!c3) break;
        if (c3.status !== "pending") break;
        c3.status = "failed_timeout";
        _bumpChallengeStat(c3.host, "failed");
        _applyRepEvent(c3.host, 'storage', false, entry.epoch);
        blobChallenges.set(td.challenge_id, c3);
      }
      break;

    case "STORAGE_MIGRATION": {
      // Data migrated away from a failing host to a new host.
      // Old host: rep ding + migration counter. New host: neutral (starts earning).
      var smOld = entry.migration_data && entry.migration_data.from_host;
      var smNew = entry.migration_data && entry.migration_data.to_host;
      if (smOld) {
        var smRep = _getNodeRep(smOld);
        smRep.storage = _updateRepScore(smRep.storage, false);
        smRep.storage.migrations = (smRep.storage.migrations || 0) + 1;
        smRep.storage.last_epoch = entry.epoch;
        smRep.overall = _computeOverall(smRep);
        nodeReputation.set(smOld, smRep);
      }
      break;
    }

    case "NODE_REPUTATION_UPDATE": {
      // Manual/oracle-driven reputation event for any service category.
      var nru = entry.reputation_data || {};
      if (nru.account && nru.category && typeof nru.passed === 'boolean') {
        _applyRepEvent(nru.account, nru.category, nru.passed, entry.epoch, nru.extra || null);
      }
      break;
    }

    // Host reports bytes served for a CID in the current epoch.
    // Chain invariants:
    //   1. the CID must already have a BLOB_STORE_COMMIT on chain
    //   2. the reporting host (`from`) must be in the committed hosts list
    //   3. bytes_served must be non-negative
    // Accumulates into bytes_served_total + bytes_served_by_host[from].
    // v2.11.1: recording only. v2.11.2+ will add verifier spot-checks
    // that can slash inflated or fraudulent serve proofs.
    case "BLOB_SERVE_PROOF":
      if (entry.blob_data && entry.blob_data.cid && from) {
        var sb = entry.blob_data;
        var servedBlob = blobs.get(sb.cid);
        if (!servedBlob) break; // No commit → drop
        if (servedBlob.hosts.indexOf(from) === -1) break; // Not a committed host
        var bytesReported = Number(sb.bytes_served) || 0;
        if (bytesReported < 0) break;
        servedBlob.bytes_served_total = _round((servedBlob.bytes_served_total || 0) + bytesReported);
        if (!servedBlob.bytes_served_by_host) servedBlob.bytes_served_by_host = {};
        servedBlob.bytes_served_by_host[from] = _round(
          (servedBlob.bytes_served_by_host[from] || 0) + bytesReported
        );
        servedBlob.serve_proof_count = (servedBlob.serve_proof_count || 0) + 1;
        servedBlob.last_serve_epoch = entry.epoch;
        blobs.set(sb.cid, servedBlob);
      }
      break;

    case "REPUTATION_VOTE":
      // entry.vote_data: { target_type: "store"|"miner"|"product", target_id, vote: +1|-1, weight, memo }
      if (entry.vote_data && entry.vote_data.target_type && entry.vote_data.target_id && from) {
        var vd = entry.vote_data;
        var targetKey = vd.target_type + "|" + vd.target_id;
        var voteKey = from + "|" + targetKey;
        var vote = (vd.vote > 0 ? 1 : -1);
        var weight = Math.max(1, Math.min(100, vd.weight || 1));

        // Existing vote from same voter? Roll back its contribution before applying new one
        var priorVote = reputationVotes.get(voteKey);
        var rep = reputation.get(targetKey) || { score: 0, votes_up: 0, votes_down: 0, completed: 0, disputed: 0, last_updated_epoch: 0 };

        if (priorVote) {
          if (priorVote.vote > 0) rep.votes_up = Math.max(0, rep.votes_up - priorVote.weight);
          else rep.votes_down = Math.max(0, rep.votes_down - priorVote.weight);
        }

        if (vote > 0) rep.votes_up += weight;
        else rep.votes_down += weight;

        // Simple weighted score: (up - down) / (up + down + smoothing) * 100, range roughly [-100, +100]
        var totalWeight = rep.votes_up + rep.votes_down + 10;
        rep.score = _round(((rep.votes_up - rep.votes_down) / totalWeight) * 100);
        rep.last_updated_epoch = entry.epoch;
        reputation.set(targetKey, rep);
        reputationVotes.set(voteKey, { vote: vote, weight: weight, epoch: entry.epoch, memo: vd.memo || null });

        // Mirror onto the target entity's rep fields for O(1) reads
        if (vd.target_type === "store") {
          var votedStore = stores.get(vd.target_id);
          if (votedStore) {
            votedStore.rep_score = rep.score;
            votedStore.rep_votes_up = rep.votes_up;
            votedStore.rep_votes_down = rep.votes_down;
            stores.set(vd.target_id, votedStore);
          }
        } else if (vd.target_type === "product") {
          var votedProduct = products.get(vd.target_id);
          if (votedProduct) {
            votedProduct.rep_score = rep.score;
            votedProduct.rep_votes_up = rep.votes_up;
            votedProduct.rep_votes_down = rep.votes_down;
            products.set(vd.target_id, votedProduct);
          }
        }
      }
      break;

    // NFT-related entries use memo-encoded JSON
    case "FAUCET_NFT":
    case "NFT_CREATE":
    case "NFT_MINT":
    case "NFT_TRANSFER":
      // Parse memo for NFT metadata and update nfts map
      try {
        if (entry.memo && typeof entry.memo === "string") {
          var nftData = JSON.parse(entry.memo);
          if (nftData.collection && nftData.tokenId) {
            var nkey = nftData.collection + "|" + nftData.tokenId;
            var existing = nfts.get(nkey) || {};
            nfts.set(nkey, Object.assign(existing, {
              owner: to || existing.owner,
              collection: nftData.collection,
              tokenId: nftData.tokenId,
              metadata: nftData.metadata || existing.metadata,
              minted_epoch: existing.minted_epoch || entry.epoch,
              soulbound: nftData.soulbound || existing.soulbound || false,
              time_locked: nftData.time_locked || existing.time_locked || false,
              unlock_epoch: nftData.unlock_epoch || existing.unlock_epoch,
              rev_share: nftData.rev_share || existing.rev_share,
            }));
          }
        }
      } catch (_) { /* malformed memo — skip NFT update */ }
      break;

    // ─────────────────────────────────────────────────────────────────
    // Amber Pill geo-pioneer NFT (v2.18)
    // ─────────────────────────────────────────────────────────────────
    // AMBER_PILL_MINT: amber_pill_data = { role, region, nft_id, expires_epoch }
    // Stores the pill in the nfts Map (soulbound) AND hydrates the in-memory
    // amberPill module so weight lookups work immediately.
    case "AMBER_PILL_MINT":
      if (entry.amber_pill_data && entry.amber_pill_data.nft_id) {
        var apd = entry.amber_pill_data;
        var apCollection = "amber-pills";
        var apTokenId = apd.nft_id;
        var apKey = apCollection + "|" + apTokenId;
        // Only set if not already present — first entry wins (soulbound)
        if (!nfts.has(apKey)) {
          nfts.set(apKey, {
            owner: to,
            collection: apCollection,
            tokenId: apTokenId,
            metadata: {
              role: apd.role,
              region: apd.region,
              account: to,
              epoch: entry.epoch,
              timestamp: entry.timestamp,
              expires_epoch: apd.expires_epoch,
            },
            minted_epoch: entry.epoch,
            soulbound: true,
            time_locked: false,
            unlock_epoch: null,
            rev_share: null,
          });
        }
        // Hydrate in-memory pioneer registry (lazy require to avoid circular dep)
        try {
          var amberPill = require('../services/amberPill');
          amberPill.loadFromEntry({
            account: to,
            role: apd.role,
            region: apd.region,
            epoch: entry.epoch,
            timestamp: entry.timestamp,
            expires_epoch: apd.expires_epoch,
            nft_id: apTokenId,
          });
        } catch (_) { /* amberPill module not available — skip */ }
      }
      break;

    // ─────────────────────────────────────────────────────────────────
    // Stateful compute entries (v2.14-beta)
    // ─────────────────────────────────────────────────────────────────

    case "SERVICE_DEPLOY_STATEFUL":
      // Deploy a service with stateful: true + snapshot config.
      // service_data: { slug, deployer, runtime_spec, stateful: true,
      //   snapshot_interval_epochs, replication_factor }
      if (entry.service_data && entry.service_data.slug) {
        var sd = entry.service_data;
        var existingSvc = services.get(sd.slug) || {};
        services.set(sd.slug, Object.assign(existingSvc, {
          slug: sd.slug,
          deployer: sd.deployer || from,
          runtime_spec: sd.runtime_spec || null,
          stateful: true,
          snapshot_interval_epochs: sd.snapshot_interval_epochs || null,
          replication_factor: typeof sd.replication_factor === "number"
            ? sd.replication_factor : 3,
          last_snapshot_cid: existingSvc.last_snapshot_cid || null,
          last_snapshot_epoch: existingSvc.last_snapshot_epoch || null,
          deployed_epoch: existingSvc.deployed_epoch || entry.epoch || 0,
          last_updated_epoch: entry.epoch || 0,
          status: "active",
        }));
      }
      break;

    case "SNAPSHOT_COMMIT":
      // Record a new snapshot for a service.
      // snapshot_data: { slug, cid, replica_hosts: [] }
      if (entry.snapshot_data && entry.snapshot_data.slug && entry.snapshot_data.cid) {
        var sc = entry.snapshot_data;
        var snapshotList = snapshots.get(sc.slug) || [];
        snapshotList.push({
          cid: sc.cid,
          epoch: entry.epoch || 0,
          replicas: Array.isArray(sc.replica_hosts) ? sc.replica_hosts.slice() : [],
          timestamp: entry.timestamp || Date.now(),
        });
        snapshots.set(sc.slug, snapshotList);

        // Update last_snapshot_cid on the service record if present
        var svcForCommit = services.get(sc.slug);
        if (svcForCommit) {
          svcForCommit.last_snapshot_cid = sc.cid;
          svcForCommit.last_snapshot_epoch = entry.epoch || 0;
          services.set(sc.slug, svcForCommit);
        }
      }
      break;

    case "SNAPSHOT_RESTORE":
      // Log a restore event on the service record.
      // snapshot_data: { slug, cid, host }
      if (entry.snapshot_data && entry.snapshot_data.slug) {
        var sr = entry.snapshot_data;
        var svcForRestore = services.get(sr.slug);
        if (svcForRestore) {
          if (!svcForRestore.restore_history) svcForRestore.restore_history = [];
          svcForRestore.restore_history.push({
            cid: sr.cid || null,
            host: sr.host || from || null,
            epoch: entry.epoch || 0,
            timestamp: entry.timestamp || Date.now(),
          });
          svcForRestore.last_restore_cid = sr.cid || null;
          svcForRestore.last_restore_epoch = entry.epoch || 0;
          services.set(sr.slug, svcForRestore);
        }
      }
      break;

    // ─────────────────────────────────────────────────────────────────
    // IoT sensor + gateway entries (v2.15-beta)
    // ─────────────────────────────────────────────────────────────────

    case "SENSOR_REGISTER":
      if (entry.sensor_data && entry.sensor_data.sensor_id) {
        var ssd = entry.sensor_data;
        var existingSensor = sensors.get(ssd.sensor_id) || {};
        sensors.set(ssd.sensor_id, Object.assign(existingSensor, {
          sensor_id: ssd.sensor_id,
          owner: ssd.owner || from,
          type: ssd.type || null,
          unit: ssd.unit || null,
          decimals: ssd.decimals !== undefined ? ssd.decimals : 2,
          region: ssd.region || null,
          lora_gateway: ssd.lora_gateway || null,
          hardware_model: ssd.hardware_model || null,
          firmware_version: ssd.firmware_version || null,
          public_key: ssd.public_key || existingSensor.public_key || null,
          status: existingSensor.status === "retired" ? "retired" : "active",
          created_epoch: existingSensor.created_epoch || entry.epoch || 0,
          last_updated_epoch: entry.epoch || 0,
          last_reading_epoch: existingSensor.last_reading_epoch || null,
          total_readings: existingSensor.total_readings || 0,
        }));
      }
      break;

    case "SENSOR_KEY_REGISTER":
      if (entry.sensor_data && entry.sensor_data.sensor_id && entry.sensor_data.public_key) {
        var skrData = entry.sensor_data;
        var skrSensor = sensors.get(skrData.sensor_id);
        if (skrSensor) {
          skrSensor.public_key = skrData.public_key;
          skrSensor.key_registered_epoch = entry.epoch || 0;
          sensors.set(skrData.sensor_id, skrSensor);
        }
      }
      break;

    case "SENSOR_VOUCH":
      if (entry.vouch_data && entry.vouch_data.target_id) {
        var svData = entry.vouch_data;
        if (!sensorVouches) sensorVouches = new Map();
        var existing_vouch = sensorVouches.get(svData.target_id) || [];
        // Replace or add voucher entry (one voucher per target)
        var vi = existing_vouch.findIndex(function (v) { return v.voucher === svData.voucher; });
        var vouchRecord = {
          voucher: svData.voucher,
          target_id: svData.target_id,
          target_type: svData.target_type || "sensor",
          stake_amount: svData.stake_amount || 0,
          epoch: entry.epoch || 0,
        };
        if (vi >= 0) {
          existing_vouch[vi] = vouchRecord;
        } else {
          existing_vouch.push(vouchRecord);
        }
        sensorVouches.set(svData.target_id, existing_vouch);
      }
      break;

    case "SENSOR_READING":
      // Buffer reading for median consensus at finalization.
      if (entry.sensor_data && entry.sensor_data.sensor_id) {
        var srdData = entry.sensor_data;
        var srdKey = srdData.sensor_id + "|" + (entry.epoch || 0);
        var srdList = sensorReadings.get(srdKey) || [];
        srdList.push({
          value: srdData.value,
          metadata: srdData.metadata || {},
          submitted_at: entry.timestamp || Date.now(),
          relay_account: srdData.relay_account || null,
        });
        sensorReadings.set(srdKey, srdList);
        // Track witnesses (relay nodes) per sensor+epoch
        if (srdData.relay_account && typeof srdData.relay_account === "string") {
          var wKey = srdData.sensor_id + "|" + (entry.epoch || 0);
          var wSet = witnessMap.get(wKey);
          if (!wSet) { wSet = new Set(); witnessMap.set(wKey, wSet); }
          wSet.add(srdData.relay_account);
        }
        // Update sensor's last_reading_epoch
        var srdSensor = sensors.get(srdData.sensor_id);
        if (srdSensor) {
          srdSensor.last_reading_epoch = entry.epoch || 0;
          srdSensor.total_readings = (srdSensor.total_readings || 0) + 1;
          sensors.set(srdData.sensor_id, srdSensor);
        }
        // Register for epoch base reward tracking (lazy require avoids circular dep)
        try {
          var _settle = require("./rewardSettlement");
          var _srdOwner = srdData.sensor_id.includes("/") ? srdData.sensor_id.split("/")[0] : (entry.from || null);
          if (_srdOwner) _settle.recordSensorReading(_srdOwner, entry.epoch || 0, srdData.sensor_id);
        } catch (_) {}
      }
      break;

    case "SENSOR_DATA_COMMIT":
      // Record the blob CID for persisted sensor data for this epoch.
      if (entry.sensor_data && entry.sensor_data.sensor_id && entry.sensor_data.cid) {
        var sdcData = entry.sensor_data;
        var sdcSensor = sensors.get(sdcData.sensor_id);
        if (sdcSensor) {
          if (!sdcSensor.data_commits) sdcSensor.data_commits = [];
          sdcSensor.data_commits.push({
            cid: sdcData.cid,
            epoch: entry.epoch || 0,
            reading_count: sdcData.reading_count || 0,
            median_value: sdcData.median_value !== undefined ? sdcData.median_value : null,
          });
          // Keep last 1000 commits to bound memory
          if (sdcSensor.data_commits.length > 1000) {
            sdcSensor.data_commits = sdcSensor.data_commits.slice(-1000);
          }
          sdcSensor.last_commit_cid = sdcData.cid;
          sdcSensor.last_commit_epoch = entry.epoch || 0;
          sensors.set(sdcData.sensor_id, sdcSensor);
        }
      }
      break;

    case "GATEWAY_REGISTER":
      if (entry.gateway_data && entry.gateway_data.gateway_id) {
        var sgd = entry.gateway_data;
        var existingGateway = gateways.get(sgd.gateway_id) || {};
        gateways.set(sgd.gateway_id, Object.assign(existingGateway, {
          gateway_id: sgd.gateway_id,
          owner: sgd.owner || from,
          region: sgd.region || null,
          latitude: sgd.latitude !== undefined ? sgd.latitude : null,
          longitude: sgd.longitude !== undefined ? sgd.longitude : null,
          antenna_gain_dbi: sgd.antenna_gain_dbi !== undefined ? sgd.antenna_gain_dbi : null,
          hardware_model: sgd.hardware_model || null,
          firmware_version: sgd.firmware_version || null,
          max_sensors: sgd.max_sensors !== undefined ? sgd.max_sensors : 50,
          status: existingGateway.status === "retired" ? "retired" : "active",
          created_epoch: existingGateway.created_epoch || entry.epoch || 0,
          last_updated_epoch: entry.epoch || 0,
          last_heartbeat_epoch: existingGateway.last_heartbeat_epoch || null,
          total_heartbeats: existingGateway.total_heartbeats || 0,
        }));
        if (!gatewayHeartbeats.has(sgd.gateway_id)) {
          gatewayHeartbeats.set(sgd.gateway_id, { epochs: new Set(), last_heartbeat_epoch: null, total_heartbeats: 0 });
        }
      }
      break;

    case "GATEWAY_HEARTBEAT":
      // Update gateway stats, track last_heartbeat_epoch.
      if (entry.gateway_data && entry.gateway_data.gateway_id) {
        var ghd = entry.gateway_data;
        var ghGateway = gateways.get(ghd.gateway_id);
        if (ghGateway) {
          ghGateway.last_heartbeat_epoch = entry.epoch || 0;
          ghGateway.total_heartbeats = (ghGateway.total_heartbeats || 0) + 1;
          ghGateway.status = "active";
          if (ghd.stats) {
            ghGateway.last_stats = Object.assign({}, ghd.stats, { reported_epoch: entry.epoch || 0 });
          }
          gateways.set(ghd.gateway_id, ghGateway);
        }
        var ghStats = gatewayHeartbeats.get(ghd.gateway_id) || { epochs: new Set(), last_heartbeat_epoch: null, total_heartbeats: 0 };
        ghStats.epochs.add(entry.epoch || 0);
        ghStats.last_heartbeat_epoch = entry.epoch || 0;
        ghStats.total_heartbeats = (ghStats.total_heartbeats || 0) + 1;
        gatewayHeartbeats.set(ghd.gateway_id, ghStats);
      }
      break;

    // ─────────────────────────────────────────────────────────────────
    // Bridge entries (v2.16-alpha)
    // Pay-for-delivery: no slashing, fees always to hone_recycle.
    // ─────────────────────────────────────────────────────────────────

    case "BRIDGE_WRAP":
      // Debit user HONE; record wrap event.
      if (from && amount > 0) {
        _debit(from, "HONE", amount);
        // Fee portion already included in amount, route fee to recycle
        if (entry.bridge_data && entry.bridge_data.fee > 0) {
          _credit("hone_recycle", "HONE", entry.bridge_data.fee);
        }
        var wrapKey = from + "|" + (entry.bridge_data && entry.bridge_data.chain_id || "") + "|" + (entry.epoch || 0) + "|" + (entry.timestamp || 0);
        bridgeWraps.set(wrapKey, {
          user: from,
          chain_id: entry.bridge_data && entry.bridge_data.chain_id || null,
          amount: amount,
          fee: entry.bridge_data && entry.bridge_data.fee || 0,
          epoch: entry.epoch || 0,
        });
      }
      break;

    case "BRIDGE_UNWRAP":
      // Credit user HONE; record unwrap event.
      if (to && amount > 0) {
        _credit(to, "HONE", amount);
        if (entry.bridge_data && entry.bridge_data.fee > 0) {
          _credit("hone_recycle", "HONE", entry.bridge_data.fee);
        }
        var unwrapKey = to + "|" + (entry.bridge_data && entry.bridge_data.chain_id || "") + "|" + (entry.epoch || 0) + "|" + (entry.timestamp || 0);
        bridgeUnwraps.set(unwrapKey, {
          user: to,
          chain_id: entry.bridge_data && entry.bridge_data.chain_id || null,
          amount: amount,
          fee: entry.bridge_data && entry.bridge_data.fee || 0,
          epoch: entry.epoch || 0,
        });
      }
      break;

    case "BRIDGE_FUND":
      // Debit funder; record LP position.
      if (from && amount > 0) {
        _debit(from, "HONE", amount);
        var bfKey = from + "|" + (entry.bridge_data && entry.bridge_data.chain_id || "");
        var bfExisting = bridgeFunders.get(bfKey) || {
          funder: from,
          chain_id: entry.bridge_data && entry.bridge_data.chain_id || null,
          amount: 0,
          lock_days: 0,
          locked_epoch: entry.epoch || 0,
          status: "locked",
        };
        bfExisting.amount = _round(bfExisting.amount + amount);
        bfExisting.lock_days = entry.bridge_data && entry.bridge_data.lock_days || 30;
        bfExisting.locked_epoch = entry.epoch || 0;
        bfExisting.status = "locked";
        bridgeFunders.set(bfKey, bfExisting);
      }
      break;

    case "BRIDGE_UNLOCK":
      // Queue LP for withdrawal (status: locked → queued).
      if (from && entry.bridge_data && entry.bridge_data.chain_id) {
        var buKey = from + "|" + entry.bridge_data.chain_id;
        var buRecord = bridgeFunders.get(buKey);
        if (buRecord) {
          buRecord.status = "queued";
          buRecord.queued_epoch = entry.epoch || 0;
          bridgeFunders.set(buKey, buRecord);
        }
      }
      break;

    // ─────────────────────────────────────────────────────────────────
    // Cross-chain monitor activity (v2.17)
    // ─────────────────────────────────────────────────────────────────
    case "CROSS_CHAIN_ACTIVITY":
      // Store external chain activity observed by the chain-monitor daemon.
      // No balance mutation — this is purely an observation record.
      if (to && entry.cross_chain_data) {
        var ccData = entry.cross_chain_data;
        var ccList = crossChainActivity.get(to) || [];
        ccList.push({
          chain: ccData.chain || null,
          address: ccData.address || null,
          activity_type: ccData.activity_type || "balance_change",
          epoch: entry.epoch || 0,
          timestamp: entry.timestamp || Date.now(),
          data: Object.assign({}, ccData),
        });
        // Bound memory: keep only the most recent N entries per user
        if (ccList.length > CROSS_CHAIN_ACTIVITY_RETENTION) {
          ccList = ccList.slice(ccList.length - CROSS_CHAIN_ACTIVITY_RETENTION);
        }
        crossChainActivity.set(to, ccList);
      }
      break;

    case "KEY_ROTATE":
      if (to && accounts.has(to) && entry.key_rotate_data && entry.key_rotate_data.new_public_keys) {
        var krAcct = accounts.get(to);
        var np = entry.key_rotate_data.new_public_keys;
        if (!krAcct.public_keys) krAcct.public_keys = {};
        if (np.active)  krAcct.public_keys.active  = np.active;
        if (np.posting) krAcct.public_keys.posting = np.posting;
        if (np.memo)    krAcct.public_keys.memo    = np.memo;
        if (np.owner)   krAcct.public_keys.owner   = np.owner;
        accounts.set(to, krAcct);
      }
      break;

    case "DEVICE_AUTHORIZE":
      // Register a device public key as an authorized signer for an account.
      // entry.to = owner account, entry.device_data = { device_pubkey, scope, hardware_hash }
      if (to && entry.device_data && entry.device_data.device_pubkey) {
        var dd = entry.device_data;
        deviceKeys.set(dd.device_pubkey, {
          owner: to,
          scope: dd.scope || 'mine',
          hardware_hash: dd.hardware_hash || null,
          authorized_epoch: entry.epoch || 0,
        });
      }
      break;

    case "DEVICE_REVOKE":
      // Remove a device key authorization.
      if (entry.device_data && entry.device_data.device_pubkey) {
        deviceKeys.delete(entry.device_data.device_pubkey);
      }
      break;

    // ─────────────────────────────────────────────────────────────────
    // IoT device key registration (v3.2)
    // Authorized by the owner's posting key — cannot move funds.
    // device_id = "<owner>/<device-name>"
    // ─────────────────────────────────────────────────────────────────
    case "DEVICE_KEY_REGISTER":
      if (entry.device_key_data && entry.device_key_data.device_id) {
        var dkrd = entry.device_key_data;
        iotDeviceKeys.set(dkrd.device_id, {
          device_id: dkrd.device_id,
          owner: dkrd.owner || from,
          device_pubkey: dkrd.device_pubkey,
          registered_epoch: entry.epoch || 0,
        });
      }
      break;

    case "TOOL_CAPABILITY_REGISTER":
      // Forward to toolRegistry (lazy require avoids circular dep)
      try {
        var _toolReg = require("../mcp/toolRegistry");
        _toolReg.registerFromEntry(entry);
      } catch (_) {}
      break;

    // ── Four-tier finality anchors (v3.2) ──────────────────────────
    case "L2_ANCHOR":
    case "ETH_ANCHOR":
    case "BTC_ANCHOR": {
      var _al = {
        type: entry.type,
        epoch: entry.epoch,
        state_root: entry.state_root || null,
        anchor_level: entry.anchor_level || null,
        anchored_at: entry.anchored_at || 0,
      };
      if (entry.calldata_hex) _al.calldata_hex = entry.calldata_hex;
      if (entry.op_return_hex) _al.op_return_hex = entry.op_return_hex;
      anchorLog.push(_al);
      if (anchorLog.length > ANCHOR_LOG_CAP) {
        anchorLog.splice(0, anchorLog.length - ANCHOR_LOG_CAP);
      }
      break;
    }

    // ─────────────────────────────────────────────────────────────────
    // Device Stake & Rent (v3.5) — pooled rent-auction model
    // ─────────────────────────────────────────────────────────────────

    case "DEVICE_YIELD_STAKE": {
      // yield_stake_data: { device_id, staker, stake_amount, rent_bid, rent_mode, owner, entry_epoch }
      var dysData = entry.yield_stake_data || {};
      var dysDeviceId = dysData.device_id;
      var dysStaker = dysData.staker || from;
      var dysStakeAmount = _round(dysData.stake_amount || amount || 0);
      var dysOwner = dysData.owner;
      var dysRentBid = _round(dysData.rent_bid || 0);
      var dysRentMode = dysData.rent_mode === "stake" ? "stake" : "earnings";
      var dysEntryEpoch = dysData.entry_epoch || entry.epoch || 0;
      var DYS_MAX_SLOTS = 10;

      if (!dysDeviceId || !dysStaker || dysStakeAmount <= 0) break;

      // Debit the staker's balance by the full stake amount
      var dysDebitOk = _debit(dysStaker, "HONE", dysStakeAmount);
      if (!dysDebitOk) break; // insufficient balance

      // Tribute: to owner (unless staker === owner).
      // If the entry carries an explicit tribute value, honour it; otherwise
      // fall back to the 1% formula so replayed blocks remain deterministic.
      var dysTribute = 0;
      if (dysOwner && dysStaker !== dysOwner) {
        var _explicitTribute = dysData.tribute !== undefined ? _round(dysData.tribute) : null;
        dysTribute = _explicitTribute !== null ? _explicitTribute : _round(dysStakeAmount * 0.01);
        if (dysTribute > 0) _credit(dysOwner, "HONE", dysTribute);
      }

      // Multi-slot rent-auction model (up to 10 slots per device).
      // Slots are ordered by rent_bid desc, then stake_amount desc.
      // If pool is full (10 slots), the new staker must beat the lowest slot.
      if (!deviceStakerPools.has(dysDeviceId)) {
        deviceStakerPools.set(dysDeviceId, new Map());
      }
      var dysPool = deviceStakerPools.get(dysDeviceId);

      // Re-staking: update existing position
      if (dysPool.has(dysStaker)) {
        var _dysExisting = dysPool.get(dysStaker);
        // Additional stake — add to existing position
        _dysExisting.stake_amount = _round((_dysExisting.stake_amount || 0) + dysStakeAmount);
        _dysExisting.rent_bid = dysRentBid;
        _dysExisting.rent_mode = dysRentMode;
        _dysExisting.tribute_paid = _round((_dysExisting.tribute_paid || 0) + dysTribute);
      } else if (dysPool.size < DYS_MAX_SLOTS) {
        // Pool has room — just add
        var dysNetStake = dysData.net_stake !== undefined ? _round(dysData.net_stake) : _round(dysStakeAmount - dysTribute);
        dysPool.set(dysStaker, {
          staker: dysStaker,
          stake_amount: dysStakeAmount,
          original_stake: dysStakeAmount,
          net_stake: dysNetStake,
          tribute_paid: dysTribute,
          slot: dysPool.size + 1,
          rent_bid: dysRentBid,
          rent_mode: dysRentMode,
          entry_epoch: dysEntryEpoch,
          rent_paid_total: 0,
        });
      } else {
        // Pool is full — evict lowest-ranked staker if new staker beats them
        var _dysSortedFull = Array.from(dysPool.values()).sort(function (a, b) {
          if (b.rent_bid !== a.rent_bid) return b.rent_bid - a.rent_bid;
          return b.stake_amount - a.stake_amount;
        });
        var _dysLowest = _dysSortedFull[_dysSortedFull.length - 1];
        var _dysNewBeats = dysRentBid > _dysLowest.rent_bid ||
          (dysRentBid === _dysLowest.rent_bid && dysStakeAmount > _dysLowest.stake_amount);
        if (_dysNewBeats) {
          // Refund the evicted staker
          var _dysEvictRefund = _round((_dysLowest.original_stake || _dysLowest.stake_amount) - (_dysLowest.tribute_paid || 0));
          if (_dysEvictRefund > 0) _credit(_dysLowest.staker, "HONE", _dysEvictRefund);
          dysPool.delete(_dysLowest.staker);
          // Add new staker
          var dysNetStake2 = dysData.net_stake !== undefined ? _round(dysData.net_stake) : _round(dysStakeAmount - dysTribute);
          dysPool.set(dysStaker, {
            staker: dysStaker,
            stake_amount: dysStakeAmount,
            original_stake: dysStakeAmount,
            net_stake: dysNetStake2,
            tribute_paid: dysTribute,
            slot: DYS_MAX_SLOTS,
            rent_bid: dysRentBid,
            rent_mode: dysRentMode,
            entry_epoch: dysEntryEpoch,
            rent_paid_total: 0,
          });
        } else {
          // New staker doesn't beat anyone — refund their stake + tribute
          _credit(dysStaker, "HONE", dysStakeAmount);
          if (dysTribute > 0 && dysOwner) {
            _debit(dysOwner, "HONE", dysTribute);
            _credit(dysStaker, "HONE", dysTribute);
          }
          break;
        }
      }

      // Re-sort pool and assign slots
      var _dysSorted = Array.from(dysPool.values()).sort(function (a, b) {
        if (b.rent_bid !== a.rent_bid) return b.rent_bid - a.rent_bid;
        return b.stake_amount - a.stake_amount;
      });
      for (var _dysi = 0; _dysi < _dysSorted.length; _dysi++) {
        _dysSorted[_dysi].slot = _dysi + 1;
      }

      // Track staker→device mapping
      if (!deviceStakerIndex) deviceStakerIndex = new Map();
      if (!deviceStakerIndex.has(dysStaker)) deviceStakerIndex.set(dysStaker, new Set());
      deviceStakerIndex.get(dysStaker).add(dysDeviceId);

      break;
    }

    case "DEVICE_YIELD_UNSTAKE": {
      // yield_stake_data: { device_id, staker }
      var dyuData = entry.yield_stake_data || {};
      var dyuDeviceId = dyuData.device_id;
      var dyuStaker = dyuData.staker || from;

      if (!dyuDeviceId || !dyuStaker) break;

      var dyuPool = deviceStakerPools.get(dyuDeviceId);
      if (!dyuPool || !dyuPool.has(dyuStaker)) break;

      var dyuRecord = dyuPool.get(dyuStaker);
      // Return current remaining stake_amount (already reduced by rent deductions).
      // If entry specifies returned_amount explicitly, honour that.
      var dyuReturnAmount;
      if (dyuData.returned_amount !== undefined && dyuData.returned_amount > 0) {
        dyuReturnAmount = _round(dyuData.returned_amount);
      } else {
        dyuReturnAmount = _round(dyuRecord.stake_amount || 0);
      }
      if (dyuReturnAmount > 0) _credit(dyuStaker, "HONE", dyuReturnAmount);
      dyuPool.delete(dyuStaker);
      break;
    }

    case "DEVICE_YIELD_RENT": {
      // yield_stake_data: { device_id, staker, rent_amount, owner }
      var dyrData = entry.yield_stake_data || {};
      var dyrDeviceId = dyrData.device_id;
      var dyrStaker = dyrData.staker;
      var dyrRentAmount = _round(dyrData.rent_amount || 0);
      var dyrOwner = dyrData.owner;

      if (!dyrDeviceId || !dyrStaker || dyrRentAmount <= 0) break;

      var dyrPool = deviceStakerPools.get(dyrDeviceId);
      if (!dyrPool || !dyrPool.has(dyrStaker)) break;

      var dyrRecord = dyrPool.get(dyrStaker);

      if (dyrRecord.rent_mode === "stake") {
        dyrRecord.stake_amount = _round(dyrRecord.stake_amount - dyrRentAmount);
      } else {
        dyrRecord.stake_amount = _round(dyrRecord.stake_amount - dyrRentAmount);
      }

      dyrRecord.rent_paid_total = _round((dyrRecord.rent_paid_total || 0) + dyrRentAmount);

      // Credit owner by rent_amount
      if (dyrOwner && dyrRentAmount > 0) _credit(dyrOwner, "HONE", dyrRentAmount);

      // Evict if stake depleted
      if (dyrRecord.stake_amount <= 0) {
        dyrPool.delete(dyrStaker);
        // Recompute slots
        var dyrSorted = Array.from(dyrPool.values()).sort(function (a, b) {
          if (b.rent_bid !== a.rent_bid) return b.rent_bid - a.rent_bid;
          return b.stake_amount - a.stake_amount;
        });
        for (var _dyri = 0; _dyri < dyrSorted.length; _dyri++) {
          dyrSorted[_dyri].slot = _dyri + 1;
        }
      }
      break;
    }

    case "DEVICE_YIELD_RENT_MODE": {
      // yield_stake_data: { device_id, staker, rent_mode }
      var dyrmData = entry.yield_stake_data || {};
      var dyrmDeviceId = dyrmData.device_id;
      var dyrmStaker = dyrmData.staker;
      var dyrmMode = dyrmData.rent_mode === "stake" ? "stake" : "earnings";

      if (!dyrmDeviceId || !dyrmStaker) break;

      var dyrmPool = deviceStakerPools.get(dyrmDeviceId);
      if (!dyrmPool || !dyrmPool.has(dyrmStaker)) break;

      dyrmPool.get(dyrmStaker).rent_mode = dyrmMode;
      break;
    }

    case "DEVICE_YIELD_AUTO_STAKE": {
      // yield_stake_data: { device_id, owner, pct }
      var dyasData = entry.yield_stake_data || {};
      var dyasDeviceId = dyasData.device_id;
      var dyasOwner = dyasData.owner;
      var dyasPct = Math.max(0, Math.min(50, Number(dyasData.pct) || 0));

      if (!dyasDeviceId || !dyasOwner) break;

      deviceAutoStake.set(dyasDeviceId, { owner: dyasOwner, pct: dyasPct });
      break;
    }

    // ─────────────────────────────────────────────────────────────────
    // Name Auction (v3.6)
    // ─────────────────────────────────────────────────────────────────

    case "NAME_AUCTION_OPEN": {
      var naoData = entry.auction_data || {};
      var naoName = naoData.name;
      if (!naoName) break;
      var existing = nameAuctions.get(naoName);
      if (existing && (existing.status === "open" || existing.status === "settled")) break;
      nameAuctions.set(naoName, {
        name: naoName,
        seller: naoData.seller || from,
        start_price_usd: naoData.start_price_usd || 0,
        min_bid_increment_usd: naoData.min_bid_increment_usd || 0,
        auction_end_epoch: naoData.auction_end_epoch || 0,
        open_epoch: entry.epoch || 0,
        bids: [],
        status: "open",
        winner: null,
      });
      break;
    }

    case "NAME_AUCTION_BID": {
      var nabData = entry.auction_data || {};
      var nabName = nabData.name;
      if (!nabName) break;
      var nabAuction = nameAuctions.get(nabName);
      if (!nabAuction || nabAuction.status !== "open") break;
      // Validate bid amount: must meet start_price or beat current_top + increment
      var nabBidUsd = nabData.bid_usd || 0;
      var nabTopBid = nabAuction.bids.length > 0
        ? nabAuction.bids[nabAuction.bids.length - 1].bid_usd
        : 0;
      var nabMinRequired = nabAuction.bids.length === 0
        ? nabAuction.start_price_usd
        : nabTopBid + nabAuction.min_bid_increment_usd;
      if (nabBidUsd < nabMinRequired) break;
      nabAuction.bids.push({
        bidder: nabData.bidder || from,
        bid_usd: nabBidUsd,
        chain: nabData.chain,
        tx_hash: nabData.tx_hash,
        hone_account: nabData.hone_account,
        hone_pubkeys: nabData.hone_pubkeys || {},
        bid_epoch: entry.epoch || 0,
      });
      // Keep bids sorted ascending by bid_usd (last entry = highest)
      nabAuction.bids.sort(function(a, b) { return a.bid_usd - b.bid_usd; });
      break;
    }

    case "NAME_AUCTION_SETTLE": {
      var nasData = entry.auction_data || {};
      var nasName = nasData.name;
      if (!nasName) break;
      var nasAuction = nameAuctions.get(nasName);
      if (!nasAuction || nasAuction.status !== "open") break;
      // Find highest bid
      var nasWinBid = null;
      for (var _nb = 0; _nb < nasAuction.bids.length; _nb++) {
        if (!nasWinBid || nasAuction.bids[_nb].bid_usd > nasWinBid.bid_usd) {
          nasWinBid = nasAuction.bids[_nb];
        }
      }
      // No bids — settle is a no-op; leave status as open
      if (!nasWinBid) break;
      nasAuction.status = "settled";
      nasAuction.winner = nasWinBid ? nasWinBid.bidder : null;
      if (nasWinBid && nasWinBid.hone_pubkeys) {
        // Transfer account keys to winner (same as KEY_ROTATE)
        var nasAcct = accounts.get(nasName);
        if (nasAcct) {
          nasAcct.public_keys = Object.assign({}, nasAcct.public_keys || {}, nasWinBid.hone_pubkeys);
        }
      }
      break;
    }

    case "NAME_AUCTION_CANCEL": {
      var nacData = entry.auction_data || {};
      var nacName = nacData.name;
      if (!nacName) break;
      var nacAuction = nameAuctions.get(nacName);
      if (!nacAuction || nacAuction.status !== "open") break;
      nacAuction.status = "cancelled";
      if (!nacAuction.cancelled_epoch) nacAuction.cancelled_epoch = entry.epoch || 0;
      break;
    }

    case "NAME_DELEGATE": {
      var ndData = entry.delegate_data || {};
      var ndName = ndData.name;
      if (!ndName) break;
      nameDelegate.set(ndName, { delegated: !!ndData.delegated, epoch: entry.epoch || 0 });
      break;
    }

    default:
      // Unknown type — safe to skip. New types will be added here.
      break;
  }
}

function applyEntries(entries) {
  if (!Array.isArray(entries)) return;
  for (var i = 0; i < entries.length; i++) applyEntry(entries[i]);
  assertBalanceIntegrity("applyEntries");
}

// ─────────────────────────────────────────────────────────────────
// Epoch + proof mutators
// ─────────────────────────────────────────────────────────────────

function setEpoch(epochNumber, metadata) {
  if (typeof epochNumber !== "number" || epochNumber < 0) return;
  var existing = epochs.get(epochNumber) || {};
  epochs.set(epochNumber, Object.assign(existing, metadata || {}));
  if (epochNumber > chainHeight) chainHeight = epochNumber;
}

function setMiningProofs(epochNumber, proofs) {
  if (typeof epochNumber !== "number" || !Array.isArray(proofs)) return;
  miningProofsByEpoch.set(epochNumber, proofs.slice());
}

function addMiningProof(epochNumber, proof) {
  if (typeof epochNumber !== "number" || !proof) return;
  var list = miningProofsByEpoch.get(epochNumber);
  if (!list) { list = []; miningProofsByEpoch.set(epochNumber, list); }
  // Dedupe by miner within the epoch
  var miner = proof.miner || proof.node_id;
  for (var i = 0; i < list.length; i++) {
    if ((list[i].miner || list[i].node_id) === miner) { list[i] = proof; return; }
  }
  list.push(proof);
}

function setComputeProofs(epochNumber, proofs) {
  if (typeof epochNumber !== "number" || !Array.isArray(proofs)) return;
  computeProofsByEpoch.set(epochNumber, proofs.slice());
}

function addComputeProof(epochNumber, proof) {
  if (typeof epochNumber !== "number" || !proof) return;
  var list = computeProofsByEpoch.get(epochNumber);
  if (!list) { list = []; computeProofsByEpoch.set(epochNumber, list); }
  list.push(proof);
}

function setChainHeight(n) {
  if (typeof n === "number" && n > chainHeight) chainHeight = n;
}

// ─────────────────────────────────────────────────────────────────
// Dynamic block cap (v3.0)
// ─────────────────────────────────────────────────────────────────

function getCurrentBlockCap() {
  return currentBlockCap;
}

function setCurrentBlockCap(cap) {
  if (typeof cap === "number" && cap > 0) {
    currentBlockCap = cap;
  }
}

// ─────────────────────────────────────────────────────────────────
// Account getters
// ─────────────────────────────────────────────────────────────────

function getBalance(username, token) {
  var units = balances.get(_balanceKey(username, token)) || 0;
  return fromUnits(units);
}

function getBalanceUnits(username, token) {
  return balances.get(_balanceKey(username, token)) || 0;
}

function getTokenBalances(username) {
  var result = {};
  if (!username) return result;
  var prefix = username + "|";
  for (var entry of balances) {
    if (entry[0].indexOf(prefix) === 0) {
      var tok = entry[0].slice(prefix.length);
      if (entry[1] !== 0) result[tok] = fromUnits(entry[1]);
    }
  }
  return result;
}

function getAccount(username) {
  if (!username) return null;
  var acc = accounts.get(username);
  if (!acc) return null;
  return {
    username: username,
    created_epoch: acc.created_epoch,
    public_keys: acc.public_keys || {},
    chain_addresses: acc.chain_addresses || {},
    heartbeat_epoch: acc.heartbeat_epoch || 0,
    balance: getBalance(username, "HONE"),
    delegated_balance: getDelegatedBalance(username, "HONE"),
    staked: (stakes.get(username) || { total_staked: 0 }).total_staked,
    // v2.10.2: multi-capability node registration fields
    node_types: acc.node_types || undefined,
    storage_capacity_gb: acc.storage_capacity_gb,
    service_capacity: acc.service_capacity,
    lora_region: acc.lora_region,
    p2p_address: acc.p2p_address,
    permissioned: acc.permissioned,
    last_registered_epoch: acc.last_registered_epoch,
  };
}

function hasAccount(username) {
  return accounts.has(username);
}

function getAllAccounts() {
  var result = [];
  for (var entry of accounts) {
    result.push({
      username: entry[0],
      created_epoch: entry[1].created_epoch,
      public_keys: entry[1].public_keys,
      chain_addresses: entry[1].chain_addresses,
    });
  }
  return result;
}

function getAccountCount() {
  return accounts.size;
}

/**
 * Bootstrap: directly inject a public key for an account that was created
 * before the key system existed (e.g., genesis accounts). Does not write a
 * ledger entry — local trust only, derived from HONE_ACTIVE_KEY env var.
 */
/**
 * Look up the owner account for a device public key.
 * Returns { owner, scope, hardware_hash, authorized_epoch } or null.
 */
function getDeviceOwner(devicePubkey) {
  return deviceKeys.get(devicePubkey) || null;
}

/**
 * Get all device public keys authorized for an account.
 */
function getAccountDevices(username) {
  var result = [];
  for (var entry of deviceKeys) {
    if (entry[1].owner === username) {
      result.push({ device_pubkey: entry[0], ...entry[1] });
    }
  }
  return result;
}

/**
 * Check whether a device pubkey is registered on-chain for the given owner.
 */
function isDeviceAuthorized(devicePubkey, owner) {
  var rec = deviceKeys.get(devicePubkey);
  return rec ? rec.owner === owner : false;
}

/**
 * Get an IoT device key record by device_id.
 * Returns { device_id, owner, device_pubkey, registered_epoch } or null.
 */
function getDeviceKey(deviceId) {
  return iotDeviceKeys.get(deviceId) || null;
}

/**
 * Get all IoT device keys registered by a given owner.
 * Returns an array of { device_id, owner, device_pubkey, registered_epoch }.
 */
function getDevicesByOwner(owner) {
  var result = [];
  for (var entry of iotDeviceKeys) {
    if (entry[1].owner === owner) result.push(entry[1]);
  }
  return result;
}

/**
 * Get the set of relay accounts (witnesses) that submitted a reading
 * for a given sensor+epoch. Returns a Set (may be empty).
 */
function getWitnesses(sensorId, epoch) {
  var key = sensorId + "|" + (epoch || 0);
  return witnessMap.get(key) || new Set();
}

function bootstrapAccountKey(username, keyRole, publicKey) {
  var acct = accounts.get(username);
  if (!acct) return;
  if (!acct.public_keys) acct.public_keys = {};
  acct.public_keys[keyRole] = publicKey;
}

// ─────────────────────────────────────────────────────────────────
// Token / NFT getters
// ─────────────────────────────────────────────────────────────────

function getToken(symbol) {
  return tokens.get(symbol) || null;
}

function getAllTokens() {
  return Array.from(tokens.values());
}

function getNFT(collection, tokenId) {
  return nfts.get(collection + "|" + tokenId) || null;
}

function getNFTsByOwner(username) {
  var result = [];
  for (var entry of nfts) {
    if (entry[1].owner === username) result.push(entry[1]);
  }
  return result;
}

function getNFTsByCollection(collection) {
  var result = [];
  var prefix = collection + "|";
  for (var entry of nfts) {
    if (entry[0].indexOf(prefix) === 0) result.push(entry[1]);
  }
  return result;
}

function getAllNFTs() {
  return Array.from(nfts.values());
}

// ─────────────────────────────────────────────────────────────────
// Staking / delegation getters
// ─────────────────────────────────────────────────────────────────

function getStakePool(username) {
  return stakes.get(username) || null;
}

/**
 * Get the total staked HONE for a username (node account).
 * Returns 0 if the account has no stake pool.
 */
function getStake(username) {
  var pool = stakes.get(username);
  return pool ? (pool.total_staked || 0) : 0;
}

function getAllStakePools() {
  var result = [];
  for (var entry of stakes) {
    result.push({ username: entry[0], ...entry[1] });
  }
  return result;
}

function getDelegatedBalance(username, token) {
  var key = _balanceKey(username, token || "HONE");
  return fromUnits(delegatedReceived.get(key) || 0);
}

function getDelegation(from, to) {
  return delegations.get(from + "|" + to) || null;
}

function getDelegationsByDelegator(from) {
  var result = [];
  var prefix = from + "|";
  for (var entry of delegations) {
    if (entry[0].indexOf(prefix) === 0) {
      result.push({ to: entry[0].slice(prefix.length), ...entry[1] });
    }
  }
  return result;
}

function getDelegationsByRecipient(to) {
  var result = [];
  for (var entry of delegations) {
    var parts = entry[0].split("|");
    if (parts[1] === to) {
      result.push({ from: parts[0], ...entry[1] });
    }
  }
  return result;
}

function getTotalStaked() {
  var total = 0;
  for (var s of stakes.values()) total += s.total_staked;
  return _round(total);
}

// ─────────────────────────────────────────────────────────────────
// Escrow getters
// ─────────────────────────────────────────────────────────────────

function getEscrow(requestId) {
  return escrows.get(requestId) || null;
}

function getEscrowsByPayer(username) {
  var result = [];
  for (var entry of escrows) {
    if (entry[1].payer === username) result.push({ requestId: entry[0], ...entry[1] });
  }
  return result;
}

function getActiveEscrows() {
  var result = [];
  for (var entry of escrows) {
    if (entry[1].status === "locked") result.push({ requestId: entry[0], ...entry[1] });
  }
  return result;
}

// ─────────────────────────────────────────────────────────────────
// Epoch + proof getters
// ─────────────────────────────────────────────────────────────────

function getEpoch(epochNumber) {
  return epochs.get(epochNumber) || null;
}

/**
 * Return the VRF beacon seed stored for an epoch, or null if not yet
 * committed.  Consumers fall back to crypto.randomBytes when null.
 */
function getEpochVrfSeed(epochNumber) {
  var ep = epochs.get(epochNumber);
  if (!ep) return null;
  return ep.vrf_seed || ep.beacon || null;
}

function getLatestEpoch() {
  return chainHeight >= 0 ? epochs.get(chainHeight) : null;
}

function getChainHeight() {
  return chainHeight;
}

function getRecentEpochs(n) {
  if (chainHeight < 0) return [];
  var result = [];
  for (var i = chainHeight; i > Math.max(0, chainHeight - n); i--) {
    if (epochs.has(i)) result.push({ epoch_number: i, ...epochs.get(i) });
  }
  return result;
}

function getMiningProofs(epochNumber) {
  return miningProofsByEpoch.get(epochNumber) || [];
}

/**
 * Get the persisted finalization consensus result for an epoch, or null.
 * Replayed from FINALIZATION_CONSENSUS ledger entries — survives restarts.
 */
function getFinalizedConsensus(epochNumber) {
  return finalizedConsensusByEpoch.get(epochNumber) || null;
}

function getComputeProofs(epochNumber) {
  return computeProofsByEpoch.get(epochNumber) || [];
}

/**
 * Collect compute proofs from a window of epochs [epochNumber - lookback, epochNumber].
 * Fire-and-forget inference completes asynchronously and may be credited to epoch N+1
 * or N+2 when the miner is busy. This sweeps the window to capture all recent work.
 * Deduplicates by result_hash so no proof is credited twice.
 */
function getRecentComputeProofs(epochNumber, lookback) {
  lookback = lookback || 3;
  var seen = new Set();
  var proofs = [];
  for (var e = epochNumber; e >= Math.max(0, epochNumber - lookback); e--) {
    var list = computeProofsByEpoch.get(e) || [];
    for (var p of list) {
      var key = (p.result_hash || '') + '|' + (p.node_id || '');
      if (!seen.has(key)) {
        seen.add(key);
        proofs.push(p);
      }
    }
  }
  return proofs;
}

/**
 * Return the last up-to-100 anchor log entries (L2/ETH/BTC anchors).
 * Anchors are written by finalityAnchoring.js and also stored here
 * when applyEntry processes L2_ANCHOR / ETH_ANCHOR / BTC_ANCHOR entries.
 */
function getRecentAnchors() {
  return anchorLog.slice();
}

function getMinerCount() {
  // Count distinct miners who have earned a mining reward in the last 100 epochs
  var miners = new Set();
  for (var epoch of miningProofsByEpoch.keys()) {
    if (epoch >= chainHeight - 100) {
      var proofs = miningProofsByEpoch.get(epoch);
      for (var p of proofs) if (p.miner) miners.add(p.miner);
    }
  }
  return miners.size;
}

// ─────────────────────────────────────────────────────────────────
// Project getters
// ─────────────────────────────────────────────────────────────────

function getProject(name) {
  return projects.get(name) || null;
}

function getAllProjects() {
  var result = [];
  for (var entry of projects) {
    result.push({ name: entry[0], ...entry[1] });
  }
  return result;
}

function getProjectRevenueSplit(projectName) {
  return projectRevenueSplits.get(projectName) || null;
}

// ── Private encrypted file storage (v3.1.110+) ───────────────────────────────

function getStoredFile(storageId) {
  return storedFiles.get(storageId) || null;
}

function getStoredFilesByOwner(owner) {
  var result = [];
  storedFiles.forEach(function(f, sid) {
    if (f.owner === owner) result.push(Object.assign({ storage_id: sid }, f));
  });
  return result;
}

function getShard(shardId) {
  return shardPtrs.get(shardId) || null;
}

function getShardsByOwner(owner) {
  var result = [];
  shardPtrs.forEach(function(s, id) {
    if (s.owner === owner) result.push({ shard_id: id, encrypted_ptr: s.encrypted_ptr, created_epoch: s.created_epoch });
  });
  return result;
}

// ── Community model registry (v3.1.97+) ──────────────────────────────────────

function getCommunityModel(modelId) {
  return communityModels.get(modelId) || null;
}

function getAllCommunityModels() {
  return Array.from(communityModels.values());
}

// Used by rewardSettlement to look up royalty % for a given model at reward time.
function getModelRoyalty(modelId) {
  var m = communityModels.get(modelId);
  if (!m || m.royalty_percent <= 0) return null;
  return { uploader: m.uploader, royalty_percent: m.royalty_percent };
}

function getEarnings(account) {
  return earnings.get(account) || { mining: 0, clock: 0, storage: 0, iot: 0, verifier: 0, service: 0, inference_split: 0, total: 0 };
}

// ─────────────────────────────────────────────────────────────────
// Commerce getters (v2.10)
// ─────────────────────────────────────────────────────────────────

function getStore(seller) {
  return stores.get(seller) || null;
}

function getAllStores(filter) {
  var result = [];
  for (var s of stores) {
    if (filter && filter.status && s[1].status !== filter.status) continue;
    result.push(s[1]);
  }
  return result;
}

function getStoreActiveProductCount(seller) {
  var count = 0;
  for (var pe of products) {
    if (pe[1].seller === seller && pe[1].status === "active") count++;
  }
  return count;
}

function getProduct(productId) {
  return products.get(productId) || null;
}

function getAllProducts(filter) {
  var result = [];
  for (var entry of products) {
    var p = entry[1];
    if (filter) {
      if (filter.status && p.status !== filter.status) continue;
      if (filter.seller && p.seller !== filter.seller) continue;
      if (filter.category && p.category !== filter.category) continue;
    }
    result.push(p);
  }
  return result;
}

function getProductsBySeller(seller) {
  return getAllProducts({ seller: seller });
}

function getOrder(orderId) {
  return orders.get(orderId) || null;
}

function getOrdersByBuyer(buyer) {
  var result = [];
  for (var entry of orders) {
    if (entry[1].buyer === buyer) result.push(entry[1]);
  }
  return result;
}

function getOrdersBySeller(seller) {
  var result = [];
  for (var entry of orders) {
    if (entry[1].seller === seller) result.push(entry[1]);
  }
  return result;
}

function getReputation(targetType, targetId) {
  return reputation.get(targetType + "|" + targetId) || null;
}

function getReputationVote(voter, targetType, targetId) {
  return reputationVotes.get(voter + "|" + targetType + "|" + targetId) || null;
}

// ─────────────────────────────────────────────────────────────────
// Blob getters (HONE-FS, v2.11+)
// ─────────────────────────────────────────────────────────────────

function getBlobCommit(cid) {
  return blobs.get(cid) || null;
}

function getAllBlobCommits(filter) {
  var result = [];
  for (var entry of blobs) {
    var b = entry[1];
    if (filter) {
      if (filter.uploader && b.uploader !== filter.uploader) continue;
      if (filter.host && b.hosts.indexOf(filter.host) === -1) continue;
    }
    result.push(b);
  }
  return result;
}

function getBlobCommitsByHost(host) {
  return getAllBlobCommits({ host: host });
}

function getBlobCommitsByUploader(uploader) {
  return getAllBlobCommits({ uploader: uploader });
}

// ─────────────────────────────────────────────────────────────────
// Storage heartbeat getters + uptime calculation (v2.11.2+)
// ─────────────────────────────────────────────────────────────────

function getStorageHeartbeat(host) {
  return storageHeartbeats.get(host) || null;
}

function getAllStorageHosts() {
  var result = [];
  for (var entry of storageHeartbeats) {
    result.push(entry[1]);
  }
  return result;
}

/**
 * Compute uptime factor (0.0 to 1.0) for a storage host over a window of
 * epochs ending at `currentEpoch`. Formula:
 *
 *   uptime_factor = min(1.0, heartbeats_in_window / expected_heartbeats)
 *
 * Where expected_heartbeats assumes the host should heartbeat at least
 * once every `heartbeatInterval` epochs. Default interval is 5 epochs
 * (~2.5 min at 30 sec epochs) — frequent enough to catch downtime quickly
 * but not so frequent that flaky home ISPs get penalized for brief drops.
 *
 * Returns 0 if host has no heartbeat record at all.
 */
function getStorageUptimeFactor(host, currentEpoch, windowEpochs, heartbeatInterval) {
  windowEpochs = windowEpochs || 100;
  heartbeatInterval = heartbeatInterval || 5;
  var record = storageHeartbeats.get(host);
  if (!record) return 0;

  var windowStart = Math.max(0, currentEpoch - windowEpochs + 1);
  var countInWindow = 0;
  for (var i = 0; i < record.heartbeats.length; i++) {
    var hb = record.heartbeats[i];
    if (hb.epoch >= windowStart && hb.epoch <= currentEpoch) {
      countInWindow++;
    }
  }

  var expected = Math.max(1, Math.floor(windowEpochs / heartbeatInterval));
  var factor = countInWindow / expected;
  if (factor > 1.0) factor = 1.0;
  if (factor < 0) factor = 0;
  return _round(factor);
}

/**
 * Get all hosts that have heartbeated recently (within the last
 * `recentEpochs` epochs). Used by the auto-selector when picking
 * hosts for new BLOB_STORE_COMMIT entries.
 */
function getActiveStorageHosts(currentEpoch, recentEpochs) {
  recentEpochs = recentEpochs || 100;
  var threshold = currentEpoch - recentEpochs;
  var result = [];
  for (var entry of storageHeartbeats) {
    if ((entry[1].last_heartbeat_epoch || 0) >= threshold) {
      result.push(entry[1]);
    }
  }
  return result;
}

/**
 * Get unique host account names that sent a STORAGE_HEARTBEAT in exactly
 * `epoch`. Used by the block-emission reward distributor (v2.13.4+) to
 * identify which storage hosts deserve a share of the storage pool for
 * this epoch.
 *
 * Returns an array of account name strings (deduped). Empty array if no
 * hosts heartbeated this epoch.
 */
function getStorageHostsForEpoch(epoch) {
  var seen = new Set();
  for (var entry of storageHeartbeats) {
    var record = entry[1];
    // Check if any heartbeat in the rolling window matches this epoch
    if (record.heartbeats) {
      for (var i = 0; i < record.heartbeats.length; i++) {
        if (record.heartbeats[i].epoch === epoch) {
          seen.add(record.host);
          break;
        }
      }
    }
  }
  return Array.from(seen);
}

/**
 * Record bytes delivered by a storage host for a given epoch.
 * Called by blob-serving routes after successful file delivery.
 */
function recordStorageDelivery(host, bytes, epoch) {
  if (!host || !epoch || epoch < 0 || !(bytes > 0)) return;
  var ep = Math.floor(epoch);
  if (!storageDeliveryByEpoch.has(ep)) storageDeliveryByEpoch.set(ep, new Map());
  var dmap = storageDeliveryByEpoch.get(ep);
  dmap.set(host, (dmap.get(host) || 0) + bytes);
  if (ep > _currentStorageDeliveryEpoch) {
    _currentStorageDeliveryEpoch = ep;
    for (var dkey of storageDeliveryByEpoch.keys()) {
      if (dkey < ep - 20) storageDeliveryByEpoch.delete(dkey);
    }
  }
}

/**
 * Get bytes delivered per host for a 3-epoch window ending at epochNumber.
 * Returns { [host]: bytes } — delivery signal for storage reward distribution.
 */
function getStorageWorkByEpoch(epochNumber) {
  var WINDOW = 3;
  var result = {};
  for (var i = 0; i <= WINDOW; i++) {
    var dmap = storageDeliveryByEpoch.get(epochNumber - i);
    if (!dmap) continue;
    for (var entry of dmap) {
      result[entry[0]] = (result[entry[0]] || 0) + entry[1];
    }
  }
  return result;
}

// ─────────────────────────────────────────────────────────────────
// Blob challenge-response getters (v2.11.2+)
// No slashing — these are for payout weighting + reputation only.
// See feedback_storage_no_slash.md.
// ─────────────────────────────────────────────────────────────────

function getBlobChallenge(challengeId) {
  return blobChallenges.get(challengeId) || null;
}

function getBlobChallengeStats(host) {
  return blobChallengeStats.get(host) || {
    total_issued: 0,
    total_passed: 0,
    total_failed: 0,
    last_challenge_epoch: 0,
  };
}

/**
 * Challenge success rate for a host, 0.0 to 1.0.
 * Returns 1.0 if the host has no challenge history (benefit of the doubt
 * for new hosts — they're not punished for not being picked yet).
 */
function getChallengeSuccessRate(host) {
  var stats = blobChallengeStats.get(host);
  if (!stats || stats.total_passed + stats.total_failed === 0) return 1.0;
  var total = stats.total_passed + stats.total_failed;
  return _round(stats.total_passed / total);
}

/**
 * Get pending challenges for a host — ones they've been asked to
 * respond to but haven't yet. Used by host-side runners to know what
 * they need to answer.
 */
function getPendingChallengesForHost(host) {
  var result = [];
  for (var entry of blobChallenges) {
    if (entry[1].host === host && entry[1].status === "pending") {
      result.push(entry[1]);
    }
  }
  return result;
}

/**
 * Get all challenges for a specific CID — audit trail.
 */
function getChallengesForCid(cid) {
  var result = [];
  for (var entry of blobChallenges) {
    if (entry[1].cid === cid) result.push(entry[1]);
  }
  return result;
}

// ─────────────────────────────────────────────────────────────────
// Device Stake & Rent getters (v3.5)
// ─────────────────────────────────────────────────────────────────

/**
 * Get sorted staker pool for a device (slot 1 first).
 * Returns array of StakerRecord sorted by slot ASC.
 */
function getDeviceStakerPool(deviceId) {
  var pool = deviceStakerPools.get(deviceId);
  if (!pool || pool.size === 0) return [];
  return Array.from(pool.values()).slice().sort(function (a, b) { return a.slot - b.slot; });
}

/**
 * Get set of all staker account names for a device.
 */
function getAllStakersForDevice(deviceId) {
  var pool = deviceStakerPools.get(deviceId);
  if (!pool) return new Set();
  return new Set(pool.keys());
}

/**
 * Returns true if account is a staker on the given device.
 */
function isStakerOnDevice(account, deviceId) {
  var pool = deviceStakerPools.get(deviceId);
  if (!pool) return false;
  return pool.has(account);
}

/**
 * Get all devices where account is a staker.
 * Returns array of { device_id, slot, stake_amount, rent_bid, rent_mode }.
 */
function getStakedDevicesForAccount(account) {
  var result = [];
  for (var entry of deviceStakerPools) {
    var pool = entry[1];
    if (pool.has(account)) {
      var rec = pool.get(account);
      result.push({
        device_id: entry[0],
        slot: rec.slot,
        stake_amount: rec.stake_amount,
        rent_bid: rec.rent_bid,
        rent_mode: rec.rent_mode,
      });
    }
  }
  return result;
}

/**
 * Get the auto-stake percentage for a device (0 if not set).
 */
function getDeviceAutoStakePct(deviceId) {
  var rec = deviceAutoStake.get(deviceId);
  return rec ? rec.pct : 0;
}

// Legacy shims — kept so old references don't hard-crash during chain replay
function getDeviceYieldStake(deviceId) {
  // Return first staker as a compatibility shim (or null)
  var pool = getDeviceStakerPool(deviceId);
  if (pool.length === 0) return null;
  var r = pool[0];
  return { staker: r.staker, stake_amount: r.stake_amount, net_stake: r.net_stake !== undefined ? r.net_stake : r.stake_amount, tribute_paid: r.tribute_paid, stake_epoch: r.entry_epoch };
}

function getTopYieldStakerForAccount(account) {
  return getStakedDevicesForAccount(account).map(function (d) {
    return Object.assign({ device_id: d.device_id }, d);
  });
}

function getAllDeviceYieldStakes() {
  var result = [];
  for (var entry of deviceStakerPools) {
    var pool = entry[1];
    for (var rec of pool.values()) {
      result.push(Object.assign({ device_id: entry[0] }, rec));
    }
  }
  return result;
}

// ─────────────────────────────────────────────────────────────────
// Sensor device lifecycle (v3.3)
// ─────────────────────────────────────────────────────────────────

// Epochs of silence before a sensor goes dormant (~48 hours at 30s epochs)
var DORMANT_THRESHOLD = 5760;

// Epochs of silence before a sensor becomes claimable (~70 days at 30s epochs)
var CLAIMABLE_THRESHOLD = 201600;

/**
 * Derive the lifecycle status of a sensor device from its last_reading_epoch.
 * Does NOT mutate the in-memory record — purely computed on request.
 *
 * Status precedence (highest wins):
 *   retired   — explicit flag set by owner
 *   active    — last reading within DORMANT_THRESHOLD epochs
 *   dormant   — silent between DORMANT_THRESHOLD and CLAIMABLE_THRESHOLD epochs
 *   claimable — silent >= CLAIMABLE_THRESHOLD epochs; ownership lock released
 *
 * @param {string} deviceId — sensor_id "<owner>/<device-name>"
 * @param {number} currentEpoch — the current chain epoch
 * @returns {string} "active" | "dormant" | "claimable" | "retired" | "unknown"
 */
function getDeviceStatus(deviceId, currentEpoch) {
  var record = sensors.get(deviceId);
  if (!record) return "unknown";
  if (record.retired === true || record.status === "retired") return "retired";
  if (record.last_reading_epoch === null || record.last_reading_epoch === undefined) {
    // Never reported — treat as dormant from creation epoch 0
    var silence = currentEpoch || 0;
    if (silence >= CLAIMABLE_THRESHOLD) return "claimable";
    if (silence >= DORMANT_THRESHOLD) return "dormant";
    return "active";
  }
  var gap = (currentEpoch || 0) - record.last_reading_epoch;
  if (gap >= CLAIMABLE_THRESHOLD) return "claimable";
  if (gap >= DORMANT_THRESHOLD) return "dormant";
  return "active";
}

// ─────────────────────────────────────────────────────────────────
// IoT sensor + gateway getters (v2.15-beta)
// ─────────────────────────────────────────────────────────────────

function getSensor(sensorId) {
  return sensors.get(sensorId) || null;
}

/**
 * Get all vouches for a sensor_id or account.
 * Returns [{ voucher, target_id, target_type, stake_amount, epoch }]
 */
function getVouchesForTarget(targetId) {
  return (sensorVouches && sensorVouches.get(targetId)) || [];
}

/**
 * Get all targets vouched for by a specific voucher.
 */
function getVouchesByVoucher(voucher) {
  var result = [];
  if (!sensorVouches) return result;
  for (var entry of sensorVouches) {
    for (var v of entry[1]) {
      if (v.voucher === voucher) result.push(v);
    }
  }
  return result;
}

function getAllSensors(filter) {
  var result = [];
  for (var entry of sensors) {
    var s = entry[1];
    if (filter) {
      if (filter.region && s.region !== filter.region) continue;
      if (filter.type && s.type !== filter.type) continue;
      if (filter.owner && s.owner !== filter.owner) continue;
      if (filter.status && s.status !== filter.status) continue;
    }
    result.push(s);
  }
  return result;
}

/**
 * Get sensors that had at least one reading in `epoch`.
 * Used by computeFinalization to find active sensors for the IoT pool.
 */
function getSensorsForEpoch(epoch) {
  var activeSensorIds = new Set();
  var prefix = "|" + (epoch || 0);
  for (var key of sensorReadings.keys()) {
    if (key.slice(key.lastIndexOf("|")) === prefix) {
      activeSensorIds.add(key.slice(0, key.lastIndexOf("|")));
    }
  }
  var result = [];
  for (var sid of activeSensorIds) {
    var s = sensors.get(sid);
    if (s) {
      var readings = sensorReadings.get(sid + "|" + epoch) || [];
      result.push({
        sensor_id: s.sensor_id,
        owner: s.owner,
        readings: readings.length,
        // Approximate uptime: fraction of epochs with readings since creation
        uptime_pct: 1.0, // real uptime computed at query time via sensorRegistry
      });
    }
  }
  return result;
}

/**
 * Get buffered readings for a sensor across an epoch range.
 * Returns array of { epoch, value, metadata, submitted_at } objects.
 * Capped at maxReadings (default 10000) to guard against huge ranges.
 */
function getSensorReadings(sensorId, fromEpoch, toEpoch, maxReadings) {
  var sid = String(sensorId || '');
  var from = Number.isFinite(Number(fromEpoch)) ? Number(fromEpoch) : 0;
  var to = Number.isFinite(Number(toEpoch)) ? Number(toEpoch) : from;
  var cap = Math.min(Math.max(1, Number(maxReadings) || 10000), 50000);
  var result = [];
  for (var ep = from; ep <= to && result.length < cap; ep++) {
    var list = sensorReadings.get(sid + '|' + ep) || [];
    for (var i = 0; i < list.length && result.length < cap; i++) {
      result.push({ epoch: ep, value: list[i].value, metadata: list[i].metadata || {}, submitted_at: list[i].submitted_at });
    }
  }
  return result;
}

function getGateway(gatewayId) {
  return gateways.get(gatewayId) || null;
}

function getAllGateways(filter) {
  var result = [];
  for (var entry of gateways) {
    var g = entry[1];
    if (filter) {
      if (filter.region && g.region !== filter.region) continue;
      if (filter.owner && g.owner !== filter.owner) continue;
      if (filter.status && g.status !== filter.status) continue;
    }
    result.push(g);
  }
  return result;
}

/**
 * Get gateways that sent a GATEWAY_HEARTBEAT in exactly `epoch`.
 * Used by computeFinalization to find active gateways for the IoT pool.
 */
function getGatewaysForEpoch(epoch) {
  var result = [];
  for (var entry of gatewayHeartbeats) {
    var ghStats = entry[1];
    if (ghStats.epochs && ghStats.epochs.has(epoch)) {
      var gwRecord = gateways.get(entry[0]);
      if (gwRecord) {
        var hbCount = ghStats.epochs.size;
        var epochSpan = Math.max(1, (gwRecord.last_heartbeat_epoch || 0) - (gwRecord.created_epoch || 0) + 1);
        result.push({
          gateway_id: gwRecord.gateway_id,
          owner: gwRecord.owner,
          packets_relayed: (gwRecord.last_stats && gwRecord.last_stats.packets_relayed_this_epoch) || 1,
          uptime_pct: Math.min(1.0, hbCount / epochSpan),
        });
      }
    }
  }
  return result;
}

// ─────────────────────────────────────────────────────────────────
// Bridge getters (v2.16-alpha)
// ─────────────────────────────────────────────────────────────────

function getBridgeFunder(funder, chainId) {
  return bridgeFunders.get(funder + "|" + chainId) || null;
}

function getAllBridgeFunders(chainId) {
  var result = [];
  for (var entry of bridgeFunders) {
    if (!chainId || entry[1].chain_id === chainId) result.push(entry[1]);
  }
  return result;
}

// ─────────────────────────────────────────────────────────────────
// Name Auction getters (v3.6)
// ─────────────────────────────────────────────────────────────────

function getAuction(name) {
  return nameAuctions.get(name) || null;
}

function getOpenAuctions() {
  var result = [];
  for (var rec of nameAuctions.values()) {
    if (rec.status === "open") result.push(rec);
  }
  result.sort(function(a, b) { return a.auction_end_epoch - b.auction_end_epoch; });
  return result;
}

function getAuctionHistory() {
  var result = [];
  for (var rec of nameAuctions.values()) {
    if (rec.status === "settled" || rec.status === "cancelled") result.push(rec);
  }
  result.sort(function(a, b) { return (b.settled_epoch || b.cancelled_epoch || 0) - (a.settled_epoch || a.cancelled_epoch || 0); });
  return result;
}

/**
 * Names owned by shindevlin with no open or settled auction.
 * These are available to be auctioned.
 */
function isDelegated(name) {
  var rec = nameDelegate.get(name);
  return rec ? rec.delegated : false;
}

function getAvailableReservedNames() {
  var unavailable = new Set();
  for (var rec of nameAuctions.values()) {
    if (rec.status === "open" || rec.status === "settled") {
      unavailable.add(rec.name);
    }
  }
  var result = [];
  for (var entry of accounts.entries()) {
    var acctName = entry[0];
    if (acctName === "shindevlin" || acctName === "hone_recycle" || acctName === "hone") continue;
    if (!unavailable.has(acctName)) {
      var acct = entry[1];
      var hasNoOwnerKey = !acct.public_keys || !acct.public_keys.active;
      if (hasNoOwnerKey) result.push(acctName);
    }
  }
  result.sort();
  return result;
}

// All reserved names with tier + delegation status for dashboard
function getAllReservedNamesWithTier() {
  var names = getAvailableReservedNames();
  return names.map(function(n) {
    var tier = _nameTier(n);
    return {
      name: n,
      tier: tier,
      min_nodes: AUCTION_TIER_GATES[tier - 1],
      delegated: isDelegated(n),
      auction: nameAuctions.get(n) || null,
    };
  });
}

// ─────────────────────────────────────────────────────────────────
// Slashing getters
// ─────────────────────────────────────────────────────────────────

function getSlashRecords(username) {
  return slashRecords.get(username) || [];
}

function getAllSlashRecords() {
  var result = {};
  for (var entry of slashRecords) {
    result[entry[0]] = entry[1].slice();
  }
  return result;
}

function getInferenceReviewVotes(jobId) {
  return (inferenceReviewVotes.get(jobId) || []).slice();
}

function addSlashRecord(username, record) {
  if (!username || !record) return;
  var records = slashRecords.get(username) || [];
  records.push(record);
  slashRecords.set(username, records);
}

function _debitStake(username, amount) {
  var pool = stakes.get(username);
  if (!pool) return false;
  var amt = _coerceAmount(amount);
  if (amt === null) return false;
  if (toUnits(pool.total_staked) < toUnits(amt)) return false;
  pool.total_staked = _round(pool.total_staked - amt);
  if (pool.total_staked <= 0) stakes.delete(username);
  else stakes.set(username, pool);
  return true;
}

function deductStake(username, amount) {
  return _debitStake(username, amount);
}

function restoreStake(username, amount) {
  if (!username) return;
  var amt = _coerceAmount(amount);
  if (amt === null) return;
  var pool = stakes.get(username) || { total_staked: 0, purpose: null, first_stake_epoch: 0 };
  pool.total_staked = _round(pool.total_staked + amt);
  stakes.set(username, pool);
}

// ─────────────────────────────────────────────────────────────────
// Stateful compute getters (v2.14-beta)
// ─────────────────────────────────────────────────────────────────

/**
 * Get all snapshot records for a service, ordered oldest → newest.
 * Returns an empty array if no snapshots recorded.
 */
function getSnapshots(slug) {
  return (snapshots.get(slug) || []).slice();
}

/**
 * Get the most recent snapshot record for a service, or null.
 */
function getLatestSnapshot(slug) {
  var list = snapshots.get(slug);
  if (!list || list.length === 0) return null;
  return list[list.length - 1];
}

/**
 * Get the stateful service deployment record, or null if not found.
 */
function getStatefulServiceRecord(slug) {
  return services.get(slug) || null;
}

// ─────────────────────────────────────────────────────────────────
// Cross-chain activity getters (v2.17)
// ─────────────────────────────────────────────────────────────────

/**
 * Get all recorded cross-chain activities for a user.
 * Optionally filter by chain name.
 *
 * @param {string} username
 * @param {string} [chain]  - optional filter (base|arbitrum|ethereum|solana|bitcoin)
 * @returns {Array}
 */
function getNodeReputation(account) {
  return _getNodeRep(account);
}

// ── Storage payout hold getters (v3.1.119+) ──────────────────────────────────

function getPendingStorageHolds() {
  var result = [];
  for (var hold of storagePayoutHolds.values()) {
    if (hold.status === "pending") result.push(hold);
  }
  return result;
}

function getStorageHoldsForHost(host) {
  var result = [];
  for (var hold of storagePayoutHolds.values()) {
    if (hold.host === host) result.push(hold);
  }
  return result;
}

function forfeitStorageHoldsForCid(cid) {
  var forfeited = [];
  for (var hold of storagePayoutHolds.values()) {
    if (hold.cid === cid && hold.status === "pending") {
      hold.status = "forfeit_pending";
      storagePayoutHolds.set(hold.hold_id, hold);
      forfeited.push(hold);
    }
  }
  return forfeited;
}

// ── Inference Marketplace getters (v3.1.119+) ────────────────────────────────

function getInferenceJob(jobId) {
  return inferenceJobs.get(jobId) || null;
}

function getOpenInferenceJobs() {
  var result = [];
  for (var job of inferenceJobs.values()) {
    if (job.status === "open" || job.status === "claimed" || job.status === "tool_pending") result.push(job);
  }
  return result;
}

function getInferenceJobsAwaitingFinality() {
  var result = [];
  for (var job of inferenceJobs.values()) {
    if (job.status === "awaiting_challenge" || job.status === "challenged" || job.status === "review_rejected" || job.status === "review_assigned") result.push(job);
  }
  return result;
}

function getJobsAwaitingToolResults(buyer) {
  var result = [];
  for (var job of inferenceJobs.values()) {
    if (job.status === "tool_pending" && job.buyer === buyer) result.push(job);
  }
  return result.sort(function(a, b) { return (b.open_epoch || 0) - (a.open_epoch || 0); });
}

// ── Browser job getters ───────────────────────────────────────────────────────

function getBrowserJob(jobId) {
  return browserJobs.get(jobId) || null;
}

function getOpenBrowserJobs() {
  var result = [];
  for (var j of browserJobs.values()) {
    if (j.status === "open" || j.status === "claimed" || j.status === "action_pending") result.push(j);
  }
  return result;
}

function getBrowserJobsByBuyer(buyer) {
  var result = [];
  for (var j of browserJobs.values()) {
    if (j.buyer === buyer) result.push(j);
  }
  return result.sort(function(a, b) { return (b.open_epoch || 0) - (a.open_epoch || 0); });
}

function getBrowserJobsByMiner(miner) {
  var result = [];
  for (var j of browserJobs.values()) {
    if (j.miner === miner) result.push(j);
  }
  return result.sort(function(a, b) { return (b.open_epoch || 0) - (a.open_epoch || 0); });
}

// ── Fine-tuning getters ───────────────────────────────────────────────────────

function getFinetuneJob(finetuneId) {
  return finetuneJobs.get(finetuneId) || null;
}

function getOpenFinetuneJobs() {
  var result = [];
  for (var ft of finetuneJobs.values()) {
    if (ft.status === "open" || ft.status === "training") result.push(ft);
  }
  return result;
}

function getFinetuneJobsByBuyer(buyer) {
  var result = [];
  for (var ft of finetuneJobs.values()) {
    if (ft.buyer === buyer) result.push(ft);
  }
  return result.sort(function(a, b) { return (b.open_epoch || 0) - (a.open_epoch || 0); });
}

function getFinetuneJobsByMiner(miner) {
  var result = [];
  for (var ft of finetuneJobs.values()) {
    if (ft.miner === miner) result.push(ft);
  }
  return result.sort(function(a, b) { return (b.open_epoch || 0) - (a.open_epoch || 0); });
}

// ── Session getters ───────────────────────────────────────────────────────────

function getSession(sessionId) {
  return sessions.get(sessionId) || null;
}

function getSessionsByBuyer(buyer) {
  var result = [];
  for (var s of sessions.values()) {
    if (s.buyer === buyer) result.push(s);
  }
  return result.sort(function(a, b) { return (b.created_epoch || 0) - (a.created_epoch || 0); });
}

// ── Tool registry getters ─────────────────────────────────────────────────────

function getRegisteredTool(name) {
  return toolRegistry.get(name) || null;
}

function getAllRegisteredTools() {
  return Array.from(toolRegistry.values());
}

function getToolsByOwner(owner) {
  var result = [];
  for (var t of toolRegistry.values()) {
    if (t.owner === owner) result.push(t);
  }
  return result;
}

// ── Miner capabilities (v3.1.125+) ────────────────────────────────────────────

// Capabilities older than this many epochs are considered stale (~100 min at 30s/epoch).
// Miners rebroadcast every 100 epochs, so 200 gives two missed heartbeats before pruning.
var CAPABILITY_STALE_EPOCHS = 200;

function _isCapabilityFresh(caps) {
  if (!caps || caps.updated_epoch == null) return false;
  var latestEpoch = getLatestEpoch();
  if (latestEpoch == null) return true; // chain not yet synced — don't prune
  return (latestEpoch - caps.updated_epoch) < CAPABILITY_STALE_EPOCHS;
}

function getMinerCapabilities(miner) {
  var caps = minerCapabilities.get(miner) || null;
  return (caps && _isCapabilityFresh(caps)) ? caps : null;
}

function getAllMinerCapabilities() {
  return Array.from(minerCapabilities.values()).filter(_isCapabilityFresh);
}

/** Return miners whose advertised capabilities satisfy the job's requirements. */
function getCapableMiners(requirements) {
  var fresh = getAllMinerCapabilities(); // already staleness-filtered
  if (!requirements) return fresh;
  var result = [];
  for (var caps of fresh) {
    if (requirements.vision && !caps.vision) continue;
    if (requirements.audio && !caps.audio) continue;
    if (requirements.code_exec && !caps.code_exec) continue;
    if (requirements.browser && !caps.browser) continue;
    if (requirements.finetune && !caps.finetune) continue;
    if (requirements.tier && !caps.tiers.includes(requirements.tier)) continue;
    result.push(caps);
  }
  return result;
}

/** Aggregate view of what the live network can serve. */
function getNetworkCapabilitySummary() {
  var all = getAllMinerCapabilities(); // already staleness-filtered
  return {
    total_miners: all.length,
    vision: all.filter(function (c) { return c.vision; }).length,
    audio: all.filter(function (c) { return c.audio; }).length,
    code_exec: all.filter(function (c) { return c.code_exec; }).length,
    browser: all.filter(function (c) { return c.browser; }).length,
    finetune: all.filter(function (c) { return c.finetune; }).length,
    reasoning: all.filter(function (c) { return c.tiers && c.tiers.includes("reasoning"); }).length,
    fast: all.filter(function (c) { return c.tiers && c.tiers.includes("fast"); }).length,
    stale_miners: Array.from(minerCapabilities.values()).filter(function (c) { return !_isCapabilityFresh(c); }).length,
  };
}

// ── Inference memory getters (v3.1.131+) ───────────────────────────────────

function getUserMemories(account) {
  var map = userMemories.get(account);
  if (!map) return [];
  return Array.from(map.values()).sort(function(a, b) { return (b.created_epoch || 0) - (a.created_epoch || 0); });
}

function getMemory(account, memoryId) {
  var map = userMemories.get(account);
  return (map && map.get(memoryId)) || null;
}

function getUserProjects(account) {
  var map = memoryProjects.get(account);
  if (!map) return [];
  return Array.from(map.values()).sort(function(a, b) { return (a.name || "").localeCompare(b.name || ""); });
}

function getMemoryProject(account, projectId) {
  var map = memoryProjects.get(account);
  return (map && map.get(projectId)) || null;
}

/**
 * Obsidian-style graph: returns nodes and edges for an account's memories.
 * Nodes: memories + projects. Edges: memory→project, memory→memory (linked_memory_ids).
 */
function getMemoryGraph(account) {
  var memories = getUserMemories(account);
  var projects = getUserProjects(account);
  var nodes = [];
  var edges = [];

  for (var p of projects) {
    nodes.push({ id: "project:" + p.project_id, type: "project", label: p.name, data: p });
  }
  for (var m of memories) {
    nodes.push({ id: "memory:" + m.memory_id, type: "memory", label: m.tags.slice(0, 3).join(", ") || m.memory_id, data: m });
    if (m.project_id) {
      edges.push({ source: "memory:" + m.memory_id, target: "project:" + m.project_id, type: "belongs_to" });
    }
    for (var linkedId of (m.linked_memory_ids || [])) {
      // Only emit each edge once (lower→higher id ordering)
      if (m.memory_id < linkedId) {
        edges.push({ source: "memory:" + m.memory_id, target: "memory:" + linkedId, type: "related" });
      }
    }
  }
  return { nodes, edges };
}

function getInferenceJobsByBuyer(buyer) {
  var result = [];
  for (var job of inferenceJobs.values()) {
    if (job.buyer === buyer) result.push(job);
  }
  return result.sort(function(a, b) { return (b.open_epoch || 0) - (a.open_epoch || 0); });
}

function getInferenceJobsByMiner(miner) {
  var result = [];
  for (var job of inferenceJobs.values()) {
    if (job.miner === miner) result.push(job);
  }
  return result.sort(function(a, b) { return (b.open_epoch || 0) - (a.open_epoch || 0); });
}

function getCrossChainCredits(account) {
  var result = {};
  for (var i = 0; i < CROSS_CHAIN_SUPPORTED.length; i++) {
    result[CROSS_CHAIN_SUPPORTED[i]] = crossChainCredits.get(account + '|' + CROSS_CHAIN_SUPPORTED[i]) || 0;
  }
  return result;
}

function getAllCrossChainCredits(account) {
  var result = {};
  for (var entry of crossChainCredits.entries()) {
    if (entry[0].startsWith(account + '|')) {
      var chain = entry[0].slice(account.length + 1);
      result[chain] = entry[1];
    }
  }
  return result;
}

function getCrossChainActivity(username, chain) {
  var list = crossChainActivity.get(username) || [];
  if (chain) {
    return list.filter(function (a) { return a.chain === chain; });
  }
  return list.slice();
}

/**
 * Get the most recent N cross-chain activities across all users.
 * Used by the explorer to show a live feed.
 */
function getRecentCrossChainActivity(n) {
  var result = [];
  for (var entry of crossChainActivity) {
    var username = entry[0];
    var list = entry[1];
    for (var i = 0; i < list.length; i++) {
      result.push(Object.assign({ username: username }, list[i]));
    }
  }
  result.sort(function (a, b) { return (b.timestamp || 0) - (a.timestamp || 0); });
  return result.slice(0, n || 50);
}

// ─────────────────────────────────────────────────────────────────
// Nested wallet getters (v3.3)
// ─────────────────────────────────────────────────────────────────

/**
 * Get all child account names of a parent.
 * Returns an array of fully-qualified account names (e.g. ["hone/treasury"]).
 */
function getChildren(parent) {
  var s = childrenMap.get(parent);
  if (!s) return [];
  return Array.from(s);
}

/**
 * Get the parent account name for a child, or null if it has none.
 */
function getParent(account) {
  return parentMap.get(account) || null;
}

/**
 * Returns true if signingAccount is authorized to act on behalf of account.
 * Rules:
 *   1. Identity: signingAccount === account.
 *   2. Parent: signingAccount is the direct parent of account (via parentMap).
 */
function isAuthorizedBy(account, signingAccount) {
  if (!account || !signingAccount) return false;
  if (signingAccount === account) return true;
  return parentMap.get(account) === signingAccount;
}

function getAliasTarget(alias) {
  if (!alias) return null;
  return aliasMap.get(alias) || null;
}

function resolveAccountName(name) {
  if (!name) return null;
  if (accounts.has(name)) return name;
  return aliasMap.get(name) || null;
}

function getAliases(account) {
  var s = accountAliases.get(account);
  if (!s) return [];
  return Array.from(s);
}

// ─────────────────────────────────────────────────────────────────
// hone_recycle Phase 2 endowment getters/setters (v3.3)
// ─────────────────────────────────────────────────────────────────

/**
 * Total HONE distributed to non-recycle accounts via MINING_REWARD.
 * When this reaches 42,000,000 Phase 2 activates.
 */
function getTotalSupplyDistributed() {
  return totalSupplyDistributed;
}

/**
 * Returns true when total distributed supply >= 42,000,000 HONE.
 * This is the canonical Phase 2 activation check.
 */
function isPhase2() {
  return totalSupplyDistributed >= 42000000;
}

/**
 * Returns true once Phase 2 has been formally activated in the reward engine.
 */
function isPhase2Activated() {
  return phase2Activated;
}

/**
 * Store the recycle rate r, mark Phase 2 active, and record the epoch.
 * Called once by rewardEngine on the first Phase 2 epoch.
 */
function setRecycleRate(r, epoch) {
  if (typeof r !== "number" || r <= 0) return;
  recycleRate = r;
  phase2Activated = true;
  phase2ActivationEpoch = epoch || 0;
}

/**
 * Get the stored recycle rate (null until Phase 2 activates).
 */
function getRecycleRate() {
  return recycleRate;
}

/**
 * Get the epoch at which Phase 2 was activated (null until then).
 */
function getPhase2ActivationEpoch() {
  return phase2ActivationEpoch;
}

// ─────────────────────────────────────────────────────────────────
// Bulk / introspection
// ─────────────────────────────────────────────────────────────────

function snapshot() {
  var balObj = {};
  for (var b of balances) balObj[b[0]] = b[1];
  return {
    chainHeight: chainHeight,
    accounts: getAllAccounts(),
    balances: balObj,
    tokens: getAllTokens(),
    stakes: getAllStakePools(),
    escrows: Array.from(escrows.entries()),
    projects: getAllProjects(),
    stores: Array.from(stores.entries()),
    products: Array.from(products.entries()),
    orders: Array.from(orders.entries()),
    reputation: Array.from(reputation.entries()),
    reputation_votes: Array.from(reputationVotes.entries()),
    slash_records: Array.from(slashRecords.entries()),
    inference_review_votes: Array.from(inferenceReviewVotes.entries()),
    blobs: Array.from(blobs.entries()),
  };
}

function stats() {
  return {
    chainHeight: chainHeight,
    accounts: accounts.size,
    balance_entries: balances.size,
    tokens: tokens.size,
    nfts: nfts.size,
    stakes: stakes.size,
    delegations: delegations.size,
    escrows: escrows.size,
    projects: projects.size,
    stores: stores.size,
    products: products.size,
    orders: orders.size,
    reputation: reputation.size,
    reputation_votes: reputationVotes.size,
    slash_records: slashRecords.size,
    inference_review_votes: inferenceReviewVotes.size,
    blobs: blobs.size,
    storage_heartbeats: storageHeartbeats.size,
    blob_challenges: blobChallenges.size,
    epochs: epochs.size,
    mining_proof_epochs: miningProofsByEpoch.size,
    seen_entries: seenEntries.size,
  };
}

// ─────────────────────────────────────────────────────────────────
// Finality snapshot integration
// ─────────────────────────────────────────────────────────────────

/**
 * Hydrate stateStore from a finality snapshot. Called by replay.js when
 * starting from a checkpoint. The snapshot may contain extended_state
 * (new format) or just accounts (old format — legacy SMT-only snapshot).
 */
function hydrateFromFinality(snapshot) {
  if (!snapshot) return;

  // Legacy accounts (SMT state): balance/staked/delegated/nonce
  if (snapshot.accounts && typeof snapshot.accounts === "object") {
    var usernames = Object.keys(snapshot.accounts);
    for (var i = 0; i < usernames.length; i++) {
      var u = usernames[i];
      var s = snapshot.accounts[u];
      _ensureAccount(u);
      if (typeof s.balance === "number") {
        _assertNonNegativeAmount(s.balance, "finality snapshot account " + u + " balance");
        balances.set(_balanceKey(u, "HONE"), toUnits(s.balance));
      }
      if (typeof s.staked === "number" && s.staked > 0) {
        _assertNonNegativeAmount(s.staked, "finality snapshot account " + u + " staked");
        stakes.set(u, { total_staked: s.staked, purpose: null, first_stake_epoch: 0 });
      }
    }
  }

  // Extended state (new format): tokens, nfts, escrows, projects
  if (snapshot.extended_state) {
    var ext = snapshot.extended_state;
    if (ext.tokens) {
      Object.keys(ext.tokens).forEach(function (sym) {
        tokens.set(sym, ext.tokens[sym]);
      });
    }
    if (ext.nfts) {
      Object.keys(ext.nfts).forEach(function (k) {
        nfts.set(k, ext.nfts[k]);
      });
    }
    if (ext.escrows) {
      Object.keys(ext.escrows).forEach(function (k) {
        escrows.set(k, ext.escrows[k]);
      });
    }
    if (ext.projects) {
      Object.keys(ext.projects).forEach(function (name) {
        projects.set(name, ext.projects[name]);
      });
    }
    if (ext.stores) {
      Object.keys(ext.stores).forEach(function (k) {
        stores.set(k, ext.stores[k]);
      });
    }
    if (ext.products) {
      Object.keys(ext.products).forEach(function (k) {
        products.set(k, ext.products[k]);
      });
    }
    if (ext.orders) {
      Object.keys(ext.orders).forEach(function (k) {
        orders.set(k, ext.orders[k]);
      });
    }
    if (ext.reputation) {
      Object.keys(ext.reputation).forEach(function (k) {
        reputation.set(k, ext.reputation[k]);
      });
    }
    if (ext.reputation_votes) {
      Object.keys(ext.reputation_votes).forEach(function (k) {
        reputationVotes.set(k, ext.reputation_votes[k]);
      });
    }
    if (ext.blobs) {
      Object.keys(ext.blobs).forEach(function (cid) {
        blobs.set(cid, ext.blobs[cid]);
      });
    }
    if (ext.delegations) {
      Object.keys(ext.delegations).forEach(function (k) {
        delegations.set(k, ext.delegations[k]);
      });
    }
    if (ext.extra_balances) {
      // Non-HONE token balances: { "user|TOKEN": amount }
      Object.keys(ext.extra_balances).forEach(function (k) {
        _assertNonNegativeAmount(ext.extra_balances[k], "finality snapshot extra balance " + k);
        balances.set(k, ext.extra_balances[k]);
      });
    }
  }

  if (typeof snapshot.finality_epoch === "number") {
    setChainHeight(snapshot.finality_epoch);
  }
  assertBalanceIntegrity("hydrateFromFinality");
}

// ─────────────────────────────────────────────────────────────────
// Reset (for tests and fresh replay)
// ─────────────────────────────────────────────────────────────────

function resetAll() {
  balances.clear();
  accounts.clear();
  tokens.clear();
  nfts.clear();
  stakes.clear();
  delegations.clear();
  delegatedReceived.clear();
  escrows.clear();
  epochs.clear();
  projects.clear();
  stores.clear();
  products.clear();
  orders.clear();
  reputation.clear();
  reputationVotes.clear();
  blobs.clear();
  storageHeartbeats.clear();
  storageDeliveryByEpoch.clear();
  blobChallenges.clear();
  blobChallengeStats.clear();
  miningProofsByEpoch.clear();
  computeProofsByEpoch.clear();
  finalizedConsensusByEpoch.clear();
  anchorLog.splice(0);
  slashRecords.clear();
  inferenceReviewVotes.clear();
  services.clear();
  snapshots.clear();
  sensors.clear();
  sensorReadings.clear();
  witnessMap.clear();
  gateways.clear();
  gatewayHeartbeats.clear();
  bridgeWraps.clear();
  bridgeUnwraps.clear();
  bridgeFunders.clear();
  crossChainActivity.clear();
  projectRevenueSplits.clear();
  earnings.clear();
  seenEntries.clear();
  deviceKeys.clear();
  iotDeviceKeys.clear();
  deviceStakerPools.clear();
  deviceAutoStake.clear();
  deviceStakerIndex.clear();
  nameAuctions.clear();
  nameDelegate.clear();
  childrenMap.clear();
  parentMap.clear();
  aliasMap.clear();
  accountAliases.clear();
  communityModels.clear();
  storagePayoutHolds.clear();
  storedFiles.clear();
  shardPtrs.clear();
  nodeReputation.clear();
  crossChainCredits.clear();
  inferenceJobs.clear();
  sessions.clear();
  finetuneJobs.clear();
  browserJobs.clear();
  toolRegistry.clear();
  minerCapabilities.clear();
  userMemories.clear();
  memoryProjects.clear();
  chainHeight = -1;
  currentBlockCap = 1 * 1024 * 1024;
  recycleRate = null;
  phase2Activated = false;
  phase2ActivationEpoch = null;
  totalSupplyDistributed = 0;
  // Ensure hone_recycle always exists as keyless protocol contract
  _ensureAccount("hone_recycle", { epoch: 0, public_keys: {}, chain_addresses: {} });
  _ensureAccount("hone", { epoch: 0, public_keys: {}, chain_addresses: {} });
}

function getExchangeQueue() { return exchangeQueue; }
function getExchangeStats() { return exchangeStats; }
function getExchangeQueueLength() { return exchangeQueue.length; }
function availableWhoneInQueue() {
  let total = 0;
  for (let d of exchangeQueue) total = _round(total + d.amount);
  return total;
}

module.exports = {

  // Mutators
  applyEntry: applyEntry,
  applyEntries: applyEntries,
  setEpoch: setEpoch,
  setMiningProofs: setMiningProofs,
  addMiningProof: addMiningProof,
  setComputeProofs: setComputeProofs,
  addComputeProof: addComputeProof,
  setChainHeight: setChainHeight,
  getCurrentBlockCap: getCurrentBlockCap,
  setCurrentBlockCap: setCurrentBlockCap,
  hydrateFromFinality: hydrateFromFinality,
  resetAll: resetAll,
  // Account
  getBalance: getBalance,
  getTokenBalances: getTokenBalances,
  getAccount: getAccount,
  hasAccount: hasAccount,
  getAllAccounts: getAllAccounts,
  getAccountCount: getAccountCount,
  bootstrapAccountKey: bootstrapAccountKey,
  resolveAccountName: resolveAccountName,
  getAliasTarget: getAliasTarget,
  getAliases: getAliases,
  getChildren: getChildren,
  getParent: getParent,
  // Device key delegation
  getDeviceOwner: getDeviceOwner,
  getAccountDevices: getAccountDevices,
  isDeviceAuthorized: isDeviceAuthorized,
  // IoT device key registry (v3.2)
  getDeviceKey: getDeviceKey,
  getDevicesByOwner: getDevicesByOwner,
  // Witness map for relay-scaled rewards (v3.2)
  getWitnesses: getWitnesses,
  // Device Stake & Rent (v3.5)
  getDeviceStakerPool: getDeviceStakerPool,
  getAllStakersForDevice: getAllStakersForDevice,
  isStakerOnDevice: isStakerOnDevice,
  getStakedDevicesForAccount: getStakedDevicesForAccount,
  getDeviceAutoStakePct: getDeviceAutoStakePct,
  SLOT_MULTIPLIERS: SLOT_MULTIPLIERS,
  // Legacy shims (v3.4 compat)
  getDeviceYieldStake: getDeviceYieldStake,
  getTopYieldStakerForAccount: getTopYieldStakerForAccount,
  getAllDeviceYieldStakes: getAllDeviceYieldStakes,
  // Token/NFT
  getToken: getToken,
  getAllTokens: getAllTokens,
  getExchangeQueue: getExchangeQueue,
  getExchangeStats: getExchangeStats,
  getExchangeQueueLength: getExchangeQueueLength,
  availableWhoneInQueue: availableWhoneInQueue,
  getNFT: getNFT,

  getNFTsByOwner: getNFTsByOwner,
  getNFTsByCollection: getNFTsByCollection,
  getAllNFTs: getAllNFTs,
  // Staking/delegation
  getStake: getStake,
  getStakePool: getStakePool,
  getAllStakePools: getAllStakePools,
  getSponsoredStake: function(beneficiary) { return sponsoredStakes.get(beneficiary) || null; },
  getNetworkPolicy: function() { return Object.assign({}, networkPolicy); },
  getDelegation: getDelegation,
  getDelegationsByDelegator: getDelegationsByDelegator,
  getDelegationsByRecipient: getDelegationsByRecipient,
  getTotalStaked: getTotalStaked,
  getDelegatedBalance: getDelegatedBalance,
  // Escrow
  getEscrow: getEscrow,
  getEscrowsByPayer: getEscrowsByPayer,
  getActiveEscrows: getActiveEscrows,
  // Epoch / proofs
  getEpoch: getEpoch,
  getEpochVrfSeed: getEpochVrfSeed,
  getLatestEpoch: getLatestEpoch,
  getChainHeight: getChainHeight,
  getRecentEpochs: getRecentEpochs,
  getMiningProofs: getMiningProofs,
  getFinalizedConsensus: getFinalizedConsensus,
  getComputeProofs: getComputeProofs,
  getRecentComputeProofs: getRecentComputeProofs,
  getMinerCount: getMinerCount,
  // Projects
  getProject: getProject,
  getAllProjects: getAllProjects,
  getProjectRevenueSplit: getProjectRevenueSplit,
  // Earnings breakdown
  getEarnings: getEarnings,
  // Commerce (v2.10)
  getStore: getStore,
  getAllStores: getAllStores,
  getStoreActiveProductCount: getStoreActiveProductCount,
  getProduct: getProduct,
  getAllProducts: getAllProducts,
  getProductsBySeller: getProductsBySeller,
  getOrder: getOrder,
  getOrdersByBuyer: getOrdersByBuyer,
  getOrdersBySeller: getOrdersBySeller,
  getReputation: getReputation,
  getReputationVote: getReputationVote,
  // Private encrypted file storage (v3.1.110+)
  getStoredFile: getStoredFile,
  getStoredFilesByOwner: getStoredFilesByOwner,
  // Quantum-resistant split storage (v3.1.111+)
  getShard: getShard,
  getShardsByOwner: getShardsByOwner,
  // Community model registry (v3.1.97+)
  getCommunityModel: getCommunityModel,
  getAllCommunityModels: getAllCommunityModels,
  getModelRoyalty: getModelRoyalty,
  // HONE-FS blobs (v2.11+)
  getBlobCommit: getBlobCommit,
  getAllBlobCommits: getAllBlobCommits,
  getBlobCommitsByHost: getBlobCommitsByHost,
  getBlobCommitsByUploader: getBlobCommitsByUploader,
  // HONE-FS storage heartbeats + uptime (v2.11.2+)
  getStorageHeartbeat: getStorageHeartbeat,
  getAllStorageHosts: getAllStorageHosts,
  getStorageUptimeFactor: getStorageUptimeFactor,
  getActiveStorageHosts: getActiveStorageHosts,
  getStorageHostsForEpoch: getStorageHostsForEpoch,
  recordStorageDelivery: recordStorageDelivery,
  getStorageWorkByEpoch: getStorageWorkByEpoch,
  // HONE-FS challenge-response (v2.11.2+, pay-for-delivery not slashing)
  getBlobChallenge: getBlobChallenge,
  getBlobChallengeStats: getBlobChallengeStats,
  getChallengeSuccessRate: getChallengeSuccessRate,
  getPendingChallengesForHost: getPendingChallengesForHost,
  getChallengesForCid: getChallengesForCid,
  // Slashing
  getSlashRecords: getSlashRecords,
  // Stateful compute (v2.14-beta)
  getSnapshots: getSnapshots,
  getLatestSnapshot: getLatestSnapshot,
  getStatefulServiceRecord: getStatefulServiceRecord,
  // Sensor device lifecycle (v3.3)
  getDeviceStatus: getDeviceStatus,
  DORMANT_THRESHOLD: DORMANT_THRESHOLD,
  CLAIMABLE_THRESHOLD: CLAIMABLE_THRESHOLD,
  // IoT sensor + gateway (v2.15-beta)
  getSensor: getSensor,
  getAllSensors: getAllSensors,
  getSensorsForEpoch: getSensorsForEpoch,
  getSensorReadings: getSensorReadings,
  getVouchesForTarget: getVouchesForTarget,
  getVouchesByVoucher: getVouchesByVoucher,
  getGateway: getGateway,
  getAllGateways: getAllGateways,
  getGatewaysForEpoch: getGatewaysForEpoch,
  // Bridge (v2.16-alpha)
  getBridgeFunder: getBridgeFunder,
  getAllBridgeFunders: getAllBridgeFunders,
  // Name Auctions (v3.6)
  getAuction: getAuction,
  getOpenAuctions: getOpenAuctions,
  getAuctionHistory: getAuctionHistory,
  getAvailableReservedNames: getAvailableReservedNames,
  getAllReservedNamesWithTier: getAllReservedNamesWithTier,
  isDelegated: isDelegated,
  AUCTION_TIER_GATES: AUCTION_TIER_GATES,
  // Cross-chain monitor (v2.17)
  getCrossChainActivity: getCrossChainActivity,
  getRecentCrossChainActivity: getRecentCrossChainActivity,
  // Four-tier finality anchor log (v3.2)
  getRecentAnchors: getRecentAnchors,
  // Integer arithmetic
  toUnits: toUnits,
  fromUnits: fromUnits,
  getBalanceUnits: getBalanceUnits,
  assertBalanceIntegrity: assertBalanceIntegrity,
  // Introspection
  snapshot: snapshot,
  stats: stats,
  // Nested wallets (v3.3)
  getChildren: getChildren,
  getParent: getParent,
  isAuthorizedBy: isAuthorizedBy,
  // hone_recycle Phase 2 endowment (v3.3)
  getTotalSupplyDistributed: getTotalSupplyDistributed,
  isPhase2: isPhase2,
  isPhase2Activated: isPhase2Activated,
  setRecycleRate: setRecycleRate,
  getRecycleRate: getRecycleRate,
  getPhase2ActivationEpoch: getPhase2ActivationEpoch,
  getCrossChainCredits,
  getAllCrossChainCredits,
  CROSS_CHAIN_SUPPORTED,
  getNodeReputation,
  deductStake,
  restoreStake,
  addSlashRecord,
  getSlashRecords,
  getAllSlashRecords,
  getInferenceReviewVotes,
  // Inference Marketplace (v3.1.119+)
  getInferenceJob,
  getOpenInferenceJobs,
  getInferenceJobsAwaitingFinality,
  getInferenceJobsByBuyer,
  getInferenceJobsByMiner,
  getJobsAwaitingToolResults,
  // Browser / computer-use jobs (v3.1.121+)
  getBrowserJob,
  getOpenBrowserJobs,
  getBrowserJobsByBuyer,
  getBrowserJobsByMiner,
  // Fine-tuning (v3.1.121+)
  getFinetuneJob,
  getOpenFinetuneJobs,
  getFinetuneJobsByBuyer,
  getFinetuneJobsByMiner,
  // Sessions (v3.1.121+)
  getSession,
  getSessionsByBuyer,
  // Tool registry (v3.1.121+)
  getRegisteredTool,
  getAllRegisteredTools,
  getToolsByOwner,
  // Miner capabilities (v3.1.125+)
  getMinerCapabilities,
  getAllMinerCapabilities,
  getCapableMiners,
  getNetworkCapabilitySummary,
  CAPABILITY_STALE_EPOCHS,
  // Inference memory (v3.1.131+)
  getUserMemories,
  getMemory,
  getUserProjects,
  getMemoryProject,
  getMemoryGraph,
  // Storage settlement lag (v3.1.119+)
  getPendingStorageHolds,
  getStorageHoldsForHost,
  forfeitStorageHoldsForCid,
};
