"use strict";

const mongoose = require("mongoose");

const escrowSchema = new mongoose.Schema({
  request_id: { type: String, required: true, unique: true, index: true },
  payer: { type: String, required: true },
  amount: { type: Number, required: true },
  status: {
    type: String,
    enum: ["locked", "released", "refunded"],
    default: "locked",
  },
  locked_at: { type: Date, default: Date.now },
  released_at: { type: Date, default: null },
  released_to: [{
    node_id: String,
    amount: Number,
  }],
});

module.exports = mongoose.model("Escrow", escrowSchema);
