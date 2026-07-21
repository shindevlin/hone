"use strict";

const crypto = require("crypto");
const secp256k1 = require("secp256k1");

jest.mock("../src/chain/stateStore", () => ({
  getAccount: jest.fn(),
}));

const stateStore = require("../src/chain/stateStore");
const activeKeyAuthorization = require("../src/services/activeKeyAuthorization");

function randomPrivateKey() {
  let priv;
  do {
    priv = crypto.randomBytes(32);
  } while (!secp256k1.privateKeyVerify(priv));
  return priv;
}

test("builds and verifies an active-key transfer challenge", async () => {
  const priv = randomPrivateKey();
  const pubHex = Buffer.from(secp256k1.publicKeyCreate(priv, true)).toString(
    "hex",
  );
  stateStore.getAccount.mockReturnValue({
    public_keys: { active: pubHex },
  });

  const challenge = activeKeyAuthorization.buildTransferChallenge("alice", {
    toAddress: "bob",
    amount: 12.5,
    token: "HONE",
    memo: "thanks",
  });

  const msg = crypto.createHash("sha256").update(challenge.challenge).digest();
  const sig = secp256k1.ecdsaSign(msg, priv);
  const sigHex = Buffer.from(sig.signature).toString("hex");

  const result = await activeKeyAuthorization.verifyActiveTransferSignature(
    "alice",
    { to: "bob", amount: 12.5, token: "HONE", memo: "thanks" },
    {
      active_challenge_id: challenge.challengeId,
      active_challenge: challenge.challenge,
      active_signature: sigHex,
    },
  );

  expect(result.ok).toBe(true);
  expect(result.signer).toBe("active");
  expect(result.challengeId).toBe(challenge.challengeId);
});

test("rejects a reused active-key challenge", async () => {
  const priv = randomPrivateKey();
  const pubHex = Buffer.from(secp256k1.publicKeyCreate(priv, true)).toString(
    "hex",
  );
  stateStore.getAccount.mockReturnValue({
    public_keys: { active: pubHex },
  });

  const challenge = activeKeyAuthorization.buildTransferChallenge("alice", {
    toAddress: "bob",
    amount: 1,
    token: "HONE",
    memo: "",
  });
  const msg = crypto.createHash("sha256").update(challenge.challenge).digest();
  const sig = secp256k1.ecdsaSign(msg, priv);
  const sigHex = Buffer.from(sig.signature).toString("hex");

  const first = await activeKeyAuthorization.verifyActiveTransferSignature(
    "alice",
    { to: "bob", amount: 1, token: "HONE", memo: "" },
    {
      active_challenge_id: challenge.challengeId,
      active_challenge: challenge.challenge,
      active_signature: sigHex,
    },
  );
  expect(first.ok).toBe(true);

  const second = await activeKeyAuthorization.verifyActiveTransferSignature(
    "alice",
    { to: "bob", amount: 1, token: "HONE", memo: "" },
    {
      active_challenge_id: challenge.challengeId,
      active_challenge: challenge.challenge,
      active_signature: sigHex,
    },
  );
  expect(second.ok).toBe(false);
});
