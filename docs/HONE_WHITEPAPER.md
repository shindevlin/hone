# HONE

### A Proof-of-Compute Blockchain for Inference, Sensors, and Sovereign Identity

**Shin Devlin**
**Version 2.0, November 2026**

This document describes HONE as it runs on mainnet on 2026-11-04 (around epoch 353,900).
Every mechanism is labelled **live**, **gated** (built and in the node, waiting for its
activation epoch) or **planned**. Section 12 lists them in one table. Where an earlier
version of this paper and the code disagreed, the code won; the differences are recorded in
`WHITEPAPER_IMPLEMENTATION_NOTES.md`.

---

## Abstract

HONE is a blockchain where every token in existence was earned by a machine doing useful
work. Mining on HONE does not mean burning electricity on a hash that disappears the moment
it is found. It means keeping the clock alive that the network depends on, storing data
that someone committed, answering an AI inference job that a user submitted, or reporting
sensor information that the network did not already have. The work had a customer.

Total supply is fixed at **42,000,000 HONE** with 10 decimal places
(**1 HONE = 10,000,000,000 hunits**). Epochs run every **30 seconds** and are sealed by a
quorum of permissionless clock nodes. There is no pre-mine and no founder allocation.

The chain is built for ordinary hardware. A phone is a full participant: it is a clock seat,
a sensor, a wallet and, in shadow mode today, a juror. A Raspberry Pi, a PC and a GPU server
run the same node binary with different profiles. A Flipper Zero is a signed sensor that
reaches the chain through a phone that hears it.

Three ideas carry the 2026 design:

1. **Identity is owned, layered and private.** An account is a name held as an NFT,
   controlled by role keys (owner, active, posting, memo, hide, seek). Devices get their own
   seats under the owner. Devices broadcast rotating IDs that only their owner can link.
2. **Sensors are paid for information, not volume.** A report earns only when it is new or
   shows a change, the first reporter is paid, confirmations are paid near zero, and a claim
   pays out only once it is proven. Repeats earn nothing. Data older than seven days earns
   nothing.
3. **Disputes go to a jury, not to an operator.** Small local models on staked devices cast
   sealed votes on every judgment entry, and a seed-drawn jury reveals them when a result is
   challenged.

---

## 1. The Problem

**Compute is rented.** AI inference is concentrated in a handful of providers who set price,
policy and rate limits. Every prompt passes through someone else's servers.

**Work is wasted.** Bitcoin's security model works, but its computation produces nothing
beyond the hash. Networks that tried to pay for "useful" work usually paid for volume, and
volume is easy to fake.

**Sensor networks pay for noise.** Earlier decentralized sensor and coverage networks paid
per reading or per beacon. Operators learned to send more readings, from the same place,
saying the same thing. The data buyer gets redundancy. The honest operator gets diluted.
HONE's own first sensor reward (`sensor_score(reading_count)`) had the same flaw, and it is
being replaced (Section 6.4).

**Tracking is the default.** Bluetooth and Wi-Fi devices broadcast stable identifiers. A
network that pays people to log those identifiers builds a surveillance system by accident.
HONE has to count the world's devices without being able to follow any of them.

**Keys get lost.** The first HONE genesis ("Mayday", 1 May 2026) let wallets be created
without durable key storage. Seed phrases were shown once and lost, leaving accounts that
were owned but unsignable. A chain that claims sovereignty has to make recovery a
precondition, not an afterthought.

**Mobile nodes are behind NAT.** Phones roam between networks and sit behind carrier NAT. A
mesh that only works on one LAN stalls as soon as its phones leave the house. This happened
on 2026-11-04 and is the reason HONE now runs a public node and is adding NAT traversal
(Section 8).

---

## 2. Design Overview

### 2.1 The chain in one page

| Property | Value | Status |
|---|---|---|
| Chain ID | `hone` (testnet `hone-testnet`) | live |
| Genesis | 4 July 2026, noon Los Angeles (`1783191600000` ms, 19:00 UTC) | live |
| Epoch | 30 s (era 0) | live |
| Supply cap | 42,000,000 HONE, block reward 2 HONE per epoch | live |
| Consensus | quorum seal by clock nodes, at least 51% of registered clocks | live |
| Storage engine | RocksDB, one shared block cache | live |
| Transport | libp2p: TCP + Noise, QUIC, WebSocket; gossipsub, Kademlia, mDNS | live |
| Keys | ed25519, SLIP-10 `m/44'/6942'/role'/0'` | live |
| Licence | Usufruct License (UFL), see `LICENSE` | n/a |

### 2.2 Node types

One binary, `hone-node`, runs everywhere. What differs is the profile and the roles the
operator turns on.

- **Full node (PC, server):** clock, storage, sensor relay, API, optional inference
  front door. Default profile.
- **Light node (Pi-class, under 2 GiB RAM):** the light profile is selected automatically
  below 2 GiB, or with `HONE_PROFILE=light`. It shrinks the RocksDB cache and memtables,
  slows catch-up vote traffic, and defaults storage, worker and embedded inference off. It
  never changes what consensus applies. Pi-based hotspot hardware such as Nebra units run
  this profile as full clocks.
- **Public relay node:** a non-clock node on a public IP that holds no posting key and only
  relays gossip and serves the public wallet API (Section 8.3).
- **Phone:** the Android app runs an embedded node as a **device clock seat**, a full sensor,
  a wallet and a jury seat (Section 5.2).
- **Flipper Zero:** a signed sensor with no network of its own. Its frames reach the chain
  through a phone (Section 5.3).

### 2.3 Hardware tiers

Inference rewards are weighted by a hardware tier from 0 to 4 with weights
**1, 4, 8, 16, 32**. Today the tier is declared by the operator (`HONE_HW_TIER`) and is not
attested; attested tiers are planned. The capability provisioner (Section 2.5) gates each role
against detected RAM, VRAM and disk before anything heavy is downloaded.

