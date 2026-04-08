"use strict";

/**
 * BTCPC Block Explorer (btcpcscan)
 * Bitcoin Proof of Compute — Chain Explorer
 * Port 4242 (42 x 101)
 *
 * Reads from the permanent ledger and block files — the source of truth.
 * MongoDB is used as a cache for fast queries.
 *
 * Designed by Shin Devlin
 */

require("dotenv").config();
const express = require("express");
const mongoose = require("mongoose");

// Models
const Epoch = require("../models/Epoch");
const Node = require("../models/Node");
const User = require("../models/User");
const StakingPool = require("../models/StakingPool");
const LedgerEntry = require("../models/LedgerEntry");

// Chain
const blockStore = require("../chain/blockStore");
const stateManager = require("../chain/stateManager");
const { verifyBalance, verifyAccountState } = require("../chain/computeVerifier");
const mempool = require("../p2p/mempool");

// Emission schedule
const { getCurrentPeriod, getPeriodTable, TOTAL_SUPPLY } = require("../services/emissionSchedule");
const ledger = require("../services/ledger");

// Views
const dashboardView = require("./views/dashboard");
const blockView = require("./views/block");
const accountView = require("./views/account");
const transactionsView = require("./views/transactions");
const minersView = require("./views/miners");

const app = express();
const PORT = 4242;

// ---------------------------------------------------------------------------
// MongoDB connection
// ---------------------------------------------------------------------------
const MONGODB_URI = process.env.MONGODB_URI || "mongodb://root:example@localhost:27017/btcpc?authSource=admin";

mongoose.connect(MONGODB_URI).then(() => {
  console.log("[btcpcscan] Connected to MongoDB");
}).catch(err => {
  console.error("[btcpcscan] MongoDB connection error:", err.message);
  process.exit(1);
});

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

/**
 * GET / — Dashboard
 */
app.get("/", async (req, res) => {
  try {
    const [
      latestEpochs,
      epochCount,
      totalMiners,
      recentLedgerEntries,
      totalLedgerEntries,
      stakingAgg
    ] = await Promise.all([
      Epoch.find().sort({ epoch_number: -1 }).limit(15).lean(),
      Epoch.countDocuments(),
      Node.countDocuments(),
      LedgerEntry.find().sort({ timestamp: -1 }).limit(10).lean(),
      LedgerEntry.countDocuments(),
      StakingPool.aggregate([
        { $match: { status: "active" } },
        { $group: { _id: null, total: { $sum: "$staked_amount" } } }
      ])
    ]);

    // Compute total mined from ledger (source of truth)
    const minedAgg = await LedgerEntry.aggregate([
      { $match: { type: "MINING_REWARD", token: "BTCPC" } },
      { $group: { _id: null, total: { $sum: "$amount" } } }
    ]);

    const totalMined = minedAgg.length ? minedAgg[0].total : 0;
    const totalStaked = stakingAgg.length ? stakingAgg[0].total : 0;
    const currentEpoch = epochCount > 0 ? latestEpochs[0].epoch_number : 0;
    const currentPeriod = getCurrentPeriod(currentEpoch);

    // Map ledger entries to transaction-like format for the view
    const recentTransactions = recentLedgerEntries.map(formatLedgerEntry);

    res.send(dashboardView({
      totalSupply: TOTAL_SUPPLY,
      totalMined,
      currentEpoch,
      totalMiners,
      totalTransactions: totalLedgerEntries,
      latestEpochs,
      recentTransactions,
      currentPeriod,
      totalStaked,
      mempoolSize: mempool.size(),
      latestBlockOnDisk: blockStore.getLatestBlockNumber(),
      stateRoot: stateManager.getStateRoot()
    }));
  } catch (err) {
    console.error("[btcpcscan] Dashboard error:", err);
    res.status(500).send("Internal server error");
  }
});

/**
 * GET /block/:epoch — Block detail (reads from disk)
 */
app.get("/block/:epoch", async (req, res) => {
  try {
    const epochNum = parseInt(req.params.epoch, 10);
    if (isNaN(epochNum)) return res.status(400).send("Invalid epoch number");

    // Read from block file (source of truth) if available
    const blockData = blockStore.readBlock(epochNum);

    // Fall back to MongoDB for epoch metadata
    const epoch = await Epoch.findOne({ epoch_number: epochNum }).lean();
    const period = getCurrentPeriod(epochNum);

    res.send(blockView(epoch, period, blockData));
  } catch (err) {
    console.error("[btcpcscan] Block error:", err);
    res.status(500).send("Internal server error");
  }
});

/**
 * GET /account/:username — Account page (balance from ledger)
 */
