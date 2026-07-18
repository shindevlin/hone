"use strict";

/**
 * Auto-stake genesis miners — one-time bootstrap script.
 *
 * Reads current balances for the three genesis accounts and stakes
 * MIN_MINER_STAKE (10 HONE) for each, so they earn full mining
 * rewards once the bootstrap period (first 1000 epochs) ends.
 *
 * Safe to run multiple times: skips accounts that already have
 * sufficient stake or insufficient balance.
 *
 * Usage:  node scripts/auto-stake-genesis.js
 */

var path = require("path");

// Boot stateStore so balances are available
var stateStore = require(path.resolve(__dirname, "..", "src", "chain", "stateStore"));
var ledger = require(path.resolve(__dirname, "..", "src", "services", "ledger"));
var blockProposal = require(path.resolve(__dirname, "..", "src", "chain", "blockProposal"));

var MIN_STAKE = blockProposal.MIN_MINER_STAKE; // 10 HONE
var GENESIS_ACCOUNTS = ["natoshisakamoto", "shindevlin", "josh"];

async function main() {
  console.log("=== HONE Auto-Stake Genesis Miners ===");
  console.log("MIN_MINER_STAKE:", MIN_STAKE, "HONE\n");

  // Replay chain state so balances are populated
  if (typeof stateStore.replayBlocks === "function") {
    console.log("Replaying block history...");
    await stateStore.replayBlocks();
    console.log("Replay complete.\n");
  }

  var staked = 0;
  var skipped = 0;

  for (var account of GENESIS_ACCOUNTS) {
    var balance = stateStore.getBalance(account, "HONE");
    var pool = stateStore.getStakePool(account);
    var currentStake = (pool && pool.total_staked) ? pool.total_staked : 0;

    console.log(account + ":");
    console.log("  balance:", balance, "HONE");
    console.log("  current stake:", currentStake, "HONE");

    if (currentStake >= MIN_STAKE) {
      console.log("  -> already staked enough, skipping\n");
      skipped++;
      continue;
    }

    var needed = MIN_STAKE - currentStake;
    if (balance < needed) {
      console.log("  -> insufficient balance (need " + needed + "), skipping\n");
      skipped++;
      continue;
    }

    console.log("  -> staking " + needed + " HONE (purpose: mining)");
    await ledger.recordStake(account, needed, "mining", 0);
    staked++;

    // Verify
    var afterPool = stateStore.getStakePool(account);
    console.log("  -> new stake total: " + (afterPool ? afterPool.total_staked : 0) + " HONE\n");
  }

  console.log("Done. Staked: " + staked + ", Skipped: " + skipped);
  // Flush pending entries to disk so the miner picks them up
  if (typeof ledger.flushPendingEntries === "function") {
    var flushed = await ledger.flushPendingEntries();
    console.log("Flushed " + (flushed || 0) + " pending entries to disk.");
  }
}

main().catch(function (err) {
  console.error("Fatal:", err);
  process.exit(1);
});
