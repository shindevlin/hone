//! Localhost HTTP sidecar.
//!   POST /decide {context, question, candidates[], kind?, task?} -> {probs[], candidates[], choice, model, latency_ms, tokens}
//!   GET  /health -> {ok, model, rss_mb, mem_available_mb}
//!   GET  /tasks  -> built-in HONE task registry

use crate::engine::Decider;
use crate::prompt::{Ordered, Record};
use crate::jury;
use crate::tasks::Registry;
use anyhow::{anyhow, bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(untagged)]
enum Cand {
    Name(String),
    Named { name: String, #[serde(default)] description: Option<Value> },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Cands {
    List(Vec<Cand>),
    Map(Ordered<Value>),
}

#[derive(Deserialize)]
pub struct DecideReq {
    #[serde(alias = "state")]
    pub context: Value,
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    candidates: Option<Cands>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    task: Option<String>,
    /// Abstain when the top probability is below this (overrides model/task defaults).
    #[serde(default)]
    pub threshold: Option<f32>,
}

/// Threshold resolution: request > model thresholds.json[task] > task.abstain_below > none.
pub fn resolve_threshold(req_t: Option<f32>, task: Option<&str>, d: &Decider, reg: &Registry) -> Option<f32> {
    req_t.or_else(|| task.and_then(|t| d.thresholds.get(t).copied()))
        .or_else(|| task.and_then(|t| reg.get(t)).and_then(|t| t.abstain_below))
}

pub fn build_record(req: DecideReq, reg: &Registry) -> Result<Record> {
    let task = match &req.task {
        Some(t) => Some(reg.get(t).ok_or_else(|| anyhow!("unknown task {t}"))?),
        None => None,
    };
    let question = req.question.clone().or_else(|| task.map(|t| t.question.clone())).ok_or_else(|| anyhow!("question required"))?;
    let kind = req.kind.clone().or_else(|| task.map(|t| t.kind.clone())).unwrap_or_else(|| "choice".into());
    let cands: Vec<(String, Option<Value>)> = match req.candidates {
        Some(Cands::List(v)) => v
            .into_iter()
            .map(|c| match c {
                Cand::Name(n) => (n, None),
                Cand::Named { name, description } => (name, description),
            })
            .collect(),
        Some(Cands::Map(m)) => m.0.into_iter().map(|(k, v)| (k, Some(v))).collect(),
        None => task.map(|t| t.candidates.0.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect()).unwrap_or_default(),
    };
    match kind.as_str() {
        "choice" => Record::choice(req.context, &question, cands),
        "score" => Record::score(req.context, &question, &cands.into_iter().map(|(n, _)| Value::String(n)).collect::<Vec<_>>()),
        "noul" => Record::noul(req.context, &question, None),
        k => bail!("kind must be choice, score or noul (got {k})"),
    }
}

pub fn proc_kb(field: &str) -> Option<u64> {
    let (path, key) = if field == "VmRSS" { ("/proc/self/status", "VmRSS:") } else { ("/proc/meminfo", "MemAvailable:") };
    let s = std::fs::read_to_string(path).ok()?;
    let l = s.lines().find(|l| l.starts_with(key))?;
    l.split_whitespace().nth(1)?.parse().ok()
}

/// Memory guard (works without the memory cgroup; nebra boots with cgroup_disable=memory,
/// so systemd's MemoryMax is not enforced there):
/// - host MemAvailable below `min_free_mb` twice in a row -> exit 75 (unit stays stopped:
///   RestartPreventExitStatus=75), so this process never squeezes the HONE node;
/// - own RSS above `max_rss_mb` -> exit 1 (systemd restarts it).
pub fn spawn_memguard(min_free_mb: u64, max_rss_mb: u64) {
    if min_free_mb == 0 && max_rss_mb == 0 {
        return;
    }
    std::thread::spawn(move || {
        let mut low = 0;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let rss = proc_kb("VmRSS").unwrap_or(0) / 1024;
            if max_rss_mb > 0 && rss > max_rss_mb {
                eprintln!("hone-decide: RSS {rss} MB > {max_rss_mb} MB; exiting");
                std::process::exit(1);
            }
            let avail = proc_kb("MemAvailable").unwrap_or(u64::MAX) / 1024;
            low = if min_free_mb > 0 && avail < min_free_mb { low + 1 } else { 0 };
            if low >= 2 {
                eprintln!("hone-decide: MemAvailable {avail} MB < {min_free_mb} MB; stopping to protect the node");
                std::process::exit(75);
            }
        }
    });
}

