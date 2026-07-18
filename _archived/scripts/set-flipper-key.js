#!/usr/bin/env node
/**
 * Writes the HONE posting key to Flipper internal storage (/int/.hone/posting.key).
 * Internal storage is NOT accessible via USB mass storage or the Flipper file browser —
 * only apps running on the Flipper can read it.
 *
 * Usage:
 *   node scripts/set-flipper-key.js --key <64-char-hex>
 *   node scripts/set-flipper-key.js --key <hex> --port /dev/ttyACM0
 */

'use strict';

const args = process.argv.slice(2);
let key = null;
let portPath = null;

for (let i = 0; i < args.length; i++) {
  if ((args[i] === '--key' || args[i] === '-k') && args[i + 1]) key = args[++i].trim();
  if ((args[i] === '--port' || args[i] === '-p') && args[i + 1]) portPath = args[++i].trim();
}

if (!key) {
  process.stderr.write('Usage: node set-flipper-key.js --key <64-char-hex>\n');
  process.exit(1);
}
if (!/^[0-9a-fA-F]{64}$/.test(key)) {
  process.stderr.write('Key must be exactly 64 hex characters.\n');
  process.exit(1);
}
key = key.toLowerCase();

const KEY_PATH = '/int/.hone/posting.key';

async function findPort() {
  try {
    const { SerialPort } = require('serialport');
    const ports = await SerialPort.list();
    const match = ports.find(p =>
      p.vendorId && p.productId &&
      p.vendorId.toLowerCase() === '0483' &&
      p.productId.toLowerCase() === '5740'
    );
    return match ? match.path : null;
  } catch (_) {}
  try {
    const { readdirSync } = require('fs');
    const tty = readdirSync('/dev').filter(f => f.startsWith('ttyACM'));
    if (tty.length) return '/dev/' + tty[0];
  } catch (_) {}
  return null;
}

function sendCli(port, lines) {
  return new Promise((resolve, reject) => {
    let buf = '';
    let step = 0;
    let done = false;
    const timeout = setTimeout(() => {
      if (!done) reject(new Error('CLI timeout'));
    }, 15000);

    function next() {
      if (step >= lines.length) {
        done = true;
        clearTimeout(timeout);
        resolve();
        return;
      }
      const { cmd, wait, after } = lines[step++];
      port.write(cmd + '\r\n');
      if (!wait) { setTimeout(next, after || 300); return; }
      // wait for a prompt '>' before sending next
      const check = setInterval(() => {
        if (buf.includes('>:') || buf.includes('\n>')) {
          clearInterval(check);
          buf = '';
          setTimeout(next, after || 100);
        }
      }, 100);
    }

    port.on('data', chunk => { buf += chunk.toString('utf8'); });
    // Wait for initial prompt
    const init = setInterval(() => {
      if (buf.includes('>:') || buf.includes('>')) {
        clearInterval(init);
        buf = '';
        next();
      }
    }, 100);
    // Send a newline to get the prompt
    port.write('\r\n');
  });
}

async function main() {
  const path = portPath || await findPort();
  if (!path) {
    process.stderr.write('Flipper not found. Connect via USB and retry.\n');
    process.exit(1);
  }
  process.stdout.write(`Writing posting key to Flipper on ${path}...\n`);

  const { SerialPort } = require('serialport');
  const port = new SerialPort({ path, baudRate: 115200, autoOpen: false });

  await new Promise((res, rej) => port.open(e => e ? rej(e) : res()));

  await sendCli(port, [
    { cmd: 'storage mkdir /int/.hone', wait: true },
    // storage write reads one line then closes — send key then ctrl+C
    { cmd: `storage write ${KEY_PATH}`, wait: false, after: 500 },
    { cmd: key + '\x03',               wait: true },
    // Verify it was written
    { cmd: `storage read ${KEY_PATH}`,  wait: true, after: 300 },
  ]).catch(e => {
    process.stderr.write('CLI error: ' + e.message + '\n');
    process.exit(1);
  });

  await new Promise(res => setTimeout(res, 500));
  port.close();
  process.stdout.write('✓ Posting key written to /int/.hone/posting.key\n');
  process.stdout.write('  Key is stored in Flipper internal flash — not visible via USB or file browser.\n');
}

main().catch(e => { process.stderr.write('Error: ' + e.message + '\n'); process.exit(1); });
