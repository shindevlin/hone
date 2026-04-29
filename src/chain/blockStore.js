"use strict";

/**
 * BTCPC Block Store — File-Based Block Storage
 * Shin Devlin
 *
 * Writes finalized blocks to disk as the chain's source of truth.
 * MongoDB is a cache; these files are the real chain.
 *
 * Block file:    data/blocks/block-NNNNNNNN.bin
 * Finality file: data/blocks/finality-NNNNNNNN.bin
 * Index file:    data/blocks/index.json
 *
 * File format:
 *   [180 bytes header] [4 bytes payload length (uint32LE)] [JSON payload]
 */

var fs = require("fs");
var path = require("path");
var crypto = require("crypto");
var Block = require("./block");
var rustChainIpc = require("./rustChainIpc");

var BLOCKS_DIR = path.resolve(__dirname, "../../data/blocks");
var _rustWarningShown = false;
var _forceFileFallback = false;

function _useRustChain() {
  if (_forceFileFallback) return false;
  return rustChainIpc.isEnabled();
}

function _rustStrict() {
  return process.env.BTCPC_RUST_CHAIN_STRICT === "true";
}

function _warnRustFallback(opName, err) {
  if (_rustWarningShown) return;
  _rustWarningShown = true;
  console.warn("[BTCPC Chain] Rust IPC fallback to file store during " + opName + ": " + err.message);
}

function _withRustFallback(opName, rustFn, fileFn) {
  if (!_useRustChain()) return fileFn();
  try {
    return rustFn();
  } catch (err) {
    if (_rustStrict()) throw err;
    _warnRustFallback(opName, err);
    var prevFallback = _forceFileFallback;
    _forceFileFallback = true;
    try {
      return fileFn();
    } finally {
      _forceFileFallback = prevFallback;
    }
  }
}

// ─────────────────────────────────────────────────────────────────
// Directory management
// ─────────────────────────────────────────────────────────────────

function ensureBlockDir() {
  if (_useRustChain()) return;
  if (!fs.existsSync(BLOCKS_DIR)) {
    fs.mkdirSync(BLOCKS_DIR, { recursive: true });
  }
}

function blockPath(epochNumber) {
  return path.join(BLOCKS_DIR, "block-" + String(epochNumber).padStart(8, "0") + ".bin");
}

function finalityPath(epochNumber) {
  return path.join(BLOCKS_DIR, "finality-" + String(epochNumber).padStart(8, "0") + ".bin");
}

var INDEX_PATH = path.join(BLOCKS_DIR, "index.json");

function _encodeBlockFileBuffer(block, payloadObj) {
  var header = block.serialize();
  var payloadJson = Buffer.from(JSON.stringify(payloadObj), "utf8");
  var lengthBuf = Buffer.alloc(4);
  lengthBuf.writeUInt32LE(payloadJson.length, 0);
  return Buffer.concat([header, lengthBuf, payloadJson]);
}

// ─────────────────────────────────────────────────────────────────
// Write operations (atomic: temp file → rename)
// ─────────────────────────────────────────────────────────────────

/**
 * Write a block to disk. Atomic via temp-file-then-rename.
 *
 * @param {Block} block — Block instance with header fields
 * @param {object} payload — { ledger_entries, rewards, compute_proofs, mining_proofs }
 */
function writeBlock(block, payload) {
  var fullBuf = _encodeBlockFileBuffer(block, payload);
  return _withRustFallback(
    "writeBlock",
    function () {
      rustChainIpc.writeBlock(block.epoch_number, fullBuf);
      return "ipc://btcpc-chain/block/" + block.epoch_number;
    },
    function () {
      ensureBlockDir();
      var filePath = blockPath(block.epoch_number);
      var tmpPath = filePath + ".tmp";
      fs.writeFileSync(tmpPath, fullBuf);
      fs.renameSync(tmpPath, filePath);
      // Update index
      updateIndex(block.epoch_number, block.computeHash());
      return filePath;
    }
  );
}

