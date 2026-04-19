"use strict";

/**
 * Tests for src/inference/ensembleCoordinator.js
 */

const assert = require("assert");
const {
  createEnsembleJob,
  submitContribution,
  getJob,
  expireOldJobs,
  computeEnsembleWorkValue,
  ENSEMBLE_BONUS_MULTIPLIER,
  ENSEMBLE_PARTIAL_MULTIPLIER,
  _jobs,
} = require("../src/inference/ensembleCoordinator");

// Helper to generate unique request IDs per test
let _seq = 0;
function rid() { return "req-test-" + (++_seq).toString(16).padStart(8, "0"); }

// ---------------------------------------------------------------------------
// Test 1: 3 nodes submit the same result_hash → consensus declared
// ---------------------------------------------------------------------------
{
  const requestId = rid();
  createEnsembleJob(requestId, "qwen3:4b", "prompthash1", 3, Date.now() + 60000);

  const r1 = submitContribution(requestId, "nodeA", "aaaa", "text A", 100, "mhash");
  assert.strictEqual(r1.consensusAchieved, false, "no consensus after 1 node");

  const r2 = submitContribution(requestId, "nodeB", "aaaa", "text A", 110, "mhash");
  assert.strictEqual(r2.consensusAchieved, false, "no consensus after 2 nodes");

  const r3 = submitContribution(requestId, "nodeC", "aaaa", "text A", 120, "mhash");
  assert.strictEqual(r3.consensusAchieved, true, "consensus should be achieved after 3 matching nodes");
  assert.strictEqual(r3.status, "consensus", "status should be 'consensus'");
  assert.deepStrictEqual(r3.consensusNodes.sort(), ["nodeA", "nodeB", "nodeC"].sort(), "all 3 nodes should be consensus nodes");
  assert.ok(r3.result, "result should be present");
  assert.strictEqual(r3.result.result_hash, "aaaa", "consensus hash correct");

  console.log("PASS: 3/3 same hash → consensus");
}

// ---------------------------------------------------------------------------
// Test 2: 2 of 3 nodes match, 1 differs → no consensus until 3rd matching node
// ---------------------------------------------------------------------------
{
  const requestId = rid();
  createEnsembleJob(requestId, "qwen3:4b", "prompthash2", 3, Date.now() + 60000);

  // nodeA and nodeB agree
  submitContribution(requestId, "nodeA", "bbbb", "text B", 100, "mhash");
  submitContribution(requestId, "nodeB", "bbbb", "text B", 110, "mhash");

  // nodeC disagrees
  const rC = submitContribution(requestId, "nodeC", "cccc", "text C", 90, "mhash");
  assert.strictEqual(rC.consensusAchieved, false, "no consensus with 2 matching / 1 different");
  assert.strictEqual(rC.status, "pending", "status stays pending");

  // nodeD agrees with A and B — now 3 match → consensus
  const rD = submitContribution(requestId, "nodeD", "bbbb", "text B", 105, "mhash");
  assert.strictEqual(rD.consensusAchieved, true, "consensus reached when 3rd matching node joins");
  assert.ok(rD.consensusNodes.includes("nodeA"), "nodeA in consensus");
  assert.ok(rD.consensusNodes.includes("nodeB"), "nodeB in consensus");
  assert.ok(rD.consensusNodes.includes("nodeD"), "nodeD in consensus");
  assert.ok(!rD.consensusNodes.includes("nodeC"), "nodeC NOT in consensus");

  console.log("PASS: 2/3 match no consensus, 3rd matching node triggers consensus");
}

// ---------------------------------------------------------------------------
// Test 3: expireOldJobs marks past-deadline jobs as expired
// ---------------------------------------------------------------------------
{
  const requestId = rid();
  // Set deadline in the past
  createEnsembleJob(requestId, "llama3:8b", "prompthash3", 3, Date.now() - 1000);

  const job = getJob(requestId);
  assert.strictEqual(job.status, "pending", "job starts pending");

  const expired = expireOldJobs();
  assert.ok(expired >= 1, "at least one job expired");

  const jobAfter = getJob(requestId);
  assert.strictEqual(jobAfter.status, "expired", "past-deadline job is marked expired");

  // Submitting to expired job should return expired status
  const r = submitContribution(requestId, "nodeZ", "zzzz", "text Z", 50, "mhash");
  assert.strictEqual(r.status, "expired", "submitting to expired job returns expired status");
  assert.strictEqual(r.consensusAchieved, false, "no consensus on expired job");

  console.log("PASS: expireOldJobs marks past-deadline jobs");
}

