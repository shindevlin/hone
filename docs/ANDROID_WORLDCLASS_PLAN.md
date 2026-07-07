# HONE Android — From "Meh" to World-Class: Architecture & Build Plan

_Authored 2026-07-07. Status: PLAN (approve before building). Owner: Shin Devlin._

---

## TL;DR

The shipping APK is a **Capacitor webview that loads a remote page** (`btcpc.net/app`)
— a 5,557-line single-file vanilla-JS app running in a browser sandbox. It works, but it
is architecturally incapable of being world-class **or** of being the "phone is a full
node" product HONE actually specifies.

Meanwhile, `rust/hone-android/` is **already a native JNI library** (`cdylib`, already
HONE-branded: `network.hone.app.MinerService` / `NativeClockService` /
`NativeSensorService`) with the real chain engine (chain, clock, miner, net, sensors,
on-device candle LLM). **The world-class app already has its hard part half-built** — it
just isn't the thing shipping.

**The plan: finish the native path.** Kotlin/Jetpack Compose UI → JNI/uniffi bridge →
the existing Rust core, running in an Android **foreground service** so the phone is a
true miner+clock+sensor node even when backgrounded. Retire the webview.

---

## 1. Why "meh" — measured diagnosis (not vibes)

| Signal | Measured | Consequence |
|---|---|---|
| App shell | `www/index.html` = **11-line stub**; loads remote `btcpc.net/app` | Offline = blank screen; web latency/jank; server-dependent |
| UI | one **5,557-line** `app.html`, vanilla JS, no components | Unmaintainable; neither native nor a real web-app architecture |
| Accessibility | **0** `aria-*` / `role=` attributes | Fails store a11y review; excludes users |
| Native depth | only `Share` + `Preferences` wired | No haptics, no notifications, no background — **feels like a website** |
| Background node | webview is **killed on background** | Phone STOPS mining/clocking when you switch apps — breaks the core product |
| Icons | emoji as tab icons | Instant "hobby app" tell |
| Brand | `appName: BTCPC`, `appId: net.btcpc.app`, loads `btcpc.net` | Pre-rename; can't ship half-branded |
| Rust core usage | webview calls server HTTP API remotely | The powerful `hone-android` native engine **isn't even used** |

The bones are strong (real mining, sensors, wallet, clock, epoch tracking). The gap is
**native feel + reliability + actually running the node** — all of which the webview
cannot deliver and the native path already can.

---

## 2. Target architecture

```
┌───────────────────────────────────────────────────────────┐
│  Kotlin + Jetpack Compose  (Material 3, the UI layer)      │
│  Screens: Home/Node · Mine · Clock · Sensors · Wallet ·    │
│           Chat(LLM) · Settings                             │
│  Native: Haptics · Notifications · Foreground service UI · │
│          BiometricPrompt (wallet unlock) · WorkManager     │
├───────────────────────────────────────────────────────────┤
│  Bridge:  uniffi (typed, auto-generated Kotlin bindings)   │
│           replacing the hand-written JNI in lib.rs          │
├───────────────────────────────────────────────────────────┤
│  Rust core: rust/hone-android/  (UNCHANGED engine)         │
│  chain.rs · clock.rs · miner.rs · net.rs · sensors.rs ·    │
│  node.rs · llm.rs (candle GGUF) · store.rs (sled)          │
│  = the SAME consensus/crypto as the full node              │
├───────────────────────────────────────────────────────────┤
│  Android Foreground Service (keeps the node alive)         │
└───────────────────────────────────────────────────────────┘
```

### Why Rust core + Kotlin UI (not one or the other)

- **Rust for the chain** — consensus, crypto, signing, RocksDB/sled, libp2p. Rewriting
  this in Kotlin would fork security-critical code into a weaker second implementation and
  invite divergence. Keep the one audited engine.
- **Kotlin/Compose for the UI** — the native way to get 120Hz-smooth scrolling, haptics,
  system notifications, background services, Material 3. This is precisely what the
  webview cannot do.
- The webview loses on **both** axes — non-native UI *and* the Rust engine sits unused.

### Why a foreground service is non-negotiable

