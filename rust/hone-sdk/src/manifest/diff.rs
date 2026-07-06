//! Diff two manifests and classify every change by kind and severity.
//!
//! This is what turns "HONE shipped a commit" into an actionable answer for a
//! consumer repo: **what is new, what is gone, what still works but is
//! deprecated, and — critically — what changed shape in a way that will break
//! me.**
//!
//! Severity drives CI exit codes in [`super::sync`]:
//!   * `Breaking`   → a consumer depending on this will fail. Exit non-zero.
//!   * `Warning`    → deprecated (still works) — migrate. Exit zero, but loud.
//!   * `Info`       → additive / compatible. Nothing to do.

use crate::manifest::schema::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// A consumer relying on this will break. (removed, or signing shape changed)
    Breaking,
    /// Still works, but should be migrated. (newly deprecated)
    Warning,
    /// Additive or cosmetic. Safe.
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Deprecated,
    /// Un-deprecated (came back to stable) — rare but possible.
    Restored,
    /// The signing shape or method/path changed in a breaking way.
    SigningChanged,
}

/// The subject of a change: an entry or a route, named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subject {
    Entry(String),
    Route(String),
}

impl Subject {
    pub fn label(&self) -> String {
        match self {
            Subject::Entry(n) => format!("entry {n}"),
            Subject::Route(n) => format!("route {n}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub kind: ChangeKind,
    pub severity: Severity,
    pub subject: Subject,
    /// Human-readable detail (what changed, and the migration hint if any).
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestDiff {
    pub from_version: String,
    pub to_version: String,
    pub from_surface_hash: String,
    pub to_surface_hash: String,
    pub changes: Vec<Change>,
}

impl ManifestDiff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
    /// True if any change is breaking.
    pub fn has_breaking(&self) -> bool {
        self.changes.iter().any(|c| c.severity == Severity::Breaking)
    }
    /// True if any change is a deprecation warning.
    pub fn has_warnings(&self) -> bool {
        self.changes.iter().any(|c| c.severity == Severity::Warning)
    }
    pub fn breaking(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.severity == Severity::Breaking)
    }
}

/// Compute the diff `old` → `new`. `filter` optionally restricts to a set of
/// entry/route names a consumer actually depends on (so a consumer only hears
/// about changes to the surface *it uses*).
pub fn diff_manifests(old: &Manifest, new: &Manifest, filter: Option<&Filter>) -> ManifestDiff {
    let mut changes = Vec::new();

    let uses_entry = |n: &str| filter.map_or(true, |f| f.entries.is_empty() || f.entries.contains(n));
    let uses_route = |n: &str| filter.map_or(true, |f| f.routes.is_empty() || f.routes.contains(n));

    // ── Entries ──────────────────────────────────────────────────────────
    for (name, new_e) in &new.entries {
        if !uses_entry(name) {
            continue;
        }
        match old.entries.get(name) {
            None => changes.push(Change {
                kind: ChangeKind::Added,
                severity: Severity::Info,
                subject: Subject::Entry(name.clone()),
                detail: added_entry_detail(new_e),
            }),
            Some(old_e) => {
                // Signing shape change = breaking (this is the silent-failure class).
                if old_e.signing_type != new_e.signing_type
                    || old_e.signing_fields != new_e.signing_fields
                    || old_e.signed_by_bound_to != new_e.signed_by_bound_to
                {
                    changes.push(Change {
                        kind: ChangeKind::SigningChanged,
                        severity: Severity::Breaking,
                        subject: Subject::Entry(name.clone()),
                        detail: signing_change_detail(old_e, new_e),
                    });
                }
                // Stability transitions.
                push_stability_change(
                    &mut changes,
                    Subject::Entry(name.clone()),
                    old_e.stability,
                    new_e.stability,
                    new_e.deprecation.as_ref(),
                );
            }
        }
    }
    for (name, old_e) in &old.entries {
        if !uses_entry(name) {
            continue;
        }
        if !new.entries.contains_key(name) {
            changes.push(Change {
                kind: ChangeKind::Removed,
                severity: Severity::Breaking,
                subject: Subject::Entry(name.clone()),
                detail: removed_detail(old_e.deprecation.as_ref()),
            });
        }
    }

    // ── Routes ───────────────────────────────────────────────────────────
    for (name, new_r) in &new.routes {
        if !uses_route(name) {
            continue;
        }
        match old.routes.get(name) {
            None => changes.push(Change {
                kind: ChangeKind::Added,
                severity: Severity::Info,
                subject: Subject::Route(name.clone()),
                detail: new_r.summary.clone(),
            }),
            Some(old_r) => push_stability_change(
                &mut changes,
                Subject::Route(name.clone()),
                old_r.stability,
                new_r.stability,
                new_r.deprecation.as_ref(),
            ),
        }
    }
    for (name, old_r) in &old.routes {
        if !uses_route(name) {
            continue;
        }
        if !new.routes.contains_key(name) {
            changes.push(Change {
                kind: ChangeKind::Removed,
                severity: Severity::Breaking,
                subject: Subject::Route(name.clone()),
                detail: removed_detail(old_r.deprecation.as_ref()),
            });
        }
    }

    // Stable, deterministic ordering: breaking first, then warnings, then info;
    // within a severity, by subject label.
    changes.sort_by(|a, b| {
        sev_rank(a.severity)
            .cmp(&sev_rank(b.severity))
            .then_with(|| a.subject.label().cmp(&b.subject.label()))
    });

    ManifestDiff {
        from_version: old.hone_version.clone(),
        to_version: new.hone_version.clone(),
        from_surface_hash: old.surface_hash.clone(),
        to_surface_hash: new.surface_hash.clone(),
        changes,
    }
}

fn sev_rank(s: Severity) -> u8 {
    match s {
        Severity::Breaking => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    }
}

fn push_stability_change(
    changes: &mut Vec<Change>,
    subject: Subject,
    old_s: Stability,
    new_s: Stability,
    dep: Option<&Deprecation>,
) {
    match (old_s, new_s) {
        (Stability::Stable, Stability::Deprecated)
        | (Stability::Experimental, Stability::Deprecated) => {
            changes.push(Change {
                kind: ChangeKind::Deprecated,
                severity: Severity::Warning,
                subject,
                detail: deprecation_detail(dep),
            });
        }
        (Stability::Deprecated, Stability::Stable) => {
            changes.push(Change {
                kind: ChangeKind::Restored,
                severity: Severity::Info,
                subject,
                detail: "no longer deprecated".to_string(),
            });
        }
        _ => {}
    }
}

fn added_entry_detail(e: &EntrySpec) -> String {
    let mut d = e.summary.clone();
    if let Some(t) = &e.signing_type {
        d.push_str(&format!(" [submit: signs {t}]"));
    } else if e.system_only {
        d.push_str(" [system-only — not submittable]");
    }
    d
}

fn signing_change_detail(old: &EntrySpec, new: &EntrySpec) -> String {
    let mut parts = Vec::new();
    if old.signing_type != new.signing_type {
        parts.push(format!(
            "signing type {:?} -> {:?}",
            old.signing_type, new.signing_type
        ));
    }
    if old.signing_fields != new.signing_fields {
        parts.push(format!(
            "signing fields {:?} -> {:?}",
            old.signing_fields, new.signing_fields
        ));
    }
    if old.signed_by_bound_to != new.signed_by_bound_to {
        parts.push(format!(
            "signed_by binding {:?} -> {:?}",
            old.signed_by_bound_to, new.signed_by_bound_to
        ));
    }
    format!("BREAKING: {}. Re-sign with the new shape.", parts.join("; "))
}

fn deprecation_detail(dep: Option<&Deprecation>) -> String {
    let Some(d) = dep else {
        return "deprecated — migrate when convenient".to_string();
    };
    let mut s = String::from("deprecated");
    if let Some(since) = &d.since {
        s.push_str(&format!(" since {since}"));
    }
    if let Some(r) = &d.replacement {
        s.push_str(&format!("; use {r} instead"));
    }
    if let Some(rem) = &d.remove_in {
        s.push_str(&format!("; removed in {rem}"));
    }
    if let Some(reason) = &d.reason {
        s.push_str(&format!(" ({reason})"));
    }
    s
}

fn removed_detail(prior_dep: Option<&Deprecation>) -> String {
    match prior_dep.and_then(|d| d.replacement.as_ref()) {
        Some(r) => format!("REMOVED — was deprecated; migrate to {r}"),
        None => "REMOVED — no longer served. This will fail.".to_string(),
    }
}

/// Restricts a diff to the surface a consumer actually uses. Empty sets = "all".
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub entries: std::collections::BTreeSet<String>,
    pub routes: std::collections::BTreeSet<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn entry(name: &str, ty: Option<&str>, fields: &[&str], stab: Stability) -> EntrySpec {
        EntrySpec {
            name: name.to_string(),
            summary: format!("{name} summary"),
            fields: vec![],
            signing_type: ty.map(|s| s.to_string()),
            signing_fields: fields.iter().map(|s| s.to_string()).collect(),
            signed_by_bound_to: Some("owner".to_string()),
            system_only: ty.is_none(),
            stability: stab,
            deprecation: if stab == Stability::Deprecated {
                Some(Deprecation {
                    since: Some("1.3.0".into()),
                    replacement: Some("NewThing".into()),
                    reason: None,
                    remove_in: Some("2.0.0".into()),
                })
            } else {
                None
            },
        }
    }

    fn manifest(entries: Vec<EntrySpec>) -> Manifest {
        let mut map = BTreeMap::new();
        for e in entries {
            map.insert(e.name.clone(), e);
        }
        Manifest {
            schema_version: 1,
            hone_version: "1.0.0".into(),
            chain_id: "hone".into(),
            surface_hash: "h".into(),
            entries: map,
            routes: BTreeMap::new(),
            invariants: vec![],
        }
    }

    #[test]
    fn detects_added_entry() {
        let old = manifest(vec![]);
        let new = manifest(vec![entry("Foo", Some("FOO"), &["a", "b"], Stability::Stable)]);
        let d = diff_manifests(&old, &new, None);
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].kind, ChangeKind::Added);
        assert_eq!(d.changes[0].severity, Severity::Info);
        assert!(!d.has_breaking());
    }

    #[test]
    fn detects_removed_entry_as_breaking() {
        let old = manifest(vec![entry("Foo", Some("FOO"), &["a"], Stability::Stable)]);
        let new = manifest(vec![]);
        let d = diff_manifests(&old, &new, None);
        assert_eq!(d.changes[0].kind, ChangeKind::Removed);
        assert!(d.has_breaking());
    }

    #[test]
    fn detects_signing_shape_change_as_breaking() {
        let old = manifest(vec![entry("Foo", Some("FOO"), &["a", "b"], Stability::Stable)]);
        let new = manifest(vec![entry("Foo", Some("FOO"), &["a", "b", "c"], Stability::Stable)]);
        let d = diff_manifests(&old, &new, None);
        assert_eq!(d.changes[0].kind, ChangeKind::SigningChanged);
        assert!(d.has_breaking());
    }

    #[test]
    fn detects_deprecation_as_warning_not_breaking() {
        let old = manifest(vec![entry("Foo", Some("FOO"), &["a"], Stability::Stable)]);
        let new = manifest(vec![entry("Foo", Some("FOO"), &["a"], Stability::Deprecated)]);
        let d = diff_manifests(&old, &new, None);
        assert_eq!(d.changes[0].kind, ChangeKind::Deprecated);
        assert_eq!(d.changes[0].severity, Severity::Warning);
        assert!(!d.has_breaking());
        assert!(d.has_warnings());
    }

    #[test]
    fn filter_restricts_to_used_surface() {
        let old = manifest(vec![
            entry("Foo", Some("FOO"), &["a"], Stability::Stable),
            entry("Bar", Some("BAR"), &["a"], Stability::Stable),
        ]);
        let new = manifest(vec![entry("Foo", Some("FOO"), &["a"], Stability::Stable)]);
        // Bar removed, but consumer only uses Foo → no changes surfaced.
        let mut f = Filter::default();
        f.entries.insert("Foo".to_string());
        let d = diff_manifests(&old, &new, Some(&f));
        assert!(d.is_empty(), "consumer using only Foo shouldn't hear about Bar");
    }
}
