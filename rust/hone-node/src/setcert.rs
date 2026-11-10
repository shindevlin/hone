//! Decide-by certified entry sets (design 2026-11-09, Josh approved build order 15:40).
//!
//! Removes the commit-timing drift class: today each node commits its OWN pending pool at
//! each seal, so an entry heard a moment late lands one epoch later on that node and every
//! reward that reads it differs. From SET_CERT_EPOCH an epoch's entries are decided by the
//! reward voters (full-node clocks) and every node applies exactly the decided set:
//!
//! 1. PROP: at its quorum seal of epoch T a voter gossips the sorted hashes of the valid
//!    entries it holds that are due (e.epoch + d <= T) and not yet decided, plus d_report
//!    (its measured p99 propagation, in epochs).
//! 2. VOTE: once it holds every voter's PROP for T (or PROP_WAIT_MS passed with a quorum of
//!    them) a voter signs digest(T, round, H(content), prev): content = union of the held
//!    PROP hash lists minus hashes already decided on its chain, plus the PROP d_reports;
//!    prev = digest of the cert it builds on for T-1. Its prev for T is locked at its first
//!    vote. If no cert forms within RETRY_MS it re-votes the next round with the grown
//!    union (unions only grow, so honest voters converge).
//! 3. CERT: a quorum of votes on one digest (same rule as reward finalization: strict
//!    majority of voters, at least MIN_OPERATORS distinct operators).
//! 4. FINAL: cert(T) is final once a cert for T+1 names it as prev. Voters lock prev, so a
//!    quorum can only ever build on one cert per epoch and finality is unique.
//!
//! Nodes apply final epochs strictly in order, only with every body in hand (fetched by
//! hash over gossip when missing). Nothing is guessed: a node that lacks a cert or a body
//! waits. Time-based rules (PROP_WAIT_MS, RETRY_MS) affect liveness only, never which
//! state a node ends in.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PROP_WAIT_MS: u64 = 6_000;
pub const RETRY_MS: u64 = 8_000;
pub const RELOCK_MS: u64 = 30_000;
pub const MIN_OPERATORS: usize = 2;
pub const MAX_SET: usize = 2_000;
pub const D_MIN: u64 = 2;
pub const D_MAX: u64 = 10;
pub const D_START: u64 = 3;
pub const D_PERIOD: u64 = 120;
pub const D_DELAY: u64 = 60;

fn sha(s: &[u8]) -> String { hex::encode(Sha256::digest(s)) }

