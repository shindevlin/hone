"use strict";

/**
 * BTCPC Sensor Keystore
 * Shin Devlin
 *
 * ed25519 keypair management for IoT sensors.
 *
 * Key derivation modes (tried in order):
 *
 *   1. HARDWARE-DERIVED (Android/Termux phones)
 *      Reads android_id + device serial from the OS — no user input.
 *      Seed = HKDF-SHA512(SHA512(android_id + ":" + serial), info="btcpc-sensor-v1:<sensor_id>")
 *      Same hardware = same key. Survives app reinstall. Resets only on factory reset.
 *
 *   2. RANDOM (dedicated hardware: Hyfix, ESP32, Flipper Zero, laptop nodes)
 *      Random ed25519 keypair generated once and stored in sensor-keys.json.
 *      If storage is lost → owner rotates key via POST /api/sensors/:id/register-key.
 *
 * The mnemonic is NOT used for sensor keys. Sensor identity is hardware-bound,
 * wallet identity is mnemonic-bound. They are separate.
 *
 * Key storage: ~/.btcpc/sensor-keys.json (or BTCPC_SENSOR_KEYS env var)
 * BTCPC_DATA_DIR is only used if that path exists on the current machine,
 * so Termux with a grouchly-specific mount falls back to ~/.btcpc safely.
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
  const envDir = process.env.BTCPC_DATA_DIR;
  if (envDir && fs.existsSync(envDir)) return path.join(envDir, 'sensor-keys.json');
  return path.join(os.homedir(), '.btcpc', 'sensor-keys.json');
}

const KEYSTORE_PATH = _resolveKeystorePath();
let _cache = null;

function _load() {
  if (_cache) return _cache;
  try { _cache = JSON.parse(fs.readFileSync(KEYSTORE_PATH, 'utf8')); }
  catch (_) { _cache = {}; }
  return _cache;
}

function _save(store) {
  const dir = path.dirname(KEYSTORE_PATH);
  if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(KEYSTORE_PATH, JSON.stringify(store, null, 2), 'utf8');
}

// PKCS8 DER wrapper for a raw 32-byte ed25519 seed.
function _seedToPkcs8(seed32) {
  // SEQUENCE { INTEGER 0, SEQUENCE { OID id-Ed25519 }, OCTET STRING { OCTET STRING seed } }
  const header = Buffer.from('302e020100300506032b657004220420', 'hex');
  return Buffer.concat([header, seed32]);
}

// ── Hardware seed (Android / Termux) ─────────────────────────────────────────

const _isAndroid = !!process.env.PREFIX && process.env.PREFIX.includes('com.termux')
  || fs.existsSync('/system/build.prop');

let _hwSeedCache = null;

/**
 * Read stable hardware identifiers from the Android OS.
 * Returns a string like "6dc17d4cb642f08c:ZL8322X6XW" or null on non-Android.
 * Result is cached for the process lifetime.
 */
function _getHardwareMaterial() {
  if (!_isAndroid) return null;
  if (_hwSeedCache !== null) return _hwSeedCache;

  try {
    const { execSync } = require('child_process');
    // android_id: stable per-device-per-user, survives reinstall, resets on factory reset
    const androidId = execSync('settings get secure android_id', { timeout: 3000 })
      .toString().trim();
    if (!androidId || androidId === 'null' || androidId.length < 8) {
      _hwSeedCache = null;
      return null;
    }

    // Serial: hardware serial number — most stable identifier
    let serial = '';
    try {
      serial = execSync('getprop ro.serialno', { timeout: 3000 }).toString().trim();
      if (serial === 'unknown' || serial.length < 4) serial = '';
    } catch (_) {}

    // Model fingerprint — adds specificity when serial is unavailable
    let model = '';
    try {
      model = execSync('getprop ro.build.fingerprint', { timeout: 3000 }).toString().trim();
    } catch (_) {}

    _hwSeedCache = androidId + ':' + serial + ':' + model;
    return _hwSeedCache;
  } catch (_) {
    _hwSeedCache = null;
    return null;
  }
}

/**
 * Derive an ed25519 keypair from hardware material + sensor_id.
 * Deterministic: same hardware + same sensor_id = same key, always.
 */
