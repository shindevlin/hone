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
/// Current d, capped at D_MAX (a stored d from before the cap, e.g. 10, is ignored at once).
pub fn d(chain: &Chain) -> u64 { read_u64(chain, D_KEY).unwrap_or(setcert::D_START).min(setcert::D_MAX) }
const _: () = assert!(setcert::D_MAX < crate::tx::STALE_WINDOW, "d must stay inside the stale window");

/// Epochs a voter works on at once beyond its first non-final epoch (catch-up batch).
pub const CATCHUP_BATCH: u64 = 32;
/// Own PROPs / votes / chosen certs of stuck epochs are re-broadcast this often.
pub const REBROADCAST_MS: u64 = 10_000;
/// No certificate this many epochs behind the tip raises a healer alert.
pub const GAP_ALERT_EPOCHS: u64 = 20;

/// A message a voter step wants gossiped.
#[derive(Debug, Clone)]
pub enum Out { P(setcert::Prop), V(setcert::Vote), C(setcert::Cert) }

/// One voter tick (pure over engine and chain, shared by the node driver and the tests):
///  - PROP for every sealed epoch (t + 2 <= cur) from the first non-final one on that this
///    voter holds no PROP for (lost in a partition, or the node was down at that seal);
///  - votes for first..=first+CATCHUP_BATCH (no fixed look-back from the tip);
///  - when `rebroadcast`: own PROPs, latest votes and chosen certs of stuck epochs
///    (t + 3 <= cur), so a partition or a lost message never ends liveness.
/// Returns (new messages, re-broadcast messages).
pub fn voter_step(e: &mut setcert::Engine, chain: &Chain, me: &str, cur: u64, now_ms: u64, d_report: u64,
                  rebroadcast: bool, sign: setcert::Sign) -> (Vec<Out>, Vec<Out>) {
    e.set_tip(cur);
    let first = e.last_final().map(|x| x + 1).unwrap_or(e.activation);
    let hi = cur.min(first + CATCHUP_BATCH);
    let (mut new, mut rb) = (Vec::new(), Vec::new());
    // Catch-up rules (2026-11-15). A node that has not applied what is already certified
    // (behind) makes no new PROPs for past epochs: it only votes on others' (DA rule). A
    // synced node's PROP for an open past epoch older than the stale window is empty (the
    // epoch closes as empty); otherwise it names only entries still valid at T, held and kept.
    let behind = applied_upto(chain).saturating_add(BEHIND_SLACK) < e.last_final().unwrap_or(0);
    for t in first..=hi {
        if t + 2 <= cur && e.own_prop(me, t).is_none() {
            let past = t + 3 <= cur;
            // Behind, or older than the stale window: an empty PROP (never stale or unheld
            // entries; and if every voter is behind, empty sets still finalize, so the chain
            // never waits on a PROP nobody would make).
            let hashes = if past && (behind || t + crate::tx::STALE_WINDOW < cur) { Vec::new() } else { due_hashes(chain, t) };
            if let Some(p) = e.propose(me, t, hashes, d_report, now_ms, sign) {
                for h in &p.hashes { keep_voted_body(chain, h); }
                new.push(Out::P(p));
            }
        }
    }
    for t in first..=hi {
        // Data availability: never vote for a set naming a body this voter does not hold.
        if let Some(hs) = e.vote_hashes(t) {
            // Already voted this content in this run (it passed this check then). A restored
            // vote held back for unheld bodies never counts as checked.
            let same = e.resend_vote_ok(t) && e.own_vote(t).is_some_and(|v| v.content.hashes == hs);
            if !hs.is_empty() && !same {
                let missing: Vec<String> = hs.iter().filter(|h| body(chain, h).is_none()).cloned().collect();
                if !missing.is_empty() { da_missing().lock().unwrap().extend(missing); continue; }
            }
        }
        if let Some(v) = e.maybe_vote(me, t, now_ms, sign) {
            e.release_vote(t);
            for h in &v.content.hashes { keep_voted_body(chain, h); }
            new.push(Out::V(v));
        }
    }
    if rebroadcast {
        for t in first..=hi {
            if t + 3 > cur { break; }
            if e.resend_prop_ok(t) { if let Some(p) = e.own_prop(me, t) { rb.push(Out::P(p.clone())); } }
            if e.resend_vote_ok(t) { if let Some(v) = e.own_vote(t) { rb.push(Out::V(v.clone())); } }
            if let Some(c) = e.chosen(t) { rb.push(Out::C(c.clone())); }
        }
    }
    (new, rb)
}

/// Hashes a voter proposes for T: pool entries due by T.
pub fn due_hashes(chain: &Chain, t: u64) -> Vec<String> {
    let d = d(chain);
    let pool = chain.pending.lock();
    // Never an entry too old to apply at T (it would be certified and then rejected; a stuck
    // node's ancient pool re-proposed such entries on 2026-11-15).
    let mut v: Vec<String> = pool.iter()
        .filter(|(e, _)| e.epoch().saturating_add(d) <= t && e.epoch().saturating_add(crate::tx::STALE_WINDOW) >= t)
        .map(|(e, _)| e.hash()).collect();
    v.sort(); v.dedup(); v
}

/// Bodies fetched from peers by hash (checked against the hash on insert).
static FETCHED: Mutex<BTreeMap<String, (LedgerEntry, Option<String>)>> = Mutex::new(BTreeMap::new());

pub fn offer_body(entry: LedgerEntry, sig: Option<String>) {
    let h = entry.hash();
    let mut m = FETCHED.lock().unwrap();
    if m.len() < crate::mem_profile::get().fetched_max { m.insert(h, (entry, sig)); }
}

/// A body by hash: the pending pool, fetched bodies, or a body this voter voted for.
pub fn body(chain: &Chain, hash: &str) -> Option<(LedgerEntry, Option<String>)> {
    // Keyed lookups only: hashing the pool entry by entry (100k+ entries on 2026-11-15) for
    // every lookup hung a voter restoring 1,052 own PROPs at startup (beastly, 7d56432b).
    if let Some(x) = FETCHED.lock().unwrap().get(hash) { return Some(x.clone()); }
    if let Some(x) = chain.pool_body(hash) { return Some(x); }
    if let Some(x) = voted_body(chain, hash) { return Some(x); }
    // A body already certified (applied or rejected) and kept: a late set naming it again
    // is held here (apply then rejects the duplicate by exactly-once), never a halt.
    kept_body(chain, hash)
}

// ── Data availability (incident 2026-11-15, epoch 384535) ──────────────────────────
// A voter votes for a set only when it holds every body in it, and keeps each such body in
// local state until the set is applied. So a certified set can never name a body that no
// quorum voter holds, and a restart, crash or state swap of a voter's memory cannot lose it.
const VB_PREFIX: &str = "setcert_vb:";
/// A node more than this many epochs behind the last final cert it holds is "behind".
pub const BEHIND_SLACK: u64 = 2;
/// Held in persisted local state (persisted pool, kept voted body, kept certified set) or memory.
pub fn held(chain: &Chain, hash: &str) -> bool {
    body(chain, hash).is_some() || kept_body(chain, hash).is_some()
}
/// Own PROPs and votes saved before a restart (setcert_own.json) are re-sent only if every
/// body they name is still held here; a state swap or crash that lost them drops them (the
/// floors still cover their epochs, so nothing different is signed there). Returns
/// (kept props, kept votes, dropped count).
/// Restore a voter's own PROPs and votes from before a restart. All of them become engine
/// state again (they are signed and out there: peers keep the first PROP of each voter, and
/// the round state lets this voter vote a later round of an epoch it already voted in;
/// dropping them left beastly unable to PROPose or vote 385572.. ever again, 2026-11-15).
/// Those naming a body no longer held here are never re-sent (rule 1). Returns how many.
pub fn restore_into(e: &mut setcert::Engine, chain: &Chain, props: Vec<setcert::Prop>, votes: Vec<setcert::Vote>) -> usize {
    let (kp, kv, held_back) = restorable(chain, props.clone(), votes.clone());
    let pe: std::collections::BTreeSet<u64> = kp.iter().map(|p| p.epoch).collect();
    let ve: std::collections::BTreeSet<(u64, u32)> = kv.iter().map(|v| (v.epoch, v.round)).collect();
    let hold_p: Vec<u64> = props.iter().map(|p| p.epoch).filter(|t| !pe.contains(t)).collect();
    let hold_v: Vec<u64> = votes.iter().filter(|v| !ve.contains(&(v.epoch, v.round))).map(|v| v.epoch).collect();
    e.restore_own(props, votes);
    e.hold_back(hold_p, hold_v);
    held_back
}

