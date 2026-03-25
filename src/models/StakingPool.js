"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * Staking Pool Schema
 */
var poolSchema = new Schema({
  name: {
    type: String,
    required: true,
    unique: true
  },
  chain: {
    type: String,
    enum: ['hive', 'ton'],
    required: true
  },
  totalStaked: {
    type: Number,
    default: 0
  },
  apy: {
    type: Number,
    default: 0
  },
  rewardLogic: {
    type: String,
    default: 'standard'
  },
  status: {
    type: String,
    enum: ['active', 'closed'],
    default: 'active'
  },
  rewardsPerUser: {
    type: Map,
    of: Number,
    default: new Map()
  },
  createdAt: {
    type: Date,
    default: Date.now
  }
});

module.exports = mongoose.model("StakingPool", poolSchema);
