//! Generate a [`Manifest`] from the node source tree.
//!
//! This is deliberately a **source parser**, not a runtime reflection: the
//! manifest must be reproducible from a git checkout alone, with no node
//! running and no network. Given the same source, it emits a byte-identical
//! manifest — which is what lets CI diff a committed manifest against a fresh
//! one and what makes cross-commit diffs meaningful.
//!
//! It parses three things:
//!   * routes  — the `.route("PATH", get|post(handler))` chain in `api.rs`
//!   * entries — the `LedgerEntry` enum variants + fields + doc comments in
//!               `entry.rs`
//!   * signing — the `canonical_signing_message` match arms in `tx.rs`, which
//!               define the authoritative signing shape per entry
//!
//! Parsing conventions (kept simple and regular on purpose — the source follows
//! a consistent style, and any deviation surfaces as a manifest diff in CI):
//!   * a route line is `.route("<path>", <method>(<handler>))`
//!   * a variant is `Ident {` preceded by `///` doc lines
//!   * a `/// @deprecated ...` line marks deprecation (see [`parse_deprecation`])

use crate::manifest::schema::*;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// Where the surface is defined, relative to the hone repo root.
pub const ENTRY_SRC: &str = "rust/hone-node/crates/hone-types/src/entry.rs";
pub const API_SRC: &str = "rust/hone-node/src/api.rs";
pub const TX_SRC: &str = "rust/hone-node/src/tx.rs";
pub const WORKSPACE_TOML: &str = "rust/Cargo.toml";

/// Hand-curated invariants every consumer must respect. Stable text; changes
/// here are intentional and show up as a manifest diff.
const INVARIANTS: &[&str] = &[
    "No local submission without peers: a node with zero peers rejects all user-submitted entries.",
    "Every user-submittable entry is signed; signed_by is bound to the owning account.",
    "The epoch is the block. System entries apply on epoch seal, not through the pending pool.",
    "Never treat a submitted entry as confirmed until it has sealed in an epoch.",
];

/// Generate a manifest by reading the source files under `repo_root`.
pub fn generate_manifest(repo_root: &Path, chain_id: &str) -> Result<Manifest> {
    let entry_src = read(repo_root, ENTRY_SRC)?;
    let api_src = read(repo_root, API_SRC)?;
    let tx_src = read(repo_root, TX_SRC)?;
    let ws_toml = read(repo_root, WORKSPACE_TOML)?;

    let hone_version = parse_workspace_version(&ws_toml)
        .unwrap_or_else(|| "unknown".to_string());

    let signing = parse_signing_arms(&tx_src);
    let bindings = parse_signed_by_bindings(&tx_src);
    let entries = parse_entries(&entry_src, &signing, &bindings);
    let routes = parse_routes(&api_src);

    let surface_hash = compute_surface_hash(&entries, &routes);

    Ok(Manifest {
        schema_version: crate::manifest::MANIFEST_SCHEMA_VERSION,
        hone_version,
        chain_id: chain_id.to_string(),
        surface_hash,
        entries,
        routes,
        invariants: INVARIANTS.iter().map(|s| s.to_string()).collect(),
    })
}

fn read(root: &Path, rel: &str) -> Result<String> {
    let p = root.join(rel);
    std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))
}

/// True for characters used to draw section-divider comment rules.
fn is_divider_char(ch: char) -> bool {
    matches!(ch, '─' | '━' | '-' | '=' | '·' | '•' | ' ')
}

