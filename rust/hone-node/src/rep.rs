//! Universal reputation (docs/spec-universal-reputation-2026-11-14.md). Gate REPUTATION_EPOCH.
//!
//! One record per subject per domain (`rep:{subject}:{domain}`): good and bad evidence in
//! fixed point (x1000), a prior, and the last decay boundary. The composite is the
//! evidence-weighted average of the domains the subject works in; the owner score
//! (`rep_owner:{account}` holds only its shared violation penalty) is derived from the owner's
//! device composites. Integer math only.
//!
//! Every write happens in decided-set processing (entry apply, certified-set apply, the
//! seal tick), never in an API handler. Before REPUTATION_EPOCH every hook returns before
//! reading or writing anything, so state stays byte-identical to the pre-reputation rules.
//!
//! Two rules (Josh, 2026-11-14) hold throughout:
//! 1. Reputation is EARNED ONLY FROM PROVEN USEFUL WORK: accepted useful data, independent
//!    confirmations, passed challenges / audits, certified seals and relays, proven storage,
//!    accepted jobs. Nothing is earned for existing, registering, uptime or stake alone (no
//!    hook adds good evidence for those, and migration seeds no more good evidence than the
//!    old records actually proved).
//! 2. OWNER TRUST IS THE MAIN SOURCE OF A NEW DEVICE'S TRUST: a device of an established
//!    owner starts at about the owner's score, carried as at most INHERIT_CAP evidence (low
//!    confidence, so bad events pull it down fast); a device of a new or unknown owner
//!    starts neutral-low and has to earn it.
//!
//! Decay is lazy and deterministic: a record is decayed x0.98 for every 2,880-epoch
//! boundary crossed since its `last_decay_epoch`, whenever it is next touched or viewed.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::chain::Chain;

// ── Tunables (the spec's small / medium / large). One place, for Josh to tune. ──────
/// Fixed point: one evidence unit = 1,000.
pub const UNIT: u64 = 1_000;
/// "+small": a useful reading, a storage proof, a seal in quorum, a relayed entry.
pub const SMALL: u64 = UNIT;
/// "+/-medium": an independent confirmation, a missed spot check, a passed audit.
pub const MEDIUM: u64 = 3 * UNIT;
/// "-large": a contradiction (about 3x a confirmation), a failed challenge, a teleport.
pub const LARGE: u64 = 9 * UNIT;
/// "-very large": a failed random audit.
pub const VERY_LARGE: u64 = 27 * UNIT;
/// Proven violation: the device's domains restart from zero good evidence with this much bad.
pub const VIOLATION_BAD: u64 = 100 * UNIT;
/// Proven violation on any device also hits its owner ("shared -large").
pub const OWNER_VIOLATION_BAD: u64 = LARGE;
/// Proven violation also kills the stake (Josh 2026-11-14): 100% of every role_stake backing the
/// device (any role, any staker) goes to the recycle fund. This overrides the 10% per-incident
/// cap for proven violation only. The owner's other devices are not slashed (reputation hit only).
///
/// Violation is per device unless PROVEN account-wide, which takes either
///  (1) violation evidence signed by the owner's account key itself (`on_account_violation`), or
///  (2) proven violations on at least max(ACCOUNT_VIOLATION_MIN_DEVICES,
///      ceil(ACCOUNT_VIOLATION_SHARE_BPS x the owner's ACTIVE devices)) of its devices within
///      ACCOUNT_VIOLATION_WINDOW epochs (coordinated). An owner with thousands of devices needs
///      proportionally more.
/// Active device: registered and with at least one useful-work (good) event within
/// ACCOUNT_VIOLATION_WINDOW, so registering idle devices never raises the threshold.
pub const ACCOUNT_VIOLATION_WINDOW: u64 = 20_160;
pub const ACCOUNT_VIOLATION_MIN_DEVICES: u64 = 2;
pub const ACCOUNT_VIOLATION_SHARE_BPS: u64 = 1_000;
/// Account-wide violation slashes 100% of every stake the account placed (any device, any role)
/// to recycle and resets the owner: an account penalty of ACCOUNT_VIOLATION_BAD (decays x0.98 a
/// day) holds the owner score, and so every new device's inheritance, at the violation floor (0)
/// while it is at least ACCOUNT_VIOLATION_FLOOR_BPS of its full size (~34 days).
pub const ACCOUNT_VIOLATION_BAD: u64 = VIOLATION_BAD;
pub const ACCOUNT_VIOLATION_FLOOR_BPS: u64 = 5_000;
/// Slash log entries kept per subject / account.
pub const SLASH_LOG: usize = 20;
/// Decay period (1 day of 30 s epochs) and factor (x0.98, half-life ~34 days).
pub const DECAY_EPOCHS: u64 = 2_880;
pub const DECAY_NUM: u64 = 98;
pub const DECAY_DEN: u64 = 100;
/// More boundaries than this decay everything to zero anyway (0.98^1000 ~ 2e-9).
pub const MAX_DECAY_STEPS: u64 = 1_000;
/// Open firmware / unknown owner: neutral-low prior (score 3,500 with no evidence).
pub const PRIOR_OPEN_G: u64 = 2 * UNIT;
pub const PRIOR_OPEN_B: u64 = 2 * UNIT;
/// Locked / attested hardware: higher prior (score 6,500 with no evidence).
pub const PRIOR_LOCKED_G: u64 = 6 * UNIT;
pub const PRIOR_LOCKED_B: u64 = 2 * UNIT;
/// Rule 2: a new device of an owner with reputation starts at the owner's score, carried
/// as a prior of min(INHERIT_CAP, owner evidence): low confidence, quick to fall. This
/// replaces the open prior (it is the device's whole prior, not an addition to it).
pub const INHERIT_CAP: u64 = 20 * UNIT;
/// Lower-confidence-bound penalty: UNCERTAINTY_K / sqrt(evidence units + 1).
pub const UNCERTAINTY_K: u64 = 3_000;
/// Confidence 0..10,000 reaches half at this much evidence.
pub const CONFIDENCE_HALF: u64 = 20 * UNIT;
/// A confirmation / contradiction counts only from another owner scoring at least this.
pub const CONFIRM_MIN_OWNER_SCORE: u64 = 2_000;
/// A new device counts toward its owner only after this many epochs.
pub const OWNER_COUNT_DELAY: u64 = 2_880;
/// Migration: evidence (in units) a seeded domain may start with.
pub const SEED_CAP_UNITS: u64 = 50;
/// Reward weight range: 0.25 .. 1.0 (bps).
pub const MIN_REWARD_BPS: u64 = 2_500;
pub const MAX_REWARD_BPS: u64 = 10_000;
/// Recent events kept per subject for the API.
pub const RECENT_EVENTS: usize = 20;

const MIGRATED_KEY: &str = "rep_migrated";

// ── Gate ────────────────────────────────────────────────────────────────────────
pub fn activation() -> u64 {
    #[cfg(test)]
    if let Some(v) = TEST_EPOCH.with(|c| c.get()) { return v; }
    hone_types::REPUTATION_EPOCH
}
#[cfg(test)]
thread_local! { pub static TEST_EPOCH: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) }; }

pub fn active(epoch: u64) -> bool { epoch >= activation() }

/// The epoch a hook runs at: the certified set's epoch while one is being applied,
/// else the entry's own epoch.
pub fn at(entry_epoch: u64) -> u64 { crate::tx::AsOfEpoch::get().unwrap_or(entry_epoch) }

// ── Domains ─────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Domain { Data, Alert, Storage, Compute, Consensus, Oracle, Relay }

impl Domain {
    pub const ALL: [Domain; 7] = [Domain::Data, Domain::Alert, Domain::Storage, Domain::Compute,
                                  Domain::Consensus, Domain::Oracle, Domain::Relay];
    pub fn as_str(self) -> &'static str {
        match self {
            Domain::Data => "data", Domain::Alert => "alert", Domain::Storage => "storage",
            Domain::Compute => "compute", Domain::Consensus => "consensus",
            Domain::Oracle => "oracle", Domain::Relay => "relay",
        }
    }
}

/// The provable violations (spec 6a). A rejected stale commit alone is none of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ViolationKind {
    /// Two different signed statements for the same slot: a clock double-sign, or two
    /// different device commits at the same device seq.
    Equivocation,
    /// Data proven made up (audit / challenge).
    Fabrication,
    /// Old data re-submitted as new (audit).
    Replay,
}
impl ViolationKind {
    pub fn as_str(self) -> &'static str {
        match self { ViolationKind::Equivocation => "equivocation", ViolationKind::Fabrication => "fabrication", ViolationKind::Replay => "replay" }
    }
}

// ── Records ─────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainRec {
    pub good: u64,
    pub bad: u64,
    pub prior_g: u64,
    pub prior_b: u64,
    pub last_decay_epoch: u64,
    /// Day (epoch / DECAY_EPOCHS) the diminishing counters belong to.
    #[serde(default)]
    pub day: u64,
    /// Diminishing events of each kind already counted this day.
    #[serde(default)]
    pub day_counts: BTreeMap<String, u64>,
}

