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
/// Certified epochs whose missing bodies one request asks for (look-ahead), and the wait
/// after which bodies are also fetched over HTTP from sync peers and the public node.
pub const BODY_LOOKAHEAD: u64 = 64;
pub const BODY_HTTP_AFTER_EPOCHS: u64 = 4;

/// Applying progress for the healer status and the guards: (applied_upto, tip, halted).
static APPLY: Mutex<(u64, u64, bool)> = parking_lot::const_mutex((0, 0, false));

/// Operator request (POST /api/setcert/unavailable, localhost only): sign the bodies-unavailable
/// statement for the halted epoch. A voter signs only after the halt lasted this long, for the
/// bodies it does not hold itself.
static UNAVAIL_REQ: Mutex<Option<u64>> = parking_lot::const_mutex(None);
pub const UNAVAIL_MIN_HALT_EPOCHS: u64 = 60;

/// GET /api/setcert/halt: the next epoch to apply, its certified hashes, the missing ones.
pub fn halt_view(chain: &Chain) -> serde_json::Value {
    let next = sa::applied_upto(chain) + 1;
    let cert = engine(chain).lock().final_cert(next).cloned();
    let (se, since) = *STALL.lock();
    let missing: Vec<String> = cert.as_ref().map(|c| c.content.hashes.iter()
        .filter(|h| sa::body(chain, h).is_none()).cloned().collect()).unwrap_or_default();
    serde_json::json!({ "next": next, "cert_content_hash": cert.as_ref().map(|c| c.content.hash()),
        "cert_hashes": cert.as_ref().map(|c| c.content.hashes.clone()), "missing": missing,
        "waiting_since_epoch": (se == next).then_some(since), "unavail_min_halt_epochs": UNAVAIL_MIN_HALT_EPOCHS,
        "unavailable_statement": sa::unavail_pending(next), "unavailable_certificate": sa::unavail_for(chain, next) })
}
pub fn request_unavail(chain: &Chain, epoch: u64) -> Result<serde_json::Value, String> {
    let next = sa::applied_upto(chain) + 1;
    if epoch != next { return Err(format!("epoch {epoch} is not the halted epoch {next}")); }
    *UNAVAIL_REQ.lock() = Some(epoch);
    Ok(halt_view(chain))
}
/// Driver step: sign a requested statement (voter, Ready, halted long enough, bodies really
/// missing here), and re-send the pending one so all voters converge on one statement.
fn unavail_step(chain: &Chain, me: &str, sk: Option<&ed25519_dalek::SigningKey>, is_clock: bool, cur: u64,
                rebroadcast: bool) -> Option<crate::setcert::Unavail> {
    let next = sa::applied_upto(chain) + 1;
    if *UNAVAIL_REQ.lock() == Some(next) && is_clock && crate::sign_guard::ready() {
        if let Some(sk) = sk {
            let cert = engine(chain).lock().final_cert(next).cloned()?;
            let (se, since) = *STALL.lock();
            if se != next || cur.saturating_sub(since) < UNAVAIL_MIN_HALT_EPOCHS { return None; }
            let missing: Vec<String> = cert.content.hashes.iter().filter(|h| sa::body(chain, h).is_none()).cloned().collect();
            *UNAVAIL_REQ.lock() = None;
            if missing.is_empty() { return None; }
            let ch = cert.content.hash();
            let sig = sign_with(sk)(&crate::setcert::unavail_message(&chain.chain_id, next, &ch, &missing));
            let u = crate::setcert::Unavail { epoch: next, content_hash: ch, hashes: missing, sigs: [(me.to_owned(), sig)].into() };
            let v = verifier(chain);
            let merged = { let e = engine(chain).lock(); sa::on_unavail(chain, &e, u, &v) };
            warn!("[setcert] signed bodies-unavailable for epoch {next}");
            return merged;
        }
    }
    if rebroadcast { return sa::unavail_pending(next); }
    None
}
pub fn apply_view() -> serde_json::Value {
    let (a, t, h) = *APPLY.lock();
    serde_json::json!({ "applied_upto": a, "tip": t, "lag": t.saturating_sub(a), "halted": h })
}

/// HTTP sources for missing bodies: the sync peers plus HONE_BODY_FALLBACK (comma list;
/// default the public node, "off" to disable).
pub fn body_http_sources(peers: &[String]) -> Vec<String> {
    let mut v = peers.to_vec();
    let fb = std::env::var("HONE_BODY_FALLBACK").unwrap_or_else(|_| "https://honemesh.net/chain".into());
    if fb != "off" { v.extend(fb.split(',').map(|s| s.trim().trim_end_matches('/').to_owned()).filter(|s| !s.is_empty())); }
    v.dedup(); v
}

