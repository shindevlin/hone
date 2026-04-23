// Phase E: Wallet/Transaction models removed — walletController uses stateStore
jest.mock('../src/models/User', () => ({
  findById: jest.fn(),
  findOne: jest.fn()
}));

jest.mock('../src/services/ledger', () => ({
  getCurrentEpoch: jest.fn(),
  recordTransfer: jest.fn(),
  recordAuthorizedTransfer: jest.fn()
}));

jest.mock('../src/services/privateAuthorization', () => ({
  verifyTransferAuthorization: jest.fn()
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
const privateAuthorization = require('../src/services/privateAuthorization');
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

  test('transfer enforces private authorization when enabled', async () => {
    User.findById.mockResolvedValue({
      username: 'alice',
      privateAuth: { enabled: true, threshold: 1, factors: [] }
    });
    User.findOne.mockResolvedValue({ username: 'bobaccount' });
    stateStore.getBalance.mockReturnValue(100);
    ledger.getCurrentEpoch.mockResolvedValue(42);
    privateAuthorization.verifyTransferAuthorization.mockResolvedValue({
      requestId: 'req-1',
      threshold: 1,
      approvalCount: 1,
      factors: [{ factorId: 'f-1', chain: 'evm', commitment: 'abc' }]
    });
    ledger.recordAuthorizedTransfer.mockResolvedValue({
      toObject: () => ({ epoch: 42, from: 'alice', to: 'bobaccount', amount: 10 }),
      timestamp: new Date('2026-04-08T00:00:00.000Z')
    });

    const req = {
      user: { id: 'user-1' },
      body: {
        toAddress: 'bobaccount',
        amount: 10,
        memo: 'payment',
        private_auth: { requestId: 'req-1', approvals: [{ factorId: 'f-1', chain: 'evm', signature: '0x01' }] }
      }
    };
    const res = createRes();

    await transfer(req, res);

    expect(privateAuthorization.verifyTransferAuthorization).toHaveBeenCalledWith(
      'alice',
      { from: 'alice', to: 'bobaccount', amount: 10, token: 'BTCPC', memo: 'payment' },
      req.body.private_auth
    );
    expect(ledger.recordAuthorizedTransfer).toHaveBeenCalledWith(
      'alice',
      'bobaccount',
      10,
      'BTCPC',
      null,
      42,
      'payment',
      expect.objectContaining({
        signedBy: 'private_auth',
        requestId: 'req-1',
        threshold: 1,
        approvalCount: 1
      })
    );
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
