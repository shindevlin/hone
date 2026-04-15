"use strict";

const crypto = require("crypto");
const keyManager = require("../src/wallet/keyManager");
const chainLink = require("../src/services/chainLink");

function ed25519PrivateKeyFromSeed(seedHex) {
  const seed = Buffer.from(seedHex, "hex");
  const pkcs8Prefix = Buffer.from("302e020100300506032b657004220420", "hex");
  return crypto.createPrivateKey({
    key: Buffer.concat([pkcs8Prefix, seed]),
    format: "der",
    type: "pkcs8",
  });
}

describe("keyManager mnemonic derivation", () => {
  const mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

  it("derives BTCPC role keys and EVM, Solana, TON, and Bitcoin wallets from the same mnemonic", async () => {
    const roleKeys = await keyManager.mnemonicToKeys(mnemonic);
    const wallets = await keyManager.deriveChainWallets(mnemonic);

    expect(roleKeys).toEqual(expect.objectContaining({
      owner: expect.any(Object),
      active: expect.any(Object),
      posting: expect.any(Object),
      memo: expect.any(Object),
    }));
    expect(wallets).toEqual(expect.objectContaining({
      evm: expect.any(Object),
      solana: expect.any(Object),
      ton: expect.any(Object),
      bitcoin: expect.any(Object),
    }));
    expect(wallets.ton.address).toMatch(/^0:[0-9a-f]{64}$/);
    expect(wallets.ton.linkAddress).toBe("ton:" + wallets.ton.publicKey);
    expect(wallets.ton.publicKey).toHaveLength(64);
  });

  it("keeps a TON link target as an experimental raw public key identifier", async () => {
    const wallets = await keyManager.deriveChainWallets(mnemonic);
    expect(wallets.ton.linkAddress).toBe("ton:" + wallets.ton.publicKey);
  });

  it("produces a TON link target that chainLink can verify", async () => {
    const message = "BTCPC-LINK:alice:ton:ton:placeholder:test";
    const wallets = await keyManager.deriveChainWallets(mnemonic);
    const keyObject = ed25519PrivateKeyFromSeed(wallets.ton.privateKey);
    const signature = crypto.sign(null, Buffer.from(message), keyObject).toString("base64");

    expect(chainLink.recoverTONAddress(message, signature, wallets.ton.linkAddress)).toBe(wallets.ton.linkAddress);
  });
});
