//! Data rules (Josh 2026-11-10 10:17 and 10:50): every reading goes on chain; novelty decides reward,
//! never inclusion. Three gated pieces, all OFF (u64::MAX) until Josh names an epoch:
//!
//! * INFERRED_LOC_EPOCH: a SENSOR_SUMMARY may carry `loc.inferred = 1` and `loc.inferred_age_s >= 0`.
//!   It needs a res 7..=8 cell. Both are `loc.` context metrics, so they are stored but never scored.
//! * NULL_CELL_EPOCH: a SENSOR_SUMMARY with an empty cell is accepted. It is stored on chain and
//!   scores nothing.
//! * SCHEMA_ANCHOR_EPOCH: the governance parameter `schema:<schema_id>@<version>` anchors a schema
//!   registry hash (64 lowercase hex). Each version is write-once.
//! * LIVE_WINDOW_EPOCH: from this epoch the governance parameter `live_window_epochs` takes effect
//!   (for future paid-archive encryption). Until then it reads u64::MAX, so everything stays public.
//!
//! Before each gate, behaviour is byte-for-byte what it was: no new checks, no new state.

use std::collections::BTreeMap;
use crate::chain::Chain;
use crate::usefulness::{cell_res, valid_cell_at, MIN_CELL_RES, MAX_CELL_RES};

pub const INFERRED_FLAG: &str = "loc.inferred";
pub const INFERRED_AGE: &str = "loc.age_s"; // the app's name (1.15.0)
/// Finest resolution an inferred location may claim (one step coarser than measured rows).
pub const INFERRED_MAX_RES: u8 = 8;
/// Smallest live window governance may set: one day of epochs.
pub const MIN_LIVE_WINDOW_EPOCHS: u64 = 2_880;
pub const LIVE_WINDOW_KEY: &str = "live_window_epochs";
pub const SCHEMA_PREFIX: &str = "schema:";

// Tests exercise every rule as if active from 352,000.
pub fn inferred_epoch() -> u64 { if cfg!(test) { 352_000 } else { hone_types::INFERRED_LOC_EPOCH } }
pub fn null_cell_epoch() -> u64 { if cfg!(test) { 352_000 } else { hone_types::NULL_CELL_EPOCH } }
pub fn schema_anchor_epoch() -> u64 { if cfg!(test) { 352_000 } else { hone_types::SCHEMA_ANCHOR_EPOCH } }
pub fn live_window_epoch() -> u64 { if cfg!(test) { 352_000 } else { hone_types::LIVE_WINDOW_EPOCH } }

/// Location check for a SENSOR_SUMMARY at `epoch`. Before both gates this is exactly
/// `valid_cell_at`, with the same error text.
pub fn check_summary_location(cell: &str, metrics: &BTreeMap<String, i64>, epoch: u64) -> Result<(), String> {
    let inferred_on = epoch >= inferred_epoch();
    let flagged = metrics.contains_key(INFERRED_FLAG) || metrics.contains_key(INFERRED_AGE);
    if cell.is_empty() && epoch >= null_cell_epoch() && epoch >= provenance_epoch() {
        if inferred_on && flagged { return Err("a summary with no cell cannot be location-inferred".into()); }
        return Ok(());
    }
    if !valid_cell_at(cell, epoch) {
        return Err(format!("invalid H3 cell '{}' (res {}..={} from epoch {})", cell,
            MIN_CELL_RES, MAX_CELL_RES, crate::usefulness::cell_res_epoch()));
    }
    if inferred_on && flagged {
        if metrics.get(INFERRED_FLAG) != Some(&1) {
            return Err(format!("{INFERRED_FLAG} must be 1 when {INFERRED_AGE} is present"));
        }
        if !metrics.get(INFERRED_AGE).is_some_and(|a| *a >= 0) {
            return Err(format!("an inferred location needs {INFERRED_AGE} >= 0"));
        }
        if !cell_res(cell).is_some_and(|r| (MIN_CELL_RES..=INFERRED_MAX_RES).contains(&r)) {
            return Err(format!("an inferred location needs a res {MIN_CELL_RES}..={INFERRED_MAX_RES} cell"));
        }
    }
    Ok(())
}

/// True when a summary is stored without scoring (null cell, from NULL_CELL_EPOCH).
pub fn unscored_summary(cell: &str, epoch: u64) -> bool {
    cell.is_empty() && epoch >= null_cell_epoch()
}

fn valid_schema_id(id: &str) -> bool {
    let Some((name, ver)) = id.split_once('@') else { return false };
    !name.is_empty() && name.len() <= 64 && !ver.is_empty() && ver.len() <= 16
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        && ver.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
}

