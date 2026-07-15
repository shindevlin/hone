# Flipper → Chain Signing Integration (design decision)

> **Status:** Design decision, 2026-07. Governs how a sensor reading captured on
> a Flipper Zero becomes a `SensorReading` ledger entry the HONE chain will
> accept. Written to resolve a concrete mismatch between the Flipper's existing
> BLE signing and the chain-side `SensorReading` authentication added in PR #7.
>
> No firmware capture code exists yet (only the protocol structs in
> `protocol/hone_protocol.h` and identity/BLE scenes). This doc is the contract
> those capture scenes and the phone relay must satisfy. It does not require
> hardware to reason about; it does require the chain-side facts below, which
> were verified against the actual code.

## The problem

Two different keys and two different signed messages are in play, and they don't
currently line up:

- **Flipper side (exists today):** the Flipper generates an ed25519 identity
  keypair on first run (hardware TRNG), and `hone_protocol.h` signs the **BLE
  frame payload bytes** — a packed `HoneSubGhzObs` / `HoneNfcScan` / etc.
  struct — with that **device key**. The signature covers the raw payload only,
  and the phone verifies it against the Flipper's public key before accepting.

- **Chain side (after PR #7):** `SensorReading` now requires a signature.
  `tx.rs` verifies it via `check_signature(chain, claimed_signer, entry,
  sig_hex, "posting")` — i.e. against the **owner account's posting key**, over
  the **canonical signing message** defined in `canonical_signing_message`
  (commit `9e393c65`): `{chain_id, type:"SENSOR_READING", sensor_id, owner,
  value, data_hash, signed_by}`, deliberately excluding the server-set `epoch`.

So the Flipper signs *packed sensor structs* with its *device key*; the chain
expects the *canonical SensorReading JSON message* signed by the *owner's
posting key*. These are incompatible as-is.

## Two candidate architectures

### Option A — Flipper device key signs the chain entry directly
The Flipper would build the canonical `SensorReading` message itself and sign it
with its device key; the chain would verify against the registered device key.

**Rejected. Verified blocker in the code:** the chain cannot verify against a
device key today. `SensorKeyRegister` (which carries `device_pubkey`) is a
**no-op in `chain.rs`** — it's recorded in the ledger but its `device_pubkey`
is NOT written to any queryable chain state ("Recorded in the ledger only; state
managed by protocol sidecars"). During `SensorReading` validation the node has
no way to look up the device key for a `sensor_id`. Making Option A work would
require: (1) `SensorKeyRegister` to actually persist `device_pubkey` keyed by
`sensor_id` in chain state, and (2) a new `check_signature` variant that accepts
a device-key role for `SensorReading`. That's real chain-consensus work, and it
also puts the Flipper's constrained TweetNaCl signing directly on the
consensus-critical path. Also: the Flipper can't know its `owner` account's
posting-key identity or the current `chain_id` reliably without the phone
anyway, so it can't build the full canonical message unaided.

### Option B — Flipper device key signs the BLE frame; phone re-signs the chain entry (CHOSEN)

Two-hop trust, each hop signed by the key that owns that hop:

1. **Flipper → phone (device key, existing mechanism):** the Flipper captures a
   reading, packs it into the existing `HoneSubGhzObs`/`HoneNfcScan`/etc.
   struct, and signs the BLE frame payload with its **device key**, exactly as
   `hone_protocol.h` already specifies. No firmware signing change needed.
2. **Phone verifies the Flipper.** The phone knows the Flipper's device public
   key (registered during pairing via `SensorKeyRegister`). It verifies the BLE
   frame signature. If it fails, the reading is dropped — this is the anti-spoof
   boundary for "did this really come from the paired Flipper."
3. **Phone → chain (owner posting key):** the phone translates the verified
   Flipper payload into a `SensorReading` entry (`sensor_id`, `owner` = the
   phone's account, `value`, `data_hash` = SHA-256 over the reading contents,
   `signed_by` = owner), and signs the **canonical SensorReading message** with
   the **owner's posting key** — which the chain already knows how to verify
   after PR #7. The phone (`hone-android`) is the component that actually holds
   the owner's posting key (per PLATFORM_PRD Phase 1.3 follow-up: it currently
   has no posting-key signing at all — that work is the prerequisite for this).

**Why B is right:**
- It requires **zero change to consensus code** — the chain side is already
  exactly what PR #7 built. Option A would reopen consensus.
- It keeps the Flipper off the consensus-critical path — a constrained
  microcontroller signing packed structs for its phone is a much smaller blast
  radius than a microcontroller signing consensus messages the whole network
  validates.
- The trust chain is honest and each hop is verifiable: Flipper-authenticity is
  proven to the phone by the device-key signature; reading-authenticity is
  proven to the chain by the owner's posting-key signature. A malicious phone
  can already forge readings for its own account (it holds the owner key) — B
  doesn't make that worse, and the device-key hop stops a *third party* from
  injecting fake Flipper data into someone else's relay.
- It matches the pairing model the Flipper README already describes ("The phone
  registers the Flipper's public key on-chain during the initial pairing flow").

## What each component must implement (the contract)

**Chain (`hone-node`):** DONE in PR #7 (`9e393c65`). No further change for
Option B. Note for later: if device-key-on-chain verification is ever wanted
(Option A as a future enhancement), `SensorKeyRegister` must first persist
`device_pubkey` in queryable state — file that separately, it is not needed for
B.

**Flipper firmware (capture scenes — not yet written):**
- Drive each radio/reader (sub-Ghz, NFC/ISO14443, RFID/125kHz, iButton, IR) to
  populate the *already-defined* payload structs in `hone_protocol.h`.
- Sign the BLE frame payload with the device key using the *existing* signing
  path — **no change to what or how the Flipper signs.**
- The capture scenes only produce data; they do not need to know about
  `chain_id`, `owner`, posting keys, or `canonical_signing_message`. That's all
  the phone's job. This keeps the firmware simple and off the consensus path.

**Phone (`hone-android`) — the integration work:**
1. Verify the Flipper BLE frame signature against the paired device public key;
   drop on failure.
2. Map the Flipper payload struct → `SensorReading` fields. Define a stable
   `data_hash` = SHA-256 over a canonical serialization of the specific payload
   struct (document the exact byte layout so it's reproducible — the packed
   struct bytes are a natural choice since they're already the signed BLE
   payload).
3. Build the canonical `SensorReading` signing message
   (`{chain_id, type:"SENSOR_READING", sensor_id, owner, value, data_hash,
   signed_by}`) and sign it with the **owner's posting key**. This depends on
   the phone gaining posting-key signing capability, which it does not have
   today (PLATFORM_PRD Phase 1.3 follow-up) — that is the true prerequisite for
   any of this to produce chain-accepted readings.
4. Submit the entry with its `sig_hex` via the normal API path.

## Sequencing note

Nothing here produces a chain-accepted reading until the phone has posting-key
signing (PLATFORM_PRD Phase 1.3 / this doc's phone step 3). The Flipper capture
scenes CAN be written independently of that — they only produce device-signed
BLE frames, which is useful and testable on hardware without any chain
interaction. But do not claim end-to-end "Flipper sensor data on chain" until
the phone re-signing path exists and PR #7 is merged with a working build.
