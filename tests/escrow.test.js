const savedTransactions = [];
const MockTransaction = jest.fn(function Transaction(data) {
  return {
    ...data,
    save: jest.fn(async () => savedTransactions.push(data))
  };
});

const MockEscrow = jest.fn(function Escrow(data) {
  return {
    ...data,
    save: jest.fn(async () => undefined)
  };
});
MockEscrow.findOne = jest.fn();
MockEscrow.find = jest.fn();

jest.mock('../src/models/Escrow', () => MockEscrow);
jest.mock('../src/models/Transaction', () => MockTransaction);
jest.mock('../src/models/User', () => ({
  findOne: jest.fn(),
  findById: jest.fn()
}));
jest.mock('../src/models/Wallet', () => ({
  findOne: jest.fn()
}));
jest.mock('../src/models/Node', () => ({
  findById: jest.fn()
}));
jest.mock('../src/services/ledger', () => ({
  getCurrentEpoch: jest.fn().mockResolvedValue(42),
  recordEscrowLock: jest.fn(),
  recordEscrowRelease: jest.fn(),
  recordEscrowRefund: jest.fn(),
  updateWalletCache: jest.fn(),
  distributeRevenueShare: jest.fn().mockResolvedValue([])
}));

const User = require('../src/models/User');
const Wallet = require('../src/models/Wallet');
const Node = require('../src/models/Node');
const ledger = require('../src/services/ledger');
const escrow = require('../src/services/escrow');

function wallet(balance) {
  return {
    balance: new Map([['BTCPC', balance]]),
    save: jest.fn().mockResolvedValue(undefined)
  };
}

describe('escrow service', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    savedTransactions.length = 0;
  });

  test('lockFunds deducts wallet balance and creates escrow records', async () => {
    const payerWallet = wallet(10);
    User.findOne.mockResolvedValue({ _id: 'user-1', username: 'alice' });
    Wallet.findOne.mockResolvedValue(payerWallet);

    const result = await escrow.lockFunds('req-1', 'alice', 4);

    expect(ledger.recordEscrowLock).toHaveBeenCalledWith('alice', 'req-1', 4, 42);
    expect(payerWallet.balance.get('BTCPC')).toBe(6);
    expect(result.request_id).toBe('req-1');
    expect(savedTransactions[0]).toEqual(expect.objectContaining({
      from: 'alice',
      to: 'escrow:req-1',
      amount: 4,
      type: 'escrow_lock'
    }));
  });

  test('releaseForJob pays miner and refunds overpayment', async () => {
    const existingEscrow = {
      request_id: 'req-2',
      payer: 'alice',
      amount: 5,
      status: 'locked',
      save: jest.fn().mockResolvedValue(undefined)
    };
    MockEscrow.findOne.mockResolvedValue(existingEscrow);

    const result = await escrow.releaseForJob('req-2', 'miner-a', 3, 'qwen3:4b');

    expect(ledger.recordEscrowRelease).toHaveBeenCalledWith('miner-a', 'req-2', 3, 42, 'Inference settlement');
    expect(ledger.updateWalletCache).toHaveBeenCalledWith('miner-a', 'BTCPC', 3);
    expect(ledger.recordEscrowRefund).toHaveBeenCalledWith('alice', 'req-2', 2, 42);
    expect(result.status).toBe('released');
  });

  test('refundFunds returns locked funds to payer', async () => {
    const payerWallet = wallet(1);
    const existingEscrow = {
      request_id: 'req-3',
      payer: 'alice',
      amount: 2,
      status: 'locked',
      save: jest.fn().mockResolvedValue(undefined)
    };
    MockEscrow.findOne.mockResolvedValue(existingEscrow);
    User.findOne.mockResolvedValue({ _id: 'user-1', username: 'alice' });
    Wallet.findOne.mockResolvedValue(payerWallet);

    await escrow.refundFunds('req-3');

    expect(ledger.recordEscrowRefund).toHaveBeenCalledWith('alice', 'req-3', 2, 42);
    expect(payerWallet.balance.get('BTCPC')).toBe(3);
    expect(existingEscrow.status).toBe('refunded');
  });

  test('releaseFunds distributes payouts to node owners', async () => {
    const minerWallet = wallet(0);
    const existingEscrow = {
      request_id: 'req-4',
      payer: 'alice',
      amount: 5,
      status: 'locked',
      save: jest.fn().mockResolvedValue(undefined)
    };
    MockEscrow.findOne.mockResolvedValue(existingEscrow);
    Node.findById.mockResolvedValue({ _id: 'node-1', account: 'user-2' });
    User.findById.mockResolvedValue({ _id: 'user-2', username: 'miner-a' });
    Wallet.findOne.mockResolvedValue(minerWallet);

    await escrow.releaseFunds('req-4', [{ node_id: 'node-1', amount: 5, rank: 1 }]);

    expect(ledger.recordEscrowRelease).toHaveBeenCalledWith('miner-a', 'req-4', 5, 42, 'Inference payout rank #1');
    expect(minerWallet.balance.get('BTCPC')).toBe(5);
    expect(existingEscrow.status).toBe('released');
  });

  test('sweepEscrows refunds stale locked escrows', async () => {
    MockEscrow.find.mockResolvedValue([{ request_id: 'req-5', amount: 2 }]);
    MockEscrow.findOne.mockResolvedValue({
      request_id: 'req-5',
      payer: 'alice',
      amount: 2,
      status: 'locked',
      save: jest.fn().mockResolvedValue(undefined)
    });
    User.findOne.mockResolvedValue({ _id: 'user-1', username: 'alice' });
    Wallet.findOne.mockResolvedValue(wallet(0));

    const result = await escrow.sweepEscrows(1);

    expect(result).toEqual({ refunded: 1, totalRefunded: 2 });
    expect(ledger.recordEscrowRefund).toHaveBeenCalledWith('alice', 'req-5', 2, 42);
  });
});
