"use strict";

/**
 * Public explorer routes — no API key required.
 * Mounted at / so scan.btcpc.net can read all chain data.
 *
 * All node/activity data is derived from on-chain ledger entries.
 * Block files are the source of truth; finality snapshots for balances.
 */

const express = require("express");
const fs = require("fs");
const path = require("path");
const router = express.Router();

const BLOCKS_DIR = path.resolve(process.cwd(), "data/blocks");
const GENESIS_TIMESTAMP = 1776236400000;
const EPOCH_MS = 30000;
const GENESIS_REWARD = 243.06;

// ─── helpers ────────────────────────────────────────────────────────────────

function bs() { return require("../chain/blockStore"); }
function ss() { return require("../chain/stateStore"); }

function ensureHydrated() {
  const store = ss();
  const bstore = bs();
  if (typeof store.hydrateFromFinality !== "function") return;
  const latestFin = bstore.getLatestFinalityNumber ? bstore.getLatestFinalityNumber() : -1;
  if (latestFin >= 0) {
    const fin = bstore.readFinality(latestFin);
    if (fin && fin.snapshot) store.hydrateFromFinality(fin.snapshot);
  }
}

function readIndex() {
  try { return JSON.parse(fs.readFileSync(path.join(BLOCKS_DIR, "index.json"), "utf8")); }
  catch (_) { return {}; }
}

function epochTimestamp(epoch) {
  return GENESIS_TIMESTAMP + epoch * EPOCH_MS;
}

// Scan a range of epochs and aggregate ledger entries.
// Returns { byAccount, byType, entries[] } where entries are the last `limit` entries.
function scanBlocks(fromEpoch, toEpoch, limit) {
  const byAccount = new Map(); // account → { roles, total_rewards, epochs_active, last_epoch }
  const byType = {};
  const entries = [];

  for (let epoch = toEpoch; epoch >= fromEpoch; epoch--) {
    const data = bs().readBlock ? bs().readBlock(epoch) : null;
    if (!data) continue;

    for (const e of (data.payload.ledger_entries || [])) {
      byType[e.type] = (byType[e.type] || 0) + 1;

      // Classify node roles from entry type
      // Only count earning entry types as active node roles.
      // SENSOR_READING is an activity proof (amount:0), not an earning event.
      const ROLE_MAP = {
        MINING_REWARD: "miner",
        STORAGE_HEARTBEAT: "storage",
        CLOCK_REWARD: "clock",
        SENSOR_REWARD: "sensor",
      };
      const role = ROLE_MAP[e.type];
      const account = e.to || e.from || e.account;
      if (role && account) {
        if (!byAccount.has(account)) {
          byAccount.set(account, { roles: new Set(), total_rewards: 0, epochs_active: 0, last_epoch: epoch });
        }
        const n = byAccount.get(account);
        n.roles.add(role);
        n.total_rewards += e.amount || 0;
        n.epochs_active++;
        if (epoch > n.last_epoch) n.last_epoch = epoch;
      }

      if (!limit || entries.length < limit) {
        entries.push({ epoch, ...e, _ts: epochTimestamp(epoch) });
      }
    }
  }

  return { byAccount, byType, entries };
}

// ─── routes ─────────────────────────────────────────────────────────────────

/**
 * GET /status — chain overview
 */
