//! Chain side of decide-by certified entry sets (see setcert.rs). Gate SET_CERT_EPOCH.
//!
//! - `due_hashes`: what a voter proposes at its seal of T (pool entries with e.epoch + d <= T).
//! - `apply_final`: apply one final cert's set, in hash order, as of T, then record + tick,
//!   exactly like a live seal did. Missing bodies are returned, never guessed.
//! - `d`: the current delay, a deterministic function of the d_reports inside final certs.
use std::collections::BTreeMap;
use std::sync::Mutex;

use hone_types::LedgerEntry;
use tracing::info;

use crate::chain::Chain;
use crate::epoch_sync;
use crate::setcert::{self, Cert};

const APPLIED_KEY: &str = "setcert_applied_upto";
const D_KEY: &str = "setcert_d";
const REPORTS_KEY: &str = "setcert_d_reports";

pub fn activation() -> u64 {
    #[cfg(test)]
    if let Some(v) = TEST_EPOCH.with(|c| c.get()) { return v; }
    hone_types::SET_CERT_EPOCH
}
#[cfg(test)]
thread_local! { pub static TEST_EPOCH: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) }; }

pub fn active(epoch: u64) -> bool { epoch >= activation() }

fn read_u64(chain: &Chain, k: &str) -> Option<u64> {
    chain.store.state_get(k).and_then(|b| String::from_utf8(b).ok()).and_then(|s| s.trim().parse().ok())
}

/// Last epoch whose certified set this node applied (activation - 1 before the first).
pub fn applied_upto(chain: &Chain) -> u64 { read_u64(chain, APPLIED_KEY).unwrap_or(activation().saturating_sub(1)) }
pub fn applied(chain: &Chain, epoch: u64) -> bool { epoch <= applied_upto(chain) }

/// The delay d in force for entries dated `epoch`.
pub fn d(chain: &Chain) -> u64 { read_u64(chain, D_KEY).unwrap_or(setcert::D_START) }

/// Hashes a voter proposes for T: pool entries due by T.
pub fn due_hashes(chain: &Chain, t: u64) -> Vec<String> {
    let d = d(chain);
    let pool = chain.pending.lock();
    let mut v: Vec<String> = pool.iter().filter(|(e, _)| e.epoch().saturating_add(d) <= t).map(|(e, _)| e.hash()).collect();
    v.sort(); v.dedup(); v
}

/// Bodies fetched from peers by hash (checked against the hash on insert).
static FETCHED: Mutex<BTreeMap<String, (LedgerEntry, Option<String>)>> = Mutex::new(BTreeMap::new());

pub fn offer_body(entry: LedgerEntry, sig: Option<String>) {
    let h = entry.hash();
    let mut m = FETCHED.lock().unwrap();
    if m.len() < 20_000 { m.insert(h, (entry, sig)); }
}

/// A body by hash: the pending pool, fetched bodies, or recent committed sets.
pub fn body(chain: &Chain, hash: &str) -> Option<(LedgerEntry, Option<String>)> {
    if let Some(x) = chain.pending.lock().iter().find(|(e, _)| e.hash() == hash) { return Some(x.clone()); }
    if let Some(x) = FETCHED.lock().unwrap().get(hash) { return Some(x.clone()); }
    None
}

/// Bodies this node can serve for `hashes` (pool and the last 64 recorded sets).
pub fn serve_bodies(chain: &Chain, hashes: &[String], tip: u64) -> Vec<serde_json::Value> {
    let want: std::collections::BTreeSet<&String> = hashes.iter().collect();
    let mut out = Vec::new();
    for (e, s) in chain.pending.lock().iter() {
        if want.contains(&e.hash()) { out.push(serde_json::json!({ "entry": e, "sig": s })); }
    }
    let v = epoch_sync::serve(chain, tip.saturating_sub(64), tip);
    for ep in v["epochs"].as_array().into_iter().flatten() {
        for r in ep["entries"].as_array().into_iter().flatten() {
            if let Ok(e) = serde_json::from_value::<LedgerEntry>(r["entry"].clone()) {
                if want.contains(&e.hash()) { out.push(r.clone()); }
            }
        }
    }
    out.truncate(hashes.len());
    out
}

