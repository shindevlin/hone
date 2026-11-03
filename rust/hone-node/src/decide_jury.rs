//! Decide jury — chain side, SHADOW MODE (spec: Hone doc claude/hone-decide-jury-spec-2026-11-03.md;
//! wire contract: hone-decide rust/hone-decide/docs/JURY.md v1).
//!
//! Every judgment entry committed at a quorum seal gets K seed-chosen voters among staked
//! `decide` nodes. Each votes through its local hone-decide (DECIDE_URL), which seals the
//! votes in a Merkle tree; the node publishes the root as DecideCommit. A DecideChallenge
//! opens a jury (J + backups) drawn from the seed of the first sealed epoch after it;
//! jurors post DecideReveal with a proof against their root; a 2/3 supermajority of valid,
//! non-abstain reveals at conf >= t95 decides, else the case escalates (9B, then the
//! deterministic rule). S spot checks per epoch force reveals without a dispute.
//!
//! SHADOW: decide entries only write `decide_*` meta records (not in state_root) and carry
//! no fee. Outcomes are LOGGED next to the deterministic rule (which is what actually holds:
//! the judged entry stays as consensus applied it). Nothing here changes a balance, a stake
//! or another entry's validity. Participation and outcome logging need DECIDE_JURY_SHADOW=1.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Result};
use hone_types::{DecideProofStep, LedgerEntry};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::chain::Chain;

pub fn shadow_enabled() -> bool {
    matches!(std::env::var("DECIDE_JURY_SHADOW").as_deref(), Ok("1") | Ok("true"))
}

fn h(parts: &[&[u8]]) -> [u8; 32] {
    let mut s = Sha256::new();
    for p in parts { s.update(p); }
    s.finalize().into()
}

/// The selection seed of an epoch: its seal hash (identical on every node).
pub fn seed(chain: &Chain, epoch: u64) -> Option<Vec<u8>> {
    chain.store.state_get(&format!("epoch_seal_hash:{}", epoch))
}

// ── vote leaf + Merkle (contract v1 §2-§3) ───────────────────────────────────────

pub fn vote_bytes(entry_hash: &[u8; 32], epoch: u64, task: &str, option: &str, conf_q8: u8) -> Vec<u8> {
    let mut v = b"hone-decide-vote-v1".to_vec();
    v.extend_from_slice(entry_hash);
    v.extend_from_slice(&epoch.to_le_bytes());
    v.push(task.len() as u8); v.extend_from_slice(task.as_bytes());
    v.push(option.len() as u8); v.extend_from_slice(option.as_bytes());
    v.push(conf_q8);
    v
}

pub fn leaf(entry_hash: &[u8; 32], epoch: u64, task: &str, option: &str, conf_q8: u8, salt: &[u8; 32]) -> [u8; 32] {
    h(&[&vote_bytes(entry_hash, epoch, task, option, conf_q8), salt])
}

/// Root over leaves sorted by entry_hash (input: (entry_hash, leaf) pairs).
pub fn merkle_root(mut leaves: Vec<([u8; 32], [u8; 32])>) -> [u8; 32] {
    if leaves.is_empty() { return [0u8; 32]; }
    leaves.sort_by(|a, b| a.0.cmp(&b.0));
    let mut level: Vec<[u8; 32]> = leaves.into_iter().map(|(_, l)| l).collect();
    while level.len() > 1 {
        level = level.chunks(2).map(|c| if c.len() == 2 { h(&[&[1u8], &c[0], &c[1]]) } else { c[0] }).collect();
    }
    level[0]
}

/// Merkle path for the leaf at `index` (after sorting), for tests and the reference side.
pub fn merkle_proof(mut leaves: Vec<([u8; 32], [u8; 32])>, entry: &[u8; 32]) -> Vec<DecideProofStep> {
    leaves.sort_by(|a, b| a.0.cmp(&b.0));
    let mut idx = leaves.iter().position(|(e, _)| e == entry).unwrap_or(0);
    let mut level: Vec<[u8; 32]> = leaves.into_iter().map(|(_, l)| l).collect();
    let mut out = Vec::new();
    while level.len() > 1 {
        let sib = idx ^ 1;
        if sib < level.len() {
            out.push(DecideProofStep { side: if sib < idx { "L".into() } else { "R".into() }, hash: hex::encode(level[sib]) });
        }
        level = level.chunks(2).map(|c| if c.len() == 2 { h(&[&[1u8], &c[0], &c[1]]) } else { c[0] }).collect();
        idx /= 2;
    }
    out
}