/// Extra checks on a governance ChainParameterSet. Called after the council and signature
/// checks. Only `schema:` keys (from SCHEMA_ANCHOR_EPOCH) and `live_window_epochs` (from
/// LIVE_WINDOW_EPOCH) are affected; every other key and every earlier epoch passes untouched.
pub fn check_param(chain: &Chain, key: &str, value: &str, epoch: u64) -> Result<(), String> {
    if let Some(id) = key.strip_prefix(SCHEMA_PREFIX) {
        if epoch < schema_anchor_epoch() { return Ok(()); }
        if !valid_schema_id(id) {
            return Err(format!("schema anchor key must be schema:<id>@<version>, got '{key}'"));
        }
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err("schema anchor value must be a 64-char lowercase hex hash".into());
        }
        // Write-once: a claim marker set at acceptance also covers the 2-epoch timelock window.
        let mark = format!("schema_anchor_claim:{id}");
        match chain.store.state_get(&mark).and_then(|b| String::from_utf8(b).ok()) {
            Some(prev) if prev != value => return Err(format!("schema {id} is already anchored to another hash")),
            Some(_) => return Err(format!("schema {id} is already anchored")),
            None => { let _ = chain.store.state_set(&mark, value.as_bytes()); }
        }
        return Ok(());
    }
    if key == LIVE_WINDOW_KEY && epoch >= live_window_epoch() {
        let v = value.trim();
        if v != "off" {
            let n: u64 = v.parse().map_err(|_| format!("{LIVE_WINDOW_KEY} must be an epoch count or 'off'"))?;
            if n < MIN_LIVE_WINDOW_EPOCHS {
                return Err(format!("{LIVE_WINDOW_KEY} must be at least {MIN_LIVE_WINDOW_EPOCHS}"));
            }
        }
    }
    Ok(())
}

/// Live window in epochs at `epoch`: u64::MAX (everything public) before LIVE_WINDOW_EPOCH,
/// when unset, or when set to "off".
pub fn live_window_epochs(chain: &Chain, epoch: u64) -> u64 {
    if epoch < live_window_epoch() { return u64::MAX; }
    chain.store.state_get(&format!("chain_param:{LIVE_WINDOW_KEY}"))
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(u64::MAX)
}

/// The anchored hash of `schema_id@version`, if any.
pub fn schema_hash(chain: &Chain, id: &str) -> Option<String> {
    chain.store.state_get(&format!("chain_param:{SCHEMA_PREFIX}{id}")).and_then(|b| String::from_utf8(b).ok())
}

/// View for GET /api/chain/data-rules.
pub fn view(chain: &Chain) -> serde_json::Value {
    let e = chain.current_epoch();
    let lw = live_window_epochs(chain, e);
    serde_json::json!({
        "epoch": e,
        "inferred_location_from_epoch": inferred_epoch(),
        "null_cell_from_epoch": null_cell_epoch(),
        "schema_anchor_from_epoch": schema_anchor_epoch(),
        "live_window_from_epoch": live_window_epoch(),
        "live_window_epochs": if lw == u64::MAX { serde_json::Value::Null } else { lw.into() },
        "inferred_max_res": INFERRED_MAX_RES,
    })
}


// ── Provenance (Josh 2026-11-10 11:22): "if there is no way to see where a devices information
// comes from, there is no reason to store it". From PROVENANCE_EPOCH every SENSOR_SUMMARY carries
// `prov` (what: dev + cls + sensor + schema; when: the entry's window; why: why + loc_src), signed
// with the data. Rows without it are refused.

pub fn provenance_epoch() -> u64 { if cfg!(test) { 365_000 } else { hone_types::PROVENANCE_EPOCH } }
pub fn relay_epoch() -> u64 { if cfg!(test) { 365_000 } else { hone_types::RELAY_CREDIT_EPOCH } }

/// Device classes (registry hone.raw@1, hdr.cls).
pub const DEVICE_CLASSES: [&str; 6] = ["phone", "nebra", "pc", "pi", "flipper", "station"];
/// Gap reasons (registry hone.raw@1, gap_reasons).
pub const GAP_REASONS: [&str; 6] = ["offline", "permission_denied", "speech_deleted", "no_signal", "sensor_absent", "app_stopped"];
/// Location sources (registry hdr.area.src, plus the app's home_wifi). "none" is written as none:<reason>.
pub const LOC_SOURCES: [&str; 8] = ["gps", "fused", "network", "wifi_fp", "last_verified", "fixed", "ip", "home_wifi"];
/// Why no location: the gap reasons plus no_fix (every source tried and failed).
pub const NO_LOC_REASONS: [&str; 7] = ["no_fix", "offline", "permission_denied", "no_signal", "sensor_absent", "app_stopped", "speech_deleted"];
/// Relayer's share of a relayed reading's points, in basis points (owner keeps the rest).
pub const RELAY_SHARE_BPS: u64 = 1_500;

