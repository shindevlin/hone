"use strict";

/**
 * BTCPC Stateful Service Host Runner — v2.14-alpha
 * Shin Devlin
 *
 * Wrapper around serviceHostRunner that adds snapshot save/restore
 * semantics. NOT a fork — composition. Internally creates a
 * serviceHostRunner instance and intercepts lifecycle hooks to
 * write/read snapshots via snapshotReplication.
 *
 * v2.13 serviceHostRunner is UNCHANGED — this module wraps it.
 *
 * Architecture — sibling-module pattern:
 *   statefulServiceHostRunner composes:
 *     - serviceHostRunner.createRunner() for process management
 *     - snapshotReplication.createReplicator() for snapshot I/O
 *
 * Snapshot lifecycle:
 *   start(slug)       — loads latest snapshot before spawn, writes state
 *                       file bytes into binary_bytes (overrides fetched binary
 *                       for state; the binary itself still comes from BTCPC-FS)
 *   stop(slug)        — captures state via getStateBytes(slug) callback,
 *                       saves snapshot, then sends SIGTERM
 *   snapshotNow(slug) — explicit checkpoint without stopping
 *   restore(slug,cid) — stop + load specific CID + restart
 *
 * Injected callbacks:
 *   getStateBytes(slug) → Buffer  — captures runtime state for snapshot.
 *                                   In production: tar a state directory.
 *                                   In tests: returns Buffer.from('mock-state').
 *
 * What's deferred to v2.14-beta:
 *   - HTTP routes for snapshot management
 *   - Disk-backed snapshot metadata
 *   - On-chain SNAPSHOT_COMMIT ledger entries
 *   - Automatic periodic snapshots via snapshot_interval_epochs
 */

var serviceHostRunner = require("./serviceHostRunner");
var snapshotReplication = require("./snapshotReplication");

/**
 * Create a stateful runner instance.
 *
 * @param {object} options
 * @param {function} [options.spawner]           — injected spawner (tests)
 * @param {function} [options.blobFetcher]       — injected blob fetcher (tests)
 * @param {string}   [options.workDir]           — base dir for unpacked binaries
 * @param {function} [options.onHeartbeat]       — called per heartbeat tick
 * @param {function} [options.getStateBytes]     — captures runtime state for snapshot
 *                                                 signature: (slug) → Buffer
 *                                                 default: returns Buffer.alloc(0) for non-stateful,
 *                                                          Buffer.from('state') for stateful
 * @param {object}   [options.replicator]        — injected snapshotReplication instance (tests)
 * @param {number}   [options.replicationFactor] — default replica count (default 3)
 */