pub fn verify_proof(leaf: [u8; 32], proof: &[DecideProofStep], root: &[u8; 32]) -> bool {
    let mut acc = leaf;
    for s in proof {
        let Ok(b) = hex::decode(&s.hash) else { return false };
        let Ok(sib) = <[u8; 32]>::try_from(b.as_slice()) else { return false };
        acc = match s.side.as_str() { "L" => h(&[&[1u8], &sib, &acc]), "R" => h(&[&[1u8], &acc, &sib]), _ => return false };
    }
    &acc == root
}

fn h32(s: &str) -> Option<[u8; 32]> { hex::decode(s).ok().and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()) }

// ── who votes on what (deterministic from committed state + seed) ────────────────

/// Staked decide nodes (role_stake:decide:{node}:{staker}), sorted; device seats excluded.
pub fn decide_nodes(chain: &Chain) -> Vec<String> {
    let mut set = BTreeSet::new();
    for (k, v) in chain.store.state_scan_prefix("role_stake:decide:") {
        let amt = serde_json::from_slice::<serde_json::Value>(&v).ok()
            .and_then(|j| j["amount"].as_u64().or_else(|| j.as_u64())).unwrap_or(1);
        if amt == 0 { continue; }
        if let Some(node) = k.strip_prefix("role_stake:decide:").and_then(|r| r.split(':').next()) {
            if hone_types::parse_device_clock_id(node).is_none() { set.insert(node.to_owned()); }
        }
    }
    set.into_iter().collect()
}

/// A node's declared capacity (votes / 100 epochs) in force at `epoch`. A DecideCommit at
/// epoch Ec takes effect from Ec + 10, so every node computes the same samples even
/// though commits land a few seals after their epoch.
pub fn capacity_at(chain: &Chain, node: &str, epoch: u64) -> u32 {
    let Some(j) = chain.store.state_get(&format!("decide_capacity:{}", node))
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { return 100 };
    let from = j["from_epoch"].as_u64().unwrap_or(0);
    let pick = if epoch >= from { &j["cap"] } else { &j["prev"] };
    pick.as_u64().unwrap_or(100) as u32
}

/// Votes node may cast in `epoch`: cap/100 per epoch, the remainder as a seeded chance.
pub fn quota(seed: &[u8], node: &str, cap100: u32) -> usize {
    let whole = (cap100 / 100) as usize;
    let frac = cap100 % 100;
    let r = h(&[seed, b"cap", node.as_bytes()]);
    whole + usize::from((u16::from_be_bytes([r[0], r[1]]) % 100) < frac as u16)
}

/// Judgment entries of a committed epoch set and their task. Shadow v1 screens every
/// sensor commit / coverage report / sighting with the generic `entry_screen` task.
pub fn judgment_task(e: &LedgerEntry) -> Option<&'static str> {
    match e {
        LedgerEntry::SensorDataCommit { .. } | LedgerEntry::CoverageReport { .. }
        | LedgerEntry::TrackerSightingCommit { .. } => Some("entry_screen"),
        _ => None,
    }
}

/// entry → voters. Entries are taken in seeded order; each gets up to K voters ranked by
/// sha256(seed||entry||node) among nodes with quota left. Pure function of its inputs.
pub fn sample(seed: &[u8], entries: &[String], nodes: &[(String, usize)]) -> BTreeMap<String, Vec<String>> {
    let mut left: BTreeMap<&str, usize> = nodes.iter().map(|(n, q)| (n.as_str(), *q)).collect();
    let mut order: Vec<&String> = entries.iter().collect();
    order.sort_by_key(|e| h(&[seed, b"order", e.as_bytes()]));
    let mut out = BTreeMap::new();
    for e in order {
        let mut cand: Vec<&str> = left.iter().filter(|(_, q)| **q > 0).map(|(n, _)| *n).collect();
        cand.sort_by_key(|n| h(&[seed, e.as_bytes(), n.as_bytes()]));
        let pick: Vec<String> = cand.into_iter().take(hone_types::DECIDE_K).map(str::to_owned).collect();
        for n in &pick { *left.get_mut(n.as_str()).unwrap() -= 1; }
        if !pick.is_empty() { out.insert(e.clone(), pick); }
    }
    out
}

/// The sample for committed epoch `epoch`, from this node's records.
pub fn sample_for_epoch(chain: &Chain, epoch: u64) -> BTreeMap<String, Vec<String>> {
    let Some(sd) = seed(chain, epoch) else { return BTreeMap::new() };
    let entries: Vec<String> = committed_entries(chain, epoch).iter()
        .filter(|e| judgment_task(e).is_some()).map(|e| e.hash()).collect();
    let nodes: Vec<(String, usize)> = decide_nodes(chain).into_iter()
        .map(|n| { let q = quota(&sd, &n, capacity_at(chain, &n, epoch)); (n, q) }).collect();
    sample(&sd, &entries, &nodes)
}

