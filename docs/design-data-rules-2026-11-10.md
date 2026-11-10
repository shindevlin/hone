# HONE data rules: every reading kept, novelty pays (design for Josh, 2026-11-10)

Josh's rules (10:17 and 10:50 local):

1. Every summary goes on chain. Novelty decides the reward, never whether a summary is included.
2. Every raw reading goes to storage nodes, encrypted and sharded. Its content hash and the storage receipts go on chain.
3. Raw data has two tiers:
   - **Hot:** held by storage nodes, which are paid for proven storage and for retrieval.
   - **Cold:** after a network-set age, raw data moves to a backup-only archive. It stays provable by hash and retrieval is paid.
4. Every node keeps recent blocks plus checkpoints. Older history lives in the storage-role archive (RS 4-of-6), and anyone can rebuild it.
5. Privacy does not change. Operators cannot decrypt personal data, and locations stay coarsened.

Branch `data-rules` (based on `release-373000`) holds this doc and the first gated pieces. Every gate is `u64::MAX`, and nothing activates without Josh.

## 1. Does anything drop non-novel summaries? (audit)

| Place | What happens to a non-novel or odd row | Verdict |
|---|---|---|
| Chain, `tx.rs` SENSOR_SUMMARY validation | Checks the signature, seat stake, cell format, metric names and count. There is no novelty check and no window-age check. | Keeps it |
| Chain, `usefulness::score` | Each metric gets a verdict (Novel, Verify, Contradict, Repeat, Duplicate). Every verdict is stored with the entry; only points differ. A second row for the same (owner, cell, window) is stored as Duplicate with 0 points. | Keeps it |
| App, `CoverageSubmitter.chainRound` (branch `android/sound-sensing`) | (a) Unlocated rows become MAP_ONLY because the chain has no null cell. (b) Res-9 rows older than 7 days (`MAX_AGE_WINDOWS = 672`) become MAP_ONLY. (c) Only one row per (chain cell, window) is sent, the one with the most location samples; the others become MAP_ONLY. (d) Res-7 rows are held, never dropped, until the chain accepts res 7. | (a), (b) and (c) drop rows from the chain |

None of the drops is about novelty, but (a), (b) and (c) still break "every summary on chain". The fixes:

- **(a)** Gate `NULL_CELL_EPOCH` (built). A summary with an empty cell is accepted and stored, and scores nothing. Once the gate is active, the app sends unlocated rows with `cell: ""`.
- **(b)** The chain has no window-age limit, so this is an app-only rule. Drop it in the app when NULL_CELL activates. Old rows get 0 points anyway, because the claim window has passed.
- **(c)** Send every row. The chain's `usum_o:{owner}:{cell}:{window}:{metric}` key already pays only the first and stores the rest as Duplicate. This is an app change.
- **Inferred locations.** Gate `INFERRED_LOC_EPOCH` (built). A row whose place is inferred (last fix, Wi-Fi fingerprint, home AP) carries `loc.inferred = 1` and `loc.inferred_age_s`. It must use a res 7 or res 8 cell, never res 9. Both metrics are `loc.*` context metrics, so they are stored but never scored. The environment metrics on that row score normally at the coarse cell.

## 2. Measured sizes

Sources:
- nebra's live queue: `/srv/hone/sensors/queue`, 116 files, 15-minute windows.
- the phone wire format: `SensorSummaryWire.body` with the metric set from `CoverageSubmitter.metrics`.
- the chain's `/api/sensor/usefulness/{josh,shindevlin}` at epoch 371,264, giving 2,025 and 511 claims since 354,420 (5.85 days).
- the live block size at the tip.

A year has 35,040 windows of 15 minutes and 1,051,200 epochs of 30 s.

### 2.1 Summaries (on chain, signed JSON)

| Device | Bytes per row | Rows per day | Per year |
|---|---|---|---|
| nebra Wi-Fi (42 APs, 14 scans, 5 fp tokens) | 325 unsigned, **~485 signed** | 96 | **17.0 MB** |
| nebra LoRa (8 channels, crc counts) | 257 unsigned, **~415 signed** | 96 | **14.5 MB** |
| **nebra total** | | 192 | **31.5 MB** |
| Phone (21 metrics: Wi-Fi with bands and histogram, BLE, hone, loc) | **648** | 40–55 observed (josh about 55, shin about 40), 96 if located every window | **9.5–13 MB** observed, **22.7 MB** at 96 per day |
| Phone with Flipper metrics | 716 | same | up to **25 MB** |
| Phone unlocated rows (NULL_CELL) | ~600 | up to 96 minus located rows | up to **~12 MB** |

