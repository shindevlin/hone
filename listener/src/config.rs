//! Paths and the saved settings (no secrets here: those live in secrets.rs).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 47811;
pub const API: &str = "https://honemesh.net/api";
pub const CHAIN: &str = "https://honemesh.net/chain";
pub const UPDATE_URL: &str = "https://honemesh.net/app/listener/version.json";

/// Where state lives: $HONE_LISTENER_STATE, systemd's $STATE_DIRECTORY, else the per-user
/// app data folder.
pub fn state_dir() -> PathBuf {
    if let Ok(p) = std::env::var("HONE_LISTENER_STATE") {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("STATE_DIRECTORY") {
        return PathBuf::from(p.split(':').next().unwrap_or(&p));
    }
    // .deb install: the systemd service's state, shared with `hone-listener open`.
    if cfg!(target_os = "linux") && std::env::current_exe().map(|p| p.starts_with("/usr/lib/hone-listener")).unwrap_or(false) {
        return PathBuf::from("/var/lib/hone-listener");
    }
    #[cfg(target_os = "windows")]
    let base = dirs::data_local_dir().map(|d| d.join("HONE").join("Listener"));
    #[cfg(target_os = "macos")]
    let base = dirs::data_dir().map(|d| d.join("HONE Listener"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = dirs::data_local_dir().map(|d| d.join("hone-listener"));
    base.unwrap_or_else(|| PathBuf::from(".hone-listener"))
}

/// Model, TFLite runtime, class names, ffmpeg: next to the executable, or $HONE_LISTENER_RES.
pub fn res_dir() -> PathBuf {
    if let Ok(p) = std::env::var("HONE_LISTENER_RES") {
        return PathBuf::from(p);
    }
    let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    let dir = exe.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    #[cfg(target_os = "macos")]
    if dir.ends_with("Contents/MacOS") {
        return dir.parent().unwrap().join("Resources");
    }
    dir
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Account {
    pub handle: String,
    pub email: String,
    #[serde(default)]
    pub name: String,
    /// Inviting account, or "none" for the community pool.
    #[serde(default)]
    pub r#ref: String,
    #[serde(default)]
    pub verified: bool,
}

impl Account {
    pub fn referrer(&self) -> Option<&str> {
        let r = self.r#ref.trim();
        (!r.is_empty() && r != "none").then_some(r)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Station {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub onvif: Option<String>,
    pub codec: String,
    pub rate: u32,
    pub tier: String,
    pub id_source: String,
    #[serde(default)]
    pub noise_db: Option<f64>,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub added_ms: u64,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(default)]
    pub account: Option<Account>,
    /// Invite code from the download link, prefilled on sign-up.
    #[serde(default)]
    pub pending_ref: Option<String>,
    #[serde(default)]
    pub referral_ack: bool,
    #[serde(default)]
    pub disclaimer_ack_ms: Option<u64>,
    #[serde(default)]
    pub stations: Vec<Station>,
    /// Start listening was pressed (and not paused since).
    #[serde(default)]
    pub listening: bool,
    /// Owner-declared location, kept only as H3 cells: res 9 (chain), res 7 (bird map).
    #[serde(default)]
    pub cell9: Option<String>,
    #[serde(default)]
    pub cell7: Option<String>,
    #[serde(default)]
    pub show_name: bool,
    /// Owner's UTC offset in minutes (from the setup page's browser), for voiceprint nights.
    #[serde(default)]
    pub tz_min: Option<i32>,
    /// Declared camera location, kept only to place voiceprints (never sent).
    #[serde(default)]
    pub lon_hint: Option<f64>,
    #[serde(default = "yes")]
    pub auto_update: bool,
    #[serde(default = "api")]
    pub api: String,
    #[serde(default = "chain")]
    pub chain: String,
    #[serde(default = "upd")]
    pub update_url: String,
}

fn api() -> String {
    std::env::var("HONE_LISTENER_API").unwrap_or_else(|_| API.into())
}
fn chain() -> String {
    std::env::var("HONE_LISTENER_CHAIN").unwrap_or_else(|_| CHAIN.into())
}
fn upd() -> String {
    std::env::var("HONE_LISTENER_UPDATE_URL").unwrap_or_else(|_| UPDATE_URL.into())
}

impl Default for Config {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap()
    }
}

pub fn ref_ok(r: &str) -> bool {
    regex::Regex::new(r"^[a-z0-9][a-z0-9._-]{0,31}$").unwrap().is_match(r)
}

impl Config {
    pub fn path() -> PathBuf {
        state_dir().join("config.json")
    }

    pub fn load() -> Self {
        let mut c: Config = std::fs::read_to_string(Self::path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        if let Ok(v) = std::env::var("HONE_LISTENER_API") { c.api = v; }
        if let Ok(v) = std::env::var("HONE_LISTENER_CHAIN") { c.chain = v; }
        if let Ok(v) = std::env::var("HONE_LISTENER_UPDATE_URL") { c.update_url = v; }
        // Invite code dropped by the installer (from the download link), once.
        let rf = state_dir().join("ref");
        if let Ok(r) = std::fs::read_to_string(&rf) {
            let r = r.trim().to_lowercase();
            if ref_ok(&r) && c.account.is_none() {
                c.pending_ref = Some(r);
            }
            let _ = std::fs::remove_file(rf);
            let _ = c.save();
        }
        c
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = Self::path();
        std::fs::create_dir_all(p.parent().unwrap())?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, p)
    }

    pub fn ready(&self) -> bool {
        self.account.is_some() && self.referral_ack && self.disclaimer_ack_ms.is_some() && !self.stations.is_empty()
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

/// Lat/lon -> (res 9, res 7) H3 cells. Raw coordinates are not kept.
pub fn cells(lat: f64, lon: f64) -> Option<(String, String)> {
    let ll = h3o::LatLng::new(lat, lon).ok()?;
    Some((ll.to_cell(h3o::Resolution::Nine).to_string(), ll.to_cell(h3o::Resolution::Seven).to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn cells_are_h3() {
        let (c9, c7) = super::cells(13.68, -89.24).unwrap();
        assert!(c9.starts_with("89") && c9.len() == 15, "{c9}");
        assert!(c7.starts_with("87") && c7.len() == 15, "{c7}");
    }
}
