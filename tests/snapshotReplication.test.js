"use strict";

/**
 * Snapshot Replication tests — v2.14-alpha
 *
 * Tests the pure data-layer snapshot module via injected mock blobStore
 * and mock hostSelector. No real BTCPC-FS disk I/O, no real chain calls.
 */

var snapshotReplication = require("../src/services/snapshotReplication");

// ─────────────────────────────────────────────────────────────────
// Mock helpers
// ─────────────────────────────────────────────────────────────────

function makeMockBlobStore() {
  var blobs = new Map();
  var nextCid = 0;

  return {
    _blobs: blobs,
    putBlob: function (buffer) {
      if (!Buffer.isBuffer(buffer)) throw new Error("putBlob requires a Buffer");
      if (buffer.length === 0) throw new Error("blob is empty");
      // Use a deterministic CID derived from content length for tests.
      // Real blobStore uses sha256; here we use a padded counter.
      var cid = Buffer.from(buffer).toString("hex").padStart(64, "0").slice(0, 64);
      // If content is too short to pad to 64 hex chars, hash it ourselves.
      var crypto = require("crypto");
      cid = crypto.createHash("sha256").update(buffer).digest("hex");
      blobs.set(cid, buffer);
      return { cid: cid, size: buffer.length, existed: blobs.has(cid) };
    },
    getBlob: function (cid) {
      if (!blobs.has(cid)) throw new Error("blob not found: " + cid);
      return blobs.get(cid);
    },
    hasBlob: function (cid) {
      return blobs.has(cid);
    },
  };
}

/**
 * Mock host selector that returns exactly `count` synthetic host names.
 * Handles replication_factor > available hosts gracefully.
 */
function makeMockHostSelector(availableHosts) {
  return {
    selectHosts: function (params) {
      var n = params.target_active || 0;
      var picked = (availableHosts || []).slice(0, n);
      return {
        active_hosts: picked,
        cold_hosts: [],
        actual_active: picked.length,
        actual_cold: 0,
        under_replicated: picked.length < n,
        candidates_considered: (availableHosts || []).length,
        rejected_by_region: 0,
      };
    },
  };
}

function makeReplicator(opts) {
  var bs = opts.blobStore || makeMockBlobStore();
  var hs = opts.hostSelector || makeMockHostSelector(["host-a", "host-b", "host-c", "host-d", "host-e"]);
  return snapshotReplication.createReplicator({
    blobStore: bs,
    hostSelector: hs,
    replicationFactor: opts.replicationFactor || 3,
  });
}

var SLUG = "shindevlin/voxel-wars";
var SLUG2 = "natoshisakamoto/oracle";

// ─────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────

describe("snapshotReplication — saveSnapshot", function () {
  it("returns valid CID + replicas + created_at", function () {
    var r = makeReplicator({});
    var bytes = Buffer.from("state-v1");
    var result = r.saveSnapshot(SLUG, bytes);
    expect(typeof result.cid).toBe("string");
    expect(result.cid.length).toBe(64);
    expect(Array.isArray(result.replicas)).toBe(true);
    expect(result.replicas.length).toBe(3);
    expect(typeof result.created_at).toBe("number");
    expect(result.created_at).toBeGreaterThan(0);
    expect(result.size).toBe(bytes.length);
  });

  it("stores bytes in blobStore (can retrieve by CID)", function () {
    var bs = makeMockBlobStore();
    var r = makeReplicator({ blobStore: bs });
    var bytes = Buffer.from("snapshot-content");
    var result = r.saveSnapshot(SLUG, bytes);
    expect(bs.hasBlob(result.cid)).toBe(true);
    expect(bs.getBlob(result.cid).toString()).toBe("snapshot-content");
  });

  it("replication_factor 1 picks one host", function () {
    var r = makeReplicator({ replicationFactor: 1 });
    var res = r.saveSnapshot(SLUG, Buffer.from("s1"));
    expect(res.replicas.length).toBe(1);
  });

  it("replication_factor 5 picks five hosts when available", function () {
    var r = makeReplicator({ replicationFactor: 5 });
    var res = r.saveSnapshot(SLUG, Buffer.from("s5"));
    expect(res.replicas.length).toBe(5);
  });

  it("replication_factor 5 picks fewer hosts when pool is smaller", function () {
    var r = makeReplicator({
      replicationFactor: 5,
      hostSelector: makeMockHostSelector(["host-a", "host-b"]),
    });
    var res = r.saveSnapshot(SLUG, Buffer.from("small-pool"));
    expect(res.replicas.length).toBe(2); // best-effort
  });

  it("fails gracefully when blobStore.putBlob throws", function () {
    var brokenBlobStore = {
      putBlob: function () { throw new Error("disk full"); },
      getBlob: function () { return null; },
      hasBlob: function () { return false; },
    };
    var r = snapshotReplication.createReplicator({
      blobStore: brokenBlobStore,
      hostSelector: makeMockHostSelector(["h1"]),
    });
    expect(function () {
      r.saveSnapshot(SLUG, Buffer.from("data"));
    }).toThrow("disk full");
  });

  it("rejects non-Buffer snapshot bytes", function () {
    var r = makeReplicator({});
    expect(function () {
      r.saveSnapshot(SLUG, "not-a-buffer");
    }).toThrow(/Buffer/);
  });

  it("rejects missing slug", function () {
    var r = makeReplicator({});
    expect(function () {
      r.saveSnapshot(null, Buffer.from("x"));
    }).toThrow(/slug required/);
  });
});

