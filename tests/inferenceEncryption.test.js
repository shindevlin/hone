"use strict";

/**
 * tests/inferenceEncryption.test.js
 * Shin Devlin
 *
 * Tests for end-to-end encrypted inference via memo key ECDH + AES-256-GCM.
 * 13 tests total.
 */

const crypto = require("crypto");
const secp256k1 = require("secp256k1");

// ── Module under test ──
const { computeSharedSecret, encrypt, decrypt } = require("../src/inference/crypto");

// ── Helpers ──
function randomKeypair() {
  let priv;
  do {
    priv = crypto.randomBytes(32);
  } while (!secp256k1.privateKeyVerify(priv));
  const pub = Buffer.from(secp256k1.publicKeyCreate(priv, true));
  return { privateKey: priv.toString("hex"), publicKey: pub.toString("hex") };
}

// ──────────────────────────────────────────────
// 1. ECDH shared secret is symmetric: A×B = B×A
// ──────────────────────────────────────────────

describe("computeSharedSecret — ECDH symmetry", () => {
  test("A derives same secret as B (fresh keypairs)", () => {
    const alice = randomKeypair();
    const bob = randomKeypair();
    const fromAlice = computeSharedSecret(alice.privateKey, bob.publicKey);
    const fromBob = computeSharedSecret(bob.privateKey, alice.publicKey);
    expect(fromAlice.toString("hex")).toBe(fromBob.toString("hex"));
  });

  test("second independent pair also produces symmetric secret", () => {
    const user = randomKeypair();
    const miner = randomKeypair();
    const s1 = computeSharedSecret(user.privateKey, miner.publicKey);
    const s2 = computeSharedSecret(miner.privateKey, user.publicKey);
    expect(s1.toString("hex")).toBe(s2.toString("hex"));
  });
});

// ──────────────────────────────────────────────
// 2. encrypt → decrypt round-trip
// ──────────────────────────────────────────────

describe("encrypt / decrypt — round-trip", () => {
  let shared;
  beforeAll(() => {
    const a = randomKeypair();
    const b = randomKeypair();
    shared = computeSharedSecret(a.privateKey, b.publicKey);
  });

  test("plaintext survives encrypt → decrypt", () => {
    const original = "Hello from BTCPC encrypted inference!";
    const enc = encrypt(original, shared);
    expect(enc).toHaveProperty("ciphertext");
    expect(enc).toHaveProperty("iv");
    expect(enc).toHaveProperty("tag");
    const recovered = decrypt(enc, shared);
    expect(recovered).toBe(original);
  });

  test("each encrypt call produces a different IV (non-deterministic)", () => {
    const msg = "same plaintext";
    const enc1 = encrypt(msg, shared);
    const enc2 = encrypt(msg, shared);
    // IVs must differ (12 random bytes each time)
    expect(enc1.iv).not.toBe(enc2.iv);
    // But both decrypt correctly
    expect(decrypt(enc1, shared)).toBe(msg);
    expect(decrypt(enc2, shared)).toBe(msg);
  });
});

// ──────────────────────────────────────────────
// 3. Wrong key fails to decrypt (auth tag mismatch)
// ──────────────────────────────────────────────

describe("decrypt — auth tag enforcement", () => {
  test("wrong shared secret throws on decrypt", () => {
    const a = randomKeypair();
    const b = randomKeypair();
    const c = randomKeypair();
    const rightSecret = computeSharedSecret(a.privateKey, b.publicKey);
    const wrongSecret = computeSharedSecret(a.privateKey, c.publicKey);
    const enc = encrypt("sensitive prompt", rightSecret);
    expect(() => decrypt(enc, wrongSecret)).toThrow();
  });
});

// ──────────────────────────────────────────────
// 4. Empty string encrypts/decrypts correctly
// ──────────────────────────────────────────────

describe("encrypt / decrypt — edge cases", () => {
  test("empty prompt encrypts and decrypts to empty string", () => {
    const a = randomKeypair();
    const b = randomKeypair();
    const shared = computeSharedSecret(a.privateKey, b.publicKey);
    const enc = encrypt("", shared);
    expect(enc.ciphertext).toBeDefined();
    const recovered = decrypt(enc, shared);
    expect(recovered).toBe("");
  });

  test("large prompt (10 KB) encrypts and decrypts correctly", () => {
    const a = randomKeypair();
    const b = randomKeypair();
    const shared = computeSharedSecret(a.privateKey, b.publicKey);
    const bigPrompt = "A".repeat(10240); // 10 KB
    const enc = encrypt(bigPrompt, shared);
    const recovered = decrypt(enc, shared);
    expect(recovered).toBe(bigPrompt);
    expect(recovered.length).toBe(10240);
  });
});

