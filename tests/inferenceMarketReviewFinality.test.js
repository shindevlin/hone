"use strict";

const path = require("path");
const os = require("os");

const TEST_DATA_DIR = path.join(os.tmpdir(), "hone-review-finality-" + process.pid);
process.env.HONE_DATA_DIR = TEST_DATA_DIR;

jest.mock("../src/p2p/network", () => ({
  broadcast: jest.fn(),
  NODE_ID: "test-node",
}));

jest.mock("../src/p2p/protocol", () => ({
  createMessage: jest.fn(() => ({})),
}));

jest.mock("../src/services/epochBandwidth", () => ({
  checkAndDeduct: jest.fn().mockReturnValue({ allowed: true, eb_remaining: 9999, cost: 0 }),
  getBalance: jest.fn().mockReturnValue({ eb: 9999, max_eb: 9999, last_epoch: 0 }),
  OPERATION_COSTS: {},
  BANDWIDTH_PER_STAKE_PER_EPOCH: 1.0,
}));

const stateStore = require("../src/chain/stateStore");
const nodeRegistry = require("../src/chain/nodeRegistry");
const ledger = require("../src/services/ledger");
const market = require("../src/services/inferenceMarket");

function createAccount(username, amount) {
  stateStore.applyEntry({
    type: "ACCOUNT_CREATE",
    to: username,
    epoch: 1,
    timestamp: 1,
    account_data: { username, public_keys: {}, chain_addresses: {} },
  });
  stateStore.applyEntry({
    type: "MINING_REWARD",
    to: username,
    amount,
    token: "HONE",
    epoch: 1,
    timestamp: 1,
  });
}

function stakeAccount(username, amount) {
  stateStore.applyEntry({
    type: "STAKE",
    from: username,
    to: "hone_staking_pool",
    amount,
    token: "HONE",
    epoch: 1,
    timestamp: 1,
    delegation_data: { purpose: "verifier" },
  });
}

function registerReviewer(username, stake) {
  nodeRegistry.registerNode(username, "verifier", stake, null, 1, false);
}

function resetAll() {
  stateStore.resetAll();
  nodeRegistry.reset();
  ledger.flushPendingEntries();
}

describe("inference marketplace review and finality", () => {
  beforeEach(() => {
    resetAll();
    stateStore.setChainHeight(100);
    createAccount("buyer1", 10);
    createAccount("miner1", 0);
    createAccount("reviewer1", 300);
    createAccount("reviewer2", 300);
    createAccount("reviewer3", 300);
    registerReviewer("buyer1", 0);
    registerReviewer("miner1", 0);
    registerReviewer("reviewer1", 200);
    registerReviewer("reviewer2", 200);
    registerReviewer("reviewer3", 200);
    stakeAccount("reviewer1", 200);
    stakeAccount("reviewer2", 200);
    stakeAccount("reviewer3", 200);
  });

  afterAll(() => {
    stateStore.resetAll();
    ledger.flushPendingEntries();
  });

  test("acceptance path finalizes after the challenge window and pays review fee", async () => {
    const opened = await market.openJob("buyer1", "test prompt", 1, {
      reviewFee: 0.1,
      challengeFee: 0.2,
      challengeWindowEpochs: 2,
      reviewMode: "human",
    });

    expect(opened.escrow_amount).toBeCloseTo(1.1, 10);
    expect(stateStore.getBalance("buyer1", "HONE")).toBeCloseTo(8.9, 10);

    await market.claimJob(opened.job_id, "miner1");
    await market.submitJob(opened.job_id, "miner1", "result", null, 1);

    const reviewed = await market.reviewJob(opened.job_id, "reviewer1", "accepted", {
      review_mode: "computer",
      challenge_window_epochs: 2,
    });

    expect(reviewed.status).toBe("awaiting_challenge");
    expect(reviewed.challenge_deadline_epoch).toBeGreaterThan(100);

    stateStore.setChainHeight(reviewed.challenge_deadline_epoch + 1);
    const finalized = await market.finalizeJob(opened.job_id, "reviewer1");

    expect(finalized.status).toBe("finalized");
    expect(finalized.challenge_status).toBe("non_fraud");
    expect(stateStore.getInferenceJob(opened.job_id).status).toBe("finalized");
    expect(stateStore.getBalance("miner1", "HONE")).toBeCloseTo(0.9, 10);
    expect(stateStore.getBalance("reviewer1", "HONE")).toBeCloseTo(100.1, 10);
    expect(stateStore.getBalance("buyer1", "HONE")).toBeCloseTo(8.9, 10);
  });

  test("challenge path refunds the inference fee when the appeal upholds the challenge", async () => {
    const opened = await market.openJob("buyer1", "test prompt", 1, {
      reviewFee: 0.1,
      challengeFee: 0.2,
      challengeWindowEpochs: 2,
      reviewMode: "human",
    });

    await market.claimJob(opened.job_id, "miner1");
    await market.submitJob(opened.job_id, "miner1", "result", null, 1);
    await market.reviewJob(opened.job_id, "reviewer1", "accepted", {
      review_mode: "computer",
      challenge_window_epochs: 2,
    });

    const challenged = await market.challengeJob(opened.job_id, "buyer1", "work does not fit");
    expect(challenged.status).toBe("challenged");
    expect(stateStore.getBalance("buyer1", "HONE")).toBeCloseTo(8.7, 10);
    expect(challenged.assigned_reviewers).toHaveLength(3);

    const [appealReviewer1, appealReviewer2, appealReviewer3] = challenged.assigned_reviewers;
    const pending1 = await market.reviewJob(opened.job_id, appealReviewer1, "fraud", {
      review_stage: "appeal",
      review_mode: "human",
      challenge_window_epochs: 2,
    });
    expect(pending1.status).toBe("review_voting");

    const pending2 = await market.reviewJob(opened.job_id, appealReviewer2, "fraud", {
      review_stage: "appeal",
      review_mode: "human",
      challenge_window_epochs: 2,
    });
    expect(pending2.status).toBe("review_voting");

    const resolved = await market.reviewJob(opened.job_id, appealReviewer3, "non_fraud", {
      review_stage: "appeal",
      review_mode: "human",
      challenge_window_epochs: 2,
    });

    expect(resolved.status).toBe("finalized");
    expect(resolved.review_outcome).toBe("fraud");
    expect(stateStore.getInferenceJob(opened.job_id).status).toBe("finalized");
    expect(stateStore.getBalance("buyer1", "HONE")).toBeCloseTo(9.7, 10);
    const slashedReviewer = challenged.assigned_reviewers.find((name) => (stateStore.getSlashRecords(name) || []).length === 1);
    expect(slashedReviewer).toBeTruthy();
    expect(stateStore.getStakePool(slashedReviewer).total_staked).toBeCloseTo(196, 10);
    expect(nodeRegistry.getNode(slashedReviewer).stake).toBeCloseTo(196, 10);
    expect(stateStore.getNodeReputation(slashedReviewer).review.failed).toBeGreaterThan(0);
    expect(stateStore.getBalance("miner1", "HONE")).toBeCloseTo(0, 10);
  });
});
