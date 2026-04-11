"use strict";
const User = require('../models/User');
const jwt = require('jsonwebtoken');
const crypto = require('crypto');
const bcrypt = require('bcryptjs');
const { createAccount } = require('../wallet/accountManager');
const { rejectObjectInputs, validAccountName, blockedAccountNameReason, sanitizeString, sanitizeTelegramId } = require('../middlewares/validate');

// D.5-gamma: secretStore is the local-first auth source. Mongo is a
// fallback for legacy installs and for accounts created before D.5.
// On a successful Mongo fallback login we lazily migrate the user into
// secretStore so the next login is zero-Mongo.
let _secretStoreLoaded = false;
async function getSecretStore() {
  const secretStore = require('../services/secretStore');
  if (!_secretStoreLoaded) {
    try {
      await secretStore.load();
      _secretStoreLoaded = true;
    } catch (err) {
      console.warn('[auth] secretStore load failed: ' + err.message);
    }
  }
  return secretStore;
}

function isBcryptHash(value) {
  return typeof value === 'string' && value.startsWith('$2');
}

function matchesLegacySha256(password, storedHash) {
  if (!password || !storedHash || isBcryptHash(storedHash)) return false;
  const sha256 = crypto.createHash('sha256').update(password).digest('hex');
  return sha256 === storedHash;
}

async function verifyAndUpgradePassword(user, password) {
  if (!user || !password) return false;

  if (isBcryptHash(user.password)) {
    return bcrypt.compareSync(password, user.password);
  }

  if (!matchesLegacySha256(password, user.password)) {
    return false;
  }

  user.password = bcrypt.hashSync(password, 10);
  await user.save();
  return true;
}

// Try secretStore first. Returns a user-like object with .username,
// .id, .password_hash, .is_active, .email, or null.
async function lookupUserSecretStore(username) {
  try {
    const ss = await getSecretStore();
    if (!ss || typeof ss.getUser !== 'function') return null;
    const rec = ss.getUser(username);
    if (!rec) return null;
    return {
      source: 'secretStore',
      id: rec.user_id,
      username: rec.username || username,
      email: rec.email,
      password_hash: rec.password_hash,
      is_active: rec.is_active !== false,
      two_factor_enabled: !!rec.two_factor_enabled,
      totp_enabled: !!rec.totp_enabled,
    };
  } catch (_) {
    return null;
  }
}

// Verify against a secretStore record. Returns true/false.
async function verifySecretStorePassword(username, plaintext) {
  try {
    const ss = await getSecretStore();
    if (!ss || typeof ss.verifyPassword !== 'function') return false;
    return await ss.verifyPassword(username, plaintext);
  } catch (_) {
    return false;
  }
}

// Lazily copy a Mongo user into secretStore so subsequent logins are
// zero-Mongo. Called after a successful Mongo fallback login.
async function migrateUserToSecretStore(mongoUser) {
  try {
    const ss = await getSecretStore();
    if (!ss || typeof ss.createUser !== 'function') return;
    if (ss.hasUser && ss.hasUser(mongoUser.username)) return; // already migrated
    await ss.createUser(mongoUser.username, {
      password_hash: mongoUser.password, // already bcrypt after verifyAndUpgradePassword
      email: mongoUser.email,
      telegram_id: mongoUser.telegramId,
      telegram_username: mongoUser.telegramUsername,
      two_factor_enabled: mongoUser.twoFactorEnabled,
      totp_secret: mongoUser.totpSecret,
      totp_enabled: mongoUser.totpEnabled,
      totp_backup_codes: mongoUser.totpBackupCodes,
      auth_profile: mongoUser.authProfile,
      owner_public_key: mongoUser.ownerPublicKey,
      active_public_key: mongoUser.activePublicKey,
      posting_public_key: mongoUser.postingPublicKey,
      memo_public_key: mongoUser.memoPublicKey,
      two_factor_public_key: mongoUser.twoFactorPublicKey,
    });
    console.log('[auth] migrated ' + mongoUser.username + ' → secretStore');
  } catch (err) {
    console.warn('[auth] secretStore migration failed for ' + mongoUser.username + ': ' + err.message);
  }
}

/**
 * Register New User
 * Creates BTCPC account with BIP-39 mnemonic and wallets on all 7 chains.
 * Mnemonic is shown ONCE — user must save it. We never store it.
 */
async function registerUser(req, res) {
  try {
    const objErr = rejectObjectInputs(req.body, ['username', 'password']);
    if (objErr) return res.status(400).json({ error: objErr });
    const username = sanitizeString(req.body.username, 20);
    const password = req.body.password;
    if (!username || !password) return res.status(400).json({ error: 'username and password required' });
    if (!validAccountName(username)) return res.status(400).json({ error: 'Username must be 3-20 chars, lowercase letters/numbers/.-_ only' });
    const blockedReason = blockedAccountNameReason(username);
    if (blockedReason) return res.status(400).json({ error: blockedReason });
    if (typeof password !== 'string' || password.length < 8 || password.length > 200) {
      return res.status(400).json({ error: 'password must be 8-200 characters' });
    }
    const result = await createAccount(username, null, password);

    res.status(201).json({
      success: true,
      username: result.username,
      mnemonic: result.mnemonic,
      wallets: result.chainWallets,
      publicKeys: result.publicKeys,
      warning: "SAVE YOUR MNEMONIC. This is the only time it will be shown."
    });
  } catch (err) {
    res.status(err.message.includes('already exists') ? 400 : 500).json({ error: err.message });
  }
}

