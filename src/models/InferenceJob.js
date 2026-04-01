"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * InferenceJob — tracks the full lifecycle of an inference request.
 *
 * Lifecycle: pending → assigned → verifying → settled / failed
 *
 * Settlement requires N verifications (3 in consensus mode, 1 in genesis).
 * Payment happens in the epoch the job SETTLES, not when work is done.
 *
 * Full timing data kept for chain optimization.
 */
var inferenceJobSchema = new Schema({
  job_id: { type: String, required: true, unique: true, index: true },
  status: {
    type: String,
    enum: ['pending', 'assigned', 'verifying', 'settled', 'failed', 'expired'],
    default: 'pending'
  },

  // ── Request ──
  model: { type: String, required: true },
  messages: { type: Array, required: true },
  max_tokens: { type: Number, default: 1024 },
  temperature: { type: Number, default: 0.7 },
  max_fee: { type: Number, default: 0 },
  prompt_hash: { type: String, default: null },

  // ── Assignment ──
  required_verifications: { type: Number, default: 1 }, // 1 in genesis, 3 in consensus
  assigned_miners: [String], // miners assigned to verify

  // ── Verifications (one per miner) ──
  verifications: [{
    miner: { type: String, required: true },
    result_text: { type: String, default: null },
    result_hash: { type: String, required: true },
    tokens_generated: { type: Number, default: 0 },
    model_hash: { type: String, default: null },
    work_value: { type: Number, default: 0 },       // tokens × verified_params
    elapsed_ms: { type: Number, default: 0 },
    claimed_at: { type: Date, default: null },       // when miner picked it up
    completed_at: { type: Date, default: null },     // when miner finished
    epoch_completed: { type: Number, default: null } // which epoch the work was done in
  }],

  // ── Settlement ──
  consensus_hash: { type: String, default: null },   // majority result hash
  consensus_reached: { type: Boolean, default: false },
  settlement_epoch: { type: Number, default: null },  // epoch where job settled (payment epoch)
  settled_at: { type: Date, default: null },

  // ── Result (consensus result shown to requester) ──
  result_text: { type: String, default: null },
  result_hash: { type: String, default: null },
  tokens_generated: { type: Number, default: 0 },
  elapsed_ms: { type: Number, default: 0 },
  node_name: { type: String, default: null },

  // ── Billing ──
  cost: { type: Number, default: 0 },
  project_id: { type: Schema.Types.ObjectId, ref: 'Project', default: null },

  // ── Timing (for chain optimization) ──
  timing: {
    submitted_at: { type: Date, default: Date.now },   // when user submitted
    first_claim_at: { type: Date, default: null },      // when first miner claimed
    last_claim_at: { type: Date, default: null },       // when last miner claimed
    first_result_at: { type: Date, default: null },     // when first result came in
    last_result_at: { type: Date, default: null },      // when last result came in (= settlement)
    total_wait_ms: { type: Number, default: null },     // submitted → settled
    avg_miner_ms: { type: Number, default: null },      // average miner processing time
    slowest_miner_ms: { type: Number, default: null },  // slowest miner
    fastest_miner_ms: { type: Number, default: null },  // fastest miner
  },

  // ── Metadata ──
  submit_epoch: { type: Number, default: null },    // epoch when submitted
  proof_hash: { type: String, default: null },
  verified: { type: Boolean, default: false },

  // Legacy compat
  claimed_by: { type: String, default: null },
  claimed_at: { type: Date, default: null },
  created_at: { type: Date, default: Date.now },
  completed_at: { type: Date, default: null },
  expires_at: { type: Date, default: null }
});

inferenceJobSchema.index({ status: 1, created_at: -1 });
inferenceJobSchema.index({ project_id: 1, created_at: -1 });
inferenceJobSchema.index({ settlement_epoch: 1 });

module.exports = mongoose.model("InferenceJob", inferenceJobSchema);
