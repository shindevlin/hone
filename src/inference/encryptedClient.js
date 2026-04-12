"use strict";

/**
 * BTCPC Encrypted Inference Client Helper
 * Shin Devlin
 *
 * Client-side helper for submitting end-to-end encrypted inference requests.
 * Callers provide their mnemonic — all key material stays local.
 *
 * Usage:
 *   const { submitEncryptedInference, decryptResult } = require('./encryptedClient');
 *
 *   // Encrypt prompt and get submission payload
 *   const payload = await submitEncryptedInference(prompt, minerAccount, userMnemonic);
 *   // payload = { prompt_encrypted: {ciphertext, iv, tag}, user_memo_pubkey }
 *
 *   // After the server returns encrypted_result:
 *   const plaintext = await decryptResult(encryptedResult, minerAccount, userMnemonic);
 */

const keyManager = require("../wallet/keyManager");
const stateStore = require("../chain/stateStore");
const { computeSharedSecret, encrypt, decrypt } = require("./crypto");

/**
 * Prepare an encrypted inference payload for submission.
 *
 * @param {string} prompt        — Plaintext prompt to encrypt
 * @param {string} minerAccount  — BTCPC account name of the target miner
 * @param {string} userMnemonic  — User's BIP-39 mnemonic (stays local, never sent)
 * @returns {Promise<{ prompt_encrypted: {ciphertext, iv, tag}, user_memo_pubkey: string }>}
 */
async function submitEncryptedInference(prompt, minerAccount, userMnemonic) {
  if (!prompt && prompt !== "") {
    throw new Error("submitEncryptedInference: prompt is required");
  }
  if (!minerAccount) {
    throw new Error("submitEncryptedInference: minerAccount is required");
  }
  if (!userMnemonic) {
    throw new Error("submitEncryptedInference: userMnemonic is required");
  }

  // 1. Derive user's memo keypair from mnemonic
  const keys = await keyManager.mnemonicToKeys(userMnemonic);
  const userMemoPriv = keys.memo.privateKey;
  const userMemoPub = keys.memo.publicKey;

  // 2. Look up miner's memo public key from chain state
  const account = stateStore.getAccount(minerAccount);
  if (!account) {
    throw new Error(`submitEncryptedInference: miner account "${minerAccount}" not found`);
  }
  const minerMemoPub = account.public_keys && account.public_keys.memo;
  if (!minerMemoPub) {
    throw new Error(`submitEncryptedInference: miner account "${minerAccount}" has no memo public key`);
  }

  // 3. Compute ECDH shared secret: user_priv × miner_pub
  const shared = computeSharedSecret(userMemoPriv, minerMemoPub);

  // 4. Encrypt the prompt
  const promptEncrypted = encrypt(String(prompt), shared);

  return {
    prompt_encrypted: promptEncrypted,
    user_memo_pubkey: userMemoPub,
  };
}

/**
 * Decrypt an encrypted inference result returned by the miner.
 *
 * @param {{ ciphertext: string, iv: string, tag: string }} encryptedResult
 * @param {string} minerAccount  — BTCPC account name of the miner that answered
 * @param {string} userMnemonic  — User's BIP-39 mnemonic
 * @returns {Promise<string>} Plaintext result
 */
async function decryptResult(encryptedResult, minerAccount, userMnemonic) {
  if (!encryptedResult) {
    throw new Error("decryptResult: encryptedResult is required");
  }
  if (!minerAccount) {
    throw new Error("decryptResult: minerAccount is required");
  }
  if (!userMnemonic) {
    throw new Error("decryptResult: userMnemonic is required");
  }

  // Derive user's memo keypair
  const keys = await keyManager.mnemonicToKeys(userMnemonic);
  const userMemoPriv = keys.memo.privateKey;

  // Look up miner's memo public key
  const account = stateStore.getAccount(minerAccount);
  if (!account) {
    throw new Error(`decryptResult: miner account "${minerAccount}" not found`);
  }
  const minerMemoPub = account.public_keys && account.public_keys.memo;
  if (!minerMemoPub) {
    throw new Error(`decryptResult: miner account "${minerAccount}" has no memo public key`);
  }

  // Reconstruct the same shared secret
  const shared = computeSharedSecret(userMemoPriv, minerMemoPub);

  return decrypt(encryptedResult, shared);
}

module.exports = { submitEncryptedInference, decryptResult };
