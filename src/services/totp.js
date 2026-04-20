"use strict";
const speakeasy = require('speakeasy');
const QRCode = require('qrcode');
const crypto = require('crypto');
const User = require('../models/User');

const ISSUER = 'BTCPC';
const BACKUP_CODE_COUNT = 8;

// D.5-delta: secretStore-first, Mongo fallback (same pattern as authController).
let _secretStoreLoaded = false;
async function getSecretStore() {
  const secretStore = require('./secretStore');
  if (!_secretStoreLoaded) {
    try {
      await secretStore.load();
      _secretStoreLoaded = true;
    } catch (err) {
      console.warn('[totp] secretStore load failed: ' + err.message);
    }
  }
  return secretStore;
}

// Resolve a user record from secretStore first, then Mongo.
// Returns { source: 'secretStore'|'mongo', record } or throws.
async function _findUser(account) {
  const ss = await getSecretStore();
  if (ss && typeof ss.getUser === 'function') {
    const rec = ss.getUser(account);
    if (rec) return { source: 'secretStore', record: rec };
  }
  // Mongo fallback
  const user = await User.findOne({ username: account });
  if (!user) throw new Error('Account not found');
  return { source: 'mongo', record: user };
}

// Write TOTP fields to both stores for backwards compat.
// secretStore fields are snake_case; Mongo fields are camelCase.
async function _saveTotpFields(account, fields, mongoUser) {
  const ss = await getSecretStore();
  if (ss && typeof ss.updateUser === 'function') {
    try {
      const patch = {};
      if (Object.prototype.hasOwnProperty.call(fields, 'totp_secret')) patch.totp_secret = fields.totp_secret;
      if (Object.prototype.hasOwnProperty.call(fields, 'totp_enabled')) patch.totp_enabled = fields.totp_enabled;
      if (Object.prototype.hasOwnProperty.call(fields, 'totp_backup_codes')) patch.totp_backup_codes = fields.totp_backup_codes;
      if (Object.keys(patch).length > 0) await ss.updateUser(account, patch);
    } catch (err) {
      console.warn('[totp] secretStore update failed for ' + account + ': ' + err.message);
    }
  }
  // Mirror to Mongo if we have the user object
  if (mongoUser) {
    if (Object.prototype.hasOwnProperty.call(fields, 'totp_secret')) mongoUser.totpSecret = fields.totp_secret;
    if (Object.prototype.hasOwnProperty.call(fields, 'totp_enabled')) mongoUser.totpEnabled = fields.totp_enabled;
    if (Object.prototype.hasOwnProperty.call(fields, 'totp_backup_codes')) mongoUser.totpBackupCodes = fields.totp_backup_codes;
    try {
      await mongoUser.save();
    } catch (_) {}
  }
}

/**
 * Generate a TOTP secret for a user.
 * Returns { secret (base32), otpauthUrl, qrDataUrl }.
 */
