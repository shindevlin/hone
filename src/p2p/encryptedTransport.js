"use strict";

/**
 * BTCPC Encrypted P2P Transport — Noise_XX with ChaCha20-Poly1305
 * Shin Devlin
 *
 * Implements the Noise_XX handshake pattern using Node.js built-in crypto.
 * No external dependencies. Works on Linux, macOS, Windows, ARM, and Electron.
 *
 * Noise_XX provides mutual authentication where neither party's static key
 * is known to the other in advance — the correct pattern for open P2P networks.
 *
 * Handshake pattern (Noise_XX):
 *   -> e
 *   <- e, ee, s, es
 *   -> s, se
 *   [Transport messages with split CipherState pair]
 *
 * Cipher suite: Noise_XX_25519_ChaChaPoly_SHA256
 *   - DH:   X25519 (Node 15+ crypto.generateKeyPairSync('x25519'))
 *   - AEAD: ChaCha20-Poly1305 (Node 15+ crypto.createCipheriv)
 *   - Hash: SHA-256
 *
 * Wire format:
 *   - Handshake messages: 4-byte big-endian length prefix + Noise handshake payload
 *   - Transport messages: 4-byte big-endian length prefix + AEAD ciphertext
 *
 * References:
 *   https://noiseprotocol.org/noise.html
 *   https://noiseprotocol.org/noise.html#the-xx-pattern
 */

const crypto = require("crypto");
const path = require("path");
const fs = require("fs");

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const NOISE_MAX_MESSAGE = 65535;
const NONCE_MAX = 0xffffffffffffffff; // 2^64 - 1
const TAG_LEN = 16;
const KEY_LEN = 32;
const HASH_LEN = 32;
const DHLEN = 32;
const PROLOGUE = Buffer.from("btcpc-p2p-v1");

// Static keypair storage path
const STATIC_KEY_PATH = path.join(
  process.env.HOME || process.env.USERPROFILE || "/root",
  ".btcpc", "noise-static-key.json"
);

// ---------------------------------------------------------------------------
// DH functions (X25519)
// ---------------------------------------------------------------------------

function generateKeypair() {
  const { privateKey, publicKey } = crypto.generateKeyPairSync("x25519", {
    privateKeyEncoding: { type: "pkcs8", format: "der" },
    publicKeyEncoding: { type: "spki", format: "der" }
  });
  // Extract raw 32 bytes from DER encoding
  const rawPriv = crypto.createPrivateKey({ key: privateKey, format: "der", type: "pkcs8" })
    .export({ type: "pkcs8", format: "der" }).slice(-32);
  const rawPub = publicKey.slice(-32);
  return { private: rawPriv, public: rawPub };
}

function dh(keypair, remotePublicRaw) {
  // Reconstruct X25519 private key object from raw bytes (pkcs8 DER)
  const privKeyObj = crypto.createPrivateKey({
    key: Buffer.concat([
      Buffer.from("302e020100300506032b656e04220420", "hex"), // X25519 pkcs8 header
      keypair.private
    ]),
    format: "der",
    type: "pkcs8"
  });
  const pubKeyObj = crypto.createPublicKey({
    key: Buffer.concat([
      Buffer.from("302a300506032b656e032100", "hex"), // X25519 spki header
      remotePublicRaw
    ]),
    format: "der",
    type: "spki"
  });
  return crypto.diffieHellman({ privateKey: privKeyObj, publicKey: pubKeyObj });
}

// ---------------------------------------------------------------------------
// AEAD functions (ChaCha20-Poly1305)
// ---------------------------------------------------------------------------

function encryptAEAD(key, nonce, ad, plaintext) {
  const nonceBuf = nonceToBuffer(nonce);
  const cipher = crypto.createCipheriv("chacha20-poly1305", key, nonceBuf, { authTagLength: TAG_LEN });
  cipher.setAAD(ad);
  const ct = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  const tag = cipher.getAuthTag();
  return Buffer.concat([ct, tag]);
}

function decryptAEAD(key, nonce, ad, ciphertextWithTag) {
  const nonceBuf = nonceToBuffer(nonce);
  const ct = ciphertextWithTag.subarray(0, ciphertextWithTag.length - TAG_LEN);
  const tag = ciphertextWithTag.subarray(ciphertextWithTag.length - TAG_LEN);
  const decipher = crypto.createDecipheriv("chacha20-poly1305", key, nonceBuf, { authTagLength: TAG_LEN });
  decipher.setAuthTag(tag);
  decipher.setAAD(ad);
  return Buffer.concat([decipher.update(ct), decipher.final()]);
}

