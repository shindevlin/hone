"use strict";

/**
 * BTCPC Four-Tier Finality Anchoring
 * Shin Devlin
 *
 * Anchors finality snapshots at increasing cadences and to increasing
 * external networks, giving the chain cryptographic commitments that
 * span from local disk all the way to Bitcoin's PoW.
 *
 * Tier 1 — Native  (every 100 epochs):  finality snapshot to local disk
 *                                        (written by blockStore, no work here)
 * Tier 2 — L2      (every 100 epochs):  L2_ANCHOR ledger entry on BTCPC chain
 * Tier 3 — Ethereum(every 1000 epochs): ABI-encoded calldata → data/anchors/eth-NNNNN.hex
 *                                        + ETH_ANCHOR ledger entry
 * Tier 4 — Bitcoin (every 10000 epochs):OP_RETURN payload → data/anchors/btc-NNNNN.hex
 *                                        + BTC_ANCHOR ledger entry
 *
 * Activation gate:
 *   BTCPC_FINALITY_TIERS  (default: "1") — how many tiers to activate
 *   BTCPC_MIN_NODES_FOR_L2 (default: 10) — minimum live peers before Tier 2+
 *
 * Called from miner.js immediately after blockStore.writeFinality().
 */

var fs = require("fs");
var path = require("path");
var crypto = require("crypto");
var stateStore = require("./stateStore");
var nodeRegistry = require("./nodeRegistry");

// ─── Configuration ────────────────────────────────────────────────

var TIER_LEVEL = (process.env.BTCPC_FINALITY_TIERS !== undefined && process.env.BTCPC_FINALITY_TIERS !== "")
  ? parseInt(process.env.BTCPC_FINALITY_TIERS, 10) || 1
  : 1;
var MIN_NODES_FOR_L2 = (process.env.BTCPC_MIN_NODES_FOR_L2 !== undefined && process.env.BTCPC_MIN_NODES_FOR_L2 !== "")
  ? parseInt(process.env.BTCPC_MIN_NODES_FOR_L2, 10)
  : 10;
// Guard: NaN → use defaults
if (isNaN(TIER_LEVEL)) TIER_LEVEL = 1;
if (isNaN(MIN_NODES_FOR_L2)) MIN_NODES_FOR_L2 = 10;

var ANCHORS_DIR = path.resolve(__dirname, "../../data/anchors");

// ─── Cadences ─────────────────────────────────────────────────────

var L2_CADENCE  = 100;     // every 100 epochs
var ETH_CADENCE = 1000;    // every 1000 epochs
var BTC_CADENCE = 10000;   // every 10000 epochs

// ─── In-memory anchor log (ring buffer, last 100 entries) ─────────
// Shared with stateStore via getRecentAnchors(). See stateStore additions.

var anchorLog = [];
var ANCHOR_LOG_CAP = 100;

function _pushAnchor(entry) {
  anchorLog.push(entry);
  if (anchorLog.length > ANCHOR_LOG_CAP) {
    anchorLog.splice(0, anchorLog.length - ANCHOR_LOG_CAP);
  }
}

/**
 * Return the last up-to-100 anchor log entries.
 */
function getRecentAnchors() {
  return anchorLog.slice();
}

// ─── Helpers ──────────────────────────────────────────────────────

function _ensureAnchorsDir() {
  if (!fs.existsSync(ANCHORS_DIR)) {
    fs.mkdirSync(ANCHORS_DIR, { recursive: true });
  }
}

/**
 * Compute SHA-256 of finalitySnapshot bytes (JSON-serialised).
 * This is the "state root" used by all anchor tiers.
 */
function _computeStateRoot(finalitySnapshot) {
  var json = typeof finalitySnapshot === "string"
    ? finalitySnapshot
    : JSON.stringify(finalitySnapshot);
  return crypto.createHash("sha256").update(json, "utf8").digest("hex");
}

/**
 * Get live peer / node count. Tries nodeRegistry first (in-memory,
 * always available), then stateStore.getAccountCount() as fallback.
 */
function _getLiveNodeCount() {
  try {
    var count = nodeRegistry.getNodeCount();
    if (typeof count === "number") return count;
  } catch (_) {}
  return 0;
}

/**
 * Write a file atomically (tmp → rename).
 */
