"use strict";

/**
 * HONE Dynamic Sensor Reward Configuration
 * Shin Devlin
 *
 * Reward multipliers per sensor type, based on:
 *   1. Natural data frequency (how often the sensor produces useful data)
 *   2. Hardware cost (expensive sensors cost more to run)
 *   3. Electricity usage (power-hungry sensors get a bonus)
 *   4. Data value (GNSS positioning is worth more than ambient light)
 *   5. Novelty (seismic sensors get a large bonus when actual events occur)
 *
 * Witness factor:
 *   1 independent witness → 10% of full epoch reward
 *   2 independent witnesses → 55% of full epoch reward
 *   3+ independent witnesses → 100% of full epoch reward
 *
 * This incentivizes sensor coverage (multiple gateways in range),
 * while making single-source fraud economically unattractive.
 */

// Per-type config. All types in sensorRegistry.VALID_TYPES must appear here
// (unmapped types fall back to _default).
//
// Fields:
//   multiplier    — base reward multiplier vs. a 1.0 reference temperature sensor
//   stake_required — HONE stake required per sensor when owner has 3+ devices
//   freq_per_hour  — natural reading frequency (informational, used for UI/docs)
//   power_w        — typical power draw in watts (informational, contributes to multiplier)
//   novelty_mult   — multiplier applied instead of base when novelty_threshold is crossed
//   novelty_threshold — |value| at which novelty_mult activates (e.g. seismic)

const SENSOR_TYPE_CONFIG = {
  // ── GNSS / positioning ──────────────────────────────────────────────────────
  // High-frequency, expensive hardware, high-power, extremely valuable
  'gps':          { multiplier: 12.0, stake_required: 500, freq_per_hour: 1000, power_w: 5.0 },
  'gps-location': { multiplier: 12.0, stake_required: 500, freq_per_hour: 1000, power_w: 5.0 },
  'uwb_position': { multiplier: 6.0,  stake_required: 200, freq_per_hour: 10,   power_w: 0.5 },
  'uwb_range':    { multiplier: 5.0,  stake_required: 200, freq_per_hour: 10,   power_w: 0.5 },

  // ── Environmental (high-value) ───────────────────────────────────────────────
  'air_quality':  { multiplier: 3.0,  stake_required: 100, freq_per_hour: 12,   power_w: 0.3 },
  'soil':         { multiplier: 2.0,  stake_required: 100, freq_per_hour: 4,    power_w: 0.2 },
  'noise':        { multiplier: 1.5,  stake_required: 50,  freq_per_hour: 6,    power_w: 0.1 },
  'pressure':     { multiplier: 1.5,  stake_required: 50,  freq_per_hour: 12,   power_w: 0.1 },
  'barometer':    { multiplier: 1.5,  stake_required: 50,  freq_per_hour: 12,   power_w: 0.1 },

  // ── Seismic — base is modest, novelty bonus fires when activity detected ────
  // novelty_threshold: absolute reading value that represents actual seismic activity
  'seismic':      {
    multiplier: 3.0,
    stake_required: 100,
    freq_per_hour: 1,
    power_w: 0.3,
    novelty_mult: 50.0,
    novelty_threshold: 0.01,
  },

  // ── Environmental (standard) ─────────────────────────────────────────────────
  'temperature':  { multiplier: 1.0,  stake_required: 50,  freq_per_hour: 4,    power_w: 0.05 },
  'humidity':     { multiplier: 1.0,  stake_required: 50,  freq_per_hour: 4,    power_w: 0.05 },
  'light':        { multiplier: 0.8,  stake_required: 50,  freq_per_hour: 6,    power_w: 0.05 },

  // ── RF / Scanning ───────────────────────────────────────────────────────────
  'rf-scanner':   { multiplier: 2.0,  stake_required: 100, freq_per_hour: 60,   power_w: 0.2 },

  // ── Motion / IMU (high frequency, low per-reading value) ────────────────────
  'accelerometer':       { multiplier: 0.5,  stake_required: 50,  freq_per_hour: 3600, power_w: 0.01 },
  'linear-acceleration': { multiplier: 0.5,  stake_required: 50,  freq_per_hour: 3600, power_w: 0.01 },
  'gravity':             { multiplier: 0.3,  stake_required: 25,  freq_per_hour: 3600, power_w: 0.005 },
  'gyroscope':           { multiplier: 0.5,  stake_required: 50,  freq_per_hour: 3600, power_w: 0.01 },
  'gyroscope-raw':       { multiplier: 0.5,  stake_required: 50,  freq_per_hour: 3600, power_w: 0.01 },
  'magnetometer':        { multiplier: 0.5,  stake_required: 50,  freq_per_hour: 3600, power_w: 0.01 },
  'magnetometer-raw':    { multiplier: 0.5,  stake_required: 50,  freq_per_hour: 3600, power_w: 0.01 },
  'orientation':         { multiplier: 0.4,  stake_required: 25,  freq_per_hour: 3600, power_w: 0.005 },
  'orientation-game':    { multiplier: 0.4,  stake_required: 25,  freq_per_hour: 3600, power_w: 0.005 },
  'orientation-geo':     { multiplier: 0.4,  stake_required: 25,  freq_per_hour: 3600, power_w: 0.005 },

  // ── Activity ─────────────────────────────────────────────────────────────────
  'motion':          { multiplier: 0.3,  stake_required: 25,  freq_per_hour: 1,    power_w: 0.005 },
  'stationary':      { multiplier: 0.2,  stake_required: 25,  freq_per_hour: 1,    power_w: 0.005 },
  'steps':           { multiplier: 0.3,  stake_required: 25,  freq_per_hour: 60,   power_w: 0.01 },
  'step-detector':   { multiplier: 0.3,  stake_required: 25,  freq_per_hour: 60,   power_w: 0.01 },
  'heart-rate':      { multiplier: 1.0,  stake_required: 50,  freq_per_hour: 60,   power_w: 0.05 },
  'proximity':       { multiplier: 0.3,  stake_required: 25,  freq_per_hour: 60,   power_w: 0.005 },
  'heading':         { multiplier: 0.5,  stake_required: 25,  freq_per_hour: 60,   power_w: 0.01 },
  'hinge-angle':     { multiplier: 0.3,  stake_required: 25,  freq_per_hour: 60,   power_w: 0.005 },

  // ── Infrastructure ───────────────────────────────────────────────────────────
  'power':    { multiplier: 1.2,  stake_required: 50,  freq_per_hour: 12,   power_w: 0.05 },
  'battery':  { multiplier: 0.5,  stake_required: 25,  freq_per_hour: 1,    power_w: 0.001 },

  // ── Fallback ──────────────────────────────────────────────────────────────────
  'custom':   { multiplier: 1.0,  stake_required: 100, freq_per_hour: 4,    power_w: 0.1 },
  '_default': { multiplier: 1.0,  stake_required: 100, freq_per_hour: 4,    power_w: 0.1 },
};

