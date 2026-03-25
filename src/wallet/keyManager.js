"use strict";

/**
 * keyManager.js — BIP-39 mnemonic wallet and protocol-level 2FA for BTCPC
 * Shin Devlin
 */

const crypto = require("crypto");
const bip39 = require("bip39");
const HDKey = require("hdkey");
const secp256k1 = require("secp256k1");

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

// BTCPC coin type for BIP-44 (unregistered; using high range)
const BTCPC_COIN_TYPE = 0x80000000 + 8888;

// BIP-44 derivation paths for each role key
const ROLE_PATHS = {
  owner:   "m/44'/8888'/0'/0/0",
  active:  "m/44'/8888'/0'/1/0",
  posting: "m/44'/8888'/0'/2/0",
  memo:    "m/44'/8888'/0'/3/0"
};

const PBKDF2_ROUNDS = 100000;
const PBKDF2_KEY_LEN = 32;
const PBKDF2_DIGEST = "sha512";

const AES_ALGORITHM = "aes-256-gcm";
const AES_IV_LEN = 16;
const AES_TAG_LEN = 16;

// ---------------------------------------------------------------------------
// Mnemonic Generation
// ---------------------------------------------------------------------------

/**
 * Generate a new 12-word BIP-39 mnemonic.
 * @returns {string} Space-separated 12-word mnemonic
 */
function generateMnemonic() {
  return bip39.generateMnemonic(128); // 128 bits = 12 words
}

/**
 * Validate a BIP-39 mnemonic.
 * @param {string} mnemonic
 * @returns {boolean}
 */
function validateMnemonic(mnemonic) {
  return bip39.validateMnemonic(mnemonic);
}

// ---------------------------------------------------------------------------
// Key Derivation
// ---------------------------------------------------------------------------

/**
 * Derive a secp256k1 keypair from an HD key at a given path.
 * @param {Buffer} seed - BIP-39 seed
 * @param {string} path - BIP-44 derivation path
 * @returns {{ privateKey: Buffer, publicKey: Buffer }}
 */
function deriveKeypairFromSeed(seed, path) {
  const master = HDKey.fromMasterSeed(seed);
  const child = master.derive(path);
  const privKey = child.privateKey;
  const pubKey = secp256k1.publicKeyCreate(privKey, true); // compressed
  return { privateKey: privKey, publicKey: Buffer.from(pubKey) };
}

/**
 * Derive all four role keypairs (owner, active, posting, memo) from a mnemonic.
 * @param {string} mnemonic - 12-word BIP-39 mnemonic
 * @param {string} [password=""] - Optional BIP-39 passphrase (not 2FA password)
 * @returns {Promise<Object>} Map of role -> { privateKey, publicKey } (hex strings)
 */
async function mnemonicToKeys(mnemonic, password) {
  if (!validateMnemonic(mnemonic)) {
    throw new Error("Invalid BIP-39 mnemonic");
  }
  const seed = await bip39.mnemonicToSeed(mnemonic, password || "");
  const keys = {};
  for (const [role, path] of Object.entries(ROLE_PATHS)) {
    const pair = deriveKeypairFromSeed(seed, path);
    keys[role] = {
      privateKey: pair.privateKey.toString("hex"),
      publicKey: pair.publicKey.toString("hex")
    };
  }
  return keys;
}

// ---------------------------------------------------------------------------
// 2FA Key Derivation (Protocol-Level)
// ---------------------------------------------------------------------------

/**
 * Derive a 2FA keypair from a password and account name using PBKDF2.
 * The password never touches the chain — only the derived public key.
 *
 * @param {string} password - User's 2FA password
 * @param {string} accountName - BTCPC account name (used as salt)
 * @returns {Promise<{ privateKey: string, publicKey: string }>} Hex-encoded keypair
 */
function deriveSecondFactor(password, accountName) {
  return new Promise(function (resolve, reject) {
    const salt = Buffer.from(accountName, "utf8");
    crypto.pbkdf2(password, salt, PBKDF2_ROUNDS, PBKDF2_KEY_LEN, PBKDF2_DIGEST, function (err, derived) {
      if (err) return reject(err);

      // Ensure the derived key is a valid secp256k1 private key
      // If not valid (extremely rare), rehash until valid
      let privKey = derived;
      let attempts = 0;
      while (!secp256k1.privateKeyVerify(privKey) && attempts < 100) {
        privKey = crypto.createHash("sha256").update(privKey).digest();
        attempts++;
      }
      if (!secp256k1.privateKeyVerify(privKey)) {
        return reject(new Error("Failed to derive valid 2FA private key"));
      }

      const pubKey = secp256k1.publicKeyCreate(privKey, true);
      resolve({
        privateKey: privKey.toString("hex"),
        publicKey: Buffer.from(pubKey).toString("hex")
      });
    });
  });
}

// ---------------------------------------------------------------------------
// Transaction Signing & Verification
// ---------------------------------------------------------------------------

