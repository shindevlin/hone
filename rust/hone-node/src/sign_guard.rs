//! Node-side slashing protection: sync before submit (spec 6b). LOCAL behaviour, never
//! chain state and not gated, so it is safe at any epoch.
//!
//! A node signs nothing that could equivocate (seals, setcert PROPs and votes, state-healer
//! checkpoints, decide commits; catch-up echoes and decide reveals wait for Ready too) until:
//!  1. it is synced: on start, restart, or after a gap of more than GAP_EPOCHS it is
//!     observe-only until its finalized head is within SYNC_SLACK_EPOCHS of the tip;
//!  2. its per-kind "highest signed" floor is raised to the max of its local record and what
//!     the chain shows signed by its own key (covers lost or restored state, snapshot swaps);
//!  3. a doppelganger wait of DOPPELGANGER_EPOCHS saw no signature by its own key that this
//!     run did not make (another instance with the same key): if it sees one it refuses to
//!     sign at all and logs loudly;
//! and from then on it never signs a kind at or below that kind's floor.
//! The floors are persisted in HONE_DATA_DIR/sign_floor.json, next to setcert_voted.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::OnceLock;

use parking_lot::Mutex;
use tracing::{error, info, warn};

use crate::chain::Chain;

/// A gap longer than this (epochs between two ticks) sends the node back to observe-only.
pub const GAP_EPOCHS: u64 = 3;
/// Synced: the finalized / applied head is at most this far behind the tip.
pub const SYNC_SLACK_EPOCHS: u64 = 3;
/// Doppelganger wait after syncing, in epochs.
pub const DOPPELGANGER_EPOCHS: u64 = 2;
/// How far back the chain scan for this node's own signatures looks.
pub const CHAIN_SCAN_EPOCHS: u64 = 2_880;

pub const SEAL: &str = "seal";
pub const SET_PROP: &str = "set_prop";
pub const SET_VOTE: &str = "set_vote";
pub const CHECKPOINT: &str = "checkpoint";
pub const DECIDE_COMMIT: &str = "decide_commit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Signs nothing until synced to the finalized head.
    Observe,
    /// Synced; listening for another instance with the same key until `until`.
    Doppelganger { until: u64 },
    /// Signs, never at or below a floor.
    Ready,
    /// Another instance signs with this key: never signs again in this run.
    Blocked,
}

pub struct Guard {
    pub phase: Phase,
    floors: BTreeMap<String, u64>,
    /// Floors fixed when the node became Ready (previous runs + chain + doppelganger window):
    /// the run never signs a kind at or below these. Own signatures of this run raise only
    /// the persisted floors (next restart), so catch-up can still sign older open slots.
    ready_floors: BTreeMap<String, u64>,
    /// Signatures this run made: (kind, epoch) -> sigs ("" where the signature is not seen).
    mine: HashMap<(String, u64), BTreeSet<String>>,
    /// Tip at the first tick of this run's current observe phase: own signatures for epochs
    /// at or after it that this run did not make are another instance's.
    start: Option<u64>,
    last_tick: Option<u64>,
    path: Option<PathBuf>,
}

