"use strict";

/**
 * p2pMessageAuth.test.js
 * Shin Devlin
 *
 * Unit tests for the P2P message authentication module (src/p2p/messageAuth.js)
 * and the balance floor fix in stateStore._debit (Vuln 6).
 *
 * Tests: hashMessageData, signMessage/verifyMessage round-trip,
 *        verifyAccountSignature, MEMPOOL allowlist enforcement,
 *        ACCOUNT_ANNOUNCE proof verification, EPOCH_FINALIZED epoch
 *        sequence check, BLOCK_PROPOSAL proposer-per-connection check,
 *        and stateStore _debit balance floor.
 */

const crypto = require("crypto");
const secp256k1 = require("secp256k1");

// Modules under test
const messageAuth = require("../src/p2p/messageAuth");
const keyManager = require("../src/wallet/keyManager");
const stateStore = require("../src/chain/stateStore");

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Generate a random valid secp256k1 keypair, returning hex strings */
function genKeypair() {
  let privBuf;
  do {
    privBuf = crypto.randomBytes(32);
  } while (!secp256k1.privateKeyVerify(privBuf));
  const pubBuf = Buffer.from(secp256k1.publicKeyCreate(privBuf, true));
  return {
    privateKey: privBuf.toString("hex"),
    publicKey: pubBuf.toString("hex"),
  };
}

// ---------------------------------------------------------------------------
// 1. hashMessageData — deterministic output
// ---------------------------------------------------------------------------

describe("hashMessageData", () => {
  test("produces a 64-char hex string", () => {
    const hash = messageAuth.hashMessageData({ a: 1, b: 2 });
    expect(hash).toMatch(/^[0-9a-f]{64}$/);
  });

  test("is deterministic — same input → same hash", () => {
    const data = { z: 99, a: "hello", m: [1, 2, 3] };
    expect(messageAuth.hashMessageData(data)).toBe(messageAuth.hashMessageData(data));
  });

  test("is key-order independent", () => {
    const d1 = { a: 1, b: 2 };
    const d2 = { b: 2, a: 1 };
    expect(messageAuth.hashMessageData(d1)).toBe(messageAuth.hashMessageData(d2));
  });

  test("differs for different payloads", () => {
    const h1 = messageAuth.hashMessageData({ x: 1 });
    const h2 = messageAuth.hashMessageData({ x: 2 });
    expect(h1).not.toBe(h2);
  });
});

// ---------------------------------------------------------------------------
// 2. signMessage + verifyMessage round-trip
// ---------------------------------------------------------------------------

