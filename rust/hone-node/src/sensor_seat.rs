//! Referral-backed sensor seats (gated by SENSOR_SEATS_EPOCH; Josh 2026-11-04).
//!
//! A referrer (e.g. josh) registers a device seat `owner@<seat pubkey>` for a referred
//! user's phone or camera station and backs it by delegating HONE to it with
//! `NodeRoleStake { node: seat, role: "sensor", staker: owner }`. A backed seat may submit
//! SensorSummary entries signed by its seat key; rewards go to the seat's owner.
//!
//! When the user gets their own wallet, `SENSOR_SEAT_TRANSFER` (owner active key + seat key
//! countersignature) moves the seat to `joe@<pubkey>`. The referrer's delegation STAYS on the
//! seat as its sponsor. From then on rewards go to joe, and `autostake_bps` (default 50%) of
//! every sensor reward joe receives is staked into joe's own position on the seat. When joe's
//! own stake reaches max(sponsor_amount, SEAT_MIN_STAKE) the sponsor's delegation is returned
//! in full (payback). The sponsor may also leave early with 24 h notice, and its delegation
//! returns automatically if the seat submits nothing for 7 days. A slash takes joe's stake
//! first and never more than 10% of the sponsor's principal. Everything is in-state and
//! processed in sorted key order at epoch_tick.
//!
//! State: sensor_seat:{seat} (record), sensor_seat_pk:{pubkey} -> seat,
//! sensor_seat_by:{owner}:{pubkey}, sensor_seat_reg:{owner}:{window} (rate limit),
//! sensor_seat_last:{seat} (last summary epoch), seat_exit:{seat} (notice),
//! seat_payback:{seat} (audit), seat_sponsor_slashed:{seat}; stakes are the ordinary
//! role_stake:sensor:{seat}:{staker} rows.

use anyhow::{bail, Result};
use hone_types::{LedgerEntry, NATIVE_TOKEN, RECYCLE_FUND_ACCOUNT};
use serde_json::{json, Value};

use crate::chain::Chain;

pub const SEAT_MIN_STAKE: u64 = 100_000_000_000; // 10 HONE
pub const AUTOSTAKE_DEFAULT_BPS: u64 = 5_000;
pub const NOTICE_EPOCHS: u64 = 2_880; // 24 h
pub const IDLE_EPOCHS: u64 = 20_160; // 7 days
pub const SPONSOR_SLASH_CAP_BPS: u64 = 1_000; // 10% of sponsor principal
pub const MAX_SEATS_PER_OWNER: usize = 25;
pub const MAX_REG_PER_WINDOW: u64 = 2;
pub const REG_WINDOW_EPOCHS: u64 = 120;

