#!/usr/bin/env node
/**
 * BTCPC Nebra Gateway Daemon
 *
 * Runs on the Nebra Hotspot's Raspberry Pi. Registers the Nebra as a BTCPC
 * LoRa gateway and relays sensor readings to the chain. Supports two
 * inbound paths — choose one or both:
 *
 *   BLE mode  (default): Scans for Flipper Zero BLE advertisements.
 *                        Requires: npm install @abandonware/noble
 *                        On RPi: sudo setcap cap_net_raw+eip $(which node)
 *
 *   HTTP mode           : Local HTTP endpoint — any device on LAN can POST
 *                        sensor data (e.g. flipper-relay.js with RELAY_URL).
 *                        Always enabled; BLE is additive.
 *
 * Usage:
 *   BTCPC_JWT=<token> node scripts/nebra-gateway.js
 *   BTCPC_JWT=<token> BTCPC_API=http://localhost:4242 GATEWAY_OWNER=natoshi node scripts/nebra-gateway.js
 *
 * Environment:
 *   BTCPC_JWT          — JWT for chain auth (required)
 *   BTCPC_API          — chain API base (default: https://btcpc.net)
 *   GATEWAY_OWNER      — BTCPC account that owns this gateway (default: natoshi)
 *   GATEWAY_NAME       — gateway slug (default: nebra-indoor)
 *   GATEWAY_REGION     — region tag (default: us-west)
 *   GATEWAY_LAT        — latitude (optional)
 *   GATEWAY_LON        — longitude (optional)
 *   SENSOR_OWNER       — BTCPC account that owns Flipper sensor (default: josh)
 *   SENSOR_NAME        — sensor slug (default: flipper-zero)
 *   RELAY_PORT         — local HTTP relay port (default: 7979)
 *   BLE_ENABLED        — set to "false" to skip BLE scan (default: true)
 *   HEARTBEAT_SEC      — heartbeat interval seconds (default: 30)
 */

const https = require('https');
const http  = require('http');

const API_BASE        = process.env.BTCPC_API       || 'https://btcpc.net';
const JWT             = process.env.BTCPC_JWT;
const GATEWAY_OWNER   = process.env.GATEWAY_OWNER   || 'natoshi';
const GATEWAY_NAME    = process.env.GATEWAY_NAME    || 'nebra-indoor';
const GATEWAY_REGION  = process.env.GATEWAY_REGION  || 'us-west';
const GATEWAY_LAT     = process.env.GATEWAY_LAT     ? parseFloat(process.env.GATEWAY_LAT) : null;
const GATEWAY_LON     = process.env.GATEWAY_LON     ? parseFloat(process.env.GATEWAY_LON) : null;
const SENSOR_OWNER    = process.env.SENSOR_OWNER    || 'josh';
const SENSOR_NAME     = process.env.SENSOR_NAME     || 'flipper-zero';
const RELAY_PORT      = parseInt(process.env.RELAY_PORT || '7979');
const BLE_ENABLED     = process.env.BLE_ENABLED !== 'false';
const HEARTBEAT_SEC   = parseInt(process.env.HEARTBEAT_SEC || '30');

if (!JWT) {
  console.error('[nebra] BTCPC_JWT env var required.');
  process.exit(1);
}

const GATEWAY_ID = GATEWAY_OWNER + '/' + GATEWAY_NAME;
const SENSOR_ID  = SENSOR_OWNER  + '/' + SENSOR_NAME;

let packetsRelayed = 0;

// ── HTTP helpers ──────────────────────────────────────────────────────────────