### 2.4 Inference runs beside the node, not inside it

The node is the front door for inference: API, routing, pricing, fees, verification,
payments and rewards all stay in the node and in consensus. **The node never loads a model
in its own process.** Models run in a separate sidecar process, `hone-node inference-serve`,
with its own systemd unit and memory budget, reached over loopback (`INFERENCE_URL`, default
`127.0.0.1:4250`). The sidecar speaks the Ollama-compatible API (`/api/chat`, `/api/tags`,
`/v1/models`, `/health`), so an operator can point the node at Ollama, vLLM or the bundled
candle GGUF sidecar instead. The sidecar holds no keys, opens no chain store and joins no
network. If it crashes or runs out of memory, inference calls fail and then recover; the node
keeps sealing. A light node never runs a model at all.

This replaces the earlier design, in which inference ran in-process so that a verifier could
re-run the job byte for byte. Keeping the clock alive turned out to matter more than
in-process determinism. Verification now rests on verifier assessment and, for disputes, the
decide jury (Section 7).

HONE is **model-agnostic**. There is no default model and nothing is downloaded
automatically. The operator chooses what the machine serves.

### 2.5 The capability provisioner

A machine does not silently become an inference, storage, sensor or render node. The node
discovers what it could do, gates each capability against its hardware, and the operator
enables roles explicitly. Suggesting is never enabling. An on-chain model registry
(`ModelManifestPublish`, keyed by the content hash of the weights, with a usage-weighted
analysis reward) is **planned**; it is specified in `MODEL_REGISTRY_PROTOCOL.md` but has no
ledger entry yet.

### 2.6 Native protocols

Four protocols are designed into HONE. Three are native ledger entry types processed by every
node today. Each is a separable business owned initially by the protocol founder
(shindevlin) and transferable by on-chain key rotation plus transfer of its GitHub
repository.

- **Freeport (live):** a peer-to-peer marketplace with chain-enforced escrow. `StoreUpdate`,
  `ProductCreate`, `OrderPlace` (locks escrow), `OrderFulfill` (digital goods encrypted from
  the seller's `seek` key to the buyer's `hide` key), `OrderDispute`, `EscrowRelease`.
  Default settlement fee 0.5%.
- **Verasens (live):** the sensor network. `SensorRegister`, `SensorKeyRegister`,
  `SensorReading`, `SensorDataCommit` (a batch hash signed by the device key),
  `GatewayHeartbeat`, `CoverageReport`, and from the usefulness activation epoch
  `SensorSummary` (Section 6.4).
- **LinkGit (live):** Git-compatible version control. Repositories, refs and access grants
  are ledger entries; objects live on HONE storage nodes. Private repositories distribute
  their key to the grantee's `hide` key. A mirror protocol pushes to GitHub at the same
  time.
- **Wiiv (planned):** a rendering marketplace for image, video, audio and 3D work with
  milestone escrow and a reviewer market. Subjective quality is never a consensus input. The
  job and state-machine types exist in the `wiiv` crate; the ledger entries and chain routes
  are not wired yet. Its reserved accounts (`wiiv`, `wiiv-escrow`) exist from genesis.

Detailed entry tables live in `NATIVE_PROTOCOLS.md`, `FREEPORT_PROTOCOL_WHITEPAPER.md`,
`VERASENS_PROTOCOL_WHITEPAPER.md`, `LINKGIT_PROTOCOL_WHITEPAPER.md` and `WIIV_PROTOCOL.md`.

---

## 3. Identity

### 3.1 Names are the primary identity

A HONE account is first a human-readable name (`@bullship`), in the ENS and Hive tradition.
A name costs a **10 HONE registration stake**, locked while the account exists, which keeps
squatting expensive. System accounts (`__treasury__`, `__recycle_fund__`) have names but no
key-derived address.

Typed, checksummed bech32 addresses for machines and tooling (`hh1…` accounts, `hk1…`
contracts, `hd1…` devices and so on, specified in `ADDRESS_SCHEME.md`) are **planned**. They
are not yet implemented in the node; today entries name accounts, and devices by their
`owner@pubkey` seat.

### 3.2 Names are NFTs (live)

Since epoch **333,333** names are transferable assets (`IdentityNft*` entries). The model is
deliberately simple: a name is a domain, and the chain never arbitrates identity. Each name
carries a protection level chosen by its holder:

| Level | Transferable | Recovery |
|---|---|---|
| Open | yes, by the active key | none |
| Guarded | yes, by the active key | guardians may recover to a destination pinned in advance |
| Locked | no | only guardian recovery to the pinned destination |

The active key may only **tighten** protection (add recovery, lock, drop the founder
backstop). Loosening needs the recovery authority. Recovery can only ever move a name to the
destination pinned beforehand, so a guardian cannot redirect a name to itself, and a sequence
number stops replay. A permanent lock can be lifted only by a 2-of-3 founder multisig. Open
minting ends after the first 1,000 registrations; after that new names go to auction.

**Reserved-name migration (live, epoch 352,000).** At the first seal at or after epoch
352,000, every reserved-name account still controlled only by the genesis issuer key became a
`NameNft` held by the issuer and bound to it. **1,162 names** migrated, with identical counts
and state roots on every node. From then on such an account is controlled by whoever holds
the NFT, and its value roles (owner, active) never fall back to a posting key.

### 3.3 Role keys

