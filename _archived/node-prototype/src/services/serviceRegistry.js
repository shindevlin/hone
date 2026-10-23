"use strict";

/**
 * HONE Service Registry — v2.13-alpha
 * Shin Devlin
 *
 * In-memory registry for stateless compute hosting. A "service" is a
 * deployable workload (HTTP API, static site, game server, etc.) that
 * runs on host nodes — same pay-for-delivery economics as storage.
 *
 * This is the standalone primitive. The chain-integrated dispatcher
 * version (SERVICE_DEPLOY ledger entries) lands in a follow-up that
 * wires this into stateStore.applyEntry. For now this module
 * functions as a self-contained reference implementation that can be
 * exercised by routes, tests, and the host-side service runner.
 *
 * Service slug format: "<deployer>/<service-name>"
 *   - deployer must be a valid HONE account name
 *   - service-name matches /^[a-z0-9][a-z0-9-]{0,62}$/
 *   - same rules as commerce product slugs (v2.10)
 *
 * Runtime spec:
 *   {
 *     type: "http" | "tcp" | "wasm" | "static",
 *     binary_cid: 64-char hex (HONE-FS reference to executable/build),
 *     cpu: number (cores requested),
 *     ram_mb: number (RAM requested),
 *     port: number | null (TCP/HTTP port, null for static/wasm),
 *     entrypoint: string (e.g., "./server" or "index.html"),
 *     env: object | null (environment variables),
 *   }
 *
 * Lifecycle:
 *   1. Deployer publishes service via deploy() → registered in `services`
 *   2. Host nodes pick it up, start the runtime, send heartbeats
 *   3. Users start sessions against active hosts via startSession()
 *   4. Session ends via endSession() (user or host can end)
 *   5. Payouts settle via blobPayoutsTwoTier-style logic (v2.13.x follow-up)
 *
 * Same pay-for-delivery rule as storage: heartbeats prove uptime,
 * sessions prove usage, no slashing for absence — see
 * feedback_storage_no_slash.md.
 */

// service slug → service deployment record
var services = new Map();

// "<slug>|<host>" → host's heartbeat record for this service
var heartbeats = new Map();

// session_id → active session record
var sessions = new Map();

// Requests served per host per epoch: Map<epoch, Map<host, count>>
var requestsByEpoch = new Map();
var _currentRequestEpoch = -1;

var SLUG_PATTERN = /^[a-z0-9][a-z0-9-]{0,62}$/;
var CID_PATTERN = /^[a-f0-9]{64}$/;

function _round(n) {
  return parseFloat(Number(n).toFixed(10));
}

/**
 * Validate a service slug. Must be "<deployer>/<name>".
 * Returns { ok: bool, deployer, name, error }
 */
function parseSlug(slug) {
  if (typeof slug !== "string") return { ok: false, error: "slug must be a string" };
  var parts = slug.split("/");
  if (parts.length !== 2) return { ok: false, error: "slug must be <deployer>/<name>" };
  if (!SLUG_PATTERN.test(parts[0])) return { ok: false, error: "invalid deployer name" };
  if (!SLUG_PATTERN.test(parts[1])) return { ok: false, error: "invalid service name" };
  return { ok: true, deployer: parts[0], name: parts[1] };
}

/**
 * Deploy or update a service. The deployer must match the slug's prefix
 * (i.e., shindevlin can deploy "shindevlin/voxel-wars" but not
 * "alice/foo"). Updates to an existing deployment can change the binary
 * CID, runtime spec, price — they cannot change the deployer.
 */
function deploy(deployer, slug, runtimeSpec, options) {
  options = options || {};
  if (!deployer) throw new Error("deployer required");

  var parsed = parseSlug(slug);
  if (!parsed.ok) throw new Error("invalid slug: " + parsed.error);
  if (parsed.deployer !== deployer) {
    throw new Error("slug deployer prefix must match the deploying account");
  }
  if (!runtimeSpec || typeof runtimeSpec !== "object") {
    throw new Error("runtime spec required");
  }
  if (!runtimeSpec.binary_cid || !CID_PATTERN.test(runtimeSpec.binary_cid)) {
    throw new Error("runtime.binary_cid must be 64-char hex sha256");
  }
  var validTypes = ["http", "tcp", "wasm", "static"];
  if (validTypes.indexOf(runtimeSpec.type) < 0) {
    throw new Error("runtime.type must be one of: " + validTypes.join(", "));
  }

  var existing = services.get(slug);
  var record;
  if (existing) {
    if (existing.deployer !== deployer) {
      throw new Error("only the original deployer can update this service");
    }
    record = existing;
    record.binary_cid = runtimeSpec.binary_cid;
    record.runtime = runtimeSpec;
    record.last_updated_epoch = options.epoch || 0;
  } else {
    record = {
      slug: slug,
      deployer: deployer,
      binary_cid: runtimeSpec.binary_cid,
      runtime: runtimeSpec,
      min_replicas: options.min_replicas || 1,
      price_per_hour: _round(options.price_per_hour || 0),
      price_token: options.price_token || "HONE",
      active_hosts: [],
      total_sessions: 0,
      deployed_epoch: options.epoch || 0,
      last_updated_epoch: options.epoch || 0,
      status: "active",
    };
  }

  if (options.min_replicas !== undefined) record.min_replicas = options.min_replicas;
  if (options.price_per_hour !== undefined) record.price_per_hour = _round(options.price_per_hour);
  if (options.price_token) record.price_token = options.price_token;

  services.set(slug, record);
  return record;
}

