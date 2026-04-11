"use strict";

/**
 * BTCPC Snapshot Replication — v2.14-alpha
 * Shin Devlin
 *
 * Pure data-layer module for snapshot save/load via BTCPC-FS with
 * N-way replication. No spawn, no service runtime — just snapshot
 * bytes in, CIDs out.
 *
 * In v2.14-alpha all snapshot METADATA (slug → history) is kept in
 * an in-memory Map. Disk-backed metadata (and on-chain SNAPSHOT_COMMIT
 * ledger entries) land in v2.14-beta.
 *
 * The actual snapshot BYTES are stored in BTCPC-FS (blobStore), which
 * IS disk-backed via the content-addressed blob layer from v2.11.
 *
 * Architecture — sibling-module pattern:
 *   This module does NOT modify serviceRegistry or serviceHostRunner.
 *   statefulServiceHostRunner composes this module for lifecycle hooks.
 *   statefulServiceRegistry composes this module for snapshot metadata.
 *
 * TODO (v2.14-beta): wire SNAPSHOT_COMMIT ledger entry into stateStore.
 * The dispatcher case should look like:
 *
 *   case "SNAPSHOT_COMMIT": {
 *     // entry.data: { slug, cid, replicas, epoch, size }
 *     // Record in stateStore.snapshotHistory[slug]
 *     // Pin CID to all replicas listed
 *     stateStore.recordSnapshotCommit(entry.data);
 *     break;
 *   }
 *
 * Usage (DI pattern):
 *   const replicator = createReplicator({ blobStore, hostSelector, replicationFactor });
 *   const { cid, replicas, created_at } = await replicator.saveSnapshot(slug, bytes);
 */

var defaultBlobStore = require("./blobStore");
var defaultHostSelector = require("./blobHostSelector");

/**
 * Create a replicator instance.
 *
 * @param {object} options
 * @param {object} options.blobStore         — injected CAS layer (default: real blobStore)
 * @param {object} options.hostSelector      — injected host-picker (default: real blobHostSelector)
 * @param {number} options.replicationFactor — default replication count (default: 3)
 *
 * The hostSelector interface used here is a simplified subset of blobHostSelector:
 *   hostSelector.selectHosts({ target_active, target_cold, stateStore }) → { active_hosts }
 *
 * In tests, inject a mock that returns a static list of host names.
 */
