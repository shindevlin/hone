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
/// Payout of proven points starts with scoring (volume sensor pay stops then).
pub const USEFULNESS_PAYOUT_EPOCH: u64 = hone_types::USEFULNESS_SCORING_EPOCH;
/// A corroborator needs at least this trust (trust starts at 0; contradicted reporters drop).
pub const TRUST_FLOOR: i64 = 0;
/// Release cap: proven points paid out per owner per epoch; the rest waits in the owner's
/// queue for later epochs, so a bulk backlog pays the same total as a spread one.
pub const RELEASE_CAP_PER_EPOCH: u64 = 1_000;
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

/// Epochs from a claim to its single verdict (decide-by, from USEFULNESS_DECIDE_BY_EPOCH).
pub const DECIDE_BY_EPOCHS: u64 = 2_880;

/// The usefulness decide-by gate. Tests exercise it from 360,000 (older tests stay on the
/// immediate rule).
pub fn decide_by_epoch() -> u64 {
    if cfg!(test) { 360_000 } else { hone_types::USEFULNESS_DECIDE_BY_EPOCH }
}

/// A claim made under decide-by carries its `decide_by` epoch.
fn deferred(c: &serde_json::Value) -> bool { c.get("decide_by").and_then(|v| v.as_u64()).is_some() }

/// Record evidence on a pending decide-by claim: `kind` = "corr" | "contra", once per owner,
/// only if applied at or before its decide_by. Returns true if recorded.
fn note(chain: &Chain, ccell: &str, m: &str, w: u64, kind: &str, by: &str, epoch: u64) -> bool {
    let ck = claim_key(ccell, m, w);
    let Some(mut c) = get_json(chain, &ck) else { return false };
    if c["status"] != "pending" { return false; }
    let Some(db) = c["decide_by"].as_u64() else { return false };
    if epoch > db { return false; }
    let mut list: Vec<String> = c[kind].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    if list.iter().any(|x| x == by) { return false; }
    list.push(by.to_owned());
    list.sort();
    c[kind] = serde_json::json!(list);
    set_json(chain, &ck, &c);
    true
}

