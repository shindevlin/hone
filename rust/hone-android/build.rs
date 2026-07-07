//! Build script for the uniffi bridge.
//!
//! This crate uses uniffi in **pure proc-macro mode** (`#[uniffi::export]` +
//! `uniffi::setup_scaffolding!()` in bridge.rs). In that mode the interface is
//! defined entirely in Rust source and NO external `.udl` file or build-time
//! scaffolding generation is required — the macros emit the scaffolding
//! directly. This build.rs is intentionally minimal; it exists only as the
//! documented hook point if a future step needs to add codegen.
fn main() {
    // Re-run if the bridge interface changes (belt-and-suspenders; cargo tracks
    // src changes anyway, but this makes the dependency explicit).
    println!("cargo:rerun-if-changed=src/bridge.rs");
}