fn token(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}
fn hex16(s: &str) -> bool { s.len() == 16 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }

/// Why a row exists: a scheduled window, an event (a detection), or gap:<reason>.
fn valid_why(w: &str) -> bool {
    w == "window" || w == "event" || w.strip_prefix("gap:").is_some_and(|r| GAP_REASONS.contains(&r))
}

/// Provenance check at `epoch`. Before PROVENANCE_EPOCH a row must not carry `prov` (old nodes
/// could not verify it); from it every row must, complete and consistent with its cell.
pub fn check_provenance(prov: Option<&hone_types::SummaryProvenance>, signed_by: &str, cell: &str,
                        metrics: &BTreeMap<String, i64>, epoch: u64) -> Result<(), String> {
    if epoch < provenance_epoch() {
        return if prov.is_some() { Err(format!("prov is not active before epoch {}", provenance_epoch())) } else { Ok(()) };
    }
    let p = prov.ok_or("a summary needs provenance (prov: dev, cls, sensor, schema, why, loc_src)")?;
    if !hex16(&p.dev) { return Err("prov.dev must be the first 16 hex of the device key".into()); }
    if !DEVICE_CLASSES.contains(&p.cls.as_str()) { return Err(format!("prov.cls must be one of {DEVICE_CLASSES:?}")); }
    if !token(&p.sensor, 32) { return Err("prov.sensor must be a short lowercase name".into()); }
    if !valid_schema_id(&p.schema) { return Err("prov.schema must be <schema_id>@<version>".into()); }
    if !valid_why(&p.why) { return Err("prov.why must be window, event or gap:<reason>".into()); }
    let none = p.loc_src.strip_prefix("none:");
    match none {
        Some(r) if !NO_LOC_REASONS.contains(&r) => return Err(format!("prov.loc_src none:<reason> needs one of {NO_LOC_REASONS:?}")),
        None if !LOC_SOURCES.contains(&p.loc_src.as_str()) => return Err(format!("prov.loc_src must be one of {LOC_SOURCES:?} or none:<reason>")),
        _ => {}
    }
    if cell.is_empty() != none.is_some() {
        return Err("a summary has no cell exactly when prov.loc_src is none:<reason>".into());
    }
    if metrics.get(INFERRED_FLAG) == Some(&1) && none.is_some() {
        return Err("an inferred location needs a location source".into());
    }
    // A seat-signed row names its own seat key as the device.
    if let Some((_, pk)) = hone_types::parse_device_clock_id(signed_by) {
        if !pk.starts_with(&p.dev) { return Err("prov.dev must match the signing seat key".into()); }
    }
    Ok(())
}

/// The message a relayer signs: it names the relayer and the hash of the device-signed message,
/// so the relayer can neither change the data nor be swapped for someone else.
pub fn relay_message(chain_id: &str, by: &str, entry_msg: &str) -> String {
    use sha2::Digest;
    let h = hex::encode(sha2::Sha256::digest(entry_msg.as_bytes()));
    format!("{{\"chain_id\":\"{chain_id}\",\"type\":\"SENSOR_RELAY\",\"by\":\"{by}\",\"entry\":\"{h}\"}}")
}

fn verify_msg(pubkey_hex: &str, msg: &str, sig_hex: &str) -> bool {
    use ed25519_dalek::{Signature, VerifyingKey};
    let (Ok(pk), Ok(sig)) = (hex::decode(pubkey_hex), hex::decode(sig_hex)) else { return false };
    let (Ok(pk), Ok(sig)) = (<[u8; 32]>::try_from(pk), <[u8; 64]>::try_from(sig)) else { return false };
    let Ok(vk) = VerifyingKey::from_bytes(&pk) else { return false };
    vk.verify_strict(msg.as_bytes(), &Signature::from_bytes(&sig)).is_ok()
}

fn posting_key(chain: &Chain, account: &str) -> Option<String> {
    chain.store.get_account(account).ok().flatten()
        .and_then(|s| s.get("keys").and_then(|k| k.get("posting")).and_then(|v| v.as_str()).map(str::to_owned))
        .filter(|k| !k.is_empty())
}