pub fn restorable(chain: &Chain, props: Vec<setcert::Prop>, votes: Vec<setcert::Vote>)
    -> (Vec<setcert::Prop>, Vec<setcert::Vote>, usize) {
    let n = props.len() + votes.len();
    // Each distinct hash is checked once (keyed lookups), however many PROPs/votes name it.
    let all: std::collections::BTreeSet<&String> = props.iter().flat_map(|p| p.hashes.iter())
        .chain(votes.iter().flat_map(|v| v.content.hashes.iter())).collect();
    let lost: std::collections::HashSet<String> = all.into_iter().filter(|h| !held(chain, h)).cloned().collect();
    let props: Vec<_> = props.into_iter().filter(|p| !p.hashes.iter().any(|h| lost.contains(h))).collect();
    let votes: Vec<_> = votes.into_iter().filter(|v| !v.content.hashes.iter().any(|h| lost.contains(h))).collect();
    let dropped = n - props.len() - votes.len();
    (props, votes, dropped)
}
/// Bodies this voter lacks to vote; the driver fetches them like missing bodies.
/// Process-wide in the node; per test thread under test (tests run in parallel).
pub fn da_missing() -> &'static Mutex<std::collections::BTreeSet<String>> {
    #[cfg(test)]
    {
        thread_local!(static M: &'static Mutex<std::collections::BTreeSet<String>> =
            Box::leak(Box::new(Mutex::new(std::collections::BTreeSet::new()))));
        M.with(|m| *m)
    }
    #[cfg(not(test))]
    {
        static M: Mutex<std::collections::BTreeSet<String>> = Mutex::new(std::collections::BTreeSet::new());
        &M
    }
}
/// When each body was last asked for (a peer that lacks one scans its whole keep window).
fn da_asked() -> &'static Mutex<BTreeMap<String, u64>> {
    #[cfg(test)]
    {
        thread_local!(static M: &'static Mutex<BTreeMap<String, u64>> = Box::leak(Box::new(Mutex::new(BTreeMap::new()))));
        M.with(|m| *m)
    }
    #[cfg(not(test))]
    {
        static M: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());
        &M
    }
}
/// Ask again for the same body at most this often.
pub const DA_REASK_MS: u64 = 30_000;
/// Bodies to fetch now for the DA rule: what voter_step found missing, minus those already
/// asked for within DA_REASK_MS (at most 512).
pub fn da_due(now_ms: u64) -> Vec<String> {
    let want = std::mem::take(&mut *da_missing().lock().unwrap());
    let mut asked = da_asked().lock().unwrap();
    asked.retain(|_, at| now_ms.saturating_sub(*at) < DA_REASK_MS);
    let out: Vec<String> = want.into_iter().filter(|h| !asked.contains_key(h)).take(512).collect();
    for h in &out { asked.insert(h.clone(), now_ms); }
    out
}
fn voted_body(chain: &Chain, hash: &str) -> Option<(LedgerEntry, Option<String>)> {
    let v: serde_json::Value = chain.store.state_get(&format!("{VB_PREFIX}{hash}")).and_then(|b| serde_json::from_slice(&b).ok())?;
    Some((serde_json::from_value(v["entry"].clone()).ok()?, v["sig"].as_str().map(str::to_owned)))
}
fn keep_voted_body(chain: &Chain, hash: &str) {
    let k = format!("{VB_PREFIX}{hash}");
    if chain.store.state_get(&k).is_some() { return; }
    if let Some((e, s)) = body(chain, hash) {
        let _ = chain.store.state_set(&k, &serde_json::to_vec(&serde_json::json!({ "entry": e, "sig": s })).unwrap_or_default());
    }
}

// ── Unavailability certificate ─────────────────────────────────────────────────────
/// Statements being signed, by epoch (memory); a quorum one is persisted (local meta).
static UNAVAIL: Mutex<BTreeMap<u64, setcert::Unavail>> = Mutex::new(BTreeMap::new());
const UNAVAIL_PREFIX: &str = "setcert_unavail:";
/// The quorum unavailability certificate this node holds for `t`.
pub fn unavail_for(chain: &Chain, t: u64) -> Option<setcert::Unavail> {
    chain.store.state_get(&format!("{UNAVAIL_PREFIX}{t:016x}")).and_then(|b| serde_json::from_slice(&b).ok())
}
/// The statement for `t` still collecting signatures (or complete), to re-broadcast.
pub fn unavail_pending(t: u64) -> Option<setcert::Unavail> { UNAVAIL.lock().unwrap().get(&t).cloned() }
/// Merge a statement's verified signatures; persist once a voter quorum signed the same
/// (epoch, content hash, sorted hashes). Returns the merged statement.
pub fn on_unavail(chain: &Chain, e: &setcert::Engine, mut u: setcert::Unavail, verify: setcert::Verify) -> Option<setcert::Unavail> {
    u.hashes.sort(); u.hashes.dedup();
    if u.hashes.is_empty() || unavail_for(chain, u.epoch).is_some() { return None; }
    let msg = setcert::unavail_message(&chain.chain_id, u.epoch, &u.content_hash, &u.hashes);
    let good: BTreeMap<String, String> = u.sigs.into_iter().filter(|(v, s)| verify(v, &msg, s)).collect();
    if good.is_empty() { return None; }
    let mut m = UNAVAIL.lock().unwrap();
    let cur = m.entry(u.epoch).or_insert_with(|| setcert::Unavail { epoch: u.epoch, content_hash: u.content_hash.clone(),
        hashes: u.hashes.clone(), sigs: BTreeMap::new() });
    if cur.content_hash != u.content_hash || cur.hashes != u.hashes {
        if good.len() <= cur.sigs.len() { return None; }
        *cur = setcert::Unavail { epoch: u.epoch, content_hash: u.content_hash.clone(), hashes: u.hashes.clone(), sigs: BTreeMap::new() };
    }
    cur.sigs.extend(good);
    let out = cur.clone();
    drop(m);
    if e.quorum_signers(&out.sigs) {
        let _ = chain.store.state_set(&format!("{UNAVAIL_PREFIX}{:016x}", out.epoch), &serde_json::to_vec(&out).unwrap_or_default());
        tracing::warn!("[setcert] epoch {}: quorum unavailability certificate for {} bodies ({} signers)", out.epoch, out.hashes.len(), out.sigs.len());
    }
    Some(out)
}

/// Every body of the last DECIDED_KEEP certified sets, applied or rejected (local memory, not
/// state). Recorded sets hold only applied entries, so without this a certified entry that
/// was rejected everywhere could never be served to a node still missing it, and that node
/// would wait forever (lab soak 2026-11-10: s2 stuck on epoch 40, 22 rejected bodies).
pub const DECIDED_KEEP: u64 = 128;
static DECIDED: Mutex<BTreeMap<u64, Vec<(LedgerEntry, Option<String>)>>> = Mutex::new(BTreeMap::new());

fn keep_decided(t: u64, set: &[(LedgerEntry, Option<String>)]) {
    let mut d = DECIDED.lock().unwrap();
    d.insert(t, set.to_vec());
    let floor = t.saturating_sub(DECIDED_KEEP.min(crate::mem_profile::get().decided_keep));
    d.retain(|e, _| *e >= floor);
}

/// Final certs this node applied, kept for epoch_sync::KEEP_EPOCHS (local meta, not consensus).
const CERT_PREFIX: &str = "setcert_cert:";
/// The final cert this node applied for `t` (kept for the keep window), for "certs" answers.
pub fn applied_cert(chain: &Chain, t: u64) -> Option<Cert> {
    chain.store.state_get(&format!("{CERT_PREFIX}{t:016x}")).and_then(|b| serde_json::from_slice(&b).ok())
}
/// Every certified set's bodies, applied AND rejected, also go to local state for
/// SET_KEEP_EPOCHS (24 h) with a hash index, so a node that fell more than DECIDED_KEEP
/// epochs behind, or whose peers restarted, can still fetch them over gossip or HTTP
/// (incident 2026-11-11..15: grouchly and nebra halted at 373,972 for 3.5 days because the
/// one rejected body of that set lived only in memory). Local data, not consensus.
pub const SET_KEEP_EPOCHS: u64 = 2_880;
fn set_key(t: u64) -> String { format!("setcert_set:{t:012}") }
fn idx_key(h: &str) -> String { format!("setcert_bidx:{h}") }