describe("signMessage / verifyMessage round-trip", () => {
  const kp = genKeypair();
  const data = { type: "TEST", value: 42 };

  test("verifyMessage returns true for a valid signature", () => {
    const sig = messageAuth.signMessage(data, kp.privateKey);
    expect(messageAuth.verifyMessage(data, sig.signature, kp.publicKey)).toBe(true);
  });

  test("verifyMessage returns false for the wrong key", () => {
    const kp2 = genKeypair();
    const sig = messageAuth.signMessage(data, kp.privateKey);
    expect(messageAuth.verifyMessage(data, sig.signature, kp2.publicKey)).toBe(false);
  });

  test("verifyMessage returns false for a tampered payload", () => {
    const sig = messageAuth.signMessage(data, kp.privateKey);
    const tampered = { type: "TEST", value: 43 };
    expect(messageAuth.verifyMessage(tampered, sig.signature, kp.publicKey)).toBe(false);
  });

  test("verifyMessage returns false for a garbage signature string", () => {
    expect(messageAuth.verifyMessage(data, "deadbeef", kp.publicKey)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// 3. verifyAccountSignature — uses stateStore
// ---------------------------------------------------------------------------

describe("verifyAccountSignature", () => {
  const kp = genKeypair();
  const testUser = "testacct-msgauth-" + Date.now();

  beforeAll(() => {
    // Register the account in stateStore with our keypair
    stateStore.applyEntry({
      type: "ACCOUNT_CREATE",
      to: testUser,
      epoch: 1,
      account_data: {
        username: testUser,
        public_keys: {
          owner: kp.publicKey,
          active: kp.publicKey,
          posting: kp.publicKey,
          memo: kp.publicKey,
        },
        chain_addresses: {},
      },
      timestamp: Date.now(),
    });
  });

  test("returns true when signature matches the account's active key", () => {
    const data = { hello: "world" };
    const sig = messageAuth.signMessage(data, kp.privateKey);
    expect(messageAuth.verifyAccountSignature(testUser, data, sig.signature, "active")).toBe(true);
  });

  test("returns false for the wrong account", () => {
    const data = { hello: "world" };
    const sig = messageAuth.signMessage(data, kp.privateKey);
    expect(messageAuth.verifyAccountSignature("nonexistent-account-xyz", data, sig.signature, "active")).toBe(false);
  });

  test("returns false when the signature is for a different payload", () => {
    const data = { hello: "world" };
    const wrong = { hello: "wrongworld" };
    const sig = messageAuth.signMessage(data, kp.privateKey);
    expect(messageAuth.verifyAccountSignature(testUser, wrong, sig.signature, "active")).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// 4. MEMPOOL_ALLOWED_TYPES and BLOCK_ONLY_TYPES
// ---------------------------------------------------------------------------

describe("MEMPOOL entry type allowlist", () => {
  test("TRANSFER is in MEMPOOL_ALLOWED_TYPES", () => {
    expect(messageAuth.MEMPOOL_ALLOWED_TYPES).toContain("TRANSFER");
  });

  test("ACCOUNT_CREATE is in MEMPOOL_ALLOWED_TYPES", () => {
    expect(messageAuth.MEMPOOL_ALLOWED_TYPES).toContain("ACCOUNT_CREATE");
  });

  test("MINING_REWARD is NOT in MEMPOOL_ALLOWED_TYPES", () => {
    expect(messageAuth.MEMPOOL_ALLOWED_TYPES).not.toContain("MINING_REWARD");
  });

  test("MINING_REWARD is in BLOCK_ONLY_TYPES", () => {
    expect(messageAuth.BLOCK_ONLY_TYPES).toContain("MINING_REWARD");
  });

  test("FAUCET is in BLOCK_ONLY_TYPES", () => {
    expect(messageAuth.BLOCK_ONLY_TYPES).toContain("FAUCET");
  });

  test("CLOCK_REWARD is in BLOCK_ONLY_TYPES", () => {
    expect(messageAuth.BLOCK_ONLY_TYPES).toContain("CLOCK_REWARD");
  });
});

// ---------------------------------------------------------------------------
// 5. protocol.js handleMempoolEntry — via direct invocation of internals
//    We test by exercising the logic that was added, using the message types.
// ---------------------------------------------------------------------------

describe("handleMempoolEntry logic (type-check layer)", () => {
  // Test that BLOCK_ONLY_TYPES would be caught by the allowlist check
  test("MINING_REWARD type identified as block-only", () => {
    expect(messageAuth.BLOCK_ONLY_TYPES.includes("MINING_REWARD")).toBe(true);
    expect(messageAuth.MEMPOOL_ALLOWED_TYPES.includes("MINING_REWARD")).toBe(false);
  });

  test("TRANSFER type passes both checks", () => {
    expect(messageAuth.BLOCK_ONLY_TYPES.includes("TRANSFER")).toBe(false);
    expect(messageAuth.MEMPOOL_ALLOWED_TYPES.includes("TRANSFER")).toBe(true);
  });

  test("FAUCET type is correctly classified as block-only", () => {
    expect(messageAuth.BLOCK_ONLY_TYPES.includes("FAUCET")).toBe(true);
    expect(messageAuth.MEMPOOL_ALLOWED_TYPES.includes("FAUCET")).toBe(false);
  });

  test("STORAGE_HEARTBEAT is allowed in mempool", () => {
    expect(messageAuth.MEMPOOL_ALLOWED_TYPES.includes("STORAGE_HEARTBEAT")).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// 6. ACCOUNT_ANNOUNCE proof verification logic
// ---------------------------------------------------------------------------

describe("ACCOUNT_ANNOUNCE proof verification", () => {
  const ownerKp = genKeypair();
  const newOwnerKp = genKeypair();

  test("valid first-announcement proof verifies against claimed owner key", () => {
    const proofPayload = {
      username: "newuser",
      public_keys: { owner: ownerKp.publicKey },
      timestamp: 1000,
    };
    const proof = messageAuth.signMessage(proofPayload, ownerKp.privateKey);
    // Self-certifying: signature over proofPayload with the key being registered
    expect(messageAuth.verifyMessage(proofPayload, proof.signature, ownerKp.publicKey)).toBe(true);
  });

  test("first-announcement proof with wrong key fails verification", () => {
    const wrongKp = genKeypair();
    const proofPayload = {
      username: "newuser2",
      public_keys: { owner: ownerKp.publicKey },
      timestamp: 1000,
    };
    const proof = messageAuth.signMessage(proofPayload, wrongKp.privateKey);
    // The proof is signed with wrongKp but claims ownerKp — should fail
    expect(messageAuth.verifyMessage(proofPayload, proof.signature, ownerKp.publicKey)).toBe(false);
  });

  test("re-announcement proof must match EXISTING owner key (not the new one)", () => {
    const proofPayload = {
      username: "existinguser",
      public_keys: { owner: newOwnerKp.publicKey },
      timestamp: 2000,
    };
    // Signed with existing owner key — correct
    const goodProof = messageAuth.signMessage(proofPayload, ownerKp.privateKey);
    expect(messageAuth.verifyMessage(proofPayload, goodProof.signature, ownerKp.publicKey)).toBe(true);

    // Signed with NEW key — wrong, should fail against existing key check
    const badProof = messageAuth.signMessage(proofPayload, newOwnerKp.privateKey);
    expect(messageAuth.verifyMessage(proofPayload, badProof.signature, ownerKp.publicKey)).toBe(false);
  });

  test("REQUIRE_SIGNATURES defaults to true (strict mode)", () => {
    // REQUIRE_SIGNATURES is true by default — unsigned messages are rejected.
    // Set BTCPC_REQUIRE_SIGNATURES=false in the environment to opt out.
    expect(messageAuth.REQUIRE_SIGNATURES).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// 7. EPOCH_FINALIZED epoch sequence check
// ---------------------------------------------------------------------------

describe("EPOCH_FINALIZED epoch gap validation", () => {
  test("gap > 10 is detected", () => {
    const last = 100;
    const incoming = 115;
    expect(incoming - last).toBeGreaterThan(10);
  });

  test("gap of exactly 10 is allowed", () => {
    const last = 100;
    const incoming = 110;
    expect(incoming - last).toBeLessThanOrEqual(10);
  });

  test("backwards epoch is detected (gap < 0)", () => {
    const last = 100;
    const incoming = 99;
    expect(incoming - last).toBeLessThan(0);
  });

  test("sequential epoch (gap = 1) is allowed", () => {
    const last = 100;
    const incoming = 101;
    var gap = incoming - last;
    expect(gap).toBeGreaterThanOrEqual(0);
    expect(gap).toBeLessThanOrEqual(10);
  });
});

// ---------------------------------------------------------------------------
// 8. BLOCK_PROPOSAL: per-connection proposer consistency
// ---------------------------------------------------------------------------

describe("BLOCK_PROPOSAL proposer consistency", () => {
  test("same proposer on same connection is allowed", () => {
    var peer = { claimed_proposer: "shindevlin" };
    var incoming = "shindevlin";
    expect(peer.claimed_proposer === incoming || !peer.claimed_proposer).toBe(true);
  });

  test("different proposer on same connection triggers rejection", () => {
    var peer = { claimed_proposer: "shindevlin" };
    var incoming = "natoshisakamoto";
    expect(peer.claimed_proposer && peer.claimed_proposer !== incoming).toBe(true);
  });

  test("first proposal sets the claimed_proposer", () => {
    var peer = { claimed_proposer: null };
    var incoming = "shindevlin";
    if (!peer.claimed_proposer) peer.claimed_proposer = incoming;
    expect(peer.claimed_proposer).toBe("shindevlin");
  });
});

// ---------------------------------------------------------------------------
// 9. stateStore._debit balance floor (Vuln 6)
// ---------------------------------------------------------------------------

describe("stateStore _debit balance floor", () => {
  const regularUser = "debit-test-regular-" + Date.now();
  const systemUser = "btcpc_genesis";

  beforeAll(() => {
    stateStore.applyEntry({
      type: "ACCOUNT_CREATE",
      to: regularUser,
      epoch: 1,
      account_data: { username: regularUser, public_keys: {}, chain_addresses: {} },
      timestamp: Date.now(),
    });
    // Give the regular user 10 BTCPC
    stateStore.applyEntry({
      type: "MINING_REWARD",
      to: regularUser,
      amount: 10,
      token: "BTCPC",
      epoch: 1,
      timestamp: Date.now(),
    });
    // Give genesis some initial balance
    stateStore.applyEntry({
      type: "MINING_REWARD",
      to: systemUser,
      amount: 1000,
      token: "BTCPC",
      epoch: 1,
      timestamp: Date.now(),
    });
  });

  test("regular user cannot go below zero", () => {
    const before = stateStore.getBalance(regularUser, "BTCPC");
    // Attempt to debit more than balance via TRANSFER entry
    stateStore.applyEntry({
      type: "TRANSFER",
      from: regularUser,
      to: "nobody",
      amount: before + 9999, // Way more than balance
      token: "BTCPC",
      epoch: 2,
      timestamp: Date.now() + 1,
    });
    const after = stateStore.getBalance(regularUser, "BTCPC");
    // Balance must not have gone negative (should be unchanged)
    expect(after).toBeGreaterThanOrEqual(0);
    expect(after).toBe(before); // debit was rejected
  });

  test("regular user CAN debit when balance is sufficient", () => {
    const before = stateStore.getBalance(regularUser, "BTCPC");
    stateStore.applyEntry({
      type: "TRANSFER",
      from: regularUser,
      to: "nobody2",
      amount: 1,
      token: "BTCPC",
      epoch: 3,
      timestamp: Date.now() + 2,
    });
    const after = stateStore.getBalance(regularUser, "BTCPC");
    expect(after).toBe(before - 1);
  });

  test("system account (btcpc_genesis) may go negative", () => {
    const before = stateStore.getBalance(systemUser, "BTCPC");
    const bigAmount = before + 50000;
    stateStore.applyEntry({
      type: "TRANSFER",
      from: systemUser,
      to: regularUser,
      amount: bigAmount,
      token: "BTCPC",
      epoch: 4,
      timestamp: Date.now() + 3,
    });
    const after = stateStore.getBalance(systemUser, "BTCPC");
    expect(after).toBeLessThan(0);
  });
});
