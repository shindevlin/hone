"use strict";

/**
 * HONE Chain Link — Link external wallets to HONE accounts
 * Shin Devlin
 *
 * Users prove ownership of external wallets (ETH, Solana, etc.) by
 * signing a challenge message. No private keys are shared — just a
 * signature that proves the user controls the address.
 *
 * Supported chains:
 *   EVM (Ethereum, Base, Arbitrum, Optimism) — secp256k1 + keccak256
 *   Solana — ed25519
 *   Bitcoin — secp256k1 compact signed message
 *   TON — ed25519
 */

var crypto = require("crypto");
var secp256k1 = require("secp256k1");
var { keccak256 } = require("js-sha3");
var bs58 = require("bs58");
var { bech32 } = require("bech32");

// Phase E: LedgerEntry model deleted — chain link entries go via ledger service
// and in-memory linked-address cache (stateStore account_data.chain_addresses).

// In-memory linked-address cache: username → { chain → [addresses] }
// Populated from stateStore on read, updated on write.
var linkedAddressCache = new Map();

// Active challenges: Map<challengeId, { username, chain, address, challenge, expiresAt }>
var pendingChallenges = new Map();

function normalizeClaimedAddress(chain, address) {
  var value = String(address || "").trim();
  return chain === "evm" || chain === "bitcoin" || chain === "ton" ? value.toLowerCase() : value;
}

// ─── Challenge Generation ────────────────────────────────────────

/**
 * Generate a link challenge for a user.
 * The user must sign this exact message with their external wallet.
 *
 * @param {string} username — HONE account
 * @param {string} chain — "evm", "solana", "bitcoin"
 * @param {string} address — the external wallet address to link
 * @returns {{ challengeId, message, expiresIn }}
 */
function generateChallenge(username, chain, address) {
  var challengeId = crypto.randomBytes(16).toString("hex");
  var message = "HONE-LINK:" + username + ":" + chain + ":" + address + ":" + challengeId;
  var expiresAt = Date.now() + 600000; // 10 minutes

  pendingChallenges.set(challengeId, {
    username: username,
    chain: chain,
    address: normalizeClaimedAddress(chain, address),
    message: message,
    expiresAt: expiresAt
  });

  // Cleanup old challenges
  for (var entry of pendingChallenges) {
    if (entry[1].expiresAt < Date.now()) pendingChallenges.delete(entry[0]);
  }

  return {
    challengeId: challengeId,
    message: message,
    expiresIn: 600
  };
}

// ─── EVM Signature Verification ──────────────────────────────────

/**
 * Recover an Ethereum address from a signed message.
 * Implements EIP-191 personal_sign recovery.
 *
 * @param {string} message — the original message
 * @param {string} signature — hex-encoded signature (130 chars = 65 bytes)
 * @returns {string} recovered address (lowercase, 0x-prefixed)
 */
function recoverEVMAddress(message, signature) {
  // EIP-191: "\x19Ethereum Signed Message:\n" + length + message
  var prefix = "\x19Ethereum Signed Message:\n" + message.length + message;
  var msgHash = Buffer.from(keccak256(Buffer.from(prefix)), "hex");

  // Parse signature
  var sigBuf = Buffer.from(signature.replace("0x", ""), "hex");
  if (sigBuf.length !== 65) throw new Error("Signature must be 65 bytes");

  var r = sigBuf.slice(0, 32);
  var s = sigBuf.slice(32, 64);
  var v = sigBuf[64];

  // Normalize v (MetaMask uses 27/28, some wallets use 0/1)
  var recovery = v >= 27 ? v - 27 : v;
  if (recovery !== 0 && recovery !== 1) throw new Error("Invalid recovery byte: " + v);

  var sigOnly = Buffer.concat([r, s]);

  // Recover public key
  var pubKey = secp256k1.ecdsaRecover(sigOnly, recovery, msgHash, false);

  // Address = last 20 bytes of keccak256(pubkey without prefix)
  var pubKeyHash = keccak256(Buffer.from(pubKey.slice(1)));
  var address = "0x" + pubKeyHash.slice(-40);

  return address.toLowerCase();
}

// ─── Non-EVM Signature Verification ───────────────────────────────

function decodeFlexible(input, encodings) {
  var value = String(input || "").trim();
  for (var i = 0; i < encodings.length; i++) {
    try {
      if (encodings[i] === "hex") return Buffer.from(value.replace(/^0x/, ""), "hex");
      if (encodings[i] === "base58") return Buffer.from(bs58.decode(value));
      if (encodings[i] === "base64") return Buffer.from(value.replace(/-/g, "+").replace(/_/g, "/"), "base64");
    } catch (_) {}
  }
  throw new Error("Unable to decode signature/public key");
}

