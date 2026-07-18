"use strict";

/**
 * HONE Stateful Service Registry — v2.14-alpha
 * Shin Devlin
 *
 * Sibling to serviceRegistry. Adds stateful deployment metadata on top
 * of the v2.13-alpha registry without modifying it. serviceRegistry.js
 * is UNCHANGED — this module wraps it.
 *
 * Additional fields on stateful deployment records:
 *   stateful: true
 *   snapshot_interval_epochs — how often to auto-snapshot (deferred to v2.14-beta)
 *   replication_factor       — snapshot replica count (default 3)
 *   last_snapshot_cid        — CID of the most recent committed snapshot
 *   last_snapshot_epoch      — epoch at which the last snapshot was committed
 *
 * Architecture — sibling-module pattern:
 *   statefulServiceRegistry wraps serviceRegistry (v2.13-alpha).
 *   forwardingDeploy() calls serviceRegistry.deploy() and layers
 *   stateful metadata on top in a separate in-memory Map.
 *
 * What's deferred to v2.14-beta:
 *   - Chain-integrated SNAPSHOT_COMMIT dispatcher case
 *   - Disk-backed stateful metadata persistence
 *   - HTTP routes for stateful deploy / snapshot management
 */

var serviceRegistry = require("./serviceRegistry");

// slug → stateful metadata overlay
// { stateful, snapshot_interval_epochs, replication_factor,
//   last_snapshot_cid, last_snapshot_epoch }
var statefulMeta = new Map();

// ─────────────────────────────────────────────────────────────────
// Core API
// ─────────────────────────────────────────────────────────────────

/**
 * Deploy (or update) a stateful service. Calls serviceRegistry.deploy()
 * so all v2.13 validation rules apply, then overlays stateful metadata.
 *
 * @param {string} deployer
 * @param {string} slug
 * @param {object} runtimeSpec
 * @param {object} [options]
 * @param {boolean} [options.stateful]                  — mark as stateful
 * @param {number}  [options.snapshot_interval_epochs]  — auto-snapshot cadence
 * @param {number}  [options.replication_factor]        — snapshot replicas (default 3)
 * @returns {object} — merged registry record + stateful metadata
 */
function forwardingDeploy(deployer, slug, runtimeSpec, options) {
  options = options || {};

  // v2.13 deploy — validates everything; may throw
  var record = serviceRegistry.deploy(deployer, slug, runtimeSpec, options);

  // Layer stateful metadata
  var existing = statefulMeta.get(slug) || {};
  var meta = {
    stateful: options.stateful === true,
    snapshot_interval_epochs: options.snapshot_interval_epochs ||
      existing.snapshot_interval_epochs || null,
    replication_factor: typeof options.replication_factor === "number"
      ? options.replication_factor
      : (existing.replication_factor || 3),
    last_snapshot_cid: existing.last_snapshot_cid || null,
    last_snapshot_epoch: existing.last_snapshot_epoch || null,
  };
  statefulMeta.set(slug, meta);

  return _merge(record, meta);
}

/**
 * Record a snapshot commit for a deployed service. Updates the
 * stateful metadata overlay; the actual ledger entry is written by
 * snapshotReplication (v2.14-beta wires the chain integration).
 *
 * @param {string}   slug
 * @param {string}   cid
 * @param {number}   epoch
 * @param {string[]} replicas
 */
function recordSnapshotCommit(slug, cid, epoch, replicas) {
  if (!slug) throw new Error("recordSnapshotCommit: slug required");
  if (!cid) throw new Error("recordSnapshotCommit: cid required");

  // Service must be known to either registry to accept a commit
  var svcRecord = serviceRegistry.getService(slug);
  var ownMeta = statefulMeta.get(slug);
  if (!svcRecord && !ownMeta) {
    throw new Error("recordSnapshotCommit: unknown service: " + slug);
  }

  var meta = ownMeta || {};
  meta.last_snapshot_cid = cid;
  meta.last_snapshot_epoch = epoch || 0;
  meta.last_snapshot_replicas = Array.isArray(replicas) ? replicas.slice() : [];
  statefulMeta.set(slug, meta);

  return {
    slug: slug,
    cid: cid,
    epoch: meta.last_snapshot_epoch,
    replicas: meta.last_snapshot_replicas,
  };
}

/**
 * List all services with stateful: true, optionally filtered.
 *
 * @param {object} [filter]
 * @param {string} [filter.deployer]
 * @param {string} [filter.status]
 * @returns {Array<object>} — merged registry + stateful records
 */
function getStatefulServices(filter) {
  filter = filter || {};
  var results = [];

  for (var entry of statefulMeta) {
    var slug = entry[0];
    var meta = entry[1];
    if (!meta.stateful) continue;

    // Get the underlying registry record (may be null if not in serviceRegistry)
    var record = serviceRegistry.getService(slug);
    if (!record) continue; // orphaned meta — skip

    // Apply filters
    if (filter.deployer && record.deployer !== filter.deployer) continue;
    if (filter.status && record.status !== filter.status) continue;

    results.push(_merge(record, meta));
  }
  return results;
}

/**
 * Get the stateful metadata overlay for a slug, or null.
 *
 * @param {string} slug
 * @returns {object|null}
 */
function getStatefulMeta(slug) {
  return statefulMeta.get(slug) || null;
}

/**
 * Get a single stateful service record (registry record + meta overlay).
 * Returns null if not found or not stateful.
 *
 * @param {string} slug
 * @returns {object|null}
 */
function getStatefulService(slug) {
  var meta = statefulMeta.get(slug);
  if (!meta || !meta.stateful) return null;
  var record = serviceRegistry.getService(slug);
  if (!record) return null;
  return _merge(record, meta);
}

// ─────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────

function _merge(record, meta) {
  return Object.assign({}, record, {
    stateful: meta.stateful || false,
    snapshot_interval_epochs: meta.snapshot_interval_epochs || null,
    replication_factor: meta.replication_factor || 3,
    last_snapshot_cid: meta.last_snapshot_cid || null,
    last_snapshot_epoch: meta.last_snapshot_epoch || null,
  });
}

/**
 * Reset all in-memory stateful metadata. For tests only.
 * Does NOT reset serviceRegistry — call serviceRegistry.resetForTests() separately.
 */
function resetForTests() {
  statefulMeta.clear();
}

module.exports = {
  // Core mutators
  forwardingDeploy: forwardingDeploy,
  recordSnapshotCommit: recordSnapshotCommit,
  // Readers
  getStatefulServices: getStatefulServices,
  getStatefulMeta: getStatefulMeta,
  getStatefulService: getStatefulService,
  // Helpers
  resetForTests: resetForTests,
};
