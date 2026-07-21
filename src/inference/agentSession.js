"use strict";

/**
 * HONE Agent Sessions
 * Shin Devlin
 *
 * Multi-turn tool-calling inference sessions. Each session holds:
 *   - ECDH session key (AES-256-GCM, derived from client + server keypair)
 *   - system_prompt and tool definitions registered at session open
 *   - conversation history (role/content pairs, capped at MAX_HISTORY_TURNS)
 *
 * Crypto reuses the same ECDH + HKDF + AES-256-GCM pattern from session.js
 * but without a SIK hardware binding (tool sessions are portable).
 * Optionally enable SIK binding via HONE_AGENT_REQUIRE_SIK=true.
 */

const crypto = require("crypto");

const SESSION_TTL_MS = parseInt(process.env.HONE_AGENT_SESSION_TTL_MS) || 30 * 60 * 1000; // 30 min
const MAX_HISTORY_TURNS = parseInt(process.env.HONE_AGENT_MAX_TURNS) || 50;

// Map<session_id, AgentSessionRecord>
const sessions = new Map();

// Clean up expired sessions every 2 minutes
setInterval(function () {
  var now = Date.now();
  for (var entry of sessions) {
    var id = entry[0];
    var s = entry[1];
    if (now > s.expiresAt) {
      if (s.key) s.key.fill(0);
      sessions.delete(id);
    }
  }
}, 120000);

// ---------------------------------------------------------------------------
// HKDF-SHA256 (same impl as session.js — keep in sync)
// ---------------------------------------------------------------------------
function hkdf(ikm, salt, info, length) {
  var prk = crypto.createHmac("sha256", salt).update(ikm).digest();
  var t = Buffer.alloc(0);
  var okm = Buffer.alloc(0);
  for (var i = 1; okm.length < length; i++) {
    t = crypto.createHmac("sha256", prk)
      .update(Buffer.concat([t, info, Buffer.from([i])]))
      .digest();
    okm = Buffer.concat([okm, t]);
  }
  return okm.subarray(0, length);
}

// ---------------------------------------------------------------------------
// Session lifecycle
// ---------------------------------------------------------------------------

/**
 * Create a new agent session.
 *
 * @param {object} opts
 * @param {string}   opts.system_prompt    — system prompt for the session
 * @param {Array}    [opts.tools]          — JSON Schema tool definitions
 * @param {string}   [opts.client_public_key] — hex secp256k1 client key (if provided, key is derived immediately)
 * @returns {{ session_id, server_public_key, expires_at }}
 */
function createAgentSession(opts) {
  opts = opts || {};
  var sessionId = crypto.randomBytes(16).toString("hex");

  var ecdh = crypto.createECDH("secp256k1");
  ecdh.generateKeys();

  var record = {
    ecdh: ecdh,
    key: null,                              // derived on first turn
    system_prompt: opts.system_prompt || "",
    tools: Array.isArray(opts.tools) ? opts.tools : [],
    history: [],                            // { role, content } pairs
    // Pending tool call state: Map<call_id, { tool_name, arguments, resolve, reject }>
    pendingToolCalls: new Map(),
    createdAt: Date.now(),
    expiresAt: Date.now() + SESSION_TTL_MS,
  };

  sessions.set(sessionId, record);

  // If client sent their public key up-front, derive the session key now
  if (opts.client_public_key) {
    deriveAgentSessionKey(sessionId, opts.client_public_key);
  }

  return {
    session_id: sessionId,
    server_public_key: ecdh.getPublicKey("hex"),
    expires_at: record.expiresAt,
  };
}

/**
 * Derive the shared session key from client's public key.
 * Idempotent: returns the existing key if already derived.
 *
 * session_key = HKDF(ECDH_shared_secret, salt=session_id, info="hone-agent-v1", 32)
 */
function deriveAgentSessionKey(sessionId, clientPublicKeyHex) {
  var session = sessions.get(sessionId);
  if (!session) throw new Error("Agent session not found or expired");
  if (session.key) return session.key;

  var sharedSecret = session.ecdh.computeSecret(Buffer.from(clientPublicKeyHex, "hex"));
  var salt = Buffer.from(sessionId, "hex");
  var info = Buffer.from("hone-agent-v1");
  var key = hkdf(sharedSecret, salt, info, 32);

  session.key = key;
  sharedSecret.fill(0);
  session.ecdh = null; // keypair no longer needed
  return key;
}

/**
 * Get a session record. Returns null if expired or not found.
 */
function getAgentSession(sessionId) {
  var session = sessions.get(sessionId);
  if (!session) return null;
  if (Date.now() > session.expiresAt) {
    if (session.key) session.key.fill(0);
    sessions.delete(sessionId);
    return null;
  }
  // Extend TTL on activity
  session.expiresAt = Date.now() + SESSION_TTL_MS;
  return session;
}

