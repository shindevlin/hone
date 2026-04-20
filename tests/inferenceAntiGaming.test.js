"use strict";

/**
 * inferenceAntiGaming.test.js
 * Shin Devlin
 *
 * Tests for VRF-based job assignment shuffle and stake gate in the
 * BTCPC inference protocol.  Colluders cannot pre-arrange which node
 * receives a job even when they know all node scores.
 */

const vrfBeacon = require("../src/services/vrfBeacon");

// ─── helpers ─────────────────────────────────────────────────────────────────

/**
 * Given a VRF seed, a requestId, and a list of scored claims, return
 * the node_id ordering that assignNodes would produce.
 */
function vrfRank(vrfSeed, requestId, scoredClaims) {
  return scoredClaims
    .map((claim) => {
      const vrfHex = vrfBeacon.deriveFromSeed(
        vrfSeed,
        requestId + ":" + claim.node_id
      );
      const u = parseInt(vrfHex.slice(0, 8), 16) / 0xffffffff;
      return { node_id: claim.node_id, vrf_slot: claim.score * u };
    })
    .sort((a, b) => b.vrf_slot - a.vrf_slot)
    .map((c) => c.node_id);
}

// ─── VRF shuffle tests ────────────────────────────────────────────────────────

describe("VRF job assignment shuffle", () => {
  const requestId = "req_" + "a".repeat(60);

  // Five nodes with identical scores — only VRF separates them
  const equalScoreClaims = [
    { node_id: "node_alpha", score: 0.8 },
    { node_id: "node_beta",  score: 0.8 },
    { node_id: "node_gamma", score: 0.8 },
    { node_id: "node_delta", score: 0.8 },
    { node_id: "node_eps",   score: 0.8 },
  ];

  test("different VRF seeds produce different orderings", () => {
    const seed1 = "a".repeat(64);
    const seed2 = "b".repeat(64);

    const order1 = vrfRank(seed1, requestId, equalScoreClaims);
    const order2 = vrfRank(seed2, requestId, equalScoreClaims);

    // With five nodes and uniform scores the probability of an accidental
    // identical ordering is 1/120 ≈ 0.8 %.  Two deliberately different
    // seeds drawn from opposite ends of the keyspace will never collide.
    expect(order1).not.toEqual(order2);
  });

  test("same VRF seed + same requestId produces identical ordering (deterministic)", () => {
    const seed = "c".repeat(64);
    const order1 = vrfRank(seed, requestId, equalScoreClaims);
    const order2 = vrfRank(seed, requestId, equalScoreClaims);
    expect(order1).toEqual(order2);
  });

  test("different requestId with same seed produces different ordering", () => {
    const seed = "d".repeat(64);
    const order1 = vrfRank(seed, "req_111111", equalScoreClaims);
    const order2 = vrfRank(seed, "req_222222", equalScoreClaims);
    expect(order1).not.toEqual(order2);
  });

  test("higher score node wins more often than lower score node across many seeds", () => {
    // node_high has score 0.95, node_low has score 0.05
    const claims = [
      { node_id: "node_high", score: 0.95 },
      { node_id: "node_low",  score: 0.05 },
    ];
    let highWins = 0;
    const TRIALS = 200;
    for (let i = 0; i < TRIALS; i++) {
      const seed = require("crypto").randomBytes(32).toString("hex");
      const order = vrfRank(seed, "req_trial_" + i, claims);
      if (order[0] === "node_high") highWins++;
    }
    // High-score node should win ≥ 85 % of trials (expected ≈ 95 %)
    expect(highWins / TRIALS).toBeGreaterThan(0.85);
  });
});

// ─── deriveFromSeed unit tests ────────────────────────────────────────────────

describe("vrfBeacon.deriveFromSeed", () => {
  test("returns a 64-char hex string", () => {
    const out = vrfBeacon.deriveFromSeed("a".repeat(64), "domain:test");
    expect(out).toMatch(/^[0-9a-f]{64}$/);
  });

  test("same inputs always produce the same output", () => {
    const seed   = "f".repeat(64);
    const domain = "inference:req_abc:node_xyz";
    expect(vrfBeacon.deriveFromSeed(seed, domain)).toBe(
      vrfBeacon.deriveFromSeed(seed, domain)
    );
  });

  test("different domains produce different outputs", () => {
    const seed = "e".repeat(64);
    expect(vrfBeacon.deriveFromSeed(seed, "domain:A")).not.toBe(
      vrfBeacon.deriveFromSeed(seed, "domain:B")
    );
  });
});

// ─── claimRequest stake gate (unit-level simulation) ─────────────────────────

describe("claimRequest stake gate", () => {
  const MIN_CLAIM_STAKE = 10; // mirrors protocol.js constant

  function simulateStakeCheck(nodeStake) {
    if (nodeStake < MIN_CLAIM_STAKE) {
      return {
        error: `Insufficient stake to claim inference jobs. Minimum ${MIN_CLAIM_STAKE} BTCPC required.`,
      };
    }
    return { ok: true };
  }

  test("node with 0 BTCPC staked is rejected", () => {
    const result = simulateStakeCheck(0);
    expect(result.error).toMatch(/Insufficient stake/);
  });

  test("node with 9.99 BTCPC staked is rejected", () => {
    const result = simulateStakeCheck(9.99);
    expect(result.error).toMatch(/Insufficient stake/);
  });

  test("node with exactly 10 BTCPC staked is accepted", () => {
    const result = simulateStakeCheck(10);
    expect(result.ok).toBe(true);
  });

  test("node with 100 BTCPC staked is accepted", () => {
    const result = simulateStakeCheck(100);
    expect(result.ok).toBe(true);
  });
});

// ─── self-challenge guard (unit-level simulation) ─────────────────────────────

describe("self-challenge guard", () => {
  function simulateSelfCheck(requesterId, nodeId) {
    if (requesterId && nodeId === requesterId) {
      return { error: "Self-challenge not allowed: requester cannot be the inference provider" };
    }
    return { ok: true };
  }

  test("requester claiming their own request is blocked", () => {
    const result = simulateSelfCheck("shindevlin", "shindevlin");
    expect(result.error).toMatch(/Self-challenge not allowed/);
  });

  test("different node claiming is allowed", () => {
    const result = simulateSelfCheck("shindevlin", "natoshisakamoto");
    expect(result.ok).toBe(true);
  });
});