From one BIP-39 mnemonic a wallet derives six ed25519 role keys on the SLIP-10 hardened path
`m/44'/6942'/role'/0'` (6942 is HONE's coin index). This is the curve and derivation that
Ledger, Trezor and Keystone already sign for other ed25519 chains, so a hardware-wallet app is
a build, not a redesign.

| Key | Signs or decrypts | Where it should live |
|---|---|---|
| `owner` | key rotation, recovery | offline |
| `active` | transfers, stakes, escrow, memory writes | wallet with biometric unlock |
| `posting` | ordinary entries: listings, repo ops, sensor registration | hot |
| `memo` | encrypted memos, selective disclosure (location sharing) | wallet |
| `hide` | receives private content; root of device ID seeds; signs Find-My grants | wallet host only |
| `seek` | delivers encrypted content (store fulfilment, repo push) | delivery daemon |

Since epoch 320,200 a missing role key falls back only to the posting key, and a signer with
no usable key is rejected. Nodes hold their own **clock key**, not the owner's posting key:
since epoch 349,100 node-originated entries (heartbeats, sensor registration, inference job
bids and completions) may be signed by the registered clock key.

The same seed also derives EVM, Bitcoin (native SegWit), Solana and TON wallets client-side.
HONE never takes custody of external assets.

### 3.4 Recoverable keystores

Every wallet writes an encrypted keystore (`<account>.keystore.json`, Argon2id +
AES-256-GCM) and cannot be created without one. Recovery has three layers: the keystore file
and password, the recovery phrase (shown once behind a write-it-down confirmation), and an
optional ciphertext backup on a relay that cannot decrypt it.

### 3.5 Devices are seats under an owner

One account owns many devices, and each device is staked separately.

- **Clock seats (live since 321,000).** A device seat is the node ID `owner@pubkey`, where
  `pubkey` is the device's ed25519 key in hex. The ID binds the key, so only that device can
  sign for the seat, and only the owner may stake on it. Clock rewards earned by the seat are
  credited to the owner.
- **Decide seats (live since 352,440).** A device seat may also hold a `decide` role stake,
  staked by the owner's active key, and sign its own jury entries (Section 7).

### 3.6 Find-My grants (gated)

A phone should be able to find its owner's devices without holding the owner's hide key. The
owner publishes a standing **`FIND_GRANT`** naming the phone's hardware key and up to 64
devices, signed by the owner's **current hide key** with no fallback to any other role.
`FIND_REVOKE` withdraws it, and `SEED_GENERATION` rolls a device to a new ID seed generation.
A grant is valid only while the hide key that signed it is still the account's hide key, so
rotating `hide` voids every older grant with no state rewrite.

The phone side checks Android Key Attestation (Google root, TEE or StrongBox, the app's
signing certificate) before the grant is issued. The per-device seed key is then imported
into the phone's Keystore as a non-exportable key. The hide and owner keys never reach the
phone. The entries are built and tested, with a cross-implementation fixture shared with the
Android granting tool. **`FIND_GRANT_EPOCH` is not set yet**, so the entries are rejected
until an activation epoch is approved. Until then a phone can still import the hide key
directly, by the owner's choice.

---

## 4. Consensus

### 4.1 Clocks and quorum seals

Epochs are 30 seconds. Clock nodes propose the seal of each epoch, and an epoch is sealed
when at least **51% of registered clocks** agree (rounded up, and never fewer than two for
rewards). A node that loses its peers still seals locally so its own chain advances, but it
earns nothing until a real quorum forms again. There is no solo-reward bypass, not even
during bootstrap: delayed but recoverable rewards are better than divergent payouts.

During the first 250,000 epochs clocks could register at zero stake and build their minimum
stake from clock rewards (the bootstrap grace). The minimum clock stake defaults to 5 HONE.

### 4.2 Seal lag (live since 351,880)

The quorum seal of epoch E commits only pending entries dated **E − 2 or earlier**. Before
this rule, an entry gossiped in the last seconds of an epoch could land at E on its origin and
at E+1 elsewhere. Balances agreed again a seal later, but every checkpoint compared states
that differed by those in-flight entries.

### 4.3 Reward finality

An epoch's rewards, recycling and decay are applied only once the epoch is **20 epochs**
(about 10 minutes) behind the tip. That is long enough that a heavier competing seal can no
longer rewrite fork choice for it.

### 4.4 Surviving restarts and partitions (live)

Three fixes from October and November 2026 make a node's state independent of when it was
restarted:

- **Persisted pending pool.** The pending pool and the outbox survive a restart, so a node's
  own heartbeat in flight at shutdown is no longer lost.
- **Backfill of entries and finalize meta.** A restarted or partitioned node asks peers for
  the quorum seals it missed and replays them, including each epoch's `EpochFinalize` meta.
  Without that meta, the reward driver stalled at the first gap. A stalled reward driver
  now triggers a finalize backfill, and migrations such as the 352,000 name migration run
  inside the backfill at their own epoch.
- **One dial per peer and a gossip watchdog.** Gossipsub sends topic subscriptions once, on
  a peer's first connection. With several parallel connections per peer, closing the one
  that carried the subscriptions left the peer invisible to gossip. The node now dials each
  peer once, and a watchdog disconnects and redials a connected peer that shows no topics
  for two 20-second checks. Separately, an operator guard restarts a node whose clock has not
  sealed for 15 minutes, at most once an hour.

### 4.5 Checkpoints and the self-healer (live; deterministic cursor planned)

Every **120 epochs** (one hour), right after applying a quorum seal, each clock records a
checkpoint: the reward driver's cursor R, the balance Merkle root, and a snapshot of the
sorted balance rows. It signs `hone-checkpoint:<chain>:<node>:<E>:<R>:<root>` with its clock
key and gossips it.

A root is **canonical** for (E, R) if at least two distinct operators attest it and it has
strictly more operators than any other root. Device seats never count, and one operator
running many nodes counts once. Weights are deliberately absent, so one large staker cannot
decide alone. A node whose own root differs from a canonical root halts, fetches the
snapshot, verifies its root, backs up its balances, rebases and resumes (`heal` mode). In
`report` mode, the default, it only reports. With no canonical root it holds and never guesses.

Checkpoints are only comparable at the same cursor. A change that makes every node checkpoint
at the same reward cursor (E − 21) is merged but not yet deployed; until it is, some
checkpoints end in "hold" rather than "agree".

### 4.6 Genesis

Genesis was 4 July 2026, America's 250th anniversary. It seeded the founding protocol and
operator accounts with preserved keys and **no balances**. Every founding key was created
under the recoverable-keystore flow. Every node must use the identical genesis timestamp; a
mismatched node forks at once and finds no peers.

---

## 5. Sensors and Data

### 5.1 Rotating seeded device IDs

**HONE devices** never broadcast a stable identifier. Each device has an ID seed derived from
the owner's hide key:

```
id_seed  = HKDF-SHA256(hide_private, "hone-id-seed" || device_pubkey [|| be32(generation)])
window   = floor(unix_utc / 900)                   15-minute UTC windows
period   = UTC calendar month of the window
share_k  = HKDF(id_seed, "hone-share" || period)
sk_w     = HKDF(share_k, "hone-pkw" || window)     X25519 secret for this window
pk_w     = X25519(sk_w, 9)                          broadcast in the advert
lid      = SHA-256(pk_w)[0..16]                     lookup id
```

- **Synced rotation.** Every device rotates at the same instant, on the UTC quarter-hour.
  Phones stop their advertising sets and start new ones, so the BLE address rotates with the
  key. A Flipper derives its two advert addresses from the same window key.
- **Advert v2.** Manufacturer data `'h' 'K' 02 win8 frag tag pk_w`: one extended advert, or
  two legacy halves joined by `(win8, tag)` rather than by address.
- **No stored seed.** The wallet mnemonic is the backup. Any device holding the hide key (or a
  Find-My grant) recomputes every owned device's keys.

**Other people's devices** are counted with keyed hashes: `oid = HMAC-SHA256(K_day, mac)[0..8]`,
where `K_day` is 32 random bytes per observer per UTC day, kept in RAM and never stored or
sent. An oid supports counts, dwell time and returns within one day on one observer. Nothing
links across days or across observers.

### 5.2 Owner-encrypted sightings and location sharing (live in the app)

When an observer hears a complete `pk_w`, it seals one record for the owner and stores it
under the lid:

```
record = {window, time, rssi, cell, accuracy, source}
blob   = epk || nonce || AES-256-GCM(HKDF(X25519(esk, pk_w)), record, aad = lid)
```

The observer keeps no plaintext location of anyone else's device. To anyone without `sk_w`
the lid is random and cannot be linked across windows. The owner recomputes the lids of the
last seven days (96 per device per day) and opens the blobs.

**Location sharing** uses the memo key. A grant is the owner's `share_k` for the current and
next month, sealed to the grantee's on-chain memo key and stored in the grantor's memory
service (`MemorySet`, active-signed, about 500 bytes). The key name does not reveal who shares
with whom. Revoking is `MemoryDelete`. A grantee who already fetched a month's key keeps it
until the month ends; we state that limit plainly rather than hide it.

### 5.3 The phone as a full sensor (live)

The Android app is a complete sensor:

- **Location:** fused location, every reading kept and placed in the finest H3 cell its
  accuracy supports (res 11, 29 m, down to res 5). A private Wi-Fi fingerprint can stand in
  for GPS indoors.
- **Wi-Fi:** per access point `(oid, rssi, channel, band)` on 2.4, 5 and, where supported,
  6 GHz.
- **BLE crowd:** distinct advertisers per window, as oids, with broad hardware filters so
  counts are the same screen-on and screen-off.
- **HONE sightings:** window keys heard from other HONE devices.
- **Flipper relay:** every Flipper frame the phone hears is stamped with the phone's cells.
- **Sound (off by default):** Perch v2 (Apache 2.0) runs on the phone over 5-second frames
  held only in RAM. If Perch detects any human class (speech, singing, crowd), the whole
  frame is dropped and only a counter goes up. What leaves the phone is species with
  confidence, noise levels and listening time. A public test that the speech filter misses
  less than 1% of speech is part of the plan before mic data can earn.
- **Clock seat, wallet and updates:** the phone is a device clock seat, sends HONE and stakes
  or unstakes role stakes with the active key held in the Android Keystore behind a biometric
  prompt for every signature, and updates itself from `https://honemesh.net/app/`.

Measured battery cost is about 3.5 mA on average indoors with the screen off (roughly 1.7%
per day on a 5,000 mAh phone).

**Signed device status every 15 minutes (planned).** Each device will post a signed status
report every **15 minutes**, aligned with the ID windows, and on state changes at most once a
minute. A report carries counts, ages, on/off states and short error strings, and at most the
H3 res-7 cell (about 5 km) of the latest fix. It never carries keys, audio, location trails or
other people's IDs. The app side is in development; the collection endpoint is not live yet.

### 5.4 Flipper Zero (live)

The HONE Flipper firmware turns a Flipper Zero, optionally with a Wi-Fi board, into a signed
sensor:

- **Signed v2 frames.** Every frame has an ed25519 signature by the device key. New bodies:
  BLE crowd, sightings and Wi-Fi v2 (top APs as oids, AP count, locally administered APs,
  probe senders). Frames are batched into signed commit groups (sub-GHz, Wi-Fi, tags, IR, BLE
  crowd, sightings) and archived on the SD card.
- **BLE observer.** The firmware ships the full ST BLE stack so it can scan: 8 seconds every
  60, with day-keyed oids. Raw addresses never leave the scanning lane.
- **Sealed seed provisioning.** At claim time the owner's phone seals the device's ID seed to
  the Flipper's X25519 key. The Flipper opens it with its hardware AES and keeps the seed in
  RAM only.
- **No GPS.** A Flipper has no location. Its sightings carry `(lid, window, rssi, hits)`, and
  location comes only from a phone that hears it. Frames without a phone-stamped cell fill the
  archive but earn nothing under usefulness scoring.

### 5.5 Birds

The app identifies birds by sound (Perch v2) and photo, and a birds indexer
(`/api/birds/*` on honemesh.net) collects sightings and shared photos and serves everyone's
sightings back. **Live:** the indexer and app sync. **Planned:** reference call playback (it
needs an upstream API key), per-bird voice fingerprints, and public publishing of the bird,
Flipper and coverage data sets.

### 5.6 The coverage map: two layers

| | Public layer | Private layer |
|---|---|---|
| Who sees it | everyone | the owner only |
| Cells | H3 res 9 (about 170 m), coarser when accuracy is poor | finer, down to res 11 |
| Contents | AP count, signal distribution, channel congestion, BLE and probe crowd estimates, HONE device count, noise, species, mobile-AP count | own devices' sightings, own routers, Wi-Fi fingerprints, Flipper frames |
| Identities | none: no MAC, BSSID, SSID, oid, lid or rid | owner's own, encrypted |
| On chain | signed per-(cell, window) summaries and batch hashes | never |

Location trails never reach the chain. Each public summary carries a location weight
(the sum of 1/accuracy²), so the map combines readings by inverse variance: better readings
tighten the estimate, and a poor reading still contributes, just less. A person's own routers
are registered privately by proof of control and appear only in the private layer.

---

## 6. Rewards and Economics

### 6.1 Supply and eras

The supply cap is **42,000,000 HONE**. The block reward is a constant **2 HONE per epoch**
during the new-supply phase. Instead of halving the reward, HONE doubles the epoch length
every 4,200,000 epochs, which halves daily emission:

| Era | Epoch length | New supply per day | Duration |
|---|---|---|---|
| 0 | 30 s | 5,760 HONE | about 4 years |
| 1 | about 1 min | 2,880 HONE | about 8 years |
| 2 | about 2 min | 1,440 HONE | about 16 years |
| 3 | about 4 min | 720 HONE | about 32 years |
| 4 | about 8 min | 360 HONE | about 64 years |
| 5+ | recycle era | 0 new supply | indefinite |

Era 0's 30 seconds is the only fixed duration. At each era boundary the next era's epoch
length is recomputed so that HONE's new supply ends with Bitcoin's last coin:

```
D_N = (target_end_ms - now_ms) / (4,200,000 x (2^remaining_eras - 1))
```

The target is Bitcoin's projected last block (block 6,930,000 at 10 minutes per block, about
2140). A Bitcoin height oracle (`BtcHeightReport`), which would let nodes that also run
Bitcoin refine the target, is **planned**; today the compile-time projection is used.

