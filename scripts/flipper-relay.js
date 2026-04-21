#!/usr/bin/env node
/**
 * BTCPC Flipper Zero Relay
 *
 * Reads sensor data from the Flipper Zero BTCPC app over USB serial and
 * submits each reading to the BTCPC chain.
 *
 * Two relay modes:
 *
 *   Direct mode (default):
 *     Submits to btcpc.net (or BTCPC_API) directly via JWT auth.
 *     BTCPC_JWT=<token> node scripts/flipper-relay.js
 *
 *   Gateway mode:
 *     Posts to a local Nebra gateway daemon (nebra-gateway.js) which
 *     relays to the chain and credits the Nebra gateway for the packet.
 *     RELAY_URL=http://192.168.68.xx:7979/relay node scripts/flipper-relay.js
 *
 * Usage:
 *   BTCPC_JWT=<token> node scripts/flipper-relay.js
 *   RELAY_URL=http://nebra-ip:7979/relay FLIPPER_PORT=/dev/ttyACM0 node scripts/flipper-relay.js
 *   BTCPC_JWT=<token> BTCPC_API=http://localhost:4242 FLIPPER_PORT=/dev/ttyACM0 node scripts/flipper-relay.js
 *
 * The Flipper must be running the BTCPC sensor app.
 */

const { SerialPort } = require('serialport');
const https = require('https');
const http  = require('http');

const FLIPPER_PORT     = process.env.FLIPPER_PORT     || '/dev/ttyACM0';
const FLIPPER_BAUD     = parseInt(process.env.FLIPPER_BAUD || '115200');
const API_BASE         = process.env.BTCPC_API        || 'https://btcpc.net';
const JWT              = process.env.BTCPC_JWT        || null;
const RELAY_URL        = process.env.RELAY_URL        || null; // point at nebra-gateway
const WALLET           = process.env.FLIPPER_WALLET   || null;
const SENSOR_OWNER     = process.env.SENSOR_OWNER     || WALLET || 'josh';
const SENSOR_NAME      = process.env.SENSOR_NAME      || 'flipper-zero';
const POLL_MS          = parseInt(process.env.FLIPPER_POLL_MS || '5000');

// Must have either JWT (direct mode) or RELAY_URL (gateway mode)
if (!JWT && !RELAY_URL) {
  console.error('[flipper] Set BTCPC_JWT for direct mode or RELAY_URL=http://<nebra-ip>:7979/relay for gateway mode.');
  process.exit(1);
}

const SENSOR_ID = SENSOR_OWNER + '/' + SENSOR_NAME;
let sensorRegistered = false;

// ── HTTP helpers ──────────────────────────────────────────────────────────────