pub fn activation_epoch() -> u64 {
    #[cfg(test)]
    if let Some(v) = TEST_EPOCH.with(|c| c.get()) { return v; }
    hone_types::SENSOR_SEATS_EPOCH
}
#[cfg(test)]
thread_local! {
    pub static TEST_EPOCH: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

fn is_hex32(s: &str) -> bool { s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }
fn get_json(chain: &Chain, k: &str) -> Option<Value> { chain.store.state_get(k).and_then(|b| serde_json::from_slice(&b).ok()) }
fn set_json(chain: &Chain, k: &str, v: &Value) -> Result<()> { chain.store.state_set(k, &serde_json::to_vec(v)?)?; Ok(()) }
fn read_u64(chain: &Chain, k: &str) -> u64 {
    chain.store.state_get(k).and_then(|b| String::from_utf8(b).ok()).and_then(|s| s.parse().ok()).unwrap_or(0)
}

pub fn rec_key(seat: &str) -> String { format!("sensor_seat:{seat}") }
pub fn get(chain: &Chain, seat: &str) -> Option<Value> { get_json(chain, &rec_key(seat)) }
fn stake_key(seat: &str, staker: &str) -> String { format!("role_stake:sensor:{seat}:{staker}") }
fn exit_key(seat: &str) -> String { format!("seat_exit:{seat}") }
fn last_key(seat: &str) -> String { format!("sensor_seat_last:{seat}") }

pub fn stake_of(chain: &Chain, seat: &str, staker: &str) -> u64 {
    get_json(chain, &stake_key(seat, staker)).and_then(|j| j["amount"].as_u64()).unwrap_or(0)
}
fn set_stake(chain: &Chain, seat: &str, staker: &str, amount: u64, epoch: u64) -> Result<()> {
    let k = stake_key(seat, staker);
    if amount == 0 { chain.store.state_delete(&k)?; return Ok(()); }
    set_json(chain, &k, &json!({ "node": seat, "role": "sensor", "staker": staker, "amount": amount, "staked_epoch": epoch }))
}
pub fn total_stake(chain: &Chain, seat: &str) -> u64 {
    chain.store.state_scan_prefix(&format!("role_stake:sensor:{seat}:")).into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<Value>(&v).ok()?["amount"].as_u64()).sum()
}
/// A registered seat whose total `sensor` stake meets the minimum may submit summaries.
pub fn backed(chain: &Chain, seat: &str) -> bool { get(chain, seat).is_some() && total_stake(chain, seat) >= SEAT_MIN_STAKE }
pub fn seat_for_pubkey(chain: &Chain, pk: &str) -> Option<String> {
    chain.store.state_get(&format!("sensor_seat_pk:{pk}")).and_then(|b| String::from_utf8(b).ok())
}
fn owner_seats(chain: &Chain, owner: &str) -> Vec<String> {
    let pre = format!("sensor_seat_by:{owner}:");
    chain.store.state_scan_prefix(&pre).into_iter().filter_map(|(k, _)| seat_for_pubkey(chain, &k[pre.len()..])).collect()
}
/// Record summary activity of a registered seat (idle tracking).
pub fn touch(chain: &Chain, seat: &str, epoch: u64) {
    if get(chain, seat).is_some() { let _ = chain.store.state_set(&last_key(seat), epoch.to_string().as_bytes()); }
}
/// Deterministic field/state checks (validation; apply re-runs them).
pub fn check(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    match entry {
        LedgerEntry::SensorSeatRegister { owner, seat_pubkey, kind, referrer, epoch, signed_by, .. } => {
            if signed_by != owner { bail!("signed_by must equal owner"); }
            if !is_hex32(seat_pubkey) { bail!("seat_pubkey must be 64 lowercase hex"); }
            if kind != "phone" && kind != "station" { bail!("kind must be phone or station"); }
            if let Some(r) = referrer {
                if r == owner { bail!("referrer must differ from owner"); }
                if chain.store.get_account(r)?.is_none() { bail!("unknown referrer '{}'", r); }
            }
            if seat_for_pubkey(chain, seat_pubkey).is_some() { bail!("seat key already registered"); }
            if owner_seats(chain, owner).len() >= MAX_SEATS_PER_OWNER { bail!("'{}' already has {} seats", owner, MAX_SEATS_PER_OWNER); }
            if read_u64(chain, &format!("sensor_seat_reg:{owner}:{}", epoch / REG_WINDOW_EPOCHS)) >= MAX_REG_PER_WINDOW {
                bail!("at most {} seat registrations per {} epochs", MAX_REG_PER_WINDOW, REG_WINDOW_EPOCHS);
            }
        }
        LedgerEntry::SensorSeatTransfer { seat, to_owner, signed_by, seat_sig, .. } => {
            let Some(rec) = get(chain, seat) else { bail!("unknown seat '{}'", seat) };
            if rec["owner"].as_str() != Some(signed_by.as_str()) { bail!("only the seat owner can transfer it"); }
            if to_owner == signed_by { bail!("to_owner must differ"); }
            if chain.store.get_account(to_owner)?.is_none() { bail!("unknown account '{}'", to_owner); }
            let Some((_, pk)) = hone_types::parse_device_clock_id(seat) else { bail!("bad seat id") };
            if get(chain, &format!("{to_owner}@{pk}")).is_some() { bail!("target seat exists"); }
            let msg = crate::tx::canonical_signing_message(entry, &chain.chain_id)?;
            if !verify_sig(pk, &msg, seat_sig) { bail!("seat key countersignature missing or invalid"); }
        }
        LedgerEntry::SensorSeatSet { seat, autostake_bps, signed_by, .. } => {
            let Some(rec) = get(chain, seat) else { bail!("unknown seat '{}'", seat) };
            if rec["owner"].as_str() != Some(signed_by.as_str()) { bail!("only the seat owner can set it"); }
            if *autostake_bps > 10_000 { bail!("autostake_bps must be <= 10000"); }
            let cur = rec["autostake_bps"].as_u64().unwrap_or(AUTOSTAKE_DEFAULT_BPS);
            if *autostake_bps < cur && !rec["sponsor"].is_null() { bail!("autostake can only be lowered once the sponsor is repaid"); }
        }
        _ => bail!("not a sensor seat entry"),
    }
    Ok(())
}