describe("snapshotReplication — loadLatestSnapshot", function () {
  it("returns null when no snapshots exist", function () {
    var r = makeReplicator({});
    expect(r.loadLatestSnapshot("nobody/nothing")).toBeNull();
  });

  it("returns the most recent saved snapshot", function () {
    var r = makeReplicator({});
    r.saveSnapshot(SLUG, Buffer.from("state-a"));
    r.saveSnapshot(SLUG, Buffer.from("state-b"));
    var snap = r.loadLatestSnapshot(SLUG);
    expect(snap).not.toBeNull();
    expect(snap.bytes.toString()).toBe("state-b");
    expect(typeof snap.cid).toBe("string");
    expect(typeof snap.created_at).toBe("number");
    expect(Array.isArray(snap.replicas)).toBe(true);
  });

  it("returns null when all snapshots are missing from blobStore", function () {
    var bs = makeMockBlobStore();
    var r = snapshotReplication.createReplicator({
      blobStore: bs,
      hostSelector: makeMockHostSelector(["h1"]),
    });
    r.saveSnapshot(SLUG, Buffer.from("data"));
    // Now wipe the blob store
    bs._blobs.clear();
    var snap = r.loadLatestSnapshot(SLUG);
    expect(snap).toBeNull();
  });

  it("returns an earlier snapshot if the latest blob is missing", function () {
    var bs = makeMockBlobStore();
    var r = snapshotReplication.createReplicator({
      blobStore: bs,
      hostSelector: makeMockHostSelector(["h1"]),
    });
    r.saveSnapshot(SLUG, Buffer.from("old-state"));
    var res2 = r.saveSnapshot(SLUG, Buffer.from("new-state"));
    // Remove the latest blob
    bs._blobs.delete(res2.cid);
    var snap = r.loadLatestSnapshot(SLUG);
    expect(snap).not.toBeNull();
    expect(snap.bytes.toString()).toBe("old-state");
  });
});

describe("snapshotReplication — loadSnapshotByCid", function () {
  it("returns bytes for a known CID", function () {
    var r = makeReplicator({});
    var res = r.saveSnapshot(SLUG, Buffer.from("specific-state"));
    var bytes = r.loadSnapshotByCid(res.cid);
    expect(bytes).not.toBeNull();
    expect(bytes.toString()).toBe("specific-state");
  });

  it("returns null for an unknown CID", function () {
    var r = makeReplicator({});
    var bytes = r.loadSnapshotByCid("a".repeat(64));
    expect(bytes).toBeNull();
  });
});