router.get("/status", (req, res) => {
  try {
    ensureHydrated();
    const store = ss();
    const bstore = bs();
    const index = readIndex();

    const height = store.getChainHeight ? store.getChainHeight() : 0;
    const allAccounts = store.getAllAccounts ? store.getAllAccounts() : [];
    const latestFin = bstore.getLatestFinalityNumber ? bstore.getLatestFinalityNumber() : -1;

    const fromEpoch = Math.max(0, height - 100);
    const { byAccount, byType } = scanBlocks(fromEpoch, height, 0);

    const roles = { miner: 0, storage: 0, clock: 0, sensor: 0 };
    for (const n of byAccount.values()) {
      for (const r of n.roles) if (r in roles) roles[r]++;
    }

    // Circulating supply = sum of all non-zero balances
    let circulating = 0;
    for (const a of allAccounts) {
      circulating += store.getBalance ? store.getBalance(a.username, "BTCPC") : (a.balance || 0);
    }

    const epochsInIndex = Object.keys(index).length;

    res.json({
      chain_height: height,
      epoch_time_ms: EPOCH_MS,
      genesis_timestamp: GENESIS_TIMESTAMP,
      latest_finality: latestFin,
      blocks_indexed: epochsInIndex,
      accounts: allAccounts.length,
      circulating_supply: Math.round(circulating * 1e8) / 1e8,
      max_supply: 42000000,
      current_reward_per_epoch: GENESIS_REWARD,
      active_window_epochs: 100,
      active_nodes: byAccount.size,
      miners: roles.miner,
      storage_nodes: roles.storage,
      clock_nodes: roles.clock,
      sensor_nodes: roles.sensor,
      entry_types_last_100: byType,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /supply — token supply breakdown
 */
router.get("/supply", (req, res) => {
  try {
    ensureHydrated();
    const store = ss();
    const allAccounts = store.getAllAccounts ? store.getAllAccounts() : [];

    const balances = {};
    let total = 0;
    for (const a of allAccounts) {
      const bal = store.getBalance ? store.getBalance(a.username, "BTCPC") : (a.balance || 0);
      if (bal > 0) { balances[a.username] = Math.round(bal * 1e8) / 1e8; total += bal; }
    }

    const height = store.getChainHeight ? store.getChainHeight() : 0;
    const estTotalMined = height * GENESIS_REWARD;

    res.json({
      max_supply: 42000000,
      estimated_mined: Math.round(estTotalMined * 1e8) / 1e8,
      circulating: Math.round(total * 1e8) / 1e8,
      accounts_with_balance: Object.keys(balances).length,
      balances,
      chain_height: height,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /miners — all nodes active in last 100 epochs
 */
router.get("/miners", (req, res) => {
  try {
    ensureHydrated();
    const store = ss();
    const height = store.getChainHeight ? store.getChainHeight() : 0;
    const fromEpoch = Math.max(0, height - 100);

    const { byAccount } = scanBlocks(fromEpoch, height, 0);

    const list = Array.from(byAccount.values()).map(n => ({
      account: n.account || "?",
      roles: Array.from(n.roles),
      balance: store.getBalance ? Math.round(store.getBalance(n.account, "BTCPC") * 1e8) / 1e8 : 0,
      last_epoch: n.last_epoch,
      last_seen: epochTimestamp(n.last_epoch),
      epochs_active: n.epochs_active,
      rewards_window: Math.round(n.total_rewards * 1e8) / 1e8,
    }));

    // Also add accounts from byAccount map that don't have an `account` field
    // (fix: byAccount keys are the account names)
    const fixedList = Array.from(byAccount.entries()).map(([account, n]) => ({
      account,
      roles: Array.from(n.roles),
      balance: store.getBalance ? Math.round(store.getBalance(account, "BTCPC") * 1e8) / 1e8 : 0,
      last_epoch: n.last_epoch,
      last_seen: epochTimestamp(n.last_epoch),
      epochs_active: n.epochs_active,
      rewards_window: Math.round(n.total_rewards * 1e8) / 1e8,
    }));

    fixedList.sort((a, b) => b.last_epoch - a.last_epoch);

    res.json({
      nodes: fixedList,
      count: fixedList.length,
      scanned_epochs: height - fromEpoch,
      chain_height: height,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /activity — recent ledger entry feed across all entry types
 */
router.get("/activity", (req, res) => {
  try {
    const limit = Math.min(parseInt(req.query.limit) || 50, 200);
    const store = ss();
    const height = store.getChainHeight ? store.getChainHeight() : 0;
    const fromEpoch = Math.max(0, height - 200);

    const { entries, byType } = scanBlocks(fromEpoch, height, limit);

    res.json({
      entries,
      entry_types: byType,
      chain_height: height,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /blocks?limit=50 — recent blocks with per-block summary
 */
router.get("/blocks", (req, res) => {
  try {
    const limit = Math.min(parseInt(req.query.limit) || 50, 200);
    const index = readIndex();
    const bstore = bs();

    const keys = Object.keys(index).map(Number).sort((a, b) => b - a).slice(0, limit);

    const recent = keys.map(k => {
      const entry = {
        epoch: k,
        hash: index[k],
        timestamp: epochTimestamp(k),
        miner: null,
        reward: null,
        entry_count: 0,
        entry_types: [],
      };
      try {
        const data = bstore.readBlock(k);
        if (data) {
          const entries = data.payload.ledger_entries || [];
          entry.entry_count = entries.length;
          entry.entry_types = [...new Set(entries.map(e => e.type))];
          const reward = entries.find(e => e.type === "MINING_REWARD");
          if (reward) { entry.miner = reward.to; entry.reward = reward.amount; }
          // Block header fields
          if (data.block) {
            entry.proposer = data.block.miner_id || null;
            entry.prev_hash = data.block.previous_block_hash
              ? data.block.previous_block_hash.toString("hex").slice(0, 16) + "..."
              : null;
          }
        }
      } catch (_) {}
      return entry;
    });

    res.json({
      blocks: recent,
      total_indexed: Object.keys(index).length,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /block/:number — full block detail
 */
router.get("/block/:number", (req, res) => {
  try {
    const num = parseInt(req.params.number, 10);
    if (isNaN(num) || num < 0) return res.status(400).json({ error: "invalid block number" });

    const bstore = bs();
    const data = bstore.readBlock ? bstore.readBlock(num) : null;
    if (!data) return res.status(404).json({ error: "block not found" });

    const block = data.block || {};
    const payload = data.payload || {};

    res.json({
      epoch: num,
      timestamp: epochTimestamp(num),
      hash: (block.hash || "").toString("hex") || null,
      previous_hash: (block.previous_block_hash || "").toString("hex") || null,
      proposer: block.miner_id || null,
      state_root: (block.state_root || "").toString("hex") || null,
      difficulty: block.difficulty || null,
      ledger_entries: payload.ledger_entries || [],
      rewards: payload.rewards || [],
      compute_proofs: payload.compute_proofs || [],
      mining_proofs: payload.mining_proofs || [],
      entry_count: (payload.ledger_entries || []).length,
      timestamp_chain: block.timestamp || null,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /account/:name — account detail with on-chain activity
 */
router.get("/account/:name", (req, res) => {
  try {
    ensureHydrated();
    const name = (req.params.name || "").toLowerCase().trim();
    if (!name) return res.status(400).json({ error: "account name required" });

    const store = ss();
    const acct = store.getAccount ? store.getAccount(name) : null;
    if (!acct) return res.status(404).json({ error: "account not found" });

    const height = store.getChainHeight ? store.getChainHeight() : 0;
    const fromEpoch = Math.max(0, height - 100);
    const { byAccount } = scanBlocks(fromEpoch, height, 0);
    const nodeInfo = byAccount.get(name);

    res.json({
      account: name,
      balance: Math.round((acct.balance || 0) * 1e8) / 1e8,
      staked: acct.staked || 0,
      delegated: acct.delegated || 0,
      nonce: acct.nonce || 0,
      created_epoch: acct.created_epoch || 0,
      created_at: epochTimestamp(acct.created_epoch || 0),
      node_roles: nodeInfo ? Array.from(nodeInfo.roles) : [],
      last_active_epoch: nodeInfo ? nodeInfo.last_epoch : null,
      last_active_at: nodeInfo ? epochTimestamp(nodeInfo.last_epoch) : null,
      epochs_active_last_100: nodeInfo ? nodeInfo.epochs_active : 0,
      rewards_last_100_epochs: nodeInfo ? Math.round(nodeInfo.total_rewards * 1e8) / 1e8 : 0,
      chain_addresses: acct.chain_addresses || {},
      identity_linked_epoch: acct.identity_linked_epoch || null,
      identity_linked_at: acct.identity_linked_epoch ? epochTimestamp(acct.identity_linked_epoch) : null,
      cross_chain_credits: store.getCrossChainCredits ? store.getCrossChainCredits(name) : {},
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /account/:name/history — ledger entries involving this account
 */
router.get("/account/:name/history", (req, res) => {
  try {
    const name = (req.params.name || "").toLowerCase().trim();
    if (!name) return res.status(400).json({ error: "account name required" });

    const limit = Math.min(parseInt(req.query.limit) || 50, 200);
    const store = ss();
    const height = store.getChainHeight ? store.getChainHeight() : 0;
    const index = readIndex();
    const keys = Object.keys(index).map(Number).sort((a, b) => b - a);
    const bstore = bs();

    const history = [];
    for (const epoch of keys) {
      if (history.length >= limit) break;
      const data = bstore.readBlock ? bstore.readBlock(epoch) : null;
      if (!data) continue;
      for (const e of (data.payload.ledger_entries || [])) {
        if (e.to === name || e.from === name || e.account === name) {
          history.push({ epoch, _ts: epochTimestamp(epoch), ...e });
          if (history.length >= limit) break;
        }
      }
    }

    res.json({
      account: name,
      history,
      count: history.length,
      chain_height: height,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /accounts — all accounts with balances
 */
router.get("/accounts", (req, res) => {
  try {
    ensureHydrated();
    const store = ss();
    const allAccounts = store.getAllAccounts ? store.getAllAccounts() : [];

    const list = allAccounts.map(a => ({
      account: a.username,
      balance: Math.round((store.getBalance ? store.getBalance(a.username, "BTCPC") : a.balance || 0) * 1e8) / 1e8,
      staked: a.staked || 0,
      nonce: a.nonce || 0,
      created_epoch: a.created_epoch || 0,
    })).sort((a, b) => b.balance - a.balance);

    res.json({
      accounts: list,
      count: list.length,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /network — reward schedule, epoch timing, work info
 */
router.get("/network", (req, res) => {
  try {
    ensureHydrated();
    const store = ss();
    const height = store.getChainHeight ? store.getChainHeight() : 0;

    // Scan last 10 blocks for average reward and entry counts
    const fromEpoch = Math.max(0, height - 10);
    let totalReward = 0;
    let rewardBlocks = 0;
    for (let e = fromEpoch; e <= height; e++) {
      const data = bs().readBlock ? bs().readBlock(e) : null;
      if (!data) continue;
      const reward = (data.payload.ledger_entries || []).find(r => r.type === "MINING_REWARD");
      if (reward) { totalReward += reward.amount || 0; rewardBlocks++; }
    }
    const avgReward = rewardBlocks > 0 ? totalReward / rewardBlocks : GENESIS_REWARD;

    // Use the block's stored timestamp if available; epoch numbers are wall-clock
    // derived and don't count from 0 at genesis, so epochTimestamp() gives wrong age.
    let latestBlockTs = null;
    try {
      const d = bs().readBlock(height);
      if (d && d.block && d.block.timestamp) latestBlockTs = d.block.timestamp;
    } catch (_) {}
    const epochAge = latestBlockTs ? Date.now() - latestBlockTs : null;

    res.json({
      chain_height: height,
      epoch_ms: EPOCH_MS,
      genesis_timestamp: GENESIS_TIMESTAMP,
      chain_age_ms: Date.now() - GENESIS_TIMESTAMP,
      latest_epoch_age_ms: epochAge,
      latest_epoch_age_seconds: epochAge !== null ? Math.round(epochAge / 1000) : null,
      genesis_reward_per_epoch: GENESIS_REWARD,
      avg_reward_last_10: Math.round(avgReward * 1e8) / 1e8,
      epochs_per_day: Math.round(86400000 / EPOCH_MS),
      daily_emission: Math.round(GENESIS_REWARD * (86400000 / EPOCH_MS) * 1e8) / 1e8,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