function _writeAtomic(filePath, content) {
  var tmpPath = filePath + ".tmp";
  fs.writeFileSync(tmpPath, content);
  fs.renameSync(tmpPath, filePath);
}

// ─── Tier 2: L2 anchor ────────────────────────────────────────────

/**
 * Write an L2_ANCHOR entry to the in-memory anchor log and to
 * stateStore so it becomes part of the BTCPC chain record.
 */
function _anchorL2(epochNumber, stateRoot) {
  var entry = {
    type: "L2_ANCHOR",
    epoch: epochNumber,
    state_root: stateRoot,
    anchor_level: "l2",
    anchored_at: Date.now(),
  };

  // Persist to stateStore (no-op if type is unknown — handled by applyEntry default)
  try { stateStore.applyEntry(entry); } catch (_) {}

  _pushAnchor(entry);

  console.log("[BTCPC][anchor] L2 anchor @ epoch " + epochNumber
    + " | root: " + stateRoot.slice(0, 16) + "...");

  return entry;
}

// ─── Tier 3: Ethereum anchor ──────────────────────────────────────

/**
 * ABI-encode (epoch uint256, stateRoot bytes32) using minimal manual
 * encoding — no ethers.js dependency needed.
 *
 * ABI encoding for (uint256, bytes32):
 *   offset 0  : epoch as 32-byte big-endian uint256
 *   offset 32 : stateRoot as 32 bytes (already hex — pad/truncate to 32 bytes)
 *
 * Returns a hex string (no 0x prefix) of exactly 64 bytes (128 hex chars).
 */
function _abiEncodeEthAnchor(epochNumber, stateRoot) {
  // epoch → 32-byte big-endian
  var epochBuf = Buffer.alloc(32, 0);
  // Safe for epoch numbers up to 2^53 (JS Number limit) — well within range
  var epochHex = epochNumber.toString(16);
  if (epochHex.length % 2 !== 0) epochHex = "0" + epochHex;
  var epochBytes = Buffer.from(epochHex, "hex");
  epochBytes.copy(epochBuf, 32 - epochBytes.length);

  // stateRoot → 32 bytes
  var rootHex = stateRoot.replace(/^0x/, "");
  // SHA-256 hex is always 64 chars = 32 bytes; keep as-is
  rootHex = rootHex.slice(0, 64).padEnd(64, "0");
  var rootBuf = Buffer.from(rootHex, "hex");

  return Buffer.concat([epochBuf, rootBuf]).toString("hex");
}

function _anchorEthereum(epochNumber, stateRoot) {
  _ensureAnchorsDir();

  var calldata = _abiEncodeEthAnchor(epochNumber, stateRoot);
  var fileName = "eth-anchor-" + String(epochNumber).padStart(8, "0") + ".hex";
  var filePath = path.join(ANCHORS_DIR, fileName);
  _writeAtomic(filePath, calldata);

  var entry = {
    type: "ETH_ANCHOR",
    epoch: epochNumber,
    state_root: stateRoot,
    anchor_level: "ethereum",
    calldata_hex: calldata,
    anchored_at: Date.now(),
  };

  try { stateStore.applyEntry(entry); } catch (_) {}
  _pushAnchor(entry);

  console.log("[BTCPC][anchor] ETH anchor @ epoch " + epochNumber
    + " → " + filePath);

  return entry;
}

// ─── Tier 4: Bitcoin anchor ───────────────────────────────────────

/**
 * Build an OP_RETURN payload (≤80 bytes):
 *   bytes  0-4  : "BTCPC" ASCII magic (5 bytes)
 *   bytes  5-8  : epoch as uint32 big-endian (4 bytes)
 *   bytes  9-40 : state root (32 bytes)
 *   bytes 41-79 : zero padding (39 bytes) → total 80 bytes
 *
 * Returns hex string (no 0x prefix), 160 chars.
 */
function _buildBtcOpReturn(epochNumber, stateRoot) {
  var buf = Buffer.alloc(80, 0);

  // Magic: "BTCPC"
  buf.write("BTCPC", 0, "ascii");

  // Epoch as uint32 BE (max ~4.29 billion — sufficient for BTCPC epochs)
  buf.writeUInt32BE(epochNumber >>> 0, 5);

  // State root (32 bytes)
  var rootHex = stateRoot.replace(/^0x/, "").slice(0, 64).padEnd(64, "0");
  Buffer.from(rootHex, "hex").copy(buf, 9);

  return buf.toString("hex");
}

