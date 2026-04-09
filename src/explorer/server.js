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
const path = require("path");

// Models
const Epoch = require("../models/Epoch");
const Node = require("../models/Node");
const User = require("../models/User");
const StakingPool = require("../models/StakingPool");
const LedgerEntry = require("../models/LedgerEntry");
const GenesisDream = require("../models/GenesisDream");
const PeerRegistry = require("../models/PeerRegistry");
const Wallet = require("../models/Wallet");
const Transaction = require("../models/Transaction");

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
const tokenomicsView = require("./views/tokenomics");
const blocksView = require("./views/blocks");
const txDetailView = require("./views/txDetail");
const userDashboardView = require("./views/userDashboard");

const { sanitizeTelegramId, sanitizeString, sanitizeAmount, sanitizePagination, validAddress, validAccountName, validChain, rejectObjectInputs } = require("../middlewares/validate");

const app = express();
const PORT = 4242;
const WEBAPP_PATH = path.join(__dirname, "..", "telegram-webapp", "index.html");

app.use(express.json());

// ---------------------------------------------------------------------------
// MongoDB connection
// ---------------------------------------------------------------------------
const MONGODB_URI = process.env.MONGODB_URI;
if (!MONGODB_URI) { console.error("[btcpcscan] FATAL: MONGODB_URI not set"); process.exit(1); }

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
 * GET /webapp — Telegram Mini App wallet.
 */
app.get("/webapp", (_req, res) => {
  res.sendFile(WEBAPP_PATH);
});

async function resolveTelegramUser(telegramId) {
  if (!telegramId) return null;
  return User.findOne({ telegramId: String(telegramId) });
}

/**
 * Validate Telegram WebApp init data.
 * Telegram signs the initData with the bot token — we verify to prevent spoofing.
 * If BOT_TOKEN is not set, these endpoints are read-only (balance/history only).
 */
function validateTelegramInitData(req, res, next) {
  var initData = req.headers['x-telegram-init-data'] || req.query.initData;
  if (!initData) {
    // No init data — allow read-only endpoints, block write operations
    req._telegramVerified = false;
    return next();
  }

  try {
    var crypto = require('crypto');
    var botToken = process.env.BTCPC_BOT_TOKEN || process.env.TELEGRAM_BOT_TOKEN;
    if (!botToken) { req._telegramVerified = false; return next(); }

    var params = new URLSearchParams(initData);
    var hash = params.get('hash');
    params.delete('hash');
    var sorted = Array.from(params.entries()).sort((a, b) => a[0].localeCompare(b[0]));
    var dataCheckString = sorted.map(([k, v]) => k + '=' + v).join('\n');
    var secretKey = crypto.createHmac('sha256', 'WebAppData').update(botToken).digest();
    var computed = crypto.createHmac('sha256', secretKey).update(dataCheckString).digest('hex');

    if (computed === hash) {
      req._telegramVerified = true;
      var userData = params.get('user');
      if (userData) req._telegramUser = JSON.parse(userData);
    } else {
      req._telegramVerified = false;
    }
  } catch (_) {
    req._telegramVerified = false;
  }
  next();
}

function requireVerifiedTelegram(req, res, next) {
  if (!req._telegramVerified) {
    return res.status(403).json({ error: 'Telegram verification required for this action' });
  }
  next();
}

/**
 * Mini App endpoints. Read-only are open, write operations require Telegram verification.
 */