/// Largest index <= `idx` that is a char boundary of `s` (std's `floor_char_boundary`
/// is still unstable). Source files contain multi-byte box-drawing chars, so any
/// fixed-length byte slice must be snapped to a boundary or it panics.
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// `[workspace.package]\nversion = "1.2.2"` → "1.2.2"
fn parse_workspace_version(toml: &str) -> Option<String> {
    let mut in_ws_pkg = false;
    for line in toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_ws_pkg = t == "[workspace.package]";
            continue;
        }
        if in_ws_pkg {
            if let Some(rest) = t.strip_prefix("version") {
                if let Some(v) = rest.split('"').nth(1) {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

// ── Routes ──────────────────────────────────────────────────────────────────

/// Parse `.route("<path>", get|post(handler))` lines. A `// comment` on the
/// immediately preceding line (that isn't a section divider) becomes the
/// route summary.
fn parse_routes(api_src: &str) -> BTreeMap<String, RouteSpec> {
    let mut routes = BTreeMap::new();
    let lines: Vec<&str> = api_src.lines().collect();

    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix(".route(") else { continue };
        // rest looks like: "PATH", get(handler)) ...  or  "PATH", get(a).post(b)) ...
        let Some(path) = rest.split('"').nth(1) else { continue };

        // Extract method(handler) segments: get(...), post(...)
        for method in ["get", "post"] {
            let pat = format!("{}(", method);
            if let Some(mpos) = rest.find(&pat) {
                let after = &rest[mpos + pat.len()..];
                let handler = after
                    .split(|c| c == ')' || c == '.')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let (stability, deprecation, summary) =
                    context_above(&lines, i);
                let key = format!("{} {}", method.to_uppercase(), path);
                routes.insert(
                    key,
                    RouteSpec {
                        method: method.to_uppercase(),
                        path: path.to_string(),
                        handler,
                        summary,
                        stability,
                        deprecation,
                    },
                );
            }
        }
    }
    routes
}

/// Look at the line(s) directly above index `i` for a `//`-comment summary and a
/// `@deprecated` marker. Section dividers (lines of box-drawing chars) are
/// ignored as summaries.
fn context_above(lines: &[&str], i: usize) -> (Stability, Option<Deprecation>, String) {
    let mut summary = String::new();
    let mut deprecation = None;
    if i == 0 {
        return (Stability::Stable, None, summary);
    }
    // Scan up to 3 lines up for a comment / marker.
    for back in 1..=3 {
        if i < back {
            break;
        }
        let prev = lines[i - back].trim();
        if let Some(dep) = prev
            .strip_prefix("//")
            .and_then(|c| parse_deprecation(c.trim()))
        {
            deprecation = Some(dep);
            continue;
        }
        if let Some(c) = prev.strip_prefix("//") {
            let c = c.trim_start_matches('/').trim();
            // Section-divider comments in this codebase come in two styles:
            //   "── Some Label ─────"  and  "───────────".
            // Strip leading/trailing runs of divider chars; if nothing readable
            // remains it was a pure rule (skip), otherwise keep the inner label.
            let cleaned = c
                .trim_matches(|ch| is_divider_char(ch))
                .trim();
            if !cleaned.is_empty() && summary.is_empty() {
                summary = cleaned.to_string();
            }
        } else if !prev.starts_with(".route(") {
            // Hit a non-comment, non-route line — stop scanning.
            break;
        }
    }
    let stability = if deprecation.is_some() {
        Stability::Deprecated
    } else {
        Stability::Stable
    };
    (stability, deprecation, summary)
}

// ── Entries ─────────────────────────────────────────────────────────────────

/// Parse `LedgerEntry` variants: `Ident {` with `///` doc lines above, fields
/// until the matching `},`.
fn parse_entries(
    entry_src: &str,
    signing: &BTreeMap<String, (String, Vec<String>)>,
    bindings: &BTreeMap<String, String>,
) -> BTreeMap<String, EntrySpec> {
    let mut entries = BTreeMap::new();
    let lines: Vec<&str> = entry_src.lines().collect();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        // A variant header: "Ident {" (capitalized, not a fn/struct/impl).
        if let Some(name) = variant_name(line) {
            // Gather doc comments immediately above.
            let (summary, deprecation) = docs_above(&lines, i);
            // Gather fields until the closing "}," at the same brace depth.
            let (fields, next) = parse_fields(&lines, i);
            i = next;

            let sig = signing.get(&name);
            let signing_type = sig.map(|(t, _)| t.clone());
            let signing_fields = sig.map(|(_, f)| f.clone()).unwrap_or_default();
            let system_only = signing_type.is_none()
                && is_system_only_variant(&name, &fields);
            let stability = if deprecation.is_some() {
                Stability::Deprecated
            } else {
                Stability::Stable
            };

            entries.insert(
                name.clone(),
                EntrySpec {
                    name: name.clone(),
                    summary,
                    fields,
                    signing_type,
                    signing_fields,
                    signed_by_bound_to: bindings.get(&name).cloned(),
                    system_only,
                    stability,
                    deprecation,
                },
            );
        } else {
            i += 1;
        }
    }
    entries
}

/// A `LedgerEntry` variant header is `SomeIdent {` — capitalized first letter,
/// ends with `{`, and is not a keyword-led line.
fn variant_name(line: &str) -> Option<String> {
    let l = line.trim_end();
    if !l.ends_with('{') {
        return None;
    }
    let ident = l.trim_end_matches('{').trim();
    // Reject compound headers (fn, struct, impl, match arms with `=>`, etc.)
    if ident.contains(' ') || ident.contains('(') || ident.contains(':') || ident.contains('<') {
        return None;
    }
    let mut chars = ident.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return None,
    }
    if ident.chars().all(|c| c.is_alphanumeric()) {
        Some(ident.to_string())
    } else {
        None
    }
}

