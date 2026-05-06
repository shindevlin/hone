"use strict";

const stateStore = require("../src/chain/stateStore");
const nodeRegistry = require("../src/chain/nodeRegistry");
const nodeController = require("../src/controllers/nodeController");

function makeRes() {
  return {
    statusCode: 200,
    payload: null,
    status(code) {
      this.statusCode = code;
      return this;
    },
    json(body) {
      this.payload = body;
      return this;
    },
  };
}

describe("nodeController health reporting", () => {
  beforeEach(() => {
    stateStore.resetAll();
    nodeRegistry.reset();
  });

  test("getEpochInfo reports the chain height epoch, not wall-clock time", async () => {
    stateStore.setChainHeight(4321);
    stateStore.setEpoch(4321, {
      epoch_number: 4321,
      started_at: new Date("2026-01-01T00:00:00Z"),
      status: "active",
      commitments: [{ node_id: "alpha" }],
      block_reward: 12.5,
    });

    const req = {};
    const res = makeRes();

    await nodeController.getEpochInfo(req, res);

    expect(res.statusCode).toBe(200);
    expect(res.payload.success).toBe(true);
    expect(res.payload.epoch.epoch_number).toBe(4321);
    expect(res.payload.epoch.status).toBe("active");
    expect(res.payload.epoch.commitments_count).toBe(1);
  });

  test("getNodes falls back to the chain account record p2p address", async () => {
    nodeRegistry.registerNode("alpha", "miner", 1000, null, 4321, false);
    stateStore.applyEntry({
      type: "NODE_REGISTER",
      from: "alpha",
      epoch: 4321,
      account_data: {
        username: "alpha",
        p2p_address: "ws://alpha.example:6942",
        node_types: ["miner"],
      },
    });

    const req = {};
    const res = makeRes();

    await nodeController.getNodes(req, res);

    expect(res.statusCode).toBe(200);
    expect(res.payload.success).toBe(true);
    expect(res.payload.nodes).toHaveLength(1);
    expect(res.payload.nodes[0].p2pAddress).toBe("ws://alpha.example:6942");
  });

  test("getNodes reflects NODE_ANNOUNCE backfill when the live registry is stale", async () => {
    nodeRegistry.registerNode("beta", "clock", 0, null, 4322, true);
    stateStore.applyEntry({
      type: "NODE_REGISTER",
      from: "beta",
      epoch: 4322,
      account_data: {
        username: "beta",
        node_types: ["clock"],
        p2p_address: null,
        permissioned: true,
      },
    });
    stateStore.applyEntry({
      type: "NODE_ANNOUNCE",
      from: "beta",
      epoch: 4323,
      account_data: {
        username: "beta",
        p2p_address: "ws://beta.example:6943",
        node_types: ["clock"],
        permissioned: true,
      },
    });

    const req = {};
    const res = makeRes();

    await nodeController.getNodes(req, res);

    expect(res.statusCode).toBe(200);
    expect(res.payload.nodes).toHaveLength(1);
    expect(res.payload.nodes[0].p2pAddress).toBe("ws://beta.example:6943");
  });
});
