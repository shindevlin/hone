# Unified observations, travel groups, capability sourcing, firmware install (vc29 / 1.17.0)

Spec: claude/hone-unified-observations-and-late-submission-2026-11-14.md (Josh, 14 Nov 13:40 + add-ons 13:52-13:54).

## Observation types
Every hone.raw reading carries `obs_type` in provenance (kf@3, ev@3, obs@2; kf@2/ev@2/obs@1 still read).
Types: pressure, temp_humidity, cell_coverage, wifi_env, ble_density, subghz, lora, aircraft, birds_noise,
outage_signal, foot_traffic, presence. Registry: services/hone-raw/hone-raw-registry.json v3
(sha256 9728a87296dd57cc58c8423ac9d5c6bd1f269892e8bd1df200d5c0c38d2425da).

Pressure and cell coverage are sampled every 15 min and on change (cell deadband 6 dB). Cell keys are
`cell:<token(op, rat, cell id)>`, so the raw cell id never leaves the phone; values are RSRP / SS-RSRP /
dBm, with rat, op, rsrq, sinr and arfcn attributes.

## Light GPS track
Kept on the phone only (hone-node/track.jsonl, 30 days). A fix is kept when it is 25 m from the last one,
or 60 s after it while moving (speed > 0.8 m/s), or 10 min after it while still. The track is used only to
place the owner's own readings: interpolated within 15 min, nearest fix within 5 min, otherwise unknown.

## Travel groups
A travel group is the owner's phone plus the owner's Flippers heard in the last 2 h (the Flipper with both
phones by default). When a Flipper reading has no live link, it gets the phone's track position at the same
time, and only with proof:
- co-location score = 0.6 Wi-Fi Jaccard + 0.2 BLE density similarity + 0.2 sub-GHz peaks within 30 s;
  match at >= 0.35, with Wi-Fi Jaccard >= 0.2 when both sides saw Wi-Fi;
- home anchor: the Flipper's home environment must not match the reading (Wi-Fi Jaccard < 0.3), so a
  Flipper left at home cannot borrow the phone's walk;
- match: tier B, `loc.src=travel_group`, with the proof hash (sha256 of the canonical evidence) in provenance;
- no match: tier C (inferred).
Ingest: `GET/POST /api/raw/colocate`, and services/hone-raw/travel_retro.py for stored frames. Today the
Flipper commit frames carry neither the reading time nor an environment digest, so a true retroactive
proof needs firmware support (signed t_first/t_last plus env digests).

## Capability-aware sourcing
Devices announce their capabilities: subghz, nfc, rfid125, ir, ble, wifi_board, gps, baro, cell, mic, plus
hygro, thermo and light on phones. Inside a travel group, each obs_type takes its best source:

| obs_type | first choice | fallback |
|---|---|---|
| wifi_env | Flipper with Wi-Fi board (precision high) | phone scans, throttle-aware: ~4 per 2 min in the foreground (normal) |
| foot_traffic | Wi-Fi board (high) | BLE-density estimate `crowd.ble_estimate` (low) |
| location | phone GPS | none |
| subghz | Flipper | none |
| pressure, cell_coverage | phone | none |

Readings of one obs_type from co-located devices in the same window are never counted twice: the ingest's
products table merges them per owner (readings = owners, devices = corroborating). Provenance records
`dev` and `cap`. One account can have several Flippers; the Flipper screen lists each one's capabilities
and says when the Wi-Fi board is missing. Event mode (Settings) scans as often as Android allows for 4 h and
reminds the owner to attach the board.

## Firmware install (phone -> Flipper)
Code: app/src/main/java/network/hone/app/fw/.
- Link: USB-C OTG as CDC-ACM (VID 0x0483, PID 0x5740) is used when plugged in; the phone sends
  `start_rpc_session\r` to the CLI, then speaks RPC. Otherwise the bonded Flipper over BLE (serial service
  8fe5b3d5-...0000, RX ...62fe0000 write, TX ...61fe0000 indicate, flow control ...63fe0000 4-byte credit).
  A plugged-in Flipper opens HONE (USB_DEVICE_ATTACHED); the user grants USB access with no pairing.
- Manifest: https://honemesh.net/app/firmware/manifest.json, `hone-fw-manifest@1`
  `{format, key_id, body, sig}`. The ed25519 signature covers `"hone-fw-manifest@1\n" + body`, byte-exact.
  body = `{version, channel, target: 7, min_app, files: [{name, size, sha256, url}]}`. The app pins public
  keys only (FirmwarePackage.RELEASE_KEYS) and fails closed while that list is empty. The private release
  key stays on grouchly.
- Never brick:
  - refuse an unsigned, tampered or wrong-key manifest, a target other than f7, or a bad name or hash;
  - every downloaded file must match its signed size and sha256;
  - the Flipper must be at >= 30 % battery or charging, with room on the SD card;
  - copy to /ext/update/hone-<version>/; files already there with the right md5 are skipped, so a retry resumes;
  - check every md5 on the Flipper;
  - SystemUpdateRequest must return OK before the reboot (mode UPDATE). The Flipper's own updater then
    flashes from the SD card;
  - reconnect and confirm `firmware.version`.
- Stock-firmware Flippers: HONE firmware is an opt-in checkbox. "Switch back to official firmware" uses
  https://honemesh.net/app/firmware/official.json: the official package, re-signed after its hashes are
  checked, with the same checks. qFlipper stays a fallback.
- Heltec / ESP32 (design note only): the same signed manifest with target "esp32-s3", flashed over USB
  using the ROM bootloader protocol (esptool-style SLIP) from the phone, or OTA over Wi-Fi from a
  HONE-signed URL. Dual OTA partitions plus rollback give the never-brick guarantee. Not built.

## Languages
New strings live in Android resources: values/strings_fw.xml (EN) and values-es/strings_fw.xml (ES). The app
follows the phone language, and Settings -> Language overrides it per app (Android 13+). Older UI strings
are still hard-coded English.
