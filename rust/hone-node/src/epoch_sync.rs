//! Restart-loss fix: a node that was down (or transplanted) misses the quorum seals of the
//! epochs it was not running for. Entries other nodes committed at those seals (sensor
//! commits from phones, other nodes' heartbeats, transfers) were gossiped once and never
//! reached it, so its `sensor_commit:` / `storage_beat:` rows and nonces differed and the
//! reward driver paid differently for many epochs afterwards (beastly vs grouchly,
//! 2026-11-03: josh sensor rewards ramped differently after a 2-epoch restart).
//!
//! Every node now records, per quorum seal, the entries it committed (`epoch_entries:{E}`,
//! local meta, not in state_root) and the last epoch it committed (`last_quorum_seal`).
//! When a quorum seal for E arrives and the last committed epoch is P < E-1, the node first
//! fetches the committed entry sets for P+1..E-1 from its sync peers and applies them in
//! epoch order, with the same multi-pass rule as a live seal, as of each entry's own epoch
//! (so the stale window does not reject them). Every entry is still signature-checked and
//! exactly-once guarded by validate_and_apply, so a peer can only omit, never forge; an
//! omission shows up at the next healer checkpoint.
use std::sync::Arc;

use hone_types::LedgerEntry;
use tracing::{info, warn};

use crate::chain::Chain;

pub const KEEP_EPOCHS: u64 = 2_880;
/// Never try to backfill more than this many epochs (a longer gap needs a transplant).
pub const MAX_GAP: u64 = 600;
const LAST_KEY: &str = "last_quorum_seal";

fn key(e: u64) -> String { format!("epoch_entries:{:016x}", e) }

pub fn last_committed(chain: &Chain) -> u64 {
    chain.store.state_get(LAST_KEY).and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

/// Record what this node committed at the quorum seal of `epoch`.
pub fn record(chain: &Chain, epoch: u64, applied: &[(LedgerEntry, Option<String>)]) {
    let rows: Vec<serde_json::Value> = applied.iter()
        .map(|(e, s)| serde_json::json!({ "entry": e, "sig": s })).collect();
    let _ = chain.store.state_set(&key(epoch), &serde_json::to_vec(&rows).unwrap_or_default());
    if epoch > last_committed(chain) {
        let _ = chain.store.state_set(LAST_KEY, epoch.to_string().as_bytes());
    }
    if epoch > KEEP_EPOCHS { let _ = chain.store.state_delete(&key(epoch - KEEP_EPOCHS)); }
}

/// The recorded entry sets for from..=to (only epochs this node committed).
pub fn serve(chain: &Chain, from: u64, to: u64) -> serde_json::Value {
    let to = to.min(from.saturating_add(MAX_GAP));
    let mut out = Vec::new();
    for e in from..=to {
        if let Some(b) = chain.store.state_get(&key(e)) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                out.push(serde_json::json!({ "epoch": e, "entries": v }));
            }
        }
    }
    // The finalized epoch meta for the range too: a node cut off from gossip misses the
    // EpochFinalize of every epoch it did not seal, and the reward driver will not pass an
    // unfinalized epoch (contiguity), so without these it stalls forever (2026-11-03).
    let mut fin = Vec::new();
    for e in from..=to {
        if let Ok(Some(m)) = chain.store.get_epoch_meta(e) {
            if m["finalized"].as_bool().unwrap_or(false) { fin.push(m); }
        }
    }
    serde_json::json!({ "epochs": out, "finalized": fin, "last_quorum_seal": last_committed(chain) })
}