function apiPost(path, body, noAuth) {
  return new Promise((resolve, reject) => {
    const raw = JSON.stringify(body);
    const urlObj = new URL(API_BASE + path);
    const mod = urlObj.protocol === 'https:' ? https : http;
    const headers = {
      'Content-Type': 'application/json',
      'Content-Length': Buffer.byteLength(raw),
    };
    if (!noAuth && JWT) headers['Authorization'] = 'Bearer ' + JWT;
    const opts = {
      hostname: urlObj.hostname,
      port: urlObj.port || (urlObj.protocol === 'https:' ? 443 : 80),
      path: urlObj.pathname,
      method: 'POST',
      headers,
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

function relayPost(body) {
  return new Promise((resolve, reject) => {
    const raw = JSON.stringify(body);
    const urlObj = new URL(RELAY_URL);
    const mod = urlObj.protocol === 'https:' ? https : http;
    const opts = {
      hostname: urlObj.hostname,
      port: urlObj.port || (urlObj.protocol === 'https:' ? 443 : 80),
      path: urlObj.pathname,
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(raw) },
    };
    const req = mod.request(opts, (res) => {
      const chunks = [];
      res.on('data', c => chunks.push(c));
      res.on('end', () => {
        try { resolve(JSON.parse(Buffer.concat(chunks).toString())); }
        catch (_) { resolve({}); }
      });
    });
    req.on('error', reject);
    req.end(raw);
  });
}

// ── Sensor auto-registration (direct mode) ────────────────────────────────────

async function ensureSensor() {
  if (sensorRegistered || RELAY_URL) return; // gateway handles registration
  try {
    const r = await apiPost('/api/sensors', {
      name: SENSOR_NAME,
      type: 'rf-scanner',
      unit: 'dbm',
      decimals: 1,
      region: 'us-west',
      hardware_model: 'flipper-zero',
      firmware_version: 'btcpc-app',
    });
    if (r.status === 201) {
      console.log(`[flipper] Sensor registered: ${SENSOR_ID}`);
    } else if (r.status === 409) {
      console.log(`[flipper] Sensor already registered: ${SENSOR_ID}`);
    } else {
      console.warn('[flipper] Sensor register:', r.status, JSON.stringify(r.body).slice(0, 100));
    }
    sensorRegistered = true;
  } catch (err) {
    console.error('[flipper] Sensor register error:', err.message);
  }
}

// ── Serial connection ─────────────────────────────────────────────────────────

let port;
let walletName = WALLET;

function openPort() {
  port = new SerialPort({ path: FLIPPER_PORT, baudRate: FLIPPER_BAUD, autoOpen: false });

  port.open((err) => {
    if (err) {
      console.error(`[flipper] Cannot open ${FLIPPER_PORT}: ${err.message}`);
      console.error('[flipper] Retrying in 5s...');
      setTimeout(openPort, 5000);
      return;
    }
    console.log(`[flipper] Connected to ${FLIPPER_PORT}`);
    wireDataHandler();
    sendCmd({ cmd: 'list' });
    setTimeout(poll, 1000);
  });

  port.on('error', (err) => {
    console.error('[flipper] Serial error:', err.message);
  });

  port.on('close', () => {
    console.warn('[flipper] Port closed — reconnecting in 5s...');
    port = null;
    setTimeout(openPort, 5000);
  });
}

// ── Protocol ──────────────────────────────────────────────────────────────────

let recvBuf = '';

function sendCmd(obj) {
  if (!port || !port.isOpen) return;
  port.write(JSON.stringify(obj) + '\r\n');
}

function wireDataHandler() {
  if (!port) return;
  port.on('data', (chunk) => {
    recvBuf += chunk.toString('utf8');
    let nl;
    while ((nl = recvBuf.indexOf('\n')) !== -1) {
      const line = recvBuf.slice(0, nl).trim();
      recvBuf = recvBuf.slice(nl + 1);
      if (!line) continue;
      try { handleMessage(JSON.parse(line)); } catch (_) {}
    }
  });
}

function handleMessage(msg) {
  if (!msg || msg.status === 'error') return;

  if (msg.wallets && msg.wallets.length > 0 && !walletName) {
    walletName = msg.wallets[0];
    console.log(`[flipper] Wallet: ${walletName}  Sensor: ${SENSOR_ID}`);
  }

  if (msg.readings) {
    const readings = msg.readings.filter(Boolean);
    if (readings.length > 0) processReadings(readings);
  }
}

// ── Poll loop ─────────────────────────────────────────────────────────────────

function poll() {
  sendCmd({ cmd: 'get' });
  setTimeout(poll, POLL_MS);
}

// ── Submit readings ───────────────────────────────────────────────────────────

async function processReadings(readings) {
  if (RELAY_URL) {
    // Gateway mode — ship raw batch to Nebra; it handles formatting + chain submission
    try {
      const r = await relayPost({ readings });
      if (r.relayed) {
        console.log(`[flipper] → Nebra gateway  value=${r.value} dBm  count=${readings.length}`);
      } else if (r.skipped) {
        console.log('[flipper] → Nebra: no dominant reading in batch');
      } else {
        console.warn('[flipper] → Nebra response:', JSON.stringify(r).slice(0, 100));
      }
    } catch (err) {
      console.error('[flipper] → Nebra error:', err.message);
    }
    return;
  }

  // Direct mode — submit to chain ourselves
  await ensureSensor();

  const dominant = dominantReading(readings);
  if (!dominant) return;

  const sensorPath = encodeURIComponent(SENSOR_ID);
  try {
    const r = await apiPost(`/api/sensors/${sensorPath}/readings`, {
      value: dominant.value,
      metadata: dominant.metadata,
    });
    if (r.status === 201 || r.status === 200) {
      const epoch = r.body && r.body.reading && r.body.reading.epoch;
      console.log(`[flipper] Submitted: ${dominant.value} dBm  epoch=${epoch || '?'}  raw=${readings.length} readings`);
    } else {
      console.warn('[flipper] Submit:', r.status, JSON.stringify(r.body).slice(0, 200));
    }
  } catch (err) {
    console.error('[flipper] Submit error:', err.message);
  }
}

function dominantReading(readings) {
  const subghz = readings.filter(r => r.type === 'subghz' && typeof r.rssi === 'number');
  if (subghz.length > 0) {
    const best = subghz.reduce((a, b) => (b.rssi > a.rssi ? b : a));
    return {
      value: best.rssi,
      metadata: {
        type: 'subghz',
        unit: 'dbm',
        freq_mhz: best.freq ? String((best.freq / 1e6).toFixed(3)) : undefined,
        signal_strength_dbm: best.rssi,
        source: 'flipper-zero',
      },
    };
  }

  const ble = readings.filter(r => r.type === 'ble' && typeof r.best_rssi === 'number');
  if (ble.length > 0) {
    const best = ble.reduce((a, b) => (b.best_rssi > a.best_rssi ? b : a));
    return {
      value: best.best_rssi,
      metadata: { type: 'ble', unit: 'dbm', signal_strength_dbm: best.best_rssi, source: 'flipper-zero' },
    };
  }

  const gpio = readings.filter(r => r.type === 'gpio' && typeof r.mv === 'number');
  if (gpio.length > 0) {
    const last = gpio[gpio.length - 1];
    return {
      value: last.mv,
      metadata: { type: 'gpio_adc', unit: 'mv', source: 'flipper-zero' },
    };
  }

  return null;
}

// ── Start ─────────────────────────────────────────────────────────────────────

const mode = RELAY_URL ? `gateway → ${RELAY_URL}` : `direct → ${API_BASE}`;
console.log(`[flipper] BTCPC Flipper Relay  mode=${mode}  port=${FLIPPER_PORT}  poll=${POLL_MS}ms`);
openPort();
