// Phase E: Wallet/Transaction models removed — walletController uses stateStore
jest.mock('../src/models/User', () => ({
  findById: jest.fn(),
  findOne: jest.fn()
}));

jest.mock('../src/services/ledger', () => ({
  getCurrentEpoch: jest.fn(),
  recordTransfer: jest.fn()
}));

jest.mock('../src/chain/stateStore', () => ({
  getBalance: jest.fn(),
  getTokenBalances: jest.fn(),
  getAccount: jest.fn(),
  hasAccount: jest.fn()
}));

jest.mock('../src/p2p/network', () => ({
  NODE_ID: 'node-1',
  broadcast: jest.fn()
}));

jest.mock('../src/p2p/protocol', () => ({
  createTransactionMessage: jest.fn(() => ({ type: 'tx-message' }))
}));

jest.mock('../src/chain/blockStore', () => ({
  hashLedgerEntry: jest.fn(() => 'tx-hash-123')
}));

const User = require('../src/models/User');
const ledger = require('../src/services/ledger');
const stateStore = require('../src/chain/stateStore');
const {
  createWallet,
  getBalance,
  transfer,
  getTransactionHistory
} = require('../src/controllers/walletController');

function createRes() {
  return {
    status: jest.fn().mockReturnThis(),
    json: jest.fn().mockReturnThis()
  };
}

describe('walletController', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  test('createWallet returns derived address for user', async () => {
    User.findById.mockResolvedValue({ _id: 'user-1', username: 'alice' });
    stateStore.getBalance.mockReturnValue(5);

    const req = { user: { id: 'user-1' }, body: { chain: 'btcpc' } };
    const res = createRes();

    await createWallet(req, res);

    expect(res.status).toHaveBeenCalledWith(201);
    expect(res.json).toHaveBeenCalledWith(expect.objectContaining({
      success: true,
      wallet: expect.objectContaining({ chain: 'btcpc' })
    }));
  });

  test('getBalance returns 404 when user not found', async () => {
    User.findById.mockResolvedValue(null);

    const req = { user: { id: 'user-1' } };
    const res = createRes();

    await getBalance(req, res);

    expect(res.status).toHaveBeenCalledWith(404);
    expect(res.json).toHaveBeenCalledWith({ error: 'User not found' });
  });

  test('getBalance returns balance from stateStore', async () => {
    User.findById.mockResolvedValue({ _id: 'user-1', username: 'alice' });
    stateStore.getBalance.mockReturnValue(42);
    stateStore.getTokenBalances.mockReturnValue({ BTCPC: 42 });

    const req = { user: { id: 'user-1' } };
    const res = createRes();

    await getBalance(req, res);

    expect(stateStore.getBalance).toHaveBeenCalledWith('alice', 'BTCPC');
    expect(res.json).toHaveBeenCalledWith(expect.objectContaining({
      success: true,
      balance: expect.objectContaining({ BTCPC: 42 })
    }));
  });

  test('transfer rejects insufficient BTCPC balance', async () => {
    User.findById.mockResolvedValue({ username: 'alice' });
    stateStore.getBalance.mockReturnValue(5);

    const req = {
      user: { id: 'user-1' },
      body: { toAddress: 'bobaccount', amount: 10, memo: 'hello' }
    };
    const res = createRes();

    await transfer(req, res);

    expect(res.status).toHaveBeenCalledWith(400);
    expect(res.json).toHaveBeenCalledWith({ error: 'Insufficient BTCPC balance' });
  });

  test('transfer records a ledger-backed transaction on success', async () => {
    User.findById.mockResolvedValue({ username: 'alice' });
    User.findOne.mockResolvedValue({ username: 'bobaccount' });
    stateStore.getBalance.mockReturnValue(100);
    ledger.getCurrentEpoch.mockResolvedValue(42);
    ledger.recordTransfer.mockResolvedValue({
      toObject: () => ({ epoch: 42, from: 'alice', to: 'bobaccount', amount: 10 }),
      timestamp: new Date('2026-04-08T00:00:00.000Z')
    });

    const req = {
      user: { id: 'user-1' },
      body: { toAddress: 'bobaccount', amount: 10, memo: 'payment' }
    };
    const res = createRes();

    await transfer(req, res);

    expect(ledger.recordTransfer).toHaveBeenCalledWith('alice', 'bobaccount', 10, 'BTCPC', null, 42, 'payment');
    expect(res.json).toHaveBeenCalledWith(expect.objectContaining({
      success: true,
      txHash: 'tx-hash-123'
    }));
  });

  test('getTransactionHistory returns note about block replay', async () => {
    User.findById.mockResolvedValue({ _id: 'user-1', username: 'alice' });
    stateStore.getBalance.mockReturnValue(5);

    const req = { user: { id: 'user-1' } };
    const res = createRes();

    await getTransactionHistory(req, res);

    expect(res.json).toHaveBeenCalledWith(expect.objectContaining({
      success: true,
      transactions: []
    }));
  });
});
