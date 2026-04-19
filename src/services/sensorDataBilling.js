"use strict";

const crypto = require("crypto");

const DEFAULT_RATE_CARD = {
  sensor_types: {
    gps: 0.004,
    barometer: 0.0025,
    magnetometer: 0.0025,
    heart_rate: 0.0035,
    motion: 0.0018,
    orientation: 0.0018,
    light: 0.001,
    proximity: 0.001,
    steps: 0.001,
    battery: 0.0008,
    network: 0.0012,
    temperature: 0.0005,
    humidity: 0.0005,
    pressure: 0.0015,
    custom: 0.0015,
  },
  resolution_multipliers: {
    raw: 1.0,
    sample: 1.0,
    epoch: 0.65,
    aggregate: 0.55,
    summary: 0.35,
  },
  age_multipliers: [
    { max_hours: 1, multiplier: 1.0 },
    { max_hours: 6, multiplier: 1.1 },
    { max_hours: 24, multiplier: 1.25 },
    { max_hours: 168, multiplier: 1.5 },
    { max_hours: Infinity, multiplier: 1.8 },
  ],
  volume_discounts: [
    { min_readings: 5000, multiplier: 0.65 },
    { min_readings: 1000, multiplier: 0.75 },
    { min_readings: 250, multiplier: 0.85 },
    { min_readings: 50, multiplier: 0.95 },
    { min_readings: 0, multiplier: 1.0 },
  ],
  splits_bps: {
    sensor_operators: 7000,
    protocol: 2000,
    recycle: 1000,
  },
  protocol_accounts: [
    { account: "shindevlin", share_bps: 5000 },
    { account: "natoshisakamoto", share_bps: 5000 },
  ],
  minimum_fee: 0,
};

function roundAmount(value) {
  return Number.parseFloat(Number(value || 0).toFixed(10));
}

function cleanString(value, maxLen) {
  if (typeof value !== "string") return "";
  var trimmed = value.trim();
  if (!trimmed) return "";
  if (maxLen && trimmed.length > maxLen) return trimmed.slice(0, maxLen);
  return trimmed;
}

function normalizeType(type) {
  var clean = cleanString(type, 32).toLowerCase();
  if (!clean) return "custom";
  if (DEFAULT_RATE_CARD.sensor_types[clean] !== undefined) return clean;
  if (clean.indexOf("android-") === 0) return "custom";
  return clean;
}

function getRatePerReading(type, rateCard) {
  var card = rateCard || DEFAULT_RATE_CARD;
  var normalized = normalizeType(type);
  return card.sensor_types[normalized] !== undefined
    ? card.sensor_types[normalized]
    : card.sensor_types.custom;
}

function getResolutionMultiplier(resolution, rateCard) {
  var card = rateCard || DEFAULT_RATE_CARD;
  var clean = cleanString(resolution, 32).toLowerCase();
  if (!clean) return card.resolution_multipliers.raw;
  return card.resolution_multipliers[clean] !== undefined
    ? card.resolution_multipliers[clean]
    : card.resolution_multipliers.raw;
}

function getAgeMultiplier(ageHours, rateCard) {
  var card = rateCard || DEFAULT_RATE_CARD;
  var hours = Number(ageHours);
  if (!Number.isFinite(hours) || hours < 0) hours = 0;
  for (var i = 0; i < card.age_multipliers.length; i++) {
    if (hours <= card.age_multipliers[i].max_hours) {
      return card.age_multipliers[i].multiplier;
    }
  }
  return 1.0;
}

function getVolumeMultiplier(readingCount, rateCard) {
  var card = rateCard || DEFAULT_RATE_CARD;
  var count = Math.max(0, Number(readingCount) || 0);
  for (var i = 0; i < card.volume_discounts.length; i++) {
    if (count >= card.volume_discounts[i].min_readings) {
      return card.volume_discounts[i].multiplier;
    }
  }
  return 1.0;
}

function estimateReadingCount(query, context) {
  var q = query || {};
  var ctx = context || {};
  if (Number.isFinite(Number(ctx.estimated_readings))) {
    return Math.max(0, Number(ctx.estimated_readings));
  }
  if (Number.isFinite(Number(q.estimated_readings))) {
    return Math.max(0, Number(q.estimated_readings));
  }
  if (Number.isFinite(Number(q.reading_count))) {
    return Math.max(0, Number(q.reading_count));
  }
  if (Number.isFinite(Number(q.limit))) {
    return Math.max(0, Number(q.limit));
  }
  if (Array.isArray(q.readings)) {
    return q.readings.length;
  }
  return 0;
}

