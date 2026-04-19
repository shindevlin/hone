"use strict";

/**
 * Tests for src/scientific/scientificComputeEngine.js
 */

// Isolate blob store and ledger so tests don't write to disk or chain
jest.mock("../src/services/blobStore", () => ({
  putBlob: jest.fn((buf) => ({ cid: "a".repeat(64), size: buf.length, existed: false })),
}));

jest.mock("../src/services/ledger", () => ({
  recordScientificResult: jest.fn().mockResolvedValue({}),
}));

const engine = require("../src/scientific/scientificComputeEngine");

const {
  createJob,
  startJob,
  completeJob,
  getJob,
  getOpenJobs,
  computeActualFee,
  computeNodePayout,
  recordLatency,
  getPathLatency,
  findOptimalShardOrder,
  OPEN_SOURCE_DISCOUNT,
  SCIENCE_NODE_MULTIPLIER,
} = engine;

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

function makeJob(overrides) {
  return createJob(Object.assign({
    title: "Test job",
    type: "general",
    model: "llama3",
    input_data: Buffer.from("test input").toString("base64"),
    requester: "shindevlin",
    max_fee: 10,
    open_source: false,
  }, overrides || {}));
}

// ─────────────────────────────────────────────────────────────────
// Fee discount
// ─────────────────────────────────────────────────────────────────

test("createJob with open_source=true applies 40% discount to fee", () => {
  var job = makeJob({ max_fee: 100, open_source: true });
  // fee_paid should be 60% of max_fee
  expect(job.fee_paid).toBeCloseTo(60, 5);
  expect(job.fee_paid).toBe(computeActualFee(100, true));
});

test("createJob with open_source=false leaves fee unchanged", () => {
  var job = makeJob({ max_fee: 100, open_source: false });
  expect(job.fee_paid).toBe(100);
  expect(job.fee_paid).toBe(computeActualFee(100, false));
});

test("computeActualFee applies correct discount constant", () => {
  expect(computeActualFee(50, true)).toBeCloseTo(50 * (1 - OPEN_SOURCE_DISCOUNT), 8);
  expect(computeActualFee(50, false)).toBe(50);
  expect(computeActualFee(0, true)).toBe(0);
});

// ─────────────────────────────────────────────────────────────────
// Node payout multiplier
// ─────────────────────────────────────────────────────────────────

test("computeNodePayout multiplies by SCIENCE_NODE_MULTIPLIER for open_source jobs", () => {
  var job = makeJob({ open_source: true });
  var jobId = job.job_id;
  // Manually inject work values (normally set during completeJob)
  job.work_values = { "nodeA": 10 };
  var payout = computeNodePayout(jobId, "nodeA");
  expect(payout).toBeCloseTo(10 * SCIENCE_NODE_MULTIPLIER, 8);
});

test("computeNodePayout does NOT multiply for closed jobs", () => {
  var job = makeJob({ open_source: false });
  var jobId = job.job_id;
  job.work_values = { "nodeB": 10 };
  var payout = computeNodePayout(jobId, "nodeB");
  expect(payout).toBe(10);
});

test("computeNodePayout returns 0 for unknown job", () => {
  expect(computeNodePayout("nonexistent-job-id", "nodeX")).toBe(0);
});

test("computeNodePayout returns 0 for unknown nodeId", () => {
  var job = makeJob({ open_source: true });
  job.work_values = { "nodeC": 5 };
  expect(computeNodePayout(job.job_id, "nodeNobody")).toBe(0);
});

// ─────────────────────────────────────────────────────────────────
// getOpenJobs
// ─────────────────────────────────────────────────────────────────

test("getOpenJobs returns only queued jobs", () => {
  var jobQ = makeJob({ title: "queued job" });
  var jobR = makeJob({ title: "running job" });

  // Start the second job to move it to running
  startJob(jobR.job_id, "shard-group-001");

  var openJobs = getOpenJobs();
  var openIds = openJobs.map(function(j) { return j.job_id; });
  expect(openIds).toContain(jobQ.job_id);
  expect(openIds).not.toContain(jobR.job_id);
});

// ─────────────────────────────────────────────────────────────────
// Latency recording + path calculation
// ─────────────────────────────────────────────────────────────────

test("recordLatency + getPathLatency calculates sum correctly", () => {
  recordLatency("n1", "n2", 10);
  recordLatency("n2", "n3", 20);
  recordLatency("n3", "n4", 30);

  var latency = getPathLatency(["n1", "n2", "n3", "n4"]);
  expect(latency).toBe(60);
});

test("getPathLatency returns Infinity for unknown hop", () => {
  recordLatency("nA", "nB", 5);
  // nB→nC is not recorded
  var latency = getPathLatency(["nA", "nB", "nC"]);
  expect(latency).toBe(Infinity);
});

test("getPathLatency returns 0 for single-node list", () => {
  expect(getPathLatency(["solo"])).toBe(0);
});

test("getPathLatency returns 0 for empty list", () => {
  expect(getPathLatency([])).toBe(0);
});

// ─────────────────────────────────────────────────────────────────
// findOptimalShardOrder
// ─────────────────────────────────────────────────────────────────

test("findOptimalShardOrder reorders nodes to minimize latency (3-node case)", () => {
  // n1→n3 is cheap (5ms), n1→n2 is expensive (100ms)
  // Optimal order starting from n1: n1 → n3 → n2 (total 5 + 15 = 20ms)
  // Original order n1 → n2 → n3 (total 100 + 15 = 115ms)
  recordLatency("ord1", "ord2", 100);
  recordLatency("ord1", "ord3", 5);
  recordLatency("ord3", "ord2", 15);

  var shards = [
    { nodeId: "ord1" },
    { nodeId: "ord2" },
    { nodeId: "ord3" },
  ];

  var ordered = findOptimalShardOrder(shards);
  var nodeIds = ordered.map(function(s) { return s.nodeId; });

  // Should start with ord1 (fixed), then ord3 (cheaper hop), then ord2
  expect(nodeIds[0]).toBe("ord1");
  expect(nodeIds[1]).toBe("ord3");
  expect(nodeIds[2]).toBe("ord2");
});

test("findOptimalShardOrder falls back to original order when no latency data", () => {
  var shards = [
    { nodeId: "unknown1" },
    { nodeId: "unknown2" },
    { nodeId: "unknown3" },
  ];
  var ordered = findOptimalShardOrder(shards);
  expect(ordered[0].nodeId).toBe("unknown1");
  expect(ordered[1].nodeId).toBe("unknown2");
  expect(ordered[2].nodeId).toBe("unknown3");
});

test("findOptimalShardOrder returns original array for single shard", () => {
  var shards = [{ nodeId: "solo" }];
  expect(findOptimalShardOrder(shards)).toEqual(shards);
});
