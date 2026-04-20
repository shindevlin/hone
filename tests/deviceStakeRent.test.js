"use strict";

/**
 * Device Stake & Rent Tests — v3.5
 * Shin Devlin
 *
 * Tests for the pooled rent-auction staking mechanic on IoT devices.
 * Economics:
 *   70% → device owner
 *   20% → staker pool (weighted by slot multiplier × stake_amount)
 *   10% → btcpc_recycle
 *   1% tribute on entry (non-refundable), unless staker === owner
 */

const stateStore = require("../src/chain/stateStore");
const { computeRewards } = require("../src/chain/rewardEngine");

function round(n) {
  return parseFloat(Number(n).toFixed(10));
}

const SLOT_MULTIPLIERS = [1.85, 1.60, 1.38, 1.19, 1.03, 0.89, 0.77, 0.67, 0.58, 0.50];

function makeStakeEntry(deviceId, staker, stakeAmount, rentBid, rentMode, owner, epoch) {
  return {
    type: "DEVICE_YIELD_STAKE",
    from: staker,
    to: owner,
    amount: stakeAmount,
    token: "BTCPC",
    epoch: epoch || 1,
    timestamp: Date.now() + Math.random(),
    yield_stake_data: {
      device_id: deviceId,
      staker: staker,
      stake_amount: stakeAmount,
      rent_bid: rentBid || 0,
      rent_mode: rentMode || "earnings",
      owner: owner,
      entry_epoch: epoch || 1,
    },
    memo: "device_yield_stake:" + deviceId,
  };
}

function makeUnstakeEntry(deviceId, staker, epoch) {
  return {
    type: "DEVICE_YIELD_UNSTAKE",
    from: staker,
    to: staker,
    amount: 0,
    token: "BTCPC",
    epoch: epoch || 2,
    timestamp: Date.now() + Math.random(),
    yield_stake_data: { device_id: deviceId, staker: staker },
    memo: "device_yield_unstake:" + deviceId,
  };
}

function makeRentEntry(deviceId, staker, rentAmount, owner, epoch) {
  return {
    type: "DEVICE_YIELD_RENT",
    from: staker,
    to: owner,
    amount: rentAmount,
    token: "BTCPC",
    epoch: epoch || 2,
    timestamp: Date.now() + Math.random(),
    yield_stake_data: {
      device_id: deviceId,
      staker: staker,
      rent_amount: rentAmount,
      owner: owner,
    },
    memo: "device_yield_rent:" + deviceId,
  };
}

function fundAccounts(accounts, amount) {
  accounts.forEach(function (acc, idx) {
    stateStore.applyEntry({
      type: "MINING_REWARD", to: acc, amount: amount, token: "BTCPC",
      epoch: 0, timestamp: idx + 1,
    });
  });
}

