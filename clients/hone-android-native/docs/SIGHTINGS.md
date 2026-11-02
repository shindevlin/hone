# Sightings and coverage (Android)

Phone side of the HONE sightings/coverage spec (2026-11-02 draft):
§1a seeded rotating IDs, §1b keyed hashes for non-HONE devices, §2a phone as a
sensor, and the phone side of §2c (private layer). Code: `app/src/main/java/network/hone/app/sightings/`.

Nothing here is submitted to the chain. The chain's summary-carrying commit and
usefulness scoring do not exist yet. Summaries are stored and queued locally.

## 1. Rotating ID (§1a)

- `id_seed`: 32 random bytes, created on first run at `files/hone-node/id_seed`
  (app-private, mode 600, next to `witness.key`). Never logged, never sent.
  **TODO(backup):** the spec backs the seed up to the owner's wallet. The app has no
  owner backup path yet, so there is no export of any kind.
- `window = floor(unix_utc / 900)` (UTC quarter-hours, the same instant on every device).
- `rid = HMAC-SHA256(id_seed, "hone-rid" || window as 8-byte big-endian)[0..16]`.

### Advert format

Legacy, non-connectable, non-scannable BLE advert, ~1 s interval (`INTERVAL_HIGH`),
`TX_POWER_MEDIUM`, no flags, no name, no TX power field. One AD structure:

| bytes | value |
|---|---|
| 0 | `0x17` length |
| 1 | `0xFF` manufacturer specific data |
| 2-3 | `0xFFFF` company id (little-endian `FF FF`) |
| 4 | `'h'` (0x68) |
| 5 | `'R'` (0x52) |
| 6 | version `0x01` |
| 7 | `win8` = `window & 0xFF` (lets the owner match a sighting from an observer whose clock is slightly off) |
| 8-23 | `rid` (16 bytes) |

The lower-case `'h'` keeps it apart from the Flipper's `'H'` frames (`'HA'` announce, `'HC'` commit).
Observers filter on manufacturer data `FF FF 68 52` with mask `FF FF`.

Not yet in the advert: the owner key that observers would encrypt the cell to (spec §1a,
the Find My model). 32 bytes do not fit next to the rid in a legacy advert. Options:
a short key id plus key lookup, or an extended advert.

### Address rotation

The rid and the BLE advertising address must change at the same instant.

- Android apps cannot set the advertising address. `AdvertisingSetParameters.setOwnAddressType`
  is a system API and is not in the public SDK (compile error on compileSdk 36).
- **What works:** each new `AdvertisingSet` gets a fresh random address from the stack.
  At every quarter-hour the app stops the old set and starts a new one with the new rid.
  An exact alarm (`SCHEDULE_EXACT_ALARM`, `setExactAndAllowWhileIdle`) wakes the CPU at
  the boundary + 250 ms. A 5 s loop re-asserts the advert if Bluetooth was off.
  Without the exact-alarm permission it falls back to an inexact alarm.
- **Verified 2026-11-02** on the moto g power 2025 (Android 16, API 36), with grouchly's
  BlueZ adapter as the second scanner (`/tmp/andr_sight_scan.py`):

  ```
  60:37:51:B0:47:6F (random) rid=352d4759.. win8=225  first 16:25:08Z last 16:30:01Z
  45:FA:B2:B1:B8:FB (random) rid=104d7782.. win8=226  first 16:30:02Z
  ```
  Phone log: `window 1992929 closed` and `rid advert started w=1992930` at 16:30:00.28Z.
  Each address carried exactly one rid. Address and rid changed together.
- Caveat: the stack may also rotate the address mid-window on its own private-address
  timer. That is harmless: it never carries a rid across a boundary.

## 2. Observing (§1b, §2a)

All of it runs in `NodeService` (foreground service, started by the Sensors switch and,
from now on, after boot or app update).

- `K_day`: 32 random bytes per UTC day, RAM only, replaced at UTC midnight. Never persisted.
- `oid = HMAC-SHA256(K_day, mac)[0..8]` for Wi-Fi BSSIDs and BLE addresses.
- **Wi-Fi:** per AP `(oid, rssi max, channel, band)`; bands 2.4 / 5 / 6 GHz from the
  frequency. 6 GHz is reported only when `WifiManager.is6GHzBandSupported()` (the moto: no).
  Scan requests:
  - throttling disabled (Developer options, `isScanThrottleEnabled() == false`): every 2 min;
  - throttled, Sensors screen open: every 30 s (foreground allowance, 4 per 2 min);
  - throttled, background: every 30 min (background allowance).
  A refused `startScan()` is counted, not an error. Every 15 s the app also reads any
  results that other apps' or the system's scans left behind. Only results less than
  2 min old are used, and each scan is used once.
- **BLE crowd:** unfiltered scan, low latency, 8 s every 60 s. Distinct advertiser oids per
  window. Android withholds unfiltered results while the screen is off, so this is a
  screen-on or docked metric.
- **HONE rids:** hardware-filtered scan (`hR`), low power, always on. It works with the
  screen off. Each sighting `(rid, window, cell9, cell11, rssi, ts)` goes to `rid_sighting`.
- **Flipper frames:** `HoneAnnounceScanner` calls `SightingsEngine.onFlipperFrame()` for
  every verified announce and every commit it hears. The frame is stamped with this phone's
  cells in `flipper_frame`. The relay itself is unchanged.
- **Mobile APs:** the same oid seen in a second res-9 cell within the UTC day sets
  "mobile AP present in cell X" for the current window (counted in the summary). It uses
  K_day, so it is RAM only, and the oid never leaves the device.

## 3. Location stamping

- `LocationManager` `fused` provider (API 31+, no Play services dependency), balanced
  power, 60 s / 10 m. Plus `PASSIVE` to reuse any GPS fix another component already pays
  for. A fix older than 5 min is ignored, and readings are then kept unlocated.
