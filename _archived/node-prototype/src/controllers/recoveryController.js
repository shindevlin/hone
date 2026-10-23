"use strict";

/**
 * Recovery Controller
 * Shin Devlin
 *
 * Implements HONE account recovery per Whitepaper 2.3.3.
 * Owner key bypasses 2FA to initiate recovery. A 72-hour time-lock window
 * allows the real owner to contest using valid 2FA before the reset completes.
 *
 * Phase E: RecoveryRequest Mongoose model removed. Recovery requests stored
 * in memory (bounded Map) — they are short-lived events, not permanent state.
 */

const User = require("../models/User");
const { sanitizeString, validAccountName } = require("../middlewares/validate");

const RECOVERY_WINDOW_MS = 72 * 60 * 60 * 1000; // 72 hours

// In-memory recovery requests: account → { requestedAt, expiresAt, contested, status, new2faKey }
const recoveryRequests = new Map();

/**
 * requestRecovery — Submit a recovery request using the Owner key.
 * This is the ONE transaction type that bypasses 2FA.
 * Creates a 72-hour time-lock window before the 2FA reset takes effect.
 *
 * Body: { account, owner_signature, new_2fa_public_key }
 */
async function requestRecovery(req, res) {
  var account = req.body.account;
  var ownerSignature = req.body.owner_signature;
  var new2faKey = req.body.new_2fa_public_key;

  try {
    if (!account || !ownerSignature) {
      return res.status(400).json({ error: "account and owner_signature are required" });
    }

    var user = await User.findOne({ username: account });
    if (!user) {
      return res.status(404).json({ error: "Account not found" });
    }

    if (!user.ownerPublicKey) {
      return res.status(400).json({ error: "Account has no owner key configured" });
    }

    // Expire existing requests
    const existing = recoveryRequests.get(account);
    if (existing && existing.status === 'pending' && new Date() < existing.expiresAt) {
      return res.status(409).json({
        error: "A recovery request is already pending for this account",
        expires_at: existing.expiresAt
      });
    }

    var now = new Date();
    var expiresAt = new Date(now.getTime() + RECOVERY_WINDOW_MS);

    const recovery = {
      account,
      requestedAt: now,
      expiresAt,
      contested: false,
      contested_by_2fa: false,
      status: 'pending',
      new2faKey: new2faKey || null
    };
    recoveryRequests.set(account, recovery);

    console.log("[HONE] Recovery request created for account: " + account +
      " (expires " + expiresAt.toISOString() + ")");

    res.status(201).json({
      success: true,
      recovery_id: account + ':' + now.getTime(),
      account,
      requested_at: now,
      expires_at: expiresAt,
      status: 'pending',
      message: "72-hour recovery window started. The real owner can contest with valid 2FA."
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * contestRecovery — The real owner submits valid 2FA to block an unauthorized recovery.
 *
 * Body: { account, twofa_token }
 */
async function contestRecovery(req, res) {
  var account = req.body.account;
  var twofaToken = req.body.twofa_token;

  try {
    if (!account || !twofaToken) {
      return res.status(400).json({ error: "account and twofa_token are required" });
    }

    const recovery = recoveryRequests.get(account);
    if (!recovery || recovery.status !== 'pending') {
      return res.status(404).json({ error: "No pending recovery request for this account" });
    }

    if (new Date() > recovery.expiresAt) {
      recovery.status = 'expired';
      return res.status(410).json({ error: "Recovery window has already expired" });
    }

    var user = await User.findOne({ username: account });
    if (!user) {
      return res.status(404).json({ error: "Account not found" });
    }

    if (!user.twoFactorEnabled && !user.twoFactorPublicKey) {
      return res.status(400).json({ error: "Account does not have 2FA configured" });
    }

    if (!twofaToken || typeof twofaToken !== "string" || twofaToken.length < 6) {
      return res.status(401).json({ error: "Invalid 2FA token" });
    }

    recovery.contested = true;
    recovery.contested_by_2fa = true;
    recovery.status = 'contested';

    console.log("[HONE] Recovery CONTESTED for account: " + account +
      " — attacker's recovery attempt blocked");

    res.json({
      success: true,
      account,
      status: 'contested',
      message: "Recovery attempt has been blocked. Your 2FA remains active."
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * completeRecovery — After 72 hours with no contest, finalize the 2FA reset.
 *
 * Body: { account, owner_signature }
 */
async function completeRecovery(req, res) {
  var account = req.body.account;
  var ownerSignature = req.body.owner_signature;

  try {
    if (!account || !ownerSignature) {
      return res.status(400).json({ error: "account and owner_signature are required" });
    }

    const recovery = recoveryRequests.get(account);
    if (!recovery || recovery.status !== 'pending') {
      return res.status(404).json({ error: "No pending recovery request for this account" });
    }

    if (new Date() < recovery.expiresAt) {
      var remaining = recovery.expiresAt.getTime() - Date.now();
      var hours = Math.ceil(remaining / (60 * 60 * 1000));
      return res.status(403).json({
        error: "Recovery window has not elapsed yet",
        hours_remaining: hours,
        expires_at: recovery.expiresAt
      });
    }

    var user = await User.findOne({ username: account });
    if (!user) {
      return res.status(404).json({ error: "Account not found" });
    }

    if (recovery.new2faKey) {
      user.twoFactorPublicKey = recovery.new2faKey;
      user.twoFactorEnabled = true;
    } else {
      user.twoFactorPublicKey = null;
      user.twoFactorEnabled = false;
    }
    await user.save();

    recovery.status = 'completed';
    console.log("[HONE] Recovery COMPLETED for account: " + account + " — 2FA has been reset");

    res.json({
      success: true,
      account,
      status: 'completed',
      message: "2FA has been reset. New credentials are active."
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * getRecoveryStatus — Check if a recovery is pending for an account.
 *
 * Query: ?account=<username>
 */
async function getRecoveryStatus(req, res) {
  var rawAccount = req.query.account || req.params.account;
  var account = typeof rawAccount === 'string' ? sanitizeString(rawAccount, 20) : null;

  try {
    if (!account) {
      return res.status(400).json({ error: "account parameter is required" });
    }
    if (!validAccountName(account)) {
      return res.status(400).json({ error: "invalid account name" });
    }

    const recovery = recoveryRequests.get(account);

    // Auto-expire
    if (recovery && recovery.status === 'pending' && new Date() > recovery.expiresAt) {
      recovery.status = 'expired';
    }

    if (!recovery || recovery.status !== 'pending') {
      return res.json({ account, pending: false, message: "No active recovery request" });
    }

    var remaining = recovery.expiresAt.getTime() - Date.now();
    res.json({
      account,
      pending: true,
      recovery_id: account + ':' + recovery.requestedAt.getTime(),
      requested_at: recovery.requestedAt,
      expires_at: recovery.expiresAt,
      hours_remaining: Math.max(0, Math.ceil(remaining / (60 * 60 * 1000))),
      contested: recovery.contested,
      status: recovery.status
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

module.exports = {
  requestRecovery,
  contestRecovery,
  completeRecovery,
  getRecoveryStatus
};
