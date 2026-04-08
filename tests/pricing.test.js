const mockEpoch = {
  find: jest.fn(),
  findOne: jest.fn()
};

const mockWorkProof = {
  countDocuments: jest.fn()
};

const mockNode = {
  countDocuments: jest.fn()
};

const mockGetModelWeight = jest.fn();
const mockGetBlockReward = jest.fn();

jest.mock('../src/models/Epoch', () => mockEpoch);
jest.mock('../src/models/WorkProof', () => mockWorkProof);
jest.mock('../src/models/Node', () => mockNode);
jest.mock('../src/mining/workGenerator', () => ({
  getModelWeight: (...args) => mockGetModelWeight(...args),
  getBlockReward: (...args) => mockGetBlockReward(...args)
}));

describe('pricing service', () => {
  let pricing;

  beforeEach(() => {
    jest.resetModules();
    jest.clearAllMocks();
    process.env.BTCPC_BASE_TOKENS = '1000';
    process.env.BTCPC_LOAD_WINDOW = '12';
    pricing = require('../src/services/pricing');
  });

  test('getNetworkLoad returns zero when there are no recent finalized epochs', async () => {
    mockEpoch.find.mockReturnValue({
      sort: () => ({
        limit: () => ({
          lean: jest.fn().mockResolvedValue([])
        })
      })
    });

    await expect(pricing.getNetworkLoad()).resolves.toBe(0);
  });

  test('getCurrentPricing scales token pricing by load and model weight', async () => {
    mockEpoch.find.mockReturnValue({
      sort: () => ({
        limit: () => ({
          lean: jest.fn().mockResolvedValue([{ epoch_number: 10 }, { epoch_number: 9 }])
        })
      })
    });
    mockWorkProof.countDocuments
      .mockResolvedValueOnce(10)
      .mockResolvedValueOnce(5);
    mockGetModelWeight.mockReturnValue(4);

    const result = await pricing.getCurrentPricing('qwen3.5:27b');

    expect(result.load).toBe(0.5);
    expect(result.loadMultiplier).toBe(1.5);
    expect(result.modelWeight).toBe(4);
    expect(result.tokensPerBtcpc).toBe(166);
  });

  test('calculateCost returns deterministic BTCPC cost from pricing', async () => {
    mockEpoch.find.mockReturnValue({
      sort: () => ({
        limit: () => ({
          lean: jest.fn().mockResolvedValue([{ epoch_number: 10 }])
        })
      })
    });
    mockWorkProof.countDocuments
      .mockResolvedValueOnce(8)
      .mockResolvedValueOnce(2);
    mockGetModelWeight.mockReturnValue(2);

    const { cost, pricing: current } = await pricing.calculateCost(500, 'llama3:8b');

    expect(current.tokensPerBtcpc).toBe(500);
    expect(cost).toBeCloseTo(1, 10);
  });

  test('getAutoBid factors in block reward coverage and miner count', async () => {
    mockEpoch.find.mockReturnValue({
      sort: () => ({
        limit: () => ({
          lean: jest.fn().mockResolvedValue([{ epoch_number: 10 }, { epoch_number: 9 }])
        })
      })
    });
    mockWorkProof.countDocuments
      .mockResolvedValueOnce(20)
      .mockResolvedValueOnce(10);
    mockGetModelWeight.mockReturnValue(4);
    mockEpoch.findOne.mockReturnValue({
      sort: () => ({
        lean: jest.fn().mockResolvedValue({ epoch_number: 123 })
      })
    });
    mockGetBlockReward.mockReturnValue(243);
    mockNode.countDocuments.mockResolvedValue(3);

    const result = await pricing.getAutoBid('qwen3.5:27b', 600);

    expect(result.miners).toBe(3);
    expect(result.reward_per_miner).toBe(81);
    expect(result.bid).toBeGreaterThan(0);
    expect(result.bid_multiplier).toBeGreaterThanOrEqual(0.05);
    expect(result.fair_cost).toBeGreaterThan(result.bid);
  });
});