### 6.2 Pools and layers

Each epoch's reward is split by measured work, not fixed percentages. Each pool (clock,
inference, storage, sensor, verifier, service, mempool, tracker, runtime) has a calibration
target, and its share is its utilization against that target, normalized across active pools.
A pool with no claimants does not accumulate; its share recycles. Four layers combine:

- **Layer D, infrastructure base:** always fires. A small per-clock reward (0.001 HONE per
  clock at era 0), a testnet operator reward, and a mandatory reserve.
- **Layer B, activity pools:** utilization over target, with targets drifting slowly toward
  long-run 50% utilization.
- **Layer A, long-term scalar:** a fast and a slow moving average of utilization set the
  reward ceiling between 70% and 100% of the nominal schedule.
- **Layer C, fee boost:** verified fee volume from the previous epoch can raise emission
  toward the Layer A ceiling, with circular same-epoch flows netted out.

Inference fees split 80% to the worker, 15% to verifiers and 5% to recycle (70/10/15/5 with
human reviewers when disputed).

### 6.3 No burn: the recycle fund

Tokens are never burned. Fees, surplus pool shares, slash proceeds and rounding remainders
flow to `__recycle_fund__`. From era 5 the fund becomes the permanent reward source: each
epoch it pays out 0.001% of its balance across the same pools. Because the draw is
proportional to the balance, the fund self-corrects, and the 42,000,000 cap is never breached.

