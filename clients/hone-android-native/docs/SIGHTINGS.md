# Sightings, coverage and location sharing (Android)

Phone side of the HONE sightings/coverage spec (2026-11-02 draft):
§1a seeded rotating IDs, §1b keyed hashes for non-HONE devices, §2a phone as a sensor,
the phone side of §2c (private layer), plus owner-encrypted sightings and location sharing.
Code: `app/src/main/java/network/hone/app/sightings/`. Reference implementation and host
CLI: `tools/hone_locshare.py` (unit test `IdKeysTest` pins Kotlin to its vectors).

No new chain entry type. Public summaries are stored and queued locally for the future
summary commit. Grants use the existing Memory service (MemorySet / MemoryDelete).

## 1. Keys

### 1.1 Where the root comes from

The wallet has six SLIP-10 ed25519 role keys (`rust/hone-node/src/wallet.rs`): owner,
active, posting, memo, hide, seek. Nodes hold posting keys, so posting must not be able to
track anyone. The device root is the **hide** key ("encrypt private content for your own
eyes only"); sharing uses the **memo** key ("encrypted messages / selective disclosure").

```
id_seed(device) = HKDF-SHA256(ikm = hide_private, salt = "", info = "hone-id-seed" || device_pubkey)
period          = UTC calendar month of the window start: year*12 + (month-1)
share_k(period) = HKDF(id_seed, info = "hone-share" || period_be32)
sk_w            = HKDF(share_k, info = "hone-pkw" || window_be64)       X25519 scalar
pk_w            = X25519(sk_w, 9)                                        in the advert
lid             = SHA-256(pk_w)[0..16]                                   lookup id
tag             = HKDF(share_k, info = "hone-tag" || window_be64)[0..2]  joins legacy fragments
window          = floor(unix_utc / 900)
```

- The seed is not stored or backed up: the wallet mnemonic is the backup. Lose a phone,
  lose nothing; any device holding the hide key recomputes every owned device's keys.
- `device_pubkey` is the device's ed25519 key: for a phone its witness seat key
  (`files/hone-node/witness.key`, seat `josh@<pubkey>`), for a Flipper its device key.

### 1.2 What is on the phone

`RoleKeys.kt` imports hide / memo / (optional) active private keys (64 hex) in Find my
things > Wallet keys. Each is checked against the account's on-chain role key when one is
registered, then stored wrapped by a non-exportable AndroidKeyStore AES-GCM key
(`files/hone-node/role_<role>.wrapped`). Never logged or exported.

State on 2026-11-02 (`/api/account/<a>`):

| account | on-chain keys | wallet file |
|---|---|---|
| josh | posting, active | not on the phone; no hide/memo registered |
| shindevlin | posting, active | not found on grouchly (beastly?); no hide/memo registered |
| natoshisakamoto | all six | grouchly `~/.hone/natoshisakamoto.wallet.key` |

Until a hide key is imported the phone runs on a **provisional** random seed (the old
`id_seed` file) and says so on the Sensors screen. Importing hide switches to the derived
seed (one-time key change) and deletes `id_seed`.

### 1.3 Provisioning other devices (Flipper, later)

At claim time the phone derives the device's seed and seals it to the device:
`blob = ECIES(X25519(device ed25519 pubkey), info "hone-seed-provision-v1", id_seed)`
(`FindMy.provisionBlob`, or `hone_locshare.py device-seed-provision`). The device opens it
with `x_sk = SHA-512(ed25519 seed)[0..32]` (standard ed25519 -> X25519 conversion), stores
`id_seed`, and from then on derives `share_k`, `sk_w/pk_w`, `tag` exactly as above. The
firmware side is a separate task; the interface is the blob above, delivered over the
claim channel.

## 2. Advert and owner-encrypted sightings (§1a, Find My style)

### 2.1 Advert format (v2)

Manufacturer data, company `0xFFFF`, non-connectable, non-scannable:

```
'h' 'K' | ver=0x02 | win8 | frag | tag[2] | payload
frag = idx<<4 | total
extended advert (BLE 5): total=1, payload = pk_w[32]                    39 bytes
legacy fallback:        two adverts, total=2, pk_w[0..16] / pk_w[16..32]  23 bytes each
```

- Extended advertising is used when `isLeExtendedAdvertisingSupported()` and the max data
  length allows; otherwise two legacy advertising sets. Each set has its own random
  address, so observers join fragments by `(win8, tag)`, not by address.
- Observers scan with `setLegacy(false)` (when supported) so they hear both kinds.
  A legacy-only observer (old phone) does not hear extended adverts.
- `'h' 'R'` v1 (plain 16-byte rid) is retired.

### 2.2 Address rotation

Apps cannot set the advertising address (`setOwnAddressType` is a system API). Every new
`AdvertisingSet` gets a fresh random address, so at each UTC quarter-hour (exact alarm,
boundary + 250 ms) all sets are stopped and new ones started with the new `pk_w`.
Verified for v1 on 2026-11-02 with grouchly's BlueZ adapter as a second scanner: three
boundaries (16:30, 16:45, 17:00 UTC), each address carried exactly one key.

### 2.3 Observer side

On a complete `pk_w` the observer seals one record and stores it under the lid:

```
record = JSON {w, w8, t, r (rssi), c (res-11 cell), cp (accuracy-matched cell), a (accuracy m), s (source)}
blob   = epk(32) || nonce(12) || AES-256-GCM(key, nonce, record, aad = lid)
key    = HKDF(X25519(esk, pk_w), salt = epk || pk_w, info = "hone-sighting-v1")
```

Stored in `enc_sighting(lid, window, blob, ts)`. A new record is written per lid when the
cell changes or RSSI improves by 3 dB. The observer keeps no plaintext location of anyone
else's device; to anyone without `sk_w` the lid is random and unlinkable across windows.

### 2.4 Owner side (Find my things)

For each owned device (this phone, devices added by public key) and each shared device,
recompute the lids of the last 7 days (96 per device per day), query a `SightingIndex`
(today the local store; later an indexer, same lids-in/blobs-out query) and open the
blobs with `sk_w`. Host equivalent: `hone_locshare.py find --wallet W --device PUB --db sightings.db`.

## 3. Location sharing

- **Grant** = the owner's `share_k` for the current and next month, sealed to the
  grantee's on-chain **memo** pubkey (ed25519 -> X25519), stored in the grantor's Memory:

  ```
  key   = "ls1-" + SHA-256("hone-locshare|" + grantee + "|" + device_pub_hex)[0..16].hex()
  value = base64(ECIES(memo_x25519(grantee), info "hone-locshare-v1",
                       JSON{v:1, from, to, device, label, keys:{period: share_k_hex}}, aad = grantor))
  ```
  About 500 bytes (limit 4096). The key name does not show who shares with whom.
- **Grantee:** lists `ls1-*` entries of known grantors (default shindevlin, natoshisakamoto;
  `files/hone-node/share_from`), trial-decrypts with its memo key, keeps grants whose
  `to`, `from` and key name match, and shows those devices under "Shared with me" with
  their sightings next to its own. Refreshed every 30 min and on demand.
- **Revoke** = MemoryDelete; the grantee's next refresh drops it. **Limit, stated plainly:**
  a grantee who already fetched a month key keeps it, so access ends at the end of the
  granted months, not instantly. The grantor re-issues next month's key monthly (or not).
  Shorter periods narrow that window; the format allows it.
- **Grantor tools:**
  - accounts whose wallets live on node hosts (natoshisakamoto on grouchly, shindevlin):
    `tools/hone_locshare.py grant|revoke|list|inbox` on that host. Keys are read from the
    wallet file into memory and never printed or copied off the host.
  - on the phone: Find my things > Share / Revoke, signed with an imported active key.
- **Signing:** MemorySet/MemoryDelete need the account's active key (posting fallback).
  The tools sign `{"account","chain_id","key","nonce","type":"MEMORY_SET","value"}`
  (sorted, compact; same as hone-cli's `sign_entry`).

**Chain signing (fixed 2026-11-02, v2.1-live-identity dcfd28b0):** `canonical_signing_message`
had no arm for MemorySet/MemoryDelete, so they fell to the generic arm (whole entry incl. the
server-set `epoch` and the `signature` field); no client could sign it, and a test grant from
natoshisakamoto was accepted by the HTTP handler and never applied. From entry epoch
`MEMORY_SIGNING_ACTIVATION_EPOCH` = **349,100** the node signs exactly what these tools sign:

```
{"account":A,"chain_id":C,"key":K,"nonce":N,"type":"MEMORY_SET","value":V}
{"account":A,"chain_id":C,"key":K,"nonce":N,"type":"MEMORY_DELETE"}
```

Sorted keys, compact separators. Note: the node build has serde_json `preserve_order` ON, so
the Rust arm writes the keys in sorted order explicitly (json! does not sort). Values must be
ASCII (base64 is): Python `json.dumps` escapes non-ASCII by default and serde_json does not.
Before 349,100 the old message applies and a client-signed memory entry is now refused at
submission (signature precheck) instead of being "accepted" and dropped.

## 4. Observing (§1b, §2a)

- `K_day`: 32 random bytes per UTC day, RAM only. `oid = HMAC-SHA256(K_day, mac)[0..8]`.
- **Wi-Fi:** per AP `(oid, rssi max, channel, band)`, bands 2.4 / 5 / 6 GHz (6 GHz only when
  `is6GHzBandSupported()`). Scan cadence: every 2 min with throttling disabled, 30 s with
  the Sensors screen open, 30 min in the background; results of other apps' scans are read
  every 15 s.
- **BLE crowd** (filter v1): Android drops unfiltered scan results with the screen off, so
  the crowd scan (low latency, 8 s per 60 s) uses broad hardware filters, the same with
  the screen on so counts stay comparable:
  - manufacturer IDs: Apple 0x004C, Microsoft 0x0006, Samsung 0x0075, Google 0x00E0,
    Garmin 0x0087, Huami 0x0157, Xiaomi 0x038F, Amazon 0x0171;
  - service UUID / service data: 0xFE2C (Fast Pair), 0xFEAA (Eddystone, Find Hub),
    0xFD6F (Exposure Notification), 0xFD5A (SmartTag), 0xFEED/0xFEEC (Tile), 0x180D
    (heart rate), 0xFE07 (Sonos), 0xFE9F (Google), 0xFD44 (Find My accessories).
- `BLUETOOTH_SCAN` no longer declares `neverForLocation`: BLE results are now location
  stamped, and with the flag Android also strips beacon results. Fine location is already
  granted, so the Flipper relay is unaffected.
- **Flipper frames** are stamped with the phone's cells (`flipper_frame`); relay unchanged.
- **Mobile APs:** the same oid in two cells (res 9 or finer) within the UTC day.

## 5. Location and accuracy

- **Source:** Play services `FusedLocationProviderClient`, `PRIORITY_HIGH_ACCURACY`, 60 s
  (20 s min) when GMS is present; otherwise `LocationManager` fused/network. Plus PASSIVE.
  Each reading carries its accuracy radius and source (`fused-gms`, `fused`, `gps`, `wifi-fp`).
- **Every reading is kept.** It goes to the finest H3 resolution whose average edge is at
  least its accuracy radius: res 11 (29 m) for good fixes, res 10 (76 m), res 9 (201 m),
  res 8 (531 m), down to res 5 (9.9 km). Worse than that: unlocated.
- **Wi-Fi fingerprints (private):** a scan taken with a fix of <= 20 m trains the res-11
  cell's `(AP tag -> mean RSSI)` fingerprint (`fingerprint` table; AP tag =
  HMAC(K_priv, bssid), persistent private key). A scan without such a fix is matched
  (>= 3 shared APs, mean |dRSSI| plus a missing-AP penalty, threshold 10 dB); a match
  becomes a 35 m `wifi-fp` fix. Only GPS-grade fixes are ground truth. Never exported.

## 6. Local store (`databases/sightings.db`, schema v2)

Public layer: `public_summary(cell, window, json, queued, submitted_at)`.
Private layer (30-day retention except fingerprints): `enc_sighting`, `flipper_frame`,
`owned_device(label, pub)`, `shared_device(grantor, device, label, keys)`, `own_router`,
`own_router_cov`, `fingerprint`. The v1 -> v2 upgrade drops `rid_sighting` (plaintext
cells) and `owned_device` (raw seeds, incl. the grouchly test seed).

### Public summary JSON (v2)

One row per (cell, window), the cell at the resolution its readings supported:

```json
{
  "v": 2, "cell": "8a6d75d96097fff", "res": 10, "window": 1992931,
  "loc": {"samples": 14, "acc_m_min": 14, "acc_m_p50": 40, "weight": 0.0127,
          "src": {"fused-gms": 12, "wifi-fp": 2}},
  "wifi": {"ap_count": 20, "by_band": {"2g": 6, "5g": 14, "6g": 0}, "bands_supported": ["2g", "5g"],
           "rssi_hist": [0, 2, 5, 5, 3, 0, 5], "rssi_p50": -65, "rssi_max": -17,
           "channels": {"2g:6": 1, "5g:44": 13}, "scans": 1},
  "ble": {"crowd": 36, "scan_s": 120, "filter": "v1"},
  "hone": {"devices": 2},
  "mobile_ap": 0
}
```

- `loc.weight = sum(1 / acc_m^2)` over the readings: the aggregator combines rows of the
  same and of parent/child cells with inverse-variance weights, so the estimate tightens
  as better readings arrive and a poor reading still contributes, just less.
- No identities: no MAC, BSSID, SSID, oid, lid or rid. Signing and the observer identity
  are added by the future chain commit.

## 7. Battery (estimate, 5000 mAh)

| piece | est. average |
|---|---|
| id advert (1 extended or 2 legacy sets, 1 s) | ~0.03-0.06 mA |
| id scan, LOW_POWER, HW filter | ~0.4 mA |
| BLE crowd, filtered, 8 s / 60 s (now also screen off) | ~1 mA |
| Wi-Fi throttled background + passive reads | <0.1 mA |
| Play services HIGH_ACCURACY, 60 s | ~2 mA indoors (no GPS lock), up to ~8-15 mA outdoors with GPS |
| sealing sightings, fingerprints | negligible |

About 3.5 mA (~1.7 %/day) indoors screen off, more outdoors while GPS is used.
Switching to BALANCED priority when on battery and moving slowly is the obvious knob.

## Standing hardware grant (no hide key on phones)

The owner's hide key stays on the wallet host. `tools/hone_findgrant.py` checks the phone's
Android Key Attestation (Google root, not revoked, TEE/StrongBox, PURPOSE_WRAP_KEY, challenge
`SHA-256("hone-find-v1|<account>|<device_pub>")`, package + signing-cert digest), signs
FIND_GRANT with the hide key, and wraps per device `PRK = HKDF-Extract(salt="", id_seed_g)`
to the phone's attested RSA-2048 wrapping key (ImportWrappedKey, RSA-OAEP SHA-256/MGF1-SHA-1,
AES-256-GCM, AAD = KeyDescription) as a non-exportable HMAC-SHA256 key.

`id_seed_g = HKDF(hide, "hone-id-seed" || device_pub || be32(g))`; g = 0 omits the suffix
(today's seed). `share_k(p) = HMAC(PRK, "hone-share" || be32(p) || 0x01)`, computed in the Keystore.

| Where | What |
|---|---|
| Hardware only (TEE on the moto g power 2025; StrongBox when present) | wrapping key, every PRK / id_seed |
| Never on the phone | hide key, owner key; active key only in the wallet (biometric per signature) |
| App RAM, briefly, zeroed after use, never stored or logged | `share_k` of the month searched or advertised; per-window `sk_w` (X25519 and AES-GCM need the raw scalar) |
| Public | `pk_w`, `lid`, `tag`, the grant entry |

A Keystore HMAC returns its output to the app, so `share_k` cannot be chained further inside
the hardware; everything above it stays there.