function normalizeQuery(query) {
  var q = query || {};
  return {
    type: normalizeType(q.type || q.sensor_type || q.sensorType),
    resolution: cleanString(q.resolution || q.mode, 32).toLowerCase() || "raw",
    age_hours: Number.isFinite(Number(q.age_hours))
      ? Number(q.age_hours)
      : (Number.isFinite(Number(q.ageHours)) ? Number(q.ageHours) : 0),
    estimated_readings: estimateReadingCount(q, q),
    enterprise: !!q.enterprise,
  };
}

function quoteSensorQuery(query, context) {
  var rateCard = (context && context.rate_card) || DEFAULT_RATE_CARD;
  var normalized = normalizeQuery(query);
  var estimatedReadings = estimateReadingCount(query, context);
  var pricePerReading = calculatePricePerReading({
    type: normalized.type,
    resolution: normalized.resolution,
    age_hours: normalized.age_hours,
    reading_count: estimatedReadings,
  }, { rate_card: rateCard, enterprise: normalized.enterprise, query: query, context: context });

  var maxCost = roundAmount(estimatedReadings * pricePerReading);
  var quoteSeed = JSON.stringify({
    query: normalized,
    rate: pricePerReading,
    max: maxCost,
    account: context && context.account ? context.account : null,
  });

  return {
    quote_id: "sdq_" + crypto.createHash("sha256").update(quoteSeed).digest("hex").slice(0, 32),
    estimated_readings: estimatedReadings,
    price_per_reading: pricePerReading,
    max_cost: maxCost,
    currency: "BTCPC",
    type: normalized.type,
    resolution: normalized.resolution,
    age_hours: normalized.age_hours,
    rate_card: {
      type_multiplier: getRatePerReading(normalized.type, rateCard),
      age_multiplier: getAgeMultiplier(normalized.age_hours, rateCard),
      resolution_multiplier: getResolutionMultiplier(normalized.resolution, rateCard),
      volume_multiplier: getVolumeMultiplier(estimatedReadings, rateCard),
    },
    enterprise: normalized.enterprise,
    expires_in_seconds: 60,
  };
}

function calculatePricePerReading(query, context) {
  var rateCard = (context && context.rate_card) || DEFAULT_RATE_CARD;
  var normalized = normalizeQuery(query);
  var base = getRatePerReading(normalized.type, rateCard);
  var ageMultiplier = getAgeMultiplier(normalized.age_hours, rateCard);
  var resolutionMultiplier = getResolutionMultiplier(normalized.resolution, rateCard);
  var volumeMultiplier = getVolumeMultiplier(normalized.estimated_readings, rateCard);
  var enterpriseMultiplier = context && context.enterprise_discount_bps
    ? Math.max(0, 1 - (Number(context.enterprise_discount_bps) / 10000))
    : 1;
  return roundAmount(base * ageMultiplier * resolutionMultiplier * volumeMultiplier * enterpriseMultiplier);
}

function blurCoordinates(reading) {
  if (!reading || typeof reading !== "object") return reading;
  var meta = reading.metadata;
  if (!meta || typeof meta !== "object") return reading;
  if (typeof meta.latitude !== "number" && typeof meta.longitude !== "number") return reading;
  return Object.assign({}, reading, {
    metadata: Object.assign({}, meta, {
      latitude: typeof meta.latitude === "number" ? Math.round(meta.latitude * 100) / 100 : meta.latitude,
      longitude: typeof meta.longitude === "number" ? Math.round(meta.longitude * 100) / 100 : meta.longitude,
    }),
  });
}

function extractOwner(reading) {
  if (!reading || typeof reading !== "object") return "unknown";
  if (typeof reading.owner === "string" && reading.owner.trim()) return reading.owner.trim();
  if (typeof reading.account === "string" && reading.account.trim()) return reading.account.trim();
  if (typeof reading.sensor_id === "string" && reading.sensor_id.includes("/")) {
    return reading.sensor_id.split("/")[0].trim() || "unknown";
  }
  return "unknown";
}