impl DomainRec {
    /// Decay both counters x0.98 per 2,880-epoch boundary crossed. Never goes backwards.
    pub fn decay_to(&mut self, epoch: u64) {
        let now = epoch / DECAY_EPOCHS;
        let last = self.last_decay_epoch / DECAY_EPOCHS;
        if now <= last { return; }
        let steps = (now - last).min(MAX_DECAY_STEPS);
        for _ in 0..steps {
            if self.good == 0 && self.bad == 0 { break; }
            self.good = self.good * DECAY_NUM / DECAY_DEN;
            self.bad = self.bad * DECAY_NUM / DECAY_DEN;
        }
        self.last_decay_epoch = now * DECAY_EPOCHS;
    }
    pub fn evidence(&self) -> u64 { self.good.saturating_add(self.bad) }
    pub fn score(&self) -> u64 { score(self.good, self.bad, self.prior_g, self.prior_b) }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub owner: String,
    pub created_epoch: u64,
    pub prior_g: u64,
    pub prior_b: u64,
    #[serde(default)]
    pub attested: bool,
    /// Epoch of the latest useful-work (good) event: decides whether the device is active.
    #[serde(default)]
    pub last_work_epoch: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event { pub epoch: u64, pub domain: String, pub kind: String, pub good: u64, pub bad: u64 }

/// Score 0..10,000: lower confidence bound of good / (good + bad), with the prior.
pub fn score(good: u64, bad: u64, prior_g: u64, prior_b: u64) -> u64 {
    let num = good as u128 + prior_g as u128;
    let den = good as u128 + bad as u128 + prior_g as u128 + prior_b as u128;
    if den == 0 { return 0; }
    let mean = (10_000u128 * num / den) as u64;
    let units = (den / UNIT as u128).min(u64::MAX as u128) as u64;
    mean.saturating_sub(UNCERTAINTY_K / (units.saturating_add(1)).isqrt())
}

/// Confidence 0..10,000 from evidence alone (no prior).
pub fn confidence(evidence: u64) -> u64 {
    (10_000u128 * evidence as u128 / (evidence as u128 + CONFIDENCE_HALF as u128)) as u64
}

fn rec_key(subject: &str, d: Domain) -> String { format!("rep:{subject}:{}", d.as_str()) }
fn meta_key(subject: &str) -> String { format!("rep_meta:{subject}") }
fn ev_key(subject: &str) -> String { format!("rep_ev:{subject}") }
fn dev_key(owner: &str, subject: &str) -> String { format!("rep_dev:{owner}:{subject}") }
fn owner_key(owner: &str) -> String { format!("rep_owner:{owner}") }
fn violation_key(owner: &str, subject: &str) -> String { format!("rep_violation:{owner}:{subject}") }
fn acct_key(owner: &str) -> String { format!("rep_account_violation:{owner}") }
fn slash_key(subject: &str) -> String { format!("rep_slash:{subject}") }

fn get<T: serde::de::DeserializeOwned>(chain: &Chain, k: &str) -> Option<T> {
    chain.store.state_get(k).and_then(|b| serde_json::from_slice(&b).ok())
}
fn put<T: Serialize>(chain: &Chain, k: &str, v: &T) {
    let _ = chain.store.state_set(k, &serde_json::to_vec(v).unwrap_or_default());
}

/// The owner a subject counts for: its recorded owner, else the owner part of a device
/// id `owner@<pubkey>`, else the subject itself (an account or node id).
pub fn owner_of(chain: &Chain, subject: &str) -> String {
    if let Some(m) = get::<Meta>(chain, &meta_key(subject)) { return m.owner; }
    derive_owner(subject)
}
fn derive_owner(subject: &str) -> String {
    hone_types::parse_device_clock_id(subject).map(|(o, _)| o.to_owned()).unwrap_or_else(|| subject.to_owned())
}

/// The prior a new subject of `owner` would get at `epoch` (not written).
/// Rule 2: an owner with counted evidence passes on its score, carried as at most
/// INHERIT_CAP of evidence; the prior's ratio is set so the device's lower-bound score
/// starts at the owner's score (bounded by 10,000). Otherwise the open (or locked) prior.
fn new_prior(chain: &Chain, owner: &str, attested: bool, epoch: u64) -> (u64, u64) {
    let base = if attested { (PRIOR_LOCKED_G, PRIOR_LOCKED_B) } else { (PRIOR_OPEN_G, PRIOR_OPEN_B) };
    let o = owner_view_x(chain, owner, epoch, false);
    let n = INHERIT_CAP.min(o.evidence);
    if n < UNIT { return base; }
    let unc = UNCERTAINTY_K / (n / UNIT + 1).isqrt();
    let target = (o.score + unc).min(10_000);
    let g = (n as u128 * target as u128).div_ceil(10_000) as u64;
    let inherited = (g, n - g);
    // An honest owner's device never starts below the open (or locked) prior: an owner still
    // short of evidence has a low, confidence-capped score. A proven violation on the owner does
    // carry over (spec 6: violation hits the inheritance for future devices).
    if o.violation_bad > 0 || score(0, 0, inherited.0, inherited.1) >= score(0, 0, base.0, base.1) { inherited } else { base }
}

fn ensure_meta(chain: &Chain, subject: &str, owner: Option<&str>, epoch: u64) -> Meta {
    if let Some(m) = get::<Meta>(chain, &meta_key(subject)) { return m; }
    let owner = owner.map(str::to_owned).unwrap_or_else(|| derive_owner(subject));
    let (prior_g, prior_b) = new_prior(chain, &owner, false, epoch);
    let m = Meta { owner: owner.clone(), created_epoch: epoch, prior_g, prior_b, attested: false, last_work_epoch: None };
    put(chain, &meta_key(subject), &m);
    let _ = chain.store.state_set(&dev_key(&owner, subject), epoch.to_string().as_bytes());
    m
}

fn load(chain: &Chain, subject: &str, d: Domain, meta: &Meta) -> DomainRec {
    get(chain, &rec_key(subject, d)).unwrap_or(DomainRec {
        prior_g: meta.prior_g, prior_b: meta.prior_b, ..Default::default()
    })
}

fn push_event(chain: &Chain, subject: &str, e: Event) {
    let k = ev_key(subject);
    let mut v: Vec<Event> = get(chain, &k).unwrap_or_default();
    v.push(e);
    if v.len() > RECENT_EVENTS { let cut = v.len() - RECENT_EVENTS; v.drain(..cut); }
    put(chain, &k, &v);
}

/// Apply one event. `diminishing`: the n-th event of this kind in a day counts 1/n.
/// Returns false (and writes nothing) before the gate.
#[allow(clippy::too_many_arguments)]
pub fn apply(chain: &Chain, subject: &str, owner: Option<&str>, d: Domain, kind: &str,
             good: u64, bad: u64, diminishing: bool, epoch: u64) -> bool {
    if !active(epoch) || subject.is_empty() { return false; }
    ensure_migrated(chain, epoch);
    let meta = ensure_meta(chain, subject, owner, epoch);
    let mut r = load(chain, subject, d, &meta);
    r.decay_to(epoch);
    let (mut g, mut b) = (good, bad);
    if diminishing {
        let day = epoch / DECAY_EPOCHS;
        if r.day != day { r.day = day; r.day_counts.clear(); }
        let c = r.day_counts.entry(kind.to_owned()).or_insert(0);
        g /= *c + 1;
        b /= *c + 1;
        *c += 1;
    }
    r.good = r.good.saturating_add(g);
    r.bad = r.bad.saturating_add(b);
    put(chain, &rec_key(subject, d), &r);
    push_event(chain, subject, Event { epoch, domain: d.as_str().into(), kind: kind.into(), good: g, bad: b });
    if good > 0 && meta.last_work_epoch.is_none_or(|e| e < epoch) {
        let mut m = meta;
        m.last_work_epoch = Some(epoch);
        put(chain, &meta_key(subject), &m);
    }
    true
}

// ── Views (pure; decay applied to a copy) ───────────────────────────────────────
/// A domain record as of `epoch`, if the subject has one.
pub fn domain_view(chain: &Chain, subject: &str, d: Domain, epoch: u64) -> Option<DomainRec> {
    let mut r: DomainRec = get(chain, &rec_key(subject, d))?;
    r.decay_to(epoch);
    Some(r)
}

/// The subject's score in one domain (its prior's score if it has no record yet).
pub fn score_in(chain: &Chain, subject: &str, d: Domain, epoch: u64) -> u64 {
    if let Some(r) = domain_view(chain, subject, d, epoch) { return r.score(); }
    let (g, b) = match get::<Meta>(chain, &meta_key(subject)) {
        Some(m) => (m.prior_g, m.prior_b),
        None => new_prior(chain, &derive_owner(subject), false, epoch),
    };
    score(0, 0, g, b)
}

/// (composite, evidence) as of `epoch`: evidence-weighted average over the domains the
/// subject has evidence in. No evidence: the data-domain prior score, evidence 0.
pub fn composite(chain: &Chain, subject: &str, epoch: u64) -> (u64, u64) {
    let (mut num, mut den) = (0u128, 0u128);
    for d in Domain::ALL {
        if let Some(r) = domain_view(chain, subject, d, epoch) {
            let n = r.evidence();
            if n == 0 { continue; }
            num += r.score() as u128 * n as u128;
            den += n as u128;
        }
    }
    if den == 0 { return (score_in(chain, subject, Domain::Data, epoch), 0); }
    ((num / den) as u64, den.min(u64::MAX as u128) as u64)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct OwnerView {
    pub account: String,
    pub score: u64,
    pub confidence: u64,
    pub evidence: u64,
    pub violation_bad: u64,
    pub account_violation_floor: bool,
    pub devices: Vec<serde_json::Value>,
}

/// Owner score: evidence-weighted average of the counted device composites (devices count
/// after OWNER_COUNT_DELAY), with the shared violation penalty as a zero-score member, capped
/// by the best device's confidence. No counted evidence: score 0.
pub fn owner_view(chain: &Chain, owner: &str, epoch: u64) -> OwnerView { owner_view_x(chain, owner, epoch, true) }

/// `detail`: also list every device (the API). Without it, devices not yet counted are
/// skipped (inheritance and confirmations only need the score).
fn owner_view_x(chain: &Chain, owner: &str, epoch: u64, detail: bool) -> OwnerView {
    let pre = format!("rep_dev:{owner}:");
    let (mut num, mut den, mut best_conf) = (0u128, 0u128, 0u64);
    let mut devices = Vec::new();
    for (k, v) in chain.store.state_scan_prefix(&pre) {
        let subject = &k[pre.len()..];
        let created: u64 = String::from_utf8(v).ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        let counted = epoch >= created.saturating_add(OWNER_COUNT_DELAY);
        if !counted && !detail { continue; }
        let (c, n) = composite(chain, subject, epoch);
        let conf = confidence(n);
        if counted && n > 0 {
            num += c as u128 * n as u128;
            den += n as u128;
            best_conf = best_conf.max(conf);
        }
        devices.push(serde_json::json!({ "subject": subject, "composite": c, "evidence": n,
            "confidence": conf, "counted": counted, "created_epoch": created }));
    }
    let violation_bad = get::<DomainRec>(chain, &owner_key(owner)).map(|mut r| { r.decay_to(epoch); r.bad }).unwrap_or(0);
    let acct = get::<DomainRec>(chain, &acct_key(owner)).map(|mut r| { r.decay_to(epoch); r.bad }).unwrap_or(0);
    den += violation_bad as u128 + acct as u128;
    let avg = if den == 0 { 0 } else { (num / den) as u64 };
    let evidence = den.min(u64::MAX as u128) as u64;
    let floor = acct > 0 && acct as u128 * 10_000 >= ACCOUNT_VIOLATION_BAD as u128 * ACCOUNT_VIOLATION_FLOOR_BPS as u128;
    OwnerView { account: owner.to_owned(), score: if floor { 0 } else { avg.min(best_conf) },
                confidence: confidence(evidence), evidence, violation_bad: violation_bad + acct, account_violation_floor: floor, devices }
}

pub fn owner_score(chain: &Chain, owner: &str, epoch: u64) -> u64 { owner_view_x(chain, owner, epoch, false).score }

/// Reward multiplier in bps, 2,500..10,000, by the subject's domain score. 10,000 (no
/// change) before the gate.
pub fn reward_weight_bps(chain: &Chain, subject: &str, d: Domain, epoch: u64) -> u64 {
    if !active(epoch) { return MAX_REWARD_BPS; }
    let s = score_in(chain, subject, d, epoch).min(10_000);
    MIN_REWARD_BPS + (MAX_REWARD_BPS - MIN_REWARD_BPS) * s / 10_000
}

/// GET /api/reputation/:subject
pub fn subject_json(chain: &Chain, subject: &str, epoch: u64) -> serde_json::Value {
    let mut domains = serde_json::Map::new();
    for d in Domain::ALL {
        if let Some(r) = domain_view(chain, subject, d, epoch) {
            domains.insert(d.as_str().into(), serde_json::json!({
                "score": r.score(), "confidence": confidence(r.evidence()),
                "good": r.good, "bad": r.bad, "prior_good": r.prior_g, "prior_bad": r.prior_b,
                "last_decay_epoch": r.last_decay_epoch,
                "reward_weight_bps": reward_weight_bps(chain, subject, d, epoch) }));
        }
    }
    let (c, n) = composite(chain, subject, epoch);
    let meta = get::<Meta>(chain, &meta_key(subject));
    let recent: Vec<Event> = get(chain, &ev_key(subject)).unwrap_or_default();
    serde_json::json!({ "subject": subject, "owner": owner_of(chain, subject), "epoch": epoch,
        "active": active(epoch), "active_from_epoch": activation(),
        "composite": c, "confidence": confidence(n), "evidence": n, "unit": UNIT,
        "created_epoch": meta.as_ref().map(|m| m.created_epoch),
        "attested": meta.as_ref().map(|m| m.attested).unwrap_or(false),
        "domains": domains, "recent": recent, "slashes": get::<serde_json::Value>(chain, &slash_key(subject)) })
}

/// GET /api/reputation/owner/:account
pub fn owner_json(chain: &Chain, account: &str, epoch: u64) -> serde_json::Value {
    let o = owner_view(chain, account, epoch);
    serde_json::json!({ "account": o.account, "epoch": epoch, "active": active(epoch),
        "active_from_epoch": activation(), "score": o.score, "confidence": o.confidence,
        "evidence": o.evidence, "violation_bad": o.violation_bad, "account_violation_floor": o.account_violation_floor,
        "account_violation": get::<serde_json::Value>(chain, &acct_key(account)).is_some(),
        "slashes": get::<serde_json::Value>(chain, &slash_key(account)),
        "unit": UNIT, "devices": o.devices })
}

// ── Typed event hooks (spec §3). All no-ops before the gate. ────────────────────
/// The confirming/contradicting owner counts: another owner with score >= threshold.
fn qualifies(chain: &Chain, other_owner: &str, owner: &str, epoch: u64) -> bool {
    other_owner != owner && owner_score(chain, other_owner, epoch) >= CONFIRM_MIN_OWNER_SCORE
}

/// Reading accepted as useful (novel/changed, proven): data +small, diminishing per day.
pub fn on_useful(chain: &Chain, subject: &str, owner: &str, epoch: u64) -> bool {
    apply(chain, subject, Some(owner), Domain::Data, "useful", SMALL, 0, true, epoch)
}

/// Reading confirmed by `confirmer_owner`: data +medium, only from an independent owner
/// with reputation; one owner's devices never confirm each other.
pub fn on_confirmed(chain: &Chain, subject: &str, owner: &str, confirmer_owner: &str, epoch: u64) -> bool {
    if !active(epoch) || !qualifies(chain, confirmer_owner, owner, epoch) { return false; }
    apply(chain, subject, Some(owner), Domain::Data, "confirmed", MEDIUM, 0, false, epoch)
}

/// Contradicted by independent owners: data -large (once), if any contradictor qualifies.
pub fn on_contradicted(chain: &Chain, subject: &str, owner: &str, by: &[String], epoch: u64) -> bool {
    if !active(epoch) || !by.iter().any(|o| qualifies(chain, o, owner, epoch)) { return false; }
    apply(chain, subject, Some(owner), Domain::Data, "contradicted", 0, LARGE, false, epoch)
}

/// Self-consistent across the owner's own radios: +small evidence quality, not confirmation.
/// Hook only (needs the cross-radio match on chain).
pub fn on_self_consistent(chain: &Chain, subject: &str, owner: &str, d: Domain, epoch: u64) -> bool {
    apply(chain, subject, Some(owner), d, "self_consistent", SMALL, 0, true, epoch)
}

/// An alert, judged against the capable devices of OTHER owners that were present.
/// Someone else capable saw it: +medium. Capable others present and none saw it: -medium.
/// Nobody else capable present: no change (absent witness is never a penalty).
/// Hook only (alerts are not on chain yet).
pub fn on_alert(chain: &Chain, subject: &str, owner: &str, capable_present: &[String], saw: &[String], epoch: u64) -> bool {
    if !active(epoch) { return false; }
    let others: BTreeSet<&String> = capable_present.iter().filter(|o| o.as_str() != owner).collect();
    if others.is_empty() { return false; }
    if saw.iter().any(|o| others.contains(o) && qualifies(chain, o, owner, epoch)) {
        apply(chain, subject, Some(owner), Domain::Alert, "alert_seen", MEDIUM, 0, false, epoch)
    } else if saw.iter().all(|o| !others.contains(o)) {
        apply(chain, subject, Some(owner), Domain::Alert, "alert_unseen", 0, MEDIUM, false, epoch)
    } else { false }
}

/// A persistent threat later re-seen by anyone: alert +medium (deferred). Hook only.
pub fn on_threat_reseen(chain: &Chain, subject: &str, owner: &str, epoch: u64) -> bool {
    apply(chain, subject, Some(owner), Domain::Alert, "threat_reseen", MEDIUM, 0, false, epoch)
}

/// Physical challenge passed (+medium) / failed (-large). Hook only.
pub fn on_challenge(chain: &Chain, subject: &str, owner: &str, passed: bool, epoch: u64) -> bool {
    if passed { apply(chain, subject, Some(owner), Domain::Data, "challenge_pass", MEDIUM, 0, false, epoch) }
    else { apply(chain, subject, Some(owner), Domain::Data, "challenge_fail", 0, LARGE, false, epoch) }
}

/// Random audit passed (+medium) / failed (-very large), any domain. Hook only.
pub fn on_audit(chain: &Chain, subject: &str, owner: &str, d: Domain, passed: bool, epoch: u64) -> bool {
    if passed { apply(chain, subject, Some(owner), d, "audit_pass", MEDIUM, 0, false, epoch) }
    else { apply(chain, subject, Some(owner), d, "audit_fail", 0, VERY_LARGE, false, epoch) }
}

/// Storage proof passed (+small, diminishing) / strike (-medium).
pub fn on_storage_proof(chain: &Chain, subject: &str, owner: &str, passed: bool, epoch: u64) -> bool {
    if passed { apply(chain, subject, Some(owner), Domain::Storage, "storage_proof", SMALL, 0, true, epoch) }
    else { apply(chain, subject, Some(owner), Domain::Storage, "storage_strike", 0, MEDIUM, false, epoch) }
}

/// Inference job completed (+small) / failed or rejected (-medium).
pub fn on_inference(chain: &Chain, subject: &str, ok: bool, epoch: u64) -> bool {
    if ok { apply(chain, subject, None, Domain::Compute, "job_ok", SMALL, 0, false, epoch) }
    else { apply(chain, subject, None, Domain::Compute, "job_failed", 0, MEDIUM, false, epoch) }
}

/// Seal in quorum / certificate on time: consensus +small, diminishing per day.
pub fn on_seal(chain: &Chain, subject: &str, epoch: u64) -> bool {
    apply(chain, subject, None, Domain::Consensus, "seal", SMALL, 0, true, epoch)
}

/// Missed spot check / decide strike: consensus -medium.
pub fn on_missed_check(chain: &Chain, subject: &str, why: &str, epoch: u64) -> bool {
    let kind = format!("strike:{why}");
    apply(chain, subject, None, Domain::Consensus, &kind, 0, MEDIUM, false, epoch)
}

/// Oracle report within band (+small, diminishing) / outside (-medium).
pub fn on_oracle(chain: &Chain, subject: &str, within_band: bool, epoch: u64) -> bool {
    if within_band { apply(chain, subject, None, Domain::Oracle, "in_band", SMALL, 0, true, epoch) }
    else { apply(chain, subject, None, Domain::Oracle, "outlier", 0, MEDIUM, false, epoch) }
}

/// A relayed entry that was certified: relay +small, diminishing.
pub fn on_relay(chain: &Chain, subject: &str, epoch: u64) -> bool {
    apply(chain, subject, None, Domain::Relay, "relayed", SMALL, 0, true, epoch)
}

/// Impossible travel: data -large (the caller voids the window). Hook only.
pub fn on_teleport(chain: &Chain, subject: &str, owner: &str, epoch: u64) -> bool {
    apply(chain, subject, Some(owner), Domain::Data, "teleport", 0, LARGE, false, epoch)
}

/// Locked / attested hardware: raise the subject's prior to the locked prior (never lowers).
/// Hook only (attestation is not on chain yet).
pub fn set_attested(chain: &Chain, subject: &str, owner: &str, epoch: u64) -> bool {
    if !active(epoch) { return false; }
    let mut m = ensure_meta(chain, subject, Some(owner), epoch);
    m.attested = true;
    m.prior_g = m.prior_g.max(PRIOR_LOCKED_G);
    m.prior_b = m.prior_b.min(PRIOR_LOCKED_B).max(1);
    put(chain, &meta_key(subject), &m);
    for d in Domain::ALL {
        if let Some(mut r) = get::<DomainRec>(chain, &rec_key(subject, d)) {
            r.prior_g = m.prior_g; r.prior_b = m.prior_b;
            put(chain, &rec_key(subject, d), &r);
        }
    }
    true
}

/// Proven violation: every domain of the device restarts from zero good evidence with
/// VIOLATION_BAD and the open prior (no inheritance, no attestation); the owner takes
/// OWNER_VIOLATION_BAD as a shared penalty, which also lowers what future devices inherit.
pub fn on_violation(chain: &Chain, subject: &str, owner: Option<&str>, kind: ViolationKind, d: Domain, epoch: u64) -> bool {
    if !active(epoch) { return false; }
    ensure_migrated(chain, epoch);
    let mut m = ensure_meta(chain, subject, owner, epoch);
    m.prior_g = PRIOR_OPEN_G; m.prior_b = PRIOR_OPEN_B; m.attested = false;
    put(chain, &meta_key(subject), &m);
    for x in Domain::ALL {
        let k = rec_key(subject, x);
        if x != d && chain.store.state_get(&k).is_none() { continue; }
        let r = DomainRec { good: 0, bad: VIOLATION_BAD, prior_g: PRIOR_OPEN_G, prior_b: PRIOR_OPEN_B,
            last_decay_epoch: (epoch / DECAY_EPOCHS) * DECAY_EPOCHS, day: 0, day_counts: BTreeMap::new() };
        put(chain, &k, &r);
    }
    let ok = owner_key(&m.owner);
    let mut o: DomainRec = get(chain, &ok).unwrap_or_default();
    o.decay_to(epoch);
    o.bad = o.bad.saturating_add(OWNER_VIOLATION_BAD);
    put(chain, &ok, &o);
    push_event(chain, subject, Event { epoch, domain: d.as_str().into(), kind: format!("violation:{}", kind.as_str()), good: 0, bad: VIOLATION_BAD });
    // Violation also kills the stake backing this device (any role, any staker), to recycle.
    let slashed = slash_stakes(chain, |j| j["node"].as_str() == Some(subject));
    log_slash(chain, subject, &format!("device:{}", kind.as_str()), slashed, epoch);
    // Coordinated: enough of this owner's devices with proven violations within the window.
    let _ = chain.store.state_set(&violation_key(&m.owner, subject), epoch.to_string().as_bytes());
    let (violations, threshold) = coordinated(chain, &m.owner, epoch);
    if violations >= threshold { account_violation(chain, &m.owner, "coordinated", epoch); }
    true
}

/// Account-key-signed violation evidence: account-wide on its own. Hook only (no such evidence
/// type on chain yet). Returns the hunits slashed.
pub fn on_account_violation(chain: &Chain, owner: &str, kind: ViolationKind, epoch: u64) -> u64 {
    if !active(epoch) { return 0; }
    ensure_migrated(chain, epoch);
    account_violation(chain, owner, &format!("account_key:{}", kind.as_str()), epoch)
}

/// (proven violations on the owner's devices within ACCOUNT_VIOLATION_WINDOW, account-wide threshold).
pub fn coordinated(chain: &Chain, owner: &str, epoch: u64) -> (u64, u64) {
    let in_window = |e: u64| epoch.saturating_sub(e) < ACCOUNT_VIOLATION_WINDOW;
    let fpre = format!("rep_violation:{owner}:");
    let violations = chain.store.state_scan_prefix(&fpre).into_iter()
        .filter(|(_, v)| std::str::from_utf8(v).ok().and_then(|s| s.parse::<u64>().ok()).is_some_and(in_window))
        .count() as u64;
    let dpre = format!("rep_dev:{owner}:");
    let active_devices = chain.store.state_scan_prefix(&dpre).into_iter()
        .filter(|(k, _)| get::<Meta>(chain, &meta_key(&k[dpre.len()..]))
            .and_then(|m| m.last_work_epoch).is_some_and(in_window))
        .count() as u64;
    let share = (active_devices * ACCOUNT_VIOLATION_SHARE_BPS).div_ceil(10_000);
    (violations, ACCOUNT_VIOLATION_MIN_DEVICES.max(share))
}

/// Account-wide violation: slash 100% of every stake the account placed (any device, any role)
/// to recycle, and reset the owner to the violation floor. Returns the hunits slashed.
fn account_violation(chain: &Chain, owner: &str, why: &str, epoch: u64) -> u64 {
    let slashed = slash_stakes(chain, |j| j["staker"].as_str() == Some(owner));
    let k = acct_key(owner);
    let mut r: DomainRec = get(chain, &k).unwrap_or_default();
    r.decay_to(epoch);
    r.bad = r.bad.max(ACCOUNT_VIOLATION_BAD);
    put(chain, &k, &r);
    log_slash(chain, owner, &format!("account:{why}"), slashed, epoch);
    slashed
}

/// Move 100% of every matching `role_stake:*` record to the recycle fund and delete it.
/// Supply-neutral: the recycle fund gets exactly the stake removed. Deterministic: keys are
/// scanned in order and matched on the record's own `node` / `staker`.
fn slash_stakes(chain: &Chain, matches: impl Fn(&serde_json::Value) -> bool) -> u64 {
    let mut hit = Vec::new();
    let mut total: u64 = 0;
    for (k, v) in chain.store.state_scan_prefix("role_stake:") {
        let Ok(j) = serde_json::from_slice::<serde_json::Value>(&v) else { continue };
        if !matches(&j) { continue; }
        total = total.saturating_add(j["amount"].as_u64().unwrap_or(0));
        hit.push(k);
    }
    if total > 0 && chain.store.credit(hone_types::RECYCLE_FUND_ACCOUNT, hone_types::NATIVE_TOKEN, total).is_err() {
        return 0;
    }
    for k in hit { let _ = chain.store.state_delete(&k); }
    total
}

fn log_slash(chain: &Chain, who: &str, scope: &str, amount: u64, epoch: u64) {
    let k = slash_key(who);
    let mut v: Vec<serde_json::Value> = get(chain, &k).unwrap_or_default();
    v.push(serde_json::json!({ "epoch": epoch, "scope": scope, "amount": amount }));
    if v.len() > SLASH_LOG { let cut = v.len() - SLASH_LOG; v.drain(..cut); }
    put(chain, &k, &v);
}

// ── Sync before submit: per-device committed head (all devices) ────────────────
/// The head of a device's committed log: every SensorDataCommit / SensorSummary of the
/// device (keyed by its sensor id) is one link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceHead { pub seq: u64, pub head_hash: String, pub epoch: u64 }

fn head_key(dev: &str) -> String { format!("dev_head:{dev}") }
fn seq_key(dev: &str, seq: u64) -> String { format!("dev_seq:{dev}:{seq:016x}") }
fn equiv_key(dev: &str, seq: u64) -> String { format!("dev_equiv:{dev}:{seq:016x}") }

pub fn device_head(chain: &Chain, dev: &str) -> Option<DeviceHead> { get(chain, &head_key(dev)) }

/// GET /api/device/:key/head
pub fn device_head_json(chain: &Chain, dev: &str, epoch: u64) -> serde_json::Value {
    serde_json::json!({ "key": dev, "head": device_head(chain, dev), "active": active(epoch),
        "active_from_epoch": activation(),
        "rule": "next commit: seq > head.seq and prev_head == head.head_hash (first commit: no prev_head)" })
}

/// Before a device commit's effects: it must extend the committed head (prev_head ==
/// head_hash and seq > head.seq; a skipped seq is a declared gap, still hash-linked). A
/// non-extending commit is rejected, never slashed. A second, different signed commit at a
/// seq the chain already holds is equivocation: recorded once as a violation, and rejected.
#[allow(clippy::too_many_arguments)]
pub fn device_check(chain: &Chain, dev: &str, owner: &str, seq: Option<u64>, prev: Option<&str>,
                    hash: &str, epoch: u64) -> anyhow::Result<()> {
    if !active(epoch) { return Ok(()); }
    let Some(seq) = seq else {
        anyhow::bail!("device '{dev}': commit has no seq/prev_head; fetch GET /api/device/{dev}/head and resync");
    };
    if let Some(prev_hash) = chain.store.state_get(&seq_key(dev, seq)).and_then(|b| String::from_utf8(b).ok()) {
        if prev_hash != hash {
            on_device_equivocation(chain, dev, owner, seq, epoch);
            anyhow::bail!("device '{dev}': equivocation, a different commit is already on chain at seq {seq}");
        }
        anyhow::bail!("device '{dev}': seq {seq} is already committed");
    }
    match device_head(chain, dev) {
        None => anyhow::ensure!(prev.is_none_or(str::is_empty),
            "device '{dev}': no committed head yet, prev_head must be empty; resync"),
        Some(h) => {
            anyhow::ensure!(seq > h.seq, "device '{dev}': stale commit seq {seq}, head is seq {}; resync", h.seq);
            anyhow::ensure!(prev == Some(h.head_hash.as_str()),
                "device '{dev}': commit does not extend the committed head (seq {}, {}); resync", h.seq, h.head_hash);
        }
    }
    Ok(())
}

/// After a device commit applied: it is the new head.
pub fn device_record(chain: &Chain, dev: &str, seq: Option<u64>, hash: &str, epoch: u64) {
    if !active(epoch) { return; }
    let Some(seq) = seq else { return };
    let _ = chain.store.state_set(&seq_key(dev, seq), hash.as_bytes());
    put(chain, &head_key(dev), &DeviceHead { seq, head_hash: hash.to_owned(), epoch });
}

/// Two different signed commits for the same device seq: equivocation, once per (dev, seq).
/// Also the hook for the same proof seen in gossip.
pub fn on_device_equivocation(chain: &Chain, dev: &str, owner: &str, seq: u64, epoch: u64) -> bool {
    if !active(epoch) || chain.store.state_get(&equiv_key(dev, seq)).is_some() { return false; }
    let _ = chain.store.state_set(&equiv_key(dev, seq), epoch.to_string().as_bytes());
    on_violation(chain, dev, Some(owner), ViolationKind::Equivocation, Domain::Data, epoch)
}

// ── Migration (spec §8) and the seal tick ───────────────────────────────────────
/// Seal-time processing (from epoch_sync::epoch_tick). Decay is lazy, so this only runs
/// the one-time migration at activation.
pub fn tick(chain: &Chain, epoch: u64) {
    if !active(epoch) { return; }
    ensure_migrated(chain, epoch);
}

/// Seed `units` of evidence at the old score's ratio, but never more good evidence than
/// `proven` units of actually proven work (rule 1).
#[allow(clippy::too_many_arguments)]
fn seed(chain: &Chain, subject: &str, d: Domain, units: u64, ratio_num: u64, ratio_den: u64, proven: u64, epoch: u64) {
    if units == 0 || ratio_den == 0 { return; }
    let n = units.min(SEED_CAP_UNITS) * UNIT;
    let g = ((n as u128 * ratio_num.min(ratio_den) as u128 / ratio_den as u128) as u64)
        .min(proven.saturating_mul(UNIT)).min(n);
    let meta = ensure_meta(chain, subject, None, epoch);
    let mut r = load(chain, subject, d, &meta);
    r.decay_to(epoch);
    r.good = r.good.saturating_add(g);
    r.bad = r.bad.saturating_add(n - g);
    put(chain, &rec_key(subject, d), &r);
    push_event(chain, subject, Event { epoch, domain: d.as_str().into(), kind: "migrated".into(), good: g, bad: n - g });
}

/// Seed the compute and oracle domains from the current inference and oracle reputation,
/// once, at the first touch at or after activation. Evidence is the old record's sample
/// count (capped at SEED_CAP_UNITS); good/(good+bad) is its score, but good never exceeds
/// the proven work (completed jobs, in-band reports). The clock domain is NOT seeded from
/// Phase 8 `clock_uptime`: it measures registration/uptime, not proven work (rule 1), so
/// consensus reputation starts from the certified seals after activation.
pub fn ensure_migrated(chain: &Chain, epoch: u64) {
    if !active(epoch) || chain.store.state_get(MIGRATED_KEY).is_some() { return; }
    let _ = chain.store.state_set(MIGRATED_KEY, epoch.to_string().as_bytes());
    for (k, v) in chain.store.state_scan_prefix("node_rep:") {
        let Ok(r) = serde_json::from_slice::<crate::inference::NodeReputation>(&v) else { continue };
        seed(chain, &k["node_rep:".len()..], Domain::Compute, r.jobs_accepted, r.score.min(10_000), 10_000, r.jobs_completed, epoch);
    }
    for (k, v) in chain.store.state_scan_prefix("oracle_rep:") {
        let Ok(r) = serde_json::from_slice::<crate::oracle::ReporterReputation>(&v) else { continue };
        seed(chain, &k["oracle_rep:".len()..], Domain::Oracle, r.reports_total, r.score.clamp(0, 100) as u64, 100,
             r.reports_total.saturating_sub(r.outliers), epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Gate;
    impl Gate { fn on(e: u64) -> Gate { TEST_EPOCH.with(|c| c.set(Some(e))); Gate } }
    impl Drop for Gate { fn drop(&mut self) { TEST_EPOCH.with(|c| c.set(None)); } }

    fn chain() -> (Chain, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        (Chain::new(crate::store::Store::open(d.path()).unwrap(), "n".into(), "hone-test".into()), d)
    }
    fn dump(c: &Chain) -> Vec<(String, Vec<u8>)> { c.store.state_scan_prefix("") }
    fn rep_dump(c: &Chain) -> Vec<(String, Vec<u8>)> {
        dump(c).into_iter().filter(|(k, _)| k.starts_with("rep:") || k.starts_with("rep_")).collect()
    }
    fn dev(owner: &str, b: u8) -> String { format!("{owner}@{}", hex::encode([b; 32])) }

    const A: u64 = 400_000; // activation used by the tests
    const DAY: u64 = DECAY_EPOCHS;

    /// Give `owner` a counted, reputable device (100 good units, made a week before `at`).
    fn reputable(c: &Chain, owner: &str, b: u8, at: u64) -> String {
        let s = dev(owner, b);
        let made = at - 7 * DAY;
        assert!(apply(c, &s, Some(owner), Domain::Data, "seed", 100 * UNIT, 0, false, made));
        s
    }

    #[test]
    fn score_is_integer_lcb_and_new_devices_start_low_middle() {
        assert_eq!(score(0, 0, PRIOR_OPEN_G, PRIOR_OPEN_B), 3_500);
        assert_eq!(score(0, 0, PRIOR_LOCKED_G, PRIOR_LOCKED_B), 6_500);
        // Confidence grows with evidence; bad evidence pulls the score down.
        let lo = score(5 * UNIT, 0, PRIOR_OPEN_G, PRIOR_OPEN_B);
        let hi = score(100 * UNIT, 0, PRIOR_OPEN_G, PRIOR_OPEN_B);
        assert!(3_500 < lo && lo < hi && hi < 10_000, "{lo} {hi}");
        assert!(score(100 * UNIT, 30 * UNIT, PRIOR_OPEN_G, PRIOR_OPEN_B) < hi);
        assert_eq!(confidence(0), 0);
        assert_eq!(confidence(CONFIDENCE_HALF), 5_000);
    }

    #[test]
    fn gate_off_every_hook_is_a_no_op_and_state_is_byte_identical() {
        let (c, _d) = chain();
        let _ = c.store.state_set("node_rep:bob", br#"{"node_id":"bob","jobs_accepted":3,"jobs_completed":3,"jobs_failed":0,"total_latency_ms":30,"score":9000}"#);
        stake(&c, "clock", &dev("josh", 1), "josh", 100);
        stake(&c, "decide", "nodex", "josh", 30);
        let before = dump(&c);
        let s = dev("josh", 1);
        let e = 500_000; // any epoch: REPUTATION_EPOCH is u64::MAX
        assert_eq!(on_account_violation(&c, "josh", ViolationKind::Fabrication, e), 0);
        assert!(!on_useful(&c, &s, "josh", e));
        assert!(!on_confirmed(&c, &s, "josh", "bob", e));
        assert!(!on_contradicted(&c, &s, "josh", &["bob".into()], e));
        assert!(!on_self_consistent(&c, &s, "josh", Domain::Data, e));
        assert!(!on_alert(&c, &s, "josh", &["bob".into()], &[], e));
        assert!(!on_threat_reseen(&c, &s, "josh", e));
        assert!(!on_challenge(&c, &s, "josh", false, e));
        assert!(!on_audit(&c, &s, "josh", Domain::Storage, false, e));
        assert!(!on_storage_proof(&c, &s, "josh", true, e));
        assert!(!on_inference(&c, "bob", true, e));
        assert!(!on_seal(&c, "grouchly", e));
        assert!(!on_missed_check(&c, "grouchly", "x", e));
        assert!(!on_oracle(&c, "r", false, e));
        assert!(!on_relay(&c, "gw", e));
        assert!(!on_teleport(&c, &s, "josh", e));
        assert!(!set_attested(&c, &s, "josh", e));
        assert!(!on_violation(&c, &s, None, ViolationKind::Equivocation, Domain::Data, e));
        tick(&c, e);
        assert_eq!(reward_weight_bps(&c, &s, Domain::Data, e), MAX_REWARD_BPS);
        assert_eq!(dump(&c), before, "gate off: no key written or changed");
    }

    #[test]
    fn gate_off_usefulness_pays_and_writes_exactly_as_before() {
        // The usefulness flow with the gate off writes no rep state and pays unscaled points.
        let (c, _d) = chain();
        let e = crate::usefulness::decide_by_epoch() + 10;
        let w = crate::usefulness::unix_at(e) / 900;
        let m: BTreeMap<String, i64> = [("wifi.ap_count".to_string(), 27i64)].into();
        let cell = "896d75d960bffff";
        crate::usefulness::score(&c, "flip-a", "josh", cell, w, &m, e);
        crate::usefulness::release_due(&c, e + crate::usefulness::CLAIM_WINDOW_EPOCHS);
        assert!(rep_dump(&c).is_empty());
        assert_eq!(crate::usefulness::proven_points(&c, "josh"), crate::usefulness::NOVEL_POINTS);
    }

    #[test]
    fn usefulness_flow_with_gate_on_scales_pay_and_records_events() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 10 * DAY;
        let bob_dev = reputable(&c, "bob", 2, e);
        let w = crate::usefulness::unix_at(e) / 900;
        let m: BTreeMap<String, i64> = [("wifi.ap_count".to_string(), 27i64)].into();
        let cell = "896d75d960bffff";
        crate::usefulness::score(&c, "flip-a", "josh", cell, w, &m, e);
        // bob (another owner, reputable) reports the same value: a decide-by corroboration.
        crate::usefulness::score(&c, &bob_dev, "bob", cell, w, &m, e + 1);
        let due = e + crate::usefulness::CLAIM_WINDOW_EPOCHS;
        crate::usefulness::release_due(&c, due);
        let r = domain_view(&c, "flip-a", Domain::Data, due).unwrap();
        assert_eq!(r.good, SMALL + MEDIUM, "useful + one independent confirmation");
        assert_eq!(r.bad, 0);
        // Paid by reputation: a new device with no evidence is paid at its prior's weight.
        let w0 = MIN_REWARD_BPS + (MAX_REWARD_BPS - MIN_REWARD_BPS) * 3_500 / 10_000;
        assert_eq!(crate::usefulness::proven_points(&c, "josh"), crate::usefulness::NOVEL_POINTS * w0 / 10_000);
    }

    #[test]
    fn decay_is_x098_per_day_boundary_and_lazy_reads_match() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let s = dev("josh", 1);
        let e = A + 5; // day A/DAY, not a boundary
        apply(&c, &s, Some("josh"), Domain::Data, "t", 100 * UNIT, 10 * UNIT, false, e);
        let same_day = (e / DAY + 1) * DAY - 1;
        assert_eq!(domain_view(&c, &s, Domain::Data, same_day).unwrap().good, 100 * UNIT);
        let b1 = (e / DAY + 1) * DAY;
        let r1 = domain_view(&c, &s, Domain::Data, b1).unwrap();
        assert_eq!((r1.good, r1.bad), (98 * UNIT, 9_800));
        let r2 = domain_view(&c, &s, Domain::Data, b1 + DAY).unwrap();
        assert_eq!((r2.good, r2.bad), (98 * UNIT * 98 / 100, 9_800 * 98 / 100));
        // A write two days later sees the same decayed counters.
        apply(&c, &s, Some("josh"), Domain::Data, "t", 0, 0, false, b1 + DAY + 7);
        let stored: DomainRec = get(&c, &rec_key(&s, Domain::Data)).unwrap();
        assert_eq!((stored.good, stored.bad), (r2.good, r2.bad));
        assert_eq!(stored.last_decay_epoch, b1 + DAY);
        // An event dated before the last decay boundary never decays backwards.
        apply(&c, &s, Some("josh"), Domain::Data, "t", UNIT, 0, false, e);
        let again: DomainRec = get(&c, &rec_key(&s, Domain::Data)).unwrap();
        assert_eq!(again.good, r2.good + UNIT);
        assert_eq!(again.last_decay_epoch, b1 + DAY);
    }

    #[test]
    fn diminishing_events_count_one_over_n_per_day() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let s = dev("josh", 1);
        let e = A + 1;
        for _ in 0..4 { on_useful(&c, &s, "josh", e); }
        let r = domain_view(&c, &s, Domain::Data, e).unwrap();
        assert_eq!(r.good, UNIT + UNIT / 2 + UNIT / 3 + UNIT / 4);
        on_useful(&c, &s, "josh", e + DAY); // next day: full again (after one decay)
        let r2 = domain_view(&c, &s, Domain::Data, e + DAY).unwrap();
        assert_eq!(r2.good, r.good * 98 / 100 + UNIT);
    }

    #[test]
    fn owner_trust_is_the_main_source_of_a_new_device_trust() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 30 * DAY;
        // Established owner: a new device starts at about the owner's score, carried as
        // INHERIT_CAP evidence, with zero confidence of its own.
        reputable(&c, "josh", 1, e);
        let o = owner_view(&c, "josh", e);
        assert!(o.score > 7_000, "{o:?}");
        let f2 = dev("josh", 9);
        let s0 = score_in(&c, &f2, Domain::Data, e);
        assert!(s0.abs_diff(o.score) <= 2, "new josh device {s0} vs owner {}", o.score);
        on_useful(&c, &f2, "josh", e); // first event writes the inherited prior
        let m: Meta = get(&c, &meta_key(&f2)).unwrap();
        assert_eq!(m.prior_g + m.prior_b, INHERIT_CAP);
        assert_eq!(confidence(composite(&c, &dev("josh", 8), e).1), 0, "no evidence of its own");
        // Low confidence: one contradiction pulls it down fast.
        apply(&c, &f2, Some("josh"), Domain::Data, "contradicted", 0, LARGE, false, e);
        assert!(score_in(&c, &f2, Domain::Data, e) + 2_000 < s0);
        // A stranger's device starts neutral-low and must earn it.
        assert_eq!(score_in(&c, &dev("stranger", 3), Domain::Data, e), 3_500);
        assert!(s0 > 3_500 + 3_000);
        // An owner still short of evidence (confidence-capped score) gives no less than the
        // open prior: its device starts neutral-low like anyone's.
        let s = dev("amy", 4);
        apply(&c, &s, Some("amy"), Domain::Data, "seed", 5 * UNIT, 0, false, e - 7 * DAY);
        let oa = owner_view(&c, "amy", e);
        let a2 = dev("amy", 5);
        on_useful(&c, &a2, "amy", e);
        let ma: Meta = get(&c, &meta_key(&a2)).unwrap();
        assert!(oa.score < 3_500, "{oa:?}");
        assert_eq!((ma.prior_g, ma.prior_b), (PRIOR_OPEN_G, PRIOR_OPEN_B));
    }

    #[test]
    fn nothing_is_earned_for_existing_registering_or_uptime_alone() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        // Phase 8 uptime record of a clock that "sealed" every epoch: seeds nothing.
        let _ = c.store.state_set("clock_uptime:grouchly", br#"{"seals":100,"epochs":100}"#);
        tick(&c, A);
        tick(&c, A + 10 * DAY);
        assert!(domain_view(&c, "grouchly", Domain::Consensus, A + 10 * DAY).is_none());
        assert_eq!(composite(&c, "grouchly", A + 10 * DAY).1, 0);
        // A device that only exists (meta, attestation) has no evidence and no confidence.
        let d = dev("josh", 7);
        set_attested(&c, &d, "josh", A + 1);
        assert_eq!(composite(&c, &d, A + 100 * DAY).1, 0);
        assert_eq!(owner_view(&c, "josh", A + 100 * DAY).evidence, 0);
        // Old records seed no more good evidence than they proved.
        let nr = crate::inference::NodeReputation { node_id: "eve".into(), jobs_accepted: 10, jobs_completed: 2,
            jobs_failed: 0, total_latency_ms: 20, score: 10_000 };
        let (c2, _d2) = chain();
        let _ = c2.store.state_set("node_rep:eve", &serde_json::to_vec(&nr).unwrap());
        tick(&c2, A);
        let r = domain_view(&c2, "eve", Domain::Compute, A).unwrap();
        assert_eq!((r.good, r.bad), (2 * UNIT, 8 * UNIT));
    }

    #[test]
    fn owner_score_is_capped_by_best_device_confidence_and_new_devices_wait_a_day() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 30 * DAY;
        // Ten cheap devices with one good unit each: owner score stays at the best device's
        // (small) confidence, far below what one well-evidenced device earns.
        for b in 0..10u8 {
            apply(&c, &dev("cheap", b), Some("cheap"), Domain::Data, "t", UNIT, 0, false, e - 2 * DAY);
        }
        let ch = owner_view(&c, "cheap", e);
        assert!(ch.score <= confidence(UNIT * 98 / 100 * 98 / 100), "{ch:?}");
        reputable(&c, "solid", 1, e);
        assert!(owner_score(&c, "solid", e) > 3 * ch.score);
        // A device made now does not count toward its owner until OWNER_COUNT_DELAY.
        let n = dev("fresh", 1);
        apply(&c, &n, Some("fresh"), Domain::Data, "t", 100 * UNIT, 0, false, e);
        assert_eq!(owner_score(&c, "fresh", e + OWNER_COUNT_DELAY - 1), 0);
        assert!(owner_score(&c, "fresh", e + OWNER_COUNT_DELAY) > 0);
    }

