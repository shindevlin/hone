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
/// Deferred payout: a novel claim is proven by an independent owner's corroboration or by
/// surviving this many epochs without a contradiction (spec §3d, Josh 2026-11-03).
pub const CLAIM_WINDOW_EPOCHS: u64 = 2880;
/// Data older than this at submit fills the map but carries no claim.
pub const MAX_CLAIM_AGE_S: u64 = 7 * 86_400;
/// Payout of proven points: off until its own approved epoch.
pub const USEFULNESS_PAYOUT_EPOCH: u64 = u64::MAX;
/// Wall-clock anchor for epoch -> unix (epoch 352,395 sealed at 2026-11-04T03:37:42Z;
/// EPOCH_DURATION_S per epoch). A constant, so every node computes the same age.
const ANCHOR_EPOCH: u64 = 352_395;
const ANCHOR_UNIX: u64 = 1_793_763_462;

pub fn unix_at(epoch: u64) -> u64 {
    let d = hone_types::EPOCH_DURATION_S as u64;
    if epoch >= ANCHOR_EPOCH { ANCHOR_UNIX + (epoch - ANCHOR_EPOCH) * d }
    else { ANCHOR_UNIX.saturating_sub((ANCHOR_EPOCH - epoch) * d) }
}

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
                // Not paid now: a pending claim, proven later (corroboration or no
                // contradiction within CLAIM_WINDOW_EPOCHS). Too-old data claims nothing.
                if window.saturating_mul(900).saturating_add(MAX_CLAIM_AGE_S) >= unix_at(epoch) {
                    let ck = claim_key(cell, m, window);
                    if chain.store.state_get(&ck).is_none() {
                        set_json(chain, &ck, &serde_json::json!({ "owner": owner, "points": NOVEL_POINTS,
                            "epoch": epoch, "status": "pending" }));
                        let _ = chain.store.state_set(&format!("uclaim_due:{:016x}:{cell}:{m}:{window}", epoch + CLAIM_WINDOW_EPOCHS), b"1");
                        pts += NOVEL_POINTS;
                    }
                }
            }
            Verdict::Verify => {
                let vkey = format!("uver:{cell}:{m}:{}:{owner}", est.as_ref().and_then(|e| e["window"].as_u64()).unwrap_or(0));
                if chain.store.state_get(&vkey).is_none() {
                    let _ = chain.store.state_set(&vkey, b"1");
                    add_trust(chain, &est_owner, 1);
                    if let Some(ew) = est.as_ref().and_then(|e| e["window"].as_u64()) {
                        settle(chain, cell, m, ew, true, epoch);
                    }
                    credit(chain, &owner, VERIFY_POINTS, epoch);
                    pts += VERIFY_POINTS;
                }
            }
            Verdict::Contradict => {
                add_trust(chain, &est_owner, -2);
                if let Some(ew) = est.as_ref().and_then(|e| e["window"].as_u64()) {
                    settle(chain, cell, m, ew, false, epoch);
                }
            }
            _ => {}
        }
        out.push((m.clone(), verdict));
    }
    (out, pts)
}

fn claim_key(cell: &str, m: &str, window: u64) -> String { format!("uclaim:{cell}:{m}:{window}") }