function calculatePayouts(readings, totalFee, options) {
  var fee = Math.max(0, Number(totalFee) || 0);
  var list = Array.isArray(readings) ? readings.filter(Boolean) : [];
  var rateCard = options && options.rate_card ? options.rate_card : DEFAULT_RATE_CARD;
  var splits = options && options.splits_bps ? options.splits_bps : DEFAULT_RATE_CARD.splits_bps;
  var protocolAccounts = options && options.protocol_accounts ? options.protocol_accounts : DEFAULT_RATE_CARD.protocol_accounts;

  var owners = new Map();
  for (var i = 0; i < list.length; i++) {
    var owner = extractOwner(list[i]);
    var current = owners.get(owner) || { owner: owner, readings: 0 };
    current.readings += 1;
    owners.set(owner, current);
  }

  var sensorPool = roundAmount(fee * ((splits.sensor_operators || 0) / 10000));
  var protocolPool = roundAmount(fee * ((splits.protocol || 0) / 10000));
  var recyclePool = roundAmount(fee * ((splits.recycle || 0) / 10000));

  var ownerEntries = Array.from(owners.values());
  var totalOwnerReadings = ownerEntries.reduce(function (sum, entry) { return sum + entry.readings; }, 0);

  var ownerPayouts = ownerEntries.map(function (entry, idx) {
    var amount = 0;
    if (totalOwnerReadings > 0 && sensorPool > 0) {
      amount = sensorPool * (entry.readings / totalOwnerReadings);
    }
    amount = roundAmount(amount);
    return {
      owner: entry.owner,
      readings: entry.readings,
      amount: amount,
      share_bps: totalOwnerReadings > 0 ? Math.round((entry.readings / totalOwnerReadings) * 10000) : 0,
      index: idx,
    };
  });

  var protocolPayouts = [];
  var totalProtocolBps = protocolAccounts.reduce(function (sum, item) { return sum + (item.share_bps || 0); }, 0) || 10000;
  for (var j = 0; j < protocolAccounts.length; j++) {
    var account = protocolAccounts[j];
    var pct = (account.share_bps || 0) / totalProtocolBps;
    protocolPayouts.push({
      account: account.account,
      amount: roundAmount(protocolPool * pct),
      kind: "protocol",
    });
  }

  var subtotal = ownerPayouts.reduce(function (sum, p) { return sum + p.amount; }, 0)
    + protocolPayouts.reduce(function (sum, p) { return sum + p.amount; }, 0)
    + recyclePool;

  var roundingDelta = roundAmount(fee - subtotal);
  if (roundingDelta !== 0) {
    recyclePool = roundAmount(recyclePool + roundingDelta);
  }

  return {
    total_fee: fee,
    rate_card: rateCard,
    sensor_pool: sensorPool,
    protocol_pool: protocolPool,
    recycle_pool: recyclePool,
    owner_payouts: ownerPayouts,
    protocol_payouts: protocolPayouts,
    total_readings: list.length,
    total_owner_readings: totalOwnerReadings,
    total_payout: roundAmount(
      ownerPayouts.reduce(function (sum, p) { return sum + p.amount; }, 0) +
      protocolPayouts.reduce(function (sum, p) { return sum + p.amount; }, 0) +
      recyclePool
    ),
  };
}