/**
 * Append a turn to the conversation history.
 * Prunes the oldest non-system turns when MAX_HISTORY_TURNS is exceeded.
 */
function addTurn(sessionId, role, content) {
  var session = sessions.get(sessionId);
  if (!session) throw new Error("Agent session not found");
  session.history.push({ role: role, content: content });
  // Prune: keep system prompt separate — history[] holds only user/assistant/tool turns
  if (session.history.length > MAX_HISTORY_TURNS) {
    session.history = session.history.slice(session.history.length - MAX_HISTORY_TURNS);
  }
}

/**
 * Build the Ollama messages array from session state.
 * Format: system message first, then history, then optionally a new user turn.
 */
function buildMessages(sessionId, newUserTurn) {
  var session = sessions.get(sessionId);
  if (!session) throw new Error("Agent session not found");
  var messages = [];
  if (session.system_prompt) {
    messages.push({ role: "system", content: session.system_prompt });
  }
  for (var i = 0; i < session.history.length; i++) {
    messages.push(session.history[i]);
  }
  if (newUserTurn) {
    messages.push({ role: "user", content: newUserTurn });
  }
  return messages;
}

/**
 * Close a session and zero its key.
 */
function closeAgentSession(sessionId) {
  var session = sessions.get(sessionId);
  if (session) {
    if (session.key) session.key.fill(0);
    sessions.delete(sessionId);
  }
}

// ---------------------------------------------------------------------------
// Encrypt / Decrypt (AES-256-GCM, same wire format as session.js)
// ---------------------------------------------------------------------------

function encrypt(key, plaintext) {
  var iv = crypto.randomBytes(12);
  var cipher = crypto.createCipheriv("aes-256-gcm", key, iv);
  var encrypted = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  var tag = cipher.getAuthTag();
  return Buffer.concat([iv, tag, encrypted]); // iv(12) + tag(16) + ciphertext
}

function decrypt(key, blob) {
  var iv = blob.subarray(0, 12);
  var tag = blob.subarray(12, 28);
  var ciphertext = blob.subarray(28);
  var decipher = crypto.createDecipheriv("aes-256-gcm", key, iv);
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(ciphertext), decipher.final()]);
}

// ---------------------------------------------------------------------------
// Tool-call state: pending calls waiting for TOOL_RESULT via P2P
// ---------------------------------------------------------------------------

/**
 * Register a pending tool call. Returns a Promise that resolves when
 * the TOOL_RESULT arrives (or rejects after timeout).
 *
 * @param {string} sessionId
 * @param {string} callId
 * @param {number} [timeoutMs]
 * @returns {Promise<{ result, error }>}
 */
function waitForToolResult(sessionId, callId, timeoutMs) {
  var session = sessions.get(sessionId);
  if (!session) return Promise.reject(new Error("Session not found"));
  var ms = timeoutMs || 60000;
  return new Promise(function (resolve, reject) {
    var timer = setTimeout(function () {
      session.pendingToolCalls.delete(callId);
      reject(new Error("Tool call timed out: " + callId));
    }, ms);
    session.pendingToolCalls.set(callId, {
      resolve: function (val) { clearTimeout(timer); resolve(val); },
      reject: function (err) { clearTimeout(timer); reject(err); }
    });
  });
}

/**
 * Deliver a TOOL_RESULT to the waiting inference turn.
 * Called by the P2P TOOL_RESULT handler.
 */
function deliverToolResult(sessionId, callId, result, error) {
  var session = sessions.get(sessionId);
  if (!session) return false;
  var pending = session.pendingToolCalls.get(callId);
  if (!pending) return false;
  session.pendingToolCalls.delete(callId);
  if (error) {
    pending.resolve({ result: null, error: error }); // resolve (not reject) — caller decides how to handle
  } else {
    pending.resolve({ result: result, error: null });
  }
  return true;
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

function getSessionCount() { return sessions.size; }

module.exports = {
  createAgentSession: createAgentSession,
  deriveAgentSessionKey: deriveAgentSessionKey,
  getAgentSession: getAgentSession,
  addTurn: addTurn,
  buildMessages: buildMessages,
  closeAgentSession: closeAgentSession,
  encrypt: encrypt,
  decrypt: decrypt,
  waitForToolResult: waitForToolResult,
  deliverToolResult: deliverToolResult,
  getSessionCount: getSessionCount,
  SESSION_TTL_MS: SESSION_TTL_MS,
  MAX_HISTORY_TURNS: MAX_HISTORY_TURNS,
};