function _deriveFromHardware(hwMaterial, sensorId) {
  const hwHash = crypto.createHash('sha512').update(hwMaterial).digest();
  const derived = crypto.hkdfSync(
    'sha512',
    hwHash,
    Buffer.alloc(32),                                      // salt
    Buffer.from('btcpc-sensor-v1:' + sensorId),            // info
    32                                                     // output length
  );
  const pkcs8 = _seedToPkcs8(Buffer.from(derived));
  const privateKey = crypto.createPrivateKey({ key: pkcs8, format: 'der', type: 'pkcs8' });
  const publicKey = crypto.createPublicKey(privateKey);
  return {
    privateKey: pkcs8.toString('hex'),
    publicKey: publicKey.export({ type: 'spki', format: 'der' }).toString('hex'),
    derived_from: 'hardware',
  };
}

// ── Public API ────────────────────────────────────────────────────────────────

/**
 * Generate or load the keypair for a sensor.
 *
 * On Android phones: derived from android_id + serial. No disk write needed,
 * but we cache to sensor-keys.json for fast reads and cross-process consistency.
 *
 * On other devices: random keypair stored in sensor-keys.json.
 *
 * Returns { publicKey: spki-der-hex, existed: bool, derived_from: 'hardware'|'random' }.
 */
function generateKeypair(sensorId) {
  const hwMaterial = _getHardwareMaterial();

  if (hwMaterial) {
    // Hardware-derived: always reproducible, check if already cached
    const store = _load();
    const kp = _deriveFromHardware(hwMaterial, sensorId);
    const existed = !!(store[sensorId] && store[sensorId].publicKey === kp.publicKey);
    if (!existed) {
      store[sensorId] = kp;
      try { _save(store); _cache = store; } catch (_) { _cache = store; }
    }
    return { publicKey: kp.publicKey, existed, derived_from: 'hardware' };
  }

  // Random keypair for dedicated hardware / servers
  const store = _load();
  if (store[sensorId]) {
    return { publicKey: store[sensorId].publicKey, existed: true, derived_from: 'random' };
  }
  const { privateKey, publicKey } = crypto.generateKeyPairSync('ed25519');
  const kp = {
    privateKey: privateKey.export({ type: 'pkcs8', format: 'der' }).toString('hex'),
    publicKey: publicKey.export({ type: 'spki', format: 'der' }).toString('hex'),
    derived_from: 'random',
  };
  store[sensorId] = kp;
  _save(store);
  _cache = store;
  return { publicKey: kp.publicKey, existed: false, derived_from: 'random' };
}

/**
 * Return the hex-encoded SPKI DER public key for a sensor, or null if not generated yet.
 */
function getPublicKey(sensorId) {
  const hw = _getHardwareMaterial();
  if (hw) return _deriveFromHardware(hw, sensorId).publicKey;
  const store = _load();
  return store[sensorId] ? store[sensorId].publicKey : null;
}

/**
 * Sign a message with the sensor's private key.
 * Returns hex-encoded signature, or null if no key exists.
 */
function sign(sensorId, message) {
  let privHex = null;

  const hw = _getHardwareMaterial();
  if (hw) {
    privHex = _deriveFromHardware(hw, sensorId).privateKey;
  } else {
    const store = _load();
    privHex = store[sensorId] ? store[sensorId].privateKey : null;
  }

  if (!privHex) return null;
  const privKey = crypto.createPrivateKey({ key: Buffer.from(privHex, 'hex'), format: 'der', type: 'pkcs8' });
  const msg = typeof message === 'string' ? message : JSON.stringify(message);
  return crypto.sign(null, Buffer.from(msg, 'utf8'), privKey).toString('hex');
}

/**
 * Verify an ed25519 signature.
 * @param {string} publicKeyHex — SPKI DER hex
 * @param {string} message
 * @param {string} signatureHex
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

/** Canonical signing payload for a reading. */
function readingPayload(sensorId, value, deviceTimestamp, epochHash) {
  let p = sensorId + '|' + String(value) + '|' + String(deviceTimestamp);
  if (epochHash) p += '|' + epochHash;
  return p;
}

function signReading(sensorId, value, deviceTimestamp, epochHash) {
  return sign(sensorId, readingPayload(sensorId, value, deviceTimestamp, epochHash));
}

function verifyReading(publicKeyHex, sensorId, value, deviceTimestamp, signatureHex, epochHash) {
  return verify(publicKeyHex, readingPayload(sensorId, value, deviceTimestamp, epochHash), signatureHex);
}

function listSensors() {
  return Object.keys(_load());
}

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
  _getHardwareMaterial,  // exported for diagnostics
};
