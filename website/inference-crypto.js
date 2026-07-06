"use strict";

/**
 * HONE Inference Crypto (Browser) -- ECDH + AES-256-GCM
 * Shin Devlin
 *
 * Browser port of src/inference/crypto.js using Web Crypto API + noble-secp256k1.
 * Produces identical shared secrets as the Node.js version.
 *
 * Flow:
 *   User:  computeSharedSecret(user_memo_priv, miner_memo_pub) -> secret
 *          encrypt(prompt, secret)  -> {ciphertext, iv, tag}
 *   Miner: computeSharedSecret(miner_memo_priv, user_memo_pub) -> same secret
 *          decrypt({ciphertext, iv, tag}, secret) -> plaintext prompt
 *          encrypt(result, secret)  -> {ciphertext, iv, tag}
 *   User:  decrypt({ciphertext, iv, tag}, secret) -> result
 */

(async function () {
  // Load noble-secp256k1 from CDN
  const secp = await import(
    "https://cdn.jsdelivr.net/npm/@noble/secp256k1@2.1.0/+esm"
  );

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  function hexToBytes(hex) {
    if (typeof hex !== "string") throw new Error("hexToBytes: expected string");
    const h = hex.length % 2 ? "0" + hex : hex;
    const bytes = new Uint8Array(h.length / 2);
    for (let i = 0; i < bytes.length; i++) {
      bytes[i] = parseInt(h.slice(i * 2, i * 2 + 2), 16);
    }
    return bytes;
  }

  function bytesToHex(bytes) {
    return Array.from(bytes)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
  }

  function bytesToBase64(bytes) {
    let binary = "";
    for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
    return btoa(binary);
  }

  function base64ToBytes(b64) {
    const binary = atob(b64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  }

  async function sha256(data) {
    const buf = await crypto.subtle.digest("SHA-256", data);
    return new Uint8Array(buf);
  }

  // ---------------------------------------------------------------------------
  // ECDH shared secret
  // ---------------------------------------------------------------------------

  /**
   * Compute ECDH shared secret from our private key + their public key.
   *
   * Matches Node.js version:
   *   1. secp256k1.ecdh(theirPub, myPriv) -> SHA-256 of raw shared point x-coord
   *   2. SHA-256 again so the AES key never equals the raw ECDH output
   *
   * noble-secp256k1 v2 getSharedSecret returns the raw shared point (compressed
   * or uncompressed). The Node secp256k1 npm package's ecdh() returns SHA-256
   * of the shared point's x-coordinate by default. We replicate that here.
   *
   * @param {string} myPrivKeyHex   - 32-byte hex private key
   * @param {string} theirPubKeyHex - 33-byte (compressed) hex public key
   * @returns {Promise<Uint8Array>} 32-byte symmetric key
   */
  async function computeSharedSecret(myPrivKeyHex, theirPubKeyHex) {
    if (!myPrivKeyHex || !theirPubKeyHex) {
      throw new Error("computeSharedSecret: both keys required");
    }

    const myPriv = hexToBytes(myPrivKeyHex);
    const theirPub = hexToBytes(theirPubKeyHex);

    // noble getSharedSecret returns the full shared point (uncompressed = 65 bytes)
    // Node secp256k1 ecdh() returns SHA-256 of just the x-coordinate (32 bytes)
    const sharedPoint = secp.getSharedSecret(myPriv, theirPub, false); // uncompressed
    // Extract x-coordinate (bytes 1..33 of uncompressed point)
    const xCoord = sharedPoint.slice(1, 33);

    // First SHA-256 (matches Node secp256k1 ecdh default hash)
    const firstHash = await sha256(xCoord);

    // Second SHA-256 (matches the extra hash in our Node crypto.js)
    const secondHash = await sha256(firstHash);

    return secondHash;
  }

  // ---------------------------------------------------------------------------
  // AES-256-GCM encrypt / decrypt
  // ---------------------------------------------------------------------------

  /**
   * Import a raw 32-byte key for AES-256-GCM.
   */
  async function importKey(sharedSecret) {
    let keyBytes;
    if (typeof sharedSecret === "string") {
      keyBytes = hexToBytes(sharedSecret);
    } else {
      keyBytes = sharedSecret;
    }
    if (keyBytes.length !== 32) {
      throw new Error("Key must be 32 bytes, got " + keyBytes.length);
    }
    return crypto.subtle.importKey("raw", keyBytes, { name: "AES-GCM" }, false, [
      "encrypt",
      "decrypt",
    ]);
  }

  /**
   * Encrypt plaintext with AES-256-GCM using a shared secret.
   *
   * @param {string} plaintext        - UTF-8 string to encrypt
   * @param {Uint8Array|string} sharedSecret - 32-byte key (Uint8Array or hex string)
   * @returns {Promise<{ciphertext: string, iv: string, tag: string}>} base64-encoded fields
   */
  async function encrypt(plaintext, sharedSecret) {
    const key = await importKey(sharedSecret);
    const iv = crypto.getRandomValues(new Uint8Array(12)); // 96-bit IV

    const encoded = new TextEncoder().encode(plaintext);

    // Web Crypto AES-GCM appends the 16-byte auth tag to the ciphertext
    const encrypted = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: iv, tagLength: 128 },
      key,
      encoded
    );

    const encryptedBytes = new Uint8Array(encrypted);
    // Split: ciphertext is everything except the last 16 bytes, tag is last 16
    const ciphertext = encryptedBytes.slice(0, encryptedBytes.length - 16);
    const tag = encryptedBytes.slice(encryptedBytes.length - 16);

    return {
      ciphertext: bytesToBase64(ciphertext),
      iv: bytesToBase64(iv),
      tag: bytesToBase64(tag),
    };
  }

  /**
   * Decrypt ciphertext with AES-256-GCM.
   * Throws if the auth tag does not verify (wrong key, tampered data).
   *
   * @param {{ciphertext: string, iv: string, tag: string}} encryptedData
   * @param {Uint8Array|string} sharedSecret - 32-byte key (Uint8Array or hex string)
   * @returns {Promise<string>} plaintext UTF-8 string
   */
  async function decrypt(encryptedData, sharedSecret) {
    if (
      !encryptedData ||
      encryptedData.ciphertext === undefined ||
      encryptedData.ciphertext === null ||
      !encryptedData.iv ||
      !encryptedData.tag
    ) {
      throw new Error("decrypt: encryptedData must have ciphertext, iv, tag");
    }

    const key = await importKey(sharedSecret);
    const iv = base64ToBytes(encryptedData.iv);
    const ciphertext = base64ToBytes(encryptedData.ciphertext);
    const tag = base64ToBytes(encryptedData.tag);

    // Web Crypto expects ciphertext + tag concatenated
    const combined = new Uint8Array(ciphertext.length + tag.length);
    combined.set(ciphertext, 0);
    combined.set(tag, ciphertext.length);

    const decrypted = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: iv, tagLength: 128 },
      key,
      combined
    );

    return new TextDecoder().decode(decrypted);
  }

  // ---------------------------------------------------------------------------
  // Export
  // ---------------------------------------------------------------------------

  window.honeCrypto = { computeSharedSecret, encrypt, decrypt };

  console.log("[HONE Crypto] Browser inference crypto ready");
})();
