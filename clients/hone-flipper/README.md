# BTCPC — Flipper Zero App

Chain identity node for the BTCPC network. The Flipper generates an ed25519
keypair on first run, signs all outgoing sensor data, and relays it to a paired
phone over BLE. The phone registers the Flipper's public key on-chain via a
`DeviceRegister` entry during the initial pairing flow.

## Prerequisites

Install ufbt (Flipper Zero build tool):

```sh
pip install ufbt
```

ufbt downloads the correct Flipper firmware SDK automatically on first build.

## Build

```sh
cd clients/btcpc-flipper
ufbt
```

The compiled `.fap` file lands in `.ufbt/build/btcpc.fap`.

## Install

With the Flipper connected over USB:

```sh
ufbt launch
```

This installs the `.fap` and launches the app immediately.

To install without launching:

```sh
ufbt install
```

## First run

1. The app checks `/ext/apps_data/btcpc/identity.key` on the microSD card.
2. If the file is missing, a new ed25519 keypair is generated from the
   Flipper's hardware TRNG (STM32WB55 RNG peripheral).
3. The secret key (64 bytes) is written to `identity.key`.
4. The public key (32 bytes) is written to `identity.pub`.
5. The **Identity / Key** screen displays the public key as a 64-character hex
   string. Note this down or photograph it — you need it for registration.

Key files on the Flipper microSD:

```
/ext/apps_data/btcpc/identity.key   — 64-byte ed25519 secret key (keep private)
/ext/apps_data/btcpc/identity.pub   — 32-byte ed25519 public key
```

## Pairing with the phone

1. Open the BTCPC mobile app on your phone.
2. Navigate to **Devices → Pair Flipper**.
3. The phone app scans for the Flipper's BLE advertisement (device name `BTCPC`).
4. On pairing, the phone submits a `DeviceRegister` chain entry containing the
   Flipper's public key (read from `identity.pub` or typed in manually).
5. Once the entry is sealed, the Flipper's signatures are accepted by the network.

Auto-reconnect: after the initial pairing, the Flipper stores the paired
device's BLE address in `/ext/apps_data/btcpc/paired.addr` and reconnects
automatically on subsequent app launches.

## Key registration (manual fallback)

If the mobile app is unavailable, copy the 64-character hex public key shown on
the Identity screen and submit a `DeviceRegister` entry via the HTTP API:

```sh
curl -X POST http://localhost:4242/api/entries \
  -H 'Content-Type: application/json' \
  -d '{
    "type": "DeviceRegister",
    "pubkey": "<64-hex-public-key>",
    "device_type": "flipper-zero"
  }'
```

## Data flow

```
Flipper sensors → sign with ed25519 sk → BLE frame → phone app → chain entry
                                         ↑
                    phone → clock sync / GPS / entry hashes → Flipper
```

### Messages Flipper sends (signed)

| Type              | Content                                    |
|-------------------|--------------------------------------------|
| `SUBGHZ_OBS`      | Frequency (Hz), RSSI (dBm), modulation     |
| `RFID_SCAN`       | Protocol, card ID bytes                    |
| `NFC_SCAN`        | Tech, UID, ATQA, SAK                       |
| `IBUTTON`         | 64-bit ROM code, family byte               |
| `HEARTBEAT`       | Battery %, uptime (s), firmware version    |
| `IR_CAPTURE`      | (TODO) raw IR pulse data                   |

### Messages phone sends (no signature required)

| Type          | Content                                  |
|---------------|------------------------------------------|
| `ENTRY_HASH`  | 32-byte SHA-256 to rebroadcast via Sub-GHz |
| `CLOCK_SYNC`  | Unix timestamp in milliseconds           |
| `GPS`         | Lat/lon/alt/accuracy from phone GPS      |

## Crypto

Ed25519 via TweetNaCl (public domain, Daniel J. Bernstein et al.).
The `randombytes()` function is backed by `furi_hal_random_get()` which reads
the STM32WB55 hardware TRNG.

Stack usage: ed25519 sign requires approximately 1.5–2 KB of stack for the
SHA-512 computation and field arithmetic. The app is configured with an 8 KB
stack to provide headroom.

## Project layout

```
clients/btcpc-flipper/
  application.fam           — ufbt app manifest
  btcpc.c                   — app entry point, identity management
  btcpc.h                   — types, constants, shared state
  crypto/
    ed25519.c/.h             — TweetNaCl wrapper + randombytes() glue
    tweetnacl.c/.h           — TweetNaCl (public domain, ed25519 subset)
  protocol/
    btcpc_protocol.h         — BLE wire format definitions
    btcpc_protocol.c         — serialise / sign / parse messages
  scenes/
    btcpc_scene_main.c       — main menu
    btcpc_scene_identity.c   — public key display
    btcpc_scene_ble.c        — BLE pairing status
  legacy/
    btcpc_wallet.c           — archived hardware wallet prototype (v0.2)
```

## Phase 9 TODOs

- Wire BLE NUS peripheral (see `scenes/btcpc_scene_ble.c` TODOs)
- Sub-GHz scan loop posting `SUBGHZ_OBS` frames
- RFID/NFC scan callbacks
- iButton scan callbacks
- IR capture and rebroadcast
- Sub-GHz rebroadcast of `ENTRY_HASH` frames received from phone
- Auto-reconnect on boot using stored paired address