// ---------------------------------------------------------------------------
// Test 4: work value calculation with ENSEMBLE_BONUS_MULTIPLIER
//
// Scenario: 3 nodes, min_size=3. All three submit contributions first
// (so all are stored), then we check work values.
// nodeA and nodeB agree; nodeC disagrees. A fourth node tips consensus.
// ---------------------------------------------------------------------------
{
  const requestId = rid();
  // min_size=3: nodeC submits first (different hash, stored), then A/B/D agree
  createEnsembleJob(requestId, "mistral:7b", "prompthash4", 3, Date.now() + 60000);

  // nodeC disagrees — stored FIRST, before any consensus
  submitContribution(requestId, "nodeC", "eeee", "text E", 100, "mhash");
  submitContribution(requestId, "nodeA", "dddd", "text D", 200, "mhash");
  submitContribution(requestId, "nodeB", "dddd", "text D", 150, "mhash");
  // nodeD tips consensus (3rd node matching "dddd")
  submitContribution(requestId, "nodeD", "dddd", "text D", 120, "mhash");

  const job = getJob(requestId);
  assert.strictEqual(job.status, "consensus", "consensus reached after 3 matching nodes");

  const paramCount = 7e9; // 7 billion params

  // Consensus nodes (nodeA, nodeB, nodeD): multiplier=1.5
  const workA = computeEnsembleWorkValue(requestId, "nodeA", paramCount);
  const expectedA = 200 * paramCount * ENSEMBLE_BONUS_MULTIPLIER;
  assert.strictEqual(workA, expectedA, "consensus node work value uses ENSEMBLE_BONUS_MULTIPLIER");

  const workB = computeEnsembleWorkValue(requestId, "nodeB", paramCount);
  const expectedB = 150 * paramCount * ENSEMBLE_BONUS_MULTIPLIER;
  assert.strictEqual(workB, expectedB, "second consensus node work value correct");

  const workD = computeEnsembleWorkValue(requestId, "nodeD", paramCount);
  const expectedD = 120 * paramCount * ENSEMBLE_BONUS_MULTIPLIER;
  assert.strictEqual(workD, expectedD, "fourth consensus node work value correct");

  // Non-consensus node (nodeC): stored before consensus, different hash → partial
  const workC = computeEnsembleWorkValue(requestId, "nodeC", paramCount);
  const expectedC = 100 * paramCount * ENSEMBLE_PARTIAL_MULTIPLIER;
  assert.strictEqual(workC, expectedC, "non-consensus node gets ENSEMBLE_PARTIAL_MULTIPLIER");

  // Node that never contributed
  const workZ = computeEnsembleWorkValue(requestId, "nodeZ", paramCount);
  assert.strictEqual(workZ, 0, "non-contributor gets 0 work value");

  // Sanity: bonus > partial
  assert.ok(workA > workC, "consensus work > non-consensus work");

  console.log("PASS: work value calculation with multipliers");
}

// ---------------------------------------------------------------------------
// Test 5: submitting to non-existent job returns not_found
// ---------------------------------------------------------------------------
{
  const r = submitContribution("no-such-req", "nodeX", "xxxx", "text X", 10, "mhash");
  assert.strictEqual(r.status, "not_found", "non-existent job returns not_found");
  assert.strictEqual(r.consensusAchieved, false);
  console.log("PASS: non-existent job returns not_found");
}

// ---------------------------------------------------------------------------
// Test 6: second submission from same node overwrites first
// ---------------------------------------------------------------------------
{
  const requestId = rid();
  createEnsembleJob(requestId, "gemma:2b", "prompthash6", 2, Date.now() + 60000);

  submitContribution(requestId, "nodeA", "ffff", "text F1", 50, "mhash");
  // nodeA resubmits with a different hash (e.g. retry)
  submitContribution(requestId, "nodeA", "gggg", "text F2", 60, "mhash");

  // nodeB agrees with nodeA's SECOND hash
  const r = submitContribution(requestId, "nodeB", "gggg", "text F2", 55, "mhash");
  assert.strictEqual(r.consensusAchieved, true, "consensus on overwritten hash");
  assert.strictEqual(r.result.result_hash, "gggg", "consensus is on the latest hash");

  console.log("PASS: second submission from same node overwrites first");
}

console.log("\nAll ensembleCoordinator tests passed.");
