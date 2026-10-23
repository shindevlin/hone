"use strict";

/**
 * HONE LoRa Gateway Registry — v2.15-alpha
 * Shin Devlin
 *
 * In-memory registry of LoRa gateways that relay sensor readings from
 * nearby HONE-nano devices to the HONE network. A gateway is a host
 * for sensors: it receives LoRa radio packets and forwards them via
 * HTTP or P2P.
 *
 * Gateway ID format: "<owner>/<gateway-name>"
 *   - same slug rules as serviceRegistry and sensorRegistry
 *
 * Pay-for-delivery rule (same as storage and sensors): gateways are
 * never slashed for going offline. A gateway that stops sending
 * heartbeats just stops earning. See feedback_storage_no_slash.md.
 *
 * Gateway states:
 *   active  — registered and heartbeating recently
 *   idle    — no heartbeat for >= IDLE_HEARTBEAT_THRESHOLD epochs
 *   retired — permanently deactivated by owner
 */

var SLUG_PATTERN = /^[a-z0-9][a-z0-9-]{0,62}$/;

// gateway_id → gateway record
var gateways = new Map();

// gateway_id → stats accumulator
var gatewayStats = new Map();

// owner → packets relayed per epoch: Map<epoch, Map<owner, count>>
var packetsByEpoch = new Map();
var _currentPacketEpoch = -1;

// Number of epochs of silence before a gateway transitions to "idle"
var IDLE_HEARTBEAT_THRESHOLD = 100;

function parseGatewayId(gatewayId) {
  if (typeof gatewayId !== "string") {
    return { ok: false, error: "gateway_id must be a string" };
  }
  var parts = gatewayId.split("/");
  if (parts.length !== 2) {
    return { ok: false, error: "gateway_id must be <owner>/<gateway-name>" };
  }
  if (!SLUG_PATTERN.test(parts[0])) return { ok: false, error: "invalid owner name" };
  if (!SLUG_PATTERN.test(parts[1])) return { ok: false, error: "invalid gateway name" };
  return { ok: true, owner: parts[0], name: parts[1] };
}

/**
 * Register a new LoRa gateway. Can also update an existing gateway's
 * spec (owner cannot change on update).
 *
 * @param {string} owner — HONE account that owns this gateway
 * @param {string} gatewayId — "<owner>/<gateway-name>"
 * @param {object} spec
 * @param {string} spec.region — geographic region identifier
 * @param {number} spec.latitude — GPS latitude
 * @param {number} spec.longitude — GPS longitude
 * @param {number} [spec.antenna_gain_dbi] — antenna gain in dBi
 * @param {string} [spec.hardware_model] — e.g., "helium-rak-v2", "dragino-lg308"
 * @param {string} [spec.firmware_version] — semver string
 * @param {number} [spec.max_sensors] — max concurrent sensors this gateway can serve
 * @param {object} [options]
 * @param {number} [options.epoch] — current epoch
 */
function registerGateway(owner, gatewayId, spec, options) {
  options = options || {};
  if (!owner) throw new Error("owner required");

  var parsed = parseGatewayId(gatewayId);
  if (!parsed.ok) throw new Error("invalid gateway_id: " + parsed.error);
  if (parsed.owner !== owner) {
    throw new Error("gateway_id owner prefix must match the registering account");
  }
  if (!spec || typeof spec !== "object") throw new Error("spec required");

  if (typeof spec.region !== "string" || spec.region.length === 0) {
    throw new Error("spec.region required");
  }

  var lat = Number(spec.latitude);
  var lon = Number(spec.longitude);
  if (!Number.isFinite(lat) || lat < -90 || lat > 90) {
    throw new Error("spec.latitude must be a finite number between -90 and 90");
  }
  if (!Number.isFinite(lon) || lon < -180 || lon > 180) {
    throw new Error("spec.longitude must be a finite number between -180 and 180");
  }

  var epoch = options.epoch || 0;
  var existing = gateways.get(gatewayId);
  var record;

  if (existing) {
    if (existing.owner !== owner) {
      throw new Error("only the original owner can update this gateway");
    }
    if (existing.status === "retired") {
      throw new Error("cannot update a retired gateway");
    }
    record = existing;
    record.region = spec.region;
    record.latitude = lat;
    record.longitude = lon;
    record.antenna_gain_dbi = spec.antenna_gain_dbi !== undefined ? Number(spec.antenna_gain_dbi) : record.antenna_gain_dbi;
    record.hardware_model = spec.hardware_model || record.hardware_model || null;
    record.firmware_version = spec.firmware_version || record.firmware_version || null;
    record.max_sensors = spec.max_sensors !== undefined ? parseInt(spec.max_sensors, 10) : record.max_sensors;
    if (spec.public_key) record.public_key = spec.public_key;
    record.last_updated_epoch = epoch;
    record.status = "active";
  } else {
    record = {
      gateway_id: gatewayId,
      owner: owner,
      region: spec.region,
      latitude: lat,
      longitude: lon,
      antenna_gain_dbi: spec.antenna_gain_dbi !== undefined ? Number(spec.antenna_gain_dbi) : null,
      hardware_model: spec.hardware_model || null,
      firmware_version: spec.firmware_version || null,
      // secp256k1 compressed hex public key for gateway signature verification
      public_key: spec.public_key || null,
      max_sensors: spec.max_sensors !== undefined ? parseInt(spec.max_sensors, 10) : 50,
      status: "active",
      created_epoch: epoch,
      last_updated_epoch: epoch,
      last_heartbeat_epoch: null,
      total_heartbeats: 0,
      total_packets_relayed: 0,
      total_sensors_served: 0,
    };
    gatewayStats.set(gatewayId, {
      heartbeat_epochs: new Set(),
      total_packets_relayed: 0,
      total_sensors_peak: 0,
      total_uptime_seconds: 0,
    });
  }

  gateways.set(gatewayId, record);
  return record;
}

