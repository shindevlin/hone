"use strict";

/**
 * BTCPC Private Inference Client
 * Shin Devlin
 *
 * User-side SDK for submitting encrypted inference requests.
 * Tokenization happens HERE on the user's device. The node never sees plaintext.
 *
 * Flow:
 *   1. Open session with node (ECDH key exchange)
 *   2. Tokenize prompt locally
 *   3. Generate per-session token remap (shuffle token IDs)
 *   4. Encrypt remapped token IDs with SIK-bound session key
 *   5. Send encrypted blob to node
 *   6. Receive encrypted response, decrypt, de-remap
 *
 * The node processes remapped integers. Even if it could read them,
 * they map to wrong vocabulary entries without the remap table.
 */

const crypto = require("crypto");
const axios = require("axios");

class BTCPCClient {
  constructor({ nodeUrl, apiKey }) {
    this.nodeUrl = nodeUrl.replace(/\/$/, "");
    this.apiKey = apiKey;
    this.session = null;
  }

  /**
   * Open an encrypted inference session with the node.
   */
  async openSession() {
    const res = await axios.post(
      `${this.nodeUrl}/v1/session`,
      {},
      { headers: { Authorization: `Bearer ${this.apiKey}` } }
    );

    const { session_id, server_public_key, sik_hash } = res.data;

    // Generate our ephemeral ECDH keypair
    const ecdh = crypto.createECDH("secp256k1");
    ecdh.generateKeys();

    // Derive shared secret + SIK-bound session key
    const sharedSecret = ecdh.computeSecret(
      Buffer.from(server_public_key, "hex")
    );
    const ikm = Buffer.concat([sharedSecret, Buffer.from(sik_hash, "hex")]);
    const salt = Buffer.from(session_id, "hex");
    const info = Buffer.from("btcpc-inference-v1");
    const sessionKey = hkdf(ikm, salt, info, 32);

    // Wipe intermediates
    sharedSecret.fill(0);
    ikm.fill(0);

    // Generate token remap table for this session
    // Use actual vocab size from model tokenizer when available
    const remap = generateRemap(200000); // covers all major model vocab sizes

    this.session = {
      id: session_id,
      key: sessionKey,
      clientPublicKey: ecdh.getPublicKey("hex"),
      remap: remap.forward,
      deremap: remap.reverse,
      sikHash: sik_hash,
    };

    return { session_id, sik_hash };
  }

  /**
   * Send a private inference request.
   * Prompt is tokenized and remapped locally — never leaves this device as text.
   */
  async inference({ model, prompt, temperature, max_tokens }) {
    if (!this.session) await this.openSession();

    // Step 1: Tokenize locally with real model vocabulary
    const tok = await getTokenizer(model);
    const tokens = tok.encode(prompt);

    // Step 2: Remap token IDs (shuffle with session-specific permutation)
    const remapped = tokens.map((t) => this.session.remap[t] ?? t);

    // Step 3: Build payload — remapped token IDs + model params
    const payload = JSON.stringify({
      model: model || "qwen3.5:27b",
      tokens: remapped,
      temperature: temperature ?? 0.7,
      max_tokens: max_tokens ?? 2048,
      remap_hash: crypto
        .createHash("sha256")
        .update(JSON.stringify(this.session.remap))
        .digest("hex")
        .slice(0, 16),
    });

    // Step 4: Encrypt with SIK-bound session key
    const encrypted = encrypt(this.session.key, Buffer.from(payload));

    // Step 5: Send to node
    const res = await axios.post(
      `${this.nodeUrl}/v1/inference/encrypted`,
      {
        session_id: this.session.id,
        client_public_key: this.session.clientPublicKey,
        payload: encrypted.toString("base64"),
      },
      {
        headers: { Authorization: `Bearer ${this.apiKey}` },
        timeout: 120000,
      }
    );

    // Step 6: Decrypt response
    const encryptedResponse = Buffer.from(res.data.payload, "base64");
    const decrypted = decrypt(this.session.key, encryptedResponse);
    const result = JSON.parse(decrypted.toString());

    // Step 7: De-remap output tokens if present
    if (result.tokens) {
      result.tokens = result.tokens.map(
        (t) => this.session.deremap[t] ?? t
      );
      const tok = await getTokenizer(model);
      result.text = tok.decode(result.tokens);
    }

    return result;
  }

  /**
   * Close the session and wipe key material.
   */
  async closeSession() {
    if (!this.session) return;
    try {
      await axios.delete(
        `${this.nodeUrl}/v1/session/${this.session.id}`,
        { headers: { Authorization: `Bearer ${this.apiKey}` } }
      );
    } catch (_) {}
    this.session.key.fill(0);
    this.session = null;
  }
}

// ── Token remap generation ──
// Creates a random permutation of token IDs so the node can't reverse them.
function generateRemap(vocabSize) {
  // Fisher-Yates shuffle of indices
  const indices = Array.from({ length: vocabSize }, (_, i) => i);
  for (let i = vocabSize - 1; i > 0; i--) {
    const j = crypto.randomInt(0, i + 1);
    [indices[i], indices[j]] = [indices[j], indices[i]];
  }

  const forward = {}; // real → remapped
  const reverse = {}; // remapped → real
  for (let i = 0; i < vocabSize; i++) {
    forward[i] = indices[i];
    reverse[indices[i]] = i;
  }
  return { forward, reverse };
}

// ── Tokenizer ──
const tokenizer = require("./tokenizer");
let _tokenizer = null;

async function getTokenizer(model) {
  if (_tokenizer && _tokenizer._model === model) return _tokenizer;
  _tokenizer = await tokenizer.load(model || "qwen3.5:27b");
  _tokenizer._model = model;
  return _tokenizer;
}

// ── Crypto helpers (same as session.js) ──
function encrypt(key, plaintext) {
  const iv = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv("aes-256-gcm", key, iv);
  const encrypted = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  const tag = cipher.getAuthTag();
  return Buffer.concat([iv, tag, encrypted]);
}

function decrypt(key, blob) {
  const iv = blob.subarray(0, 12);
  const tag = blob.subarray(12, 28);
  const ciphertext = blob.subarray(28);
  const decipher = crypto.createDecipheriv("aes-256-gcm", key, iv);
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(ciphertext), decipher.final()]);
}

function hkdf(ikm, salt, info, length) {
  const prk = crypto.createHmac("sha256", salt).update(ikm).digest();
  let t = Buffer.alloc(0);
  let okm = Buffer.alloc(0);
  for (let i = 1; okm.length < length; i++) {
    t = crypto
      .createHmac("sha256", prk)
      .update(Buffer.concat([t, info, Buffer.from([i])]))
      .digest();
    okm = Buffer.concat([okm, t]);
  }
  return okm.subarray(0, length);
}

module.exports = { BTCPCClient };