/// Doc comments (`///`) directly above index `i`; returns (summary, deprecation).
fn docs_above(lines: &[&str], i: usize) -> (String, Option<Deprecation>) {
    let mut summary_parts: Vec<String> = Vec::new();
    let mut deprecation = None;
    let mut back = 1;
    while i >= back {
        let prev = lines[i - back].trim();
        let Some(doc) = prev.strip_prefix("///") else { break };
        let doc = doc.trim();
        if let Some(dep) = parse_deprecation(doc) {
            deprecation = Some(dep);
        } else if !doc.is_empty() {
            summary_parts.push(doc.to_string());
        }
        back += 1;
    }
    summary_parts.reverse();
    // Use only the first non-empty doc line as the summary.
    let summary = summary_parts.into_iter().next().unwrap_or_default();
    (summary, deprecation)
}

/// Parse fields of a variant starting at its header line `i`. Returns the fields
/// and the index just past the variant's closing brace.
fn parse_fields(lines: &[&str], header: usize) -> (Vec<SigningField>, usize) {
    let mut fields = Vec::new();
    let mut i = header + 1;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.starts_with("},") || line == "}" {
            i += 1;
            break;
        }
        // A field is `name: Type,` (skip doc comments and attributes).
        if !line.starts_with("///") && !line.starts_with("#[") {
            if let Some((name, ty)) = parse_field(line) {
                fields.push(SigningField { name, ty });
            }
        }
        i += 1;
    }
    (fields, i)
}

/// `name: Type,` → (name, Type). Handles generic types with commas by taking
/// everything after the first colon up to the trailing comma.
fn parse_field(line: &str) -> Option<(String, String)> {
    let colon = line.find(':')?;
    let name = line[..colon].trim();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let ty = line[colon + 1..].trim().trim_end_matches(',').trim().to_string();
    if ty.is_empty() {
        return None;
    }
    Some((name.to_string(), ty))
}

/// Heuristic: a variant with no signing arm and no `signed_by` field is a
/// system-emitted entry (rewards, seals) a consumer cannot submit.
fn is_system_only_variant(_name: &str, fields: &[SigningField]) -> bool {
    !fields.iter().any(|f| f.name == "signed_by")
}

// ── Signing arms (tx.rs) ────────────────────────────────────────────────────

/// Parse `canonical_signing_message`'s match arms:
///   `LedgerEntry::Foo { a, b, signed_by, .. } => serde_json::json!({`
///   `    "chain_id": chain_id, "type": "FOO", "a": a, ... })`
/// Returns variant → (signing_type, [signing field names in order]).
fn parse_signing_arms(tx_src: &str) -> BTreeMap<String, (String, Vec<String>)> {
    let mut out = BTreeMap::new();
    let text = tx_src;

    // Find the canonical_signing_message fn body region to avoid false matches.
    let region = match text.find("fn canonical_signing_message") {
        Some(start) => &text[start..],
        None => text,
    };

    let bytes: Vec<char> = region.chars().collect();
    let s: String = bytes.into_iter().collect();
    // Walk arms: split on "LedgerEntry::"
    for arm in s.split("LedgerEntry::").skip(1) {
        // variant name is up to the first non-ident char
        let name: String = arm
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        // Find the json!({ ... }) block for this arm (first one after the arm).
        let Some(jpos) = arm.find("json!({") else { continue };
        // Bound the json block to before the next arm (already split) — take a
        // reasonable window.
        let json_region = &arm[jpos..];
        let end = json_region.find("})").map(|e| e + 2).unwrap_or(json_region.len());
        let json_block = &json_region[..end];

        // Extract "type": "FOO"
        let signing_type = extract_json_type(json_block);
        // Extract the ordered list of "key": ... pairs.
        let fields = extract_json_keys(json_block);
        if let Some(t) = signing_type {
            out.insert(name, (t, fields));
        }
    }
    out
}

