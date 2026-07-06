//! Consumer-side sync: keep a repo's `HONE.md` + `HONE.lock` current and tell
//! the developer/agent exactly what changed.
//!
//! `HONE.lock` is the machine record: the last manifest this repo synced to
//! (surface hash, version, and the subset of entries/routes this repo declares
//! it uses). `HONE.md` is the human/agent-readable rendering: the current
//! contract plus the changelog since the last sync.
//!
//! Flow (`hone sync`):
//!   1. obtain the new manifest (from a path, or fetched from a node),
//!   2. load the old manifest from `HONE.lock` (if any),
//!   3. diff, restricted to the surface this repo uses,
//!   4. rewrite `HONE.md` (contract + changelog) and `HONE.lock`,
//!   5. return the diff so the CLI can set an exit code.

use crate::manifest::diff::{diff_manifests, Filter, ManifestDiff, Severity, ChangeKind};
use crate::manifest::schema::*;
use crate::manifest::{CONSUMER_DOC, CONSUMER_LOCK};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// The `HONE.lock` on-disk record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFile {
    pub hone_version: String,
    pub surface_hash: String,
    pub chain_id: String,
    /// Entry names this repo declares it uses. Empty = "track everything".
    #[serde(default)]
    pub uses_entries: BTreeSet<String>,
    /// Route keys ("METHOD /path") this repo declares it uses. Empty = all.
    #[serde(default)]
    pub uses_routes: BTreeSet<String>,
    /// The full manifest at last sync, so the next diff has a baseline without a
    /// network round-trip.
    pub manifest: Manifest,
}

impl LockFile {
    pub fn filter(&self) -> Filter {
        Filter {
            entries: self.uses_entries.clone(),
            routes: self.uses_routes.clone(),
        }
    }
}

/// Result of a sync operation.
pub struct SyncOutcome {
    pub diff: Option<ManifestDiff>,
    pub wrote_doc: bool,
    pub first_sync: bool,
}

/// Perform a sync in `repo_dir` against `new_manifest`. Reads/writes
/// `HONE.md` and `HONE.lock`. Preserves the repo's declared `uses_*` sets.
pub fn sync_repo(repo_dir: &Path, new_manifest: &Manifest) -> Result<SyncOutcome> {
    let lock_path = repo_dir.join(CONSUMER_LOCK);
    let doc_path = repo_dir.join(CONSUMER_DOC);

    let prior: Option<LockFile> = if lock_path.exists() {
        let raw = std::fs::read_to_string(&lock_path)
            .with_context(|| format!("reading {}", lock_path.display()))?;
        Some(serde_json::from_str(&raw).context("parsing HONE.lock")?)
    } else {
        None
    };

    let (filter, uses_entries, uses_routes) = match &prior {
        Some(l) => (l.filter(), l.uses_entries.clone(), l.uses_routes.clone()),
        None => (Filter::default(), BTreeSet::new(), BTreeSet::new()),
    };

    let diff = prior
        .as_ref()
        .map(|l| diff_manifests(&l.manifest, new_manifest, Some(&filter)));

    // Write the new lock.
    let lock = LockFile {
        hone_version: new_manifest.hone_version.clone(),
        surface_hash: new_manifest.surface_hash.clone(),
        chain_id: new_manifest.chain_id.clone(),
        uses_entries,
        uses_routes,
        manifest: new_manifest.clone(),
    };
    std::fs::write(&lock_path, serde_json::to_string_pretty(&lock)?)
        .with_context(|| format!("writing {}", lock_path.display()))?;

    // Render the human/agent doc.
    let doc = render_doc(new_manifest, diff.as_ref(), &lock);
    std::fs::write(&doc_path, doc)
        .with_context(|| format!("writing {}", doc_path.display()))?;

    Ok(SyncOutcome {
        diff,
        wrote_doc: true,
        first_sync: prior.is_none(),
    })
}

/// Render the changelog as a plain-text block for terminal / CI output.
pub fn render_changelog(diff: &ManifestDiff) -> String {
    if diff.is_empty() {
        return format!(
            "HONE surface unchanged ({} @ {}).",
            &diff.to_surface_hash[..diff.to_surface_hash.len().min(12)],
            diff.to_version
        );
    }
    let mut out = String::new();
    out.push_str(&format!(
        "HONE surface changed: {} -> {}\n",
        diff.from_version, diff.to_version
    ));
    let mut section = |title: &str, sev: Severity| {
        let items: Vec<&_> = diff.changes.iter().filter(|c| c.severity == sev).collect();
        if items.is_empty() {
            return;
        }
        out.push_str(&format!("\n{title}\n"));
        for c in items {
            let tag = match c.kind {
                ChangeKind::Added => "ADDED",
                ChangeKind::Removed => "REMOVED",
                ChangeKind::Deprecated => "DEPRECATED",
                ChangeKind::Restored => "RESTORED",
                ChangeKind::SigningChanged => "CHANGED",
            };
            out.push_str(&format!("  [{tag}] {} — {}\n", c.subject.label(), c.detail));
        }
    };
    section("BREAKING (your build depends on these — fix required):", Severity::Breaking);
    section("DEPRECATED (still works — migrate):", Severity::Warning);
    section("Additive (new capabilities):", Severity::Info);
    out
}

