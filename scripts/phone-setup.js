#!/usr/bin/env node
/**
 * BTCPC Phone Setup Tool
 *
 * Configures a connected Android phone for BTCPC Flipper relay:
 *   1. Sets up adb reverse so the phone's localhost:4242 maps to this PC's BTCPC node
 *   2. Pushes account name, JWT, and API URL to the phone's SharedPreferences
 *   3. Registers the Flipper sensor on the chain if not already registered
 *
 * Requirements:
 *   - Android phone plugged in via USB with USB debugging enabled
 *   - adb on PATH (Android SDK platform-tools)
 *   - BTCPC node running on this PC (port 4242)
 *   - .env file in project root with JWT_SECRET
 *
 * Usage:
 *   node scripts/phone-setup.js --account josh --jwt <token>
 *   node scripts/phone-setup.js --account josh  (auto-generates JWT from .env)
 *   node scripts/phone-setup.js --account josh --sensor flipper-zero
 */

'use strict';

const http         = require('http');
const crypto       = require('crypto');
const { execSync } = require('child_process');
const path         = require('path');
const fs           = require('fs');

// ── CLI args ──────────────────────────────────────────────────────────────────

const args = process.argv.slice(2);

function getArg(name) {
  const idx = args.findIndex(a => a === name || a === name.replace('--', '-'));
  return idx >= 0 && args[idx + 1] ? args[idx + 1] : null;
}

const account    = getArg('--account') || getArg('-a') || 'josh';
let   jwtToken   = getArg('--jwt') || getArg('-j');
const sensorName = getArg('--sensor') || getArg('-s') || 'flipper-zero';
const apiUrl     = getArg('--api')    || 'http://127.0.0.1:4242';
const PHONE_PORT = 4242;
const APP_PKG    = 'network.btcpc.app';
const PREFS_FILE = 'btcpc_native_state';

// ── Helpers ───────────────────────────────────────────────────────────────────

function log(msg)  { process.stdout.write('[btcpc-setup] ' + msg + '\n'); }
function ok(msg)   { process.stdout.write('[btcpc-setup] ✓ ' + msg + '\n'); }
function warn(msg) { process.stderr.write('[btcpc-setup] WARN: ' + msg + '\n'); }
function fail(msg) { process.stderr.write('[btcpc-setup] ERROR: ' + msg + '\n'); process.exit(1); }

function adb(cmd, opts = {}) {
  try {
    const out = execSync('adb ' + cmd, { stdio: opts.silent ? 'pipe' : 'inherit', encoding: 'utf8' });
    return out ? out.trim() : '';
  } catch (e) {
    return null;
  }
}

function loadJwtSecret() {
  const envPath = path.join(__dirname, '..', '.env');
  if (!fs.existsSync(envPath)) return null;
  const lines = fs.readFileSync(envPath, 'utf8').split('\n');
  for (const line of lines) {
    const m = line.match(/^(?:JWT_SECRET|BTCPC_JWT_SECRET)\s*=\s*(.+)/);
    if (m) return m[1].trim();
  }
  return null;
}

function generateJwt(username, userId, secret) {
  try {
    const jwt = require('jsonwebtoken');
    return jwt.sign({ username, id: userId }, secret, { expiresIn: '365d' });
  } catch (_) {}
  // Fallback: manual HS256 (no external dep)
  const header  = Buffer.from(JSON.stringify({ alg: 'HS256', typ: 'JWT' })).toString('base64url');
  const payload = Buffer.from(JSON.stringify({
    username, id: userId,
    iat: Math.floor(Date.now() / 1000),
    exp: Math.floor(Date.now() / 1000) + 365 * 24 * 3600,
  })).toString('base64url');
  const sig = crypto.createHmac('sha256', secret).update(header + '.' + payload).digest('base64url');
  return `${header}.${payload}.${sig}`;
}

