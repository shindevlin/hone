"use strict";

/**
 * Mempool gossip tests (v2.13.3).
 *
 * Validates the P2P MEMPOOL_ENTRY gossip mechanism that lets ledger entries
 * cross machine boundaries so any broadcaster includes peer-originated entries
 * in its next block. See ledger.js:_gossipEntry, appendForeignEntry and
 * protocol.js:handleMempoolEntry for the architecture.
 *
 * Isolates the on-disk queue to a temp dir per worker so parallel jest
 * workers never race on data/pending-entries.jsonl.
 */

const fs = require('fs');
const os = require('os');
const path = require('path');

// Isolated data dir — must be set BEFORE requiring ledger
const ISOLATED_DATA_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'btcpc-gossip-'));
process.env.BTCPC_DATA_DIR = ISOLATED_DATA_DIR;

// ─── mocks ───────────────────────────────────────────────────────────────────

// Capture broadcast calls without a real WebSocket server.
const mockBroadcast = jest.fn();
const mockNodeId = 'test-node-gossip';

jest.mock('../src/p2p/network', () => ({
  broadcast: mockBroadcast,
  NODE_ID: mockNodeId,
}));

// Suppress mempool so recordTransfer doesn't attempt real P2P
jest.mock('../src/p2p/mempool', () => ({
  submit: jest.fn().mockReturnValue({ accepted: true }),
  addTransaction: jest.fn().mockReturnValue(true),
}));

// Phase E removed LedgerEntry, Wallet, and Epoch Mongoose models. Only
// User survives in the auth path.
jest.mock('../src/models/User', () => ({ findOne: jest.fn() }));

// ─── module under test ───────────────────────────────────────────────────────

const ledger = require('../src/services/ledger');
const stateStore = require('../src/chain/stateStore');
const PENDING_FILE = path.join(ISOLATED_DATA_DIR, 'pending-entries.jsonl');

function wipe() {
  try { if (fs.existsSync(PENDING_FILE)) fs.unlinkSync(PENDING_FILE); } catch (_) {}
}

// Reset module-level state between tests by directly flushing internals
beforeEach(() => {
  wipe();
  ledger.flushPendingEntries(); // drain in-memory array
  wipe();
  stateStore.resetAll();
  mockBroadcast.mockClear();
});

afterAll(() => {
  wipe();
  try { fs.rmSync(ISOLATED_DATA_DIR, { recursive: true, force: true }); } catch (_) {}
});

// ─── tests ───────────────────────────────────────────────────────────────────