/// Render `HONE.md`: the current contract plus the since-last-sync changelog.
fn render_doc(m: &Manifest, diff: Option<&ManifestDiff>, lock: &LockFile) -> String {
    let (stable, deprecated, experimental) = m.stability_counts();
    let mut s = String::new();

    s.push_str("<!-- HONE.md — how this repo uses HONE. GENERATED by `hone sync`.\n");
    s.push_str("     Do not edit the generated block by hand. To change which surface\n");
    s.push_str("     this repo tracks, edit `uses_entries`/`uses_routes` in HONE.lock. -->\n\n");
    s.push_str("# How this repo uses HONE\n\n");
    s.push_str(&format!(
        "- **HONE version:** {}\n- **Chain:** {}\n- **Surface hash:** `{}`\n",
        m.hone_version, m.chain_id, m.surface_hash
    ));
    s.push_str(&format!(
        "- **Surface size:** {} entries, {} routes ({stable} stable, {deprecated} deprecated, {experimental} experimental)\n",
        m.entries.len(),
        m.routes.len()
    ));
    s.push_str("- **Refresh:** `hone sync --node <url>` (or `--manifest <path>`)\n\n");

    // Changelog since last sync.
    if let Some(d) = diff {
        s.push_str("## Changes since last sync\n\n");
        if d.is_empty() {
            s.push_str("_No changes to the surface this repo uses._\n\n");
        } else {
            s.push_str("```\n");
            s.push_str(&render_changelog(d));
            s.push_str("\n```\n\n");
        }
    }

    // What this repo declares it uses (or a hint to declare it).
    s.push_str("## Surface this repo depends on\n\n");
    if lock.uses_entries.is_empty() && lock.uses_routes.is_empty() {
        s.push_str(
            "_This repo tracks the **entire** HONE surface. To get change alerts \
             for only the parts you use, list them under `uses_entries` / \
             `uses_routes` in `HONE.lock`._\n\n",
        );
    } else {
        if !lock.uses_entries.is_empty() {
            s.push_str("**Entries submitted:**\n\n");
            for name in &lock.uses_entries {
                if let Some(e) = m.entries.get(name) {
                    s.push_str(&format!("- `{}`", name));
                    if let Some(t) = &e.signing_type {
                        s.push_str(&format!(" — signs `{}` over `[{}]`", t, e.signing_fields.join(", ")));
                        if let Some(b) = &e.signed_by_bound_to {
                            s.push_str(&format!("; `signed_by` must equal `{}`", b));
                        }
                    }
                    if e.stability == Stability::Deprecated {
                        s.push_str("  ⚠️ DEPRECATED");
                    }
                    s.push('\n');
                } else {
                    s.push_str(&format!("- `{}`  ❌ NO LONGER EXISTS\n", name));
                }
            }
            s.push('\n');
        }
        if !lock.uses_routes.is_empty() {
            s.push_str("**Routes called:**\n\n");
            for key in &lock.uses_routes {
                match m.routes.get(key) {
                    Some(r) => {
                        s.push_str(&format!("- `{}`", key));
                        if !r.summary.is_empty() {
                            s.push_str(&format!(" — {}", r.summary));
                        }
                        if r.stability == Stability::Deprecated {
                            s.push_str("  ⚠️ DEPRECATED");
                        }
                        s.push('\n');
                    }
                    None => s.push_str(&format!("- `{}`  ❌ NO LONGER EXISTS\n", key)),
                }
            }
            s.push('\n');
        }
    }

    // Invariants — always shown; these are the rules an agent must respect.
    s.push_str("## Invariants (must respect)\n\n");
    for inv in &m.invariants {
        s.push_str(&format!("- {}\n", inv));
    }
    s.push('\n');

    s.push_str(&format!(
        "---\n_Generated from HONE {} surface `{}`. Re-run `hone sync` after pulling HONE updates._\n",
        m.hone_version,
        &m.surface_hash[..m.surface_hash.len().min(12)]
    ));
    s
}

/// Exit code policy for CI: 2 on breaking, 0 otherwise (warnings are loud but
/// non-fatal). First sync is always 0.
pub fn exit_code(outcome: &SyncOutcome) -> i32 {
    match &outcome.diff {
        Some(d) if d.has_breaking() => 2,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn tiny_manifest(version: &str, hash: &str) -> Manifest {
        Manifest {
            schema_version: 1,
            hone_version: version.into(),
            chain_id: "hone".into(),
            surface_hash: hash.into(),
            entries: BTreeMap::new(),
            routes: BTreeMap::new(),
            invariants: vec!["No local submission without peers.".into()],
        }
    }

    #[test]
    fn first_sync_writes_files_and_exits_zero() {
        let dir = std::env::temp_dir().join(format!("hone_sync_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let m = tiny_manifest("1.0.0", "abc123");
        let outcome = sync_repo(&dir, &m).unwrap();
        assert!(outcome.first_sync);
        assert!(dir.join(CONSUMER_DOC).exists());
        assert!(dir.join(CONSUMER_LOCK).exists());
        assert_eq!(exit_code(&outcome), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn second_sync_reports_no_change() {
        let dir = std::env::temp_dir().join(format!("hone_sync_test2_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let m = tiny_manifest("1.0.0", "abc123");
        sync_repo(&dir, &m).unwrap();
        let outcome = sync_repo(&dir, &m).unwrap();
        assert!(!outcome.first_sync);
        assert_eq!(exit_code(&outcome), 0);
        let d = outcome.diff.expect("diff on second sync");
        assert!(d.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