pub fn operator(id: &str) -> &str { id.split_once('@').map(|(o, _)| o).unwrap_or(id) }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Prop { pub epoch: u64, pub voter: String, pub hashes: Vec<String>, pub d_report: u64, pub sig: String }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Content {
    /// Sorted, deduplicated entry hashes decided for the epoch.
    pub hashes: Vec<String>,
    /// (voter, d_report) of the PROPs the union was built from, sorted by voter.
    pub reports: Vec<(String, u64)>,
}
impl Content {
    pub fn hash(&self) -> String { sha(&serde_json::to_vec(self).unwrap_or_default()) }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Vote { pub epoch: u64, pub round: u32, pub voter: String, pub prev: String, pub content: Content, pub sig: String }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cert { pub epoch: u64, pub round: u32, pub prev: String, pub content: Content, pub votes: BTreeMap<String, String> }

pub fn digest(chain: &str, epoch: u64, round: u32, content_hash: &str, prev: &str) -> String {
    sha(format!("hone-setcert:{chain}:{epoch}:{round}:{content_hash}:{prev}").as_bytes())
}
pub fn prop_message(chain: &str, p: &Prop) -> String {
    let h = sha(&serde_json::to_vec(&(&p.hashes, p.d_report)).unwrap_or_default());
    format!("hone-setprop:{chain}:{}:{}:{h}", p.epoch, p.voter)
}
impl Vote { pub fn digest(&self, chain: &str) -> String { digest(chain, self.epoch, self.round, &self.content.hash(), &self.prev) } }
impl Cert { pub fn digest(&self, chain: &str) -> String { digest(chain, self.epoch, self.round, &self.content.hash(), &self.prev) } }

/// The prev of the first certified epoch.
pub fn genesis_prev(activation: u64) -> String { format!("genesis:{activation}") }

/// Deterministic d schedule: median of the reports in the period, clamped, at most ±1 step.
pub fn next_d(cur: u64, reports: &[u64]) -> u64 {
    if reports.is_empty() { return cur; }
    let mut r = reports.to_vec();
    r.sort_unstable();
    let med = r[(r.len() - 1) / 2].clamp(D_MIN, D_MAX);
    if med > cur { cur + 1 } else if med < cur { cur - 1 } else { cur }
}

pub type Verify<'a> = &'a dyn Fn(&str, &str, &str) -> bool; // (voter, message, sig)
pub type Sign<'a> = &'a dyn Fn(&str) -> String;
/// Own votes are signed locally; no need to re-verify them.
fn accept_own(_: &str, _: &str, _: &str) -> bool { true }

#[derive(Default)]
struct VoteState { round: u32, prev: String, content: Content, at_ms: u64, prev_locked_at: u64 }

pub struct Engine {
    pub chain: String,
    pub activation: u64,
    voters: Vec<String>,
    props: BTreeMap<u64, BTreeMap<String, Prop>>,
    prop_first_ms: BTreeMap<u64, u64>,
    tallies: BTreeMap<String, (Vote, BTreeMap<String, String>)>,
    certs: BTreeMap<u64, BTreeMap<String, Cert>>,
    final_: BTreeMap<u64, Cert>,
    mine: BTreeMap<u64, VoteState>,
}

impl Engine {
    pub fn new(chain: &str, activation: u64, mut voters: Vec<String>) -> Self {
        voters.sort(); voters.dedup();
        Engine { chain: chain.into(), activation, voters, props: Default::default(), prop_first_ms: Default::default(),
            tallies: Default::default(), certs: Default::default(), final_: Default::default(), mine: Default::default() }
    }
    pub fn voters(&self) -> &[String] { &self.voters }
    pub fn set_voters(&mut self, mut v: Vec<String>) { v.sort(); v.dedup(); self.voters = v; }
    pub fn quorum(&self) -> usize { ((self.voters.len() * 51 + 99) / 100).max(2).min(self.voters.len().max(1)) }
    fn is_voter(&self, v: &str) -> bool { self.voters.iter().any(|x| x == v) }
    fn quorum_ok(&self, signers: &BTreeMap<String, String>) -> bool {
        let n = signers.keys().filter(|v| self.is_voter(v)).count();
        let ops: BTreeSet<&str> = signers.keys().filter(|v| self.is_voter(v)).map(|v| operator(v)).collect();
        n >= self.quorum() && ops.len() >= MIN_OPERATORS.min(self.voters.len())
    }

    pub fn final_cert(&self, epoch: u64) -> Option<&Cert> { self.final_.get(&epoch) }
    pub fn last_final(&self) -> Option<u64> { self.final_.keys().next_back().copied() }
    pub fn certs_at(&self, epoch: u64) -> usize { self.certs.get(&epoch).map(|m| m.len()).unwrap_or(0) }

    /// Hashes already decided on the chain this node builds on (final certs, plus the
    /// chosen cert of the newest non-final epochs back to the last final one).
    fn decided(&self, upto: u64) -> BTreeSet<String> {
        let mut s = BTreeSet::new();
        for c in self.final_.values() { s.extend(c.content.hashes.iter().cloned()); }
        let from = self.last_final().map(|e| e + 1).unwrap_or(self.activation);
        for e in from..upto { if let Some(c) = self.chosen(e) { s.extend(c.content.hashes.iter().cloned()); } }
        s
    }

    /// The cert of `epoch` this node builds on: the final one, else the lowest (round, digest).
    pub fn chosen(&self, epoch: u64) -> Option<&Cert> {
        if let Some(c) = self.final_.get(&epoch) { return Some(c); }
        self.certs.get(&epoch)?.values().min_by_key(|c| (c.round, c.digest(&self.chain)))
    }
    fn prev_for(&self, epoch: u64) -> Option<String> {
        if epoch == self.activation { return Some(genesis_prev(self.activation)); }
        self.chosen(epoch - 1).map(|c| c.digest(&self.chain))
    }

    pub fn propose(&mut self, me: &str, epoch: u64, mut hashes: Vec<String>, d_report: u64, now_ms: u64, sign: Sign) -> Option<Prop> {
        if !self.is_voter(me) || epoch < self.activation { return None; }
        let decided = self.decided(epoch);
        hashes.retain(|h| !decided.contains(h));
        hashes.sort(); hashes.dedup(); hashes.truncate(MAX_SET);
        let mut p = Prop { epoch, voter: me.into(), hashes, d_report, sig: String::new() };
        p.sig = sign(&prop_message(&self.chain, &p));
        self.prop_first_ms.entry(epoch).or_insert(now_ms);
        self.props.entry(epoch).or_default().insert(me.into(), p.clone());
        Some(p)
    }

