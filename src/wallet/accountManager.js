"use strict";

/**
 * accountManager.js — Account creation, recovery, and 2FA management for BTCPC
 * Shin Devlin
 */

const mongoose = require("mongoose");
const crypto = require("crypto");
const keyManager = require("./keyManager");

// Lazy-loaded models (avoids circular require issues at import time)
let _User, _Wallet;

function User() {
  if (!_User) _User = require("../models/User");
  return _User;
}

function Wallet() {
  if (!_Wallet) _Wallet = require("../models/Wallet");
  return _Wallet;
}

// ---------------------------------------------------------------------------
// Username validation
// ---------------------------------------------------------------------------

const USERNAME_RE = /^[a-z][a-z0-9.\-]{2,15}$/;

/**
 * Validate a BTCPC username.
 * Rules: 3-16 chars, lowercase letters/numbers/hyphens/dots, must start with letter.
 * @param {string} username
 * @returns {{ valid: boolean, reason?: string }}
 */
function validateUsername(username) {
  if (!username || typeof username !== "string") {
    return { valid: false, reason: "Username is required" };
  }
  if (username.length < 3 || username.length > 16) {
    return { valid: false, reason: "Username must be 3-16 characters" };
  }
  if (!/^[a-z]/.test(username)) {
    return { valid: false, reason: "Username must start with a lowercase letter" };
  }
  if (!USERNAME_RE.test(username)) {
    return { valid: false, reason: "Username may only contain lowercase letters, numbers, hyphens, and dots" };
  }
  return { valid: true };
}

// ---------------------------------------------------------------------------
// Account Creation
// ---------------------------------------------------------------------------

/**
 * Create a new BTCPC account.
 *
 * 1. Generate a 12-word BIP-39 mnemonic
 * 2. Derive all four role keypairs (owner, active, posting, memo) via BIP-44
 * 3. Derive the 2FA keypair from password + account name via PBKDF2
 * 4. Store user record and wallet with public keys on-chain
 *
 * @param {string} username - Desired account name
 * @param {string} mnemonic - BIP-39 mnemonic (provide one or null to generate)
 * @param {string} password - 2FA password for protocol-level multi-factor auth
 * @returns {Promise<Object>} Account creation result with mnemonic and public keys
 */
async function createAccount(username, mnemonic, password) {
  // Validate username
  const usernameCheck = validateUsername(username);
  if (!usernameCheck.valid) {
    throw new Error(usernameCheck.reason);
  }

  // Check uniqueness
  const existing = await User().findOne({ username: username });
  if (existing) {
    throw new Error("Account '" + username + "' already exists");
  }

  // Generate or validate mnemonic
  if (!mnemonic) {
    mnemonic = keyManager.generateMnemonic();
  } else if (!keyManager.validateMnemonic(mnemonic)) {
    throw new Error("Invalid BIP-39 mnemonic");
  }

  // Derive role keys from mnemonic
  const keys = await keyManager.mnemonicToKeys(mnemonic);

  // Derive 2FA keypair from password
  let twoFactorPublicKey = null;
  let authProfile = "none";
  if (password) {
    const factor2 = await keyManager.deriveSecondFactor(password, username);
    twoFactorPublicKey = factor2.publicKey;
    authProfile = "password";
  }

  // Compute wallet address from owner public key (first 20 bytes of SHA-256)
  const addrHash = crypto.createHash("sha256").update(Buffer.from(keys.owner.publicKey, "hex")).digest();
  const address = "BTCPC" + addrHash.subarray(0, 20).toString("hex");

  // Store user record
  const user = await User().create({
    username: username,
    email: username + "@btcpc.local", // placeholder — real email set later
    password: crypto.createHash("sha256").update(password || crypto.randomBytes(32)).digest("hex"),
    twoFactorEnabled: !!password,
    authProfile: authProfile,
    ownerPublicKey: keys.owner.publicKey,
    activePublicKey: keys.active.publicKey,
    postingPublicKey: keys.posting.publicKey,
    memoPublicKey: keys.memo.publicKey,
    twoFactorPublicKey: twoFactorPublicKey
  });

  // Store wallet record
  await Wallet().create({
    userId: user._id,
    chain: "btcpc",
    address: address,
    publicKey: keys.owner.publicKey,
    balance: new Map([["BTCPC", 0]])
  });

  return {
    username: username,
    mnemonic: mnemonic,
    address: address,
    publicKeys: {
      owner: keys.owner.publicKey,
      active: keys.active.publicKey,
      posting: keys.posting.publicKey,
      memo: keys.memo.publicKey
    },
    twoFactorPublicKey: twoFactorPublicKey,
    authProfile: authProfile
  };
}

