"use strict";

/**
 * Ledger file-queue bridge tests (v2.13.1).
 *
 * Validates the on-disk pending-entries.jsonl bridge that lets API
 * server entries reach the miner's block flush. See ledger.js:_persist
 * and flushPendingEntries for the architecture notes.
 *
 * These tests manipulate the real data/pending-entries.jsonl path so
 * they wipe it before and after every case to stay isolated.
 */

const fs = require('fs');
const os = require('os');
const path = require('path');
const { execSync } = require('child_process');

// Isolate this worker's data dir so parallel jest runs don't race on
// the shared data/pending-entries.jsonl file.
const ISOLATED_DATA_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'hone-ledger-bridge-'));
process.env.HONE_DATA_DIR = ISOLATED_DATA_DIR;

// Mock mempool so recordTransfer doesn't try to P2P-broadcast
jest.mock('../src/p2p/mempool', () => ({
  submit: jest.fn().mockReturnValue({ accepted: true }),
  addTransaction: jest.fn().mockReturnValue(true),
}));

// Phase E: LedgerEntry, Wallet, Epoch models removed — only User mock remains
jest.mock('../src/models/User', () => ({ findOne: jest.fn() }));

const ledger = require('../src/services/ledger');
const stateStore = require('../src/chain/stateStore');

const PENDING_FILE = path.join(ISOLATED_DATA_DIR, 'pending-entries.jsonl');

function wipe() {
  try { if (fs.existsSync(PENDING_FILE)) fs.unlinkSync(PENDING_FILE); } catch (_) {}
}

