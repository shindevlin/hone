// Phase E: Escrow test — models removed, uses stateStore/ledger
jest.mock('../src/models/User', () => ({
  findOne: jest.fn(),
  findById: jest.fn()
}));
jest.mock('../src/chain/stateStore', () => ({
  getBalance: jest.fn().mockReturnValue(10),
  getEscrow: jest.fn(),
  getAllEscrows: jest.fn().mockReturnValue({}),
  setEscrow: jest.fn()
}));
jest.mock('../src/services/ledger', () => ({
  getCurrentEpoch: jest.fn().mockResolvedValue(42),
  recordEscrowLock: jest.fn(),
  recordEscrowRelease: jest.fn(),
  recordEscrowRefund: jest.fn(),
  updateWalletCache: jest.fn(),
  distributeRevenueShare: jest.fn().mockResolvedValue([])
}));

const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');
const escrow = require('../src/services/escrow');

describe('escrow service', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    stateStore.getBalance.mockReturnValue(10);
  });

  test('lockFunds checks balance and records escrow lock', async () => {
    const result = await escrow.lockFunds('req-1', 'alice', 4);

    expect(ledger.recordEscrowLock).toHaveBeenCalledWith('alice', 'req-1', 4, 42);
    expect(result.request_id).toBe('req-1');
    expect(result.payer).toBe('alice');
    expect(result.amount).toBe(4);
    expect(result.status).toBe('locked');
  });

  test('lockFunds throws on insufficient balance', async () => {
    stateStore.getBalance.mockReturnValue(2);

    await expect(escrow.lockFunds('req-1', 'alice', 4)).rejects.toThrow('Insufficient balance');
  });

  test('releaseForJob pays miner and refunds overpayment', async () => {
    stateStore.getEscrow.mockReturnValue({
      request_id: 'req-2',
      payer: 'alice',
      amount: 5,
      status: 'locked'
    });

    const result = await escrow.releaseForJob('req-2', 'miner-a', 3, 'qwen3:4b');

    expect(ledger.recordEscrowRelease).toHaveBeenCalledWith('miner-a', 'req-2', 3, 42, 'Inference settlement');
    expect(ledger.updateWalletCache).toHaveBeenCalledWith('miner-a', 'BTCPC', 3);
    expect(ledger.recordEscrowRefund).toHaveBeenCalledWith('alice', 'req-2', 2, 42);
    expect(result.status).toBe('released');
  });

  test('refundFunds returns locked funds to payer', async () => {
    stateStore.getEscrow.mockReturnValue({
      request_id: 'req-3',
      payer: 'alice',
      amount: 2,
      status: 'locked'
    });

    await escrow.refundFunds('req-3');

    expect(ledger.recordEscrowRefund).toHaveBeenCalledWith('alice', 'req-3', 2, 42);
  });

  test('sweepEscrows refunds stale locked escrows', async () => {
    const staleEscrow = {
      request_id: 'req-5',
      payer: 'alice',
      amount: 2,
      status: 'locked',
      locked_at: new Date(Date.now() - 700000).toISOString()
    };
    stateStore.getAllEscrows.mockReturnValue({ 'req-5': staleEscrow });
    stateStore.getEscrow.mockReturnValue(staleEscrow);

    const result = await escrow.sweepEscrows(1);

    expect(result.refunded).toBe(1);
    expect(result.totalRefunded).toBe(2);
    expect(ledger.recordEscrowRefund).toHaveBeenCalledWith('alice', 'req-5', 2, 42);
  });
});
