"use strict";
const speakeasy = require('speakeasy');
const QRCode = require('qrcode');
const crypto = require('crypto');
const User = require('../models/User');

const ISSUER = 'BTCPC';
const BACKUP_CODE_COUNT = 8;

/**
 * Generate a TOTP secret for a user.
 * Returns { secret (base32), otpauthUrl, qrDataUrl }.
 */
async function generateSecret(account) {
  const user = await User.findOne({ username: account });
  if (!user) throw new Error('Account not found');
  if (user.totpEnabled) throw new Error('TOTP already enabled');

  const secret = speakeasy.generateSecret({
    name: `${ISSUER}:${account}`,
    issuer: ISSUER,
    length: 20
  });

  // Store secret (not yet enabled — user must verify first)
  user.totpSecret = secret.base32;
  await user.save();

  const qrDataUrl = await QRCode.toDataURL(secret.otpauth_url);

  return {
    secret: secret.base32,
    otpauthUrl: secret.otpauth_url,
    qrDataUrl
  };
}

/**
 * Verify a 6-digit TOTP token against the user's stored secret.
 */
async function verifyToken(account, token) {
  const user = await User.findOne({ username: account });
  if (!user) throw new Error('Account not found');
  if (!user.totpSecret) throw new Error('TOTP not set up');

  return speakeasy.totp.verify({
    secret: user.totpSecret,
    encoding: 'base32',
    token: String(token),
    window: 1 // +-1 step (30s) tolerance
  });
}

/**
 * Enable TOTP after verifying the first token from the user's authenticator app.
 * Also generates backup codes.
 */
async function enableTOTP(account, token) {
  const user = await User.findOne({ username: account });
  if (!user) throw new Error('Account not found');
  if (user.totpEnabled) throw new Error('TOTP already enabled');
  if (!user.totpSecret) throw new Error('Call /api/totp/setup first');

  const valid = speakeasy.totp.verify({
    secret: user.totpSecret,
    encoding: 'base32',
    token: String(token),
    window: 1
  });

  if (!valid) throw new Error('Invalid TOTP code');

  const backupCodes = _generateBackupCodes();

  user.totpEnabled = true;
  user.totpBackupCodes = backupCodes;
  await user.save();

  return { enabled: true, backupCodes };
}

/**
 * Disable TOTP. Requires a valid token to prevent unauthorized disable.
 */
async function disableTOTP(account, token) {
  const user = await User.findOne({ username: account });
  if (!user) throw new Error('Account not found');
  if (!user.totpEnabled) throw new Error('TOTP not enabled');

  const valid = speakeasy.totp.verify({
    secret: user.totpSecret,
    encoding: 'base32',
    token: String(token),
    window: 1
  });

  if (!valid) throw new Error('Invalid TOTP code');

  user.totpEnabled = false;
  user.totpSecret = null;
  user.totpBackupCodes = [];
  await user.save();

  return { disabled: true };
}

/**
 * Regenerate backup codes (requires valid TOTP token).
 */
async function generateBackupCodes(account, token) {
  const user = await User.findOne({ username: account });
  if (!user) throw new Error('Account not found');
  if (!user.totpEnabled) throw new Error('TOTP not enabled');

  const valid = speakeasy.totp.verify({
    secret: user.totpSecret,
    encoding: 'base32',
    token: String(token),
    window: 1
  });

  if (!valid) throw new Error('Invalid TOTP code');

  const backupCodes = _generateBackupCodes();
  user.totpBackupCodes = backupCodes;
  await user.save();

  return { backupCodes };
}

/**
 * Verify and consume a backup code. Returns true if valid.
 */
async function verifyBackupCode(account, code) {
  const user = await User.findOne({ username: account });
  if (!user) throw new Error('Account not found');
  if (!user.totpEnabled) throw new Error('TOTP not enabled');

  const normalized = String(code).trim().toLowerCase();
  const idx = user.totpBackupCodes.indexOf(normalized);
  if (idx === -1) return false;

  // Consume the code — single use
  user.totpBackupCodes.splice(idx, 1);
  await user.save();
  return true;
}

/**
 * Middleware: require TOTP verification for sensitive endpoints.
 * If user has TOTP enabled, requires `totp_code` in request body.
 * If user has TOTP disabled, passes through.
 */
function requireTOTP(req, res, next) {
  const userId = req.user && req.user.id;
  if (!userId) return res.status(401).json({ error: 'Authentication required' });

  User.findById(userId).then(user => {
    if (!user) return res.status(404).json({ error: 'User not found' });
    if (!user.totpEnabled) return next(); // TOTP not enabled, skip

    const code = req.body.totp_code || req.headers['x-totp-code'];
    if (!code) {
      return res.status(403).json({
        error: '2FA required',
        message: 'This action requires a TOTP code. Provide totp_code in body or X-TOTP-Code header.'
      });
    }

    // Try TOTP first, then backup code
    const valid = speakeasy.totp.verify({
      secret: user.totpSecret,
      encoding: 'base32',
      token: String(code),
      window: 1
    });

    if (valid) return next();

    // Check backup codes
    const normalized = String(code).trim().toLowerCase();
    const idx = user.totpBackupCodes.indexOf(normalized);
    if (idx !== -1) {
      user.totpBackupCodes.splice(idx, 1);
      user.save().then(() => next());
      return;
    }

    return res.status(403).json({ error: 'Invalid TOTP code' });
  }).catch(err => {
    return res.status(500).json({ error: err.message });
  });
}

/** Generate N random backup codes */
function _generateBackupCodes() {
  const codes = [];
  for (let i = 0; i < BACKUP_CODE_COUNT; i++) {
    codes.push(crypto.randomBytes(4).toString('hex')); // 8-char hex codes
  }
  return codes;
}

module.exports = {
  generateSecret,
  verifyToken,
  enableTOTP,
  disableTOTP,
  generateBackupCodes,
  verifyBackupCode,
  requireTOTP
};