app.get("/api/bot/balance", async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: "valid telegramId required" });
    const user = await resolveTelegramUser(tid);
    if (!user) return res.status(404).json({ error: "Not linked" });

    const wallet = await Wallet.findOne({ userId: user._id, chain: "btcpc" });
    const node = await Node.findOne({ account: user._id });
    res.json({
      username: user.username,
      balance: wallet?.balance?.get("BTCPC") || 0,
      staked: node?.stake_amount || 0,
      address: wallet?.address || null
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.get("/api/bot/history", async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: "valid telegramId required" });
    const user = await resolveTelegramUser(tid);
    if (!user) return res.status(404).json({ error: "Not linked" });
    const wallet = await Wallet.findOne({ userId: user._id, chain: "btcpc" });
    if (!wallet) return res.json({ transactions: [] });

    const txs = await Transaction.find({
      $or: [{ from: wallet.address }, { to: wallet.address }, { from: user.username }, { to: user.username }]
    }).sort({ timestamp: -1 }).limit(10).lean();

    res.json({
      transactions: txs.map(tx => ({
        type: tx.type,
        amount: tx.amount,
        from: tx.from,
        to: tx.to,
        direction: (tx.to === wallet.address || tx.to === user.username) ? "in" : "out",
        timestamp: tx.timestamp
      }))
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.post("/api/bot/heartbeat", validateTelegramInitData, requireVerifiedTelegram, async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.body.telegramId);
    if (!tid) return res.status(400).json({ error: "valid telegramId required" });
    const user = await resolveTelegramUser(tid);
    if (!user) return res.status(404).json({ error: "Not linked" });
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordHeartbeat(user.username, epoch);
    res.json({ success: true, username: user.username, message: "Heartbeat recorded. Your dormancy clock is reset for 5 years." });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.post("/api/bot/link-chain", validateTelegramInitData, requireVerifiedTelegram, async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['telegramId', 'chain', 'address']);
    if (objErr) return res.status(400).json({ error: objErr });
    const tid = sanitizeTelegramId(req.body.telegramId);
    if (!tid) return res.status(400).json({ error: "valid telegramId required" });
    const user = await resolveTelegramUser(tid);
    if (!user) return res.status(404).json({ error: "Not linked" });
    const chain = sanitizeString(req.body.chain, 20);
    const address = sanitizeString(req.body.address, 200);
    if (!chain || !address) return res.status(400).json({ error: "chain and address required" });
    if (!validChain(chain)) return res.status(400).json({ error: "unsupported chain" });
    const chainLink = require("../services/chainLink");
    const challenge = chainLink.generateChallenge(user.username, chain, address);
    res.json({
      challengeId: challenge.challengeId,
      message: challenge.message,
      instructions: "Sign this exact message with your " + chain.toUpperCase() + " wallet, then submit the signature via /verify-chain",
      expiresIn: challenge.expiresIn
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.get("/api/bot/linked-addresses", async (req, res) => {
  try {
    const tid = sanitizeTelegramId(req.query.telegramId);
    if (!tid) return res.status(400).json({ error: "valid telegramId required" });
    const user = await resolveTelegramUser(tid);
    if (!user) return res.status(404).json({ error: "Not linked" });
    const chainLink = require("../services/chainLink");
    res.json({ username: user.username, linked: await chainLink.getLinkedAddresses(user.username) });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.post("/api/wallet/transfer", validateTelegramInitData, requireVerifiedTelegram, async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['telegramId', 'toAddress', 'amount', 'memo']);
    if (objErr) return res.status(400).json({ error: objErr });
    const tid = sanitizeTelegramId(req.body.telegramId);
    if (!tid) return res.status(400).json({ error: "valid telegramId required" });
    const user = await resolveTelegramUser(tid);
    if (!user) return res.status(404).json({ error: "Not linked" });
    const fromWallet = await Wallet.findOne({ userId: user._id, chain: "btcpc" });
    if (!fromWallet) return res.status(404).json({ error: "Wallet not found" });

    const toAddress = sanitizeString(req.body.toAddress, 200);
    const amount = sanitizeAmount(req.body.amount);
    if (!toAddress || !amount) return res.status(400).json({ error: "toAddress and positive amount required" });
    if (!validAddress(toAddress)) return res.status(400).json({ error: "invalid address format" });
    if (fromWallet.address === toAddress) return res.status(400).json({ error: "Cannot transfer to your own wallet" });

    const toWallet = await Wallet.findOne({ address: toAddress, chain: "btcpc" });
    if (!toWallet) return res.status(404).json({ error: "Recipient wallet not found" });
    const recipient = await User.findById(toWallet.userId);
    if (!recipient) return res.status(404).json({ error: "Recipient user not found" });

    const balance = fromWallet.balance.get("BTCPC") || 0;
    if (balance < amount) return res.status(400).json({ error: "Insufficient BTCPC balance" });

    const epoch = await ledger.getCurrentEpoch();
    const memo = sanitizeString(req.body.memo, 500) || null;
    const entry = await ledger.recordTransfer(user.username, recipient.username, amount, "BTCPC", null, epoch, memo);
    const txHash = blockStore.hashLedgerEntry(entry);
    res.json({ success: true, txHash, epoch });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

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
      stakingAgg,
      activeClocks,
      peerCount,
      recentDreams
    ] = await Promise.all([
      Epoch.find().sort({ epoch_number: -1 }).limit(15).lean(),
      Epoch.countDocuments(),
      Node.countDocuments(),
      LedgerEntry.find().sort({ timestamp: -1 }).limit(10).lean(),
      LedgerEntry.countDocuments(),
      StakingPool.aggregate([
        { $match: { status: "active" } },
        { $group: { _id: null, total: { $sum: "$staked_amount" } } }
      ]),
      LedgerEntry.countDocuments({ type: "NODE_REGISTER", "account_data.node_type": "clock" }),
      PeerRegistry.countDocuments({ last_seen: { $gte: new Date(Date.now() - 15 * 60 * 1000) } }),
      GenesisDream.find().sort({ block_number: -1 }).limit(6).lean()
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
      stateRoot: stateManager.getStateRoot(),
      activeClocks,
      peerCount,
      epochAgeMs: latestEpochs[0]?.started_at ? Date.now() - new Date(latestEpochs[0].started_at).getTime() : null,
      recentDreams
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
    const username = sanitizeString(req.params.username, 20);
    if (!username || !validAccountName(username)) return res.send(accountView({ user: null }));
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
    const pag = sanitizePagination(req.query.page, 25, 100);
    const page = pag.page;
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
 * GET /tx/:txHash — Transaction detail by hash (HTML for browsers, JSON for API)
 */
app.get("/tx/:txHash", async (req, res) => {
  try {
    var txHash = sanitizeString(req.params.txHash, 128);
    if (!txHash || !/^[a-fA-F0-9]+$/.test(txHash)) {
      if (req.accepts("html")) return res.send(txDetailView({ tx: null, txHash: txHash }));
      return res.status(400).json({ error: "invalid transaction hash" });
    }

    var txData = null;

    // Search mempool first
    if (mempool.hasTransaction(txHash)) {
      var mempoolTx = mempool.getTransactions().find(function (t) { return t.txHash === txHash; });
      if (mempoolTx) {
        txData = {
          status: "pending",
          txHash: txHash,
          type: mempoolTx.type,
          from: mempoolTx.from,
          to: mempoolTx.to,
          amount: mempoolTx.amount,
          token: mempoolTx.token || "BTCPC",
          timestamp: mempoolTx.timestamp,
          memo: mempoolTx.memo
        };
      }
    }

    // Search ledger entries by computing hashes (limited scan for recent entries)
    if (!txData) {
      var recentEntries = await LedgerEntry.find().sort({ timestamp: -1 }).limit(500).lean();
      for (var i = 0; i < recentEntries.length; i++) {
        var entryHash = blockStore.hashLedgerEntry(recentEntries[i]);
        if (entryHash === txHash) {
          txData = formatLedgerEntry(recentEntries[i]);
          txData.status = "confirmed";
          break;
        }
      }
    }

    if (!txData) {
      if (req.accepts("html")) return res.send(txDetailView({ tx: null, txHash: txHash }));
      return res.status(404).json({ error: "Transaction not found", txHash: txHash });
    }

    if (req.accepts("html")) {
      return res.send(txDetailView({ tx: txData }));
    }
    res.json(txData);
  } catch (err) {
    if (req.accepts("html")) return res.status(500).send("Internal server error");
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
    var uname = sanitizeString(req.params.username, 20);
    if (!uname || !validAccountName(uname)) return res.status(400).json({ error: "invalid account name" });
    var result = verifyAccountState(uname);
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
    var username = sanitizeString(req.params.username, 20);
    if (!username || !validAccountName(username)) return res.status(400).json({ error: "invalid account name" });
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

// ---------------------------------------------------------------------------
// Tokenomics, Blocks, Dashboard, TX Detail (HTML) routes
// ---------------------------------------------------------------------------

/**
 * GET /tokenomics — Tokenomics dashboard
 */
app.get("/tokenomics", async (req, res) => {
  try {
    const [
      epochCount,
      latestEpoch,
      stakingAgg,
      minedAgg,
      activeMiners,
      activeClocks,
      totalVerifiers
    ] = await Promise.all([
      Epoch.countDocuments(),
      Epoch.findOne().sort({ epoch_number: -1 }).lean(),
      StakingPool.aggregate([
        { $match: { status: "active" } },
        { $group: { _id: null, total: { $sum: "$staked_amount" } } }
      ]),
      LedgerEntry.aggregate([
        { $match: { type: "MINING_REWARD", token: "BTCPC" } },
        { $group: { _id: null, total: { $sum: "$amount" } } }
      ]),
      Node.countDocuments(),
      LedgerEntry.countDocuments({ type: "NODE_REGISTER", "account_data.node_type": "clock" }),
      LedgerEntry.countDocuments({ type: "NODE_REGISTER", "account_data.node_type": "verifier" })
    ]);

    var currentEpoch = latestEpoch ? latestEpoch.epoch_number : 0;
    var currentPeriod = getCurrentPeriod(currentEpoch);
    var periodTable = getPeriodTable();
    var totalMined = minedAgg.length ? minedAgg[0].total : 0;
    var totalStaked = stakingAgg.length ? stakingAgg[0].total : 0;

    var deployments = {};
    try { deployments = require("../../contracts/deployments.json"); } catch (_) {}

    res.send(tokenomicsView({
      totalSupply: TOTAL_SUPPLY,
      totalMined,
      currentEpoch,
      currentPeriod,
      periodTable,
      deployments,
      totalStaked,
      activeMiners,
      activeClocks,
      totalVerifiers
    }));
  } catch (err) {
    console.error("[btcpcscan] Tokenomics error:", err);
    res.status(500).send("Internal server error");
  }
});

/**
 * GET /blocks — Paginated block list
 */
app.get("/blocks", async (req, res) => {
  try {
    var pag = sanitizePagination(req.query.page, 25, 100);
    var page = pag.page;
    var perPage = 25;
    var skip = (page - 1) * perPage;

    var [epochs, total] = await Promise.all([
      Epoch.find().sort({ epoch_number: -1 }).skip(skip).limit(perPage).lean(),
      Epoch.countDocuments()
    ]);

    res.send(blocksView({ epochs, total, page, perPage }));
  } catch (err) {
    console.error("[btcpcscan] Blocks error:", err);
    res.status(500).send("Internal server error");
  }
});

/**
 * GET /dashboard — User dashboard (public info for any account)
 */
app.get("/dashboard", async (req, res) => {
  try {
    var accountName = sanitizeString(req.query.account, 20);
    if (!accountName || !validAccountName(accountName)) {
      return res.send(userDashboardView({ user: null }));
    }

    var user = await User.findOne({ username: accountName }).lean();
    if (!user) {
      return res.send(userDashboardView({ user: null }));
    }

    var [node, stake, ledgerEntries, miningRewards] = await Promise.all([
      Node.findOne({ account: user._id }).lean(),
      StakingPool.findOne({ account: user._id, status: { $in: ["active", "unstaking"] } }).lean(),
      LedgerEntry.find({
        $or: [{ from: accountName }, { to: accountName }]
      }).sort({ timestamp: -1 }).limit(20).lean(),
      LedgerEntry.find({ to: accountName, type: "MINING_REWARD" }).lean()
    ]);

    var balance = await ledger.getBalance(accountName, "BTCPC");
    var pendingDebit = mempool.getPendingDebit(accountName);
    var smtState = stateManager.getAccountState(accountName);

    // Try to get linked addresses
    var linkedAddresses = [];
    try {
      var chainLink = require("../services/chainLink");
      linkedAddresses = await chainLink.getLinkedAddresses(accountName);
    } catch (_) {}

    // Try to get pending claims
    var pendingClaims = [];
    try {
      pendingClaims = await LedgerEntry.find({
        to: accountName,
        type: "CROSS_CHAIN_CLAIM",
        status: "pending"
      }).lean();
    } catch (_) {}

    // Check for clock node registration
    var clockNode = null;
    try {
      var clockEntry = await LedgerEntry.findOne({
        to: accountName,
        type: "NODE_REGISTER",
        "account_data.node_type": "clock"
      }).lean();
      if (clockEntry) clockNode = clockEntry.account_data || clockEntry;
    } catch (_) {}

    // Check for delegation
    var delegation = null;
    try {
      var delegationEntry = await LedgerEntry.findOne({
        from: accountName,
        type: "DELEGATION"
      }).sort({ timestamp: -1 }).lean();
      if (delegationEntry) {
        delegation = {
          delegatee: delegationEntry.to,
          amount: delegationEntry.amount,
          status: "active"
        };
      }
    } catch (_) {}

    var recentTxs = ledgerEntries.map(formatLedgerEntry);

    res.send(userDashboardView({
      user,
      node,
      stake,
      balance,
      pendingDebit,
      availableBalance: balance - pendingDebit,
      miningRewards,
      recentTxs,
      linkedAddresses: Array.isArray(linkedAddresses) ? linkedAddresses : [],
      pendingClaims,
      delegation,
      clockNode,
      smtProof: smtState ? { root: smtState.root, state: smtState.state } : null
    }));
  } catch (err) {
    console.error("[btcpcscan] Dashboard error:", err);
    res.status(500).send("Internal server error");
  }
});

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
    if (typeof body.action !== 'string' && body.action !== undefined) return res.status(400).send("Invalid action");

    if (body.action === "pause") resourceManager.pause();
    else if (body.action === "resume") resourceManager.resume();
    else if (body.action === "full") resourceManager.setManualMode("full");
    else if (body.action === "reduced") resourceManager.setManualMode("reduced");
    else if (body.action === "auto") resourceManager.setManualMode(null);

    if (body.cpuCap) {
      var cpu = parseInt(body.cpuCap, 10);
      if (!isNaN(cpu) && cpu >= 0 && cpu <= 100) resourceManager.setCpuCap(cpu);
    }
    if (body.gpuCap) {
      var gpu = parseInt(body.gpuCap, 10);
      if (!isNaN(gpu) && gpu >= 0 && gpu <= 100) resourceManager.setGpuCap(gpu);
    }

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