function _anchorBitcoin(epochNumber, stateRoot) {
  _ensureAnchorsDir();

  var opReturn = _buildBtcOpReturn(epochNumber, stateRoot);
  var fileName = "btc-anchor-" + String(epochNumber).padStart(8, "0") + ".hex";
  var filePath = path.join(ANCHORS_DIR, fileName);
  _writeAtomic(filePath, opReturn);

  var entry = {
    type: "BTC_ANCHOR",
    epoch: epochNumber,
    state_root: stateRoot,
    anchor_level: "bitcoin",
    op_return_hex: opReturn,
    anchored_at: Date.now(),
  };

  try { stateStore.applyEntry(entry); } catch (_) {}
  _pushAnchor(entry);

  console.log("[BTCPC][anchor] BTC anchor @ epoch " + epochNumber
    + " → " + filePath);

  return entry;
}

// ─── Main entry point ─────────────────────────────────────────────

/**
 * Evaluate which anchor tiers are due and fire them.
 *
 * @param {number} epochNumber  — the epoch that just completed finality
 * @param {object|string} finalitySnapshot — snapshot data (object or JSON string)
 * @returns {Promise<object>} result — { tiersRun: [], anchors: [] }
 */
async function anchorIfDue(epochNumber, finalitySnapshot) {
  var result = { tiersRun: [], anchors: [], skipped: false, reason: null };

  // Tier 1 is always handled by blockStore; nothing to do here.

  // Tier 2+ gate: node count must meet minimum threshold
  if (TIER_LEVEL < 2) {
    result.skipped = true;
    result.reason = "BTCPC_FINALITY_TIERS=" + TIER_LEVEL + " (native-only)";
    return result;
  }

  var liveNodes = _getLiveNodeCount();
  if (liveNodes < MIN_NODES_FOR_L2) {
    result.skipped = true;
    result.reason = "live nodes " + liveNodes + " < BTCPC_MIN_NODES_FOR_L2=" + MIN_NODES_FOR_L2;
    return result;
  }

  var stateRoot = _computeStateRoot(finalitySnapshot);

  // ── Tier 2: L2 anchor (every L2_CADENCE epochs) ──
  if (epochNumber > 0 && epochNumber % L2_CADENCE === 0) {
    var l2 = _anchorL2(epochNumber, stateRoot);
    result.tiersRun.push(2);
    result.anchors.push(l2);
  }

  // ── Tier 3: Ethereum anchor (every ETH_CADENCE epochs) ──
  if (TIER_LEVEL >= 3 && epochNumber > 0 && epochNumber % ETH_CADENCE === 0) {
    var eth = _anchorEthereum(epochNumber, stateRoot);
    result.tiersRun.push(3);
    result.anchors.push(eth);
  }

  // ── Tier 4: Bitcoin anchor (every BTC_CADENCE epochs) ──
  if (TIER_LEVEL >= 4 && epochNumber > 0 && epochNumber % BTC_CADENCE === 0) {
    var btc = _anchorBitcoin(epochNumber, stateRoot);
    result.tiersRun.push(4);
    result.anchors.push(btc);
  }

  return result;
}

// ─── Exports ──────────────────────────────────────────────────────

module.exports = {
  anchorIfDue: anchorIfDue,
  getRecentAnchors: getRecentAnchors,

  // Exposed for testing
  _computeStateRoot: _computeStateRoot,
  _abiEncodeEthAnchor: _abiEncodeEthAnchor,
  _buildBtcOpReturn: _buildBtcOpReturn,
  _getLiveNodeCount: _getLiveNodeCount,
  _anchorL2: _anchorL2,
  _anchorEthereum: _anchorEthereum,
  _anchorBitcoin: _anchorBitcoin,

  // Constants (allow tests to inspect)
  L2_CADENCE: L2_CADENCE,
  ETH_CADENCE: ETH_CADENCE,
  BTC_CADENCE: BTC_CADENCE,
  ANCHOR_LOG_CAP: ANCHOR_LOG_CAP,
};