/**
 * Write a finality block to disk.
 *
 * @param {Block} block — Block instance (same epoch as a regular block)
 * @param {object} snapshot — Full state snapshot (accounts, SMT, rolling commitment)
 */
function writeFinality(block, snapshot) {
  var fullBuf = _encodeBlockFileBuffer(block, snapshot);
  return _withRustFallback(
    "writeFinality",
    function () {
      rustChainIpc.writeFinality(block.epoch_number, fullBuf);
      return "ipc://btcpc-chain/finality/" + block.epoch_number;
    },
    function () {
      ensureBlockDir();
      var filePath = finalityPath(block.epoch_number);
      var tmpPath = filePath + ".tmp";
      fs.writeFileSync(tmpPath, fullBuf);
      fs.renameSync(tmpPath, filePath);
      return filePath;
    }
  );
}

// ─────────────────────────────────────────────────────────────────
// Read operations
// ─────────────────────────────────────────────────────────────────

/**
 * Read a block from disk.
 * Returns { block, payload } or null if file does not exist.
 */
function readBlock(epochNumber) {
  return _withRustFallback(
    "readBlock",
    function () {
      var buf = rustChainIpc.readBlock(epochNumber);
      if (!buf) return null;
      return _parseBlockFile(buf);
    },
    function () {
      var filePath = blockPath(epochNumber);
      if (!fs.existsSync(filePath)) return null;
      var buf = fs.readFileSync(filePath);
      return _parseBlockFile(buf);
    }
  );
}

/**
 * Read only the block header from disk (first 180 bytes).
 * Returns a Block instance or null.
 */
function readBlockHeader(epochNumber) {
  return _withRustFallback(
    "readBlockHeader",
    function () {
      var buf = rustChainIpc.readBlock(epochNumber);
      if (!buf || buf.length < Block.HEADER_SIZE) return null;
      return Block.deserialize(buf.slice(0, Block.HEADER_SIZE));
    },
    function () {
      var filePath = blockPath(epochNumber);
      if (!fs.existsSync(filePath)) return null;
      var fd = fs.openSync(filePath, "r");
      var headerBuf = Buffer.alloc(Block.HEADER_SIZE);
      fs.readSync(fd, headerBuf, 0, Block.HEADER_SIZE, 0);
      fs.closeSync(fd);
      return Block.deserialize(headerBuf);
    }
  );
}

/**
 * Read a finality block from disk.
 * Returns { block, snapshot } or null.
 */
function readFinality(epochNumber) {
  return _withRustFallback(
    "readFinality",
    function () {
      var buf = rustChainIpc.readFinality(epochNumber);
      if (!buf) return null;
      var parsed = _parseBlockFile(buf);
      return parsed ? { block: parsed.block, snapshot: parsed.payload } : null;
    },
    function () {
      var filePath = finalityPath(epochNumber);
      if (!fs.existsSync(filePath)) return null;
      var buf = fs.readFileSync(filePath);
      var parsed = _parseBlockFile(buf);
      return parsed ? { block: parsed.block, snapshot: parsed.payload } : null;
    }
  );
}

/**
 * Parse a block file buffer into { block, payload }.
 */
function _parseBlockFile(buf) {
  if (buf.length < Block.HEADER_SIZE + 4) return null;

  var block = Block.deserialize(buf.slice(0, Block.HEADER_SIZE));
  var payloadLength = buf.readUInt32LE(Block.HEADER_SIZE);
  var payloadStart = Block.HEADER_SIZE + 4;
  var payloadEnd = payloadStart + payloadLength;

  if (buf.length < payloadEnd) return null;

  var payloadJson = buf.slice(payloadStart, payloadEnd).toString("utf8");
  var payload;
  try {
    payload = JSON.parse(payloadJson);
  } catch (e) {
    return null;
  }

  return { block: block, payload: payload };
}

// ─────────────────────────────────────────────────────────────────
// Query operations
// ─────────────────────────────────────────────────────────────────

/**
 * Check if a block file exists on disk.
 */
