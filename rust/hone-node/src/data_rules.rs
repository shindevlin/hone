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
pub const INFERRED_AGE: &str = "loc.inferred_age_s";
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
    if cell.is_empty() && epoch >= null_cell_epoch() {
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
        assert!(check_summary_location("", &ms, 352_000).is_ok());
        assert!(unscored_summary("", 352_000));
        assert!(!unscored_summary(R9, 352_000));
        assert!(check_summary_location("", &m(&[(INFERRED_FLAG, 1), (INFERRED_AGE, 1)]), 352_000).is_err());
        assert!(crate::usefulness::scored_metric(INFERRED_FLAG) == false);
        assert!(crate::usefulness::scored_metric(INFERRED_AGE) == false);
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