/// Create the seat record `owner@seat_pubkey` (register and voucher claim). Returns the seat id.
pub fn create_record(chain: &Chain, owner: &str, seat_pubkey: &str, kind: &str, referrer: Option<String>, epoch: u64) -> Result<String> {
    let seat = format!("{owner}@{seat_pubkey}");
    set_json(chain, &rec_key(&seat), &json!({ "seat": seat, "owner": owner, "seat_pubkey": seat_pubkey, "kind": kind,
        "referrer": referrer, "origin_owner": owner, "registered_epoch": epoch, "sponsor": null,
        "sponsor_amount": 0, "autostake_bps": AUTOSTAKE_DEFAULT_BPS, "transferred_epoch": null }))?;
    chain.store.state_set(&format!("sensor_seat_pk:{seat_pubkey}"), seat.as_bytes())?;
    chain.store.state_set(&format!("sensor_seat_by:{owner}:{seat_pubkey}"), b"1")?;
    chain.store.state_set(&last_key(&seat), epoch.to_string().as_bytes())?;
    Ok(seat)
}
pub fn put_stake(chain: &Chain, seat: &str, staker: &str, amount: u64, epoch: u64) -> Result<()> { set_stake(chain, seat, staker, amount, epoch) }

pub fn apply(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    check(chain, entry)?;
    match entry {
        LedgerEntry::SensorSeatRegister { owner, seat_pubkey, kind, referrer, epoch, .. } => {
            create_record(chain, owner, seat_pubkey, kind, referrer.clone(), *epoch)?;
            let rk = format!("sensor_seat_reg:{owner}:{}", epoch / REG_WINDOW_EPOCHS);
            chain.store.state_set(&rk, (read_u64(chain, &rk) + 1).to_string().as_bytes())?;
        }
        LedgerEntry::SensorSeatTransfer { seat, to_owner, epoch, .. } => {
            let mut rec = get(chain, seat).unwrap_or_default();
            let from = rec["owner"].as_str().unwrap_or_default().to_owned();
            let pk = rec["seat_pubkey"].as_str().unwrap_or_default().to_owned();
            let new = format!("{to_owner}@{pk}");
            let mut sponsor = rec["sponsor"].as_str().map(str::to_owned);
            let mut sponsor_amount = rec["sponsor_amount"].as_u64().unwrap_or(0);
            let pre = format!("role_stake:sensor:{seat}:");
            for (k, v) in chain.store.state_scan_prefix(&pre) {
                let staker = k[pre.len()..].to_owned();
                let amt = serde_json::from_slice::<Value>(&v).ok().and_then(|j| j["amount"].as_u64()).unwrap_or(0);
                chain.store.state_delete(&k)?;
                if sponsor.is_none() && staker == from && amt > 0 {
                    sponsor = Some(from.clone()); sponsor_amount = amt;
                    set_stake(chain, &new, &staker, amt, *epoch)?;
                } else if sponsor.as_deref() == Some(staker.as_str()) {
                    set_stake(chain, &new, &staker, amt, *epoch)?;
                } else if amt > 0 {
                    chain.store.credit(&staker, NATIVE_TOKEN, amt)?; // other stake returns to its staker
                }
            }
            rec["seat"] = json!(new); rec["owner"] = json!(to_owner);
            rec["sponsor"] = json!(sponsor); rec["sponsor_amount"] = json!(sponsor_amount);
            rec["transferred_epoch"] = json!(epoch);
            chain.store.state_delete(&rec_key(seat))?;
            set_json(chain, &rec_key(&new), &rec)?;
            chain.store.state_set(&format!("sensor_seat_pk:{pk}"), new.as_bytes())?;
            chain.store.state_delete(&format!("sensor_seat_by:{from}:{pk}"))?;
            chain.store.state_set(&format!("sensor_seat_by:{to_owner}:{pk}"), b"1")?;
            chain.store.state_delete(&last_key(seat))?;
            chain.store.state_set(&last_key(&new), epoch.to_string().as_bytes())?;
            chain.store.state_delete(&exit_key(seat))?;
        }
        LedgerEntry::SensorSeatSet { seat, autostake_bps, .. } => {
            let mut rec = get(chain, seat).unwrap_or_default();
            rec["autostake_bps"] = json!(autostake_bps);
            set_json(chain, &rec_key(seat), &rec)?;
        }
        _ => {}
    }
    Ok(())
}
/// Ed25519 strict verify of `msg` under a 32-byte hex pubkey.
pub fn verify_sig(pk_hex: &str, msg: &str, sig_hex: &str) -> bool {
    use ed25519_dalek::{Signature, VerifyingKey};
    let (Some(pk), Some(sig)) = (hex::decode(pk_hex).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()),
                                 hex::decode(sig_hex).ok().and_then(|b| <[u8; 64]>::try_from(b).ok())) else { return false };
    VerifyingKey::from_bytes(&pk).map(|vk| vk.verify_strict(msg.as_bytes(), &Signature::from_bytes(&sig)).is_ok()).unwrap_or(false)
}

