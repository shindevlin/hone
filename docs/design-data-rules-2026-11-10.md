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

None of the drops is about novelty, but (a), (b) and (c) still break "every summary on chain".

Update 11:30: app 1.15.0 (branch `app/key-vault-agent` @4d365c9e) already fixes (b) and (c): it sends every row at any age. It also parks rows with no area at all until the null-cell gate; their wire shape is defined in section 6.1. The fixes:

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

## 6. Josh's decisions (11:22 local) and how they are built

### 6.1 Provenance on every stored reading (gate `PROVENANCE_EPOCH`)

> "if there is no way to see where a devices information comes from, there is no reason to store it"

From the gate, every SENSOR_SUMMARY carries `prov`, and a row without it is refused. `prov` is signed together with the data: it comes after `metrics` in the canonical message.

| Question | Field | Rule |
|---|---|---|
| What: device | `prov.dev` | The first 16 hex of the device or seat key. A seat-signed row must name its own seat key. A relayed row must name the device key that signed it. |
| What: class | `prov.cls` | phone, nebra, pc, pi, flipper or station (registry `hdr.cls`) |
| What: sensor | `prov.sensor` | A short lowercase name (coverage, subghz, lora, wifi, ...) |
| What: schema | `prov.schema` | `<id>@<version>` of the raw records summarised (for example `hone.raw.hdr@1`) |
| When | the entry's `window` | It was already required. |
| Why | `prov.why` | `window` (scheduled), `event` (a detection) or `gap:<reason>`, using the registry's gap reasons |
| Why: where | `prov.loc_src` | gps, fused, network, wifi_fp, last_verified, fixed, ip or home_wifi; or `none:<reason>` (no_fix, offline, permission_denied, no_signal, sensor_absent, app_stopped) |

- A row has an empty cell **exactly when** `loc_src` is `none:<reason>`. A null-cell row also needs the provenance gate, so no unexplained row is ever stored.
- An inferred row (`loc.inferred = 1`) needs a real source, never `none`.
- Before the gate, `prov` is refused so old nodes never see data they can't verify. Without `prov` the signed message is byte-for-byte the old one. Both are pinned by tests.

Wire format, the exact bytes the device signs (test `summary_wire_message_with_provenance_and_null_cell`):

```
{"chain_id":"hone","type":"SENSOR_SUMMARY","sensor_id":"coverage-0123456789abcdef","owner":"josh","batch_hash":"","cell":"","window":5,"metrics":{"obs.none":1},"prov":{"dev":"0123456789abcdef","cls":"phone","sensor":"coverage","schema":"hone.raw.hdr@1","why":"window","loc_src":"none:no_fix"}}
```

The POST body adds `"prov":{...}` and, for relayed rows, `"relay":{"by":...,"sig":...}`. `cell` may be `""` or `null` in the body; it is always signed as `""`.

### 6.2 Inferred locations earn only when corroborated (gate `INFERRED_LOC_EPOCH`)