impl Guard {
    pub fn new(path: Option<PathBuf>) -> Self {
        let floors = path.as_ref().and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<BTreeMap<String, u64>>(&b).ok()).unwrap_or_default();
        Guard { phase: Phase::Observe, floors, ready_floors: BTreeMap::new(), mine: HashMap::new(), start: None, last_tick: None, path }
    }

    /// The floor this run respects (0 until Ready; nothing is signed before Ready anyway).
    pub fn floor(&self, kind: &str) -> u64 { self.ready_floors.get(kind).copied().unwrap_or(0) }
    /// The persisted floor (all runs).
    pub fn persisted_floor(&self, kind: &str) -> u64 { self.floors.get(kind).copied().unwrap_or(0) }

    fn raise(&mut self, kind: &str, epoch: u64) -> bool {
        let f = self.floors.entry(kind.to_owned()).or_insert(0);
        if epoch > *f { *f = epoch; true } else { false }
    }

    fn persist(&self) {
        let Some(p) = &self.path else { return };
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_vec(&self.floors).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, p);
        }
    }

    /// One driver tick at tip `tip` with the node's finalized head `head` and the highest final
    /// epoch it knows exists (`known_final`). Synced: head within SYNC_SLACK_EPOCHS of the tip,
    /// or (a stalled network) it holds every final it knows of after observing for
    /// DOPPELGANGER_EPOCHS. `chain_floors`:
    /// per kind, the highest epoch the chain shows signed by this node's key (only read
    /// while not Ready, it may be empty otherwise).
    pub fn tick(&mut self, tip: u64, head: u64, known_final: u64, chain_floors: &BTreeMap<String, u64>) {
        if self.phase == Phase::Blocked { return; }
        // Never goes backwards: a caller with a lagging epoch (the decide worker passes the
        // sealed epoch) must not make the next on-time tick look like a gap (nebra 2026-11-15:
        // 9-epoch "gaps" every 30 s kept it observe-only, so it never voted).
        let tip = tip.max(self.last_tick.unwrap_or(0));
        if let Some(l) = self.last_tick {
            if tip > l + GAP_EPOCHS && self.phase != Phase::Observe {
                warn!("[sign-guard] gap of {} epochs: observe-only until synced", tip - l);
                self.phase = Phase::Observe;
                self.start = None;
            }
        }
        self.last_tick = Some(tip);
        if self.start.is_none() { self.start = Some(tip); }
        let mut changed = false;
        for (k, e) in chain_floors { changed |= self.raise(k, *e); }
        if changed { self.persist(); }
        match self.phase {
            Phase::Observe if head.saturating_add(SYNC_SLACK_EPOCHS) >= tip
                || (head >= known_final && self.start.is_some_and(|s| tip >= s + DOPPELGANGER_EPOCHS)) => {
                self.phase = Phase::Doppelganger { until: tip + DOPPELGANGER_EPOCHS };
                info!("[sign-guard] synced at {head} (tip {tip}); floors {:?}; doppelganger wait to {}",
                    self.floors, tip + DOPPELGANGER_EPOCHS);
            }
            Phase::Doppelganger { until } if tip >= until => {
                self.phase = Phase::Ready;
                self.ready_floors = self.floors.clone();
                info!("[sign-guard] no other instance seen: signing from epoch {tip}");
            }
            _ => {}
        }
    }

    /// May this node sign `kind` for `epoch` now?
    /// Ready, above the run's floor, and not already signed by this run (one per slot).
    pub fn may_sign(&self, kind: &str, epoch: u64) -> bool {
        self.phase == Phase::Ready && epoch > self.floor(kind) && !self.mine.contains_key(&(kind.to_owned(), epoch))
    }
    /// For signatures with no epoch slot (catch-up echoes, decide reveals): Ready only.
    pub fn ready(&self) -> bool { self.phase == Phase::Ready }

    /// Record a signature this node made (right after signing, or restored from its own
    /// record of a previous run).
    pub fn signed(&mut self, kind: &str, epoch: u64, sig: &str) {
        self.mine.entry((kind.to_owned(), epoch)).or_default().insert(sig.to_owned());
        if self.raise(kind, epoch) { self.persist(); }
    }

    /// A VERIFIED signature by this node's own key seen on chain or gossip. Not made by this
    /// run: raises the floor; and if it is for an epoch at or after this run came up, another
    /// instance is signing with the same key, so this node stops signing.
    pub fn observe_own(&mut self, kind: &str, epoch: u64, sig: &str) {
        if self.mine.get(&(kind.to_owned(), epoch)).is_some_and(|s| s.contains(sig)) { return; }
        if self.raise(kind, epoch) { self.persist(); }
        if self.start.is_some_and(|s| epoch > s) && self.phase != Phase::Blocked {
            self.phase = Phase::Blocked;
            error!("[sign-guard] DOPPELGANGER: a {kind} for epoch {epoch} signed with THIS node's key was not made \
                by this process. Another instance is running with the same key. Refusing to sign anything \
                until restarted with the duplicate stopped.");
        }
    }
}

static GUARD: OnceLock<Mutex<Guard>> = OnceLock::new();
static ME: OnceLock<String> = OnceLock::new();

