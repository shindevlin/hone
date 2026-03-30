"use strict";

const mongoose = require("mongoose");

const peerRegistrySchema = new mongoose.Schema({
  node_id: { type: String, required: true, index: true },
  address: { type: String, required: true },
  username: { type: String, required: true },
  gpu: { type: String, default: null },
  last_seen: { type: Date, default: Date.now },
  registered_at: { type: Date, default: Date.now },
});

// Auto-expire peers not seen in 24 hours
peerRegistrySchema.index({ last_seen: 1 }, { expireAfterSeconds: 86400 });

module.exports = mongoose.model("PeerRegistry", peerRegistrySchema);
