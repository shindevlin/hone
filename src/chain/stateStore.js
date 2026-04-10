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

// Mining proofs indexed by epoch
var miningProofsByEpoch = new Map();

// Compute proofs indexed by epoch
var computeProofsByEpoch = new Map();

// Slashing records: username → [ { epoch, offenseType, tier, amount, evidence } ]
var slashRecords = new Map();

// Chain height: highest known finalized epoch
var chainHeight = -1;

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
  balances.set(key, _round((balances.get(key) || 0) - amount));
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
// Slashing getters
// ─────────────────────────────────────────────────────────────────

function getSlashRecords(username) {
  return slashRecords.get(username) || [];
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
  miningProofsByEpoch.clear();
  computeProofsByEpoch.clear();
  slashRecords.clear();
  seenEntries.clear();
  chainHeight = -1;
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
  // Slashing
  getSlashRecords: getSlashRecords,
  // Introspection
  snapshot: snapshot,
  stats: stats,
};
