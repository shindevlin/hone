#!/usr/bin/env node
/**
 * HONE Flipper Bridge
 *
 * Universal bridge between Flipper Zero devices and the HONE chain.
 * Starts every transport that has the necessary hardware and packages
 * available, in parallel. Works with whatever is present — no assumptions
 * about which transport exists.
 *
 * Transports (all optional, all independent):
 *
 *   USB serial   — Flipper plugged into this machine via USB-C.
 *                  Auto-discovers all connected Flippers by VID:PID.
 *                  Package: serialport (npm install serialport)
 *
 *   BLE          — Flipper HONE app in BLE peripheral mode advertising
 *                  sensor data. Any nearby machine with a BT adapter works.
 *                  Package: @abandonware/noble (npm install @abandonware/noble)
 *                  Linux: sudo setcap cap_net_raw+eip $(which node)
 *
 *   SubGHz/SDR   — Flipper transmits FSK packets; RTL-SDR dongle receives
 *                  them via rtl_433 (or rtl_fm). Requires hardware: RTL-SDR
 *                  dongle + rtl_433 installed (apt install rtl-433).
 *                  Falls back gracefully if neither is installed.
 *
 *   WiFi/HTTP    — Flipper WiFi Dev Board POSTs sensor data directly to
 *                  this machine over local WiFi. HTTP server, no packages needed.
 *                  Flipper hits: POST http://<this-machine-ip>:WIFI_PORT/ingest
 *
 * Any transport that finds a Flipper feeds into the same submission path.
 * Multiple Flippers on multiple transports simultaneously — each wallet's
 * readings are attributed separately.
 *
 * Usage:
 *   HONE_JWT=<token> node scripts/hone-flipper-bridge.js
 *   HONE_JWT=<token> HONE_API=http://localhost:4242 node scripts/hone-flipper-bridge.js
 *   HONE_JWT=<token> RELAY_URL=http://other-node:7979/flipper node scripts/hone-flipper-bridge.js
 *
 * If RELAY_URL is set, all ingested readings are forwarded to another HONE
 * node instead of submitted directly (useful when this machine has no JWT).
 *
 * Environment:
 *   HONE_JWT          HONE auth token (required unless RELAY_URL is set)
 *   HONE_API          chain API base URL (default: https://hone.net)
 *   RELAY_URL          forward to another hone-flipper-bridge instead (optional)
 *   DEFAULT_WALLET     fallback wallet name if Flipper doesn't identify (default: josh)
 *   SENSOR_REGION      region tag for sensor registration (default: us-west)
 *   GATEWAY_OWNER      account that owns any gateway registered (default: same as JWT user)
 *   GATEWAY_NAME       gateway slug (default: hone-bridge)
 *   USB_ENABLED        false to skip USB transport (default: true)
 *   BLE_ENABLED        false to skip BLE transport (default: true)
 *   SDR_ENABLED        false to skip SDR/SubGHz transport (default: true)
 *   WIFI_ENABLED       false to skip WiFi/HTTP transport (default: true)
 *   WIFI_PORT          WiFi ingest HTTP port (default: 7979)
 *   USB_POLL_MS        Flipper poll interval over USB (default: 5000)
 */

'use strict';

const https   = require('https');
const http    = require('http');
const { spawn } = require('child_process');

// ── Config ────────────────────────────────────────────────────────────────────

const API_BASE       = process.env.HONE_API       || 'https://hone.net';
const JWT            = process.env.HONE_JWT       || null;
const RELAY_URL      = process.env.RELAY_URL       || null;
const DEFAULT_WALLET = process.env.DEFAULT_WALLET  || 'josh';
const SENSOR_REGION  = process.env.SENSOR_REGION   || 'us-west';
const GATEWAY_OWNER  = process.env.GATEWAY_OWNER   || null; // resolved after JWT decode
const GATEWAY_NAME   = process.env.GATEWAY_NAME    || 'hone-bridge';
const USB_ENABLED    = process.env.USB_ENABLED     !== 'false';
const BLE_ENABLED    = process.env.BLE_ENABLED     !== 'false';
const SDR_ENABLED    = process.env.SDR_ENABLED     !== 'false';
const WIFI_ENABLED   = process.env.WIFI_ENABLED    !== 'false';
const WIFI_PORT      = parseInt(process.env.WIFI_PORT  || '7979');
const USB_POLL_MS    = parseInt(process.env.USB_POLL_MS || '5000');

