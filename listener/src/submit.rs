//! Where reports go, the way the Android app sends them for an email-signed-in, referred user:
//!  1. Bird map (indexer /api/birds/sightings): publishable species (>= 70 % or heard twice in
//!     the window), source "station", signed with the station key (key "seat"; the indexer
//!     counts it once the key is linked to the account, which sign-in does).
//!  2. Chain (/api/sensor/summary): one SENSOR_SUMMARY per window for the seat
//!     "<referrer>@<station key>", signed by the station key. Until the sponsored seat exists
//!     the chain refuses it; the report then waits in a local queue and the listener shows
//!     "waiting for your listener to be registered". Nothing is ever shown as accepted
//!     unless the chain said accepted:true.
//! Queues hold only small signed reports (no audio).

use crate::account::agent;
use crate::config::{now_ms, state_dir, Config};
use crate::report::Report;
use crate::{canonical, live, perch, secrets};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

pub const WAITING: &str = "waiting for your listener to be registered";
const PUBLISH_MIN_CONF: f64 = 70.0;
const MAX_AGE_WINDOWS: u64 = 7 * 96;
const PER_ROUND: usize = 30;

fn dir(name: &str) -> PathBuf {
    let d = state_dir().join(name);
    let _ = std::fs::create_dir_all(&d);
    d
}

fn queue_files(name: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir(name)).map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map(|x| x == "json").unwrap_or(false)).collect()).unwrap_or_default();
    v.sort();
    v
}

fn append_local(r: &Report) {
    let p = state_dir().join("reports.jsonl");
    let mut lines: Vec<String> = std::fs::read_to_string(&p).map(|t| t.lines().map(String::from).collect()).unwrap_or_default();
    lines.push(r.signed.clone());
    let keep = lines.len().saturating_sub(2000);
    let _ = std::fs::write(&p, lines[keep..].join("\n") + "\n");
}

/// The bird-map sighting for one window, or None when nothing is publishable.
pub fn sighting_payload(r: &Report, account: &str, cell: &str, show_name: bool) -> Option<Value> {
    let sp: Vec<Value> = r.payload["species"].as_array()?.iter()
        .filter(|s| s["frames"].as_u64().unwrap_or(0) >= 2 || s["conf_q8"].as_f64().unwrap_or(0.0) / 2.55 >= PUBLISH_MIN_CONF)
        .map(|s| json!({"name": s["name"], "conf_q8": s["conf_q8"], "frames": s["frames"]}))
        .collect();
    if sp.is_empty() {
        return None;
    }
    Some(json!({"v": 1, "sid": uuid::Uuid::new_v4().to_string(), "window": r.window, "cell": cell, "model": perch::MODEL_ID,
        "source": "station", "tier": r.tier, "station": r.station, "species": sp, "account": account, "show_name": show_name}))
}

fn post(url: &str, body: &str) -> Result<(u16, String), String> {
    match agent().post(url).set("Content-Type", "application/json").send_string(body) {
        Ok(r) => Ok((r.status(), r.into_string().unwrap_or_default())),
        Err(ureq::Error::Status(c, r)) => Ok((c, r.into_string().unwrap_or_default())),
        Err(e) => Err(format!("offline ({})", e.kind())),
    }
}

fn map_round(cfg: &Config) {
    let Some(acct) = cfg.account.as_ref() else {
        live::set(|l| l.map_state = Some("sign in to share birds on the map".into()));
        return;
    };
    let Ok(key) = secrets::station_key() else { return };
    for f in queue_files("map-queue").into_iter().take(PER_ROUND) {
        let Ok(p) = std::fs::read_to_string(&f) else { continue };
        let body = canonical::encode(&canonical::envelope(&p, "seat", &key));
        match post(&format!("{}/birds/sightings", cfg.api), &body) {
            Err(e) => {
                live::set(|l| l.map_state = Some(e));
                return;
            }
            Ok((c, _)) if c == 429 || c >= 500 => {
                live::set(|l| l.map_state = Some(format!("bird map busy (HTTP {c}), will retry")));
                return;
            }
            Ok((c, b)) => {
                let _ = std::fs::remove_file(&f);
                live::set(|l| l.map_state = Some(if (200..300).contains(&c) { "birds shared on the map".into() } else { format!("bird map refused a report (HTTP {c}): {}", b.chars().take(100).collect::<String>()) }));
            }
        }
        let _ = acct;
    }
}