pub fn serve(d: Decider, bind: &str, votes_dir: &std::path::Path) -> Result<()> {
    let reg = Registry::load()?;
    let mut store = jury::Store::open(votes_dir)?;
    let model_hash = jury::model_hash(&d.dir, &d.cfg.weights, &d.cfg.head)?;
    let mut lat: std::collections::VecDeque<f64> = std::collections::VecDeque::new();
    eprintln!("hone-decide: model_hash {model_hash}, votes in {votes_dir:?} (newest epoch {})", store.newest);
    let server = tiny_http::Server::http(bind).map_err(|e| anyhow!("bind {bind}: {e}"))?;
    eprintln!("hone-decide: model {} listening on http://{bind}", d.name());
    let jhdr = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    for mut rq in server.incoming_requests() {
        let (code, body) = match (rq.method().as_str(), rq.url()) {
            ("GET", "/health") => (200, json!({"ok": true, "model": d.name(), "model_hash": model_hash,
                "rss_mb": proc_kb("VmRSS").map(|k| k / 1024), "mem_available_mb": proc_kb("MemAvailable").map(|k| k / 1024)})),
            ("GET", "/tasks") => (200, reg.to_json()),
            ("POST", "/decide") => {
                let mut buf = String::new();
                let r = (|| -> Result<Value> {
                    if rq.body_length().unwrap_or(0) > 256 * 1024 {
                        bail!("request too large");
                    }
                    std::io::Read::read_to_string(rq.as_reader(), &mut buf)?;
                    let req: DecideReq = serde_json::from_str(&buf)?;
                    let (task, req_t) = (req.task.clone(), req.threshold);
                    let rec = build_record(req, &reg)?;
                    let out = d.decide(&rec)?;
                    let best = (0..out.probs.len()).max_by(|&a, &b| out.probs[a].total_cmp(&out.probs[b])).unwrap_or(0);
                    let conf = out.probs[best];
                    let th = resolve_threshold(req_t, task.as_deref(), &d, &reg);
                    let decision = if th.map(|t| conf < t).unwrap_or(false) { "abstain" } else { "answer" };
                    Ok(json!({"probs": out.probs, "candidates": out.keys, "choice": out.keys[best], "model": d.name(),
                              "confidence": conf, "threshold": th, "decision": decision,
                              "latency_ms": (out.latency_ms * 10.0).round() / 10.0, "tokens": out.tokens}))
                })();
                match r {
                    Ok(v) => (200, v),
                    Err(e) => (400, json!({"error": e.to_string()})),
                }
            }
            ("POST", "/vote") => {
                let mut buf = String::new();
                let r = (|| -> Result<(u16, Value)> {
                    if rq.body_length().unwrap_or(0) > 256 * 1024 {
                        bail!("request too large");
                    }
                    std::io::Read::read_to_string(rq.as_reader(), &mut buf)?;
                    let req: VoteReq = serde_json::from_str(&buf)?;
                    vote(&d, &reg, &mut store, &model_hash, &mut lat, req)
                })();
                match r {
                    Ok(x) => x,
                    Err(e) => (400, json!({"error": e.to_string()})),
                }
            }
            ("GET", "/capacity") => {
                if lat.is_empty() {
                    if let Ok(ms) = self_bench(&d, &reg) {
                        lat.push_back(ms);
                    }
                }
                let mut v: Vec<f64> = lat.iter().copied().collect();
                v.sort_by(|a, b| a.total_cmp(b));
                let p50 = v.get(v.len() / 2).copied();
                let (epoch_ms, frac) = (30000.0, budget_fraction());
                (200, json!({"ms_per_vote_p50": p50, "samples": v.len(), "epoch_ms": epoch_ms, "budget_fraction": frac,
                    "concurrency": 1, "votes_per_epoch": p50.map(|p| ((epoch_ms * frac) / p.max(1.0)).floor() as u64).unwrap_or(0)}))
            }
            ("GET", u) if u.starts_with("/commit/") => match u[8..].parse::<u64>() {
                Ok(e) => match store.seal(e, &model_hash) {
                    Ok(c) => (200, serde_json::to_value(c).unwrap_or_default()),
                    Err(err) => (500, json!({"error": err.to_string()})),
                },
                Err(_) => (400, json!({"error": "bad epoch"})),
            },
            ("GET", u) if u.starts_with("/reveal/") => {
                let parts: Vec<&str> = u[8..].split('/').collect();
                match (parts.first().and_then(|e| e.parse::<u64>().ok()), parts.get(1)) {
                    (Some(e), Some(h)) => match store.reveal(e, &h.to_lowercase()) {
                        Some(v) => (200, v),
                        None => (404, json!({"error": "no sealed vote for that epoch/entry (unsealed, pruned or never voted)"})),
                    },
                    _ => (400, json!({"error": "use /reveal/{epoch}/{entry_hash}"})),
                }
            }
            _ => (404, json!({"error": "not found"})),
        };
        let resp = tiny_http::Response::from_string(body.to_string()).with_status_code(code).with_header(jhdr.clone());
        let _ = rq.respond(resp);
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoteItem {
    pub entry_hash: String,
    pub task: String,
    pub features: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoteReq {
    pub epoch: u64,
    pub items: Vec<VoteItem>,
}

fn budget_fraction() -> f64 {
    std::env::var("HONE_DECIDE_BUDGET").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5)
}

fn task_record(reg: &Registry, task: &str, features: Value) -> Result<Record> {
    let t = reg.get(task).ok_or_else(|| anyhow!("unknown task {task}"))?;
    if t.kind != "choice" {
        bail!("jury tasks must be choice tasks");
    }
    jury::check_features(&features)?;
    Record::choice(features, &t.question, t.candidates.0.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect())
}

/// One decision with fixed sample features, to seed /capacity before any vote.
fn self_bench(d: &Decider, reg: &Registry) -> Result<f64> {
    let f = json!({"peer": "bench", "stats": {"uptime_pct_24h": 99.0, "clock_skew_ms": 50, "invalid_signatures_1h": 0,
        "dup_msgs_1h": 1, "msgs_per_min": 12.0}, "network_medians": {"clock_skew_ms": 80, "msgs_per_min": 18}});
    Ok(d.decide(&task_record(reg, "peer_behavior", f)?)?.latency_ms)
}

fn vote(d: &Decider, reg: &Registry, store: &mut jury::Store, model_hash: &str,
        lat: &mut std::collections::VecDeque<f64>, req: VoteReq) -> Result<(u16, Value)> {
    if req.items.len() > jury::MAX_ITEMS {
        bail!("at most {} items per call", jury::MAX_ITEMS);
    }
    if store.commit(req.epoch).is_some() {
        return Ok((409, json!({"error": format!("epoch {} is sealed", req.epoch)})));
    }
    let t0 = std::time::Instant::now();
    let (mut votes, mut errors, mut fresh) = (vec![], vec![], vec![]);
    for it in req.items {
        let eh = it.entry_hash.to_lowercase();
        let r = (|| -> Result<jury::Stored> {
            jury::unhex32(&eh)?;
            if let Some(s) = store.get(req.epoch, &eh) {
                return Ok(s);
            }
            if let Some(s) = fresh.iter().find(|s: &&jury::Stored| s.vote.entry_hash == eh) {
                return Ok(s.clone());
            }
            let rec = task_record(reg, &it.task, it.features)?;
            let out = d.decide(&rec)?;
            lat.push_back(out.latency_ms);
            while lat.len() > 50 {
                lat.pop_front();
            }
            let best = (0..out.probs.len()).max_by(|&a, &b| out.probs[a].total_cmp(&out.probs[b])).unwrap_or(0);
            let conf = out.probs[best];
            let th = resolve_threshold(None, Some(&it.task), d, reg).unwrap_or(0.95);
            let option = if conf < th { "abstain".to_string() } else { out.keys[best].clone() };
            let v = jury::Vote { entry_hash: eh.clone(), epoch: req.epoch, task: it.task.clone(), option,
                                 conf_q8: (conf * 255.0).round().clamp(0.0, 255.0) as u8 };
            let salt = jury::salt()?;
            let s = jury::Stored { leaf: jury::hex(&v.leaf(&salt)?), salt: jury::hex(&salt), vote: v };
            fresh.push(s.clone());
            Ok(s)
        })();
        match r {
            Ok(s) => {
                let t95 = resolve_threshold(None, Some(&s.vote.task), d, reg).unwrap_or(0.95);
                votes.push(json!({"entry_hash": s.vote.entry_hash, "task": s.vote.task, "option": s.vote.option,
                    "conf_q8": s.vote.conf_q8, "t95_q8": (t95 * 255.0).round().min(255.0) as u16, "leaf": s.leaf}))
            }
            Err(e) => errors.push(json!({"entry_hash": eh, "error": e.to_string()})),
        }
    }
    store.append(req.epoch, &fresh)?;
    Ok((200, json!({"epoch": req.epoch, "model_hash": model_hash, "votes": votes, "errors": errors,
                    "latency_ms": (t0.elapsed().as_secs_f64() * 1000.0).round()})))
}