if (!JWT && !RELAY_URL) {
  console.error('[bridge] Set HONE_JWT for direct mode, or RELAY_URL=http://<node>:7979/flipper to relay.');
  process.exit(1);
}

// Flipper Zero USB VID:PID (STMicro VCP)
const FLIPPER_VID = '0483';
const FLIPPER_PID = '5740';

// ── State ─────────────────────────────────────────────────────────────────────

// wallet → true once registered on chain
const registeredSensors = new Set();
let gatewayRegistered = false;
let gatewayOwner = GATEWAY_OWNER;

// Decode wallet from JWT to use as gateway owner
if (!gatewayOwner && JWT) {
  try {
    const payload = JSON.parse(Buffer.from(JWT.split('.')[1], 'base64').toString());
    gatewayOwner = payload.username || payload.account || null;
  } catch (_) {}
}
if (!gatewayOwner) gatewayOwner = DEFAULT_WALLET;

// ── HTTP helpers ──────────────────────────────────────────────────────────────

function apiPost(path, body) {
  return new Promise((resolve, reject) => {
    const raw = JSON.stringify(body);
    const urlObj = new URL(API_BASE + path);
    const mod = urlObj.protocol === 'https:' ? https : http;
    const req = mod.request({
      hostname: urlObj.hostname,
      port: urlObj.port || (urlObj.protocol === 'https:' ? 443 : 80),
      path: urlObj.pathname,
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(raw),
        'Authorization': 'Bearer ' + JWT,
      },
    }, (res) => {
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

// ── Gateway + sensor registration ────────────────────────────────────────────

async function ensureGateway() {
  if (gatewayRegistered || RELAY_URL) return;
  const gatewayId = gatewayOwner + '/' + GATEWAY_NAME;
  try {
    const r = await apiPost('/api/gateways', {
      name: GATEWAY_NAME,
      region: SENSOR_REGION,
      latitude: 0,
      longitude: 0,
      hardware_model: 'hone-flipper-bridge',
    });
    if (r.status === 201) console.log(`[bridge] Gateway registered: ${gatewayId}`);
    else if (r.status === 409) console.log(`[bridge] Gateway active: ${gatewayId}`);
    else console.warn('[bridge] Gateway register:', r.status);
    gatewayRegistered = true;
  } catch (err) {
    console.warn('[bridge] Gateway register error:', err.message);
  }
}

async function ensureSensor(wallet) {
  if (registeredSensors.has(wallet) || RELAY_URL) return;
  const sensorName = 'flipper-zero';
  try {
    const r = await apiPost('/api/sensors', {
      name: sensorName,
      type: 'rf-scanner',
      unit: 'dbm',
      decimals: 1,
      region: SENSOR_REGION,
      hardware_model: 'flipper-zero',
      firmware_version: 'hone-app',
      lora_gateway: gatewayOwner + '/' + GATEWAY_NAME,
    });
    if (r.status === 201) console.log(`[bridge] Sensor registered: ${wallet}/${sensorName}`);
    else if (r.status === 409) console.log(`[bridge] Sensor active: ${wallet}/${sensorName}`);
    else console.warn('[bridge] Sensor register:', r.status, JSON.stringify(r.body).slice(0, 80));
    registeredSensors.add(wallet);
  } catch (err) {
    console.warn('[bridge] Sensor register error:', err.message);
  }
}

// ── Core ingest — called by every transport ───────────────────────────────────

async function ingest(wallet, readings, transport) {
  if (!Array.isArray(readings) || readings.length === 0) return;

  const dominant = pickDominant(readings);
  if (!dominant) return;

  if (RELAY_URL) {
    // Forward to another bridge node
    return forwardToRelay({ wallet, readings, transport });
  }

  await ensureSensor(wallet);

  const sensorId  = wallet + '/flipper-zero';
  const sensorPath = encodeURIComponent(sensorId);
  const gatewayId  = gatewayOwner + '/' + GATEWAY_NAME;

  try {
    const r = await apiPost(`/api/sensors/${sensorPath}/readings`, {
      value: dominant.value,
      relay_account: gatewayOwner,
      metadata: {
        gateway_id: gatewayId,
        transport,
        source: 'flipper-zero',
        signal_strength_dbm: dominant.value,
        ...dominant.meta,
      },
    });
    if (r.status === 201) {
      const epoch = r.body && r.body.reading && r.body.reading.epoch;
      console.log(`[${transport}] ${wallet}: ${dominant.value} dBm  epoch=${epoch || '?'}  (${readings.length} raw)`);
    } else if (r.status === 200 && r.body && r.body.witnessed) {
      const n = r.body.witness_count;
      const epoch = r.body.reading && r.body.reading.epoch;
      console.log(`[${transport}] ${wallet}: witnessed (${n} nodes)  epoch=${epoch || '?'}  median=${r.body.reading && r.body.reading.witnessed_median}`);
    } else if (r.status === 200 && r.body && r.body.duplicate) {
      // same gateway already submitted this epoch — normal, no log noise
    } else {
      console.warn(`[${transport}] submit ${r.status}:`, JSON.stringify(r.body).slice(0, 150));
    }
  } catch (err) {
    console.error(`[${transport}] ingest error:`, err.message);
  }
}

function pickDominant(readings) {
  const subghz = readings.filter(r => r && r.type === 'subghz' && typeof r.rssi === 'number');
  if (subghz.length > 0) {
    const best = subghz.reduce((a, b) => b.rssi > a.rssi ? b : a);
    return {
      value: best.rssi,
      meta: {
        type: 'subghz',
        freq_mhz: best.freq ? String((best.freq / 1e6).toFixed(3)) : undefined,
        subghz_band_count: subghz.length,
        ble_hits: readings.filter(r => r && r.type === 'ble').reduce((s, r) => s + (r.ch_hits || 0), 0),
      },
    };
  }

  const ble = readings.filter(r => r && r.type === 'ble' && typeof r.best_rssi === 'number');
  if (ble.length > 0) {
    const best = ble.reduce((a, b) => b.best_rssi > a.best_rssi ? b : a);
    return { value: best.best_rssi, meta: { type: 'ble', ble_ch_hits: best.ch_hits } };
  }

  const gpio = readings.filter(r => r && r.type === 'gpio' && typeof r.mv === 'number');
  if (gpio.length > 0) {
    const last = gpio[gpio.length - 1];
    return { value: last.mv, meta: { type: 'gpio_adc', unit: 'mv' } };
  }

  return null;
}

function forwardToRelay(payload) {
  return new Promise((resolve) => {
    const raw = JSON.stringify(payload);
    const urlObj = new URL(RELAY_URL);
    const mod = urlObj.protocol === 'https:' ? https : http;
    const req = mod.request({
      hostname: urlObj.hostname,
      port: urlObj.port || (urlObj.protocol === 'https:' ? 443 : 80),
      path: urlObj.pathname,
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(raw) },
    }, (res) => {
      res.resume();
      console.log(`[relay→] forwarded ${payload.transport} readings (${res.statusCode})`);
      resolve();
    });
    req.on('error', (e) => { console.error('[relay→] forward error:', e.message); resolve(); });
    req.end(raw);
  });
}

// ── Transport: USB serial ─────────────────────────────────────────────────────

function startUsbTransport() {
  let SerialPort, list;
  try {
    ({ SerialPort } = require('serialport'));
    list = SerialPort.list;
  } catch (_) {
    console.warn('[usb] serialport not installed — skipping USB transport');
    console.warn('[usb] npm install serialport');
    return;
  }

  // Track open connections: port path → { port, wallet, recvBuf }
  const connections = new Map();

  async function scanPorts() {
    let ports;
    try { ports = await SerialPort.list(); } catch (_) { return; }

    const flippers = ports.filter(p =>
      p.vendorId && p.productId &&
      p.vendorId.toLowerCase() === FLIPPER_VID &&
      p.productId.toLowerCase() === FLIPPER_PID
    );

    // Connect to newly discovered Flippers
    for (const portInfo of flippers) {
      if (connections.has(portInfo.path)) continue;
      console.log(`[usb] Flipper found at ${portInfo.path}`);
      openFlipperPort(portInfo.path, SerialPort, connections);
    }

    // Clean up connections to ports that disappeared
    for (const [path] of connections) {
      if (!flippers.find(p => p.path === path)) {
        console.log(`[usb] Flipper disconnected: ${path}`);
        const conn = connections.get(path);
        if (conn && conn.port && conn.port.isOpen) conn.port.close();
        connections.delete(path);
      }
    }
  }

  // Scan immediately, then every 10 seconds
  scanPorts();
  setInterval(scanPorts, 10000);
}

function openFlipperPort(portPath, SerialPort, connections) {
  const conn = { port: null, wallet: null, recvBuf: '', pollTimer: null };
  connections.set(portPath, conn);

  const port = new SerialPort({ path: portPath, baudRate: 115200, autoOpen: false });
  conn.port = port;

  port.open((err) => {
    if (err) {
      console.error(`[usb] Cannot open ${portPath}:`, err.message);
      connections.delete(portPath);
      return;
    }

    port.on('data', (chunk) => {
      conn.recvBuf += chunk.toString('utf8');
      let nl;
      while ((nl = conn.recvBuf.indexOf('\n')) !== -1) {
        const line = conn.recvBuf.slice(0, nl).trim();
        conn.recvBuf = conn.recvBuf.slice(nl + 1);
        if (!line) continue;
        try {
          const msg = JSON.parse(line);
          handleUsbMessage(conn, msg, portPath);
        } catch (_) {}
      }
    });

    port.on('error', (err) => console.error(`[usb] ${portPath} error:`, err.message));
    port.on('close', () => {
      console.warn(`[usb] ${portPath} closed`);
      if (conn.pollTimer) clearTimeout(conn.pollTimer);
      connections.delete(portPath);
    });

    // Identify wallet then start polling
    port.write(JSON.stringify({ cmd: 'list' }) + '\r\n');
    schedulePoll(conn, port);
  });
}

function schedulePoll(conn, port) {
  conn.pollTimer = setTimeout(() => {
    if (port.isOpen) {
      port.write(JSON.stringify({ cmd: 'get' }) + '\r\n');
      schedulePoll(conn, port);
    }
  }, USB_POLL_MS);
}

function handleUsbMessage(conn, msg, portPath) {
  if (!msg || msg.status === 'error') return;

  if (msg.wallets && msg.wallets.length > 0 && !conn.wallet) {
    conn.wallet = msg.wallets[0];
    console.log(`[usb] ${portPath} → wallet: ${conn.wallet}`);
  }

  if (msg.readings) {
    const wallet = conn.wallet || DEFAULT_WALLET;
    ingest(wallet, msg.readings.filter(Boolean), 'usb');
  }
}

// ── Transport: BLE ────────────────────────────────────────────────────────────

function startBleTransport() {
  let noble;
  try { noble = require('@abandonware/noble'); } catch (_) {
    console.warn('[ble] @abandonware/noble not installed — skipping BLE transport');
    console.warn('[ble] npm install @abandonware/noble');
    console.warn('[ble] Linux: sudo setcap cap_net_raw+eip $(which node)');
    return;
  }

  noble.on('stateChange', (state) => {
    if (state === 'poweredOn') {
      console.log('[ble] Scanning for Flipper advertisements...');
      noble.startScanning([], true);
    } else {
      noble.stopScanning();
      if (state !== 'unknown') console.warn('[ble] BT adapter state:', state);
    }
  });

  noble.on('discover', (peripheral) => {
    const name = peripheral.advertisement && peripheral.advertisement.localName;
    // Flipper HONE app advertises as "hone[-<wallet>]"
    if (!name || !name.toLowerCase().startsWith('hone')) return;

    // Wallet name is the suffix after "hone-" if present
    const wallet = name.toLowerCase().startsWith('hone-')
      ? name.slice(6).replace(/[^a-z0-9]/g, '') || DEFAULT_WALLET
      : DEFAULT_WALLET;

    const mfr = peripheral.advertisement && peripheral.advertisement.manufacturerData;

    if (mfr && mfr.length >= 4 && mfr.readUInt16LE(0) === 0x4342) {
      // Structured payload: [0x42,0x43] magic + int16 rssi×10 + uint24 freq_khz
      const rssi = mfr.readInt16LE(2) / 10.0;
      const freqKhz = mfr.length >= 7 ? mfr.readUIntLE(4, 3) : null;
      const readings = [{ type: 'subghz', rssi, freq: freqKhz ? freqKhz * 1000 : null }];
      ingest(wallet, readings, 'ble');
    } else {
      // Fallback: BLE signal strength is the only available measurement
      const readings = [{ type: 'ble', best_rssi: peripheral.rssi, ch_hits: 1 }];
      ingest(wallet, readings, 'ble');
    }
  });
}

// ── Transport: SubGHz / SDR ───────────────────────────────────────────────────
// Uses rtl_433 (https://github.com/merbanan/rtl_433) to receive FSK packets
// from a Flipper transmitting on common ISM frequencies.
// The Flipper HONE app transmits a simple JSON payload via OOK/FSK.

function startSdrTransport() {
  // Check if rtl_433 or rtl_fm is available
  const candidates = ['rtl_433', 'rtl_fm'];
  let tool = null;
  const { execSync } = require('child_process');

  for (const t of candidates) {
    try { execSync(`which ${t} 2>/dev/null`, { stdio: 'pipe' }); tool = t; break; } catch (_) {}
  }

  if (!tool) {
    console.warn('[sdr] rtl_433 not found — skipping SDR/SubGHz transport');
    console.warn('[sdr] apt install rtl-433   or   brew install rtl_433');
    return;
  }

  console.log(`[sdr] Starting SDR receiver with ${tool}...`);

  // rtl_433 in JSON output mode, listening on Flipper's preferred frequencies
  // 433.92 MHz (EU ISM), 315 MHz (US), 868 MHz (EU LoRa), 915 MHz (US LoRa)
  const args = tool === 'rtl_433'
    ? ['-F', 'json', '-f', '433920000', '-f', '315000000', '-f', '915000000']
    : ['-f', '433920000', '-s', '250000', '-'];

  const proc = spawn(tool, args, { stdio: ['ignore', 'pipe', 'pipe'] });

  let buf = '';
  proc.stdout.on('data', (chunk) => {
    buf += chunk.toString();
    let nl;
    while ((nl = buf.indexOf('\n')) !== -1) {
      const line = buf.slice(0, nl).trim();
      buf = buf.slice(nl + 1);
      if (!line) continue;
      try {
        const decoded = JSON.parse(line);
        handleSdrMessage(decoded);
      } catch (_) {}
    }
  });

  proc.on('error', (err) => {
    console.error(`[sdr] ${tool} error:`, err.message);
  });

  proc.on('exit', (code) => {
    console.warn(`[sdr] ${tool} exited (code ${code}) — restarting in 10s`);
    setTimeout(startSdrTransport, 10000);
  });
}

function handleSdrMessage(decoded) {
  // rtl_433 output for a Flipper packet. The Flipper HONE app embeds a
  // JSON payload in its transmission; rtl_433 decodes the raw FSK frame.
  // Look for our custom fields: hone_wallet, rssi, freq.
  const wallet = decoded.hone_wallet || decoded.id || DEFAULT_WALLET;
  const rssi   = decoded.rssi || decoded.signal || null;
  const freq   = decoded.freq_hz || decoded.frequency || null;

  if (!rssi) return;

  const readings = [{ type: 'subghz', rssi: Number(rssi), freq: freq ? Number(freq) : null }];
  ingest(String(wallet).replace(/[^a-z0-9]/gi, ''), readings, 'subghz');
}

// ── Transport: WiFi / HTTP ────────────────────────────────────────────────────
// Flipper WiFi Dev Board (ESP8266/ESP32) POSTs sensor data directly here.
// The Flipper app hits: POST http://<this-machine-ip>:WIFI_PORT/ingest
// Body: { wallet, readings: [...] }  OR  { wallet, value, metadata }

function startWifiTransport() {
  const server = http.createServer((req, res) => {
    // Also accept forwarded batches from other bridge instances (relay chain)
    if (req.method !== 'POST' || (req.url !== '/ingest' && req.url !== '/flipper')) {
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

      const wallet   = (body.wallet || DEFAULT_WALLET).replace(/[^a-z0-9]/gi, '');
      const readings = normalizeWifiPayload(body);

      if (readings.length === 0) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ ok: true, skipped: true }));
      }

      const transport = body.transport || 'wifi';
      await ingest(wallet, readings, transport);
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ ok: true }));
    });
  });

  server.listen(WIFI_PORT, '0.0.0.0', () => {
    console.log(`[wifi] HTTP ingest endpoint: http://0.0.0.0:${WIFI_PORT}/ingest`);
    console.log(`[wifi] Flipper WiFi Dev Board: POST http://<this-ip>:${WIFI_PORT}/ingest`);
  });

  server.on('error', (err) => {
    if (err.code === 'EADDRINUSE') {
      console.warn(`[wifi] Port ${WIFI_PORT} in use — skipping WiFi transport`);
    } else {
      console.error('[wifi] Server error:', err.message);
    }
  });
}

