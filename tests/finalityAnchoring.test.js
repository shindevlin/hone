"use strict";

/**
 * Four-Tier Finality Anchoring — Unit Tests
 * Shin Devlin
 */

var assert = require("assert");
var fs = require("fs");
var path = require("path");
var os = require("os");
var crypto = require("crypto");

// ─── Module under test ────────────────────────────────────────────
// We load finalityAnchoring fresh per test-group using isolated env vars.
// Node caches modules, so we delete the cache key and re-require with
// the env already set before each require.

function freshLoad(envOverrides) {
  envOverrides = envOverrides || {};
  // Apply env vars
  var restores = [];
  Object.keys(envOverrides).forEach(function (k) {
    var prev = process.env[k];
    process.env[k] = envOverrides[k];
    restores.push(function () {
      if (prev === undefined) delete process.env[k];
      else process.env[k] = prev;
    });
  });

  // Bust module cache for finalityAnchoring so env re-reads happen.
  // In Jest, also call jest.resetModules() so Jest's own module registry
  // is flushed (jest.resetModules is a no-op outside Jest).
  if (typeof jest !== "undefined") jest.resetModules();
  var modKey = require.resolve("../src/chain/finalityAnchoring");
  delete require.cache[modKey];

  var mod = require("../src/chain/finalityAnchoring");
  return { mod: mod, restore: function () { restores.forEach(function (r) { r(); }); } };
}

// ─── Helpers ──────────────────────────────────────────────────────

var SNAPSHOT = { accounts: { shindevlin: 1000 }, epoch: 100, rolling_commitment: "abc" };

function sha256hex(input) {
  return crypto.createHash("sha256").update(input, "utf8").digest("hex");
}

// ─── Tests ────────────────────────────────────────────────────────

var passed = 0;
var failed = 0;
var tests = [];

function test(name, fn) {
  tests.push({ name: name, fn: fn });
}

async function runAll() {
  for (var i = 0; i < tests.length; i++) {
    var t = tests[i];
    try {
      await t.fn();
      console.log("  PASS  " + t.name);
      passed++;
    } catch (e) {
      console.error("  FAIL  " + t.name);
      console.error("        " + e.message);
      failed++;
    }
  }
  console.log("\n" + (passed + failed) + " tests: " + passed + " passed, " + failed + " failed");
  if (failed > 0) {
    // In Jest, throwing causes the test suite to fail properly.
    // Outside Jest, fall back to process.exit.
    if (typeof jest !== "undefined") {
      throw new Error(failed + " finality anchoring test(s) failed — see console.error above");
    }
    process.exit(1);
  }
}

// ─────────────────────────────────────────────────────────────────
// 1. Tier gating: BTCPC_FINALITY_TIERS=1 skips all higher tiers
// ─────────────────────────────────────────────────────────────────

test("tier gating: BTCPC_FINALITY_TIERS=1 returns skipped=true", async function () {
  var r = freshLoad({ BTCPC_FINALITY_TIERS: "1", BTCPC_MIN_NODES_FOR_L2: "0" });
  var result = await r.mod.anchorIfDue(100, SNAPSHOT);
  r.restore();
  assert.strictEqual(result.skipped, true);
  assert.ok(result.reason.includes("native-only"));
});

// ─────────────────────────────────────────────────────────────────
// 2. Node count gate: below MIN_NODES_FOR_L2 → dormant
// ─────────────────────────────────────────────────────────────────

test("node count gate: 2 nodes < 10 = dormant even with TIERS=4", async function () {
  var r = freshLoad({ BTCPC_FINALITY_TIERS: "4", BTCPC_MIN_NODES_FOR_L2: "10" });
  // nodeRegistry has at most 2 nodes in test environment
  var result = await r.mod.anchorIfDue(100, SNAPSHOT);
  r.restore();
  assert.strictEqual(result.skipped, true);
  assert.ok(result.reason.includes("MIN_NODES_FOR_L2"));
});

// ─────────────────────────────────────────────────────────────────
// 3. Node count gate passes when threshold is 0
// ─────────────────────────────────────────────────────────────────