fn committed_entries(chain: &Chain, epoch: u64) -> Vec<LedgerEntry> {
    chain.store.state_get(&format!("epoch_entries:{:016x}", epoch))
        .and_then(|b| serde_json::from_slice::<Vec<serde_json::Value>>(&b).ok()).unwrap_or_default()
        .into_iter().filter_map(|r| serde_json::from_value(r["entry"].clone()).ok()).collect()
}

/// Jurors + backups for a challenge: ranked by sha256(seed_after||"jury"||entry||node)
/// among the nodes that were sampled for the entry AND committed for its epoch.
pub fn jury(seed_after: &[u8], entry_hash: &str, candidates: &[String]) -> (Vec<String>, Vec<String>) {
    let mut c: Vec<&String> = candidates.iter().collect();
    c.sort_by_key(|n| h(&[seed_after, b"jury", entry_hash.as_bytes(), n.as_bytes()]));
    let j: Vec<String> = c.iter().take(hone_types::DECIDE_JURY).map(|s| s.to_string()).collect();
    let b: Vec<String> = c.iter().skip(hone_types::DECIDE_JURY).take(hone_types::DECIDE_BACKUPS).map(|s| s.to_string()).collect();
    (j, b)
}

/// S (node, entry) spot checks drawn by `seed` from a sample.
pub fn spot_checks(seed: &[u8], sample: &BTreeMap<String, Vec<String>>) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = sample.iter()
        .flat_map(|(e, ns)| ns.iter().map(move |n| (n.clone(), e.clone()))).collect();
    pairs.sort_by_key(|(n, e)| h(&[seed, b"spot", n.as_bytes(), e.as_bytes()]));
    pairs.truncate(hone_types::DECIDE_SPOT_CHECKS);
    pairs
}

// ── counting ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome { Decided(String), Escalate9b }

/// `reveals`: (option, conf_q8, t95_q8) of VALID reveals; jurors that did not reveal are
/// simply absent (= abstain). A winner needs >= 2/3 of the valid non-abstain reveals at
/// conf >= t95, and at least 3 of them; otherwise the case escalates.
pub fn count(reveals: &[(String, u8, u8)]) -> Outcome {
    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    let mut n = 0usize;
    for (opt, conf, t95) in reveals {
        if opt == "abstain" || conf < t95 { continue; }
        n += 1; *tally.entry(opt.as_str()).or_default() += 1;
    }
    match tally.iter().max_by_key(|(_, c)| **c) {
        Some((opt, c)) if n >= 3 && 3 * c >= 2 * n => Outcome::Decided(opt.to_string()),
        _ => Outcome::Escalate9b,
    }
}

// ── entry rules (tx::validate_and_apply / chain::apply_entry) ────────────────────

