"use strict";

/**
 * Session token routes — mounted at /api/auth.
 * Third-party apps (e.g. OpenClaw) use these to get spending-limited
 * session tokens for inference without needing the user's private key.
 *
 * Shared state lives in services/sessionStore.js (in-memory, lost on restart).
 * The inference API's authenticateBearer reads from the same store.
 */

const express = require('express');
const crypto = require('crypto');
const router = express.Router();
const rateLimit = require('express-rate-limit');

const sessionStore = require('../services/sessionStore');
const {
  rejectObjectInputs, sanitizeString, validAccountName,
} = require('../middlewares/validate');

// CORS for browser access
router.use((req, res, next) => {
  res.header('Access-Control-Allow-Origin', '*');
  res.header('Access-Control-Allow-Methods', 'GET, POST, DELETE, OPTIONS');
  res.header('Access-Control-Allow-Headers', 'Content-Type');
  if (req.method === 'OPTIONS') return res.sendStatus(204);
  next();
});

const sessionLimiter = rateLimit({
  windowMs: 60 * 1000,
  max: 10,
  standardHeaders: true,
  legacyHeaders: false,
  message: { error: 'Too many session requests. Try again in a minute.' },
});

// Helper: verify password via secretStore first, Mongo fallback.
async function verifyPassword(username, password) {
  let authenticated = false;
  try {
    const secretStore = require('../services/secretStore');
    try { await secretStore.load(); } catch (_) {}
    if (typeof secretStore.verifyPassword === 'function') {
      authenticated = await secretStore.verifyPassword(username, password);
    }
  } catch (_) {}

  if (!authenticated) {
    try {
      const User = require('../models/User');
      const bcrypt = require('bcryptjs');
      const mongoUser = await User.findOne({ username });
      if (mongoUser && mongoUser.password) {
        if (mongoUser.password.startsWith('$2')) {
          authenticated = bcrypt.compareSync(password, mongoUser.password);
        } else {
          const sha256 = crypto.createHash('sha256').update(password).digest('hex');
          authenticated = sha256 === mongoUser.password;
        }
      }
    } catch (_) {}
  }

  return authenticated;
}

/**
 * POST /api/auth/session
 * Create a session token for third-party app authorization.
 * Body: { username, password, app_name, spending_limit, expires_hours }
 */
router.post('/session', sessionLimiter, async (req, res) => {
  try {
    const objErr = rejectObjectInputs(req.body, ['username', 'password', 'app_name', 'spending_limit', 'expires_hours']);
    if (objErr) return res.status(400).json({ error: objErr });

    const username = sanitizeString(req.body.username, 20);
    const password = req.body.password;
    const appName = sanitizeString(req.body.app_name, 64) || 'Unknown App';
    const spendingLimit = Math.max(0, parseFloat(req.body.spending_limit) || 1);
    const expiresHours = Math.min(168, Math.max(1, parseFloat(req.body.expires_hours) || 24));

    if (!username || !validAccountName(username)) {
      return res.status(400).json({ error: 'Valid account name required.' });
    }
    if (!password || typeof password !== 'string') {
      return res.status(400).json({ error: 'Password required.' });
    }

    const authenticated = await verifyPassword(username, password);
    if (!authenticated) {
      return res.status(401).json({ error: 'Invalid username or password.' });
    }

    // Verify account exists on-chain
    const stateStore = require('../chain/stateStore');
    const onChainAccount = stateStore.getAccount(username);
    if (!onChainAccount) {
      return res.status(404).json({ error: 'Account not found on-chain.' });
    }

    const session = sessionStore.createSession(username, appName, spendingLimit, expiresHours);

    console.log('[session] created token for ' + username + ' (app: ' + appName + ', limit: ' + spendingLimit + ' BTCPC)');

    res.json({
      token: session.token,
      account: username,
      app_name: appName,
      spending_limit: spendingLimit,
      expires_at: session.expires_at,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/auth/session/:token
 * Check session status. Used by third-party apps to verify a token.
 */
router.get('/session/:token', (req, res) => {
  const session = sessionStore.getSession(req.params.token);
  if (!session) {
    return res.status(404).json({ error: 'Session not found or expired.' });
  }

  res.json({
    account: session.account,
    app_name: session.app_name,
    spending_limit: session.spending_limit,
    spent: session.spent,
    remaining: Math.max(0, session.spending_limit - session.spent),
    expires_at: session.expires_at,
    created_at: session.created_at,
  });
});

/**
 * DELETE /api/auth/session/:token
 * Revoke a session. Requires password verification.
 */
router.delete('/session/:token', async (req, res) => {
  try {
    const session = sessionStore.getSession(req.params.token);
    if (!session) {
      return res.status(404).json({ error: 'Session not found or expired.' });
    }

    const password = req.body && req.body.password;
    if (!password) {
      return res.status(400).json({ error: 'Password required to revoke session.' });
    }

    const authenticated = await verifyPassword(session.account, password);
    if (!authenticated) {
      return res.status(401).json({ error: 'Invalid password.' });
    }

    sessionStore.revokeSession(req.params.token);
    console.log('[session] revoked token for ' + session.account + ' (app: ' + session.app_name + ')');

    res.json({ success: true, message: 'Session revoked.' });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
