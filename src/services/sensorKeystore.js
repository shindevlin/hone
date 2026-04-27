"use strict";

/**
 * BTCPC Sensor Keystore
 * Shin Devlin
 *
 * ed25519 keypair management for IoT sensors.
 *
 * Two key modes:
 *
 *   1. MNEMONIC-DERIVED (preferred for phones and nodes with a mnemonic)
 *      Key = HKDF(SHA512(mnemonic), salt=0x00*32, info="btcpc-sensor-v1:<sensor_id>", len=32)
 *      Same mnemonic + same sensor_id = same key on any device, survives reinstalls.
 *      Set BTCPC_MNEMONIC (or BTCPC_MNEMONIC_NATOSHI etc.) to enable.
 *
 *   2. RANDOM (hardware devices — Hyfix, ESP32, Flipper)
 *      Generates a random ed25519 keypair on first run and stores it in sensor-keys.json.
 *      Key is device-local. If device storage is lost, key must be rotated via /register-key.
 *
 * Key storage: ~/.btcpc/sensor-keys.json (or BTCPC_SENSOR_KEYS env var)
 * BTCPC_DATA_DIR is only used if that directory already exists on the current machine,
 * so mobile devices (Termux) with a grouchly-specific BTCPC_DATA_DIR fall back to ~/.btcpc.
 *
 * Signing payload format (pipe-delimited, matches btcpc-gnss-capture Rust):
 *   "<sensor_id>|<value>|<device_timestamp_ms>"
 *   or with epoch_hash:
 *   "<sensor_id>|<value>|<device_timestamp_ms>|<epoch_hash>"
 *
 * Replay protection: server rejects device_timestamp outside ±90s of server clock.
 */

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');
const os = require('os');

function _resolveKeystorePath() {
  if (process.env.BTCPC_SENSOR_KEYS) return process.env.BTCPC_SENSOR_KEYS;
  // Use BTCPC_DATA_DIR only if that directory actually exists on this machine.
  // Falls back to ~/.btcpc so mobile (Termux) doesn't try to write to a
  // grouchly-specific mount that doesn't exist on the device.
  const envDir = process.env.BTCPC_DATA_DIR;
  if (envDir && fs.existsSync(envDir)) return path.join(envDir, 'sensor-keys.json');
  return path.join(os.homedir(), '.btcpc', 'sensor-keys.json');
}
const KEYSTORE_PATH = _resolveKeystorePath();

let _cache = null;

function _load() {
  if (_cache) return _cache;
  try {
    _cache = JSON.parse(fs.readFileSync(KEYSTORE_PATH, 'utf8'));
  } catch (_) {
    _cache = {};
  }
  return _cache;
}

function _save(store) {
  const dir = path.dirname(KEYSTORE_PATH);
  if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(KEYSTORE_PATH, JSON.stringify(store, null, 2), 'utf8');
}

// PKCS8 DER wrapper for a raw 32-byte ed25519 seed.
// Structure: SEQUENCE { INTEGER 0, SEQUENCE { OID id-Ed25519 }, OCTET STRING { OCTET STRING seed } }
function _seedToPkcs8(seed32) {
  const header = Buffer.from('302e020100300506032b657004220420', 'hex');
  return Buffer.concat([header, seed32]);
}

/**
 * Derive a deterministic ed25519 keypair from a mnemonic and sensor_id.
 * Same inputs → same key, on any device, after any reinstall.
 *
 * Derivation: HKDF-SHA512(key=SHA512(mnemonic), salt=0x00*32, info="btcpc-sensor-v1:<sensor_id>", len=32)
 */
function _deriveFromMnemonic(mnemonic, sensorId) {
  const mnemonicSeed = crypto.createHash('sha512').update(mnemonic.trim()).digest();
  const derived = crypto.hkdfSync(
    'sha512',
    mnemonicSeed,
    Buffer.alloc(32),
    Buffer.from('btcpc-sensor-v1:' + sensorId),
    32
  );
  const pkcs8 = _seedToPkcs8(Buffer.from(derived));
  const privateKey = crypto.createPrivateKey({ key: pkcs8, format: 'der', type: 'pkcs8' });
  const publicKey = crypto.createPublicKey(privateKey);
  return {
    privateKey: pkcs8.toString('hex'),
    publicKey: publicKey.export({ type: 'spki', format: 'der' }).toString('hex'),
    derived: true,
  };
}

/**
 * Find the account mnemonic to use for sensor key derivation.
 * Tries BTCPC_MNEMONIC, then mnemonic matching the sensor owner (BTCPC_MNEMONIC_<ACCOUNT>).
 */
function _findMnemonic(sensorId) {
  if (process.env.BTCPC_MNEMONIC) return process.env.BTCPC_MNEMONIC;
  // Try account-specific mnemonic: sensor "shindevlin/foo" → BTCPC_MNEMONIC_SHIN or BTCPC_MNEMONIC_SHINDEVLIN
  const owner = sensorId.split('/')[0];
  if (owner) {
    const key1 = 'BTCPC_MNEMONIC_' + owner.toUpperCase();
    if (process.env[key1]) return process.env[key1];
    // Try short prefix (natoshisakamoto → NATOSHI)
    const prefix = owner.replace(/[^a-z]/g, '').slice(0, 7).toUpperCase();
    const key2 = 'BTCPC_MNEMONIC_' + prefix;
    if (process.env[key2]) return process.env[key2];
  }
  return null;
}

