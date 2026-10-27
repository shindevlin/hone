# hone-android uniffi bridge — build notes

Phase 0a of the world-class Android plan (`docs/ANDROID_WORLDCLASS_PLAN.md`).
The Rust core now exposes a **uniffi** interface (`src/bridge.rs`) that generates
typed Kotlin bindings, replacing the hand-written JNI in `lib.rs`.

## What was added

- `src/bridge.rs` — the `HoneNode` uniffi object + `NodeConfig` / `NodeStatus`
  records + `NodeError` enum. This is the new Kotlin surface.
- `src/uniffi_bindgen.rs` — in-crate binding generator (`uniffi-bindgen` bin).
- `build.rs` — minimal (pure proc-macro mode; no `.udl`).
- `lib.rs` — `uniffi::setup_scaffolding!()` at crate root (REQUIRED there, not in a
  submodule — it emits the crate-root `UniFfiTag` the derives reference).
- Cargo.toml — `uniffi` dep (+ build-dep), `crate-type = ["cdylib","staticlib","lib"]`,
  `thiserror`, and the `uniffi-bindgen` bin.

The legacy JNI in `lib.rs` is retained transitionally (no live consumer depends on it —
the shipping webview APK calls the server HTTP API, not this `.so`). It is deleted once
the Kotlin app is on the uniffi surface.

## Generating the Kotlin bindings

```bash
cargo build --lib                       # produces the cdylib
cargo run --features=uniffi/cli --bin uniffi-bindgen -- \
  generate --library target/<triple>/<profile>/libhone_miner.so \
  --language kotlin --out-dir <android-module>/src/main/java
```
On Windows host the artifact is `hone_miner.dll`; on Android it is
`libhone_miner.so` per-ABI. Output lands in `generated/kotlin/uniffi/hone_miner/`
(gitignored — it is a build product, regenerated each build).

## ⚠️ Known toolchain gotcha: vendored OpenSSL vs MSYS perl

`Cargo.toml` declares `openssl = { features = ["vendored"] }`. Building vendored
OpenSSL on **Windows/MSYS (Git-Bash)** FAILS — MSYS `perl` emits VMS-style paths
and is missing `Locale::Maketext::Simple`. This is the same blocker seen during
the BTCPC→HONE rename.

**Host compile-check / bindgen workaround** (do NOT hard-code these into the crate):
```bash
export OPENSSL_NO_VENDOR=1
export OPENSSL_DIR="C:/Program Files/PostgreSQL/17"      # any system OpenSSL w/ headers
export OPENSSL_LIB_DIR="C:/Program Files/PostgreSQL/17/lib"
export OPENSSL_INCLUDE_DIR="C:/Program Files/PostgreSQL/17/include"
cargo +1.90.0 build --lib
```
(PostgreSQL ships a full OpenSSL with headers; it's a convenient host stand-in.)

**On the real Android build this does NOT apply** — the Android target cross-compiles
with the NDK (not MSYS perl) and `reqwest` is already on `rustls-tls`. Verify whether
`openssl vendored` is even needed for the Android ABIs during Phase 0b; if nothing on
the `aarch64-linux-android` graph pulls native-tls, the `openssl` dep can likely be
dropped entirely, removing this gotcha for good.

## Status

- ✅ Phase 0a: uniffi bridge compiles for host (rustc 1.90), Kotlin bindings generate
  (`HoneNode`, `NodeConfig`, `NodeStatus`, sealed `NodeException` — 1,824 lines).
- ⏳ Phase 0b: cross-compile the `.so` for `aarch64-linux-android` (+ armv7/x86_64) via
  `cargo-ndk`, load it from a Kotlin activity, prove `start()/status()` round-trip on a
  device. Needs Android NDK + JDK 17 (not present in this MSYS host — do on a Linux/dev
  box, e.g. Grouchly, or install the NDK toolchain).
