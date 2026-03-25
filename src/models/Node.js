"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * BTCPC Node Schema
 * Represents a registered mining node in the BTCPC compute network.
 * Nodes declare hardware specs, available Ollama models, and API endpoint.
 * Minimum stake of 1000 BTCPC required to register.
 */
var nodeSchema = new Schema({
  account: {
    type: Schema.Types.ObjectId,
    required: true,
    ref: 'User',
    unique: true
  },
  hive_account: {
    type: String,
    default: null
  },
  base_wallet: {
    type: String,
    default: null
  },
  endpoint: {
    type: String,
    required: true
  },
  models: {
    type: [String],
    default: []
  },
  hardware: {
    gpu: { type: String, default: null },
    vram_gb: { type: Number, default: 0 },
    cpu_cores: { type: Number, default: 0 },
    ram_gb: { type: Number, default: 0 }
  },
  stake_amount: {
    type: Number,
    required: true,
    min: 1000
  },
  status: {
    type: String,
    enum: ['registered', 'active', 'suspended', 'banned'],
    default: 'registered'
  },
  reputation: {
    type: Number,
    default: 100
  },
  registered_at: {
    type: Date,
    default: Date.now
  },
  last_epoch_commitment: {
    type: Number,
    default: -1
  }
});

nodeSchema.index({ status: 1 });
nodeSchema.index({ account: 1 });

module.exports = mongoose.model("Node", nodeSchema);
