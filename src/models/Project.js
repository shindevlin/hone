"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

var projectSchema = new Schema({
  name: { type: String, required: true },
  repoUrl: { type: String, required: true, unique: true },
  owner: { type: String, required: true },         // GitHub username
  repo: { type: String, required: true },           // repo name
  apiKey: { type: String, required: true, unique: true },
  walletAddress: { type: String, required: true },
  balance: { type: Number, default: 0 },            // BTCPC balance
  verified: { type: Boolean, default: false },
  verifiedAt: { type: Date, default: null },
  totalSpent: { type: Number, default: 0 },
  totalRequests: { type: Number, default: 0 },
  isActive: { type: Boolean, default: true },
  createdAt: { type: Date, default: Date.now }
});

projectSchema.index({ apiKey: 1 });
projectSchema.index({ owner: 1, repo: 1 });

module.exports = mongoose.model("Project", projectSchema);