    #[test]
    fn same_owner_and_low_reputation_confirmations_are_ignored() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 30 * DAY;
        reputable(&c, "josh", 1, e);
        let flip = dev("josh", 2);
        assert!(!on_confirmed(&c, &flip, "josh", "josh", e), "own devices never confirm each other");
        assert!(!on_confirmed(&c, &flip, "josh", "nobody", e), "owner without reputation");
        assert!(domain_view(&c, &flip, Domain::Data, e).is_none());
        reputable(&c, "bob", 3, e);
        assert!(on_confirmed(&c, &flip, "josh", "bob", e));
        assert_eq!(domain_view(&c, &flip, Domain::Data, e).unwrap().good, MEDIUM);
        // Contradictions: same rule.
        assert!(!on_contradicted(&c, &flip, "josh", &["josh".into(), "nobody".into()], e));
        assert!(on_contradicted(&c, &flip, "josh", &["nobody".into(), "bob".into()], e));
        assert_eq!(domain_view(&c, &flip, Domain::Data, e).unwrap().bad, LARGE);
        assert_eq!(LARGE, 3 * MEDIUM, "a contradiction costs 3x a confirmation");
    }

    #[test]
    fn absent_witness_is_no_penalty() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 30 * DAY;
        reputable(&c, "bob", 3, e);
        let flip = dev("josh", 2);
        // Nobody else capable present (or only josh's own devices): no change.
        assert!(!on_alert(&c, &flip, "josh", &[], &[], e));
        assert!(!on_alert(&c, &flip, "josh", &["josh".into()], &[], e));
        assert!(domain_view(&c, &flip, Domain::Alert, e).is_none());
        // Another owner capable and present, saw nothing: -medium.
        assert!(on_alert(&c, &flip, "josh", &["bob".into()], &[], e));
        assert_eq!(domain_view(&c, &flip, Domain::Alert, e).unwrap().bad, MEDIUM);
        // Another owner saw it too: +medium.
        assert!(on_alert(&c, &flip, "josh", &["bob".into()], &["bob".into()], e));
        assert_eq!(domain_view(&c, &flip, Domain::Alert, e).unwrap().good, MEDIUM);
    }

    #[test]
    fn violation_resets_the_device_hits_the_owner_and_future_inheritance() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 30 * DAY;
        let good_dev = reputable(&c, "josh", 1, e);
        let bad_dev = dev("josh", 2);
        apply(&c, &bad_dev, Some("josh"), Domain::Data, "t", 50 * UNIT, 0, false, e - 7 * DAY);
        apply(&c, &bad_dev, Some("josh"), Domain::Compute, "t", 50 * UNIT, 0, false, e - 7 * DAY);
        let before = owner_view(&c, "josh", e);
        let inherit_before = new_prior(&c, "josh", false, e);
        assert!(on_violation(&c, &bad_dev, None, ViolationKind::Equivocation, Domain::Data, e));
        for d in [Domain::Data, Domain::Compute] {
            let r = domain_view(&c, &bad_dev, d, e).unwrap();
            assert_eq!((r.good, r.bad, r.prior_g, r.prior_b), (0, VIOLATION_BAD, PRIOR_OPEN_G, PRIOR_OPEN_B));
        }
        assert!(domain_view(&c, &bad_dev, Domain::Storage, e).is_none());
        let after = owner_view(&c, "josh", e);
        assert_eq!(after.violation_bad, OWNER_VIOLATION_BAD);
        assert!(after.score < before.score);
        let inherit_after = new_prior(&c, "josh", false, e);
        assert!(inherit_after.0 * 10_000 / (inherit_after.0 + inherit_after.1)
              < inherit_before.0 * 10_000 / (inherit_before.0 + inherit_before.1));
        // The honest device keeps its own record.
        assert_eq!(domain_view(&c, &good_dev, Domain::Data, e).unwrap().bad, 0);
    }

    #[test]
    fn reward_weight_is_between_quarter_and_full() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 1;
        let (good, bad) = (dev("a", 1), dev("b", 2));
        apply(&c, &good, None, Domain::Data, "t", 10_000 * UNIT, 0, false, e);
        apply(&c, &bad, None, Domain::Data, "t", 0, 10_000 * UNIT, false, e);
        let (wg, wb) = (reward_weight_bps(&c, &good, Domain::Data, e), reward_weight_bps(&c, &bad, Domain::Data, e));
        assert!(wg > 9_900 && wg <= MAX_REWARD_BPS, "{wg}");
        assert_eq!(wb, MIN_REWARD_BPS);
        assert_eq!(reward_weight_bps(&c, &good, Domain::Data, A - 1), MAX_REWARD_BPS, "gate off");
    }

    #[test]
    fn migration_seeds_match_current_inference_and_oracle_values_and_uptime_seeds_nothing() {
        let (c, _d) = chain();
        let nr = crate::inference::NodeReputation { node_id: "bob".into(), jobs_accepted: 10, jobs_completed: 8,
            jobs_failed: 2, total_latency_ms: 800, score: 8_000 };
        let _ = c.store.state_set("node_rep:bob", &serde_json::to_vec(&nr).unwrap());
        let _ = c.store.state_set("oracle_rep:rita", br#"{"reporter":"rita","score":90,"reports_total":20,"outliers":2}"#);
        let _ = c.store.state_set("clock_uptime:grouchly", br#"{"seals":90,"epochs":100}"#);
        tick(&c, A); // gate off: nothing
        assert!(rep_dump(&c).is_empty());
        let _g = Gate::on(A);
        tick(&c, A);
        let ratio = |s: &str, d: Domain| { let r = domain_view(&c, s, d, A).unwrap(); (r.good * 10_000 / r.evidence(), r.evidence()) };
        assert_eq!(ratio("bob", Domain::Compute), (8_000, 10 * UNIT));
        assert_eq!(ratio("rita", Domain::Oracle), (9_000, 20 * UNIT));
        assert!(domain_view(&c, "grouchly", Domain::Consensus, A).is_none(), "uptime seeds nothing");
        // Once only.
        let snap = rep_dump(&c);
        tick(&c, A + 1);
        on_seal(&c, "other", A + 1);
        assert_eq!(domain_view(&c, "bob", Domain::Compute, A + 1).unwrap().evidence(), 10 * UNIT);
        assert!(rep_dump(&c).len() > snap.len());
    }

    #[test]
    fn same_decided_events_give_identical_state_on_every_node() {
        let _g = Gate::on(A);
        let (c1, _d1) = chain();
        let (c2, _d2) = chain();
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut events = Vec::new();
        for i in 0..400u64 {
            x ^= x << 13; x ^= x >> 7; x ^= x << 17;
            events.push((i * 97 + A, (x % 6) as u8, (x >> 8) % 4, (x >> 16) % 3 == 0));
        }
        for c in [&c1, &c2] {
            for (e, kind, who, flag) in &events {
                let s = dev(["josh", "bob", "amy", "joe"][*who as usize], *kind);
                let o = derive_owner(&s);
                match kind {
                    0 => { on_useful(c, &s, &o, *e); }
                    1 => { on_confirmed(c, &s, &o, "bob", *e); }
                    2 => { on_contradicted(c, &s, &o, &["amy".into()], *e); }
                    3 => { on_storage_proof(c, &s, &o, *flag, *e); }
                    4 => { on_seal(c, &s, *e); }
                    _ => { if *flag { on_violation(c, &s, None, ViolationKind::Equivocation, Domain::Consensus, *e); } else { on_oracle(c, &s, true, *e); } }
                }
            }
        }
        let (a, b) = (rep_dump(&c1), rep_dump(&c2));
        assert!(!a.is_empty());
        assert_eq!(a, b);
        assert_eq!(subject_json(&c1, &dev("josh", 0), A + 50_000), subject_json(&c2, &dev("josh", 0), A + 50_000));
        assert_eq!(owner_json(&c1, "bob", A + 50_000), owner_json(&c2, "bob", A + 50_000));
    }

    fn stake(c: &Chain, role: &str, node: &str, staker: &str, amount: u64) {
        let _ = c.store.state_set(&format!("role_stake:{role}:{node}:{staker}"), &serde_json::to_vec(&serde_json::json!({
            "node": node, "role": role, "staker": staker, "amount": amount, "staked_epoch": 1 })).unwrap());
    }
    fn staked(c: &Chain) -> u64 {
        c.store.state_scan_prefix("role_stake:").into_iter()
            .filter_map(|(_, v)| serde_json::from_slice::<serde_json::Value>(&v).ok()?["amount"].as_u64()).sum()
    }
    fn recycle(c: &Chain) -> u64 { c.store.get_balance(hone_types::RECYCLE_FUND_ACCOUNT, hone_types::NATIVE_TOKEN) }
    fn has_stake(c: &Chain, role: &str, node: &str, staker: &str) -> bool {
        c.store.state_get(&format!("role_stake:{role}:{node}:{staker}")).is_some()
    }
    /// josh: two active devices with stakes, plus a stake on someone else's node.
    fn violation_setup(c: &Chain, e: u64) -> (String, String) {
        let (d1, d2) = (dev("josh", 1), dev("josh", 2));
        on_useful(c, &d1, "josh", e - 10);
        on_useful(c, &d2, "josh", e - 10);
        stake(c, "clock", &d1, "josh", 100);
        stake(c, "decide", &d1, "bob", 50);
        stake(c, "clock", &d2, "josh", 70);
        stake(c, "decide", "nodex", "josh", 30);
        stake(c, "decide", "nodey", "bob", 40);
        (d1, d2)
    }

    #[test]
    fn device_violation_slashes_all_stake_on_that_device_only() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 30 * DAY;
        let (d1, _) = violation_setup(&c, e);
        let total = staked(&c) + recycle(&c);
        let r0 = recycle(&c);
        assert!(on_violation(&c, &d1, None, ViolationKind::Equivocation, Domain::Data, e));
        assert_eq!(recycle(&c) - r0, 150, "recycle receives exactly the device's stake");
        assert!(!has_stake(&c, "clock", &d1, "josh") && !has_stake(&c, "decide", &d1, "bob"));
        assert!(has_stake(&c, "clock", &dev("josh", 2), "josh"), "owner's other device untouched");
        assert!(has_stake(&c, "decide", "nodex", "josh") && has_stake(&c, "decide", "nodey", "bob"));
        assert_eq!(staked(&c) + recycle(&c), total, "supply conserved");
        assert!(c.store.state_get(&acct_key("josh")).is_none(), "one violation of two devices stays per device");
    }

    #[test]
    fn two_device_owner_with_two_violations_in_window_goes_account_wide() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 30 * DAY;
        let (d1, d2) = violation_setup(&c, e);
        let total = staked(&c) + recycle(&c);
        let r0 = recycle(&c);
        on_violation(&c, &d1, None, ViolationKind::Equivocation, Domain::Data, e);
        on_violation(&c, &d2, None, ViolationKind::Equivocation, Domain::Data, e + 100);
        assert_eq!(coordinated(&c, "josh", e + 100), (2, 2));
        assert_eq!(recycle(&c) - r0, 150 + 70 + 30, "both devices + every stake josh placed");
        assert!(!has_stake(&c, "decide", "nodex", "josh"));
        assert!(has_stake(&c, "decide", "nodey", "bob"), "other accounts' stakes elsewhere untouched");
        assert_eq!(staked(&c) + recycle(&c), total);
        // Owner reset: score 0, and a new device inherits the violation floor.
        let o = owner_view(&c, "josh", e + 100);
        assert!(o.account_violation_floor);
        assert_eq!(o.score, 0);
        assert_eq!(score_in(&c, &dev("josh", 3), Domain::Data, e + 100), 0);
        assert!(score_in(&c, &dev("stranger", 3), Domain::Data, e + 100) > 0);
    }

    #[test]
    fn two_violations_outside_the_window_stay_per_device() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 30 * DAY;
        let (d1, d2) = violation_setup(&c, e);
        let r0 = recycle(&c);
        on_violation(&c, &d1, None, ViolationKind::Equivocation, Domain::Data, e);
        on_violation(&c, &d2, None, ViolationKind::Equivocation, Domain::Data, e + ACCOUNT_VIOLATION_WINDOW);
        assert_eq!(recycle(&c) - r0, 150 + 70);
        assert!(has_stake(&c, "decide", "nodex", "josh"));
        assert!(c.store.state_get(&acct_key("josh")).is_none());
    }

    #[test]
    fn account_key_evidence_is_account_wide_on_its_own() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 30 * DAY;
        violation_setup(&c, e);
        let total = staked(&c) + recycle(&c);
        assert_eq!(on_account_violation(&c, "josh", ViolationKind::Fabrication, e), 100 + 70 + 30, "every stake josh placed");
        assert!(has_stake(&c, "decide", &dev("josh", 1), "bob"), "others' stake on josh's devices is not josh's");
        assert_eq!(staked(&c) + recycle(&c), total);
        assert_eq!(owner_score(&c, "josh", e), 0);
    }

    #[test]
    fn a_big_owner_needs_ten_percent_of_its_active_devices() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 30 * DAY;
        let devs: Vec<String> = (0..1_000u32).map(|i| format!("big@{:064x}", i)).collect();
        for d in &devs { on_useful(&c, d, "big", e - 10); }
        stake(&c, "decide", "elsewhere", "big", 500);
        let r0 = recycle(&c);
        for d in &devs[..99] { on_violation(&c, d, None, ViolationKind::Equivocation, Domain::Data, e); }
        assert_eq!(coordinated(&c, "big", e), (99, 100));
        assert!(has_stake(&c, "decide", "elsewhere", "big"), "99 of 1,000: per device only");
        assert!(c.store.state_get(&acct_key("big")).is_none());
        on_violation(&c, &devs[99], None, ViolationKind::Equivocation, Domain::Data, e);
        assert!(!has_stake(&c, "decide", "elsewhere", "big"), "100 of 1,000: account-wide");
        assert_eq!(recycle(&c) - r0, 500);
    }

    #[test]
    fn idle_registered_devices_do_not_raise_the_threshold() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 30 * DAY;
        let (d1, d2) = violation_setup(&c, e);
        // 300 registered devices with no useful work: not active, not in the denominator.
        for i in 0..300u32 { apply(&c, &format!("josh@{:064x}", 1_000 + i), Some("josh"), Domain::Data, "registered", 0, 0, false, e - 5); }
        // Devices whose last work is older than the window do not count either.
        apply(&c, &dev("josh", 7), Some("josh"), Domain::Data, "t", UNIT, 0, false, e - ACCOUNT_VIOLATION_WINDOW - 1);
        assert_eq!(coordinated(&c, "josh", e).1, 2);
        on_violation(&c, &d1, None, ViolationKind::Equivocation, Domain::Data, e);
        on_violation(&c, &d2, None, ViolationKind::Equivocation, Domain::Data, e + 1);
        assert!(!has_stake(&c, "decide", "nodex", "josh"), "account-wide despite 300 idle devices");
    }

    fn dcommit(c: &Chain, sensor: &str, owner: &str, batch: &str, epoch: u64, seq: Option<u64>, prev: Option<String>)
        -> (anyhow::Result<()>, String) {
        let e = hone_types::LedgerEntry::SensorDataCommit { seq, prev_head: prev,
            sensor_id: sensor.into(), owner: owner.into(), batch_hash: batch.into(), reading_count: 3,
            sensor_type: "continuous".into(), epoch, signed_by: owner.into(), gateway_account: None,
        };
        (c.apply_entry(&e), e.hash())
    }
    /// A well-behaved device: builds on its committed head.
    fn commit(c: &Chain, sensor: &str, owner: &str, batch: &str, epoch: u64) {
        let h = device_head(c, sensor);
        let (r, _) = dcommit(c, sensor, owner, batch, epoch, Some(h.as_ref().map_or(1, |h| h.seq + 1)), h.map(|h| h.head_hash));
        r.unwrap();
    }
    fn dev_state(c: &Chain) -> Vec<(String, Vec<u8>)> { c.store.state_scan_prefix("dev_") }

    #[test]
    fn device_commits_must_extend_the_committed_head() {
        let (off, _d0) = chain();
        // Gate off: commits without seq apply as before and no head is kept.
        assert!(dcommit(&off, "flip-a", "josh", "b0", A + 1, None, None).0.is_ok());
        assert!(dev_state(&off).is_empty());
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 1;
        let err = |r: anyhow::Result<()>| r.unwrap_err().to_string();
        assert!(err(dcommit(&c, "flip-a", "josh", "x", e, None, None).0).contains("resync"), "no seq: rejected");
        assert!(dcommit(&c, "flip-a", "josh", "x0", e, Some(1), Some("junk".into())).0.is_err(), "first commit has no prev");
        let (r1, h1) = dcommit(&c, "flip-a", "josh", "b1", e, Some(1), None);
        r1.unwrap();
        assert_eq!(device_head(&c, "flip-a"), Some(DeviceHead { seq: 1, head_hash: h1.clone(), epoch: e }));
        let (r2, h2) = dcommit(&c, "flip-a", "josh", "b2", e, Some(2), Some(h1.clone()));
        r2.unwrap();
        // Declared gap (seq 3 lost), still hash-linked: accepted.
        let (r4, h4) = dcommit(&c, "flip-a", "josh", "b4", e + 1, Some(4), Some(h2.clone()));
        r4.unwrap();
        // Not extending: wrong prev, or stale seq. Rejected, never a violation.
        assert!(err(dcommit(&c, "flip-a", "josh", "b5", e + 1, Some(5), Some(h2.clone())).0).contains("does not extend"));
        assert!(err(dcommit(&c, "flip-a", "josh", "b3", e + 1, Some(3), Some(h2)).0).contains("stale"));
        assert!(domain_view(&c, "flip-a", Domain::Data, e + 1).is_none(), "a stale commit is not a violation");
        assert_eq!(device_head(&c, "flip-a").unwrap().head_hash, h4);
    }

    #[test]
    fn two_different_commits_at_one_seq_are_equivocation_once() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        let e = A + 1;
        stake(&c, "sensor", "flip-a", "josh", 40);
        let (r1, h1) = dcommit(&c, "flip-a", "josh", "b1", e, Some(1), None);
        r1.unwrap();
        // The identical commit again: rejected, not a violation.
        assert!(dcommit(&c, "flip-a", "josh", "b1", e, Some(1), None).0.is_err());
        assert!(domain_view(&c, "flip-a", Domain::Data, e).is_none());
        // A different signed commit at seq 1: equivocation, recorded once, stake slashed.
        let r0 = recycle(&c);
        assert!(dcommit(&c, "flip-a", "josh", "b1-other", e, Some(1), None).0.unwrap_err().to_string().contains("equivocation"));
        assert!(dcommit(&c, "flip-a", "josh", "b1-other", e, Some(1), None).0.is_err());
        assert_eq!(domain_view(&c, "flip-a", Domain::Data, e).unwrap().bad, VIOLATION_BAD);
        assert_eq!(recycle(&c) - r0, 40);
        assert_eq!(device_head(&c, "flip-a").unwrap().head_hash, h1, "the head is unchanged");
    }

    #[test]
    fn device_heads_are_identical_on_every_node() {
        let _g = Gate::on(A);
        let (c1, _d1) = chain();
        let (c2, _d2) = chain();
        for c in [&c1, &c2] {
            for i in 0..20u64 { commit(c, ["flip-a", "phone-b"][(i % 2) as usize], "josh", &format!("b{i}"), A + i); }
            let _ = dcommit(c, "flip-a", "josh", "evil", A + 30, Some(3), None);
        }
        assert_eq!(dev_state(&c1), dev_state(&c2));
        assert_eq!(device_head(&c1, "flip-a").unwrap().seq, 10);
    }
    fn readings(c: &Chain) -> Vec<(String, Vec<u8>)> { c.store.state_scan_prefix("sensor_readings:") }

    #[test]
    fn decided_sensor_commits_record_readings_identically_and_not_before_the_gate() {
        let (off, _d0) = chain();
        commit(&off, "phone-gnss", "josh", "b1", A + 1);
        assert!(readings(&off).is_empty(), "gate off: the decided commit writes no reading");
        let _g = Gate::on(A);
        let (c1, _d1) = chain();
        let (c2, _d2) = chain();
        for c in [&c1, &c2] {
            commit(c, "phone-gnss", "josh", "b1", A + 1);
            commit(c, "phone-b", "bob", "b2", A + 1);
            commit(c, "flip-a", "josh", "b3", A + 2);
        }
        assert_eq!(readings(&c1).len(), 3);
        assert_eq!(readings(&c1), readings(&c2));
    }

    #[test]
    fn decide_strike_and_double_sign_hooks_land_in_consensus() {
        let _g = Gate::on(A);
        let (c, _d) = chain();
        assert!(on_missed_check(&c, "nebra", "juror did not reveal", A + 3));
        assert_eq!(domain_view(&c, "nebra", Domain::Consensus, A + 3).unwrap().bad, MEDIUM);
        assert!(on_violation(&c, "nebra", None, ViolationKind::Equivocation, Domain::Consensus, A + 4));
        assert_eq!(domain_view(&c, "nebra", Domain::Consensus, A + 4).unwrap().bad, VIOLATION_BAD);
        let j = subject_json(&c, "nebra", A + 4);
        assert_eq!(j["recent"].as_array().unwrap().last().unwrap()["kind"], "violation:equivocation");
    }
}
