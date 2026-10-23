"use strict";

const { recordCrash, recentCrashCount, reset, WINDOW_MS, MAX_CRASHES } = require("../src/supervisor/circuitBreaker");

function freshState() { return new Map(); }

// Fixed time anchor so tests are deterministic
const NOW = 1_000_000_000;

describe("recordCrash", () => {
  test("returns false for crashes under the limit", () => {
    const s = freshState();
    for (let i = 0; i < MAX_CRASHES; i++) {
      const tripped = recordCrash(s, "miner", NOW + i);
      expect(tripped).toBe(false);
    }
  });

  test("returns true when crash count exceeds MAX_CRASHES", () => {
    const s = freshState();
    for (let i = 0; i < MAX_CRASHES; i++) recordCrash(s, "miner", NOW + i);
    const tripped = recordCrash(s, "miner", NOW + MAX_CRASHES);
    expect(tripped).toBe(true);
  });

  test("crashes older than WINDOW_MS are pruned before checking", () => {
    const s = freshState();
    // Fill up to the limit at t=0
    for (let i = 0; i < MAX_CRASHES; i++) recordCrash(s, "miner", NOW + i);
    // Advance past the window — all old crashes should be pruned
    const future = NOW + WINDOW_MS + 1000;
    // This is crash #1 in the new window — should NOT trip
    const tripped = recordCrash(s, "miner", future);
    expect(tripped).toBe(false);
    expect(recentCrashCount(s, "miner", future)).toBe(1);
  });

  test("different roles have independent breakers", () => {
    const s = freshState();
    for (let i = 0; i <= MAX_CRASHES; i++) recordCrash(s, "miner", NOW + i);
    // clock is brand new — should not be tripped
    const tripped = recordCrash(s, "clock", NOW);
    expect(tripped).toBe(false);
  });
});

describe("recentCrashCount", () => {
  test("counts only crashes within the window", () => {
    const s = freshState();
    recordCrash(s, "storage", NOW - WINDOW_MS - 1); // outside — pruned on next write
    recordCrash(s, "storage", NOW - 1000);           // inside
    recordCrash(s, "storage", NOW);                  // inside
    // Force a read-only count at NOW (no new crash)
    expect(recentCrashCount(s, "storage", NOW)).toBe(2);
  });

  test("returns 0 for unknown role", () => {
    expect(recentCrashCount(freshState(), "ghost", NOW)).toBe(0);
  });
});

describe("reset", () => {
  test("clears crash history so the breaker can close again", () => {
    const s = freshState();
    for (let i = 0; i <= MAX_CRASHES; i++) recordCrash(s, "api", NOW + i);
    reset(s, "api");
    expect(recentCrashCount(s, "api", NOW + MAX_CRASHES + 1)).toBe(0);
    // After reset, role can accumulate crashes again from scratch
    const tripped = recordCrash(s, "api", NOW + MAX_CRASHES + 2);
    expect(tripped).toBe(false);
  });

  test("reset on unknown role is a no-op", () => {
    const s = freshState();
    expect(() => reset(s, "nonexistent")).not.toThrow();
  });
});

describe("boundary conditions", () => {
  test("exactly MAX_CRASHES crashes does not trip (limit is strictly greater-than)", () => {
    const s = freshState();
    for (let i = 0; i < MAX_CRASHES; i++) recordCrash(s, "miner", NOW + i);
    expect(recentCrashCount(s, "miner", NOW + MAX_CRASHES)).toBe(MAX_CRASHES);
    // One more trips it
    expect(recordCrash(s, "miner", NOW + MAX_CRASHES)).toBe(true);
  });

  test("crash at the exact window edge is excluded", () => {
    const s = freshState();
    // Crash exactly at windowStart — NOT inside (windowStart is exclusive)
    const windowStart = NOW - WINDOW_MS;
    recordCrash(s, "clock", windowStart);
    expect(recentCrashCount(s, "clock", NOW)).toBe(0);
  });
});
