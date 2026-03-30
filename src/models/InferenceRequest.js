"use strict";

const mongoose = require("mongoose");

const inferenceRequestSchema = new mongoose.Schema({
  request_id: { type: String, required: true, unique: true, index: true },
  requester_id: { type: String, required: true },
  model: { type: String, required: true },
  prompt_hash: { type: String, required: true },
  max_fee: { type: Number, required: true },
  redundancy: { type: Number, default: 1 },
  assignments: [{
    node_id: String,
    sik_hash: String,
    price: Number,
  }],
  consensus_hash: { type: String, default: null },
  honest_nodes: [String],
  dishonest_nodes: [String],
  payouts: [{
    node_id: String,
    amount: Number,
    rank: Number,
  }],
  status: {
    type: String,
    enum: ["open", "assigned", "committing", "revealing", "finalized", "expired"],
    default: "open",
  },
  created_at: { type: Date, default: Date.now },
  finalized_at: { type: Date, default: null },
});

module.exports = mongoose.model("InferenceRequest", inferenceRequestSchema);
