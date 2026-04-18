"use strict";

/**
 * Public explorer routes — no API key required.
 * Mounted at / so scan.btcpc.net shows block explorer data directly.
 *
 * Node activity is derived from on-chain MINING_REWARD / STORAGE_HEARTBEAT /
 * CLOCK_REWARD ledger entries — the actual source of truth — not in-memory
 * node registry (which is local-only and not written to blocks).
 */

const express = require("express");
const fs = require("fs");
const path = require("path");
const router = express.Router();

const BLOCKS_DIR = path.resolve(process.cwd(), "data/blocks");
const ACTIVE_WINDOW_EPOCHS = 100; // consider a node "active" if seen in last 100 epochs

function bs() { return require("../chain/blockStore"); }
function ss() { return require("../chain/stateStore"); }

function ensureHydrated() {
  const store = ss();
  const bstore = bs();
  const latestFin = bstore.getLatestFinalityNumber ? bstore.getLatestFinalityNumber() : -1;
  if (latestFin >= 0) {
    const fin = bstore.readFinality(latestFin);
    if (fin && fin.snapshot && typeof store.hydrateFromFinality === "function") {
      store.hydrateFromFinality(fin.snapshot);
    }
  }
}

/**
 * Scan recent blocks and build a map of active nodes from ledger entries.
 * Returns Map<account, { account, roles: Set, last_epoch, total_rewards, epochs_active }>
 */
function scanActiveNodes(fromEpoch, toEpoch) {
  const nodes = new Map();
  const ROLE_ENTRY_TYPES = {
    MINING_REWARD: "miner",
    STORAGE_HEARTBEAT: "storage",
    CLOCK_REWARD: "clock",
    SENSOR_REWARD: "sensor",
  };

  for (let epoch = toEpoch; epoch >= fromEpoch; epoch--) {
    const data = bs().readBlock ? bs().readBlock(epoch) : null;
    if (!data) continue;
    const entries = (data.payload && data.payload.ledger_entries) || [];
    for (const entry of entries) {
      const role = ROLE_ENTRY_TYPES[entry.type];
      if (!role) continue;
      const account = entry.to || entry.from || entry.account;
      if (!account) continue;
      if (!nodes.has(account)) {
        nodes.set(account, { account, roles: new Set(), last_epoch: epoch, total_rewards: 0, epochs_active: 0 });
      }
      const node = nodes.get(account);
      node.roles.add(role);
      node.total_rewards += entry.amount || 0;
      node.epochs_active++;
      if (epoch > node.last_epoch) node.last_epoch = epoch;
    }
  }

  return nodes;
}

/**
 * GET /status — chain overview derived from actual block data
 */
router.get("/status", (req, res) => {
  try {
    ensureHydrated();
    const store = ss();
    const bstore = bs();

    const height = store.getChainHeight ? store.getChainHeight() : 0;
    const accounts = store.getAllAccounts ? store.getAllAccounts().length : 0;
    const latestFin = bstore.getLatestFinalityNumber ? bstore.getLatestFinalityNumber() : -1;

    const fromEpoch = Math.max(0, height - ACTIVE_WINDOW_EPOCHS);
    const activeNodes = scanActiveNodes(fromEpoch, height);

    const roles = { miner: 0, storage: 0, clock: 0, sensor: 0 };
    for (const node of activeNodes.values()) {
      for (const role of node.roles) {
        if (roles[role] !== undefined) roles[role]++;
      }
    }

    res.json({
      chain_height: height,
      accounts,
      active_nodes: activeNodes.size,
      miners: roles.miner,
      storage: roles.storage,
      clocks: roles.clock,
      sensors: roles.sensor,
      latest_finality: latestFin,
      active_window_epochs: ACTIVE_WINDOW_EPOCHS,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /miners — all nodes active in the last 100 epochs, with roles and earnings
 */
router.get("/miners", (req, res) => {
  try {
    ensureHydrated();
    const store = ss();
    const height = store.getChainHeight ? store.getChainHeight() : 0;
    const fromEpoch = Math.max(0, height - ACTIVE_WINDOW_EPOCHS);
    const activeNodes = scanActiveNodes(fromEpoch, height);

    const list = Array.from(activeNodes.values()).map(n => ({
      account: n.account,
      roles: Array.from(n.roles),
      balance: store.getBalance ? store.getBalance(n.account, "BTCPC") : 0,
      last_epoch: n.last_epoch,
      epochs_active: n.epochs_active,
      total_rewards: Math.round(n.total_rewards * 1e8) / 1e8,
    }));

    list.sort((a, b) => b.last_epoch - a.last_epoch);

    res.json({
      nodes: list,
      count: list.length,
      scanned_epochs: height - fromEpoch,
      chain_height: height,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /account/:name — account balance, on-chain activity summary
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
    const fromEpoch = Math.max(0, height - ACTIVE_WINDOW_EPOCHS);
    const activeNodes = scanActiveNodes(fromEpoch, height);
    const nodeInfo = activeNodes.get(name);

    res.json({
      account: name,
      balance: acct.balance || 0,
      staked: acct.staked || 0,
      nonce: acct.nonce || 0,
      created_epoch: acct.created_epoch || 0,
      node_roles: nodeInfo ? Array.from(nodeInfo.roles) : [],
      last_active_epoch: nodeInfo ? nodeInfo.last_epoch : null,
      epochs_active_last_100: nodeInfo ? nodeInfo.epochs_active : 0,
      rewards_last_100_epochs: nodeInfo ? Math.round(nodeInfo.total_rewards * 1e8) / 1e8 : 0,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /block/:number — block payload data
 */
router.get("/block/:number", (req, res) => {
  try {
    const num = parseInt(req.params.number, 10);
    if (isNaN(num) || num < 0) return res.status(400).json({ error: "invalid block number" });

    const data = bs().readBlock ? bs().readBlock(num) : null;
    if (!data) return res.status(404).json({ error: "block not found" });

    res.json({ block_number: num, ...data, timestamp: Date.now() });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /blocks — recent blocks with miner info from ledger entries
 */
router.get("/blocks", (req, res) => {
  try {
    const indexPath = path.join(BLOCKS_DIR, "index.json");
    let index = {};
    try { index = JSON.parse(fs.readFileSync(indexPath, "utf8")); } catch (_) {}

    const keys = Object.keys(index).map(Number).sort((a, b) => b - a).slice(0, 50);
    const bstore = bs();

    const recent = keys.map(k => {
      const entry = { epoch: k, hash: index[k], miner: null, reward: null };
      try {
        const data = bstore.readBlock(k);
        if (data) {
          const reward = (data.payload.ledger_entries || []).find(e => e.type === "MINING_REWARD");
          if (reward) { entry.miner = reward.to; entry.reward = reward.amount; }
        }
      } catch (_) {}
      return entry;
    });

    res.json({ recent_blocks: recent, count: Object.keys(index).length, timestamp: Date.now() });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