/// NodeRoleUnstake on a transferred seat by its sponsor: start the 24 h notice instead of
/// unstaking now (the whole delegation returns at the due epoch). Returns true when handled.
pub fn notice_unstake(chain: &Chain, seat: &str, staker: &str, amount: u64, epoch: u64) -> Result<bool> {
    let Some(rec) = get(chain, seat) else { return Ok(false) };
    if rec["transferred_epoch"].is_null() || rec["sponsor"].as_str() != Some(staker) { return Ok(false); }
    if amount != stake_of(chain, seat, staker) { bail!("a sponsor withdraws its whole delegation (with {} epochs notice)", NOTICE_EPOCHS); }
    if chain.store.state_get(&exit_key(seat)).is_some() { bail!("notice already given"); }
    set_json(chain, &exit_key(seat), &json!({ "staker": staker, "due": epoch + NOTICE_EPOCHS }))?;
    Ok(true)
}

fn return_sponsor(chain: &Chain, seat: &str, reason: &str, epoch: u64) -> Result<()> {
    let Some(mut rec) = get(chain, seat) else { return Ok(()) };
    let Some(sponsor) = rec["sponsor"].as_str().map(str::to_owned) else { return Ok(()) };
    let amt = stake_of(chain, seat, &sponsor);
    set_stake(chain, seat, &sponsor, 0, epoch)?;
    if amt > 0 { chain.store.credit(&sponsor, NATIVE_TOKEN, amt)?; }
    rec["sponsor"] = Value::Null; rec["sponsor_amount"] = json!(0);
    set_json(chain, &rec_key(seat), &rec)?;
    chain.store.state_delete(&exit_key(seat))?;
    set_json(chain, &format!("seat_payback:{seat}"), &json!({ "sponsor": sponsor, "amount": amt, "epoch": epoch, "reason": reason }))?;
    tracing::info!("[sensor-seat] {} delegation of {} returned to {} ({})", seat, amt, sponsor, reason);
    Ok(())
}

/// Per quorum seal (epoch_tick): payback, notice and idle returns, in key order.
pub fn tick(chain: &Chain, epoch: u64) {
    if epoch < activation_epoch() { return; }
    for (k, v) in chain.store.state_scan_prefix("sensor_seat:") {
        let Ok(rec) = serde_json::from_slice::<Value>(&v) else { continue };
        if rec["sponsor"].is_null() || rec["transferred_epoch"].is_null() { continue; }
        let seat = &k["sensor_seat:".len()..];
        let owner = rec["owner"].as_str().unwrap_or_default();
        let need = rec["sponsor_amount"].as_u64().unwrap_or(0).max(SEAT_MIN_STAKE);
        let reason = if stake_of(chain, seat, owner) >= need { Some("payback") }
            else if get_json(chain, &exit_key(seat)).and_then(|x| x["due"].as_u64()).is_some_and(|d| d <= epoch) { Some("notice") }
            else if read_u64(chain, &last_key(seat)) + IDLE_EPOCHS <= epoch { Some("idle") }
            else { None };
        if let Some(r) = reason { let _ = return_sponsor(chain, seat, r, epoch); }
    }
}

/// SensorReward credited to `account`: stake `autostake_bps` of it into the account's own
/// position on its first (key order) transferred seat that still has a sponsor.
pub fn on_sensor_reward(chain: &Chain, account: &str, amount: u64, epoch: u64) {
    if epoch < activation_epoch() { return; }
    for seat in owner_seats(chain, account) {
        let Some(rec) = get(chain, &seat) else { continue };
        if rec["sponsor"].is_null() || rec["transferred_epoch"].is_null() { continue; }
        let s = (amount as u128 * rec["autostake_bps"].as_u64().unwrap_or(AUTOSTAKE_DEFAULT_BPS) as u128 / 10_000) as u64;
        if s > 0 && chain.store.debit(account, NATIVE_TOKEN, s).is_ok() {
            let _ = set_stake(chain, &seat, account, stake_of(chain, &seat, account) + s, epoch);
        }
        return;
    }
}

/// Slash a seat: the owner's own stake first, then the sponsor's, never more than
/// SPONSOR_SLASH_CAP_BPS of the sponsor's principal in total. Slashed HONE goes to the
/// recycle fund. Returns the amount slashed.
pub fn slash(chain: &Chain, seat: &str, amount: u64, epoch: u64) -> Result<u64> {
    let Some(rec) = get(chain, seat) else { return Ok(0) };
    let owner = rec["owner"].as_str().unwrap_or_default().to_owned();
    let own = stake_of(chain, seat, &owner);
    let a = own.min(amount);
    set_stake(chain, seat, &owner, own - a, epoch)?;
    let mut total = a;
    if let Some(sp) = rec["sponsor"].as_str() {
        let sk = format!("seat_sponsor_slashed:{seat}");
        let done = read_u64(chain, &sk);
        let cap = (rec["sponsor_amount"].as_u64().unwrap_or(0) as u128 * SPONSOR_SLASH_CAP_BPS as u128 / 10_000) as u64;
        let cur = stake_of(chain, seat, sp);
        let b = (amount - a).min(cap.saturating_sub(done)).min(cur);
        if b > 0 {
            set_stake(chain, seat, sp, cur - b, epoch)?;
            chain.store.state_set(&sk, (done + b).to_string().as_bytes())?;
            total += b;
        }
    }
    if total > 0 { chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, total)?; }
    Ok(total)
}