/// Apply the final cert of `applied_upto + 1`. Err(missing hashes) if any body is absent.
pub fn apply_final(chain: &Chain, cert: &Cert) -> Result<usize, Vec<String>> {
    let t = cert.epoch;
    assert_eq!(t, applied_upto(chain) + 1, "certified sets apply strictly in order");
    let mut set = Vec::with_capacity(cert.content.hashes.len());
    let mut missing = Vec::new();
    for h in &cert.content.hashes {
        match body(chain, h) { Some(x) => set.push(x), None => missing.push(h.clone()) }
    }
    if !missing.is_empty() { return Err(missing); }
    // Out of the pool: they are decided now (applied or recorded as failed below).
    {
        let mut pool = chain.pending.lock();
        pool.retain(|(e, _)| cert.content.hashes.binary_search(&e.hash()).is_err());
    }
    {
        let mut f = FETCHED.lock().unwrap();
        for h in &cert.content.hashes { f.remove(h); }
    }
    let _ = crate::identity_nft::migrate_reserved(chain, t);
    set.sort_by_key(|(e, _)| e.hash());
    let applied = epoch_sync::apply_set(chain, t, set, Some(t));
    epoch_sync::record(chain, t, &applied);
    epoch_sync::epoch_tick(chain, t, applied.iter().map(|(x, _)| hone_types::entry_weight(x)).sum());
    // d schedule: collect reports; step at period boundaries, effective D_DELAY later.
    let mut reports: Vec<u64> = chain.store.state_get(REPORTS_KEY)
        .and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    reports.extend(cert.content.reports.iter().map(|(_, r)| *r));
    if t % setcert::D_PERIOD == 0 {
        let next = setcert::next_d(d(chain), &reports);
        let _ = chain.store.state_set(&format!("setcert_d_next:{}", t + setcert::D_DELAY), next.to_string().as_bytes());
        reports.clear();
    }
    let _ = chain.store.state_set(REPORTS_KEY, &serde_json::to_vec(&reports).unwrap_or_default());
    if let Some(n) = read_u64(chain, &format!("setcert_d_next:{t}")) {
        let _ = chain.store.state_set(D_KEY, n.to_string().as_bytes());
        let _ = chain.store.state_delete(&format!("setcert_d_next:{t}"));
    }
    let _ = chain.store.state_set(&format!("setcert_final:{t}"), &serde_json::to_vec(&serde_json::json!({
        "round": cert.round, "prev": cert.prev, "content_hash": cert.content.hash(), "n": cert.content.hashes.len(),
        "voters": cert.votes.keys().collect::<Vec<_>>(),
    })).unwrap_or_default());
    let _ = chain.store.state_set(APPLIED_KEY, t.to_string().as_bytes());
    info!("[setcert] epoch {} applied {}/{} certified entries (round {})", t, applied.len(), cert.content.hashes.len(), cert.round);
    Ok(applied.len())
}