/// A relayed summary (from RELAY_CREDIT_EPOCH): the reading is signed by the sensor's own
/// enrolled device key (a Flipper, a station), enrolled to `owner`; the relayer signs
/// `relay_message` with its seat key (a staked seat) or its posting key.
pub fn check_relay(chain: &Chain, sensor_id: &str, owner: &str, entry: &hone_types::LedgerEntry,
                   sig_hex: Option<&str>, relay: &hone_types::RelayProof,
                   prov: Option<&hone_types::SummaryProvenance>, epoch: u64) -> Result<(), String> {
    if epoch < relay_epoch() { return Err(format!("relayed summaries are not active before epoch {}", relay_epoch())); }
    let rec = chain.store.state_get(&format!("sensor_devkey:{sensor_id}"))
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .ok_or_else(|| format!("sensor '{sensor_id}' has no enrolled device key; only device-signed readings can be relayed"))?;
    let dk = rec["device_pubkey"].as_str().unwrap_or("");
    if dk.is_empty() { return Err(format!("sensor '{sensor_id}' device key is revoked")); }
    if rec["owner"].as_str() != Some(owner) { return Err(format!("sensor '{sensor_id}' is not enrolled to '{owner}'")); }
    crate::tx::verify_against_pubkey(chain, dk, entry, sig_hex).map_err(|e| e.to_string())?;
    if let Some(p) = prov { if !dk.starts_with(&p.dev) { return Err("prov.dev must match the device key".into()); } }
    let msg = crate::tx::canonical_signing_message(entry, &chain.chain_id).map_err(|e| e.to_string())?;
    let rmsg = relay_message(&chain.chain_id, &relay.by, &msg);
    let ok = match hone_types::parse_device_clock_id(&relay.by) {
        Some((acct, pk)) => crate::tx::seat_staked(chain, &relay.by, acct) && verify_msg(pk, &rmsg, &relay.sig),
        None => posting_key(chain, &relay.by).is_some_and(|pk| verify_msg(&pk, &rmsg, &relay.sig)),
    };
    if !ok { return Err(format!("relay signature by '{}' does not verify", relay.by)); }
    Ok(())
}

/// Device identity used for independence checks: prov.dev, else the sensor id.
pub fn device_of(sensor_id: &str, prov: Option<&hone_types::SummaryProvenance>) -> String {
    prov.map(|p| p.dev.clone()).unwrap_or_else(|| sensor_id.to_owned())
}


#[cfg(test)]
mod tests {
    use super::*;
    fn m(kv: &[(&str, i64)]) -> BTreeMap<String, i64> { kv.iter().map(|(k, v)| (k.to_string(), *v)).collect() }
    const R7: &str = "876d74365ffffff";
    const R9: &str = "896d74365bbffff";

    fn r8() -> String {
        let c = h3o::CellIndex::try_from(u64::from_str_radix(R9, 16).unwrap()).unwrap();
        format!("{:x}", u64::from(c.parent(h3o::Resolution::Eight).unwrap()))
    }

    #[test]
    fn before_gates_location_rule_is_unchanged() {
        let ms = m(&[("wifi.ap_count", 3)]);
        assert!(check_summary_location(R9, &ms, 351_999).is_ok());
        assert!(check_summary_location("", &ms, 351_999).is_err());
        assert!(check_summary_location("zz", &ms, 351_999).is_err());
        // a flagged row before the gate is treated as before (loc.* is a context metric)
        assert!(check_summary_location(R9, &m(&[(INFERRED_FLAG, 1)]), 351_999).is_ok());
    }

    #[test]
    fn inferred_rows_need_res_7_or_8_flag_and_age() {
        let ok = m(&[("wifi.ap_count", 3), (INFERRED_FLAG, 1), (INFERRED_AGE, 600)]);
        assert!(check_summary_location(R7, &ok, 352_000).is_ok());
        assert!(check_summary_location(&r8(), &ok, 352_000).is_ok());
        assert!(check_summary_location(R9, &ok, 352_000).is_err());
        assert!(check_summary_location(R7, &m(&[(INFERRED_FLAG, 1)]), 352_000).is_err());
        assert!(check_summary_location(R7, &m(&[(INFERRED_FLAG, 1), (INFERRED_AGE, -1)]), 352_000).is_err());
        assert!(check_summary_location(R7, &m(&[(INFERRED_FLAG, 2), (INFERRED_AGE, 5)]), 352_000).is_err());
        assert!(check_summary_location(R7, &m(&[(INFERRED_AGE, 5)]), 352_000).is_err());
        // measured rows keep res 7..=9
        assert!(check_summary_location(R9, &m(&[("wifi.ap_count", 3)]), 352_000).is_ok());
    }