    pub fn on_prop(&mut self, p: Prop, now_ms: u64, verify: Verify) -> bool {
        if p.epoch < self.activation || !self.is_voter(&p.voter) || p.hashes.len() > MAX_SET { return false; }
        if !verify(&p.voter, &prop_message(&self.chain, &p), &p.sig) { return false; }
        let m = self.props.entry(p.epoch).or_default();
        if m.contains_key(&p.voter) { return false; }
        self.prop_first_ms.entry(p.epoch).or_insert(now_ms);
        m.insert(p.voter.clone(), p);
        true
    }

    /// Vote (or re-vote a later round) for `epoch` when the rules allow it.
    pub fn maybe_vote(&mut self, me: &str, epoch: u64, now_ms: u64, sign: Sign) -> Option<Vote> {
        if !self.is_voter(me) || epoch < self.activation || self.final_.contains_key(&epoch) { return None; }
        let props = self.props.get(&epoch)?;
        let all = self.voters.iter().all(|v| props.contains_key(v));
        let waited = now_ms.saturating_sub(*self.prop_first_ms.get(&epoch).unwrap_or(&now_ms)) >= PROP_WAIT_MS;
        if !(all || (waited && props.len() >= self.quorum())) { return None; }
        let decided = self.decided(epoch);
        let mut hs: BTreeSet<String> = props.values().flat_map(|p| p.hashes.iter().cloned()).collect();
        // Unions only grow: fold in what other voters already voted for this epoch.
        for (v, _) in self.tallies.values().filter(|(v, _)| v.epoch == epoch) { hs.extend(v.content.hashes.iter().cloned()); }
        let hashes: Vec<String> = hs.into_iter().filter(|h| !decided.contains(h)).take(MAX_SET).collect();
        let reports: Vec<(String, u64)> = props.values().map(|p| (p.voter.clone(), p.d_report)).collect();
        let content = Content { hashes, reports };
        let have_cert = self.certs_at(epoch) > 0;
        let (round, prev) = match self.mine.get(&epoch) {
            None => (0, self.prev_for(epoch)?),
            Some(st) => {
                if have_cert || now_ms.saturating_sub(st.at_ms) < RETRY_MS { return None; }
                // Prev stays locked; relock only to a strictly lower competing cert of T-1
                // after RELOCK_MS without progress (deadlock escape, see module docs).
                let mut prev = st.prev.clone();
                if let Some(p) = self.prev_for(epoch) {
                    if p != prev && now_ms.saturating_sub(st.prev_locked_at) >= RELOCK_MS { prev = p; }
                }
                if content == st.content && prev == st.prev { return None; }
                (st.round + 1, prev)
            }
        };
        let locked_at = match self.mine.get(&epoch) { Some(st) if st.prev == prev => st.prev_locked_at, _ => now_ms };
        let mut v = Vote { epoch, round, voter: me.into(), prev: prev.clone(), content: content.clone(), sig: String::new() };
        v.sig = sign(&v.digest(&self.chain));
        self.mine.insert(epoch, VoteState { round, prev, content, at_ms: now_ms, prev_locked_at: locked_at });
        let _ = self.on_vote(v.clone(), &accept_own);
        Some(v)
    }

    /// Tally a vote; returns a cert when this vote completes a quorum.
    pub fn on_vote(&mut self, v: Vote, verify: Verify) -> Option<Cert> {
        if v.epoch < self.activation || !self.is_voter(&v.voter) || v.content.hashes.len() > MAX_SET { return None; }
        let d = v.digest(&self.chain);
        if !verify(&v.voter, &d, &v.sig) { return None; }
        let entry = self.tallies.entry(d.clone()).or_insert_with(|| (v.clone(), BTreeMap::new()));
        entry.1.insert(v.voter.clone(), v.sig.clone());
        let (vote, sigs) = (entry.0.clone(), entry.1.clone());
        if !self.quorum_ok(&sigs) { return None; }
        let cert = Cert { epoch: vote.epoch, round: vote.round, prev: vote.prev, content: vote.content, votes: sigs };
        if self.add_cert(cert.clone()) { Some(cert) } else { None }
    }

    /// Accept a cert from gossip (every signature checked).
    pub fn on_cert(&mut self, c: Cert, verify: Verify) -> bool {
        if c.epoch < self.activation || c.content.hashes.len() > MAX_SET { return false; }
        let d = c.digest(&self.chain);
        let good: BTreeMap<String, String> = c.votes.iter()
            .filter(|(v, s)| self.is_voter(v) && verify(v, &d, s)).map(|(a, b)| (a.clone(), b.clone())).collect();
        if !self.quorum_ok(&good) { return false; }
        self.add_cert(Cert { votes: good, ..c })
    }

