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

function __setBillingLoader(loader) {
  billingLoader = typeof loader === "function" ? loader : billingLoader;
}

module.exports = { router, loadBilling, __setBillingLoader };
