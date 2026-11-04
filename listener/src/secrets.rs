//! Secrets: the station key, the account session and camera logins.
//! Windows: Credential Manager (DPAPI). macOS: Keychain. Linux (headless systemd service, no
//! desktop keyring): a 0600 file inside the service's private 0700 state directory.
//! Values are never logged.

use ed25519_dalek::SigningKey;
use std::collections::BTreeMap;
use std::sync::Mutex;

const SERVICE: &str = "HONE Listener";
static LOCK: Mutex<()> = Mutex::new(());

fn file() -> std::path::PathBuf {
    crate::config::state_dir().join("secrets.json")
}

fn file_map() -> BTreeMap<String, String> {
    std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn file_save(m: &BTreeMap<String, String>) -> Result<(), String> {
    let p = file();
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(&serde_json::to_vec(m).unwrap()).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, serde_json::to_vec(m).unwrap()).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, p).map_err(|e| e.to_string())
}

/// "keychain" or "file", for the status page.
pub fn backend() -> &'static str {
    #[cfg(any(windows, target_os = "macos"))]
    {
        if std::env::var("HONE_LISTENER_FILE_SECRETS").is_err() {
            return "keychain";
        }
    }
    "file"
}

pub fn get(name: &str) -> Option<String> {
    let _g = LOCK.lock().unwrap();
    #[cfg(any(windows, target_os = "macos"))]
    if backend() == "keychain" {
        return keyring::Entry::new(SERVICE, name).ok()?.get_password().ok();
    }
    let _ = SERVICE;
    file_map().get(name).cloned()
}

pub fn set(name: &str, value: &str) -> Result<(), String> {
    let _g = LOCK.lock().unwrap();
    #[cfg(any(windows, target_os = "macos"))]
    if backend() == "keychain" {
        return keyring::Entry::new(SERVICE, name).and_then(|e| e.set_password(value)).map_err(|e| format!("keychain: {e}"));
    }
    let mut m = file_map();
    m.insert(name.into(), value.into());
    file_save(&m)
}

pub fn delete(name: &str) {
    let _g = LOCK.lock().unwrap();
    #[cfg(any(windows, target_os = "macos"))]
    if backend() == "keychain" {
        if let Ok(e) = keyring::Entry::new(SERVICE, name) {
            let _ = e.delete_credential();
        }
        return;
    }
    let mut m = file_map();
    if m.remove(name).is_some() {
        let _ = file_save(&m);
    }
}

/// The station key: generated on this machine on first use, kept in secure storage.
pub fn station_key() -> Result<SigningKey, String> {
    static KEY: Mutex<()> = Mutex::new(());
    let _k = KEY.lock().unwrap();
    if let Some(h) = get("station_key") {
        let b = hex::decode(h.trim()).map_err(|_| "station key unreadable")?;
        let a: [u8; 32] = b.try_into().map_err(|_| "station key unreadable")?;
        return Ok(SigningKey::from_bytes(&a));
    }
    let k = SigningKey::generate(&mut rand::rngs::OsRng);
    set("station_key", &hex::encode(k.to_bytes()))?;
    // read back, so a keychain that silently drops writes is caught now, not after reboot
    match get("station_key") {
        Some(h) if h == hex::encode(k.to_bytes()) => Ok(k),
        _ => Err("could not save the station key in secure storage".into()),
    }
}

pub fn camera_creds(id: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(&get(&format!("cam:{id}"))?).ok()?;
    Some((v["u"].as_str()?.to_string(), v["p"].as_str()?.to_string()))
}

pub fn set_camera_creds(id: &str, user: &str, pass: &str) -> Result<(), String> {
    set(&format!("cam:{id}"), &serde_json::json!({"u": user, "p": pass}).to_string())
}