/**
 * Sign a transaction (or arbitrary data hash) with a private key.
 * @param {Object|string|Buffer} tx - Transaction object, hex string, or Buffer to sign
 * @param {string} privateKey - Hex-encoded private key
 * @returns {{ signature: string, recovery: number }} DER-encoded signature (hex) + recovery id
 */
function signTransaction(tx, privateKey) {
  const hash = txHash(tx);
  const privBuf = Buffer.from(privateKey, "hex");
  const sigObj = secp256k1.ecdsaSign(hash, privBuf);
  return {
    signature: Buffer.from(sigObj.signature).toString("hex"),
    recovery: sigObj.recid
  };
}

/**
 * Verify a signature against a transaction and public key.
 * @param {Object|string|Buffer} tx - Transaction object, hex string, or Buffer
 * @param {string} signature - Hex-encoded signature
 * @param {string} publicKey - Hex-encoded compressed public key
 * @returns {boolean}
 */
function verifySignature(tx, signature, publicKey) {
  const hash = txHash(tx);
  const sigBuf = Buffer.from(signature, "hex");
  const pubBuf = Buffer.from(publicKey, "hex");
  return secp256k1.ecdsaVerify(sigBuf, hash, pubBuf);
}

/**
 * Compute a SHA-256 hash of a transaction for signing.
 * @param {Object|string|Buffer} tx
 * @returns {Buffer} 32-byte hash
 */
function txHash(tx) {
  if (Buffer.isBuffer(tx)) return tx.length === 32 ? tx : crypto.createHash("sha256").update(tx).digest();
  if (typeof tx === "string") return crypto.createHash("sha256").update(Buffer.from(tx, "hex")).digest();
  // Object — deterministic JSON serialization
  const canonical = JSON.stringify(tx, Object.keys(tx).sort());
  return crypto.createHash("sha256").update(canonical).digest();
}

// ---------------------------------------------------------------------------
// Key Encryption / Decryption (at-rest protection)
// ---------------------------------------------------------------------------

/**
 * Encrypt a private key for storage using AES-256-GCM.
 * The encryption key is derived from the provided password/TOTP via PBKDF2.
 *
 * @param {string} key - Hex-encoded private key to encrypt
 * @param {string} totpOrPassword - Password or TOTP code used as encryption key material
 * @returns {Promise<string>} Base64-encoded encrypted payload (salt:iv:tag:ciphertext)
 */
function encryptKey(key, totpOrPassword) {
  return new Promise(function (resolve, reject) {
    const salt = crypto.randomBytes(16);
    crypto.pbkdf2(totpOrPassword, salt, PBKDF2_ROUNDS, PBKDF2_KEY_LEN, PBKDF2_DIGEST, function (err, derived) {
      if (err) return reject(err);
      const iv = crypto.randomBytes(AES_IV_LEN);
      const cipher = crypto.createCipheriv(AES_ALGORITHM, derived, iv);
      const keyBuf = Buffer.from(key, "hex");
      let encrypted = cipher.update(keyBuf);
      encrypted = Buffer.concat([encrypted, cipher.final()]);
      const tag = cipher.getAuthTag();

      // Pack: salt(16) + iv(16) + tag(16) + ciphertext
      const packed = Buffer.concat([salt, iv, tag, encrypted]);
      resolve(packed.toString("base64"));
    });
  });
}

/**
 * Decrypt a private key from its encrypted at-rest form.
 *
 * @param {string} encrypted - Base64-encoded encrypted payload
 * @param {string} totpOrPassword - Password or TOTP code used during encryption
 * @returns {Promise<string>} Hex-encoded private key
 */
function decryptKey(encrypted, totpOrPassword) {
  return new Promise(function (resolve, reject) {
    const packed = Buffer.from(encrypted, "base64");
    const salt = packed.subarray(0, 16);
    const iv = packed.subarray(16, 32);
    const tag = packed.subarray(32, 48);
    const ciphertext = packed.subarray(48);

    crypto.pbkdf2(totpOrPassword, salt, PBKDF2_ROUNDS, PBKDF2_KEY_LEN, PBKDF2_DIGEST, function (err, derived) {
      if (err) return reject(err);
      try {
        const decipher = crypto.createDecipheriv(AES_ALGORITHM, derived, iv);
        decipher.setAuthTag(tag);
        let decrypted = decipher.update(ciphertext);
        decrypted = Buffer.concat([decrypted, decipher.final()]);
        resolve(decrypted.toString("hex"));
      } catch (e) {
        reject(new Error("Decryption failed — wrong password or corrupted data"));
      }
    });
  });
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  generateMnemonic,
  validateMnemonic,
  mnemonicToKeys,
  deriveSecondFactor,
  signTransaction,
  verifySignature,
  encryptKey,
  decryptKey,
  ROLE_PATHS,
  txHash
};