/// GET /api/sensor/seat/:seat (also accepts a bare seat pubkey).
pub fn view(chain: &Chain, seat: &str) -> Value {
    let seat = if seat.contains('@') { seat.to_owned() } else { seat_for_pubkey(chain, seat).unwrap_or_default() };
    let Some(rec) = get(chain, &seat) else {
        return json!({ "seat": seat, "registered": false, "seat_register_from_epoch": activation_epoch() });
    };
    let owner = rec["owner"].as_str().unwrap_or_default();
    let sponsor = rec["sponsor"].as_str();
    let need = rec["sponsor_amount"].as_u64().unwrap_or(0).max(SEAT_MIN_STAKE);
    json!({ "seat": seat, "registered": true, "record": rec, "seat_register_from_epoch": activation_epoch(),
        "total_stake": total_stake(chain, &seat), "backed": backed(chain, &seat), "min_stake": SEAT_MIN_STAKE,
        "owner_stake": stake_of(chain, &seat, owner),
        "sponsor_stake": sponsor.map(|s| stake_of(chain, &seat, s)).unwrap_or(0), "payback_target": need,
        "exit": get_json(chain, &exit_key(&seat)), "last_active_epoch": read_u64(chain, &last_key(&seat)),
        "payback": get_json(chain, &format!("seat_payback:{seat}")) })
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tx::{canonical_signing_message, validate_and_apply};
    use ed25519_dalek::{Signer, SigningKey};

    const E: u64 = 1_000;
    const H: u64 = 10_000_000_000; // 1 HONE

    pub(crate) fn key(seed: u8) -> SigningKey { SigningKey::from_bytes(&[seed; 32]) }
    pub(crate) fn pk(seed: u8) -> String { hex::encode(key(seed).verifying_key().to_bytes()) }
    pub(crate) fn sig(sk: &SigningKey, e: &LedgerEntry) -> String {
        hex::encode(sk.sign(canonical_signing_message(e, "hone-testnet").unwrap().as_bytes()).to_bytes())
    }
    /// Accounts: posting = seed, active = seed+1, owner = seed+2.
    pub(crate) fn mk(chain: &Chain, a: &str, seed: u8, bal: u64) {
        chain.apply_entry(&LedgerEntry::AccountCreate { account: a.into(), keys: Default::default(), chain_proofs: vec![],
            epoch: 0, funded_by: None, machine_fingerprint: None }).unwrap();
        for (r, s) in [("posting", seed), ("active", seed + 1), ("owner", seed + 2)] {
            chain.apply_entry(&LedgerEntry::AccountUpdateKey { account: a.into(), role: r.into(), new_public_key: pk(s),
                epoch: 0, signed_by: a.into() }).unwrap();
        }
        chain.apply_entry(&LedgerEntry::GenesisAlloc { account: a.into(), amount: bal, token: NATIVE_TOKEN.into() }).unwrap();
    }
    pub(crate) fn world() -> (Chain, tempfile::TempDir) {
        TEST_EPOCH.with(|c| c.set(Some(E)));
        crate::bird_cards::TEST_EPOCH.with(|c| c.set(Some(E)));
        let d = tempfile::Builder::new().prefix("hone_seat_test_").tempdir().unwrap();
        let chain = Chain::new(crate::store::Store::open(d.path()).unwrap(), "t".into(), "hone-testnet".into());
        mk(&chain, "josh", 0x10, 1_000 * H);
        mk(&chain, "joe", 0x20, 10 * H);
        mk(&chain, "mallory", 0x30, 100 * H);
        (chain, d)
    }
    pub(crate) fn nonce(c: &Chain, a: &str) -> u64 { c.store.get_account(a).unwrap().unwrap()["nonce"].as_u64().unwrap_or(0) + 1 }
    pub(crate) fn bal(c: &Chain, a: &str) -> u64 { c.get_balance(a, NATIVE_TOKEN) }
    pub(crate) const SEAT_SEED: u8 = 0x5a;
    pub(crate) fn seat_of(owner: &str) -> String { format!("{owner}@{}", pk(SEAT_SEED)) }

    pub(crate) fn register(c: &Chain, owner: &str, seed: u8, seat_seed: u8) -> Result<String> {
        let e = LedgerEntry::SensorSeatRegister { owner: owner.into(), seat_pubkey: pk(seat_seed), kind: "station".into(),
            referrer: None, nonce: nonce(c, owner), epoch: E, signed_by: owner.into() };
        validate_and_apply(c, &e, Some(&sig(&key(seed + 1), &e)))
    }
    pub(crate) fn stake(c: &Chain, seat: &str, staker: &str, seed: u8, amount: u64) -> Result<String> {
        let e = LedgerEntry::NodeRoleStake { node: seat.into(), role: "sensor".into(), staker: staker.into(), amount,
            epoch: E, nonce: nonce(c, staker), signed_by: staker.into() };
        validate_and_apply(c, &e, Some(&sig(&key(seed + 1), &e)))
    }
    fn unstake(c: &Chain, seat: &str, staker: &str, seed: u8, amount: u64) -> Result<String> {
        let e = LedgerEntry::NodeRoleUnstake { node: seat.into(), role: "sensor".into(), staker: staker.into(), amount,
            epoch: E, nonce: nonce(c, staker), signed_by: staker.into() };
        validate_and_apply(c, &e, Some(&sig(&key(seed + 1), &e)))
    }
    fn transfer(c: &Chain, seat: &str, from: &str, seed: u8, to: &str, with_seat_sig: bool) -> Result<String> {
        let mut e = LedgerEntry::SensorSeatTransfer { seat: seat.into(), to_owner: to.into(), nonce: nonce(c, from), epoch: E,
            signed_by: from.into(), seat_sig: String::new() };
        if with_seat_sig { let s = sig(&key(SEAT_SEED), &e); if let LedgerEntry::SensorSeatTransfer { seat_sig, .. } = &mut e { *seat_sig = s; } }
        validate_and_apply(c, &e, Some(&sig(&key(seed + 1), &e)))
    }
    /// josh registers + backs a seat with 20 HONE, then transfers it to joe.
    fn sponsored(c: &Chain) -> String {
        register(c, "josh", 0x10, SEAT_SEED).unwrap();
        stake(c, &seat_of("josh"), "josh", 0x10, 20 * H).unwrap();
        transfer(c, &seat_of("josh"), "josh", 0x10, "joe", true).unwrap();
        seat_of("joe")
    }
    fn summary(c: &Chain, seat: &str, w: u64) -> Result<String> {
        let e = LedgerEntry::SensorSummary { sensor_id: "cam".into(), owner: seat.into(), batch_hash: String::new(),
            cell: "896d75d960bffff".into(), window: w, metrics: [("birds.species".to_string(), w as i64)].into_iter().collect(),
            epoch: 352_700, signed_by: seat.into() };
        validate_and_apply(c, &e, Some(&sig(&key(SEAT_SEED), &e)))
    }
    fn total(c: &Chain, seat: &str) -> u64 {
        bal(c, "josh") + bal(c, "joe") + bal(c, "mallory") + bal(c, RECYCLE_FUND_ACCOUNT) + total_stake(c, seat)
    }

    #[test]
    fn gated_before_activation() {
        let (c, _d) = world();
        TEST_EPOCH.with(|x| x.set(Some(E + 1)));
        assert!(register(&c, "josh", 0x10, SEAT_SEED).is_err());
        assert!(stake(&c, &seat_of("josh"), "josh", 0x10, 20 * H).is_err(), "no sensor role on an unregistered seat");
        TEST_EPOCH.with(|x| x.set(Some(E)));
        register(&c, "josh", 0x10, SEAT_SEED).unwrap();
    }

    #[test]
    fn register_needs_active_key_unique_pubkey_rate_and_cap() {
        let (c, _d) = world();
        let e = LedgerEntry::SensorSeatRegister { owner: "josh".into(), seat_pubkey: pk(SEAT_SEED), kind: "station".into(),
            referrer: None, nonce: nonce(&c, "josh"), epoch: E, signed_by: "josh".into() };
        assert!(validate_and_apply(&c, &e, Some(&sig(&key(0x10), &e))).is_err(), "posting key refused");
        register(&c, "josh", 0x10, SEAT_SEED).unwrap();
        assert!(register(&c, "mallory", 0x30, SEAT_SEED).is_err(), "pubkey already registered");
        register(&c, "josh", 0x10, 0x5b).unwrap();
        assert!(register(&c, "josh", 0x10, 0x5c).is_err(), "3rd registration in one window refused");
        for i in 0..23u8 { c.store.state_set(&format!("sensor_seat_by:josh:{}", pk(0x70 + i)), b"1").unwrap();
            c.store.state_set(&format!("sensor_seat_pk:{}", pk(0x70 + i)), format!("josh@{}", pk(0x70 + i)).as_bytes()).unwrap(); }
        TEST_EPOCH.with(|x| x.set(Some(0)));
        let e = LedgerEntry::SensorSeatRegister { owner: "josh".into(), seat_pubkey: pk(0x5d), kind: "phone".into(),
            referrer: None, nonce: nonce(&c, "josh"), epoch: E + REG_WINDOW_EPOCHS, signed_by: "josh".into() };
        assert!(validate_and_apply(&c, &e, Some(&sig(&key(0x11), &e))).is_err(), "25-seat cap");
    }
    #[test]
    fn unbacked_refused_backed_accepted_owner_pays() {
        let (c, _d) = world();
        register(&c, "josh", 0x10, SEAT_SEED).unwrap();
        let seat = seat_of("josh");
        assert!(summary(&c, &seat, 1).is_err(), "unbacked seat");
        stake(&c, &seat, "josh", 0x10, 5 * H).unwrap();
        assert!(summary(&c, &seat, 2).is_err(), "below minimum");
        stake(&c, &seat, "josh", 0x10, 5 * H).unwrap();
        let before = bal(&c, "josh");
        summary(&c, &seat, 3).unwrap();
        assert!(bal(&c, "josh") < before, "owner pays the fee");
        assert!(stake(&c, &seat, "mallory", 0x30, 20 * H).is_err(), "only the seat owner stakes");
    }

    #[test]
    fn pre_transfer_rewards_go_to_referrer_whole() {
        let (c, _d) = world();
        register(&c, "josh", 0x10, SEAT_SEED).unwrap();
        stake(&c, &seat_of("josh"), "josh", 0x10, 20 * H).unwrap();
        let b = bal(&c, "josh");
        c.apply_entry(&LedgerEntry::SensorReward { node_id: "josh".into(), amount: 4 * H, epoch: E }).unwrap();
        assert_eq!(bal(&c, "josh"), b + 4 * H, "no autostake before transfer");
    }

    #[test]
    fn transfer_needs_both_signatures_and_moves_the_sponsor_stake() {
        let (c, _d) = world();
        register(&c, "josh", 0x10, SEAT_SEED).unwrap();
        stake(&c, &seat_of("josh"), "josh", 0x10, 20 * H).unwrap();
        assert!(transfer(&c, &seat_of("josh"), "josh", 0x10, "joe", false).is_err(), "seat countersignature required");
        assert!(transfer(&c, &seat_of("josh"), "mallory", 0x30, "joe", true).is_err(), "only the owner");
        transfer(&c, &seat_of("josh"), "josh", 0x10, "joe", true).unwrap();
        let seat = seat_of("joe");
        assert!(get(&c, &seat_of("josh")).is_none());
        let r = get(&c, &seat).unwrap();
        assert_eq!(r["owner"], "joe"); assert_eq!(r["sponsor"], "josh"); assert_eq!(r["sponsor_amount"], 20 * H);
        assert_eq!(stake_of(&c, &seat, "josh"), 20 * H);
        assert_eq!(seat_for_pubkey(&c, &pk(SEAT_SEED)).as_deref(), Some(seat.as_str()));
        summary(&c, &seat, 1).unwrap();
    }

    #[test]
    fn autostake_half_of_each_reward() {
        let (c, _d) = world();
        let seat = sponsored(&c);
        let b = bal(&c, "joe");
        c.apply_entry(&LedgerEntry::SensorReward { node_id: "joe".into(), amount: 3 * H, epoch: E }).unwrap();
        assert_eq!(stake_of(&c, &seat, "joe"), 3 * H / 2);
        assert_eq!(bal(&c, "joe") - b + stake_of(&c, &seat, "joe"), 3 * H, "balance + stake == reward");
        let s = LedgerEntry::SensorSeatSet { seat: seat.clone(), autostake_bps: 1_000, nonce: nonce(&c, "joe"), epoch: E, signed_by: "joe".into() };
        assert!(validate_and_apply(&c, &s, Some(&sig(&key(0x21), &s))).is_err(), "cannot lower before repaid");
        let s = LedgerEntry::SensorSeatSet { seat: seat.clone(), autostake_bps: 10_000, nonce: nonce(&c, "joe"), epoch: E, signed_by: "joe".into() };
        validate_and_apply(&c, &s, Some(&sig(&key(0x21), &s))).unwrap();
    }

    #[test]
    fn payback_returns_sponsor_principal_once() {
        let (c, _d) = world();
        let seat = sponsored(&c);
        let j = bal(&c, "josh");
        for _ in 0..13 { c.apply_entry(&LedgerEntry::SensorReward { node_id: "joe".into(), amount: 3 * H, epoch: E }).unwrap(); }
        tick(&c, E + 1);
        assert_eq!(bal(&c, "josh"), j, "19.5 < 20 HONE: not yet");
        c.apply_entry(&LedgerEntry::SensorReward { node_id: "joe".into(), amount: 3 * H, epoch: E }).unwrap();
        tick(&c, E + 2);
        assert_eq!(bal(&c, "josh"), j + 20 * H, "full principal back");
        assert!(get(&c, &seat).unwrap()["sponsor"].is_null());
        assert_eq!(stake_of(&c, &seat, "josh"), 0);
        assert!(backed(&c, &seat), "joe's own stake backs the seat");
        tick(&c, E + 3);
        assert_eq!(bal(&c, "josh"), j + 20 * H, "once");
        let b = bal(&c, "joe");
        c.apply_entry(&LedgerEntry::SensorReward { node_id: "joe".into(), amount: 2 * H, epoch: E }).unwrap();
        assert_eq!(bal(&c, "joe"), b + 2 * H, "no autostake after payback");
    }

    #[test]
    fn sponsor_leaves_with_notice() {
        let (c, _d) = world();
        let seat = sponsored(&c);
        assert!(unstake(&c, &seat, "josh", 0x10, 5 * H).is_err(), "whole delegation only");
        unstake(&c, &seat, "josh", 0x10, 20 * H).unwrap();
        let j = bal(&c, "josh");
        assert_eq!(stake_of(&c, &seat, "josh"), 20 * H, "not yet");
        tick(&c, E + NOTICE_EPOCHS - 1);
        assert_eq!(stake_of(&c, &seat, "josh"), 20 * H);
        stake(&c, &seat, "joe", 0x20, 5 * H).unwrap();
        tick(&c, E + NOTICE_EPOCHS);
        assert_eq!(bal(&c, "josh"), j + 20 * H);
        assert_eq!(stake_of(&c, &seat, "josh"), 0);
        assert_eq!(get_json(&c, &format!("seat_payback:{seat}")).unwrap()["reason"], "notice");
    }

    #[test]
    fn idle_seat_returns_the_delegation() {
        let (c, _d) = world();
        let seat = sponsored(&c);
        tick(&c, E + IDLE_EPOCHS - 1);
        assert_eq!(stake_of(&c, &seat, "josh"), 20 * H);
        tick(&c, E + IDLE_EPOCHS);
        assert_eq!(stake_of(&c, &seat, "josh"), 0);
        assert_eq!(get_json(&c, &format!("seat_payback:{seat}")).unwrap()["reason"], "idle");
    }

    #[test]
    fn slash_takes_owner_first_and_caps_the_sponsor() {
        let (c, _d) = world();
        let seat = sponsored(&c);
        stake(&c, &seat, "joe", 0x20, H).unwrap();
        let t = total(&c, &seat);
        assert_eq!(slash(&c, &seat, 100 * H, E).unwrap(), H + 2 * H, "joe 1 + sponsor cap 10% of 20");
        assert_eq!(stake_of(&c, &seat, "joe"), 0);
        assert_eq!(stake_of(&c, &seat, "josh"), 18 * H);
        assert_eq!(slash(&c, &seat, 100 * H, E).unwrap(), 0, "cap is lifetime");
        assert_eq!(total(&c, &seat), t, "slashed HONE goes to the recycle fund");
    }

    #[test]
    fn supply_is_conserved() {
        let (c, _d) = world();
        let seat = seat_of("joe");
        let t0 = total(&c, &seat) ;
        sponsored(&c);
        assert_eq!(total(&c, &seat), t0, "register, stake, transfer: fees go to recycle");
        for _ in 0..14 { c.apply_entry(&LedgerEntry::SensorReward { node_id: "joe".into(), amount: 3 * H, epoch: E }).unwrap(); }
        tick(&c, E + 1);
        assert_eq!(total(&c, &seat), t0 + 42 * H, "only the rewards add supply");
    }

    #[test]
    fn deterministic_on_two_chains() {
        let run = || {
            let (c, d) = world();
            let seat = sponsored(&c);
            for _ in 0..14 { c.apply_entry(&LedgerEntry::SensorReward { node_id: "joe".into(), amount: 3 * H, epoch: E }).unwrap(); }
            tick(&c, E + 1);
            let mut s: Vec<(String, Vec<u8>)> = Vec::new();
            for p in ["sensor_seat", "role_stake:sensor:", "seat_", "balance"] { s.extend(c.store.state_scan_prefix(p)); }
            let _ = seat; drop(d); s
        };
        assert_eq!(run(), run());
    }
}