app.get("/account/:username", async (req, res) => {
  try {
    const username = req.params.username;
    const user = await User.findOne({ username }).lean();

    if (!user) {
      return res.send(accountView({ user: null }));
    }

    const [node, stake, ledgerEntries, miningRewards] = await Promise.all([
      Node.findOne({ account: user._id }).lean(),
      StakingPool.findOne({ account: user._id, status: { $in: ["active", "unstaking"] } }).lean(),
      LedgerEntry.find({
        $or: [{ from: username }, { to: username }]
      }).sort({ timestamp: -1 }).limit(50).lean(),
      LedgerEntry.find({ to: username, type: "MINING_REWARD" }).lean()
    ]);

    // Compute balance from ledger (source of truth)
    const balance = await ledger.getBalance(username, "BTCPC");
    const pendingDebit = mempool.getPendingDebit(username);

    // Get SMT proof if available
    const smtState = stateManager.getAccountState(username);

    // Map ledger entries to transaction-like format for the view
    const transactions = ledgerEntries.map(formatLedgerEntry);

    res.send(accountView({
      user, node, stake, transactions, miningRewards,
      balance,
      pendingDebit,
      availableBalance: balance - pendingDebit,
      smtProof: smtState ? { root: smtState.root, state: smtState.state } : null
    }));
  } catch (err) {
    console.error("[btcpcscan] Account error:", err);
    res.status(500).send("Internal server error");
  }
});

/**
 * GET /tx — Ledger entries list (permanent on-chain transactions)
 */
app.get("/tx", async (req, res) => {
  try {
    const page = Math.max(1, parseInt(req.query.page, 10) || 1);
    const perPage = 25;
    const skip = (page - 1) * perPage;

    const [entries, total] = await Promise.all([
      LedgerEntry.find().sort({ timestamp: -1 }).skip(skip).limit(perPage).lean(),
      LedgerEntry.countDocuments()
    ]);

    const transactions = entries.map(formatLedgerEntry);

    res.send(transactionsView({ transactions, total, page, perPage }));
  } catch (err) {
    console.error("[btcpcscan] Transactions error:", err);
    res.status(500).send("Internal server error");
  }
});

/**
 * GET /tx/:txHash — Transaction detail by hash
 */