static HTTP_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Fetch the kept certified sets from..=to over HTTP and offer their bodies (each is
/// matched by hash on apply and signature-checked by the normal apply path).
fn fetch_sets_http(sources: Vec<String>, from: u64, to: u64) {
    use std::sync::atomic::Ordering::*;
    if sources.is_empty() || HTTP_BUSY.swap(true, AcqRel) { return; }
    tokio::spawn(async move {
        if let Ok(http) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build() {
            for p in &sources {
                let url = format!("{p}/api/sync/epoch_entries?from={from}&to={to}&certified=1");
                let Ok(r) = http.get(&url).send().await else { continue };
                let Ok(v) = r.json::<serde_json::Value>().await else { continue };
                let mut n = 0;
                for set in v["certified"].as_array().into_iter().flatten() {
                    for r in set["entries"].as_array().into_iter().flatten() {
                        if let Ok(x) = serde_json::from_value::<LedgerEntry>(r["entry"].clone()) {
                            sa::offer_body(x, r["sig"].as_str().map(str::to_owned)); n += 1;
                        }
                    }
                }
                info!("[setcert] http body fetch {from}..{to} from {p}: {n} bodies");
                if n > 0 { break; }
            }
        }
        HTTP_BUSY.store(false, Release);
    });
}

/// Missing hashes of `next` plus those of the following final certs (up to 512).
pub fn missing_ahead(chain: &Chain, e: &Engine, next: u64, missing: &[String]) -> Vec<String> {
    let mut v = missing.to_vec();
    for t in next + 1..=next + BODY_LOOKAHEAD {
        let Some(c) = e.final_cert(t) else { break };
        for h in &c.content.hashes {
            if v.len() >= 512 { return v; }
            if sa::body(chain, h).is_none() && !v.contains(h) { v.push(h.clone()); }
        }
    }
    v
}

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
            {
                let mut g = crate::sign_guard::guard().lock();
                for p in &props { g.signed(crate::sign_guard::SET_PROP, p.epoch, &p.sig); }
                for v in &votes { g.signed(crate::sign_guard::SET_VOTE, v.epoch, &v.sig); }
            }
            // Re-send only what this node can still back with bodies (state swap 2026-11-15).
            let (props, votes, dropped) = sa::restorable(chain, props, votes);
            if dropped > 0 { warn!("[setcert] dropped {dropped} own prop(s)/vote(s) from before the restart: bodies no longer held"); }
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
    if !crate::sign_guard::ready() { return; }
    let hashes = sa::due_hashes(chain, t);
    let p = {
        let mut e = engine(chain).lock();
        e.set_voters(voters_now(chain));
        e.set_prop_floor(crate::sign_guard::floor(crate::sign_guard::SET_PROP));
        let p = e.propose(me, t, hashes, sa::d_report(), now_ms(), &sign_with(sk));
        if p.is_some() { persist_own(&e, me); }
        p
    };
    if let Some(p) = p {
        crate::sign_guard::signed(crate::sign_guard::SET_PROP, p.epoch, &p.sig);
        send(cmd_tx, serde_json::json!({ "type": "set_prop", "prop": p, "nonce": now_ms() })).await;
    }
}

