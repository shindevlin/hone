"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * BTCPC Delegation Schema
 * Allows BTCPC holders to delegate tokens to miners and earn proportional
 * mining rewards without running a node. 7-day unlock period on undelegation.
 */
var delegationSchema = new Schema({
  delegator: {
    type: Schema.Types.ObjectId,
    required: true,
    ref: 'User'
  },
  miner: {
    type: Schema.Types.ObjectId,
    required: true,
    ref: 'User'
  },
  amount: {
    type: Number,
    required: true,
    min: 1
  },
  delegated_at: {
    type: Date,
    default: Date.now
  },
  status: {
    type: String,
    enum: ['active', 'undelegating', 'withdrawn'],
    default: 'active'
  },
  undelegate_requested_at: {
    type: Date,
    default: null
  },
  undelegate_available_at: {
    type: Date,
    default: null
  }
});

delegationSchema.index({ delegator: 1, status: 1 });
delegationSchema.index({ miner: 1, status: 1 });

module.exports = mongoose.model("Delegation", delegationSchema);
