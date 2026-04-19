"use strict";

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

let _seq = 0;
function rid() { return "req-test-" + (++_seq).toString(16).padStart(8, "0"); }

beforeEach(() => { _jobs.clear(); });

test("3 nodes same result_hash → consensus declared", () => {
  const id = rid();
  createEnsembleJob(id, "qwen3:4b", "ph1", 3, Date.now() + 60000);

  expect(submitContribution(id, "A", "aaaa", "txt", 100, "mh").consensusAchieved).toBe(false);
  expect(submitContribution(id, "B", "aaaa", "txt", 110, "mh").consensusAchieved).toBe(false);
  const r = submitContribution(id, "C", "aaaa", "txt", 120, "mh");
  expect(r.consensusAchieved).toBe(true);
  expect(r.status).toBe("consensus");
  expect(r.consensusNodes.sort()).toEqual(["A", "B", "C"].sort());
  expect(r.result.result_hash).toBe("aaaa");
});

test("2 of 3 match, 1 differs → consensus waits for 3rd matching node", () => {
  const id = rid();
  createEnsembleJob(id, "qwen3:4b", "ph2", 3, Date.now() + 60000);

  submitContribution(id, "A", "bbbb", "txt", 100, "mh");
  submitContribution(id, "B", "bbbb", "txt", 110, "mh");
  const rC = submitContribution(id, "C", "cccc", "txt", 90, "mh");
  expect(rC.consensusAchieved).toBe(false);

  const rD = submitContribution(id, "D", "bbbb", "txt", 105, "mh");
  expect(rD.consensusAchieved).toBe(true);
  expect(rD.consensusNodes).toContain("A");
  expect(rD.consensusNodes).toContain("B");
  expect(rD.consensusNodes).toContain("D");
  expect(rD.consensusNodes).not.toContain("C");
});

test("expireOldJobs marks past-deadline jobs expired", () => {
  const id = rid();
  createEnsembleJob(id, "llama3:8b", "ph3", 3, Date.now() - 1000);
  expect(getJob(id).status).toBe("pending");

  expect(expireOldJobs()).toBeGreaterThanOrEqual(1);
  expect(getJob(id).status).toBe("expired");

  const r = submitContribution(id, "Z", "zzzz", "txt", 50, "mh");
  expect(r.status).toBe("expired");
  expect(r.consensusAchieved).toBe(false);
});

test("work value uses ENSEMBLE_BONUS_MULTIPLIER for consensus, PARTIAL for non-consensus", () => {
  const id = rid();
  const maxTok = 512;
  createEnsembleJob(id, "mistral:7b", "ph4", 3, Date.now() + 60000, maxTok, 0);

  submitContribution(id, "C", "eeee", "txt", 100, "mh");
  submitContribution(id, "A", "dddd", "txt", 200, "mh");
  submitContribution(id, "B", "dddd", "txt", 150, "mh");
  submitContribution(id, "D", "dddd", "txt", 120, "mh");

  expect(getJob(id).status).toBe("consensus");
  const params = 7e9;

  expect(computeEnsembleWorkValue(id, "A", params)).toBe(200 * params * ENSEMBLE_BONUS_MULTIPLIER);
  expect(computeEnsembleWorkValue(id, "B", params)).toBe(150 * params * ENSEMBLE_BONUS_MULTIPLIER);
  expect(computeEnsembleWorkValue(id, "D", params)).toBe(120 * params * ENSEMBLE_BONUS_MULTIPLIER);
  expect(computeEnsembleWorkValue(id, "C", params)).toBe(100 * params * ENSEMBLE_PARTIAL_MULTIPLIER);
  expect(computeEnsembleWorkValue(id, "Z", params)).toBe(0);
});

test("tokens clamped to max_tokens — over-reporting node earns no extra", () => {
  const id = rid();
  createEnsembleJob(id, "qwen3:4b", "ph7", 2, Date.now() + 60000, 512, 0);

  // nodeA reports 1000 tokens but max_tokens=512 → clamped to 512
  submitContribution(id, "A", "hhhh", "txt", 1000, "mh");
  submitContribution(id, "B", "hhhh", "txt", 512, "mh");

  expect(getJob(id).status).toBe("consensus");
  const params = 4e9;
  const workA = computeEnsembleWorkValue(id, "A", params);
  const workB = computeEnsembleWorkValue(id, "B", params);
  // Both should be clamped to 512
  expect(workA).toBe(512 * params * ENSEMBLE_BONUS_MULTIPLIER);
  expect(workA).toBe(workB);
});

test("non-existent job returns not_found", () => {
  const r = submitContribution("no-such-req", "X", "xxxx", "txt", 10, "mh");
  expect(r.status).toBe("not_found");
  expect(r.consensusAchieved).toBe(false);
});

test("second submission from same node overwrites first", () => {
  const id = rid();
  createEnsembleJob(id, "gemma:2b", "ph6", 2, Date.now() + 60000);

  submitContribution(id, "A", "ffff", "txt1", 50, "mh");
  submitContribution(id, "A", "gggg", "txt2", 60, "mh");
  const r = submitContribution(id, "B", "gggg", "txt2", 55, "mh");
  expect(r.consensusAchieved).toBe(true);
  expect(r.result.result_hash).toBe("gggg");
});
