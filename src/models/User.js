"use strict";
var mongoose = require("mongoose");
var Schema = mongoose.Schema;

/**
 * User Schema
 * Stores user credentials, Telegram account linking status, and 2FA settings
 */
var userSchema = new Schema({
  username: {
    type: String,
    required: true,
    unique: true,
    trim: true
  },
  email: {
    type: String,
    required: true,
    lowercase: true,
    match: /^[^\s@]+@[^\s@]+\./
  },
  telegramId: {
    type: String,
    default: null
  },
  telegramUsername: {
    type: String,
    default: null
  },
  password: {
    type: String,
    required: true
  },
  twoFactorEnabled: {
    type: Boolean,
    default: false
  },
  totpSecret: {
    type: String,
    default: null
  },
  totpEnabled: {
    type: Boolean,
    default: false
  },
  totpBackupCodes: {
    type: [String],
    default: []
  },
  authProfile: {
    type: String,
    enum: ["password", "totp", "both", "none"],
    default: "none"
  },
  ownerPublicKey: {
    type: String,
    default: null
  },
  activePublicKey: {
    type: String,
    default: null
  },
  postingPublicKey: {
    type: String,
    default: null
  },
  memoPublicKey: {
    type: String,
    default: null
  },
  twoFactorPublicKey: {
    type: String,
    default: null
  },
  lastLogin: {
    type: Date,
    default: Date.now
  },
  isActive: {
    type: Boolean,
    default: true
  },
  createdAt: {
    type: Date,
    default: Date.now
  },
  // Telegram link verification
  pendingTelegramLink: {
    telegramId: { type: String, default: null },
    telegramUsername: { type: String, default: null },
    challenge: { type: String, default: null },
    expiresAt: { type: Date, default: null }
  },
  // Saved MCP servers — user brings their own, saves favorites to profile
  mcpServers: [{
    name: { type: String, required: true },      // e.g. "github", "my-postgres"
    url: { type: String, required: true },        // e.g. "http://localhost:3001"
    tools: [{ type: String }],                    // e.g. ["search_code", "read_file"]
    description: { type: String, default: null }  // optional human note
  }]
});

module.exports = mongoose.model("User", userSchema);