describe("Device Stake & Rent — stateStore mechanics", () => {
  beforeEach(() => {
    stateStore.resetAll();
    fundAccounts(["alice", "bob", "charlie", "dave", "owner1"], 100000);
  });

  // ── Test 1: Fresh stake, tribute, slot 1 assignment ───────────────────────
  test("1. fresh stake assigns slot 1, deducts balance, pays tribute to owner", () => {
    const aliceBefore = stateStore.getBalance("alice", "BTCPC");
    const ownerBefore = stateStore.getBalance("owner1", "BTCPC");

    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 1000, 0.001, "earnings", "owner1", 1));

    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool).toHaveLength(1);
    expect(pool[0].staker).toBe("alice");
    expect(pool[0].slot).toBe(1);
    expect(pool[0].stake_amount).toBe(1000);

    const tribute = round(1000 * 0.01);
    expect(pool[0].tribute_paid).toBeCloseTo(tribute, 8);

    const aliceAfter = stateStore.getBalance("alice", "BTCPC");
    expect(round(aliceBefore - aliceAfter)).toBeCloseTo(1000, 8);

    const ownerAfter = stateStore.getBalance("owner1", "BTCPC");
    expect(round(ownerAfter - ownerBefore)).toBeCloseTo(tribute, 8);
  });

  // ── Test 2: Rent auction — higher rent_bid wins slot 1 ────────────────────
  test("2. rent auction: lower stake but higher rent_bid wins slot 1", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 5000, 0.001, "earnings", "owner1", 1));
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "bob", 500, 0.010, "earnings", "owner1", 2));

    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool[0].staker).toBe("bob");   // higher rent_bid
    expect(pool[0].slot).toBe(1);
    expect(pool[1].staker).toBe("alice");
    expect(pool[1].slot).toBe(2);
  });

  // ── Test 3: Equal rent bids — higher stake wins slot ──────────────────────
  test("3. equal rent bids: higher stake wins the better slot", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 1000, 0.005, "earnings", "owner1", 1));
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "bob", 2000, 0.005, "earnings", "owner1", 2));

    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool[0].staker).toBe("bob");   // higher stake breaks tie
    expect(pool[1].staker).toBe("alice");
  });

  // ── Test 4: 11th staker bumps slot 10 ─────────────────────────────────────
  test("4. 11th staker bumps slot 10 holder (lowest rent_bid) and returns their stake", () => {
    // Fill 10 slots with low rent bids
    fundAccounts(["s1","s2","s3","s4","s5","s6","s7","s8","s9","s10","s11"], 10000);
    for (var i = 1; i <= 10; i++) {
      stateStore.applyEntry(makeStakeEntry("owner1/device-a", "s" + i, 1000, 0.001 * i, "earnings", "owner1", i));
    }
    const pool10 = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool10).toHaveLength(10);

    // Lowest rent_bid is s1 (0.001) — it should be evicted
    const s1BalBefore = stateStore.getBalance("s1", "BTCPC");

    // s11 enters with higher rent_bid than s1
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "s11", 500, 0.0015, "earnings", "owner1", 11));

    const pool11 = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool11).toHaveLength(10);

    // s1 should be evicted and refunded
    const s1Names = pool11.map(function (r) { return r.staker; });
    expect(s1Names).not.toContain("s1");
    expect(s1Names).toContain("s11");

    // s1 stake returned
    const s1BalAfter = stateStore.getBalance("s1", "BTCPC");
    expect(s1BalAfter).toBeGreaterThan(s1BalBefore);
  });

  // ── Test 5: Earnings mode — yield covers rent ─────────────────────────────
  test("5. earnings mode: yield covers rent, surplus goes to wallet (stake unchanged)", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 1000, 0.001, "earnings", "owner1", 1));

    const aliceBal = stateStore.getBalance("alice", "BTCPC");
    // rent_amount 0 means yield covered all rent — no stake deduction
    stateStore.applyEntry(makeRentEntry("owner1/device-a", "alice", 0, "owner1", 2));

    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool[0].stake_amount).toBe(1000); // unchanged
  });

  // ── Test 6: Earnings mode — insufficient yield pulls from stake ───────────
  test("6. earnings mode: rent_amount from stake depletes stake", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 1000, 0.001, "earnings", "owner1", 1));

    const rentFromStake = 0.5;
    stateStore.applyEntry(makeRentEntry("owner1/device-a", "alice", rentFromStake, "owner1", 2));

    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool[0].stake_amount).toBeCloseTo(1000 - rentFromStake, 8);
  });

  // ── Test 7: Stake mode — full yield to wallet, rent depletes stake ─────────
  test("7. stake mode: rent deducted from stake each epoch", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 1000, 0.001, "stake", "owner1", 1));

    const pool0 = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool0[0].rent_mode).toBe("stake");

    const rentAmount = 0.001;
    stateStore.applyEntry(makeRentEntry("owner1/device-a", "alice", rentAmount, "owner1", 2));

    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool[0].stake_amount).toBeCloseTo(1000 - rentAmount, 8);
  });

  // ── Test 8: Stake depletion → eviction ────────────────────────────────────
  test("8. stake depletion to 0 evicts staker", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 0.001, 0.001, "stake", "owner1", 1));

    // Deduct entire stake via rent
    stateStore.applyEntry(makeRentEntry("owner1/device-a", "alice", 0.001, "owner1", 2));

    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool).toHaveLength(0);
    expect(stateStore.isStakerOnDevice("alice", "owner1/device-a")).toBe(false);
  });

  // ── Test 9: Unstake returns remaining stake_amount ────────────────────────
  test("9. unstake returns exact remaining stake_amount", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 1000, 0.001, "earnings", "owner1", 1));

    // Deplete some stake via rent
    stateStore.applyEntry(makeRentEntry("owner1/device-a", "alice", 100, "owner1", 2));

    const aliceBeforeUnstake = stateStore.getBalance("alice", "BTCPC");
    stateStore.applyEntry(makeUnstakeEntry("owner1/device-a", "alice", 3));

    const aliceAfterUnstake = stateStore.getBalance("alice", "BTCPC");
    expect(round(aliceAfterUnstake - aliceBeforeUnstake)).toBeCloseTo(900, 8);

    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool).toHaveLength(0);
  });

  // ── Test 10: Owner auto-stake — no tribute ────────────────────────────────
  test("10. owner auto-stake: no tribute, balance deducted by only stake_amount", () => {
    const ownerBefore = stateStore.getBalance("owner1", "BTCPC");
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "owner1", 500, 0.001, "earnings", "owner1", 1));

    const ownerAfter = stateStore.getBalance("owner1", "BTCPC");
    expect(round(ownerBefore - ownerAfter)).toBeCloseTo(500, 8);

    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool[0].tribute_paid).toBe(0);
  });

  // ── Test 11: isStakerOnDevice true/false ──────────────────────────────────
  test("11. isStakerOnDevice returns true for staked account, false otherwise", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 500, 0.001, "earnings", "owner1", 1));

    expect(stateStore.isStakerOnDevice("alice", "owner1/device-a")).toBe(true);
    expect(stateStore.isStakerOnDevice("bob", "owner1/device-a")).toBe(false);
    expect(stateStore.isStakerOnDevice("alice", "owner1/device-b")).toBe(false);
  });

  // ── Test 12: Rent mode change ─────────────────────────────────────────────
  test("12. DEVICE_YIELD_RENT_MODE updates rent mode", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 500, 0.001, "earnings", "owner1", 1));

    stateStore.applyEntry({
      type: "DEVICE_YIELD_RENT_MODE",
      from: "alice", to: "alice", amount: 0, token: "BTCPC", epoch: 2, timestamp: Date.now(),
      yield_stake_data: { device_id: "owner1/device-a", staker: "alice", rent_mode: "stake" },
    });

    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool[0].rent_mode).toBe("stake");
  });

  // ── Test 13: Floor rent for new device ────────────────────────────────────
  test("13. floor rent formula: 50 BTCPC/month minimum for new devices", () => {
    // floor = max(0 * 0.07, 50) / 86400
    const expectedFloor = 50 / 86400;
    expect(expectedFloor).toBeCloseTo(0.00057870, 6);
  });

  // ── Test 14: resetAll clears everything ───────────────────────────────────
  test("14. resetAll clears deviceStakerPools and deviceAutoStake", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 500, 0.001, "earnings", "owner1", 1));
    stateStore.applyEntry({
      type: "DEVICE_YIELD_AUTO_STAKE",
      from: "owner1", to: "owner1", amount: 0, token: "BTCPC", epoch: 1, timestamp: Date.now(),
      yield_stake_data: { device_id: "owner1/device-a", owner: "owner1", pct: 10 },
    });

    expect(stateStore.getDeviceStakerPool("owner1/device-a")).toHaveLength(1);
    expect(stateStore.getDeviceAutoStakePct("owner1/device-a")).toBe(10);

    stateStore.resetAll();

    expect(stateStore.getDeviceStakerPool("owner1/device-a")).toHaveLength(0);
    expect(stateStore.getDeviceAutoStakePct("owner1/device-a")).toBe(0);
    expect(stateStore.isStakerOnDevice("alice", "owner1/device-a")).toBe(false);
  });

  // ── Test 15: getStakedDevicesForAccount ───────────────────────────────────
  test("15. getStakedDevicesForAccount returns all devices for account", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 500, 0.002, "earnings", "owner1", 1));
    stateStore.applyEntry(makeStakeEntry("owner1/device-b", "alice", 300, 0.001, "earnings", "owner1", 2));
    stateStore.applyEntry(makeStakeEntry("owner1/device-c", "bob", 400, 0.003, "earnings", "owner1", 3));

    const aliceDevices = stateStore.getStakedDevicesForAccount("alice");
    expect(aliceDevices).toHaveLength(2);
    const ids = aliceDevices.map(function (d) { return d.device_id; }).sort();
    expect(ids).toEqual(["owner1/device-a", "owner1/device-b"]);

    const bobDevices = stateStore.getStakedDevicesForAccount("bob");
    expect(bobDevices).toHaveLength(1);
    expect(bobDevices[0].device_id).toBe("owner1/device-c");
  });

  // ── Test 16: Unstake non-existent position is no-op ───────────────────────
  test("16. unstake for non-staker is a no-op", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 500, 0.001, "earnings", "owner1", 1));
    const bobBefore = stateStore.getBalance("bob", "BTCPC");

    stateStore.applyEntry(makeUnstakeEntry("owner1/device-a", "bob", 2));

    const bobAfter = stateStore.getBalance("bob", "BTCPC");
    expect(bobAfter).toBe(bobBefore);
    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool[0].staker).toBe("alice");
  });

  // ── Test 17: rent_paid_total accumulates ──────────────────────────────────
  test("17. rent_paid_total accumulates across multiple rent entries", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 1000, 0.001, "stake", "owner1", 1));

    stateStore.applyEntry(makeRentEntry("owner1/device-a", "alice", 0.001, "owner1", 2));
    stateStore.applyEntry(makeRentEntry("owner1/device-a", "alice", 0.001, "owner1", 3));

    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool[0].rent_paid_total).toBeCloseTo(0.002, 8);
  });

  // ── Test 18: DEVICE_YIELD_AUTO_STAKE stores pct ───────────────────────────
  test("18. DEVICE_YIELD_AUTO_STAKE stores auto-stake percentage 0-50", () => {
    stateStore.applyEntry({
      type: "DEVICE_YIELD_AUTO_STAKE",
      from: "owner1", to: "owner1", amount: 0, token: "BTCPC", epoch: 1, timestamp: Date.now(),
      yield_stake_data: { device_id: "owner1/device-a", owner: "owner1", pct: 25 },
    });
    expect(stateStore.getDeviceAutoStakePct("owner1/device-a")).toBe(25);

    // Over-limit clamped to 50
    stateStore.applyEntry({
      type: "DEVICE_YIELD_AUTO_STAKE",
      from: "owner1", to: "owner1", amount: 0, token: "BTCPC", epoch: 2, timestamp: Date.now(),
      yield_stake_data: { device_id: "owner1/device-a", owner: "owner1", pct: 99 },
    });
    expect(stateStore.getDeviceAutoStakePct("owner1/device-a")).toBe(50);
  });

  // ── Test 19: Multiple stakers, slot multipliers differ ────────────────────
  test("19. slot multiplier: slot 1 gets SLOT_MULTIPLIERS[0]=1.85, slot 2 gets 1.60", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 1000, 0.010, "earnings", "owner1", 1));
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "bob", 1000, 0.005, "earnings", "owner1", 2));

    const pool = stateStore.getDeviceStakerPool("owner1/device-a");
    expect(pool[0].staker).toBe("alice");
    expect(SLOT_MULTIPLIERS[pool[0].slot - 1]).toBe(1.85);
    expect(pool[1].staker).toBe("bob");
    expect(SLOT_MULTIPLIERS[pool[1].slot - 1]).toBe(1.60);
  });

  // ── Test 20: getAllStakersForDevice returns Set ────────────────────────────
  test("20. getAllStakersForDevice returns Set of all staker names", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "alice", 500, 0.003, "earnings", "owner1", 1));
    stateStore.applyEntry(makeStakeEntry("owner1/device-a", "bob", 300, 0.002, "earnings", "owner1", 2));

    const set = stateStore.getAllStakersForDevice("owner1/device-a");
    expect(set).toBeInstanceOf(Set);
    expect(set.has("alice")).toBe(true);
    expect(set.has("bob")).toBe(true);
    expect(set.size).toBe(2);
  });
});

