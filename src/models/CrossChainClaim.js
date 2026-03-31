"use strict";

/**
 * CrossChainClaim Schema
 * Shin Devlin
 *
 * Tracks cross-chain claim proofs generated after each epoch.
 * Used by the CLI to show pending claims and by the claim
 * submission flow to prevent duplicates.
 */

var mongoose = require("mongoose");
var Schema = mongoose.Schema;

var crossChainClaimSchema = new Schema({
  miner: {
    type: String,
    required: true,
    index: true
  },
  chain: {
    type: String,
    required: true,
    enum: ["hive", "base", "solana", "arbitrum", "optimism", "ton", "bitcoin"]
  },
  target_wallet: {
    type: String,
    required: true
  },
  epoch: {
    type: Number,
    required: true
  },
  native_reward: {
    type: Number,
    required: true
  },
  claim_amount: {
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
  proof_recovery: {
    type: Number,
    default: 0
  },
  status: {
    type: String,
    enum: ["generated", "submitted", "confirmed", "failed"],
    default: "generated"
  },
  tx_hash: {
    type: String,
    default: null
  },
  error_message: {
    type: String,
    default: null
  },
  created_at: {
    type: Date,
    default: Date.now
  },
  submitted_at: {
    type: Date,
    default: null
  }
});

crossChainClaimSchema.index({ miner: 1, chain: 1, epoch: 1 }, { unique: true });
crossChainClaimSchema.index({ status: 1 });
crossChainClaimSchema.index({ chain: 1, epoch: 1 });

module.exports = mongoose.model("CrossChainClaim", crossChainClaimSchema);
