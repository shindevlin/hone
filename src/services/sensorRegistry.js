"use strict";

/**
 * BTCPC Sensor Registry — v2.15-alpha
 * Shin Devlin
 *
 * In-memory registry of IoT sensor devices that report environmental
 * data to the BTCPC chain. A sensor is essentially an oracle feed
 * where readings come from a physical device instead of an API.
 *
 * Median consensus for epoch finalization delegates to oracleFeeds._median()
 * so the math stays in one place.
 *
 * Sensor ID format: "<owner>/<device-name>"
 *   - same slug rules as serviceRegistry and oracleFeeds
 *
 * Pay-for-delivery rule (same as storage): sensors are never slashed for
 * going offline. A sensor that stops reporting just stops earning. Its
 * reputation dips over time if it is chronically flaky, but its on-chain
 * balance is untouched. See feedback_storage_no_slash.md.
 *
 * Sensor states:
 *   active  — registered and sending readings recently
 *   idle    — no readings for >= IDLE_EPOCH_THRESHOLD epochs
 *   retired — permanently deactivated by owner
 */

var oracle = require('./oracleFeeds');

// sensor_id → sensor record
var sensors = new Map();

// "<sensor_id>|<epoch>" → array of reading records
var readingsByEpoch = new Map();

// "<sensor_id>|<epoch>" → finalized consensus record
var finalizedReadings = new Map();

// sensor_id → stats accumulator
var sensorStats = new Map();

// Fraud prevention: per-sensor rolling history of last 10 values
var readingHistory = new Map(); // sensorId → array of last 10 numeric values

// Fraud prevention: rate-limit tracking — last epoch a sensor submitted
var lastEpochBySensor = new Map(); // sensorId → epoch number

var SLUG_PATTERN = /^[a-z0-9][a-z0-9-]{0,62}$/;
var VALID_TYPES = ["temperature", "humidity", "air_quality", "gps", "soil", "uwb_position", "uwb_range", "motion", "power", "noise", "light", "pressure", "custom"];

// Number of epochs of silence before a sensor transitions to "idle"
var IDLE_EPOCH_THRESHOLD = 100;

function _readingsKey(sensorId, epoch) {
  return sensorId + "|" + epoch;
}

function parseSensorId(sensorId) {
  if (typeof sensorId !== "string") {
    return { ok: false, error: "sensor_id must be a string" };
  }
  var parts = sensorId.split("/");
  if (parts.length !== 2) {
    return { ok: false, error: "sensor_id must be <owner>/<device-name>" };
  }
  if (!SLUG_PATTERN.test(parts[0])) return { ok: false, error: "invalid owner name" };
  if (!SLUG_PATTERN.test(parts[1])) return { ok: false, error: "invalid device name" };
  return { ok: true, owner: parts[0], name: parts[1] };
}

/**
 * Register a new IoT sensor. Can also update an existing sensor's spec
 * (owner cannot change on update).
 *
 * @param {string} owner — BTCPC account that owns this sensor
 * @param {string} sensorId — "<owner>/<device-name>"
 * @param {object} spec
 * @param {string} spec.type — one of VALID_TYPES
 * @param {string} spec.unit — e.g., "celsius", "pct", "ppm", "degrees"
 * @param {number} spec.decimals — decimal precision (0-10)
 * @param {string} spec.region — geographic region identifier
 * @param {string} [spec.lora_gateway] — gateway_id relaying this sensor
 * @param {string} [spec.hardware_model] — e.g., "raspberry-pi-4", "esp32", "helium-miner"
 * @param {string} [spec.firmware_version] — semver string
 * @param {object} [options]
 * @param {number} [options.epoch] — current epoch
 */
