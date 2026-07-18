"use strict";

/**
 * HONE Epoch Finalizer
 * Shin Devlin
 *
 * Shared module: applies an epoch's winning consensus proposal to
 * the ledger, stateStore, and block files.
 *
 * Used by both hone-mine (fallback, single-node) and hone-clock
 * (correct: clock owns finalization in multi-node deployments).
 *
 * Target architecture:
 *   verifiers compute rewards → finalizationConsensus → clock applies via applyFinalization
 * Transition flag:
 *   HONE_MINER_CLOCK=false  disables the clock loop inside hone-mine
 */

var ledger       = require("../services/ledger");
var stateStore   = require("./stateStore");
var stateManager = require("./stateManager");
var blockStore   = require("./blockStore");
var Block        = require("./block");
var blockchain   = require("./blockchain");
var mempool      = require("../p2p/mempool");
var finalityAnchoring = require("./finalityAnchoring");

var FINALITY_INTERVAL = parseInt(process.env.HONE_FINALITY_INTERVAL) || 100;

/**
 * Apply the winning consensus proposal: write rewards to ledger, mark epoch
 * finalized in stateStore, write the block file, and handle finality snapshots.
 *
 * Idempotent: returns the existing epoch if already finalized.
 *
 * @param {number} epochNumber
 * @param {object} proposal — winning finalization proposal from finalizationConsensus
 * @param {string} authorAccount — account name of the node writing this block (clock or miner)
 * @returns {Promise<object|null>} epoch record with _blockData attached for broadcast, or null
 */
