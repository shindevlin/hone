"use strict";

/**
 * BTCPC Encrypted Inference Sessions
 * Shin Devlin
 *
 * SIK-bound session key exchange for private inference.
 * The session key can only be derived on the machine with the registered GPU.
 *
 * Flow:
 *   1. User requests session → gets node's session_public_key + session_id
 *   2. User encrypts token IDs with session key (derived from ECDH + SIK hash)
 *   3. Node decrypts token IDs inside locked process using SIK-derived key
 *   4. No plaintext ever exists — only token integers
 */

const crypto = require("crypto");
const silicon = require("../silicon");

// Active sessions: Map<session_id, { key, createdAt, expiresAt }>
const sessions = new Map();
const SESSION_TTL_MS = 5 * 60 * 1000; // 5 minutes

// Cleanup expired sessions every 60s
setInterval(() => {
  const now = Date.now();
  for (const [id, session] of sessions) {
    if (now > session.expiresAt) {
      session.key.fill(0); // zero the key
      sessions.delete(id);
    }
  }
}, 60000);

/**
 * Create a new inference session.
 * Returns { session_id, server_public_key, sik_hash } for the client.
 */
async function createSession() {
  // Generate ephemeral ECDH keypair for this session
  const ecdh = crypto.createECDH("secp256k1");
  ecdh.generateKeys();

  const sessionId = crypto.randomBytes(16).toString("hex");

  // Get SIK from physical GPU
  const fp = await silicon.getFingerprint();

  sessions.set(sessionId, {
    ecdh,
    sikHash: fp.sik_hash,
    sik: fp.sik,
    createdAt: Date.now(),
    expiresAt: Date.now() + SESSION_TTL_MS,
    key: null, // derived when client sends their public key
  });

  return {
    session_id: sessionId,
    server_public_key: ecdh.getPublicKey("hex"),
    sik_hash: fp.sik_hash,
  };
}

/**
 * Derive the shared session key from client's public key + SIK.
 * Called when the client sends their first encrypted request.
 *
 * session_key = HKDF(ECDH_shared_secret + SIK, salt=session_id)
 *
 * The SIK component means this key can only be derived on this physical GPU.
 */
function deriveSessionKey(sessionId, clientPublicKeyHex) {
  const session = sessions.get(sessionId);
  if (!session) throw new Error("Session not found or expired");
  if (session.key) return session.key; // already derived

  // ECDH shared secret
  const sharedSecret = session.ecdh.computeSecret(
    Buffer.from(clientPublicKeyHex, "hex")
  );

  // Combine ECDH secret with SIK — binds the key to physical silicon
  const ikm = Buffer.concat([sharedSecret, Buffer.from(session.sik, "hex")]);

  // HKDF-SHA256
  const salt = Buffer.from(sessionId, "hex");
  const info = Buffer.from("btcpc-inference-v1");
  const key = hkdf(ikm, salt, info, 32);

  // Store derived key, wipe intermediates
  session.key = key;
  sharedSecret.fill(0);
  ikm.fill(0);
  session.sik = "0".repeat(64); // wipe SIK from session object
  session.ecdh = null; // no longer needed

  return key;
}

/**
 * Get the session key for an active session.
 */
function getSessionKey(sessionId) {
  const session = sessions.get(sessionId);
  if (!session || !session.key) return null;
  if (Date.now() > session.expiresAt) {
    session.key.fill(0);
    sessions.delete(sessionId);
    return null;
  }
  return session.key;
}

/**
 * Destroy a session and zero its key material.
 */
function destroySession(sessionId) {
  const session = sessions.get(sessionId);
  if (session) {
    if (session.key) session.key.fill(0);
    sessions.delete(sessionId);
  }
}

/**
 * Encrypt data with a session key (AES-256-GCM).
 */
function encrypt(key, plaintext) {
  const iv = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv("aes-256-gcm", key, iv);
  const encrypted = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  const tag = cipher.getAuthTag();
  // Format: iv(12) + tag(16) + ciphertext
  return Buffer.concat([iv, tag, encrypted]);
}

/**
 * Decrypt data with a session key (AES-256-GCM).
 */
function decrypt(key, blob) {
  const iv = blob.subarray(0, 12);
  const tag = blob.subarray(12, 28);
  const ciphertext = blob.subarray(28);
  const decipher = crypto.createDecipheriv("aes-256-gcm", key, iv);
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(ciphertext), decipher.final()]);
}

/**
 * HKDF-SHA256 key derivation.
 */
function hkdf(ikm, salt, info, length) {
  // Extract
  const prk = crypto.createHmac("sha256", salt).update(ikm).digest();
  // Expand
  let t = Buffer.alloc(0);
  let okm = Buffer.alloc(0);
  for (let i = 1; okm.length < length; i++) {
    t = crypto.createHmac("sha256", prk)
      .update(Buffer.concat([t, info, Buffer.from([i])]))
      .digest();
    okm = Buffer.concat([okm, t]);
  }
  return okm.subarray(0, length);
}

module.exports = {
  createSession,
  deriveSessionKey,
  getSessionKey,
  destroySession,
  encrypt,
  decrypt,
};