function createReplicator(options) {
  options = options || {};
  var blobStore = options.blobStore || defaultBlobStore;
  var hostSelector = options.hostSelector || defaultHostSelector;
  var defaultReplicationFactor = options.replicationFactor || 3;

  // slug → [ { cid, created_at, replicas, size } ]  (newest last for push/pop)
  // Exposed as-is for debugging; callers use the public API methods.
  var snapshots = new Map();

  // ───────────────────────────────────────────────────────────────
  // Internal helpers
  // ───────────────────────────────────────────────────────────────

  function _pickHosts(replicationFactor, stateStore) {
    // blobHostSelector.selectHosts wants active + cold separately.
    // For snapshot replication we flatten both lists and dedupe.
    try {
      var selection = hostSelector.selectHosts({
        target_active: replicationFactor,
        target_cold: 0,
        region_constraints: [],
        current_epoch: 0,
        stateStore: stateStore || { getActiveStorageHosts: function () { return []; } },
      });
      var combined = (selection.active_hosts || []).concat(selection.cold_hosts || []);
      var seen = {};
      var result = [];
      for (var i = 0; i < combined.length; i++) {
        if (!seen[combined[i]]) {
          seen[combined[i]] = true;
          result.push(combined[i]);
        }
      }
      return result.slice(0, replicationFactor);
    } catch (_) {
      return [];
    }
  }

  // ───────────────────────────────────────────────────────────────
  // Public API
  // ───────────────────────────────────────────────────────────────

  /**
   * Save a snapshot for the given service slug.
   *
   * @param {string} slug             — service slug, e.g. "shindevlin/voxel-wars"
   * @param {Buffer} snapshotBytes    — serialised state from the runtime
   * @param {object} [opts]
   * @param {number} [opts.replication_factor]  — overrides instance default
   * @param {object} [opts.stateStore]          — injected for host selection in tests
   * @returns {{ cid, replicas, created_at, size }}
   */
  function saveSnapshot(slug, snapshotBytes, opts) {
    opts = opts || {};
    if (!slug || typeof slug !== "string") {
      throw new Error("saveSnapshot: slug required");
    }
    if (!Buffer.isBuffer(snapshotBytes)) {
      throw new Error("saveSnapshot: snapshotBytes must be a Buffer");
    }

    var replicationFactor =
      typeof opts.replication_factor === "number"
        ? opts.replication_factor
        : defaultReplicationFactor;

    // Store bytes in BTCPC-FS — throws if empty or too large.
    var result = blobStore.putBlob(snapshotBytes);
    var cid = result.cid;
    var size = result.size || snapshotBytes.length;

    // Pick hosts to hold the replica (best-effort; may be empty if pool is thin)
    var replicas = _pickHosts(replicationFactor, opts.stateStore);

    var entry = {
      cid: cid,
      created_at: Date.now(),
      replicas: replicas,
      size: size,
    };

    var history = snapshots.get(slug) || [];
    history.push(entry);
    snapshots.set(slug, history);

    // TODO (v2.14-beta): emit SNAPSHOT_COMMIT ledger entry
    // stateStore.applyEntry({ type: "SNAPSHOT_COMMIT",
    //   data: { slug, cid, replicas, epoch, size } });

    return {
      cid: cid,
      replicas: replicas.slice(),
      created_at: entry.created_at,
      size: size,
    };
  }

  /**
   * Load the most recent snapshot for the given slug whose bytes are
   * still retrievable from BTCPC-FS.
   *
   * @param {string} slug
   * @returns {{ cid, bytes, created_at, replicas }} or null
   */
  function loadLatestSnapshot(slug) {
    var history = snapshots.get(slug);
    if (!history || history.length === 0) return null;

    // Walk newest → oldest until we find one we can actually read.
    for (var i = history.length - 1; i >= 0; i--) {
      var entry = history[i];
      try {
        if (!blobStore.hasBlob(entry.cid)) continue;
        var bytes = blobStore.getBlob(entry.cid);
        return {
          cid: entry.cid,
          bytes: bytes,
          created_at: entry.created_at,
          replicas: (entry.replicas || []).slice(),
        };
      } catch (_) {
        continue;
      }
    }
    return null;
  }

  /**
   * Load a specific snapshot by CID.
   *
   * @param {string} cid
   * @returns {Buffer} or null
   */
  function loadSnapshotByCid(cid) {
    try {
      if (!blobStore.hasBlob(cid)) return null;
      return blobStore.getBlob(cid);
    } catch (_) {
      return null;
    }
  }

  /**
   * List snapshots for a slug, sorted newest first.
   *
   * @param {string} slug
   * @param {object} [opts]
   * @param {number} [opts.limit]
   * @returns {Array<{ cid, created_at, replicas, size }>}
   */
  function listSnapshots(slug, opts) {
    opts = opts || {};
    var history = snapshots.get(slug);
    if (!history || history.length === 0) return [];

    // Copy and sort newest first (history is push-ordered = oldest first)
    var sorted = history.slice().reverse();

    if (typeof opts.limit === "number" && opts.limit > 0) {
      sorted = sorted.slice(0, opts.limit);
    }

    return sorted.map(function (e) {
      return {
        cid: e.cid,
        created_at: e.created_at,
        replicas: (e.replicas || []).slice(),
        size: e.size,
      };
    });
  }

  /**
   * Prune old snapshots for a slug, keeping the most recent `keep` entries.
   * Returns the number of entries deleted.
   *
   * @param {string} slug
   * @param {number} keep — how many to retain (must be >= 1)
   * @returns {number} deleted count
   */
  function pruneOldSnapshots(slug, keep) {
    if (typeof keep !== "number" || keep < 1) keep = 1;
    var history = snapshots.get(slug);
    if (!history || history.length <= keep) return 0;

    var deleted = history.length - keep;
    // history is oldest-first; slice off the front (oldest)
    snapshots.set(slug, history.slice(deleted));
    return deleted;
  }

  /**
   * Verify that a given host holds a replica of the given CID.
   * v2.14-alpha: placeholder. Real implementation does an HTTP GET
   * against the host's storage daemon endpoint.
   *
   * @param {string} host
   * @param {string} cid
   * @returns {boolean}
   */
  function verifyReplica(host, cid) {
    // TODO (v2.14-beta): HTTP GET host/api/blobs/:cid/status
    // For now, return true if we have the blob locally (best-effort).
    try {
      return blobStore.hasBlob(cid);
    } catch (_) {
      return false;
    }
  }

  /**
   * Reset all in-memory snapshot metadata. For tests only.
   */
  function resetForTests() {
    snapshots.clear();
  }

  return {
    saveSnapshot: saveSnapshot,
    loadLatestSnapshot: loadLatestSnapshot,
    loadSnapshotByCid: loadSnapshotByCid,
    listSnapshots: listSnapshots,
    pruneOldSnapshots: pruneOldSnapshots,
    verifyReplica: verifyReplica,
    resetForTests: resetForTests,
    // Exposed for debugging in tests
    _snapshots: snapshots,
  };
}

// Default singleton for use in production code paths
var _defaultReplicator = null;

function getDefaultReplicator() {
  if (!_defaultReplicator) {
    _defaultReplicator = createReplicator({});
  }
  return _defaultReplicator;
}

module.exports = {
  createReplicator: createReplicator,
  getDefaultReplicator: getDefaultReplicator,
};
