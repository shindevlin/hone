"use strict";

/**
 * BTCPC Sensor Rewards
 * Shin Devlin
 *
 * Sensor rewards are NOT epoch-based. They trigger when data is purchased,
 * which may happen many epochs — days or weeks — after the original reading.
 *
 * Flow:
 *   1. Sensor submits SENSOR_READING (any epoch) → recorded on-chain, amount=0
 *      The reading is an immutable fact. No reward yet.
 *
 *   2. Buyer queries sensor data (any epoch, any time later)
 *      Buyer pays BTCPC for the data → purchase recorded on-chain
 *
 *   3. Purchase triggers reward distribution:
 *      - Look up original reading's sensor_id → find owner account
 *      - Split payment: SENSOR_CUT to sensor owner, GATEWAY_CUT to gateway (if any),
 *        RECYCLE_CUT to btcpc_recycle as protocol fee
 *      - Emit SENSOR_REWARD ledger entries for this purchase epoch
 *
 * The original reading epoch is irrelevant to reward timing.
 * A reading from epoch 100 can be purchased and rewarded at epoch 50,000.
 *
 * Reward splits (configurable):
 *   Sensor owner:  80%
 *   Gateway:       10%  (if routed through a gateway node)
 *   Protocol fee:  10%  → btcpc_recycle (never burned)
 */

const EventEmitter = require("events");

const SENSOR_CUT    = parseFloat(process.env.BTCPC_SENSOR_CUT)    || 0.80;
const GATEWAY_CUT   = parseFloat(process.env.BTCPC_GATEWAY_CUT)   || 0.10;
const RECYCLE_CUT   = 1 - SENSOR_CUT - GATEWAY_CUT;                        // remainder
const RECYCLE_ACCOUNT = "btcpc_recycle";

const emitter = new EventEmitter();

// In-memory index of known sensor readings: sensor_id → { owner, reading_epoch, data_hash, metadata }
// Populated from on-chain replay and live SENSOR_READING entries.
// Map<data_hash, SensorRecord>
const sensorIndex = new Map();

// Purchase history to prevent double-reward on same data_hash
// Map<purchase_id, { data_hash, buyer, purchase_epoch, rewarded }>
const purchaseHistory = new Map();

// ── Sensor reading registration ───────────────────────────────────────────────

/**
 * Index a SENSOR_READING ledger entry.
 * Called during block replay and live block processing.
 * No payment — just records that this data exists and who owns it.
 */
function indexReading(entry) {
  if (!entry || entry.type !== "SENSOR_READING") return;

  const sensorData = entry.sensor_data || {};
  const sensorId = sensorData.sensor_id || entry.from;
  const dataHash = (sensorData.metadata && sensorData.metadata.sha256) ||
    sensorData.data_hash || null;

  if (!sensorId || !dataHash) return;

  // Owner = account portion of sensor_id (format: "account/sensor-name")
  const owner = sensorId.includes("/") ? sensorId.split("/")[0] : (entry.from || sensorId);

  sensorIndex.set(dataHash, {
    sensor_id: sensorId,
    owner,
    reading_epoch: entry.epoch || 0,
    data_hash: dataHash,
    metadata: sensorData.metadata || {},
    indexed_at: Date.now(),
  });
}

/**
 * Index multiple entries (batch replay).
 */
function indexEntries(entries) {
  for (const e of (entries || [])) indexReading(e);
}

// ── Purchase processing ───────────────────────────────────────────────────────

/**
 * Process a sensor data purchase and compute reward distribution.
 *
 * @param {object} purchase
 *   data_hash     — SHA256 of the purchased data
 *   buyer         — account name of buyer
 *   payment_btcpc — amount buyer is paying in BTCPC
 *   purchase_epoch — current epoch number
 *   gateway       — optional gateway account that routed the purchase
 *   purchase_id   — unique ID for this purchase (prevents double reward)
 *
 * @returns {{ rewards: LedgerEntry[], reason: string } | null}
 */