/// The single verdict of a claim at its due epoch. Legacy claims (no decide_by): proven,
/// as before. Decide-by claims: proven unless contradicted by at least as many owners as
/// corroborated it; then the owner's trust moves (+1 per corroborator, -2 per contradictor),
/// the owner is paid and each corroborator gets VERIFY_POINTS, all at decide_by.
fn decide(chain: &Chain, cell: &str, m: &str, w: u64, epoch: u64) {
    let Some(c) = get_json(chain, &claim_key(cell, m, w)) else { return };
    if !deferred(&c) { settle(chain, cell, m, w, true, epoch); return; }
    if c["status"] != "pending" { return; }
    let list = |k: &str| -> Vec<String> { c[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default() };
    let (corr, contra) = (list("corr"), list("contra"));
    let proven = contra.is_empty() || corr.len() > contra.len();
    let owner = c["owner"].as_str().unwrap_or("").to_owned();
    let dt = corr.len() as i64 - 2 * contra.len() as i64;
    if dt != 0 { add_trust(chain, &owner, dt); }
    settle(chain, cell, m, w, proven, epoch);
    if proven { for v in &corr { credit(chain, v, VERIFY_POINTS, epoch); } }
}

/// Coarsest and finest H3 resolution a summary cell may have from CELL_RES_RANGE_EPOCH.
pub const MIN_CELL_RES: u8 = 7;
pub const MAX_CELL_RES: u8 = 9;

/// The cell-resolution gate. Tests exercise the rule as if it were active from 352,000.
pub fn cell_res_epoch() -> u64 {
    if cfg!(test) { 352_000 } else { hone_types::CELL_RES_RANGE_EPOCH }
}
/// The lora/wifi metric-family gate. Tests: active from 352,000.
pub fn families_epoch() -> u64 {
    if cfg!(test) { 352_000 } else { hone_types::SENSOR_FAMILIES_EPOCH }
}

fn h3(cell: &str) -> Option<h3o::CellIndex> {
    u64::from_str_radix(cell, 16).ok().and_then(|v| h3o::CellIndex::try_from(v).ok())
}
fn hex(c: h3o::CellIndex) -> String { format!("{:x}", u64::from(c)) }

/// H3 resolution of a cell string (None if it is not a valid cell index).
pub fn cell_res(cell: &str) -> Option<u8> { h3(cell).map(|c| u8::from(c.resolution())) }

/// Cell check at `epoch`: the format before CELL_RES_RANGE_EPOCH (unchanged), a real H3 cell
/// at res 7..=9 from it.
pub fn valid_cell_at(cell: &str, epoch: u64) -> bool {
    valid_cell(cell)
        && (epoch < cell_res_epoch()
            || cell_res(cell).is_some_and(|r| (MIN_CELL_RES..=MAX_CELL_RES).contains(&r)))
}

/// Noise band at `epoch`: band(), plus the lora/wifi family bands from SENSOR_FAMILIES_EPOCH.
pub fn band_at(metric: &str, est: i64, epoch: u64) -> i64 {
    if epoch >= families_epoch() {
        let m = metric;
        if m.starts_with("lora.") && !(m.ends_with("rssi_p50") || m.ends_with("rssi_max")) {
            if m.ends_with("snr_p50") { return 3; }
            return (est.abs() * 30 / 100).max(2);
        }
        if m.starts_with("wifi.fp:") { return 8; }
    }
    band(metric, est)
}

/// scored_metric at `epoch`: from SENSOR_FAMILIES_EPOCH Wi-Fi fingerprint tokens are location
/// context (an anchor against GPS spoofing), accepted and kept but never scored or paid.
pub fn scored_metric_at(m: &str, epoch: u64) -> bool {
    scored_metric(m) && !(epoch >= families_epoch() && m.starts_with("wifi.fp:"))
}

/// Context metrics describe effort or location quality, not the environment (scan counts,
/// sample counts, histograms): accepted in a summary but never scored or paid.
pub fn scored_metric(m: &str) -> bool {
    !(m.starts_with("loc.") || m.ends_with(".scans") || m.ends_with(".scan_s") || m.ends_with(".samples")
      || m.contains(".rssi_hist."))
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

/// The owner a report counts for: a device seat `owner@<pubkey>` counts as its owner, so
/// one owner's phones, Flippers and seats are one reporter.
pub fn owner_of(a: &str) -> String {
    hone_types::parse_device_clock_id(a).map(|(o, _)| o.to_owned()).unwrap_or_else(|| a.to_owned())
}

pub fn trust(chain: &Chain, owner: &str) -> i64 {
    chain.store.state_get(&format!("utrust:{owner}")).and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// The H3 cells within 1 ring of `cell`, excluding it (empty for an invalid index).
pub fn ring1(cell: &str) -> Vec<String> {
    let Some(c) = u64::from_str_radix(cell, 16).ok().and_then(|v| h3o::CellIndex::try_from(v).ok()) else { return Vec::new() };
    let mut out: Vec<String> = c.grid_disk::<Vec<_>>(1).into_iter()
        .filter(|n| *n != c).map(|n| format!("{:x}", u64::from(n))).collect();
    out.sort();
    out
}

/// A report by `owner` of (`metric`, `value`) in `cell`/`window` proves pending claims of
/// other owners in the 1-ring neighbourhood within ±VERIFY_WINDOWS whose value it matches.
/// Returns the verification points earned (near 0).
fn corroborate_ring(chain: &Chain, owner: &str, cell: &str, window: u64, m: &str, v: i64, epoch: u64) -> u64 {
    if trust(chain, owner) < TRUST_FLOOR { return 0; }
    let mut pts = 0;
    for n in ring1(cell) {
        for w in window.saturating_sub(VERIFY_WINDOWS)..=window + VERIFY_WINDOWS {
            pts += try_prove(chain, owner, &n, m, w, v, epoch);
        }
    }
    pts
}

/// A report of (`m`, `v`) by `owner` proves another owner's pending claim at
/// (`ccell`, `m`, `w`) if `v` is within the claim's noise band. Once per (claim, owner).
fn try_prove(chain: &Chain, owner: &str, ccell: &str, m: &str, w: u64, v: i64, epoch: u64) -> u64 {
    let ck = claim_key(ccell, m, w);
    let Some(c) = get_json(chain, &ck) else { return 0 };
    if c["status"] != "pending" || c["owner"].as_str() == Some(owner) { return 0; }
    let Some(cv) = c["value"].as_i64() else { return 0 };
    if (v - cv).abs() > band_at(m, cv, epoch) { return 0; }
    let vkey = format!("uver:{ccell}:{m}:{w}:{owner}");
    if chain.store.state_get(&vkey).is_some() { return 0; }
    if deferred(&c) {
        // Decide-by: evidence only; verdict and pay at decide_by.
        if note(chain, ccell, m, w, "corr", owner, epoch) { let _ = chain.store.state_set(&vkey, b"1"); }
        return 0;
    }
    let _ = chain.store.state_set(&vkey, b"1");
    if let Some(co) = c["owner"].as_str() { add_trust(chain, co, 1); }
    settle(chain, ccell, m, w, true, epoch);
    credit(chain, owner, VERIFY_POINTS, epoch);
    VERIFY_POINTS
}

/// `cell` and its 1-ring (the cell first).
fn disk1(c: h3o::CellIndex) -> Vec<String> {
    let mut out = vec![hex(c)];
    let mut ring: Vec<String> = c.grid_disk::<Vec<_>>(1).into_iter().filter(|n| *n != c).map(hex).collect();
    ring.sort();
    out.extend(ring);
    out
}

/// Index key of a pending claim at a finer cell under one of its coarser ancestors (res >= 7),
/// so a coarse report can find the finer claims it contains.
fn up_key(ancestor: &str, m: &str, w: u64, cell: &str) -> String { format!("uclaim_up:{ancestor}:{m}:{w}:{cell}") }

fn index_claim_up(chain: &Chain, cell: &str, m: &str, w: u64) {
    let Some(c) = h3(cell) else { return };
    let res = u8::from(c.resolution());
    for r in MIN_CELL_RES..res {
        let Ok(rr) = h3o::Resolution::try_from(r) else { continue };
        if let Some(p) = c.parent(rr) { let _ = chain.store.state_set(&up_key(&hex(p), m, w, cell), b"1"); }
    }
}

/// Corroboration across resolutions by containment (from CELL_RES_RANGE_EPOCH), with the
/// 1-ring rule applied at the coarser resolution of the pair:
/// - a finer report proves a coarser claim whose cell is, or neighbours, the report's
///   ancestor at the claim's resolution;
/// - a coarser report proves a finer claim whose ancestor at the report's resolution is the
///   report's cell or one of its neighbours.
fn corroborate_cross_res(chain: &Chain, owner: &str, cell: &str, window: u64, m: &str, v: i64, epoch: u64) -> u64 {
    if trust(chain, owner) < TRUST_FLOOR { return 0; }
    let Some(c) = h3(cell) else { return 0 };
    let res = u8::from(c.resolution());
    let wins = window.saturating_sub(VERIFY_WINDOWS)..=window + VERIFY_WINDOWS;
    let mut pts = 0;
    for r in MIN_CELL_RES..res {
        let Some(p) = h3o::Resolution::try_from(r).ok().and_then(|rr| c.parent(rr)) else { continue };
        for n in disk1(p) {
            for w in wins.clone() { pts += try_prove(chain, owner, &n, m, w, v, epoch); }
        }
    }
    for n in disk1(c) {
        for w in wins.clone() {
            let pre = format!("uclaim_up:{n}:{m}:{w}:");
            for (k, _) in chain.store.state_scan_prefix(&pre) {
                let fine = &k[pre.len()..];
                if !valid_cell(fine) { continue; }
                pts += try_prove(chain, owner, fine, m, w, v, epoch);
            }
        }
    }
    pts
}

/// Score one summary. Writes only usefulness state. Returns per-metric verdicts and the
/// points credited to the owner for `epoch`.
pub fn score(chain: &Chain, sensor_id: &str, owner: &str, cell: &str, window: u64,
             metrics: &BTreeMap<String, i64>, epoch: u64) -> (Vec<(String, Verdict)>, u64) {
    if crate::data_rules::unscored_summary(cell, epoch) { return (Vec::new(), 0); }
    let owner = owner_of(owner);
    let mut out = Vec::new();
    let mut pts = 0u64;
    for (m, &v) in metrics {
        if !scored_metric_at(m, epoch) { continue; }
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
                let same = (v - ev).abs() <= band_at(m, ev, epoch);
                let near = window.abs_diff(ew) <= VERIFY_WINDOWS;
                if eo != owner && near {
                    if trust(chain, &owner) < TRUST_FLOOR { Verdict::Repeat }
                    else if same { Verdict::Verify } else { Verdict::Contradict }
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
                        let mut cj = serde_json::json!({ "owner": owner, "points": NOVEL_POINTS,
                            "epoch": epoch, "status": "pending", "value": v, "sensor": sensor_id });
                        if epoch >= decide_by_epoch() { cj["decide_by"] = serde_json::json!(epoch + DECIDE_BY_EPOCHS); }
                        set_json(chain, &ck, &cj);
                        let _ = chain.store.state_set(&format!("uclaim_o:{owner}:{cell}:{m}:{window}"), b"1");
                        let _ = chain.store.state_set(&format!("uclaim_due:{:016x}:{cell}:{m}:{window}", epoch + CLAIM_WINDOW_EPOCHS), b"1");
                        if epoch >= cell_res_epoch() { index_claim_up(chain, cell, m, window); }
                        pts += NOVEL_POINTS;
                    }
                }
            }
            Verdict::Verify => {
                let ew = est.as_ref().and_then(|e| e["window"].as_u64());
                let vkey = format!("uver:{cell}:{m}:{}:{owner}", ew.unwrap_or(0));
                let dclaim = ew.and_then(|w| get_json(chain, &claim_key(cell, m, w))).filter(deferred);
                if chain.store.state_get(&vkey).is_none() {
                    if dclaim.is_some() {
                        // Decide-by: evidence only; verdict and pay at decide_by.
                        if note(chain, cell, m, ew.unwrap_or(0), "corr", &owner, epoch) { let _ = chain.store.state_set(&vkey, b"1"); }
                    } else {
                        let _ = chain.store.state_set(&vkey, b"1");
                        add_trust(chain, &est_owner, 1);
                        if let Some(ew) = ew { settle(chain, cell, m, ew, true, epoch); }
                        credit(chain, &owner, VERIFY_POINTS, epoch);
                        pts += VERIFY_POINTS;
                    }
                }
            }
            Verdict::Contradict => {
                let ew = est.as_ref().and_then(|e| e["window"].as_u64());
                let dclaim = ew.and_then(|w| get_json(chain, &claim_key(cell, m, w))).filter(deferred);
                if dclaim.is_some() {
                    note(chain, cell, m, ew.unwrap_or(0), "contra", &owner, epoch);
                } else {
                    add_trust(chain, &est_owner, -2);
                    if let Some(ew) = ew { settle(chain, cell, m, ew, false, epoch); }
                }
            }
            _ => {}
        }
        // Neighbouring cells: this report may corroborate other owners' pending claims.
        pts += corroborate_ring(chain, &owner, cell, window, m, v, epoch);
        if epoch >= cell_res_epoch() { pts += corroborate_cross_res(chain, &owner, cell, window, m, v, epoch); }
        out.push((m.clone(), verdict));
    }
    (out, pts)
}

