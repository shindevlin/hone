jest.mock('../src/models/Wallet', () => ({
  findOne: jest.fn()
}));

jest.mock('../src/models/Transaction', () => ({
  find: jest.fn()
}));

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

const Wallet = require('../src/models/Wallet');
const User = require('../src/models/User');
const ledger = require('../src/services/ledger');
const Transaction = require('../src/models/Transaction');
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

  test('createWallet rejects duplicate wallets for a chain', async () => {
    Wallet.findOne.mockResolvedValue({ _id: 'wallet-1' });

    const req = { user: { id: 'user-1' }, body: { chain: 'btcpc' } };
    const res = createRes();

    await createWallet(req, res);

    expect(res.status).toHaveBeenCalledWith(400);
    expect(res.json).toHaveBeenCalledWith({ error: 'Wallet already exists for this chain' });
  });

  test('getBalance returns 404 when the user has no wallet', async () => {
    Wallet.findOne.mockResolvedValue(null);

    const req = { user: { id: 'user-1' } };
    const res = createRes();

    await getBalance(req, res);

    expect(res.status).toHaveBeenCalledWith(404);
    expect(res.json).toHaveBeenCalledWith({ error: 'Wallet not found' });
  });

  test('transfer rejects insufficient BTCPC balance', async () => {
    Wallet.findOne.mockResolvedValue({
      address: 'BTCPCsender01',
      balance: new Map([['BTCPC', 5]])
    });
    User.findById.mockResolvedValue({ username: 'alice' });
    stateStore.getBalance.mockReturnValue(5);

    const req = {
      user: { id: 'user-1' },
      body: { toAddress: 'BTCPCrecipient01', amount: 10, memo: 'hello' }
    };
    const res = createRes();

    await transfer(req, res);

    expect(res.status).toHaveBeenCalledWith(400);
    expect(res.json).toHaveBeenCalledWith({ error: 'Insufficient BTCPC balance' });
  });

  test('transfer rejects self-transfers', async () => {
    Wallet.findOne
      .mockResolvedValueOnce({
        address: 'BTCPCsameaddr',
        balance: new Map([['BTCPC', 100]])
      })
      .mockResolvedValueOnce({
        address: 'BTCPCsameaddr',
        balance: new Map([['BTCPC', 0]])
      });
    User.findById.mockResolvedValue({ username: 'alice' });
    stateStore.getBalance.mockReturnValue(100);

    const req = {
      user: { id: 'user-1' },
      body: { toAddress: 'BTCPCsameaddr', amount: 10 }
    };
    const res = createRes();

    await transfer(req, res);

    expect(res.status).toHaveBeenCalledWith(400);
    expect(res.json).toHaveBeenCalledWith({ error: 'Cannot transfer to your own wallet' });
  });

  test('transfer records a ledger-backed transaction on success', async () => {
    Wallet.findOne
      .mockResolvedValueOnce({
        address: 'BTCPCsender01',
        balance: new Map([['BTCPC', 100]])
      })
      .mockResolvedValueOnce({
        userId: 'user-2',
        address: 'BTCPCrecipient01',
        balance: new Map([['BTCPC', 0]])
      });
    User.findById.mockResolvedValue({ username: 'alice' });
    User.findOne.mockResolvedValue({ username: 'bob' });
    stateStore.getBalance.mockReturnValue(100);
    ledger.getCurrentEpoch.mockResolvedValue(42);
    ledger.recordTransfer.mockResolvedValue({
      toObject: () => ({ epoch: 42, from: 'alice', to: 'bob', amount: 10 }),
      timestamp: new Date('2026-04-08T00:00:00.000Z')
    });

    const req = {
      user: { id: 'user-1' },
      body: { toAddress: 'BTCPCrecipient01', amount: 10, memo: 'payment' }
    };
    const res = createRes();

    await transfer(req, res);

    expect(ledger.recordTransfer).toHaveBeenCalledWith('alice', 'bob', 10, 'BTCPC', null, 42, 'payment');
    expect(res.json).toHaveBeenCalledWith(expect.objectContaining({
      success: true,
      txHash: 'tx-hash-123'
    }));
  });

  test('getTransactionHistory returns transactions for the user wallet', async () => {
    Wallet.findOne.mockResolvedValue({ address: 'BTCPCsender01' });
    const sort = jest.fn().mockResolvedValue([{ amount: 1 }]);
    Transaction.find.mockReturnValue({ sort });

    const req = { user: { id: 'user-1' } };
    const res = createRes();

    await getTransactionHistory(req, res);

    expect(Transaction.find).toHaveBeenCalledWith({
      $or: [{ from: 'BTCPCsender01' }, { to: 'BTCPCsender01' }]
    });
    expect(res.json).toHaveBeenCalledWith({
      success: true,
      address: 'BTCPCsender01',
      transactions: [{ amount: 1 }]
    });
  });
});