function processPurchase(purchase) {
  const { data_hash, buyer, payment_btcpc, purchase_epoch, gateway, purchase_id } = purchase;

  if (!data_hash || !buyer || !payment_btcpc || payment_btcpc <= 0) {
    return { rewards: [], reason: "invalid_purchase_params" };
  }

  // Prevent double-reward
  const pid = purchase_id || (data_hash + ":" + buyer + ":" + purchase_epoch);
  if (purchaseHistory.has(pid)) {
    return { rewards: [], reason: "already_rewarded" };
  }

  // Look up the sensor reading
  const record = sensorIndex.get(data_hash);
  if (!record) {
    // Data not indexed — may have been submitted before this node synced.
    // Log and return a partial reward to btcpc_recycle for now.
    console.warn("[BTCPC SensorRewards] Data hash " + data_hash.slice(0, 16) +
      " not found in sensor index — routing full payment to recycle");
    purchaseHistory.set(pid, { data_hash, buyer, purchase_epoch, rewarded: false });
    return {
      rewards: [_recycleEntry(payment_btcpc, purchase_epoch, "sensor_owner_unknown")],
      reason: "sensor_not_indexed",
    };
  }

  // Compute reward split
  const sensorReward  = round(payment_btcpc * SENSOR_CUT);
  const gatewayReward = gateway ? round(payment_btcpc * GATEWAY_CUT) : 0;
  const recycleAmount = round(payment_btcpc - sensorReward - gatewayReward);

  const rewards = [];

  // Sensor owner reward
  rewards.push({
    type: "SENSOR_REWARD",
    from: buyer,
    to: record.owner,
    token: "BTCPC",
    amount: sensorReward,
    epoch: purchase_epoch,
    memo: "sensor_data_purchase",
    sensor_id: record.sensor_id,
    data_hash,
    original_reading_epoch: record.reading_epoch,
    purchase_id: pid,
    timestamp: Date.now(),
  });

  // Gateway reward (if routed through a gateway)
  if (gateway && gatewayReward > 0) {
    rewards.push({
      type: "GATEWAY_REWARD",
      from: buyer,
      to: gateway,
      token: "BTCPC",
      amount: gatewayReward,
      epoch: purchase_epoch,
      memo: "sensor_gateway_fee",
      data_hash,
      purchase_id: pid,
      timestamp: Date.now(),
    });
  }

  // Protocol fee → recycle
  if (recycleAmount > 0) {
    rewards.push(_recycleEntry(recycleAmount, purchase_epoch, "sensor_protocol_fee"));
  }

  // Mark purchased
  purchaseHistory.set(pid, {
    data_hash,
    buyer,
    purchase_epoch,
    rewarded: true,
    sensor_owner: record.owner,
    payment: payment_btcpc,
    original_reading_epoch: record.reading_epoch,
    epochs_between: purchase_epoch - record.reading_epoch,
  });

  console.log("[BTCPC SensorRewards] Purchase processed:" +
    " data=" + data_hash.slice(0, 12) + "..." +
    " | buyer=" + buyer +
    " | owner=" + record.owner +
    " | payment=" + payment_btcpc + " BTCPC" +
    " | sensor_reward=" + sensorReward +
    (gateway ? " | gateway=" + gateway + ":" + gatewayReward : "") +
    " | reading was epoch " + record.reading_epoch + " (now epoch " + purchase_epoch + ")" +
    " | " + (purchase_epoch - record.reading_epoch) + " epochs ago");

  emitter.emit("purchase_rewarded", {
    purchase_id: pid,
    data_hash,
    buyer,
    sensor_owner: record.owner,
    gateway,
    payment_btcpc,
    rewards,
    original_reading_epoch: record.reading_epoch,
    purchase_epoch,
  });

  return { rewards, reason: "ok" };
}

// ── Query: what data is available for purchase ────────────────────────────────

/**
 * List available sensor readings, optionally filtered by sensor_id prefix or epoch range.
 */
function listAvailableReadings({ sensor_id_prefix, from_epoch, to_epoch, limit } = {}) {
  let results = Array.from(sensorIndex.values());

  if (sensor_id_prefix) {
    results = results.filter(r => r.sensor_id.startsWith(sensor_id_prefix));
  }
  if (typeof from_epoch === "number") {
    results = results.filter(r => r.reading_epoch >= from_epoch);
  }
  if (typeof to_epoch === "number") {
    results = results.filter(r => r.reading_epoch <= to_epoch);
  }

  results.sort((a, b) => b.reading_epoch - a.reading_epoch);
  if (limit) results = results.slice(0, limit);

  return results.map(r => ({
    sensor_id: r.sensor_id,
    owner: r.owner,
    data_hash: r.data_hash,
    reading_epoch: r.reading_epoch,
    metadata: r.metadata,
    already_purchased: purchaseHistory.has(r.data_hash + ":*"), // approximate
  }));
}

/**
 * Look up a specific sensor reading by data hash.
 */
function getReading(dataHash) {
  return sensorIndex.get(dataHash) || null;
}

/**
 * Get purchase history for a sensor owner.
 */
function getPurchaseHistory(ownerAccount) {
  return Array.from(purchaseHistory.values())
    .filter(p => {
      const record = sensorIndex.get(p.data_hash);
      return record && record.owner === ownerAccount;
    })
    .sort((a, b) => b.purchase_epoch - a.purchase_epoch);
}

// ── Stats ─────────────────────────────────────────────────────────────────────

function getStats() {
  const totalReadings = sensorIndex.size;
  const totalPurchases = purchaseHistory.size;
  const rewarded = Array.from(purchaseHistory.values()).filter(p => p.rewarded).length;
  const totalRevenue = Array.from(purchaseHistory.values())
    .filter(p => p.rewarded)
    .reduce((s, p) => s + (p.payment || 0), 0);

  // Unique sensors
  const sensors = new Set(Array.from(sensorIndex.values()).map(r => r.sensor_id));
  const owners = new Set(Array.from(sensorIndex.values()).map(r => r.owner));

  return {
    indexed_readings: totalReadings,
    unique_sensors: sensors.size,
    unique_owners: owners.size,
    total_purchases: totalPurchases,
    rewarded_purchases: rewarded,
    total_revenue_btcpc: round(totalRevenue),
    sensor_cut_pct: SENSOR_CUT * 100,
    gateway_cut_pct: GATEWAY_CUT * 100,
    recycle_cut_pct: RECYCLE_CUT * 100,
  };
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function round(n) { return parseFloat(Number(n).toFixed(10)); }

function _recycleEntry(amount, epoch, memo) {
  return {
    type: "RECYCLE",
    from: null,
    to: RECYCLE_ACCOUNT,
    token: "BTCPC",
    amount: round(amount),
    epoch,
    memo,
    timestamp: Date.now(),
  };
}

function on(event, fn) { emitter.on(event, fn); }

module.exports = {
  indexReading,
  indexEntries,
  processPurchase,
  listAvailableReadings,
  getReading,
  getPurchaseHistory,
  getStats,
  on,
  SENSOR_CUT,
  GATEWAY_CUT,
  RECYCLE_CUT,
};
