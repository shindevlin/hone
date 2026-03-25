"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * BTCPC Staking Pool Schema
 * Tracks individual user stakes required for node operation and mining.
 * Minimum stake: 1000 BTCPC. Unstaking has a 7-day unlock period.
 */
var stakingPoolSchema = new Schema({
  account: {
    type: Schema.Types.ObjectId,
    required: true,
    ref: 'User'
  },
  wallet_address: {
    type: String,
    required: true
  },
  staked_amount: {
    type: Number,
    default: 0
  },
  staked_at: {
    type: Date,
    default: null
  },
  unlock_requested_at: {
    type: Date,
    default: null
  },
  unlock_available_at: {
    type: Date,
    default: null
  },
  status: {
    type: String,
    enum: ['active', 'unstaking', 'withdrawn'],
    default: 'active'
  },
  slashed_amount: {
    type: Number,
    default: 0
  }
});

module.exports = mongoose.model("StakingPool", stakingPoolSchema);
