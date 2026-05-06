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

jest.mock("fs", () => ({
  existsSync: jest.fn(() => false),
  readFileSync: jest.fn(),
  writeFileSync: jest.fn(),
}));
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

const protocol = require("../src/p2p/protocol");

describe("P2P discovery filtering", () => {
  beforeEach(() => {
    protocol.knownPeers.clear();
    jest.clearAllMocks();
  });

  afterEach(() => {
    delete process.env.BTCPC_PUBLIC_ADDRESS;
    delete process.env.P2P_PORT;
  });

  test("ignores localhost in handshake public_address", () => {
    protocol.handleMessage(
      { nodeId: "peer-1", address: "ws://peer-1", ws: { readyState: 1 } },
      {
        type: "HANDSHAKE",
        nodeId: "peer-1",
        data: {
          chainHeight: 11,
          version: "3.1.144",
          public_address: "ws://localhost:6942",
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

    expect(protocol.knownPeers.has("ws://localhost:6942")).toBe(false);
  });

  test("records and keeps a routable handshake address", () => {
    protocol.handleMessage(
      { nodeId: "peer-2", address: "ws://peer-2", ws: { readyState: 1 } },
      {
        type: "HANDSHAKE",
        nodeId: "peer-2",
        data: {
          chainHeight: 11,
          version: "3.1.144",
          public_address: "wss://node.example:9443/ws",
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

    expect(protocol.knownPeers.has("wss://node.example:9443/ws")).toBe(true);
  });

  test("filters localhost out of peer list gossip before connecting", () => {
    const connectToPeer = jest.fn();

    protocol.handleMessage(
      { nodeId: "peer-3", address: "ws://peer-3", ws: { readyState: 1 } },
      {
        type: "PEER_LIST",
        nodeId: "peer-3",
        data: {
          peers: [
            "ws://localhost:6942",
            "ws://node.example:6942",
          ],
        },
      },
      {
        broadcast: jest.fn(),
        send: jest.fn(),
        peers: new Map(),
        NODE_ID: "local-node",
        connectToPeer,
      }
    );

    expect(connectToPeer).toHaveBeenCalledTimes(1);
    expect(connectToPeer).toHaveBeenCalledWith("ws://node.example:6942");
  });
});