/// Apply peer EpochFinalize meta for epochs this node has not finalized. Goes through the
/// normal EpochFinalize apply (same fork-choice rule as gossip), so it can only fill a gap
/// or be outvoted by a higher-quorum finalization, never silently rewrite one. Returns
/// the number of epochs newly finalized.
pub fn apply_finalized(chain: &Chain, v: &serde_json::Value) -> usize {
    let mut n = 0;
    for m in v["finalized"].as_array().into_iter().flatten() {
        let Some(epoch) = m["epoch"].as_u64() else { continue };
        let local_final = chain.store.get_epoch_meta(epoch).ok().flatten()
            .map(|x| x["finalized"].as_bool().unwrap_or(false)).unwrap_or(false);
        if local_final { continue; }
        let entry = LedgerEntry::EpochFinalize {
            epoch,
            node_id: "epoch-sync".into(),
            rewards_hash: m["rewards_hash"].as_str().unwrap_or_default().to_owned(),
            quorum: m["quorum"].as_u64().unwrap_or(0),
            sealed_by: m["sealed_by"].as_array().into_iter().flatten()
                .filter_map(|s| s.as_str().map(str::to_owned)).collect(),
            state_root: m["state_root"].as_str().unwrap_or_default().to_owned(),
            timestamp: m["finalized_at"].as_u64().unwrap_or(0),
        };
        if entry_ok(&entry) && chain.apply_entry(&entry).is_ok() { n += 1; }
    }
    n
}

fn entry_ok(e: &LedgerEntry) -> bool {
    matches!(e, LedgerEntry::EpochFinalize { rewards_hash, quorum, .. } if !rewards_hash.is_empty() && *quorum > 0)
}

static FIN_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static FIN_LAST_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The reward driver is stuck behind an epoch peers finalized while this node was cut
/// off. Fetch the finalized meta for from..=to from peers, at most once a minute.
pub fn request_finalize_backfill(chain: &Arc<Chain>, peers: &[String], from: u64, to: u64) {
    use std::sync::atomic::Ordering::*;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64).unwrap_or(0);
    if from > to || peers.is_empty() || now.saturating_sub(FIN_LAST_MS.load(Relaxed)) < 60_000 { return; }
    if FIN_BUSY.swap(true, AcqRel) { return; }
    FIN_LAST_MS.store(now, Relaxed);
    let (chain, peers, to) = (chain.clone(), peers.to_vec(), to.min(from.saturating_add(MAX_GAP)));
    tokio::spawn(async move {
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build();
        if let Ok(http) = http {
            for p in &peers {
                let url = format!("{p}/api/sync/epoch_entries?from={from}&to={to}");
                let Ok(r) = http.get(&url).send().await else { continue };
                let Ok(v) = r.json::<serde_json::Value>().await else { continue };
                let n = apply_finalized(&chain, &v);
                info!("[epoch-sync] reward driver stalled at {from}: {n} finalized epoch(s) in {from}..{to} from {p}");
                if n > 0 { break; }
            }
        }
        FIN_BUSY.store(false, Release);
    });
}

/// Multi-pass apply of one epoch's entry set (hash order, retry until no progress) — the
/// same rule the live seal handler uses. `as_of` is the epoch the set was sealed at.
pub fn apply_set(chain: &Chain, epoch: u64, mut remaining: Vec<(LedgerEntry, Option<String>)>, as_of: Option<u64>)
    -> Vec<(LedgerEntry, Option<String>)>
{
    let _g = crate::tx::AsOfEpoch::set(as_of);
    let mut applied = Vec::new();
    loop {
        let mut next = Vec::new();
        let mut progress = false;
        for (e, sig) in remaining {
            match crate::tx::validate_and_apply(chain, &e, sig.as_deref()) {
                Ok(_) => { progress = true; chain.record_entry_result(&e.hash(), epoch, None); applied.push((e, sig)); }
                Err(err) => next.push((e, sig, err.to_string())),
            }
        }
        if next.is_empty() || !progress {
            for (e, _, err) in &next { chain.record_entry_result(&e.hash(), epoch, Some(err.clone())); }
            break;
        }
        remaining = next.into_iter().map(|(e, s, _)| (e, s)).collect();
    }
    applied
}

