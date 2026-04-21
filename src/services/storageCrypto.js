"use strict";

/**
 * BTCPC-FS Storage Crypto — ECIES key wrapping + AES-256-GCM manifest encryption
 * Shin Devlin
 *
 * Encrypted private storage for BTCPC-FS:
 *   1. generateDEK()         — random 32-byte data encryption key per file
 *   2. wrapDEK(dek, pub)     — ECIES: ephemeral ECDH + AES-256-GCM wraps the DEK
 *   3. unwrapDEK(wrap, priv) — recipient unwraps with their memo private key
 *   4. encryptManifest / decryptManifest — wrap/unwrap the chunk manifest JSON
 *
 * The server generates DEK and wraps it for the owner's memo public key.
 * The server never stores the plaintext DEK — once wrapped and written to chain,
 * only the key holder can recover it. Adding grantees requires the owner to
 * unwrap locally (CLI/app), re-wrap for grantee, and POST the new grant.
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

module.exports = { generateDEK, wrapDEK, unwrapDEK, encryptManifest, decryptManifest };
