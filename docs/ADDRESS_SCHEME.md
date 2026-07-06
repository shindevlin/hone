# HONE Address Scheme

**Version 0.1 — July 2026**

---

## Overview

HONE identities are **human-readable names first** (`bullship`, `hone-market`, written
`@bullship`). Names are the primary, ENS/Hive-style identity layer — a person is a name,
not a hex blob.

Alongside names, HONE defines **typed machine addresses**: a bech32-encoded, checksummed
address whose prefix tells you *what kind of on-chain entity* it refers to (account,
contract, token, device, …). This is the layer for tooling, interop, QR codes, and
anywhere a raw key/entity reference is needed instead of a claimed name.

Two design commitments shape everything below:

1. **Typed + enforced, not decorative.** The prefix is part of a checksummed encoding, so
   a mistyped address, or one of the wrong entity type, is *rejected* — you cannot send a
   token to a contract address by fat-fingering it.
2. **Hardware-wallet compatible from day one of the design.** HONE keys are
   **ed25519 / SLIP-10** (`m/44'/6942'/role'/0'`) — the same curve + derivation Ledger,
   Trezor, Keystone, etc. support natively (as they do for Cosmos, Solana, Stellar). The
   address format is deliberately the Cosmos-style **bech32-over-ed25519-pubkey** pattern
   that hardware wallets already sign for, so a future HONE Ledger app is a build, not a
   redesign.

Not a hex `0x…` scheme: that fights HONE's name-based identity and echoes Ethereum. HONE
addresses are their own thing.

---

## Encoding

An address is standard **bech32** (BIP-173): `<hrp>` + `1` (separator) + `<data>` +
`<6-char checksum>`. HONE already depends on `bech32` (used for its Cosmos-interop
addresses), so this reuses a primitive in the tree.

- `<hrp>` — the human-readable prefix, encoding the entity **type** (table below).
- `<data>` — the entity's 32-byte ed25519 public key (for keyed entities: accounts,
  vaults, escrows) or the entity's content/derivation hash (for derived entities:
  contracts, tokens), base32-encoded.
- `<checksum>` — bech32's built-in BCH checksum. Detects typos and, because the HRP is
  part of the checksum computation, **rejects an address parsed under the wrong type**.

Example (account, all-zero data for illustration):
```
hh 1 qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq mfpt2y
│    └──────── ed25519 pubkey (base32) ────────┘ └checksum┘
└ hrp "hh" = account
```

---

## Type Prefixes (HRPs)

Every HONE address begins with `h`, so it is unmistakably HONE, and is a uniform two
letters: `h` + the entity-type letter.

| Entity | HRP | Example | Keyed by |
|---|---|---|---|
| Account / user | `hh` | `hh1…` | ed25519 account pubkey |
| Contract | `hk` | `hk1…` | contract code/deploy hash |
| Token | `ht` | `ht1…` | token mint/derivation hash |
| Device / sensor | `hd` | `hd1…` | device ed25519 pubkey (hardware identity) |
| Vault | `hv` | `hv1…` | vault ed25519 pubkey |
| Escrow | `he` | `he1…` | escrow account pubkey |

All six are validated as legal bech32 HRPs (they encode to `hh1…`, `hk1…`, `ht1…`, `hd1…`,
`hv1…`, `he1…`). The set is **extensible**: new entity types get a new `h<letter>` HRP
(e.g. a future repo/product/worker address) without changing the encoding.
Reserved-but-unassigned letters should be documented here before use so two entity types
never collide on a prefix. (`hh` deliberately avoids `hx`, which would faintly echo the
`0x` hex convention HONE is not using.)

**Names vs. addresses.** A claimed account name (`@bullship`) and its typed address
(`hh1…`) refer to the same account — the name is the friendly handle, the `hh1…` address is
the key-derived machine form. Resolvers accept either. System/keyless accounts
(`__treasury__`, `__recycle_fund__`) have names but no keyed address.

---

## Validation Rules

A HONE address is valid iff:

1. It is well-formed bech32 (BIP-173): valid charset, correct separator, **checksum
   verifies**.
2. Its HRP is a **known, assigned** HONE type prefix (table above).
3. The decoded data length matches what the type expects (32 bytes for keyed entities).
4. The context accepts that type — e.g. a "send tokens to" field accepts `hh1…` (account)
   and rejects `hk1…` (contract) unless contract-recipients are explicitly allowed.

Rules 1–2 make wrong-type / mistyped addresses **hard-fail at parse time**, before any
value moves. This is the safety the typed scheme buys over a bare hex string.

---

## Hardware Wallet Compatibility (Ledger et al.)

A first-class requirement, satisfied by construction:

- **Curve:** ed25519 — natively supported by Ledger's crypto API and by Trezor/Keystone.
  HONE did not pick an exotic curve, so on-device signing is standard.
- **Derivation:** SLIP-10 hardened path `m/44'/6942'/role'/0'` (6942 = HONE's coin index;
  role = owner/active/posting/memo/hide/seek). A HONE Ledger app derives this on-device;
  the private key never leaves the device.
- **Address display:** the bech32 `hh1…` form is short and human-verifiable on a hardware
  screen — the operator confirms the `hh1…` recipient on the device before signing, exactly
  as Cosmos/Solana Ledger apps show their bech32/base58 addresses.
- **Signing:** a HONE ledger entry's canonical signing bytes are signed by the device's
  ed25519 key; the node verifies against the account's registered pubkey as it does today.

What remains a *separate future build* (not blocked by this design): the actual **HONE
Ledger app** (the small on-device application submitted to Ledger that formats + displays
HONE transactions for signing). Nothing in this address/key scheme obstructs it; that is
the point of specifying it now.

---

## Relationship to Names & Genesis

- **Genesis is unaffected.** `genesis.json` holds account *names* + public keys. Typed
  addresses are a derived/display layer computed from those keys — they are not stored in
  genesis and do not change the block-0 hash. This spec can land before or after
  re-genesis without touching it.
- **Names stay primary.** Humans use `@bullship`; addresses are for machines, interop, and
  hardware confirmation. Both resolve to the same account.

---

## Build Notes (for implementation, after sign-off)

- Reuse the `bech32 = "0.9"` dep already in `hone-node`.
- Add an `address` module (candidate: `hone-types`) with `encode(kind, data) -> String`,
  `decode(&str) -> Result<(Kind, Vec<u8>)>`, and a `Kind` enum mirroring the HRP table.
- Wire `decode` validation into entry-parsing where addresses appear, so wrong-type
  addresses fail at the boundary.
- Keep the HRP table in ONE place (the `Kind` enum) so names/prefixes can't drift.

---

_Names for humans, typed checksummed addresses for machines, ed25519 for the hardware in
your pocket._