function nonceToBuffer(n) {
  // Noise spec: 12-byte nonce, little-endian 8-byte counter at bytes 4-11
  const buf = Buffer.alloc(12, 0);
  // n is a JS number (safe integer range: up to 2^53)
  const lo = n >>> 0;
  const hi = Math.floor(n / 0x100000000) >>> 0;
  buf.writeUInt32LE(lo, 4);
  buf.writeUInt32LE(hi, 8);
  return buf;
}

// ---------------------------------------------------------------------------
// Hash / HKDF
// ---------------------------------------------------------------------------

function hash(data) {
  return crypto.createHash("sha256").update(data).digest();
}

function hmacHash(key, data) {
  return crypto.createHmac("sha256", key).update(data).digest();
}

function hkdf2(chainingKey, inputKeyMaterial) {
  const temp = hmacHash(chainingKey, inputKeyMaterial);
  const out1 = hmacHash(temp, Buffer.from([0x01]));
  const out2 = hmacHash(temp, Buffer.concat([out1, Buffer.from([0x02])]));
  return [out1, out2];
}

function hkdf3(chainingKey, inputKeyMaterial) {
  const temp = hmacHash(chainingKey, inputKeyMaterial);
  const out1 = hmacHash(temp, Buffer.from([0x01]));
  const out2 = hmacHash(temp, Buffer.concat([out1, Buffer.from([0x02])]));
  const out3 = hmacHash(temp, Buffer.concat([out2, Buffer.from([0x03])]));
  return [out1, out2, out3];
}

// ---------------------------------------------------------------------------
// CipherState
// ---------------------------------------------------------------------------

function CipherState(k) {
  this.k = k ? Buffer.from(k) : null;
  this.n = 0;
}

CipherState.prototype.hasKey = function () { return this.k !== null; };

CipherState.prototype.setNonce = function (n) { this.n = n; };

CipherState.prototype.encryptWithAd = function (ad, plaintext) {
  if (!this.k) return plaintext;
  if (this.n >= NONCE_MAX) throw new Error("Noise: nonce overflow");
  const ct = encryptAEAD(this.k, this.n, ad, plaintext);
  this.n++;
  return ct;
};

CipherState.prototype.decryptWithAd = function (ad, ciphertext) {
  if (!this.k) return ciphertext;
  if (this.n >= NONCE_MAX) throw new Error("Noise: nonce overflow");
  const pt = decryptAEAD(this.k, this.n, ad, ciphertext);
  this.n++;
  return pt;
};

// ---------------------------------------------------------------------------
// SymmetricState
// ---------------------------------------------------------------------------

function SymmetricState(protocolName) {
  const protoBuf = Buffer.from(protocolName, "ascii");
  if (protoBuf.length <= HASH_LEN) {
    this.h = Buffer.alloc(HASH_LEN);
    protoBuf.copy(this.h);
  } else {
    this.h = hash(protoBuf);
  }
  this.ck = Buffer.from(this.h);
  this.cs = new CipherState(null);
}

SymmetricState.prototype.mixKey = function (inputKeyMaterial) {
  const [ck, tempK] = hkdf2(this.ck, inputKeyMaterial);
  this.ck = ck;
  this.cs = new CipherState(tempK.subarray(0, KEY_LEN));
  this.cs.setNonce(0);
};

SymmetricState.prototype.mixHash = function (data) {
  this.h = hash(Buffer.concat([this.h, data]));
};

SymmetricState.prototype.mixKeyAndHash = function (inputKeyMaterial) {
  const [ck, tempH, tempK] = hkdf3(this.ck, inputKeyMaterial);
  this.ck = ck;
  this.mixHash(tempH);
  this.cs = new CipherState(tempK.subarray(0, KEY_LEN));
  this.cs.setNonce(0);
};

SymmetricState.prototype.encryptAndHash = function (plaintext) {
  const ct = this.cs.encryptWithAd(this.h, plaintext);
  this.mixHash(ct);
  return ct;
};

SymmetricState.prototype.decryptAndHash = function (ciphertext) {
  const pt = this.cs.decryptWithAd(this.h, ciphertext);
  this.mixHash(ciphertext);
  return pt;
};

SymmetricState.prototype.split = function () {
  const [tempK1, tempK2] = hkdf2(this.ck, Buffer.alloc(0));
  const cs1 = new CipherState(tempK1.subarray(0, KEY_LEN));
  const cs2 = new CipherState(tempK2.subarray(0, KEY_LEN));
  return [cs1, cs2];
};