/**
 * Permanently deactivate a gateway. Only the owner can retire.
 */
function retireGateway(owner, gatewayId, epoch) {
  var record = gateways.get(gatewayId);
  if (!record) throw new Error("gateway not found");
  if (record.owner !== owner) {
    throw new Error("only the owner can retire this gateway");
  }
  if (record.status === "retired") return record;
  record.status = "retired";
  record.retired_epoch = epoch || 0;
  gateways.set(gatewayId, record);
  return record;
}

/**
 * Record a gateway heartbeat — gateway reports it is online.
 *
 * @param {string} gatewayId
 * @param {object} stats — { sensors_connected, packets_relayed_this_epoch, uptime_seconds, battery_pct }
 * @param {number} epoch
 */
function heartbeat(gatewayId, stats, epoch) {
  var record = gateways.get(gatewayId);
  if (!record) throw new Error("gateway not found");
  if (record.status === "retired") {
    throw new Error("cannot send heartbeat for a retired gateway");
  }

  var ep = epoch || 0;
  stats = stats || {};

  // Reactivate if idle
  if (record.status === "idle") {
    record.status = "active";
  }

  record.last_heartbeat_epoch = ep;
  record.total_heartbeats = (record.total_heartbeats || 0) + 1;

  var packetsThisEpoch = parseInt(stats.packets_relayed_this_epoch, 10) || 0;
  record.total_packets_relayed = (record.total_packets_relayed || 0) + packetsThisEpoch;

  var sensorsConnected = parseInt(stats.sensors_connected, 10) || 0;
  record.total_sensors_served = Math.max(record.total_sensors_served || 0, sensorsConnected);

  // Record last reported stats for UX / diagnostics
  record.last_stats = {
    sensors_connected: sensorsConnected,
    packets_relayed_this_epoch: packetsThisEpoch,
    uptime_seconds: stats.uptime_seconds || null,
    battery_pct: stats.battery_pct !== undefined ? stats.battery_pct : null,
    reported_epoch: ep,
  };

  gateways.set(gatewayId, record);

  // Update stats accumulator
  var acc = gatewayStats.get(gatewayId);
  if (acc) {
    acc.heartbeat_epochs.add(ep);
    acc.total_packets_relayed += packetsThisEpoch;
    acc.total_sensors_peak = Math.max(acc.total_sensors_peak, sensorsConnected);
    if (stats.uptime_seconds) {
      acc.total_uptime_seconds += Number(stats.uptime_seconds) || 0;
    }
  }

  // Record packets per owner per epoch for reward distribution
  if (packetsThisEpoch > 0 && ep > 0) {
    var owner = parseGatewayId(gatewayId).owner;
    if (!packetsByEpoch.has(ep)) packetsByEpoch.set(ep, new Map());
    var pmap = packetsByEpoch.get(ep);
    pmap.set(owner, (pmap.get(owner) || 0) + packetsThisEpoch);
    if (ep > _currentPacketEpoch) {
      _currentPacketEpoch = ep;
      for (var pkey of packetsByEpoch.keys()) {
        if (pkey < ep - 20) packetsByEpoch.delete(pkey);
      }
    }
  }

  return record;
}

/**
 * Check and apply idle state transition for a gateway.
 */
