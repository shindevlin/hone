"use strict";

describe("clock consensus truth gating", () => {
  function loadClockConsensus(env) {
    const prev = {};
    Object.keys(env || {}).forEach((key) => {
      prev[key] = process.env[key];
      process.env[key] = env[key];
    });
    jest.resetModules();
    const mod = require("../src/chain/clockConsensus");
    return {
      mod,
      restore() {
        Object.keys(env || {}).forEach((key) => {
          if (prev[key] === undefined) delete process.env[key];
          else process.env[key] = prev[key];
        });
      },
    };
  }

  beforeEach(() => {
    jest.useFakeTimers();
    jest.spyOn(console, "log").mockImplementation(() => {});
    jest.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(() => {
    jest.useRealTimers();
    jest.restoreAllMocks();
  });

  test("does not seal with only one connected peer", () => {
    const r = loadClockConsensus({ BTCPC_SEAL_COLLECT_MS: "10" });
    const events = [];
    r.mod.updatePeerConnectivity([{ address: "ws://192.168.1.10:6942" }]);
    r.mod.on("epoch_sealed", (evt) => events.push(evt));

    r.mod.receiveSeal({
      epoch_number: 100,
      node_id: "clock-a",
      timestamp: 1234567890000,
      seal_hash: "a".repeat(64),
    });
    jest.advanceTimersByTime(20);

    expect(events).toHaveLength(0);
    expect(r.mod.isObserver()).toBe(false);
    r.restore();
  });

  test("can seal with one connected peer and two clock seals", () => {
    const r = loadClockConsensus({ BTCPC_SEAL_COLLECT_MS: "10" });
    const events = [];
    r.mod.updatePeerConnectivity([
      { address: "ws://192.168.1.10:6942" },
    ]);
    r.mod.on("epoch_sealed", (evt) => events.push(evt));
    jest.setSystemTime(1234567890000);

    r.mod.receiveSeal({
      epoch_number: 200,
      node_id: "clock-a",
      timestamp: 1234567890000,
      seal_hash: "b".repeat(64),
    });
    r.mod.receiveSeal({
      epoch_number: 200,
      node_id: "clock-b",
      timestamp: 1234567890000,
      seal_hash: "b".repeat(64),
    });
    jest.advanceTimersByTime(20);

    expect(events).toHaveLength(1);
    expect(events[0].sealed).toBe(true);
    expect(events[0].quorum).toBeGreaterThanOrEqual(2);
    expect(r.mod.getTruthStatus().truth_bearing).toBe(true);
    r.restore();
  });
});
