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
const epochBandwidth = require('../src/services/epochBandwidth');

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
    epochBandwidth.resetAll();
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

  test('recordAuthorizedTransfer records private-auth metadata', async () => {
    mockMempoolSubmit.mockReturnValue({ accepted: true });

    const entry = await ledger.recordAuthorizedTransfer('alice', 'bob', 7, 'BTCPC', null, 42, 'memo', {
      signedBy: 'private_auth',
      requestId: 'req-1',
      threshold: 2,
      approvalCount: 2,
      factors: [{ factorId: 'f-1', chain: 'evm', commitment: 'abc' }]
    });

    expect(entry).toEqual(expect.objectContaining({
      type: 'TRANSFER',
      signed_by: 'private_auth',
      authorization: expect.objectContaining({
        requestId: 'req-1',
        threshold: 2,
        approvalCount: 2
      })
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

  test('recordFaucet issues delegated credit, not spendable wallet balance', async () => {
    const stateStore = require('../src/chain/stateStore');
    stateStore.resetAll();
    stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'bob', epoch: 1, account_data: { public_keys: {}, chain_addresses: {} } });
    stateStore.applyEntry({ type: 'FAUCET', to: 'btcpc_faucet', token: 'BTCPC', amount: 10, epoch: 1, timestamp: 1 });

    const entry = await ledger.recordFaucet('bob', 1, 2);

    expect(entry.type).toBe('DELEGATE');
    expect(entry.from).toBe('btcpc_faucet');
    expect(entry.to).toBe('bob');
    expect(stateStore.getBalance('bob', 'BTCPC')).toBe(0);
    expect(stateStore.getDelegatedBalance('bob', 'BTCPC')).toBe(1);

    await ledger.recordTransfer('bob', 'alice', 1, 'BTCPC', null, 3, null);
    expect(stateStore.getBalance('alice', 'BTCPC')).toBe(0);
    expect(stateStore.getDelegatedBalance('bob', 'BTCPC')).toBe(1);
  });

  test('delegated faucet credit can fund direct network-service escrow and refunds as delegated', async () => {
    const stateStore = require('../src/chain/stateStore');
    stateStore.resetAll();
    stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'bob', epoch: 1, account_data: { public_keys: {}, chain_addresses: {} } });
    stateStore.applyEntry({ type: 'FAUCET', to: 'btcpc_faucet', token: 'BTCPC', amount: 10, epoch: 1, timestamp: 1 });
    await ledger.recordFaucet('bob', 1, 2);

    await ledger.recordEscrowLock('bob', 'sensor-query-1', 1, 3);
    expect(stateStore.getBalance('bob', 'BTCPC')).toBe(0);
    expect(stateStore.getDelegatedBalance('bob', 'BTCPC')).toBe(0);
    expect(stateStore.getEscrow('sensor-query-1')).toEqual(expect.objectContaining({
      payer: 'bob',
      amount: 1,
      source: 'delegated',
      status: 'locked',
    }));

    await ledger.recordEscrowRefund('bob', 'sensor-query-1', 1, 4);
    expect(stateStore.getBalance('bob', 'BTCPC')).toBe(0);
    expect(stateStore.getDelegatedBalance('bob', 'BTCPC')).toBe(1);
  });

  test('network service charge does not mint when payer has no usable balance', async () => {
    const stateStore = require('../src/chain/stateStore');
    stateStore.resetAll();

    await ledger.recordInferenceCharge('bob', 'miner-a', 1, 2, 'req-no-funds');

    expect(stateStore.getBalance('miner-a', 'BTCPC')).toBe(0);
    expect(stateStore.getBalance('bob', 'BTCPC')).toBe(0);
    expect(stateStore.getDelegatedBalance('bob', 'BTCPC')).toBe(0);
  });

  test('stake and unstake cannot mint from insufficient owned balance', async () => {
    const stateStore = require('../src/chain/stateStore');
    stateStore.resetAll();

    await ledger.recordStake('bob', 5, 'mining', 2);
    expect(stateStore.getStakePool('bob')).toBe(null);
    expect(stateStore.getBalance('bob', 'BTCPC')).toBe(0);

    stateStore.applyEntry({ type: 'FAUCET', to: 'bob', token: 'BTCPC', amount: 3, epoch: 3, timestamp: 3 });
    await ledger.recordStake('bob', 2, 'mining', 4);
    expect(stateStore.getStakePool('bob').total_staked).toBe(2);
    expect(stateStore.getBalance('bob', 'BTCPC')).toBe(1);

    await ledger.recordUnstake('bob', 5, 5);
    expect(stateStore.getStakePool('bob').total_staked).toBe(2);
    expect(stateStore.getBalance('bob', 'BTCPC')).toBe(1);
  });

  test('escrow release cannot pay more than the locked service amount', async () => {
    const stateStore = require('../src/chain/stateStore');
    stateStore.resetAll();
    stateStore.applyEntry({ type: 'FAUCET', to: 'bob', token: 'BTCPC', amount: 2, epoch: 1, timestamp: 1 });

    await ledger.recordEscrowLock('bob', 'service-escrow-1', 2, 2);
    await ledger.recordEscrowRelease('miner-a', 'service-escrow-1', 1.25, 3, 'service payout');
    await ledger.recordEscrowRelease('miner-b', 'service-escrow-1', 1.25, 4, 'service payout');

    expect(stateStore.getBalance('miner-a', 'BTCPC')).toBe(1.25);
    expect(stateStore.getBalance('miner-b', 'BTCPC')).toBe(0);
    expect(stateStore.getEscrow('service-escrow-1')).toEqual(expect.objectContaining({
      amount: 2,
      released_amount: 1.25,
    }));
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
