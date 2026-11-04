//! Usefulness scoring for sensor summaries (spec 2026-11-02 §3): pay for what is new or
//! what changed, not for volume.
//!
//! A `SensorSummary` carries one public `(cell, window)` row: metric name -> integer value
//! (the phone's `public_summary` v1/v2 rows and Flipper Wi-Fi/sub-GHz/IR summaries map onto
//! this). Scoring is deterministic and runs at apply, in commit order, on every node:
//!
//! - one summary per device per metric per window, and one per OWNER per (cell, window,
//!   metric): a phone and a Flipper of the same owner count once (§3a). Duplicates and
//!   replays score 0;
//! - **novel** = the first report for (cell, metric, day-bucket), or a value outside the
//!   metric's noise band around the chain's current estimate. The first valid reporter
//!   gets `NOVEL_POINTS`, and the estimate moves to the new value;
//! - a later matching report by a different owner within ±2 windows is verification:
//!   `VERIFY_POINTS` (near 0) and +1 trust for the original reporter. One owner's devices
//!   never verify each other;
//! - a different owner, same cell, within ±2 windows, outside the band is a contradiction:
//!   the reporter earns 0 and the estimate's owner loses trust. (Claw-back, the location
//!   fingerprint check and teleport voiding (§3b) are follow-ups.)
//!
//! Sub-GHz and IR use the same rule per band (spec open question, proposed yes).
//! The hone-decide `coverage_usefulness` and `sensor_plausibility_v2` tasks run beside this
//! in shadow mode only (`shadow_*`): their answers are logged, never applied.
use crate::chain::Chain;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

pub const NOVEL_POINTS: u64 = 100;
pub const VERIFY_POINTS: u64 = 1;
pub const BUCKET_WINDOWS: u64 = 96; // 1 day of 15-minute windows
pub const VERIFY_WINDOWS: u64 = 2;
pub const MAX_METRICS: usize = 32;

/// Noise band: a change smaller than or equal to this is "the same value".
pub fn band(metric: &str, est: i64) -> i64 {
    let a = est.abs();
    let pct = |p: i64, min: i64| (a * p / 100).max(min);
    let m = metric;
    if m.ends_with("rssi_p50") || m.ends_with("rssi_max") || m.ends_with("_dbm") { return 6; }
    if m == "hone.devices" || m == "mobile_ap" { return 0; }
    if m.starts_with("ble.") || m.contains("crowd") || m.contains("distinct_devices") || m.contains("probe") { return pct(30, 3); }
    if m.starts_with("subghz.") || m.starts_with("ir.") || m.starts_with("nfc.") { return pct(25, 1); }
    if m.starts_with("wifi.") { return pct(20, 2); }
    pct(20, 1)
}

