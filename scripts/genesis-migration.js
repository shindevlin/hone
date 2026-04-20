"use strict";

/**
 * BTCPC Genesis Migration Snapshot
 * Shin Devlin
 *
 * Replays the current chain from disk, then exports a complete snapshot of all
 * account state into data/genesis-migration.json. This file is the input for
 * scripts/reset-chain.js when the v3.0 chain restarts at epoch 0.
 *
 * Usage:
 *   node scripts/genesis-migration.js
 *
 * Output:
 *   data/genesis-migration.json
 */

const path = require("path");
const fs = require("fs");

// Resolve paths relative to repo root regardless of cwd
const REPO_ROOT = path.resolve(__dirname, "..");
process.chdir(REPO_ROOT);

const replay = require(path.join(REPO_ROOT, "src/chain/replay"));
const stateStore = require(path.join(REPO_ROOT, "src/chain/stateStore"));
const blockStore = require(path.join(REPO_ROOT, "src/chain/blockStore"));
const { RESERVED_OWNER, getReservedNamesForShin } = require(path.join(REPO_ROOT, "src/services/reservedNames"));

const OUTPUT_PATH = path.join(REPO_ROOT, "data", "genesis-migration.json");
const WHITEPAPER_PATH = path.join(REPO_ROOT, "docs", "BTCPC_WHITEPAPER.md");

const SYSTEM_ACCOUNTS = ["btcpc_recycle", "btcpc_treasury", "btcpc_staking_pool", "btcpc_escrow", "btcpc_genesis", "btcpc_mint"];

