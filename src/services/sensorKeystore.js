"use strict";

/**
 * BTCPC Sensor Keystore
 * Shin Devlin
 *
 * ed25519 keypair management for IoT sensors. Each sensor gets a unique
 * keypair at registration. The private key stays on the device (or on
 * the node running the sensor daemon). The public key is registered
 * on-chain via SENSOR_KEY_REGISTER so readings can be verified.
 *
 * Key storage: ~/.btcpc/sensor-keys.json (or BTCPC_SENSOR_KEYS env var)
 * Format: { "<sensor_id>": { privateKey: "<pkcs8-der-hex>", publicKey: "<spki-der-hex>" } }
 *
 * Signing payload format (pipe-delimited for cross-platform compatibility):
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

const KEYSTORE_PATH = process.env.BTCPC_SENSOR_KEYS ||
  path.join(process.env.BTCPC_DATA_DIR || path.join(os.homedir(), '.btcpc'), 'sensor-keys.json');

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

/**
 * Generate a new ed25519 keypair for a sensor, or return the existing one.
 * Returns { publicKey: spki-der-hex, existed: bool }.
 */
function generateKeypair(sensorId) {
  const store = _load();
  if (store[sensorId]) {
    return { publicKey: store[sensorId].publicKey, existed: true };
  }

  const { privateKey, publicKey } = crypto.generateKeyPairSync('ed25519');
  const privHex = privateKey.export({ type: 'pkcs8', format: 'der' }).toString('hex');
  const pubHex = publicKey.export({ type: 'spki', format: 'der' }).toString('hex');

  store[sensorId] = { privateKey: privHex, publicKey: pubHex };
  _save(store);
  _cache = store;

  return { publicKey: pubHex, existed: false };
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