function registerSensor(owner, sensorId, spec, options) {
  options = options || {};
  if (!owner) throw new Error("owner required");

  var parsed = parseSensorId(sensorId);
  if (!parsed.ok) throw new Error("invalid sensor_id: " + parsed.error);
  if (parsed.owner !== owner) {
    throw new Error("sensor_id owner prefix must match the registering account");
  }
  if (!spec || typeof spec !== "object") throw new Error("spec required");

  if (VALID_TYPES.indexOf(spec.type) < 0) {
    throw new Error("spec.type must be one of: " + VALID_TYPES.join(", "));
  }
  if (typeof spec.unit !== "string" || spec.unit.length === 0) {
    throw new Error("spec.unit required");
  }
  var decimals = parseInt(spec.decimals, 10);
  if (!Number.isFinite(decimals) || decimals < 0 || decimals > 10) {
    throw new Error("spec.decimals must be 0-10");
  }
  if (typeof spec.region !== "string" || spec.region.length === 0) {
    throw new Error("spec.region required");
  }

  var existing = sensors.get(sensorId);
  var epoch = options.epoch || 0;
  var record;

  if (existing) {
    if (existing.owner !== owner) {
      throw new Error("only the original owner can update this sensor");
    }
    if (existing.status === "retired") {
      throw new Error("cannot update a retired sensor");
    }
    record = existing;
    record.type = spec.type;
    record.unit = spec.unit;
    record.decimals = decimals;
    record.region = spec.region;
    record.lora_gateway = spec.lora_gateway || record.lora_gateway || null;
    record.hardware_model = spec.hardware_model || record.hardware_model || null;
    record.firmware_version = spec.firmware_version || record.firmware_version || null;
    record.last_updated_epoch = epoch;
    record.status = "active";
  } else {
    record = {
      sensor_id: sensorId,
      owner: owner,
      type: spec.type,
      unit: spec.unit,
      decimals: decimals,
      region: spec.region,
      lora_gateway: spec.lora_gateway || null,
      hardware_model: spec.hardware_model || null,
      firmware_version: spec.firmware_version || null,
      status: "active",
      created_epoch: epoch,
      last_updated_epoch: epoch,
      last_reading_epoch: null,
      total_readings: 0,
      total_epochs_active: 0,
      total_outliers: 0,
    };
    // Initialize stats accumulator
    sensorStats.set(sensorId, {
      total_readings: 0,
      epochs_with_readings: new Set(),
      total_outliers: 0,
      last_reading_epoch: null,
      outlier_epochs: new Set(),
    });
  }

  sensors.set(sensorId, record);
  return record;
}

/**
 * Permanently deactivate a sensor. Only the owner can retire.
 * No new readings can be submitted after retirement.
 */
function retireSensor(owner, sensorId, epoch) {
  var record = sensors.get(sensorId);
  if (!record) throw new Error("sensor not found");
  if (record.owner !== owner) {
    throw new Error("only the owner can retire this sensor");
  }
  if (record.status === "retired") return record;
  record.status = "retired";
  record.retired_epoch = epoch || 0;
  sensors.set(sensorId, record);
  return record;
}

/**
 * Submit a sensor reading for an epoch.
 *
 * @param {string} sensorId
 * @param {number} value — the measurement value
 * @param {object} metadata — { latitude, longitude, altitude, battery_pct, signal_strength_dbm, gateway_id }
 * @param {number} epoch
 */
function submitReading(sensorId, value, metadata, epoch) {
  var record = sensors.get(sensorId);
  if (!record) throw new Error("sensor not found");
  if (record.status === "retired") {
    throw new Error("cannot submit reading for a retired sensor");
  }

  var numeric = Number(value);
  if (!Number.isFinite(numeric)) {
    throw new Error("reading value must be a finite number");
  }

  var ep = epoch || 0;

  // ── Fraud prevention: rate limit (one reading per sensor per epoch) ──
  var prevEpoch = lastEpochBySensor.get(sensorId);
  if (prevEpoch !== undefined && prevEpoch === ep) {
    throw new Error("duplicate reading: sensor " + sensorId + " already submitted in epoch " + ep);
  }
  lastEpochBySensor.set(sensorId, ep);

  // ── Fraud prevention: outlier detection (3-sigma on rolling window) ──
  var history = readingHistory.get(sensorId) || [];
  var outlier = false;
  if (history.length >= 3) {
    var sum = 0;
    for (var h = 0; h < history.length; h++) sum += history[h];
    var mean = sum / history.length;
    var sqSum = 0;
    for (var h2 = 0; h2 < history.length; h2++) sqSum += (history[h2] - mean) * (history[h2] - mean);
    var stddev = Math.sqrt(sqSum / history.length);
    if (stddev > 0 && Math.abs(numeric - mean) > 3 * stddev) {
      outlier = true;
    }
  }
  // Update rolling buffer (keep last 10)
  history.push(numeric);
  if (history.length > 10) history.shift();
  readingHistory.set(sensorId, history);

  // ── Fraud prevention: gateway attestation ──
  var meta = metadata || {};
  var gatewayVerified = false;
  var trustWeight = 1;
  if (meta.gateway_id) {
    // Check if this gateway_id matches the sensor's registered lora_gateway
    if (record.lora_gateway && meta.gateway_id === record.lora_gateway) {
      gatewayVerified = true;
    }
  } else {
    trustWeight = 0.5;
  }

  var key = _readingsKey(sensorId, ep);

  var readings = readingsByEpoch.get(key) || [];

  var reading = {
    sensor_id: sensorId,
    epoch: ep,
    value: numeric,
    metadata: meta,
    submitted_at: Date.now(),
    outlier: outlier,
    gateway_verified: gatewayVerified,
    trust_weight: trustWeight,
  };

  readings.push(reading);
  readingsByEpoch.set(key, readings);

  // Update sensor record
  record.total_readings = (record.total_readings || 0) + 1;
  record.last_reading_epoch = ep;
  // Reactivate if idle
  if (record.status === "idle") {
    record.status = "active";
  }
  sensors.set(sensorId, record);

  // Update stats accumulator
  var stats = sensorStats.get(sensorId);
  if (stats) {
    stats.total_readings += 1;
    stats.epochs_with_readings.add(ep);
    stats.last_reading_epoch = ep;
  }

  return reading;
}