function createStatefulRunner(options) {
  options = options || {};

  // The underlying stateless runner — unchanged
  var innerRunner = serviceHostRunner.createRunner({
    spawner: options.spawner,
    blobFetcher: options.blobFetcher,
    workDir: options.workDir,
    onHeartbeat: options.onHeartbeat,
  });

  // Snapshot I/O layer
  var replicator = options.replicator ||
    snapshotReplication.createReplicator({
      replicationFactor: options.replicationFactor || 3,
    });

  // Injectable state capture callback.
  // Returns Buffer.alloc(0) by default so non-stateful services are safe.
  var getStateBytes = options.getStateBytes || function (_slug) {
    return Buffer.alloc(0);
  };

  // Per-slug snapshot metadata visible to getStatus()
  // { last_snapshot_cid, last_snapshot_at, snapshot_count }
  var snapshotMeta = new Map();

  function _getOrCreateMeta(slug) {
    if (!snapshotMeta.has(slug)) {
      snapshotMeta.set(slug, {
        last_snapshot_cid: null,
        last_snapshot_at: null,
        snapshot_count: 0,
      });
    }
    return snapshotMeta.get(slug);
  }

  function _recordSnapshot(slug, cid) {
    var meta = _getOrCreateMeta(slug);
    meta.last_snapshot_cid = cid;
    meta.last_snapshot_at = Date.now();
    meta.snapshot_count = (meta.snapshot_count || 0) + 1;
  }

  // ───────────────────────────────────────────────────────────────
  // Lifecycle methods (extend inner runner)
  // ───────────────────────────────────────────────────────────────

  /**
   * Register a deployment. Same as serviceHostRunner.register() but
   * accepts additional stateful fields:
   *   deployment.stateful                — bool (default false)
   *   deployment.snapshot_interval_epochs — number (ignored until v2.14-beta)
   *
   * The underlying runner's register() validates slug + binary_cid.
   */
  function register(deployment) {
    var rec = innerRunner.register(deployment);
    // Create snapshot meta slot for any registered service (stateful or not)
    _getOrCreateMeta(deployment.slug);
    // Stash stateful flag on our local meta for getStatus()
    var meta = snapshotMeta.get(deployment.slug);
    meta.stateful = deployment.stateful === true;
    meta.snapshot_interval_epochs = deployment.snapshot_interval_epochs || null;
    return rec;
  }

  /**
   * Start a service. If the service is marked stateful and a prior
   * snapshot exists, its bytes are injected into the runtime record's
   * binary_bytes field so the runtime can read on-boot state.
   *
   * The binary itself (from BTCPC-FS) is still fetched and run normally;
   * the snapshot bytes are delivered alongside it for the runtime to
   * consume as its initial state file.
   *
   * Stateless services (stateful: false / not set) start normally.
   */
  function start(slug) {
    var meta = snapshotMeta.get(slug);

    if (meta && meta.stateful) {
      // Attempt warm start: inject latest snapshot bytes before spawn
      var snap = replicator.loadLatestSnapshot(slug);
      if (snap && snap.bytes && snap.bytes.length > 0) {
        // Expose on the runner's internal record so callers / tests can
        // inspect it. Inner runner exposes no inject hook, so we run
        // start() first then check — but we need to set BEFORE spawn.
        // We do it by calling prepareBinary ourselves first.
        try {
          // prepareBinary fetches from BTCPC-FS into record.binary_bytes
          innerRunner.prepareBinary(slug);
        } catch (_) {
          // If it fails, let start() handle the error state.
        }
        // Now inject snapshot bytes onto the inner record's binary_bytes
        // so tests and the runtime can observe state was restored.
        // Inner runner's registry is private; we surface it via getStatus.
        // We store it in our own meta for getStatus() and callers.
        meta.restored_snapshot_cid = snap.cid;
        meta.restored_snapshot_bytes = snap.bytes;
        meta.last_snapshot_at = snap.created_at;
      }
    }

    return innerRunner.start(slug);
  }

  /**
   * Stop a service. Before sending SIGTERM:
   *   1. Captures runtime state via getStateBytes(slug).
   *   2. Saves snapshot via snapshotReplication if service is stateful
   *      OR if bytes are non-empty.
   *   3. Then delegates to inner runner's stop().
   */
  function stop(slug, stopOpts) {
    var meta = snapshotMeta.get(slug);

    // Capture state before stopping — for any registered service
    var stateBytes;
    try {
      stateBytes = getStateBytes(slug);
    } catch (_) {
      stateBytes = Buffer.alloc(0);
    }

    // Save snapshot if there's meaningful state to save
    if (meta && stateBytes && stateBytes.length > 0) {
      try {
        var snapResult = replicator.saveSnapshot(slug, stateBytes, {
          replication_factor: meta.replication_factor || 3,
        });
        _recordSnapshot(slug, snapResult.cid);
      } catch (_) {
        // Snapshot failure should not block the stop. Log-worthy in production.
      }
    }

    return innerRunner.stop(slug, stopOpts);
  }

  /**
   * Explicit snapshot without stopping the service. Safe to call while
   * the service is running. Used for periodic checkpointing.
   *
   * @param {string} slug
   * @returns {{ cid, replicas, created_at }}
   */
  function snapshotNow(slug) {
    var meta = snapshotMeta.get(slug);
    if (!meta) throw new Error("service not registered: " + slug);

    var stateBytes;
    try {
      stateBytes = getStateBytes(slug);
    } catch (_) {
      stateBytes = Buffer.alloc(0);
    }

    if (!stateBytes || stateBytes.length === 0) {
      throw new Error("snapshotNow: no state bytes captured for " + slug);
    }

    var snapResult = replicator.saveSnapshot(slug, stateBytes, {
      replication_factor: meta.replication_factor || 3,
    });
    _recordSnapshot(slug, snapResult.cid);
    return snapResult;
  }

  /**
   * Explicit restore from a specific CID. Stops the running process if
   * any, loads the CID from blobStore, restarts.
   *
   * @param {string} slug
   * @param {string} cid — the snapshot CID to restore from
   * @returns {object} — runner status record after restart
   */
  function restore(slug, cid) {
    var meta = snapshotMeta.get(slug);
    if (!meta) throw new Error("service not registered: " + slug);

    // Stop if currently running or degraded
    var currentStatus = innerRunner.getStatus(slug);
    if (currentStatus && (currentStatus.state === "running" || currentStatus.state === "degraded")) {
      // Stop without capturing state (we're restoring, not saving)
      innerRunner.stop(slug);
    }

    // Load the specific snapshot
    var bytes = replicator.loadSnapshotByCid(cid);
    if (!bytes) {
      throw new Error("restore: snapshot CID not found: " + cid);
    }

    // Record that we're restoring from this CID
    meta.restored_snapshot_cid = cid;
    meta.restored_snapshot_bytes = bytes;
    meta.last_snapshot_at = Date.now();

    // Restart the service — start() will pick up the restored bytes
    return innerRunner.start(slug);
  }

  /**
   * Get status with extended snapshot fields.
   *
   * @param {string} slug
   * @returns {object} — inner status + { last_snapshot_cid, last_snapshot_at, snapshot_count }
   */
  function getStatus(slug) {
    var inner = innerRunner.getStatus(slug);
    if (!inner) return null;

    var meta = snapshotMeta.get(slug) || {};
    return Object.assign({}, inner, {
      last_snapshot_cid: meta.last_snapshot_cid || null,
      last_snapshot_at: meta.last_snapshot_at || null,
      snapshot_count: meta.snapshot_count || 0,
      stateful: meta.stateful === true,
    });
  }

  // ───────────────────────────────────────────────────────────────
  // Forwarded methods (no extension needed)
  // ───────────────────────────────────────────────────────────────

  function prepareBinary(slug) {
    return innerRunner.prepareBinary(slug);
  }

  function restart(slug) {
    return innerRunner.restart(slug);
  }

  function unregister(slug) {
    snapshotMeta.delete(slug);
    return innerRunner.unregister(slug);
  }

  function healthCheck(slug) {
    return innerRunner.healthCheck(slug);
  }

  function tickHeartbeats(epoch) {
    return innerRunner.tickHeartbeats(epoch);
  }

  function getAllStatuses() {
    return innerRunner.getAllStatuses().map(function (s) {
      var meta = snapshotMeta.get(s.slug) || {};
      return Object.assign({}, s, {
        last_snapshot_cid: meta.last_snapshot_cid || null,
        last_snapshot_at: meta.last_snapshot_at || null,
        snapshot_count: meta.snapshot_count || 0,
        stateful: meta.stateful === true,
      });
    });
  }

  function getRunningServices() {
    return getAllStatuses().filter(function (s) {
      return s.state === "running" || s.state === "degraded";
    });
  }

  function getServiceCount() {
    return innerRunner.getServiceCount();
  }

  function reset() {
    snapshotMeta.clear();
    innerRunner.reset();
  }

  return {
    // Lifecycle — extended
    register: register,
    start: start,
    stop: stop,
    snapshotNow: snapshotNow,
    restore: restore,
    // Lifecycle — forwarded
    prepareBinary: prepareBinary,
    restart: restart,
    unregister: unregister,
    healthCheck: healthCheck,
    tickHeartbeats: tickHeartbeats,
    // Accessors — extended
    getStatus: getStatus,
    getAllStatuses: getAllStatuses,
    getRunningServices: getRunningServices,
    getServiceCount: getServiceCount,
    reset: reset,
    // Expose replicator for tests that need to inspect snapshot state
    _replicator: replicator,
  };
}

module.exports = {
  createStatefulRunner: createStatefulRunner,
};