function apiRequest(method, urlPath, body) {
  return new Promise((resolve, reject) => {
    const bodyStr = body ? JSON.stringify(body) : null;
    const req = http.request({
      hostname: '127.0.0.1',
      port: PHONE_PORT,
      path: urlPath,
      method,
      headers: {
        'Content-Type': 'application/json',
        'Authorization': 'Bearer ' + jwtToken,
        ...(bodyStr ? { 'Content-Length': Buffer.byteLength(bodyStr) } : {}),
      },
    }, (res) => {
      const chunks = [];
      res.on('data', c => chunks.push(c));
      res.on('end', () => {
        try {
          resolve({ status: res.statusCode, body: JSON.parse(Buffer.concat(chunks).toString()) });
        } catch (_) {
          resolve({ status: res.statusCode, body: null });
        }
      });
    });
    req.on('error', reject);
    req.setTimeout(8000, () => req.destroy(new Error('timeout')));
    if (bodyStr) req.write(bodyStr);
    req.end();
  });
}

function escapeXml(str) {
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;');
}

// ── Steps ─────────────────────────────────────────────────────────────────────

async function checkAdb() {
  log('Checking adb...');
  const devices = adb('devices', { silent: true });
  if (devices === null) {
    fail('adb not found on PATH. Install Android SDK platform-tools and add to PATH.');
  }
  const connected = devices.split('\n').slice(1).some(l => l.includes('\tdevice'));
  if (!connected) {
    fail('No Android device detected. Connect phone via USB and enable USB debugging.');
  }
  ok('Phone detected');
}

async function setupReverse() {
  log(`Setting up adb reverse tcp:${PHONE_PORT}...`);
  const result = adb(`reverse tcp:${PHONE_PORT} tcp:${PHONE_PORT}`, { silent: true });
  if (result === null) {
    warn('adb reverse failed — phone will use remote API instead of local node');
    return false;
  }
  ok(`Port reversed: phone localhost:${PHONE_PORT} → this PC's port ${PHONE_PORT}`);
  return true;
}

async function ensureJwt() {
  if (jwtToken) {
    log('Using provided JWT');
    return;
  }

  log('Generating JWT from .env...');
  const secret = loadJwtSecret();
  if (!secret) {
    fail('No JWT provided and JWT_SECRET not found in .env. Pass --jwt <token>.');
  }

  // Try to load user ID from secretStore data if accessible
  const secretsPath = path.join(process.env.HOME || '', '.btcpc', 'secrets.json');
  let userId = '00000000-0000-0000-0000-000000000001';
  if (fs.existsSync(secretsPath)) {
    try {
      const data = JSON.parse(fs.readFileSync(secretsPath, 'utf8'));
      const u = data.users && data.users[account];
      if (u && u.user_id) userId = u.user_id;
    } catch (_) {}
  }

  jwtToken = generateJwt(account, userId, secret);
  ok('JWT generated (365-day expiry)');
}