/**
 * Finalize epoch readings for a sensor — compute median consensus across
 * all readings from sensors in the same region+type for this epoch,
 * flag outliers using oracleFeeds._median().
 *
 * Returns the finalized consensus record or throws if no readings exist.
 */
function finalizeEpochReadings(sensorId, epoch) {
  var record = sensors.get(sensorId);
  if (!record) throw new Error("sensor not found");

  var ep = epoch || 0;
  var key = _readingsKey(sensorId, ep);

  // Idempotent
  if (finalizedReadings.has(key)) {
    return finalizedReadings.get(key);
  }

  var readings = readingsByEpoch.get(key) || [];
  if (readings.length === 0) {
    throw new Error("no readings for sensor " + sensorId + " in epoch " + ep);
  }

  // Gather all readings from same region+type for this epoch (cross-sensor consensus)
  var regionValues = [];
  for (var entry of sensors) {
    var s = entry[1];
    if (s.type !== record.type || s.region !== record.region) continue;
    var peerKey = _readingsKey(s.sensor_id, ep);
    var peerReadings = readingsByEpoch.get(peerKey) || [];
    for (var i = 0; i < peerReadings.length; i++) {
      regionValues.push(peerReadings[i].value);
    }
  }

  var median = oracle._median(regionValues.length > 0 ? regionValues : readings.map(function(r) { return r.value; }));
  var medianRounded = parseFloat(Number(median).toFixed(record.decimals));

  // Flag outlier readings for this sensor
  // Use 500 bps (5%) as the default deviation threshold for sensors
  var OUTLIER_BPS = 500;
  var outliers = [];
  var insiders = [];

  for (var j = 0; j < readings.length; j++) {
    var r = readings[j];
    var isOutlier = false;
    if (medianRounded === 0) {
      isOutlier = r.value !== 0;
    } else {
      var deviation = Math.abs((r.value - medianRounded) / medianRounded) * 10000;
      if (deviation > OUTLIER_BPS) isOutlier = true;
    }
    if (isOutlier) {
      outliers.push({ value: r.value, deviation_bps: medianRounded === 0 ? Infinity : Math.abs((r.value - medianRounded) / medianRounded) * 10000 });
    } else {
      insiders.push(r.value);
    }
  }

  // Update sensor stats
  var stats = sensorStats.get(sensorId);
  if (stats && outliers.length > 0) {
    stats.total_outliers += outliers.length;
    stats.outlier_epochs.add(ep);
    record.total_outliers = (record.total_outliers || 0) + outliers.length;
    sensors.set(sensorId, record);
  }
  if (stats) {
    stats.total_epochs_active = stats.epochs_with_readings.size;
  }
  record.total_epochs_active = stats ? stats.epochs_with_readings.size : 0;
  sensors.set(sensorId, record);

  var finalization = {
    sensor_id: sensorId,
    epoch: ep,
    median: medianRounded,
    reading_count: readings.length,
    region_value_count: regionValues.length,
    insiders: insiders,
    outliers: outliers,
    finalized_at: Date.now(),
  };

  finalizedReadings.set(key, finalization);
  return finalization;
}

