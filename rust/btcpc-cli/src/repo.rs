use anyhow::{anyhow, bail, Context, Result};
use btcpc_sdk::KeyPair;
use colored::Colorize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::api::ApiClient;
use crate::session;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Save a repo keypair to ~/.btcpc/repos/{name}.key.json.
fn save_repo_key(repo_name: &str, account: &str, private_key_hex: &str) -> Result<()> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let dir = PathBuf::from(home).join(".btcpc").join("repos");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.key.json", repo_name));
    let json = serde_json::to_string_pretty(&json!({
        "account": account,
        "private_key_hex": private_key_hex,
    }))?;
    std::fs::write(&path, &json)
        .with_context(|| format!("write {}", path.display()))?;
    println!("{} {}", "Repo key saved:".bold(), path.display());
    Ok(())
}

fn resolve_key_file(key_file: Option<&Path>) -> Result<PathBuf> {
    session::resolve_key_file(key_file, session::load().as_ref())
}

fn load_keypair(path: &Path) -> Result<KeyPair> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read key file {}", path.display()))?;
    let v: Value = serde_json::from_str(&raw)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;
    if let Some(hex) = v.get("btcpc_active_private_key").and_then(|h| h.as_str()).filter(|h| !h.is_empty()) {
        return KeyPair::from_hex(hex)
            .with_context(|| format!("bad btcpc_active_private_key in {}", path.display()));
    }
    if let Some(hex) = v.get("private_key_hex").and_then(|h| h.as_str()).filter(|h| !h.is_empty()) {
        return KeyPair::from_hex(hex)
            .with_context(|| format!("bad private_key_hex in {}", path.display()));
    }
    Err(anyhow!(
        "{}: unrecognised key file — expected wallet.key (btcpc_active_private_key) or key.json (private_key_hex)",
        path.display()
    ))
}

fn resolve_account(explicit: Option<&str>) -> Result<String> {
    if let Some(a) = explicit {
        return Ok(a.to_owned());
    }
    if let Some(s) = session::load() {
        return Ok(s.account);
    }
    bail!("no account specified — pass --account or run `btcpc login` first")
}

fn node_chain_id(api: &ApiClient) -> Result<String> {
    let info: Value = api.get("/api/node/info")?;
    info.get("chain_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("node info missing chain_id"))
}

fn node_base_url(api: &ApiClient) -> String {
    api.base_url.clone()
}

/// Sign a LinkGitRepoCreate entry (posting key).
fn sign_repo_create(keypair: &KeyPair, chain_id: &str, repo_id: &str, owner: &str, name: &str, visibility: &str) -> String {
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": "LINKGIT_REPO_CREATE",
        "repo_id": repo_id,
        "owner": owner,
        "name": name,
        "visibility": visibility,
    })).unwrap_or_default();
    keypair.sign_entry_json(&msg)
}

// ── btcpc repo create ─────────────────────────────────────────────────────────

pub fn cmd_repo_create(
    name: &str,
    account: Option<&str>,
    private: bool,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let owner = resolve_account(account)?;
    let key_path = resolve_key_file(key_file)?;
    let keypair = load_keypair(&key_path)?;
    let chain_id = node_chain_id(&api)?;

    let visibility = if private { "private" } else { "public" };
    let repo_id = format!("{}/{}", owner, name);
    let sig = sign_repo_create(&keypair, &chain_id, &repo_id, &owner, name, visibility);

    let body = json!({
        "repo_id": repo_id,
        "name": name,
        "visibility": visibility,
        "signed_by": owner,
        "signature": sig,
    });

    let _resp: Value = api.post("/api/linkgit/repo/create", &body)?;

    // Create the repo's on-chain wallet (account "{owner}.{name}") stamped with
    // the node's machine fingerprint so self-dealing bids are blocked.
    let wallet_path = format!("/api/linkgit/repo/{}/{}/wallet", owner, name);
    let wallet_resp: Value = api.post(&wallet_path, &json!({}))?;
    let repo_account = wallet_resp.get("account").and_then(|v| v.as_str()).unwrap_or("");
    let repo_privkey = wallet_resp.get("private_key_hex").and_then(|v| v.as_str()).unwrap_or("");
    if !repo_account.is_empty() {
        save_repo_key(name, repo_account, repo_privkey)?;
    }

    let git_url = format!("{}/git/{}/{}", node_base_url(&api), owner, name);
    println!("{}", "Repository created.".green().bold());
    println!("{} {}/{}", "Repo:".bold(), owner, name);
    println!("{} {}", "Visibility:".bold(), visibility);
    println!("{} {}", "Chain account:".bold(), if repo_account.is_empty() { "n/a" } else { repo_account });
    println!("{} {}", "Git URL:".bold(), git_url);
    println!();
    println!("To use this repo:");
    println!("  git remote add origin {}", git_url);
    println!("  git push -u origin main");
    Ok(())
}

// ── btcpc repo init ───────────────────────────────────────────────────────────