function hasBlock(epochNumber) {
  return _withRustFallback(
    "hasBlock",
    function () { return rustChainIpc.hasBlock(epochNumber); },
    function () { return fs.existsSync(blockPath(epochNumber)); }
  );
}

/**
 * Check if a finality block exists on disk.
 */
function hasFinality(epochNumber) {
  return _withRustFallback(
    "hasFinality",
    function () { return rustChainIpc.hasFinality(epochNumber); },
    function () { return fs.existsSync(finalityPath(epochNumber)); }
  );
}

/**
 * Get the highest epoch number stored on disk.
 * Returns -1 if no blocks exist.
 */
function getLatestBlockNumber() {
  return _withRustFallback(
    "getLatestBlockNumber",
    function () { return rustChainIpc.latestBlock(); },
    function () {
      var index = loadIndex();
      var epochs = Object.keys(index).map(Number);
      if (epochs.length === 0) return -1;
      return Math.max.apply(null, epochs);
    }
  );
}

/**
 * Get the latest finality block epoch number.
 * Returns -1 if no finality blocks exist.
 */
function getLatestFinalityNumber() {
  return _withRustFallback(
    "getLatestFinalityNumber",
    function () { return rustChainIpc.latestFinality(); },
    function () {
      ensureBlockDir();
      var files = fs.readdirSync(BLOCKS_DIR).filter(function (f) {
        return f.startsWith("finality-") && f.endsWith(".bin");
      });
      if (files.length === 0) return -1;
      var epochs = files.map(function (f) {
        return parseInt(f.replace("finality-", "").replace(".bin", ""), 10);
      });
      return Math.max.apply(null, epochs);
    }
  );
}

// ─────────────────────────────────────────────────────────────────
// Index management
// ─────────────────────────────────────────────────────────────────

var _indexCache = null;

/**
 * Load the index from disk. Returns { epochNumber: blockHash, ... }.
 */
function loadIndex() {
  if (_useRustChain()) return {};
  if (_indexCache) return _indexCache;

  if (fs.existsSync(INDEX_PATH)) {
    try {
      _indexCache = JSON.parse(fs.readFileSync(INDEX_PATH, "utf8"));
      return _indexCache;
    } catch (e) {
      // Corrupted index — rebuild
    }
  }

  _indexCache = rebuildIndex();
  return _indexCache;
}

/**
 * Save the index to disk.
 */
function saveIndex() {
  if (_useRustChain()) return;
  if (!_indexCache) return;
  ensureBlockDir();
  var tmpPath = INDEX_PATH + ".tmp";
  fs.writeFileSync(tmpPath, JSON.stringify(_indexCache, null, 2));
  fs.renameSync(tmpPath, INDEX_PATH);
}

/**
 * Update a single entry in the index and persist.
 */
function updateIndex(epochNumber, blockHash) {
  if (_useRustChain()) return;
  if (!_indexCache) loadIndex();
  _indexCache[String(epochNumber)] = blockHash;
  saveIndex();
}

/**
 * Rebuild the index by scanning all block files on disk.
 */
function rebuildIndex() {
  if (_useRustChain()) return {};
  ensureBlockDir();
  var index = {};

  var files = fs.readdirSync(BLOCKS_DIR).filter(function (f) {
    return f.startsWith("block-") && f.endsWith(".bin");
  });

  for (var i = 0; i < files.length; i++) {
    var epochStr = files[i].replace("block-", "").replace(".bin", "");
    var epochNum = parseInt(epochStr, 10);
    var header = readBlockHeader(epochNum);
    if (header) {
      index[String(epochNum)] = header.computeHash();
    }
  }

  _indexCache = index;
  saveIndex();
  return index;
}

// ─────────────────────────────────────────────────────────────────
// Deterministic hashing (for Merkle root computation)
// ─────────────────────────────────────────────────────────────────

/**
 * Compute a deterministic SHA-256 hash of a ledger entry.
 * Strips MongoDB fields (_id, __v, createdAt), uses sorted keys.
 */
