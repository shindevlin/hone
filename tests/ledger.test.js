// Phase E: LedgerEntry, Wallet, Epoch models removed — ledger uses stateStore directly
jest.mock('../src/models/User', () => ({
  findOne: jest.fn()
}));

const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args)
}));

const fs = require('fs');
const os = require('os');
const path = require('path');

// v2.13.1: _persist also appends to <BTCPC_DATA_DIR>/pending-entries.jsonl
// so that entries created in the API server process are picked up by the
// miner process on flush. Tests use an isolated data dir per worker so
// parallel jest runs don't race.
const ISOLATED_DATA_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'btcpc-ledger-test-'));
process.env.BTCPC_DATA_DIR = ISOLATED_DATA_DIR;

const User = require('../src/models/User');
const ledger = require('../src/services/ledger');

const PENDING_FILE = path.join(ISOLATED_DATA_DIR, 'pending-entries.jsonl');
function wipePendingFile() {
  try { if (fs.existsSync(PENDING_FILE)) fs.unlinkSync(PENDING_FILE); } catch (_) {}
}

describe('ledger service', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    wipePendingFile();
    ledger.flushPendingEntries();
    wipePendingFile();
  });

  afterAll(() => {
    wipePendingFile();
    try { fs.rmSync(ISOLATED_DATA_DIR, { recursive: true, force: true }); } catch (_) {}
  });

  test('recordTransfer rejects self-transfers before touching the mempool', async () => {
    await expect(ledger.recordTransfer('alice', 'alice', 1, 'BTCPC', null, 1)).rejects.toThrow('Cannot transfer to self');
    expect(mockMempoolSubmit).not.toHaveBeenCalled();
  });

  test('recordTransfer writes a ledger entry to pending store', async () => {
    mockMempoolSubmit.mockReturnValue({ accepted: true });

    const entry = await ledger.recordTransfer('alice', 'bob', 7, 'BTCPC', null, 42, 'memo');

    expect(mockMempoolSubmit).toHaveBeenCalled();
    // Phase E: recordTransfer returns a plain object (no Mongoose doc).
    expect(entry).toEqual(expect.objectContaining({
      type: 'TRANSFER',
      from: 'alice',
      to: 'bob',
      amount: 7,
      epoch: 42
    }));
  });

  test('getBalance reads from stateStore', async () => {
    const stateStore = require('../src/chain/stateStore');
    stateStore.resetAll();
    // Credit 13.5, debit 3.25 — net 10.25
    stateStore.applyEntry({ type: 'FAUCET', to: 'alice', token: 'BTCPC', amount: 13.5, epoch: 1, timestamp: 1 });
    stateStore.applyEntry({ type: 'TRANSFER', from: 'alice', to: 'bob', token: 'BTCPC', amount: 3.25, epoch: 1, timestamp: 2 });

    const balance = await ledger.getBalance('alice', 'BTCPC');
    expect(balance).toBe(10.25);
  });

  test('flushPendingEntries returns and clears pending entries', async () => {
    mockMempoolSubmit.mockReturnValue({ accepted: true });

    await ledger.recordTransfer('alice', 'bob', 1, 'BTCPC', null, 1, null);

    const firstFlush = ledger.flushPendingEntries();
    const secondFlush = ledger.flushPendingEntries();

    expect(firstFlush).toHaveLength(1);
    expect(secondFlush).toHaveLength(0);
  });

  test('getCurrentEpoch returns stateStore chain height or zero', async () => {
    const stateStore = require('../src/chain/stateStore');
    stateStore.resetAll();
    stateStore.setChainHeight(77);
    await expect(ledger.getCurrentEpoch()).resolves.toBe(77);

    stateStore.resetAll();
    await expect(ledger.getCurrentEpoch()).resolves.toBe(0);
  });
});
