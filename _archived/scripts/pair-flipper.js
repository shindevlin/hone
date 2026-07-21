#!/usr/bin/env node
/**
 * HONE Flipper Pairing Tool
 *
 * One-shot pairing: reads the Flipper Zero BLE MAC from USB serial and
 * pushes it to the HONE Android app running on a connected phone.
 *
 * Requirements on the PC:
 *   - Flipper plugged in via USB running hone_relay.fap
 *   - Android phone plugged in via USB with USB debugging enabled
 *   - adb installed and on PATH (included in Android SDK platform-tools)
 *
 * Bundle as a standalone exe (no node required on end-user's machine):
 *   npm install -g pkg
 *   pkg scripts/pair-flipper.js -t node18-win,node18-linux,node18-macos \
 *       --output dist/honeflipper
 *   → dist/honeflipper.exe  (Windows)
 *     dist/honeflipper-linux
 *     dist/honeflipper-macos
 *
 * Usage:
 *   node scripts/pair-flipper.js
 *   node scripts/pair-flipper.js --port /dev/ttyACM0    (explicit Flipper port)
 *   node scripts/pair-flipper.js --port COM3             (Windows)
 *   node scripts/pair-flipper.js --timeout 20            (seconds to wait for MAC)
 */

'use strict';

const http         = require('http');
const crypto       = require('crypto');
const { execSync } = require('child_process');

// ── CLI args ──────────────────────────────────────────────────────────────────

const args = process.argv.slice(2);
let explicitPort = null;
let timeoutSec   = 15;

for (let i = 0; i < args.length; i++) {
  if ((args[i] === '--port' || args[i] === '-p') && args[i + 1]) {
    explicitPort = args[++i];
  } else if ((args[i] === '--timeout' || args[i] === '-t') && args[i + 1]) {
    timeoutSec = parseInt(args[++i]) || 15;
  }
}

const PHONE_PORT = 6942;

// ── Helpers ───────────────────────────────────────────────────────────────────

function log(msg)  { process.stdout.write('[hone-pair] ' + msg + '\n'); }
function err(msg)  { process.stderr.write('[hone-pair] ERROR: ' + msg + '\n'); }
function ok(msg)   { process.stdout.write('[hone-pair] ✓ ' + msg + '\n'); }
function fail(msg) { err(msg); process.exit(1); }

function postPair(mac) {
  return new Promise((resolve, reject) => {
    const body = JSON.stringify({ mac });
    const req = http.request({
      hostname: '127.0.0.1',
      port: PHONE_PORT,
      path: '/flipper/pair',
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(body) },
    }, (res) => {
      const chunks = [];
      res.on('data', c => chunks.push(c));
      res.on('end', () => {
        try {
          const json = JSON.parse(Buffer.concat(chunks).toString());
          if (res.statusCode === 200 && json.ok) resolve(json);
          else reject(new Error(`phone returned ${res.statusCode}: ${JSON.stringify(json)}`));
        } catch (_) {
          reject(new Error(`phone returned ${res.statusCode} (non-JSON)`));
        }
      });
    });
    req.on('error', reject);
    req.setTimeout(6000, () => { req.destroy(new Error('timeout posting to phone')); });
    req.end(body);
  });
}

function adb(args, { silent = false } = {}) {
  try {
    const out = execSync(`adb ${args}`, { stdio: silent ? 'pipe' : 'inherit', encoding: 'utf8' });
    return out ? out.trim() : '';
  } catch (e) {
    return null;
  }
}

function findFlipperPort() {
  // Try serialport if available (preferred — works cross-platform)
  try {
    const { SerialPort } = require('serialport');
    return SerialPort.list().then(ports => {
      const match = ports.find(p =>
        p.vendorId && p.vendorId.toLowerCase() === '0483' &&
        p.productId && p.productId.toLowerCase() === '5740'
      );
      return match ? match.path : null;
    });
  } catch (_) {}

  // Fallback: scan common paths on Linux/macOS
  const { readdirSync } = require('fs');
  try {
    const tty = readdirSync('/dev').filter(f => f.startsWith('ttyACM') || f.startsWith('ttyUSB'));
    if (tty.length > 0) return Promise.resolve('/dev/' + tty[0]);
  } catch (_) {}

  // Windows COM port heuristic via wmic
  try {
    const out = execSync(
      'wmic path Win32_PnPEntity where "Caption like \'%(COM%)\'" get Caption /format:list',
      { encoding: 'utf8', stdio: 'pipe' }
    );
    const match = out.match(/COM\d+/g);
    if (match) return Promise.resolve(match[0]);
  } catch (_) {}

  return Promise.resolve(null);
}