/// A set_* message from gossip.
pub async fn on_msg(chain: &Arc<Chain>, cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>, msg: serde_json::Value) {
    let ty = msg["type"].as_str().unwrap_or("");
    let v = verifier(chain);
    if ty == "set_unavail" {
        if let Ok(u) = serde_json::from_value::<crate::setcert::Unavail>(msg["unavail"].clone()) {
            let e = engine(chain).lock();
            let _ = sa::on_unavail(chain, &e, u, &v);
        }
        return;
    }
    // Sync before submit (spec 6b): a verified PROP / vote by our own key that this process
    // did not make raises the floor, or, after we came up, means a doppelganger.
    if let Some(me) = crate::sign_guard::me() {
        match ty {
            "set_prop" => if let Ok(p) = serde_json::from_value::<Prop>(msg["prop"].clone()) {
                if p.voter == me && v(&p.voter, &crate::setcert::prop_message(&chain.chain_id, &p), &p.sig) {
                    crate::sign_guard::observe_own(crate::sign_guard::SET_PROP, p.epoch, &p.sig);
                }
            },
            "set_vote" => if let Ok(x) = serde_json::from_value::<Vote>(msg["vote"].clone()) {
                if x.voter == me && v(&x.voter, &x.digest(&chain.chain_id), &x.sig) {
                    crate::sign_guard::observe_own(crate::sign_guard::SET_VOTE, x.epoch, &x.sig);
                }
            },
            _ => {}
        }
    }
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
    sk: Arc<Option<ed25519_dalek::SigningKey>>, is_clock: bool, http_sources: Vec<String>)
{
    let mut last_fetch = 0u64;
    let mut last_rb = 0u64;
    let mut last_da = 0u64;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let cur = chain.current_epoch();
        if !sa::active(cur) { continue; }
        let rebroadcast = now_ms().saturating_sub(last_rb) >= sa::REBROADCAST_MS;
        if rebroadcast { last_rb = now_ms(); }
        // Sync before submit (spec 6b): observe-only until synced and past the doppelganger
        // wait; the engine never signs at or below the floors. (Outside the engine lock.)
        crate::sign_guard::tick(&chain, &me, cur);
        let guard_ready = crate::sign_guard::ready();
        let (new, rb, finals) = {
            let mut e = engine(&chain).lock();
            e.set_voters(voters_now(&chain));
            let (mut new, mut rb) = (Vec::new(), Vec::new());
            if let (true, true, Some(k)) = (is_clock, guard_ready, sk.as_ref().as_ref()) {
                e.set_vote_floor(crate::sign_guard::floor(crate::sign_guard::SET_VOTE));
                e.set_prop_floor(crate::sign_guard::floor(crate::sign_guard::SET_PROP));
                (new, rb) = sa::voter_step(&mut e, &chain, &me, cur, now_ms(), sa::d_report(), rebroadcast, &sign_with(k));
                // Persist before sending: a restart re-sends these, never different ones.
                if !new.is_empty() { persist_own(&e, &me); }
            }
            (new, rb, e.finalize())
        };
        for o in &new {
            match o {
                sa::Out::P(p) => crate::sign_guard::signed(crate::sign_guard::SET_PROP, p.epoch, &p.sig),
                sa::Out::V(v) => crate::sign_guard::signed(crate::sign_guard::SET_VOTE, v.epoch, &v.sig),
                sa::Out::C(_) => {}
            }
        }
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
        { let mut a = APPLY.lock(); a.0 = sa::applied_upto(&chain); a.1 = cur; }
        // Data availability: bodies this voter lacks to vote are fetched like missing ones.
        if now_ms().saturating_sub(last_da) > 3_000 {
            let want = sa::da_due(now_ms());
            if !want.is_empty() {
                last_da = now_ms();
                crate::epoch_sync::request_hashes(&cmd_tx, &me, cur, &want);
            }
        }
        if let Some(u) = unavail_step(&chain, &me, sk.as_ref().as_ref(), is_clock, cur, rebroadcast) {
            send(&cmd_tx, serde_json::json!({ "type": "set_unavail", "unavail": u, "nonce": now_ms() })).await;
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
                    APPLY.lock().2 = false;
                }
                Err(missing) => {
                    let waited = body_wait_alert(&mut STALL.lock(), next, cur);
                    if let Some(w) = waited {
                        crate::state_healer::raise_alert(next, format!(
                            "setcert: epoch {next} certified but {} bodies still missing after {w} epochs; applying halted",
                            missing.len()));
                        tracing::error!("[setcert] HEALER ALERT: epoch {} missing {} bodies for {} epochs; applying halted", next, missing.len(), w);
                        APPLY.lock().2 = true;
                    }
                    if cur.saturating_sub(STALL.lock().1) >= BODY_HTTP_AFTER_EPOCHS {
                        fetch_sets_http(http_sources.clone(), next, next + BODY_LOOKAHEAD - 1);
                    }
                    if now_ms().saturating_sub(last_fetch) > 3_000 {
                        last_fetch = now_ms();
                        let ahead = missing_ahead(&chain, &engine(&chain).lock(), next, &missing);
                        warn!("[setcert] epoch {} waits for {} missing bodies; fetching {} by hash (look-ahead)", next, missing.len(), ahead.len());
                        crate::epoch_sync::request_hashes(&cmd_tx, &me, next, &ahead);
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
    fn body_sources_default_to_the_public_node_and_can_be_off() {
        std::env::remove_var("HONE_BODY_FALLBACK");
        let v = body_http_sources(&["http://192.168.68.75:4242".into()]);
        assert_eq!(v, vec!["http://192.168.68.75:4242".to_string(), "https://honemesh.net/chain".to_string()]);
        std::env::set_var("HONE_BODY_FALLBACK", "off");
        assert_eq!(body_http_sources(&[]), Vec::<String>::new());
        std::env::remove_var("HONE_BODY_FALLBACK");
    }

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
