"use strict";

/**
 * In-memory session token store for third-party app authorization.
 *
 * Tokens are prefixed with hone_session_ and have a spending limit,
 * expiry, and per-session spend tracker. Lost on restart (by design —
 * sessions are short-lived, 24h default).
 */

const crypto = require('crypto');

// Map<token, { token, account, app_name, spending_limit, spent, expires_at, created_at }>
const sessions = new Map();

function pruneExpired() {
  const now = Date.now();
  for (const [token, session] of sessions) {
    if (now >= session.expires_at) {
      sessions.delete(token);
    }
  }
}

/**
 * Create a new session token.
 * @param {string} account - HONE account name
 * @param {string} appName - Name of the requesting app
 * @param {number} spendingLimit - Max HONE spend for this session
 * @param {number} expiresHours - Hours until expiry (default 24, max 168)
 * @returns {{ token, account, app_name, spending_limit, expires_at }}
 */
function createSession(account, appName, spendingLimit, expiresHours) {
  const token = 'hone_session_' + crypto.randomBytes(32).toString('hex');
  const now = Date.now();
  const hours = Math.min(168, Math.max(1, expiresHours || 24));
  const expiresAt = now + hours * 60 * 60 * 1000;

  const session = {
    token,
    account,
    app_name: appName || 'Unknown App',
    spending_limit: spendingLimit,
    spent: 0,
    expires_at: expiresAt,
    created_at: now,
  };

  sessions.set(token, session);
  pruneExpired();
  return session;
}

/**
 * Look up a session token. Returns null if not found or expired.
 */
function getSession(token) {
  pruneExpired();
  return sessions.get(token) || null;
}

/**
 * Record spending against a session. Returns true if within limit, false if exceeded.
 * @param {string} token
 * @param {number} amount - HONE cost to record
 * @returns {boolean}
 */
function recordSpend(token, amount) {
  const session = getSession(token);
  if (!session) return false;
  if (session.spent + amount > session.spending_limit) return false;
  session.spent += amount;
  return true;
}

/**
 * Check remaining balance for a session token.
 * @param {string} token
 * @returns {number} remaining HONE, or 0 if invalid/expired
 */
function getRemaining(token) {
  const session = getSession(token);
  if (!session) return 0;
  return Math.max(0, session.spending_limit - session.spent);
}

/**
 * Revoke (delete) a session.
 */
function revokeSession(token) {
  return sessions.delete(token);
}

module.exports = {
  createSession,
  getSession,
  recordSpend,
  getRemaining,
  revokeSession,
  sessions,   // direct access for debugging/admin
};