fn floor_path() -> PathBuf {
    PathBuf::from(std::env::var("HONE_DATA_DIR").unwrap_or_else(|_| ".".into())).join("sign_floor.json")
}
pub fn guard() -> &'static Mutex<Guard> { GUARD.get_or_init(|| Mutex::new(Guard::new(Some(floor_path())))) }
/// This node's signing identity (node id / voter id).
pub fn set_me(me: &str) { let _ = ME.set(me.to_owned()); }
pub fn is_me(id: &str) -> bool { ME.get().is_some_and(|m| m == id) }
pub fn me() -> Option<&'static str> { ME.get().map(String::as_str) }
pub fn may_sign(kind: &str, epoch: u64) -> bool { guard().lock().may_sign(kind, epoch) }
pub fn ready() -> bool { guard().lock().ready() }
pub fn signed(kind: &str, epoch: u64, sig: &str) { guard().lock().signed(kind, epoch, sig) }
pub fn observe_own(kind: &str, epoch: u64, sig: &str) { guard().lock().observe_own(kind, epoch, sig) }
/// The setcert vote floor the engine must respect (feeds Engine::set_vote_floor).
pub fn floor(kind: &str) -> u64 { guard().lock().floor(kind) }

static MY_PK: OnceLock<String> = OnceLock::new();
/// This node's own verifying key (hex), to verify signatures that claim to be ours.
pub fn set_my_pubkey(pk: &str) { let _ = MY_PK.set(pk.to_owned()); }
pub fn my_pubkey() -> Option<&'static str> { MY_PK.get().map(String::as_str) }

/// A clock seal from gossip: if it claims to be ours and verifies under OUR key, observe it.
pub fn observe_seal(seal: &serde_json::Value) {
    let (Some(n), Some(e), Some(h), Some(ts), Some(sig)) = (seal["node_id"].as_str(), seal["epoch_number"].as_u64(),
        seal["seal_hash"].as_str(), seal["timestamp"].as_u64(), seal["signature"].as_str()) else { return };
    if !is_me(n) { return; }
    let Some(pk) = my_pubkey() else { return };
    if crate::clock::verify_clock_seal_sig(e, h, n, ts, pk, sig) { observe_own(SEAL, e, sig); }
}

/// (finalized head, highest final known): certified sets when setcert is active, else the
/// last committed epoch.
pub fn heads(chain: &Chain, tip: u64) -> (u64, u64) {
    if crate::setcert_apply::active(tip) {
        // The finalized head is the last final cert this node holds, applied or not: an apply
        // halt (missing bodies, 2026-11-15) must not stop a synced voter from voting, or the
        // apply halt becomes a finalization halt.
        let h = crate::setcert_apply::applied_upto(chain);
        let f = crate::setcert_node::engine(chain).lock().last_final().unwrap_or(0);
        (h.max(f), h.max(f))
    } else {
        let h = crate::epoch_sync::last_committed(chain);
        (h, h)
    }
}

/// The node tick (seal loop, setcert driver, decide worker): reads the chain only while
/// not Ready. Never call it while holding the setcert engine lock.
pub fn tick(chain: &Chain, me: &str, _caller_epoch: u64) {
    set_me(me);
    // One clock for every caller: the chain's current epoch.
    let tip = chain.current_epoch();
    let (head, known) = heads(chain, tip);
    let need = !guard().lock().ready();
    let floors = if need { chain_floors(chain, me, tip) } else { BTreeMap::new() };
    guard().lock().tick(tip, head, known, &floors);
}