### 6.4 Sensors: pay only for useful information (gated)

**One rule for every metric: pay for what is new or what changed.** A change is an access
point appearing in or disappearing from a cell, a value (signal, congestion, crowd, noise,
species activity) leaving its noise band, the same device seen in a different cell, or a
species arriving or leaving.

A `SensorSummary` carries one public `(cell, window)` row of `metric -> value`. Scoring is
deterministic and runs at apply, in commit order, on every node:

- **Novel** is the first report for (cell, metric, day bucket), or a value outside the
  metric's noise band around the chain's current estimate. Noise bands are fixed per metric
  (for example ±6 dB for signal strength, ±30% for crowd counts, exact for HONE device
  counts). The first valid reporter earns the novelty points (100), and the estimate moves.
- **Repeats earn 0.** One summary per device per metric per window, and one per *owner* per
  (cell, window, metric): a phone and a Flipper of the same owner count once.
- **Verification is paid near 0.** A later matching report by a different owner within ±2
  windows earns 1 point and raises the original reporter's trust. One owner's devices never
  verify each other.
- **Contradiction:** a different owner in the same cell and window, outside the band, earns
  0 and lowers the original reporter's trust.

**Deferred payout.** A novel claim is not paid when it arrives. It pays once it is proven:
by an independent owner's corroboration, or by surviving a challenge window of
**2,880 epochs (about 24 hours)** without a contradiction.