test("node count gate passes when MIN_NODES_FOR_L2=0", async function () {
  var r = freshLoad({ BTCPC_FINALITY_TIERS: "2", BTCPC_MIN_NODES_FOR_L2: "0" });
  var result = await r.mod.anchorIfDue(100, SNAPSHOT);
  r.restore();
  assert.strictEqual(result.skipped, false);
});

// ─────────────────────────────────────────────────────────────────
// 4. L2 anchor payload — correct type and fields
// ─────────────────────────────────────────────────────────────────

test("L2 anchor entry has type=L2_ANCHOR with epoch and state_root", async function () {
  var r = freshLoad({ BTCPC_FINALITY_TIERS: "2", BTCPC_MIN_NODES_FOR_L2: "0" });
  var result = await r.mod.anchorIfDue(100, SNAPSHOT);
  r.restore();
  assert.strictEqual(result.tiersRun.indexOf(2) !== -1, true);
  var l2 = result.anchors.find(function (a) { return a.type === "L2_ANCHOR"; });
  assert.ok(l2, "L2_ANCHOR entry present");
  assert.strictEqual(l2.epoch, 100);
  assert.strictEqual(typeof l2.state_root, "string");
  assert.strictEqual(l2.state_root.length, 64);
  assert.strictEqual(l2.anchor_level, "l2");
});

// ─────────────────────────────────────────────────────────────────
// 5. State root is SHA-256 of the JSON-serialised snapshot
// ─────────────────────────────────────────────────────────────────

test("state root equals SHA-256 of JSON snapshot", function () {
  var r = freshLoad({ BTCPC_FINALITY_TIERS: "2", BTCPC_MIN_NODES_FOR_L2: "0" });
  var expected = sha256hex(JSON.stringify(SNAPSHOT));
  var actual = r.mod._computeStateRoot(SNAPSHOT);
  r.restore();
  assert.strictEqual(actual, expected);
});

// ─────────────────────────────────────────────────────────────────
// 6. ETH ABI encoding — output is 128 hex chars (64 bytes)
// ─────────────────────────────────────────────────────────────────

test("ETH ABI encoding produces 128-char hex string (64 bytes)", function () {
  var r = freshLoad({});
  var hex = r.mod._abiEncodeEthAnchor(1000, "a".repeat(64));
  r.restore();
  assert.strictEqual(hex.length, 128);
  assert.ok(/^[0-9a-f]+$/.test(hex));
});

// ─────────────────────────────────────────────────────────────────
// 7. ETH ABI encoding — epoch encoded as uint256 big-endian
// ─────────────────────────────────────────────────────────────────

test("ETH ABI: epoch 1000 correctly ABI-encoded at bytes 0-31", function () {
  var r = freshLoad({});
  var hex = r.mod._abiEncodeEthAnchor(1000, "00".repeat(32));
  r.restore();
  // 1000 = 0x3E8 → last two bytes of first 32 bytes should be 03e8
  var epochPart = hex.slice(0, 64); // first 32 bytes as hex
  assert.ok(epochPart.endsWith("03e8"), "epoch bytes end with 03e8");
  assert.ok(epochPart.startsWith("0".repeat(60)), "epoch bytes have leading zeros");
});

// ─────────────────────────────────────────────────────────────────
// 8. ETH ABI encoding — state root preserved in bytes 32-63
// ─────────────────────────────────────────────────────────────────

test("ETH ABI: state root occupies bytes 32-63", function () {
  var r = freshLoad({});
  var stateRoot = "dead" + "beef".repeat(15); // 64 hex chars
  var hex = r.mod._abiEncodeEthAnchor(0, stateRoot);
  r.restore();
  var rootPart = hex.slice(64); // bytes 32-63
  assert.strictEqual(rootPart, stateRoot);
});

// ─────────────────────────────────────────────────────────────────
// 9. BTC OP_RETURN — exactly 160 hex chars (80 bytes)
// ─────────────────────────────────────────────────────────────────

test("BTC OP_RETURN is exactly 160 hex chars (80 bytes)", function () {
  var r = freshLoad({});
  var hex = r.mod._buildBtcOpReturn(10000, "ab".repeat(32));
  r.restore();
  assert.strictEqual(hex.length, 160);
  assert.ok(/^[0-9a-f]+$/.test(hex));
});

