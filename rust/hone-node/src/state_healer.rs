//! State checkpoints and the self-healer.
//!
//! Every CHECKPOINT_INTERVAL epochs, right after a clock applies the quorum seal of epoch E
//! (entries, unbonding/params/governance, inference daemon), it records its balance state:
//!   * `R` = the reward driver's contiguous cursor at that instant (read before and after;
//!     if it moved, the checkpoint is skipped — the root would be a torn read),
//!   * `root` = balance_merkle_root() (the same root EpochFinalize carries),
//!   * a snapshot file `<data_dir>/checkpoints/<E>.json` of the sorted balance rows.
//! It signs `hone-checkpoint:<chain_id>:<node>:<E>:<R>:<root>` with its clock key and
//! gossips it on hone/consensus. Only checkpoints with the same (E, R) are comparable.
//!
//! CANONICAL RULE (exact). For a given (E, R), consider only attestations whose signature
//! verifies against the signer's clock registration / account keys and whose signer is a
//! reward voter for E (full-node clocks; device seats "owner@pubkey" never count). Group by
//! root and count DISTINCT OPERATORS (`clock_operator`, the part before '@'). A root is
//! canonical iff its operator count is >= 2 AND strictly greater than every other root's
//! operator count for that (E, R). Ties, or a single operator however many nodes it runs,
//! produce no canonical root. Unweighted on purpose: weights would let one large staker
//! decide alone, which is the thing this rule exists to prevent.
//!
//! N = 120 epochs (1 h at 30 s): long enough that the O(accounts) root + snapshot cost
//! and one small gossip message per clock are negligible; well past the reward finality
//! depth so a checkpoint never straddles the fork-choice window; short enough that drift
//! is caught within an hour, before many more epochs build on it.
//!
//! VERDICTS for our own record at (E, R):
//!   * agree    — canonical == ours. Nothing to do.
//!   * diverged — a canonical root exists and ours differs. In `heal` mode the node HALTS
//!     (no seals, votes, catch-up, submissions), asks peers for the (E, R) snapshot, verifies
//!     the snapshot's merkle root equals the canonical root, backs up its current balances,
//!     rebases atomically (current + canonical_E − own_E per key, so everything applied
//!     after E is kept), logs loudly and resumes. In `report` mode it only logs + reports.
//!   * hold     — peers disagree with us but there is no canonical root. Report, never guess.
//!
//! HONE_STATE_HEALER = off | report (default) | heal. Report is the default so the healer
//! cannot act on drift that predates it until an operator turns it on.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::chain::Chain;

pub const CHECKPOINT_INTERVAL: u64 = 120;
const KEEP_SNAPSHOTS: usize = 6;
const KEEP_ATTEST_EPOCHS: u64 = CHECKPOINT_INTERVAL * 24;

static HALTED: AtomicBool = AtomicBool::new(false);
/// Serialises tests that touch the process-wide HALTED / REWARD_CURSOR / PEER_COUNT.
#[cfg(test)]
pub static TEST_GLOBALS: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
/// The reward driver's contiguous cursor (last epoch whose rewards are applied).
pub static REWARD_CURSOR: AtomicU64 = AtomicU64::new(0);

/// Seqlock around reward-driver mutations: odd while the driver is applying rewards.
static DRIVER_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn driver_enter() { DRIVER_SEQ.fetch_add(1, Ordering::SeqCst); }
pub fn driver_exit(cursor: u64) {
    REWARD_CURSOR.store(cursor, Ordering::SeqCst);
    DRIVER_SEQ.fetch_add(1, Ordering::SeqCst);
}

/// True while the healer has stopped this node (diverged, rebasing).
pub fn halted() -> bool { HALTED.load(Ordering::SeqCst) }