How the phone row rate was derived: josh's claims average 6.25 metrics per (cell, window) and shin's 2.2, so 2,025 / 6.25 / 5.85 ≈ 55 rows per day and 511 / 2.2 / 5.85 ≈ 40 rows per day. These counts cover only rows that produced a claim, so they are a lower bound.

### 2.2 Raw readings (to storage, not on chain)

Encoding is columnar binary per window: an AP or device table once per window, then one RSSI byte per scan, then zstd. JSON would be about 40 times larger (shown for comparison).

| Device | Raw per window | Per year, plain | Per year stored (AEAD plus RS 4-of-6 = ×1.5) |
|---|---|---|---|
| nebra Wi-Fi: 42 APs × 14 B table, plus 15 scans × 42 × 1 B | ~1.25 KB (JSON ~57 KB) | **43.8 MB** (JSON ~2.0 GB) | 66 MB |
| nebra LoRa: PHY metadata about 40 B per packet, no payload; 12 per window today (all CRC-bad) | ~0.5 KB | **16.8 MB** (a busy gateway with 1,000 packets an hour: ~350 MB) | 25 MB (525 MB busy) |
| **nebra total** | | **~61 MB** | **~92 MB** |
| Phone: Wi-Fi 38 APs plus 8 scans, BLE 41 devices plus 4 scans, about 12 GNSS fixes | ~1.8 KB | **36 MB** at 55 windows per day, **63 MB** at 96 | 55–95 MB |

Each storage node holds 1/4 of each object it is assigned, so 6 holders together carry 1.5 times the plain size. Per year that is about 23 MB per node for nebra and about 24 MB per node for a phone.

### 2.3 On-chain cost of raw readings

| Item | Size | Rate | Per year |
|---|---|---|---|
| `RAW_ANCHOR` (one per device per checkpoint hour, Merkle root of its 4 windows: object_id, size, shard roots root, schema id, signature) | ~380 B | 8,760 per device | **3.3 MB per device** |
| Storage receipt (one per storage node per checkpoint, the root over all shards held) | ~250 B | 8,760 per node | 2.2 MB per node (6 nodes: 13 MB for the whole network) |
| Challenge verdicts (pass or fail per challenged shard; the 1 KiB proof bodies go through the decide-by set and are pruned with old blocks) | ~100 B | 4 per node per hour | 3.5 MB per node |

Summary, per device per year:

| | nebra | phone (observed to 96/day) |
|---|---|---|
| Summaries on chain | 31.5 MB | 9.5–23 MB |
| Raw anchors on chain | 3.3 MB | 3.3 MB |
| Raw in hot storage (network total, ×1.5) | ~92 MB | 55–95 MB |
| Raw plain | ~61 MB | 36–63 MB |

Raw data costs about 2 to 4 times the summaries in bytes, but nearly all of it is off chain. The chain carries about 10% more per device than it does for summaries alone.

### 2.4 The chain itself

An idle block (seal and header, JSON) averages **855 B**, which is ~0.9 GB a year of block shells before any entries. On grouchly the state directory today is 386 MB. With ten devices at the rates above, summaries add 0.2–0.35 GB a year. This is why old history moves to the archive (section 5).

## 3. Raw readings: format and flow

1. **On the device.** The device closes the window and builds the raw object: columnar, zstd, with a `schema_id@version` header.
2. **Encryption.** A fresh data key encrypts the object with XChaCha20-Poly1305. The data key is wrapped to the owner's device key. The plaintext never leaves the device.
3. **Hash and anchor.**
   - `object_id = H(ciphertext)`.
   - `commit = HMAC(data_key, H(plaintext))`. This lets the owner later prove the content to a buyer without revealing it now.
   - The four windows of a checkpoint hour form one Merkle root. The device signs `RAW_ANCHOR { device, hour, root, objects: [object_id, size, schema], epoch }`.
4. **Upload.** The device or its gateway uploads the ciphertext. Placement follows §3.3 of the useful-rewards design (rendezvous hashing, at most k−1 shards per operator). The new class is **R: raw readings**, with privacy like class U, coded RS 4-of-6. Raw objects are small, so they are packed into daily per-device bundles before sharding to keep the manifest count low.
5. **Storage.** Nodes hold shards, answer challenges (§3.4 of useful-rewards) and sign receipts. Pay follows §3.6: proven bytes only.
6. **Summary link.** The summary for the same window may carry `raw = object_id` in a later wire version (schema registry, section 6). Until then, the anchor's (device, window) is the join key.

