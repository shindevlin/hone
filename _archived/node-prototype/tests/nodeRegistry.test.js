"use strict";

const nodeRegistry = require("../src/chain/nodeRegistry");

describe("nodeRegistry multi-role support", () => {
  beforeEach(() => {
    nodeRegistry.reset();
  });

  test("preserves multi-capability node_types from NODE_REGISTER entries", () => {
    nodeRegistry.processLedgerEntry({
      type: "NODE_REGISTER",
      from: "natoshi",
      epoch: 42,
      account_data: {
        node_types: ["clock", "miner", "verifier", "storage_host"],
        p2p_address: "127.0.0.1:8333",
        permissioned: true,
      },
    });

    const node = nodeRegistry.getNode("natoshi");
    expect(node.type).toBe("clock");
    expect(node.node_types).toEqual(["clock", "miner", "verifier", "storage_host"]);
    expect(node.p2pAddress).toBe("127.0.0.1:8333");
    expect(node.permissioned).toBe(true);
  });

  test("mining rewards add miner role to an existing non-miner node", () => {
    nodeRegistry.processLedgerEntry({
      type: "NODE_REGISTER",
      from: "natoshi",
      epoch: 1,
      account_data: { node_types: ["clock", "verifier"] },
    });

    nodeRegistry.processLedgerEntry({
      type: "MINING_REWARD",
      to: "natoshi",
      amount: 10,
      epoch: 2,
    });

    expect(nodeRegistry.getNode("natoshi").node_types).toEqual(["clock", "verifier", "miner"]);
  });
});