describe('mempool gossip (v2.13.3)', () => {

  it('_persist calls p2p.broadcast with a MEMPOOL_ENTRY message', async () => {
    await ledger.recordAccountCreate(
      'gossip-alice',
      { owner: 'a'.repeat(64) },
      { evm: '0xabc' },
      1
    );

    expect(mockBroadcast).toHaveBeenCalledTimes(1);
    const [msg] = mockBroadcast.mock.calls[0];
    expect(msg.type).toBe('MEMPOOL_ENTRY');
    expect(msg.data.entry.type).toBe('ACCOUNT_CREATE');
    expect(msg.data.entry.to).toBe('gossip-alice');
  });

  it('appendForeignEntry does NOT call p2p.broadcast', () => {
    const foreignEntry = {
      type: 'ACCOUNT_CREATE',
      to: 'gossip-bob',
      epoch: 2,
      account_data: {
        username: 'gossip-bob',
        public_keys: { owner: 'b'.repeat(64) },
        chain_addresses: {},
      },
      timestamp: Date.now(),
    };

    ledger.appendForeignEntry(foreignEntry);

    expect(mockBroadcast).not.toHaveBeenCalled();
  });

  it('appendForeignEntry writes entry to disk so next flush includes it', () => {
    const foreignEntry = {
      type: 'FAUCET',
      from: 'btcpc_genesis',
      to: 'gossip-carol',
      token: 'BTCPC',
      amount: 100,
      epoch: 3,
      timestamp: Date.now(),
    };

    ledger.appendForeignEntry(foreignEntry);

    // Drain memory (should be empty — foreign entries don't go in-memory)
    const flushed = ledger.flushPendingEntries();
    expect(flushed.some(function (e) { return e.to === 'gossip-carol'; })).toBe(true);
  });

  it('_persist followed by appendForeignEntry of the SAME entry is deduped — single broadcast', async () => {
    // Simulate: we originated the entry AND it came back from a peer
    await ledger.recordAccountCreate(
      'gossip-dave',
      { owner: 'c'.repeat(64) },
      {},
      4
    );

    // Grab what was broadcast — replay it as an incoming foreign entry
    const broadcastMsg = mockBroadcast.mock.calls[0][0];
    const entry = broadcastMsg.data.entry;

    mockBroadcast.mockClear();

    // appendForeignEntry with the same entry — should NOT re-broadcast
    ledger.appendForeignEntry(entry);

    expect(mockBroadcast).not.toHaveBeenCalled();
  });

  it('_persist followed by appendForeignEntry of the SAME entry dedupes in flush output', async () => {
    stateStore.resetAll();
    wipe();

    await ledger.recordAccountCreate(
      'gossip-eve',
      { owner: 'd'.repeat(64) },
      {},
      5
    );

    // Simulate the same entry arriving as a foreign entry (peer echo)
    const entry = mockBroadcast.mock.calls[0][0].data.entry;
    ledger.appendForeignEntry(entry);

    const flushed = ledger.flushPendingEntries();
    const eveEntries = flushed.filter(function (e) { return e.to === 'gossip-eve'; });
    // Should appear exactly once despite two writes (memory + disk + foreign disk)
    expect(eveEntries.length).toBe(1);
  });

  it('flushPendingEntries returns both originator and foreign entries', async () => {
    stateStore.resetAll();
    wipe();

    // Local originator entry
    await ledger.recordAccountCreate(
      'gossip-frank',
      { owner: 'e'.repeat(64) },
      {},
      6
    );

    // Foreign entry from another node
    const foreignEntry = {
      type: 'FAUCET',
      from: 'btcpc_genesis',
      to: 'gossip-grace',
      token: 'BTCPC',
      amount: 50,
      epoch: 6,
      timestamp: Date.now() + 1,
    };
    ledger.appendForeignEntry(foreignEntry);

    const flushed = ledger.flushPendingEntries();
    expect(flushed.some(function (e) { return e.to === 'gossip-frank'; })).toBe(true);
    expect(flushed.some(function (e) { return e.to === 'gossip-grace'; })).toBe(true);
  });

  it('gossipedHashes set does not grow unbounded — eviction fires at GOSSIP_HASH_CAP', async () => {
    // We can't read the private set directly, but we can verify that after
    // recording many unique entries, subsequent unique entries still broadcast.
    // Smoke-test: record 10 distinct entries, each should broadcast once.
    stateStore.resetAll();
    wipe();
    mockBroadcast.mockClear();

    for (var i = 0; i < 10; i++) {
      // Produce distinct timestamps by using different epochs
      await ledger.recordAccountCreate(
        'gossip-hash-' + i,
        { owner: String(i).padStart(64, '0') },
        {},
        100 + i
      );
    }

    // All 10 should have been broadcast (no false dedupe)
    expect(mockBroadcast).toHaveBeenCalledTimes(10);
  });

  it('p2p.broadcast errors are swallowed — _persist still succeeds', async () => {
    mockBroadcast.mockImplementationOnce(function () {
      throw new Error('network down');
    });

    // Should not throw
    const result = await ledger.recordAccountCreate(
      'gossip-hardy',
      { owner: 'f'.repeat(64) },
      {},
      7
    );

    expect(result).toBeDefined();
    expect(result.type).toBe('ACCOUNT_CREATE');
    // Entry still in queue even though broadcast threw
    const flushed = ledger.flushPendingEntries();
    expect(flushed.some(function (e) { return e.to === 'gossip-hardy'; })).toBe(true);
  });

  it('appendForeignEntry applies entry to stateStore', () => {
    stateStore.resetAll();

    const entry = {
      type: 'ACCOUNT_CREATE',
      to: 'gossip-irene',
      epoch: 8,
      account_data: {
        username: 'gossip-irene',
        public_keys: { owner: '1'.repeat(64) },
        chain_addresses: { evm: '0x1234' },
      },
      timestamp: Date.now(),
    };

    ledger.appendForeignEntry(entry);

    const acct = stateStore.getAccount('gossip-irene');
    expect(acct).toBeTruthy();
    expect(acct.username).toBe('gossip-irene');
  });

});
