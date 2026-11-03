//! Localhost HTTP sidecar.
//!   POST /decide {context, question, candidates[], kind?, task?} -> {probs[], candidates[], choice, model, latency_ms, tokens}
//!   GET  /health -> {ok, model, rss_mb, mem_available_mb}
//!   GET  /tasks  -> built-in HONE task registry

use crate::engine::Decider;
use crate::prompt::{Ordered, Record};
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

pub fn serve(d: Decider, bind: &str) -> Result<()> {
    let reg = Registry::load()?;
    let server = tiny_http::Server::http(bind).map_err(|e| anyhow!("bind {bind}: {e}"))?;
    eprintln!("hone-decide: model {} listening on http://{bind}", d.name());
    let jhdr = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    for mut rq in server.incoming_requests() {
        let (code, body) = match (rq.method().as_str(), rq.url()) {
            ("GET", "/health") => (200, json!({"ok": true, "model": d.name(),
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
            _ => (404, json!({"error": "not found"})),
        };
        let resp = tiny_http::Response::from_string(body.to_string()).with_status_code(code).with_header(jhdr.clone());
        let _ = rq.respond(resp);
    }
    Ok(())
}