function normalizeWifiPayload(body) {
  if (body.readings && Array.isArray(body.readings)) return body.readings.filter(Boolean);

  // Single-reading form: { wallet, type, rssi, freq }
  if (body.rssi !== undefined) {
    return [{ type: body.type || 'subghz', rssi: Number(body.rssi), freq: body.freq ? Number(body.freq) : null }];
  }
  if (body.value !== undefined) {
    return [{ type: body.type || 'subghz', rssi: Number(body.value), freq: body.freq_hz ? Number(body.freq_hz) : null }];
  }

  return [];
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  const mode = RELAY_URL ? `relay → ${RELAY_URL}` : `direct → ${API_BASE}`;
  console.log(`[bridge] HONE Flipper Bridge  ${mode}`);

  if (!RELAY_URL) await ensureGateway();

  const transports = [];
  if (USB_ENABLED)  transports.push('USB');
  if (BLE_ENABLED)  transports.push('BLE');
  if (SDR_ENABLED)  transports.push('SDR/SubGHz');
  if (WIFI_ENABLED) transports.push(`WiFi:${WIFI_PORT}`);
  console.log(`[bridge] Starting transports: ${transports.join(', ')}`);

  if (USB_ENABLED)  startUsbTransport();
  if (BLE_ENABLED)  startBleTransport();
  if (SDR_ENABLED)  startSdrTransport();
  if (WIFI_ENABLED) startWifiTransport();
}

main().catch((err) => { console.error('[bridge] Fatal:', err); process.exit(1); });