/// Per kind, the highest epoch the chain shows signed by `me` (within CHAIN_SCAN_EPOCHS):
/// setcert votes and PROPs from stored certificates (where kept) and the engine's final
/// certs, decide commits from state.
pub fn chain_floors(chain: &Chain, me: &str, tip: u64) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    let from = tip.saturating_sub(CHAIN_SCAN_EPOCHS);
    for (_, v) in chain.store.state_scan_prefix("setcert_cert:") {
        let Ok(c) = serde_json::from_slice::<crate::setcert::Cert>(&v) else { continue };
        if c.epoch < from { continue; }
        if c.votes.contains_key(me) { let f = out.entry(SET_VOTE.to_owned()).or_insert(0); *f = (*f).max(c.epoch); }
        if c.content.reports.iter().any(|(v, _)| v == me) { let f = out.entry(SET_PROP.to_owned()).or_insert(0); *f = (*f).max(c.epoch); }
    }
    // Final certs the setcert engine holds (fetched from peers while catching up): a state
    // swap from another node never carries this node's local vote record, but its votes and
    // PROPs are in the certs it helped finalize.
    {
        let e = crate::setcert_node::engine(chain).lock();
        if let Some(lf) = e.last_final() {
            for t in lf.saturating_sub(CHAIN_SCAN_EPOCHS)..=lf {
                let Some(c) = e.final_cert(t) else { continue };
                if c.votes.contains_key(me) { let f = out.entry(SET_VOTE.to_owned()).or_insert(0); *f = (*f).max(c.epoch); }
                if c.content.reports.iter().any(|(v, _)| v == me) { let f = out.entry(SET_PROP.to_owned()).or_insert(0); *f = (*f).max(c.epoch); }
            }
        }
    }
    let suffix = format!(":{me}");
    for (k, _) in chain.store.state_scan_prefix("decide_commit:") {
        let Some(rest) = k.strip_prefix("decide_commit:") else { continue };
        if !rest.ends_with(&suffix) || rest.len() != 16 + suffix.len() { continue; }
        let Ok(e) = u64::from_str_radix(&rest[..16], 16) else { continue };
        if e >= from { let f = out.entry(DECIDE_COMMIT.to_owned()).or_insert(0); *f = (*f).max(e); }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floors(p: &[(&str, u64)]) -> BTreeMap<String, u64> { p.iter().map(|(k, v)| (k.to_string(), *v)).collect() }
    /// Bring a guard up: observe while behind, sync at `tip`, wait out the doppelganger window.
    fn up(g: &mut Guard, tip: u64, chain: &BTreeMap<String, u64>) {
        g.tick(tip, tip - 50, tip, chain);
        assert_eq!(g.phase, Phase::Observe, "behind the finalized head: observe-only");
        assert!(!g.may_sign(SET_VOTE, tip + 100));
        g.tick(tip + 1, tip, tip, chain);
        assert!(matches!(g.phase, Phase::Doppelganger { .. }));
        assert!(!g.may_sign(SET_VOTE, tip + 100), "no signing during the doppelganger wait");
        g.tick(tip + 1 + DOPPELGANGER_EPOCHS, tip + 1 + DOPPELGANGER_EPOCHS, tip + 1, chain);
        assert_eq!(g.phase, Phase::Ready);
    }

    #[test]
    fn restart_with_wiped_local_floor_never_resigns_what_the_chain_shows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sign_floor.json");
        let mut g = Guard::new(Some(path.clone()));
        assert_eq!(g.floor(SET_VOTE), 0, "local floor wiped");
        up(&mut g, 1_000, &floors(&[(SET_VOTE, 1_003), (DECIDE_COMMIT, 990)]));
        assert!(!g.may_sign(SET_VOTE, 1_003), "the chain shows a vote at 1,003: never again");
        assert!(g.may_sign(SET_VOTE, 1_004));
        assert!(!g.may_sign(DECIDE_COMMIT, 990) && g.may_sign(DECIDE_COMMIT, 991));
        // The raised floor is persisted for the next restart.
        assert_eq!(Guard::new(Some(path)).persisted_floor(SET_VOTE), 1_003);
    }

    #[test]
    fn a_doppelganger_blocks_signing() {
        let mut g = Guard::new(None);
        g.tick(500, 499, 499, &BTreeMap::new());
        assert!(matches!(g.phase, Phase::Doppelganger { .. }));
        // A seal signed with our key for an epoch after we came up, not made by us.
        g.observe_own(SEAL, 501, "sig-from-the-other-box");
        assert_eq!(g.phase, Phase::Blocked);
        g.tick(510, 510, 510, &BTreeMap::new());
        assert_eq!(g.phase, Phase::Blocked, "stays blocked");
        assert!(!g.may_sign(SEAL, 600) && !g.ready());
    }

    #[test]
    fn own_signatures_from_before_the_restart_only_raise_the_floor() {
        let mut g = Guard::new(None);
        g.tick(500, 450, 470, &BTreeMap::new());
        g.observe_own(SEAL, 480, "my-old-seal"); // gossip replay of the previous run
        assert_eq!(g.phase, Phase::Observe);
        up(&mut g, 600, &BTreeMap::new());
        assert!(!g.may_sign(SEAL, 480) && g.may_sign(SEAL, 603));
        // What this run signed is recognised as its own when it comes back over gossip.
        g.signed(SEAL, 603, "s603");
        g.observe_own(SEAL, 603, "s603");
        assert_eq!(g.phase, Phase::Ready);
        assert!(!g.may_sign(SEAL, 603), "never at or below the floor");
        // But a different signature for the same slot is another instance.
        g.observe_own(SEAL, 603, "other");
        assert_eq!(g.phase, Phase::Blocked);
    }

    #[test]
    fn normal_restart_resumes_after_sync_and_a_gap_goes_back_to_observe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sign_floor.json");
        let mut g = Guard::new(Some(path.clone()));
        up(&mut g, 2_000, &BTreeMap::new());
        g.signed(SET_PROP, 2_003, "p");
        drop(g);
        let mut g = Guard::new(Some(path)); // restart
        assert_eq!(g.persisted_floor(SET_PROP), 2_003);
        up(&mut g, 2_010, &BTreeMap::new());
        assert!(!g.may_sign(SET_PROP, 2_003) && g.may_sign(SET_PROP, 2_004));
        // Asleep for 10 epochs: observe-only again until synced.
        g.tick(2_030, 2_013, 2_020, &BTreeMap::new());
        assert_eq!(g.phase, Phase::Observe);
        assert!(!g.may_sign(SET_PROP, 2_031));
    }

    #[test]
    fn a_lagging_caller_never_trips_the_gap_rule() {
        let mut g = Guard::new(None);
        up(&mut g, 1_000, &BTreeMap::new());
        assert!(g.ready());
        // The decide worker ticks with the sealed epoch (9 behind), the driver with the tip.
        for i in 0..20 {
            g.tick(1_003 + i - 9, 1_003 + i, 1_003 + i, &BTreeMap::new());
            g.tick(1_003 + i, 1_003 + i, 1_003 + i, &BTreeMap::new());
            assert!(g.ready(), "tick {i}");
        }
        // A real pause still sends it back to observe-only.
        g.tick(1_040, 1_030, 1_030, &BTreeMap::new());
        assert_eq!(g.phase, Phase::Observe);
    }

    #[test]
    fn a_stalled_network_still_lets_a_synced_node_sign() {
        // Nothing finalizes (the network waits for this node): holding every final it knows
        // of, after observing, counts as synced; then the doppelganger wait.
        let mut g = Guard::new(None);
        g.tick(1_000, 900, 900, &BTreeMap::new());
        assert_eq!(g.phase, Phase::Observe);
        g.tick(1_001, 900, 950, &BTreeMap::new());
        g.tick(1_002, 900, 950, &BTreeMap::new());
        assert_eq!(g.phase, Phase::Observe, "a final it does not hold yet: keep observing");
        g.tick(1_003, 950, 950, &BTreeMap::new());
        assert!(matches!(g.phase, Phase::Doppelganger { .. }));
        g.tick(1_005, 950, 950, &BTreeMap::new());
        assert!(g.ready());
    }

    #[test]
    fn chain_floors_read_own_votes_props_and_decide_commits() {
        let d = tempfile::tempdir().unwrap();
        let c = Chain::new(crate::store::Store::open(d.path()).unwrap(), "n".into(), "hone-test".into());
        let cert = crate::setcert::Cert { epoch: 5_000, round: 0, prev: "p".into(),
            content: crate::setcert::Content { hashes: vec![], reports: vec![("me".into(), 1), ("you".into(), 1)] },
            votes: [("me".to_string(), "s".to_string())].into() };
        let _ = c.store.state_set(&format!("setcert_cert:{:016x}", 5_000u64), &serde_json::to_vec(&cert).unwrap());
        let _ = c.store.state_set(&format!("decide_commit:{:016x}:me", 4_990u64), b"{}");
        let _ = c.store.state_set(&format!("decide_commit:{:016x}:mexico", 4_999u64), b"{}");
        let f = chain_floors(&c, "me", 5_010);
        assert_eq!(f, floors(&[(SET_VOTE, 5_000), (SET_PROP, 5_000), (DECIDE_COMMIT, 4_990)]));
    }
}
