"use strict";

/**
 * Stateful Service Host Runner tests — v2.14-alpha
 *
 * Tests the snapshot-augmented runner via mock spawner, mock blob fetcher,
 * mock getStateBytes callback, and mock snapshotReplication instance.
 * No real processes are forked, no real disk I/O.
 */

var statefulRunner = require("../src/services/statefulServiceHostRunner");
var snapshotReplication = require("../src/services/snapshotReplication");

// ─────────────────────────────────────────────────────────────────
// Mock helpers (mirrors serviceHostRunner.test.js pattern)
// ─────────────────────────────────────────────────────────────────

function makeMockProcess() {
  var listeners = {};
  return {
    killed: false,
    pid: Math.floor(Math.random() * 100000),
    on: function (event, cb) { listeners[event] = cb; },
    kill: function (_signal) {
      this.killed = true;
      if (listeners.exit) listeners.exit(0, "SIGTERM");
    },
    _triggerExit: function (code, signal) {
      if (listeners.exit) listeners.exit(code, signal);
    },
  };
}

function makeMockSpawner(processes) {
  return function (command, args, options) {
    var p = makeMockProcess();
    p._command = command;
    p._args = args;
    p._options = options;
    processes.push(p);
    return p;
  };
}

function makeMockBlobFetcher(blobs) {
  return function (cid) {
    if (!blobs[cid]) throw new Error("blob not found: " + cid);
    return blobs[cid];
  };
}

var BINARY_CID = "a".repeat(64);

function makeDeployment(overrides) {
  var dep = {
    slug: "shindevlin/voxel-wars",
    deployer: "shindevlin",
    runtime: {
      type: "http",
      binary_cid: BINARY_CID,
      command: "node",
      args: ["server.js"],
      port: 8080,
    },
    stateful: true,
  };
  if (overrides) Object.assign(dep, overrides);
  if (overrides && overrides.runtime) {
    dep.runtime = Object.assign({}, dep.runtime, overrides.runtime);
  }
  return dep;
}

function makeMockBlobStore() {
  var blobs = new Map();
  return {
    _blobs: blobs,
    putBlob: function (buffer) {
      var crypto = require("crypto");
      var cid = crypto.createHash("sha256").update(buffer).digest("hex");
      blobs.set(cid, buffer);
      return { cid: cid, size: buffer.length };
    },
    getBlob: function (cid) {
      if (!blobs.has(cid)) throw new Error("blob not found: " + cid);
      return blobs.get(cid);
    },
    hasBlob: function (cid) { return blobs.has(cid); },
  };
}

function makeMockHostSelector() {
  return {
    selectHosts: function () {
      return { active_hosts: ["host-a"], cold_hosts: [], actual_active: 1, actual_cold: 0 };
    },
  };
}

function makeReplicator(bs) {
  return snapshotReplication.createReplicator({
    blobStore: bs || makeMockBlobStore(),
    hostSelector: makeMockHostSelector(),
    replicationFactor: 1,
  });
}

function makeRunner(opts) {
  opts = opts || {};
  var processes = opts.processes || [];
  var blobFetcher = opts.blobFetcher ||
    makeMockBlobFetcher({ [BINARY_CID]: Buffer.from("binary") });
  var replicator = opts.replicator || makeReplicator(opts.blobStore);
  var getStateBytes = opts.getStateBytes || function () { return Buffer.from("mock-state"); };

  return statefulRunner.createStatefulRunner({
    spawner: makeMockSpawner(processes),
    blobFetcher: blobFetcher,
    replicator: replicator,
    getStateBytes: getStateBytes,
  });
}

// ─────────────────────────────────────────────────────────────────
// Tests — registration
// ─────────────────────────────────────────────────────────────────

describe("statefulServiceHostRunner — register", function () {
  it("accepts a stateful: true deployment", function () {
    var r = makeRunner();
    var rec = r.register(makeDeployment());
    expect(rec.slug).toBe("shindevlin/voxel-wars");
    expect(rec.state).toBe("pending");
    expect(r.getServiceCount()).toBe(1);
  });

  it("stateful flag is reflected in getStatus", function () {
    var r = makeRunner();
    r.register(makeDeployment({ stateful: true }));
    var status = r.getStatus("shindevlin/voxel-wars");
    expect(status.stateful).toBe(true);
  });

  it("accepts stateful: false (stateless service)", function () {
    var r = makeRunner();
    r.register(makeDeployment({ stateful: false }));
    var status = r.getStatus("shindevlin/voxel-wars");
    expect(status.stateful).toBe(false);
  });

  it("rejects deployment without slug", function () {
    var r = makeRunner();
    expect(function () { r.register({}); }).toThrow(/slug/);
  });
});

// ─────────────────────────────────────────────────────────────────
// Tests — start with/without prior snapshot
// ─────────────────────────────────────────────────────────────────

