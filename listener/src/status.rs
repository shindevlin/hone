//! Signed device status (indexer /api/devices/status), like the phone's DeviceStatus but with
//! device_kind "camera listener" and a label, so the account page (/me) lists it as a camera.
//! Counts and states only: no audio, no camera address, no credentials. Integers only.

use crate::config::{now_ms, Config};
use crate::{canonical, live, secrets};
use serde_json::{json, Value};
use std::time::Duration;

pub const DEVICE_KIND: &str = "camera listener";

pub fn label(cfg: &Config) -> String {
    match cfg.stations.first() {
        Some(s) if cfg.stations.len() == 1 => format!("{} (camera listener)", s.label).chars().take(40).collect(),
        Some(_) => format!("{} cameras (listener)", cfg.stations.len()),
        None => "HONE camera listener".into(),
    }
}

pub fn payload(cfg: &Config) -> Option<Value> {
    let acct = cfg.account.as_ref()?;
    let key = secrets::station_key().ok()?;
    let pubh = canonical::pub_hex(&key);
    let l = live::snapshot();
    let day0 = now_ms() / 86_400_000 * 86_400_000;
    let today = l.recent.iter().filter(|d| d.t >= day0).count() as u64;
    let cams: Vec<Value> = cfg.stations.iter().map(|s| {
        let x = l.stations.get(&s.id).cloned().unwrap_or_default();
        json!({"station": s.id, "tier": s.tier, "codec": s.codec, "rate": s.rate, "enabled": s.enabled, "running": x.running,
            "reachable": x.reachable, "frames": x.frames, "voices_removed": x.voices_removed, "birds": x.birds, "busy_dropped": x.busy_dropped})
    }).collect();
    Some(json!({
        "v": 1, "kind": "device_status", "ts": now_ms() / 1000, "account": acct.handle, "device": &pubh[..16], "seat_pub": pubh,
        "device_kind": DEVICE_KIND, "label": label(cfg), "cell7": cfg.cell7,
        "listener": {"version": env!("CARGO_PKG_VERSION"), "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
            "listening": cfg.listening, "ready": cfg.ready(), "perch": l.model_error.is_none(), "reports_made": l.reports_made,
            "chain_state": l.chain_state, "chain_queued": l.chain_queued as u64, "chain_accepted": l.chain_accepted,
            "update": l.update_state, "secret_store": secrets::backend()},
        "cameras": cams,
        "birds": {"detections_today": today},
    }))
}

pub fn send(cfg: &Config) -> Result<(), String> {
    let p = payload(cfg).ok_or("not signed in")?;
    let key = secrets::station_key()?;
    let body = canonical::encode(&canonical::envelope(&canonical::encode(&p), "seat", &key));
    match crate::account::agent().post(&format!("{}/devices/status", cfg.api)).set("Content-Type", "application/json").send_string(&body) {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(c, r)) => Err(format!("HTTP {c} {}", r.into_string().unwrap_or_default().chars().take(100).collect::<String>())),
        Err(e) => Err(format!("offline ({})", e.kind())),
    }
}

pub fn spawn() {
    std::thread::Builder::new()
        .name("hone-status".into())
        .spawn(|| {
            std::thread::sleep(Duration::from_secs(20));
            loop {
                let cfg = Config::load();
                if cfg.account.is_some() {
                    if let Err(e) = send(&cfg) {
                        log::warn!("device status: {e}");
                    }
                }
                std::thread::sleep(Duration::from_secs(15 * 60));
            }
        })
        .unwrap();
}

/// Local, unauthenticated summary for 127.0.0.1:<port>/status (used by the HONE MCP).
/// Non-secret id of this install (hash of its page token), to tell two listeners apart.
pub fn instance_id() -> String {
    use sha2::{Digest, Sha256};
    let t = secrets::get("ui_token").unwrap_or_default();
    hex::encode(Sha256::digest(format!("hone-listener-instance|{t}").as_bytes()))[..16].to_string()
}

pub fn local(cfg: &Config) -> Value {
    let l = live::snapshot();
    json!({"name": "hone-listener", "instance": instance_id(), "version": env!("CARGO_PKG_VERSION"), "signed_in": cfg.account.is_some(), "ready": cfg.ready(),
        "listening": cfg.listening, "cameras": cfg.stations.len(),
        "cameras_running": l.stations.values().filter(|s| s.running).count(),
        "birds_heard": l.stations.values().map(|s| s.birds).sum::<u64>(),
        "voices_removed": l.stations.values().map(|s| s.voices_removed).sum::<u64>(),
        "reports_made": l.reports_made, "chain_state": l.chain_state, "summary": live::summary(cfg.listening)})
}

#[cfg(test)]
mod tests {
    #[test]
    fn payload_is_canonical_integers_only() {
        std::env::set_var("HONE_LISTENER_STATE", std::env::temp_dir().join("hl-test-shared"));
        std::env::set_var("HONE_LISTENER_FILE_SECRETS", "1");
        let mut c = crate::config::Config::default();
        c.account = Some(crate::config::Account { handle: "u-abc".into(), ..Default::default() });
        let p = super::payload(&c).unwrap();
        assert_eq!(p["device_kind"], "camera listener");
        assert_eq!(p["device"].as_str().unwrap().len(), 16);
        let _ = crate::canonical::encode(&p); // panics on floats
    }
}
