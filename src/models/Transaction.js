"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * Transaction Schema
 * Records all BTCPC token transactions
 */
var transactionSchema = new Schema({
  from: {
    type: String,
    required: true
  },
  to: {
    type: String,
    required: true
  },
  amount: {
    type: Number,
    required: true
  },
  type: {
    type: String,
    enum: ['transfer', 'mining_reward', 'staking_reward', 'staking', 'unstaking', 'faucet', 'verification'],
    required: true
  },
  timestamp: {
    type: Date,
    default: Date.now
  },
  memo: {
    type: String,
    default: null
  }
});

module.exports = mongoose.model("Transaction", transactionSchema);