/// React to a verdict on one of our own checkpoints. In `heal` mode a Diverged verdict
/// HALTS the node at once (no seals, reward votes, catch-up votes or submissions) and
/// returns true: the caller asks the attesters for the canonical snapshot. The halt is
/// lifted only by a successful `heal`. Report mode and Hold never halt.
pub fn on_verdict(v: &Verdict, mode: Mode) -> bool {
    match (v, mode) {
        (Verdict::Diverged { .. }, Mode::Heal) => {
            if !HALTED.swap(true, Ordering::SeqCst) {
                error!("[healer] HALTED: our checkpoint differs from the canonical root; \
                    not sealing, voting or accepting submissions until healed");
            }
            true
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode { Off, Report, Heal }

pub fn mode() -> Mode {
    match std::env::var("HONE_STATE_HEALER").as_deref() {
        Ok("off") => Mode::Off,
        Ok("heal") => Mode::Heal,
        _ => Mode::Report,
    }
}

pub fn checkpoint_due(epoch: u64) -> bool { epoch > 0 && epoch % CHECKPOINT_INTERVAL == 0 }

pub fn message(chain_id: &str, node: &str, epoch: u64, cursor: u64, root: &str) -> String {
    format!("hone-checkpoint:{}:{}:{}:{}:{}", chain_id, node, epoch, cursor, root)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub epoch: u64,
    pub cursor: u64,
    pub root: String,
    /// Sorted (key, value) balance rows, both hex.
    pub rows: Vec<(String, String)>,
}

impl Snapshot {
    pub fn from_rows(epoch: u64, cursor: u64, sorted: &[(String, Vec<u8>)]) -> Self {
        Snapshot {
            epoch, cursor,
            root: crate::store::merkle_root_of(sorted),
            rows: sorted.iter().map(|(k, v)| (hex::encode(k.as_bytes()), hex::encode(v))).collect(),
        }
    }

    pub fn decoded(&self) -> Result<Vec<(String, Vec<u8>)>> {
        let mut out = Vec::with_capacity(self.rows.len());
        for (k, v) in &self.rows {
            out.push((String::from_utf8(hex::decode(k)?)?, hex::decode(v)?));
        }
        // Must already be sorted and unique — the root is order-sensitive.
        if out.windows(2).any(|w| w[0].0 >= w[1].0) { bail!("snapshot rows not sorted/unique"); }
        Ok(out)
    }

    /// The root recomputed from the rows (never trust the `root` field).
    pub fn verified_root(&self) -> Result<String> {
        Ok(crate::store::merkle_root_of(&self.decoded()?))
    }
}

fn amount(v: &[u8]) -> u64 {
    <[u8; 8]>::try_from(v).map(u64::from_le_bytes).unwrap_or(0)
}

// ── Tally ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct Tally {
    /// (epoch, cursor) → node → root
    att: BTreeMap<(u64, u64), BTreeMap<String, String>>,
}

impl Tally {
    pub fn add(&mut self, epoch: u64, cursor: u64, node: &str, root: &str) {
        self.att.entry((epoch, cursor)).or_default().insert(node.to_owned(), root.to_owned());
        if let Some(&(newest, _)) = self.att.keys().next_back() {
            self.att.retain(|(e, _), _| e + KEEP_ATTEST_EPOCHS >= newest);
        }
    }

    pub fn get(&self, epoch: u64, cursor: u64) -> Option<&BTreeMap<String, String>> {
        self.att.get(&(epoch, cursor))
    }

    pub fn summary(&self) -> serde_json::Value {
        let v: Vec<serde_json::Value> = self.att.iter().rev().take(12).map(|((e, r), m)| {
            serde_json::json!({ "epoch": e, "cursor": r, "roots": m })
        }).collect();
        serde_json::json!(v)
    }
}

/// The canonical root for one (E, R) under the rule in the module docs.
pub fn canonical(att: &BTreeMap<String, String>, voters: &[String]) -> Option<(String, Vec<String>)> {
    // Same filter as clock::reward_voters: full-node clocks only, never device seats.
    let voters: BTreeSet<&str> = voters.iter().map(String::as_str)
        .filter(|v| hone_types::parse_device_clock_id(v).is_none())
        .collect();
    let mut ops: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut nodes: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (node, root) in att {
        if !voters.contains(node.as_str()) { continue; }
        ops.entry(root.as_str()).or_default().insert(crate::clock::clock_operator(node));
        nodes.entry(root.as_str()).or_default().push(node.clone());
    }
    let mut ranked: Vec<(&str, usize)> = ops.iter().map(|(r, o)| (*r, o.len())).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    match ranked.as_slice() {
        [(root, n), rest @ ..] if *n >= 2 && rest.iter().all(|(_, m)| m < n) =>
            Some((root.to_string(), nodes.remove(root).unwrap_or_default())),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Agree,
    Diverged { canonical: String, attesters: Vec<String> },
    Hold { reason: String },
}

/// Our verdict for (E, R), given our own root and everyone's attestations.
pub fn evaluate(own_node: &str, own_root: &str, att: &BTreeMap<String, String>, voters: &[String]) -> Verdict {
    match canonical(att, voters) {
        Some((root, _)) if root == own_root => Verdict::Agree,
        Some((root, nodes)) => Verdict::Diverged {
            canonical: root,
            attesters: nodes.into_iter().filter(|n| n != own_node).collect(),
        },
        None if att.iter().any(|(n, r)| n != own_node && r != own_root) =>
            Verdict::Hold { reason: "peers disagree and no root has >=2 operators uncontested".into() },
        None => Verdict::Hold { reason: "no canonical checkpoint yet".into() },
    }
}

// ── Snapshot files ───────────────────────────────────────────────────────────

fn dir(data_dir: &Path) -> PathBuf { data_dir.join("checkpoints") }

fn path(data_dir: &Path, epoch: u64, cursor: u64) -> PathBuf {
    dir(data_dir).join(format!("{}-{}.json", epoch, cursor))
}

pub fn save(data_dir: &Path, snap: &Snapshot) -> Result<()> {
    let d = dir(data_dir);
    std::fs::create_dir_all(&d)?;
    let p = path(data_dir, snap.epoch, snap.cursor);
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(snap)?)?;
    std::fs::rename(&tmp, &p)?;
    // Keep the newest KEEP_SNAPSHOTS.
    let mut files: Vec<PathBuf> = std::fs::read_dir(&d)?.flatten().map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false)).collect();
    files.sort_by_key(|p| p.file_stem().and_then(|s| s.to_str())
        .and_then(|s| s.split('-').next()).and_then(|e| e.parse::<u64>().ok()).unwrap_or(0));
    while files.len() > KEEP_SNAPSHOTS { let _ = std::fs::remove_file(files.remove(0)); }
    Ok(())
}