describe("snapshotReplication — listSnapshots", function () {
  it("returns newest first", function () {
    var r = makeReplicator({});
    r.saveSnapshot(SLUG, Buffer.from("s1"));
    r.saveSnapshot(SLUG, Buffer.from("s2"));
    r.saveSnapshot(SLUG, Buffer.from("s3"));
    var list = r.listSnapshots(SLUG);
    expect(list.length).toBe(3);
    // Newest first — verify created_at ordering
    expect(list[0].created_at).toBeGreaterThanOrEqual(list[1].created_at);
    expect(list[1].created_at).toBeGreaterThanOrEqual(list[2].created_at);
  });

  it("respects limit option", function () {
    var r = makeReplicator({});
    r.saveSnapshot(SLUG, Buffer.from("s1"));
    r.saveSnapshot(SLUG, Buffer.from("s2"));
    r.saveSnapshot(SLUG, Buffer.from("s3"));
    var list = r.listSnapshots(SLUG, { limit: 2 });
    expect(list.length).toBe(2);
  });

  it("returns empty array for unknown slug", function () {
    var r = makeReplicator({});
    expect(r.listSnapshots("nobody/nothing")).toEqual([]);
  });

  it("returns each entry with cid, created_at, replicas, size", function () {
    var r = makeReplicator({});
    r.saveSnapshot(SLUG, Buffer.from("entry-test"));
    var list = r.listSnapshots(SLUG);
    var entry = list[0];
    expect(typeof entry.cid).toBe("string");
    expect(typeof entry.created_at).toBe("number");
    expect(Array.isArray(entry.replicas)).toBe(true);
    expect(typeof entry.size).toBe("number");
  });
});

describe("snapshotReplication — pruneOldSnapshots", function () {
  it("keeps the most recent N and deletes the rest", function () {
    var r = makeReplicator({});
    r.saveSnapshot(SLUG, Buffer.from("s1"));
    r.saveSnapshot(SLUG, Buffer.from("s2"));
    r.saveSnapshot(SLUG, Buffer.from("s3"));
    r.saveSnapshot(SLUG, Buffer.from("s4"));
    r.saveSnapshot(SLUG, Buffer.from("s5"));

    var deleted = r.pruneOldSnapshots(SLUG, 3);
    expect(deleted).toBe(2);

    var list = r.listSnapshots(SLUG);
    expect(list.length).toBe(3);
  });

  it("does not delete anything when count <= keep", function () {
    var r = makeReplicator({});
    r.saveSnapshot(SLUG, Buffer.from("s1"));
    r.saveSnapshot(SLUG, Buffer.from("s2"));
    var deleted = r.pruneOldSnapshots(SLUG, 5);
    expect(deleted).toBe(0);
    expect(r.listSnapshots(SLUG).length).toBe(2);
  });

  it("most recent snapshot is retained after pruning", function () {
    var r = makeReplicator({});
    r.saveSnapshot(SLUG, Buffer.from("old-data"));
    r.saveSnapshot(SLUG, Buffer.from("new-data"));
    r.pruneOldSnapshots(SLUG, 1);
    var snap = r.loadLatestSnapshot(SLUG);
    expect(snap.bytes.toString()).toBe("new-data");
  });
});

describe("snapshotReplication — verifyReplica", function () {
  it("returns true when blob is locally available (mock always has it)", function () {
    var r = makeReplicator({});
    var res = r.saveSnapshot(SLUG, Buffer.from("verify-me"));
    var result = r.verifyReplica("host-a", res.cid);
    expect(result).toBe(true);
  });

  it("returns false for an unknown CID", function () {
    var r = makeReplicator({});
    var result = r.verifyReplica("host-a", "b".repeat(64));
    expect(result).toBe(false);
  });

  it("returns a boolean in both cases", function () {
    var r = makeReplicator({});
    var resA = r.verifyReplica("host-a", "c".repeat(64));
    expect(typeof resA).toBe("boolean");
  });
});

describe("snapshotReplication — resetForTests", function () {
  it("wipes all snapshot metadata", function () {
    var r = makeReplicator({});
    r.saveSnapshot(SLUG, Buffer.from("data"));
    r.saveSnapshot(SLUG2, Buffer.from("data2"));
    r.resetForTests();
    expect(r.listSnapshots(SLUG)).toEqual([]);
    expect(r.listSnapshots(SLUG2)).toEqual([]);
    expect(r.loadLatestSnapshot(SLUG)).toBeNull();
  });

  it("does not affect blobStore (bytes persist)", function () {
    var bs = makeMockBlobStore();
    var r = snapshotReplication.createReplicator({
      blobStore: bs,
      hostSelector: makeMockHostSelector(["h1"]),
    });
    var res = r.saveSnapshot(SLUG, Buffer.from("persistent"));
    r.resetForTests();
    // bytes are still in blobStore
    expect(bs.hasBlob(res.cid)).toBe(true);
    // but metadata is gone
    expect(r.loadLatestSnapshot(SLUG)).toBeNull();
  });
});