async function generateSecret(account) {
  const { source, record } = await _findUser(account);

  const totpEnabled = source === 'secretStore' ? record.totp_enabled : record.totpEnabled;
  if (totpEnabled) throw new Error('TOTP already enabled');

  const secret = speakeasy.generateSecret({
    name: `${ISSUER}:${account}`,
    issuer: ISSUER,
    length: 20
  });

  const mongoUser = source === 'mongo' ? record : null;
  await _saveTotpFields(account, { totp_secret: secret.base32 }, mongoUser);

  const qrDataUrl = await QRCode.toDataURL(secret.otpauth_url);

  // TOTP secret is sensitive — never logged, returned ONLY on initial setup
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
  const { source, record } = await _findUser(account);
  const totpSecret = source === 'secretStore' ? record.totp_secret : record.totpSecret;
  if (!totpSecret) throw new Error('TOTP not set up');

  return speakeasy.totp.verify({
    secret: totpSecret,
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
  const { source, record } = await _findUser(account);
  const totpEnabled = source === 'secretStore' ? record.totp_enabled : record.totpEnabled;
  const totpSecret = source === 'secretStore' ? record.totp_secret : record.totpSecret;

  if (totpEnabled) throw new Error('TOTP already enabled');
  if (!totpSecret) throw new Error('Call /api/totp/setup first');

  const valid = speakeasy.totp.verify({
    secret: totpSecret,
    encoding: 'base32',
    token: String(token),
    window: 1
  });

  if (!valid) throw new Error('Invalid TOTP code');

  const backupCodes = _generateBackupCodes();
  const mongoUser = source === 'mongo' ? record : null;
  await _saveTotpFields(account, { totp_enabled: true, totp_backup_codes: backupCodes }, mongoUser);

  return { enabled: true, backupCodes };
}

/**
 * Disable TOTP. Requires a valid token to prevent unauthorized disable.
 */
async function disableTOTP(account, token) {
  const { source, record } = await _findUser(account);
  const totpEnabled = source === 'secretStore' ? record.totp_enabled : record.totpEnabled;
  const totpSecret = source === 'secretStore' ? record.totp_secret : record.totpSecret;

  if (!totpEnabled) throw new Error('TOTP not enabled');

  const valid = speakeasy.totp.verify({
    secret: totpSecret,
    encoding: 'base32',
    token: String(token),
    window: 1
  });

  if (!valid) throw new Error('Invalid TOTP code');

  const mongoUser = source === 'mongo' ? record : null;
  await _saveTotpFields(account, { totp_enabled: false, totp_secret: null, totp_backup_codes: [] }, mongoUser);

  return { disabled: true };
}

/**
 * Regenerate backup codes (requires valid TOTP token).
 */
async function generateBackupCodes(account, token) {
  const { source, record } = await _findUser(account);
  const totpEnabled = source === 'secretStore' ? record.totp_enabled : record.totpEnabled;
  const totpSecret = source === 'secretStore' ? record.totp_secret : record.totpSecret;

  if (!totpEnabled) throw new Error('TOTP not enabled');

  const valid = speakeasy.totp.verify({
    secret: totpSecret,
    encoding: 'base32',
    token: String(token),
    window: 1
  });

  if (!valid) throw new Error('Invalid TOTP code');

  const backupCodes = _generateBackupCodes();
  const mongoUser = source === 'mongo' ? record : null;
  await _saveTotpFields(account, { totp_backup_codes: backupCodes }, mongoUser);

  return { backupCodes };
}

/**
 * Verify and consume a backup code. Returns true if valid.
 */
async function verifyBackupCode(account, code) {
  const { source, record } = await _findUser(account);
  const totpEnabled = source === 'secretStore' ? record.totp_enabled : record.totpEnabled;
  if (!totpEnabled) throw new Error('TOTP not enabled');

  const backupCodes = source === 'secretStore' ? record.totp_backup_codes : record.totpBackupCodes;
  const normalized = String(code).trim().toLowerCase();
  const idx = (backupCodes || []).indexOf(normalized);
  if (idx === -1) return false;

  const newCodes = [...backupCodes];
  newCodes.splice(idx, 1);
  const mongoUser = source === 'mongo' ? record : null;
  await _saveTotpFields(account, { totp_backup_codes: newCodes }, mongoUser);
  return true;
}

/**
 * Middleware: require TOTP verification for sensitive endpoints.
 * If user has TOTP enabled, requires `totp_code` in request body.
 * If user has TOTP disabled, passes through.
 */
function requireTOTP(req, res, next) {
  const authUser = req.user;
  if (!authUser || (!authUser.id && !authUser.username)) {
    return res.status(401).json({ error: 'Authentication required' });
  }

  const { resolveUserFromAuth, loadSecretStore } = require('./userResolver');

  resolveUserFromAuth(authUser).then(async user => {
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
    const backupCodes = user.totpBackupCodes || [];
    const idx = backupCodes.indexOf(normalized);
    if (idx !== -1) {
      backupCodes.splice(idx, 1);
      if (user.source === 'secretStore') {
        const ss = await loadSecretStore();
        await ss.updateUser(user.username, { totp_backup_codes: backupCodes });
        return next();
      }
      user.record.totpBackupCodes = backupCodes;
      user.record.save().then(() => next());
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