// ─────────────────────────────────────────────────────────────────
// 10. BTC OP_RETURN — magic bytes "BTCPC" at start
// ─────────────────────────────────────────────────────────────────

test('BTC OP_RETURN starts with "BTCPC" magic (hex 4254435043)', function () {
  var r = freshLoad({});
  var hex = r.mod._buildBtcOpReturn(10000, "00".repeat(32));
  r.restore();
  // "BTCPC" ASCII → hex: 42 54 43 50 43
  assert.ok(hex.startsWith("4254435043"), 'starts with BTCPC magic');
});

// ─────────────────────────────────────────────────────────────────
// 11. BTC OP_RETURN — epoch uint32 BE at bytes 5-8
// ─────────────────────────────────────────────────────────────────

test("BTC OP_RETURN: epoch 10000 (0x2710) at bytes 5-8 big-endian", function () {
  var r = freshLoad({});
  var hex = r.mod._buildBtcOpReturn(10000, "00".repeat(32));
  r.restore();
  // bytes 5-8 in hex are chars 10-17
  var epochHex = hex.slice(10, 18);
  assert.strictEqual(epochHex, "00002710", "epoch 10000 = 0x00002710");
});

// ─────────────────────────────────────────────────────────────────
// 12. BTC OP_RETURN — state root at bytes 9-40
// ─────────────────────────────────────────────────────────────────

test("BTC OP_RETURN: state root at bytes 9-40 (hex chars 18-81)", function () {
  var r = freshLoad({});
  var stateRoot = "cafe" + "babe".repeat(15); // 64 chars
  var hex = r.mod._buildBtcOpReturn(0, stateRoot);
  r.restore();
  var rootPart = hex.slice(18, 18 + 64); // 32 bytes = 64 hex chars from offset 9
  assert.strictEqual(rootPart, stateRoot);
});

// ─────────────────────────────────────────────────────────────────
// 13. Tier 3 only fires at ETH_CADENCE (1000-epoch multiple)
// ─────────────────────────────────────────────────────────────────

test("ETH anchor fires only at epoch 1000 (not 100)", async function () {
  var r = freshLoad({ BTCPC_FINALITY_TIERS: "3", BTCPC_MIN_NODES_FOR_L2: "0" });
  // epoch 100 — L2 cadence but not ETH cadence
  var r100 = await r.mod.anchorIfDue(100, SNAPSHOT);
  assert.strictEqual(r100.tiersRun.indexOf(3), -1, "ETH tier not in tiersRun at epoch 100");
  // epoch 1000 — both L2 and ETH cadence
  var r1000 = await r.mod.anchorIfDue(1000, SNAPSHOT);
  r.restore();
  assert.ok(r1000.tiersRun.indexOf(3) !== -1, "ETH tier runs at epoch 1000");
  assert.ok(r1000.tiersRun.indexOf(2) !== -1, "L2 tier also runs at epoch 1000");
});

// ─────────────────────────────────────────────────────────────────
// 14. Tier 4 fires only at BTC_CADENCE (10000-epoch multiple)
// ─────────────────────────────────────────────────────────────────

test("BTC anchor fires only at epoch 10000 (not 1000)", async function () {
  var r = freshLoad({ BTCPC_FINALITY_TIERS: "4", BTCPC_MIN_NODES_FOR_L2: "0" });
  var r1000 = await r.mod.anchorIfDue(1000, SNAPSHOT);
  assert.strictEqual(r1000.tiersRun.indexOf(4), -1, "BTC tier not at epoch 1000");
  var r10k = await r.mod.anchorIfDue(10000, SNAPSHOT);
  r.restore();
  assert.ok(r10k.tiersRun.indexOf(4) !== -1, "BTC tier runs at epoch 10000");
  assert.ok(r10k.tiersRun.indexOf(3) !== -1, "ETH tier also runs at epoch 10000");
  assert.ok(r10k.tiersRun.indexOf(2) !== -1, "L2 tier also runs at epoch 10000");
});

// ─────────────────────────────────────────────────────────────────
// 15. Anchor log ring buffer — capped at ANCHOR_LOG_CAP
// ─────────────────────────────────────────────────────────────────