function makeEd25519PublicKey(publicKeyBytes) {
  if (publicKeyBytes.length !== 32) throw new Error("ed25519 public key must be 32 bytes");
  var spkiPrefix = Buffer.from("302a300506032b6570032100", "hex");
  return crypto.createPublicKey({
    key: Buffer.concat([spkiPrefix, publicKeyBytes]),
    format: "der",
    type: "spki"
  });
}

function verifyEd25519(message, signatureBytes, publicKeyBytes) {
  return crypto.verify(null, Buffer.from(message), makeEd25519PublicKey(publicKeyBytes), signatureBytes);
}

function recoverSolanaAddress(message, signature, address) {
  var publicKeyBytes = Buffer.from(bs58.decode(address));
  var signatureBytes = decodeFlexible(signature, ["base58", "base64", "hex"]);
  if (!verifyEd25519(message, signatureBytes, publicKeyBytes)) {
    throw new Error("Invalid Solana signature");
  }
  return bs58.encode(publicKeyBytes);
}

function parseTonPublicKey(address) {
  var value = String(address || "").trim().replace(/^ton:/i, "");
  var decoders = [
    function () { return Buffer.from(value.replace(/^0x/, ""), "hex"); },
    function () { return Buffer.from(bs58.decode(value)); },
    function () { return Buffer.from(value.replace(/-/g, "+").replace(/_/g, "/"), "base64"); }
  ];
  for (var i = 0; i < decoders.length; i++) {
    try {
      var key = decoders[i]();
      if (key.length === 32) return key;
    } catch (_) {}
  }
  throw new Error("TON address must include a 32-byte public key for experimental linking");
}

function recoverTONAddress(message, signature, address) {
  var publicKeyBytes = parseTonPublicKey(address);
  var signatureBytes = decodeFlexible(signature, ["base64", "base58", "hex"]);
  if (!verifyEd25519(message, signatureBytes, publicKeyBytes)) {
    throw new Error("Invalid TON signature");
  }
  return String(address).trim().toLowerCase();
}

function varIntBuffer(length) {
  if (length < 0xfd) return Buffer.from([length]);
  if (length <= 0xffff) {
    var small = Buffer.alloc(3);
    small[0] = 0xfd;
    small.writeUInt16LE(length, 1);
    return small;
  }
  var large = Buffer.alloc(5);
  large[0] = 0xfe;
  large.writeUInt32LE(length, 1);
  return large;
}

function bitcoinMessageHash(message) {
  var msg = Buffer.from(message);
  var payload = Buffer.concat([
    Buffer.from("\x18Bitcoin Signed Message:\n", "binary"),
    varIntBuffer(msg.length),
    msg
  ]);
  var first = crypto.createHash("sha256").update(payload).digest();
  return crypto.createHash("sha256").update(first).digest();
}

function hash160(buffer) {
  var sha = crypto.createHash("sha256").update(buffer).digest();
  return crypto.createHash("ripemd160").update(sha).digest();
}

function pubkeyToBitcoinBech32(pubKey) {
  var words = bech32.toWords(hash160(Buffer.from(pubKey)));
  words.unshift(0);
  return bech32.encode("bc", words);
}

function recoverBitcoinAddress(message, signature) {
  var sigBuf = decodeFlexible(signature, ["base64", "hex"]);
  if (sigBuf.length !== 65) throw new Error("Bitcoin compact signature must be 65 bytes");

  var header = sigBuf[0];
  if (header < 27 || header > 42) throw new Error("Invalid Bitcoin compact signature header");
  var recovery = (header - 27) & 3;
  var compressed = header >= 31;
  var sigOnly = sigBuf.slice(1);
  var msgHash = bitcoinMessageHash(message);
  var pubKey = secp256k1.ecdsaRecover(sigOnly, recovery, msgHash, compressed);
  return pubkeyToBitcoinBech32(pubKey).toLowerCase();
}

// ─── Verification & Linking ──────────────────────────────────────

/**
 * Verify a signed challenge and link the address to the HONE account.
 *
 * @param {string} challengeId
 * @param {string} signature — hex signature from the external wallet
 * @returns {{ success, username, chain, address, error }}
 */
