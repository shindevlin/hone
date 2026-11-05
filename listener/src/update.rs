//! Auto-update from https://honemesh.net/app/listener/version.json.
//! Trust chain (same contract as the install page and the HONE listener MCP):
//!   1. version.json.sig is an ed25519 signature over the exact version.json bytes by the
//!      pinned HONE listener release key (RELEASE_PUB_HEX, built in);
//!   2. version.json = {"name":"hone-listener","version",...,"files":{<name>:{"sha256"}}};
//!   3. the downloaded artifact's SHA-256 must equal files[<name>].sha256.
//! Nothing unverified is ever installed.
//! - Linux .deb installs: the verified .deb is staged with the manifest; a root systemd path
//!   unit (hone-listener-update.path) re-verifies everything and installs it.
//! - Linux user installs, Windows (install.ps1) and macOS: the executable replaces itself.
//! - Microsoft Store (MSIX): the Store updates the app; self-update is off.

use crate::config::{state_dir, Config};
use crate::{live, perch};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

/// HONE listener release key (ed25519 raw public key). The private key stays on grouchly.
pub const RELEASE_PUB_HEX: &str = "27ef54588674c3391e660056d873b89136e75125b5a433de0747191baa94a10d";

pub fn platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows-x86_64"
    } else if cfg!(target_os = "macos") {
        "macos-universal"
    } else if cfg!(target_arch = "aarch64") {
        "linux-aarch64"
    } else {
        "linux-x86_64"
    }
}

/// Release file this install updates from.
pub fn artifact_name(version: &str) -> String {
    match platform() {
        "windows-x86_64" => format!("hone-listener-{version}-windows-x86_64.exe"),
        "macos-universal" => format!("hone-listener-{version}-macos-universal"),
        p if is_deb_install() => format!("hone-listener_{version}_{}.deb", if p == "linux-aarch64" { "arm64" } else { "amd64" }),
        p => format!("hone-listener-{version}-{p}"),
    }
}

pub fn parse_sig(s: &[u8]) -> Option<Signature> {
    let t = String::from_utf8_lossy(s).trim().to_string();
    let b = if t.len() == 128 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        hex::decode(&t).ok()?
    } else {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(&t).ok()?
    };
    Signature::from_slice(&b).ok()
}

pub fn verify_manifest(bytes: &[u8], sig: &[u8]) -> bool {
    let Ok(pk) = hex::decode(RELEASE_PUB_HEX) else { return false };
    let Ok(pk) = <[u8; 32]>::try_from(pk.as_slice()) else { return false };
    let Ok(vk) = VerifyingKey::from_bytes(&pk) else { return false };
    parse_sig(sig).map(|s| vk.verify(bytes, &s).is_ok()).unwrap_or(false)
}