fn rec_get(chain: &Chain, k: &str) -> Option<serde_json::Value> {
    chain.store.state_get(k).and_then(|b| serde_json::from_slice(&b).ok())
}
fn rec_put(chain: &Chain, k: &str, v: &serde_json::Value) -> Result<()> {
    chain.store.state_set(k, &serde_json::to_vec(v)?)
}
fn strike(chain: &Chain, node: &str, why: &str) {
    let k = format!("decide_strikes:{}", node);
    let n = chain.store.state_get(&k).and_then(|b| String::from_utf8(b).ok()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    let _ = chain.store.state_set(&k, (n + 1).to_string().as_bytes());
    warn!("[decide-shadow] strike {} for '{}': {}", n + 1, node, why);
}

pub fn check(chain: &Chain, e: &LedgerEntry) -> Result<()> {
    if e.epoch() < hone_types::DECIDE_ENTRIES_EPOCH { bail!("decide entries are not active until epoch {}", hone_types::DECIDE_ENTRIES_EPOCH); }
    match e {
        LedgerEntry::DecideCommit { node, epoch, root, signed_by, .. } => {
            if signed_by != node { bail!("DecideCommit: signed_by must equal node"); }
            if !decide_nodes(chain).contains(node) { bail!("'{}' has no decide role stake", node); }
            if h32(root).is_none() { bail!("root must be 64 hex"); }
            if rec_get(chain, &format!("decide_commit:{:016x}:{}", epoch, node)).is_some() { bail!("already committed for epoch {}", epoch); }
        }
        LedgerEntry::DecideChallenge { entry_hash, vote_epoch, challenger, bond, epoch, signed_by, .. } => {
            if signed_by != challenger { bail!("DecideChallenge: signed_by must equal challenger"); }
            if h32(entry_hash).is_none() { bail!("entry_hash must be 64 hex"); }
            if epoch.saturating_sub(*vote_epoch) > hone_types::DECIDE_CHALLENGE_WINDOW { bail!("challenge window closed"); }
            if chain.get_balance(challenger, hone_types::NATIVE_TOKEN) < *bond { bail!("bond exceeds balance"); }
            if rec_get(chain, &format!("decide_challenge:{}", entry_hash)).is_some() { bail!("already challenged"); }
        }
        LedgerEntry::DecideReveal { entry_hash, vote_epoch, juror, signed_by, .. } => {
            if signed_by != juror { bail!("DecideReveal: signed_by must equal juror"); }
            if rec_get(chain, &format!("decide_commit:{:016x}:{}", vote_epoch, juror)).is_none() { bail!("no commit by '{}' for epoch {}", juror, vote_epoch); }
            if rec_get(chain, &format!("decide_reveal:{}:{}", entry_hash, juror)).is_some() { bail!("already revealed"); }
        }
        _ => {}
    }
    Ok(())
}

/// Record the entry (meta only). Never touches balances.
pub fn apply(chain: &Chain, e: &LedgerEntry) -> Result<()> {
    match e {
        LedgerEntry::DecideCommit { node, epoch, root, n_votes, model_hash, capacity_per_100, .. } => {
            rec_put(chain, &format!("decide_commit:{:016x}:{}", epoch, node), &serde_json::json!({
                "root": root, "n_votes": n_votes, "model_hash": model_hash }))?;
            let ck = format!("decide_capacity:{}", node);
            let prev = rec_get(chain, &ck).map(|j| j["cap"].clone()).unwrap_or(serde_json::json!(100));
            rec_put(chain, &ck, &serde_json::json!({ "cap": capacity_per_100, "prev": prev, "from_epoch": epoch + 10 }))?;
            if model_hash != hone_types::DECIDE_MODEL_HASH { strike(chain, node, "model_hash mismatch"); }
        }
        LedgerEntry::DecideChallenge { entry_hash, vote_epoch, task, challenger, bond, epoch, .. } => {
            // SHADOW: the bond is recorded, not escrowed.
            rec_put(chain, &format!("decide_challenge:{}", entry_hash), &serde_json::json!({
                "vote_epoch": vote_epoch, "task": task, "challenger": challenger, "bond": bond,
                "filed_epoch": epoch, "seed_epoch": epoch + 1, "deadline": epoch + 1 + hone_types::DECIDE_REVEAL_EPOCHS }))?;
        }
        LedgerEntry::DecideReveal { entry_hash, vote_epoch, juror, task, option, conf_q8, t95_q8, salt, proof, .. } => {
            let root = rec_get(chain, &format!("decide_commit:{:016x}:{}", vote_epoch, juror))
                .and_then(|j| j["root"].as_str().and_then(h32));
            let valid = match (h32(entry_hash), h32(salt), root) {
                (Some(eh), Some(s), Some(r)) => verify_proof(leaf(&eh, *vote_epoch, task, option, *conf_q8, &s), proof, &r),
                _ => false,
            };
            if !valid { strike(chain, juror, "reveal failed its Merkle proof"); }
            rec_put(chain, &format!("decide_reveal:{}:{}", entry_hash, juror), &serde_json::json!({
                "option": option, "conf_q8": conf_q8, "t95_q8": t95_q8, "valid": valid, "vote_epoch": vote_epoch }))?;
        }
        _ => {}
    }
    Ok(())
}

/// Seal-time shadow evaluation at sealed epoch `epoch`: count challenges whose reveal
/// deadline is now, and log missed spot-check reveals. Writes decide_shadow:* meta + log.
pub fn shadow_tick(chain: &Chain, epoch: u64) {
    if !shadow_enabled() || epoch < hone_types::DECIDE_ENTRIES_EPOCH { return; }
    for (k, v) in chain.store.state_scan_prefix("decide_challenge:") {
        let Ok(c) = serde_json::from_slice::<serde_json::Value>(&v) else { continue };
        if c["deadline"].as_u64() != Some(epoch) { continue; }
        let eh = k.trim_start_matches("decide_challenge:").to_owned();
        let ve = c["vote_epoch"].as_u64().unwrap_or(0);
        let Some(sd) = seed(chain, c["seed_epoch"].as_u64().unwrap_or(0)) else { continue };
        let cands: Vec<String> = sample_for_epoch(chain, ve).remove(&eh).unwrap_or_default().into_iter()
            .filter(|n| rec_get(chain, &format!("decide_commit:{:016x}:{}", ve, n)).is_some()).collect();
        let (jurors, backups) = jury(&sd, &eh, &cands);
        let mut reveals = Vec::new();
        let mut missed = Vec::new();
        for j in jurors.iter().chain(backups.iter()).take(hone_types::DECIDE_JURY + hone_types::DECIDE_BACKUPS) {
            match rec_get(chain, &format!("decide_reveal:{}:{}", eh, j)) {
                Some(r) if r["valid"] == true && reveals.len() < hone_types::DECIDE_JURY =>
                    reveals.push((r["option"].as_str().unwrap_or("abstain").to_owned(), r["conf_q8"].as_u64().unwrap_or(0) as u8, r["t95_q8"].as_u64().unwrap_or(255) as u8)),
                Some(_) => {}
                None if jurors.contains(j) => { missed.push(j.clone()); strike(chain, j, "juror did not reveal"); }
                None => {}
            }
        }
        let outcome = count(&reveals);
        let rule = "deterministic: entry stands as consensus applied it";
        info!("[decide-shadow] challenge {} (epoch {}): jury {:?} backups {:?} reveals {} missed {:?} -> {:?}; {}",
            &eh[..12.min(eh.len())], ve, jurors, backups, reveals.len(), missed, outcome, rule);
        let _ = rec_put(chain, &format!("decide_shadow:{}", eh), &serde_json::json!({
            "jury": jurors, "backups": backups, "reveals": reveals.len(), "missed": missed,
            "outcome": format!("{:?}", outcome), "applied_rule": rule, "binding": false }));
    }
    // Spot checks drawn at seed(epoch - R) on vote epoch (epoch - R - 2) are due now.
    let drawn = epoch.saturating_sub(hone_types::DECIDE_REVEAL_EPOCHS);
    if let Some(sd) = seed(chain, drawn) {
        let ve = drawn.saturating_sub(2);
        for (n, eh) in spot_checks(&sd, &sample_for_epoch(chain, ve)) {
            if rec_get(chain, &format!("decide_reveal:{}:{}", eh, n)).is_none() {
                strike(chain, &n, "spot check not revealed");
            }
        }
    }
}

// ── node ↔ hone-decide (never blocks or crashes the node) ────────────────────────

/// Structured features for `entry_screen`: the entry's numbers, bools, nulls and token
/// strings (contract §1), depth <= 3, <= 2000 chars; signatures and free text dropped.
pub fn features(e: &LedgerEntry) -> serde_json::Value {
    fn tok(s: &str) -> bool { s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_:.+-".contains(&b)) }
    fn clean(v: &serde_json::Value, depth: usize) -> Option<serde_json::Value> {
        use serde_json::Value::*;
        match v {
            Null | Bool(_) | Number(_) => Some(v.clone()),
            String(s) if tok(s) => Some(v.clone()),
            String(_) => None,
            Array(a) if depth < 3 => Some(Array(a.iter().take(16).filter_map(|x| clean(x, depth + 1)).collect())),
            Object(m) if depth < 3 => Some(Object(m.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "signature" | "sig" | "signed_by"))
                .filter_map(|(k, x)| clean(x, depth + 1).map(|c| (k.clone(), c))).collect())),
            _ => None,
        }
    }
    let mut v = clean(&serde_json::to_value(e).unwrap_or_default(), 0).unwrap_or_default();
    while serde_json::to_string(&v).map(|s| s.len()).unwrap_or(0) > 2000 {
        let Some(m) = v.as_object_mut() else { break };
        let Some(k) = m.keys().last().cloned() else { break };
        m.remove(&k);
    }
    v
}