**Privacy.**
- Storage nodes see ciphertext, size and the device id. The anchor carries no cell.
- Precise fixes exist only inside the ciphertext. Summaries keep res 7 at or near home and at most res 9 elsewhere.
- The `LIVE_WINDOW_EPOCHS` parameter (built, OFF) is reserved for the paid-archive phase. Inside the live window, the owner may publish data keys for public raw access. After it, keys are released only to paying buyers. With the parameter OFF (u64::MAX) nothing changes: summaries are public and raw stays owner-only.

## 4. Hot and cold tiers

| | Hot | Cold |
|---|---|---|
| Age | younger than `cold_after_epochs` (network parameter; proposal 259,200 = 90 days) | older |
| Holders | staked storage nodes, RS 4-of-6 | storage nodes that also stake for `archive`, RS 4-of-6 (the same objects are re-placed, never re-encrypted) |
| Proof | challenges every checkpoint (§3.4) | challenges every 2,880 epochs (1 day), same Merkle roots |
| Pay | proven GB-epochs at the hot rate, plus retrieval receipts | proven GB-epochs at a lower cold rate (proposal 1/4 of hot) |
| Retrieval | free to the owner; receipts pay the server from the pool | **paid by the requester** (a fee per GB to the serving node), with the receipt on chain |
| Delete | owner may request deletion; holders drop shards and the anchor stays | same |

The move from hot to cold is deterministic: at each checkpoint, objects whose anchor epoch is older than `cold_after_epochs` are re-placed onto archive holders, and the old holders keep proving until the new ones pass.

## 5. Chain history: recent blocks on every node, the rest in the archive

- **Every node** keeps blocks newer than `hot_history_epochs` (proposal 86,400, which is 30 days), every checkpoint (one per 120 epochs), and the current state.
- **Older blocks** are packed into daily eras (2,880 epochs, about 2.5 MB at today's size) as class H objects in the storage role, RS 4-of-6, with at least 3 operators. Each era's root is written into the first checkpoint after the era ends. That makes it anchored, and anyone can rebuild it from any 4 shards and verify it against the checkpoint.
- **A new node** syncs the latest checkpoint state plus the hot blocks. A full-history node fetches the eras on demand and pays retrieval like the cold tier.
- **Pruning** a block is allowed only after its era's anchor is final **and** at least 4 shard holders of the era have passed a challenge.

## 6. Built now (branch `data-rules`, gates OFF)

| Gate (hone-types) | Value | What it does |
|---|---|---|
| `INFERRED_LOC_EPOCH` | u64::MAX | A row with `loc.inferred = 1` and `loc.inferred_age_s >= 0` must use a res 7 or res 8 cell. Both metrics are context and never scored. |
| `NULL_CELL_EPOCH` | u64::MAX | A summary with an empty cell is accepted, stored and scores nothing. It cannot also be flagged inferred. |
| `SCHEMA_ANCHOR_EPOCH` | u64::MAX | The governance `ChainParameterSet` key `schema:<id>@<version>` takes a value of 64 lowercase hex. Each version is write-once. The council and posting signature are checked as for every other parameter, and the 2-epoch timelock applies. Read it with `GET /api/chain/param/schema:<id>@<version>`. |
| `LIVE_WINDOW_EPOCH` | u64::MAX | From this epoch, the governance key `live_window_epochs` (an integer of at least 2,880, or `off`) is read by `data_rules::live_window_epochs`. Before it, or when unset, the value is u64::MAX, meaning everything stays public. |

- New view: `GET /api/chain/data-rules` shows every gate and the current live window.
- **Before each gate, behaviour is unchanged:** there are no new checks or state writes, and the error text for bad cells is the same.

**Not built yet** (needs the useful-storage gate first): the `RAW_ANCHOR` entry, class R placement, hot/cold re-placement, era archiving and pruning. They go on `useful-rewards` after Josh's answers below.

## 7. Questions for Josh

1. **Inferred rows.** Should they earn like measured rows at their coarse cell (as built), or only corroborate, never earning alone?
2. **Null-cell rows** score nothing (as built). Should they count toward a device's uptime or other liveness credit?
3. **Cold age.** Is the proposal of 90 days for `cold_after_epochs` right? And should cold retrieval cost about 1/4 of hot storage pay per GB?
4. **History on every node.** Is 30 days of blocks right for `hot_history_epochs`?
5. **LoRa raw.** Should it keep only PHY metadata (as proposed, no third-party payloads), or also store encrypted payload hashes?

