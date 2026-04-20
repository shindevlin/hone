"use strict";

const crypto = require("crypto");
const secp256k1 = require("secp256k1");

jest.mock("../src/services/userResolver", () => ({
  resolveUserFromAuth: jest.fn(async () => ({ username: "alice", id: "user-1" })),
}));

jest.mock("../src/services/ledger", () => ({
  getCurrentEpoch: jest.fn(async () => 1),
  recordStake: jest.fn(async () => ({})),
}));

let mockActivePublicKey = null;
jest.mock("../src/chain/stateStore", () => ({
  getAccount: jest.fn(() => ({ public_keys: { active: mockActivePublicKey } })),
  getBalance: jest.fn(() => 5000),
  getStakePool: jest.fn(() => ({ total_staked: 1000 })),
}));

const ledger = require("../src/services/ledger");
const { stake, activeKeyChallenge } = require("../src/controllers/stakingController");

function res() {
  return {
    statusCode: 200,
    body: null,
    status(code) { this.statusCode = code; return this; },
    json(obj) { this.body = obj; return this; },
  };
}

function makeKeypair() {
  let priv;
  do { priv = crypto.randomBytes(32); } while (!secp256k1.privateKeyVerify(priv));
  const pub = Buffer.from(secp256k1.publicKeyCreate(priv, true)).toString("hex");
  return { priv, pub };
}

function signChallenge(challenge, priv) {
  const msg = crypto.createHash("sha256").update(challenge).digest();
  return Buffer.from(secp256k1.ecdsaSign(msg, priv).signature).toString("hex");
}

describe("stakingController active key authorization", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockActivePublicKey = null;
  });

  it("rejects stake without active key signature", async () => {
    const kp = makeKeypair();
    mockActivePublicKey = kp.pub;
    const r = res();

    await stake({ user: { username: "alice" }, body: { amount: 1000 } }, r);

    expect(r.statusCode).toBe(403);
    expect(r.body.error).toMatch(/active key signature required/i);
    expect(ledger.recordStake).not.toHaveBeenCalled();
  });

  it("accepts stake with signature from registered active key", async () => {
    const kp = makeKeypair();
    mockActivePublicKey = kp.pub;
    const challenge = activeKeyChallenge("stake", "alice", 1000);
    const signature = signChallenge(challenge, kp.priv);
    const r = res();

    await stake({
      user: { username: "alice" },
      body: { amount: 1000, active_challenge: challenge, active_signature: signature },
    }, r);

    expect(r.statusCode).toBe(200);
    expect(ledger.recordStake).toHaveBeenCalledWith("alice", 1000, "mining", 1);
  });
});