    fn add_cert(&mut self, c: Cert) -> bool {
        let d = c.digest(&self.chain);
        let m = self.certs.entry(c.epoch).or_default();
        if m.contains_key(&d) { return false; }
        m.insert(d, c);
        true
    }

    /// Finalize every cert that a cert of the next epoch builds on. Returns them in order.
    pub fn finalize(&mut self) -> Vec<Cert> {
        let mut out = Vec::new();
        let mut e = self.last_final().map(|x| x + 1).unwrap_or(self.activation);
        loop {
            let Some(here) = self.certs.get(&e) else { break };
            let Some(next) = self.certs.get(&(e + 1)) else { break };
            let expected_prev = if e == self.activation { Some(genesis_prev(self.activation)) }
                else { self.final_.get(&(e - 1)).map(|c| c.digest(&self.chain)) };
            let pick = here.iter()
                .filter(|(_, c)| Some(&c.prev) == expected_prev.as_ref())
                .find(|(d, _)| next.values().any(|n| &n.prev == *d))
                .map(|(_, c)| c.clone());
            let Some(c) = pick else { break };
            self.final_.insert(e, c.clone());
            out.push(c);
            e += 1;
        }
        // Bound memory: drop working state far behind the last final epoch.
        if let Some(lf) = self.last_final() {
            let keep = lf.saturating_sub(16);
            self.props.retain(|k, _| *k >= keep);
            self.prop_first_ms.retain(|k, _| *k >= keep);
            self.certs.retain(|k, _| *k >= keep);
            self.mine.retain(|k, _| *k >= keep);
            self.tallies.retain(|_, (v, _)| v.epoch >= keep);
            while self.final_.len() > 4_096 { let k = *self.final_.keys().next().unwrap(); self.final_.remove(&k); }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    // Deterministic xorshift for the harness.
    struct Rng(u64);
    impl Rng { fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 } }

    fn signer(me: &str) -> impl Fn(&str) -> String + '_ { move |m: &str| format!("{me}|{}", sha(m.as_bytes())) }
    fn verify(v: &str, m: &str, s: &str) -> bool { s == format!("{v}|{}", sha(m.as_bytes())) }

    #[derive(Clone)]
    enum Msg { P(Prop), V(Vote), C(Cert) }

    /// Voters see entries at different times and messages arrive in random order with
    /// random delays and drops (re-sent later); a non-voting observer follows by gossip.
    /// Every node must finalize the identical sequence of certs.
    fn run(seed: u64, voters: &[&str], epochs: u64, drop_pct: u64) -> Vec<Vec<(u64, String)>> {
        let act = 1_000;
        let vs: Vec<String> = voters.iter().map(|s| s.to_string()).collect();
        let n = vs.len() + 1; // + observer
        let mut eng: Vec<Engine> = (0..n).map(|_| Engine::new("hone-test", act, vs.clone())).collect();
        let mut rng = Rng(seed | 1);
        // queue of (deliver_at_ms, to, msg)
        let mut q: VecDeque<(u64, usize, Msg)> = VecDeque::new();
        let mut t = 0u64;
        let all_entries: Vec<Vec<String>> = (0..epochs).map(|e| (0..(rng.next() % 6)).map(|i| format!("e{e}-{i}")).collect()).collect();
        for step in 0..(epochs * 30 + 400) {
            t = step * 1_000;
            let epoch = act + step / 30;
            let phase = step % 30;
            // Proposals at the seal (phase 5), each voter holding a random subset of due
            // entries (late gossip): what it misses it proposes at a later epoch.
            if phase == 5 && epoch < act + epochs {
                for (i, v) in vs.iter().enumerate() {
                    let mut held = Vec::new();
                    for (k, es) in all_entries.iter().enumerate() {
                        let due = act + k as u64;
                        if due > epoch { continue; }
                        for h in es { if due + (rng.next() % 3) <= epoch { held.push(h.clone()); } }
                    }
                    let s = signer(v);
                    if let Some(p) = eng[i].propose(v, epoch, held, 3, t, &s) {
                        for j in 0..n { if j != i { q.push_back((t + rng.next() % 4_000, j, Msg::P(p.clone()))); } }
                    }
                }
            }
            // Deliver due messages in random order.
            let mut due: Vec<(u64, usize, Msg)> = Vec::new();
            let mut rest = VecDeque::new();
            while let Some(x) = q.pop_front() { if x.0 <= t { due.push(x) } else { rest.push_back(x) } }
            q = rest;
            while !due.is_empty() {
                let k = (rng.next() as usize) % due.len();
                let (_, to, m) = due.swap_remove(k);
                if rng.next() % 100 < drop_pct { q.push_back((t + 2_000 + rng.next() % 5_000, to, m)); continue; }
                match m {
                    Msg::P(p) => { eng[to].on_prop(p, t, &verify); }
                    Msg::V(v) => {
                        if let Some(c) = eng[to].on_vote(v, &verify) {
                            for j in 0..n { if j != to { q.push_back((t + rng.next() % 3_000, j, Msg::C(c.clone()))); } }
                        }
                    }
                    Msg::C(c) => { eng[to].on_cert(c, &verify); }
                }
            }
            for (i, v) in vs.iter().enumerate() {
                for e in act..=epoch {
                    let s = signer(v);
                    if let Some(vote) = eng[i].maybe_vote(v, e, t, &s) {
                        for j in 0..n { if j != i { q.push_back((t + rng.next() % 3_000, j, Msg::V(vote.clone()))); } }
                    }
                }
            }
            for e in eng.iter_mut() { e.finalize(); }
        }
        let _ = t;
        eng.iter().map(|e| e.final_.iter().map(|(k, c)| (*k, c.digest("hone-test"))).collect()).collect()
    }

    #[test]
    fn all_nodes_finalize_identical_sets_under_random_gossip() {
        for seed in [1u64, 7, 42, 99, 1234, 98765] {
            for voters in [&["josh", "natoshisakamoto", "shindevlin"][..], &["natoshisakamoto", "shindevlin"][..]] {
                let fin = run(seed, voters, 40, 15);
                assert!(fin[0].len() >= 30, "seed {seed}: liveness, only {} final", fin[0].len());
                for (i, f) in fin.iter().enumerate() {
                    let common = fin[0].len().min(f.len());
                    assert_eq!(&fin[0][..common], &f[..common], "seed {seed} node {i} diverged");
                }
            }
        }
    }

    #[test]
    fn every_entry_is_decided_exactly_once() {
        let act = 1_000;
        let vs = vec!["a".to_string(), "b".to_string()];
        let mut e = Engine::new("c", act, vs.clone());
        let (sa, sb) = (signer("a"), signer("b"));
        for ep in act..act + 3 {
            let hs = vec![format!("x{ep}"), "dup".to_string()];
            let pa = e.propose("a", ep, hs.clone(), 3, 0, &sa).unwrap();
            let _ = pa;
            let mut pb = Prop { epoch: ep, voter: "b".into(), hashes: hs.clone(), d_report: 3, sig: String::new() };
            pb.hashes.sort();
            pb.sig = sb(&prop_message("c", &pb));
            assert!(e.on_prop(pb, 0, &verify));
            let va = e.maybe_vote("a", ep, 0, &sa).unwrap();
            let mut vb = va.clone(); vb.voter = "b".into(); vb.sig = sb(&vb.digest("c"));
            assert!(e.on_vote(vb, &verify).is_some());
            e.finalize();
        }
        let all: Vec<String> = (act..act + 2).flat_map(|k| e.final_cert(k).unwrap().content.hashes.clone()).collect();
        assert_eq!(all.iter().filter(|h| *h == "dup").count(), 1, "decided once, later epochs exclude it");
    }

    #[test]
    fn one_operator_cannot_certify_and_forged_votes_fail() {
        let mut e = Engine::new("c", 10, vec!["josh".into(), "josh@aa".into(), "shin".into()]);
        let mut v = Vote { epoch: 10, round: 0, voter: "josh".into(), prev: genesis_prev(10), content: Content::default(), sig: String::new() };
        v.sig = signer("josh")(&v.digest("c"));
        assert!(e.on_vote(v.clone(), &verify).is_none());
        let mut v2 = v.clone(); v2.voter = "josh@aa".into(); v2.sig = signer("josh@aa")(&v2.digest("c"));
        assert!(e.on_vote(v2, &verify).is_none(), "two seats of one operator are not a quorum");
        let mut f = v.clone(); f.voter = "shin".into(); f.sig = "shin|bogus".into();
        assert!(e.on_vote(f, &verify).is_none());
    }

    #[test]
    fn d_schedule_is_bounded_and_slow() {
        assert_eq!(next_d(3, &[9, 9, 9]), 4);
        assert_eq!(next_d(3, &[1, 1]), 2);
        assert_eq!(next_d(2, &[0]), 2);
        assert_eq!(next_d(10, &[50]), 10);
        assert_eq!(next_d(5, &[]), 5);
        assert_eq!(next_d(4, &[2, 4, 9]), 4);
    }
}