/// SENSOR_SUMMARY metrics from a report: integers, lowercase names, at most 32.
pub fn metrics(r: &Value) -> std::collections::BTreeMap<String, i64> {
    let mut m = std::collections::BTreeMap::new();
    let tier = match r["tier"].as_str() { Some("A") => 3, Some("B") => 2, _ => 1 };
    m.insert("station.tier".to_string(), tier);
    for (k, f) in [("station.listen_s", "listen_seconds"), ("station.frames", "frames"), ("station.human_frames", "frames_discarded_human")] {
        m.insert(k.to_string(), r[f].as_i64().unwrap_or(0));
    }
    let species = r["species"].as_array().cloned().unwrap_or_default();
    m.insert("station.species".into(), species.len() as i64);
    for k in ["l50", "l90"] {
        if let Some(v) = r["noise"][k].as_i64() {
            m.insert(format!("noise.{k}"), v);
        }
    }
    let ok = regex::Regex::new(r"^[a-z0-9._:]{1,40}$").unwrap();
    for s in species {
        if m.len() >= 32 {
            break;
        }
        let n = format!("bird:{}", s["name"].as_str().unwrap_or("").to_lowercase().replace(' ', "_"));
        if ok.is_match(&n) {
            m.insert(n, s["conf_q8"].as_i64().unwrap_or(0));
        }
    }
    m
}

/// SENSOR_SUMMARY wire message (telemetry/SensorSummaryWire.kt): keys in this exact order.
pub fn wire_message(chain_id: &str, sensor_id: &str, owner: &str, batch_hash: &str, cell: &str, window: u64, metrics: &std::collections::BTreeMap<String, i64>) -> String {
    let q = |s: &str| canonical::encode(&json!(s));
    let m = metrics.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect::<Vec<_>>().join(",");
    format!("{{\"chain_id\":{},\"type\":\"SENSOR_SUMMARY\",\"sensor_id\":{},\"owner\":{},\"batch_hash\":{},\"cell\":\"{cell}\",\"window\":{window},\"metrics\":{{{m}}}}}",
        q(chain_id), q(sensor_id), q(owner), q(batch_hash))
}

/// Refusals meaning "this seat is not registered/active yet": keep the report queued.
pub fn is_seat_wait(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("has no stake") || e.contains("not registered") || e.contains("no such seat") || e.contains("unknown seat")
        || e.contains("seat not found") || e.contains("is not active")
}

fn chain_round(cfg: &Config) {
    let files = queue_files("chain-queue");
    let now_w = now_ms() / crate::pipeline::WINDOW_MS;
    let mut queued = 0;
    for f in &files {
        let w: u64 = f.file_stem().and_then(|s| s.to_str()).and_then(|s| s.split('-').next()).and_then(|s| s.parse().ok()).unwrap_or(0);
        if w + MAX_AGE_WINDOWS < now_w {
            let _ = std::fs::remove_file(f); // a week old: earns nothing on chain any more
        } else {
            queued += 1;
        }
    }
    live::set(|l| l.chain_queued = queued);
    if queued == 0 {
        return;
    }
    let Some(acct) = cfg.account.as_ref() else { return };
    let Some(referrer) = acct.referrer() else {
        live::set(|l| l.chain_state = Some("bird reports go to the map; chain rewards need an invite code".into()));
        return;
    };
    let Some(cell9) = cfg.cell9.as_deref() else {
        live::set(|l| l.chain_state = Some("set the camera's location to send reports".into()));
        return;
    };
    let Ok(key) = secrets::station_key() else { return };
    let owner = format!("{referrer}@{}", canonical::pub_hex(&key));
    let mut sent = 0;
    for f in queue_files("chain-queue") {
        if sent >= PER_ROUND {
            break;
        }
        let Ok(env) = std::fs::read_to_string(&f).map(|t| serde_json::from_str::<Value>(&t).unwrap_or(json!({}))) else { continue };
        let Ok(rep) = serde_json::from_str::<Value>(env["payload"].as_str().unwrap_or("")) else {
            let _ = std::fs::remove_file(&f);
            continue;
        };
        let window = rep["window"].as_u64().unwrap_or(0);
        let sensor_id = format!("station-{}", rep["station"].as_str().unwrap_or(""));
        let met = metrics(&rep);
        let msg = wire_message("hone", &sensor_id, &owner, "", cell9, window, &met);
        let body = canonical::encode(&json!({"sensor_id": sensor_id, "owner": owner, "batch_hash": "", "cell": cell9, "window": window,
            "metrics": met, "signature": canonical::sign_hex(&key, &msg)}));
        match post(&format!("{}/api/sensor/summary", cfg.chain), &body) {
            Err(e) => {
                live::set(|l| l.chain_state = Some(format!("{e}, will retry")));
                return;
            }
            Ok((c, _)) if c == 429 || c >= 500 => {
                live::set(|l| l.chain_state = Some(format!("chain busy (HTTP {c}), will retry")));
                return;
            }
            Ok((_, b)) => {
                let j: Value = serde_json::from_str(&b).unwrap_or(json!({}));
                if j["accepted"].as_bool() == Some(true) {
                    let _ = std::fs::remove_file(&f);
                    live::set(|l| {
                        l.chain_accepted += 1;
                        l.chain_queued = l.chain_queued.saturating_sub(1);
                        l.chain_state = Some("reports accepted by the chain".into());
                    });
                } else {
                    let e = j["error"].as_str().unwrap_or(&b).to_string();
                    if is_seat_wait(&e) {
                        live::set(|l| l.chain_state = Some(WAITING.into()));
                        log::info!("chain: seat not registered yet; {queued} report(s) queued");
                        return;
                    }
                    log::warn!("chain refused a report: {}", e.chars().take(160).collect::<String>());
                    let _ = std::fs::rename(&f, dir("chain-refused").join(f.file_name().unwrap()));
                    live::set(|l| {
                        l.chain_queued = l.chain_queued.saturating_sub(1);
                        l.chain_state = Some(format!("chain refused a report: {}", e.chars().take(100).collect::<String>()));
                    });
                }
            }
        }
        sent += 1;
        std::thread::sleep(Duration::from_secs(2));
    }
}

