"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * Wallet Schema
 * Multi-chain wallet storage (Hive, TON)
 */
var walletSchema = new Schema({
  userId: {
    type: Schema.Types.ObjectId,
    required: true,
    ref: 'User'
  },
  chain: {
    type: String,
    enum: ['hive', 'ton', 'btcpc'],
    required: true
  },
  address: {
    type: String,
    required: true
  },
  publicKey: {
    type: String,
    default: null
  },
  balance: {
    type: Map,
    of: Number,
    default: new Map()
  },
  nfts: {
    type: Array,
    default: []
  },
  transactions: {
    type: Array,
    default: []
  }
});

module.exports = mongoose.model("Wallet", walletSchema);