async function applyFinalization(epochNumber, proposal, authorAccount) {
  var epoch = stateStore.getEpoch(epochNumber);
  if (!epoch) {
    epoch = {
      epoch_number: epochNumber,
      status: "active",
      started_at: new Date(),
      block_reward: proposal.block_reward || 0,
      commitments: [],
    };
  }
  if (epoch.status === "finalized") return epoch;

  var rewards = proposal.rewards || [];

  var epochProofs = stateStore.getMiningProofs(epochNumber).slice();
  for (var i = 0; i < rewards.length; i++) {
    var r = rewards[i];
    await ledger.recordMiningReward(r.miner, r.amount, epochNumber, null, null, r.type || "mining");
    for (var j = 0; j < epochProofs.length; j++) {
      if (epochProofs[j].miner === r.miner) {
        epochProofs[j].reward_earned = r.amount;
        break;
      }
    }
    console.log("[HONE]   " + r.miner + ": " + r.amount.toFixed(4) + " HONE (" + (r.type || "mining") + ")");
  }
  stateStore.setMiningProofs(epochNumber, epochProofs);

  epoch.consensus_hash = proposal.consensus_hash || "0".repeat(64);
  epoch.total_work = proposal.total_work || 0;
  epoch.consensus_nodes = proposal.consensus_nodes || 1;
  epoch.consensus_proposals = proposal.consensus_proposals || 1;
  epoch.rewards_distributed = rewards.map(function (r) {
    return { node_id: r.miner, amount: r.amount, type: r.type || "mining" };
  });
  epoch.block_reward = proposal.block_reward || 0;
  epoch.reward_number = proposal.reward_number;
  epoch.epochs_deferred = proposal.epochs_deferred || 0;
  epoch.ended_at = new Date();
  epoch.status = "finalized";
  epoch.settled_jobs = proposal.settled_jobs || 0;
  stateStore.setEpoch(epochNumber, epoch);

  var rewardNumber = proposal.reward_number !== undefined ? proposal.reward_number : epochNumber;
  console.log("[HONE] Epoch " + epochNumber + " finalized | reward #" + rewardNumber +
    " | " + rewards.length + " reward(s) | " + (proposal.block_reward || 0).toFixed(4) + " HONE");

  try {
    var epochLedgerEntries = ledger.flushPendingEntries();
    stateManager.applyLedgerEntries(epochLedgerEntries);
    try {
      stateStore.applyEntries(epochLedgerEntries);
    } catch (_) {}
    var stateRoot = stateManager.getStateRoot();

    var txHashes = epochLedgerEntries.map(function (e) { return blockStore.hashLedgerEntry(e); });
    var txMerkleRoot = Block.computeMerkleRoot(txHashes);

    var recentProofs = stateStore.getRecentComputeProofs
      ? stateStore.getRecentComputeProofs(epochNumber, 3)
      : stateStore.getComputeProofs(epochNumber);
    var proofHashes = recentProofs.map(function (p) { return blockStore.hashComputeProof(p); });
    var cpMerkleRoot = Block.computeMerkleRoot(proofHashes);

    var prevHash = "0".repeat(64);
    if (epochNumber > 0) {
      var prevHeader = blockStore.readBlockHeader(epochNumber - 1);
      if (prevHeader) prevHash = prevHeader.computeHash();
    }

    var block = new Block({
      version: 1,
      epoch_number: epochNumber,
      previous_block_hash: prevHash,
      merkle_root_transactions: txMerkleRoot,
      merkle_root_compute_proofs: cpMerkleRoot,
      state_root: stateRoot,
      timestamp: epoch.ended_at.getTime(),
      difficulty: epoch.difficulty || 1,
      miner_id: authorAccount || "hone-node"
    });

    var miningProofs = stateStore.getMiningProofs(epochNumber);
    var payload = {
      ledger_entries: epochLedgerEntries,
      consensus_nodes: proposal.consensus_nodes || 1,
      consensus_proposals: proposal.consensus_proposals || 1,
      rewards: rewards.map(function (r) { return { miner: r.node_id || r.miner, amount: r.amount }; }),
      compute_proofs: recentProofs.map(function (p) {
        return {
          node_id: p.node_id, prompt_hash: p.prompt_hash,
          result_hash: p.result_hash, model: p.model,
          tokens_generated: p.tokens_generated, work_value: p.work_value,
          tools_used: p.tools_used || null,
          tool_trace_hash: p.tool_trace_hash || null,
        };
      }),
      mining_proofs: miningProofs.map(function (p) {
        return {
          miner: p.miner, reward_earned: p.reward_earned,
          model: p.model, tokens_computed: p.tokens_computed,
          work_value: p.work_value, state_hash: p.state_hash
        };
      })
    };

    blockStore.writeBlock(block, payload);
    blockchain.addBlock(block);

    var blockHash = block.computeHash();
    console.log("[HONE] Block " + epochNumber + " written | " + blockHash.slice(0, 16) + "... | state: " + stateRoot.slice(0, 16) + "...");

    if (epochNumber > 0 && epochNumber % FINALITY_INTERVAL === 0) {
      var snapshot = stateManager.generateFinalitySnapshot();
      var prevFinalityEpoch = epochNumber - FINALITY_INTERVAL;
      var prevFinalityHash = "0".repeat(64);
      if (prevFinalityEpoch >= 0 && blockStore.hasFinality(prevFinalityEpoch)) {
        var prevFin = blockStore.readFinality(prevFinalityEpoch);
        if (prevFin && prevFin.snapshot.rolling_commitment) {
          prevFinalityHash = prevFin.snapshot.rolling_commitment;
        }
      }
      var crypto = require("crypto");
      snapshot.rolling_commitment = crypto.createHash("sha256")
        .update(prevFinalityHash + stateRoot)
        .digest("hex");
      snapshot.finality_epoch = epochNumber;
      snapshot.block_hash = blockHash;

      blockStore.writeFinality(block, snapshot);
      console.log("[HONE] Finality block " + epochNumber + " written | " + snapshot.account_count +
        " accounts | commitment: " + snapshot.rolling_commitment.slice(0, 16) + "...");

      finalityAnchoring.anchorIfDue(epochNumber, snapshot).catch(function (err) {
        console.warn("[HONE][anchor] anchorIfDue error (non-fatal):", err.message);
      });

      var pruned = blockStore.pruneBeforeFinality(epochNumber);
      if (pruned > 0) {
        console.log("[HONE] Lucid Pruning: " + pruned + " block files pruned (before epoch " + epochNumber + ")");
      }
    }

    var mempoolTxs = mempool.getTransactions();
    var clearedHashes = mempoolTxs.map(function (t) { return t.txHash; }).filter(Boolean);
    if (clearedHashes.length > 0) {
      mempool.removeTransactions(clearedHashes);
      console.log("[HONE] Mempool: " + clearedHashes.length + " transactions included in block " + epochNumber);
    }

    epoch._blockData = {
      header_hex: block.serialize().toString("hex"),
      block_hash: blockHash,
      state_root: stateRoot,
      ledger: epochLedgerEntries,
      is_finality: epochNumber > 0 && epochNumber % FINALITY_INTERVAL === 0
    };
  } catch (err) {
    console.error("[HONE] Failed to write block to disk: " + err.message);
    epoch._blockData = { ledger: ledger.flushPendingEntries() };
  }

  return epoch;
}

module.exports = {
  applyFinalization: applyFinalization,
  FINALITY_INTERVAL: FINALITY_INTERVAL,
};