/// Seal-time processing of one committed epoch, identical on every node: the inference
/// daemon and expiry sweepers, unbonding / timelocked params / governance, then the base
/// fee step from the committed weight.
pub fn epoch_tick(chain: &Chain, epoch: u64, committed_weight: u64) {
    crate::usefulness::release_due(chain, epoch);
    crate::sensor_seat::tick(chain, epoch);
    crate::bird_cards::tick(chain, epoch);
    crate::inference_daemon::process_epoch(chain, epoch);
    crate::ensemble::sweep_expired(chain, epoch);
    crate::session_market::sweep_expired(chain, epoch);
    crate::agent_session::sweep_expired(chain, epoch);
    crate::vrf::sweep_epoch(chain, epoch);
    chain.drain_unbonding(epoch);
    chain.drain_pending_params(epoch);
    chain.drain_governance_proposals(epoch);
    let cur: u64 = chain.store.state_get("chain_param:base_fee")
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|j| j["fee"].as_u64())
        .unwrap_or(hone_types::BASE_FEE_INITIAL_HUNITS);
    let new = hone_types::compute_next_base_fee(cur, committed_weight, hone_types::EPOCH_TARGET_WEIGHT_UNITS);
    let _ = chain.store.state_set("chain_param:base_fee",
        &serde_json::to_vec(&serde_json::json!({ "fee": new })).unwrap_or_default());
    if new != cur {
        info!("clock: epoch {} base_fee {} → {} (committed weight {})", epoch, cur, new, committed_weight);
    }
}

/// Apply fetched epoch sets in order and record them. Returns entries applied.
pub fn apply_backfill(chain: &Chain, mut sets: Vec<(u64, Vec<(LedgerEntry, Option<String>)>)>) -> usize {
    sets.sort_by_key(|(e, _)| *e);
    let mut n = 0;
    for (e, mut entries) in sets {
        // The reserved-name migration runs at the first quorum seal at/after activation,
        // before that epoch's entries. A node backfilling across activation must do the
        // same at the same epoch, not later at the seal that triggered the backfill.
        let _ = crate::identity_nft::migrate_reserved(chain, e);
        entries.sort_by_key(|(x, _)| x.hash());
        let applied = apply_set(chain, e, entries, Some(e));
        n += applied.len();
        record(chain, e, &applied);
        epoch_tick(chain, e, applied.iter().map(|(x, _)| hone_types::entry_weight(x)).sum());
    }
    n
}

fn parse(v: &serde_json::Value) -> Vec<(u64, Vec<(LedgerEntry, Option<String>)>)> {
    v["epochs"].as_array().into_iter().flatten().filter_map(|x| {
        let e = x["epoch"].as_u64()?;
        let entries = x["entries"].as_array()?.iter().filter_map(|r| Some((
            serde_json::from_value::<LedgerEntry>(r["entry"].clone()).ok()?,
            r["sig"].as_str().map(str::to_owned),
        ))).collect();
        Some((e, entries))
    }).collect()
}

/// HTTP base URLs to backfill from: HONE_SYNC_PEERS (comma list), else the bootstrap
/// peers' IPs on :4242.
pub fn peers(bootstrap: &[String]) -> Vec<String> {
    if let Ok(v) = std::env::var("HONE_SYNC_PEERS") {
        return v.split(',').map(|s| s.trim().trim_end_matches('/').to_owned()).filter(|s| !s.is_empty()).collect();
    }
    bootstrap.iter().filter_map(|m| {
        let mut it = m.split('/');
        it.find(|p| *p == "ip4").and_then(|_| it.next()).map(|ip| format!("http://{ip}:4242"))
    }).collect()
}