/**
 * Generate a keypair for a sensor.
 *
 * If the device has a mnemonic in env (BTCPC_MNEMONIC or BTCPC_MNEMONIC_<ACCOUNT>),
 * the keypair is derived deterministically — no disk storage needed, survives reinstalls.
 *
 * Otherwise, a random keypair is generated and saved to sensor-keys.json.
 *
 * Returns { publicKey: spki-der-hex, existed: bool, derived: bool }.
 */
function generateKeypair(sensorId) {
  // Check for cached key first (fast path for non-mnemonic devices)
  const store = _load();
  if (store[sensorId] && !store[sensorId].derived) {
    return { publicKey: store[sensorId].publicKey, existed: true, derived: false };
  }

  // Mnemonic-derived key takes precedence
  const mnemonic = _findMnemonic(sensorId);
  if (mnemonic) {
    const kp = _deriveFromMnemonic(mnemonic, sensorId);
    // Cache in memory only — mnemonic devices don't need disk storage
    // (the mnemonic IS the backup), but store for consistency
    if (!store[sensorId] || store[sensorId].publicKey !== kp.publicKey) {
      store[sensorId] = kp;
      try { _save(store); _cache = store; } catch (_) { _cache = store; }
    }
    return { publicKey: kp.publicKey, existed: !!store[sensorId], derived: true };
  }

  // Random keypair for hardware devices with no mnemonic
  const existed = !!store[sensorId];
  if (existed) return { publicKey: store[sensorId].publicKey, existed: true, derived: false };

  const { privateKey, publicKey } = crypto.generateKeyPairSync('ed25519');
  const privHex = privateKey.export({ type: 'pkcs8', format: 'der' }).toString('hex');
  const pubHex = publicKey.export({ type: 'spki', format: 'der' }).toString('hex');

  store[sensorId] = { privateKey: privHex, publicKey: pubHex, derived: false };
  _save(store);
  _cache = store;

  return { publicKey: pubHex, existed: false, derived: false };
}

/**
 * Return the hex-encoded SPKI DER public key for a sensor, or null if not found.
 */
function getPublicKey(sensorId) {
  const store = _load();
  return store[sensorId] ? store[sensorId].publicKey : null;
}

/**
 * Sign a message with the sensor's private key.
 * Returns hex-encoded signature, or null if the sensor has no key.
 */
function sign(sensorId, message) {
  const store = _load();
  const entry = store[sensorId];
  if (!entry) return null;

  const privKey = crypto.createPrivateKey({
    key: Buffer.from(entry.privateKey, 'hex'),
    format: 'der',
    type: 'pkcs8',
  });

  const msg = typeof message === 'string' ? message : JSON.stringify(message);
  const sig = crypto.sign(null, Buffer.from(msg, 'utf8'), privKey);
  return sig.toString('hex');
}

/**
 * Verify an ed25519 signature.
 * @param {string} publicKeyHex — SPKI DER hex
 * @param {string} message — the message string that was signed
 * @param {string} signatureHex — hex-encoded signature
 */
function verify(publicKeyHex, message, signatureHex) {
  try {
    const pubKey = crypto.createPublicKey({
      key: Buffer.from(publicKeyHex, 'hex'),
      format: 'der',
      type: 'spki',
    });
    const msg = typeof message === 'string' ? message : JSON.stringify(message);
    return crypto.verify(null, Buffer.from(msg, 'utf8'), pubKey, Buffer.from(signatureHex, 'hex'));
  } catch (_) {
    return false;
  }
}

/**
 * Build the canonical signing payload for a sensor reading.
 * Optionally includes epoch_hash for stronger chain binding.
 */
function readingPayload(sensorId, value, deviceTimestamp, epochHash) {
  let payload = sensorId + '|' + String(value) + '|' + String(deviceTimestamp);
  if (epochHash) payload += '|' + epochHash;
  return payload;
}

/**
 * Sign a reading payload with the sensor's private key.
 * Returns hex signature or null if no key registered.
 */
function signReading(sensorId, value, deviceTimestamp, epochHash) {
  const payload = readingPayload(sensorId, value, deviceTimestamp, epochHash);
  return sign(sensorId, payload);
}

/**
 * Verify a reading signature.
 * Handles the optional epoch_hash automatically.
 */
function verifyReading(publicKeyHex, sensorId, value, deviceTimestamp, signatureHex, epochHash) {
  const payload = readingPayload(sensorId, value, deviceTimestamp, epochHash);
  return verify(publicKeyHex, payload, signatureHex);
}

/**
 * List all sensor IDs that have keypairs in this keystore.
 */
function listSensors() {
  return Object.keys(_load());
}

/**
 * Delete a sensor's keypair from local storage (use with care).
 */
function deleteKeypair(sensorId) {
  const store = _load();
  if (store[sensorId]) {
    delete store[sensorId];
    _save(store);
    _cache = store;
    return true;
  }
  return false;
}

module.exports = {
  generateKeypair,
  getPublicKey,
  sign,
  verify,
  readingPayload,
  signReading,
  verifyReading,
  listSensors,
  deleteKeypair,
};