- Cells: H3 via `com.uber:h3:4.5.1`. Its `android-arm64` and `android-arm` JNI builds are
  extracted from the jar into a generated `jniLibs` dir by `app/build.gradle.kts` (AGP does
  not package `.so` files from jar resources), and loaded with `H3Core.newSystemInstance()`.
  - Public: res 9 (~174 m edge).
  - Private: res 11 (~25 m edge, matching phone GPS error).
- Permissions: fine + coarse location, background location (optional), and the
  `location` foreground-service type next to `specialUse`. If the location type is refused,
  the service runs without it. If location is denied, everything still runs, readings are
  stored as `"unlocated"`, and nothing is queued.

## 4. Local store (`databases/sightings.db`)

Public layer:

- `public_summary(cell, window, json, queued, submitted_at)`: one row per (res-9 cell, window).
  `queued=1` = waiting for the chain's summary commit. Unlocated rows have `queued=0`.

Private layer (never leaves the phone, 30-day retention):

- `rid_sighting(rid, window, cell9, cellf, rssi, ts)`: other HONE devices heard.
- `flipper_frame(dev, kind, window, cell9, cellf, acc, rssi, ts)`: Flipper frames with
  the phone's cells. No raw lat/lon is stored.
- `owned_device(label, seed)`: seeds of the owner's other devices (Find my things input).
- `own_router(tag, label, registered_at)`, `own_router_cov(tag, window, cellf, rssi)`:
  the owner's routers. `tag = HMAC-SHA256(K_priv, bssid)[0..8]`, where `K_priv` is a
  persistent private-layer key in `files/hone-node/priv_layer_key`. Registration is by
  proof of control: "Add my router" shows a code, and the router is registered when the
  phone sees an SSID containing it within 10 min. The SSID and BSSID are not stored.

### Public summary JSON (v1)

```json
{
  "v": 1,
  "cell": "896d75d960bffff",     // H3 res 9
  "res": 9,
  "window": 1992929,             // floor(unix_utc / 900)
  "wifi": {
    "ap_count": 27,              // distinct APs (oids) heard in this cell+window
    "by_band": {"2g": 11, "5g": 16, "6g": 0},
    "bands_supported": ["2g", "5g"],   // so "6g: 0" can be told apart from "can't hear 6 GHz"
    "rssi_hist": [0, 2, 7, 8, 3, 0, 7],// per-AP max rssi: <-90, -90..-81, -80..-71, -70..-61, -60..-51, -50..-41, >=-40 dBm
    "rssi_p50": -64,
    "rssi_max": -17,
    "channels": {"2g:6": 1, "5g:44": 15},  // AP count per band:channel (congestion)
    "scans": 2                   // Wi-Fi scan results ingested
  },
  "ble": {"crowd": 20, "scan_s": 48},  // distinct BLE advertisers; seconds of crowd scanning
  "hone": {"devices": 1},        // distinct rids + Flipper ids heard
  "mobile_ap": 0,                // APs flagged as mobile in this cell+window
  "loc_acc_m": 100               // best fix accuracy in the window (m)
}
```

There are no identities, MACs, BSSIDs, SSIDs, oids or rids in it. Signing, the batch hash
and the observer identity are added at submission time by the (future) chain commit. The
`(cell, window, metric, value)` tuples of spec §3 map one-to-one onto these fields.

Known gap: a window cut short by an app restart (e.g. an update) loses the part before the
restart. The summary written at the boundary covers only the rest of the window.

## 5. Find my things

Sensors → Coverage → **Find my things**. It recomputes the rids of every owned seed
(this phone's, plus any added by hand) for the last 7 days (96 per device per day) and
looks them up through a `SightingIndex`. Today the only index is the local
`rid_sighting` table. The same query (a rid list in, sightings out) is meant for a chain
indexer later (`TODO(indexer)` in `FindMy.kt`). A phone cannot hear its own advert, so
this phone shows sightings only once other observers report them.

Verified on 2026-11-02 with a test advertiser on grouchly (`/tmp/andr_sight_adv2.py`,
fixed test seed `sha256("hone-find-my-test")`). Python computed the rid, the phone sighted
it, and Find my things matched it from the seed (Python and Kotlin give the same rid):
`grouchly-test · cell 8b6d75d96098fff · -50 dBm · window 1992930`.

## 6. Battery (estimate, moto g power 2025, 5000 mAh)

Measured duty cycles (from `dumpsys bluetooth_manager`) combined with typical radio
currents:

| piece | duty | est. average |
|---|---|---|
| rid advert, legacy, 1 s, non-connectable | ~3 x 0.4 ms TX per second | ~0.03 mA |
| rid scan, LOW_POWER, HW filter | ~10 % RX, CPU asleep | ~0.4 mA |
| BLE crowd, LOW_LATENCY 8 s / 60 s | 13 % RX + CPU while results arrive | ~1 mA screen on, ~0 screen off (results withheld) |
| Wi-Fi, throttled background (1 / 30 min) + passive reads | | <0.1 mA |
| Wi-Fi, throttling off (1 / 2 min) | ~3 s at ~150 mA per scan | ~3-4 mA |
| fused location, balanced, 60 s | network-based | ~0.3-0.5 mA |
| boundary alarm (96 / day), 5 s tick | | negligible |

That is about **0.8 mA (~0.4 %/day)** with the screen off and Wi-Fi throttled, and
**~2 mA screen on**. Add **~4 mA (~2 %/day)** if Wi-Fi throttling is turned off. For
comparison, the existing pieces already cost more: the Flipper `'HA'/'HC'` scan runs
continuously in BALANCED mode, and `SensorCapture` requests GPS every 10 s.
