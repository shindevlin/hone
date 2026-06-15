"use strict";

/**
 * HD-derivation parity guard.
 *
 * keyManager's BIP-32 backend was migrated from the pure-JS `hdkey` package
 * (ecurve/bigi) to native `bip32` + `tiny-secp256k1` for speed. These golden
 * vectors were captured from the ORIGINAL hdkey implementation. They must
 * never change: a different derived key for the same mnemonic would fork every
 * existing wallet. If this test ever fails, the HD derivation has drifted —
 * do NOT update the expected values to make it pass.
 */

const keyManager = require("../src/wallet/keyManager");

// mnemonic → { role: { privateKey, publicKey } }, captured from hdkey.
const GOLDEN = {
  "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about": {
    owner:   { privateKey: "0aac91ea7cd859b80e87e8f9214e4be880d863159b703ddd3f2ff2708c2ee5bc", publicKey: "03ae62ade894b15c2b7aa2c61ac1103ee2de672f93668ab05a2760060d7f59b397" },
    active:  { privateKey: "b1c3ea98d7f1b427b0333311ef984ebefa08c47534f7d04228dfdd3079203679", publicKey: "03a0a2a438e49aff935fed3039e3f66a44ff1415834fa6b5bcb836059112dd171f" },
    posting: { privateKey: "6eba4ef4a1b8c6e970c7c2a2eb06953869256508dabb8fd97d48e38093af44f5" },
    memo:    { privateKey: "052d6f7d9941d5fcd5ff3e4fb215fa4a1f1061a6a61bd331e73476e1d8a1cdc3" },
  },
  "legal winner thank year wave sausage worth useful legal winner thank yellow": {
    owner:   { privateKey: "097b8fcf3450ea255d228fa2907c4cd32f8646d7054a97e7f37f261905196f03" },
    active:  { privateKey: "eff7c3adc9274f7bb75996b3619ee00d5ee2e8cac8295752ff14b894db816b26" },
    posting: { privateKey: "0a74c5f9889a5ffab72b18e950d7cc43ce89a97f6298468c62e5a289c450be5e" },
    memo:    { privateKey: "a4f4becf893dc099fe5ee327dd407df752d8b53def627fe2252673e4f4bc1595" },
  },
  "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong": {
    owner:   { privateKey: "639e1329972aa5d3e9da976b85e5f8ccba4e4a07234187bc941844a8ef020b15" },
    active:  { privateKey: "fc9ec18b6971d10373ed7dacd0055678efbcbd2ec51e9907a0d5a713ed399514" },
    posting: { privateKey: "ea75fc9c9d46cfc53be48c8018499ff18c8a7e9279421d286faf74494c4540e5" },
    memo:    { privateKey: "d691206915f74710a6d19e41ef53969ba2a6d7e9c1039f64198c8a1c9f895980" },
  },
};

describe("keyManager HD derivation parity (hdkey → bip32 migration)", () => {
  for (const [mnemonic, roles] of Object.entries(GOLDEN)) {
    const label = mnemonic.split(" ").slice(0, 2).join(" ") + "…";
    test(`derives identical role keys for "${label}"`, async () => {
      const keys = await keyManager.mnemonicToKeys(mnemonic);
      for (const [role, expected] of Object.entries(roles)) {
        expect(keys[role].privateKey).toBe(expected.privateKey);
        if (expected.publicKey) {
          expect(keys[role].publicKey).toBe(expected.publicKey);
        }
      }
    });
  }

  test("derived keys are deterministic across calls", async () => {
    const m = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const a = await keyManager.mnemonicToKeys(m);
    const b = await keyManager.mnemonicToKeys(m);
    expect(a.owner.privateKey).toBe(b.owner.privateKey);
  });
});