pub fn valid_cell(cell: &str) -> bool {
    cell.len() == 15 && cell.starts_with('8')
        && cell.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn valid_metric(m: &str) -> bool {
    !m.is_empty() && m.len() <= 40
        && m.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_' || b == b':')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict { Novel, Verify, Contradict, Repeat, Duplicate }

fn get_json(chain: &Chain, k: &str) -> Option<serde_json::Value> {
    chain.store.state_get(k).and_then(|b| serde_json::from_slice(&b).ok())
}
fn set_json(chain: &Chain, k: &str, v: &serde_json::Value) {
    let _ = chain.store.state_set(k, &serde_json::to_vec(v).unwrap_or_default());
}
fn add_trust(chain: &Chain, owner: &str, d: i64) {
    let k = format!("utrust:{owner}");
    let cur: i64 = chain.store.state_get(&k).and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.parse().ok()).unwrap_or(0);
    let _ = chain.store.state_set(&k, (cur + d).clamp(-1000, 1000).to_string().as_bytes());
}

pub fn owner_of(a: &str) -> String {
    hone_types::parse_device_clock_id(a).map(|(o, _)| o.to_owned()).unwrap_or_else(|| a.to_owned())
}

/// Score one summary. Writes only usefulness state. Returns per-metric verdicts and the
/// points credited to the owner for `epoch`.
pub fn score(chain: &Chain, sensor_id: &str, owner: &str, cell: &str, window: u64,
             metrics: &BTreeMap<String, i64>, epoch: u64) -> (Vec<(String, Verdict)>, u64) {
    let owner = owner_of(owner);
    let mut out = Vec::new();
    let mut pts = 0u64;
    for (m, &v) in metrics {
        let dev_key = format!("usum:{sensor_id}:{window}:{m}");
        let own_key = format!("usum_o:{owner}:{cell}:{window}:{m}");
        if chain.store.state_get(&dev_key).is_some() || chain.store.state_get(&own_key).is_some() {
            out.push((m.clone(), Verdict::Duplicate));
            continue;
        }
        let _ = chain.store.state_set(&dev_key, b"1");
        let _ = chain.store.state_set(&own_key, b"1");
        let est_key = format!("uest:{cell}:{m}");
        let est = get_json(chain, &est_key);
        let bucket = window / BUCKET_WINDOWS;
        let verdict = match &est {
            None => Verdict::Novel,
            Some(est) => {
                let ev = est["value"].as_i64().unwrap_or(0);
                let ew = est["window"].as_u64().unwrap_or(0);
                let eo = est["owner"].as_str().unwrap_or("");
                let same = (v - ev).abs() <= band(m, ev);
                let near = window.abs_diff(ew) <= VERIFY_WINDOWS;
                if eo != owner && near {
                    if same { Verdict::Verify } else { Verdict::Contradict }
                } else if window > ew && (!same || bucket != ew / BUCKET_WINDOWS) {
                    Verdict::Novel
                } else {
                    Verdict::Repeat
                }
            }
        };
        let est_owner = est.as_ref().and_then(|e| e["owner"].as_str().map(str::to_owned)).unwrap_or_default();
        match verdict {
            Verdict::Novel => {
                let mut recent: Vec<serde_json::Value> = est.as_ref()
                    .and_then(|e| e["recent"].as_array().cloned()).unwrap_or_default();
                recent.push(serde_json::json!([window, v]));
                if recent.len() > 5 { recent.remove(0); }
                set_json(chain, &est_key, &serde_json::json!({
                    "value": v, "window": window, "owner": owner, "sensor": sensor_id, "recent": recent }));
                pts += NOVEL_POINTS;
            }
            Verdict::Verify => {
                let vkey = format!("uver:{cell}:{m}:{}:{owner}", est.as_ref().and_then(|e| e["window"].as_u64()).unwrap_or(0));
                if chain.store.state_get(&vkey).is_none() {
                    let _ = chain.store.state_set(&vkey, b"1");
                    add_trust(chain, &est_owner, 1);
                    pts += VERIFY_POINTS;
                }
            }
            Verdict::Contradict => add_trust(chain, &est_owner, -2),
            _ => {}
        }
        out.push((m.clone(), verdict));
    }
    if pts > 0 {
        let k = format!("useful_pts:{epoch}:{owner}");
        let cur: u64 = chain.store.state_get(&k).and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| s.parse().ok()).unwrap_or(0);
        let _ = chain.store.state_set(&k, (cur + pts).to_string().as_bytes());
    }
    (out, pts)
}

/// Usefulness points per owner for `epoch` (reward pool input).
pub fn points_for_epoch(chain: &Chain, epoch: u64) -> Vec<(String, u64)> {
    let pre = format!("useful_pts:{epoch}:");
    chain.store.state_scan_prefix(&pre).into_iter().filter_map(|(k, v)| {
        let owner = k.strip_prefix(&pre)?.to_owned();
        let p: u64 = String::from_utf8(v).ok()?.parse().ok()?;
        (p > 0).then_some((owner, p))
    }).collect()
}

// ── shadow (hone-decide), never applied ─────────────────────────────────────────

static SHADOW: Mutex<VecDeque<serde_json::Value>> = Mutex::new(VecDeque::new());

pub fn shadow_enabled() -> bool {
    matches!(std::env::var("DECIDE_JURY_SHADOW").as_deref(), Ok("1") | Ok("true"))
        && std::env::var("DECIDE_URL").is_ok()
}