/**
 * Check and apply idle state transition for a sensor.
 * Called with the current epoch; if the sensor's last_reading_epoch
 * is more than IDLE_EPOCH_THRESHOLD epochs ago, set status to "idle".
 * No penalty — just not earning.
 */
function _checkIdleTransition(sensorId, currentEpoch) {
  var record = sensors.get(sensorId);
  if (!record || record.status === "retired") return;
  if (record.last_reading_epoch === null) return;
  var gap = (currentEpoch || 0) - record.last_reading_epoch;
  if (gap >= IDLE_EPOCH_THRESHOLD && record.status === "active") {
    record.status = "idle";
    sensors.set(sensorId, record);
  }
}

// ─────────────────────────────────────────────────────────────────
// Read accessors
// ─────────────────────────────────────────────────────────────────

function getSensor(sensorId) {
  return sensors.get(sensorId) || null;
}

function getAllSensors(filter) {
  var result = [];
  for (var entry of sensors) {
    var s = entry[1];
    if (filter) {
      if (filter.region && s.region !== filter.region) continue;
      if (filter.type && s.type !== filter.type) continue;
      if (filter.owner && s.owner !== filter.owner) continue;
      if (filter.status && s.status !== filter.status) continue;
    }
    result.push(s);
  }
  return result;
}

function getSensorsInRegion(region) {
  return getAllSensors({ region: region });
}

function getReadingsForEpoch(sensorId, epoch) {
  var key = _readingsKey(sensorId, epoch || 0);
  return (readingsByEpoch.get(key) || []).slice();
}

function getFinalizedReading(sensorId, epoch) {
  var key = _readingsKey(sensorId, epoch || 0);
  return finalizedReadings.get(key) || null;
}

/**
 * Return aggregated stats for a sensor.
 * uptime_pct is based on epochs_with_readings / total_epochs since creation.
 */
function getSensorStats(sensorId, currentEpoch) {
  var record = sensors.get(sensorId);
  if (!record) return null;

  var stats = sensorStats.get(sensorId) || {
    total_readings: 0,
    epochs_with_readings: new Set(),
    total_outliers: 0,
    last_reading_epoch: null,
  };

  var totalReadings = stats.total_readings;
  var totalEpochsActive = stats.epochs_with_readings.size;
  var outlierRate = totalReadings > 0 ? stats.total_outliers / totalReadings : 0;

  // Uptime: fraction of epochs (since creation) where we got at least one reading
  var epochSpan = currentEpoch !== undefined
    ? Math.max(1, (currentEpoch || 0) - record.created_epoch + 1)
    : Math.max(1, totalEpochsActive);
  var uptimePct = Math.min(1, totalEpochsActive / epochSpan);

  return {
    sensor_id: sensorId,
    total_readings: totalReadings,
    total_epochs_active: totalEpochsActive,
    outlier_rate: outlierRate,
    last_reading_epoch: stats.last_reading_epoch,
    uptime_pct: uptimePct,
  };
}

function resetForTests() {
  sensors.clear();
  readingsByEpoch.clear();
  finalizedReadings.clear();
  sensorStats.clear();
  readingHistory.clear();
  lastEpochBySensor.clear();
}

module.exports = {
  // Mutators
  registerSensor: registerSensor,
  retireSensor: retireSensor,
  submitReading: submitReading,
  finalizeEpochReadings: finalizeEpochReadings,
  // Readers
  getSensor: getSensor,
  getAllSensors: getAllSensors,
  getSensorsInRegion: getSensorsInRegion,
  getReadingsForEpoch: getReadingsForEpoch,
  getFinalizedReading: getFinalizedReading,
  getSensorStats: getSensorStats,
  // Helpers
  parseSensorId: parseSensorId,
  _checkIdleTransition: _checkIdleTransition,
  resetForTests: resetForTests,
  SLUG_PATTERN: SLUG_PATTERN,
  VALID_TYPES: VALID_TYPES,
  IDLE_EPOCH_THRESHOLD: IDLE_EPOCH_THRESHOLD,
};