/**
 * Retire a service. Only the deployer can retire. Marks status=retired
 * and removes from active selection. Existing sessions can finish; no
 * new sessions can start.
 */
function retire(deployer, slug, epoch) {
  var record = services.get(slug);
  if (!record) throw new Error("service not found");
  if (record.deployer !== deployer) {
    throw new Error("only the deployer can retire this service");
  }
  if (record.status === "retired") return record;
  record.status = "retired";
  record.retired_epoch = epoch || 0;
  services.set(slug, record);
  return record;
}

/**
 * Record a heartbeat from a host serving a deployed service.
 * Updates the host's heartbeat record and adds the host to the
 * service's active_hosts list if not already present.
 */
function heartbeat(host, slug, options) {
  options = options || {};
  if (!host) throw new Error("host required");
  var svc = services.get(slug);
  if (!svc) throw new Error("service not found");
  if (svc.status !== "active") throw new Error("service is not active");

  var key = slug + "|" + host;
  var record = heartbeats.get(key) || {
    slug: slug,
    host: host,
    first_heartbeat_epoch: options.epoch || 0,
    total_heartbeats: 0,
  };
  record.last_heartbeat_epoch = options.epoch || 0;
  record.port_addr = options.port_addr || record.port_addr || null;
  record.state_hash = options.state_hash || record.state_hash || null;
  record.total_heartbeats = (record.total_heartbeats || 0) + 1;
  heartbeats.set(key, record);

  if (svc.active_hosts.indexOf(host) === -1) {
    svc.active_hosts.push(host);
    services.set(slug, svc);
  }

  return record;
}

/**
 * Start a session against a deployed service.
 * The host must be in the service's active hosts list.
 */
function startSession(user, host, slug, sessionId, options) {
  options = options || {};
  if (!user) throw new Error("user required");
  if (!host) throw new Error("host required");
  if (!sessionId) throw new Error("sessionId required");

  var svc = services.get(slug);
  if (!svc) throw new Error("service not found");
  if (svc.status !== "active") throw new Error("service is not active");
  if (svc.active_hosts.indexOf(host) === -1) {
    throw new Error("host is not active for this service");
  }
  if (sessions.has(sessionId)) {
    throw new Error("session id already in use");
  }

  var session = {
    session_id: sessionId,
    service_slug: slug,
    user: user,
    host: host,
    escrow_id: options.escrow_id || null,
    max_duration_epochs: options.max_duration_epochs || 100,
    started_epoch: options.epoch || 0,
    ended_epoch: null,
    actual_duration_epochs: 0,
    status: "active",
  };
  sessions.set(sessionId, session);

  svc.total_sessions = (svc.total_sessions || 0) + 1;
  services.set(slug, svc);

  return session;
}

/**
 * End an active session. Either the user or the host can end it.
 * Sets the actual_duration_epochs based on (epoch - started_epoch).
 */
function endSession(party, sessionId, epoch) {
  var session = sessions.get(sessionId);
  if (!session) throw new Error("session not found");
  if (session.user !== party && session.host !== party) {
    throw new Error("only the user or host can end this session");
  }
  if (session.status !== "active") return session; // already ended, idempotent

  session.ended_epoch = epoch || 0;
  session.actual_duration_epochs = (epoch || 0) - session.started_epoch;
  if (session.actual_duration_epochs < 0) session.actual_duration_epochs = 0;
  session.status = "ended";
  session.ended_by = party;
  sessions.set(sessionId, session);
  return session;
}

// ─────────────────────────────────────────────────────────────────
// Read accessors
// ─────────────────────────────────────────────────────────────────

