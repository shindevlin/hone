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

## 7. Open decisions for Shin

1. **Module home:** new `clients/hone-android-native/` (Kotlin) vs. converting the existing
   `clients/btcpc-android` dir. (Recommend: new module; retire old after parity.)
2. **Icon system:** Material Symbols vs. a commissioned HONE icon set.
3. **Crash/analytics:** self-hosted (privacy-preserving, on-brand) vs none at launch.
4. **Beta channel:** Play internal testing vs. direct-APK for founders first.
5. **Bridge:** confirm uniffi migration (recommended) vs. extend hand-written JNI.

---

## 8. Cross-cutting constraints (must hold)

- **Brand:** HONE everywhere (`network.hone.app` already used in the Rust JNI names — good).
  No BTCPC/btcpc in the new module.
- **Wallet safety:** NEVER auto-sign a transfer; human confirm + biometric; route founder
  transfers as sign-requests (per standing rule). Secrets never in git/logs.
- **No divergence:** the phone runs the SAME Rust consensus as the full node — do not
  reimplement chain logic in Kotlin.
- **Offline integrity:** show last-known state offline, but never present unconfirmed local
  entries as confirmed (mirrors the "no local submission without peers" hardline).