pub fn enqueue(r: &Report, cfg: &Config) {
    append_local(r);
    if let (Some(a), Some(c7)) = (cfg.account.as_ref(), cfg.cell7.as_deref()) {
        if let Some(p) = sighting_payload(r, &a.handle, c7, cfg.show_name) {
            let _ = std::fs::write(dir("map-queue").join(format!("{}-{}.json", r.window, r.station)), canonical::encode(&p));
        }
    }
    let _ = std::fs::write(dir("chain-queue").join(format!("{}-{}.json", r.window, r.station)), &r.signed);
}

pub fn spawn(rx: mpsc::Receiver<Report>) {
    std::thread::Builder::new()
        .name("hone-submit".into())
        .spawn(move || {
            let mut last_chain = 0u64;
            loop {
                let got = rx.recv_timeout(Duration::from_secs(60));
                let cfg = Config::load();
                if let Ok(r) = &got {
                    live::set(|l| {
                        l.reports_made += 1;
                        l.last_report_ms = Some(now_ms());
                        l.last_report_bytes = Some(r.signed.len());
                    });
                    log::info!("report: station {} window {} ({} bytes, {} species, {} voice frames dropped)", &r.station[..6], r.window, r.signed.len(),
                        r.payload["species"].as_array().map(|a| a.len()).unwrap_or(0), r.payload["frames_discarded_human"]);
                    enqueue(r, &cfg);
                }
                if got.is_ok() || now_ms() - last_chain > 30 * 60_000 {
                    last_chain = now_ms();
                    map_round(&cfg);
                    chain_round(&cfg);
                }
            }
        })
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seat_refusal_is_a_wait_not_a_failure() {
        assert!(is_seat_wait("device seat 'maria@ab..' has no stake by 'maria'"));
        assert!(!is_seat_wait("SensorSummary signature does not verify under seat key 'x'"));
        assert!(!is_seat_wait("invalid H3 cell 'x'"));
    }

    #[test]
    fn wire_matches_android_layout() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("station.frames".to_string(), 3i64);
        m.insert("noise.l50".to_string(), -41);
        let s = wire_message("hone", "station-0123", "maria@ab", "", "8966c0d1a6bffff", 1993200, &m);
        assert_eq!(s, r#"{"chain_id":"hone","type":"SENSOR_SUMMARY","sensor_id":"station-0123","owner":"maria@ab","batch_hash":"","cell":"8966c0d1a6bffff","window":1993200,"metrics":{"noise.l50":-41,"station.frames":3}}"#);
    }

    #[test]
    fn publish_rule() {
        let r = Report { station: "0123456789abcdef".into(), tier: "C".into(), window: 5, signed: String::new(),
            payload: json!({"species": [{"name": "A b", "conf_q8": 200, "frames": 1}, {"name": "C d", "conf_q8": 140, "frames": 1}, {"name": "E f", "conf_q8": 140, "frames": 2}]}) };
        let p = sighting_payload(&r, "u-1", "87", false).unwrap();
        let names: Vec<&str> = p["species"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["A b", "E f"]);
        assert_eq!(p["source"], "station");
    }
}
