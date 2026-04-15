"use strict";

const fs = require("fs");
const path = require("path");
const os = require("os");

const TEST_ROOT = path.join(os.tmpdir(), "btcpc-storage-challenge-" + process.pid);
process.env.BTCPC_BLOB_DIR = TEST_ROOT;

const blobStore = require("../src/services/blobStore");
const storageChallenge = require("../src/services/storageChallenge");

describe("storageChallenge", function () {
  beforeEach(function () {
    if (fs.existsSync(TEST_ROOT)) fs.rmSync(TEST_ROOT, { recursive: true, force: true });
    storageChallenge._resetForTests();
  });

  afterAll(function () {
    if (fs.existsSync(TEST_ROOT)) fs.rmSync(TEST_ROOT, { recursive: true, force: true });
  });

  test("validates a host response by challenged range hash", function () {
    var content = Buffer.from("abcdefghijklmnopqrstuvwxyz0123456789");
    var put = blobStore.putBlob(content);
    var challenge = storageChallenge.generateChallenge("host1", put.cid, 10, { offset: 5, length: 8 });
    var response = blobStore.readBlobRange(put.cid, challenge.offset, challenge.length);

    var result = storageChallenge.verifyResponse(challenge.id, response);

    expect(challenge.offset).toBe(5);
    expect(challenge.length).toBe(8);
    expect(challenge.expected_hash).toMatch(/^[a-f0-9]{64}$/);
    expect(result).toEqual({ valid: true, host: "host1" });
  });

  test("rejects wrong bytes even when response length matches", function () {
    var put = blobStore.putBlob(Buffer.from("abcdefghijklmnopqrstuvwxyz0123456789"));
    var challenge = storageChallenge.generateChallenge("host1", put.cid, 10, { offset: 5, length: 8 });

    var result = storageChallenge.verifyResponse(challenge.id, Buffer.from("XXXXXXXX"));

    expect(result.valid).toBe(false);
    expect(result.reason).toBe("range hash mismatch");
    expect(result.failures).toBe(1);
  });

  test("throws when generating a challenge for a missing blob", function () {
    expect(function () {
      storageChallenge.generateChallenge("host1", "a".repeat(64), 10);
    }).toThrow("cannot challenge missing blob");
  });
});
