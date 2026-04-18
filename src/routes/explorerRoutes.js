"use strict";

/**
 * Public explorer routes — no API key required.
 * Mounted at / so scan.btcpc.net shows block explorer data directly.
 */

const express = require("express");
const router = express.Router();

function stateStore() { return require("../chain/stateStore"); }
function blockStore() { return require("../chain/blockStore"); }
function nodeRegistry() { return require("../chain/nodeRegistry"); }

function ensureHydrated() {
  const ss = stateStore();
  const bs = blockStore();
  const latestFin = bs.getLatestFinalityNumber ? bs.getLatestFinalityNumber() : -1;
  if (latestFin >= 0) {
    const fin = bs.readFinality(latestFin);
    if (fin && fin.snapshot && typeof ss.hydrateFromFinality === "function") {
      ss.hydrateFromFinality(fin.snapshot);
    }
  }
}

/**
 * GET /status — chain overview
 */
router.get("/status", (req, res) => {
  try {
    ensureHydrated();
    const ss = stateStore();
    const bs = blockStore();
    const reg = nodeRegistry();

    if (reg.getNodeCount() === 0 && typeof reg.loadFromBlocks === "function") {
      try { reg.loadFromBlocks(); } catch (_) {}
    }

    const height = ss.getChainHeight ? ss.getChainHeight() : 0;
    const accounts = ss.getAllAccounts ? ss.getAllAccounts().length : 0;
    const nodes = reg.getRegisteredNodes ? reg.getRegisteredNodes() : [];
    const latestFin = bs.getLatestFinalityNumber ? bs.getLatestFinalityNumber() : -1;

    res.json({
      chain_height: height,
      accounts,
      nodes: nodes.length,
      miners: nodes.filter(n => n.type === "miner").length,
      clocks: nodes.filter(n => n.type === "clock").length,
      storage: nodes.filter(n => n.type === "storage").length,
      latest_finality: latestFin,
      timestamp: Date.now(),
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /account/:name — account balance and node info
 */
router.get("/account/:name", (req, res) => {
  try {
    ensureHydrated();
    const name = (req.params.name || "").toLowerCase().trim();
    if (!name) return res.status(400).json({ error: "account name required" });

    const ss = stateStore();
    const acct = ss.getAccount ? ss.getAccount(name) : null;
    if (!acct) return res.status(404).json({ error: "account not found" });

    const reg = nodeRegistry();
    if (reg.getNodeCount() === 0 && typeof reg.loadFromBlocks === "function") {
      try { reg.loadFromBlocks(); } catch (_) {}
    }
    const nodes = (reg.getRegisteredNodes ? reg.getRegisteredNodes() : [])
      .filter(n => n.account === name)
      .map(n => ({ type: n.type, registered_epoch: n.registered_epoch }));

    res.json({
      account: name,
      balance: acct.balance || 0,
      staked: acct.staked || 0,
      nonce: acct.nonce || 0,
      created_epoch: acct.created_epoch || 0,
      nodes,
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

    const bs = blockStore();
    const data = bs.readBlock ? bs.readBlock(num) : null;
    if (!data) return res.status(404).json({ error: "block not found" });

    res.json({ block_number: num, ...data, timestamp: Date.now() });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /blocks — recent blocks index
 */
router.get("/blocks", (req, res) => {
  try {
    const fs = require("fs");
    const path = require("path");
    const blocksDir = path.resolve(process.cwd(), "data/blocks");
    const indexPath = path.join(blocksDir, "index.json");

    let index = {};
    try { index = JSON.parse(fs.readFileSync(indexPath, "utf8")); } catch (_) {}

    const keys = Object.keys(index).map(Number).sort((a, b) => b - a).slice(0, 50);
    const recent = keys.map(k => ({ epoch: k, hash: index[k] }));

    res.json({ recent_blocks: recent, count: Object.keys(index).length, timestamp: Date.now() });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