async function run() {
  console.log("=".repeat(60));
  console.log("  BTCPC Genesis Migration Snapshot");
  console.log("=".repeat(60));

  // ── Step 1: Replay current chain from disk ──────────────────────
  console.log("\n[1/5] Replaying chain from disk...");
  const replayResult = await replay.replayFromDisk({ verbose: true });
  const chainHeight = replayResult.chainHeight;
  console.log("[1/5] Chain height: epoch " + chainHeight);
  console.log("[1/5] Accounts in state: " + replayResult.accounts);

  if (chainHeight < 0) {
    console.error("[ERROR] No blocks found on disk. Run the chain at least once before migrating.");
    process.exit(1);
  }

  // ── Step 1b: Load keys from Mongo (fallback for pre-v2.13.1 accounts) ──
  // stateStore has empty public_keys for accounts created before the cross-
  // process queue fix. Mongo's User collection still has the keys from the
  // original createAccount() call. Pull them as a fallback.
  let mongoKeys = {};
  try {
    const mongoose = require("mongoose");
    await mongoose.connect(
      process.env.MONGODB_URI || "mongodb://root:example@localhost:27017/btcpc?authSource=admin",
      { serverSelectionTimeoutMS: 5000 }
    );
    const User = require(path.join(REPO_ROOT, "src/models/User"));
    const users = await User.find({}).lean();
    for (const u of users) {
      if (u.ownerPublicKey) {
        mongoKeys[u.username] = {
          public_keys: {
            owner: u.ownerPublicKey,
            active: u.activePublicKey || null,
            posting: u.postingPublicKey || null,
            memo: u.memoPublicKey || null,
          },
          email: u.email,
          telegram_id: u.telegramId,
        };
      }
    }
    await mongoose.disconnect();
    console.log("[1b] Loaded " + Object.keys(mongoKeys).length + " key sets from Mongo");
  } catch (e) {
    console.log("[1b] Mongo unavailable (" + e.message + ") — using stateStore keys only");
  }

  // ── Step 2: Export all accounts ─────────────────────────────────
  console.log("\n[2/5] Exporting accounts...");
  const allAccounts = stateStore.getAllAccounts();
  const accounts = [];
  const systemAccountEntries = [];
  let totalSupplyMigrated = 0;

  for (const acct of allAccounts) {
    const username = acct.username;

    // Pull full account detail via getAccount (includes balance + stake)
    const detail = stateStore.getAccount(username);

    // Pull all token balances (BTCPC + any user-created tokens)
    const tokenBalances = stateStore.getTokenBalances(username);

    const isSystem = SYSTEM_ACCOUNTS.indexOf(username) !== -1 ||
                     username.startsWith("project:") ||
                     username.startsWith("escrow:");

    const btcpcBalance = detail ? detail.balance : 0;

    if (isSystem) {
      systemAccountEntries.push({
        username: username,
        balance: btcpcBalance,
        token_balances: tokenBalances,
      });
    } else {
      totalSupplyMigrated += btcpcBalance;
      // Merge keys: prefer stateStore (on-chain), fall back to Mongo
      let publicKeys = acct.public_keys || {};
      if ((!publicKeys.owner) && mongoKeys[username]) {
        publicKeys = mongoKeys[username].public_keys;
        console.log("[2/5]   " + username + " — keys recovered from Mongo ✓");
      }

      accounts.push({
        username: username,
        balance: btcpcBalance,
        token_balances: tokenBalances,
        public_keys: publicKeys,
        chain_addresses: acct.chain_addresses || {},
        created_epoch: acct.created_epoch || 0,
      });
    }
  }

  // Also count system account balances in total
  for (const sys of systemAccountEntries) {
    totalSupplyMigrated += sys.balance || 0;
  }

  console.log("[2/5] User accounts: " + accounts.length);
  console.log("[2/5] System accounts: " + systemAccountEntries.length);
  console.log("[2/5] Total BTCPC migrated: " + totalSupplyMigrated.toFixed(10));

  // ── Step 3: Build genesis entries ───────────────────────────────
  console.log("\n[3/5] Building genesis entries...");
  const genesisEntries = [];
  const now = new Date().toISOString();
  const existingAccountNames = new Set(accounts.map(function (acct) { return acct.username; }));
  const existingSystemNames = new Set(systemAccountEntries.map(function (acct) { return acct.username; }));

  // ACCOUNT_CREATE entries — one per user account, preserving all keys.
  // Shape matches what stateStore.applyEntry expects:
  //   entry.to = username, entry.account_data = { public_keys, chain_addresses }
  for (const acct of accounts) {
    genesisEntries.push({
      type: "ACCOUNT_CREATE",
      from: "btcpc_genesis",
      to: acct.username,
      amount: 0,
      token: "BTCPC",
      epoch: 0,
      timestamp: now,
      signed_by: "btcpc_genesis",
      memo: "genesis migration v3.0",
      account_data: {
        public_keys: acct.public_keys,
        chain_addresses: acct.chain_addresses,
      },
    });
  }

  // Reserved names are represented as nested wallets under Shin, not as
  // native top-level accounts. A later WALLET_UNNEST promotes one reservation
  // to its native name with the recipient's public keys.
  let reservedNestedCount = 0;
  if (existingAccountNames.has(RESERVED_OWNER) || stateStore.hasAccount(RESERVED_OWNER)) {
    const reservedNames = getReservedNamesForShin();
    const existingNested = new Set();
    for (const name of reservedNames) {
      const nestedWallet = RESERVED_OWNER + "/" + name;
      if (existingAccountNames.has(name) || existingSystemNames.has(name)) continue;
      if (existingAccountNames.has(nestedWallet) || existingNested.has(nestedWallet)) continue;
      genesisEntries.push({
        type: "WALLET_CREATE_CHILD",
        from: RESERVED_OWNER,
        to: nestedWallet,
        amount: 0,
        token: "BTCPC",
        epoch: 0,
        timestamp: now,
        signed_by: "btcpc_genesis",
        memo: "genesis migration v3.4 — reserve premium name as nested wallet for shindevlin",
        wallet_data: {
          parent: RESERVED_OWNER,
          child_name: name,
          nested_name: name,
        },
      });
      existingNested.add(nestedWallet);
      reservedNestedCount++;
    }
  }

  // GENESIS_MINT entries — one per user account, preserving exact balance.
  // GENESIS_MINT is handled like FAUCET/MINING_REWARD: credits `to` without debiting anyone.
  for (const acct of accounts) {
    if (acct.balance > 0) {
      genesisEntries.push({
        type: "GENESIS_MINT",
        from: "btcpc_genesis",
        to: acct.username,
        token: "BTCPC",
        amount: acct.balance,
        epoch: 0,
        timestamp: now,
        signed_by: "btcpc_genesis",
        memo: "genesis migration v3.0 — balance preserved from epoch " + chainHeight,
      });
    }
    // Mint any user-created token balances too
    for (const [tok, bal] of Object.entries(acct.token_balances || {})) {
      if (tok !== "BTCPC" && bal > 0) {
        genesisEntries.push({
          type: "GENESIS_MINT",
          from: "btcpc_genesis",
          to: acct.username,
          token: tok,
          amount: bal,
          epoch: 0,
          timestamp: now,
          signed_by: "btcpc_genesis",
          memo: "genesis migration v3.0 — " + tok + " balance preserved",
        });
      }
    }
  }

  // System account initializations
  for (const sys of systemAccountEntries) {
    genesisEntries.push({
      type: "ACCOUNT_CREATE",
      from: "btcpc_genesis",
      to: sys.username,
      amount: 0,
      token: "BTCPC",
      epoch: 0,
      timestamp: now,
      signed_by: "btcpc_genesis",
      memo: "genesis migration v3.0 — system account",
      account_data: { public_keys: {}, chain_addresses: {} },
    });
    if (sys.balance > 0) {
      genesisEntries.push({
        type: "GENESIS_MINT",
        from: "btcpc_genesis",
        to: sys.username,
        token: "BTCPC",
        amount: sys.balance,
        epoch: 0,
        timestamp: now,
        signed_by: "btcpc_genesis",
        memo: "genesis migration v3.0 — system account balance preserved",
      });
    }
  }

  console.log("[3/5] Genesis entries built: " + genesisEntries.length);

  // ── Step 4: Verify whitepaper exists ────────────────────────────
  console.log("\n[4/5] Verifying whitepaper...");
  if (!fs.existsSync(WHITEPAPER_PATH)) {
    console.error("[ERROR] Whitepaper not found at: " + WHITEPAPER_PATH);
    console.error("        Run the whitepaper rewrite before the migration.");
    process.exit(1);
  }
  const whitepaperStat = fs.statSync(WHITEPAPER_PATH);
  console.log("[4/5] Whitepaper found: " + whitepaperStat.size + " bytes");

  // ── Step 5: Write output file ────────────────────────────────────
  console.log("\n[5/5] Writing genesis-migration.json...");
  const output = {
    version: 1,
    source_chain_height: chainHeight,
    migration_timestamp: now,
    accounts: accounts,
    system_accounts: systemAccountEntries,
    reserved_nested_wallets_owner: RESERVED_OWNER,
    reserved_nested_wallets_count: reservedNestedCount,
    genesis_entries: genesisEntries,
    total_supply_migrated: parseFloat(totalSupplyMigrated.toFixed(10)),
    whitepaper_path: "docs/BTCPC_WHITEPAPER.md",
    whitepaper_size_bytes: whitepaperStat.size,
  };

  const dataDir = path.join(REPO_ROOT, "data");
  if (!fs.existsSync(dataDir)) fs.mkdirSync(dataDir, { recursive: true });
  fs.writeFileSync(OUTPUT_PATH, JSON.stringify(output, null, 2), "utf8");

  console.log("[5/5] Written to: " + OUTPUT_PATH);

  // ── Summary ──────────────────────────────────────────────────────
  console.log("\n" + "=".repeat(60));
  console.log("  Migration Snapshot Complete");
  console.log("=".repeat(60));
  console.log("  Source chain height:  epoch " + chainHeight);
  console.log("  User accounts:        " + accounts.length);
  console.log("  System accounts:      " + systemAccountEntries.length);
  console.log("  Reserved nested:      " + reservedNestedCount);
  console.log("  Genesis entries:      " + genesisEntries.length);
  console.log("  Total BTCPC:          " + totalSupplyMigrated.toFixed(4));
  console.log("  Output:               data/genesis-migration.json");
  console.log("=".repeat(60));
  console.log("\nNext step: node scripts/reset-chain.js");

  process.exit(0);
}

run().catch(function (err) {
  console.error("[FATAL] genesis-migration.js failed: " + err.message);
  console.error(err.stack);
  process.exit(1);
});