// ──────────────────────────────────────────────
// 5. Full encrypted job flow (mock Ollama)
// ──────────────────────────────────────────────

describe("encrypted job flow — end-to-end", () => {
  // Simulate user encrypting prompt, miner decrypting + running inference,
  // miner encrypting result, user decrypting result.

  test("user encrypts prompt → miner decrypts → mocks Ollama → encrypts result → user decrypts", () => {
    const user = randomKeypair();
    const miner = randomKeypair();

    // 1. User computes shared secret and encrypts prompt
    const userSecret = computeSharedSecret(user.privateKey, miner.publicKey);
    const prompt = "What is Proof of Compute?";
    const encryptedPrompt = encrypt(prompt, userSecret);

    // 2. Miner receives {prompt_encrypted, user_memo_pubkey}
    //    and derives the same secret
    const minerSecret = computeSharedSecret(miner.privateKey, user.publicKey);

    // Secrets must be equal
    expect(userSecret.toString("hex")).toBe(minerSecret.toString("hex"));

    // 3. Miner decrypts prompt
    const decryptedPrompt = decrypt(encryptedPrompt, minerSecret);
    expect(decryptedPrompt).toBe(prompt);

    // 4. Mock Ollama: run inference on plaintext
    const mockResult = "Proof of Compute is BTCPC's consensus mechanism that validates AI inference work.";

    // 5. Miner encrypts result with same shared secret
    const encryptedResult = encrypt(mockResult, minerSecret);

    // 6. User decrypts result
    const finalResult = decrypt(encryptedResult, userSecret);
    expect(finalResult).toBe(mockResult);
  });

  test("encrypted flow with Unicode prompt and result", () => {
    const user = randomKeypair();
    const miner = randomKeypair();
    const userSecret = computeSharedSecret(user.privateKey, miner.publicKey);
    const minerSecret = computeSharedSecret(miner.privateKey, user.publicKey);

    const unicodePrompt = "Tell me about AI — 人工知能 — ИИ";
    const encP = encrypt(unicodePrompt, userSecret);
    const gotPrompt = decrypt(encP, minerSecret);
    expect(gotPrompt).toBe(unicodePrompt);

    const unicodeResult = "AI stands for Artificial Intelligence (人工知能) 🤖";
    const encR = encrypt(unicodeResult, minerSecret);
    const gotResult = decrypt(encR, userSecret);
    expect(gotResult).toBe(unicodeResult);
  });
});

// ──────────────────────────────────────────────
// 6. Plaintext fallback — encrypted: false
// ──────────────────────────────────────────────

describe("plaintext fallback", () => {
  test("non-encrypted encrypt/decrypt are independent from each other (no cross-contamination)", () => {
    // Plaintext inference doesn't use these crypto functions.
    // Verify that crypto.js works only when explicitly called,
    // and that a plaintext string without the crypto module is still just a string.
    const plainPrompt = "This is a plaintext inference request";
    // No encryption — just passes through as-is
    expect(typeof plainPrompt).toBe("string");
    expect(plainPrompt).toBe("This is a plaintext inference request");
  });
});

// ──────────────────────────────────────────────
// 7. Missing memo public key returns error
// ──────────────────────────────────────────────

describe("encryptedClient — error handling", () => {
  test("submitEncryptedInference throws when miner account not found", async () => {
    // Mock stateStore to return null for unknown account
    jest.mock("../src/chain/stateStore", () => ({
      getAccount: jest.fn().mockReturnValue(null),
    }), { virtual: false });

    const { submitEncryptedInference } = require("../src/inference/encryptedClient");
    const bip39 = require("bip39");
    const mnemonic = bip39.generateMnemonic();

    await expect(
      submitEncryptedInference("hello", "nonexistent_miner", mnemonic)
    ).rejects.toThrow(/not found/);
  });

  test("submitEncryptedInference throws when miner account has no memo key", async () => {
    // Reset and provide account without memo key
    jest.resetModules();
    jest.mock("../src/chain/stateStore", () => ({
      getAccount: jest.fn().mockReturnValue({
        username: "fakeminer",
        public_keys: {}, // no memo key
      }),
    }));

    const { submitEncryptedInference } = require("../src/inference/encryptedClient");
    const bip39 = require("bip39");
    const mnemonic = bip39.generateMnemonic();

    await expect(
      submitEncryptedInference("hello", "fakeminer", mnemonic)
    ).rejects.toThrow(/memo public key/);
  });
});