// ---------------------------------------------------------------------------
// HandshakeState (Noise_XX)
// ---------------------------------------------------------------------------

const PROTOCOL_NAME = "Noise_XX_25519_ChaChaPoly_SHA256";

/**
 * Create a new handshake state.
 *
 * @param {boolean} initiator — true if this side initiates
 * @param {{ private: Buffer, public: Buffer }} staticKeypair — this node's static keypair
 * @returns HandshakeState object
 */
function createHandshakeState(initiator, staticKeypair) {
  return {
    initiator: initiator,
    s: staticKeypair,             // static keypair
    e: null,                      // ephemeral keypair (generated during handshake)
    rs: null,                     // remote static public key (learned during handshake)
    re: null,                     // remote ephemeral public key
    ss: new SymmetricState(PROTOCOL_NAME),
    messageIndex: 0,              // tracks which step of XX we're on
  };
}

/**
 * Initialize the handshake state (mix prologue).
 */
function initializeHandshake(hs) {
  hs.ss.mixHash(PROLOGUE);
}

/**
 * Write handshake message (initiator/responder alternates).
 *
 * Returns a Buffer containing the handshake payload ready to send.
 * @param {object} hs — HandshakeState
 * @param {Buffer} [payload] — application payload to include (default empty)
 * @returns {{ message: Buffer, complete: boolean, cs1?, cs2? }}
 */
function writeMessage(hs, payload) {
  payload = payload || Buffer.alloc(0);
  var output = [];
  var complete = false;
  var cs1, cs2;

  if (hs.initiator && hs.messageIndex === 0) {
    // -> e
    hs.e = generateKeypair();
    output.push(hs.e.public);
    hs.ss.mixHash(hs.e.public);
    var encPayload = hs.ss.encryptAndHash(payload);
    output.push(encPayload);
    hs.messageIndex++;

  } else if (!hs.initiator && hs.messageIndex === 1) {
    // <- e, ee, s, es
    hs.e = generateKeypair();
    output.push(hs.e.public);
    hs.ss.mixHash(hs.e.public);
    // ee: DH(e, re)
    hs.ss.mixKey(dh(hs.e, hs.re));
    // s: encrypt static public key
    var encS = hs.ss.encryptAndHash(hs.s.public);
    output.push(encS);
    // es: DH(s, re)
    hs.ss.mixKey(dh(hs.s, hs.re));
    var encPayload1 = hs.ss.encryptAndHash(payload);
    output.push(encPayload1);
    hs.messageIndex++;

  } else if (hs.initiator && hs.messageIndex === 2) {
    // -> s, se
    // s: encrypt static public key
    var encS2 = hs.ss.encryptAndHash(hs.s.public);
    output.push(encS2);
    // se: DH(e, rs)
    hs.ss.mixKey(dh(hs.e, hs.rs));
    var encPayload2 = hs.ss.encryptAndHash(payload);
    output.push(encPayload2);
    hs.messageIndex++;
    // Split — handshake complete
    var split = hs.ss.split();
    cs1 = split[0];
    cs2 = split[1];
    complete = true;
  }

  return { message: Buffer.concat(output), complete: complete, cs1: cs1, cs2: cs2 };
}

/**
 * Read a handshake message.
 *
 * @returns {{ payload: Buffer, complete: boolean, remoteStaticKey?: Buffer, cs1?, cs2? }}
 */
function readMessage(hs, message) {
  var offset = 0;
  var complete = false;
  var cs1, cs2;
  var payload;

  if (!hs.initiator && hs.messageIndex === 0) {
    // <- e  (responder reads initiator's first message)
    hs.re = message.subarray(offset, offset + DHLEN);
    offset += DHLEN;
    hs.ss.mixHash(hs.re);
    payload = hs.ss.decryptAndHash(message.subarray(offset));
    hs.messageIndex++;

  } else if (hs.initiator && hs.messageIndex === 1) {
    // -> e, ee, s, es  (initiator reads responder's message)
    hs.re = message.subarray(offset, offset + DHLEN);
    offset += DHLEN;
    hs.ss.mixHash(hs.re);
    // ee: DH(e, re)
    hs.ss.mixKey(dh(hs.e, hs.re));
    // s: decrypt remote static
    var encS = message.subarray(offset, offset + DHLEN + TAG_LEN);
    offset += DHLEN + TAG_LEN;
    hs.rs = hs.ss.decryptAndHash(encS);
    // es: DH(e, rs)
    hs.ss.mixKey(dh(hs.e, hs.rs));
    payload = hs.ss.decryptAndHash(message.subarray(offset));
    hs.messageIndex++;

  } else if (!hs.initiator && hs.messageIndex === 2) {
    // -> s, se  (responder reads initiator's final message)
    var encS2 = message.subarray(offset, offset + DHLEN + TAG_LEN);
    offset += DHLEN + TAG_LEN;
    hs.rs = hs.ss.decryptAndHash(encS2);
    // se: DH(s, e)
    hs.ss.mixKey(dh(hs.s, hs.re));
    payload = hs.ss.decryptAndHash(message.subarray(offset));
    hs.messageIndex++;
    var split = hs.ss.split();
    cs1 = split[0];
    cs2 = split[1];
    complete = true;
  }

  return {
    payload: payload || Buffer.alloc(0),
    complete: complete,
    remoteStaticKey: hs.rs || null,
    cs1: cs1,
    cs2: cs2
  };
}