function getService(slug) {
  return services.get(slug) || null;
}

function getAllServices(filter) {
  var result = [];
  for (var entry of services) {
    var s = entry[1];
    if (filter) {
      if (filter.status && s.status !== filter.status) continue;
      if (filter.deployer && s.deployer !== filter.deployer) continue;
      if (filter.type && s.runtime.type !== filter.type) continue;
    }
    result.push(s);
  }
  return result;
}

function getServicesByDeployer(deployer) {
  return getAllServices({ deployer: deployer });
}

function getHeartbeat(slug, host) {
  return heartbeats.get(slug + "|" + host) || null;
}

function getActiveHostsForService(slug, currentEpoch, recentEpochs) {
  recentEpochs = recentEpochs || 100;
  var threshold = (currentEpoch || 0) - recentEpochs;
  var result = [];
  for (var entry of heartbeats) {
    var hb = entry[1];
    if (hb.slug !== slug) continue;
    if ((hb.last_heartbeat_epoch || 0) >= threshold) result.push(hb);
  }
  return result;
}

function getSession(sessionId) {
  return sessions.get(sessionId) || null;
}

function getSessionsForUser(user) {
  var result = [];
  for (var entry of sessions) {
    if (entry[1].user === user) result.push(entry[1]);
  }
  return result;
}

function getSessionsForHost(host) {
  var result = [];
  for (var entry of sessions) {
    if (entry[1].host === host) result.push(entry[1]);
  }
  return result;
}

function getActiveSessions(slug) {
  var result = [];
  for (var entry of sessions) {
    var s = entry[1];
    if (s.service_slug !== slug) continue;
    if (s.status === "active") result.push(s);
  }
  return result;
}

/**
 * Get all heartbeat records whose last_heartbeat_epoch matches `epoch`.
 * Used by the block-emission reward distributor (v2.13.4+) to identify
 * which service hosts are eligible for the service pool this epoch.
 *
 * Returns an array of heartbeat record objects (one per slug+host pair).
 */
function _getHeartbeatsForEpoch(epoch) {
  var result = [];
  for (var entry of heartbeats) {
    if (entry[1].last_heartbeat_epoch === epoch) {
      result.push(entry[1]);
    }
  }
  return result;
}

/**
 * Reset all state. For tests only.
 */
function resetForTests() {
  services.clear();
  heartbeats.clear();
  sessions.clear();
  requestsByEpoch.clear();
}

/**
 * Record requests served by a host in an epoch.
 * Called by the host-side service runner after each completed request batch.
 */
function recordRequest(host, count, epoch) {
  if (!host || !epoch || epoch < 0) return;
  var n = parseInt(count, 10) || 1;
  if (!requestsByEpoch.has(epoch)) requestsByEpoch.set(epoch, new Map());
  var rmap = requestsByEpoch.get(epoch);
  rmap.set(host, (rmap.get(host) || 0) + n);
  if (epoch > _currentRequestEpoch) {
    _currentRequestEpoch = epoch;
    for (var key of requestsByEpoch.keys()) {
      if (key < epoch - 20) requestsByEpoch.delete(key);
    }
  }
}

/**
 * Get requests served per host for an epoch window.
 * Returns { [host]: request_count } — actual work signal for reward distribution.
 */
function getServiceWorkByEpoch(epochNumber) {
  var WINDOW = 3;
  var result = {};
  for (var i = 0; i <= WINDOW; i++) {
    var rmap = requestsByEpoch.get(epochNumber - i);
    if (!rmap) continue;
    for (var entry of rmap) {
      result[entry[0]] = (result[entry[0]] || 0) + entry[1];
    }
  }
  return result;
}

module.exports = {
  // Mutators
  deploy: deploy,
  retire: retire,
  heartbeat: heartbeat,
  startSession: startSession,
  endSession: endSession,
  recordRequest: recordRequest,
  // Readers
  getService: getService,
  getAllServices: getAllServices,
  getServicesByDeployer: getServicesByDeployer,
  getHeartbeat: getHeartbeat,
  getActiveHostsForService: getActiveHostsForService,
  getServiceWorkByEpoch: getServiceWorkByEpoch,
  getSession: getSession,
  getSessionsForUser: getSessionsForUser,
  getSessionsForHost: getSessionsForHost,
  getActiveSessions: getActiveSessions,
  // Helpers
  parseSlug: parseSlug,
  _getHeartbeatsForEpoch: _getHeartbeatsForEpoch,
  resetForTests: resetForTests,
  SLUG_PATTERN: SLUG_PATTERN,
  CID_PATTERN: CID_PATTERN,
};