fn read_u64(chain: &Chain, k: &str) -> u64 {
    chain.store.state_get(k).and_then(|b| String::from_utf8(b).ok()).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Proven points: per owner in total and per proving epoch (payout input, once enabled).
fn credit(chain: &Chain, owner: &str, pts: u64, epoch: u64) {
    for k in [format!("uproven:{owner}"), format!("uproven_e:{epoch}:{owner}")] {
        let _ = chain.store.state_set(&k, (read_u64(chain, &k) + pts).to_string().as_bytes());
    }
}

/// Prove (true) or void (false) a pending claim. Settled claims never change again.
fn settle(chain: &Chain, cell: &str, m: &str, window: u64, proven: bool, epoch: u64) {
    let ck = claim_key(cell, m, window);
    let Some(mut c) = get_json(chain, &ck) else { return };
    if c["status"] != "pending" { return; }
    c["status"] = serde_json::json!(if proven { "proven" } else { "void" });
    c["settled_epoch"] = serde_json::json!(epoch);
    set_json(chain, &ck, &c);
    if proven {
        let owner = c["owner"].as_str().unwrap_or("").to_owned();
        credit(chain, &owner, c["points"].as_u64().unwrap_or(0), epoch);
    }
}

/// The scoring gate. Tests exercise the rule as if it were active from 352,700.
pub fn scoring_epoch() -> u64 {
    if cfg!(test) { 352_700 } else { hone_types::USEFULNESS_SCORING_EPOCH }
}

/// Per quorum seal (deterministic): claims whose window passed with no contradiction are
/// proven. Called from epoch_sync::epoch_tick.
pub fn release_due(chain: &Chain, epoch: u64) -> usize {
    if epoch < scoring_epoch() { return 0; }
    let mut n = 0;
    for (k, _) in chain.store.state_scan_prefix("uclaim_due:") {
        let rest = &k["uclaim_due:".len()..];
        let Some(due) = rest.get(..16).and_then(|h| u64::from_str_radix(h, 16).ok()) else { continue };
        if due > epoch { break; }
        let mut parts = rest[17..].splitn(3, ':');
        let (Some(cell), Some(m), Some(w)) = (parts.next(), parts.next(), parts.next()) else { continue };
        if let Ok(w) = w.parse::<u64>() { settle(chain, cell, m, w, true, epoch); n += 1; }
        let _ = chain.store.state_delete(&k);
    }
    n
}

/// Total proven points of an owner.
pub fn proven_points(chain: &Chain, owner: &str) -> u64 { read_u64(chain, &format!("uproven:{owner}")) }

/// Proven points per owner released at `epoch`: the deferred payout input. Empty until
/// USEFULNESS_PAYOUT_EPOCH (off), so nothing is paid by the epoch a summary lands in.
pub fn points_for_epoch(chain: &Chain, epoch: u64) -> Vec<(String, u64)> {
    if epoch < USEFULNESS_PAYOUT_EPOCH { return Vec::new(); }
    let pre = format!("uproven_e:{epoch}:");
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

    const W0: u64 = 1_793_763_462 / 900; // window at the anchor epoch (fresh data)
    const E0: u64 = 352_395;

    fn claim(c: &Chain, w: u64, m: &str) -> Option<serde_json::Value> { get_json(c, &claim_key(C, m, w)) }

    #[test]
    fn replay_and_duplicate_frames_claim_nothing() {
        let (c, _d) = chain();
        let s = m(&[("wifi.ap_count", 27), ("ble.crowd", 20)]);
        let (_, p1) = score(&c, "phone-a", "josh", C, W0, &s, E0);
        assert_eq!(p1, 2 * NOVEL_POINTS);
        assert_eq!(claim(&c, W0, "wifi.ap_count").unwrap()["status"], "pending");
        // Exact replay, same owner's other device, same owner's seat: duplicates, 0.
        let seat = format!("josh@{}", "ab".repeat(32));
        for (dev, own) in [("phone-a", "josh"), ("flipper-x", "josh"), ("seat-dev", seat.as_str())] {
            let (v, p) = score(&c, dev, own, C, W0, &s, E0 + 1);
            assert_eq!(p, 0);
            assert!(v.iter().all(|(_, x)| *x == Verdict::Duplicate));
        }
        // Next window, within band: repeat, no claim.
        let (v, p) = score(&c, "phone-a", "josh", C, W0 + 1, &m(&[("wifi.ap_count", 26), ("ble.crowd", 21)]), E0 + 2);
        assert_eq!(p, 0);
        assert!(v.iter().all(|(_, x)| *x == Verdict::Repeat));
        assert!(claim(&c, W0 + 1, "wifi.ap_count").is_none());
        // Nothing is paid at submit time.
        assert_eq!(proven_points(&c, "josh"), 0);
    }

    #[test]
    fn unproven_never_pays_and_window_or_corroboration_proves() {
        let (c, _d) = chain();
        score(&c, "phone-a", "josh", C, W0, &m(&[("wifi.ap_count", 20)]), E0);
        score(&c, "phone-a", "josh", C, W0 + 10, &m(&[("ble.crowd", 40)]), E0);
        // Before the window passes (and with no corroboration): nothing.
        release_due(&c, E0 + CLAIM_WINDOW_EPOCHS - 1);
        assert_eq!(proven_points(&c, "josh"), 0);
        // Independent owner corroborates the wifi claim: proven at once (+ near-0 for them).
        let (v, _) = score(&c, "fl-b", "natoshisakamoto", C, W0 + 1, &m(&[("wifi.ap_count", 21)]), E0 + 5);
        assert_eq!(v[0].1, Verdict::Verify);
        assert_eq!(proven_points(&c, "josh"), NOVEL_POINTS);
        assert_eq!(proven_points(&c, "natoshisakamoto"), VERIFY_POINTS);
        // The crowd claim survives the window with no contradiction: proven then.
        release_due(&c, E0 + CLAIM_WINDOW_EPOCHS);
        assert_eq!(proven_points(&c, "josh"), 2 * NOVEL_POINTS);
        // Payout is off: no epoch hands out points yet.
        assert!(points_for_epoch(&c, E0 + CLAIM_WINDOW_EPOCHS).is_empty());
    }

    #[test]
    fn contradiction_voids_and_old_data_claims_nothing() {
        let (c, _d) = chain();
        score(&c, "phone-a", "josh", C, W0, &m(&[("wifi.ap_count", 20)]), E0);
        let (v, _) = score(&c, "fl-b", "natoshisakamoto", C, W0 + 1, &m(&[("wifi.ap_count", 60)]), E0 + 1);
        assert_eq!(v[0].1, Verdict::Contradict);
        assert_eq!(claim(&c, W0, "wifi.ap_count").unwrap()["status"], "void");
        release_due(&c, E0 + CLAIM_WINDOW_EPOCHS + 1);
        assert_eq!(proven_points(&c, "josh"), 0, "a voided claim never pays");
        assert_eq!(c.store.state_get("utrust:josh"), Some(b"-2".to_vec()));
        // 8-day-old data fills the map but claims nothing.
        let old = W0 - 8 * 96;
        let (v, p) = score(&c, "phone-a", "josh", "896d75d960affff", old, &m(&[("wifi.ap_count", 9)]), E0);
        assert_eq!((v[0].1.clone(), p), (Verdict::Novel, 0));
        assert!(get_json(&c, "uest:896d75d960affff:wifi.ap_count").is_some());
        assert!(get_json(&c, &claim_key("896d75d960affff", "wifi.ap_count", old)).is_none());
    }

    #[test]
    fn bulk_and_spread_submission_prove_the_same_total() {
        let rows: Vec<(u64, i64)> = (0..12).map(|i| (W0 + 3 * i, 10i64 << i)).collect();
        let run = |spread: bool| {
            let (c, d) = chain();
            for (i, (w, v)) in rows.iter().enumerate() {
                let e = if spread { E0 + i as u64 * 7 } else { E0 };
                score(&c, "phone-a", "josh", C, *w, &m(&[("wifi.ap_count", *v)]), e);
            }
            release_due(&c, E0 + 12 * 7 + CLAIM_WINDOW_EPOCHS);
            let p = proven_points(&c, "josh");
            drop(d);
            p
        };
        assert_eq!(run(false), run(true));
        assert_eq!(run(false), 12 * NOVEL_POINTS);
    }

    #[test]
    fn subghz_uses_the_same_rule() {
        let (c, _d) = chain();
        let (_, p) = score(&c, "fl-b", "natoshisakamoto", C, W0, &m(&[("subghz.433", 5)]), E0);
        assert_eq!(p, NOVEL_POINTS);
        let (_, p) = score(&c, "fl-b", "natoshisakamoto", C, W0 + 1, &m(&[("subghz.433", 6)]), E0);
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