function hashLedgerEntry(entry) {
  var canonical = {
    amount: entry.amount || 0,
    epoch: entry.epoch,
    from: entry.from || null,
    memo: entry.memo || null,
    signed_by: entry.signed_by || null,
    to: entry.to || null,
    token: entry.token || "BTCPC",
    type: entry.type
  };
  return crypto.createHash("sha256")
    .update(JSON.stringify(canonical))
    .digest("hex");
}

/**
 * Compute a deterministic SHA-256 hash of a compute proof.
 */
function hashComputeProof(proof) {
  var canonical = {
    model: proof.model,
    node_id: proof.node_id,
    prompt_hash: proof.prompt_hash,
    result_hash: proof.result_hash,
    tokens_generated: proof.tokens_generated,
    work_value: proof.work_value
  };
  return crypto.createHash("sha256")
    .update(JSON.stringify(canonical))
    .digest("hex");
}

// ─────────────────────────────────────────────────────────────────
// Pruning (Lucid Pruning)
// ─────────────────────────────────────────────────────────────────

/**
 * Remove block files before a finality epoch.
 * Keeps the finality block itself. Returns count of files removed.
 */
function pruneBeforeFinality(finalityEpoch) {
  return _withRustFallback(
    "pruneBeforeFinality",
    function () {
      return rustChainIpc.pruneBefore(finalityEpoch);
    },
    function () {
      ensureBlockDir();
      var removed = 0;
      var files = fs.readdirSync(BLOCKS_DIR).filter(function (f) {
        return f.startsWith("block-") && f.endsWith(".bin");
      });
      for (var i = 0; i < files.length; i++) {
        var epochStr = files[i].replace("block-", "").replace(".bin", "");
        var epochNum = parseInt(epochStr, 10);
        if (epochNum < finalityEpoch) {
          fs.unlinkSync(path.join(BLOCKS_DIR, files[i]));
          if (_indexCache) delete _indexCache[String(epochNum)];
          removed++;
        }
      }
      if (removed > 0) saveIndex();
      return removed;
    }
  );
}

/**
 * Iterate blocks from disk in order.
 * Calls callback(block, payload, epochNumber) for each block.
 */
function iterateBlocks(fromEpoch, toEpoch, callback) {
  for (var epoch = fromEpoch; epoch <= toEpoch; epoch++) {
    var data = readBlock(epoch);
    if (data) {
      callback(data.block, data.payload, epoch);
    }
  }
}

// ─────────────────────────────────────────────────────────────────
// Exports
// ─────────────────────────────────────────────────────────────────

/**
 * Poll for new blocks starting from fromEpoch. Calls callback(epochNumber, payload)
 * for each block found. Returns a stop() function.
 */
function watchBlocks(fromEpoch, callback) {
  var nextEpoch = fromEpoch;
  var pollMs = _useRustChain() ? 200 : 20;
  var timer = setInterval(function() {
    while (nextEpoch <= getLatestBlockNumber()) {
      var data = readBlock(nextEpoch);
      if (data) callback(nextEpoch, data.payload);
      nextEpoch++;
    }
  }, pollMs);
  return function stop() { clearInterval(timer); };
}

module.exports = {
  ensureBlockDir: ensureBlockDir,
  writeBlock: writeBlock,
  writeFinality: writeFinality,
  readBlock: readBlock,
  readBlockHeader: readBlockHeader,
  readFinality: readFinality,
  hasBlock: hasBlock,
  hasFinality: hasFinality,
  getLatestBlockNumber: getLatestBlockNumber,
  getLatestFinalityNumber: getLatestFinalityNumber,
  loadIndex: loadIndex,
  saveIndex: saveIndex,
  rebuildIndex: rebuildIndex,
  updateIndex: updateIndex,
  hashLedgerEntry: hashLedgerEntry,
  hashComputeProof: hashComputeProof,
  pruneBeforeFinality: pruneBeforeFinality,
  iterateBlocks: iterateBlocks,
  watchBlocks: watchBlocks,
  BLOCKS_DIR: BLOCKS_DIR
};