fn keep_set(chain: &Chain, t: u64, set: &[(LedgerEntry, Option<String>)]) {
    let rows: Vec<serde_json::Value> = set.iter().map(|(e, s)| serde_json::json!({ "entry": e, "sig": s })).collect();
    let _ = chain.store.state_set(&set_key(t), &serde_json::to_vec(&rows).unwrap_or_default());
    for (e, _) in set { let _ = chain.store.state_set(&idx_key(&e.hash()), t.to_string().as_bytes()); }
    if t > SET_KEEP_EPOCHS {
        let old = t - SET_KEEP_EPOCHS;
        for r in kept_set(chain, old).unwrap_or_default() {
            if let Ok(e) = serde_json::from_value::<LedgerEntry>(r["entry"].clone()) {
                let k = idx_key(&e.hash());
                if read_u64(chain, &k) == Some(old) { let _ = chain.store.state_delete(&k); }
            }
        }
        let _ = chain.store.state_delete(&set_key(old));
    }
}

/// The kept certified set of epoch `t` (rows {entry, sig}), applied and rejected bodies.
pub fn kept_set(chain: &Chain, t: u64) -> Option<Vec<serde_json::Value>> {
    chain.store.state_get(&set_key(t)).and_then(|b| serde_json::from_slice(&b).ok())
}

fn kept_body(chain: &Chain, hash: &str) -> Option<(LedgerEntry, Option<String>)> {
    let t = read_u64(chain, &idx_key(hash))?;
    kept_set(chain, t)?.into_iter().find_map(|r| {
        let e = serde_json::from_value::<LedgerEntry>(r["entry"].clone()).ok()?;
        (e.hash() == hash).then(|| (e, r["sig"].as_str().map(str::to_owned)))
    })
}

/// A certified body by hash from the kept certified sets (applied or rejected).
pub fn certified_body(chain: &Chain, hash: &str) -> Option<serde_json::Value> {
    kept_body(chain, hash).map(|(e, s)| serde_json::json!({ "entry": e, "sig": s }))
}

/// Bodies this node can serve for `hashes`: the pool, the last certified sets in memory, the
/// kept certified sets (keep window), then recorded sets back to the keep window.
pub fn serve_bodies(chain: &Chain, hashes: &[String], tip: u64) -> Vec<serde_json::Value> {
    let want: std::collections::BTreeSet<&String> = hashes.iter().collect();
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for h in &want {
        if let Some((e, s)) = chain.pool_body(h) { seen.insert((*h).clone()); out.push(serde_json::json!({ "entry": e, "sig": s })); }
    }
    for set in DECIDED.lock().unwrap().values() {
        for (e, s) in set {
            if want.contains(&e.hash()) && seen.insert(e.hash()) { out.push(serde_json::json!({ "entry": e, "sig": s })); }
        }
    }
    for h in &want {
        if seen.contains(*h) { continue; }
        if let Some(r) = certified_body(chain, h) { seen.insert((*h).clone()); out.push(r); }
    }
    // Sets applied before the persisted index existed: scan recorded sets back over the whole
    // keep window, newest first, in chunks (rare: only for bodies nobody else holds).
    let mut hi = tip;
    let floor = tip.saturating_sub(epoch_sync::KEEP_EPOCHS);
    while seen.len() < want.len() && hi > floor {
        let lo = hi.saturating_sub(256).max(floor);
        let v = epoch_sync::serve(chain, lo, hi);
        for ep in v["epochs"].as_array().into_iter().flatten() {
            for r in ep["entries"].as_array().into_iter().flatten() {
                if let Ok(e) = serde_json::from_value::<LedgerEntry>(r["entry"].clone()) {
                    if want.contains(&e.hash()) && seen.insert(e.hash()) { out.push(r.clone()); }
                }
            }
        }
        if lo == floor { break; }
        hi = lo.saturating_sub(1);
    }
    for h in hashes {
        if seen.contains(h) { continue; }
        if let Some((e, s)) = kept_body(chain, h) { seen.insert(h.clone()); out.push(serde_json::json!({ "entry": e, "sig": s })); }
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
    // A quorum unavailability certificate for exactly this set: those bodies are left out
    // everywhere (even where present), recorded as rejected, so every node applies the same.
    let un: std::collections::BTreeSet<String> = unavail_for(chain, t)
        .filter(|u| u.content_hash == cert.content.hash())
        .map(|u| u.hashes.into_iter().collect()).unwrap_or_default();
    for h in &cert.content.hashes {
        if un.contains(h) { continue; }
        match body(chain, h) { Some(x) => set.push(x), None => missing.push(h.clone()) }
    }
    if !missing.is_empty() { return Err(missing); }
    // Out of the pool: they are decided now (applied or recorded as failed below).
    {
        // Also drop what can never be applied after t (dated more than the stale window
        // before it): the pool stays bounded by the window, not by the backlog.
        let w = crate::tx::STALE_WINDOW;
        let mut stale = Vec::new();
        let mut pool = chain.pending.lock();
        pool.retain(|(e, _)| {
            if e.epoch().saturating_add(w) < t { stale.push(e.hash()); return false; }
            cert.content.hashes.binary_search(&e.hash()).is_err()
        });
        drop(pool);
        chain.pool_forget(&stale);
    }
    chain.pool_forget(&cert.content.hashes);
    {
        let mut f = FETCHED.lock().unwrap();
        for h in &cert.content.hashes { f.remove(h); }
    }
    let _ = crate::identity_nft::migrate_reserved(chain, t);
    set.sort_by_key(|(e, _)| e.hash());
    keep_decided(t, &set);
    keep_set(chain, t, &set);
    let _ = chain.store.state_set(&format!("{CERT_PREFIX}{t:016x}"), &serde_json::to_vec(cert).unwrap_or_default());
    if t > epoch_sync::KEEP_EPOCHS { let _ = chain.store.state_delete(&format!("{CERT_PREFIX}{:016x}", t - epoch_sync::KEEP_EPOCHS)); }
    let applied = epoch_sync::apply_set(chain, t, set, Some(t));
    for h in &un { chain.record_entry_result(h, t, Some("unavailable: no node held this certified body (quorum unavailability certificate)".into())); }
    UNAVAIL.lock().unwrap().remove(&t);
    for h in &cert.content.hashes { let _ = chain.store.state_delete(&format!("{VB_PREFIX}{h}")); }
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
    })).unwrap_or_default());
    let _ = chain.store.state_set(APPLIED_KEY, t.to_string().as_bytes());
    info!("[setcert] epoch {} applied {}/{} certified entries (round {})", t, applied.len(), cert.content.hashes.len(), cert.round);
    Ok(applied.len())
}