/**
 * Login User — D.5-gamma: secretStore-first, Mongo fallback with
 * lazy migration. On a fresh machine (no Mongo) the secretStore
 * path works on its own. On existing installs the Mongo fallback
 * handles legacy users and migrates them into secretStore so the
 * next login skips Mongo entirely.
 */
async function loginUser(req, res) {
  try {
    const objErr = rejectObjectInputs(req.body, ['email', 'username', 'password']);
    if (objErr) return res.status(400).json({ error: objErr });
    const email = sanitizeString(req.body.email, 100);
    const username = sanitizeString(req.body.username, 20);
    const password = req.body.password;
    const identifier = (email || username || '').trim().toLowerCase();

    if (!identifier || !password) {
      return res.status(400).json({ error: 'username/email and password are required' });
    }
    if (typeof password !== 'string' || password.length > 200) {
      return res.status(400).json({ error: 'invalid password' });
    }

    // ── Try secretStore first ──
    const ssUser = await lookupUserSecretStore(identifier);
    if (ssUser && ssUser.is_active && ssUser.password_hash) {
      const ok = await verifySecretStorePassword(identifier, password);
      if (ok) {
        const token = jwt.sign(
          { id: ssUser.id, username: ssUser.username, src: 'ss' },
          process.env.JWT_SECRET,
          { expiresIn: process.env.JWT_EXPIRES_IN || '7d' }
        );
        return res.json({
          success: true,
          token,
          user: { id: ssUser.id, username: ssUser.username, email: ssUser.email },
        });
      }
      // secretStore has the user but password mismatch — DON'T fall
      // through to Mongo (would be a credential-oracle). Return fail.
      return res.status(401).json({ error: 'Invalid credentials' });
    }

    // ── Mongo fallback (legacy installs) ──
    let mongoUser = null;
    try {
      mongoUser = await User.findOne({
        $or: [{ email: identifier }, { username: identifier }],
      });
    } catch (err) {
      // Mongo unavailable — if we got here, secretStore also didn't
      // know the user, so this is a real "invalid credentials".
      return res.status(401).json({ error: 'Invalid credentials' });
    }

    if (!mongoUser || !mongoUser.isActive) {
      return res.status(401).json({ error: 'Invalid credentials' });
    }

    const isMatch = await verifyAndUpgradePassword(mongoUser, password);
    if (!isMatch) {
      return res.status(401).json({ error: 'Invalid credentials' });
    }

    // Lazy migration into secretStore for next login
    await migrateUserToSecretStore(mongoUser);

    const token = jwt.sign(
      { id: mongoUser._id, username: mongoUser.username, src: 'mongo' },
      process.env.JWT_SECRET,
      { expiresIn: process.env.JWT_EXPIRES_IN || '7d' }
    );

    res.json({
      success: true,
      token,
      user: { id: mongoUser._id, username: mongoUser.username, email: mongoUser.email },
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Link Telegram Account
 */
async function linkTelegram(req, res) {
  const userId = req.user.id;

  try {
    const objErr = rejectObjectInputs(req.body, ['telegramId', 'telegramUsername']);
    if (objErr) return res.status(400).json({ error: objErr });
    const telegramId = sanitizeTelegramId(req.body.telegramId);
    const telegramUsername = sanitizeString(req.body.telegramUsername, 40);
    if (!telegramId) return res.status(400).json({ error: 'valid telegramId required' });

    const user = await User.findById(userId);
    if (!user) {
      return res.status(404).json({ error: 'User not found' });
    }

    if (user.telegramId) {
      return res.status(400).json({ error: 'Telegram account already linked' });
    }

    const existingLink = await User.findOne({ telegramId: String(telegramId) });
    if (existingLink) {
      return res.status(400).json({ error: 'This Telegram account is already linked to another user' });
    }

    // DISABLED: Direct linking without Telegram verification is an identity hijack vector.
    // Use /link <username> in the @btcpcbot Telegram bot instead.
    return res.status(403).json({
      error: 'Direct Telegram linking disabled. Use /link <username> in @btcpcbot instead.',
      help: 'The bot verifies your posting key signature before linking.'
    });

    res.json({ success: true, message: 'Telegram account linked. For security, use the bot /link command instead.' });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Enable 2FA for User
 */
async function enable2FA(req, res) {
  const userId = req.user.id;
  try {
    if (typeof req.body.token === 'object') return res.status(400).json({ error: 'token must be a string' });
    const token = sanitizeString(req.body.token, 20);
    if (!token || token !== process.env.TWOFA_TOKEN) {
      return res.status(400).json({ error: 'Invalid 2FA token' });
    }

    const user = await User.findById(userId);
    if (user.twoFactorEnabled) {
      return res.json({ success: true, message: '2FA already enabled' });
    }

    user.twoFactorEnabled = true;
    await user.save();

    res.json({ success: true, message: '2FA enabled successfully' });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Check if Telegram account is linked
 */
function checkTelegramLink(req, res, next) {
  const { id } = req.user;
  User.findById(id)
    .then(user => {
      req.isTelegramLinked = !!user.telegramId;
      next();
    })
    .catch(next);
}

module.exports = {
  registerUser,
  loginUser,
  linkTelegram,
  enable2FA,
  checkTelegramLink,
  verifyAndUpgradePassword,
  matchesLegacySha256
};
