//! Node wiring for decide-by certified entry sets: gossip messages on hone/consensus
//! ("set_prop", "set_vote", "set_cert", "sync_ents"), the per-second driver that votes,
//! finalizes, applies, and fetches what is missing. Inert before SET_CERT_EPOCH.
use std::sync::{Arc, OnceLock};

use hone_types::LedgerEntry;
use parking_lot::Mutex;
use tracing::{info, warn};

use crate::chain::Chain;
use crate::net::NetCmd;
use crate::setcert::{Cert, Engine, Prop, Vote};
use crate::setcert_apply as sa;

static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();

/// A final epoch whose bodies are still missing this many epochs later halts applying
/// (it already waits; nothing after it applies) and raises a healer alert.
pub const BODY_HALT_EPOCHS: u64 = 20;
/// (epoch waiting for bodies, current epoch when that wait began).
static STALL: Mutex<(u64, u64)> = parking_lot::const_mutex((0, 0));

/// Healer-alert decision for a wait on `next` at `cur` (pure, so it is testable).
pub fn body_wait_alert(stall: &mut (u64, u64), next: u64, cur: u64) -> Option<u64> {
    if stall.0 != next { *stall = (next, cur); }
    let waited = cur.saturating_sub(stall.1);
    (waited >= BODY_HALT_EPOCHS).then_some(waited)
}

fn voters_now(chain: &Chain) -> Vec<String> {
    crate::clock::reward_voters(&crate::clock::registered_clock_nodes(&chain.store, chain.current_epoch()))
}

pub fn engine(chain: &Chain) -> &'static Mutex<Engine> {
    ENGINE.get_or_init(|| {
        let mut e = Engine::new(&chain.chain_id, sa::activation(), voters_now(chain));
        // After a restart, continue from the last applied epoch (its final cert is in state).
        let up = sa::applied_upto(chain);
        if up >= sa::activation() {
            if let Some(f) = chain.store.state_get(&format!("setcert_final:{up}"))
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
                let d = crate::setcert::digest(&chain.chain_id, up, f["round"].as_u64().unwrap_or(0) as u32,
                    f["content_hash"].as_str().unwrap_or_default(), f["prev"].as_str().unwrap_or_default());
                e.resume(up, d);
            }
        }
        if let Some(v) = std::fs::read_to_string(voted_path()).ok().and_then(|s| s.trim().parse::<u64>().ok()) { e.set_vote_floor(v); }
        // Own PROPs and votes sent before a restart: re-send exactly those.
        if let Some(j) = std::fs::read(own_path()).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
            let props: Vec<Prop> = serde_json::from_value(j["props"].clone()).unwrap_or_default();
            let votes: Vec<Vote> = serde_json::from_value(j["votes"].clone()).unwrap_or_default();
            if !props.is_empty() || !votes.is_empty() {
                info!("[setcert] restored {} own prop(s) and {} vote(s) from before the restart", props.len(), votes.len());
            }
            e.restore_own(props, votes);
        }
        Mutex::new(e)
    })
}

fn now_ms() -> u64 { crate::utils::now_ms() }

/// Local (not chain) record of this voter's highest voted epoch, so a restart never votes twice.
fn voted_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HONE_DATA_DIR").unwrap_or_else(|_| ".".into())).join("setcert_voted")
}
/// Local record of this voter's own PROPs and latest votes of non-final epochs, written
/// before they are sent (restored after a restart, see Engine::restore_own).
fn own_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HONE_DATA_DIR").unwrap_or_else(|_| ".".into())).join("setcert_own.json")
}
fn persist_own(e: &Engine, me: &str) {
    let from = e.last_final().map(|x| x + 1).unwrap_or(e.activation);
    let (props, votes) = e.own_since(me, from);
    let p = own_path();
    let tmp = p.with_extension("tmp");
    let body = serde_json::to_vec(&serde_json::json!({ "props": props, "votes": votes })).unwrap_or_default();
    if std::fs::write(&tmp, body).is_ok() { let _ = std::fs::rename(&tmp, &p); }
}

/// (epoch with no certificate, alert raised) for the gap alert.
static GAP: Mutex<u64> = parking_lot::const_mutex(0);

fn out_msg(o: &sa::Out) -> serde_json::Value {
    match o {
        sa::Out::P(p) => serde_json::json!({ "type": "set_prop", "prop": p, "nonce": now_ms() }),
        sa::Out::V(v) => serde_json::json!({ "type": "set_vote", "vote": v, "nonce": now_ms() }),
        sa::Out::C(c) => serde_json::json!({ "type": "set_cert", "cert": c, "nonce": now_ms() }),
    }
}