test("anchor log ring buffer capped at ANCHOR_LOG_CAP (100)", async function () {
  var r = freshLoad({ BTCPC_FINALITY_TIERS: "2", BTCPC_MIN_NODES_FOR_L2: "0" });
  var cap = r.mod.ANCHOR_LOG_CAP;
  // Fire 150 L2 anchors (multiples of L2_CADENCE)
  var cadence = r.mod.L2_CADENCE;
  for (var i = 1; i <= 150; i++) {
    r.mod._anchorL2(i * cadence, "a".repeat(64));
  }
  var recent = r.mod.getRecentAnchors();
  r.restore();
  assert.ok(recent.length <= cap, "ring buffer capped: " + recent.length + " <= " + cap);
  assert.ok(recent.length > 0, "ring buffer non-empty");
});

// ─────────────────────────────────────────────────────────────────
// 16. Ledger entry types — L2_ANCHOR stateStore
// ─────────────────────────────────────────────────────────────────

test("stateStore applyEntry handles L2_ANCHOR without throwing", function () {
  var stateStore = require("../src/chain/stateStore");
  // Use a unique epoch unlikely to have been applied already
  var testEpoch = 999901;
  assert.doesNotThrow(function () {
    stateStore.applyEntry({
      type: "L2_ANCHOR",
      epoch: testEpoch,
      state_root: "a".repeat(64),
      anchor_level: "l2",
      anchored_at: Date.now(),
    });
  });
  var recent = stateStore.getRecentAnchors();
  assert.ok(Array.isArray(recent), "getRecentAnchors returns array");
  var found = recent.find(function (a) { return a.type === "L2_ANCHOR" && a.epoch === testEpoch; });
  assert.ok(found, "L2_ANCHOR entry stored in anchor log");
});

// ─────────────────────────────────────────────────────────────────
// 17. Ledger entry types — ETH_ANCHOR stateStore
// ─────────────────────────────────────────────────────────────────

test("stateStore applyEntry handles ETH_ANCHOR without throwing", function () {
  var stateStore = require("../src/chain/stateStore");
  var testEpoch = 999902;
  assert.doesNotThrow(function () {
    stateStore.applyEntry({
      type: "ETH_ANCHOR",
      epoch: testEpoch,
      state_root: "b".repeat(64),
      anchor_level: "ethereum",
      calldata_hex: "00".repeat(64),
      anchored_at: Date.now(),
    });
  });
  var recent = stateStore.getRecentAnchors();
  var found = recent.find(function (a) { return a.type === "ETH_ANCHOR" && a.epoch === testEpoch; });
  assert.ok(found, "ETH_ANCHOR entry stored in anchor log");
  assert.strictEqual(found.calldata_hex, "00".repeat(64));
});

// ─────────────────────────────────────────────────────────────────
// 18. Ledger entry types — BTC_ANCHOR stateStore
// ─────────────────────────────────────────────────────────────────

test("stateStore applyEntry handles BTC_ANCHOR without throwing", function () {
  var stateStore = require("../src/chain/stateStore");
  var testEpoch = 999903;
  assert.doesNotThrow(function () {
    stateStore.applyEntry({
      type: "BTC_ANCHOR",
      epoch: testEpoch,
      state_root: "c".repeat(64),
      anchor_level: "bitcoin",
      op_return_hex: "4254435043" + "00".repeat(75),
      anchored_at: Date.now(),
    });
  });
  var recent = stateStore.getRecentAnchors();
  var found = recent.find(function (a) { return a.type === "BTC_ANCHOR" && a.epoch === testEpoch; });
  assert.ok(found, "BTC_ANCHOR entry stored in anchor log");
  assert.ok(found.op_return_hex.startsWith("4254435043"), "op_return_hex magic preserved");
});

// ─────────────────────────────────────────────────────────────────
// Run — Jest-aware wrapper
// ─────────────────────────────────────────────────────────────────

if (typeof it !== "undefined" && typeof describe !== "undefined") {
  // Running inside Jest: wrap the entire custom suite in a single Jest test
  // so Jest awaits completion and reports the result correctly.
  it("finality anchoring — all custom tests pass", function () {
    jest.setTimeout(30000);
    return runAll();
  });
} else {
  // Running directly via node
  runAll().catch(function (err) {
    console.error("Test runner error:", err);
    process.exit(1);
  });
}
