#!/usr/bin/env node
"use strict";

/**
 * One-shot migration: read users + projects from MongoDB, write to
 * ~/.btcpc/secrets.json via secretStore.
 *
 * Run this ONCE before deploying Phase E. After migration, MongoDB can be
 * dropped for chain state without losing passwords, TOTP, or project API keys.
 *
 * Usage:
 *   node scripts/migrate-secrets-from-mongo.js
 *
 * What it does:
 * 1. Loads existing secrets.json (or creates empty)
 * 2. Reads User collection from Mongo → secretStore.createUser()
 * 3. Reads Project collection from Mongo:
 *    - stores public project data in secretStore (without the old plaintext API key)
 *    - generates a NEW API key for each project
 *    - prints the new plaintext keys to stdout ONCE
 * 4. Saves secrets.json
 *
 * The user must distribute the new API keys to any services that need them.
 */

require("dotenv").config();

var mongoose = require("mongoose");
var secretStore = require("../src/services/secretStore");

async function main() {
  if (!process.env.MONGODB_URI) {
    console.error("MONGODB_URI not set — cannot migrate");
    process.exit(1);
  }

  console.log("Connecting to MongoDB...");
  await mongoose.connect(process.env.MONGODB_URI);

  console.log("Loading secretStore from " + secretStore.SECRETS_PATH + "...");
  await secretStore.load();

  var User = require("../src/models/User");
  var Project = require("../src/models/Project");

  // Migrate users
  console.log("\n=== MIGRATING USERS ===");
  var users = await User.find({}).lean();
  console.log("Found " + users.length + " users in Mongo");

  var userMigrated = 0;
  var userSkipped = 0;
  for (var u of users) {
    var existing = secretStore.getUser(u.username);
    if (existing) {
      console.log("  skip " + u.username + " (already in secretStore)");
      userSkipped++;
      continue;
    }
    try {
      await secretStore.createUser(u.username, {
        user_id: String(u._id),
        email: u.email,
        password_hash: u.password, // already bcrypt
        totp_secret: u.totpSecret,
        totp_backup_codes: u.totpBackupCodes || [],
        totp_enabled: u.totpEnabled || false,
        two_factor_enabled: u.twoFactorEnabled || false,
        auth_profile: u.authProfile || "local",
        telegram_id: u.telegramId,
        telegram_username: u.telegramUsername,
        mcp_servers: u.mcpServers || [],
      });
      console.log("  + " + u.username);
      userMigrated++;
    } catch (e) {
      console.error("  ERR " + u.username + ": " + e.message);
    }
  }

  // Migrate projects (with fresh API keys)
  console.log("\n=== MIGRATING PROJECTS ===");
  var projects = await Project.find({}).lean();
  console.log("Found " + projects.length + " projects in Mongo");

  var newKeys = [];
  for (var p of projects) {
    var existing = secretStore.getProject(p.name);
    if (existing) {
      console.log("  skip " + p.name + " (already in secretStore)");
      continue;
    }
    try {
      var result = await secretStore.createProject({
        name: p.name,
        owner: p.owner,
        repo_url: p.repoUrl,
        repo: p.repo,
        wallet_address: p.walletAddress,
      });
      console.log("  + " + p.name);
      newKeys.push({ name: p.name, apiKey: result.apiKey, owner: p.owner });
    } catch (e) {
      console.error("  ERR " + p.name + ": " + e.message);
    }
  }

  console.log("\n=== SUMMARY ===");
  console.log("Users: " + userMigrated + " migrated, " + userSkipped + " skipped");
  console.log("Projects: " + newKeys.length + " migrated with NEW API keys");

  if (newKeys.length > 0) {
    console.log("\n=== NEW PROJECT API KEYS (shown ONCE — save these now) ===");
    for (var k of newKeys) {
      console.log("  " + k.name + " (owner: " + k.owner + "):");
      console.log("    " + k.apiKey);
    }
    console.log("\nDistribute these to whoever needs to submit inference for each project.");
  }

  console.log("\nSecrets file: " + secretStore.SECRETS_PATH);
  console.log("Done.");
  await mongoose.disconnect();
}

main().catch(function (err) {
  console.error("Migration failed:", err);
  process.exit(1);
});