function apiPost(path, body) {
  return new Promise((resolve, reject) => {
    const raw = JSON.stringify(body);
    const urlObj = new URL(API_BASE + path);
    const mod = urlObj.protocol === 'https:' ? https : http;
    const opts = {
      hostname: urlObj.hostname,
      port: urlObj.port || (urlObj.protocol === 'https:' ? 443 : 80),
      path: urlObj.pathname,
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(raw),
        'Authorization': 'Bearer ' + JWT,
      },
    };
    const req = mod.request(opts, (res) => {
      const chunks = [];
      res.on('data', c => chunks.push(c));
      res.on('end', () => {
        try { resolve({ status: res.statusCode, body: JSON.parse(Buffer.concat(chunks).toString()) }); }
        catch (_) { resolve({ status: res.statusCode, body: {} }); }
      });
    });
    req.on('error', reject);
    req.end(raw);
  });
}

// ── Gateway registration ──────────────────────────────────────────────────────

async function ensureGateway() {
  const spec = {
    name: GATEWAY_NAME,
    region: GATEWAY_REGION,
    latitude: GATEWAY_LAT || 0,
    longitude: GATEWAY_LON || 0,
    hardware_model: 'nebra-indoor-gen1',
    antenna_gain_dbi: 3,
  };

  try {
    const r = await apiPost('/api/gateways', spec);
    if (r.status === 201) {
      console.log(`[nebra] Gateway registered: ${GATEWAY_ID}`);
    } else if (r.status === 409) {
      console.log(`[nebra] Gateway already registered: ${GATEWAY_ID}`);
    } else {
      console.warn('[nebra] Gateway register:', r.status, JSON.stringify(r.body).slice(0, 120));
    }
  } catch (err) {
    console.error('[nebra] Gateway register error:', err.message);
  }
}

// ── Sensor registration ───────────────────────────────────────────────────────

async function ensureSensor() {
  const spec = {
    name: SENSOR_NAME,
    type: 'rf-scanner',
    unit: 'dbm',
    decimals: 1,
    region: GATEWAY_REGION,
    lora_gateway: GATEWAY_ID,
    hardware_model: 'flipper-zero',
    firmware_version: 'btcpc-app',
  };

  try {
    const r = await apiPost('/api/sensors', spec);
    if (r.status === 201) {
      console.log(`[nebra] Sensor registered: ${SENSOR_ID}`);
    } else if (r.status === 409) {
      console.log(`[nebra] Sensor already registered: ${SENSOR_ID}`);
    } else {
      console.warn('[nebra] Sensor register:', r.status, JSON.stringify(r.body).slice(0, 120));
    }
  } catch (err) {
    console.error('[nebra] Sensor register error:', err.message);
  }
}

// ── Submit reading to chain ───────────────────────────────────────────────────

async function submitReading(value, metadata) {
  const sensorPath = encodeURIComponent(SENSOR_ID);
  try {
    const r = await apiPost(`/api/sensors/${sensorPath}/readings`, {
      value,
      relay_account: GATEWAY_OWNER,
      metadata: {
        gateway_id: GATEWAY_ID,
        source: 'flipper-zero',
        ...metadata,
      },
    });
    if (r.status === 201 || r.status === 200) {
      packetsRelayed++;
      const epoch = r.body && r.body.reading && r.body.reading.epoch;
      console.log(`[nebra] Reading submitted: ${value} dBm  epoch=${epoch || '?'}  total=${packetsRelayed}`);
    } else {
      console.warn('[nebra] Submit response:', r.status, JSON.stringify(r.body).slice(0, 200));
    }
  } catch (err) {
    console.error('[nebra] Submit error:', err.message);
  }
}

// ── Parse Flipper sensor batch → dominant reading ─────────────────────────────