pub fn load(data_dir: &Path, epoch: u64, cursor: u64) -> Option<Snapshot> {
    serde_json::from_slice(&std::fs::read(path(data_dir, epoch, cursor)).ok()?).ok()
}

/// Take our checkpoint for sealed epoch E. None if the reward driver moved mid-read.
pub fn take(chain: &Chain, epoch: u64) -> Option<Snapshot> {
    let seq = DRIVER_SEQ.load(Ordering::SeqCst);
    let cursor = REWARD_CURSOR.load(Ordering::SeqCst);
    if seq % 2 == 1 || cursor == 0 { return None; }
    let rows = chain.store.sorted_balance_entries();
    if DRIVER_SEQ.load(Ordering::SeqCst) != seq { return None; }
    Some(Snapshot::from_rows(epoch, cursor, &rows))
}

// ── Rebase ───────────────────────────────────────────────────────────────────

/// Per balance key: current + canonical_E − own_E. Everything applied after E (which every
/// node applied identically) is kept; only the divergence present at E is removed.
pub fn rebase_rows(
    current: &[(String, Vec<u8>)],
    own: &[(String, Vec<u8>)],
    canon: &[(String, Vec<u8>)],
) -> (Vec<(String, u64)>, Vec<String>) {
    let cur: HashMap<&str, u64> = current.iter().map(|(k, v)| (k.as_str(), amount(v))).collect();
    let own: HashMap<&str, u64> = own.iter().map(|(k, v)| (k.as_str(), amount(v))).collect();
    let canon: HashMap<&str, u64> = canon.iter().map(|(k, v)| (k.as_str(), amount(v))).collect();
    let keys: BTreeSet<&str> = own.keys().chain(canon.keys()).copied().collect();
    let mut out = Vec::new();
    let mut clamped = Vec::new();
    for k in keys {
        let delta = *canon.get(k).unwrap_or(&0) as i128 - *own.get(k).unwrap_or(&0) as i128;
        if delta == 0 { continue; }
        let c = *cur.get(k).unwrap_or(&0) as i128;
        let n = c + delta;
        if n < 0 { clamped.push(k.replace('\0', "/")); }
        out.push((k.to_owned(), n.clamp(0, u64::MAX as i128) as u64));
    }
    (out, clamped)
}