// ---------------------------------------------------------------------------
// Static keypair persistence
// ---------------------------------------------------------------------------

/**
 * Load or generate the persistent Noise static keypair.
 * Stored in ~/.btcpc/noise-static-key.json.
 * File mode 0600 (user-readable only).
 */
function loadOrGenerateStaticKeypair() {
  try {
    if (fs.existsSync(STATIC_KEY_PATH)) {
      var raw = JSON.parse(fs.readFileSync(STATIC_KEY_PATH, "utf8"));
      var kp = {
        private: Buffer.from(raw.private, "hex"),
        public: Buffer.from(raw.public, "hex")
      };
      if (kp.private.length === 32 && kp.public.length === 32) {
        return kp;
      }
    }
  } catch (_) { /* generate fresh */ }

  var kp = generateKeypair();
  try {
    var dir = path.dirname(STATIC_KEY_PATH);
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
    fs.writeFileSync(STATIC_KEY_PATH, JSON.stringify({
      private: kp.private.toString("hex"),
      public: kp.public.toString("hex"),
      created: new Date().toISOString()
    }), { mode: 0o600 });
    console.log("[BTCPC Noise] Generated new static keypair → " + STATIC_KEY_PATH);
  } catch (err) {
    console.warn("[BTCPC Noise] Could not persist static keypair:", err.message);
  }
  return kp;
}

// ---------------------------------------------------------------------------
// Peer key registry — persist known peer static keys for pinning
// ---------------------------------------------------------------------------

const PEER_KEYS_PATH = path.join(
  process.env.HOME || process.env.USERPROFILE || "/root",
  ".btcpc", "noise-peer-keys.json"
);

var peerKeys = new Map(); // peerId (address:port or nodeId) → hex public key

function loadPeerKeys() {
  try {
    if (fs.existsSync(PEER_KEYS_PATH)) {
      var raw = JSON.parse(fs.readFileSync(PEER_KEYS_PATH, "utf8"));
      for (var k in raw) peerKeys.set(k, raw[k]);
    }
  } catch (_) { /* start fresh */ }
}

function savePeerKeys() {
  try {
    var obj = {};
    for (var entry of peerKeys) obj[entry[0]] = entry[1];
    fs.writeFileSync(PEER_KEYS_PATH, JSON.stringify(obj, null, 2), { mode: 0o600 });
  } catch (_) { /* best-effort */ }
}

function recordPeerKey(peerId, publicKeyHex) {
  var existing = peerKeys.get(peerId);
  if (existing && existing !== publicKeyHex) {
    console.warn("[BTCPC Noise] PEER KEY CHANGE for " + peerId +
      " — old=" + existing.slice(0, 16) + "... new=" + publicKeyHex.slice(0, 16) + "...");
    if (process.env.BTCPC_NOISE_STRICT_KEY_PIN === "true") {
      return false; // reject connection
    }
  }
  peerKeys.set(peerId, publicKeyHex);
  savePeerKeys(); // persist asynchronously in practice; sync here for simplicity
  return true;
}

function getKnownPeerKeys() {
  var result = {};
  for (var entry of peerKeys) result[entry[0]] = entry[1];
  return result;
}

// ---------------------------------------------------------------------------
// Session — wraps a WebSocket with a Noise transport
// ---------------------------------------------------------------------------

/**
 * Per-connection state after handshake completes.
 * { sendCs: CipherState, recvCs: CipherState, remoteStaticKey: Buffer }
 */