/// From a `json!({ "chain_id": chain_id, "type": "FOO", ... })` block, get "FOO".
fn extract_json_type(block: &str) -> Option<String> {
    let key = "\"type\":";
    let pos = block.find(key)? + key.len();
    let after = block[pos..].trim_start();
    // after starts with "FOO"
    after.split('"').nth(1).map(|s| s.to_string())
}

/// Ordered list of quoted keys in the json block (the signing field names).
fn extract_json_keys(block: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = block;
    while let Some(q1) = rest.find('"') {
        let after = &rest[q1 + 1..];
        if let Some(q2) = after.find('"') {
            let key = &after[..q2];
            // A key is followed (after optional space) by ':'
            let tail = after[q2 + 1..].trim_start();
            if tail.starts_with(':') && !key.is_empty() && key.chars().all(|c| {
                c.is_alphanumeric() || c == '_'
            }) {
                keys.push(key.to_string());
            }
            rest = &after[q2 + 1..];
        } else {
            break;
        }
    }
    keys
}

/// Parse `if signed_by != owner { bail!... }`-style bindings from
/// `validate_and_apply` arms → variant → bound field.
fn parse_signed_by_bindings(tx_src: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for arm in tx_src.split("LedgerEntry::").skip(1) {
        let name: String = arm
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        // Look for `signed_by != <field>` within the first ~400 bytes of the arm.
        // Slice on a char boundary — source contains multi-byte box-drawing chars.
        let window = &arm[..floor_char_boundary(arm, 400)];
        if let Some(pos) = window.find("signed_by != ") {
            let after = &window[pos + "signed_by != ".len()..];
            let field: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !field.is_empty() && !out.contains_key(&name) {
                out.insert(name, field);
            }
        }
    }
    out
}

// ── Deprecation marker ──────────────────────────────────────────────────────

/// Parse a `@deprecated` marker from a doc/comment line.
/// Format: `@deprecated since=1.3.0 use=SensorDataCommit remove=2.0.0 reason="..."`
/// All fields optional; `reason` may be quoted and contain spaces.
pub fn parse_deprecation(doc: &str) -> Option<Deprecation> {
    let idx = doc.find("@deprecated")?;
    let rest = doc[idx + "@deprecated".len()..].trim();
    let mut dep = Deprecation {
        since: None,
        replacement: None,
        reason: None,
        remove_in: None,
    };
    // Pull reason="..." first (may contain spaces).
    let mut scan = rest.to_string();
    if let Some(rpos) = scan.find("reason=\"") {
        let after = &scan[rpos + "reason=\"".len()..];
        if let Some(endq) = after.find('"') {
            dep.reason = Some(after[..endq].to_string());
            // Remove the reason token from scan so key=val parsing is clean.
            scan = format!("{}{}", &scan[..rpos], &after[endq + 1..]);
        }
    }
    for tok in scan.split_whitespace() {
        if let Some(v) = tok.strip_prefix("since=") {
            dep.since = Some(v.trim_matches('"').to_string());
        } else if let Some(v) = tok.strip_prefix("use=") {
            dep.replacement = Some(v.trim_matches('"').to_string());
        } else if let Some(v) = tok.strip_prefix("remove=") {
            dep.remove_in = Some(v.trim_matches('"').to_string());
        }
    }
    Some(dep)
}

// ── Surface hash ────────────────────────────────────────────────────────────

/// Deterministic hash over the surface (entries + routes), independent of the
/// hone_version so a version bump alone doesn't churn the hash. Consumers pin
/// this to detect any surface change.
fn compute_surface_hash(
    entries: &BTreeMap<String, EntrySpec>,
    routes: &BTreeMap<String, RouteSpec>,
) -> String {
    let mut h = Sha256::new();
    for (k, e) in entries {
        h.update(b"E");
        h.update(k.as_bytes());
        if let Some(t) = &e.signing_type {
            h.update(t.as_bytes());
        }
        for f in &e.signing_fields {
            h.update(b"|");
            h.update(f.as_bytes());
        }
        h.update(format!("{:?}", e.stability).as_bytes());
    }
    for (k, r) in routes {
        h.update(b"R");
        h.update(k.as_bytes());
        h.update(format!("{:?}", r.stability).as_bytes());
    }
    hex::encode(h.finalize())
}