fn record_voted(epoch: u64) {
    let p = voted_path();
    let cur = std::fs::read_to_string(&p).ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
    if epoch > cur {
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, epoch.to_string()).is_ok() { let _ = std::fs::rename(&tmp, &p); }
    }
}

fn verifier(chain: &Chain) -> impl Fn(&str, &str, &str) -> bool + '_ {
    move |voter: &str, msg: &str, sig: &str| crate::verify_clock_signature(chain, voter, msg.as_bytes(), sig).is_ok()
}

fn sign_with(sk: &ed25519_dalek::SigningKey) -> impl Fn(&str) -> String + '_ {
    use ed25519_dalek::Signer;
    move |m: &str| hex::encode(sk.sign(m.as_bytes()).to_bytes())
}

async fn send(cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>, v: serde_json::Value) {
    if let Ok(data) = serde_json::to_vec(&v) {
        let _ = cmd_tx.send(NetCmd::Broadcast { topic: "hone/consensus", data }).await;
    }
}

/// At this node's quorum seal of `t`: a voter proposes its due pool entries.
pub async fn on_seal(chain: &Arc<Chain>, cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>, me: &str,
    sk: Option<&ed25519_dalek::SigningKey>, t: u64)
{
    if !sa::active(t) { return; }
    let Some(sk) = sk else { return };
    let hashes = sa::due_hashes(chain, t);
    let p = {
        let mut e = engine(chain).lock();
        e.set_voters(voters_now(chain));
        let p = e.propose(me, t, hashes, sa::d_report(), now_ms(), &sign_with(sk));
        if p.is_some() { persist_own(&e, me); }
        p
    };
    if let Some(p) = p {
        send(cmd_tx, serde_json::json!({ "type": "set_prop", "prop": p, "nonce": now_ms() })).await;
    }
}

/// A set_* message from gossip.
pub async fn on_msg(chain: &Arc<Chain>, cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>, msg: serde_json::Value) {
    let ty = msg["type"].as_str().unwrap_or("");
    let v = verifier(chain);
    let formed: Option<Cert> = {
        let mut e = engine(chain).lock();
        match ty {
            "set_prop" => { if let Ok(p) = serde_json::from_value::<Prop>(msg["prop"].clone()) { e.on_prop(p, now_ms(), &v); } None }
            "set_vote" => serde_json::from_value::<Vote>(msg["vote"].clone()).ok().and_then(|x| e.on_vote(x, &v)),
            "set_cert" => { if let Ok(c) = serde_json::from_value::<Cert>(msg["cert"].clone()) { e.on_cert(c, &v); } None }
            _ => None,
        }
    };
    if ty == "sync_ents" {
        let mut n = 0;
        for r in msg["entries"].as_array().into_iter().flatten() {
            if let Ok(x) = serde_json::from_value::<LedgerEntry>(r["entry"].clone()) {
                sa::offer_body(x, r["sig"].as_str().map(str::to_owned)); n += 1;
            }
        }
        if n > 0 { info!("[setcert] {} missing bodies received over gossip", n); }
    }
    if let Some(c) = formed {
        send(cmd_tx, serde_json::json!({ "type": "set_cert", "cert": c, "nonce": now_ms() })).await;
    }
}

/// Answer a peer's "ents" (bodies by hash) or "certs" (final certs for a range) request.
pub fn answer(chain: &Chain, me: &str, kind: &str, from: u64, to: u64, hashes: &[String]) -> Vec<serde_json::Value> {
    match kind {
        "ents" => {
            let b = sa::serve_bodies(chain, &hashes[..hashes.len().min(512)], chain.current_epoch());
            if b.is_empty() { vec![] } else { vec![serde_json::json!({ "type": "sync_ents", "node_id": me, "entries": b, "nonce": now_ms() })] }
        }
        "certs" => {
            let e = engine(chain).lock();
            (from..=to.min(from + 31)).filter_map(|t| e.final_cert(t).cloned().or_else(|| sa::applied_cert(chain, t)))
                .map(|c| serde_json::json!({ "type": "set_cert", "cert": c, "nonce": now_ms() })).collect()
        }
        _ => vec![],
    }
}

