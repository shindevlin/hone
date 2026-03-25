"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * BTCPC Genesis Dream Schema
 * The first dream of each block — a transferable NFT with mandatory metadata.
 * Every block must have exactly one genesis dream. No block is valid without it.
 */
// Dream #0 is unlimited. All others: 4200 chars max (42 x 100).
const INSCRIPTION_CHAR_LIMIT = 4200;

var genesisDreamSchema = new Schema({
  block_number: {
    type: Number,
    required: true,
    unique: true,
    index: true
  },
  original_miner: {
    type: String,
    required: true
  },
  current_owner: {
    type: String,
    required: true
  },
  inscription: {
    project: { type: String, required: true },
    tag: { type: String, required: true },
    custom_data: { type: Schema.Types.Mixed, default: null }
  },
  proof: {
    state_hash: { type: String, required: true },
    work_hash: { type: String, required: true },
    tokens_computed: { type: Number, required: true },
    model: { type: String, required: true }
  },
  inscribed_at: {
    type: Date,
    default: Date.now
  },
  transferred: {
    type: Boolean,
    default: false
  },
  transfer_history: [{
    from: String,
    to: String,
    timestamp: { type: Date, default: Date.now }
  }]
});

// Validate inscription size: Dream #0 unlimited, all others capped at 4200 chars
genesisDreamSchema.pre('save', function(next) {
  if (this.block_number > 0 && this.inscription && this.inscription.custom_data) {
    const dataStr = JSON.stringify(this.inscription.custom_data);
    if (dataStr.length > INSCRIPTION_CHAR_LIMIT) {
      return next(new Error(`Inscription exceeds ${INSCRIPTION_CHAR_LIMIT} char limit (${dataStr.length} chars). Only Dream #0 is unlimited.`));
    }
  }
  next();
});

module.exports = mongoose.model("GenesisDream", genesisDreamSchema);
module.exports.INSCRIPTION_CHAR_LIMIT = INSCRIPTION_CHAR_LIMIT;
