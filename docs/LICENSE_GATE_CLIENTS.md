# License gate: client follow-up (design note)

**Status: follow-up. Nothing here is implemented yet.**

The Rust binaries enforce the UFL 3.4 Decentralized acceptance step through
`rust/hone-license` (see [LICENSE_ACCEPTANCE.md](LICENSE_ACCEPTANCE.md)). The
clients below run HONE through their own launch paths, so each one needs its
own first-run step. Until a client has one, it does not present the step, and
anything it runs outside the Rust gate is licensed under the Unconditional
scope rather than Decentralized (UFL Section 1A).

Every client step has the same requirements as the Rust gate:

- Shown before first use, before any node, miner, or wallet code runs.
- Names the license: UFL 3.4, Operational Scope: Decentralized.
- Shows the dispute process (exclusive online ICC arbitration seated in
  California, individual claims only), the US$1-in-value mutual liability
  limit (money or HONE at the payer's choice), and the own-risk terms for
  forks and bypassers, with a way to read the full text. Reuse
  `hone_license::summary()` and `hone_license::LICENSE_TEXT` (or the same text)
  so every surface shows identical terms.
- An affirmative action: a button labelled "I accept" (not pre-checked, not
  defaulted), or the typed phrase. Declining exits or stays on the screen.
- Records locally only: license, version, scope, UTC time, binary/app name.
  Nothing over the network or on chain.
- Asks again when the license version or scope changes.

## Desktop (`clients/hone-desktop`, Tauri)

Today `start_node` spawns `node bin/hone-mine` / `bin/hone-clock`: the
deprecated Node.js prototype, not the Rust node, so the Rust gate never runs.

Needs:
1. A first-run license screen in the web UI that blocks `start_node` until
   accepted. The Tauri command checks the record (link `hone-license` into
   `src-tauri` and call `read_record` / `write_record`) and refuses to spawn
   otherwise.
2. When the desktop app moves to the Rust `hone-node`, pass acceptance by
   launching it with the record already written (preferred) or with
   `HONE_ACCEPT_LICENSE=UFL-3.4-D` set by the app *only after* the user clicked
   "I accept" in that session.

## Android (`clients/hone-android` Capacitor app; `android/` app with `rust/hone-android` and `android/rust/hone-miner` cdylibs)

The Rust miner is loaded as a native library (`MinerService`,
`NativeClockService`, `NativeFlipperService`), so there is no `main()` to gate.

Needs:
1. A first-run activity/screen shown before any service starts, with the
   summary, a "Read full license" view, and an "I accept" button.
2. Store the record in app-private storage (`filesDir/license-acceptance.json`,
   same fields). Services check it in `onCreate` and stop themselves if absent.
3. Optional defence in depth: export a JNI entry that calls
   `hone_license::read_record` and have the native miner refuse to start
   without a matching record.

## Flipper Zero (`clients/hone-flipper`, `flipper/hone_*`)

Firmware apps (C, ufbt) cannot link the Rust crate and have a small screen.

Needs:
1. A first-launch scene: a short notice ("UFL 3.4 Decentralized: ICC
   arbitration, US$1 liability cap, forks at own risk. Full text:
   github.com/shindevlin/hone, LICENSE") with Accept/Decline buttons.
2. Persist acceptance in the app's storage folder (`APP_DATA_PATH`), same
   fields where practical (version, scope, time if the RTC is set, app name).
3. The paired phone app's own gate (above) covers the phone-side relay; the
   Flipper scene covers the device itself.

## Other surfaces without the gate

- **Deprecated Node.js prototype** (`src/`, `bin/`, `deploy/systemd/hone-node.service`
  running `bin/hone-all`, `etc/*.service`): not gated, by instruction. It still
  ships and runs.
- **GUI binaries** (`hone-gui`, `hone-wallet-create`): gated, but they can only
  prompt when started from a terminal. Need an egui first-run dialog.
- **Installers** (`start.sh`, `start-windows.bat`, `scripts/install.sh`,
  `systemd/install.sh`, `website/install.sh`, `website/install-termux.sh`,
  `website/hone-start.bat`, `website/docker-compose.yml`): need a step that shows the
  terms and either runs `hone-license accept` or writes the variable only after
  the user accepts.
- **Telegram bots in JS** (`bots/honebot`, `bots/honewalletbot`): Node.js; not
  covered by the Rust gate. Their users interact with a service, not the
  software, but the operator running them is a Licensee.