/// Verify `canon_snap` against the canonical root and rebase this node onto it.
pub fn heal(chain: &Chain, data_dir: &Path, canonical_root: &str, canon_snap: &Snapshot) -> Result<usize> {
    let canon_rows = canon_snap.decoded()?;
    let got = crate::store::merkle_root_of(&canon_rows);
    if got != canonical_root {
        bail!("peer snapshot root {} != canonical {}", got, canonical_root);
    }
    let own = load(data_dir, canon_snap.epoch, canon_snap.cursor)
        .context("own snapshot for this checkpoint is missing")?;
    let own_rows = own.decoded()?;

    let was_halted = HALTED.swap(true, Ordering::SeqCst);
    let res = (|| -> Result<usize> {
        let _g = chain.write_lock.lock();
        let current = chain.store.sorted_balance_entries();
        let backup = data_dir.join(format!("healer-backup-{}-{}.json",
            canon_snap.epoch, crate::utils::now_ms()));
        std::fs::write(&backup, serde_json::to_vec(&Snapshot::from_rows(
            canon_snap.epoch, canon_snap.cursor, &current))?)
            .with_context(|| format!("writing backup {:?}", backup))?;
        let (rows, clamped) = rebase_rows(&current, &own_rows, &canon_rows);
        if !clamped.is_empty() {
            error!("[healer] {} balance(s) would go negative and were clamped to 0: {:?}", clamped.len(), clamped);
        }
        chain.store.write_balances_atomic(&rows)?;
        save(data_dir, canon_snap)?;
        error!("[healer] STATE REBASED at checkpoint epoch {} (cursor {}): {} balance row(s) changed, \
            own root {} -> canonical {}; backup {:?}",
            canon_snap.epoch, canon_snap.cursor, rows.len(), own.root, canonical_root, backup);
        Ok(rows.len())
    })();
    // Success lifts the halt; a failed attempt leaves the node as halted as it was.
    HALTED.store(if res.is_ok() { false } else { was_halted }, Ordering::SeqCst);
    res
}

// ── Runtime state shared with main/net/api ───────────────────────────────────

/// The running node's healer, for the API.
pub static GLOBAL: std::sync::OnceLock<std::sync::Arc<Healer>> = std::sync::OnceLock::new();
/// Data dir, for serving snapshot files over the API.
pub static DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

#[derive(Default)]
pub struct Healer {
    pub tally: Mutex<Tally>,
    /// (epoch, cursor) of our own checkpoints → root.
    pub own: Mutex<BTreeMap<(u64, u64), String>>,
    pub last_verdict: Mutex<Option<(u64, u64, Verdict)>>,
    /// When we last asked peers for / served a snapshot of (E, R) — rate limit, 60 s.
    pub requested: Mutex<BTreeMap<(u64, u64), std::time::Instant>>,
    pub served: Mutex<BTreeMap<(u64, u64), std::time::Instant>>,
}

/// True (and records now) if `key` was not touched in the last 60 s.
pub fn rate_ok(m: &Mutex<BTreeMap<(u64, u64), std::time::Instant>>, key: (u64, u64)) -> bool {
    let mut g = m.lock();
    let now = std::time::Instant::now();
    if g.get(&key).map(|t| now.duration_since(*t).as_secs() < 60).unwrap_or(false) { return false; }
    g.insert(key, now);
    if g.len() > 64 { if let Some(k) = g.keys().next().copied() { g.remove(&k); } }
    true
}

