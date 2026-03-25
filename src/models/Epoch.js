"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * BTCPC Epoch Schema
 * One epoch = 5 minutes. Tracks all node commitments, consensus state,
 * and reward distribution for each epoch in the BTCPC network.
 */
var epochSchema = new Schema({
  epoch_number: {
    type: Number,
    required: true,
    unique: true,
    index: true
  },
  started_at: {
    type: Date,
    required: true
  },
  ended_at: {
    type: Date,
    default: null
  },
  block_reward: {
    type: Number,
    required: true
  },
  total_work: {
    type: Number,
    default: 0
  },
  commitments: [{
    node_id: { type: Schema.Types.ObjectId, ref: 'Node', required: true },
    state_hash: { type: String, required: true },
    tx_count: { type: Number, default: 0 },
    inference_count: { type: Number, default: 0 },
    submitted_at: { type: Date, default: Date.now }
  }],
  consensus_hash: {
    type: String,
    default: null
  },
  rewards_distributed: [{
    node_id: { type: Schema.Types.ObjectId, ref: 'Node' },
    amount: { type: Number }
  }],
  status: {
    type: String,
    enum: ['active', 'committed', 'finalized'],
    default: 'active'
  }
});

epochSchema.index({ status: 1 });
epochSchema.index({ started_at: -1 });

module.exports = mongoose.model("Epoch", epochSchema);
