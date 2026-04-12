"use strict";

/**
 * Dynamic Block Cap tests — v3.0
 *
 * Verifies the adaptive cap growth/shrink logic introduced in v3.0.
 */

const {
  computeNextBlockCap,
  DEFAULT_BLOCK_CAP,
  MAX_BLOCK_CAP,
  MIN_BLOCK_CAP,
} = require("../src/chain/blockSizeCap");

const MB = 1024 * 1024;

describe("dynamic block cap (v3.0)", () => {
  test("constants: DEFAULT_BLOCK_CAP is 1 MB", () => {
    expect(DEFAULT_BLOCK_CAP).toBe(1 * MB);
  });

  test("constants: MAX_BLOCK_CAP is 32 MB", () => {
    expect(MAX_BLOCK_CAP).toBe(32 * MB);
  });

  test("constants: MIN_BLOCK_CAP is 1 MB", () => {
    expect(MIN_BLOCK_CAP).toBe(1 * MB);
  });

  test("block >50% full → cap grows by 25%", () => {
    const cap = 1 * MB;
    const blockSize = Math.floor(cap * 0.6); // 60% full
    const next = computeNextBlockCap(cap, blockSize);
    expect(next).toBe(Math.floor(cap * 1.25));
  });

  test("block <25% full → cap shrinks by 25%", () => {
    const cap = 2 * MB;
    const blockSize = Math.floor(cap * 0.1); // 10% full
    const next = computeNextBlockCap(cap, blockSize);
    expect(next).toBe(Math.floor(cap * 0.75));
  });

  test("block between 25-50% full → cap unchanged", () => {
    const cap = 4 * MB;
    const blockSize = Math.floor(cap * 0.37); // 37% full
    const next = computeNextBlockCap(cap, blockSize);
    expect(next).toBe(cap);
  });

  test("cap never exceeds MAX_BLOCK_CAP (32 MB)", () => {
    const cap = 30 * MB;
    const blockSize = Math.floor(cap * 0.9); // very full
    const next = computeNextBlockCap(cap, blockSize);
    expect(next).toBeLessThanOrEqual(MAX_BLOCK_CAP);
    expect(next).toBe(MAX_BLOCK_CAP);
  });

  test("cap never goes below MIN_BLOCK_CAP (1 MB)", () => {
    const cap = 1 * MB;
    const blockSize = 0; // empty block
    const next = computeNextBlockCap(cap, blockSize);
    expect(next).toBeGreaterThanOrEqual(MIN_BLOCK_CAP);
    expect(next).toBe(MIN_BLOCK_CAP);
  });

  test("repeated growth from 1 MB eventually hits 32 MB ceiling", () => {
    let cap = 1 * MB;
    for (let i = 0; i < 50; i++) {
      // always >50% full → always grows
      cap = computeNextBlockCap(cap, Math.floor(cap * 0.8));
    }
    expect(cap).toBe(MAX_BLOCK_CAP);
  });

  test("repeated shrink from 32 MB eventually hits 1 MB floor", () => {
    let cap = 32 * MB;
    for (let i = 0; i < 100; i++) {
      // always <25% full → always shrinks
      cap = computeNextBlockCap(cap, Math.floor(cap * 0.1));
    }
    expect(cap).toBe(MIN_BLOCK_CAP);
  });
});
