//! `hone` — the ecosystem CLI. Its job here is the integration manifest:
//! generate it (in the hone repo), enforce it in CI, and sync it into any
//! consumer repo so that repo stays correct across HONE updates.
//!
//! Subcommands:
//!   hone manifest generate [--repo <dir>] [--out hone-manifest.json]
//!       Regenerate the manifest from the HONE source tree.
//!
//!   hone manifest check [--repo <dir>]
//!       CI gate: regenerate and compare against the committed
//!       hone-manifest.json. Exit non-zero if they differ (the API changed but
//!       the manifest wasn't updated in the same commit).
//!
//!   hone manifest diff <old.json> <new.json>
//!       Print the ADDED / REMOVED / DEPRECATED / breaking changelog.
//!
//!   hone sync [--node <url> | --manifest <path>] [--dir <consumer repo>]
//!       Consumer side: refresh HONE.md + HONE.lock in the current repo and
//!       print what changed since last sync. Exit 2 on breaking changes.
//!
//! No arg parser dependency is added — a tiny hand-rolled parser keeps the SDK's
//! dependency surface small (this binary ships to consumer CI).

use anyhow::{anyhow, bail, Context, Result};
use hone_sdk::manifest::{
    diff::diff_manifests, generate, schema::Manifest, sync, MANIFEST_FILENAME,
};
use std::path::{Path, PathBuf};
use std::process::exit;

fn main() {
    match run() {
        Ok(code) => exit(code),
        Err(e) => {
            eprintln!("hone: error: {e:#}");
            exit(1);
        }
    }
}

fn run() -> Result<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut it = args.iter();
    match it.next().map(String::as_str) {
        Some("manifest") => match it.next().map(String::as_str) {
            Some("generate") => cmd_generate(&args[2..]),
            Some("check") => cmd_check(&args[2..]),
            Some("diff") => cmd_diff(&args[2..]),
            _ => {
                usage();
                Ok(1)
            }
        },
        Some("sync") => cmd_sync(&args[1..]),
        Some("wallet") => wallet_cmd::run(&args[1..]),
        Some("--help") | Some("-h") | None => {
            usage();
            Ok(0)
        }
        Some(other) => {
            eprintln!("hone: unknown command '{other}'");
            usage();
            Ok(1)
        }
    }
}

fn usage() {
    eprintln!(
        "hone — HONE ecosystem CLI\n\n\
         USAGE:\n\
         \x20 hone manifest generate [--repo DIR] [--out FILE] [--chain-id ID]\n\
         \x20 hone manifest check    [--repo DIR] [--chain-id ID]\n\
         \x20 hone manifest diff     OLD.json NEW.json\n\
         \x20 hone sync              [--manifest FILE | --node URL] [--dir DIR] [--chain-id ID]\n\
         \x20 hone wallet new        --account NAME --vault DIR   (creates a recoverable keystore)\n\
         \x20 hone wallet import     --account NAME --vault DIR   (from an existing mnemonic)\n\
         \x20 hone wallet unlock     --keystore FILE              (prints PUBLIC keys only)\n\
         \x20 hone wallet pubkeys    --keystore FILE              (public keys for genesis)\n"
    );
}

#[path = "wallet_cmd.rs"]
mod wallet_cmd;

// ── flag parsing (tiny) ─────────────────────────────────────────────────────

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            return args.get(i + 1).map(String::as_str);
        }
        i += 1;
    }
    None
}

fn positionals(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.starts_with("--") {
            i += 2; // skip flag + value
        } else {
            out.push(a.as_str());
            i += 1;
        }
    }
    out
}