// ---------------------------------------------------------------------------
// Account Recovery
// ---------------------------------------------------------------------------

/**
 * Recover an account from a 12-word mnemonic.
 * Regenerates all four role keypairs and the wallet address.
 *
 * @param {string} mnemonic - 12-word BIP-39 mnemonic
 * @returns {Promise<Object>} Recovered public keys and address
 */
async function recoverAccount(mnemonic) {
  if (!keyManager.validateMnemonic(mnemonic)) {
    throw new Error("Invalid BIP-39 mnemonic");
  }

  // Derive all keys
  const keys = await keyManager.mnemonicToKeys(mnemonic);

  // Compute the address from owner public key
  const addrHash = crypto.createHash("sha256").update(Buffer.from(keys.owner.publicKey, "hex")).digest();
  const address = "BTCPC" + addrHash.subarray(0, 20).toString("hex");

  // Try to find the account on-chain by owner public key
  const user = await User().findOne({ ownerPublicKey: keys.owner.publicKey });

  return {
    found: !!user,
    username: user ? user.username : null,
    address: address,
    publicKeys: {
      owner: keys.owner.publicKey,
      active: keys.active.publicKey,
      posting: keys.posting.publicKey,
      memo: keys.memo.publicKey
    }
  };
}

// ---------------------------------------------------------------------------
// Password Change (rotates 2FA key)
// ---------------------------------------------------------------------------

/**
 * Change the 2FA password for an account.
 * Requires the owner key for authorization (per whitepaper section 2.3.1).
 * Derives a new 2FA keypair and stores the new public key on-chain.
 *
 * @param {string} username - Account name
 * @param {string} oldPassword - Current 2FA password
 * @param {string} newPassword - New 2FA password
 * @param {string} ownerKey - Hex-encoded owner private key (for authorization)
 * @returns {Promise<Object>} Result with new 2FA public key
 */
async function changePassword(username, oldPassword, newPassword, ownerKey) {
  const user = await User().findOne({ username: username });
  if (!user) {
    throw new Error("Account '" + username + "' not found");
  }

  // Verify owner key by checking its public key matches
  const secp256k1 = require("secp256k1");
  const ownerPub = Buffer.from(secp256k1.publicKeyCreate(Buffer.from(ownerKey, "hex"), true)).toString("hex");
  if (ownerPub !== user.ownerPublicKey) {
    throw new Error("Owner key does not match this account");
  }

  // Verify old password (if 2FA was enabled)
  if (user.twoFactorEnabled && user.twoFactorPublicKey) {
    const oldFactor = await keyManager.deriveSecondFactor(oldPassword, username);
    if (oldFactor.publicKey !== user.twoFactorPublicKey) {
      throw new Error("Current password is incorrect");
    }
  }

  // Derive new 2FA keypair
  const newFactor = await keyManager.deriveSecondFactor(newPassword, username);

  // Update on-chain
  user.twoFactorPublicKey = newFactor.publicKey;
  user.twoFactorEnabled = true;
  user.password = crypto.createHash("sha256").update(newPassword).digest("hex");
  await user.save();

  return {
    username: username,
    twoFactorPublicKey: newFactor.publicKey,
    rotated: true
  };
}

// ---------------------------------------------------------------------------
// Authentication Profile
// ---------------------------------------------------------------------------

/**
 * Set the authentication profile for an account.
 * Configures which 2FA factors are required for transactions.
 *
 * @param {string} username - Account name
 * @param {string} profile - One of: "password", "totp", "both", "none"
 * @param {string} ownerKey - Hex-encoded owner private key (for authorization)
 * @returns {Promise<Object>} Updated profile info
 */
async function setAuthProfile(username, profile, ownerKey) {
  const validProfiles = ["password", "totp", "both", "none"];
  if (validProfiles.indexOf(profile) === -1) {
    throw new Error("Invalid auth profile. Must be one of: " + validProfiles.join(", "));
  }

  const user = await User().findOne({ username: username });
  if (!user) {
    throw new Error("Account '" + username + "' not found");
  }

  // Verify owner key
  const secp256k1 = require("secp256k1");
  const ownerPub = Buffer.from(secp256k1.publicKeyCreate(Buffer.from(ownerKey, "hex"), true)).toString("hex");
  if (ownerPub !== user.ownerPublicKey) {
    throw new Error("Owner key does not match this account");
  }

  user.authProfile = profile;
  user.twoFactorEnabled = profile !== "none";
  await user.save();

  return {
    username: username,
    authProfile: profile,
    twoFactorEnabled: user.twoFactorEnabled
  };
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  validateUsername,
  createAccount,
  recoverAccount,
  changePassword,
  setAuthProfile
};