/// Serialize a manifest to canonical pretty JSON (stable key order via BTreeMap).
pub fn to_json(m: &Manifest) -> Result<String> {
    Ok(serde_json::to_string_pretty(m)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_route_line() {
        let src = r#"
    Router::new()
        // Submit a signed sensor batch
        .route("/api/sensor/commit", post(post_sensor_commit))
        .route("/api/latest", get(get_latest))
"#;
        let routes = parse_routes(src);
        let commit = routes.get("POST /api/sensor/commit").expect("route present");
        assert_eq!(commit.path, "/api/sensor/commit");
        assert_eq!(commit.handler, "post_sensor_commit");
        assert_eq!(commit.summary, "Submit a signed sensor batch");
        assert!(routes.contains_key("GET /api/latest"));
    }

    #[test]
    fn parses_combined_get_post_route() {
        let src = r#"
        .route("/api/chain/min_node_version", get(get_min).post(post_min))
"#;
        let routes = parse_routes(src);
        assert!(routes.contains_key("GET /api/chain/min_node_version"));
        assert!(routes.contains_key("POST /api/chain/min_node_version"));
    }

    #[test]
    fn parses_deprecation_marker() {
        let dep = parse_deprecation(
            r#"@deprecated since=1.3.0 use=SensorDataCommit remove=2.0.0 reason="folded in""#,
        )
        .expect("parsed");
        assert_eq!(dep.since.as_deref(), Some("1.3.0"));
        assert_eq!(dep.replacement.as_deref(), Some("SensorDataCommit"));
        assert_eq!(dep.remove_in.as_deref(), Some("2.0.0"));
        assert_eq!(dep.reason.as_deref(), Some("folded in"));
    }

    #[test]
    fn parses_entry_variant_and_fields() {
        let src = r#"
    /// Register a sensor.
    SensorRegister {
        sensor_id: String,
        owner: AccountId,
        signed_by: AccountId,
    },
    /// System reward — consumers cannot submit this.
    SensorReward {
        owner: AccountId,
        amount: Hunits,
        epoch: Epoch,
    },
"#;
        let signing = BTreeMap::new();
        let bindings = BTreeMap::new();
        let entries = parse_entries(src, &signing, &bindings);
        let reg = entries.get("SensorRegister").expect("SensorRegister");
        assert_eq!(reg.summary, "Register a sensor.");
        assert_eq!(reg.fields.len(), 3);
        assert_eq!(reg.fields[0].name, "sensor_id");
        assert!(!reg.system_only); // has signed_by
        let rew = entries.get("SensorReward").expect("SensorReward");
        assert!(rew.system_only); // no signed_by
    }

    #[test]
    fn extracts_signing_type_and_keys() {
        let tx = r#"
pub fn canonical_signing_message(entry: &LedgerEntry, chain_id: &str) -> Result<String> {
    let v = match entry {
        LedgerEntry::SensorDataCommit { sensor_id, owner, batch_hash, reading_count, sensor_type, signed_by, .. } =>
            serde_json::json!({
                "chain_id": chain_id, "type": "SENSOR_DATA_COMMIT",
                "sensor_id": sensor_id, "owner": owner, "batch_hash": batch_hash,
                "reading_count": reading_count, "sensor_type": sensor_type, "signed_by": signed_by,
            }),
    };
}
"#;
        let arms = parse_signing_arms(tx);
        let (ty, fields) = arms.get("SensorDataCommit").expect("arm present");
        assert_eq!(ty, "SENSOR_DATA_COMMIT");
        assert_eq!(
            fields,
            &vec![
                "chain_id".to_string(),
                "type".to_string(),
                "sensor_id".to_string(),
                "owner".to_string(),
                "batch_hash".to_string(),
                "reading_count".to_string(),
                "sensor_type".to_string(),
                "signed_by".to_string(),
            ]
        );
    }

    #[test]
    fn extracts_signed_by_binding() {
        let tx = r#"
        LedgerEntry::SensorDataCommit { owner, signed_by, .. } => {
            if signed_by != owner { bail!("SensorDataCommit: signed_by must equal owner"); }
        }
"#;
        let b = parse_signed_by_bindings(tx);
        assert_eq!(b.get("SensorDataCommit").map(|s| s.as_str()), Some("owner"));
    }
}