Memory (`project_phone_full_node`): **phones are full nodes** (miner+sensor+clock), and
the clock design tolerates intermittent devices *by design*. Android kills a webview when
backgrounded → the phone stops participating the moment the user leaves the app. Only a
**Kotlin foreground service driving the Rust core** lets the phone be the node HONE
specifies. This is product-defining, not polish.

---

## 3. Bridge decision: keep JNI or move to uniffi?

Current `lib.rs` uses **hand-written JNI** (`Java_network_hone_app_MinerService_native*`).
It works but is brittle: every function is manual `unsafe`, string marshalling by hand,
no type safety across the boundary, easy to desync signatures.

**Recommendation: migrate to `uniffi`** (Mozilla; powers Firefox's Rust↔Kotlin).
- Define the interface once in Rust (`#[uniffi::export]`); it **generates** the Kotlin
  bindings — typed, null-safe, with async support.
- Eliminates a whole class of JNI signature-mismatch crashes.
- Keeps a hand-written JNI shim only where uniffi can't reach (rare).

Trade-off: one-time migration cost of the ~10 existing native fns. Worth it — the current
JNI surface is small and the safety/velocity win compounds across the whole rebuild.

---

## 4. World-class bar — the checklist we're building to

**Feel**
- [ ] Jetpack Compose + Material 3 (dynamic color, dark-first to match the chain aesthetic)
- [ ] Haptic feedback on every meaningful action (tx sent, block sealed, reward earned)
- [ ] 60/120Hz-smooth lists; skeleton loaders, never spinners-on-blank
- [ ] Real icon set (Material Symbols or a custom HONE set) — no emoji
- [ ] Motion: shared-element transitions between screens; earn/seal animations

**Node-as-product**
- [ ] Foreground service with a live notification (epoch, balance, peers, mining state)
- [ ] Background mining/clock survives app switch, screen off, reboot (WorkManager restart)
- [ ] Battery-aware: throttle/pause on low battery or thermal; user-set "mine only on WiFi/charging"
- [ ] Push/local notification on: reward earned, node went offline, epoch milestones, tx confirmed

**Wallet (highest trust surface)**
- [ ] BiometricPrompt (fingerprint/face) to unlock signing — never a plaintext key in memory longer than needed
- [ ] Recoverable keystore (Argon2id+AES-GCM, already in `hone-sdk`) as the on-device store
- [ ] Clear send flow with human-readable confirm; **NEVER auto-sign** (route per the founder sign-request rule)
- [ ] QR receive/scan; address book; tx history with status

**Trust & polish**
- [ ] Full accessibility (content descriptions, TalkBack, dynamic font scaling, contrast)
- [ ] Offline-first: app opens and shows last state with no network; syncs when back
- [ ] Onboarding that explains "your phone is a node" in 3 screens
- [ ] Crash reporting (self-hosted / privacy-preserving), no third-party trackers
- [ ] Localizable strings (no hard-coded UI text)

**Store-ready**
- [ ] HONE branding end-to-end (appId `network.hone.app`, name "HONE", icon, splash)
- [ ] Play Store listing assets (feature graphic, screenshots, privacy policy)
- [ ] Signed release keystore (kept in the vault, NOT in git), reproducible build
- [ ] Target latest Android SDK, 64-bit, R8/shrink, size budget

---

## 5. Phased build

### Phase 0 — De-risk the bridge (1 sprint)
- Migrate the ~10 existing JNI fns in `hone-android/lib.rs` to **uniffi**; generate Kotlin
  bindings; prove `startNode()/getStatus()/getBalance()/getEpoch()` round-trip from a
  throwaway Kotlin activity. **Exit test:** Rust node starts, seals/syncs, returns live
  epoch to Kotlin. This is the riskiest piece — prove it before UI investment.

### Phase 1 — Native app shell + foreground service (1–2 sprints)
- New Kotlin/Compose module (or convert `clients/*-android`), `network.hone.app`.
- Nav scaffold (bottom bar: Node · Mine · Wallet · more), Material 3 theme, HONE brand.
- **Foreground service** hosting the Rust node; persistent notification with live stats.
- WorkManager to restart the node on boot / after kill. Battery/thermal guards.
- **Exit test:** phone mines + clocks with the app backgrounded and screen off.

### Phase 2 — Core screens against the live Rust core (2–3 sprints)
- Node/Home dashboard (epoch, peers, roles, earnings, hardware).
- Mine + Clock panels (start/stop, live stats, reward history) via the bridge.
- Sensors panel (permissions, live readings, submission status).
- Wallet: biometric unlock, balance, send (human-confirm, no auto-sign), receive QR,
  history, keystore export/import.
- **Exit test:** every screen reads/writes the real chain via Rust; no server dependency.

### Phase 3 — Polish, notifications, LLM chat (1–2 sprints)
- Haptics, transitions, skeletons, empty/error states.
- Local + push notifications (reward/offline/tx/epoch).
- On-device LLM chat screen wired to `llm.rs` (candle GGUF).
- Full accessibility pass; dynamic font/contrast; localization scaffold.

### Phase 4 — Store readiness (1 sprint)
- Icon/splash/branding final; Play listing assets; privacy policy.
- Release signing (keystore from vault), R8 shrink, size budget, reproducible build.
- Internal test track → closed beta (founders/Grouchly devices) → production.

---

## 6. What to retire / migrate

- **Retire** `clients/btcpc-android` webview path (Capacitor + remote `btcpc.net/app`)
  once Phase 2 reaches parity. Keep it buildable until then so there's always a shippable
  APK.
- **Salvage** from `website/app.html`: the *information architecture* and copy (which
  screens, what stats, the wallet flow) — it's a working spec of the product. Rebuild the
  presentation natively; keep the domain logic in Rust.
- **Reuse** `rust/hone-android/` engine as-is (only the bridge layer changes).

---

## 7. Decisions — LOCKED (Shin, 2026-07-07)

1. **Module home:** ✅ **new `clients/hone-android-native/`** (Kotlin/Compose). Keep the
   old `clients/btcpc-android` webview buildable until the native app reaches parity, then
   retire it. No risk to the currently-shipping APK.
2. **Bridge:** ✅ **migrate to uniffi** — define the interface once in Rust, auto-generate
   typed/null-safe Kotlin bindings. Migrate the ~10 existing JNI fns in `hone-android/lib.rs`.
3. **Icon system:** ✅ **Material Symbols now, commissioned HONE set later** — ship with
   Material Symbols to kill the emoji-icon problem immediately; commission a bespoke HONE
   icon set post-launch and swap in without blocking the build.
4. **Crash/analytics:** ✅ **self-hosted, privacy-preserving** — route crash reports to
   HONE-controlled infra; NO third-party trackers (Firebase/Sentry). On-brand for a
   sovereign/privacy chain; also keeps the Play data-safety listing clean.
5. **Distribution:** ✅ **direct signed APK to founders/Grouchly devices first** (matches
   how nodes are deployed today); stand up a Play internal-test track later. Fast iteration,
   no store-review latency at the start.

---

## 7b. Future track — HONE Telegram Mini App (planned, not yet built)

Separate from the native Android app: build a **HONE Telegram Mini App** as a
second distribution channel. Rationale — HONE already runs Telegram bots
(`btcpcbot`/wallet bot → renaming to hone), so a Mini App meets users where they
already are, with no install. Distinct codebase (web, inside Telegram); the
native Kotlin app remains the flagship and does the heavy lifting.

### What research (2026-07-07, 107-agent multi-source pass, 23/25 claims
confirmed 3-0/2-1) established about what a Mini App can actually DO:

| Role | Verdict | Why |
|---|---|---|
| Sensor (point-in-time) | ✅ FEASIBLE | Bot API 8.0+ exposes native `Accelerometer`/`Gyroscope`/`DeviceOrientation`/`LocationManager` as first-class SDK APIs. Foreground-only (fires while the app is open), not continuous background capture. |
| Compute (on-device inference) | 🟡 PARTIAL, foreground-only | WebGPU + WASM SIMD gives real in-browser LLM inference (~71-80% of native for 3.8B-8B models) — but only on high-end hardware, and WebGPU exposure specifically inside Telegram's WKWebView/Android WebView is UNPROVEN. |
| Miner / Clock | ❌ INFEASIBLE | Telegram's own lifecycle marks the app "deactivated" on minimize/background. No Service Worker, background sync, or background-task API — JS does not run persistently when the app is closed or backgrounded. An always-on role that dies on background isn't that role. |
| P2P Relay | ❌ INFEASIBLE standalone | Browser js-libp2p can speak WebRTC/WebSocket/WebTransport, but browser-to-browser connections need external STUN + a Circuit Relay server to bootstrap — a webview cannot self-form a gossipsub mesh. Degrades to a relay-dependent client, not an autonomous peer. |
| Wallet — key custody | 🔴 must NOT hold keys in-webview | `SecureStorage` (OS Keychain/Keystore) could technically hold a key safely, but Telegram's blockchain guidelines (effective Feb 2025) mandate that Mini Apps only interact with wallets via **TON Connect** and explicitly prohibit signing transactions on non-TON chains outside allowed bridging. Generating/holding a native HONE key and signing it in the Mini App's own JS is a **platform ToS violation**, not just a UX tradeoff — Telegram has already forced other projects off-chain on this exact rule. |