var transportState = new WeakMap(); // ws → { sendCs, recvCs, remoteStaticKey, ready }

/**
 * Pending handshake state. Once complete, moves to transportState.
 * WeakMap ws → { hs, buffer: Buffer, initiator, resolver, rejecter }
 */
var handshakeState = new WeakMap();

/**
 * Start the Noise_XX handshake as initiator.
 *
 * @param {WebSocket} ws — outbound connection
 * @param {object} staticKeypair — this node's static keypair
 * @param {string} peerId — identifier for peer key pinning
 * @returns {Promise} resolves when handshake completes
 */
function startHandshake(ws, staticKeypair, peerId) {
  return new Promise(function (resolve, reject) {
    var hs = createHandshakeState(true, staticKeypair);
    initializeHandshake(hs);
    handshakeState.set(ws, {
      hs: hs,
      buffer: Buffer.alloc(0),
      initiator: true,
      peerId: peerId || "",
      resolve: resolve,
      reject: reject
    });

    // Send first message: -> e
    var result = writeMessage(hs);
    _sendRaw(ws, result.message);
  });
}

/**
 * Accept a Noise_XX handshake as responder.
 *
 * @param {WebSocket} ws — inbound connection
 * @param {object} staticKeypair — this node's static keypair
 * @param {string} peerId — identifier for peer key pinning
 * @returns {Promise} resolves when handshake completes
 */
function acceptHandshake(ws, staticKeypair, peerId) {
  return new Promise(function (resolve, reject) {
    var hs = createHandshakeState(false, staticKeypair);
    initializeHandshake(hs);
    handshakeState.set(ws, {
      hs: hs,
      buffer: Buffer.alloc(0),
      initiator: false,
      peerId: peerId || "",
      resolve: resolve,
      reject: reject
    });
    // Responder waits for initiator's first message
  });
}

/**
 * Process incoming data during handshake.
 * Should be called from ws.on('message') until handshake is complete.
 *
 * Returns true if handshake is complete (caller can switch to transport mode).
 */
function processHandshakeData(ws, data, staticKeypair) {
  var state = handshakeState.get(ws);
  if (!state) return false;

  state.buffer = Buffer.concat([state.buffer, Buffer.from(data)]);

  // Try to read a complete length-prefixed message
  while (state.buffer.length >= 4) {
    var msgLen = state.buffer.readUInt32BE(0);
    if (state.buffer.length < 4 + msgLen) break;

    var message = state.buffer.subarray(4, 4 + msgLen);
    state.buffer = state.buffer.subarray(4 + msgLen);

    try {
      var result = readMessage(state.hs, message);

      if (!state.initiator && state.hs.messageIndex === 1) {
        // Responder: read msg 0 (-> e), now send msg 1 (<- e, ee, s, es)
        var resp = writeMessage(state.hs);
        _sendRaw(ws, resp.message);

      } else if (state.initiator && state.hs.messageIndex === 2) {
        // Initiator: read msg 1 (<- e, ee, s, es), now send msg 2 (-> s, se)
        var final = writeMessage(state.hs);
        _sendRaw(ws, final.message);

        // Initiator completes here
        var ok = _finalizeHandshake(ws, state, final.cs1, final.cs2, result.remoteStaticKey);
        if (!ok) return false;
        return true;

      } else if (!state.initiator && result.complete) {
        // Responder: read msg 2 (-> s, se), handshake done
        var ok2 = _finalizeHandshake(ws, state, result.cs1, result.cs2, result.remoteStaticKey);
        if (!ok2) return false;
        return true;
      }
    } catch (err) {
      console.error("[BTCPC Noise] Handshake error:", err.message);
      handshakeState.delete(ws);
      state.reject(err);
      return false;
    }
  }
  return false;
}

function _finalizeHandshake(ws, state, cs1, cs2, remoteStaticKey) {
  // Initiator sends cs1, receives cs2. Responder is the mirror.
  var sendCs = state.initiator ? cs1 : cs2;
  var recvCs = state.initiator ? cs2 : cs1;

  // Key pinning
  if (remoteStaticKey && state.peerId) {
    var ok = recordPeerKey(state.peerId, remoteStaticKey.toString("hex"));
    if (!ok) {
      console.error("[BTCPC Noise] Rejecting peer " + state.peerId + " due to key change (strict mode)");
      handshakeState.delete(ws);
      state.reject(new Error("Peer key changed (strict mode active)"));
      return false;
    }
  }

  transportState.set(ws, {
    sendCs: sendCs,
    recvCs: recvCs,
    remoteStaticKey: remoteStaticKey,
    ready: true
  });

  handshakeState.delete(ws);
  state.resolve({ remoteStaticKey: remoteStaticKey });
  console.log("[BTCPC Noise] Handshake complete with " + (state.peerId || "peer") +
    " remote_key=" + (remoteStaticKey ? remoteStaticKey.toString("hex").slice(0, 16) + "..." : "?"));
  return true;
}

