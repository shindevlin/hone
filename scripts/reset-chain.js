"use strict";

/**
 * BTCPC Chain Reset — Genesis v3.0
 * Shin Devlin
 *
 * Performs the actual v3.0 chain restart:
 *   1. Reads data/genesis-migration.json (produced by genesis-migration.js)
 *   2. Reads the whitepaper (for Dream #0 inscription)
 *   3. Wipes data/blocks/*, data/statedb/*, data/pending-entries.jsonl
 *   4. Creates epoch 0 block with all migration entries + Dream #0
 *   5. Initializes stateStore from the genesis entries
 *   6. Writes data/blocks/block-00000000.bin
 *   7. Prints summary: accounts migrated, total supply, genesis hash
 *
 * Usage:
 *   node scripts/reset-chain.js
 *
 * DANGER: This permanently wipes the current chain. Run genesis-migration.js first
 * to snapshot the current state.
 */

const path = require("path");
const fs = require("fs");
const crypto = require("crypto");

// Resolve paths relative to repo root regardless of cwd
const REPO_ROOT = path.resolve(__dirname, "..");
process.chdir(REPO_ROOT);

const Block = require(path.join(REPO_ROOT, "src/chain/block"));
const blockStore = require(path.join(REPO_ROOT, "src/chain/blockStore"));
const stateStore = require(path.join(REPO_ROOT, "src/chain/stateStore"));
const stateManager = require(path.join(REPO_ROOT, "src/chain/stateManager"));

const MIGRATION_PATH = path.join(REPO_ROOT, "data", "genesis-migration.json");
const WHITEPAPER_PATH = path.join(REPO_ROOT, "docs", "BTCPC_WHITEPAPER.md");
const BLOCKS_DIR = path.join(REPO_ROOT, "data", "blocks");
const STATEDB_DIR = path.join(REPO_ROOT, "data", "statedb");
const PENDING_ENTRIES_PATH = path.join(REPO_ROOT, "data", "pending-entries.jsonl");

const GENESIS_MINER = "shindevlin";
const GENESIS_MESSAGE = "The Answer to the Ultimate Question of Life, the Universe, and Everything";
const GENESIS_STATE_HASH = "0".repeat(64);

// ── Safety check ────────────────────────────────────────────────────
function confirmDanger() {
  // In CI or non-TTY environments, require an explicit env flag
  if (!process.stdin.isTTY) {
    if (process.env.BTCPC_RESET_CONFIRMED !== "yes") {
      console.error("[ABORT] Non-interactive mode: set BTCPC_RESET_CONFIRMED=yes to proceed.");
      process.exit(1);
    }
    return;
  }

  // Interactive: print a very visible warning. The user must type the phrase.
  console.log("\n" + "!".repeat(60));
  console.log("  DANGER: This will permanently wipe the current chain.");
  console.log("  ALL block files, statedb, and pending entries will be");
  console.log("  deleted and replaced with the v3.0 genesis block.");
  console.log("\n  Only proceed if you have run:");
  console.log("    node scripts/genesis-migration.js");
  console.log("\n  Type 'reset the chain' to confirm, or Ctrl+C to abort.");
  console.log("!".repeat(60) + "\n");

  const readline = require("readline");
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  return new Promise((resolve, reject) => {
    rl.question("Confirm: ", (answer) => {
      rl.close();
      if (answer.trim().toLowerCase() === "reset the chain") {
        resolve();
      } else {
        console.log("[ABORT] Confirmation not received. Chain untouched.");
        process.exit(0);
      }
    });
  });
}

// ── Directory wipe ───────────────────────────────────────────────────
function wipeDir(dirPath) {
  if (!fs.existsSync(dirPath)) return 0;
  const files = fs.readdirSync(dirPath);
  let count = 0;
  for (const f of files) {
    const full = path.join(dirPath, f);
    const stat = fs.statSync(full);
    if (stat.isFile()) {
      fs.unlinkSync(full);
      count++;
    } else if (stat.isDirectory()) {
      fs.rmSync(full, { recursive: true });
      count++;
    }
  }
  return count;
}