pub fn cmd_repo_init(
    name: &str,
    account: Option<&str>,
    private: bool,
    key_file: Option<&Path>,
) -> Result<()> {
    // Create on-chain first.
    let api = ApiClient::new();
    let owner = resolve_account(account)?;
    let key_path = resolve_key_file(key_file)?;
    let keypair = load_keypair(&key_path)?;
    let chain_id = node_chain_id(&api)?;

    let visibility = if private { "private" } else { "public" };
    let repo_id = format!("{}/{}", owner, name);
    let sig = sign_repo_create(&keypair, &chain_id, &repo_id, &owner, name, visibility);

    let body = json!({
        "repo_id": repo_id,
        "name": name,
        "visibility": visibility,
        "signed_by": owner,
        "signature": sig,
    });
    let _resp: Value = api.post("/api/linkgit/repo/create", &body)?;

    // Create the repo's on-chain wallet stamped with the machine fingerprint.
    let wallet_path = format!("/api/linkgit/repo/{}/{}/wallet", owner, name);
    if let Ok(wallet_resp) = api.post::<Value>(&wallet_path, &json!({})) {
        let repo_account = wallet_resp.get("account").and_then(|v| v.as_str()).unwrap_or("");
        let repo_privkey = wallet_resp.get("private_key_hex").and_then(|v| v.as_str()).unwrap_or("");
        if !repo_account.is_empty() {
            let _ = save_repo_key(name, repo_account, repo_privkey);
        }
    }

    let git_url = format!("{}/git/{}/{}", node_base_url(&api), owner, name);

    // Init git repo if not already one.
    let is_git = std::process::Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !is_git {
        let status = std::process::Command::new("git").arg("init").status()?;
        if !status.success() {
            bail!("git init failed");
        }
    }

    // Check if 'origin' already exists.
    let origin_check = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()?;

    if origin_check.status.success() {
        let existing = String::from_utf8_lossy(&origin_check.stdout).trim().to_string();
        if existing != git_url {
            bail!("'origin' already points to {}\nSet a different remote name or remove it first.", existing);
        }
        println!("{}", "origin already set correctly.".yellow());
    } else {
        let status = std::process::Command::new("git")
            .args(["remote", "add", "origin", &git_url])
            .status()?;
        if !status.success() {
            bail!("git remote add failed");
        }
    }

    println!("{}", "Repository ready.".green().bold());
    println!("{} {}/{}", "Repo:".bold(), owner, name);
    println!("{} {}", "Visibility:".bold(), visibility);
    println!("{} {}", "Remote:".bold(), git_url);
    println!();
    println!("Next:");
    println!("  git add .");
    println!("  git commit -m 'initial commit'");
    println!("  git push -u origin main");
    Ok(())
}

// ── btcpc repo list ───────────────────────────────────────────────────────────

pub fn cmd_repo_list(owner: Option<&str>) -> Result<()> {
    let api = ApiClient::new();
    let owner = resolve_account(owner)?;

    let path = format!("/api/linkgit/repos/{}", owner);
    let resp: Value = api.get(&path)?;
    let base = node_base_url(&api);

    let repos = resp.as_array().cloned().unwrap_or_default();
    if repos.is_empty() {
        println!("No repositories for {}.", owner);
        return Ok(());
    }

    println!("{}", format!("Repositories for {}:", owner).bold());
    for r in &repos {
        let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let vis = r.get("visibility").and_then(|v| v.as_str()).unwrap_or("?");
        let refs_count = r.get("refs").and_then(|v| v.as_object()).map(|m| m.len()).unwrap_or(0);
        let git_url = format!("{}/git/{}/{}", base, owner, name);
        println!("  {} [{}]  {} refs", name.bold(), vis, refs_count);
        println!("    {}", git_url.dimmed());
    }
    Ok(())
}

// ── btcpc repo info ───────────────────────────────────────────────────────────

pub fn cmd_repo_info(owner: &str, name: &str) -> Result<()> {
    let api = ApiClient::new();
    let path = format!("/api/linkgit/repo/{}/{}", owner, name);
    let resp: Value = api.get(&path)?;
    let base = node_base_url(&api);

    let vis = resp.get("visibility").and_then(|v| v.as_str()).unwrap_or("?");
    let git_url = format!("{}/git/{}/{}", base, owner, name);

    println!("{} {}/{}", "Repo:".bold(), owner, name);
    println!("{} {}", "Visibility:".bold(), vis);
    println!("{} {}", "Git URL:".bold(), git_url);

    if let Some(refs) = resp.get("refs").and_then(|v| v.as_object()) {
        if refs.is_empty() {
            println!("{} (no refs yet)", "Refs:".bold());
        } else {
            println!("{}",  "Refs:".bold());
            let mut sorted: Vec<_> = refs.iter().collect();
            sorted.sort_by_key(|(k, _)| k.clone());
            for (refname, sha) in sorted {
                let sha_str = sha.as_str().unwrap_or("?");
                let short = if sha_str.len() >= 8 { &sha_str[..8] } else { sha_str };
                println!("  {}  {}", short.yellow(), refname);
            }
        }
    }

    println!();
    println!("  git clone {}", git_url);
    Ok(())
}

// ── btcpc repo clone ──────────────────────────────────────────────────────────

pub fn cmd_repo_clone(owner: &str, name: &str, dir: Option<&str>) -> Result<()> {
    let api = ApiClient::new();
    let git_url = format!("{}/git/{}/{}", node_base_url(&api), owner, name);

    let mut cmd = std::process::Command::new("git");
    cmd.arg("clone").arg(&git_url);
    if let Some(d) = dir {
        cmd.arg(d);
    }

    println!("Cloning {} ...", git_url.bold());
    let status = cmd.status().context("failed to run git clone")?;
    if !status.success() {
        bail!("git clone failed");
    }
    Ok(())
}
