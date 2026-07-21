"use strict";

/**
 * Epoch timing tests — v3.0
 *
 * Verifies 30-second epochs and the updated emission schedule.
 */

const { EPOCH_DURATION_MS } = require("../src/services/epochManager");
const {
  getBlockReward,
  getEpochsPerMonth,
  getPeriodTable,
  EPOCHS_PER_MONTH,
  GENESIS_ALLOTMENT,
  TOTAL_SUPPLY,
} = require("../src/services/emissionSchedule");

describe("epoch timing (v3.0 — 30s epochs)", () => {
  test("EPOCH_DURATION_MS is 30 seconds", () => {
    expect(EPOCH_DURATION_MS).toBe(30000);
  });

  test("EPOCH_DURATION_MS is NOT 300 seconds (old value)", () => {
    expect(EPOCH_DURATION_MS).not.toBe(300000);
  });

  test("EPOCHS_PER_MONTH reflects 30-second epochs (86400)", () => {
    // 30 days × 24 h × 60 min × 60 sec / 30 sec per epoch = 86400
    expect(EPOCHS_PER_MONTH).toBe(86400);
    expect(getEpochsPerMonth()).toBe(86400);
  });

  test("genesis reward per epoch is ~24.306 HONE (1/10 of old 243.06)", () => {
    const reward = getBlockReward(0);
    // Period 1: 2,100,000 HONE / 86400 epochs ≈ 24.3056
    expect(reward).toBeGreaterThan(24.0);
    expect(reward).toBeLessThan(25.0);
  });

  test("genesis reward is approximately 24.306 HONE (within 0.01)", () => {
    const expected = 2100000 / 86400;
    const actual = getBlockReward(0);
    expect(Math.abs(actual - expected)).toBeLessThan(0.01);
  });

  test("period 1 lasts 86400 epochs (1 month at 30s)", () => {
    const table = getPeriodTable();
    const p1 = table[0];
    expect(p1.epochs).toBe(86400);
  });

  test("period 2 lasts 172800 epochs (2 months at 30s)", () => {
    const table = getPeriodTable();
    const p2 = table[1];
    expect(p2.epochs).toBe(172800);
  });

  test("reward at epoch 86400 (start of period 2) is less than period 1", () => {
    const p1Reward = getBlockReward(0);
    const p2Reward = getBlockReward(86400);
    expect(p2Reward).toBeLessThan(p1Reward);
  });

  test("reward at a very late epoch (post all periods) is 0", () => {
    // Epoch 999999999 is way past all periods
    const reward = getBlockReward(999999999);
    expect(reward).toBe(0);
  });

  test("total supply across all periods is 42,000,000 HONE", () => {
    const table = getPeriodTable();
    const totalMined = table.reduce((sum, p) => sum + p.allotment, 0);
    // Should be within floating point rounding of TOTAL_SUPPLY
    expect(Math.abs(totalMined - TOTAL_SUPPLY)).toBeLessThan(1);
  });

  test("30 seconds × 2880 epochs/day = 86400 epochs per month", () => {
    const secondsPerDay = 86400;
    const epochsPerDay = secondsPerDay / (EPOCH_DURATION_MS / 1000);
    expect(epochsPerDay).toBe(2880);
    const epochsPerMonth = epochsPerDay * 30;
    expect(epochsPerMonth).toBe(EPOCHS_PER_MONTH);
  });
});
