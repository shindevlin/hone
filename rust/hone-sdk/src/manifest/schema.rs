//! The manifest data model. Serialized to `hone-manifest.json` with stable
//! field ordering (BTreeMap / sorted vecs) so byte-for-byte diffs are meaningful
//! and CI can compare a committed manifest against a freshly generated one.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stability of a surface element — how much a consumer can rely on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stability {
    /// Fully supported. Safe to depend on.
    Stable,
    /// Still works, but scheduled for removal. Consumers should migrate.
    Deprecated,
    /// New / may change. Depend on it at your own risk.
    Experimental,
}

impl Default for Stability {
    fn default() -> Self {
        Stability::Stable
    }
}

/// A deprecation notice parsed from a `/// @deprecated ...` source marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deprecation {
    /// Version the deprecation was announced in, if given (`since=1.3.0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Recommended replacement, if given (`use=SensorDataCommit`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<String>,
    /// Free-text reason (`reason="..."`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Version it will be removed in, if given (`remove=2.0.0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_in: Option<String>,
}

/// One field of a ledger entry's canonical signing message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SigningField {
    pub name: String,
    /// The Rust type as written in the entry (e.g. "AccountId", "Hunits",
    /// "Option<String>"). Purely informational for consumers.
    pub ty: String,
}

/// A ledger entry type: the on-chain action a consumer can submit, plus the
/// exact shape it must sign. This is the part that, when wrong, silently fails
/// (the class of bug that bit the Flipper receiver — wrong signing message).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntrySpec {
    /// Variant name, e.g. "SensorDataCommit".
    pub name: String,
    /// One-line description (first line of the doc comment).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    /// All fields on the variant, in source order.
    pub fields: Vec<SigningField>,
    /// The `type` string used in the canonical signing message
    /// (e.g. "SENSOR_DATA_COMMIT"), if this entry is user-signable and has a
    /// `canonical_signing_message` arm. `None` for system-only entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_type: Option<String>,
    /// The exact fields the canonical signing message includes, in order.
    /// Empty for system entries. This is the authoritative signing contract.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signing_fields: Vec<String>,
    /// Whether `signed_by` must equal a specific field (e.g. "owner"), the
    /// account-binding rule enforced by `check_signature`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_by_bound_to: Option<String>,
    /// True if this entry only ever originates from the node (epoch seal), never
    /// from a consumer — e.g. rewards. Consumers cannot submit these.
    #[serde(default)]
    pub system_only: bool,
    pub stability: Stability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecation: Option<Deprecation>,
}

/// An HTTP route on the node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteSpec {
    /// e.g. "POST /api/sensor/commit".
    pub method: String,
    pub path: String,
    /// The handler function name (source cross-reference).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub handler: String,
    /// One-line summary from a `// ...` comment immediately above, if present.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    pub stability: Stability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecation: Option<Deprecation>,
}

/// The full manifest. All collections are sorted for deterministic output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version of the manifest format itself.
    pub schema_version: u32,
    /// The HONE workspace version this manifest was generated from
    /// (`rust/Cargo.toml` `[workspace.package] version`).
    pub hone_version: String,
    pub chain_id: String,
    /// Content hash of the surface (routes + entries), stable across
    /// regenerations of the same source. Consumers pin this.
    pub surface_hash: String,
    /// Ledger entry types, keyed by variant name (sorted).
    pub entries: BTreeMap<String, EntrySpec>,
    /// HTTP routes, keyed by "METHOD PATH" (sorted).
    pub routes: BTreeMap<String, RouteSpec>,
    /// Cross-chain invariants a consumer must respect (hand-curated, stable).
    pub invariants: Vec<String>,
}

impl Manifest {
    /// Count of stable / deprecated / experimental across the whole surface.
    pub fn stability_counts(&self) -> (usize, usize, usize) {
        let mut stable = 0;
        let mut deprecated = 0;
        let mut experimental = 0;
        let bump = |s: Stability, (a, b, c): &mut (usize, usize, usize)| match s {
            Stability::Stable => *a += 1,
            Stability::Deprecated => *b += 1,
            Stability::Experimental => *c += 1,
        };
        let mut acc = (0, 0, 0);
        for e in self.entries.values() {
            bump(e.stability, &mut acc);
        }
        for r in self.routes.values() {
            bump(r.stability, &mut acc);
        }
        stable += acc.0;
        deprecated += acc.1;
        experimental += acc.2;
        (stable, deprecated, experimental)
    }
}
