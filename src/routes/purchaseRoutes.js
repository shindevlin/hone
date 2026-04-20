"use strict";

/**
 * BTCPC Purchase Routes — stablecoin → BTCPC
 * Shin Devlin
 *
 * Endpoints:
 *   GET  /api/purchase/quote
 *   POST /api/purchase/submit
 *   GET  /api/purchase/:purchase_id/status
 *   POST /api/purchase/fulfill  (localhost only)
 */

const express = require("express");
const router = express.Router();
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const rateLimit = require("express-rate-limit");

const ORDERS_FILE = path.resolve(__dirname, "../../data/purchase-orders.jsonl");
const HISTORY_FILE = path.resolve(__dirname, "../../data/purchase-history.jsonl");

// ── Constants ────────────────────────────────────────────────────────────────

const ETH_CONTRACTS = {
  USDC: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
  USDT: "0xdAC17F958D2ee523a2206206994597C13D831ec7",
  DAI:  "0x6B175474E89094C44Da98b954EedeAC495271d0F",
};

const SOL_MINTS = {
  USDC: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
};

const TON_STABLES = {
  USDC: "native",
};

const SUPPORTED_CHAINS = ["ethereum", "solana", "ton"];
const SUPPORTED_TOKENS = {
  ethereum: ["USDC", "USDT", "DAI"],
  solana: ["USDC"],
  ton: ["USDC"],
};

function getPrice() {
  return parseFloat(process.env.BTCPC_PRICE_USD || "0.001");
}
function getMinUsd() {
  return parseFloat(process.env.BTCPC_PURCHASE_MIN_USD || "10");
}
function getMaxUsd() {
  return parseFloat(process.env.BTCPC_PURCHASE_MAX_USD || "10000");
}

function getPaymentAddress(chain) {
  if (chain === "ethereum") return process.env.BTCPC_PURCHASE_ETH_ADDRESS || null;
  if (chain === "solana")   return process.env.BTCPC_PURCHASE_SOL_ADDRESS || null;
  if (chain === "ton")      return process.env.BTCPC_PURCHASE_TON_ADDRESS || null;
  return null;
}

function getTokenContract(chain, token) {
  if (chain === "ethereum") return ETH_CONTRACTS[token] || null;
  if (chain === "solana")   return SOL_MINTS[token] || null;
  if (chain === "ton")      return TON_STABLES[token] || null;
  return null;
}

// ── File helpers ─────────────────────────────────────────────────────────────

function ensureDataDir() {
  const dir = path.resolve(__dirname, "../../data");
  if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
}

function appendJsonl(file, record) {
  ensureDataDir();
  fs.appendFileSync(file, JSON.stringify(record) + "\n");
}

function readJsonl(file) {
  if (!fs.existsSync(file)) return [];
  return fs.readFileSync(file, "utf8")
    .split("\n")
    .filter(Boolean)
    .map(l => { try { return JSON.parse(l); } catch { return null; } })
    .filter(Boolean);
}

function updateOrderStatus(purchaseId, updates) {
  ensureDataDir();
  if (!fs.existsSync(ORDERS_FILE)) return;
  const lines = fs.readFileSync(ORDERS_FILE, "utf8").split("\n").filter(Boolean);
  const updated = lines.map(l => {
    try {
      const rec = JSON.parse(l);
      if (rec.purchase_id === purchaseId) {
        return JSON.stringify(Object.assign({}, rec, updates));
      }
      return l;
    } catch { return l; }
  });
  fs.writeFileSync(ORDERS_FILE, updated.join("\n") + "\n");
}

function findOrder(purchaseId) {
  const orders = readJsonl(ORDERS_FILE);
  return orders.find(o => o.purchase_id === purchaseId) || null;
}

function isAlreadyProcessed(txHash) {
  const history = readJsonl(HISTORY_FILE);
  return history.some(h => h.tx_hash === txHash);
}

// ── Rate limiter for submit ──────────────────────────────────────────────────

const submitLimiter = rateLimit({
  windowMs: 60 * 60 * 1000,
  max: 10,
  standardHeaders: true,
  legacyHeaders: false,
  message: { error: "Too many purchase requests — try again in an hour" },
});

// ── Localhost-only middleware ─────────────────────────────────────────────────

function localhostOnly(req, res, next) {
  const addr = req.socket.remoteAddress;
  if (addr === "127.0.0.1" || addr === "::1" || addr === "::ffff:127.0.0.1") {
    return next();
  }
  return res.status(403).json({ error: "Forbidden — internal endpoint" });
}

// ── GET /api/purchase/quote ──────────────────────────────────────────────────

