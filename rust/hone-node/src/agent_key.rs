//! Scoped "agent" key (gate AGENT_KEY_EPOCH; approved by Josh 2026-11-10 08:25).
//!
//! An account may hold one agent key (state `agent_key:{account}` = {pubkey, cap}), set or
//! rotated by SET_AGENT_KEY (active or owner key) and removed by REVOKE_AGENT_KEY (active,
//! owner or the agent key itself). The agent key is NOT an account role key: it never
//! satisfies `check_signature`, so every entry type not listed here stays out of its reach
//! (transfers, key changes, vouchers, anything else). It may sign, for its own account only:
//! - SENSOR_SEAT_REGISTER of a seat `<account>@<device>` with no referrer;
//! - NODE_ROLE_STAKE / NODE_ROLE_UNSTAKE with staker = the account on the account's own node
//!   or its own seats (no sponsoring other accounts). Unstaked funds always return to the
//!   staker, i.e. the account. Moving stake between own seats = unstake + stake.
//! The stake the agent key has placed (`agent_staked:{account}`, + agent stakes, - agent
//! unstakes) never exceeds the cap.
use anyhow::{bail, Result};
use hone_types::LedgerEntry;

use crate::chain::Chain;

pub fn gate() -> u64 {
    #[cfg(test)]
    if let Some(v) = TEST_EPOCH.with(|c| c.get()) { return v; }
    hone_types::AGENT_KEY_EPOCH
}
#[cfg(test)]
thread_local! { pub static TEST_EPOCH: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) }; }

/// (agent pubkey, cap in hunits) of an account.
pub fn key_of(chain: &Chain, account: &str) -> Option<(String, u64)> {
    let v: serde_json::Value = chain.store.state_get(&format!("agent_key:{account}"))
        .and_then(|b| serde_json::from_slice(&b).ok())?;
    Some((v["pubkey"].as_str()?.to_owned(), v["cap"].as_u64()?))
}

/// Stake currently placed under agent authority.
pub fn staked(chain: &Chain, account: &str) -> u64 {
    chain.store.state_get(&format!("agent_staked:{account}"))
        .and_then(|b| String::from_utf8(b).ok()).and_then(|s| s.parse().ok()).unwrap_or(0)
}
fn set_staked(chain: &Chain, account: &str, v: u64) {
    let _ = chain.store.state_set(&format!("agent_staked:{account}"), v.to_string().as_bytes());
}

/// `sig_hex` is a valid signature of `entry` by exactly the account's `role` key (no fallback).
pub fn role_signed(chain: &Chain, account: &str, role: &str, entry: &LedgerEntry, sig_hex: Option<&str>) -> bool {
    chain.store.get_account(account).ok().flatten()
        .and_then(|a| a["keys"][role].as_str().map(str::to_owned))
        .filter(|k| !k.is_empty())
        .map(|pk| crate::tx::verify_against_pubkey(chain, &pk, entry, sig_hex).is_ok())
        .unwrap_or(false)
}

/// `sig_hex` is a valid signature of `entry` by the account's agent key.
pub fn agent_signed(chain: &Chain, account: &str, entry: &LedgerEntry, sig_hex: Option<&str>) -> bool {
    entry.epoch() >= gate()
        && key_of(chain, account)
            .map(|(pk, _)| crate::tx::verify_against_pubkey(chain, &pk, entry, sig_hex).is_ok())
            .unwrap_or(false)
}

fn own_node(account: &str, node: &str) -> bool {
    node == account || hone_types::parse_device_clock_id(node).map(|(o, _)| o == account).unwrap_or(false)
}