describe('ledger file-queue bridge (v2.13.1)', () => {
  beforeEach(() => {
    wipe();
    ledger.flushPendingEntries(); // drain any stragglers
    wipe();
    stateStore.resetAll();
  });

  afterAll(() => {
    wipe();
    try { fs.rmSync(ISOLATED_DATA_DIR, { recursive: true, force: true }); } catch (_) {}
  });

  it('appends entries to data/pending-entries.jsonl when _persist runs', async () => {
    await ledger.recordAccountCreate(
      'alice',
      { owner: 'a'.repeat(64), active: 'b'.repeat(64), posting: 'c'.repeat(64), memo: 'd'.repeat(64) },
      { evm: '0xabc' },
      1
    );

    expect(fs.existsSync(PENDING_FILE)).toBe(true);
    const lines = fs.readFileSync(PENDING_FILE, 'utf8').split('\n').filter(Boolean);
    expect(lines.length).toBe(1);
    const parsed = JSON.parse(lines[0]);
    expect(parsed.type).toBe('ACCOUNT_CREATE');
    expect(parsed.to).toBe('alice');
    expect(parsed.account_data.public_keys.owner).toBe('a'.repeat(64));
  });

  it('flushPendingEntries drains the file and deletes it', async () => {
    await ledger.recordAccountCreate('bob', { owner: 'e'.repeat(64) }, {}, 2);
    expect(fs.existsSync(PENDING_FILE)).toBe(true);

    const flushed = ledger.flushPendingEntries();
    expect(flushed.length).toBeGreaterThanOrEqual(1);
    expect(flushed.some(function (e) { return e.to === 'bob'; })).toBe(true);
    expect(fs.existsSync(PENDING_FILE)).toBe(false);
  });

  it('dedupes between in-memory and disk for single-process writers', async () => {
    // In a single-process scenario, _persist() writes to BOTH the in-memory
    // pendingEntries array and the on-disk file. The same entry appears in
    // both sources when we flush. The dedupe logic should collapse it to 1.
    await ledger.recordAccountCreate('charlie', { owner: 'f'.repeat(64) }, {}, 3);
    const flushed = ledger.flushPendingEntries();
    const charlieEntries = flushed.filter(function (e) { return e.to === 'charlie'; });
    expect(charlieEntries.length).toBe(1);
  });

  it('reads cross-process entries from the file even with empty in-memory queue', () => {
    // Simulate an entry appearing on disk from a different Node process.
    // Our in-memory pendingEntries starts empty; only the file has data.
    const externalEntry = {
      type: 'ACCOUNT_CREATE',
      to: 'dave',
      epoch: 4,
      account_data: {
        username: 'dave',
        public_keys: { owner: '1'.repeat(64) },
        chain_addresses: {},
      },
      timestamp: Date.now(),
    };

    // Make sure data dir exists
    const dataDir = path.dirname(PENDING_FILE);
    if (!fs.existsSync(dataDir)) fs.mkdirSync(dataDir, { recursive: true });
    fs.writeFileSync(PENDING_FILE, JSON.stringify(externalEntry) + '\n');

    const flushed = ledger.flushPendingEntries();
    expect(flushed.length).toBe(1);
    expect(flushed[0].to).toBe('dave');
    expect(fs.existsSync(PENDING_FILE)).toBe(false);
  });

  it('handles corrupt lines in the file gracefully', () => {
    const dataDir = path.dirname(PENDING_FILE);
    if (!fs.existsSync(dataDir)) fs.mkdirSync(dataDir, { recursive: true });
    const goodEntry = JSON.stringify({
      type: 'FAUCET',
      to: 'eve',
      token: 'HONE',
      amount: 10,
      epoch: 5,
      timestamp: Date.now(),
    });
    fs.writeFileSync(
      PENDING_FILE,
      '{garbage\n' + goodEntry + '\n{"incomplete":\n'
    );

    const flushed = ledger.flushPendingEntries();
    // The one valid line should come through; corrupt lines skipped.
    expect(flushed.length).toBe(1);
    expect(flushed[0].to).toBe('eve');
  });

  it('flush on empty file returns an empty array without crashing', () => {
    expect(fs.existsSync(PENDING_FILE)).toBe(false);
    const flushed = ledger.flushPendingEntries();
    expect(Array.isArray(flushed)).toBe(true);
    expect(flushed.length).toBe(0);
  });

  it('cross-process: a separate node invocation writes, we drain', () => {
    // Simulates the API server process calling _persist while we act as
    // the miner process. Uses execSync with a temp script file to invoke
    // a fresh Node process that shares nothing in memory with this test.
    const scriptPath = path.join(ISOLATED_DATA_DIR, 'writer.js');
    const ledgerPath = path.resolve(__dirname, '..', 'src', 'services', 'ledger');
    const writerScript = [
      "process.env.HONE_DATA_DIR = " + JSON.stringify(ISOLATED_DATA_DIR) + ";",
      "const ledger = require(" + JSON.stringify(ledgerPath) + ");",
      "(async () => {",
      "  await ledger.recordAccountCreate('frank', { owner: '9'.repeat(64) }, { evm: '0xfeed' }, 6);",
      "})();"
    ].join('\n');
    fs.writeFileSync(scriptPath, writerScript);
    try {
      execSync('node ' + JSON.stringify(scriptPath), {
        cwd: path.resolve(__dirname, '..'),
        stdio: 'pipe',
        env: Object.assign({}, process.env, { HONE_DATA_DIR: ISOLATED_DATA_DIR }),
      });
    } finally {
      try { fs.unlinkSync(scriptPath); } catch (_) {}
    }

    // File should now exist with frank's entry, written by that other process.
    expect(fs.existsSync(PENDING_FILE)).toBe(true);

    // Drain it from THIS process — our in-memory pendingEntries is empty.
    const flushed = ledger.flushPendingEntries();
    expect(flushed.length).toBe(1);
    expect(flushed[0].type).toBe('ACCOUNT_CREATE');
    expect(flushed[0].to).toBe('frank');
    expect(flushed[0].account_data.chain_addresses.evm).toBe('0xfeed');
    expect(fs.existsSync(PENDING_FILE)).toBe(false);
  });
});