router.get("/quote", (req, res) => {
  const usdAmount = parseFloat(req.query.usd_amount) || 100;
  const chain = (req.query.chain || "ethereum").toLowerCase();
  const token = (req.query.token || "USDC").toUpperCase();

  if (!SUPPORTED_CHAINS.includes(chain)) {
    return res.status(400).json({ error: "Unsupported chain: " + chain });
  }
  if (!SUPPORTED_TOKENS[chain].includes(token)) {
    return res.status(400).json({ error: "Unsupported token " + token + " on " + chain });
  }

  const price = getPrice();
  const btcpcAmount = usdAmount / price;
  const paymentAddress = getPaymentAddress(chain);
  const tokenContract = getTokenContract(chain, token);

  res.json({
    btcpc_amount: btcpcAmount,
    price_per_btcpc_usd: price,
    payment_address: paymentAddress,
    token_contract: tokenContract,
    min_usd: getMinUsd(),
    max_usd: getMaxUsd(),
  });
});

// ── POST /api/purchase/submit ────────────────────────────────────────────────

router.post("/submit", submitLimiter, (req, res) => {
  const body = req.body || {};
  const { btcpc_account, btcpc_pubkeys, usd_amount, chain, token } = body;

  if (!btcpc_account || typeof btcpc_account !== "string") {
    return res.status(400).json({ error: "btcpc_account is required" });
  }
  if (!btcpc_pubkeys || typeof btcpc_pubkeys !== "object") {
    return res.status(400).json({ error: "btcpc_pubkeys is required (owner, active, posting, memo)" });
  }
  if (!usd_amount || isNaN(parseFloat(usd_amount))) {
    return res.status(400).json({ error: "usd_amount is required" });
  }

  const amount = parseFloat(usd_amount);
  const minUsd = getMinUsd();
  const maxUsd = getMaxUsd();
  if (amount < minUsd) {
    return res.status(400).json({ error: `Minimum purchase is $${minUsd} USD` });
  }
  if (amount > maxUsd) {
    return res.status(400).json({ error: `Maximum purchase is $${maxUsd} USD per transaction` });
  }

  const chainNorm = (chain || "ethereum").toLowerCase();
  const tokenNorm = (token || "USDC").toUpperCase();

  if (!SUPPORTED_CHAINS.includes(chainNorm)) {
    return res.status(400).json({ error: "Unsupported chain: " + chainNorm });
  }
  if (!SUPPORTED_TOKENS[chainNorm].includes(tokenNorm)) {
    return res.status(400).json({ error: "Unsupported token " + tokenNorm + " on " + chainNorm });
  }

  const paymentAddress = getPaymentAddress(chainNorm);
  if (!paymentAddress) {
    return res.status(503).json({ error: "Payment address not configured for " + chainNorm });
  }

  const purchaseId = crypto.randomBytes(8).toString("hex");
  const price = getPrice();
  const btcpcAmount = amount / price;

  // Chain-specific memo instructions
  let memoRequired = false;
  let memoValue = null;
  let instructions = "";

  if (chainNorm === "solana") {
    memoRequired = true;
    memoValue = purchaseId;
    instructions = `Send exactly ${amount} ${tokenNorm} to ${paymentAddress} on Solana. Include memo: "${purchaseId}". Use the SPL Token transfer with Memo program.`;
  } else if (chainNorm === "ton") {
    memoRequired = true;
    memoValue = purchaseId;
    instructions = `Send exactly ${amount} ${tokenNorm} to ${paymentAddress} on TON. Include comment: "${purchaseId}".`;
  } else {
    // Ethereum — no memo, match by amount+time
    memoRequired = false;
    memoValue = null;
    instructions = `Send exactly ${amount} ${tokenNorm} to ${paymentAddress} on Ethereum. Use the token contract at ${getTokenContract(chainNorm, tokenNorm)}. No memo needed — match is by amount and timing. Complete within 30 minutes.`;
  }

  const order = {
    purchase_id: purchaseId,
    btcpc_account,
    btcpc_pubkeys,
    usd_amount: amount,
    btcpc_amount: btcpcAmount,
    chain: chainNorm,
    token: tokenNorm,
    payment_address: paymentAddress,
    token_contract: getTokenContract(chainNorm, tokenNorm),
    status: "pending",
    created_at: Date.now(),
    expires_at: Date.now() + 30 * 60 * 1000, // 30 minutes
    memo_value: memoValue,
  };

  try {
    appendJsonl(ORDERS_FILE, order);
  } catch (err) {
    console.error("[purchase] Failed to write order:", err.message);
    return res.status(500).json({ error: "Failed to record purchase order" });
  }

  res.json({
    purchase_id: purchaseId,
    payment_address: paymentAddress,
    amount_usd: amount,
    btcpc_amount: btcpcAmount,
    token: tokenNorm,
    chain: chainNorm,
    memo_required: memoRequired,
    memo_value: memoValue,
    instructions,
    expires_at: order.expires_at,
  });
});