async function settleSensorDataPayment(input, options) {
  options = options || {};
  var ledger = options.ledger || require("./ledger");
  var escrow = options.escrow || require("./escrow");
  var stateStore = options.stateStore || require("../chain/stateStore");
  var payer = input && (input.payer || input.account || input.user);
  var query = input && (input.query || input);
  var quote = input && input.quote ? input.quote : quoteSensorQuery(query, options);
  var readings = Array.isArray(input && input.readings) ? input.readings : [];
  var actual = Math.max(0, Number(input && (input.actual_fee !== undefined ? input.actual_fee : input.total_fee)) || 0);
  var settledFee = Math.min(actual, quote.max_cost);
  var epoch = Number.isFinite(Number(options.epoch)) ? Number(options.epoch) : await ledger.getCurrentEpoch();
  var queryId = (input && (input.query_id || input.quote_id)) || quote.quote_id;
  var shouldUseEscrow = options.use_escrow !== false && escrow && typeof escrow.lockFunds === "function";

  if (!payer) {
    throw new Error("payer required");
  }

  if (shouldUseEscrow) {
    await escrow.lockFunds(queryId, payer, quote.max_cost);
  }

  if (readings.length === 0) {
    var fullRefundAmount = quote.max_cost;
    if (shouldUseEscrow) {
      await ledger.recordEscrowRefund(payer, queryId, fullRefundAmount, epoch);
    } else if (fullRefundAmount > 0) {
      await ledger.recordTransfer("btcpc_treasury", payer, fullRefundAmount, "BTCPC", null, epoch, "Sensor data refund — no data");
    }
    return {
      query_id: queryId,
      payer: payer,
      quote: quote,
      total_fee: 0,
      charged: 0,
      refunded: true,
      refund: fullRefundAmount,
      transfers: [],
      payouts: calculatePayouts([], 0, options),
      sensor_count: 0,
      block_height: stateStore && typeof stateStore.getChainHeight === "function" ? stateStore.getChainHeight() : null,
    };
  }

  var payouts = calculatePayouts(readings, settledFee, options);
  var transfers = [];

  if (settledFee > 0) {
    for (var i = 0; i < payouts.owner_payouts.length; i++) {
      var ownerPayout = payouts.owner_payouts[i];
      if (ownerPayout.amount > 0) {
        await ledger.recordEscrowRelease(ownerPayout.owner, queryId, ownerPayout.amount, epoch, "Sensor data payout");
        transfers.push({ to: ownerPayout.owner, amount: ownerPayout.amount, kind: "sensor_owner" });
      }
    }
    for (var j = 0; j < payouts.protocol_payouts.length; j++) {
      var protocolPayout = payouts.protocol_payouts[j];
      if (protocolPayout.amount > 0) {
        await ledger.recordEscrowRelease(protocolPayout.account, queryId, protocolPayout.amount, epoch, "Sensor data protocol share");
        transfers.push({ to: protocolPayout.account, amount: protocolPayout.amount, kind: "protocol" });
      }
    }
    if (payouts.recycle_pool > 0) {
      await ledger.recordEscrowRelease("btcpc_recycle", queryId, payouts.recycle_pool, epoch, "Sensor data recycle share");
      transfers.push({ to: "btcpc_recycle", amount: payouts.recycle_pool, kind: "recycle" });
    }
  }

  var refund = roundAmount(quote.max_cost - settledFee);
  if (refund > 0) {
    if (shouldUseEscrow) {
      await ledger.recordEscrowRefund(payer, queryId, refund, epoch);
    } else {
      await ledger.recordTransfer("btcpc_treasury", payer, refund, "BTCPC", null, epoch, "Sensor data refund");
    }
  }

  return {
    query_id: queryId,
    payer: payer,
    quote: quote,
    total_fee: settledFee,
    refund: refund,
    transfers: transfers,
    payouts: payouts,
    sensor_count: readings.length,
    block_height: stateStore && typeof stateStore.getChainHeight === "function" ? stateStore.getChainHeight() : null,
  };
}

function getRateCard() {
  return DEFAULT_RATE_CARD;
}

/**
 * Execute a paid sensor data query end-to-end:
 *   1. Resolve matching sensors from stateStore
 *   2. Collect readings in [from_epoch, to_epoch]
 *   3. Quote cost based on actual readings
 *   4. Validate payer balance
 *   5. Settle payment (escrow lock → distribute → refund overage)
 *   6. Optionally run AI analysis on the data via Ollama
 *   7. Return readings + payment receipt
 *
 * Query body params:
 *   sensor_id    — exact sensor (overrides owner/type/region)
 *   owner        — filter by account name
 *   type         — sensor type (gps, temperature, …)
 *   region       — filter by region tag
 *   from_epoch   — start of range (default: last 100 epochs)
 *   to_epoch     — end of range (default: current height)
 *   limit        — max readings returned (default 1000, max 10000)
 *   resolution   — raw | epoch | aggregate (default raw)
 *   analyze      — true to run AI analysis on returned data
 *   analyze_prompt — custom analysis question (optional)
 *   analyze_model  — which model to use (default qwen3:4b)
 */