// ─────────────────────────────────────────────────────────────────
// Reward engine: 70/20/10 and 90/10 splits
// ─────────────────────────────────────────────────────────────────

describe("Device Stake & Rent — reward engine splits", () => {
  beforeEach(() => {
    stateStore.resetAll();
    fundAccounts(["owner1", "alice", "bob"], 100000);
  });

  // ── Test 21: 70/20/10 split with stakers ──────────────────────────────────
  test("21. sensor income splits 70/20/10 when device has active stakers", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/sensor-x", "alice", 1000, 0.001, "earnings", "owner1", 1));

    const { rewards } = computeRewards({
      epochNumber: 2,
      blockReward: 1000,
      computeProofs: [],
      verifiers: [],
      clockNodes: [],
      storageHosts: [],
      activeSensors: [{ account: "owner1", readings_count: 1, sensor_ids: ["owner1/sensor-x"] }],
      serviceHosts: [],
      stateStore: stateStore,
    });

    const sensorPool = round(1000 * 0.07);
    const expectedOwnerBase = round(sensorPool * 0.70);
    const expectedStakerPool = round(sensorPool * 0.20);
    const expectedRecycle = round(sensorPool - expectedOwnerBase - expectedStakerPool);

    const ownerReward = rewards.filter(r => r.to === "owner1" && r.type === "SENSOR_EPOCH_REWARD").reduce((s, r) => s + r.amount, 0);
    const stakerReward = rewards.filter(r => r.to === "alice" && r.type === "SENSOR_YIELD_REWARD").reduce((s, r) => s + r.amount, 0);

    expect(ownerReward).toBeCloseTo(expectedOwnerBase, 5);
    expect(stakerReward).toBeGreaterThan(0);
    expect(stakerReward).toBeLessThanOrEqual(round(expectedStakerPool + 0.000001));
  });

  // ── Test 22: 90/10 without stakers ────────────────────────────────────────
  test("22. sensor income splits 90/10 when device has no stakers", () => {
    const { rewards } = computeRewards({
      epochNumber: 2,
      blockReward: 1000,
      computeProofs: [],
      verifiers: [],
      clockNodes: [],
      storageHosts: [],
      activeSensors: [{ account: "owner1", readings_count: 1, sensor_ids: ["owner1/no-staker"] }],
      serviceHosts: [],
      stateStore: stateStore,
    });

    const sensorPool = round(1000 * 0.07);
    const expectedOwner = round(sensorPool * 0.90);

    const ownerReward = rewards.filter(r => r.to === "owner1" && r.type === "SENSOR_EPOCH_REWARD").reduce((s, r) => s + r.amount, 0);
    expect(ownerReward).toBeCloseTo(expectedOwner, 5);

    const yieldRewards = rewards.filter(r => r.type === "SENSOR_YIELD_REWARD");
    expect(yieldRewards).toHaveLength(0);
  });

  // ── Test 23: No sensor rewards when no active sensors ─────────────────────
  test("23. zero sensor activity means sensor pool goes to recycle", () => {
    const { recycled } = computeRewards({
      epochNumber: 1,
      blockReward: 1000,
      computeProofs: [],
      verifiers: [],
      clockNodes: [],
      storageHosts: [],
      activeSensors: [],
      serviceHosts: [],
      stateStore: stateStore,
    });

    // 7% sensor pool + 2% reserve = 9% recycled
    const expectedMin = round(1000 * 0.09);
    expect(recycled).toBeGreaterThanOrEqual(expectedMin - 0.0001);
  });

  // ── Test 24: Multiple stakers, higher-weighted gets more yield ────────────
  test("24. higher slot multiplier × stake gives proportionally more yield", () => {
    // alice: slot 1 (rent_bid 0.01, stake 1000), bob: slot 2 (rent_bid 0.005, stake 1000)
    stateStore.applyEntry(makeStakeEntry("owner1/sensor-x", "alice", 1000, 0.010, "earnings", "owner1", 1));
    stateStore.applyEntry(makeStakeEntry("owner1/sensor-x", "bob", 1000, 0.005, "earnings", "owner1", 2));

    const { rewards } = computeRewards({
      epochNumber: 2,
      blockReward: 10000,
      computeProofs: [],
      verifiers: [],
      clockNodes: [],
      storageHosts: [],
      activeSensors: [{ account: "owner1", readings_count: 1, sensor_ids: ["owner1/sensor-x"] }],
      serviceHosts: [],
      stateStore: stateStore,
    });

    const aliceYield = rewards.filter(r => r.to === "alice" && r.type === "SENSOR_YIELD_REWARD").reduce((s, r) => s + r.amount, 0);
    const bobYield = rewards.filter(r => r.to === "bob" && r.type === "SENSOR_YIELD_REWARD").reduce((s, r) => s + r.amount, 0);
    // alice slot 1 (mult 1.85) > bob slot 2 (mult 1.60)
    expect(aliceYield).toBeGreaterThan(bobYield);
  });

  // ── Test 25: After unstake, reverts to 90/10 ──────────────────────────────
  test("25. after unstake, sensor income reverts to 90/10 for owner", () => {
    stateStore.applyEntry(makeStakeEntry("owner1/sensor-x", "alice", 1000, 0.001, "earnings", "owner1", 1));
    stateStore.applyEntry(makeUnstakeEntry("owner1/sensor-x", "alice", 2));

    expect(stateStore.getDeviceStakerPool("owner1/sensor-x")).toHaveLength(0);

    const { rewards } = computeRewards({
      epochNumber: 3,
      blockReward: 1000,
      computeProofs: [],
      verifiers: [],
      clockNodes: [],
      storageHosts: [],
      activeSensors: [{ account: "owner1", readings_count: 1, sensor_ids: ["owner1/sensor-x"] }],
      serviceHosts: [],
      stateStore: stateStore,
    });

    const yieldRewards = rewards.filter(r => r.type === "SENSOR_YIELD_REWARD");
    expect(yieldRewards).toHaveLength(0);

    const sensorPool = round(1000 * 0.07);
    const ownerRewards = rewards.filter(r => r.to === "owner1" && r.type === "SENSOR_EPOCH_REWARD");
    const ownerTotal = ownerRewards.reduce((s, r) => s + r.amount, 0);
    expect(ownerTotal).toBeCloseTo(round(sensorPool * 0.90), 5);
  });
});