fn claim_key(cell: &str, m: &str, window: u64) -> String { format!("uclaim:{cell}:{m}:{window}") }

fn read_u64(chain: &Chain, k: &str) -> u64 {
    chain.store.state_get(k).and_then(|b| String::from_utf8(b).ok()).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Proven points: added to the owner's total and payout queue; release_due pays the queue
/// out at most RELEASE_CAP_PER_EPOCH per owner per epoch.
fn credit(chain: &Chain, owner: &str, pts: u64, _epoch: u64) {
    for k in [format!("uproven:{owner}"), format!("uqueue:{owner}")] {
        let _ = chain.store.state_set(&k, (read_u64(chain, &k) + pts).to_string().as_bytes());
    }
}

/// Move up to the cap from every owner's queue into `uproven_e:{epoch}:{owner}`.
fn release_queues(chain: &Chain, epoch: u64) {
    for (k, v) in chain.store.state_scan_prefix("uqueue:") {
        let owner = &k["uqueue:".len()..];
        let q: u64 = String::from_utf8(v).ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        if q == 0 { continue; }
        let pay = q.min(RELEASE_CAP_PER_EPOCH);
        let ek = format!("uproven_e:{epoch}:{owner}");
        let _ = chain.store.state_set(&ek, (read_u64(chain, &ek) + pay).to_string().as_bytes());
        if q == pay { let _ = chain.store.state_delete(&k); }
        else { let _ = chain.store.state_set(&k, (q - pay).to_string().as_bytes()); }
    }
}

/// Paid out (released) points of an owner at `epoch`.
pub fn released_at(chain: &Chain, owner: &str, epoch: u64) -> u64 { read_u64(chain, &format!("uproven_e:{epoch}:{owner}")) }
/// Proven points still waiting for release.
pub fn queued(chain: &Chain, owner: &str) -> u64 { read_u64(chain, &format!("uqueue:{owner}")) }

/// GET /api/sensor/usefulness/:owner — totals and the newest claims of an owner.
pub fn owner_view(chain: &Chain, owner: &str) -> serde_json::Value {
    let owner = owner_of(owner);
    let pre = format!("uclaim_o:{owner}:");
    let mut counts = BTreeMap::<String, u64>::new();
    let mut claims = Vec::new();
    for (k, _) in chain.store.state_scan_prefix(&pre) {
        let rest = &k[pre.len()..];
        let mut p = rest.splitn(3, ':');
        let (Some(cell), Some(m), Some(w)) = (p.next(), p.next(), p.next()) else { continue };
        let Ok(w) = w.parse::<u64>() else { continue };
        let Some(c) = get_json(chain, &claim_key(cell, m, w)) else { continue };
        let st = c["status"].as_str().unwrap_or("pending").to_owned();
        *counts.entry(st.clone()).or_default() += 1;
        claims.push(serde_json::json!({ "cell": cell, "metric": m, "window": w, "status": st,
            "points": c["points"], "claimed_epoch": c["epoch"], "settled_epoch": c["settled_epoch"] }));
    }
    claims.sort_by_key(|c| std::cmp::Reverse(c["claimed_epoch"].as_u64().unwrap_or(0)));
    claims.truncate(50);
    let res_gate = cell_res_epoch();
    serde_json::json!({ "owner": owner, "active_from_epoch": hone_types::USEFULNESS_SCORING_EPOCH,
        // Read by the phone app (CoverageSubmitter.coarseCellsActive): res 7-8 rows are held
        // until the tip reaches this epoch.
        "coarse_cells_from_epoch": res_gate,
        "summary_min_res": if chain.current_epoch() >= res_gate { MIN_CELL_RES } else { MAX_CELL_RES },
        "proven_points": proven_points(chain, &owner), "queued_points": queued(chain, &owner),
        "trust": trust(chain, &owner), "counts": counts, "claims": claims,
        "claim_window_epochs": CLAIM_WINDOW_EPOCHS, "release_cap_per_epoch": RELEASE_CAP_PER_EPOCH })
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
/// The payout gate: the same epoch as scoring.
pub fn payout_epoch() -> u64 {
    if cfg!(test) { 352_700 } else { USEFULNESS_PAYOUT_EPOCH }
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
        if let Ok(w) = w.parse::<u64>() { decide(chain, cell, m, w, epoch); n += 1; }
        let _ = chain.store.state_delete(&k);
    }
    release_queues(chain, epoch);
    n
}

/// Total proven points of an owner.
pub fn proven_points(chain: &Chain, owner: &str) -> u64 { read_u64(chain, &format!("uproven:{owner}")) }

/// Proven points per owner released at `epoch`: the deferred payout input. Empty until
/// USEFULNESS_PAYOUT_EPOCH (off), so nothing is paid by the epoch a summary lands in.
pub fn points_for_epoch(chain: &Chain, epoch: u64) -> Vec<(String, u64)> {
    if epoch < payout_epoch() { return Vec::new(); }
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
        // Payout: proven points are released (capped) at the releasing epoch, and only then.
        let paid: u64 = points_for_epoch(&c, E0 + CLAIM_WINDOW_EPOCHS).iter().filter(|(o, _)| o == "josh").map(|(_, p)| *p).sum();
        assert_eq!(paid, 2 * NOVEL_POINTS);
        assert!(points_for_epoch(&c, E0).is_empty(), "nothing is paid in the submit epoch");
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
        // Paid out (released) totals match too.
        let paid = |spread: bool| {
            let (c, _d) = chain();
            for (i, (w, v)) in rows.iter().enumerate() {
                let e = if spread { E0 + i as u64 * 7 } else { E0 };
                score(&c, "phone-a", "josh", C, *w, &m(&[("wifi.ap_count", *v)]), e);
            }
            (E0 + CLAIM_WINDOW_EPOCHS..E0 + CLAIM_WINDOW_EPOCHS + 12 * 7 + 5)
                .map(|e| { release_due(&c, e); released_at(&c, "josh", e) }).sum::<u64>()
        };
        assert_eq!(paid(false), paid(true));
        assert_eq!(paid(false), 12 * NOVEL_POINTS);
    }

    #[test]
    fn release_cap_spreads_a_bulk_backlog() {
        let (c, _d) = chain();
        let s: BTreeMap<String, i64> = (0..25).map(|i| (format!("wifi.m{i}"), 10 + i)).collect();
        let (_, p) = score(&c, "phone-a", "josh", C, W0, &s, E0);
        assert_eq!(p, 25 * NOVEL_POINTS);
        let e = E0 + CLAIM_WINDOW_EPOCHS;
        release_due(&c, e);
        assert_eq!(proven_points(&c, "josh"), 2_500);
        assert_eq!(released_at(&c, "josh", e), RELEASE_CAP_PER_EPOCH);
        assert_eq!(queued(&c, "josh"), 1_500);
        release_due(&c, e + 1);
        release_due(&c, e + 2);
        assert_eq!(released_at(&c, "josh", e + 1), 1_000);
        assert_eq!(released_at(&c, "josh", e + 2), 500);
        assert_eq!(queued(&c, "josh"), 0);
    }

    #[test]
    fn ring_neighbour_corroborates_only_for_another_trusted_owner() {
        let (c, _d) = chain();
        let n = ring1(C);
        assert_eq!(n.len(), 6, "a res-9 cell has 6 neighbours");
        score(&c, "phone-a", "josh", C, W0, &m(&[("wifi.ap_count", 20)]), E0);
        // The same owner's other device / seat next door: never corroborates.
        let seat = format!("josh@{}", "cd".repeat(32));
        score(&c, "seat", &seat, &n[0], W0 + 1, &m(&[("wifi.ap_count", 20)]), E0 + 1);
        assert_eq!(claim(&c, W0, "wifi.ap_count").unwrap()["status"], "pending");
        // Another owner, but 3 windows away: no.
        score(&c, "fl-b", "natoshisakamoto", &n[1], W0 + 3, &m(&[("wifi.ap_count", 20)]), E0 + 1);
        assert_eq!(claim(&c, W0, "wifi.ap_count").unwrap()["status"], "pending");
        // Another owner with trust below the floor: no.
        c.store.state_set("utrust:lowtrust", b"-3").unwrap();
        score(&c, "x", "lowtrust", &n[2], W0 + 1, &m(&[("wifi.ap_count", 20)]), E0 + 1);
        assert_eq!(claim(&c, W0, "wifi.ap_count").unwrap()["status"], "pending");
        // Another owner, outside the band: no.
        score(&c, "fl-c", "shindevlin", &n[3], W0 - 1, &m(&[("wifi.ap_count", 90)]), E0 + 2);
        assert_eq!(claim(&c, W0, "wifi.ap_count").unwrap()["status"], "pending");
        // Another trusted owner next door, within ±2 windows and the band: proven.
        score(&c, "fl-d", "bob", &n[4], W0 + 2, &m(&[("wifi.ap_count", 21)]), E0 + 3);
        assert_eq!(claim(&c, W0, "wifi.ap_count").unwrap()["status"], "proven");
        // (josh's seat claim in n[0] may be proven by the same report: still josh's own data.)
        assert!(proven_points(&c, "josh") >= NOVEL_POINTS);
        assert!(proven_points(&c, "bob") >= VERIFY_POINTS);
    }

    #[test]
    fn context_metrics_are_never_paid() {
        let (c, _d) = chain();
        let (v, p) = score(&c, "phone-a", "josh", C, W0,
            &m(&[("loc.samples", 40), ("wifi.scans", 9), ("ble.scan_s", 30), ("wifi.rssi_hist.3", 5), ("wifi.ap_count", 7)]), E0);
        assert_eq!(p, NOVEL_POINTS);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn subghz_uses_the_same_rule() {
        let (c, _d) = chain();
        let (_, p) = score(&c, "fl-b", "natoshisakamoto", C, W0, &m(&[("subghz.433", 5)]), E0);
        assert_eq!(p, NOVEL_POINTS);
        let (_, p) = score(&c, "fl-b", "natoshisakamoto", C, W0 + 1, &m(&[("subghz.433", 6)]), E0);
        assert_eq!(p, 0);
    }

    // Cells around C for the cross-resolution tests.
    const P7: &str = "876d75d96ffffff"; // C's res-7 parent
    const P8: &str = "886d75d961fffff"; // C's res-8 parent
    const N7: &str = "876d74365ffffff"; // a res-7 neighbour of P7
    const C9N7: &str = "896d7436503ffff"; // a res-9 cell inside N7
    const FAR9: &str = "896d7436003ffff"; // a res-9 cell whose res-7 parent is 2 rings from P7
    const BEFORE: u64 = 351_999; // test gates (cell res, families) are active from 352,000

    #[test]
    fn cell_resolution_range_is_gated() {
        assert_eq!(cell_res(C), Some(9));
        assert_eq!(cell_res(P7), Some(7));
        assert_eq!(cell_res(P8), Some(8));
        for c in [P7, P8, C] { assert!(valid_cell_at(c, E0), "{c} accepted after the gate"); }
        // Res 6 and res 10: accepted before the gate (format check only), rejected after.
        for c in ["866d75d97ffffff", "8a6d75d96087fff"] {
            assert!(valid_cell_at(c, BEFORE), "{c} before the gate");
            assert!(!valid_cell_at(c, E0), "{c} after the gate");
        }
        // A well-formed string that is not an H3 cell: format-only before, rejected after.
        assert!(valid_cell_at("8fffffffffffff0", BEFORE));
        assert!(!valid_cell_at("8fffffffffffff0", E0));
    }

    #[test]
    fn novelty_is_per_cell_at_its_own_resolution() {
        let (c, _d) = chain();
        // A res-7 home summary and a res-9 phone summary of another owner inside it are each
        // the first report for their own (cell, window, metric): both novel.
        let (v7, p7) = score(&c, "nebra-wifi", "josh", P7, W0, &m(&[("wifi.ap_count", 30)]), E0);
        assert_eq!((v7[0].1.clone(), p7), (Verdict::Novel, NOVEL_POINTS));
        let (v9, _) = score(&c, "phone-b", "bob", C, W0 + 5, &m(&[("wifi.ap_count", 30)]), E0);
        assert_eq!(v9[0].1, Verdict::Novel);
        assert!(get_json(&c, &format!("uest:{P7}:wifi.ap_count")).is_some());
        assert!(get_json(&c, &format!("uest:{C}:wifi.ap_count")).is_some());
    }

    #[test]
    fn a_finer_report_proves_a_coarser_claim_it_lies_in_or_next_to() {
        let (c, _d) = chain();
        let ck = |cell: &str, w: u64| get_json(&c, &claim_key(cell, "lora.packets", w)).unwrap()["status"].clone();
        score(&c, "nebra-lora", "josh", P7, W0, &m(&[("lora.packets", 40)]), E0);
        score(&c, "nebra-lora", "josh", N7, W0 + 10, &m(&[("lora.packets", 40)]), E0);
        // Same owner's phone inside: never.
        score(&c, "phone-a", "josh", C, W0 + 1, &m(&[("lora.packets", 40)]), E0 + 1);
        assert_eq!(ck(P7, W0), "pending");
        // Another owner far away (res-7 parent 2 rings out): no.
        score(&c, "gw-b", "bob", FAR9, W0 + 1, &m(&[("lora.packets", 40)]), E0 + 1);
        assert_eq!(ck(P7, W0), "pending");
        // Another owner at res 9 inside P7, within the band (30 %): proven.
        score(&c, "gw-b", "bob", C, W0 + 2, &m(&[("lora.packets", 50)]), E0 + 2);
        assert_eq!(ck(P7, W0), "proven");
        // The 1-ring rule at the coarser resolution: a res-8 report inside P7 proves N7's claim
        // (a res-7 neighbour), once per (claim, owner).
        assert_eq!(ck(N7, W0 + 10), "pending");
        let before = proven_points(&c, "carol");
        score(&c, "gw-c", "carol", P8, W0 + 11, &m(&[("lora.packets", 41)]), E0 + 3);
        assert_eq!(ck(N7, W0 + 10), "proven");
        assert_eq!(proven_points(&c, "carol") - before, VERIFY_POINTS);
    }

    #[test]
    fn a_coarser_report_proves_finer_claims_it_contains_or_neighbours() {
        let (c, _d) = chain();
        let ck = |cell: &str| get_json(&c, &claim_key(cell, "wifi.ap_count", W0)).unwrap()["status"].clone();
        score(&c, "phone-b", "bob", C, W0, &m(&[("wifi.ap_count", 25)]), E0);
        score(&c, "phone-c", "carol", C9N7, W0, &m(&[("wifi.ap_count", 25)]), E0);
        score(&c, "phone-d", "dave", FAR9, W0, &m(&[("wifi.ap_count", 25)]), E0);
        // josh's res-7 home summary at P7: proves C (inside) and C9N7 (inside a neighbour of P7),
        // not FAR9 (two rings out at res 7).
        score(&c, "nebra-wifi", "josh", P7, W0 + 1, &m(&[("wifi.ap_count", 26)]), E0 + 1);
        assert_eq!(ck(C), "proven");
        assert_eq!(ck(C9N7), "proven");
        assert_eq!(ck(FAR9), "pending");
        assert_eq!(proven_points(&c, "josh"), 2 * VERIFY_POINTS);
    }

    #[test]
    fn cross_resolution_corroboration_is_off_before_the_gate() {
        let (c, _d) = chain();
        score(&c, "nebra-lora", "josh", P7, W0, &m(&[("lora.packets", 40)]), BEFORE);
        score(&c, "gw-b", "bob", C, W0 + 1, &m(&[("lora.packets", 40)]), BEFORE);
        assert_eq!(get_json(&c, &claim_key(P7, "lora.packets", W0)).unwrap()["status"], "pending");
        assert!(c.store.state_scan_prefix("uclaim_up:").is_empty());
    }

    #[test]
    fn lora_and_wifi_family_bands_are_gated() {
        // LoRa counts: 30 % / min 2 after the gate; 20 % / min 1 before.
        assert_eq!(band_at("lora.packets", 10, E0), 3);
        assert_eq!(band_at("lora.packets", 10, BEFORE), 2);
        assert_eq!(band_at("lora.ch3", 0, E0), 2);
        assert_eq!(band_at("lora.devaddr_unique", 20, E0), 6);
        assert_eq!(band_at("lora.snr_p50", 7, E0), 3);
        assert_eq!(band_at("lora.snr_p50", 7, BEFORE), 1);
        assert_eq!(band_at("lora.rssi_p50", -110, E0), 6);
        assert_eq!(band_at("wifi.fp:0a1b2c3d", -60, E0), 8);
        assert_eq!(band_at("wifi.ap_count", 20, E0), band("wifi.ap_count", 20));
        // Next window, same day: within the new band is a repeat; before the gate it was novel.
        let (c, _d) = chain();
        score(&c, "nebra-lora", "josh", P7, W0, &m(&[("lora.packets", 10), ("lora.snr_p50", 5)]), E0);
        let (v, p) = score(&c, "nebra-lora", "josh", P7, W0 + 1, &m(&[("lora.packets", 13), ("lora.snr_p50", 8)]), E0);
        assert_eq!(p, 0);
        assert!(v.iter().all(|(_, x)| *x == Verdict::Repeat));
        let (c2, _d2) = chain();
        score(&c2, "nebra-lora", "josh", P7, W0, &m(&[("lora.packets", 10), ("lora.snr_p50", 5)]), BEFORE);
        let (_, p) = score(&c2, "nebra-lora", "josh", P7, W0 + 1, &m(&[("lora.packets", 13), ("lora.snr_p50", 8)]), BEFORE);
        assert_eq!(p, 2 * NOVEL_POINTS);
    }

    #[test]
    fn wifi_fingerprint_tokens_are_context_after_the_gate() {
        let (c, _d) = chain();
        let s = m(&[("wifi.ap_count", 29), ("wifi.fp:0a1b2c3d", -48), ("wifi.fp:99887766", -61)]);
        let (v, p) = score(&c, "nebra-wifi", "josh", P7, W0, &s, E0);
        assert_eq!((v.len(), p), (1, NOVEL_POINTS));
        let (c2, _d2) = chain();
        let (v, p) = score(&c2, "nebra-wifi", "josh", P7, W0, &s, BEFORE);
        assert_eq!((v.len(), p), (3, 3 * NOVEL_POINTS));
    }

    const G: u64 = 360_000; // test decide-by gate
    fn st(c: &Chain, cell: &str, w: u64, m: &str) -> serde_json::Value { get_json(c, &claim_key(cell, m, w)).unwrap()["status"].clone() }

    #[test]
    fn decide_by_claim_waits_and_pays_once_at_decide_by() {
        let (c, _d) = chain();
        score(&c, "phone-a", "josh", C, W0, &m(&[("wifi.ap_count", 20)]), G);
        let cl = get_json(&c, &claim_key(C, "wifi.ap_count", W0)).unwrap();
        assert_eq!(cl["decide_by"].as_u64(), Some(G + DECIDE_BY_EPOCHS));
        // Same-cell verification and a ring neighbour: evidence only, nothing paid, still pending.
        let (_, p) = score(&c, "phone-b", "bob", C, W0 + 1, &m(&[("wifi.ap_count", 21)]), G + 1);
        let n = ring1(C);
        let (_, p2) = score(&c, "fl-c", "carol", &n[0], W0 + 1, &m(&[("wifi.ap_count", 20)]), G + 2);
        assert_eq!((p, p2), (0, NOVEL_POINTS)); // carol's own novel claim only
        assert_eq!(st(&c, C, W0, "wifi.ap_count"), "pending");
        assert_eq!(proven_points(&c, "josh") + proven_points(&c, "bob") + proven_points(&c, "carol"), 0);
        assert_eq!(trust(&c, "josh"), 0);
        // Not before decide_by.
        release_due(&c, G + DECIDE_BY_EPOCHS - 1);
        assert_eq!(st(&c, C, W0, "wifi.ap_count"), "pending");
        release_due(&c, G + DECIDE_BY_EPOCHS);
        assert_eq!(st(&c, C, W0, "wifi.ap_count"), "proven");
        assert_eq!(proven_points(&c, "josh"), NOVEL_POINTS);
        assert_eq!(proven_points(&c, "bob"), VERIFY_POINTS);
        assert_eq!(proven_points(&c, "carol"), VERIFY_POINTS); // carol's own claim (in n[0]) is due 2 epochs later
        assert_eq!(trust(&c, "josh"), 2);
        // Settled once: running the due epoch again changes nothing.
        release_due(&c, G + DECIDE_BY_EPOCHS + 1);
        assert_eq!(proven_points(&c, "josh"), NOVEL_POINTS);
    }

    #[test]
    fn decide_by_verdict_does_not_depend_on_arrival_order() {
        // One corroboration and one contradiction: a tie voids, in either order.
        // Two corroborations and one contradiction: proven, in any order.
        let run = |order: &[(&str, &str, i64)]| -> (serde_json::Value, u64, i64) {
            let (c, _d) = chain();
            score(&c, "phone-a", "josh", C, W0, &m(&[("wifi.ap_count", 20)]), G);
            for (i, (dev, own, v)) in order.iter().enumerate() {
                score(&c, dev, own, C, W0 + 1, &m(&[("wifi.ap_count", *v)]), G + 1 + i as u64);
            }
            release_due(&c, G + DECIDE_BY_EPOCHS);
            (st(&c, C, W0, "wifi.ap_count"), proven_points(&c, "josh"), trust(&c, "josh"))
        };
        let (a, b) = (("pb", "bob", 21), ("pc", "carol", 90));
        assert_eq!(run(&[a, b]), run(&[b, a]));
        assert_eq!(run(&[a, b]).0, "void");
        let d = ("pd", "dave", 19);
        let r1 = run(&[a, b, d]); let r2 = run(&[b, d, a]); let r3 = run(&[d, a, b]);
        assert_eq!(r1, r2); assert_eq!(r2, r3);
        assert_eq!(r1.0, "proven");
        assert_eq!((r1.1, r1.2), (NOVEL_POINTS, 2 - 2));
    }

    #[test]
    fn decide_by_ignores_evidence_after_decide_by() {
        let (c, _d) = chain();
        score(&c, "phone-a", "josh", C, W0, &m(&[("wifi.ap_count", 20)]), G);
        // A contradiction applied after decide_by (release not run yet): ignored.
        assert!(!note(&c, C, "wifi.ap_count", W0, "contra", "bob", G + DECIDE_BY_EPOCHS + 1));
        assert!(note(&c, C, "wifi.ap_count", W0, "corr", "carol", G + DECIDE_BY_EPOCHS));
        assert!(!note(&c, C, "wifi.ap_count", W0, "corr", "carol", G + 5), "once per owner");
        release_due(&c, G + DECIDE_BY_EPOCHS + 1);
        assert_eq!(st(&c, C, W0, "wifi.ap_count"), "proven");
        assert_eq!(proven_points(&c, "carol"), VERIFY_POINTS);
    }

    #[test]
    fn decide_by_cross_resolution_evidence_also_waits() {
        let (c, _d) = chain();
        score(&c, "nebra-lora", "josh", P7, W0, &m(&[("lora.packets", 40)]), G);
        score(&c, "gw-b", "bob", C, W0 + 1, &m(&[("lora.packets", 44)]), G + 1);
        assert_eq!(st(&c, P7, W0, "lora.packets"), "pending");
        assert_eq!(proven_points(&c, "bob"), 0);
        release_due(&c, G + DECIDE_BY_EPOCHS);
        assert_eq!(st(&c, P7, W0, "lora.packets"), "proven");
        assert_eq!(proven_points(&c, "bob"), VERIFY_POINTS);
    }

    #[test]
    fn claims_before_the_gate_keep_the_immediate_rule() {
        let (c, _d) = chain();
        score(&c, "phone-a", "josh", C, W0, &m(&[("wifi.ap_count", 20)]), G - 10);
        assert!(get_json(&c, &claim_key(C, "wifi.ap_count", W0)).unwrap().get("decide_by").is_none());
        score(&c, "phone-b", "bob", C, W0 + 1, &m(&[("wifi.ap_count", 21)]), G + 5);
        assert_eq!(st(&c, C, W0, "wifi.ap_count"), "proven");
        assert_eq!(proven_points(&c, "bob"), VERIFY_POINTS);
    }

    #[test]
    fn cells_and_metrics_are_validated() {
        assert!(valid_cell(C));
        assert!(!valid_cell("896D75D960BFFFF"));
        assert!(!valid_cell("abc"));
        assert!(valid_metric("wifi.ap_count") && valid_metric("subghz.433") && !valid_metric("Wifi Count"));
    }
}