// Witness factors indexed by witness_count - 1, capped at index 2.
// 1 witness: 10%, 2 witnesses: 55%, 3+ witnesses: 100%.
const WITNESS_FACTORS = [0.10, 0.55, 1.00];

// Number of sensors per owner before a stake is required for new registrations.
const SYBIL_SENSOR_THRESHOLD = 3;

function _cfg(type) {
  return SENSOR_TYPE_CONFIG[type] || SENSOR_TYPE_CONFIG['_default'];
}

/**
 * Return the reward multiplier for a given sensor type and reading value.
 * For seismic, applies a large novelty_mult when value crosses the threshold.
 */
function getTypeMultiplier(type, value) {
  const cfg = _cfg(type);
  if (cfg.novelty_mult && cfg.novelty_threshold !== undefined) {
    if (Math.abs(Number(value) || 0) >= cfg.novelty_threshold) {
      return cfg.novelty_mult;
    }
  }
  return cfg.multiplier;
}

/**
 * Return the stake amount (in HONE) required per sensor when the owner
 * already has SYBIL_SENSOR_THRESHOLD or more sensors.
 */
function getStakeRequirement(type) {
  return _cfg(type).stake_required;
}

/**
 * Return the witness factor for a given number of independent witnesses.
 */
function getWitnessFactor(witnessCount) {
  const idx = Math.min(Math.max((witnessCount || 1) - 1, 0), 2);
  return WITNESS_FACTORS[idx];
}

/**
 * Compute the epoch reward for a single sensor reading.
 *
 * @param {number} baseRate    — base HONE per epoch per standard sensor (governance param)
 * @param {string} type        — sensor type
 * @param {number} value       — the reading value (needed for novelty check)
 * @param {number} witnessCount — number of independent witnesses for this epoch
 */
function computeEpochReward(baseRate, type, value, witnessCount) {
  const typeMult = getTypeMultiplier(type, value);
  const witnessFactor = getWitnessFactor(witnessCount);
  return parseFloat((baseRate * typeMult * witnessFactor).toFixed(10));
}

/**
 * Return the full config entry for a sensor type (for UI/docs).
 */
function getTypeConfig(type) {
  return Object.assign({}, _cfg(type));
}

/**
 * Return the full config table (for UI display).
 */
function getAllTypeConfigs() {
  const result = {};
  for (const [k, v] of Object.entries(SENSOR_TYPE_CONFIG)) {
    if (k !== '_default') result[k] = Object.assign({}, v);
  }
  return result;
}

module.exports = {
  SENSOR_TYPE_CONFIG,
  WITNESS_FACTORS,
  SYBIL_SENSOR_THRESHOLD,
  getTypeMultiplier,
  getStakeRequirement,
  getWitnessFactor,
  computeEpochReward,
  getTypeConfig,
  getAllTypeConfigs,
};
