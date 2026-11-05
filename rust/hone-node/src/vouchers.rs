//! Sponsor vouchers (gated by VOUCHERS_EPOCH; Josh 2026-11-04 19:08).
//!
//! A sponsor locks `amount_per_seat x max_seats` HONE behind a voucher key. The voucher's
//! PRIVATE key travels in an invite link (`?v=`); the chain stores only its pubkey. A new
//! user's device claims a seat with `SENSOR_SEAT_CLAIM`, signed by the voucher key over a
//! message that names the seat pubkey, plus a seat-key countersignature, so a claim copied
//! from the pool cannot be redirected to another seat. The claim registers seat
//! `{sponsor}@{seat_pubkey}` and moves `amount_per_seat` from the lock into the sponsor's
//! `sensor` stake on it; from then on it is an ordinary sponsored seat (idle return,
//! transfer, autostake, payback). Claims are fee-free; one per seat pubkey, at most
//! max_seats per voucher. At `claim_until_epoch` (or on SPONSOR_VOUCHER_CANCEL) the unclaimed
//! remainder returns to the sponsor.
//!
//! State: voucher:{pub} (record, holds the lock), voucher_open:{sponsor}:{pub}.

use anyhow::{bail, Result};
use hone_types::{LedgerEntry, NATIVE_TOKEN};
use serde_json::{json, Value};

use crate::chain::Chain;
use crate::sensor_seat::{self, SEAT_MIN_STAKE};

pub const DEFAULT_CLAIM_EPOCHS: u64 = 40_320; // 14 days
pub const MAX_CLAIM_EPOCHS: u64 = 86_400; // 30 days
pub const MAX_SEATS: u64 = 100;
pub const MAX_OPEN_PER_SPONSOR: usize = 20;