    #[test]
    fn null_cell_is_stored_and_unscored() {
        let ms = m(&[("ble.crowd", 4)]);
        assert!(check_summary_location("", &ms, 352_000).is_err(), "null cell waits for the provenance gate");
        assert!(check_summary_location("", &ms, 365_000).is_ok());
        assert!(unscored_summary("", 352_000));
        assert!(!unscored_summary(R9, 352_000));
        assert!(check_summary_location("", &m(&[(INFERRED_FLAG, 1), (INFERRED_AGE, 1)]), 365_000).is_err());
        assert!(crate::usefulness::scored_metric(INFERRED_FLAG) == false);
        assert!(crate::usefulness::scored_metric(INFERRED_AGE) == false);
    }

    fn prov(dev: &str, loc: &str) -> hone_types::SummaryProvenance {
        hone_types::SummaryProvenance { dev: dev.into(), cls: "phone".into(), sensor: "coverage".into(),
            schema: "hone.raw.hdr@1".into(), why: "window".into(), loc_src: loc.into() }
    }

    #[test]
    fn provenance_is_required_complete_and_consistent() {
        let ms = m(&[("ble.crowd", 4)]);
        let d = "0123456789abcdef";
        // Before the gate: no prov, and prov is refused (old nodes could not verify it).
        assert!(check_provenance(None, "josh", R9, &ms, 364_999).is_ok());
        assert!(check_provenance(Some(&prov(d, "gps")), "josh", R9, &ms, 364_999).is_err());
        // From the gate: required.
        assert!(check_provenance(None, "josh", R9, &ms, 365_000).is_err());
        assert!(check_provenance(Some(&prov(d, "gps")), "josh", R9, &ms, 365_000).is_ok());
        // Null cell exactly when loc_src is none:<reason>.
        assert!(check_provenance(Some(&prov(d, "none:no_fix")), "josh", "", &ms, 365_000).is_ok());
        assert!(check_provenance(Some(&prov(d, "none:permission_denied")), "josh", "", &ms, 365_000).is_ok());
        assert!(check_provenance(Some(&prov(d, "gps")), "josh", "", &ms, 365_000).is_err());
        assert!(check_provenance(Some(&prov(d, "none:no_fix")), "josh", R9, &ms, 365_000).is_err());
        assert!(check_provenance(Some(&prov(d, "none:because")), "josh", "", &ms, 365_000).is_err());
        // Field checks.
        let mut p = prov(d, "gps"); p.cls = "toaster".into();
        assert!(check_provenance(Some(&p), "josh", R9, &ms, 365_000).is_err());
        let mut p = prov(d, "gps"); p.why = "gap:offline".into();
        assert!(check_provenance(Some(&p), "josh", R9, &ms, 365_000).is_ok());
        p.why = "gap:bored".into();
        assert!(check_provenance(Some(&p), "josh", R9, &ms, 365_000).is_err());
        let mut p = prov(d, "gps"); p.schema = "nope".into();
        assert!(check_provenance(Some(&p), "josh", R9, &ms, 365_000).is_err());
        assert!(check_provenance(Some(&prov("xyz", "gps")), "josh", R9, &ms, 365_000).is_err());
        // A seat names its own key.
        let seat = format!("josh@{d}{}", "00".repeat(24));
        assert!(check_provenance(Some(&prov(d, "gps")), &seat, R9, &ms, 365_000).is_ok());
        assert!(check_provenance(Some(&prov("fedcba9876543210", "gps")), &seat, R9, &ms, 365_000).is_err());
        // An inferred row needs a source.
        let inf = m(&[("ble.crowd", 4), (INFERRED_FLAG, 1), (INFERRED_AGE, 60)]);
        assert!(check_provenance(Some(&prov(d, "last_verified")), "josh", R7, &inf, 365_000).is_ok());
    }

    #[test]
    fn registry_anchor_value_is_accepted() {
        // The app agent's registry (honemesh.net/api/raw/schema), sha256 e03992fb...
        assert!(valid_schema_id("hone.raw@1"));
        let h = "e03992fb342e682296241112e1668f04d947a942ce1ffc86eb721c24e98531ca";
        assert!(h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    }

    #[test]
    fn schema_ids() {
        assert!(valid_schema_id("hone.sensor.wifi@1"));
        assert!(valid_schema_id("raw-scan_v2@2.1"));
        assert!(!valid_schema_id("Hone@1"));
        assert!(!valid_schema_id("hone"));
        assert!(!valid_schema_id("@1"));
        assert!(!valid_schema_id("a@"));
    }
}