function flipperBatchToDominant(readings) {
  if (!Array.isArray(readings) || readings.length === 0) return null;

  // Best SubGHz reading (highest RSSI = strongest signal)
  const subghz = readings.filter(r => r && r.type === 'subghz' && typeof r.rssi === 'number');
  if (subghz.length > 0) {
    const best = subghz.reduce((a, b) => (b.rssi > a.rssi ? b : a));
    return {
      value: best.rssi,
      metadata: {
        type: 'subghz',
        unit: 'dbm',
        freq_hz: best.freq,
        freq_mhz: best.freq ? (best.freq / 1e6).toFixed(3) : undefined,
        signal_strength_dbm: best.rssi,
        subghz_count: subghz.length,
        ble_hits: readings.filter(r => r && r.type === 'ble').reduce((s, r) => s + (r.ch_hits || 0), 0),
      },
    };
  }

  // BLE fallback — use best RSSI
  const ble = readings.filter(r => r && r.type === 'ble' && typeof r.best_rssi === 'number');
  if (ble.length > 0) {
    const best = ble.reduce((a, b) => (b.best_rssi > a.best_rssi ? b : a));
    return {
      value: best.best_rssi,
      metadata: { type: 'ble', unit: 'dbm', signal_strength_dbm: best.best_rssi, ble_ch_hits: best.ch_hits },
    };
  }

  // GPIO ADC fallback — mV reading
  const gpio = readings.filter(r => r && r.type === 'gpio' && typeof r.mv === 'number');
  if (gpio.length > 0) {
    const last = gpio[gpio.length - 1];
    return {
      value: last.mv,
      metadata: { type: 'gpio_adc', unit: 'mv', signal_strength_dbm: null },
    };
  }

  return null;
}

// ── Heartbeat ─────────────────────────────────────────────────────────────────

async function sendHeartbeat() {
  const gwPath = encodeURIComponent(GATEWAY_ID);
  try {
    const r = await apiPost(`/api/gateways/${gwPath}/heartbeat`, {
      packets_relayed_this_epoch: packetsRelayed,
      sensors_connected: 1,
    });
    if (r.status === 200) {
      console.log(`[nebra] Heartbeat sent (packets=${packetsRelayed})`);
    } else {
      console.warn('[nebra] Heartbeat:', r.status, JSON.stringify(r.body).slice(0, 100));
    }
  } catch (err) {
    console.error('[nebra] Heartbeat error:', err.message);
  }
}

// ── Local HTTP relay endpoint ─────────────────────────────────────────────────
// Any client on LAN can POST /relay with a Flipper readings batch:
//   { readings: [...] }            — array of Flipper sensor objects
//   { value, metadata? }           — already-processed single reading
//   { device_id, fields, ts, ... } — legacy flipper-relay.js format

function startLocalRelay() {
  const server = http.createServer(async (req, res) => {
    if (req.method !== 'POST' || req.url !== '/relay') {
      res.writeHead(404);
      return res.end('Not found');
    }

    const chunks = [];
    req.on('data', c => chunks.push(c));
    req.on('end', async () => {
      let body;
      try { body = JSON.parse(Buffer.concat(chunks).toString()); } catch (_) {
        res.writeHead(400); return res.end('Bad JSON');
      }

      let value, metadata;

      if (body.readings) {
        // Flipper batch format
        const r = flipperBatchToDominant(body.readings);
        if (!r) { res.writeHead(200); return res.end(JSON.stringify({ ok: true, skipped: true })); }
        value = r.value; metadata = r.metadata;
      } else if (body.value !== undefined) {
        // Pre-processed single reading
        value = Number(body.value);
        metadata = body.metadata || {};
      } else if (body.fields) {
        // Legacy flipper-relay.js payload — extract dominant field
        const fields = body.fields;
        const rfKeys = Object.keys(fields).filter(k => k.endsWith('_rssi'));
        if (rfKeys.length > 0) {
          const best = rfKeys.reduce((a, b) => (fields[b] > fields[a] ? b : a));
          value = fields[best];
          const mhz = best.replace(/^rf_/, '').replace(/_rssi$/, '').replace(/_/g, '.');
          metadata = { type: 'subghz', unit: 'dbm', freq_mhz: mhz, signal_strength_dbm: value };
          if (fields.ble_best_rssi !== undefined) metadata.ble_best_rssi = fields.ble_best_rssi;
        } else if (fields.ble_best_rssi !== undefined) {
          value = fields.ble_best_rssi;
          metadata = { type: 'ble', unit: 'dbm', signal_strength_dbm: value };
        } else {
          res.writeHead(200); return res.end(JSON.stringify({ ok: true, skipped: true }));
        }
      } else {
        res.writeHead(400); return res.end('Missing readings or value');
      }

      if (!Number.isFinite(value)) { res.writeHead(400); return res.end('Non-finite value'); }

      await submitReading(value, metadata);
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ ok: true, relayed: true, value }));
    });
  });

  server.listen(RELAY_PORT, '0.0.0.0', () => {
    console.log(`[nebra] Local relay endpoint: http://0.0.0.0:${RELAY_PORT}/relay`);
    console.log(`[nebra] Flipper relay: set RELAY_URL=http://<nebra-ip>:${RELAY_PORT}/relay`);
  });
}