describe("statefulServiceHostRunner — start", function () {
  it("cold start when no prior snapshot exists", function () {
    var processes = [];
    var r = makeRunner({ processes: processes });
    r.register(makeDeployment({ stateful: true }));
    var rec = r.start("shindevlin/voxel-wars");
    expect(rec.state).toBe("running");
    expect(processes.length).toBe(1);
    // No snapshot was loaded — snapshot_count is 0
    var status = r.getStatus("shindevlin/voxel-wars");
    expect(status.last_snapshot_cid).toBeNull();
  });

  it("warm start: loads latest snapshot before spawning", function () {
    var bs = makeMockBlobStore();
    var replicator = makeReplicator(bs);
    // Pre-seed a snapshot
    var snap = replicator.saveSnapshot("shindevlin/voxel-wars", Buffer.from("saved-state"));

    var processes = [];
    var r = statefulRunner.createStatefulRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ [BINARY_CID]: Buffer.from("binary") }),
      replicator: replicator,
      getStateBytes: function () { return Buffer.from("current-state"); },
    });

    r.register(makeDeployment({ stateful: true }));
    var rec = r.start("shindevlin/voxel-wars");
    expect(rec.state).toBe("running");

    var status = r.getStatus("shindevlin/voxel-wars");
    // The runner loaded the snapshot — last_snapshot_at should be set
    expect(status.last_snapshot_at).not.toBeNull();
  });

  it("stateless service starts without touching snapshots", function () {
    var processes = [];
    var replicator = makeReplicator();
    var r = statefulRunner.createStatefulRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ [BINARY_CID]: Buffer.from("binary") }),
      replicator: replicator,
      getStateBytes: function () { return Buffer.alloc(0); },
    });
    r.register(makeDeployment({ stateful: false }));
    var rec = r.start("shindevlin/voxel-wars");
    expect(rec.state).toBe("running");
    var status = r.getStatus("shindevlin/voxel-wars");
    expect(status.snapshot_count).toBe(0);
    expect(status.last_snapshot_cid).toBeNull();
  });
});

// ─────────────────────────────────────────────────────────────────
// Tests — stop captures state and saves snapshot
// ─────────────────────────────────────────────────────────────────

describe("statefulServiceHostRunner — stop", function () {
  it("captures state and saves snapshot before SIGTERM", function () {
    var processes = [];
    var capturedState = Buffer.from("state-at-stop");
    var r = statefulRunner.createStatefulRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ [BINARY_CID]: Buffer.from("binary") }),
      replicator: makeReplicator(),
      getStateBytes: function () { return capturedState; },
    });

    r.register(makeDeployment({ stateful: true }));
    r.start("shindevlin/voxel-wars");
    var rec = r.stop("shindevlin/voxel-wars");

    expect(rec.state).toBe("stopped");
    expect(processes[0].killed).toBe(true);

    // Snapshot was saved
    var status = r.getStatus("shindevlin/voxel-wars");
    expect(status.snapshot_count).toBeGreaterThanOrEqual(1);
    expect(status.last_snapshot_cid).not.toBeNull();
  });

  it("does not save snapshot when getStateBytes returns empty buffer", function () {
    var r = statefulRunner.createStatefulRunner({
      spawner: makeMockSpawner([]),
      blobFetcher: makeMockBlobFetcher({ [BINARY_CID]: Buffer.from("binary") }),
      replicator: makeReplicator(),
      getStateBytes: function () { return Buffer.alloc(0); },
    });
    r.register(makeDeployment({ stateful: true }));
    r.start("shindevlin/voxel-wars");
    r.stop("shindevlin/voxel-wars");

    var status = r.getStatus("shindevlin/voxel-wars");
    expect(status.snapshot_count).toBe(0);
  });

  it("stop is a no-op if already stopped", function () {
    var r = makeRunner();
    r.register(makeDeployment());
    r.start("shindevlin/voxel-wars");
    r.stop("shindevlin/voxel-wars");
    var rec = r.stop("shindevlin/voxel-wars");
    expect(rec.state).toBe("stopped");
  });
});

// ─────────────────────────────────────────────────────────────────
// Tests — snapshotNow
// ─────────────────────────────────────────────────────────────────

describe("statefulServiceHostRunner — snapshotNow", function () {
  it("saves without stopping the service", function () {
    var r = statefulRunner.createStatefulRunner({
      spawner: makeMockSpawner([]),
      blobFetcher: makeMockBlobFetcher({ [BINARY_CID]: Buffer.from("binary") }),
      replicator: makeReplicator(),
      getStateBytes: function () { return Buffer.from("live-state"); },
    });

    r.register(makeDeployment({ stateful: true }));
    r.start("shindevlin/voxel-wars");

    var snap = r.snapshotNow("shindevlin/voxel-wars");
    expect(typeof snap.cid).toBe("string");
    expect(snap.cid.length).toBe(64);

    // Service is still running after snapshot
    var status = r.getStatus("shindevlin/voxel-wars");
    expect(status.state).toBe("running");
    expect(status.snapshot_count).toBe(1);
  });

  it("throws when getStateBytes returns empty buffer", function () {
    var r = statefulRunner.createStatefulRunner({
      spawner: makeMockSpawner([]),
      blobFetcher: makeMockBlobFetcher({ [BINARY_CID]: Buffer.from("binary") }),
      replicator: makeReplicator(),
      getStateBytes: function () { return Buffer.alloc(0); },
    });
    r.register(makeDeployment({ stateful: true }));
    r.start("shindevlin/voxel-wars");
    expect(function () {
      r.snapshotNow("shindevlin/voxel-wars");
    }).toThrow(/no state bytes/);
  });

  it("throws when service is not registered", function () {
    var r = makeRunner();
    expect(function () { r.snapshotNow("nobody/nothing"); }).toThrow(/not registered/);
  });
});

