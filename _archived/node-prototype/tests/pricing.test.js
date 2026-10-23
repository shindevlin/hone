// Phase E: Epoch/WorkProof/Node models removed — pricing uses stateStore/nodeRegistry
jest.mock('../src/chain/stateStore', () => ({
  getChainHeight: jest.fn().mockReturnValue(10),
  getEpoch: jest.fn(),
  getLatestEpoch: jest.fn(),
  getAllComputeProofs: jest.fn().mockReturnValue({})
}));

jest.mock('../src/chain/nodeRegistry', () => ({
  getRegisteredNodes: jest.fn().mockReturnValue([])
}));

jest.mock('../src/mining/workGenerator', () => ({
  getModelWeight: jest.fn().mockReturnValue(1),
  getBlockReward: jest.fn().mockReturnValue(243)
}));

describe('pricing service', () => {
  let pricing;
  let stateStore;
  let nodeRegistry;

  beforeEach(() => {
    jest.resetModules();
    jest.clearAllMocks();
    process.env.HONE_BASE_TOKENS = '1000';
    process.env.HONE_LOAD_WINDOW = '12';
    // Re-require mocks after resetModules
    stateStore = require('../src/chain/stateStore');
    nodeRegistry = require('../src/chain/nodeRegistry');
    pricing = require('../src/services/pricing');
  });

  test('getNetworkLoad returns zero when there are no recent compute proofs', async () => {
    stateStore.getAllComputeProofs.mockReturnValue({});
    stateStore.getChainHeight.mockReturnValue(0);

    await expect(pricing.getNetworkLoad()).resolves.toBe(0);
  });

  test('getCurrentPricing scales token pricing by load and model weight', async () => {
    const { getModelWeight } = require('../src/mining/workGenerator');
    getModelWeight.mockReturnValue(4);
    stateStore.getAllComputeProofs.mockReturnValue({
      '9': [{ node_id: 'miner1', tokens_generated: 5 }],
      '10': [{ node_id: 'miner1', tokens_generated: 5 }]
    });

    const result = await pricing.getCurrentPricing('qwen3.5:27b');

    expect(result).toHaveProperty('tokensPerHone');
    expect(result).toHaveProperty('load');
    expect(result).toHaveProperty('modelWeight');
    expect(typeof result.tokensPerHone).toBe('number');
  });

  test('calculateCost returns deterministic HONE cost from pricing', async () => {
    const { getModelWeight } = require('../src/mining/workGenerator');
    getModelWeight.mockReturnValue(2);
    stateStore.getAllComputeProofs.mockReturnValue({});

    const { cost, pricing: current } = await pricing.calculateCost(500, 'llama3:8b');

    expect(typeof cost).toBe('number');
    expect(cost).toBeGreaterThan(0);
    expect(current).toHaveProperty('tokensPerHone');
  });

  test('getAutoBid factors in block reward coverage and miner count', async () => {
    const { getModelWeight, getBlockReward } = require('../src/mining/workGenerator');
    getModelWeight.mockReturnValue(4);
    getBlockReward.mockReturnValue(243);
    nodeRegistry.getRegisteredNodes.mockReturnValue([{ username: 'a', type: 'miner' }, { username: 'b', type: 'miner' }, { username: 'c', type: 'miner' }]);
    stateStore.getAllComputeProofs.mockReturnValue({});
    stateStore.getLatestEpoch.mockReturnValue({ epoch_number: 123 });

    const result = await pricing.getAutoBid('qwen3.5:27b', 600);

    expect(result.miners).toBe(3);
    expect(result.bid).toBeGreaterThan(0);
  });
});