pub fn activation_epoch() -> u64 {
    #[cfg(test)]
    if let Some(v) = TEST_EPOCH.with(|c| c.get()) { return v; }
    hone_types::VOUCHERS_EPOCH
}
#[cfg(test)]
thread_local! {
    pub static TEST_EPOCH: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

fn is_hex32(s: &str) -> bool { s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }
fn get_json(chain: &Chain, k: &str) -> Option<Value> { chain.store.state_get(k).and_then(|b| serde_json::from_slice(&b).ok()) }
fn set_json(chain: &Chain, k: &str, v: &Value) -> Result<()> { chain.store.state_set(k, &serde_json::to_vec(v)?)?; Ok(()) }
pub fn key(pk: &str) -> String { format!("voucher:{pk}") }
pub fn get(chain: &Chain, pk: &str) -> Option<Value> { get_json(chain, &key(pk)) }
fn remaining(v: &Value) -> u64 { v["locked"].as_u64().unwrap_or(0) }

/// The claim window end a create entry asks for (0 = default 14 days).
fn until(claim_until_epoch: u64, epoch: u64) -> u64 {
    if claim_until_epoch == 0 { epoch + DEFAULT_CLAIM_EPOCHS } else { claim_until_epoch }
}

pub fn check(chain: &Chain, entry: &LedgerEntry, sig_hex: Option<&str>) -> Result<()> {
    match entry {
        LedgerEntry::SponsorVoucherCreate { sponsor, voucher_pubkey, amount_per_seat, max_seats, claim_until_epoch, epoch, signed_by, .. } => {
            if signed_by != sponsor { bail!("signed_by must equal sponsor"); }
            if !is_hex32(voucher_pubkey) { bail!("voucher_pubkey must be 64 lowercase hex"); }
            if get(chain, voucher_pubkey).is_some() { bail!("voucher key already used"); }
            if *amount_per_seat < SEAT_MIN_STAKE { bail!("amount_per_seat must be at least {} hunits (seat minimum)", SEAT_MIN_STAKE); }
            if *max_seats == 0 || *max_seats > MAX_SEATS { bail!("max_seats must be 1..={}", MAX_SEATS); }
            let u = until(*claim_until_epoch, *epoch);
            if u <= *epoch || u > epoch + MAX_CLAIM_EPOCHS { bail!("claim window must end within {} epochs", MAX_CLAIM_EPOCHS); }
            if chain.store.state_scan_prefix(&format!("voucher_open:{sponsor}:")).len() >= MAX_OPEN_PER_SPONSOR {
                bail!("at most {} open vouchers per sponsor", MAX_OPEN_PER_SPONSOR);
            }
            let total = amount_per_seat.checked_mul(*max_seats).ok_or_else(|| anyhow::anyhow!("amount overflow"))?;
            if chain.get_balance(sponsor, NATIVE_TOKEN) < total { bail!("sponsor balance below the {} hunits to lock", total); }
        }
        LedgerEntry::SponsorVoucherCancel { sponsor, voucher_pubkey, signed_by, .. } => {
            if signed_by != sponsor { bail!("signed_by must equal sponsor"); }
            let Some(v) = get(chain, voucher_pubkey) else { bail!("unknown voucher") };
            if v["sponsor"].as_str() != Some(sponsor.as_str()) { bail!("not this sponsor's voucher"); }
            if v["status"] != "open" { bail!("voucher is not open"); }
        }
        LedgerEntry::SensorSeatClaim { voucher_pubkey, seat_pubkey, kind, epoch, seat_sig, .. } => {
            let Some(v) = get(chain, voucher_pubkey) else { bail!("unknown voucher") };
            if v["status"] != "open" { bail!("voucher is not open"); }
            if v["claim_until"].as_u64().unwrap_or(0) <= *epoch { bail!("voucher expired"); }
            if v["claimed"].as_u64().unwrap_or(0) >= v["max_seats"].as_u64().unwrap_or(0) { bail!("voucher fully claimed"); }
            if !is_hex32(seat_pubkey) { bail!("seat_pubkey must be 64 lowercase hex"); }
            if kind != "phone" && kind != "station" { bail!("kind must be phone or station"); }
            if sensor_seat::seat_for_pubkey(chain, seat_pubkey).is_some() { bail!("seat key already registered"); }
            let msg = crate::tx::canonical_signing_message(entry, &chain.chain_id)?;
            if !sensor_seat::verify_sig(voucher_pubkey, &msg, sig_hex.unwrap_or("")) { bail!("claim must be signed by the voucher key"); }
            if !sensor_seat::verify_sig(seat_pubkey, &msg, seat_sig) { bail!("seat key countersignature missing or invalid"); }
        }
        _ => bail!("not a voucher entry"),
    }
    Ok(())
}

fn close(chain: &Chain, pk: &str, status: &str, epoch: u64) -> Result<()> {
    let Some(mut v) = get(chain, pk) else { return Ok(()) };
    let sponsor = v["sponsor"].as_str().unwrap_or_default().to_owned();
    let rest = remaining(&v);
    if rest > 0 { chain.store.credit(&sponsor, NATIVE_TOKEN, rest)?; }
    v["locked"] = json!(0); v["status"] = json!(status); v["closed_epoch"] = json!(epoch); v["refunded"] = json!(rest);
    set_json(chain, &key(pk), &v)?;
    chain.store.state_delete(&format!("voucher_open:{sponsor}:{pk}"))?;
    Ok(())
}

pub fn apply(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    match entry {
        LedgerEntry::SponsorVoucherCreate { sponsor, voucher_pubkey, amount_per_seat, max_seats, claim_until_epoch, epoch, .. } => {
            let total = amount_per_seat * max_seats;
            chain.store.debit(sponsor, NATIVE_TOKEN, total)?;
            set_json(chain, &key(voucher_pubkey), &json!({ "voucher_pubkey": voucher_pubkey, "sponsor": sponsor,
                "amount_per_seat": amount_per_seat, "max_seats": max_seats, "claimed": 0, "locked": total,
                "claim_until": until(*claim_until_epoch, *epoch), "created_epoch": epoch, "status": "open", "seats": [] }))?;
            chain.store.state_set(&format!("voucher_open:{sponsor}:{voucher_pubkey}"), b"1")?;
        }
        LedgerEntry::SponsorVoucherCancel { voucher_pubkey, epoch, .. } => close(chain, voucher_pubkey, "cancelled", *epoch)?,
        LedgerEntry::SensorSeatClaim { voucher_pubkey, seat_pubkey, kind, epoch, .. } => {
            let mut v = get(chain, voucher_pubkey).unwrap_or_default();
            let sponsor = v["sponsor"].as_str().unwrap_or_default().to_owned();
            let amt = v["amount_per_seat"].as_u64().unwrap_or(0);
            let seat = sensor_seat::create_record(chain, &sponsor, seat_pubkey, kind, None, *epoch)?;
            sensor_seat::put_stake(chain, &seat, &sponsor, sensor_seat::stake_of(chain, &seat, &sponsor) + amt, *epoch)?;
            v["locked"] = json!(remaining(&v) - amt);
            v["claimed"] = json!(v["claimed"].as_u64().unwrap_or(0) + 1);
            if let Some(s) = v["seats"].as_array_mut() { s.push(json!({ "seat": seat, "epoch": epoch })); }
            set_json(chain, &key(voucher_pubkey), &v)?;
            if v["claimed"].as_u64() >= v["max_seats"].as_u64() { close(chain, voucher_pubkey, "claimed", *epoch)?; }
        }
        _ => {}
    }
    Ok(())
}

/// Per quorum seal: expired open vouchers refund their remainder (sorted key order).
pub fn tick(chain: &Chain, epoch: u64) {
    if epoch < activation_epoch() { return; }
    for (k, _) in chain.store.state_scan_prefix("voucher_open:") {
        let pk = k.rsplit(':').next().unwrap_or_default().to_owned();
        if get(chain, &pk).and_then(|v| v["claim_until"].as_u64()).is_some_and(|u| u <= epoch) {
            let _ = close(chain, &pk, "expired", epoch);
        }
    }
}

/// GET /api/sponsor/voucher/:pub
pub fn view(chain: &Chain, pk: &str) -> Value {
    match get(chain, pk) {
        Some(mut v) => {
            let per = v["amount_per_seat"].as_u64().unwrap_or(0);
            v["remaining_seats"] = json!(if v["status"] == "open" { v["max_seats"].as_u64().unwrap_or(0) - v["claimed"].as_u64().unwrap_or(0) } else { 0 });
            v["remaining_hunits"] = json!(remaining(&v));
            let _ = per;
            v["active_from_epoch"] = json!(activation_epoch());
            v
        }
        None => json!({ "voucher_pubkey": pk, "exists": false, "active_from_epoch": activation_epoch() }),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor_seat::tests::{world, nonce, bal, sig, pk, key as sk};
    use crate::tx::{canonical_signing_message, validate_and_apply};
    use ed25519_dalek::Signer;

    const E: u64 = 1_000;
    const H: u64 = 10_000_000_000;
    const V: u8 = 0x61; // voucher key seed

    fn vw() -> (Chain, tempfile::TempDir) { TEST_EPOCH.with(|c| c.set(Some(E))); world() }
    fn create(c: &Chain, per: u64, seats: u64, until: u64) -> anyhow::Result<String> {
        let e = LedgerEntry::SponsorVoucherCreate { sponsor: "josh".into(), voucher_pubkey: pk(V), amount_per_seat: per, max_seats: seats,
            claim_until_epoch: until, nonce: nonce(c, "josh"), epoch: E, signed_by: "josh".into() };
        validate_and_apply(c, &e, Some(&sig(&sk(0x11), &e)))
    }
    fn claim_entry(seat_seed: u8) -> LedgerEntry {
        LedgerEntry::SensorSeatClaim { voucher_pubkey: pk(V), seat_pubkey: pk(seat_seed), kind: "phone".into(), epoch: E, seat_sig: String::new() }
    }
    fn signed_claim(seat_seed: u8, seat_signer: u8) -> (LedgerEntry, String) {
        let mut e = claim_entry(seat_seed);
        let m = canonical_signing_message(&e, "hone-testnet").unwrap();
        if let LedgerEntry::SensorSeatClaim { seat_sig, .. } = &mut e { *seat_sig = hex::encode(sk(seat_signer).sign(m.as_bytes()).to_bytes()); }
        (e, hex::encode(sk(V).sign(m.as_bytes()).to_bytes()))
    }
    fn claim(c: &Chain, seat_seed: u8) -> anyhow::Result<String> {
        let (e, s) = signed_claim(seat_seed, seat_seed);
        validate_and_apply(c, &e, Some(&s))
    }
    fn locked(c: &Chain) -> u64 { get(c, &pk(V)).map(|v| v["locked"].as_u64().unwrap_or(0)).unwrap_or(0) }
    fn supply(c: &Chain) -> u64 {
        let stakes: u64 = c.store.state_scan_prefix("role_stake:sensor:").into_iter()
            .filter_map(|(_, v)| serde_json::from_slice::<Value>(&v).ok()?["amount"].as_u64()).sum();
        bal(c, "josh") + bal(c, "joe") + bal(c, "mallory") + bal(c, hone_types::RECYCLE_FUND_ACCOUNT) + stakes + locked(c)
    }

    #[test]
    fn create_locks_and_claim_registers_a_backed_sponsored_seat() {
        let (c, _d) = vw();
        let b = bal(&c, "josh");
        assert!(create(&c, 5 * H, 2, 0).is_err(), "below the seat minimum");
        create(&c, 10 * H, 2, 0).unwrap();
        assert!(b - bal(&c, "josh") >= 20 * H, "20 HONE locked");
        assert_eq!(locked(&c), 20 * H);
        assert_eq!(get(&c, &pk(V)).unwrap()["claim_until"], E + DEFAULT_CLAIM_EPOCHS);
        let joe_before = bal(&c, "joe");
        claim(&c, 0x70).unwrap();
        let seat = format!("josh@{}", pk(0x70));
        assert!(sensor_seat::backed(&c, &seat));
        assert_eq!(sensor_seat::stake_of(&c, &seat, "josh"), 10 * H);
        assert_eq!(locked(&c), 10 * H);
        assert_eq!(bal(&c, "joe"), joe_before, "claims are fee-free and touch no one else");
        assert!(sensor_seat::get(&c, &seat).unwrap()["referrer"].is_null());
    }

    #[test]
    fn double_claim_cap_and_front_run_refused() {
        let (c, _d) = vw();
        create(&c, 10 * H, 2, 0).unwrap();
        claim(&c, 0x70).unwrap();
        assert!(claim(&c, 0x70).is_err(), "one claim per seat key");
        // Front-run: copy the voucher signature for seat 0x71 but swap in another seat key.
        let (good, vsig) = signed_claim(0x71, 0x71);
        let mut stolen = good.clone();
        if let LedgerEntry::SensorSeatClaim { seat_pubkey, .. } = &mut stolen { *seat_pubkey = pk(0x72); }
        assert!(validate_and_apply(&c, &stolen, Some(&vsig)).is_err(), "voucher sig covers the seat key");
        let (wrong_cs, vs2) = signed_claim(0x72, 0x73);
        assert!(validate_and_apply(&c, &wrong_cs, Some(&vs2)).is_err(), "seat countersignature must be the seat's");
        let bad = claim_entry(0x74);
        assert!(validate_and_apply(&c, &bad, Some(&hex::encode([0u8; 64]))).is_err(), "not the voucher key");
        assert!(crate::tx::precheck_submission(&c, &stolen, Some(&vsig)).is_err(), "refused at admission too");
        validate_and_apply(&c, &good, Some(&vsig)).unwrap();
        assert_eq!(get(&c, &pk(V)).unwrap()["status"], "claimed");
        assert!(claim(&c, 0x75).is_err(), "multi-use cap reached");
        assert!(c.store.state_scan_prefix("voucher_open:").is_empty());
    }

    #[test]
    fn expiry_refunds_remainder() {
        let (c, _d) = vw();
        assert!(create(&c, 10 * H, 3, E + MAX_CLAIM_EPOCHS + 1).is_err(), "window over 30 days");
        create(&c, 10 * H, 3, E + 100).unwrap();
        claim(&c, 0x70).unwrap();
        let b = bal(&c, "josh");
        tick(&c, E + 99);
        assert_eq!(get(&c, &pk(V)).unwrap()["status"], "open");
        tick(&c, E + 100);
        assert_eq!(bal(&c, "josh"), b + 20 * H, "unclaimed 2 seats refunded");
        assert_eq!(get(&c, &pk(V)).unwrap()["status"], "expired");
        let (e, s) = signed_claim(0x71, 0x71);
        let mut late = e; if let LedgerEntry::SensorSeatClaim { epoch, .. } = &mut late { *epoch = E + 101; }
        assert!(validate_and_apply(&c, &late, Some(&s)).is_err());
    }

    #[test]
    fn cancel_unlocks_and_only_by_sponsor() {
        let (c, _d) = vw();
        create(&c, 10 * H, 4, 0).unwrap();
        claim(&c, 0x70).unwrap();
        let x = LedgerEntry::SponsorVoucherCancel { sponsor: "mallory".into(), voucher_pubkey: pk(V), nonce: nonce(&c, "mallory"), epoch: E, signed_by: "mallory".into() };
        assert!(validate_and_apply(&c, &x, Some(&sig(&sk(0x31), &x))).is_err());
        let b = bal(&c, "josh");
        let e = LedgerEntry::SponsorVoucherCancel { sponsor: "josh".into(), voucher_pubkey: pk(V), nonce: nonce(&c, "josh"), epoch: E, signed_by: "josh".into() };
        validate_and_apply(&c, &e, Some(&sig(&sk(0x11), &e))).unwrap();
        assert!(bal(&c, "josh") > b, "refund credited");
        let v = get(&c, &pk(V)).unwrap();
        assert_eq!((v["status"].as_str(), v["refunded"].as_u64()), (Some("cancelled"), Some(30 * H)), "3 unclaimed seats back");
        assert!(claim(&c, 0x71).is_err(), "closed");
        assert_eq!(sensor_seat::stake_of(&c, &format!("josh@{}", pk(0x70)), "josh"), 10 * H, "claimed seat keeps its stake");
    }

    #[test]
    fn limits_and_gate() {
        let (c, _d) = vw();
        assert!(create(&c, 10 * H, 101, 0).is_err(), "max_seats <= 100");
        for i in 0..MAX_OPEN_PER_SPONSOR { c.store.state_set(&format!("voucher_open:josh:{}", pk(0x80 + i as u8)), b"1").unwrap(); }
        assert!(create(&c, 10 * H, 1, 0).is_err(), "20 open vouchers");
        TEST_EPOCH.with(|x| x.set(Some(E + 1)));
        let (c2, _d2) = world();
        TEST_EPOCH.with(|x| x.set(Some(E + 1)));
        assert!(create(&c2, 10 * H, 1, 0).is_err(), "gated");
    }

    #[test]
    fn conservation_and_determinism() {
        let run = || {
            let (c, d) = vw();
            let s0 = supply(&c);
            create(&c, 10 * H, 3, E + 50).unwrap();
            claim(&c, 0x70).unwrap();
            claim(&c, 0x71).unwrap();
            tick(&c, E + 50);
            let s1 = supply(&c);
            assert_eq!(s0, s1, "lock, claim, refund move value; nothing is created");
            let mut st: Vec<(String, Vec<u8>)> = Vec::new();
            for p in ["voucher", "sensor_seat", "role_stake:sensor:"] { st.extend(c.store.state_scan_prefix(p)); }
            drop(d); st
        };
        assert_eq!(run(), run());
    }
}