pub fn newer(a: &str, b: &str) -> bool {
    let p = |s: &str| s.split('.').map(|x| x.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    p(a) > p(b)
}

pub fn is_store_install() -> bool {
    std::env::current_exe().map(|p| p.to_string_lossy().contains("\\WindowsApps\\")).unwrap_or(false)
}

pub fn is_deb_install() -> bool {
    cfg!(target_os = "linux") && std::env::current_exe().map(|p| p.starts_with("/usr/lib/hone-listener")).unwrap_or(false)
}

fn base(cfg: &Config) -> String {
    cfg.update_url.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default()
}

fn get(url: &str, max: u64) -> Option<Vec<u8>> {
    let r = crate::account::agent().get(url).timeout(Duration::from_secs(600)).call().ok()?;
    let mut b = Vec::new();
    r.into_reader().take(max).read_to_end(&mut b).ok()?;
    Some(b)
}

/// The verified manifest and the artifact's expected sha256, if a newer release exists.
pub fn latest(cfg: &Config) -> Result<Option<(Vec<u8>, Vec<u8>, String, String, String)>, String> {
    let man = get(&cfg.update_url, 64 * 1024).ok_or("update check failed (offline?)")?;
    let sig = get(&format!("{}/version.json.sig", base(cfg)), 4096).ok_or("update check failed (no signature)")?;
    if !verify_manifest(&man, &sig) {
        log::error!("version.json signature does not verify against the HONE release key; ignored");
        return Err("an update was offered but its signature did not verify; ignored".into());
    }
    let m: Value = serde_json::from_slice(&man).map_err(|_| "update manifest unreadable")?;
    if m["name"] != "hone-listener" {
        return Err("update manifest is not for this app".into());
    }
    let v = m["version"].as_str().unwrap_or("0").to_string();
    if !newer(&v, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let name = artifact_name(&v);
    let sha = m["files"][&name]["sha256"].as_str().map(|s| s.to_lowercase()).filter(|s| s.len() == 64).ok_or(format!("version {v} is out, not yet for this computer"))?;
    Ok(Some((man, sig, v, name, sha)))
}

/// Checks once; returns a short state line.
pub fn check(cfg: &Config, apply: bool) -> String {
    if is_store_install() {
        return "updates come from the Microsoft Store".into();
    }
    let (man, sig, v, name, sha) = match latest(cfg) {
        Ok(Some(x)) => x,
        Ok(None) => return format!("up to date ({})", env!("CARGO_PKG_VERSION")),
        Err(e) => return e,
    };
    if !apply {
        return format!("version {v} is available");
    }
    let Some(bytes) = get(&format!("{}/{name}", base(cfg)), 400 << 20) else { return "update download failed; will retry".into() };
    use sha2::{Digest, Sha256};
    if hex::encode(Sha256::digest(&bytes)) != sha {
        return "update download did not match the signed checksum; ignored".into();
    }
    let dir = state_dir().join("update");
    let _ = std::fs::create_dir_all(&dir);
    if is_deb_install() {
        let ok = std::fs::write(dir.join("version.json"), &man).is_ok() && std::fs::write(dir.join("version.json.sig"), &sig).is_ok()
            && std::fs::write(dir.join("hone-listener.deb"), &bytes).is_ok();
        if !ok {
            return "could not stage the update".into();
        }
        let _ = std::fs::write(dir.join("ready"), &name);
        return format!("version {v} staged; installing");
    }
    let tmp = dir.join(if cfg!(windows) { "hone-listener.new.exe" } else { "hone-listener.new" });
    if std::fs::write(&tmp, &bytes).is_err() {
        return "could not stage the update".into();
    }
    match self_replace::self_replace(&tmp) {
        Ok(()) => {
            let _ = std::fs::remove_file(&tmp);
            log::info!("updated to {v}; restarting");
            restart();
            format!("updated to {v}")
        }
        Err(e) => format!("update failed: {e}"),
    }
}

fn restart() {
    if let Ok(exe) = std::env::current_exe() {
        // launchd/systemd restart us when we exit; elsewhere start the new copy first.
        crate::voiceprint::save_now();
        if std::env::var("HONE_LISTENER_SUPERVISED").is_err() {
            let _ = std::process::Command::new(exe).args(std::env::args().skip(1)).spawn();
        }
        std::process::exit(0);
    }
}

/// Root side of the Linux updater: re-verify the staged manifest and .deb, then install.
pub fn apply_staged_deb() -> Result<String, String> {
    let dir = state_dir().join("update");
    let name = std::fs::read_to_string(dir.join("ready")).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(dir.join("ready"));
    let man = std::fs::read(dir.join("version.json")).map_err(|e| e.to_string())?;
    let sig = std::fs::read(dir.join("version.json.sig")).map_err(|e| e.to_string())?;
    let deb = dir.join("hone-listener.deb");
    let safe = std::env::temp_dir().join(format!("hone-listener-update-{}.deb", std::process::id()));
    let res = (|| {
        if !verify_manifest(&man, &sig) {
            return Err("staged manifest signature does not verify".to_string());
        }
        let m: Value = serde_json::from_slice(&man).map_err(|e| e.to_string())?;
        let v = m["version"].as_str().unwrap_or("0");
        if m["name"] != "hone-listener" || !newer(v, env!("CARGO_PKG_VERSION")) || artifact_name(v) != name.trim() {
            return Err("staged update is not a newer release for this computer".into());
        }
        let want = m["files"][name.trim()]["sha256"].as_str().unwrap_or("").to_lowercase();
        // copy out of the service-writable directory first, then check the copy we install
        std::fs::copy(&deb, &safe).map_err(|e| e.to_string())?;
        if perch::sha256_file(&safe).map_err(|e| e.to_string())? != want {
            return Err("staged .deb does not match the signed checksum".into());
        }
        let st = std::process::Command::new("apt-get").args(["install", "-y", "--no-install-recommends"]).arg(&safe).status().map_err(|e| e.to_string())?;
        if st.success() { Ok(format!("installed {v}")) } else { Err("apt-get install failed".into()) }
    })();
    let _ = std::fs::remove_file(&safe);
    let _ = std::fs::remove_file(&deb);
    res
}

pub fn spawn() {
    std::thread::Builder::new()
        .name("hone-update".into())
        .spawn(|| {
            std::thread::sleep(Duration::from_secs(90));
            loop {
                let cfg = Config::load();
                let s = check(&cfg, cfg.auto_update);
                live::set(|l| l.update_state = Some(s));
                std::thread::sleep(Duration::from_secs(6 * 3600 + rand::random::<u64>() % 1800));
            }
        })
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn version_order() {
        assert!(newer("0.1.10", "0.1.9"));
        assert!(!newer("0.1.0", "0.1.0"));
    }

    #[test]
    fn manifest_needs_the_pinned_key() {
        let other = SigningKey::from_bytes(&[3u8; 32]);
        let man = br#"{"name":"hone-listener","version":"9.9.9","files":{}}"#;
        let sig = hex::encode(other.sign(man).to_bytes());
        assert!(!verify_manifest(man, sig.as_bytes()));
        assert!(parse_sig(sig.as_bytes()).is_some());
    }
}
