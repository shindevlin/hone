//! uniffi binding generator entry point.
//!
//! Run to emit the Kotlin bindings for the Android app:
//!   cargo run --features=uniffi/cli --bin uniffi-bindgen -- \
//!     generate --library target/<triple>/release/libhone_miner.so \
//!     --language kotlin --out-dir <android module>/src/main/java
//!
//! Keeping the generator in-crate guarantees the bindgen version always matches
//! the uniffi runtime version compiled into the .so (a common source of subtle
//! ABI mismatches when they drift).
fn main() {
    uniffi::uniffi_bindgen_main()
}
