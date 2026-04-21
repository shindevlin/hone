"use strict";

/**
 * BTCPC-FS Storage Crypto — ECIES key wrapping + AES-256-GCM manifest encryption
 * Shin Devlin
 *
 * Two storage modes:
 *
 * Standard (FILE_STORE):
 *   generateDEK / wrapDEK / unwrapDEK / encryptManifest / decryptManifest
 *   Single encrypted manifest, DEK wrapped for owner's memo key + any grantees.
 *
 * Quantum-resistant split (FILE_SHARD):
 *   The manifest ciphertext is split into two byte-halves. The DEK is XOR-split
 *   into shares S1 + S2. Each half's location + its DEK share is wrapped with a
 *   DIFFERENT key — posting key for shard A, memo key for shard B — and written
 *   as two separate, unlinkable FILE_SHARD chain entries. A pair_id ties them
 *   client-side only; nothing on chain connects the two entries.
 *
 *   Quantum resistance property: a quantum adversary must break every posting key
 *   AND every memo key across the entire chain, then solve the combinatorial
 *   pairing problem across all resulting plaintext blobs — with no on-chain
 *   metadata to narrow the search. The information needed to reconstruct any
 *   single file is never co-located in one place.
 *
 *   wrapJSON / unwrapJSON — general ECIES for arbitrary pointer structs
 *   splitDEK / mergeDEK   — XOR key splitting
 */

const crypto = require("crypto");
const secp256k1 = require("secp256k1");
const inferenceCrypto = require("../inference/crypto");

function generateDEK() {
  return crypto.randomBytes(32);
}

/**
 * ECIES-wrap a 32-byte key for a recipient's secp256k1 memo public key.
 * Produces an opaque object the recipient unwraps with their memo private key.
 */
function wrapDEK(dek, recipientMemoPubHex) {
  if (!Buffer.isBuffer(dek) || dek.length !== 32) throw new Error("wrapDEK: dek must be 32-byte Buffer");
  let ephemPriv;
  do { ephemPriv = crypto.randomBytes(32); }
  while (!secp256k1.privateKeyVerify(ephemPriv));

  const ephemPub = Buffer.from(secp256k1.publicKeyCreate(ephemPriv, true));
  const shared = inferenceCrypto.computeSharedSecret(ephemPriv.toString("hex"), recipientMemoPubHex);
  const wrapped = inferenceCrypto.encrypt(dek.toString("hex"), shared);
  return {
    ephem_pub: ephemPub.toString("hex"),
    ciphertext: wrapped.ciphertext,
    iv: wrapped.iv,
    tag: wrapped.tag,
  };
}

/**
 * Unwrap a DEK using the recipient's memo private key.
 * Returns a 32-byte Buffer.
 */
function unwrapDEK(wrappedDEK, myMemoPrivHex) {
  const shared = inferenceCrypto.computeSharedSecret(myMemoPrivHex, wrappedDEK.ephem_pub);
  const dekHex = inferenceCrypto.decrypt(
    { ciphertext: wrappedDEK.ciphertext, iv: wrappedDEK.iv, tag: wrappedDEK.tag },
    shared
  );
  return Buffer.from(dekHex, "hex");
}

/**
 * Encrypt a manifest object with a DEK. Returns {ciphertext, iv, tag}.
 */
function encryptManifest(manifest, dek) {
  if (!Buffer.isBuffer(dek) || dek.length !== 32) throw new Error("encryptManifest: dek must be 32-byte Buffer");
  return inferenceCrypto.encrypt(JSON.stringify(manifest), dek);
}

/**
 * Decrypt a manifest with a DEK. Returns parsed object.
 */
function decryptManifest(encryptedManifest, dek) {
  if (!Buffer.isBuffer(dek) || dek.length !== 32) throw new Error("decryptManifest: dek must be 32-byte Buffer");
  return JSON.parse(inferenceCrypto.decrypt(encryptedManifest, dek));
}

/**
 * General ECIES wrap for any JSON-serialisable data (used for shard pointers).
 * Works with any secp256k1 public key — posting or memo.
 */
function wrapJSON(data, recipientPubHex) {
  let ephemPriv;
  do { ephemPriv = crypto.randomBytes(32); }
  while (!secp256k1.privateKeyVerify(ephemPriv));

  const ephemPub = Buffer.from(secp256k1.publicKeyCreate(ephemPriv, true));
  const shared = inferenceCrypto.computeSharedSecret(ephemPriv.toString("hex"), recipientPubHex);
  const wrapped = inferenceCrypto.encrypt(JSON.stringify(data), shared);
  return {
    ephem_pub: ephemPub.toString("hex"),
    ciphertext: wrapped.ciphertext,
    iv: wrapped.iv,
    tag: wrapped.tag,
  };
}

function unwrapJSON(wrappedData, myPrivHex) {
  const shared = inferenceCrypto.computeSharedSecret(myPrivHex, wrappedData.ephem_pub);
  return JSON.parse(inferenceCrypto.decrypt(
    { ciphertext: wrappedData.ciphertext, iv: wrappedData.iv, tag: wrappedData.tag },
    shared
  ));
}

/**
 * XOR-split a 32-byte DEK into two shares. Neither share alone is the DEK.
 * Returns [s1, s2] — both are 32-byte Buffers.
 */
function splitDEK(dek) {
  if (!Buffer.isBuffer(dek) || dek.length !== 32) throw new Error("splitDEK: dek must be 32-byte Buffer");
  const s1 = crypto.randomBytes(32);
  const s2 = Buffer.alloc(32);
  for (let i = 0; i < 32; i++) s2[i] = dek[i] ^ s1[i];
  return [s1, s2];
}

/**
 * Reconstruct DEK from two XOR shares.
 */
function mergeDEK(s1, s2) {
  if (s1.length !== 32 || s2.length !== 32) throw new Error("mergeDEK: both shares must be 32 bytes");
  const dek = Buffer.alloc(32);
  for (let i = 0; i < 32; i++) dek[i] = s1[i] ^ s2[i];
  return dek;
}

module.exports = {
  generateDEK,
  wrapDEK, unwrapDEK,
  encryptManifest, decryptManifest,
  wrapJSON, unwrapJSON,
  splitDEK, mergeDEK,
};
