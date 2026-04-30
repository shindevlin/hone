#!/usr/bin/env node
/**
 * smoke-testnet.js — verifies the Rust chain node is alive and producing a valid chain.
 *
 * Usage: node scripts/smoke-testnet.js [BASE_URL]
 * Env:   TESTNET_BASE_URL  (default: http://localhost:4242)
 *        API_BASE_URL      (same fallback)
 */

const BASE = (
  process.env.TESTNET_BASE_URL ||
  process.env.API_BASE_URL ||
  (process.argv[2] && !process.argv[2].startsWith('-') ? process.argv[2] : '') ||
  'http://localhost:4242'
).replace(/\/$/, '');

const RETRY_MAX    = 5;
const RETRY_DELAY  = 3000;
const TIMEOUT_MS   = 10000;

let passed = 0;
let failed = 0;

async function fetchWithTimeout(url, opts = {}) {
  const ctrl = new AbortController();
  const tid = setTimeout(() => ctrl.abort(), TIMEOUT_MS);
  try {
    const res = await fetch(url, { ...opts, signal: ctrl.signal });
    return res;
  } finally {
    clearTimeout(tid);
  }
}

async function retry(label, fn) {
  for (let attempt = 1; attempt <= RETRY_MAX; attempt++) {
    try {
      await fn();
      return;
    } catch (err) {
      if (attempt === RETRY_MAX) {
        console.error(`  FAIL [${label}]: ${err.message}`);
        failed++;
        return;
      }
      await new Promise(r => setTimeout(r, RETRY_DELAY));
    }
  }
}

async function check(label, fn) {
  process.stdout.write(`  checking ${label}... `);
  try {
    await fn();
    console.log('ok');
    passed++;
  } catch (err) {
    console.log(`FAIL: ${err.message}`);
    failed++;
  }
}

function assert(condition, msg) {
  if (!condition) throw new Error(msg);
}

async function main() {
  console.log(`\nSmoke: ${BASE}\n`);

  // 1. Health
  await check('/health', async () => {
    const res = await fetchWithTimeout(`${BASE}/health`);
    assert(res.ok, `HTTP ${res.status}`);
    const text = await res.text();
    assert(text.trim().toLowerCase().includes('ok'), `body: ${text.slice(0, 80)}`);
  });

  // 2. /api/latest — returns chain head with epoch and hash
  let latestEpoch = 0;
  await check('/api/latest', async () => {
    const res = await fetchWithTimeout(`${BASE}/api/latest`);
    assert(res.ok, `HTTP ${res.status}`);
    const j = await res.json();
    assert(typeof j.current_epoch === 'number', `missing current_epoch in ${JSON.stringify(j).slice(0,100)}`);
    latestEpoch = j.current_epoch;
  });

  // 3. Genesis block (epoch 0) — must exist and have a hash
  await check('/api/block/0 (genesis)', async () => {
    const res = await fetchWithTimeout(`${BASE}/api/block/0`);
    assert(res.ok, `HTTP ${res.status}`);
    const j = await res.json();
    assert(j.hash, `missing hash in ${JSON.stringify(j).slice(0,100)}`);
    assert(j.epoch === 0 || j.epoch_number === 0, `epoch != 0`);
  });

  // 4. Genesis account exists (shindevlin is in genesis.json)
  await check('/api/account/shindevlin', async () => {
    const res = await fetchWithTimeout(`${BASE}/api/account/shindevlin`);
    assert(res.ok, `HTTP ${res.status}`);
    const j = await res.json();
    assert(j.account || j.account_id || j.name || j.id, `unexpected shape: ${JSON.stringify(j).slice(0,100)}`);
  });

  // 5. Balance endpoint is responsive
  await check('/api/balance/shindevlin', async () => {
    const res = await fetchWithTimeout(`${BASE}/api/balance/shindevlin`);
    assert(res.ok, `HTTP ${res.status}`);
    const j = await res.json();
    assert('balance' in j || 'amount' in j || 'balances' in j || typeof j === 'number',
      `unexpected shape: ${JSON.stringify(j).slice(0,100)}`);
  });

  // 6. Inference jobs endpoint
  await check('/api/inference/jobs', async () => {
    const res = await fetchWithTimeout(`${BASE}/api/inference/jobs`);
    assert(res.ok, `HTTP ${res.status}`);
    const j = await res.json();
    assert(Array.isArray(j) || (j && typeof j === 'object'), `unexpected shape`);
  });

  // 7. Repeated /health reads (quorum — 3/3 must pass)
  let healthOk = 0;
  for (let i = 0; i < 3; i++) {
    try {
      const res = await fetchWithTimeout(`${BASE}/health`);
      if (res.ok) healthOk++;
    } catch (_) {}
    if (i < 2) await new Promise(r => setTimeout(r, 500));
  }
  await check('/health quorum (3/3)', async () => {
    assert(healthOk >= 3, `only ${healthOk}/3 reads returned ok`);
  });

  // Summary
  console.log(`\n${passed + failed} checks: ${passed} passed, ${failed} failed`);
  if (failed > 0) {
    process.exit(1);
  }
  console.log('All smoke checks passed.\n');
}

main().catch(err => {
  console.error('Smoke script error:', err);
  process.exit(1);
});