async function verifyAndLink(challengeId, signature) {
  var challenge = pendingChallenges.get(challengeId);
  if (!challenge) {
    return { success: false, error: "Challenge not found or expired" };
  }

  if (Date.now() > challenge.expiresAt) {
    pendingChallenges.delete(challengeId);
    return { success: false, error: "Challenge expired" };
  }

  var recoveredAddress;

  try {
    if (challenge.chain === "evm") {
      recoveredAddress = recoverEVMAddress(challenge.message, signature);
    } else if (challenge.chain === "solana") {
      recoveredAddress = recoverSolanaAddress(challenge.message, signature, challenge.address);
    } else if (challenge.chain === "bitcoin") {
      recoveredAddress = recoverBitcoinAddress(challenge.message, signature);
    } else if (challenge.chain === "ton") {
      recoveredAddress = recoverTONAddress(challenge.message, signature, challenge.address);
    } else if (challenge.chain === "reddit") {
      // Reddit linking: signature must be HMAC(challengeId, DEVVIT_SECRET).
      // Only the Devvit app knows the secret, proving the request came from Reddit context.
      var devvitSecret = process.env.HONE_DEVVIT_SECRET;
      if (!devvitSecret) {
        return { success: false, error: "Reddit linking not configured (HONE_DEVVIT_SECRET missing)" };
      }
      var expectedSig = crypto.createHmac("sha256", devvitSecret).update(challengeId).digest("hex");
      if (signature === expectedSig) {
        recoveredAddress = challenge.address; // Reddit username
      } else {
        return { success: false, error: "Invalid Reddit verification signature" };
      }
    } else {
      return { success: false, error: "Chain '" + challenge.chain + "' verification not yet supported" };
    }
  } catch (err) {
    return { success: false, error: "Signature verification failed: " + err.message };
  }

  // Check recovered address matches the claimed address
  if (normalizeClaimedAddress(challenge.chain, recoveredAddress) !== normalizeClaimedAddress(challenge.chain, challenge.address)) {
    return {
      success: false,
      error: "Address mismatch. Expected " + challenge.address + ", recovered " + recoveredAddress
    };
  }

  // Record the link on the permanent ledger via the ledger service
  var ledger = require("./ledger");
  var epoch = await ledger.getCurrentEpoch();

  try {
    await ledger.recordAccountCreate(
      challenge.username,
      {}, // no new public keys
      { [challenge.chain]: recoveredAddress },
      epoch,
      "chain-link:" + challenge.chain + ":" + recoveredAddress
    );
  } catch (_) {
    // Non-fatal: link is still cached in-memory and on User doc
  }

  // Update in-memory cache
  var cached = linkedAddressCache.get(challenge.username) || {};
  if (!cached[challenge.chain]) cached[challenge.chain] = [];
  if (cached[challenge.chain].indexOf(recoveredAddress) === -1) {
    cached[challenge.chain].push(recoveredAddress);
  }
  linkedAddressCache.set(challenge.username, cached);

  // Also update the User model if it exists
  try {
    var User = require("../models/User");
    var user = await User.findOne({ username: challenge.username });
    if (user) {
      if (!user.linkedAddresses) user.linkedAddresses = {};
      if (!user.linkedAddresses[challenge.chain]) user.linkedAddresses[challenge.chain] = [];
      if (user.linkedAddresses[challenge.chain].indexOf(recoveredAddress) === -1) {
        user.linkedAddresses[challenge.chain].push(recoveredAddress);
      }
      user.markModified("linkedAddresses");
      await user.save();
    }
  } catch (_) {}

  // Cleanup
  pendingChallenges.delete(challengeId);

  return {
    success: true,
    username: challenge.username,
    chain: challenge.chain,
    address: recoveredAddress
  };
}

/**
 * Get all linked addresses for a user.
 * Phase E: reads from in-memory cache (populated from stateStore account_data).
 * Falls back to User.linkedAddresses if cache is empty.
 */
async function getLinkedAddresses(username) {
  // Check in-memory cache first
  var cached = linkedAddressCache.get(username);
  if (cached) return cached;

  // Try stateStore account chain_addresses
  try {
    var stateStore = require("../chain/stateStore");
    var acct = stateStore.getAccount(username);
    if (acct && acct.chain_addresses) {
      // Build linked map from chain_addresses (chain → [address])
      var linked = {};
      for (var chain of Object.keys(acct.chain_addresses)) {
        var addr = acct.chain_addresses[chain];
        if (addr) linked[chain] = Array.isArray(addr) ? addr : [addr];
      }
      linkedAddressCache.set(username, linked);
      return linked;
    }
  } catch (_) {}

  // Fall back to User.linkedAddresses
  try {
    var User = require("../models/User");
    var user = await User.findOne({ username: username });
    if (user && user.linkedAddresses) {
      linkedAddressCache.set(username, user.linkedAddresses);
      return user.linkedAddresses;
    }
  } catch (_) {}

  return {};
}

module.exports = {
  generateChallenge: generateChallenge,
  verifyAndLink: verifyAndLink,
  recoverEVMAddress: recoverEVMAddress,
  recoverBitcoinAddress: recoverBitcoinAddress,
  recoverSolanaAddress: recoverSolanaAddress,
  recoverTONAddress: recoverTONAddress,
  getLinkedAddresses: getLinkedAddresses
};