/// Local p99 first-seen lag (epochs) over the last entries this node admitted. Not
/// consensus: it is only this voter's report; the certified median decides d.
static LAGS: Mutex<Vec<u64>> = Mutex::new(Vec::new());
pub fn note_seen(entry_epoch: u64, now_epoch: u64) {
    // Only live propagation counts: an entry first seen more than 2 * D_MAX epochs after its
    // date is backlog (a pool restored after a restart, a catch-up), not gossip lag.
    if now_epoch.saturating_sub(entry_epoch) > 2 * setcert::D_MAX { return; }
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

    const D_TEST: u64 = 3;
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

    /// Incident 2026-11-11..15: a certified set whose only entry is rejected everywhere.
    /// A node that applies it keeps the rejected body in state (not only in the 128-epoch
    /// memory), serves it after its memory is gone, and prunes it after SET_KEEP_EPOCHS.
    #[test]
    fn rejected_certified_body_is_kept_and_served_after_memory_is_gone() {
        let act = 500u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk = SigningKey::from_bytes(&[0x71; 32]);
        let (c, _d) = chain("keep");
        setup(&c, &sk, "alice");
        // More than alice holds: certified (voters only order hashes) but rejected on apply.
        let (e, sig) = xfer(&sk, "alice", "bob", 1, act, u64::MAX);
        let h = e.hash();
        c.push_pending(e.clone(), sig);
        let cert = Cert { epoch: act, round: 0, prev: String::new(),
            content: crate::setcert::Content { hashes: vec![h.clone()], reports: vec![] }, votes: BTreeMap::new() };
        assert_eq!(apply_final(&c, &cert), Ok(0), "rejected on apply");
        DECIDED.lock().unwrap().clear();                      // memory gone (restart / >128 epochs)
        let got = serve_bodies(&c, &[h.clone()], act + 5_000);
        assert_eq!(got.len(), 1, "served from the kept set");
        assert_eq!(serde_json::from_value::<LedgerEntry>(got[0]["entry"].clone()).unwrap().hash(), h);
        assert!(kept_set(&c, act).is_some());
        // pruned once SET_KEEP_EPOCHS later sets are kept
        keep_set(&c, act + SET_KEEP_EPOCHS, &[]);
        assert!(kept_set(&c, act).is_none());
        assert!(serve_bodies(&c, &[h], act + 5_000).is_empty());
        TEST_EPOCH.with(|c| c.set(None));
    }

    /// Incident 2026-11-11 replay at scale. Three clock voters; two funded senders submit one
    /// transfer each per epoch for 760 epochs (nonces in order). From act+15 every voter is cut
    /// off from every other for 730 epochs: all PROPs, votes, certs and entry gossip are lost,
    /// and each transfer reaches only the node it was submitted to. Then the network heals and
    /// the voters catch up with voter_step (re-proposals, no look-back limit, re-broadcast).
    /// Mid catch-up one voter restarts: a fresh engine resumed from chain state, its persisted
    /// own PROPs/votes restored, its pool reloaded from disk. Required: every node applies every
    /// epoch with identical balance roots per epoch, and every transfer exactly once.
    #[test]
    fn total_partition_730_epoch_backlog_catch_up_and_restart() {
        let act = 500u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let d = D_TEST;
        let voters = vec!["josh".to_string(), "natoshisakamoto".to_string(), "shindevlin".to_string()];
        let sk_a = SigningKey::from_bytes(&[0x61; 32]);
        let sk_b = SigningKey::from_bytes(&[0x62; 32]);
        let dirs: Vec<tempfile::TempDir> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
        let stores: Vec<crate::store::Store> = dirs.iter().map(|d| crate::store::Store::open(d.path()).unwrap()).collect();
        let mut nodes: Vec<Chain> = stores.iter().enumerate().map(|(i, st)| Chain::new(st.clone(), format!("cu-{i}"), "hone-test".into())).collect();
        for c in &nodes { setup(c, &sk_a, "alice"); setup(c, &sk_b, "bob"); }
        let mut eng: Vec<Engine> = (0..3).map(|_| Engine::new("hone-test", act, voters.clone())).collect();
        let n_ep = 760u64;
        let (part_from, part_to) = (act + 15, act + 15 + 730);
        let mut rng = Rng(2026_1111);
        let mut q: Vec<(u64, usize, Msg)> = Vec::new();
        let mut txs = Vec::new();
        for i in 0..n_ep {
            let ep = act + i;
            let a = xfer(&sk_a, "alice", "carol", i + 1, ep, 10_000 + i);
            let b = xfer(&sk_b, "bob", "carol", i + 1, ep, 20_000 + i);
            for (k, x) in [a, b].into_iter().enumerate() {
                let at = (ep - act) * 30_000 + rng.next() % 20_000;
                if ep >= part_from && ep < part_to { q.push((at, (i as usize + k) % 3, Msg::E(x.0.clone(), x.1.clone()))); }
                else { for to in 0..3 { q.push((at + rng.next() % 5_000, to, Msg::E(x.0.clone(), x.1.clone()))); } }
                txs.push((ep, x.0.hash()));
            }
        }
        let part = |t_ms: u64| { let e = act + t_ms / 30_000; e >= part_from && e < part_to };
        let mut roots: Vec<BTreeMap<u64, String>> = vec![BTreeMap::new(); 3];
        let mut restarted = false;
        let mut stalled_seen = false;
        let total_s = (n_ep + 160) * 30;
        for sec in 0..total_s {
            let t = sec * 1_000;
            let epoch = act + sec / 30;
            let cut = part(t);
            let mut due: Vec<(u64, usize, Msg)> = Vec::new();
            let mut rest = Vec::new();
            for x in q.drain(..) { if x.0 <= t { due.push(x) } else { rest.push(x) } }
            q = rest;
            while !due.is_empty() {
                let k = (rng.next() as usize) % due.len();
                let (_, to, m) = due.swap_remove(k);
                let c = &nodes[to];
                match m {
                    Msg::E(e, s) => { if !applied_hash(c, &e) { c.push_pending(e, s); } }
                    Msg::P(p) => { eng[to].on_prop(p, t, &verify); }
                    Msg::V(v) => { let _ = eng[to].on_vote(v, &verify); }
                    Msg::C(cert) => { eng[to].on_cert(cert, &verify); }
                }
            }
            // Data availability: bodies a voter lacks to vote are fetched from the peers, as the
            // driver does (request_hashes). Only the ones served here are taken off the list.
            if !cut && sec % 3 == 0 {
                let want = da_due(t);
                if !want.is_empty() {
                    for j in 0..3 {
                        for r in serve_bodies(&nodes[j], &want, epoch) {
                            if let Ok(e) = serde_json::from_value::<LedgerEntry>(r["entry"].clone()) {
                                offer_body(e, r["sig"].as_str().map(str::to_owned));
                            }
                        }
                    }
                }
            }
            // Gossip helper: everything between nodes is lost while partitioned.
            let mut send = |q: &mut Vec<(u64, usize, Msg)>, from: usize, m: Msg, rng: &mut Rng| {
                if cut { return; }
                for j in 0..3 { if j != from { q.push((t + 200 + rng.next() % 3_000, j, m.clone())); } }
            };
            for (i, v) in voters.iter().enumerate() {
                let s = signer(v);
                if sec % 30 == 5 {
                    let hs = due_hashes(&nodes[i], epoch);
                    if let Some(p) = eng[i].propose(v, epoch, hs, d, t, &s) { send(&mut q, i, Msg::P(p), &mut rng); }
                }
                let (new, rb) = voter_step(&mut eng[i], &nodes[i], v, epoch, t, d, sec % 10 == 0, &s);
                for o in new.into_iter().chain(rb) {
                    let m = match o { Out::P(p) => Msg::P(p), Out::V(v) => Msg::V(v), Out::C(c) => Msg::C(c) };
                    send(&mut q, i, m, &mut rng);
                }
            }
            for i in 0..3 {
                eng[i].finalize();
                // A node behind its peers asks for final certs (the "certs" request, answered
                // from the peers' engines), as the driver does when next + 3 < tip.
                if !cut && sec % 10 == 0 {
                    let next = applied_upto(&nodes[i]) + 1;
                    if eng[i].final_cert(next).is_none() && next + 3 < epoch {
                        for j in 0..3 { if j == i { continue; }
                            for e in next..next + 32 {
                                if let Some(c) = eng[j].final_cert(e).cloned() { q.push((t + 500, i, Msg::C(c))); }
                            }
                        }
                    }
                }
                loop {
                    let next = applied_upto(&nodes[i]) + 1;
                    let Some(cert) = eng[i].final_cert(next).cloned() else { break };
                    match apply_final(&nodes[i], &cert) {
                        Ok(_) => {
                            roots[i].insert(next, nodes[i].store.balance_merkle_root());
                            // Every entry of this epoch's record is dated exactly next - d.
                            let rec = epoch_sync::serve(&nodes[i], next, next);
                            for r in rec["epochs"][0]["entries"].as_array().into_iter().flatten() {
                                let e: LedgerEntry = serde_json::from_value(r["entry"].clone()).unwrap();
                                assert!(e.epoch() + d <= next && next <= e.epoch() + crate::tx::STALE_WINDOW,
                                    "node {i}: entry dated {} applied at {next}", e.epoch());
                            }
                        }
                        Err(missing) => {
                            if cut { break; }
                            let mut got = false;
                            for j in 0..3 { if j == i { continue; }
                                for r in serve_bodies(&nodes[j], &missing, epoch) {
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
            if cut && epoch == part_to - 1 && sec % 30 == 29 {
                for c in &nodes { assert!(applied_upto(c) < part_from, "nothing certifies while every voter is cut off"); }
                stalled_seen = true;
            }
            // Restart voter 1 mid catch-up.
            if !restarted && !cut && applied_upto(&nodes[1]) >= part_from + 300 {
                restarted = true;
                let me = &voters[1];
                let lf = eng[1].last_final().unwrap();
                let (props, votes) = eng[1].own_since(me, lf + 1);
                let floor = eng[1].max_voted().unwrap_or(0);
                nodes[1] = Chain::new(stores[1].clone(), "cu-1".into(), "hone-test".into());
                let (np, _) = nodes[1].restore_pools();
                let mut e = Engine::new("hone-test", act, voters.clone());
                let up = applied_upto(&nodes[1]);
                let f: serde_json::Value = serde_json::from_slice(&nodes[1].store.state_get(&format!("setcert_final:{up}")).unwrap()).unwrap();
                e.resume(up, setcert::digest("hone-test", up, f["round"].as_u64().unwrap() as u32,
                    f["content_hash"].as_str().unwrap(), f["prev"].as_str().unwrap()));
                e.set_vote_floor(floor);
                e.restore_own(props, votes);
                eng[1] = e;
                q.retain(|(_, to, _)| *to != 1); // in-flight messages to it are lost
                let _ = np;
            }
        }
        assert!(stalled_seen && restarted, "scenario ran: partition {stalled_seen}, restart {restarted}");
        let last = act + n_ep - 1 + d;
        for (i, c) in nodes.iter().enumerate() {
            assert!(applied_upto(c) >= last, "node {i}: applied only to {} of {last}", applied_upto(c));
        }
        for i in 1..3 {
            for (e, r) in roots[0].range(..=last) { if let Some(x) = roots[i].get(e) { assert_eq!(x, r, "node {i} epoch {e}"); } }
        }
        // Every voter PROPosed in time during the partition (each naming what reached it); those
        // own PROPs are re-sent after the heal and the others' bodies are fetched for the DA
        // rule, so every transfer is still applied exactly once, at its own epoch + d. (Catch-up
        // rule 4 closes empty only epochs a voter never PROPosed for in time.)
        let want: u64 = (0..n_ep).map(|i| 10_000 + i + 20_000 + i).sum();
        for (i, c) in nodes.iter().enumerate() {
            assert_eq!(c.store.get_balance("carol", hone_types::NATIVE_TOKEN), want, "node {i}: every transfer applied once");
            for (ep, h) in &txs {
                let st = c.entry_status(h);
                assert_eq!(st["status"], "committed", "node {i}: {h} ({ep}) {st}");
                // The in-memory result map does not survive the restart (node 1): there the
                // per-epoch record check above (every applied entry dated next - d) covers it.
                if let Some(at) = st["epoch"].as_u64() { assert_eq!(at, ep + d, "node {i}: applied at its own epoch"); }
            }
            assert_eq!(epoch_sync::last_committed(c), applied_upto(c), "records follow in order");
        }
        TEST_EPOCH.with(|c| c.set(None));
    }

    /// grouchly/nebra 2026-11-11: a lagging node needs a body certified far behind every
    /// peer's tip (not in any pool, out of the in-memory 128 sets and the 64-epoch recorded
    /// window). Both an applied and a rejected body must still be servable from the peer.
    #[test]
    fn a_body_certified_far_behind_the_tip_is_still_served() {
        let act = 7_000u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk = SigningKey::from_bytes(&[0x71; 32]);
        let (c, _d) = chain("deep");
        setup(&c, &sk, "dora");
        let (good, gs) = xfer(&sk, "dora", "eve", 1, act, 5);
        let (bad, bs) = xfer(&sk, "dora", "eve", 9, act, 6); // nonce gap: certified, rejected
        let mut hs = vec![good.hash(), bad.hash()]; hs.sort();
        offer_body(good.clone(), gs); offer_body(bad.clone(), bs);
        let mut prev = setcert::genesis_prev(act);
        for t in act..act + 400 {
            let content = setcert::Content { hashes: if t == act + 3 { hs.clone() } else { vec![] }, reports: vec![] };
            let cert = Cert { epoch: t, round: 0, prev: prev.clone(), content, votes: Default::default() };
            prev = cert.digest("hone-test");
            assert!(apply_final(&c, &cert).is_ok(), "epoch {t}");
        }
        let tip = act + 400;
        let served = serve_bodies(&c, &[good.hash(), bad.hash()], tip);
        let got: Vec<String> = served.iter().filter_map(|r| serde_json::from_value::<LedgerEntry>(r["entry"].clone()).ok()).map(|e| e.hash()).collect();
        assert!(got.contains(&good.hash()), "applied body 397 epochs behind the tip");
        assert!(got.contains(&bad.hash()), "rejected body 397 epochs behind the tip");
        TEST_EPOCH.with(|c| c.set(None));
    }

    /// Offline inventory of a node's pending pool from a COPY of its state store (never a live
    /// node): HONE_INV_DB=<copy of state dir> cargo test inventory_pool -- --ignored --nocapture
    #[test]
    #[ignore]
    fn inventory_pool_from_store_copy() {
        let p = std::env::var("HONE_INV_DB").expect("HONE_INV_DB");
        let st = crate::store::Store::open(std::path::Path::new(&p)).unwrap();
        let mut rows: Vec<serde_json::Value> = st.state_get("pending_pool")
            .and_then(|b| serde_json::from_slice::<Vec<serde_json::Value>>(&b).ok()).unwrap_or_default();
        st.for_each_meta_prefix("pending_pool:e:", |_, v| { if let Ok(r) = serde_json::from_slice(v) { rows.push(r); } });
        for r in rows {
            let Ok(e) = serde_json::from_value::<LedgerEntry>(r["entry"].clone()) else { continue };
            let j = serde_json::to_value(&e).unwrap();
            let who = ["signed_by", "from", "owner", "node", "reporter", "account"].iter()
                .find_map(|k| j[*k].as_str().map(str::to_owned)).unwrap_or_default();
            println!("INV {}", serde_json::json!({ "hash": e.hash(), "type": j["type"], "epoch": e.epoch(), "who": who,
                "system": crate::tx::is_system_entry(&e) }));
        }
    }

    /// 2026-11-11: restored backlog entries inflated every voter's lag report to 10, d ratcheted
    /// to 10 and every user entry was certified 10 epochs late and rejected as stale. Now:
    /// backlog first-sightings are not lag, d_report and d never exceed D_MAX (< STALE_WINDOW),
    /// and a stored d of 10 is capped at once.
    #[test]
    fn d_never_leaves_the_stale_window() {
        for _ in 0..3_000 { note_seen(100, 1_500); } // a restored pool: 1,400 epochs "late"
        assert!(d_report() <= setcert::D_MAX && d_report() <= 3, "backlog is not gossip lag: {}", d_report());
        for _ in 0..3_000 { note_seen(1_000, 1_007); } // genuinely slow gossip
        assert_eq!(d_report(), setcert::D_MAX);
        let (c, _d) = chain("dcap");
        c.store.state_set(D_KEY, b"10").unwrap();
        assert_eq!(d(&c), setcert::D_MAX);
        assert!(setcert::D_MAX < crate::tx::STALE_WINDOW);
        let mut cur = setcert::D_START;
        for _ in 0..50 { cur = setcert::next_d(cur, &[10, 10, 10]); }
        assert_eq!(cur, setcert::D_MAX, "reports of 10 still end at D_MAX");
    }

    fn applied_hash(c: &Chain, e: &LedgerEntry) -> bool {
        let v = epoch_sync::serve(c, applied_upto(c).saturating_sub(64), applied_upto(c));
        v["epochs"].as_array().into_iter().flatten().flat_map(|x| x["entries"].as_array().cloned().unwrap_or_default())
            .any(|r| r["entry"] == serde_json::to_value(e).unwrap())
    }

    #[test]
    fn a_rejected_certified_entry_can_still_be_served() {
        let act = 60u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk = SigningKey::from_bytes(&[0x54; 32]);
        let (c, _d) = chain("rej");
        setup(&c, &sk, "carol");
        // Bad nonce: certified, then rejected at apply on every node.
        let (e, s) = xfer(&sk, "carol", "dave", 7, act, 5);
        let cert = Cert { epoch: act, round: 0, prev: setcert::genesis_prev(act),
            content: setcert::Content { hashes: vec![e.hash()], reports: vec![] }, votes: Default::default() };
        offer_body(e.clone(), s);
        assert_eq!(apply_final(&c, &cert), Ok(0), "rejected at apply");
        assert!(c.pool_body(&e.hash()).is_none(), "gone from the pool");
        assert!(!FETCHED.lock().unwrap().contains_key(&e.hash()), "and from the fetch cache");
        assert!(body(&c, &e.hash()).is_some(), "still held from the kept certified set");
        let served = serve_bodies(&c, &[e.hash()], act);
        assert_eq!(served.len(), 1, "a peer missing it can still fetch it");
        TEST_EPOCH.with(|c| c.set(None));
    }

    #[test]
    fn a_voter_never_votes_for_a_body_it_does_not_hold() {
        let act = 70u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk = SigningKey::from_bytes(&[0x61; 32]);
        let (c, _d) = chain("da");
        setup(&c, &sk, "alice");
        let (e, s) = xfer(&sk, "alice", "bob", 1, act, 5);
        let mut eng = Engine::new("hone-test", act, vec!["v1".into(), "v2".into()]);
        let mut p = Prop { epoch: act, voter: "v2".into(), hashes: vec![e.hash()], d_report: 2, sig: String::new() };
        p.sig = signer("v2")(&setcert::prop_message("hone-test", &p));
        assert!(eng.on_prop(p, 0, &verify));
        let sign = signer("v1");
        let (new, _) = voter_step(&mut eng, &c, "v1", act + 2, 100_000, 2, false, &sign);
        assert!(!new.iter().any(|o| matches!(o, Out::V(_))), "no vote without the body");
        assert!(da_missing().lock().unwrap().contains(&e.hash()));
        offer_body(e.clone(), s);
        let (new, _) = voter_step(&mut eng, &c, "v1", act + 2, 200_000, 2, false, &sign);
        let v = new.iter().find_map(|o| if let Out::V(v) = o { Some(v.clone()) } else { None }).expect("votes once it holds it");
        assert!(v.content.hashes.contains(&e.hash()));
        // The voted body is kept in state: gone from memory, still there.
        FETCHED.lock().unwrap().remove(&e.hash());
        assert!(body(&c, &e.hash()).is_some());
        TEST_EPOCH.with(|c| c.set(None));
    }

    #[test]
    fn entries_too_old_to_apply_are_never_proposed() {
        let act = 80u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk = SigningKey::from_bytes(&[0x63; 32]);
        let (c, _d) = chain("stale");
        setup(&c, &sk, "alice");
        let (old, so) = xfer(&sk, "alice", "bob", 1, act - 10, 5);
        let (ok, sok) = xfer(&sk, "alice", "bob", 2, act - 3, 5);
        c.push_pending(old.clone(), so);
        c.push_pending(ok.clone(), sok);
        let due = due_hashes(&c, act);
        assert!(!due.contains(&old.hash()), "a stuck node's ancient pool is never re-proposed");
        assert!(due.contains(&ok.hash()));
        TEST_EPOCH.with(|c| c.set(None));
    }

    fn prop(v: &str, t: u64, hashes: Vec<String>) -> Prop {
        let mut p = Prop { epoch: t, voter: v.into(), hashes, d_report: 3, sig: String::new() };
        p.sig = signer(v)(&setcert::prop_message("hone-test", &p));
        p
    }

    /// Rule 1 + the swap scenario: own PROPs restored after a state swap that name bodies the
    /// node no longer holds are dropped, and the floor keeps the node from proposing anew there.
    #[test]
    fn restored_props_naming_swapped_out_bodies_are_dropped_and_nothing_halts() {
        let act = 110u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk = SigningKey::from_bytes(&[0x65; 32]);
        let (c, _d) = chain("swap");
        setup(&c, &sk, "alice");
        let (kept, sk_) = xfer(&sk, "alice", "bob", 1, act, 5);
        let (lost, _sl) = xfer(&sk, "alice", "bob", 2, act, 5);
        c.push_pending(kept.clone(), sk_);   // persisted pool: still held after the swap
        let p_ok = prop("v1", act, vec![kept.hash()]);
        let p_bad = prop("v1", act + 1, vec![lost.hash()]); // its body was in the swapped-out state
        let (ps, vs, dropped) = restorable(&c, vec![p_ok.clone(), p_bad.clone()], vec![]);
        assert_eq!((ps.len(), vs.len(), dropped), (1, 0, 1));
        let mut eng = Engine::new("hone-test", act, vec!["v1".into(), "v2".into()]);
        eng.restore_own(ps, vs);
        eng.set_prop_floor(act + 1); // sign_guard's floor covers every restored epoch
        let sign = signer("v1");
        let (new, _) = voter_step(&mut eng, &c, "v1", act + 3, 100_000, 3, false, &sign);
        assert!(!new.iter().any(|o| matches!(o, Out::P(p) if p.epoch == act + 1)), "no new, different PROP at a restored epoch");
        assert!(eng.own_prop("v1", act + 1).is_none(), "the PROP naming a lost body is never re-sent");
        TEST_EPOCH.with(|c| c.set(None));
    }

    /// Rule 2: a node behind what is already certified PROPoses only empty sets for past
    /// epochs (no stale or unheld entries), so even with every voter behind the chain keeps
    /// finalizing (no voter waits on a PROP nobody would make).
    #[test]
    fn a_node_behind_makes_no_proposals_for_past_epochs() {
        let act = 120u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk = SigningKey::from_bytes(&[0x66; 32]);
        let (c, _d) = chain("behind");
        setup(&c, &sk, "alice");
        let (e1, s1) = xfer(&sk, "alice", "bob", 1, act, 5);
        c.push_pending(e1, s1);
        let mut eng = Engine::new("hone-test", act, vec!["v1".into(), "v2".into()]);
        eng.resume(act + 10, "d".into()); // certified up to act+10, applied only act-1
        let sign = signer("v1");
        let (new, _) = voter_step(&mut eng, &c, "v1", act + 20, 100_000, 3, false, &sign);
        let past: Vec<&Prop> = new.iter().filter_map(|o| if let Out::P(p) = o { Some(p) } else { None })
            .filter(|p| p.epoch + 3 <= act + 20).collect();
        assert!(!past.is_empty(), "behind: still PROPoses past epochs (no deadlock)");
        assert!(past.iter().all(|p| p.hashes.is_empty()), "behind: only empty PROPs for past epochs");
        TEST_EPOCH.with(|c| c.set(None));
    }

    /// Rules 3 + 4: a synced node's PROP for an open past epoch older than the stale window is
    /// empty; for a recent one it names only entries still valid at T, and keeps their bodies.
    #[test]
    fn old_open_epochs_close_empty_and_catch_up_props_name_only_valid_held_entries() {
        let act = 130u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk = SigningKey::from_bytes(&[0x67; 32]);
        let (c, _d) = chain("horizon");
        setup(&c, &sk, "alice");
        let (e1, s1) = xfer(&sk, "alice", "bob", 1, act, 5);
        c.push_pending(e1.clone(), s1);
        let mut eng = Engine::new("hone-test", act, vec!["v1".into(), "v2".into()]);
        let sign = signer("v1");
        let cur = act + 20; // synced (nothing certified yet), epochs act..act+18 open
        let (new, _) = voter_step(&mut eng, &c, "v1", cur, 100_000, 3, false, &sign);
        let props: Vec<Prop> = new.iter().filter_map(|o| if let Out::P(p) = o { Some(p.clone()) } else { None }).collect();
        assert!(!props.is_empty());
        for p in &props {
            if p.epoch + crate::tx::STALE_WINDOW < cur { assert!(p.hashes.is_empty(), "epoch {} closes empty", p.epoch); }
            for h in &p.hashes { assert!(held(&c, h)); }
        }
        TEST_EPOCH.with(|c| c.set(None));
    }

    /// Today's halt end to end (2026-11-15, epoch 384535). A stuck voter on the old rule
    /// re-proposes an ancient entry from its own pool, then loses that pool in a state swap.
    /// Old rule: the other voter votes the union anyway, a cert forms, and every node halts on
    /// a body nobody holds. New rule: the other voter refuses to vote without the body, so no
    /// cert can form on it; and a voter on the new rule never proposes the ancient entry.
    #[test]
    fn todays_halt_end_to_end_stale_reproposal_is_refused() {
        let act = 1_000u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk = SigningKey::from_bytes(&[0x64; 32]);
        let (c, _d) = chain("e2e");
        setup(&c, &sk, "alice");
        // The ancient entry, dated far before T: only in the stuck voter's pool.
        let (ancient, sa_) = xfer(&sk, "alice", "bob", 1, act - 900, 5);
        // 1) The new rule never proposes it, even when it sits in a pool.
        c.push_pending(ancient.clone(), sa_.clone());
        assert!(!due_hashes(&c, act + 2).contains(&ancient.hash()));
        c.pending.lock().clear();
        c.pool_forget(&[ancient.hash()]);
        // 2) The stuck voter (old rule) proposes it anyway; its body is gone (state swap).
        let mut eng = Engine::new("hone-test", act, vec!["v1".into(), "v2".into()]);
        let mut p = Prop { epoch: act, voter: "v2".into(), hashes: vec![ancient.hash()], d_report: 10, sig: String::new() };
        p.sig = signer("v2")(&setcert::prop_message("hone-test", &p));
        assert!(eng.on_prop(p, 0, &verify));
        // v2 votes the union (old rule); its vote alone is not a quorum.
        let mut v2 = setcert::Vote { epoch: act, round: 0, voter: "v2".into(), prev: setcert::genesis_prev(act),
            content: setcert::Content { hashes: vec![ancient.hash()], reports: vec![("v2".into(), 10)] }, sig: String::new() };
        v2.sig = signer("v2")(&v2.digest("hone-test"));
        assert!(eng.on_vote(v2, &verify).is_none());
        // 3) v1 (new rule) does not hold the body: it refuses to vote and asks for it.
        let sign = signer("v1");
        for now in [100_000u64, 200_000, 300_000] {
            let (new, _) = voter_step(&mut eng, &c, "v1", act + 2, now, 3, false, &sign);
            assert!(!new.iter().any(|o| matches!(o, Out::V(_))), "data availability: no vote without the body");
        }
        assert!(da_missing().lock().unwrap().contains(&ancient.hash()));
        // 4) So no certificate exists for T, and nothing can halt on that body.
        assert!(eng.final_cert(act).is_none());
        assert!(eng.chosen(act).is_none());
        TEST_EPOCH.with(|c| c.set(None));
    }

    /// The 384535 halt: a final cert names a body no node holds. A voter quorum signs the
    /// unavailability certificate; every node then applies the set without it, identically.
    #[test]
    fn a_quorum_unavailability_certificate_unhalts_every_node_identically() {
        let act = 90u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk = SigningKey::from_bytes(&[0x62; 32]);
        let (c1, _d1) = chain("ua1");
        let (c2, _d2) = chain("ua2");
        for c in [&c1, &c2] { setup(c, &sk, "alice"); }
        let (a, sa_) = xfer(&sk, "alice", "bob", 1, act, 5);
        let (b, _sb) = xfer(&sk, "alice", "carol", 2, act, 7); // its body is lost everywhere
        let mut hs = vec![a.hash(), b.hash()]; hs.sort();
        let cert = Cert { epoch: act, round: 0, prev: setcert::genesis_prev(act),
            content: setcert::Content { hashes: hs, reports: vec![] }, votes: Default::default() };
        offer_body(a.clone(), sa_.clone());
        assert_eq!(apply_final(&c1, &cert), Err(vec![b.hash()]), "halted: a body nobody holds");
        let eng = Engine::new("hone-test", act, vec!["v1".into(), "v2".into(), "v3".into()]);
        let ch = cert.content.hash();
        let mk = |v: &str| {
            let m = setcert::unavail_message("hone-test", act, &ch, &[b.hash()]);
            setcert::Unavail { epoch: act, content_hash: ch.clone(), hashes: vec![b.hash()], sigs: [(v.to_string(), signer(v)(&m))].into() }
        };
        on_unavail(&c1, &eng, mk("v1"), &verify);
        assert!(unavail_for(&c1, act).is_none(), "one signer is not a quorum");
        let mut forged = mk("v2"); forged.sigs.insert("v2".into(), "forged".into());
        on_unavail(&c1, &eng, forged, &verify);
        assert!(unavail_for(&c1, act).is_none(), "a forged signature does not count");
        let mut other = mk("v2"); other.content_hash = "another-set".into();
        on_unavail(&c1, &eng, other, &verify);
        assert!(unavail_for(&c1, act).is_none(), "a statement about another set does not count");
        on_unavail(&c1, &eng, mk("v2"), &verify);
        let u = unavail_for(&c1, act).expect("quorum of 3 voters: 2");
        // The certificate reaches the other node over gossip.
        on_unavail(&c2, &eng, u, &verify);
        assert!(unavail_for(&c2, act).is_some());
        assert_eq!(apply_final(&c1, &cert), Ok(1));
        offer_body(a.clone(), sa_.clone());
        assert_eq!(apply_final(&c2, &cert), Ok(1));
        for c in [&c1, &c2] {
            assert_eq!(applied_upto(c), act);
            assert_eq!(c.store.get_balance("bob", hone_types::NATIVE_TOKEN), 5);
            assert_eq!(c.store.get_balance("carol", hone_types::NATIVE_TOKEN), 0, "left out, its submitter can resubmit");
        }
        assert_eq!(c1.store.balance_merkle_root(), c2.store.balance_merkle_root());
        TEST_EPOCH.with(|c| c.set(None));
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
    /// beastly 2026-11-15 (7d56432b): restoring 1,052 own PROPs against a 100k+ pool hashed the
    /// whole pool per body lookup and held a tokio worker at 100% for minutes (API dead). With
    /// keyed lookups, 1,100 PROPs x 20 hashes against a 30,000-entry pool restore in seconds,
    /// on a blocking thread, while the API (entry status) keeps answering.
    #[test]
    fn restoring_many_own_props_against_a_large_pool_is_fast_and_never_blocks_the_api() {
        let act = 400u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let (c, _d) = chain("restore-big");
        let c = std::sync::Arc::new(c);
        let mut pool_hashes = Vec::new();
        for i in 0..30_000u64 {
            let from = format!("u{}", i % 97);
            let e = LedgerEntry::Transfer { from: from.clone(), to: "bob".into(), amount: 1 + i,
                token: hone_types::NATIVE_TOKEN.into(), memo: None, nonce: i, epoch: act, signed_by: from, twofactor: None };
            pool_hashes.push(e.hash());
            c.push_pending(e, None);
        }
        let mut props = Vec::new();
        for k in 0..1_100u64 {
            let mut hashes: Vec<String> = (0..20u64).map(|j| pool_hashes[((k * 20 + j) * 7 % 30_000) as usize].clone()).collect();
            if k % 10 == 0 { hashes.push(format!("{:064x}", k + 1)); } // a body lost in a swap
            hashes.sort(); hashes.dedup();
            props.push(Prop { epoch: act + k, voter: "v1".into(), hashes, d_report: 3, sig: String::new() });
        }
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let ph = pool_hashes.clone();
        let (kept, dropped, api_max, took) = rt.block_on(async move {
            let t0 = std::time::Instant::now();
            let c2 = c.clone();
            let job = tokio::task::spawn_blocking(move || restorable(&c2, props, Vec::new()));
            // The API keeps answering while the restore runs.
            let mut api_max = std::time::Duration::ZERO;
            while !job.is_finished() && t0.elapsed() < std::time::Duration::from_secs(30) {
                let q = std::time::Instant::now();
                let st = tokio::time::timeout(std::time::Duration::from_secs(2),
                    async { c.entry_status(&ph[123]) }).await.expect("API answers");
                assert_eq!(st["status"], "pending");
                api_max = api_max.max(q.elapsed());
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            let (p, _v, dropped) = job.await.unwrap();
            let took = t0.elapsed();
            // A voter's body lookups over a large pool stay cheap as well.
            for h in ph.iter().take(2_000) { assert!(body(&c, h).is_some()); }
            (p.len(), dropped, api_max, took)
        });
        assert!(took < std::time::Duration::from_secs(5), "restore took {took:?}");
        assert_eq!(dropped, 110, "PROPs naming a lost body are dropped");
        assert_eq!(kept, 990);
        assert!(api_max < std::time::Duration::from_millis(500), "API answered in {api_max:?}");
        TEST_EPOCH.with(|c| c.set(None));
    }

    /// Rule 2 with every voter behind: all hold final certs they cannot apply yet. Each still
    /// PROPoses (empty) for the open past epochs, so the next epoch certifies (no deadlock).
    #[test]
    fn every_voter_behind_still_finalizes_the_next_epoch() {
        let act = 140u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let voters: Vec<String> = vec!["v1".into(), "v2".into(), "v3".into()];
        let chains: Vec<(Chain, tempfile::TempDir)> = (0..3).map(|i| chain(&format!("allbehind{i}"))).collect();
        let mut eng: Vec<Engine> = (0..3).map(|_| { let mut e = Engine::new("hone-test", act, voters.clone()); e.resume(act + 10, "d".into()); e }).collect();
        let cur = act + 20; // certified to act+10, applied to act-1 everywhere: all behind
        let mut msgs: Vec<Out> = Vec::new();
        for now in [100_000u64, 200_000, 300_000, 400_000, 500_000, 600_000] {
            for i in 0..3 {
                let s = signer(&voters[i]);
                let (new, rb) = voter_step(&mut eng[i], &chains[i].0, &voters[i], cur, now, 3, true, &s);
                msgs.extend(new.into_iter().chain(rb));
            }
            while let Some(m) = msgs.pop() {
                for e in eng.iter_mut() {
                    match &m {
                        Out::P(p) => { e.on_prop(p.clone(), now, &verify); }
                        Out::V(v) => { if let Some(c) = e.on_vote(v.clone(), &verify) { msgs.insert(0, Out::C(c)); } }
                        Out::C(c) => { e.on_cert(c.clone(), &verify); }
                    }
                }
            }
            for e in eng.iter_mut() { e.finalize(); }
        }
        for e in &eng {
            let f = e.final_cert(act + 11).expect("the next epoch certifies with every voter behind");
            assert!(f.content.hashes.is_empty());
        }
        TEST_EPOCH.with(|c| c.set(None));
    }
    /// nebra profile (Pi, 905 MB): steady state with a 120,000-entry pool on disk and 1,000
    /// restored own PROPs stays under 300 MB RSS, and the API answers throughout.
    /// Run: HONE_PROFILE=light cargo test lean_steady_state -- --ignored --test-threads 1
    #[test]
    #[ignore]
    fn lean_steady_state_rss_under_300mb_with_big_pool_and_restored_props() {
        let prof = crate::mem_profile::get();
        assert!(prof.light, "run with HONE_PROFILE=light");
        let act = 500u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let d = tempfile::tempdir().unwrap();
        let st = crate::store::Store::open(d.path()).unwrap();
        let mut hashes = Vec::new();
        {
            let c = Chain::new(st.clone(), "lean".into(), "hone-test".into());
            for i in 0..120_000u64 {
                let from = format!("u{}", i % 997);
                let e = LedgerEntry::Transfer { from: from.clone(), to: "bob".into(), amount: 1 + i,
                    token: hone_types::NATIVE_TOKEN.into(), memo: Some(format!("m{i:032}")), nonce: i,
                    epoch: act + 900 + (i % 40), signed_by: from, twofactor: None };
                hashes.push(e.hash());
                c.push_pending(e, Some(format!("{:0128x}", i)));
            }
            assert!(c.pending_count() <= prof.pool_mem_max, "in-memory pool bounded");
        }
        crate::mem_profile::trim_now();
        // Restart: stream the pool back, restore 1,000 own PROPs, run voter ticks.
        let c = std::sync::Arc::new(Chain::new(st.clone(), "lean".into(), "hone-test".into()));
        let (np, _) = c.restore_pools();
        assert_eq!(np, 120_000);
        assert!(c.pending_count() <= prof.pool_mem_max);
        let props: Vec<Prop> = (0..1_000u64).map(|k| Prop { epoch: act + 10 + k, voter: "v1".into(),
            hashes: { let mut h: Vec<String> = (0..20u64).map(|j| hashes[((k * 20 + j) * 13 % 120_000) as usize].clone()).collect(); h.sort(); h.dedup(); h },
            d_report: 3, sig: String::new() }).collect();
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let probe = hashes[77].clone();
        let (api_max, peak) = rt.block_on(async move {
            let c2 = c.clone();
            let job = tokio::task::spawn_blocking(move || {
                let (ps, vs, _) = restorable(&c2, props, Vec::new());
                let mut eng = Engine::new("hone-test", act, vec!["v1".into(), "v2".into()]);
                eng.restore_own(ps, vs);
                let s = signer("v1");
                for k in 0..30u64 { let _ = voter_step(&mut eng, &c2, "v1", act + 1_020, 1_000_000 + k * 1_000, 3, k % 10 == 0, &s); }
                eng
            });
            let mut api_max = std::time::Duration::ZERO;
            let mut peak = 0u64;
            while !job.is_finished() {
                let q = std::time::Instant::now();
                let st = c.entry_status(&probe);
                assert_eq!(st["status"], "pending");
                api_max = api_max.max(q.elapsed());
                peak = peak.max(crate::mem_profile::rss_kb().unwrap_or(0));
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            let _eng = job.await.unwrap();
            crate::mem_profile::trim_now();
            peak = peak.max(crate::mem_profile::rss_kb().unwrap_or(0));
            (api_max, peak)
        });
        eprintln!("lean steady state: peak RSS {} MB, slowest API answer {:?}", peak / 1024, api_max);
        assert!(peak < 300 * 1024, "peak RSS {} MB", peak / 1024);
        assert!(api_max < std::time::Duration::from_millis(500), "API answered in {api_max:?}");
        TEST_EPOCH.with(|c| c.set(None));
    }

    /// The pool stays bounded by the stale window, not by the backlog: applying T drops what
    /// can never be applied after it, from memory and from disk.
    #[test]
    fn applying_drops_pool_entries_too_old_to_ever_apply() {
        let act = 600u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let sk = SigningKey::from_bytes(&[0x68; 32]);
        let (c, _d) = chain("stale-prune");
        setup(&c, &sk, "alice");
        let (old, so) = xfer(&sk, "alice", "bob", 1, act - 50, 5);
        let (new, sn) = xfer(&sk, "alice", "bob", 2, act + 1, 5);
        c.push_pending(old.clone(), so);
        c.push_pending(new.clone(), sn);
        let cert = Cert { epoch: act, round: 0, prev: setcert::genesis_prev(act),
            content: setcert::Content { hashes: vec![], reports: vec![] }, votes: Default::default() };
        assert_eq!(apply_final(&c, &cert), Ok(0));
        assert!(c.pool_body(&old.hash()).is_none(), "too old to apply: gone from disk");
        assert!(!c.pending.lock().iter().any(|(e, _)| e.hash() == old.hash()), "and from memory");
        assert!(c.pool_body(&new.hash()).is_some(), "still applicable: kept");
        assert_eq!(c.pool_prune_stale(act), 0);
        TEST_EPOCH.with(|c| c.set(None));
    }
    /// beastly 2026-11-15 (6adcdea0): restored own PROPs/votes naming bodies no longer held
    /// were dropped, so with its floors covering 385572.. it could never PROPose or vote
    /// there again. They are now kept as state (not re-sent): the voter votes a later round
    /// of the epoch once it holds the union's bodies, and the epoch certifies.
    #[test]
    fn a_restarted_voter_rejoins_epochs_it_voted_in_before_losing_their_bodies() {
        let act = 700u64;
        TEST_EPOCH.with(|c| c.set(Some(act)));
        let voters: Vec<String> = vec!["v1".into(), "v2".into()];
        let (c, _d) = chain("rejoin");
        let sk = SigningKey::from_bytes(&[0x69; 32]);
        let (le, ls) = xfer(&sk, "alice", "bob", 1, act, 5);
        let lost = le.hash();
        // Before the restart: v1 PROPosed and voted round 0 of `act` naming a body since lost.
        let mut old = Engine::new("hone-test", act, voters.clone());
        let s1 = signer("v1");
        let p1 = old.propose("v1", act, vec![lost.clone()], 3, 0, &s1).unwrap();
        let mut p2 = Prop { epoch: act, voter: "v2".into(), hashes: vec![], d_report: 3, sig: String::new() };
        p2.sig = signer("v2")(&setcert::prop_message("hone-test", &p2));
        assert!(old.on_prop(p2.clone(), 0, &verify));
        let v1 = old.maybe_vote("v1", act, 0, &s1).expect("voted before the restart");
        // Restart with floors covering `act` (as sign_guard sets them).
        let mut e = Engine::new("hone-test", act, voters.clone());
        e.set_prop_floor(act + 50);
        e.set_vote_floor(act + 1);
        assert_eq!(restore_into(&mut e, &c, vec![p1.clone()], vec![v1.clone()]), 2);
        assert!(e.own_prop("v1", act).is_some(), "kept as state");
        assert!(e.on_prop(p2, 1_000, &verify));
        // Nothing naming the lost body is re-sent.
        let (_, rb) = voter_step(&mut e, &c, "v1", act + 10, 100_000, 3, true, &s1);
        assert!(!rb.iter().any(|o| matches!(o, Out::P(p) if p.epoch == act)));
        assert!(!rb.iter().any(|o| matches!(o, Out::V(v) if v.epoch == act)));
        // Not held: no vote (DA). Once the body is served by a peer, v1 votes a later round.
        let mut v2 = setcert::Vote { epoch: act, round: 1, voter: "v2".into(), prev: setcert::genesis_prev(act),
            content: setcert::Content { hashes: vec![lost.clone()], reports: vec![("v1".into(), 3), ("v2".into(), 3)] }, sig: String::new() };
        v2.sig = signer("v2")(&v2.digest("hone-test"));
        let _ = e.on_vote(v2, &verify);
        let (new, _) = voter_step(&mut e, &c, "v1", act + 10, 200_000, 3, false, &s1);
        assert!(!new.iter().any(|o| matches!(o, Out::V(_))), "no vote without the body");
        offer_body(le, ls); // a peer serves it (DA fetch)
        let (new, _) = voter_step(&mut e, &c, "v1", act + 10, 300_000, 3, false, &s1);
        let v = new.iter().find_map(|o| if let Out::V(v) = o { Some(v.clone()) } else { None })
            .expect("votes a later round of an epoch it voted in before the restart");
        assert!(v.round >= 1 && v.content.hashes.contains(&lost));
        // It joined the peer's round with the same content: with both votes the epoch certifies.
        let mut w = setcert::Vote { voter: "v2".into(), sig: String::new(), ..v.clone() };
        w.sig = signer("v2")(&w.digest("hone-test"));
        let _ = e.on_vote(v, &verify);
        let _ = e.on_vote(w, &verify);
        let c = e.chosen(act).expect("the epoch certifies");
        assert!(c.votes.contains_key("v1") && c.votes.contains_key("v2"));
        TEST_EPOCH.with(|c| c.set(None));
    }
}