// ── BLE scanner (optional — requires @abandonware/noble) ─────────────────────

function startBleScanner() {
  let noble;
  try {
    noble = require('@abandonware/noble');
  } catch (_) {
    console.warn('[nebra] BLE: @abandonware/noble not installed — skipping BLE scan.');
    console.warn('[nebra] BLE: npm install @abandonware/noble  (then: sudo setcap cap_net_raw+eip $(which node))');
    return;
  }

  noble.on('stateChange', (state) => {
    if (state === 'poweredOn') {
      console.log('[nebra] BLE scanning for Flipper...');
      noble.startScanning([], true); // scan all, allow duplicates for live readings
    } else {
      noble.stopScanning();
      console.warn('[nebra] BLE adapter state:', state);
    }
  });

  noble.on('discover', (peripheral) => {
    const name = peripheral.advertisement && peripheral.advertisement.localName;
    // Flipper BTCPC app advertises as "btcpc" prefix
    if (!name || !name.startsWith('btcpc')) return;

    console.log(`[nebra] BLE: found Flipper "${name}" rssi=${peripheral.rssi}`);

    // Read manufacturer data if present (compact binary payload from Flipper app)
    const mfr = peripheral.advertisement && peripheral.advertisement.manufacturerData;
    if (mfr && mfr.length >= 4) {
      // Expected format: [0x42, 0x43] (magic) + int16 RSSI LE + 3-byte freq (MHz uint24 LE)
      const magic = mfr.readUInt16LE(0);
      if (magic === 0x4342) { // "BC" magic bytes
        const rssi = mfr.readInt16LE(2) / 10.0; // scaled by 10
        const freqMhz = mfr.length >= 7 ? (mfr.readUIntLE(4, 3) / 1000.0) : null;
        console.log(`[nebra] BLE payload: rssi=${rssi} dBm  freq=${freqMhz} MHz`);
        submitReading(rssi, {
          type: 'subghz',
          unit: 'dbm',
          freq_mhz: freqMhz,
          signal_strength_dbm: rssi,
          ble_advertisement_rssi: peripheral.rssi,
          source: name,
        });
        return;
      }
    }

    // Fallback: use BLE RSSI as the value (Flipper seen, signal strength)
    submitReading(peripheral.rssi, {
      type: 'ble',
      unit: 'dbm',
      signal_strength_dbm: peripheral.rssi,
      source: name,
    });
  });
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  console.log(`[nebra] BTCPC Nebra Gateway starting`);
  console.log(`[nebra] Gateway: ${GATEWAY_ID}  API: ${API_BASE}`);
  console.log(`[nebra] Sensor: ${SENSOR_ID}`);

  await ensureGateway();
  await ensureSensor();

  startLocalRelay();

  if (BLE_ENABLED) {
    startBleScanner();
  } else {
    console.log('[nebra] BLE scan disabled (BLE_ENABLED=false)');
  }

  // Periodic heartbeat
  setInterval(sendHeartbeat, HEARTBEAT_SEC * 1000);
  setTimeout(sendHeartbeat, 5000); // first heartbeat shortly after start
}

main().catch((err) => { console.error('[nebra] Fatal:', err); process.exit(1); });