- An inferred row needs `loc.inferred = 1`, `loc.age_s >= 0` (the app's metric name) and a **res 7 or 8** cell. `loc.*` metrics are context and never scored.
- Its claims are marked `inferred` together with the device. When the claim is decided, it is proven only if one of these holds:
  - another owner's evidence was recorded on the claim (corroborate or verify), or
  - **an independent device** (a different `prov.dev`; it may belong to the same owner, such as the phone a Flipper is live-matched with) reported a **measured** row in the same or a neighbouring cell (H3 ring 1, same resolution) within one window either side.
- Otherwise the claim is void (`void_reason: inferred_uncorroborated`). Uncorroborated inferred rows carry no trust penalty.
- Measured rows are indexed at their own cell and every ancestor down to res 7 (`upres:{cell}:{window}:{dev}`). A res-9 phone row inside a res-7 inferred cell therefore counts.
- Possible later tightening: also require that the corroborating device actually heard the other one (the phone's `flipper:<id>` key in its raw records).

### 6.3 Relay credit (gate `RELAY_CREDIT_EPOCH`)

- Any HONE phone may upload another sensor's reading. The reading is signed by the **sensor's own enrolled device key** (`sensor_devkey:{sensor_id}`, enrolled to the owner).
- The relayer adds `relay = {by, sig}`, where `sig` signs `{"chain_id","type":"SENSOR_RELAY","by","entry": sha256(device-signed message)}` with the relayer's staked seat key or posting key.
- The relayer can't change the data, because the device signature covers it. It can't be swapped for someone else, because its own signature names it. A relayer that signs the data itself is refused.
- **Share: 15%** (`RELAY_SHARE_BPS = 1500`) of that reading's points when the claim is proven. The owner keeps 85%. No share is paid when the relayer is the owner (the owner's own phone), so this pays only when the owner's phone wasn't the one connected.
- Example: shindevlin's Flipper relayed through josh's phone pays josh 15 of a 100-point novel claim, and pays shindevlin 85.

### 6.4 Null-cell rows earn nothing (confirmed)

They are stored with provenance and score nothing.

### 6.5 Dynamic cold age (proposal, not built; it needs the storage role)

`cold_after_epochs` is recomputed once a day (at the first checkpoint of each UTC day) from hot capacity and demand:

```
free   = 1 - used_hot_bytes / declared_hot_capacity        (staked storage nodes, at that checkpoint)
target = 0.20                                              (governance: cold_target_free)
step   = clamp(0.5 * (free - target), -0.10, +0.10)        (at most 10% a day either way)
cold_after_next = clamp(cold_after * (1 + step), 86_400, cold_after_max_epochs)
```

- The floor is 86,400 epochs (30 days). `cold_after_max_epochs` is a governance ceiling; the proposal is 1,051,200 (1 year).
- **Demand:** a paid retrieval (one with a receipt) of a hot object resets that object's hot clock, up to the ceiling. Data people use stays hot.
- **Effect:** spare capacity keeps data hot longer, which raises hot GB-epochs paid, which pulls in more storage nodes. When capacity is tight, data cools faster but never before 30 days.
- **Example:** 1 TB declared and 500 GB used gives free = 0.5 and step +0.10, so cold_after rises 10% a day until free falls toward 20% or it reaches the ceiling.

### 6.6 Owner self-storage (proposal for the storage role)

- Every node is the **home holder** of its own devices' raw data. Nebra holds nebra's data; an owner's node holds their phones' and Flipper's data. It keeps as much as its disk allows, newest first, so the chain can find the data even when the storage network is down.
- **Placement:** the home holder counts as one replica. In class R's RS 4-of-6 it is deterministically given shard 0 (and may also keep the whole object). Shards 1 to 5 go to at least 3 other operators under the usual cap. Because the data is the owner's own, the per-operator cap does not apply to the home holder.
- **Pay:** only for proven retrieval it actually serves, meaning a receipt signed by a requester of a **different** operator. There is no GB-epoch storage pay for the home copy and none for the owner reading its own data. This follows the useful-only rule.
- **Started on nebra:** full LoRa packets are now kept on nebra in `/srv/hone/sensors/raw-private/<window>-lora-pkt.jsonl.gz` (mode 0600, never uploaded in the clear). This is the home-holder copy until class R exists.

### 6.7 LoRa: full packets, payloads included

- Every `rxpk` is stored as received: the base64 PHYPayload (`data`) plus all metadata (`tmst`, `time`, `freq`, `chan`, `rfch`, `stat` including CRC-bad, `modu`, `datr`, `codr`, `rssi`, `lsnr`, `size`). This supports town LoRa maps. The record is `{"s":"hone.raw.pkt@1","t":...,"fam":"lora","rx":{rxpk}}`.
- **Privacy:** the frame header carries the DevAddr and FCnt in the clear. The FRMPayload itself is LoRaWAN-encrypted. So packet records are **owner-only**:
  - on nebra: kept in raw-private;
  - in the storage network: encrypted (class R), like all raw data;
  - never in the public change-based blob, which keeps daily-rotating DevAddr tokens only.
- **Registry:** this needs `hone.raw.pkt@1` in the app agent's registry (honemesh.net/api/raw/schema), plus a privacy line saying private record types are owner-encrypted. That changes the registry hash, so a new anchor version is needed (`schema:hone.raw@2`).

### 6.8 Schema-registry anchor

- The registry at honemesh.net/api/raw/schema has sha256 `e03992fb342e682296241112e1668f04d947a942ce1ffc86eb721c24e98531ca` (checked 2026-11-10 17:30Z). The anchor is the governance `ChainParameterSet` with key `schema:hone.raw@1` and that value (test `schema_anchor_is_governance_only_and_write_once` uses this exact value).
- On the live chain the gate is off, so today the entry would be stored as a plain parameter, with the write-once rule applying only from SCHEMA_ANCHOR_EPOCH.
- **Not submitted:** it needs a governance key signature on the live chain, and Josh's OK.

### 6.9 Gates (all u64::MAX)

| Gate | Effect |
|---|---|
| `INFERRED_LOC_EPOCH` | inferred rows: res 7 or 8, `loc.age_s`, earn only when corroborated |
| `NULL_CELL_EPOCH` | rows with no cell are accepted (they also need `PROVENANCE_EPOCH`), stored, unscored |
| `PROVENANCE_EPOCH` | `prov` is required on every summary and refused before the gate |
| `RELAY_CREDIT_EPOCH` | device-signed relayed rows and the 15% relayer share |
| `SCHEMA_ANCHOR_EPOCH` | `schema:<id>@<ver>` anchors become write-once with a 64-hex value |
| `LIVE_WINDOW_EPOCH` | `live_window_epochs` takes effect |

New view: `GET /api/chain/data-rules`.

### 6.10 What the app needs to change (for the app agent)

1. Send `prov` on every summary and sign it (exact bytes above). Use `dev` = the first 16 hex of the seat key, `cls` = phone, `sensor` = coverage, `schema` = hone.raw.hdr@1, `why` = window or gap:<reason>, and `loc_src` from the area source or none:<reason>.
2. Rows with no area: `cell: ""` with `loc_src: "none:<reason>"`. `SensorSummaryWire.message` currently requires res 7 to 9 and must allow `""`.
3. Inferred rows: always send `loc.age_s`, and coarsen to res 8 or coarser (today an inferred row away from home can be res 9, which would be refused).
4. Flipper relay: the Flipper signs the summary with its enrolled device key. The phone adds `relay: {by: "<account>@<seat pk>", sig}` over the SENSOR_RELAY message.

## 7. Remaining questions for Josh

1. **Relay share:** is 15% right (you suggested 10 to 20%)?
2. **Registry anchor:** may we submit `schema:hone.raw@1` = e03992fb... on the live chain now (signed by a governance key), or wait for `hone.raw@2` with the packet schema?
3. **Cold age:** are the ceiling (1 year), the 20% free target and the ±10% daily step right?