function readMacFromSerial(portPath, timeoutMs) {
  return new Promise((resolve, reject) => {
    let SerialPort;
    try { ({ SerialPort } = require('serialport')); } catch (_) {
      return reject(new Error(
        'serialport package not found.\n' +
        '  Run:  npm install serialport\n' +
        '  Then retry.'
      ));
    }

    const port = new SerialPort({ path: portPath, baudRate: 115200, autoOpen: false });
    let buf = '';
    let resolved = false;

    const timer = setTimeout(() => {
      if (!resolved) {
        port.close();
        reject(new Error(`No HONE_MAC: seen from ${portPath} within ${timeoutMs / 1000}s.\n` +
          '  Make sure hone_relay.fap is running on the Flipper.'));
      }
    }, timeoutMs);

    port.open(openErr => {
      if (openErr) {
        clearTimeout(timer);
        return reject(new Error(`Cannot open ${portPath}: ${openErr.message}`));
      }

      port.on('data', chunk => {
        buf += chunk.toString('utf8');
        let nl;
        while ((nl = buf.indexOf('\n')) !== -1) {
          const line = buf.slice(0, nl).trim();
          buf = buf.slice(nl + 1);
          if (line.startsWith('HONE_MAC:')) {
            const parts = line.split(' ');
            const mac = parts[0].slice('HONE_MAC:'.length).trim().toUpperCase();
            const sigPart = parts.find(p => p.startsWith('HONE_SIG:'));
            const sig = sigPart ? sigPart.slice('HONE_SIG:'.length).trim().toLowerCase() : null;
            if (mac.match(/^([0-9A-F]{2}:){5}[0-9A-F]{2}$/)) {
              resolved = true;
              clearTimeout(timer);
              port.close();
              resolve({ mac, sig });
            }
          }
        }
      });

      port.on('error', e => {
        if (!resolved) { clearTimeout(timer); reject(e); }
      });
    });
  });
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  log('HONE Flipper Pairing Tool');
  log('');

  // 1. Check adb is reachable
  log('Checking adb...');
  const adbDevices = adb('devices', { silent: true });
  if (adbDevices === null) {
    fail(
      'adb not found on PATH.\n' +
      '  Download Android SDK platform-tools:\n' +
      '  https://developer.android.com/studio/releases/platform-tools\n' +
      '  Then add the folder to your PATH and retry.'
    );
  }
  const phoneAttached = adbDevices && adbDevices.split('\n').slice(1).some(l => l.includes('\tdevice'));
  if (!phoneAttached) {
    fail(
      'No Android device detected by adb.\n' +
      '  1. Connect your phone via USB\n' +
      '  2. Enable Developer Options → USB Debugging\n' +
      '  3. Tap "Allow" when prompted on the phone\n' +
      '  Then retry.'
    );
  }
  ok('Phone detected via adb');

  // 2. Forward phone port to localhost
  log(`Forwarding phone port ${PHONE_PORT}...`);
  const fwd = adb(`forward tcp:${PHONE_PORT} tcp:${PHONE_PORT}`, { silent: true });
  if (fwd === null) fail('adb forward failed — is the phone unlocked?');
  ok('Port forwarded');

  // 3. Find Flipper serial port
  log('Looking for Flipper Zero serial port...');
  const portPath = explicitPort || await findFlipperPort();
  if (!portPath) {
    fail(
      'Flipper Zero not found on any serial port.\n' +
      '  1. Connect Flipper via USB\n' +
      '  2. Open hone_relay.fap on the Flipper\n' +
      '  3. Or specify the port manually:  --port /dev/ttyACM0\n' +
      '     (Windows: --port COM3)'
    );
  }
  ok(`Flipper found at ${portPath}`);

  // 4. Read MAC + signature
  log(`Waiting for BLE MAC from Flipper (up to ${timeoutSec}s)...`);
  log('  → If hone_relay.fap is not open yet, open it now.');
  const { mac, sig } = await readMacFromSerial(portPath, timeoutSec * 1000);
  ok(`Got BLE MAC: ${mac}`);
  if (sig) {
    log(`Signature present — verifying...`);
    // Caller can pass --posting-key <hex> to verify; skip if not provided
    const keyArg = args[args.indexOf('--posting-key') + 1] || args[args.indexOf('--key') + 1];
    if (keyArg && /^[0-9a-fA-F]{64}$/.test(keyArg)) {
      const expected = crypto.createHmac('sha256', Buffer.from(keyArg, 'hex'))
                             .update(mac).digest('hex');
      if (expected !== sig) {
        fail(`Signature mismatch — wrong posting key or tampered Flipper.\n  expected: ${expected}\n  got:      ${sig}`);
      }
      ok('Signature verified');
    } else {
      log('  (pass --posting-key <hex> to verify signature)');
    }
  } else {
    log('  No signature in announce — key not loaded on Flipper yet.');
    log('  Run: node scripts/set-flipper-key.js --key <hex>');
  }

  // 5. POST to phone
  log('Sending MAC to phone...');
  try {
    const result = await postPair(mac);
    ok(`Paired! Phone confirmed: ${result.mac}`);
    log('');
    log('Pairing complete. You can now unplug the Flipper from the PC.');
    log('On your phone: open the HONE app → Flipper tab to verify.');
  } catch (e) {
    fail(
      `Phone did not accept the MAC: ${e.message}\n` +
      '  Make sure the HONE app is running on your phone.'
    );
  }

  // 6. Clean up port forward
  adb(`forward --remove tcp:${PHONE_PORT}`, { silent: true });
}

main().catch(e => { err(e.message); process.exit(1); });