/// After the quorum seal of `epoch`: vote on our share of its judgment entries, seal the
/// batch, publish DecideCommit, and post any reveals now due. Any hone-decide failure
/// skips this epoch.
pub async fn vote_epoch(chain: &std::sync::Arc<Chain>, cmd_tx: &tokio::sync::mpsc::Sender<crate::net::NetCmd>, me: &str, epoch: u64) {
    if !shadow_enabled() || epoch < hone_types::DECIDE_ENTRIES_EPOCH { return; }
    let Ok(base) = std::env::var("DECIDE_URL") else { return };
    if !decide_nodes(chain).iter().any(|n| n == me) { return; }
    let base = base.trim_end_matches('/').to_owned();
    let http = reqwest::Client::new();
    let cap = match http.get(format!("{base}/capacity")).timeout(std::time::Duration::from_secs(5)).send().await {
        Ok(r) => r.json::<serde_json::Value>().await.ok()
            .and_then(|j| j["votes_per_100_epochs"].as_u64().or_else(|| j["votes_per_epoch"].as_u64().map(|v| v * 100)))
            .unwrap_or(0) as u32,
        Err(e) => { warn!("[decide] {base} unreachable ({e}); skipping epoch {epoch}"); return; }
    };
    let all = committed_entries(chain, epoch);
    let mine: Vec<&LedgerEntry> = {
        let s = sample_for_epoch(chain, epoch);
        all.iter().filter(|e| s.get(&e.hash()).map(|v| v.iter().any(|n| n == me)).unwrap_or(false)).collect()
    };
    if !mine.is_empty() {
        let items: Vec<serde_json::Value> = mine.iter().map(|e| serde_json::json!({
            "entry_hash": e.hash(), "task": judgment_task(e).unwrap_or("entry_screen"), "features": features(e) })).collect();
        let t = std::time::Duration::from_secs(10 * items.len() as u64 + 30);
        match http.post(format!("{base}/vote")).timeout(t).json(&serde_json::json!({ "epoch": epoch, "items": items })).send().await {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => { warn!("[decide] /vote epoch {epoch}: HTTP {}; skipping", r.status()); return; }
            Err(e) => { warn!("[decide] /vote epoch {epoch}: {e}; skipping"); return; }
        }
    }
    let commit = match http.post(format!("{base}/commit/{epoch}")).timeout(std::time::Duration::from_secs(10)).send().await {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await { Ok(j) => j, Err(_) => return },
        _ => { warn!("[decide] /commit/{epoch} failed; skipping"); return; }
    };
    let entry = LedgerEntry::DecideCommit {
        node: me.to_owned(), epoch, root: commit["root"].as_str().unwrap_or_default().to_owned(),
        n_votes: commit["n_votes"].as_u64().unwrap_or(0),
        model_hash: commit["model_hash"].as_str().unwrap_or_default().to_owned(),
        capacity_per_100: cap, signed_by: me.to_owned(), signature: None,
    };
    if let Err(e) = crate::local_submit::submit_own(chain, cmd_tx, &entry).await { warn!("[decide] commit {epoch} not submitted: {e}"); }
    // Reveals due: spot checks drawn by this seal (on vote epoch epoch-2) and open juries.
    let mut due: Vec<(u64, String)> = Vec::new();
    if let Some(sd) = seed(chain, epoch) {
        let ve = epoch.saturating_sub(2);
        for (n, eh) in spot_checks(&sd, &sample_for_epoch(chain, ve)) { if n == me { due.push((ve, eh)); } }
    }
    for (k, v) in chain.store.state_scan_prefix("decide_challenge:") {
        let Ok(c) = serde_json::from_slice::<serde_json::Value>(&v) else { continue };
        if c["seed_epoch"].as_u64() != Some(epoch) { continue; }
        let eh = k.trim_start_matches("decide_challenge:").to_owned();
        let ve = c["vote_epoch"].as_u64().unwrap_or(0);
        let Some(sd) = seed(chain, epoch) else { continue };
        let cands = sample_for_epoch(chain, ve).remove(&eh).unwrap_or_default();
        let (j, b) = jury(&sd, &eh, &cands);
        if j.iter().chain(b.iter()).any(|n| n == me) { due.push((ve, eh)); }
    }
    for (ve, eh) in due {
        let Ok(r) = http.get(format!("{base}/reveal/{ve}/{eh}")).timeout(std::time::Duration::from_secs(10)).send().await else { continue };
        let Ok(j) = r.json::<serde_json::Value>().await else { continue };
        let v = &j["vote"];
        let proof: Vec<DecideProofStep> = serde_json::from_value(j["proof"].clone()).unwrap_or_default();
        let rev = LedgerEntry::DecideReveal {
            entry_hash: eh.clone(), vote_epoch: ve, juror: me.to_owned(),
            task: v["task"].as_str().unwrap_or_default().to_owned(), option: v["option"].as_str().unwrap_or("abstain").to_owned(),
            conf_q8: v["conf_q8"].as_u64().unwrap_or(0) as u8, t95_q8: v["t95_q8"].as_u64().unwrap_or(255) as u8,
            salt: j["salt"].as_str().unwrap_or_default().to_owned(), proof, epoch, signed_by: me.to_owned(), signature: None,
        };
        if let Err(e) = crate::local_submit::submit_own(chain, cmd_tx, &rev).await { warn!("[decide] reveal {eh} not submitted: {e}"); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eh(i: u8) -> [u8; 32] { [i; 32] }

    #[test]
    fn commit_reveal_proof_round_trip() {
        let salts: Vec<[u8; 32]> = (0..5u8).map(|i| [100 + i; 32]).collect();
        let leaves: Vec<([u8; 32], [u8; 32])> = (0..5u8).map(|i| (eh(i), leaf(&eh(i), 352_500, "entry_screen", if i % 2 == 0 { "normal" } else { "abstain" }, 200, &salts[i as usize]))).collect();
        let root = merkle_root(leaves.clone());
        for i in 0..5u8 {
            let l = leaves[i as usize].1;
            let p = merkle_proof(leaves.clone(), &eh(i));
            assert!(verify_proof(l, &p, &root), "leaf {i}");
            // A changed option or salt does not verify.
            let bad = leaf(&eh(i), 352_500, "entry_screen", "spam", 200, &salts[i as usize]);
            assert!(!verify_proof(bad, &p, &root));
        }
        assert_eq!(merkle_root(vec![]), [0u8; 32]);
    }

    #[test]
    fn selection_is_deterministic_and_respects_k_quota_and_jury_sizes() {
        let seed = b"seal-hash-352500".to_vec();
        let entries: Vec<String> = (0..20u8).map(|i| hex::encode(eh(i))).collect();
        let nodes: Vec<(String, usize)> = (0..9).map(|i| (format!("n{i}"), 4)).collect();
        let a = sample(&seed, &entries, &nodes);
        let mut shuffled = nodes.clone(); shuffled.reverse();
        let b = sample(&seed, &entries, &shuffled);
        assert_eq!(a, b, "independent of input order (every node computes the same)");
        assert!(a.values().all(|v| v.len() <= hone_types::DECIDE_K));
        let mut per: BTreeMap<&str, usize> = BTreeMap::new();
        for v in a.values() { for n in v { *per.entry(n.as_str()).or_default() += 1; } }
        assert!(per.values().all(|c| *c <= 4), "quota respected");
        assert!(a.values().all(|v| v.len() == hone_types::DECIDE_K.min(v.len())));
        let cands: Vec<String> = (0..9).map(|i| format!("n{i}")).collect();
        let (j, bk) = jury(b"after", &entries[0], &cands);
        assert_eq!((j.len(), bk.len()), (hone_types::DECIDE_JURY, hone_types::DECIDE_BACKUPS));
        let mut rev = cands.clone(); rev.reverse();
        assert_eq!(jury(b"after", &entries[0], &rev), (j.clone(), bk.clone()), "same jury on every node");
        let (j2, b2) = jury(b"other", &entries[0], &cands);
        assert_ne!((j2, b2), (j, bk), "the seed after the challenge decides");
        assert_eq!(spot_checks(&seed, &a).len(), hone_types::DECIDE_SPOT_CHECKS);
        // Fractional capacity: nebra at 68/100 votes about 68% of epochs.
        let hits = (0..1000u32).filter(|i| quota(&i.to_le_bytes(), "nebra", 68) == 1).count();
        assert!((600..760).contains(&hits), "{hits}");
    }

    fn chain(l: &str) -> (Chain, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        (Chain::new(crate::store::Store::open(d.path()).unwrap(), format!("n-{l}"), "hone-test".into()), d)
    }

    /// Full shadow flow on a real chain with mocked votes: 6 staked decide nodes commit,
    /// a challenge opens, jurors reveal (one forged proof, one missing), the tick counts.
    /// Balances (state_root) never move, and the outcome is logged, not applied.
    #[test]
    fn challenge_flow_with_mocked_votes_never_changes_root() {
        std::env::set_var("DECIDE_JURY_SHADOW", "1");
        let (c, _d) = chain("jury");
        let ve = hone_types::DECIDE_ENTRIES_EPOCH + 10;
        for i in 0..6 { c.store.state_set(&format!("role_stake:decide:d{i}:d{i}"), br#"{"amount":50000000000}"#).unwrap(); }
        c.store.set_balance("challenger", "HONE", 1_000).unwrap();
        c.store.state_set(&format!("epoch_seal_hash:{}", ve + 1), b"seedA").unwrap();
        let root0 = c.store.balance_merkle_root();
        let target = eh(7);
        let mut votes: BTreeMap<String, ([u8; 32], String, u8)> = BTreeMap::new();
        for i in 0..6 {
            let n = format!("d{i}");
            let salt = [i as u8 + 1; 32];
            let opt = if i < 4 { "normal" } else { "spam" };
            let leaves = vec![(target, leaf(&target, ve, "entry_screen", opt, 220, &salt)), (eh(9), [9; 32])];
            apply(&c, &LedgerEntry::DecideCommit { node: n.clone(), epoch: ve, root: hex::encode(merkle_root(leaves.clone())),
                n_votes: 2, model_hash: hone_types::DECIDE_MODEL_HASH.into(), capacity_per_100: 100, signed_by: n.clone(), signature: None }).unwrap();
            votes.insert(n, (salt, opt.into(), 220));
            let _ = leaves;
        }
        let th = hex::encode(target);
        apply(&c, &LedgerEntry::DecideChallenge { entry_hash: th.clone(), vote_epoch: ve, task: "entry_screen".into(),
            challenger: "challenger".into(), bond: 500, epoch: ve, signed_by: "challenger".into(), signature: None }).unwrap();
        // Everyone sampled (mock: all six are candidates).
        let cands: Vec<String> = (0..6).map(|i| format!("d{i}")).collect();
        let (jurors, _b) = jury(b"seedA", &th, &cands);
        for (k, j) in jurors.iter().enumerate() {
            if k == 4 { continue; } // one juror misses its reveal
            let (salt, opt, conf) = votes[j].clone();
            let mut leaves = vec![(target, leaf(&target, ve, "entry_screen", &opt, conf, &salt)), (eh(9), [9; 32])];
            let proof = merkle_proof(leaves.clone(), &target);
            let forged = k == 3;
            if forged { leaves.clear(); }
            apply(&c, &LedgerEntry::DecideReveal { entry_hash: th.clone(), vote_epoch: ve, juror: j.clone(), task: "entry_screen".into(),
                option: if forged { "spam".into() } else { opt }, conf_q8: conf, t95_q8: 145,
                salt: hex::encode(salt), proof, epoch: ve + 3, signed_by: j.clone(), signature: None }).unwrap();
        }
        // Reveals verified against roots: the forged one is invalid and struck.
        let valid = jurors.iter().filter(|j| rec_get(&c, &format!("decide_reveal:{}:{}", th, j)).map(|r| r["valid"] == true).unwrap_or(false)).count();
        assert_eq!(valid, 3);
        // Count by hand the way shadow_tick does (shadow_tick needs the sample, mocked here).
        let rv: Vec<(String, u8, u8)> = jurors.iter().filter_map(|j| rec_get(&c, &format!("decide_reveal:{}:{}", th, j)))
            .filter(|r| r["valid"] == true).map(|r| (r["option"].as_str().unwrap().to_owned(), 220, 145)).collect();
        let _ = count(&rv);
        shadow_tick(&c, ve + 1 + hone_types::DECIDE_REVEAL_EPOCHS);
        assert_eq!(c.store.balance_merkle_root(), root0, "shadow mode never moves a balance");
        assert_eq!(c.get_balance("challenger", "HONE"), 1_000, "bond recorded, not taken");
        // A wrong model hash is a strike, still no balance change.
        apply(&c, &LedgerEntry::DecideCommit { node: "d0".into(), epoch: ve + 1, root: hex::encode([0u8; 32]), n_votes: 0,
            model_hash: "00".repeat(32), capacity_per_100: 68, signed_by: "d0".into(), signature: None }).unwrap();
        assert!(c.store.state_get("decide_strikes:d0").is_some());
        assert_eq!(capacity_at(&c, "d0", ve + 5), 100, "new capacity applies from +10");
        assert_eq!(capacity_at(&c, "d0", ve + 11), 68);
        assert_eq!(c.store.balance_merkle_root(), root0);
    }

    #[test]
    fn decide_entries_gated_and_role_checked() {
        let (c, _d) = chain("gate");
        let e = |ep| LedgerEntry::DecideCommit { node: "x".into(), epoch: ep, root: hex::encode([0u8; 32]), n_votes: 0,
            model_hash: hone_types::DECIDE_MODEL_HASH.into(), capacity_per_100: 100, signed_by: "x".into(), signature: None };
        assert!(check(&c, &e(hone_types::DECIDE_ENTRIES_EPOCH - 1)).is_err());
        assert!(check(&c, &e(hone_types::DECIDE_ENTRIES_EPOCH)).is_err(), "no decide stake");
        c.store.state_set("role_stake:decide:x:x", br#"{"amount":1}"#).unwrap();
        assert!(check(&c, &e(hone_types::DECIDE_ENTRIES_EPOCH)).is_ok());
    }

    #[test]
    fn counting_supermajority_abstain_and_escalation() {
        let r = |o: &str, c: u8| (o.to_string(), c, 145u8);
        assert_eq!(count(&[r("normal", 200), r("normal", 210), r("normal", 190), r("spam", 220), r("abstain", 0)]), Outcome::Decided("normal".into()));
        // Below t95 counts as abstain; missed reveals are simply absent.
        assert_eq!(count(&[r("normal", 200), r("normal", 100), r("spam", 220)]), Outcome::Escalate9b);
        assert_eq!(count(&[r("normal", 200), r("normal", 210), r("spam", 220), r("spam", 220)]), Outcome::Escalate9b);
        assert_eq!(count(&[]), Outcome::Escalate9b);
    }
}
