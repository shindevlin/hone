"use strict";

const mockStateStore = {
  getEpoch: jest.fn(),
  applyEntries: jest.fn(),
  setMiningProofs: jest.fn(),
  setComputeProofs: jest.fn(),
  setEpoch: jest.fn(),
};

const mockStateManager = {
  applyLedgerEntries: jest.fn(),
};

const mockBlockStore = {
  hasBlock: jest.fn(() => false),
  readBlock: jest.fn(() => null),
  writeBlock: jest.fn(),
};

const mockBlockchain = {
  addBlock: jest.fn(),
};

jest.mock("../src/chain/stateStore", () => mockStateStore);
jest.mock("../src/chain/stateManager", () => mockStateManager);
jest.mock("../src/chain/blockStore", () => mockBlockStore);
jest.mock("../src/chain/blockchain", () => mockBlockchain);
jest.mock("../src/chain/forkResolver", () => ({
  isHealInProgress: () => false,
  checkForFork: () => ({ forked: false }),
  onPeerBlock: jest.fn(() => Promise.resolve()),
}));
jest.mock("../src/p2p/chainSync", () => ({
  validateBlock: () => true,
  getChainHeight: () => 10,
  getBlockRange: () => [],
}));

const Block = require("../src/chain/block");
const protocol = require("../src/p2p/protocol");

describe("P2P sync replay", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockStateStore.getEpoch.mockReturnValue(null);
  });

  test("RESPONSE_BLOCKS replays ledger entries into state before writing", () => {
    const block = new Block({
      epoch_number: 42,
      timestamp: 123456789,
      state_root: "a".repeat(64),
      miner_id: "b".repeat(64),
    });
    const header_hex = block.serialize().toString("hex");
    const ledgerEntries = [
      { type: "TRANSFER", from: "alice", to: "bob", amount: 5, epoch: 42 },
      { type: "STAKE", from: "bob", amount: 2, epoch: 42 },
    ];

    protocol.handleMessage(
      { nodeId: "peer-1", address: "ws://peer-1", ws: { readyState: 1 } },
      {
        type: "RESPONSE_BLOCKS",
        nodeId: "peer-1",
        data: {
          blocks: [
            {
              epoch_number: 42,
              header_hex,
              ledger_entries: ledgerEntries,
              rewards: [{ miner: "bob", amount: 5 }],
              compute_proofs: [{ node_id: "bob", work_value: 3 }],
              mining_proofs: [{ node_id: "bob", proof: "proof" }],
              state_root: "a".repeat(64),
              consensus_hash: "c".repeat(64),
              block_reward: 5,
              total_work: 3,
            },
          ],
        },
      },
      {
        broadcast: jest.fn(),
        send: jest.fn(),
        peers: new Map(),
        NODE_ID: "local-node",
        connectToPeer: jest.fn(),
      }
    );

    expect(mockStateStore.applyEntries).toHaveBeenCalledWith(ledgerEntries);
    expect(mockStateManager.applyLedgerEntries).toHaveBeenCalledWith(ledgerEntries);
    expect(mockStateStore.setMiningProofs).toHaveBeenCalledWith(42, [{ node_id: "bob", proof: "proof" }]);
    expect(mockStateStore.setComputeProofs).toHaveBeenCalledWith(42, [{ node_id: "bob", work_value: 3 }]);
    expect(mockStateStore.setEpoch).toHaveBeenCalledWith(42, expect.objectContaining({
      status: "finalized",
      block_reward: 5,
      total_work: 3,
    }));
    expect(mockBlockStore.writeBlock).toHaveBeenCalledTimes(1);
    expect(mockBlockchain.addBlock).toHaveBeenCalledTimes(1);
  });
});