// ── Main ─────────────────────────────────────────────────────────────
async function run() {
  console.log("=".repeat(60));
  console.log("  BTCPC Chain Reset — Genesis v3.0");
  console.log("=".repeat(60));

  // ── Step 0: Danger confirmation ──────────────────────────────────
  await confirmDanger();

  // ── Step 1: Read genesis-migration.json ─────────────────────────
  console.log("\n[1/7] Reading genesis-migration.json...");
  if (!fs.existsSync(MIGRATION_PATH)) {
    console.error("[ERROR] genesis-migration.json not found at: " + MIGRATION_PATH);
    console.error("        Run: node scripts/genesis-migration.js");
    process.exit(1);
  }
  const migration = JSON.parse(fs.readFileSync(MIGRATION_PATH, "utf8"));
  console.log("[1/7] Source chain height: epoch " + migration.source_chain_height);
  console.log("[1/7] Accounts to migrate: " + migration.accounts.length);
  console.log("[1/7] System accounts: " + migration.system_accounts.length);
  console.log("[1/7] Genesis entries: " + migration.genesis_entries.length);
  console.log("[1/7] Total BTCPC: " + migration.total_supply_migrated);

  // ── Step 2: Read whitepaper ──────────────────────────────────────
  console.log("\n[2/7] Reading whitepaper for Dream #0...");
  if (!fs.existsSync(WHITEPAPER_PATH)) {
    console.error("[ERROR] Whitepaper not found at: " + WHITEPAPER_PATH);
    process.exit(1);
  }
  const whitepaper = fs.readFileSync(WHITEPAPER_PATH, "utf8");
  console.log("[2/7] Whitepaper: " + whitepaper.length + " chars");

  // ── Step 3: Wipe existing chain data ────────────────────────────
  console.log("\n[3/7] Wiping current chain data...");
  const blocksWiped = wipeDir(BLOCKS_DIR);
  const statedbWiped = wipeDir(STATEDB_DIR);
  let pendingWiped = 0;
  if (fs.existsSync(PENDING_ENTRIES_PATH)) {
    fs.unlinkSync(PENDING_ENTRIES_PATH);
    pendingWiped = 1;
  }
  console.log("[3/7] Wiped: " + blocksWiped + " block file(s), " +
              statedbWiped + " statedb file(s), " +
              (pendingWiped ? "pending-entries.jsonl" : "no pending-entries.jsonl"));

  // Reset in-memory state
  stateStore.resetAll();
  stateManager.reset();

  // ── Step 4: Build genesis block entries ─────────────────────────
  console.log("\n[4/7] Building genesis block entries...");
  const now = new Date();
  const nowIso = now.toISOString();

  // Dream #0 — whitepaper inscription
  const genesisDreamEntry = {
    type: "GENESIS_DREAM",
    from: GENESIS_MINER,
    to: GENESIS_MINER,
    token: "BTCPC",
    amount: 0,
    epoch: 0,
    timestamp: nowIso,
    signed_by: GENESIS_MINER,
    memo: "Genesis Dream #0 — BTCPC v3.0 Whitepaper",
    account_data: {
      block_number: 0,
      original_miner: GENESIS_MINER,
      inscription: {
        project: "btcpc",
        tag: "Genesis — The chain dreamed itself into existence",
        custom_data: {
          title: "Bitcoin Proof of Compute — Whitepaper v3.0",
          author: "Shin Devlin",
          version: "3.0",
          message: GENESIS_MESSAGE,
          content: whitepaper,
        },
      },
      proof: {
        state_hash: GENESIS_STATE_HASH,
        work_hash: GENESIS_STATE_HASH,
        tokens_computed: 0,
        model: "genesis",
      },
    },
  };

  // All migration entries (ACCOUNT_CREATE + GENESIS_MINT for all accounts)
  const allEntries = [genesisDreamEntry, ...migration.genesis_entries];
  console.log("[4/7] Total genesis entries: " + allEntries.length);

  // ── Step 5: Apply entries to stateStore + stateManager ──────────
  console.log("\n[5/7] Initializing stateStore from genesis entries...");
  stateStore.applyEntries(allEntries);
  stateManager.applyLedgerEntries(allEntries);

  stateStore.setEpoch(0, {
    epoch_number: 0,
    started_at: now,
    ended_at: now,
    block_reward: 0,
    total_work: 0,
    consensus_hash: GENESIS_STATE_HASH,
    state_root: GENESIS_STATE_HASH,
    status: "finalized",
    rewards_distributed: [],
  });
  stateStore.setChainHeight(0);

  const accountCount = stateStore.getAccountCount();
  console.log("[5/7] Accounts in stateStore: " + accountCount);

  // ── Step 6: Write block-00000000.bin ────────────────────────────
  console.log("\n[6/7] Writing genesis block to disk...");
  blockStore.ensureBlockDir();

  const stateRoot = stateManager.getStateRoot();
  const txHashes = allEntries.map((e) => blockStore.hashLedgerEntry(e));
  const txMerkleRoot = Block.computeMerkleRoot(txHashes);

  const genesisBlock = new Block({
    version: 3,
    epoch_number: 0,
    previous_block_hash: "0".repeat(64),
    merkle_root_transactions: txMerkleRoot,
    merkle_root_compute_proofs: "0".repeat(64),
    state_root: stateRoot,
    timestamp: now.getTime(),
    difficulty: 1,
    miner_id: GENESIS_MINER,
  });

  const payload = {
    ledger_entries: allEntries,
    rewards: [],
    compute_proofs: [],
    mining_proofs: [],
    genesis_migration: {
      source_chain_height: migration.source_chain_height,
      migration_timestamp: migration.migration_timestamp,
      accounts_migrated: migration.accounts.length,
      total_supply_migrated: migration.total_supply_migrated,
    },
  };

  blockStore.writeBlock(genesisBlock, payload);
  const blockHash = genesisBlock.computeHash();
  console.log("[6/7] Genesis block written: " + blockHash.slice(0, 16) + "...");
  console.log("[6/7] State root:            " + stateRoot.slice(0, 16) + "...");
  console.log("[6/7] Tx Merkle root:        " + txMerkleRoot.slice(0, 16) + "...");

  // ── Step 7: Write initial finality snapshot ──────────────────────
  console.log("\n[7/7] Writing initial finality snapshot...");
  try {
    const snapshot = stateStore.getSnapshot ? stateStore.getSnapshot() : null;
    if (snapshot) {
      blockStore.writeFinality(genesisBlock, snapshot);
      console.log("[7/7] Finality snapshot written at epoch 0");
    } else {
      console.log("[7/7] No getSnapshot() available — skipping finality write (non-fatal)");
    }
  } catch (e) {
    console.log("[7/7] Finality snapshot skipped: " + e.message + " (non-fatal)");
  }

  // ── Summary ──────────────────────────────────────────────────────
  console.log("\n" + "=".repeat(60));
  console.log("  v3.0 Genesis Block Created");
  console.log("=".repeat(60));
  console.log("  Genesis block hash:      " + blockHash.slice(0, 32) + "...");
  console.log("  State root:              " + stateRoot.slice(0, 32) + "...");
  console.log("  Accounts migrated:       " + migration.accounts.length + " user + " +
              migration.system_accounts.length + " system");
  console.log("  Total BTCPC supply:      " + migration.total_supply_migrated);
  console.log("  Genesis entries:         " + allEntries.length);
  console.log("  Dream #0 whitepaper:     " + whitepaper.length + " chars");
  console.log("  Source chain height:     epoch " + migration.source_chain_height);
  console.log("=".repeat(60));
  console.log("\n  The chain has been reset. Start nodes to begin epoch 1.");
  console.log("  node bin/btcpc-all\n");

  process.exit(0);
}

run().catch(function (err) {
  console.error("[FATAL] reset-chain.js failed: " + err.message);
  console.error(err.stack);
  process.exit(1);
});