/// Queue a scored metric for the shadow models (bounded; drops when full). Reads state,
/// never writes it.
pub fn shadow_push(chain: &Chain, sensor_id: &str, owner: &str, cell: &str, window: u64,
                   metric: &str, value: i64, verdict: &Verdict) {
    if !shadow_enabled() { return; }
    let est = get_json(chain, &format!("uest:{cell}:{metric}")).unwrap_or(serde_json::Value::Null);
    let item = serde_json::json!({ "sensor_id": sensor_id, "owner": owner, "cell": cell, "window": window,
        "metric": metric, "value": value, "det": format!("{verdict:?}"), "estimate": est,
        "band": band(metric, est["value"].as_i64().unwrap_or(0)) });
    let mut q = SHADOW.lock().unwrap();
    if q.len() < 256 { q.push_back(item); }
}

fn det_label(d: &str) -> &'static str {
    match d { "Novel" => "new", "Verify" => "adds", _ => "repeat" }
}

fn request(task: &str, ctx: &serde_json::Value) -> serde_json::Value {
    let (q, c): (&str, [&str; 3]) = if task == "coverage_usefulness" {
        ("Does this coverage report add useful information to what the chain already holds for this cell and carrier this epoch?",
         ["new", "adds", "repeat"])
    } else {
        ("Is the latest reading plausible given this sensor's type, its physical limits, its recent readings and the derived comparisons?",
         ["yes", "no", "unsure"])
    };
    serde_json::json!({ "task": task, "question": q, "candidates": c, "context": ctx.to_string() })
}