app.get("/tx/:txHash", async (req, res) => {
  try {
    var txHash = req.params.txHash;

    // Search mempool first
    if (mempool.hasTransaction(txHash)) {
      var mempoolTx = mempool.getTransactions().find(function (t) { return t.txHash === txHash; });
      if (mempoolTx) {
        return res.json({
          status: "pending",
          txHash: txHash,
          type: mempoolTx.type,
          from: mempoolTx.from,
          to: mempoolTx.to,
          amount: mempoolTx.amount,
          token: mempoolTx.token || "BTCPC",
          timestamp: mempoolTx.timestamp,
          memo: mempoolTx.memo
        });
      }
    }

    // Search ledger — compute hash of each entry to find match
    // For now, return 404 — txHash lookup requires an index we'll add later
    res.status(404).json({ error: "Transaction not found", txHash: txHash });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /miners — Active miner list
 */
app.get("/miners", async (req, res) => {
  try {
    const miners = await Node.find().sort({ reputation: -1 }).populate("account", "username").lean();

    const mappedMiners = miners.map(m => ({
      ...m,
      _account: m.account && typeof m.account === "object" ? m.account : null
    }));

    const activeCount = miners.filter(m => m.status === "active").length;
    const totalStaked = miners.reduce((sum, m) => sum + (m.stake_amount || 0), 0);

    res.send(minersView({
      miners: mappedMiners,
      totalStaked,
      activeCount,
      totalCount: miners.length
    }));
  } catch (err) {
    console.error("[btcpcscan] Miners error:", err);
    res.status(500).send("Internal server error");
  }
});

/**
 * GET /mempool — Pending transactions
 */
app.get("/mempool", async (req, res) => {
  try {
    var txs = mempool.getTransactions();
    var stats = mempool.getStats();
    res.json({
      size: stats.size,
      maxSize: stats.maxSize,
      oldestAgeSec: stats.oldestAgeSec,
      transactions: txs.map(function (t) {
        return {
          txHash: t.txHash,
          type: t.type,
          from: t.from,
          to: t.to,
          amount: t.amount,
          token: t.token || "BTCPC",
          timestamp: t.timestamp,
          age_ms: Date.now() - t._mempoolAddedAt
        };
      })
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/stats — JSON endpoint for programmatic access
 */
app.get("/api/stats", async (req, res) => {
  try {
    const [
      epochCount,
      latestEpoch,
      totalMiners,
      activeMiners,
      totalLedgerEntries,
      stakingAgg,
      minedAgg
    ] = await Promise.all([
      Epoch.countDocuments(),
      Epoch.findOne().sort({ epoch_number: -1 }).lean(),
      Node.countDocuments(),
      Node.countDocuments({ status: "active" }),
      LedgerEntry.countDocuments(),
      StakingPool.aggregate([
        { $match: { status: "active" } },
        { $group: { _id: null, total: { $sum: "$staked_amount" } } }
      ]),
      LedgerEntry.aggregate([
        { $match: { type: "MINING_REWARD", token: "BTCPC" } },
        { $group: { _id: null, total: { $sum: "$amount" } } }
      ])
    ]);

    const currentEpoch = latestEpoch ? latestEpoch.epoch_number : 0;
    const currentPeriod = getCurrentPeriod(currentEpoch);
    const periodTable = getPeriodTable();

    res.json({
      chain: "BTCPC",
      name: "Bitcoin Proof of Compute",
      total_supply: TOTAL_SUPPLY,
      total_mined: minedAgg.length ? minedAgg[0].total : 0,
      current_epoch: currentEpoch,
      current_period: currentPeriod ? {
        period: currentPeriod.period,
        reward_per_epoch: currentPeriod.reward_per_epoch,
        duration_months: currentPeriod.duration_months,
        allotment: currentPeriod.allotment
      } : null,
      total_miners: totalMiners,
      active_miners: activeMiners,
      total_ledger_entries: totalLedgerEntries,
      total_staked: stakingAgg.length ? stakingAgg[0].total : 0,
      mempool_size: mempool.size(),
      blocks_on_disk: blockStore.getLatestBlockNumber() + 1,
      latest_finality: blockStore.getLatestFinalityNumber(),
      state_root: stateManager.getStateRoot(),
      emission_periods: periodTable.length,
      explorer_version: "2.0.0"
    });
  } catch (err) {
    console.error("[btcpcscan] API stats error:", err);
    res.status(500).json({ error: "Internal server error" });
  }
});

/**
 * GET /api/verify/:username — Verify account state via SMT
 */
app.get("/api/verify/:username", async (req, res) => {
  try {
    var result = verifyAccountState(req.params.username);
    res.json(result);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/block/:epoch — Block data as JSON (from disk)
 */
app.get("/api/block/:epoch", async (req, res) => {
  try {
    var epochNum = parseInt(req.params.epoch, 10);
    if (isNaN(epochNum)) return res.status(400).json({ error: "Invalid epoch" });

    var blockData = blockStore.readBlock(epochNum);
    if (!blockData) return res.status(404).json({ error: "Block not found on disk" });

    res.json({
      epoch: epochNum,
      hash: blockData.block.computeHash(),
      previous_hash: blockData.block.previous_block_hash,
      state_root: blockData.block.state_root,
      merkle_root_transactions: blockData.block.merkle_root_transactions,
      merkle_root_compute_proofs: blockData.block.merkle_root_compute_proofs,
      timestamp: blockData.block.timestamp,
      difficulty: blockData.block.difficulty,
      miner_id: blockData.block.miner_id,
      ledger_entries: blockData.payload.ledger_entries,
      rewards: blockData.payload.rewards,
      compute_proofs: blockData.payload.compute_proofs,
      mining_proofs: blockData.payload.mining_proofs
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/account/:username — Account data as JSON (from ledger)
 */
app.get("/api/account/:username", async (req, res) => {
  try {
    var username = req.params.username;
    var balance = await ledger.getBalance(username, "BTCPC");
    var allBalances = await ledger.getTokenBalances(username);
    var accountRecord = await ledger.getAccountRecord(username);
    var pendingDebit = mempool.getPendingDebit(username);
    var smtState = stateManager.getAccountState(username);

    res.json({
      username: username,
      balance: balance,
      pending_debit: pendingDebit,
      available: balance - pendingDebit,
      token_balances: allBalances,
      account: accountRecord,
      smt: smtState ? { root: smtState.root, state: smtState.state } : null
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Format a LedgerEntry to look like a transaction for the view layer.
 */
function formatLedgerEntry(entry) {
  return {
    _id: entry._id,
    txHash: blockStore.hashLedgerEntry(entry),
    type: entry.type ? entry.type.toLowerCase() : "unknown",
    from: entry.from || "",
    to: entry.to || "",
    amount: entry.amount || 0,
    token: entry.token || "BTCPC",
    memo: entry.memo || "",
    timestamp: entry.timestamp,
    epoch: entry.epoch,
    signed_by: entry.signed_by
  };
}

/**
 * GET /settings — Miner resource management UI
 */
app.get("/settings", async (req, res) => {
  try {
    var resourceManager = require("../services/resourceManager");
    var status = resourceManager.getStatus();
    var settingsView = require("./views/settings");
    res.send(settingsView(status));
  } catch (err) {
    res.status(500).send("Error: " + err.message);
  }
});

/**
 * POST /settings — Update miner settings via form
 */
app.use(express.urlencoded({ extended: false }));
app.post("/settings", async (req, res) => {
  try {
    var resourceManager = require("../services/resourceManager");
    var body = req.body;

    if (body.action === "pause") resourceManager.pause();
    else if (body.action === "resume") resourceManager.resume();
    else if (body.action === "full") resourceManager.setManualMode("full");
    else if (body.action === "reduced") resourceManager.setManualMode("reduced");
    else if (body.action === "auto") resourceManager.setManualMode(null);

    if (body.cpuCap) resourceManager.setCpuCap(parseInt(body.cpuCap));
    if (body.gpuCap) resourceManager.setGpuCap(parseInt(body.gpuCap));

    res.redirect("/settings");
  } catch (err) {
    res.status(500).send("Error: " + err.message);
  }
});

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------
app.listen(PORT, () => {
  console.log(`
  ============================================
    btcpcscan — BTCPC Chain Explorer
    Bitcoin Proof of Compute
    http://localhost:${PORT}
  ============================================
  `);
});
