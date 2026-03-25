"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * RecoveryRequest Schema
 * Shin Devlin
 *
 * Tracks account recovery requests initiated via Owner key (bypasses 2FA).
 * Implements the 72-hour time-lock window described in BTCPC Whitepaper 2.3.3.
 */
var recoveryRequestSchema = new Schema({
  account: {
    type: String,
    required: true,
    trim: true,
    index: true
  },
  requested_at: {
    type: Date,
    required: true,
    default: Date.now
  },
  expires_at: {
    type: Date,
    required: true,
    default: function () {
      return new Date(Date.now() + 72 * 60 * 60 * 1000);
    }
  },
  contested: {
    type: Boolean,
    default: false
  },
  contested_by_2fa: {
    type: Boolean,
    default: false
  },
  status: {
    type: String,
    enum: ["pending", "completed", "contested", "expired"],
    default: "pending"
  },
  new_2fa_public_key: {
    type: String,
    default: null
  }
});

module.exports = mongoose.model("RecoveryRequest", recoveryRequestSchema);
