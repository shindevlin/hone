const mockLedgerEntryInstance = (data) => ({
  ...data,
  save: jest.fn().mockResolvedValue(undefined),
  toObject: jest.fn(() => ({ ...data }))
});

const mockLedgerEntry = jest.fn(function LedgerEntry(data) {
  return mockLedgerEntryInstance(data);
});
mockLedgerEntry.aggregate = jest.fn();
mockLedgerEntry.distinct = jest.fn();
mockLedgerEntry.findOne = jest.fn();
mockLedgerEntry.find = jest.fn();
mockLedgerEntry.create = jest.fn();

jest.mock('../src/models/LedgerEntry', () => mockLedgerEntry);

jest.mock('../src/models/Wallet', () => ({
  findOne: jest.fn()
}));

jest.mock('../src/models/User', () => ({
  findOne: jest.fn()
}));

jest.mock('../src/models/Epoch', () => ({
  findOne: jest.fn()
}));

const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args)
}));

const Wallet = require('../src/models/Wallet');
const User = require('../src/models/User');
const Epoch = require('../src/models/Epoch');
const ledger = require('../src/services/ledger');

describe('ledger service', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    ledger.flushPendingEntries();
  });

  test('recordTransfer rejects self-transfers before touching the mempool', async () => {
    await expect(ledger.recordTransfer('alice', 'alice', 1, 'BTCPC', null, 1)).rejects.toThrow('Cannot transfer to self');
    expect(mockMempoolSubmit).not.toHaveBeenCalled();
  });

  test('recordTransfer writes a ledger entry and updates both wallet caches', async () => {
    mockMempoolSubmit.mockReturnValue({ accepted: true });

    const senderWallet = {
      balance: new Map([['BTCPC', 25]]),
      save: jest.fn().mockResolvedValue(undefined)
    };
    const recipientWallet = {
      balance: new Map([['BTCPC', 5]]),
      save: jest.fn().mockResolvedValue(undefined)
    };

    User.findOne
      .mockResolvedValueOnce({ _id: 'user-1', username: 'alice' })
      .mockResolvedValueOnce({ _id: 'user-2', username: 'bob' });
    Wallet.findOne
      .mockResolvedValueOnce(senderWallet)
      .mockResolvedValueOnce(recipientWallet);

    const entry = await ledger.recordTransfer('alice', 'bob', 7, 'BTCPC', null, 42, 'memo');

    expect(mockMempoolSubmit).toHaveBeenCalled();
    expect(entry.toObject()).toEqual(expect.objectContaining({
      type: 'TRANSFER',
      from: 'alice',
      to: 'bob',
      amount: 7,
      epoch: 42
    }));
    expect(senderWallet.balance.get('BTCPC')).toBe(18);
    expect(recipientWallet.balance.get('BTCPC')).toBe(12);
    expect(senderWallet.save).toHaveBeenCalled();
    expect(recipientWallet.save).toHaveBeenCalled();
  });

  test('getBalance computes net token balance from incoming and outgoing ledger totals', async () => {
    mockLedgerEntry.aggregate
      .mockResolvedValueOnce([{ total: 13.5 }])
      .mockResolvedValueOnce([{ total: 3.25 }]);

    const balance = await ledger.getBalance('alice', 'BTCPC');

    expect(balance).toBe(10.25);
    expect(mockLedgerEntry.aggregate).toHaveBeenCalledTimes(2);
  });

  test('flushPendingEntries returns and clears pending entries', async () => {
    mockMempoolSubmit.mockReturnValue({ accepted: true });
    User.findOne
      .mockResolvedValueOnce({ _id: 'user-1' })
      .mockResolvedValueOnce({ _id: 'user-2' });
    Wallet.findOne
      .mockResolvedValueOnce({ balance: new Map(), save: jest.fn().mockResolvedValue(undefined) })
      .mockResolvedValueOnce({ balance: new Map(), save: jest.fn().mockResolvedValue(undefined) });

    await ledger.recordTransfer('alice', 'bob', 1, 'BTCPC', null, 1, null);

    const firstFlush = ledger.flushPendingEntries();
    const secondFlush = ledger.flushPendingEntries();

    expect(firstFlush).toHaveLength(1);
    expect(secondFlush).toHaveLength(0);
  });

  test('getCurrentEpoch returns the latest epoch number or zero when none exists', async () => {
    const lean = jest.fn().mockResolvedValue({ epoch_number: 77 });
    const sort = jest.fn().mockReturnValue({ lean });
    Epoch.findOne.mockReturnValue({ sort });
    await expect(ledger.getCurrentEpoch()).resolves.toBe(77);

    const leanMissing = jest.fn().mockResolvedValue(null);
    const sortMissing = jest.fn().mockReturnValue({ lean: leanMissing });
    Epoch.findOne.mockReturnValue({ sort: sortMissing });
    await expect(ledger.getCurrentEpoch()).resolves.toBe(0);
  });
});
