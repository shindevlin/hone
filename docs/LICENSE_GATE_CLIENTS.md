# License gate across clients

The Rust binaries enforce the UFL 3.4 Decentralized acceptance step through
`rust/hone-license` (see [LICENSE_ACCEPTANCE.md](LICENSE_ACCEPTANCE.md)). Clients
that do not run a Rust `main()` have their own first-run step, built to the
same rules:

- Shown before first use, before any node, miner, or wallet code runs.
- Names the license: UFL 3.4, Operational Scope: Decentralized.
- Shows the dispute process (exclusive online ICC arbitration seated in
  California, individual claims only), the US$1-in-value mutual liability
  limit (money or HONE at the payer's choice), and the own-risk terms for
  forks and bypassers, with a way to read the full text.
- An affirmative action: "I accept" (never pre-selected or defaulted).
  Declining closes the app or stops the install without doing anything else.
- Records locally only: license, version, scope, time, app name. Nothing over
  the network or on chain.
- Asks again when the license version or scope changes.

The summary text is `hone_license::summary()`. The Android app and the
installers carry a copy; `rust/hone-license/tests/surfaces.rs` fails if any copy
drifts, and checks that the records they write are accepted by the Rust gate.

## Status

| Surface | Step | Record | Verified |
|---|---|---|---|
| Rust CLIs, daemons, bots | terminal prompt, or `--accept-ufl 3.4` / `HONE_ACCEPT_LICENSE=UFL-3.4-D` | `$HONE_DATA_DIR` or `~/.hone` | unit + process tests; live `hone-node` |
| `hone-gui`, `hone-wallet-create` (egui) | first-run window (`rust/hone-license-egui`); the real app is built only after "I accept" | same | rendered under Xvfb; accept and decline clicked through |
| Desktop (`clients/hone-desktop`, Tauri) | first-run screen in `index.html`; `install_hone` and `start_node` refuse until accepted | same (`~/.hone`) | `cargo check` |
| Android (`android/`) | `LicenseActivity` before `MainActivity`; `BootReceiver` starts nothing and services stop themselves until accepted | app `filesDir` | `assembleDebug`; APK carries `assets/LICENSE.txt` |
| Flipper (`clients/hone-flipper`, `flipper/hone_sensor`, `hone_relay`, `hone_ble_relay`) | four dialog pages ending Decline/Accept | `/ext/apps_data/hone/license-acceptance.json`, shared by all HONE apps | `ufbt` build of all four |
| Installers (`website/install.sh`, `scripts/install.sh`) | summary, then typed "I accept" from the terminal, or `HONE_ACCEPT_LICENSE=UFL-3.4-D`; nothing is installed otherwise | `~/.hone`, or `/var/lib/hone` and `/var/lib/hone-testnet` | shell tests of each path |

Notes:

- **Desktop** still launches the deprecated Node.js prototype (`node
  bin/hone-mine` / `bin/hone-clock`). Its own gate covers that. When it moves
  to the Rust `hone-node`, the shared `~/.hone` record already satisfies the
  node's gate.
- **Android** services call `startForeground` before checking (Android
  requires it within seconds of `startForegroundService`), then stop at once
  if the license is not accepted. `MinerService` loads its native library when
  the class loads; it does no work until the check passes.
- **Flipper**: the RTC holds device-local time with no time zone, so the
  record's time has no `Z`.

## Not gated

- **Deprecated Node.js prototype** (`src/`, `bin/`, `deploy/systemd/hone-node.service`
  running `bin/hone-all`, `etc/*.service`, `website/install-termux.sh`): out of
  scope by instruction. It still ships and runs, and anything it runs is under
  the Unconditional scope (UFL Section 1A), not Decentralized.
- **`clients/hone-android` (Capacitor)**: a WebView of the hosted web app
  (`server.url`). It runs no HONE software on the phone, so there is nothing
  to gate; the hosted service is used, not the software.
- **JS Telegram bots** (`bots/honebot`, `bots/honewalletbot`): Node.js. Users
  interact with a service; the operator running the bot is a Licensee and is
  not prompted.
- **`website/hone-start.bat` + `website/docker-compose.yml`**: run the
  `hone:latest` image. Compose passes `HONE_ACCEPT_LICENSE` through; the batch
  file does not ask yet. If that image runs the Rust node, it exits 3 until
  the variable is set.
- **`start.sh`, `start-windows.bat`**: start `hone-node` in a terminal, so the
  node's own prompt appears.
