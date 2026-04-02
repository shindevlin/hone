"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * BTCPC Mining Proof Schema
 * Soulbound badge — permanently bound to the miner who produced the block.
 * Cannot be transferred, sold, traded, or reassigned. Ever.
 */
var miningProofSchema = new Schema({
  block_number: {
    type: Number,
    required: true,
    index: true
  },
  miner: {
    type: String,
    required: true,
    immutable: true
  },
  reward_earned: {
    type: Number,
    required: true
  },
  model: {
    type: String,
    required: true
  },
  tokens_computed: {
    type: Number,
    required: true
  },
  work_value: {
    type: Number,
    required: true
  },
  state_hash: {
    type: String,
    required: true
  },
  timestamp: {
    type: Date,
    default: Date.now
  }
});

// Compound unique: one proof per miner per block (multiple miners per block is expected)
miningProofSchema.index({ block_number: 1, miner: 1 }, { unique: true });

// Soulbound: prevent any update to the miner field
miningProofSchema.pre('save', function(next) {
  if (!this.isNew && this.isModified('miner')) {
    return next(new Error('Mining proofs are soulbound. The miner field cannot be changed.'));
  }
  next();
});

module.exports = mongoose.model("MiningProof", miningProofSchema);
