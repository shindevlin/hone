"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * SlashRecord — tracks slashing offenses and appeals.
 *
 * Every offense is recorded permanently. Tier escalates with repeat offenses
 * of the same type. Slashed tokens go to btcpc_recycle (never burned).
 */
var slashRecordSchema = new Schema({
  // The offending account (username)
  account: { type: String, required: true, index: true },

  // Role at time of offense
  role: {
    type: String,
    enum: ['miner', 'verifier', 'clock'],
    required: true
  },

  // Offense classification
  offenseType: {
    type: String,
    enum: [
      // Miner offenses
      'EMPTY_GARBAGE_INFERENCE',
      'TIMING_FRAUD',
      'REPEATED_ZERO_QUALITY',
      // Verifier offenses
      'RUBBER_STAMPING',
      'GRIEFING',
      'COLLUSION',
      // Clock offenses
      'TIME_DRIFT',
      'CLOCK_OFFLINE'
    ],
    required: true,
    index: true
  },

  // Current tier for this offense (0 = warning, 1 = first slash, 2 = second, etc.)
  tier: { type: Number, required: true, default: 0 },

  // Amount slashed (0 for warnings)
  amount: { type: Number, default: 0 },

  // Evidence blob — whatever the caller provides
  evidence: { type: Schema.Types.Mixed, default: null },

  // Ledger transaction hash (memo) for the slash transfer
  slashTxId: { type: String, default: null },

  // Whether this resulted in deregistration
  deregistered: { type: Boolean, default: false },

  // Appeal tracking
  appeal: {
    submitted: { type: Boolean, default: false },
    submittedAt: { type: Date, default: null },
    submittedAtEpoch: { type: Number, default: null },
    deadline: { type: Number, default: null }, // epoch number
    panelSize: { type: Number, default: null },
    verdicts: [{ verifier: String, vote: String }], // vote: 'overturn' or 'uphold'
    resolved: { type: Boolean, default: false },
    outcome: { type: String, enum: ['overturned', 'upheld', null], default: null }
  },

  // Epoch when the offense occurred
  epoch: { type: Number, required: true, index: true },

  timestamp: { type: Date, default: Date.now }
});

slashRecordSchema.index({ account: 1, offenseType: 1 });

module.exports = mongoose.model("SlashRecord", slashRecordSchema);
