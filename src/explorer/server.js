"use strict";

/**
 * BTCPC Block Explorer
 * Bitcoin Proof of Compute — Web Explorer
 * Port 4242 (42 x 101)
 *
 * Designed by Shin Devlin
 */

require("dotenv").config();
const express = require("express");
const mongoose = require("mongoose");

// Models
const Epoch = require("../models/Epoch");
const Node = require("../models/Node");
const Transaction = require("../models/Transaction");
const User = require("../models/User");
const StakingPool = require("../models/StakingPool");
const WorkProof = require("../models/WorkProof");

// Emission schedule
const { getCurrentPeriod, getPeriodTable, TOTAL_SUPPLY } = require("../services/emissionSchedule");

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
  console.log("[explorer] Connected to MongoDB");
}).catch(err => {
  console.error("[explorer] MongoDB connection error:", err.message);
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
      recentTransactions,
      totalTransactions,
      stakingAgg
    ] = await Promise.all([
      Epoch.find().sort({ epoch_number: -1 }).limit(15).lean(),
      Epoch.countDocuments(),
      Node.countDocuments(),
      Transaction.find().sort({ timestamp: -1 }).limit(10).lean(),
      Transaction.countDocuments(),
      StakingPool.aggregate([
        { $match: { status: "active" } },
        { $group: { _id: null, total: { $sum: "$staked_amount" } } }
      ])
    ]);

    // Compute total mined from finalized epochs
    const minedAgg = await Transaction.aggregate([
      { $match: { type: "mining_reward" } },
      { $group: { _id: null, total: { $sum: "$amount" } } }
    ]);

    const totalMined = minedAgg.length ? minedAgg[0].total : 0;
    const totalStaked = stakingAgg.length ? stakingAgg[0].total : 0;
    const currentEpoch = epochCount > 0 ? latestEpochs[0].epoch_number : 0;
    const currentPeriod = getCurrentPeriod(currentEpoch);

    res.send(dashboardView({
      totalSupply: TOTAL_SUPPLY,
      totalMined,
      currentEpoch,
      totalMiners,
      totalTransactions,
      latestEpochs,
      recentTransactions,
      currentPeriod,
      totalStaked
    }));
  } catch (err) {
    console.error("[explorer] Dashboard error:", err);
    res.status(500).send("Internal server error");
  }
});

/**
 * GET /block/:epoch — Epoch detail
 */
app.get("/block/:epoch", async (req, res) => {
  try {
    const epochNum = parseInt(req.params.epoch, 10);
    if (isNaN(epochNum)) return res.status(400).send("Invalid epoch number");

    const epoch = await Epoch.findOne({ epoch_number: epochNum }).lean();
    const period = getCurrentPeriod(epochNum);

    res.send(blockView(epoch, period));
  } catch (err) {
    console.error("[explorer] Block error:", err);
    res.status(500).send("Internal server error");
  }
});

/**
 * GET /account/:username — Account page
 */
app.get("/account/:username", async (req, res) => {
  try {
    const username = req.params.username;
    const user = await User.findOne({ username }).lean();

    if (!user) {
      return res.send(accountView({ user: null }));
    }

    const [node, stake, transactions, miningRewards] = await Promise.all([
      Node.findOne({ account: user._id }).lean(),
      StakingPool.findOne({ account: user._id, status: { $in: ["active", "unstaking"] } }).lean(),
      Transaction.find({
        $or: [{ from: username }, { to: username }]
      }).sort({ timestamp: -1 }).limit(50).lean(),
      Transaction.find({ to: username, type: "mining_reward" }).lean()
    ]);

    // Compute balance: sum of incoming - sum of outgoing
    const balanceAgg = await Transaction.aggregate([
      {
        $facet: {
          incoming: [
            { $match: { to: username } },
            { $group: { _id: null, total: { $sum: "$amount" } } }
          ],
          outgoing: [
            { $match: { from: username } },
            { $group: { _id: null, total: { $sum: "$amount" } } }
          ]
        }
      }
    ]);

    const incoming = balanceAgg[0].incoming.length ? balanceAgg[0].incoming[0].total : 0;
    const outgoing = balanceAgg[0].outgoing.length ? balanceAgg[0].outgoing[0].total : 0;
    const balance = incoming - outgoing;

    res.send(accountView({ user, node, stake, transactions, miningRewards, balance }));
  } catch (err) {
    console.error("[explorer] Account error:", err);
    res.status(500).send("Internal server error");
  }
});

/**
 * GET /tx — Recent transactions list
 */
app.get("/tx", async (req, res) => {
  try {
    const page = Math.max(1, parseInt(req.query.page, 10) || 1);
    const perPage = 25;
    const skip = (page - 1) * perPage;

    const [transactions, total] = await Promise.all([
      Transaction.find().sort({ timestamp: -1 }).skip(skip).limit(perPage).lean(),
      Transaction.countDocuments()
    ]);

    res.send(transactionsView({ transactions, total, page, perPage }));
  } catch (err) {
    console.error("[explorer] Transactions error:", err);
    res.status(500).send("Internal server error");
  }
});

/**
 * GET /miners — Active miner list
 */
app.get("/miners", async (req, res) => {
  try {
    const miners = await Node.find().sort({ reputation: -1 }).populate("account", "username").lean();

    // Map populated account to _account for the view
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
    console.error("[explorer] Miners error:", err);
    res.status(500).send("Internal server error");
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
      totalTransactions,
      stakingAgg,
      minedAgg
    ] = await Promise.all([
      Epoch.countDocuments(),
      Epoch.findOne().sort({ epoch_number: -1 }).lean(),
      Node.countDocuments(),
      Node.countDocuments({ status: "active" }),
      Transaction.countDocuments(),
      StakingPool.aggregate([
        { $match: { status: "active" } },
        { $group: { _id: null, total: { $sum: "$staked_amount" } } }
      ]),
      Transaction.aggregate([
        { $match: { type: "mining_reward" } },
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
      total_transactions: totalTransactions,
      total_staked: stakingAgg.length ? stakingAgg[0].total : 0,
      emission_periods: periodTable.length,
      explorer_version: "1.0.0"
    });
  } catch (err) {
    console.error("[explorer] API stats error:", err);
    res.status(500).json({ error: "Internal server error" });
  }
});

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------
app.listen(PORT, () => {
  console.log(`
  ============================================
    BTCPC Block Explorer
    Bitcoin Proof of Compute
    http://localhost:${PORT}
  ============================================
  `);
});
