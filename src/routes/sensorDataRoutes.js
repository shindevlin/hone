"use strict";

const express = require("express");
const { authenticateToken } = require("../middlewares/auth");

let billingLoader = function billingLoader() {
  try {
    return require("../services/sensorDataBilling");
  } catch (_) {
    return null;
  }
};

function loadBilling() {
  return billingLoader();
}

function requireBilling(res) {
  const billing = loadBilling();
  if (!billing) {
    res.status(503).json({
      error: {
        message: "Sensor data billing service is not wired yet.",
        type: "service_unavailable",
        code: "sensor_billing_unavailable",
      },
    });
    return null;
  }
  return billing;
}

function normalizeQueryBody(body) {
  return body && typeof body === "object" ? body : {};
}

const router = express.Router();

router.get("/rate-card", (req, res) => {
  const billing = requireBilling(res);
  if (!billing) return;

  const rateCard = typeof billing.getRateCard === "function"
    ? billing.getRateCard()
    : null;

  if (!rateCard) {
    return res.status(503).json({
      error: {
        message: "Rate card lookup is not available yet.",
        type: "service_unavailable",
        code: "rate_card_unavailable",
      },
    });
  }

  return res.json({ success: true, rate_card: rateCard });
});

router.post("/quote", authenticateToken, async (req, res) => {
  const billing = requireBilling(res);
  if (!billing) return;

  const body = normalizeQueryBody(req.body);
  if (typeof billing.quoteSensorQuery !== "function") {
    return res.status(503).json({
      error: {
        message: "Sensor data quote calculation is not available yet.",
        type: "service_unavailable",
        code: "quote_unavailable",
      },
    });
  }

  try {
    const account = (req.user && req.user.username) || body.account || body.payer || null;
    const quote = await billing.quoteSensorQuery(body, { account, user: req.user || null, req });
    return res.json({ success: true, quote });
  } catch (err) {
    return res.status(422).json({
      error: {
        message: err.message,
        type: "billing_error",
        code: "quote_failed",
      },
    });
  }
});

router.post("/query", authenticateToken, async (req, res) => {
  const billing = requireBilling(res);
  if (!billing) return;

  const body = normalizeQueryBody(req.body);
  if (typeof billing.executePaidSensorQuery !== "function") {
    return res.status(503).json({
      error: {
        message: "Sensor data query execution is not available yet.",
        type: "service_unavailable",
        code: "query_unavailable",
      },
    });
  }

  try {
    const account = (req.user && req.user.username) || body.account || body.payer || null;

    // Free access for stakers — staking on a device grants free data queries
    const sensorId = body.sensor_id || body.device_id || body.sensor || null;
    if (sensorId && account) {
      let _ss = null;
      try { _ss = require("../chain/stateStore"); } catch (_) {}
      if (_ss && typeof _ss.isStakerOnDevice === "function" && _ss.isStakerOnDevice(account, sensorId)) {
        // Run the query without billing
        if (typeof billing.executeSensorQuery === "function") {
          const result = await billing.executeSensorQuery(body);
          return res.json({ success: true, result, free_access: true, reason: "staker" });
        }
        // If executeSensorQuery not available, fall through to paid path
      }
    }

    const result = await billing.executePaidSensorQuery(body, { account, user: req.user || null, req });
    return res.json({ success: true, result });
  } catch (err) {
    return res.status(422).json({
      error: {
        message: err.message,
        type: "billing_error",
        code: "query_failed",
      },
    });
  }
});

// ── GET /api/sensor-data/coverage ────────────────────────────────────────────
// Returns geographic coverage: how many active sensors per grid cell / region.
// Public endpoint for B2B buyers to assess network density before purchasing.
router.get("/coverage", (req, res) => {
  let stateStore;
  try { stateStore = require("../chain/stateStore"); } catch (_) {}

  if (!stateStore || typeof stateStore.getAllSensors !== "function") {
    return res.status(503).json({ error: "Sensor registry unavailable" });
  }

  const sensors = stateStore.getAllSensors ? stateStore.getAllSensors() : [];
  const gridPrecision = parseInt(req.query.precision) || 2; // decimal degrees, default ~11km cell

  const cells = {};
  let activeCount = 0;

  for (const sensor of sensors) {
    if (!sensor.lat || !sensor.lon) continue;
    const lat = parseFloat(sensor.lat.toFixed(gridPrecision));
    const lon = parseFloat(sensor.lon.toFixed(gridPrecision));
    const key = `${lat},${lon}`;
    if (!cells[key]) {
      cells[key] = { lat, lon, count: 0, types: new Set() };
    }
    cells[key].count++;
    if (sensor.sensor_types) {
      sensor.sensor_types.forEach(t => cells[key].types.add(t));
    }
    activeCount++;
  }

  const grid = Object.values(cells).map(c => ({
    lat: c.lat,
    lon: c.lon,
    sensor_count: c.count,
    data_types: Array.from(c.types),
  }));

  res.json({
    active_sensors: activeCount,
    grid_cells: grid.length,
    grid_precision_degrees: Math.pow(10, -gridPrecision),
    grid,
  });
});

// ── GET /api/sensor-data/availability ────────────────────────────────────────
// Returns what sensor data types are available right now and at what density.
// Used by B2B buyers to scope API subscriptions.
router.get("/availability", (req, res) => {
  let stateStore;
  try { stateStore = require("../chain/stateStore"); } catch (_) {}

  if (!stateStore || typeof stateStore.getAllSensors !== "function") {
    return res.status(503).json({ error: "Sensor registry unavailable" });
  }

  const billing = loadBilling();
  const rateCard = billing && typeof billing.getRateCard === "function"
    ? billing.getRateCard()
    : null;

  const sensors = stateStore.getAllSensors ? stateStore.getAllSensors() : [];
  const typeCounts = {};

  for (const sensor of sensors) {
    if (!sensor.sensor_types) continue;
    for (const t of sensor.sensor_types) {
      typeCounts[t] = (typeCounts[t] || 0) + 1;
    }
  }

  const availability = Object.entries(typeCounts).map(([type, count]) => ({
    type,
    sensor_count: count,
    price_per_reading_hone: rateCard && rateCard.price_per_reading
      ? rateCard.price_per_reading[type] || rateCard.price_per_reading.default
      : null,
  }));

  res.json({
    total_active_sensors: sensors.length,
    data_types: availability,
    rate_card_available: !!rateCard,
  });
});

function __setBillingLoader(loader) {
  billingLoader = typeof loader === "function" ? loader : billingLoader;
}

module.exports = { router, loadBilling, __setBillingLoader };