async function pushPreferences() {
  log('Pushing preferences to phone...');

  // Read existing prefs first so we only update keys we care about
  const existingXml = adb(
    `shell run-as ${APP_PKG} cat /data/data/${APP_PKG}/shared_prefs/${PREFS_FILE}.xml`,
    { silent: true }
  ) || '';

  // Build minimal XML. If existing file is present, we try to merge;
  // otherwise write a fresh file with only the keys we set.
  // Strategy: extract all existing <string name="..."> entries, override our keys.
  const existingEntries = {};
  for (const m of existingXml.matchAll(/<string name="([^"]+)">([^<]*)<\/string>/g)) {
    existingEntries[m[1]] = m[2];
  }
  for (const m of existingXml.matchAll(/<boolean name="([^"]+)" value="([^"]+)"/g)) {
    existingEntries['__bool__' + m[1]] = m[2];
  }
  for (const m of existingXml.matchAll(/<int name="([^"]+)" value="([^"]+)"/g)) {
    existingEntries['__int__' + m[1]] = m[2];
  }

  // Override our keys
  existingEntries['account']  = account;
  existingEntries['jwt']      = jwtToken;
  existingEntries['apiUrl']   = apiUrl;

  // Rebuild XML
  let xml = `<?xml version='1.0' encoding='utf-8' standalone='yes' ?>\n<map>\n`;
  for (const [k, v] of Object.entries(existingEntries)) {
    if (k.startsWith('__bool__')) {
      xml += `    <boolean name="${escapeXml(k.slice(8))}" value="${v}" />\n`;
    } else if (k.startsWith('__int__')) {
      xml += `    <int name="${escapeXml(k.slice(7))}" value="${v}" />\n`;
    } else {
      xml += `    <string name="${escapeXml(k)}">${escapeXml(v)}</string>\n`;
    }
  }
  xml += `</map>\n`;

  // Write via run-as
  // Escape for shell: write to a temp file first, then copy
  const tmpPath = `/data/local/tmp/btcpc_prefs_${Date.now()}.xml`;
  const encoded = Buffer.from(xml).toString('base64');

  // Write via base64 decode (avoids shell escaping issues with the XML content)
  const writeCmd = `shell "echo '${encoded}' | base64 -d > ${tmpPath} && run-as ${APP_PKG} cp ${tmpPath} /data/data/${APP_PKG}/shared_prefs/${PREFS_FILE}.xml && rm ${tmpPath}"`;
  const writeResult = adb(writeCmd, { silent: true });
  if (writeResult === null) {
    // Fallback: try direct run-as write
    const directCmd = `shell run-as ${APP_PKG} sh -c "printf '%s' '${xml.replace(/'/g, "'\\''")}' > /data/data/${APP_PKG}/shared_prefs/${PREFS_FILE}.xml"`;
    const direct = adb(directCmd, { silent: true });
    if (direct === null) {
      warn('Could not write preferences via adb run-as. Manually enter settings in the app:');
      warn(`  Account: ${account}`);
      warn(`  JWT: ${jwtToken.slice(0, 40)}...`);
      warn(`  API URL: ${apiUrl}`);
      return;
    }
  }

  ok(`Pushed: account=${account}, apiUrl=${apiUrl}, jwt=<set>`);
  log('  Restart the BTCPC app on your phone for settings to take effect.');
}

async function registerSensor() {
  const sensorId = `${account}/${sensorName}`;
  log(`Checking sensor ${sensorId}...`);

  // Check if already exists
  try {
    const check = await apiRequest('GET', `/api/sensors/${encodeURIComponent(sensorId)}`);
    if (check.status === 200) {
      ok(`Sensor ${sensorId} already registered`);
      return;
    }
  } catch (_) {}

  // Register it
  log(`Registering sensor ${sensorId}...`);
  try {
    const res = await apiRequest('POST', '/api/sensors', {
      name:     sensorName,
      type:     'rf-scanner',
      unit:     'count',
      decimals: 0,
      region:   'us-west',
      hardware_model: 'flipper-zero',
      account:  account,  // fallback if JWT lookup doesn't populate req.user
    });
    if (res.status === 201 || res.status === 200) {
      ok(`Sensor ${sensorId} registered`);
    } else if (res.status === 409) {
      ok(`Sensor ${sensorId} already exists`);
    } else {
      warn(`Sensor registration returned ${res.status}: ${JSON.stringify(res.body)}`);
    }
  } catch (e) {
    warn(`Sensor registration failed: ${e.message}`);
    warn('  Make sure the BTCPC node is running on port 4242 (systemctl --user status btcpc-miner)');
  }
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  log('BTCPC Phone Setup Tool');
  log(`Account: ${account} | Sensor: ${account}/${sensorName}`);
  log('');

  await checkAdb();
  await setupReverse();
  await ensureJwt();
  await pushPreferences();
  await registerSensor();

  log('');
  log('Setup complete!');
  log('');
  log('Next steps:');
  log('  1. Force-stop and restart the BTCPC app on your phone');
  log(`  2. On the Flipper, run BTCPC > BTCPC BLE Relay`);
  log('  3. Phone will auto-connect to Flipper via BLE and relay sensor data');
  log(`  4. Check readings: curl http://127.0.0.1:4242/api/sensors/${encodeURIComponent(account + '/' + sensorName)}`);
}

main().catch(e => { fail(e.message); });