function _checkIdleTransition(gatewayId, currentEpoch) {
  var record = gateways.get(gatewayId);
  if (!record || record.status === "retired") return;
  if (record.last_heartbeat_epoch === null) return;
  var gap = (currentEpoch || 0) - record.last_heartbeat_epoch;
  if (gap >= IDLE_HEARTBEAT_THRESHOLD && record.status === "active") {
    record.status = "idle";
    gateways.set(gatewayId, record);
  }
}

// ─────────────────────────────────────────────────────────────────
// Read accessors
// ─────────────────────────────────────────────────────────────────

function getGateway(gatewayId) {
  return gateways.get(gatewayId) || null;
}

function getAllGateways(filter) {
  var result = [];
  for (var entry of gateways) {
    var g = entry[1];
    if (filter) {
      if (filter.region && g.region !== filter.region) continue;
      if (filter.owner && g.owner !== filter.owner) continue;
      if (filter.status && g.status !== filter.status) continue;
    }
    result.push(g);
  }
  return result;
}

function getGatewaysInRegion(region) {
  return getAllGateways({ region: region });
}

/**
 * Return aggregated stats for a gateway.
 * uptime_pct based on heartbeat epochs vs total epochs since creation.
 */
function getGatewayStats(gatewayId, currentEpoch) {
  var record = gateways.get(gatewayId);
  if (!record) return null;

  var acc = gatewayStats.get(gatewayId) || {
    heartbeat_epochs: new Set(),
    total_packets_relayed: 0,
    total_sensors_peak: 0,
    total_uptime_seconds: 0,
  };

  var heartbeatCount = acc.heartbeat_epochs.size;
  var epochSpan = currentEpoch !== undefined
    ? Math.max(1, (currentEpoch || 0) - record.created_epoch + 1)
    : Math.max(1, heartbeatCount);
  var uptimePct = Math.min(1, heartbeatCount / epochSpan);

  return {
    gateway_id: gatewayId,
    total_heartbeats: record.total_heartbeats,
    total_packets_relayed: acc.total_packets_relayed,
    peak_sensors_connected: acc.total_sensors_peak,
    total_uptime_seconds: acc.total_uptime_seconds,
    uptime_pct: uptimePct,
    last_heartbeat_epoch: record.last_heartbeat_epoch,
  };
}

/**
 * Verify a gateway signature over a sensor reading payload.
 * Signature covers { sensor_id, epoch, value, nonce } deterministically.
 *
 * @param {string} gatewayId
 * @param {{ sensor_id, epoch, value, nonce }} payload
 * @param {string} signature — hex secp256k1 signature
 * @returns {boolean}
 */
function verifyGatewaySignature(gatewayId, payload, signature) {
  if (!gatewayId || !payload || !signature) return false;
  var record = gateways.get(gatewayId);
  if (!record || !record.public_key) return false;
  try {
    var messageAuth = require("./messageAuth");
    var canonical = {
      sensor_id: payload.sensor_id,
      epoch: payload.epoch,
      value: payload.value,
      nonce: payload.nonce || ""
    };
    return messageAuth.verifyMessage(canonical, signature, record.public_key);
  } catch (_) {
    return false;
  }
}

/**
 * Get packets relayed per gateway owner for an epoch window.
 * Returns { [owner]: packet_count } — the actual work signal for reward distribution.
 */
function getGatewayWorkByEpoch(epochNumber) {
  var WINDOW = 3;
  var result = {};
  for (var i = 0; i <= WINDOW; i++) {
    var pmap = packetsByEpoch.get(epochNumber - i);
    if (!pmap) continue;
    for (var entry of pmap) {
      result[entry[0]] = (result[entry[0]] || 0) + entry[1];
    }
  }
  return result;
}

function resetForTests() {
  gateways.clear();
  gatewayStats.clear();
  packetsByEpoch.clear();
}

module.exports = {
  // Mutators
  registerGateway: registerGateway,
  retireGateway: retireGateway,
  heartbeat: heartbeat,
  // Readers
  getGateway: getGateway,
  getAllGateways: getAllGateways,
  getGatewaysInRegion: getGatewaysInRegion,
  getGatewayStats: getGatewayStats,
  getGatewayWorkByEpoch: getGatewayWorkByEpoch,
  // Security
  verifyGatewaySignature: verifyGatewaySignature,
  // Helpers
  parseGatewayId: parseGatewayId,
  _checkIdleTransition: _checkIdleTransition,
  resetForTests: resetForTests,
  SLUG_PATTERN: SLUG_PATTERN,
  IDLE_HEARTBEAT_THRESHOLD: IDLE_HEARTBEAT_THRESHOLD,
};