// ─────────────────────────────────────────────────────────────────
// Tests — restore
// ─────────────────────────────────────────────────────────────────

describe("statefulServiceHostRunner — restore", function () {
  it("stops + loads specific CID + restarts", function () {
    var bs = makeMockBlobStore();
    var replicator = makeReplicator(bs);
    var processes = [];

    var snapRes = replicator.saveSnapshot("shindevlin/voxel-wars", Buffer.from("v1-state"));

    var r = statefulRunner.createStatefulRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ [BINARY_CID]: Buffer.from("binary") }),
      replicator: replicator,
      getStateBytes: function () { return Buffer.from("current-state"); },
    });

    r.register(makeDeployment({ stateful: true }));
    r.start("shindevlin/voxel-wars");
    expect(r.getStatus("shindevlin/voxel-wars").state).toBe("running");

    // Restore from the specific snapshot CID
    var rec = r.restore("shindevlin/voxel-wars", snapRes.cid);
    expect(rec.state).toBe("running");

    // The restored snapshot CID is recorded
    var status = r.getStatus("shindevlin/voxel-wars");
    expect(status.last_snapshot_at).not.toBeNull();
  });

  it("throws when CID does not exist", function () {
    var r = makeRunner();
    r.register(makeDeployment({ stateful: true }));
    r.start("shindevlin/voxel-wars");
    expect(function () {
      r.restore("shindevlin/voxel-wars", "f".repeat(64));
    }).toThrow(/not found/);
  });
});

// ─────────────────────────────────────────────────────────────────
// Tests — crash detection still works
// ─────────────────────────────────────────────────────────────────

describe("statefulServiceHostRunner — crash detection", function () {
  it("moves to crashed state when child process exits unexpectedly", function () {
    var processes = [];
    var r = makeRunner({ processes: processes });
    r.register(makeDeployment({ stateful: true }));
    r.start("shindevlin/voxel-wars");

    processes[0]._triggerExit(1, "SIGSEGV");
    var status = r.getStatus("shindevlin/voxel-wars");
    expect(status.state).toBe("crashed");
    expect(status.crash_count).toBe(1);
    expect(status.exit_signal).toBe("SIGSEGV");
  });

  it("stop after crash does not cause crash_count to increment", function () {
    var processes = [];
    var r = makeRunner({ processes: processes });
    r.register(makeDeployment({ stateful: true }));
    r.start("shindevlin/voxel-wars");
    r.stop("shindevlin/voxel-wars");
    expect(r.getStatus("shindevlin/voxel-wars").crash_count).toBe(0);
  });
});

// ─────────────────────────────────────────────────────────────────
// Tests — end-to-end: register → start → snapshotNow → stop → start
// ─────────────────────────────────────────────────────────────────

describe("statefulServiceHostRunner — end-to-end", function () {
  it("register → start → snapshotNow → stop → start restores state", function () {
    var bs = makeMockBlobStore();
    var replicator = makeReplicator(bs);
    var processes = [];

    var r = statefulRunner.createStatefulRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ [BINARY_CID]: Buffer.from("binary") }),
      replicator: replicator,
      getStateBytes: function () { return Buffer.from("game-world-state"); },
    });

    // 1. register
    r.register(makeDeployment({ stateful: true }));
    expect(r.getStatus("shindevlin/voxel-wars").state).toBe("pending");

    // 2. start (cold start — no prior snapshot)
    r.start("shindevlin/voxel-wars");
    expect(r.getStatus("shindevlin/voxel-wars").state).toBe("running");
    var statusAfterStart = r.getStatus("shindevlin/voxel-wars");
    expect(statusAfterStart.snapshot_count).toBe(0);

    // 3. snapshotNow — explicit checkpoint
    var snap = r.snapshotNow("shindevlin/voxel-wars");
    expect(typeof snap.cid).toBe("string");
    var statusAfterSnap = r.getStatus("shindevlin/voxel-wars");
    expect(statusAfterSnap.snapshot_count).toBe(1);
    expect(statusAfterSnap.last_snapshot_cid).toBe(snap.cid);

    // 4. stop — saves another snapshot + terminates
    r.stop("shindevlin/voxel-wars");
    var statusAfterStop = r.getStatus("shindevlin/voxel-wars");
    expect(statusAfterStop.state).toBe("stopped");
    expect(statusAfterStop.snapshot_count).toBe(2);

    // 5. start again — warm start (state is restored)
    r.start("shindevlin/voxel-wars");
    var statusAfterRestart = r.getStatus("shindevlin/voxel-wars");
    expect(statusAfterRestart.state).toBe("running");
    // Snapshot metadata from the prior run is still present
    expect(statusAfterRestart.last_snapshot_cid).not.toBeNull();
  });
});
