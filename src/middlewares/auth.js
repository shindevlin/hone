"use strict";
const jwt = require('jsonwebtoken');

// D.5-delta: secretStore-first user lookup, Mongo fallback.
// Same lazy-load pattern as authController (D.5-gamma).
let _secretStoreLoaded = false;
async function getSecretStore() {
  const secretStore = require('../services/secretStore');
  if (!_secretStoreLoaded) {
    try {
      await secretStore.load();
      _secretStoreLoaded = true;
    } catch (err) {
      console.warn('[auth-mw] secretStore load failed: ' + err.message);
    }
  }
  return secretStore;
}

/**
 * Authentication Middleware — D.5-delta.
 *
 * Decodes JWT then resolves the user from secretStore first.
 * Falls back to Mongo for legacy installs. Normalises the user
 * object so req.user always has { id, username, email, ... }
 * regardless of which source answered.
 */
async function authenticateToken(req, res, next) {
  const authHeader = req.headers['authorization'];
  const token = authHeader && authHeader.split(' ')[1]; // Bearer TOKEN

  if (!token) return res.status(401).json({ error: 'Access denied. No token provided.' });

  let decoded;
  try {
    decoded = jwt.verify(token, process.env.JWT_SECRET || process.env.HONE_JWT_SECRET);
  } catch (err) {
    return res.status(403).json({ error: 'Invalid token.' });
  }

  // ── Try secretStore first ──
  try {
    const ss = await getSecretStore();
    if (ss && typeof ss.getUser === 'function') {
      let ssRec = null;
      if (decoded.username) {
        ssRec = ss.getUser(decoded.username);
      }
      if (!ssRec && decoded.id) {
        ssRec = (typeof ss.getUserById === 'function') ? ss.getUserById(decoded.id) : null;
      }
      if (ssRec) {
        // Normalise to the shape downstream code expects.
        // secretStore uses user_id; Mongo uses _id — both become id.
        // Legacy records may lack a username field — patch in-place so the
        // record is correct for future lookups without requiring a restart.
        if (!ssRec.username && decoded.username) {
          ssRec.username = decoded.username;
        }
        req.user = {
          id: ssRec.user_id || decoded.id,
          username: ssRec.username || decoded.username,
          email: ssRec.email,
          is_active: ssRec.is_active !== false,
          two_factor_enabled: !!ssRec.two_factor_enabled,
          totp_enabled: !!ssRec.totp_enabled,
          // Forward any extra JWT claims (src, iat, exp, etc.)
          ...decoded,
          // Override with secretStore data to stay canonical
          id: ssRec.user_id || decoded.id,
          username: ssRec.username || decoded.username,
        };
        return next();
      }
    }
  } catch (_) {
    // secretStore failed — fall through to Mongo
  }

  // ── Mongo fallback ──
  try {
    const User = require('../models/User');
    const mongoUser = await User.findById(decoded.id);
    if (mongoUser) {
      req.user = {
        ...decoded,
        // Normalise Mongo _id → id
        id: mongoUser._id.toString(),
        username: mongoUser.username,
        email: mongoUser.email,
      };
      return next();
    }
  } catch (_) {
    // Mongo unavailable — cannot verify user exists
  }

  // If neither secretStore nor Mongo could verify the user exists,
  // reject the request. Never proceed with unverified JWT claims.
  return res.status(401).json({ error: 'authentication service unavailable' });
}

/**
 * Rate Limiting Middleware
 */
const rateLimit = require('express-rate-limit');
const limiter = rateLimit({ windowMs: 60 * 1000, max: 100 });

module.exports = { authenticateToken, limiter };