/// Backfill from..=to from the first peer(s) that recorded them, then apply.
pub async fn backfill(chain: &Arc<Chain>, peers: &[String], from: u64, to: u64) {
    if from > to { return; }
    if to - from + 1 > MAX_GAP {
        warn!("[epoch-sync] gap {}..{} is {} epochs (> {}); not backfilling — needs a state transplant",
            from, to, to - from + 1, MAX_GAP);
        return;
    }
    let http = match reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build() { Ok(h) => h, Err(_) => return };
    let mut got: std::collections::BTreeMap<u64, Vec<(LedgerEntry, Option<String>)>> = Default::default();
    let mut fins: Vec<serde_json::Value> = Vec::new();
    for p in peers {
        let url = format!("{p}/api/sync/epoch_entries?from={from}&to={to}");
        match http.get(&url).send().await {
            Ok(r) => match r.json::<serde_json::Value>().await {
                Ok(v) => { for (e, s) in parse(&v) { got.entry(e).or_insert(s); } fins.push(v); },
                Err(e) => warn!("[epoch-sync] {url}: {e}"),
            },
            Err(e) => warn!("[epoch-sync] {url}: {e}"),
        }
        if (from..=to).all(|e| got.contains_key(&e)) { break; }
    }
    let missing: Vec<u64> = (from..=to).filter(|e| !got.contains_key(e)).collect();
    let n = apply_backfill(chain, got.into_iter().collect());
    let f: usize = fins.iter().map(|v| apply_finalized(chain, v)).sum();
    if f > 0 { info!("[epoch-sync] {} finalized epoch(s) taken from peers for {}..{}", f, from, to); }
    info!("[epoch-sync] backfilled epochs {}..{}: {} entries applied; no peer record for {} epoch(s){}",
        from, to, n, missing.len(), if missing.is_empty() { String::new() } else { format!(" {:?}", &missing[..missing.len().min(8)]) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn chain(l: &str) -> (Chain, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        (Chain::new(crate::store::Store::open(d.path()).unwrap(), format!("n-{l}"), "hone-test".into()), d)
    }
    fn seal(c: &Chain, e: u64) {
        c.apply_entry(&LedgerEntry::EpochSeal { node_id: "x".into(), epoch: e, timestamp: 0,
            seal_hash: "h".into(), signature: None, node_version: None }).unwrap();
    }
    fn setup(c: &Chain, sk: &SigningKey) {
        c.apply_entry(&LedgerEntry::AccountCreate { account: "alice".into(),
            keys: [("posting".to_string(), hex::encode(sk.verifying_key().to_bytes()))].into_iter().collect(),
            chain_proofs: vec![], epoch: 0, funded_by: None, machine_fingerprint: None }).unwrap();
        c.apply_entry(&LedgerEntry::GenesisAlloc { account: "alice".into(), amount: 1_000_000_000_000_000,
            token: hone_types::NATIVE_TOKEN.into() }).unwrap();
    }
    fn xfer(sk: &SigningKey, nonce: u64, epoch: u64) -> (LedgerEntry, Option<String>) {
        let e = LedgerEntry::Transfer { from: "alice".into(), to: "bob".into(), amount: 10_000_000_000,
            token: hone_types::NATIVE_TOKEN.into(), memo: None, nonce, epoch, signed_by: "alice".into(), twofactor: None };
        let m = crate::tx::canonical_signing_message(&e, "hone-test").unwrap();
        let s = hex::encode(sk.sign(m.as_bytes()).to_bytes());
        (e, Some(s))
    }

    /// A keeps running; B is down for epochs 101..=103 (mid-epoch restart: it also loses
    /// what was pending). B then sees the quorum seal of 104 and backfills from A's records:
    /// roots converge, nonce order holds (104's entry depends on the missed ones), and the
    /// stale window does not reject the missed epochs' entries.
    #[test]
    fn restarted_node_backfills_missed_epochs_and_converges() {
        let sk = SigningKey::from_bytes(&[0x42; 32]);
        let (a, _da) = chain("a");
        let (b, _db) = chain("b");
        for c in [&a, &b] { setup(c, &sk); seal(c, 100); }
        let e100 = vec![xfer(&sk, 1, 100)];
        for c in [&a, &b] { let ap = apply_set(c, 100, e100.clone(), None); record(c, 100, &ap); }
        // B goes down. A commits 101..=103 (two transfers in 102, out of hash order on purpose).
        for (ep, set) in [(101, vec![xfer(&sk, 2, 101)]), (102, vec![xfer(&sk, 4, 102), xfer(&sk, 3, 102)]), (103, vec![xfer(&sk, 5, 103)])] {
            seal(&a, ep);
            let mut set = set; set.sort_by_key(|(x, _)| x.hash());
            let ap = apply_set(&a, ep, set, None);
            assert!(!ap.is_empty());
            record(&a, ep, &ap);
            epoch_tick(&a, ep, ap.iter().map(|(x, _)| hone_types::entry_weight(x)).sum());
        }
        // Back up, tip is 110 (entries of 101..103 are now outside the stale window).
        seal(&b, 110); seal(&a, 110);
        assert_ne!(a.store.balance_merkle_root(), b.store.balance_merkle_root());
        assert_eq!(last_committed(&b), 100);
        let n = apply_backfill(&b, parse(&serve(&a, 101, 103)));
        assert_eq!(n, 4);
        // The per-epoch tick ran for the missed epochs too: same base fee as A.
        assert_eq!(b.store.state_get("chain_param:base_fee"), a.store.state_get("chain_param:base_fee"));
        // Epoch 104's entry (nonce 6) applies on both now.
        let e104 = vec![xfer(&sk, 6, 110)];
        for c in [&a, &b] { let ap = apply_set(c, 110, e104.clone(), None); assert_eq!(ap.len(), 1); record(c, 110, &ap); }
        assert_eq!(a.store.balance_merkle_root(), b.store.balance_merkle_root());
        assert_eq!(last_committed(&b), 110);
        // Exactly once: a second backfill applies nothing and changes nothing.
        let root = b.store.balance_merkle_root();
        assert_eq!(apply_backfill(&b, parse(&serve(&a, 101, 103))), 0);
        assert_eq!(b.store.balance_merkle_root(), root);
    }

    /// Seal lag: an entry dated E-1 waits for the seal of E+1, so a node that heard it late
    /// in E commits it at the same seal as the node that originated it.
    #[test]
    fn seal_lag_commits_entries_two_seals_after_their_epoch() {
        let sk = SigningKey::from_bytes(&[0x42; 32]);
        let (a, _da) = chain("lag");
        let act = hone_types::SEAL_LAG_ACTIVATION_EPOCH;
        let (old, s1) = xfer(&sk, 1, act);
        let (new, s2) = xfer(&sk, 2, act + 1);
        let (far, s3) = xfer(&sk, 3, act + 500);
        a.push_pending(old.clone(), s1); a.push_pending(new.clone(), s2); a.push_pending(far, s3);
        let got = a.drain_pending_for_seal(act + 2);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0.hash(), old.hash());
        assert_eq!(a.pending_count(), 1, "act+1 waits; far-future dropped");
        assert!(!a.push_pending(new.clone(), None), "still deduplicated while waiting");
        assert_eq!(a.drain_pending_for_seal(act + 3).len(), 1);
        // Before activation: everything, as before.
        let (x, s) = xfer(&sk, 9, act - 1);
        a.push_pending(x, s);
        assert_eq!(a.drain_pending_for_seal(act - 1).len(), 1);
    }

    /// Restart mid-epoch: what was pending (a node's own heartbeat gossiped just before the
    /// stop) and its outbox come back on a fresh Chain over the same store.
    #[test]
    fn pending_pool_and_outbox_survive_restart() {
        let sk = SigningKey::from_bytes(&[0x42; 32]);
        let d = tempfile::tempdir().unwrap();
        let st = crate::store::Store::open(d.path()).unwrap();
        let (e1, s1) = xfer(&sk, 1, 500);
        {
            let a = Chain::new(st.clone(), "a".into(), "hone-test".into());
            a.push_pending(e1.clone(), s1.clone());
            a.outbox_push(e1.clone(), s1.clone(), 500);
        }
        let b = Chain::new(st.clone(), "a".into(), "hone-test".into());
        assert_eq!(b.pending_count(), 0);
        assert_eq!(b.restore_pools(), (1, 1));
        assert_eq!(b.pending_count(), 1);
        assert_eq!(b.drain_pending_sorted()[0].0.hash(), e1.hash());
        // A drained pool stays drained across another restart.
        let c = Chain::new(st.clone(), "a".into(), "hone-test".into());
        assert_eq!(c.restore_pools().0, 0);
    }

    #[test]
    fn sync_peers_from_env_or_bootstrap() {
        std::env::remove_var("HONE_SYNC_PEERS");
        assert_eq!(peers(&["/ip4/192.168.68.72/tcp/6942/p2p/12D3".into()]), vec!["http://192.168.68.72:4242"]);
    }

    /// 2026-11-03 partition: A (grouchly) could not publish, so N (nebra) saw 2 of 5 seals
    /// and resolved nothing for 101..=110, while A heard every clock and sealed and
    /// finalized them. The gap crosses the name migration (105). N's first quorum seal
    /// after the fix is 111: the backfill must commit A's entries, run the migration inside
    /// the gap, and take A's finalize meta, or N's reward driver stalls at 101 for good.
    #[test]
    fn partitioned_node_backfills_entries_finalizes_and_migration() {
        crate::identity_nft::TEST_MIGRATION_EPOCH.with(|c| c.set(Some(105)));
        let sk = SigningKey::from_bytes(&[0x43; 32]);
        let (a, _da) = chain("a");
        let (n, _dn) = chain("n");
        for c in [&a, &n] { setup(c, &sk); seal(c, 100); }
        for c in [&a, &n] { let ap = apply_set(c, 100, vec![xfer(&sk, 1, 100)], None); record(c, 100, &ap); }
        let fin = |c: &Chain, e: u64| c.apply_entry(&LedgerEntry::EpochFinalize {
            epoch: e, node_id: "natoshisakamoto".into(), rewards_hash: format!("h{e}"), quorum: 3,
            sealed_by: vec!["josh".into(), "natoshisakamoto".into(), "shindevlin".into()],
            state_root: format!("r{e}"), timestamp: e }).unwrap();
        fin(&a, 100); fin(&n, 100);
        // Partition: A commits and finalizes 101..=110; N commits and finalizes nothing.
        for ep in 101..=110u64 {
            seal(&a, ep);
            if ep == 105 { crate::identity_nft::migrate_reserved(&a, ep).unwrap(); }
            let ap = apply_set(&a, ep, vec![xfer(&sk, ep - 99, ep)], None);
            assert_eq!(ap.len(), 1);
            record(&a, ep, &ap);
            epoch_tick(&a, ep, ap.iter().map(|(x, _)| hone_types::entry_weight(x)).sum());
            fin(&a, ep);
        }
        assert!(a.store.state_get("name_migration_done").is_some());
        // N reconnects: first quorum seal is 111.
        seal(&n, 111); seal(&a, 111);
        assert_eq!(last_committed(&n), 100);
        assert!(n.store.get_epoch_meta(101).unwrap().is_none());
        let v = serve(&a, 101, 110);
        assert_eq!(apply_backfill(&n, parse(&v)), 10);
        assert!(n.store.state_get("name_migration_done").is_some(), "migration must run inside the gap");
        assert_eq!(apply_finalized(&n, &v), 10);
        for ep in 101..=110u64 {
            let (ma, mn) = (a.store.get_epoch_meta(ep).unwrap().unwrap(), n.store.get_epoch_meta(ep).unwrap().unwrap());
            for k in ["finalized", "rewards_hash", "quorum", "sealed_by", "state_root", "finalized_at"] {
                assert_eq!(ma[k], mn[k], "epoch {ep} field {k}");
            }
        }
        assert_eq!(a.store.balance_merkle_root(), n.store.balance_merkle_root());
        assert_eq!(last_committed(&n), 110);
        // Idempotent: nothing new on a second pass, and a finalize is never rewritten.
        assert_eq!(apply_backfill(&n, parse(&v)), 0);
        assert_eq!(apply_finalized(&n, &v), 0);
        // An empty or zero-quorum peer row is ignored rather than finalizing an epoch.
        let bogus = serde_json::json!({ "finalized": [ { "epoch": 120, "rewards_hash": "", "quorum": 0 } ] });
        assert_eq!(apply_finalized(&n, &bogus), 0);
        assert!(n.store.get_epoch_meta(120).unwrap().is_none());
        crate::identity_nft::TEST_MIGRATION_EPOCH.with(|c| c.set(None));
    }
}