/// Local p99 first-seen lag (epochs) over the last entries this node admitted. Not
/// consensus: it is only this voter's report; the certified median decides d.
static LAGS: Mutex<Vec<u64>> = Mutex::new(Vec::new());
pub fn note_seen(entry_epoch: u64, now_epoch: u64) {
    let mut l = LAGS.lock().unwrap();
    l.push(now_epoch.saturating_sub(entry_epoch));
    if l.len() > 2_000 { let n = l.len() - 2_000; l.drain(..n); }
}
pub fn d_report() -> u64 {
    let mut l = LAGS.lock().unwrap().clone();
    if l.is_empty() { return setcert::D_START; }
    l.sort_unstable();
    let p99 = l[((l.len() as f64) * 0.99).ceil() as usize - 1];
    (p99 + 1).clamp(setcert::D_MIN, setcert::D_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setcert::{Engine, Prop, Vote};
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::Digest;

    struct Rng(u64);
    impl Rng { fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 } }

    fn chain(l: &str) -> (Chain, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        (Chain::new(crate::store::Store::open(d.path()).unwrap(), format!("n-{l}"), "hone-test".into()), d)
    }
    fn setup(c: &Chain, sk: &SigningKey, who: &str) {
        c.apply_entry(&LedgerEntry::AccountCreate { account: who.into(),
            keys: [("posting".to_string(), hex::encode(sk.verifying_key().to_bytes()))].into_iter().collect(),
            chain_proofs: vec![], epoch: 0, funded_by: None, machine_fingerprint: None }).unwrap();
        c.apply_entry(&LedgerEntry::GenesisAlloc { account: who.into(), amount: 1_000_000_000_000_000,
            token: hone_types::NATIVE_TOKEN.into() }).unwrap();
    }
    fn xfer(sk: &SigningKey, from: &str, to: &str, nonce: u64, epoch: u64, amt: u64) -> (LedgerEntry, Option<String>) {
        let e = LedgerEntry::Transfer { from: from.into(), to: to.into(), amount: amt,
            token: hone_types::NATIVE_TOKEN.into(), memo: None, nonce, epoch, signed_by: from.into(), twofactor: None };
        let m = crate::tx::canonical_signing_message(&e, "hone-test").unwrap();
        (e, Some(hex::encode(sk.sign(m.as_bytes()).to_bytes())))
    }
    fn signer(me: &str) -> impl Fn(&str) -> String + '_ { move |m: &str| format!("{me}|{}", hex::encode(sha2::Sha256::digest(m.as_bytes()))) }
    fn verify(v: &str, m: &str, s: &str) -> bool { s == format!("{v}|{}", hex::encode(sha2::Sha256::digest(m.as_bytes()))) }

    #[derive(Clone)]
    enum Msg { P(Prop), V(Vote), C(Cert), E(LedgerEntry, Option<String>) }

    /// The drift this removes: three nodes hear the same signed transfers in different
    /// orders and at different times (some only epochs later, some never directly), two of
    /// them are voters, one only observes. Every node must end every applied epoch with the
    /// identical state root, and every transfer must be applied exactly once.
    #[test]
    fn nodes_with_different_gossip_timing_end_identical() {
        let act = 500u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk_a = SigningKey::from_bytes(&[0x51; 32]);
        let sk_b = SigningKey::from_bytes(&[0x52; 32]);
        let voters = vec!["natoshisakamoto".to_string(), "shindevlin".to_string()];
        for seed in [3u64, 17, 2026] {
            let mut rng = Rng(seed);
            let nodes: Vec<(Chain, tempfile::TempDir)> = (0..3).map(|i| chain(&format!("{seed}-{i}"))).collect();
            for (c, _) in &nodes { setup(c, &sk_a, "alice"); setup(c, &sk_b, "bob"); }
            let mut eng: Vec<Engine> = (0..3).map(|_| Engine::new("hone-test", act, voters.clone())).collect();
            // 40 transfers alternating senders, dated over 20 epochs; nonces in order per sender.
            let mut txs = Vec::new();
            let (mut na, mut nb) = (0u64, 0u64);
            for i in 0..40u64 {
                let ep = act + i / 2;
                if i % 2 == 0 { na += 1; txs.push((ep, xfer(&sk_a, "alice", "bob", na, ep, 1_000 + i))); }
                else { nb += 1; txs.push((ep, xfer(&sk_b, "bob", "carol", nb, ep, 2_000 + i))); }
            }
            let mut q: Vec<(u64, usize, Msg)> = Vec::new();
            for (ep, (e, s)) in &txs {
                for to in 0..3 {
                    // each node hears it 0..4 epochs after its date, at a random second
                    let at = (ep - act) * 30_000 + (rng.next() % 4) * 30_000 + rng.next() % 30_000;
                    if to == 2 && rng.next() % 3 == 0 { continue; } // the observer misses some entirely
                    q.push((at, to, Msg::E(e.clone(), s.clone())));
                }
            }
            let total_s = 60 * 30;
            let mut roots: Vec<BTreeMap<u64, String>> = vec![BTreeMap::new(); 3];
            for sec in 0..total_s {
                let t = sec * 1_000;
                let epoch = act + sec / 30;
                let mut due: Vec<(u64, usize, Msg)> = Vec::new();
                let mut rest = Vec::new();
                for x in q.drain(..) { if x.0 <= t { due.push(x) } else { rest.push(x) } }
                q = rest;
                while !due.is_empty() {
                    let k = (rng.next() as usize) % due.len();
                    let (_, to, m) = due.swap_remove(k);
                    let c = &nodes[to].0;
                    match m {
                        Msg::E(e, s) => { if !applied_hash(c, &e) { c.push_pending(e, s); } }
                        Msg::P(p) => { eng[to].on_prop(p, t, &verify); }
                        Msg::V(v) => if let Some(cert) = eng[to].on_vote(v, &verify) {
                            for j in 0..3 { if j != to { q.push((t + rng.next() % 3_000, j, Msg::C(cert.clone()))); } }
                        },
                        Msg::C(cert) => { eng[to].on_cert(cert, &verify); }
                    }
                }
                if sec % 30 == 5 {
                    for (i, v) in voters.iter().enumerate() {
                        let s = signer(v);
                        let hs = due_hashes(&nodes[i].0, epoch);
                        if let Some(p) = eng[i].propose(v, epoch, hs, 3, t, &s) {
                            for j in 0..3 { if j != i { q.push((t + rng.next() % 4_000, j, Msg::P(p.clone()))); } }
                        }
                    }
                }
                for (i, v) in voters.iter().enumerate() {
                    for e in act..=epoch {
                        let s = signer(v);
                        if let Some(vote) = eng[i].maybe_vote(v, e, t, &s) {
                            for j in 0..3 { if j != i { q.push((t + rng.next() % 3_000, j, Msg::V(vote.clone()))); } }
                        }
                    }
                }
                for i in 0..3 {
                    eng[i].finalize();
                    loop {
                        let next = applied_upto(&nodes[i].0) + 1;
                        let Some(cert) = eng[i].final_cert(next).cloned() else { break };
                        match apply_final(&nodes[i].0, &cert) {
                            Ok(_) => { roots[i].insert(next, nodes[i].0.store.balance_merkle_root()); }
                            Err(missing) => {
                                // fetch by hash from a peer that has them (gossip "ents")
                                let mut got = false;
                                for j in 0..3 { if j == i { continue; }
                                    for r in serve_bodies(&nodes[j].0, &missing, epoch) {
                                        if let Ok(e) = serde_json::from_value::<LedgerEntry>(r["entry"].clone()) {
                                            offer_body(e, r["sig"].as_str().map(str::to_owned)); got = true;
                                        }
                                    }
                                }
                                if !got { break; }
                            }
                        }
                    }
                }
            }
            let common = roots.iter().map(|r| r.len()).min().unwrap();
            assert!(common >= 25, "seed {seed}: liveness, only {common} epochs applied");
            for i in 1..3 {
                for (e, r) in roots[0].iter().take(common) { assert_eq!(roots[i].get(e), Some(r), "seed {seed} node {i} epoch {e}"); }
            }
            // every transfer applied exactly once on every node that got that far
            let last = *roots[0].keys().nth(common - 1).unwrap();
            if last >= act + 26 {
                for (c, _) in &nodes {
                    let bal = |a: &str| c.store.get_balance(a, hone_types::NATIVE_TOKEN);
                    assert_eq!(bal("carol"), (0..40u64).filter(|i| i % 2 == 1).map(|i| 2_000 + i).sum::<u64>());
                }
            }
        }
        TEST_EPOCH.with(|c| c.set(None));
    }

    fn applied_hash(c: &Chain, e: &LedgerEntry) -> bool {
        let v = epoch_sync::serve(c, applied_upto(c).saturating_sub(64), applied_upto(c));
        v["epochs"].as_array().into_iter().flatten().flat_map(|x| x["entries"].as_array().cloned().unwrap_or_default())
            .any(|r| r["entry"] == serde_json::to_value(e).unwrap())
    }

    #[test]
    fn missing_body_waits_never_guesses() {
        let act = 50u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk = SigningKey::from_bytes(&[0x53; 32]);
        let (c, _d) = chain("mb");
        setup(&c, &sk, "alice");
        let (e, s) = xfer(&sk, "alice", "bob", 1, act, 5);
        let cert = Cert { epoch: act, round: 0, prev: setcert::genesis_prev(act),
            content: setcert::Content { hashes: vec![e.hash()], reports: vec![] }, votes: Default::default() };
        let root = c.store.balance_merkle_root();
        assert_eq!(apply_final(&c, &cert), Err(vec![e.hash()]));
        assert_eq!(c.store.balance_merkle_root(), root, "nothing applied while a body is missing");
        assert_eq!(applied_upto(&c), act - 1);
        offer_body(e, s);
        assert_eq!(apply_final(&c, &cert), Ok(1));
        assert_eq!(applied_upto(&c), act);
        TEST_EPOCH.with(|c| c.set(None));
    }
}
