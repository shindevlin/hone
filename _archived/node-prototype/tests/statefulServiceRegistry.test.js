"use strict";

/**
 * Stateful Service Registry tests — v2.14-alpha
 *
 * Tests the stateful overlay on top of serviceRegistry without
 * modifying serviceRegistry.js itself.
 */

var serviceRegistry = require("../src/services/serviceRegistry");
var statefulRegistry = require("../src/services/statefulServiceRegistry");

var VALID_CID = "a".repeat(64);
var VALID_CID_2 = "b".repeat(64);

function basicRuntime(opts) {
  opts = opts || {};
  return {
    type: opts.type || "http",
    binary_cid: opts.binary_cid || VALID_CID,
    cpu: 1,
    ram_mb: 512,
    port: 8080,
    entrypoint: "./server",
  };
}

beforeEach(function () {
  serviceRegistry.resetForTests();
  statefulRegistry.resetForTests();
});

// ─────────────────────────────────────────────────────────────────
// forwardingDeploy
// ─────────────────────────────────────────────────────────────────

describe("statefulServiceRegistry — forwardingDeploy", function () {
  it("produces the same base shape as serviceRegistry.deploy", function () {
    var rec = statefulRegistry.forwardingDeploy(
      "shindevlin",
      "shindevlin/voxel-wars",
      basicRuntime(),
      { epoch: 5, price_per_hour: 0.1 }
    );

    expect(rec.slug).toBe("shindevlin/voxel-wars");
    expect(rec.deployer).toBe("shindevlin");
    expect(rec.binary_cid).toBe(VALID_CID);
    expect(rec.runtime.type).toBe("http");
    expect(rec.status).toBe("active");
    expect(rec.deployed_epoch).toBe(5);
    expect(rec.price_per_hour).toBe(0.1);
  });

  it("persists stateful: true metadata", function () {
    var rec = statefulRegistry.forwardingDeploy(
      "shindevlin",
      "shindevlin/voxel-wars",
      basicRuntime(),
      { stateful: true, snapshot_interval_epochs: 10, replication_factor: 2 }
    );

    expect(rec.stateful).toBe(true);
    expect(rec.snapshot_interval_epochs).toBe(10);
    expect(rec.replication_factor).toBe(2);
  });

  it("stateful defaults to false when not specified", function () {
    var rec = statefulRegistry.forwardingDeploy(
      "shindevlin",
      "shindevlin/voxel-wars",
      basicRuntime()
    );
    expect(rec.stateful).toBe(false);
  });

  it("replication_factor defaults to 3", function () {
    var rec = statefulRegistry.forwardingDeploy(
      "shindevlin",
      "shindevlin/voxel-wars",
      basicRuntime(),
      { stateful: true }
    );
    expect(rec.replication_factor).toBe(3);
  });

  it("last_snapshot_cid and last_snapshot_epoch default to null", function () {
    var rec = statefulRegistry.forwardingDeploy(
      "shindevlin",
      "shindevlin/voxel-wars",
      basicRuntime(),
      { stateful: true }
    );
    expect(rec.last_snapshot_cid).toBeNull();
    expect(rec.last_snapshot_epoch).toBeNull();
  });

  it("v2.13 validation rules still apply (deployer mismatch)", function () {
    expect(function () {
      statefulRegistry.forwardingDeploy(
        "mallory",
        "shindevlin/voxel-wars",
        basicRuntime()
      );
    }).toThrow(/slug deployer prefix/);
  });

  it("v2.13 validation rules still apply (invalid CID)", function () {
    expect(function () {
      statefulRegistry.forwardingDeploy(
        "shindevlin",
        "shindevlin/voxel-wars",
        Object.assign(basicRuntime(), { binary_cid: "not-hex" })
      );
    }).toThrow(/64-char hex/);
  });
});

// ─────────────────────────────────────────────────────────────────
// recordSnapshotCommit
// ─────────────────────────────────────────────────────────────────

describe("statefulServiceRegistry — recordSnapshotCommit", function () {
  beforeEach(function () {
    statefulRegistry.forwardingDeploy(
      "shindevlin",
      "shindevlin/voxel-wars",
      basicRuntime(),
      { stateful: true }
    );
  });

  it("updates last_snapshot_cid and last_snapshot_epoch", function () {
    var result = statefulRegistry.recordSnapshotCommit(
      "shindevlin/voxel-wars",
      VALID_CID,
      42,
      ["host-a", "host-b"]
    );

    expect(result.slug).toBe("shindevlin/voxel-wars");
    expect(result.cid).toBe(VALID_CID);
    expect(result.epoch).toBe(42);
    expect(result.replicas).toEqual(["host-a", "host-b"]);

    var meta = statefulRegistry.getStatefulMeta("shindevlin/voxel-wars");
    expect(meta.last_snapshot_cid).toBe(VALID_CID);
    expect(meta.last_snapshot_epoch).toBe(42);
  });

  it("overwrites previous snapshot commit", function () {
    statefulRegistry.recordSnapshotCommit("shindevlin/voxel-wars", VALID_CID, 10, []);
    statefulRegistry.recordSnapshotCommit("shindevlin/voxel-wars", VALID_CID_2, 20, ["h1"]);

    var meta = statefulRegistry.getStatefulMeta("shindevlin/voxel-wars");
    expect(meta.last_snapshot_cid).toBe(VALID_CID_2);
    expect(meta.last_snapshot_epoch).toBe(20);
  });

  it("throws for unknown service", function () {
    expect(function () {
      statefulRegistry.recordSnapshotCommit("nobody/nothing", VALID_CID, 1, []);
    }).toThrow(/unknown service/);
  });

  it("throws when slug is missing", function () {
    expect(function () {
      statefulRegistry.recordSnapshotCommit(null, VALID_CID, 1, []);
    }).toThrow(/slug required/);
  });

  it("throws when cid is missing", function () {
    expect(function () {
      statefulRegistry.recordSnapshotCommit("shindevlin/voxel-wars", null, 1, []);
    }).toThrow(/cid required/);
  });
});

