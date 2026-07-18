"use strict";

/**
 * HONE State Backend — v3.0
 * Shin Devlin
 *
 * Pluggable storage backend for stateStore. Abstracts the difference
 * between an in-memory Map (fast, ephemeral — for tests and development)
 * and LevelDB (persistent, scalable — for production nodes).
 *
 * Selection via environment variable:
 *   HONE_STATE_BACKEND=memory  (default) — plain Maps, zero I/O
 *   HONE_STATE_BACKEND=leveldb           — LevelDB at HONE_DATA_DIR/statedb/
 *
 * Both backends implement the same synchronous-like interface. The
 * MemoryBackend is synchronous. The LevelDBBackend wraps async LevelDB
 * calls — callers that need persistence should await them.
 *
 * API surface (both backends implement):
 *   get(prefix, key)             → value or null  (sync for memory, async for leveldb)
 *   set(prefix, key, value)      → void           (sync for memory, async for leveldb)
 *   delete(prefix, key)          → void           (sync for memory, async for leveldb)
 *   has(prefix, key)             → bool           (sync for memory, async for leveldb)
 *   iterate(prefix, callback)    → void           (sync for memory, async for leveldb)
 *   clear(prefix)                → void           (sync for memory, async for leveldb)
 *   clearAll()                   → void           (sync for memory, async for leveldb)
 *   close()                      → void/Promise   (no-op for memory, closes DB for leveldb)
 *   type                         → 'memory' | 'leveldb'
 *
 * IMPORTANT: stateStore.js itself is 100% synchronous and uses the
 * MemoryBackend by default. The LevelDB backend is wired in for
 * production use when HONE_STATE_BACKEND=leveldb. The architecture
 * is designed so stateStore can adopt async LevelDB calls in a future
 * pass without changing any other module's interface.
 */

var path = require("path");

// ─────────────────────────────────────────────────────────────────
// MemoryBackend — plain Maps, current stateStore.js behavior
// ─────────────────────────────────────────────────────────────────

function MemoryBackend() {
  this.maps = {};
  this.type = "memory";
}

MemoryBackend.prototype._map = function(prefix) {
  if (!this.maps[prefix]) this.maps[prefix] = new Map();
  return this.maps[prefix];
};

MemoryBackend.prototype.get = function(prefix, key) {
  var m = this.maps[prefix];
  if (!m) return null;
  var v = m.get(key);
  return v === undefined ? null : v;
};

MemoryBackend.prototype.set = function(prefix, key, value) {
  this._map(prefix).set(key, value);
};

MemoryBackend.prototype.delete = function(prefix, key) {
  var m = this.maps[prefix];
  if (m) m.delete(key);
};

MemoryBackend.prototype.has = function(prefix, key) {
  var m = this.maps[prefix];
  if (!m) return false;
  return m.has(key);
};

/**
 * Iterate all entries in a prefix. callback(key, value) called for each.
 */
MemoryBackend.prototype.iterate = function(prefix, callback) {
  var m = this.maps[prefix];
  if (!m) return;
  for (var entry of m) {
    callback(entry[0], entry[1]);
  }
};

MemoryBackend.prototype.clear = function(prefix) {
  if (this.maps[prefix]) this.maps[prefix].clear();
};

MemoryBackend.prototype.clearAll = function() {
  this.maps = {};
};

MemoryBackend.prototype.close = function() {
  // no-op for in-memory backend
};

// ─────────────────────────────────────────────────────────────────
// LevelDBBackend — persistent, disk-backed
// ─────────────────────────────────────────────────────────────────

function LevelDBBackend(dataDir) {
  var dbPath = path.join(dataDir, "statedb");
  var Level;
  try {
    Level = require("level").Level;
  } catch (e) {
    throw new Error(
      "[stateBackend] LevelDB backend requested but 'level' package is not installed. " +
      "Run: npm install level"
    );
  }
  this.db = new Level(dbPath, { valueEncoding: "json" });
  this.type = "leveldb";
  this._dbPath = dbPath;
}

function _levelKey(prefix, key) {
  return prefix + ":" + key;
}

/**
 * Get a value. Returns null if not found.
 * Returns a Promise<value|null>.
 */
LevelDBBackend.prototype.get = async function(prefix, key) {
  try {
    var val = await this.db.get(_levelKey(prefix, key));
    // Level v10 returns undefined for missing keys (no throw)
    return val === undefined ? null : val;
  } catch (e) {
    if (e.code === "LEVEL_NOT_FOUND" || e.notFound) return null;
    throw e;
  }
};

/**
 * Set a value. Returns a Promise.
 */
LevelDBBackend.prototype.set = async function(prefix, key, value) {
  await this.db.put(_levelKey(prefix, key), value);
};

/**
 * Delete a key. Returns a Promise.
 */
LevelDBBackend.prototype.delete = async function(prefix, key) {
  try {
    await this.db.del(_levelKey(prefix, key));
  } catch (e) {
    if (e.code !== "LEVEL_NOT_FOUND" && !e.notFound) throw e;
  }
};

/**
 * Check existence. Returns Promise<bool>.
 */
LevelDBBackend.prototype.has = async function(prefix, key) {
  try {
    var val = await this.db.get(_levelKey(prefix, key));
    // Level v10 returns undefined for missing keys
    return val !== undefined;
  } catch (e) {
    if (e.code === "LEVEL_NOT_FOUND" || e.notFound) return false;
    throw e;
  }
};

/**
 * Iterate all entries for a prefix. callback(key, value) called for each.
 * key here is the bare key (without prefix:).
 * Returns a Promise that resolves when iteration is complete.
 */
LevelDBBackend.prototype.iterate = async function(prefix, callback) {
  var dbPrefix = prefix + ":";
  for await (var [rawKey, value] of this.db.iterator({
    gte: dbPrefix,
    lte: dbPrefix + "\xff",
  })) {
    var bareKey = rawKey.slice(dbPrefix.length);
    await callback(bareKey, value);
  }
};

/**
 * Delete all keys under a prefix. Returns a Promise.
 */
LevelDBBackend.prototype.clear = async function(prefix) {
  var dbPrefix = prefix + ":";
  var keysToDelete = [];
  for await (var [rawKey] of this.db.iterator({
    gte: dbPrefix,
    lte: dbPrefix + "\xff",
    keys: true,
    values: false,
  })) {
    keysToDelete.push(rawKey);
  }
  if (keysToDelete.length > 0) {
    var batch = this.db.batch();
    for (var k of keysToDelete) batch.del(k);
    await batch.write();
  }
};

/**
 * Wipe the entire database. Returns a Promise.
 */
LevelDBBackend.prototype.clearAll = async function() {
  await this.db.clear();
};

/**
 * Close the LevelDB instance gracefully. Returns a Promise.
 */
LevelDBBackend.prototype.close = async function() {
  await this.db.close();
};

// ─────────────────────────────────────────────────────────────────
// Factory
// ─────────────────────────────────────────────────────────────────

/**
 * Create a backend instance.
 *
 * @param {string} type    'memory' or 'leveldb'
 * @param {string} dataDir directory for LevelDB files (ignored for memory)
 * @returns {MemoryBackend|LevelDBBackend}
 */
function createBackend(type, dataDir) {
  if (type === "leveldb") {
    if (!dataDir) {
      throw new Error("[stateBackend] dataDir is required for LevelDB backend");
    }
    return new LevelDBBackend(dataDir);
  }
  return new MemoryBackend();
}

module.exports = {
  createBackend: createBackend,
  MemoryBackend: MemoryBackend,
  LevelDBBackend: LevelDBBackend,
};
