"use strict";

/**
 * HONE Inference Crypto — ECDH + AES-256-GCM
 * Shin Devlin
 *
 * Encrypted inference primitives using the memo key ECDH channel.
 * Nobody on the P2P network sees plaintext prompts or results.
 *
 * Flow:
 *   User:  computeSharedSecret(user_memo_priv, miner_memo_pub) → secret
 *          encrypt(prompt, secret)  → {ciphertext, iv, tag}
 *   Miner: computeSharedSecret(miner_memo_priv, user_memo_pub) → same secret
 *          decrypt({ciphertext, iv, tag}, secret) → plaintext prompt
 *          encrypt(result, secret)  → {ciphertext, iv, tag}
 *   User:  decrypt({ciphertext, iv, tag}, secret) → result
 */

const crypto = require("crypto");
const secp256k1 = require("secp256k1");

/**
 * Compute ECDH shared secret from our private key + their public key.
 * secp256k1.ecdh already returns a 32-byte SHA-256 of the raw shared point,
 * so we hash it once more to produce a clean AES-256 key.
 *
 * @param {string} myPrivKeyHex   — 32-byte hex private key
 * @param {string} theirPubKeyHex — 33-byte (compressed) hex public key
 * @returns {Buffer} 32-byte symmetric key
 */
function computeSharedSecret(myPrivKeyHex, theirPubKeyHex) {
  if (!myPrivKeyHex || !theirPubKeyHex) {
    throw new Error("computeSharedSecret: both keys required");
  }
  const myPriv = Buffer.from(myPrivKeyHex, "hex");
  const theirPub = Buffer.from(theirPubKeyHex, "hex");
  // secp256k1.ecdh(pubkey, privkey) → Uint8Array(32) — already SHA-256 of shared point
  const raw = secp256k1.ecdh(theirPub, myPriv);
  // Hash again so the AES key never equals the raw ECDH output
  return crypto.createHash("sha256").update(Buffer.from(raw)).digest();
}

/**
 * Encrypt plaintext with AES-256-GCM using a shared secret.
 *
 * @param {string} plaintext    — UTF-8 string to encrypt
 * @param {Buffer} sharedSecret — 32-byte key from computeSharedSecret
 * @returns {{ ciphertext: string, iv: string, tag: string }} base64-encoded fields
 */
function encrypt(plaintext, sharedSecret) {
  if (sharedSecret.length !== 32) {
    throw new Error("encrypt: sharedSecret must be 32 bytes");
  }
  const iv = crypto.randomBytes(12); // 96-bit IV is standard for GCM
  const cipher = crypto.createCipheriv("aes-256-gcm", sharedSecret, iv);
  const encrypted = Buffer.concat([
    cipher.update(plaintext, "utf8"),
    cipher.final(),
  ]);
  const tag = cipher.getAuthTag();
  return {
    ciphertext: encrypted.toString("base64"),
    iv: iv.toString("base64"),
    tag: tag.toString("base64"),
  };
}

/**
 * Decrypt ciphertext with AES-256-GCM.
 * Throws if the auth tag does not verify (wrong key, tampered data).
 *
 * @param {{ ciphertext: string, iv: string, tag: string }} encryptedData
 * @param {Buffer} sharedSecret — 32-byte key from computeSharedSecret
 * @returns {string} plaintext UTF-8 string
 */
function decrypt(encryptedData, sharedSecret) {
  if (
    !encryptedData ||
    encryptedData.ciphertext === undefined || encryptedData.ciphertext === null ||
    !encryptedData.iv ||
    !encryptedData.tag
  ) {
    throw new Error("decrypt: encryptedData must have ciphertext, iv, tag");
  }
  if (sharedSecret.length !== 32) {
    throw new Error("decrypt: sharedSecret must be 32 bytes");
  }
  const iv = Buffer.from(encryptedData.iv, "base64");
  const tag = Buffer.from(encryptedData.tag, "base64");
  const ciphertext = Buffer.from(encryptedData.ciphertext, "base64");
  const decipher = crypto.createDecipheriv("aes-256-gcm", sharedSecret, iv);
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(ciphertext), decipher.final()]).toString("utf8");
}

module.exports = { computeSharedSecret, encrypt, decrypt };
