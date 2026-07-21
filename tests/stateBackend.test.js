"use strict";

/**
 * stateBackend tests — v3.0
 *
 * Tests for both the MemoryBackend (synchronous Maps) and the LevelDB
 * backend (async, disk-backed). Also verifies that stateStore continues
 * to work correctly with its default memory backend.
 */

const os = require("os");
const path = require("path");
const fs = require("fs");
const { createBackend, MemoryBackend, LevelDBBackend } = require("../src/chain/stateBackend");

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

function makeTempDir() {
  const dir = path.join(os.tmpdir(), "hone-statebackend-test-" + Date.now() + "-" + Math.random().toString(36).slice(2));
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

function removeTempDir(dir) {
  try {
    fs.rmSync(dir, { recursive: true, force: true });
  } catch (_) {}
}

// ─────────────────────────────────────────────────────────────────
// MemoryBackend tests
// ─────────────────────────────────────────────────────────────────

describe("MemoryBackend", () => {
  let backend;

  beforeEach(() => {
    backend = new MemoryBackend();
  });

  test("type is 'memory'", () => {
    expect(backend.type).toBe("memory");
  });

  test("get returns null for missing key", () => {
    expect(backend.get("balances", "alice|HONE")).toBeNull();
  });

  test("set and get round-trip", () => {
    backend.set("balances", "alice|HONE", 100);
    expect(backend.get("balances", "alice|HONE")).toBe(100);
  });

  test("has returns false for missing key", () => {
    expect(backend.has("accounts", "alice")).toBe(false);
  });

  test("has returns true after set", () => {
    backend.set("accounts", "alice", { created_epoch: 0 });
    expect(backend.has("accounts", "alice")).toBe(true);
  });

  test("delete removes a key", () => {
    backend.set("balances", "alice|HONE", 50);
    backend.delete("balances", "alice|HONE");
    expect(backend.get("balances", "alice|HONE")).toBeNull();
    expect(backend.has("balances", "alice|HONE")).toBe(false);
  });

  test("iterate visits all keys in prefix", () => {
    backend.set("balances", "alice|HONE", 100);
    backend.set("balances", "bob|HONE", 200);
    backend.set("accounts", "alice", { created_epoch: 0 });

    const entries = [];
    backend.iterate("balances", (key, value) => {
      entries.push({ key, value });
    });

    expect(entries.length).toBe(2);
    const keys = entries.map((e) => e.key).sort();
    expect(keys).toEqual(["alice|HONE", "bob|HONE"]);
  });

  test("clear wipes a single prefix, leaves others", () => {
    backend.set("balances", "alice|HONE", 100);
    backend.set("accounts", "alice", { created_epoch: 0 });
    backend.clear("balances");
    expect(backend.get("balances", "alice|HONE")).toBeNull();
    expect(backend.get("accounts", "alice")).not.toBeNull();
  });

  test("clearAll wipes everything", () => {
    backend.set("balances", "alice|HONE", 100);
    backend.set("accounts", "alice", { created_epoch: 0 });
    backend.clearAll();
    expect(backend.get("balances", "alice|HONE")).toBeNull();
    expect(backend.get("accounts", "alice")).toBeNull();
  });

  test("createBackend('memory') returns MemoryBackend", () => {
    const b = createBackend("memory");
    expect(b).toBeInstanceOf(MemoryBackend);
    expect(b.type).toBe("memory");
  });

  test("default createBackend (no type) returns MemoryBackend", () => {
    // No type defaults to memory
    const b = createBackend();
    expect(b).toBeInstanceOf(MemoryBackend);
  });
});

// ─────────────────────────────────────────────────────────────────
// LevelDBBackend tests
// ─────────────────────────────────────────────────────────────────

describe("LevelDBBackend", () => {
  let backend;
  let tmpDir;

  beforeEach(async () => {
    tmpDir = makeTempDir();
    backend = createBackend("leveldb", tmpDir);
    // Wait for LevelDB to open
    await new Promise((r) => setTimeout(r, 50));
  });

  afterEach(async () => {
    try {
      await backend.close();
    } catch (_) {}
    removeTempDir(tmpDir);
  });

  test("type is 'leveldb'", () => {
    expect(backend.type).toBe("leveldb");
  });

  test("get returns null for missing key", async () => {
    const val = await backend.get("balances", "alice|HONE");
    expect(val).toBeNull();
  });

  test("set and get round-trip", async () => {
    await backend.set("balances", "alice|HONE", 100);
    const val = await backend.get("balances", "alice|HONE");
    expect(val).toBe(100);
  });

  test("has returns false for missing key", async () => {
    const result = await backend.has("accounts", "alice");
    expect(result).toBe(false);
  });

  test("has returns true after set", async () => {
    await backend.set("accounts", "alice", { created_epoch: 0 });
    const result = await backend.has("accounts", "alice");
    expect(result).toBe(true);
  });

  test("delete removes a key", async () => {
    await backend.set("balances", "alice|HONE", 50);
    await backend.delete("balances", "alice|HONE");
    const val = await backend.get("balances", "alice|HONE");
    expect(val).toBeNull();
  });

  test("iterate visits all keys in prefix", async () => {
    await backend.set("balances", "alice|HONE", 100);
    await backend.set("balances", "bob|HONE", 200);
    await backend.set("accounts", "alice", { created_epoch: 0 });

    const entries = [];
    await backend.iterate("balances", async (key, value) => {
      entries.push({ key, value });
    });

    expect(entries.length).toBe(2);
    const keys = entries.map((e) => e.key).sort();
    expect(keys).toEqual(["alice|HONE", "bob|HONE"]);
  });

  test("clear wipes a single prefix, leaves others", async () => {
    await backend.set("balances", "alice|HONE", 100);
    await backend.set("accounts", "alice", { created_epoch: 0 });
    await backend.clear("balances");
    expect(await backend.get("balances", "alice|HONE")).toBeNull();
    expect(await backend.get("accounts", "alice")).not.toBeNull();
  });

  test("clearAll wipes everything", async () => {
    await backend.set("balances", "alice|HONE", 100);
    await backend.set("accounts", "alice", { created_epoch: 0 });
    await backend.clearAll();
    expect(await backend.get("balances", "alice|HONE")).toBeNull();
    expect(await backend.get("accounts", "alice")).toBeNull();
  });

  test("data persists across reopening the database", async () => {
    await backend.set("balances", "alice|HONE", 999);
    await backend.close();

    // Re-open
    backend = createBackend("leveldb", tmpDir);
    await new Promise((r) => setTimeout(r, 50));
    const val = await backend.get("balances", "alice|HONE");
    expect(val).toBe(999);
  });

  test("objects round-trip correctly through JSON serialization", async () => {
    const account = { created_epoch: 42, public_keys: { ed25519: "abc" }, chain_addresses: {} };
    await backend.set("accounts", "shindevlin", account);
    const retrieved = await backend.get("accounts", "shindevlin");
    expect(retrieved).toEqual(account);
  });
});

// ─────────────────────────────────────────────────────────────────
// stateStore with memory backend (existing behavior unchanged)
// ─────────────────────────────────────────────────────────────────

describe("stateStore with memory backend (existing behavior)", () => {
  const stateStore = require("../src/chain/stateStore");

  beforeEach(() => {
    stateStore.resetAll();
  });

  test("applyEntry ACCOUNT_CREATE creates account", () => {
    stateStore.applyEntry({ type: "ACCOUNT_CREATE", to: "testuser", epoch: 1 });
    expect(stateStore.hasAccount("testuser")).toBe(true);
  });

  test("applyEntry MINING_REWARD credits balance", () => {
    stateStore.applyEntry({ type: "ACCOUNT_CREATE", to: "miner1", epoch: 1 });
    stateStore.applyEntry({
      type: "MINING_REWARD",
      to: "miner1",
      amount: 24.306,
      token: "HONE",
      epoch: 1,
    });
    const bal = stateStore.getBalance("miner1", "HONE");
    expect(bal).toBeCloseTo(24.306, 3);
  });

  test("getTokenBalances returns HONE display amounts, not raw units", () => {
    stateStore.applyEntry({ type: "ACCOUNT_CREATE", to: "miner1", epoch: 1 });
    stateStore.applyEntry({
      type: "MINING_REWARD",
      to: "miner1",
      amount: 24.3055555556,
      token: "HONE",
      epoch: 1,
    });

    expect(stateStore.getTokenBalances("miner1").HONE).toBeCloseTo(24.3055555556, 10);
  });

  test("hydrateFromFinality converts account HONE balances into internal units", () => {
    stateStore.hydrateFromFinality({
      finality_epoch: 100,
      accounts: {
        miner1: { balance: 26324.8611112036, staked: 0, delegated: 0, nonce: 0 },
      },
    });

    expect(stateStore.getBalance("miner1", "HONE")).toBeCloseTo(26324.8611112036, 10);
    expect(stateStore.getTokenBalances("miner1").HONE).toBeCloseTo(26324.8611112036, 10);
  });

  test("hydrateFromFinality rejects negative balances", () => {
    expect(() => {
      stateStore.hydrateFromFinality({
        finality_epoch: 100,
        accounts: {
          miner1: { balance: -1, staked: 0, delegated: 0, nonce: 0 },
        },
      });
    }).toThrow(/negative/i);
  });

  test("applyEntry TRANSFER moves funds between accounts", () => {
    stateStore.applyEntry({ type: "ACCOUNT_CREATE", to: "alice", epoch: 1 });
    stateStore.applyEntry({ type: "ACCOUNT_CREATE", to: "bob", epoch: 1 });
    stateStore.applyEntry({ type: "MINING_REWARD", to: "alice", amount: 100, token: "HONE", epoch: 1 });
    stateStore.applyEntry({ type: "TRANSFER", from: "alice", to: "bob", amount: 30, token: "HONE", epoch: 2 });
    expect(stateStore.getBalance("alice", "HONE")).toBeCloseTo(70, 5);
    expect(stateStore.getBalance("bob", "HONE")).toBeCloseTo(30, 5);
  });

  test("getCurrentBlockCap and setCurrentBlockCap work", () => {
    const MB = 1024 * 1024;
    expect(stateStore.getCurrentBlockCap()).toBe(MB); // default 1 MB
    stateStore.setCurrentBlockCap(2 * MB);
    expect(stateStore.getCurrentBlockCap()).toBe(2 * MB);
  });

  test("resetAll resets currentBlockCap to 1 MB", () => {
    const MB = 1024 * 1024;
    stateStore.setCurrentBlockCap(4 * MB);
    stateStore.resetAll();
    expect(stateStore.getCurrentBlockCap()).toBe(MB);
  });
});