/// Once a second: vote, finalize, apply in order, fetch what is missing.
pub async fn driver(chain: Arc<Chain>, cmd_tx: tokio::sync::mpsc::Sender<NetCmd>, me: String,
    sk: Arc<Option<ed25519_dalek::SigningKey>>, is_clock: bool)
{
    let mut last_fetch = 0u64;
    let mut last_rb = 0u64;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let cur = chain.current_epoch();
        if !sa::active(cur) { continue; }
        let rebroadcast = now_ms().saturating_sub(last_rb) >= sa::REBROADCAST_MS;
        if rebroadcast { last_rb = now_ms(); }
        let (new, rb, finals) = {
            let mut e = engine(&chain).lock();
            e.set_voters(voters_now(&chain));
            let (mut new, mut rb) = (Vec::new(), Vec::new());
            if let (true, Some(k)) = (is_clock, sk.as_ref().as_ref()) {
                (new, rb) = sa::voter_step(&mut e, &chain, &me, cur, now_ms(), sa::d_report(), rebroadcast, &sign_with(k));
                // Persist before sending: a restart re-sends these, never different ones.
                if !new.is_empty() { persist_own(&e, &me); }
            }
            (new, rb, e.finalize())
        };
        if let Some(m) = new.iter().filter_map(|o| if let sa::Out::V(v) = o { Some(v.epoch) } else { None }).max() { record_voted(m); }
        let catchup_props = new.iter().filter(|o| matches!(o, sa::Out::P(p) if p.epoch + 2 <= cur)).count();
        if catchup_props > 0 { info!("[setcert] catch-up: proposed {} sealed epoch(s) without a prop", catchup_props); }
        for o in new.iter().chain(rb.iter()) { send(&cmd_tx, out_msg(o)).await; }
        let _ = finals;
        // Gap alert: no certificate for the next epoch long after its seal.
        {
            let next = sa::applied_upto(&chain) + 1;
            let mut g = GAP.lock();
            if cur >= next + sa::GAP_ALERT_EPOCHS {
                if *g != next {
                    *g = next;
                    crate::state_healer::raise_alert(next, format!(
                        "setcert: no certificate for epoch {next} after {} epochs; finalization stalled", sa::GAP_ALERT_EPOCHS));
                    tracing::error!("[setcert] HEALER ALERT: no certificate for epoch {} after {} epochs (tip {})", next, sa::GAP_ALERT_EPOCHS, cur);
                }
            } else if *g != 0 && next > *g {
                *g = 0;
                crate::state_healer::clear_alerts("setcert: no certificate");
                info!("[setcert] certificates resumed (next {next}, tip {cur})");
            }
        }
        // A lagging applier: accept fetched final certs as a chain from what it applied.
        {
            let up = sa::applied_upto(&chain);
            if let Some(f) = chain.store.state_get(&format!("setcert_final:{up}")).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
                let d = crate::setcert::digest(&chain.chain_id, up, f["round"].as_u64().unwrap_or(0) as u32,
                    f["content_hash"].as_str().unwrap_or_default(), f["prev"].as_str().unwrap_or_default());
                let n = engine(&chain).lock().advance_lag(up, &d);
                if n > 0 { info!("[setcert] lagging apply: accepted {n} fetched final cert(s) from {}", up + 1); }
            }
        }
        // Apply final certs strictly in order.
        loop {
            let next = sa::applied_upto(&chain) + 1;
            let cert = engine(&chain).lock().final_cert(next).cloned();
            let Some(cert) = cert else {
                // Behind with no cert for `next` (restart, missed gossip): ask the voters.
                if next + 3 < cur && now_ms().saturating_sub(last_fetch) > 10_000 {
                    last_fetch = now_ms();
                    crate::epoch_sync::request(&cmd_tx, &me, "certs", next, cur, 0);
                }
                break;
            };
            match sa::apply_final(&chain, &cert) {
                Ok(_) => {
                    let was = STALL.lock().0 == next;
                    if was { *STALL.lock() = (0, 0); crate::state_healer::clear_alerts("setcert:"); }
                }
                Err(missing) => {
                    let waited = body_wait_alert(&mut STALL.lock(), next, cur);
                    if let Some(w) = waited {
                        crate::state_healer::raise_alert(next, format!(
                            "setcert: epoch {next} certified but {} bodies still missing after {w} epochs; applying halted",
                            missing.len()));
                        tracing::error!("[setcert] HEALER ALERT: epoch {} missing {} bodies for {} epochs; applying halted", next, missing.len(), w);
                    }
                    if now_ms().saturating_sub(last_fetch) > 3_000 {
                        last_fetch = now_ms();
                        warn!("[setcert] epoch {} waits for {} missing bodies; fetching by hash", next, missing.len());
                        crate::epoch_sync::request_hashes(&cmd_tx, &me, &missing);
                    }
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_body_alert_after_20_epochs_of_the_same_wait() {
        let mut st = (0, 0);
        assert_eq!(body_wait_alert(&mut st, 500, 505), None);
        assert_eq!(body_wait_alert(&mut st, 500, 524), None);
        assert_eq!(body_wait_alert(&mut st, 500, 525), Some(20));
        // A new epoch waiting restarts the count.
        assert_eq!(body_wait_alert(&mut st, 501, 530), None);
        assert_eq!(body_wait_alert(&mut st, 501, 550), Some(20));
    }
}