/// Background task: drain the shadow queue through hone-decide and log both answers next
/// to the deterministic verdict. The model never decides payouts.
pub async fn shadow_loop() {
    let Ok(base) = std::env::var("DECIDE_URL") else { return };
    let Ok(http) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build() else { return };
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let batch: Vec<serde_json::Value> = {
            let mut q = SHADOW.lock().unwrap();
            let n = q.len().min(4);
            q.drain(..n).collect()
        };
        for it in batch {
            let est = &it["estimate"];
            let ev = est["value"].as_i64();
            let val = it["value"].as_i64().unwrap_or(0);
            let cov = serde_json::json!({
                "report": { "reporter": it["owner"], "cell": it["cell"], "metric": it["metric"], "value": val, "window": it["window"] },
                "cell_this_epoch": { "estimate": ev, "estimate_window": est["window"], "estimate_owner": est["owner"] },
                "derived": { "delta_vs_estimate": ev.map(|e| val - e), "noise_band": it["band"] },
                "rules": { "novel": "first report for cell+metric+day, or a change beyond the noise band", "repeat_pays": 0 } });
            let pl = serde_json::json!({ "sensor_id": it["sensor_id"], "kind": it["metric"], "unit": "count",
                "physical_range": [-150, 100000], "recent": est["recent"],
                "latest": { "window": it["window"], "value": val },
                "derived": { "delta_vs_estimate": ev.map(|e| val - e) } });
            let mut answers = Vec::new();
            for (task, ctx) in [("coverage_usefulness", cov), ("sensor_plausibility_v2", pl)] {
                let r = http.post(format!("{base}/decide")).json(&request(task, &ctx)).send().await;
                let a = match r { Ok(r) => r.json::<serde_json::Value>().await.ok(), Err(_) => None };
                answers.push(match a {
                    Some(a) => {
                        let p = a["probs"].as_array().map(|p| p.iter().filter_map(|x| x.as_f64()).fold(0.0f64, f64::max)).unwrap_or(0.0);
                        format!("{task}={} p={p:.2}", a["choice"].as_str().unwrap_or("?"))
                    }
                    None => format!("{task}=unavailable"),
                });
            }
            let det = it["det"].as_str().unwrap_or("");
            tracing::info!("[usefulness-shadow] {} {} w{} {}={} det={} ({}) | {} | model never decides",
                it["owner"].as_str().unwrap_or(""), it["cell"].as_str().unwrap_or(""), it["window"],
                it["metric"].as_str().unwrap_or(""), val, det, det_label(det), answers.join(" | "));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chain() -> (Chain, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        (Chain::new(crate::store::Store::open(d.path()).unwrap(), "n".into(), "hone-test".into()), d)
    }
    fn m(pairs: &[(&str, i64)]) -> BTreeMap<String, i64> { pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect() }
    const C: &str = "896d75d960bffff";

    #[test]
    fn replay_and_duplicate_frames_score_zero() {
        let (c, _d) = chain();
        let s = m(&[("wifi.ap_count", 27), ("ble.crowd", 20)]);
        let (_, p1) = score(&c, "phone-a", "josh", C, 1992929, &s, 10);
        assert_eq!(p1, 2 * NOVEL_POINTS);
        // Exact replay of the same summary: 0.
        let (v, p2) = score(&c, "phone-a", "josh", C, 1992929, &s, 11);
        assert_eq!(p2, 0);
        assert!(v.iter().all(|(_, x)| *x == Verdict::Duplicate));
        // Same owner's other device, same cell/window/metric (phone + Flipper): 0.
        let (_, p3) = score(&c, "flipper-x", "josh", C, 1992929, &s, 11);
        assert_eq!(p3, 0);
        // A device seat of the same owner counts as that owner.
        let seat = format!("josh@{}", "ab".repeat(32));
        let (_, p4) = score(&c, "seat-dev", &seat, C, 1992929, &s, 11);
        assert_eq!(p4, 0);
        // Next window, values within band, same day bucket: repeat, 0.
        let (v5, p5) = score(&c, "phone-a", "josh", C, 1992930, &m(&[("wifi.ap_count", 26), ("ble.crowd", 21)]), 12);
        assert_eq!(p5, 0);
        assert!(v5.iter().all(|(_, x)| *x == Verdict::Repeat));
        assert_eq!(points_for_epoch(&c, 10), vec![("josh".to_string(), 200)]);
        assert!(points_for_epoch(&c, 11).is_empty());
        assert!(points_for_epoch(&c, 12).is_empty());
    }

    #[test]
    fn change_beyond_band_pays_and_verification_is_near_zero() {
        let (c, _d) = chain();
        score(&c, "phone-a", "josh", C, 1992929, &m(&[("wifi.ap_count", 20)]), 10);
        // A different owner confirms within ±2 windows: verification only.
        let (v, p) = score(&c, "fl-b", "natoshisakamoto", C, 1992930, &m(&[("wifi.ap_count", 21)]), 10);
        assert_eq!((v[0].1.clone(), p), (Verdict::Verify, VERIFY_POINTS));
        assert_eq!(c.store.state_get("utrust:josh"), Some(b"1".to_vec()));
        // A different owner much later, big change (outage): novel.
        let (v, p) = score(&c, "fl-b", "natoshisakamoto", C, 1992999, &m(&[("wifi.ap_count", 3)]), 12);
        assert_eq!((v[0].1.clone(), p), (Verdict::Novel, NOVEL_POINTS));
        // Contradiction by another owner within ±2 windows of the new estimate: 0, trust down.
        let (v, p) = score(&c, "phone-a", "josh", C, 1993000, &m(&[("wifi.ap_count", 30)]), 12);
        assert_eq!((v[0].1.clone(), p), (Verdict::Contradict, 0));
        assert_eq!(c.store.state_get("utrust:natoshisakamoto"), Some(b"-2".to_vec()));
        // Same value next day by the same owner: first report for the new bucket pays.
        let (v, p) = score(&c, "fl-b", "natoshisakamoto", C, 1992999 + 96, &m(&[("wifi.ap_count", 3)]), 20);
        assert_eq!((v[0].1.clone(), p), (Verdict::Novel, NOVEL_POINTS));
        // Sub-GHz per band uses the same rule.
        let (_, p) = score(&c, "fl-b", "natoshisakamoto", C, 1993100, &m(&[("subghz.433", 5)]), 21);
        assert_eq!(p, NOVEL_POINTS);
        let (_, p) = score(&c, "fl-b", "natoshisakamoto", C, 1993101, &m(&[("subghz.433", 6)]), 21);
        assert_eq!(p, 0);
    }

    #[test]
    fn cells_and_metrics_are_validated() {
        assert!(valid_cell(C));
        assert!(!valid_cell("896D75D960BFFFF"));
        assert!(!valid_cell("abc"));
        assert!(valid_metric("wifi.ap_count") && valid_metric("subghz.433") && !valid_metric("Wifi Count"));
    }
}