### The corrected design: Mini App SENDS, it just never SIGNS locally

The wallet finding does **not** mean the Mini App is read-only. TON Connect's
actual pattern is the model to copy: **the dApp (Mini App) never touches the
private key — it requests a signature, and a separate wallet performs it and
returns the result.** Applied to HONE:

- The Mini App builds the transaction (recipient, amount, memo) and shows the
  human-confirm UI — same "push button, no copy-paste" ethos as the native app.
- Instead of signing locally, it issues a **sign-request** that is fulfilled by
  the **native Kotlin app** (which holds the Rust keystore) via a **"HONE
  Connect"** handshake — a HONE-native analogue of TON Connect: a deep-link /
  QR / Telegram-bot-relayed handshake that hands the unsigned tx to the phone's
  native app, gets it signed there (behind the SAME biometric gate as the
  native Wallet screen — see §4), and returns the signed result to the Mini App
  / chain. The private key and the signing operation NEVER execute inside
  Telegram's webview.
- This is fully compliant (Telegram's rule is about what executes in THEIR
  webview, not about whether the user can transact) and it is the same
  founder-safe pattern already mandated elsewhere in this codebase: signing is
  never auto-performed by an untrusted surface, it is always a confirmed,
  gated act by the key-holding app.
- If no native app / HONE Connect peer is available (e.g. a brand-new user
  with only Telegram), the Mini App falls back to **read-only** (balance,
  history, sensor submission) and prompts the user to install the native app
  to unlock sending — it does not fall back to holding a key itself.

### Always-on roles stay off the webview, by design

Miner, Clock, and Relay belong to the **native Kotlin app** (foreground
service, §2) full stop — this was already the plan, and the research confirms
it's not just the better choice but close to the *only* choice: the platform
gives no primitive for persistent background execution. The Mini App's job is
reach (zero-install dashboard + sensor snapshots + a signed-send handshake to
the real node), not node participation. This matches the existing hardline
that a peerless/webview surface must never be treated as an authoritative
node — see "Hardline: No Local Submission Without Peers" in CLAUDE.md.

Scope the Mini App build after the native app's Phase 2 (wallet/biometric
patterns need to be settled first, since the Mini App's sign flow depends on
them) and after a HONE Connect handshake spec exists.

## 8. Cross-cutting constraints (must hold)

- **Brand:** HONE everywhere (`network.hone.app` already used in the Rust JNI names — good).
  No BTCPC/btcpc in the new module.
- **Wallet safety:** NEVER auto-sign a transfer; human confirm + biometric; route founder
  transfers as sign-requests (per standing rule). Secrets never in git/logs.
- **No divergence:** the phone runs the SAME Rust consensus as the full node — do not
  reimplement chain logic in Kotlin.
- **Offline integrity:** show last-known state offline, but never present unconfirmed local
  entries as confirmed (mirrors the "no local submission without peers" hardline).
