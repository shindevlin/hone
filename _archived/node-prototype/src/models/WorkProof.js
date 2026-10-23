"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * HONE Work Proof Schema
 * Records each unit of verified inference work performed by a mining node.
 * Work value = tokens_generated x model_weight_factor.
 */
var workProofSchema = new Schema({
  epoch_number: {
    type: Number,
    required: true,
    index: true
  },
  node_id: {
    type: String,
    required: true
  },
  prompt_hash: {
    type: String,
    required: true
  },
  result_hash: {
    type: String,
    required: true
  },
  model: {
    type: String,
    required: true
  },
  tokens_generated: {
    type: Number,
    required: true
  },
  model_weight_factor: {
    type: Number,
    required: true
  },
  work_value: {
    type: Number,
    required: true
  },
  timestamp: {
    type: Date,
    default: Date.now
  }
});

workProofSchema.index({ epoch_number: 1, node_id: 1 });

module.exports = mongoose.model("WorkProof", workProofSchema);