describe('crash-mid-drain recovery (.draining-* stale files)', () => {
  function drainPath(pid, ts) {
    return path.join(ISOLATED_DATA_DIR, 'pending-entries.jsonl.draining-' + pid + '-' + ts);
  }

  function writeEntry(filePath, obj) {
    const dir = path.dirname(filePath);
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(filePath, JSON.stringify(obj) + '\n');
  }

  beforeEach(() => {
    // Wipe normal pending file and any leftover drain files
    try { fs.unlinkSync(PENDING_FILE); } catch (_) {}
    try {
      fs.readdirSync(ISOLATED_DATA_DIR)
        .filter(f => f.startsWith('pending-entries.jsonl.draining-'))
        .forEach(f => fs.unlinkSync(path.join(ISOLATED_DATA_DIR, f)));
    } catch (_) {}
    ledger.flushPendingEntries();
  });

  it('recovers entries from a single stale drain file', () => {
    writeEntry(drainPath(99999, 1234567890), {
      type: 'FAUCET', to: 'crash-ghost', token: 'HONE', amount: 5, epoch: 20, timestamp: Date.now(),
    });

    expect(fs.existsSync(PENDING_FILE)).toBe(false);

    const flushed = ledger.flushPendingEntries();
    expect(flushed.some(e => e.to === 'crash-ghost')).toBe(true);
    // Stale file must be cleaned up
    expect(fs.existsSync(drainPath(99999, 1234567890))).toBe(false);
  });

  it('recovers entries from multiple stale drain files (multiple crashed processes)', () => {
    writeEntry(drainPath(11111, 100), { type: 'FAUCET', to: 'ghost-a', token: 'HONE', amount: 1, epoch: 21, timestamp: Date.now() });
    writeEntry(drainPath(22222, 200), { type: 'FAUCET', to: 'ghost-b', token: 'HONE', amount: 2, epoch: 21, timestamp: Date.now() });

    const flushed = ledger.flushPendingEntries();
    const recipients = flushed.map(e => e.to);
    expect(recipients).toContain('ghost-a');
    expect(recipients).toContain('ghost-b');
    expect(fs.existsSync(drainPath(11111, 100))).toBe(false);
    expect(fs.existsSync(drainPath(22222, 200))).toBe(false);
  });

  it('skips corrupt lines in a stale drain file, keeps valid ones', () => {
    const p = drainPath(33333, 300);
    const good = JSON.stringify({ type: 'FAUCET', to: 'good-ghost', token: 'HONE', amount: 3, epoch: 22, timestamp: Date.now() });
    if (!fs.existsSync(ISOLATED_DATA_DIR)) fs.mkdirSync(ISOLATED_DATA_DIR, { recursive: true });
    fs.writeFileSync(p, '{corrupt\n' + good + '\n');

    const flushed = ledger.flushPendingEntries();
    expect(flushed.some(e => e.to === 'good-ghost')).toBe(true);
    expect(fs.existsSync(p)).toBe(false);
  });

  it('combines stale drain entries with entries from the normal pending file', () => {
    // Stale drain file from crashed old process
    writeEntry(drainPath(44444, 400), { type: 'FAUCET', to: 'old-process', token: 'HONE', amount: 1, epoch: 23, timestamp: Date.now() });
    // Fresh normal pending file written by a running process
    if (!fs.existsSync(ISOLATED_DATA_DIR)) fs.mkdirSync(ISOLATED_DATA_DIR, { recursive: true });
    fs.writeFileSync(PENDING_FILE,
      JSON.stringify({ type: 'FAUCET', to: 'new-process', token: 'HONE', amount: 2, epoch: 23, timestamp: Date.now() }) + '\n'
    );

    const flushed = ledger.flushPendingEntries();
    const recipients = flushed.map(e => e.to);
    expect(recipients).toContain('old-process');
    expect(recipients).toContain('new-process');
    expect(fs.existsSync(PENDING_FILE)).toBe(false);
    expect(fs.existsSync(drainPath(44444, 400))).toBe(false);
  });

  it('no-op when no stale drain files and no pending file', () => {
    const flushed = ledger.flushPendingEntries();
    expect(Array.isArray(flushed)).toBe(true);
    expect(flushed.length).toBe(0);
  });
});