**Old data pays nothing.** Data older than **7 days** at submission fills the map but carries
no claim.

**Activation.** `USEFULNESS_SCORING_EPOCH` is currently `u64::MAX`: the scoring code is built
and tested, but inactive. Proven-point payout has its own gate, `USEFULNESS_PAYOUT_EPOCH`, also
unset. An earlier activation at 352,700 was withdrawn so that deferred payout could be built
first. **Until activation, the original volume scoring (`sensor_score(reading_count)`) still
pays** for `SensorDataCommit`. Both gates switch together on every node at an approved epoch.

Anti-gaming rules beyond the above are planned: claw-back on contradiction, a location check
that requires a report's Wi-Fi and BLE fingerprint to match the cell's known access points
(GPS is spoofable), voiding windows that "teleport" faster than plausible travel, and escrow
plus independent corroboration for rare acoustic events.

---

## 7. The Decide Jury (shadow mode)

Some questions cannot be answered by a deterministic rule: is this coverage report useful, is
this sensor reading plausible, is this entry normal. HONE answers them with a jury of small
models on staked devices. It is designed so a juror cannot see others' votes before voting,
cannot pick its cases, and cannot win by voting with the crowd.

### 7.1 Always-on sealed votes

Every judgment entry committed at a quorum seal gets **K = 7** voters, drawn deterministically
from the epoch's seal hash among staked `decide` nodes with capacity left. Each voter asks its
local `hone-decide` service, which returns an option and a confidence. The voter seals its
votes as leaves of a Merkle tree (each leaf salted, over entry hash, epoch, task, option and
confidence) and publishes only the root, as one **`DecideCommit` per epoch**. Votes stay
hidden until revealed.

### 7.2 Random jury on dispute

A `DecideChallenge` opens a jury of **5 jurors plus 3 backups**, drawn from the seed of the
**first sealed epoch after the challenge**, so nobody can know the jury when deciding whether
to challenge. Jurors post `DecideReveal` with a Merkle proof against the root they committed
earlier, within **20 epochs**. A reveal that fails its proof is a strike. A **2/3
supermajority** of valid, non-abstaining reveals decides. Otherwise the case escalates to a
larger model and then to the deterministic rule. The challenge window is 2,880 epochs.

**At most one device per owner** sits on any jury, across jurors and backups.

### 7.3 Spot checks and canaries

Three (node, entry) pairs per epoch are drawn as **spot checks** and must be revealed without
any dispute, so a node that never really votes is caught. **Canaries**, items with a known
answer, are shaped exactly like real items (unknown fields are rejected, so there is no canary
flag to detect).

### 7.4 Models and confidence

The jury models are small students distilled from **Open-Jev** (MIT), at **135M and 360M**
parameters, small enough to run on a phone. Each model ships a `thresholds.json` fitted on
held-out data. A juror whose confidence is below the threshold for 95% accuracy (t95)
**abstains** rather than guess. The pinned model is `smollm2-135m-r3` Q4, model hash
`04df1989…2ab2ed`. Hashing the whole model directory, thresholds included, keeps every juror
on the same model.

### 7.5 Stake and seats

A voting device holds a `decide` `NodeRoleStake`. The intended stake is **100 HONE per voting
device**; the chain currently accepts any non-zero stake, and enforcing the minimum is planned.
Device seats may hold a decide stake from epoch 352,440, and phones have light-seat API routes
for work, commit and reveal.

### 7.6 Status

Decide entries are accepted from epoch **352,400**, in **shadow mode only**. They write
`decide_*` meta records that are outside the state root, carry no fee, and change no balance,
stake or other entry's validity. Outcomes are logged next to the deterministic rule, which is
what actually holds. `DECIDE_JURY_BINDING_EPOCH` is unset. Jury outcomes become binding one
task at a time, only after shadow results justify it. The usefulness tasks
(`coverage_usefulness`, `sensor_plausibility_v2`) run beside usefulness scoring the same way.

---

## 8. Networking

### 8.1 Transport

HONE uses libp2p: TCP with Noise and Yamux, QUIC for low-latency and mobile-friendly
connections, and WebSocket for nodes behind restrictive networks. Gossipsub carries entries
and seals, Kademlia finds peers, Identify and Ping keep connections honest, and mDNS finds
peers on the local network. Peers identify themselves with a signed `hone-id` message
(`GET /api/net/peers`).

### 8.2 Every node finds every other node

The requirement is simple: a phone, a PC and a Pi must find each other anywhere in the world,
not only on one LAN. On 2026-11-04 the network learned what happens otherwise. Both phones
left the LAN, a third clock went offline, quorum fell to 2 of 5, and finality stalled until a
public path existed.

Discovery, in order:

1. **Peer cache** in RocksDB, peers seen in earlier sessions.
2. **Public-first bootstrap (live in the app and node configs).** Every node dials the
   public node at `/dns4/honemesh.net/tcp/6942` before anything else.
3. **DNS seed (planned).** `/dnsaddr/bootstrap.honemesh.net`, read from
   `_dnsaddr.bootstrap.honemesh.net` TXT records, so seeds change in DNS with no release.
   This replaces the earlier `bootstrap1` and `bootstrap2` names and the `_hone._udp` TXT
   design, none of which were ever published.
4. **Kademlia** walks the DHT from any connected peer.
5. **Censorship-resistant fallbacks:** a peer list on the Hive blockchain (live code), with
   TON and Bitcoin Ordinals registries designed and stubbed.

**NAT traversal (planned, in development):** circuit relay v2 (the public node as relay),
AutoNAT to learn whether a node is reachable, DCUtR hole punching to upgrade relayed
connections to direct ones, and UPnP port mapping on home routers.

### 8.3 The public node and wallet API (live)

