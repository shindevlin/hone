"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * InferenceJob — tracks the lifecycle of an inference request on the blockchain.
 *
 * Submit → pending → claimed → processing → completed/failed
 *
 * Requesters submit and get a job_id back immediately.
 * They poll GET /v1/inference/:job_id until status is completed.
 */
var inferenceJobSchema = new Schema({
  job_id: { type: String, required: true, unique: true, index: true },
  status: {
    type: String,
    enum: ['pending', 'claimed', 'processing', 'completed', 'failed', 'expired'],
    default: 'pending'
  },
  // Request
  model: { type: String, required: true },
  messages: { type: Array, required: true },
  max_tokens: { type: Number, default: 1024 },
  temperature: { type: Number, default: 0.7 },
  max_fee: { type: Number, default: 0 },
  prompt_hash: { type: String, default: null },

  // Assignment
  assigned_miners: [String],
  claimed_by: { type: String, default: null },
  claimed_at: { type: Date, default: null },

  // Result
  result_text: { type: String, default: null },
  result_hash: { type: String, default: null },
  tokens_generated: { type: Number, default: 0 },
  elapsed_ms: { type: Number, default: 0 },
  node_name: { type: String, default: null },

  // Billing
  cost: { type: Number, default: 0 },
  project_id: { type: Schema.Types.ObjectId, ref: 'Project', default: null },

  // Metadata
  epoch: { type: Number, default: null },
  proof_hash: { type: String, default: null },
  verified: { type: Boolean, default: false },

  created_at: { type: Date, default: Date.now },
  completed_at: { type: Date, default: null },
  expires_at: { type: Date, default: null }
});

inferenceJobSchema.index({ status: 1, created_at: -1 });
inferenceJobSchema.index({ project_id: 1, created_at: -1 });

module.exports = mongoose.model("InferenceJob", inferenceJobSchema);
