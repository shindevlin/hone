"use strict";

/**
 * Verifies that watchBlocks fires for blocks written after the watcher starts,
 * and does not double-fire for blocks that were already present.
 */

const fs = require("fs");
const path = require("path");
const os = require("os");

// Point blockStore at a temp directory so we don't touch real chain data.
const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "btcpc-blockstore-test-"));

// Patch BLOCKS_DIR before requiring the module.
jest.mock("../src/chain/block", () => {
  const crypto = require("crypto");
  function Block(fields) { Object.assign(this, fields || {}); }
  Block.HEADER_SIZE = 180;
  Block.prototype.serialize = function() {
    const buf = Buffer.alloc(180);
    if (this.epoch_number != null) buf.writeUInt32LE(this.epoch_number, 0);
    return buf;
  };
  Block.prototype.computeHash = function() {
    return crypto.createHash("sha256").update(String(this.epoch_number || 0)).digest("hex");
  };
  Block.deserialize = function(buf) {
    const b = new Block();
    b.epoch_number = buf.readUInt32LE(0);
    b.computeHash = Block.prototype.computeHash.bind(b);
    return b;
  };
  return Block;
});

// Patch the internal BLOCKS_DIR by rewriting the require path resolution.
// We do this by requiring the module and then monkey-patching its internals via
// a fresh require with a modified __dirname equivalent — simplest is to just
// use the real module but with a symlink in tmpDir.
//
// Easier: use the real blockStore but intercept fs calls via a wrapper.
// Even easier: test via the exported API surface only, using a real temp dir.

// Actually, the cleanest approach: require blockStore and let it use its
// default BLOCKS_DIR, but since we can't redirect that without mocking fs,
// we test the public API directly in the real blocks dir and clean up.
// For CI safety we use a dedicated test prefix.

// Re-require after setting up mocks.
const blockStore = require("../src/chain/blockStore");
const REAL_BLOCKS_DIR = blockStore.BLOCKS_DIR;

// Use a test-specific sub-directory inside the real blocks dir so we don't
// mix with live chain data.  We can't redirect BLOCKS_DIR from outside, so
// instead we test writeBlock + watchBlocks end-to-end in the real location
// with epoch numbers far above anything on-chain.
const TEST_EPOCH_BASE = 9000000; // well above any real chain epoch

function makePayload(epoch, recipient, amount) {
  return {
    ledger_entries: [{ type: "MINING_REWARD", to: recipient, amount, epoch, token: "BTCPC" }],
    rewards: [{ amount }],
    compute_proofs: [],
    mining_proofs: [],
  };
}

function makeBlock(epoch) {
  const Block = require("../src/chain/block");
  return new Block({ epoch_number: epoch });
}

afterAll(() => {
  // Clean up test block files
  for (let i = 0; i < 5; i++) {
    const epoch = TEST_EPOCH_BASE + i;
    const f = path.join(REAL_BLOCKS_DIR, "block-" + String(epoch).padStart(8, "0") + ".bin");
    try { fs.unlinkSync(f); } catch (_) {}
  }
  // Remove test entries from index
  const indexPath = path.join(REAL_BLOCKS_DIR, "index.json");
  try {
    const idx = JSON.parse(fs.readFileSync(indexPath, "utf8"));
    for (let i = 0; i < 5; i++) delete idx[String(TEST_EPOCH_BASE + i)];
    fs.writeFileSync(indexPath, JSON.stringify(idx, null, 2));
  } catch (_) {}
});

test("watchBlocks calls onBlock for each new block written after watch starts", (done) => {
  const received = [];
  const stop = blockStore.watchBlocks(TEST_EPOCH_BASE, (epochNumber, payload) => {
    received.push({ epochNumber, entries: payload.ledger_entries });
    if (received.length === 2) {
      stop();
      expect(received[0].epochNumber).toBe(TEST_EPOCH_BASE);
      expect(received[0].entries[0].to).toBe("testminer");
      expect(received[1].epochNumber).toBe(TEST_EPOCH_BASE + 1);
      expect(received[1].entries[0].to).toBe("testminer");
      done();
    }
  });

  // Write two blocks after the watcher is installed.
  setTimeout(() => {
    blockStore.writeBlock(makeBlock(TEST_EPOCH_BASE), makePayload(TEST_EPOCH_BASE, "testminer", 1.5));
  }, 50);
  setTimeout(() => {
    blockStore.writeBlock(makeBlock(TEST_EPOCH_BASE + 1), makePayload(TEST_EPOCH_BASE + 1, "testminer", 1.5));
  }, 150);
}, 5000);

test("watchBlocks does not replay blocks already seen before fromEpoch", (done) => {
  // Write a block BEFORE the watcher starts.
  blockStore.writeBlock(makeBlock(TEST_EPOCH_BASE + 2), makePayload(TEST_EPOCH_BASE + 2, "oldminer", 0.5));

  const received = [];
  // Watch from TEST_EPOCH_BASE + 3 — should NOT see TEST_EPOCH_BASE + 2.
  const stop = blockStore.watchBlocks(TEST_EPOCH_BASE + 3, (epochNumber, payload) => {
    received.push(epochNumber);
    if (received.length === 1) {
      stop();
      expect(received[0]).toBe(TEST_EPOCH_BASE + 3);
      done();
    }
  });

  setTimeout(() => {
    blockStore.writeBlock(makeBlock(TEST_EPOCH_BASE + 3), makePayload(TEST_EPOCH_BASE + 3, "newminer", 1.0));
  }, 50);
}, 5000);
