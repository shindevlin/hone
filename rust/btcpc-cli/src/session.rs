use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub account: String,
    pub key_file: PathBuf,
    pub node_url: String,
}

fn session_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .context("HOME is not set; cannot locate session file")?;
    Ok(PathBuf::from(home).join(".btcpc").join("session.json"))
}

pub fn load() -> Option<Session> {
    let path = session_path().ok()?;
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save(s: &Session) -> Result<()> {
    let path = session_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(s)?;
    std::fs::write(&path, json)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn clear() -> Result<()> {
    let path = session_path()?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

/// Resolve an account name: explicit arg > session account
#[allow(dead_code)]
pub fn resolve_account<'a>(explicit: Option<&'a str>, sess: Option<&'a Session>) -> Result<&'a str> {
    if let Some(a) = explicit {
        return Ok(a);
    }
    if let Some(s) = sess {
        return Ok(&s.account);
    }
    anyhow::bail!("no account specified — pass an account name or run `btcpc login` first")
}

/// Resolve a key file: explicit arg > env > session > default ~/.btcpc/key.json
pub fn resolve_key_file(explicit: Option<&Path>, sess: Option<&Session>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    if let Ok(e) = std::env::var("BTCPC_KEY_FILE") {
        let t = e.trim().to_owned();
        if !t.is_empty() {
            return Ok(PathBuf::from(t));
        }
    }
    if let Some(s) = sess {
        return Ok(s.key_file.clone());
    }
    // fall back to default key location
    let home = std::env::var("HOME")
        .context("HOME is not set; pass --key-file explicitly")?;
    Ok(PathBuf::from(home).join(".btcpc").join("key.json"))
}
