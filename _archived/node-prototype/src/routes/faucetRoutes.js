"use strict";

/**
 * HONE Faucet
 * Shin Devlin
 *
 * Faucet issues DELEGATED tokens, not gifted HONE.
 * Delegated tokens can only be spent on direct network services — AI,
 * sensor data, storage, and other on-chain service requests. They are not
 * transferable, sellable, stakeable, or bridgeable.
 *
 * This prevents faucet farming (claim → sell → repeat).
 * Real earning happens via mining, sensors, storage, and data purchases.
 *
 * On-chain: recordDelegate(FAUCET_ACCOUNT → recipient, amount, 'faucet')
 * Network service billing deducts from delegated_balance before wallet balance.
 * TRANSFER, STAKE, and bridge flows must not treat delegated balance as owned.
 */

const express = require("express");
const router = express.Router();
const ledger = require("../services/ledger");
const stateStore = require("../chain/stateStore");
const { shouldStartBackgroundTimers } = require("../services/backgroundTimers");

const FAUCET_ACCOUNT = "hone_faucet";      // holds the faucet reserve
const DELEGATION_EPOCHS = 720;              // ~6 hours at 30s epochs — expires automatically

// Scales with network maturity — same service utility regardless of epoch
function getFaucetAmount() {
  const epoch = stateStore.getChainHeight();
  if (epoch <= 1000) return 1;      // bootstrap: ~1000 small queries
  if (epoch <= 10000) return 0.1;   // growth: ~100 small queries
  return 0.01;                       // mature: ~10 small queries
}

// Anti-spam: per-IP and per-account claim window
const claimsByIP = new Map();       // ip → { count, lastClaim }
const claimedAccounts = new Map();  // account → lastClaimEpoch
const MAX_CLAIMS_PER_IP = 3;
const CLAIM_WINDOW_MS = 24 * 60 * 60 * 1000;

function checkIPLimit(ip) {
  const now = Date.now();
  const record = claimsByIP.get(ip);
  if (!record || (now - record.lastClaim) > CLAIM_WINDOW_MS) {
    claimsByIP.set(ip, { count: 1, lastClaim: now });
    return true;
  }
  if (record.count >= MAX_CLAIMS_PER_IP) return false;
  record.count++;
  record.lastClaim = now;
  return true;
}

if (shouldStartBackgroundTimers()) {
  setInterval(() => {
    const cutoff = Date.now() - CLAIM_WINDOW_MS;
    for (const [ip, rec] of claimsByIP) {
      if (rec.lastClaim < cutoff) claimsByIP.delete(ip);
    }
  }, 60 * 60 * 1000);
}

/**
 * POST /api/faucet/claim
 * Issues delegated HONE to a zero-balance account.
 * Tokens are spendable on direct network services only — not transferable.
 * Body: { account: "username" }
 */
router.post("/claim", async (req, res) => {
  try {
    const account = req.body && req.body.account;
    if (!account || typeof account !== "string") {
      return res.status(400).json({ error: "account required" });
    }

    // IP rate limit
    const ip = req.ip || req.connection.remoteAddress || "unknown";
    if (!checkIPLimit(ip)) {
      return res.status(429).json({
        error: "Too many faucet claims from this IP. Max 3 per day. Earn HONE by mining, running sensors, or storing data.",
      });
    }

    // Account must exist on chain
    const acc = stateStore.getAccount ? stateStore.getAccount(account) : null;
    if (!acc) {
      return res.status(404).json({ error: "Account not found on chain. Register first." });
    }

    // Account age gate: must be at least 120 epochs (~1 hour) old to prevent mass-wallet farming
    const MIN_ACCOUNT_AGE_EPOCHS = 120;
    const ageEpoch = stateStore.getChainHeight();
    const accountEpoch = acc.epoch || 0;
    if ((ageEpoch - accountEpoch) < MIN_ACCOUNT_AGE_EPOCHS) {
      const epochsLeft = MIN_ACCOUNT_AGE_EPOCHS - (ageEpoch - accountEpoch);
      return res.status(429).json({
        error: "Account too new. Wait " + Math.ceil(epochsLeft * 30 / 60) + " more minutes before claiming.",
        epochs_remaining: epochsLeft,
      });
    }

    // Only claim if wallet balance is zero (delegated balance excluded — can re-claim when used up)
    const walletBalance = stateStore.getBalance(account, "HONE");
    if (walletBalance > 0) {
      return res.status(400).json({
        error: "You already have " + walletBalance.toFixed(4) + " HONE. Use your tokens before claiming more.",
        balance: walletBalance,
      });
    }

    // Block re-claim while delegated balance is still active
    const currentDelegated = stateStore.getDelegatedBalance
      ? stateStore.getDelegatedBalance(account, "HONE")
      : 0;
    if (currentDelegated > 0) {
      return res.status(429).json({
        error: "You still have " + currentDelegated.toFixed(4) + " delegated HONE. Use it for AI, sensor data, storage, or other network services before claiming more.",
        delegated_balance: currentDelegated,
      });
    }

    // Per-account claim cooldown — must have used up previous delegation
    const currentEpoch = stateStore.getChainHeight();
    const lastClaim = claimedAccounts.get(account);
    if (lastClaim && (currentEpoch - lastClaim) < DELEGATION_EPOCHS) {
      const epochsRemaining = DELEGATION_EPOCHS - (currentEpoch - lastClaim);
      const minutesRemaining = Math.ceil(epochsRemaining * 30 / 60);
      return res.status(429).json({
        error: "Faucet delegation active — " + minutesRemaining + " minutes remaining. Use your delegated tokens on direct network services.",
        epochs_remaining: epochsRemaining,
      });
    }

    // Faucet reserve must have funds
    const faucetBalance = stateStore.getBalance(FAUCET_ACCOUNT, "HONE");
    const amount = getFaucetAmount();
    if (faucetBalance < amount) {
      return res.status(503).json({ error: "Faucet reserve is empty. Check back later." });
    }

    // Issue as DELEGATE (not TRANSFER) — network-use only, not wallet balance.
    await ledger.recordDelegate(FAUCET_ACCOUNT, account, amount, "faucet", currentEpoch);

    claimedAccounts.set(account, currentEpoch);

    const delegatedBalance = stateStore.getDelegatedBalance
      ? stateStore.getDelegatedBalance(account, "HONE")
      : amount;

    res.json({
      success: true,
      type: "delegation",
      amount,
      delegated_balance: delegatedBalance,
      expires_epochs: DELEGATION_EPOCHS,
      expires_minutes: Math.ceil(DELEGATION_EPOCHS * 30 / 60),
      message: `Welcome to HONE — ${amount} HONE delegated for direct network services. Use it for AI, sensor data, storage, or other on-chain service requests. Earn owned HONE by mining, running sensors, or storing data.`,
      note: "Delegated tokens are network-use only. They cannot be transferred, sold, staked, or bridged.",
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/faucet/status
 * Public — faucet reserve balance and current claim amount.
 */
router.get("/status", (req, res) => {
  const faucetBalance = stateStore.getBalance(FAUCET_ACCOUNT, "HONE");
  res.json({
    faucet_account: FAUCET_ACCOUNT,
    reserve_balance: faucetBalance,
    claim_amount: getFaucetAmount(),
    claim_type: "delegation",
    delegation_epochs: DELEGATION_EPOCHS,
    delegation_hours: Math.ceil(DELEGATION_EPOCHS * 30 / 3600),
    note: "Faucet issues delegated tokens — spendable on direct network services only.",
  });
});

module.exports = router;