// ---------------------------------------------------------------------------
// Transport send/receive
// ---------------------------------------------------------------------------

/**
 * Send an encrypted transport message.
 *
 * @param {WebSocket} ws
 * @param {string|object} message — will be JSON-stringified if object
 */
function send(ws, message) {
  var ts = transportState.get(ws);
  if (!ts || !ts.ready) {
    throw new Error("Noise transport not ready — handshake incomplete");
  }
  var plaintext = typeof message === "string" ? Buffer.from(message, "utf8") :
    Buffer.isBuffer(message) ? message :
    Buffer.from(JSON.stringify(message), "utf8");

  if (plaintext.length > NOISE_MAX_MESSAGE) {
    throw new Error("Noise: message too large (" + plaintext.length + " > " + NOISE_MAX_MESSAGE + ")");
  }

  var ciphertext = ts.sendCs.encryptWithAd(Buffer.alloc(0), plaintext);
  _sendRaw(ws, ciphertext);
}

/**
 * Decrypt and parse an incoming transport message.
 *
 * @param {WebSocket} ws
 * @param {Buffer} data — raw incoming WebSocket binary frame
 * @returns {string} — decrypted plaintext as string
 */
function receive(ws, data) {
  var ts = transportState.get(ws);
  if (!ts || !ts.ready) {
    throw new Error("Noise transport not ready");
  }
  var buf = Buffer.from(data);
  if (buf.length < 4) throw new Error("Noise: message too short");
  var msgLen = buf.readUInt32BE(0);
  var ciphertext = buf.subarray(4, 4 + msgLen);
  var plaintext = ts.recvCs.decryptWithAd(Buffer.alloc(0), ciphertext);
  return plaintext.toString("utf8");
}

/**
 * Check if the handshake for a connection is complete.
 */
function isReady(ws) {
  var ts = transportState.get(ws);
  return !!(ts && ts.ready);
}

/**
 * Check if a connection is in handshake state.
 */
function isHandshaking(ws) {
  return handshakeState.has(ws);
}

/**
 * Get the remote static public key for an established connection.
 */
function getRemoteStaticKey(ws) {
  var ts = transportState.get(ws);
  return ts ? ts.remoteStaticKey : null;
}

/**
 * Clean up transport state for a closed connection.
 */
function closeTransport(ws) {
  transportState.delete(ws);
  handshakeState.delete(ws);
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

function _sendRaw(ws, payload) {
  var len = Buffer.alloc(4);
  len.writeUInt32BE(payload.length, 0);
  var frame = Buffer.concat([len, payload]);
  try {
    ws.send(frame);
  } catch (_) { /* connection may have closed */ }
}

// ---------------------------------------------------------------------------
// Module init: load static keypair and peer key registry
// ---------------------------------------------------------------------------

var _staticKeypair = null;

function getStaticKeypair() {
  if (!_staticKeypair) {
    _staticKeypair = loadOrGenerateStaticKeypair();
  }
  return _staticKeypair;
}

// Load peer keys on startup
loadPeerKeys();

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  // Handshake
  startHandshake: startHandshake,
  acceptHandshake: acceptHandshake,
  processHandshakeData: processHandshakeData,
  // Transport
  send: send,
  receive: receive,
  isReady: isReady,
  isHandshaking: isHandshaking,
  getRemoteStaticKey: getRemoteStaticKey,
  closeTransport: closeTransport,
  // Static keypair
  getStaticKeypair: getStaticKeypair,
  loadOrGenerateStaticKeypair: loadOrGenerateStaticKeypair,
  // Peer key registry
  recordPeerKey: recordPeerKey,
  getKnownPeerKeys: getKnownPeerKeys,
  // Low-level (for tests)
  createHandshakeState: createHandshakeState,
  initializeHandshake: initializeHandshake,
  writeMessage: writeMessage,
  readMessage: readMessage,
  generateKeypair: generateKeypair,
  dh: dh,
  encryptAEAD: encryptAEAD,
  decryptAEAD: decryptAEAD,
  PROTOCOL_NAME: PROTOCOL_NAME,
};