fn repo_root(args: &[String]) -> PathBuf {
    flag(args, "--repo")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn chain_id(args: &[String]) -> String {
    flag(args, "--chain-id").unwrap_or("hone").to_string()
}

// ── commands ────────────────────────────────────────────────────────────────

fn cmd_generate(args: &[String]) -> Result<i32> {
    let root = repo_root(args);
    let out = flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(MANIFEST_FILENAME));
    let m = generate::generate_manifest(&root, &chain_id(args))
        .context("generating manifest from source")?;
    std::fs::write(&out, generate::to_json(&m)?)
        .with_context(|| format!("writing {}", out.display()))?;
    let (stable, dep, exp) = m.stability_counts();
    println!(
        "Generated {} — {} entries, {} routes ({stable} stable, {dep} deprecated, {exp} experimental)\nsurface {}",
        out.display(),
        m.entries.len(),
        m.routes.len(),
        m.surface_hash
    );
    Ok(0)
}

fn cmd_check(args: &[String]) -> Result<i32> {
    let root = repo_root(args);
    let committed_path = root.join(MANIFEST_FILENAME);
    let fresh = generate::generate_manifest(&root, &chain_id(args))
        .context("generating manifest from source")?;

    if !committed_path.exists() {
        eprintln!(
            "FAIL: {} does not exist. Run `hone manifest generate` and commit it.",
            committed_path.display()
        );
        return Ok(2);
    }
    let committed: Manifest = load_manifest(&committed_path)?;

    if committed.surface_hash == fresh.surface_hash
        && committed.hone_version == fresh.hone_version
    {
        println!("OK: committed manifest matches source (surface {}).", fresh.surface_hash);
        return Ok(0);
    }

    // Show exactly what's out of date.
    eprintln!("FAIL: committed manifest is stale — the source surface changed but the manifest wasn't regenerated.\n");
    let d = diff_manifests(&committed, &fresh, None);
    eprintln!("{}", sync::render_changelog(&d));
    eprintln!("\nFix: run `hone manifest generate` and commit {}.", MANIFEST_FILENAME);
    Ok(2)
}

fn cmd_diff(args: &[String]) -> Result<i32> {
    let pos = positionals(args);
    if pos.len() != 2 {
        bail!("usage: hone manifest diff OLD.json NEW.json");
    }
    let old = load_manifest(Path::new(pos[0]))?;
    let new = load_manifest(Path::new(pos[1]))?;
    let d = diff_manifests(&old, &new, None);
    println!("{}", sync::render_changelog(&d));
    Ok(if d.has_breaking() { 2 } else { 0 })
}

fn cmd_sync(args: &[String]) -> Result<i32> {
    let dir = flag(args, "--dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let new_manifest = if let Some(path) = flag(args, "--manifest") {
        load_manifest(Path::new(path))?
    } else if let Some(node) = flag(args, "--node") {
        fetch_manifest_from_node(node)?
    } else {
        bail!("provide --manifest <path> or --node <url>");
    };

    let outcome = sync::sync_repo(&dir, &new_manifest)?;

    if outcome.first_sync {
        println!(
            "Initialized HONE.md + HONE.lock (HONE {} surface {}).\n\
             Tip: list the entries/routes you use under uses_entries/uses_routes in HONE.lock \
             to get targeted change alerts.",
            new_manifest.hone_version,
            &new_manifest.surface_hash[..new_manifest.surface_hash.len().min(12)]
        );
    } else if let Some(d) = &outcome.diff {
        println!("{}", sync::render_changelog(d));
        if d.has_breaking() {
            eprintln!(
                "\nhone sync: BREAKING changes above affect surface this repo uses. \
                 Update your integration before deploying."
            );
        }
    }
    Ok(sync::exit_code(&outcome))
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn load_manifest(path: &Path) -> Result<Manifest> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parsing manifest {}", path.display()))
}

/// Fetch the live manifest from a node's `GET /api/integration/manifest`.
/// Blocking reqwest to keep this binary free of an async runtime requirement.
fn fetch_manifest_from_node(node: &str) -> Result<Manifest> {
    let base = node.trim_end_matches('/');
    let url = format!("{base}/api/integration/manifest");
    let body = reqwest::blocking::get(&url)
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("node returned error for {url}"))?
        .text()
        .context("reading manifest response body")?;
    serde_json::from_str(&body)
        .map_err(|e| anyhow!("node manifest did not parse (is the node on a version that serves it?): {e}"))
}