async function executePaidSensorQuery(queryBody, options) {
  options = options || {};
  var stateStore = options.stateStore || require("../chain/stateStore");
  var ledger = options.ledger || require("./ledger");
  var account = options.account;
  if (!account) throw new Error("account required");

  var body = queryBody || {};
  var sensorId = typeof body.sensor_id === "string" ? body.sensor_id.trim() : null;
  var owner = typeof body.owner === "string" ? body.owner.trim() : null;
  var typeFilter = typeof body.type === "string" ? body.type.trim().toLowerCase() : null;
  var region = typeof body.region === "string" ? body.region.trim() : null;
  var limit = Math.min(Math.max(1, Number(body.limit) || 1000), 10000);
  var resolution = typeof body.resolution === "string" ? body.resolution.trim().toLowerCase() : "raw";
  var withAnalysis = !!body.analyze;
  var analysisPrompt = typeof body.analyze_prompt === "string" ? body.analyze_prompt.slice(0, 500) : null;
  var analysisModel = typeof body.analyze_model === "string" ? body.analyze_model.trim() : "qwen3:4b";

  var currentEpoch = stateStore.getChainHeight() || 1;
  var toEpoch = Number.isFinite(Number(body.to_epoch)) ? Number(body.to_epoch) : currentEpoch;
  var fromEpoch = Number.isFinite(Number(body.from_epoch)) ? Number(body.from_epoch) : Math.max(1, toEpoch - 100);
  if (fromEpoch > toEpoch) fromEpoch = toEpoch;

  // Find matching sensors
  var matchingSensors;
  if (sensorId) {
    var single = stateStore.getSensor(sensorId);
    matchingSensors = single ? [single] : [];
  } else {
    var sFilter = { status: "active" };
    if (owner) sFilter.owner = owner;
    if (typeFilter) sFilter.type = typeFilter;
    if (region) sFilter.region = region;
    matchingSensors = stateStore.getAllSensors(sFilter);
  }

  if (matchingSensors.length === 0) {
    throw new Error("No active sensors match the query filters");
  }

  // Collect readings across epoch range
  var allReadings = [];
  var effectiveType = typeFilter || (matchingSensors[0] && matchingSensors[0].type) || "custom";

  for (var si = 0; si < matchingSensors.length && allReadings.length < limit; si++) {
    var sensor = matchingSensors[si];
    var sid = sensor.sensor_id;
    var rawReadings = stateStore.getSensorReadings(sid, fromEpoch, toEpoch, limit - allReadings.length);
    if (!rawReadings || rawReadings.length === 0) continue;

    if (resolution === "epoch" || resolution === "aggregate") {
      // One value per epoch: median of readings in that epoch
      var byEpoch = {};
      for (var ri = 0; ri < rawReadings.length; ri++) {
        var ep = rawReadings[ri].epoch;
        if (!byEpoch[ep]) byEpoch[ep] = [];
        byEpoch[ep].push(rawReadings[ri].value);
      }
      var epochKeys = Object.keys(byEpoch).map(Number).sort(function (a, b) { return a - b; });
      for (var ei = 0; ei < epochKeys.length && allReadings.length < limit; ei++) {
        var epKey = epochKeys[ei];
        var vals = byEpoch[epKey].filter(function (v) { return Number.isFinite(Number(v)); }).sort(function (a, b) { return a - b; });
        if (vals.length === 0) continue;
        allReadings.push({
          sensor_id: sid, owner: sensor.owner, type: sensor.type,
          epoch: epKey, value: vals[Math.floor(vals.length / 2)],
          reading_count: vals.length, resolution: resolution,
          unit: sensor.unit || null, region: sensor.region || null,
        });
      }
    } else {
      for (var rj = 0; rj < rawReadings.length && allReadings.length < limit; rj++) {
        allReadings.push({
          sensor_id: sid, owner: sensor.owner, type: sensor.type,
          epoch: rawReadings[rj].epoch, value: rawReadings[rj].value,
          metadata: rawReadings[rj].metadata, submitted_at: rawReadings[rj].submitted_at,
          resolution: "raw", unit: sensor.unit || null, region: sensor.region || null,
        });
      }
    }
  }

  if (allReadings.length === 0) {
    return {
      query: {
        sensor_id: sensorId, owner: owner, type: typeFilter, region: region,
        from_epoch: fromEpoch, to_epoch: toEpoch, resolution: resolution,
        sensors_matched: matchingSensors.length,
      },
      readings: [],
      reading_count: 0,
      charged: 0,
      refunded: true,
      payment: null,
      analysis: null,
    };
  }

  // Build a lookup of sensor allow_precise_location flags
  var preciseAllowed = {};
  for (var pi = 0; pi < matchingSensors.length; pi++) {
    var ps = matchingSensors[pi];
    preciseAllowed[ps.sensor_id] = ps.allow_precise_location === true;
  }

  // Blur GPS coordinates for sensors that have not opted in to precise location sharing
  allReadings = allReadings.map(function (r) {
    return preciseAllowed[r.sensor_id] ? r : blurCoordinates(r);
  });

  // Quote based on actual readings returned
  var ageHours = ((currentEpoch - fromEpoch) * 30) / 3600;
  var quoteQuery = { type: effectiveType, resolution: resolution, age_hours: ageHours, estimated_readings: allReadings.length };
  var quote = quoteSensorQuery(quoteQuery, { account: account });

  // Check balance
  var balance = stateStore.getBalance(account, "BTCPC");
  if ((balance || 0) < quote.max_cost && quote.max_cost > 0) {
    throw new Error("Insufficient balance: need " + quote.max_cost.toFixed(10) + " BTCPC, have " + (balance || 0).toFixed(10));
  }

  // Settle payment
  var currentEpochNum = await ledger.getCurrentEpoch();
  var receipt = await settleSensorDataPayment({
    payer: account,
    query: quoteQuery,
    quote: quote,
    readings: allReadings,
    actual_fee: quote.max_cost,
  }, { ledger: ledger, stateStore: stateStore, epoch: currentEpochNum });

  var queryResult = {
    query: {
      sensor_id: sensorId, owner: owner, type: typeFilter, region: region,
      from_epoch: fromEpoch, to_epoch: toEpoch, resolution: resolution,
      sensors_matched: matchingSensors.length,
    },
    readings: allReadings,
    reading_count: allReadings.length,
    payment: {
      query_id: receipt.query_id,
      total_fee: receipt.total_fee,
      currency: "BTCPC",
      per_sensor: receipt.payouts.owner_payouts,
      protocol: receipt.payouts.protocol_payouts,
    },
    analysis: null,
  };

  // Optional AI analysis — format data as a compact summary for the model
  if (withAnalysis && allReadings.length > 0) {
    try {
      var axios = require("axios");
      var ollamaUrl = process.env.OLLAMA_URL || "http://localhost:11434";
      var prompt = analysisPrompt ||
        "Analyze the following sensor data. Identify any anomalies, trends, or notable patterns. Be concise.";
      // Build a compact data summary (max 200 readings in the prompt to keep tokens reasonable)
      var sampleReadings = allReadings.slice(0, 200);
      var dataSummary = sampleReadings.map(function (r) {
        return "epoch=" + r.epoch + " sensor=" + r.sensor_id + " value=" + r.value + (r.unit ? r.unit : "");
      }).join("\n");
      var analysisMessages = [
        { role: "system", content: "You are a sensor data analyst. Respond in plain text only." },
        { role: "user", content: prompt + "\n\nData (" + allReadings.length + " readings, showing first " + sampleReadings.length + "):\n" + dataSummary },
      ];
      var ollamaResp = await axios.post(ollamaUrl + "/api/chat", {
        model: analysisModel, messages: analysisMessages, stream: false,
      }, { timeout: 60000 });
      queryResult.analysis = {
        model: analysisModel,
        content: ollamaResp.data && ollamaResp.data.message && ollamaResp.data.message.content
          ? ollamaResp.data.message.content
          : null,
      };
    } catch (analysisErr) {
      queryResult.analysis = { error: analysisErr.message };
    }
  }

  return queryResult;
}

module.exports = {
  DEFAULT_RATE_CARD,
  getRateCard,
  quoteSensorQuery,
  calculatePricePerReading,
  calculatePayouts,
  settleSensorDataPayment,
  executePaidSensorQuery,
  normalizeQuery,
  extractOwner,
  estimateReadingCount,
  blurCoordinates,
};
