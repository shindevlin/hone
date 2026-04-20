"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");

const ISOLATED_DIR = fs.mkdtempSync(path.join(os.tmpdir(), "btcpc-account-creation-"));
process.env.BTCPC_SECRETS_PATH = path.join(ISOLATED_DIR, "secrets.json");

const mockUsers = {};
jest.mock("../src/models/User", () => ({
  findOne: jest.fn(async (query) => mockUsers[query.username] || null),
  create: jest.fn(async (fields) => {
    mockUsers[fields.username] = { ...fields, _id: "mongo-" + fields.username };
    return mockUsers[fields.username];
  }),
}));

jest.mock("../src/services/ledger", () => ({
  recordAccountCreate: jest.fn(async () => ({})),
  recordDelegate: jest.fn(async () => ({})),
}));

jest.mock("../src/chain/stateStore", () => ({
  getAccount: jest.fn(() => null),
  getChainHeight: jest.fn(() => 0),
  getBalance: jest.fn((account) => account === "btcpc_faucet" ? 100 : 0),
}));

const ledger = require("../src/services/ledger");
const secretStore = require("../src/services/secretStore");
const { createAccountForUser } = require("../src/services/accountCreation");

describe("accountCreation canonical wallet export", () => {
  beforeEach(async () => {
    secretStore.resetForTests();
    await secretStore.load();
    for (const key of Object.keys(mockUsers)) delete mockUsers[key];
    jest.clearAllMocks();
  });

  afterAll(() => {
    try { fs.rmSync(ISOLATED_DIR, { recursive: true, force: true }); } catch (_) {}
  });

  it("records account creation with public keys and chain addresses in the correct ledger argument shape", async () => {
    const result = await createAccountForUser({
      username: "alice",
      password: "password123",
      claimFaucet: true,
    });

    expect(ledger.recordAccountCreate).toHaveBeenCalledTimes(1);
    const [username, publicKeys, chainAddresses, epoch] = ledger.recordAccountCreate.mock.calls[0];
    expect(username).toBe("alice");
    expect(publicKeys.owner).toBeTruthy();
    expect(publicKeys.public_keys).toBeUndefined();
    expect(chainAddresses.btcpc).toMatch(/^BTCPC[0-9a-f]{40}$/);
    expect(chainAddresses.hive).toBe("alice");
    expect(epoch).toBe(0);

    expect(result.wallet_export).toContain("Mnemonic:");
    expect(result.wallet_export).toContain("BTCPC active key");
    expect(result.wallet_export).toContain("Private key:");
  });

  it("stores public keys in secretStore and issues starter value as delegated faucet credit", async () => {
    const result = await createAccountForUser({
      username: "bob",
      password: "password123",
      claimFaucet: true,
    });

    const stored = secretStore.getUser("bob");
    expect(stored.username).toBe("bob");
    expect(stored.owner_public_key).toBe(result.public_keys.owner);
    expect(stored.active_public_key).toBe(result.public_keys.active);
    expect(stored.posting_public_key).toBe(result.public_keys.posting);
    expect(stored.memo_public_key).toBe(result.public_keys.memo);

    expect(ledger.recordDelegate).toHaveBeenCalledWith("btcpc_faucet", "bob", 1, "faucet", 0);
    expect(result.wallet_balance).toBe(0);
    expect(result.delegated_balance).toBe(1);
  });
});
