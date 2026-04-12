"use strict";

/**
 * BTCPC State Store — in-memory chain state cache.
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

// Token metadata: symbol → { name, symbol, supply, decimals, type, creator, created_epoch }
var tokens = new Map();

// NFTs: "collection|tokenId" → { owner, metadata, minted_epoch, transferable, soulbound, time_locked, unlock_epoch, evolving, metrics }
var nfts = new Map();

// Staking pool state: username → { total_staked, purpose, first_stake_epoch }
var stakes = new Map();

// Delegations: "from|to" → { amount, purpose, epoch }
var delegations = new Map();

// Escrows: requestId → { payer, amount, status, locked_epoch, released_to }
var escrows = new Map();

// Epoch metadata: epochNumber → { started_at, ended_at, block_reward, total_work, consensus_hash, status, rewards_distributed }
var epochs = new Map();

// Projects (PROJECT_CREATE ledger entries): name → { owner, repo_url, wallet_address, created_epoch }
// Note: API keys are NOT here — they live in ~/.btcpc/secrets.json
var projects = new Map();

// ─────────────────────────────────────────────────────────────────
// Commerce state (v2.10)
// ─────────────────────────────────────────────────────────────────
// Stores: seller → { name, banner_cid, description_cid, categories, stake_amount, stake_paid_usd, capacity, rep_score, rep_votes_up, rep_votes_down, total_sales, total_fulfilled, total_disputed, created_epoch, status }
// "capacity" is how many products the store can hold, bought via bonding curve.
var stores = new Map();

// Products: product_id → { store, seller, title, description_snippet, content_cid, category, price, token, stock, rep_score, rep_votes_up, rep_votes_down, total_sold, created_epoch, updated_epoch, status }
var products = new Map();

// Orders: order_id → { buyer, seller, product_id, quantity, unit_price, total, token, escrow_id, status, placed_epoch, fulfilled_epoch, delivered_epoch }
var orders = new Map();

// Reputation aggregates: "store|<id>" or "miner|<id>" or "product|<id>" → { score, votes_up, votes_down, completed, disputed, last_updated_epoch }
var reputation = new Map();

// Reputation votes (dedupe + recall): "voter|target_type|target_id" → { vote: +1|-1, weight, epoch, memo }
var reputationVotes = new Map();

// BTCPC-FS blobs (v2.11+): cid → { size, uploader, hosts[], committed_epoch, expires_epoch, payment_btcpc }
// Tracks which CIDs have been committed on chain. The actual bytes live in
// src/services/blobStore.js — this Map is the chain-level metadata only.
var blobs = new Map();

// BTCPC-FS storage heartbeats (v2.11.2+): host → {
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

// Keep at most this many recent heartbeats per host to bound memory.
// 1000 epochs ≈ 3.5 days at 5 min epochs — enough history for uptime
// calculation windows without unbounded growth.
var STORAGE_HEARTBEAT_RETENTION = 1000;

// BTCPC-FS blob challenges (v2.11.2+): challenge_id → {
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

// Slashing records: username → [ { epoch, offenseType, tier, amount, evidence } ]
var slashRecords = new Map();

// Chain height: highest known finalized epoch
var chainHeight = -1;

// Current dynamic block cap (v3.0). Updated each epoch via setCurrentBlockCap.
// Initialized to DEFAULT_BLOCK_CAP (1 MB). Stored here so all nodes agree.
var currentBlockCap = 1 * 1024 * 1024; // 1 MB default

// Dedupe: entries we've already applied (by hash of canonical fields)
var seenEntries = new Set();
var SEEN_ENTRIES_CAP = 100000;

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

function _balanceKey(username, token) {
  return username + "|" + (token || "BTCPC");
}

function _round(n) {
  return parseFloat(Number(n).toFixed(10));
}

function _credit(username, token, amount) {
  if (!username || !amount) return;
  var key = _balanceKey(username, token);
  balances.set(key, _round((balances.get(key) || 0) + amount));
}

function _debit(username, token, amount) {
  if (!username || !amount) return;
  var key = _balanceKey(username, token);
  var current = balances.get(key) || 0;
  // System/issuance accounts may go negative; all others are floor-checked.
  if (!_isSystemAccount(username) && current < amount) {
    return; // insufficient balance — reject silently (Vuln 6 fix)
  }
  balances.set(key, _round(current - amount));
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
  return username === "btcpc_staking_pool" ||
         username === "btcpc_escrow" ||
         username === "btcpc_genesis" ||
         username === "btcpc_mint" ||
         username === "btcpc_recycle" ||
         username === "btcpc_treasury" ||
         username.startsWith("project:") ||
         username.startsWith("escrow:");
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
    // Heartbeats are per-host-per-epoch; dedupe on that key.
    domainId = "sh:" + (entry.from || "") + ":" + (entry.epoch || 0);
  } else if (entry.type === "SENSOR_REGISTER") {
    domainId = "sr:" + (entry.sensor_data && entry.sensor_data.sensor_id || "");
  } else if (entry.type === "SENSOR_READING") {
    domainId = "srd:" + (entry.sensor_data && entry.sensor_data.sensor_id || "") + ":" + (entry.epoch || 0) + ":" + (entry.sensor_data && entry.sensor_data.value !== undefined ? entry.sensor_data.value : "") + ":" + (entry.timestamp || 0);
  } else if (entry.type === "SENSOR_DATA_COMMIT") {
    domainId = "sdc:" + (entry.sensor_data && entry.sensor_data.cid || "");
  } else if (entry.type === "GATEWAY_REGISTER") {
    domainId = "gr:" + (entry.gateway_data && entry.gateway_data.gateway_id || "");
  } else if (entry.type === "GATEWAY_HEARTBEAT") {
    domainId = "gh:" + (entry.gateway_data && entry.gateway_data.gateway_id || "") + ":" + (entry.epoch || 0);
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

  var from = entry.from;
  var to = entry.to;
  var amount = entry.amount || 0;
  var token = entry.token || "BTCPC";

  switch (entry.type) {
    case "ACCOUNT_CREATE":
      _ensureAccount(to, {
        epoch: entry.epoch,
        public_keys: entry.account_data && entry.account_data.public_keys,
        chain_addresses: entry.account_data && entry.account_data.chain_addresses,
      });
      break;

    case "TRANSFER":
      _debit(from, token, amount);
      _credit(to, token, amount);
      break;

    case "MINING_REWARD":
    case "FAUCET":
      _credit(to, token, amount);
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
      _debit(from, "BTCPC", amount);
      if (from) {
        var s = stakes.get(from) || { total_staked: 0, purpose: entry.memo, first_stake_epoch: entry.epoch };
        s.total_staked = _round(s.total_staked + amount);
        stakes.set(from, s);
      }
      break;

    case "UNSTAKE":
      _credit(to, "BTCPC", amount);
      if (to) {
        var us = stakes.get(to);
        if (us) {
          us.total_staked = _round(us.total_staked - amount);
          if (us.total_staked <= 0) stakes.delete(to);
          else stakes.set(to, us);
        }
      }
      break;

    case "DELEGATE":
      _debit(from, "BTCPC", amount);
      if (from && to) {
        var dkey = from + "|" + to;
        var d = delegations.get(dkey) || { amount: 0, purpose: entry.memo, epoch: entry.epoch };
        d.amount = _round(d.amount + amount);
        delegations.set(dkey, d);
      }
      break;

    case "UNDELEGATE":
      _credit(to, "BTCPC", amount);
      if (entry.delegation_data) {
        var dkey2 = entry.delegation_data.delegator + "|" + entry.delegation_data.miner;
        var d2 = delegations.get(dkey2);
        if (d2) {
          d2.amount = _round(d2.amount - amount);
          if (d2.amount <= 0) delegations.delete(dkey2);
          else delegations.set(dkey2, d2);
        }
      }
      break;

    case "ESCROW_LOCK":
      _debit(from, "BTCPC", amount);
      if (entry.memo) {
        // memo is usually "escrow:request_id"
        var rid = entry.memo.startsWith("escrow:") ? entry.memo.slice(7) : entry.memo;
        escrows.set(rid, {
          payer: from,
          amount: amount,
          status: "locked",
          locked_epoch: entry.epoch,
          released_to: null,
        });
      }
      break;

    case "ESCROW_RELEASE":
      _credit(to, "BTCPC", amount);
      if (entry.memo) {
        var rid2 = entry.memo.startsWith("escrow:") ? entry.memo.slice(7) : entry.memo;
        var e2 = escrows.get(rid2);
        if (e2) {
          e2.status = "released";
          e2.released_to = to;
          escrows.set(rid2, e2);
        }
      }
      break;

    case "ESCROW_REFUND":
      _credit(to, "BTCPC", amount);
      if (entry.memo) {
        var rid3 = entry.memo.startsWith("escrow:") ? entry.memo.slice(7) : entry.memo;
        var e3 = escrows.get(rid3);
        if (e3) {
          e3.status = "refunded";
          escrows.set(rid3, e3);
        }
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
          token: pd.token || "BTCPC",
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
          token: od.token || "BTCPC",
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

    // ── BTCPC-FS blob commits (v2.11+) ─────────────────────────────
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
          payment_btcpc: 0,
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
        existingBlob.payment_btcpc = _round(existingBlob.payment_btcpc + (bd.payment_btcpc || 0));
        if (bd.size && !existingBlob.size) existingBlob.size = bd.size;
        blobs.set(bd.cid, existingBlob);
      }
      break;

    // ── BTCPC-FS storage heartbeats (v2.11.2+) ─────────────────────
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

    // ── BTCPC-FS challenge-response (v2.11.2+) ─────────────────────
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
          } else {
            challenge.status = "failed_mismatch";
            _bumpChallengeStat(from, "failed");
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
        } else {
          c2.status = "failed_mismatch";
          _bumpChallengeStat(c2.host, "failed");
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
        blobChallenges.set(td.challenge_id, c3);
      }
      break;

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
          status: existingSensor.status === "retired" ? "retired" : "active",
          created_epoch: existingSensor.created_epoch || entry.epoch || 0,
          last_updated_epoch: entry.epoch || 0,
          last_reading_epoch: existingSensor.last_reading_epoch || null,
          total_readings: existingSensor.total_readings || 0,
        }));
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
        });
        sensorReadings.set(srdKey, srdList);
        // Update sensor's last_reading_epoch
        var srdSensor = sensors.get(srdData.sensor_id);
        if (srdSensor) {
          srdSensor.last_reading_epoch = entry.epoch || 0;
          srdSensor.total_readings = (srdSensor.total_readings || 0) + 1;
          sensors.set(srdData.sensor_id, srdSensor);
        }
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
    // Pay-for-delivery: no slashing, fees always to btcpc_recycle.
    // ─────────────────────────────────────────────────────────────────

    case "BRIDGE_WRAP":
      // Debit user BTCPC; record wrap event.
      if (from && amount > 0) {
        _debit(from, "BTCPC", amount);
        // Fee portion already included in amount, route fee to recycle
        if (entry.bridge_data && entry.bridge_data.fee > 0) {
          _credit("btcpc_recycle", "BTCPC", entry.bridge_data.fee);
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
      // Credit user BTCPC; record unwrap event.
      if (to && amount > 0) {
        _credit(to, "BTCPC", amount);
        if (entry.bridge_data && entry.bridge_data.fee > 0) {
          _credit("btcpc_recycle", "BTCPC", entry.bridge_data.fee);
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
        _debit(from, "BTCPC", amount);
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

    default:
      // Unknown type — safe to skip. New types will be added here.
      break;
  }
}

function applyEntries(entries) {
  if (!Array.isArray(entries)) return;
  for (var i = 0; i < entries.length; i++) applyEntry(entries[i]);
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
  return balances.get(_balanceKey(username, token)) || 0;
}

function getTokenBalances(username) {
  var result = {};
  if (!username) return result;
  var prefix = username + "|";
  for (var entry of balances) {
    if (entry[0].indexOf(prefix) === 0) {
      var tok = entry[0].slice(prefix.length);
      if (entry[1] !== 0) result[tok] = entry[1];
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
    balance: getBalance(username, "BTCPC"),
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

function getAllStakePools() {
  var result = [];
  for (var entry of stakes) {
    result.push({ username: entry[0], ...entry[1] });
  }
  return result;
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

function getComputeProofs(epochNumber) {
  return computeProofsByEpoch.get(epochNumber) || [];
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
// Blob getters (BTCPC-FS, v2.11+)
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
 * (~25 min at 5 min epochs) — frequent enough to catch downtime quickly
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
// IoT sensor + gateway getters (v2.15-beta)
// ─────────────────────────────────────────────────────────────────

function getSensor(sensorId) {
  return sensors.get(sensorId) || null;
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
// Slashing getters
// ─────────────────────────────────────────────────────────────────

function getSlashRecords(username) {
  return slashRecords.get(username) || [];
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
        balances.set(_balanceKey(u, "BTCPC"), _round(s.balance));
      }
      if (typeof s.staked === "number" && s.staked > 0) {
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
      // Non-BTCPC token balances: { "user|TOKEN": amount }
      Object.keys(ext.extra_balances).forEach(function (k) {
        balances.set(k, ext.extra_balances[k]);
      });
    }
  }

  if (typeof snapshot.finality_epoch === "number") {
    setChainHeight(snapshot.finality_epoch);
  }
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
  blobChallenges.clear();
  blobChallengeStats.clear();
  miningProofsByEpoch.clear();
  computeProofsByEpoch.clear();
  slashRecords.clear();
  services.clear();
  snapshots.clear();
  sensors.clear();
  sensorReadings.clear();
  gateways.clear();
  gatewayHeartbeats.clear();
  bridgeWraps.clear();
  bridgeUnwraps.clear();
  bridgeFunders.clear();
  seenEntries.clear();
  chainHeight = -1;
  currentBlockCap = 1 * 1024 * 1024;
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
  // Token/NFT
  getToken: getToken,
  getAllTokens: getAllTokens,
  getNFT: getNFT,
  getNFTsByOwner: getNFTsByOwner,
  getNFTsByCollection: getNFTsByCollection,
  getAllNFTs: getAllNFTs,
  // Staking/delegation
  getStakePool: getStakePool,
  getAllStakePools: getAllStakePools,
  getDelegation: getDelegation,
  getDelegationsByDelegator: getDelegationsByDelegator,
  getDelegationsByRecipient: getDelegationsByRecipient,
  getTotalStaked: getTotalStaked,
  // Escrow
  getEscrow: getEscrow,
  getEscrowsByPayer: getEscrowsByPayer,
  getActiveEscrows: getActiveEscrows,
  // Epoch / proofs
  getEpoch: getEpoch,
  getLatestEpoch: getLatestEpoch,
  getChainHeight: getChainHeight,
  getRecentEpochs: getRecentEpochs,
  getMiningProofs: getMiningProofs,
  getComputeProofs: getComputeProofs,
  getMinerCount: getMinerCount,
  // Projects
  getProject: getProject,
  getAllProjects: getAllProjects,
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
  // BTCPC-FS blobs (v2.11+)
  getBlobCommit: getBlobCommit,
  getAllBlobCommits: getAllBlobCommits,
  getBlobCommitsByHost: getBlobCommitsByHost,
  getBlobCommitsByUploader: getBlobCommitsByUploader,
  // BTCPC-FS storage heartbeats + uptime (v2.11.2+)
  getStorageHeartbeat: getStorageHeartbeat,
  getAllStorageHosts: getAllStorageHosts,
  getStorageUptimeFactor: getStorageUptimeFactor,
  getActiveStorageHosts: getActiveStorageHosts,
  getStorageHostsForEpoch: getStorageHostsForEpoch,
  // BTCPC-FS challenge-response (v2.11.2+, pay-for-delivery not slashing)
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
  // IoT sensor + gateway (v2.15-beta)
  getSensor: getSensor,
  getAllSensors: getAllSensors,
  getSensorsForEpoch: getSensorsForEpoch,
  getGateway: getGateway,
  getAllGateways: getAllGateways,
  getGatewaysForEpoch: getGatewaysForEpoch,
  // Bridge (v2.16-alpha)
  getBridgeFunder: getBridgeFunder,
  getAllBridgeFunders: getAllBridgeFunders,
  // Introspection
  snapshot: snapshot,
  stats: stats,
};
