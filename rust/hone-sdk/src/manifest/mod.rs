//! HONE integration manifest — the self-updating contract between the chain
//! and every repo in the ecosystem.
//!
//! # The problem this solves
//!
//! A repo that consumes HONE (Bullship, a bot, a service worker) hard-codes an
//! understanding of HONE's surface: which routes exist, which ledger entries it
//! submits, the exact fields it must sign. When HONE ships a commit that adds,
//! removes, or deprecates part of that surface, the consumer breaks — silently,
//! at runtime, in production.
//!
//! # The fix: a generated, versioned, diffable manifest
//!
//! The manifest is a **pure function of the node source** (`api.rs` routes +
//! `LedgerEntry` variants and their canonical signing shapes). It is:
//!
//!   1. **Generated** by [`generate::generate_manifest`] from the source tree —
//!      never hand-maintained, so it cannot drift from what the node serves.
//!   2. **Committed** to the hone repo as `hone-manifest.json`. A CI check
//!      (`hone manifest check`) fails the build if the committed manifest and
//!      the freshly-generated one disagree — so every commit that changes the
//!      API surface MUST update the manifest in the same commit. "What changed"
//!      is therefore a real `git diff`.
//!   3. **Diffable** by [`diff::diff_manifests`], which classifies every change
//!      as ADDED / REMOVED / DEPRECATED / CHANGED (breaking vs. compatible).
//!   4. **Consumed** by `hone sync` (see [`sync`]), which regenerates a
//!      consumer repo's `HONE.md` + `HONE.lock` and prints the changelog,
//!      exiting non-zero on breaking changes so the consumer's CI catches them.
//!
//! # Deprecation convention (the "instamagic")
//!
//! There is no runtime flag for deprecation — you annotate the *source*:
//!
//! ```ignore
//! /// @deprecated since=1.3.0 use=SensorDataCommit reason="folded into commit"
//! /// Legacy single-reading submit. Still accepted; will be removed in 2.0.
//! SensorReadingLegacy { .. }
//! ```
//!
//! The generator reads the `@deprecated` marker into the manifest. Consumers see
//! it as a WARNING in their changelog ("still works, but migrate"), distinct
//! from REMOVED (a hard break). A consumer knows, per HONE commit, exactly what
//! is new, what is gone, and what still works but should be migrated.

pub mod schema;
pub mod generate;
pub mod diff;
pub mod sync;

pub use schema::{Manifest, EntrySpec, RouteSpec, Deprecation, Stability};
pub use diff::{ManifestDiff, Change, ChangeKind, Severity};

/// The manifest schema version. Bump only when the *shape of the manifest
/// itself* changes (not when the chain surface changes). Consumers check this
/// to know they can parse a manifest at all.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Canonical filename for the in-repo generated manifest (hone repo root).
pub const MANIFEST_FILENAME: &str = "hone-manifest.json";

/// Canonical filenames a consumer repo carries.
pub const CONSUMER_DOC: &str = "HONE.md";
pub const CONSUMER_LOCK: &str = "HONE.lock";