/// Ok(true): the entry is signed by its account's agent key AND is within the agent's scope
/// and cap. Ok(false): not agent-signed (the caller applies its normal key check).
/// Err: agent-signed but outside scope or over the cap.
pub fn authorizes(chain: &Chain, entry: &LedgerEntry, sig_hex: Option<&str>) -> Result<bool> {
    let signer = match entry {
        LedgerEntry::SensorSeatRegister { signed_by, .. }
        | LedgerEntry::NodeRoleStake { signed_by, .. }
        | LedgerEntry::NodeRoleUnstake { signed_by, .. } => signed_by,
        _ => return Ok(false),
    };
    if !agent_signed(chain, signer, entry, sig_hex) { return Ok(false); }
    let (_, cap) = key_of(chain, signer).unwrap_or_default();
    match entry {
        LedgerEntry::SensorSeatRegister { owner, referrer, signed_by, .. } => {
            if owner != signed_by { bail!("agent key: only the account's own seats"); }
            if referrer.is_some() { bail!("agent key may not name a referrer"); }
        }
        LedgerEntry::NodeRoleStake { node, staker, signed_by, amount, .. } => {
            if staker != signed_by || !own_node(staker, node) {
                bail!("agent key: stake only from the account onto its own node or seats");
            }
            let used = staked(chain, staker);
            if used.saturating_add(*amount) > cap {
                bail!("agent key cap: {} + {} hunits would exceed the cap of {} hunits", used, amount, cap);
            }
        }
        LedgerEntry::NodeRoleUnstake { node, staker, signed_by, .. } => {
            if staker != signed_by || !own_node(staker, node) {
                bail!("agent key: unstake only the account's own stake on its own node or seats");
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn seat_key(account: &str, role: &str, node: &str) -> String { format!("agent_stake:{account}:{role}:{node}") }
fn read_u64(chain: &Chain, k: &str) -> u64 {
    chain.store.state_get(k).and_then(|b| String::from_utf8(b).ok()).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Stake the agent key has placed, per (node, role), in hunits.
pub fn stakes(chain: &Chain, account: &str) -> Vec<(String, String, u64)> {
    let pre = format!("agent_stake:{account}:");
    let mut out: Vec<(String, String, u64)> = chain.store.state_scan_prefix(&pre).into_iter().filter_map(|(k, v)| {
        let (role, node) = k[pre.len()..].split_once(':')?;
        let amt: u64 = String::from_utf8(v).ok()?.parse().ok()?;
        (amt > 0).then(|| (node.to_owned(), role.to_owned(), amt))
    }).collect();
    out.sort();
    out
}

/// After an agent-signed stake or unstake applied: move the per-(node, role) record and the
/// total by the same amount, so the total always equals the sum of the per-seat records. An
/// agent unstake frees cap only for stake the agent itself placed on that (node, role).
pub fn record(chain: &Chain, entry: &LedgerEntry) {
    match entry {
        LedgerEntry::NodeRoleStake { node, role, staker, amount, .. } => {
            let k = seat_key(staker, role, node);
            let _ = chain.store.state_set(&k, read_u64(chain, &k).saturating_add(*amount).to_string().as_bytes());
            set_staked(chain, staker, staked(chain, staker).saturating_add(*amount));
        }
        LedgerEntry::NodeRoleUnstake { node, role, staker, amount, .. } => {
            let k = seat_key(staker, role, node);
            let cur = read_u64(chain, &k);
            let d = cur.min(*amount);
            if cur - d == 0 { let _ = chain.store.state_delete(&k); }
            else { let _ = chain.store.state_set(&k, (cur - d).to_string().as_bytes()); }
            set_staked(chain, staker, staked(chain, staker).saturating_sub(d));
        }
        _ => {}
    }
}

/// GET /api/account/:account/agent-key. Amounts in hunits.
pub fn view(chain: &Chain, account: &str) -> serde_json::Value {
    serde_json::json!({
        "active_from_epoch": gate(),
        "epoch": chain.current_epoch(),
        "agent_key": key_of(chain, account).map(|(pubkey, cap)| serde_json::json!({ "pubkey": pubkey, "cap": cap })),
        "staked": staked(chain, account),
        "stakes": stakes(chain, account).into_iter()
            .map(|(node, role, amount)| serde_json::json!({ "node": node, "role": role, "amount": amount }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor_seat::tests::{bal, key, nonce, pk, sig, world};
    use crate::tx::validate_and_apply;
    use hone_types::NATIVE_TOKEN;

    const E: u64 = 1_000;
    const H: u64 = 10_000_000_000; // 1 HONE in hunits
    const AG: u8 = 0x77; // agent key seed
    // josh: posting 0x10, active 0x11, owner 0x12 (sensor_seat::tests::mk)
    const POSTING: u8 = 0x10;
    const ACTIVE: u8 = 0x11;
    const OWNER: u8 = 0x12;

    fn gate_at(e: u64) { TEST_EPOCH.with(|c| c.set(Some(e))); }
    fn run(c: &Chain, e: &LedgerEntry, seed: u8) -> anyhow::Result<()> { validate_and_apply(c, e, Some(&sig(&key(seed), e))).map(|_| ()) }
    fn set_key(c: &Chain, cap: u64, seed: u8) -> anyhow::Result<()> {
        let e = LedgerEntry::SetAgentKey { account: "josh".into(), agent_pubkey: pk(AG), cap,
            nonce: nonce(c, "josh"), epoch: E, signed_by: "josh".into() };
        run(c, &e, seed)
    }
    fn revoke(c: &Chain, seed: u8) -> anyhow::Result<()> {
        let e = LedgerEntry::RevokeAgentKey { account: "josh".into(), nonce: nonce(c, "josh"), epoch: E, signed_by: "josh".into() };
        run(c, &e, seed)
    }
    fn seat(s: u8) -> String { format!("josh@{}", pk(s)) }
    fn register(c: &Chain, s: u8, referrer: Option<&str>, seed: u8) -> anyhow::Result<()> {
        let e = LedgerEntry::SensorSeatRegister { owner: "josh".into(), seat_pubkey: pk(s), kind: "station".into(),
            referrer: referrer.map(str::to_owned), nonce: nonce(c, "josh"), epoch: E, signed_by: "josh".into() };
        run(c, &e, seed)
    }
    fn stake(c: &Chain, node: &str, staker: &str, amount: u64, seed: u8) -> anyhow::Result<()> {
        let e = LedgerEntry::NodeRoleStake { node: node.into(), role: "clock".into(), staker: staker.into(), amount,
            epoch: E, nonce: nonce(c, staker), signed_by: staker.into() };
        run(c, &e, seed)
    }
    fn unstake(c: &Chain, node: &str, staker: &str, amount: u64, seed: u8) -> anyhow::Result<()> {
        let e = LedgerEntry::NodeRoleUnstake { node: node.into(), role: "clock".into(), staker: staker.into(), amount,
            epoch: E, nonce: nonce(c, staker), signed_by: staker.into() };
        run(c, &e, seed)
    }
    fn role_stake(c: &Chain, node: &str) -> u64 {
        c.store.state_get(&format!("role_stake:clock:{node}:josh")).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|j| j["amount"].as_u64()).unwrap_or(0)
    }

    #[test]
    fn default_cap_is_100_hone_in_hunits() {
        assert_eq!(hone_types::AGENT_DEFAULT_CAP_HUNITS, 100 * H);
        assert_eq!(hone_types::AGENT_DEFAULT_CAP_HUNITS, 1_000_000_000_000);
        let e: LedgerEntry = serde_json::from_value(serde_json::json!({ "type": "SET_AGENT_KEY", "account": "josh",
            "agent_pubkey": pk(AG), "nonce": 1, "epoch": E, "signed_by": "josh" })).unwrap();
        let LedgerEntry::SetAgentKey { cap, .. } = e else { panic!() };
        assert_eq!(cap, 100 * H);
    }

    #[test]
    fn before_the_gate_nothing_changes() {
        let (c, _d) = world();
        gate_at(E + 1);
        assert!(set_key(&c, 10 * H, ACTIVE).is_err(), "SET_AGENT_KEY refused before the gate");
        // Even with a key written into state by hand, the agent signature is not accepted
        // before the gate: the account's active key is the only way, as today.
        c.store.state_set("agent_key:josh", serde_json::json!({ "pubkey": pk(AG), "cap": 100 * H }).to_string().as_bytes()).unwrap();
        assert!(stake(&c, &seat(0x61), "josh", H, AG).is_err());
        assert!(register(&c, 0x61, None, AG).is_err());
        stake(&c, &seat(0x61), "josh", H, ACTIVE).unwrap();
        assert_eq!(staked(&c, "josh"), 0, "active-signed stake never counts toward the agent");
        gate_at(E);
    }

    #[test]
    fn only_active_or_owner_set_or_rotate_the_key() {
        let (c, _d) = world();
        gate_at(E);
        assert!(set_key(&c, 10 * H, POSTING).is_err(), "posting may not set it");
        assert!(set_key(&c, 10 * H, AG).is_err(), "the agent key may not set itself");
        set_key(&c, 10 * H, ACTIVE).unwrap();
        assert_eq!(key_of(&c, "josh"), Some((pk(AG), 10 * H)));
        set_key(&c, 20 * H, OWNER).unwrap();
        assert_eq!(key_of(&c, "josh"), Some((pk(AG), 20 * H)), "owner rotates / changes the cap");
        assert!(set_key(&c, 30 * H, AG).is_err(), "no key changes by the agent except self-revoke");
    }

    #[test]
    fn agent_registers_and_stakes_own_seats_and_the_cap_holds_at_the_boundary() {
        let (c, _d) = world();
        gate_at(E);
        set_key(&c, 3 * H, ACTIVE).unwrap();
        register(&c, 0x61, None, AG).unwrap();
        assert!(crate::sensor_seat::get(&c, &seat(0x61)).is_some());
        stake(&c, &seat(0x61), "josh", 2 * H, AG).unwrap();
        stake(&c, "josh", "josh", H, AG).unwrap(); // exactly the cap: 3 HONE
        assert_eq!(staked(&c, "josh"), 3 * H);
        assert!(stake(&c, &seat(0x62), "josh", 1, AG).is_err(), "1 hunit over the cap is refused");
        assert_eq!(staked(&c, "josh"), 3 * H);
        // Moving stake between own seats: unstake from one, stake onto another.
        let b0 = bal(&c, "josh");
        unstake(&c, &seat(0x61), "josh", H, AG).unwrap();
        // The account gets the H back, less only the ordinary per-entry fee.
        let got = bal(&c, "josh") - b0;
        assert!(got <= H && got > H - H / 100, "unstake returns the funds to the account (got {got} hunits)");
        assert_eq!(role_stake(&c, &seat(0x61)), H);
        assert_eq!(staked(&c, "josh"), 2 * H);
        stake(&c, &seat(0x62), "josh", H, AG).unwrap();
        assert_eq!(staked(&c, "josh"), 3 * H);
        assert!(stake(&c, &seat(0x62), "josh", 1, AG).is_err());
        // The account's own active key is not limited by the agent cap.
        stake(&c, &seat(0x62), "josh", H, ACTIVE).unwrap();
    }

    #[test]
    fn agent_cannot_sign_anything_else() {
        let (c, _d) = world();
        gate_at(E);
        set_key(&c, 100 * H, ACTIVE).unwrap();
        // Transfers of any kind.
        let t = LedgerEntry::Transfer { from: "josh".into(), to: "joe".into(), amount: H, token: NATIVE_TOKEN.into(), memo: None,
            nonce: nonce(&c, "josh"), epoch: E, signed_by: "josh".into(), twofactor: None };
        assert!(run(&c, &t, AG).is_err(), "transfer");
        // Sponsoring another account: staking onto someone else's node or seat.
        assert!(stake(&c, "joe", "josh", H, AG).is_err(), "stake on another account's node");
        assert!(stake(&c, &format!("joe@{}", pk(0x63)), "josh", H, AG).is_err(), "stake on another account's seat");
        // A referral (sponsorship of another account) is not in scope.
        assert!(register(&c, 0x64, Some("joe"), AG).is_err(), "referrer");
        // Seats of other accounts, and seat transfer / settings.
        let other = LedgerEntry::SensorSeatRegister { owner: "joe".into(), seat_pubkey: pk(0x65), kind: "station".into(),
            referrer: None, nonce: nonce(&c, "joe"), epoch: E, signed_by: "joe".into() };
        assert!(run(&c, &other, AG).is_err(), "register for another account");
        register(&c, 0x66, None, AG).unwrap();
        let s = seat(0x66);
        let st = LedgerEntry::SensorSeatSet { seat: s.clone(), autostake_bps: 0, nonce: nonce(&c, "josh"), epoch: E, signed_by: "josh".into() };
        assert!(run(&c, &st, AG).is_err(), "seat settings");
        // Unstaking someone else's stake.
        stake(&c, &format!("joe@{}", pk(0x67)), "joe", H, 0x21).ok();
        assert!(unstake(&c, &format!("joe@{}", pk(0x67)), "joe", H, AG).is_err(), "another account's unstake");
        // Key changes: none, except self-revoke (next test). The agent key is not an account
        // role key, so posting-level entries are refused too.
        assert!(set_key(&c, 200 * H, AG).is_err());
        let summary = LedgerEntry::SensorSummary { prov: None, relay: None, sensor_id: "x".into(), owner: "josh".into(), batch_hash: String::new(),
            cell: "896d75d960bffff".into(), window: 1_993_148, metrics: [("wifi.ap_count".to_string(), 3i64)].into_iter().collect(),
            epoch: E, signed_by: "josh".into() };
        assert!(run(&c, &summary, AG).is_err(), "posting-level entry");
        assert_eq!(staked(&c, "josh"), 0, "nothing out of scope was counted");
    }

    fn sum(c: &Chain) -> u64 { stakes(c, "josh").iter().map(|x| x.2).sum() }

    #[test]
    fn per_seat_records_sum_to_the_total_and_the_view_lists_them() {
        let (c, _d) = world();
        gate_at(E);
        set_key(&c, 10 * H, ACTIVE).unwrap();
        stake(&c, &seat(0x61), "josh", 2 * H, AG).unwrap();
        stake(&c, &seat(0x62), "josh", 3 * H, AG).unwrap();
        stake(&c, "josh", "josh", H, AG).unwrap();
        stake(&c, &seat(0x61), "josh", H, ACTIVE).unwrap(); // not agent-placed: not recorded
        assert_eq!((staked(&c, "josh"), sum(&c)), (6 * H, 6 * H));
        // Unstaking more than the agent placed on a seat frees only the agent-placed part.
        unstake(&c, &seat(0x61), "josh", 3 * H, AG).unwrap();
        assert_eq!((staked(&c, "josh"), sum(&c)), (4 * H, 4 * H));
        unstake(&c, &seat(0x62), "josh", H, AG).unwrap();
        assert_eq!((staked(&c, "josh"), sum(&c)), (3 * H, 3 * H));
        let v = view(&c, "josh");
        assert_eq!(v["active_from_epoch"], E);
        assert_eq!(v["agent_key"]["pubkey"], pk(AG));
        assert_eq!(v["agent_key"]["cap"], 10 * H);
        assert_eq!(v["staked"], 3 * H);
        let mut listed: Vec<(String, String, u64)> = v["stakes"].as_array().unwrap().iter()
            .map(|s| (s["node"].as_str().unwrap().to_owned(), s["role"].as_str().unwrap().to_owned(), s["amount"].as_u64().unwrap())).collect();
        listed.sort();
        let mut want = vec![("josh".to_string(), "clock".to_string(), H), (seat(0x62), "clock".to_string(), 2 * H)];
        want.sort();
        assert_eq!(listed, want, "seat 0x61 is gone once its agent stake is fully unstaked");
        // No key: agent_key null, stakes still listed until unstaked.
        revoke(&c, ACTIVE).unwrap();
        let v = view(&c, "josh");
        assert!(v["agent_key"].is_null());
        assert_eq!(v["staked"], 3 * H);
        // Before the gate nothing is ever recorded.
        let (c2, _d2) = world();
        gate_at(E + 1);
        stake(&c2, &seat(0x61), "josh", H, ACTIVE).unwrap();
        assert!(stakes(&c2, "josh").is_empty() && staked(&c2, "josh") == 0);
        gate_at(E);
    }

    #[test]
    fn revoke_takes_effect() {
        let (c, _d) = world();
        gate_at(E);
        set_key(&c, 10 * H, ACTIVE).unwrap();
        stake(&c, &seat(0x61), "josh", H, AG).unwrap();
        revoke(&c, AG).unwrap(); // self-revoke
        assert!(key_of(&c, "josh").is_none());
        assert!(stake(&c, &seat(0x61), "josh", H, AG).is_err(), "revoked key signs nothing");
        assert!(unstake(&c, &seat(0x61), "josh", H, AG).is_err());
        assert!(revoke(&c, ACTIVE).is_err(), "nothing left to revoke");
        set_key(&c, 10 * H, ACTIVE).unwrap();
        assert!(revoke(&c, POSTING).is_err(), "posting may not revoke");
        revoke(&c, OWNER).unwrap();
        assert!(key_of(&c, "josh").is_none());
        // The account keeps full control with its own keys.
        unstake(&c, &seat(0x61), "josh", H, ACTIVE).unwrap();
    }
}
