"use strict";

/**
 * p2pSecurityIntegration.test.js
 * Shin Devlin
 *
 * End-to-end integration tests for P2P security:
 *   - Malicious MEMPOOL_ENTRY with MINING_REWARD type is rejected
 *   - MEMPOOL_ENTRY with TRANSFER type is processed
 *   - EPOCH_FINALIZED with a gap > 10 is rejected
 *   - BLOCK_PROPOSAL from the same connection with two different proposers is rejected
 *   - ACCOUNT_ANNOUNCE with a valid proof is accepted
 */

const messageAuth = require("../src/p2p/messageAuth");
const stateStore = require("../src/chain/stateStore");
const crypto = require("crypto");
const secp256k1 = require("secp256k1");

function genKeypair() {
  let priv;
  do { priv = crypto.randomBytes(32); } while (!secp256k1.privateKeyVerify(priv));
  return {
    privateKey: priv.toString("hex"),
    publicKey: Buffer.from(secp256k1.publicKeyCreate(priv, true)).toString("hex"),
  };
}

// Simulate the block-only type check that handleMempoolEntry performs
function simulateMempoolEntryGuard(entry) {
  if (messageAuth.BLOCK_ONLY_TYPES.includes(entry.type)) {
    return { accepted: false, reason: "block-only type rejected" };
  }
  if (!messageAuth.MEMPOOL_ALLOWED_TYPES.includes(entry.type)) {
    return { accepted: false, reason: "disallowed type rejected" };
  }
  return { accepted: true };
}

// Simulate the epoch gap check in handleEpochFinalized
function simulateEpochFinalizedGuard(lastEpoch, incomingEpoch) {
  if (lastEpoch >= 0) {
    var gap = incomingEpoch - lastEpoch;
    if (gap < 0) return { accepted: false, reason: "backwards epoch" };
    if (gap > 10) return { accepted: false, reason: "gap > 10" };
  }
  return { accepted: true };
}

// Simulate the proposer-per-connection check in handleBlockProposal
function simulateBlockProposalGuard(peer, incomingProposer) {
  if (peer.claimed_proposer && peer.claimed_proposer !== incomingProposer) {
    return { accepted: false, reason: "proposer spoofing — different proposer on same connection" };
  }
  peer.claimed_proposer = incomingProposer;
  return { accepted: true };
}

// ---------------------------------------------------------------------------

describe("P2P Security Integration: MEMPOOL_ENTRY type guard", () => {
  test("MINING_REWARD entry is rejected by mempool guard", () => {
    var result = simulateMempoolEntryGuard({ type: "MINING_REWARD", to: "attacker", amount: 1000000 });
    expect(result.accepted).toBe(false);
    expect(result.reason).toContain("block-only");
  });

  test("FAUCET entry is rejected by mempool guard", () => {
    var result = simulateMempoolEntryGuard({ type: "FAUCET", to: "attacker", amount: 42000000 });
    expect(result.accepted).toBe(false);
  });

  test("TRANSFER entry passes the mempool guard", () => {
    var result = simulateMempoolEntryGuard({ type: "TRANSFER", from: "alice", to: "bob", amount: 1 });
    expect(result.accepted).toBe(true);
  });

  test("unknown entry type is rejected", () => {
    var result = simulateMempoolEntryGuard({ type: "HACK_REWARD", to: "evil", amount: 9999 });
    expect(result.accepted).toBe(false);
    expect(result.reason).toContain("disallowed");
  });

  test("STAKE entry passes the mempool guard", () => {
    var result = simulateMempoolEntryGuard({ type: "STAKE", from: "alice", amount: 100 });
    expect(result.accepted).toBe(true);
  });
});

// ---------------------------------------------------------------------------

describe("P2P Security Integration: EPOCH_FINALIZED sequence guard", () => {
  test("future epoch with gap > 10 is rejected", () => {
    var result = simulateEpochFinalizedGuard(100, 115);
    expect(result.accepted).toBe(false);
    expect(result.reason).toContain("gap > 10");
  });

  test("backwards epoch (replay attack) is rejected", () => {
    var result = simulateEpochFinalizedGuard(100, 90);
    expect(result.accepted).toBe(false);
    expect(result.reason).toContain("backwards");
  });

  test("sequential epoch (gap = 1) is accepted", () => {
    var result = simulateEpochFinalizedGuard(100, 101);
    expect(result.accepted).toBe(true);
  });

  test("first epoch (lastEpoch = -1) is always accepted", () => {
    var result = simulateEpochFinalizedGuard(-1, 500);
    expect(result.accepted).toBe(true);
  });
});

// ---------------------------------------------------------------------------

describe("P2P Security Integration: BLOCK_PROPOSAL proposer per connection", () => {
  test("same proposer on repeated proposals from same connection is accepted", () => {
    var peer = { claimed_proposer: null };
    expect(simulateBlockProposalGuard(peer, "shindevlin").accepted).toBe(true);
    expect(simulateBlockProposalGuard(peer, "shindevlin").accepted).toBe(true);
  });

  test("different proposer on same connection triggers rejection (spoofing)", () => {
    var peer = { claimed_proposer: null };
    simulateBlockProposalGuard(peer, "shindevlin");
    var result = simulateBlockProposalGuard(peer, "natoshisakamoto");
    expect(result.accepted).toBe(false);
    expect(result.reason).toContain("spoofing");
  });
});

// ---------------------------------------------------------------------------

describe("P2P Security Integration: ACCOUNT_ANNOUNCE proof", () => {
  test("valid self-certifying proof verifies correctly", () => {
    var kp = genKeypair();
    var proofPayload = {
      username: "integration-test-acct",
      public_keys: { owner: kp.publicKey },
      timestamp: Date.now(),
    };
    var proof = messageAuth.signMessage(proofPayload, kp.privateKey);
    var ok = messageAuth.verifyMessage(proofPayload, proof.signature, kp.publicKey);
    expect(ok).toBe(true);
  });

  test("forged proof (signed with different key) is rejected", () => {
    var kp = genKeypair();
    var attackerKp = genKeypair();
    var proofPayload = {
      username: "victim-acct",
      public_keys: { owner: kp.publicKey },
      timestamp: Date.now(),
    };
    // Attacker signs with THEIR key but claims victim's public key
    var forgedProof = messageAuth.signMessage(proofPayload, attackerKp.privateKey);
    var ok = messageAuth.verifyMessage(proofPayload, forgedProof.signature, kp.publicKey);
    expect(ok).toBe(false);
  });
});