// ── GET /api/purchase/:purchase_id/status ────────────────────────────────────

router.get("/:purchase_id/status", (req, res) => {
  const { purchase_id } = req.params;
  const order = findOrder(purchase_id);
  if (!order) {
    return res.status(404).json({ error: "Purchase order not found" });
  }

  // Check history for fulfillment
  const history = readJsonl(HISTORY_FILE);
  const fulfilled = history.find(h => h.purchase_id === purchase_id);
  if (fulfilled) {
    return res.json({
      status: "fulfilled",
      purchase_id,
      tx_hash: fulfilled.tx_hash,
      btcpc_amount: fulfilled.btcpc_amount,
      fulfilled_epoch: fulfilled.fulfilled_epoch,
      fulfilled_at: fulfilled.fulfilled_at,
    });
  }

  // Check expiry
  if (order.expires_at && Date.now() > order.expires_at && order.status === "pending") {
    return res.json({ status: "expired", purchase_id });
  }

  return res.json({
    status: order.status || "pending",
    purchase_id,
    chain: order.chain,
    token: order.token,
    usd_amount: order.usd_amount,
    btcpc_amount: order.btcpc_amount,
    created_at: order.created_at,
    expires_at: order.expires_at,
  });
});

// ── POST /api/purchase/fulfill  (internal / localhost only) ──────────────────

router.post("/fulfill", localhostOnly, async (req, res) => {
  const body = req.body || {};
  const { purchase_id, tx_hash, confirmed_usd, chain } = body;

  if (!purchase_id || !tx_hash) {
    return res.status(400).json({ error: "purchase_id and tx_hash are required" });
  }

  // Deduplication
  if (isAlreadyProcessed(tx_hash)) {
    return res.status(409).json({ error: "Transaction already processed", tx_hash });
  }

  const order = findOrder(purchase_id);
  if (!order) {
    return res.status(404).json({ error: "Purchase order not found: " + purchase_id });
  }
  if (order.status === "fulfilled") {
    return res.status(409).json({ error: "Order already fulfilled" });
  }

  const usdAmount = confirmed_usd || order.usd_amount;
  const price = getPrice();
  const btcpcAmount = usdAmount / price;
  const fulfillerAccount = process.env.BTCPC_FULFILLER_ACCOUNT || "shindevlin";

  try {
    const ledger = require("../services/ledger");
    const stateStore = require("../chain/stateStore");
    const epoch = stateStore.getChainHeight() || 0;

    // Create account if needed
    const stateBackend = require("../chain/stateBackend");
    const accountExists = stateBackend.getAccount(order.btcpc_account);
    if (!accountExists) {
      await ledger.recordAccountCreate(
        order.btcpc_account,
        order.btcpc_pubkeys || {},
        {},
        epoch
      );
    }

    // Transfer BTCPC to buyer
    await ledger.recordTransfer(
      fulfillerAccount,
      order.btcpc_account,
      btcpcAmount,
      "BTCPC",
      null,
      epoch,
      `Purchase: ${usdAmount} USD via ${chain || order.chain}`
    );

    const histRecord = {
      purchase_id,
      tx_hash,
      chain: chain || order.chain,
      token: order.token,
      confirmed_usd: usdAmount,
      btcpc_amount: btcpcAmount,
      btcpc_account: order.btcpc_account,
      fulfilled_epoch: epoch,
      fulfilled_at: Date.now(),
    };

    appendJsonl(HISTORY_FILE, histRecord);
    updateOrderStatus(purchase_id, { status: "fulfilled", tx_hash, fulfilled_at: Date.now() });

    console.log(`[purchase] Fulfilled ${purchase_id}: ${btcpcAmount} BTCPC → ${order.btcpc_account} (tx: ${tx_hash})`);

    return res.json({
      ok: true,
      purchase_id,
      btcpc_amount: btcpcAmount,
      btcpc_account: order.btcpc_account,
      fulfilled_epoch: epoch,
    });
  } catch (err) {
    console.error("[purchase] Fulfillment error:", err.message);
    return res.status(500).json({ error: "Fulfillment failed: " + err.message });
  }
});

module.exports = router;
