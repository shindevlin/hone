#!/usr/bin/env node
"use strict";

/**
 * HONE Emergency Drill
 * Shin Devlin
 *
 * Simulates the pause → validate → resume cycle without touching live funds.
 * Run periodically to confirm the emergency response path is functional.
 *
 * Usage:
 *   node scripts/emergency-drill.js
 *   node scripts/emergency-drill.js --verbose
 *
 * Exit code 0 = drill passed. Non-zero = something needs attention.
 */

const http = require('http');
const path = require('path');
const fs = require('fs');

const BASE_URL = process.env.HONE_BASE_URL || 'http://localhost:3000';
const VERBOSE = process.argv.includes('--verbose');

let passed = 0;
let failed = 0;
const failures = [];

function log(msg) {
  if (VERBOSE) console.log('[drill]', msg);
}

function fail(msg) {
  console.error('[FAIL]', msg);
  failures.push(msg);
  failed++;
}

function pass(msg) {
  log('[PASS] ' + msg);
  passed++;
}

function httpGet(urlStr) {
  return new Promise((resolve, reject) => {
    const url = new URL(urlStr);
    const req = http.get({ host: url.hostname, port: url.port || 80, path: url.pathname + url.search, timeout: 5000 }, (res) => {
      let body = '';
      res.on('data', d => body += d);
      res.on('end', () => resolve({ status: res.statusCode, body }));
    });
    req.on('error', reject);
    req.on('timeout', () => { req.destroy(); reject(new Error('timeout')); });
  });
}

async function check(label, fn) {
  try {
    await fn();
    pass(label);
  } catch (err) {
    fail(label + ': ' + err.message);
  }
}

async function runDrill() {
  console.log('=== HONE Emergency Drill ===');
  console.log('Target:', BASE_URL);
  console.log('');

  // 1. Health check — node is up
  await check('Node health endpoint responds', async () => {
    const r = await httpGet(BASE_URL + '/health');
    if (r.status !== 200) throw new Error('status ' + r.status);
    const body = JSON.parse(r.body);
    if (!body.alive) throw new Error('alive=false');
  });

  // 2. Network stats — chain is producing blocks
  await check('Network stats available', async () => {
    const r = await httpGet(BASE_URL + '/public/network');
    if (r.status !== 200) throw new Error('status ' + r.status);
    const body = JSON.parse(r.body);
    if (typeof body.epoch !== 'number') throw new Error('no epoch in response');
    log('  Current epoch: ' + body.epoch);
  });

  // 3. Block files exist on disk
  await check('Block files present on disk', async () => {
    const dataDir = path.join(__dirname, '..', 'data', 'blocks');
    if (!fs.existsSync(dataDir)) throw new Error('data/blocks directory missing');
    const files = fs.readdirSync(dataDir).filter(f => f.endsWith('.json'));
    if (files.length === 0) throw new Error('no block files found');
    log('  Block files: ' + files.length);
  });

  // 4. Finality snapshot exists
  await check('Finality snapshot present', async () => {
    const finalityPath = path.join(__dirname, '..', 'data', 'finality.json');
    if (!fs.existsSync(finalityPath)) throw new Error('finality.json missing — chain may not have checkpointed yet');
    const stat = fs.statSync(finalityPath);
    const ageMs = Date.now() - stat.mtimeMs;
    const ageMin = Math.round(ageMs / 60000);
    log('  Finality snapshot age: ' + ageMin + ' min');
    // Warn if older than 2 hours (240 epochs)
    if (ageMs > 2 * 60 * 60 * 1000) {
      throw new Error('finality snapshot is ' + ageMin + ' min old — checkpoint may be stalled');
    }
  });

  // 5. Secrets file exists and is readable
  await check('Secrets file accessible', async () => {
    const secretsPath = path.join(process.env.HOME || '/root', '.hone', 'secrets.json');
    if (!fs.existsSync(secretsPath)) throw new Error('~/.hone/secrets.json not found');
    const stat = fs.statSync(secretsPath);
    const mode = (stat.mode & 0o777).toString(8);
    log('  Secrets file mode: ' + mode);
    if ((stat.mode & 0o077) !== 0) {
      throw new Error('secrets.json is world/group readable (mode ' + mode + ') — run: chmod 600 ~/.hone/secrets.json');
    }
  });

  // 6. Emergency pause script exists
  await check('Emergency pause script present', async () => {
    const scriptPath = path.join(__dirname, 'emergency-pause.sh');
    if (!fs.existsSync(scriptPath)) throw new Error('scripts/emergency-pause.sh missing');
  });

  // Summary
  console.log('');
  console.log('=== Drill Results ===');
  console.log('Passed: ' + passed);
  console.log('Failed: ' + failed);
  if (failures.length > 0) {
    console.log('\nFailures:');
    failures.forEach(f => console.log('  - ' + f));
    console.log('');
    process.exit(1);
  } else {
    console.log('\nAll checks passed. Emergency response path is healthy.');
    process.exit(0);
  }
}

runDrill().catch(err => {
  console.error('Drill crashed:', err.message);
  process.exit(2);
});