// ─────────────────────────────────────────────────────────────────
// getStatefulServices
// ─────────────────────────────────────────────────────────────────

describe("statefulServiceRegistry — getStatefulServices", function () {
  beforeEach(function () {
    statefulRegistry.forwardingDeploy(
      "shindevlin",
      "shindevlin/voxel-wars",
      basicRuntime(),
      { stateful: true }
    );
    statefulRegistry.forwardingDeploy(
      "shindevlin",
      "shindevlin/static-site",
      basicRuntime({ type: "static" }),
      { stateful: false }
    );
    statefulRegistry.forwardingDeploy(
      "natoshisakamoto",
      "natoshisakamoto/oracle",
      basicRuntime({ type: "tcp" }),
      { stateful: true }
    );
  });

  it("includes only stateful: true services", function () {
    var list = statefulRegistry.getStatefulServices();
    expect(list.length).toBe(2);
    var slugs = list.map(function (s) { return s.slug; }).sort();
    expect(slugs).toEqual(["natoshisakamoto/oracle", "shindevlin/voxel-wars"]);
  });

  it("excludes stateless services", function () {
    var list = statefulRegistry.getStatefulServices();
    var found = list.filter(function (s) { return s.slug === "shindevlin/static-site"; });
    expect(found.length).toBe(0);
  });

  it("filters by deployer", function () {
    var list = statefulRegistry.getStatefulServices({ deployer: "shindevlin" });
    expect(list.length).toBe(1);
    expect(list[0].slug).toBe("shindevlin/voxel-wars");
  });

  it("filters by status", function () {
    serviceRegistry.retire("shindevlin", "shindevlin/voxel-wars", 99);
    var active = statefulRegistry.getStatefulServices({ status: "active" });
    expect(active.length).toBe(1);
    expect(active[0].slug).toBe("natoshisakamoto/oracle");

    var retired = statefulRegistry.getStatefulServices({ status: "retired" });
    expect(retired.length).toBe(1);
    expect(retired[0].slug).toBe("shindevlin/voxel-wars");
  });

  it("returns merged records with stateful metadata", function () {
    var list = statefulRegistry.getStatefulServices();
    var voxel = list.find(function (s) { return s.slug === "shindevlin/voxel-wars"; });
    expect(voxel.stateful).toBe(true);
    expect(typeof voxel.replication_factor).toBe("number");
    expect(voxel.last_snapshot_cid).toBeNull();
  });
});

// ─────────────────────────────────────────────────────────────────
// getStatefulMeta + getStatefulService
// ─────────────────────────────────────────────────────────────────

describe("statefulServiceRegistry — getStatefulMeta / getStatefulService", function () {
  it("getStatefulMeta returns null for undeployed service", function () {
    expect(statefulRegistry.getStatefulMeta("nobody/nothing")).toBeNull();
  });

  it("getStatefulMeta returns meta after forwardingDeploy", function () {
    statefulRegistry.forwardingDeploy("alice", "alice/app", basicRuntime(), { stateful: true });
    var meta = statefulRegistry.getStatefulMeta("alice/app");
    expect(meta).not.toBeNull();
    expect(meta.stateful).toBe(true);
  });

  it("getStatefulService returns null for stateless service", function () {
    statefulRegistry.forwardingDeploy("alice", "alice/app", basicRuntime(), { stateful: false });
    expect(statefulRegistry.getStatefulService("alice/app")).toBeNull();
  });

  it("getStatefulService returns merged record for stateful service", function () {
    statefulRegistry.forwardingDeploy("alice", "alice/app", basicRuntime(), { stateful: true });
    var svc = statefulRegistry.getStatefulService("alice/app");
    expect(svc).not.toBeNull();
    expect(svc.slug).toBe("alice/app");
    expect(svc.stateful).toBe(true);
  });
});

// ─────────────────────────────────────────────────────────────────
// resetForTests
// ─────────────────────────────────────────────────────────────────

describe("statefulServiceRegistry — resetForTests", function () {
  it("wipes stateful metadata without touching serviceRegistry", function () {
    statefulRegistry.forwardingDeploy(
      "shindevlin",
      "shindevlin/voxel-wars",
      basicRuntime(),
      { stateful: true }
    );
    statefulRegistry.resetForTests();

    // serviceRegistry still has the record (reset was not called on it)
    var svcRecord = serviceRegistry.getService("shindevlin/voxel-wars");
    expect(svcRecord).not.toBeNull();

    // stateful meta is gone
    expect(statefulRegistry.getStatefulMeta("shindevlin/voxel-wars")).toBeNull();
    expect(statefulRegistry.getStatefulServices().length).toBe(0);
  });
});