impl Healer {
    /// The canonical root for (E, R) if one exists and ours differs from it.
    pub fn diverged_from(&self, epoch: u64, cursor: u64, voters: &[String]) -> Option<String> {
        let own = self.own.lock().get(&(epoch, cursor)).cloned()?;
        let att = self.tally.lock().get(epoch, cursor).cloned()?;
        canonical(&att, voters).map(|(r, _)| r).filter(|r| *r != own)
    }

    pub fn status(&self) -> serde_json::Value {
        let v = self.last_verdict.lock().clone();
        serde_json::json!({
            "mode": format!("{:?}", mode()),
            "halted": halted(),
            "interval_epochs": CHECKPOINT_INTERVAL,
            "reward_cursor": REWARD_CURSOR.load(Ordering::SeqCst),
            "own": self.own.lock().iter().rev().take(6)
                .map(|((e, r), root)| serde_json::json!({"epoch": e, "cursor": r, "root": root}))
                .collect::<Vec<_>>(),
            "last_verdict": v.map(|(e, r, v)| serde_json::json!({
                "epoch": e, "cursor": r, "verdict": format!("{:?}", v)})),
            "attestations": self.tally.lock().summary(),
        })
    }

    /// Record an attestation and, if we hold our own root for it, evaluate.
    pub fn observe(&self, epoch: u64, cursor: u64, node: &str, root: &str, own_node: &str, voters: &[String])
        -> Option<Verdict>
    {
        self.tally.lock().add(epoch, cursor, node, root);
        let own_root = self.own.lock().get(&(epoch, cursor)).cloned()?;
        let att = self.tally.lock().get(epoch, cursor).cloned().unwrap_or_default();
        let v = evaluate(own_node, &own_root, &att, voters);
        let prev = self.last_verdict.lock().replace((epoch, cursor, v.clone()));
        if prev.as_ref().map(|p| p.2 != v || p.0 != epoch).unwrap_or(true) {
            match &v {
                Verdict::Agree => info!("[healer] checkpoint {}/{}: agree ({})", epoch, cursor, &own_root[..12.min(own_root.len())]),
                Verdict::Diverged { canonical, attesters } => error!(
                    "[healer] checkpoint {}/{}: DIVERGED — ours {} canonical {} (attested by {:?}); mode {:?}",
                    epoch, cursor, own_root, canonical, attesters, mode()),
                Verdict::Hold { reason } => warn!("[healer] checkpoint {}/{}: hold — {}", epoch, cursor, reason),
            }
        }
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(n, r)| (n.to_string(), r.to_string())).collect()
    }
    fn voters(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }
    const PHONE: &str = "josh@8848e0d2aa0000000000000000000000000000000000000000000000000000aa";

    #[test]
    fn two_operators_make_a_canonical_root() {
        let v = voters(&["natoshisakamoto", "shindevlin", "josh"]);
        let a = att(&[("natoshisakamoto", "A"), ("shindevlin", "A"), ("josh", "B")]);
        assert_eq!(canonical(&a, &v).unwrap().0, "A");
    }

    #[test]
    fn one_operator_never_makes_one_and_devices_never_count() {
        let v = voters(&["josh", "josh@node2", PHONE, "shindevlin"]);
        // Same operator twice.
        assert!(canonical(&att(&[("josh", "A"), ("josh@node2", "A")]), &v).is_none());
        // A device seat is not a reward voter.
        assert!(canonical(&att(&[("josh", "A"), (PHONE, "A")]), &v).is_none());
        // ...even when the device's owner is a different operator from the other attester.
        assert!(canonical(&att(&[("shindevlin", "A"), (PHONE, "A")]), &v).is_none());
        // Not a voter for this epoch at all.
        assert!(canonical(&att(&[("josh", "A"), ("mallory", "A")]), &v).is_none());
    }

    #[test]
    fn ties_and_contested_roots_are_not_canonical() {
        let v = voters(&["a", "b", "c", "d"]);
        assert!(canonical(&att(&[("a", "X"), ("b", "X"), ("c", "Y"), ("d", "Y")]), &v).is_none());
        assert_eq!(canonical(&att(&[("a", "X"), ("b", "X"), ("c", "X"), ("d", "Y")]), &v).unwrap().0, "X");
    }

    #[test]
    fn verdicts() {
        let v = voters(&["a", "b", "c"]);
        assert_eq!(evaluate("a", "X", &att(&[("a", "X"), ("b", "X")]), &v), Verdict::Agree);
        assert_eq!(evaluate("c", "Z", &att(&[("a", "X"), ("b", "X"), ("c", "Z")]), &v),
            Verdict::Diverged { canonical: "X".into(), attesters: vec!["a".into(), "b".into()] });
        // Two nodes, two roots: never guess.
        assert!(matches!(evaluate("a", "X", &att(&[("a", "X"), ("b", "Y")]), &v), Verdict::Hold { .. }));
        assert!(matches!(evaluate("a", "X", &att(&[("a", "X")]), &v), Verdict::Hold { .. }));
    }

    fn row(k: &str, v: u64) -> (String, Vec<u8>) { (k.to_string(), v.to_le_bytes().to_vec()) }

    #[test]
    fn rebase_keeps_post_checkpoint_deltas() {
        // At E: canonical alice=100 bob=50; we (wrongly) had alice=130 (self-paid storage)
        // and no carol. Since E alice spent 10 and bob got 5 — on every node.
        let canon = vec![row("alice\0HONE", 100), row("bob\0HONE", 50), row("carol\0HONE", 7)];
        let own = vec![row("alice\0HONE", 130), row("bob\0HONE", 50)];
        let cur = vec![row("alice\0HONE", 120), row("bob\0HONE", 55)];
        let (rows, clamped) = rebase_rows(&cur, &own, &canon);
        assert!(clamped.is_empty());
        let m: HashMap<_, _> = rows.into_iter().collect();
        assert_eq!(m["alice\0HONE"], 90);
        assert_eq!(m["carol\0HONE"], 7);
        assert!(!m.contains_key("bob\0HONE"), "unchanged keys are not rewritten");
    }

    #[test]
    fn snapshot_root_is_recomputed_and_matches_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        store.set_balance("alice", "HONE", 5).unwrap();
        store.set_balance("bob", "HONE", 9).unwrap();
        let snap = Snapshot::from_rows(240, 200, &store.sorted_balance_entries());
        assert_eq!(snap.root, store.balance_merkle_root());
        assert_eq!(snap.verified_root().unwrap(), snap.root);
        let mut forged = snap.clone();
        forged.rows[0].1 = hex::encode(999u64.to_le_bytes());
        assert_ne!(forged.verified_root().unwrap(), snap.root);
    }

    /// End to end on real stores: B self-paid at E; heal rebases B onto A's verified
    /// snapshot, keeps B's later transfer, writes a backup, and lifts the halt.
    #[test]
    fn heal_converges_diverged_node() {
        let _g = TEST_GLOBALS.lock();
        let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let a = Chain::new(crate::store::Store::open(da.path()).unwrap(), "a".into(), "t".into());
        let b = Chain::new(crate::store::Store::open(db.path()).unwrap(), "b".into(), "t".into());
        for c in [&a, &b] { c.store.set_balance("alice", "HONE", 100).unwrap(); }
        b.store.set_balance("bnode", "HONE", 30).unwrap(); // the self-paid reward
        let sa = Snapshot::from_rows(120, 100, &a.store.sorted_balance_entries());
        let sb = Snapshot::from_rows(120, 100, &b.store.sorted_balance_entries());
        save(db.path(), &sb).unwrap();
        // After E both apply the same transfer.
        for c in [&a, &b] { c.store.debit("alice", "HONE", 10).unwrap(); c.store.credit("zed", "HONE", 10).unwrap(); }
        assert_ne!(a.store.balance_merkle_root(), b.store.balance_merkle_root());

        // A wrong snapshot is refused.
        assert!(heal(&b, db.path(), &sa.root, &sb).is_err());
        let n = heal(&b, db.path(), &sa.root, &sa).unwrap();
        assert_eq!(n, 1);
        assert!(!halted());
        assert_eq!(b.store.get_balance("bnode", "HONE"), 0);
        // bnode now holds an explicit 0 row; compare non-zero balances.
        let nz = |c: &Chain| c.store.sorted_balance_entries().into_iter()
            .filter(|(_, v)| amount(v) != 0).collect::<Vec<_>>();
        assert_eq!(nz(&a), nz(&b));
        assert!(std::fs::read_dir(db.path()).unwrap().flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("healer-backup-")));
    }

    #[test]
    fn diverged_halts_only_in_heal_mode_until_healed() {
        let _g = TEST_GLOBALS.lock();
        HALTED.store(false, Ordering::SeqCst);
        let d = Verdict::Diverged { canonical: "X".into(), attesters: vec!["a".into(), "b".into()] };
        assert!(!on_verdict(&d, Mode::Report));
        assert!(!halted(), "report mode never halts");
        assert!(!on_verdict(&Verdict::Hold { reason: "x".into() }, Mode::Heal));
        assert!(!halted(), "hold never halts: no canonical root, never guess");
        assert!(on_verdict(&d, Mode::Heal));
        assert!(halted());

        // A forged snapshot does not lift the halt; the verified one does.
        let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let a = Chain::new(crate::store::Store::open(da.path()).unwrap(), "a".into(), "t".into());
        let b = Chain::new(crate::store::Store::open(db.path()).unwrap(), "b".into(), "t".into());
        a.store.set_balance("alice", "HONE", 100).unwrap();
        b.store.set_balance("alice", "HONE", 130).unwrap();
        let sa = Snapshot::from_rows(240, 9, &a.store.sorted_balance_entries());
        save(db.path(), &Snapshot::from_rows(240, 9, &b.store.sorted_balance_entries())).unwrap();
        let mut forged = sa.clone();
        forged.rows[0].1 = hex::encode(500u64.to_le_bytes());
        assert!(heal(&b, db.path(), &sa.root, &forged).is_err());
        assert!(halted(), "bad snapshot keeps the halt");
        heal(&b, db.path(), &sa.root, &sa).unwrap();
        assert!(!halted());
        assert_eq!(b.store.balance_merkle_root(), a.store.balance_merkle_root());
    }

    /// The drift that exists today (three nodes, three roots) can never produce a
    /// canonical checkpoint, so the healer cannot fire on it.
    #[test]
    fn current_three_way_drift_has_no_canonical_root() {
        let v = voters(&["natoshisakamoto", "shindevlin", "josh", PHONE]);
        let a = att(&[("natoshisakamoto", "G"), ("shindevlin", "B"), ("josh", "N"), (PHONE, "G")]);
        assert!(canonical(&a, &v).is_none());
        assert!(matches!(evaluate("shindevlin", "B", &a, &v), Verdict::Hold { .. }));
    }

    #[test]
    fn take_skips_torn_reads_and_unknown_cursor() {
        let _g = TEST_GLOBALS.lock();
        let d = tempfile::tempdir().unwrap();
        let c = Chain::new(crate::store::Store::open(d.path()).unwrap(), "a".into(), "t".into());
        REWARD_CURSOR.store(0, Ordering::SeqCst);
        assert!(take(&c, 120).is_none());
        driver_enter();
        REWARD_CURSOR.store(77, Ordering::SeqCst);
        assert!(take(&c, 120).is_none(), "driver mid-walk");
        driver_exit(77);
        assert_eq!(take(&c, 120).unwrap().cursor, 77);
    }

    #[test]
    fn tally_keeps_recent_and_snapshot_files_rotate() {
        let mut t = Tally::default();
        t.add(120, 1, "a", "X");
        t.add(120 + KEEP_ATTEST_EPOCHS + 1, 1, "a", "X");
        assert!(t.get(120, 1).is_none());
        let d = tempfile::tempdir().unwrap();
        for i in 1..=(KEEP_SNAPSHOTS as u64 + 3) {
            save(d.path(), &Snapshot { epoch: i * 120, cursor: 1, root: "r".into(), rows: vec![] }).unwrap();
        }
        assert_eq!(std::fs::read_dir(dir(d.path())).unwrap().count(), KEEP_SNAPSHOTS);
        assert!(load(d.path(), 120, 1).is_none());
        assert!(load(d.path(), (KEEP_SNAPSHOTS as u64 + 3) * 120, 1).is_some());
    }
}
