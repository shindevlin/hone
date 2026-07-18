"use strict";

/**
 * hiveClaimManager.js — Submit and track HONE cross-chain claims on Hive
 * Shin Devlin
 *
 * Posts custom_json operations to the Hive blockchain with id "hone-claim".
 * Tracks all submitted claims in MongoDB for audit and duplicate prevention.
 */

const { Client, PrivateKey } = require("@hiveio/dhive");
const mongoose = require("mongoose");

// ---------------------------------------------------------------------------
// Hive client setup
// ---------------------------------------------------------------------------

const HIVE_NODES = [
  "https://api.hive.blog",
  "https://api.deathwing.me",
  "https://anyx.io"
];

let hiveClient = null;

/**
 * Get or create the dhive client instance.
 * @returns {Client}
 */
function getHiveClient() {
  if (!hiveClient) {
    hiveClient = new Client(HIVE_NODES);
  }
  return hiveClient;
}

// ---------------------------------------------------------------------------
// MongoDB schema for tracking Hive claims
// ---------------------------------------------------------------------------

const hiveClaimSchema = new mongoose.Schema({
  miner: {
    type: String,
    required: true,
    index: true
  },
  hive_account: {
    type: String,
    required: true,
    index: true
  },
  epoch: {
    type: Number,
    required: true
  },
  amount: {
    type: String,
    required: true
  },
  period: {
    type: Number,
    required: true
  },
  cross_chain_ratio: {
    type: String,
    required: true
  },
  proof_signature: {
    type: String,
    required: true
  },
  hive_tx_id: {
    type: String,
    default: null
  },
  status: {
    type: String,
    enum: ["pending", "broadcast", "confirmed", "failed"],
    default: "pending"
  },
  error_message: {
    type: String,
    default: null
  },
  created_at: {
    type: Date,
    default: Date.now
  },
  broadcast_at: {
    type: Date,
    default: null
  }
});

hiveClaimSchema.index({ miner: 1, epoch: 1 }, { unique: true });

const HiveClaim = mongoose.model("HiveClaim", hiveClaimSchema);

// ---------------------------------------------------------------------------
// Core functions
// ---------------------------------------------------------------------------

/**
 * Post a cross-chain claim proof to the Hive blockchain as a custom_json.
 *
 * @param {Object} proof          - Claim proof from claimProofGenerator
 * @param {string} hiveAccount    - Hive account name to post from
 * @param {string} hivePostingKey - Hive posting key (WIF format)
 * @returns {Object} Result with tx_id and status
 */
async function postClaimToHive(proof, hiveAccount, hivePostingKey) {
  // Check for duplicate claim
  const existing = await HiveClaim.findOne({
    miner: proof.miner,
    epoch: proof.epoch
  });

  if (existing && (existing.status === "broadcast" || existing.status === "confirmed")) {
    console.log("[HONE] Hive claim already submitted for epoch " + proof.epoch);
    return {
      status: "duplicate",
      tx_id: existing.hive_tx_id,
      claim: existing
    };
  }

  // Create or update the claim record
  let claim;
  if (existing) {
    claim = existing;
    claim.status = "pending";
    claim.error_message = null;
  } else {
    claim = new HiveClaim({
      miner: proof.miner,
      hive_account: hiveAccount,
      epoch: proof.epoch,
      amount: proof.amount,
      period: proof.period,
      cross_chain_ratio: proof.cross_chain_ratio,
      proof_signature: proof.proof_signature
    });
  }
  await claim.save();

  // Build the custom_json payload
  const jsonPayload = {
    type: "hone-claim",
    version: 1,
    miner: proof.miner,
    epoch: proof.epoch,
    amount: proof.amount,
    period: proof.period,
    cross_chain_ratio: proof.cross_chain_ratio,
    proof_signature: proof.proof_signature,
    timestamp: proof.timestamp
  };

  try {
    const client = getHiveClient();
    const key = PrivateKey.fromString(hivePostingKey);

    const result = await client.broadcast.json({
      required_auths: [],
      required_posting_auths: [hiveAccount],
      id: "hone-claim",
      json: JSON.stringify(jsonPayload)
    }, key);

    claim.hive_tx_id = result.id;
    claim.status = "broadcast";
    claim.broadcast_at = new Date();
    await claim.save();

    console.log("[HONE] Hive claim broadcast for epoch " + proof.epoch + " tx: " + result.id);

    return {
      status: "broadcast",
      tx_id: result.id,
      claim: claim
    };
  } catch (err) {
    claim.status = "failed";
    claim.error_message = err.message;
    await claim.save();

    console.error("[HONE] Hive claim failed for epoch " + proof.epoch + ": " + err.message);

    return {
      status: "failed",
      error: err.message,
      claim: claim
    };
  }
}

/**
 * Get all claims for a specific miner.
 * @param {string} miner - HONE miner account name
 * @returns {Array<Object>} Array of claim records
 */
async function getClaimsForMiner(miner) {
  return HiveClaim.find({ miner }).sort({ epoch: -1 });
}

/**
 * Get pending (unsubmitted) claims for a miner.
 * @param {string} miner - HONE miner account name
 * @returns {Array<Object>} Array of pending claim records
 */
async function getPendingClaims(miner) {
  return HiveClaim.find({ miner, status: "pending" }).sort({ epoch: 1 });
}

/**
 * Get claim status for a specific epoch.
 * @param {string} miner - HONE miner account name
 * @param {number} epoch - Epoch number
 * @returns {Object|null} Claim record or null
 */
async function getClaimForEpoch(miner, epoch) {
  return HiveClaim.findOne({ miner, epoch });
}

module.exports = {
  postClaimToHive,
  getClaimsForMiner,
  getPendingClaims,
  getClaimForEpoch,
  HiveClaim,
  getHiveClient
};