A public, non-clock node runs at **honemesh.net**. It holds no posting key, relays gossip
and serves a **public wallet API at `https://honemesh.net/chain/*`**. The API is an
allowlist: reads (balances, accounts and history, role stakes, name resolution, entry
status, memory, sensors, Find-My grants, supply, node info) and signed-entry submits
(transfer, role stake and unstake, sensor commit and key registration, memory writes). Every
other path returns 403. Bodies are capped at 256 KB and requests are rate-limited per IP.
Clients sign locally; the public node never holds a user key.

---

## 9. Privacy

HONE's privacy rules are design constraints, not settings.

- **No stable identifiers on air.** HONE devices rotate their ID and BLE address every 15
  minutes on the UTC quarter-hour, all at once.
- **No linkable identifiers for anyone else.** Other devices are reduced to day-keyed hashes
  that cannot be linked across days or observers.
- **Sightings are for the owner.** Locations of a device are encrypted to that device's
  window key. Observers keep nothing they can read.
- **The chain sees aggregates.** Only per-(cell, window) summaries with no identities reach
  the chain. Location trails never do.
- **Sharing is explicit, scoped and expiring.** Location sharing is a memo-key grant per
  device per month, revocable, with its limit stated.
- **Audio never leaves the phone.** Frames live in RAM for five seconds. Any frame containing
  human sound is discarded whole. Mic sensing is off by default and needs the user's explicit
  acceptance of a notice. Whether using the mic is lawful where the user is, is the user's
  call.
- **Keys stay where they belong.** The hide and owner keys stay on the wallet host. A phone
  finds its owner's devices through a hardware-bound grant, and its active key is
  biometric-gated per signature.
- **Status reports are coarse.** The planned 15-minute device status carries at most a
  5 km cell, never trails, audio, keys or other people's IDs.

---

## 10. Security and Threat Model

| Threat | Defence | Status |
|---|---|---|
| Unsigned spend for keyless accounts | role fallback only to posting; no-key signers rejected (320,200) | live |
| Compromised node steals funds | nodes hold a clock key; transfers and unstakes need the active key | live |
| Stolen name | NFT protection levels, pinned-destination recovery, founder backstop | live |
| One owner sybils quorum or checkpoints | checkpoints count distinct operators; device seats never count | live |
| Node drifts from consensus state | hourly signed checkpoints, self-healer halts and rebases | live (report mode default) |
| Restart or partition loses entries | persisted pending pool, entry and finalize backfill | live |
| Gossip silently stops | one dial per peer, gossip watchdog, stall guard | live |
| Phones isolated behind NAT | public node; relay, AutoNAT, DCUtR, UPnP | public node live; NAT traversal planned |
| Sensor volume farming | usefulness scoring; repeats 0; one reporter per owner | gated |
| Self-corroboration | one owner's devices never verify each other; one juror per owner | gated / shadow |
| Paid for unproven claims | deferred payout: corroboration or a 24-hour challenge window | gated |
| Stale data replay | no claim on data older than 7 days; duplicate and replay detection | gated |
| GPS spoofing, teleporting | fingerprint match to known APs; teleport voids window | planned |
| Lazy or colluding jurors | sealed commits, post-challenge jury seed, spot checks, canaries, abstain under t95 | shadow |
| Inference crash takes down a node | models run in a separate sidecar with its own memory budget | live |
| Tracking through the sensor network | rotating IDs, day-keyed hashes, owner-encrypted sightings | live |
| Lost keys | mandatory recoverable keystore; mnemonic is the device-seed backup | live |

**Known limits, stated plainly.** Hardware tiers are self-declared. Usefulness and jury
outcomes do not move balances yet. A location-share grantee keeps a month key it already
fetched. Until the deterministic checkpoint cursor is deployed, some checkpoints hold rather
than agree. Until NAT traversal ships, phones off the LAN depend on the public node.

**Operational rule.** No balance is ever rewritten by hand without the founder's explicit
approval. One-time corrections use a dedicated, audited `state-correct` path.

---

## 11. Governance and Activation Epochs

There is no on-chain governance token, council or multisig that can change HONE's fundamental
parameters (epoch length, supply, coin type). Protocol parameters (fee rates, timeouts,
storage pricing) are set by each protocol account's owner. Changes to consensus rules ship as
**activation epochs**: a constant compiled into every node. Every node must run a binary that
knows the rule before the epoch is reached, and the rule switches on at the same epoch
everywhere. An activation epoch is set only after the founder approves it, and is kept well
clear of checkpoints and at least an hour after the last node upgrades. An unapproved rule
ships with its epoch set to `u64::MAX`, which means built, tested and off.

| Constant | Epoch | Status |
|---|---|---|
| `CLOCK_BOOTSTRAP_GRACE_END_EPOCH` | 250,000 | past |
| `SIGNED_AUTH_ACTIVATION_EPOCH` | 320,200 | live |
| `DEVICE_CLOCK_ACTIVATION_EPOCH` | 321,000 | live |
| `IDENTITY_NFT_ACTIVATION_EPOCH` | 333,333 | live |
| `MEMORY_SIGNING_ACTIVATION_EPOCH` | 349,100 | live |
| `NODE_ENTRY_CLOCK_SIG_EPOCH` | 349,100 | live |
| `SEAL_LAG_ACTIVATION_EPOCH` (lag 2) | 351,880 | live |
| `NAME_NFT_MIGRATION_EPOCH` | 352,000 | live (1,162 names) |
| `DECIDE_ENTRIES_EPOCH` | 352,400 | live, shadow |
| `DECIDE_DEVICE_SEATS_EPOCH` | 352,440 | live |
| `USEFULNESS_SCORING_EPOCH` | unset (`u64::MAX`) | gated |
| `USEFULNESS_PAYOUT_EPOCH` | unset (`u64::MAX`) | gated |
| `FIND_GRANT_EPOCH` | unset (`u64::MAX`) | gated |
| `DECIDE_JURY_BINDING_EPOCH` | unset (`u64::MAX`) | gated |

