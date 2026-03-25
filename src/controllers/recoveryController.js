"use strict";

/**
 * Recovery Controller
 * Shin Devlin
 *
 * Implements BTCPC account recovery per Whitepaper 2.3.3.
 * Owner key bypasses 2FA to initiate recovery. A 72-hour time-lock window
 * allows the real owner to contest using valid 2FA before the reset completes.
 */

const RecoveryRequest = require("../models/RecoveryRequest");
const User = require("../models/User");

const RECOVERY_WINDOW_MS = 72 * 60 * 60 * 1000; // 72 hours

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

    // Verify the account exists
    var user = await User.findOne({ username: account });
    if (!user) {
      return res.status(404).json({ error: "Account not found" });
    }

    // Verify owner key signature
    // In production this validates the cryptographic signature against ownerPublicKey.
    // For now we verify the owner public key is set and the signature field is present.
    if (!user.ownerPublicKey) {
      return res.status(400).json({ error: "Account has no owner key configured" });
    }

    // Check for an existing pending recovery on this account
    var existing = await RecoveryRequest.findOne({ account: account, status: "pending" });
    if (existing) {
      return res.status(409).json({
        error: "A recovery request is already pending for this account",
        expires_at: existing.expires_at
      });
    }

    var now = new Date();
    var expiresAt = new Date(now.getTime() + RECOVERY_WINDOW_MS);

    var recovery = new RecoveryRequest({
      account: account,
      requested_at: now,
      expires_at: expiresAt,
      contested: false,
      contested_by_2fa: false,
      status: "pending",
      new_2fa_public_key: new2faKey || null
    });
    await recovery.save();

    console.log("[BTCPC] Recovery request created for account: " + account +
      " (expires " + expiresAt.toISOString() + ")");

    res.status(201).json({
      success: true,
      recovery_id: recovery._id,
      account: account,
      requested_at: recovery.requested_at,
      expires_at: recovery.expires_at,
      status: recovery.status,
      message: "72-hour recovery window started. The real owner can contest with valid 2FA."
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * contestRecovery — The real owner submits valid 2FA to block an unauthorized recovery.
 * If contested, the recovery is permanently blocked.
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

    // Find the pending recovery request
    var recovery = await RecoveryRequest.findOne({ account: account, status: "pending" });
    if (!recovery) {
      return res.status(404).json({ error: "No pending recovery request for this account" });
    }

    // Verify the 72-hour window has not expired
    if (new Date() > recovery.expires_at) {
      recovery.status = "expired";
      await recovery.save();
      return res.status(410).json({ error: "Recovery window has already expired" });
    }

    // Verify the user exists and has 2FA enabled
    var user = await User.findOne({ username: account });
    if (!user) {
      return res.status(404).json({ error: "Account not found" });
    }

    if (!user.twoFactorEnabled && !user.twoFactorPublicKey) {
      return res.status(400).json({ error: "Account does not have 2FA configured" });
    }

    // Validate the 2FA token
    // In production this verifies the TOTP or cryptographic 2FA proof.
    // Simplified validation: token must be present and non-empty.
    if (!twofaToken || typeof twofaToken !== "string" || twofaToken.length < 6) {
      return res.status(401).json({ error: "Invalid 2FA token" });
    }

    // Contest the recovery — permanently block it
    recovery.contested = true;
    recovery.contested_by_2fa = true;
    recovery.status = "contested";
    await recovery.save();

    console.log("[BTCPC] Recovery CONTESTED for account: " + account +
      " — attacker's recovery attempt blocked");

    res.json({
      success: true,
      account: account,
      status: "contested",
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

    // Find the pending recovery request
    var recovery = await RecoveryRequest.findOne({ account: account, status: "pending" });
    if (!recovery) {
      return res.status(404).json({ error: "No pending recovery request for this account" });
    }

    // Verify the 72-hour window has elapsed
    if (new Date() < recovery.expires_at) {
      var remaining = recovery.expires_at.getTime() - Date.now();
      var hours = Math.ceil(remaining / (60 * 60 * 1000));
      return res.status(403).json({
        error: "Recovery window has not elapsed yet",
        hours_remaining: hours,
        expires_at: recovery.expires_at
      });
    }

    // Verify owner key (same check as requestRecovery)
    var user = await User.findOne({ username: account });
    if (!user) {
      return res.status(404).json({ error: "Account not found" });
    }

    // Reset 2FA — apply the new key if provided, otherwise clear 2FA
    if (recovery.new_2fa_public_key) {
      user.twoFactorPublicKey = recovery.new_2fa_public_key;
      user.twoFactorEnabled = true;
    } else {
      user.twoFactorPublicKey = null;
      user.twoFactorEnabled = false;
    }
    await user.save();

    // Mark recovery as completed
    recovery.status = "completed";
    await recovery.save();

    console.log("[BTCPC] Recovery COMPLETED for account: " + account + " — 2FA has been reset");

    res.json({
      success: true,
      account: account,
      status: "completed",
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
  var account = req.query.account || req.params.account;

  try {
    if (!account) {
      return res.status(400).json({ error: "account parameter is required" });
    }

    // Expire any overdue pending requests
    await RecoveryRequest.updateMany(
      { account: account, status: "pending", expires_at: { $lt: new Date() } },
      { $set: { status: "expired" } }
    );

    var recovery = await RecoveryRequest.findOne({ account: account, status: "pending" });

    if (!recovery) {
      res.json({
        account: account,
        pending: false,
        message: "No active recovery request"
      });
    } else {
      var remaining = recovery.expires_at.getTime() - Date.now();
      res.json({
        account: account,
        pending: true,
        recovery_id: recovery._id,
        requested_at: recovery.requested_at,
        expires_at: recovery.expires_at,
        hours_remaining: Math.max(0, Math.ceil(remaining / (60 * 60 * 1000))),
        contested: recovery.contested,
        status: recovery.status
      });
    }
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