Future on-chain governance, if any, will be opt-in and weighted by contribution, not imposed.

---

## 12. Implementation Status

As of 2026-11-04, branch `v2.1-live-identity`.

| Area | Mechanism | Status |
|---|---|---|
| Consensus | 30 s epochs, 51% quorum seals, no solo rewards | live |
| | Seal lag E−2 | live (351,880) |
| | Reward finality depth 20 | live |
| | Persisted pending pool and outbox | live |
| | Backfill of entries and finalize meta | live |
| | Hourly checkpoints, self-healer | live (report mode by default) |
| | Deterministic checkpoint cursor (E−21) | merged, not deployed |
| Identity | Six role keys, recoverable keystore | live |
| | Names as NFTs | live (333,333) |
| | Reserved-name migration, 1,162 names | live (352,000) |
| | Device clock seats `owner@pubkey` | live (321,000) |
| | Decide seats on devices | live (352,440) |
| | Find-My grants (FIND_GRANT, FIND_REVOKE, SEED_GENERATION) | gated, epoch not set |
| | Typed bech32 addresses (`hh1…`) | planned |
| Sensors | Rotating seeded IDs, 15-min synced rotation | live (app, Flipper) |
| | Keyed hashes for non-HONE devices | live |
| | Owner-encrypted sightings, memo-key location sharing | live (app) |
| | Phone full sensor (location, Wi-Fi, BLE crowd, sightings, Flipper relay) | live |
| | Perch v2 sound and bird sensing with speech dropped | live in app, off by default |
| | Birds indexer | live |
| | Flipper signed v2 frames, BLE observer, sealed seed, SD archive | live |
| | Signed device status every 15 minutes | planned (app side in development) |
| | Public bird, Flipper and coverage data sets | planned |
| | Two-layer coverage map | private layer live on device; public summaries queued for the summary commit |
| Rewards | Volume sensor scoring | live until usefulness activates |
| | Usefulness scoring, verification near 0, 7-day limit | gated |
| | Deferred payout (corroboration or 24 h window) | gated |
| | Claw-back, location fingerprint check, teleport voiding | planned |
| Jury | Sealed commits, Merkle root per epoch, post-challenge jury, spot checks, one device per owner | live, shadow |
| | 135M / 360M Open-Jev-derived models with t95 abstain | live (pinned 135M) |
| | 100 HONE minimum decide stake | intended; not enforced by the chain yet |
| | Binding outcomes | gated |
| Networking | TCP, QUIC, WebSocket, gossipsub, Kademlia, mDNS | live |
| | One dial per peer, gossip watchdog | live |
| | Public node and wallet API at honemesh.net | live |
| | Public-first bootstrap | live in configs and app |
| | DNS seed `bootstrap.honemesh.net` | planned |
| | Circuit relay, AutoNAT, DCUtR, UPnP | planned, in development |
| Inference | Separate sidecar process, node never loads models | live |
| | On-chain model registry | planned |
| Phones | Clock seat, sensors, wallet send and stake (biometric), in-app updater | live |
| Protocols | Freeport, Verasens, LinkGit | live |
| | Wiiv | planned (types only) |
| Economics | Era schedule, pools, recycle fund | live |
| | Bitcoin height oracle | planned |

---

## 13. Roadmap

**Next (weeks):**

1. Approve and activate `FIND_GRANT_EPOCH`, then switch phones to hardware grants.
2. Ship NAT traversal (relay, AutoNAT, DCUtR, UPnP) and publish the
   `bootstrap.honemesh.net` DNS seed, so phones on cellular networks keep quorum.
3. Deploy the deterministic checkpoint cursor and run the healer in heal mode everywhere.
4. Launch the 15-minute signed device status endpoint and its dashboard.
5. Set the usefulness scoring and payout epochs together, then resume the queued Flipper
   replay under the new scoring.

**Then (months):**

- Enforce the 100 HONE decide stake. Make the first jury task binding once shadow results
  justify it.
- Add claw-back, location fingerprint checks, teleport voiding and rare-event escrow.
- Publish the public bird, Flipper and coverage data sets, and bird voice fingerprints.
- Verifiable speech removal with a public test, before mic data earns.
- Attested hardware tiers. The on-chain model registry. Wiiv ledger entries.
- The Bitcoin height oracle.

**Later:**

- A HONE hardware-wallet app.
- Research pools where negative results also have value, and sensor data correlated with
  external events.

---

## 14. Links

- **Website:** [honemesh.net](https://honemesh.net)
- **White paper (PDF):** [honemesh.net/HONE_WHITEPAPER.pdf](https://honemesh.net/HONE_WHITEPAPER.pdf)
- **Android app:** [honemesh.net/app/hone-latest.apk](https://honemesh.net/app/hone-latest.apk)
- **Public wallet API:** `https://honemesh.net/chain/api/...`
- **GitHub:** [github.com/shindevlin/hone](https://github.com/shindevlin/hone)
- **Native Protocols Overview:** [NATIVE_PROTOCOLS.md](NATIVE_PROTOCOLS.md)
- **Sightings, coverage and rewards spec:** [spec-sightings-coverage-rewards.md](spec-sightings-coverage-rewards.md)
- **Address Scheme:** [ADDRESS_SCHEME.md](ADDRESS_SCHEME.md)
- **Chain constants:** [CHAIN_CONSTANTS.md](CHAIN_CONSTANTS.md)

**Licence.** HONE is published under the Usufruct License (UFL). The canonical text is the
`LICENSE` file in the repository.

---

*HONE v2.0, November 2026, Shin Devlin*
