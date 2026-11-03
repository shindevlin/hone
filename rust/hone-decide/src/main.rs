use anyhow::{anyhow, bail, Result};
use candle_core::quantized::{gguf_file, GgmlDType, QTensor};
use candle_core::Device;
use hone_decide::engine::Decider;
use hone_decide::prompt::{Ordered, Record, Request};
use hone_decide::server;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

const USAGE: &str = "hone-decide <command> [--key value ...]
  serve    --model DIR [--bind 127.0.0.1:4250] [--max-length N] [--min-free-mb N] [--max-rss-mb N]
  score    --model DIR --rows rows.jsonl --out out.jsonl [--dump-tokens f.jsonl] [--no-prefix-cache]
  quantize --model DIR --out DIR --dtype q8_0|q4
  bench    --model DIR [--ctx 200,500] [--n 5]";

fn args() -> (String, HashMap<String, String>) {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().unwrap_or_default();
    let mut m = HashMap::new();
    let v: Vec<String> = it.collect();
    let mut i = 0;
    while i < v.len() {
        let k = v[i].trim_start_matches("--").to_string();
        if i + 1 < v.len() && !v[i + 1].starts_with("--") {
            m.insert(k, v[i + 1].clone());
            i += 2;
        } else {
            m.insert(k, "1".into());
            i += 1;
        }
    }
    (cmd, m)
}

fn need<'a>(m: &'a HashMap<String, String>, k: &str) -> Result<&'a String> {
    m.get(k).ok_or_else(|| anyhow!("missing --{k}\n{USAGE}"))
}

fn main() -> Result<()> {
    let (cmd, a) = args();
    match cmd.as_str() {
        "serve" => {
            let d = Decider::load(Path::new(need(&a, "model")?), a.get("max-length").map(|s| s.parse()).transpose()?)?;
            let min_free: u64 = a.get("min-free-mb").map(|s| s.parse()).transpose()?.unwrap_or(0);
            let max_rss: u64 = a.get("max-rss-mb").map(|s| s.parse()).transpose()?.unwrap_or(0);
            server::spawn_memguard(min_free, max_rss);
            let bind = a.get("bind").cloned().unwrap_or_else(|| "127.0.0.1:4250".into());
            server::serve(d, &bind)
        }
        "score" => score(&a),
        "quantize" => quantize(&a),
        "bench" => bench(&a),
        _ => {
            eprintln!("{USAGE}");
            Ok(())
        }
    }
}

#[derive(serde::Deserialize)]
struct Row {
    id: String,
    #[serde(default)]
    request: Option<Request>,
    #[serde(default)]
    context: Option<Value>,
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    candidates: Option<Ordered<Value>>,
}

fn row_records(r: Row) -> Result<Vec<(String, Record)>> {
    if let Some(req) = r.request {
        return Ok(req.compile()?.into_iter().map(|(q, rec)| (format!("{}/{}", r.id, q), rec)).collect());
    }
    let c = r.candidates.ok_or_else(|| anyhow!("row {} has no candidates", r.id))?;
    let rec = Record::choice(
        r.context.unwrap_or(Value::Null),
        &r.question.unwrap_or_default(),
        c.0.into_iter().map(|(k, v)| (k, Some(v))).collect(),
    )?;
    Ok(vec![(r.id, rec)])
}

fn score(a: &HashMap<String, String>) -> Result<()> {
    let mut d = Decider::load(Path::new(need(a, "model")?), a.get("max-length").map(|s| s.parse()).transpose()?)?;
    if a.contains_key("no-prefix-cache") {
        d.cfg.prefix_cache = false;
    }
    let mut out = std::fs::File::create(need(a, "out")?)?;
    let mut dump = a.get("dump-tokens").map(std::fs::File::create).transpose()?;
    let f = std::io::BufReader::new(std::fs::File::open(need(a, "rows")?)?);
    let t0 = std::time::Instant::now();
    let mut n = 0;
    for line in f.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        for (id, rec) in row_records(serde_json::from_str(&line)?)? {
            if let Some(df) = dump.as_mut() {
                for p in rec.candidate_prompts() {
                    writeln!(df, "{}", json!({"id": id, "ids": d.encode(&p)?}))?;
                }
            }
            let r = d.decide(&rec)?;
            writeln!(out, "{}", json!({"id": id, "answer_keys": r.keys, "logits": r.logits, "probs": r.probs,
                "tokens": r.tokens, "latency_ms": r.latency_ms}))?;
            n += 1;
        }
    }
    eprintln!("scored {n} records in {:.1}s", t0.elapsed().as_secs_f64());
    Ok(())
}

fn quantize(a: &HashMap<String, String>) -> Result<()> {
    let src = PathBuf::from(need(a, "model")?);
    let dst = PathBuf::from(need(a, "out")?);
    let dtype = need(a, "dtype")?.as_str();
    std::fs::create_dir_all(&dst)?;
    let mut cfg: Value = serde_json::from_slice(&std::fs::read(src.join("decide.json"))?)?;
    let wfile = cfg["weights"].as_str().unwrap_or("model.safetensors").to_string();
    let tensors = candle_core::safetensors::load(src.join(&wfile), &Device::Cpu)?;
    let mut q: Vec<(String, QTensor)> = Vec::new();
    let mut names: Vec<&String> = tensors.keys().collect();
    names.sort();
    for name in names {
        let t = tensors[name].to_dtype(candle_core::DType::F32)?;
        let dims = t.dims().to_vec();
        let qt = if name.ends_with("embed_tokens.weight") {
            QTensor::quantize(&t, GgmlDType::Q8_0)?
        } else if dims.len() == 2 && name.ends_with("proj.weight") {
            let g = match dtype {
                "q8_0" => GgmlDType::Q8_0,
                "q4" if dims[1] % 256 == 0 => GgmlDType::Q4K,
                "q4" => GgmlDType::Q4_0,
                _ => bail!("dtype must be q8_0 or q4"),
            };
            QTensor::quantize(&t, g)?
        } else {
            QTensor::quantize(&t, GgmlDType::F32)?
        };
        q.push((name.clone(), qt));
    }
    let out_name = format!("model-{dtype}.gguf");
    let mut f = std::fs::File::create(dst.join(&out_name))?;
    let arch = gguf_file::Value::String(format!("hone-decide-{}", cfg["arch"].as_str().unwrap_or("llama")));
    let refs: Vec<(&str, &QTensor)> = q.iter().map(|(n, t)| (n.as_str(), t)).collect();
    gguf_file::write(&mut f, &[("general.architecture", &arch)], &refs)?;
    for extra in ["config.json", "tokenizer.json", "head.safetensors"] {
        std::fs::copy(src.join(extra), dst.join(extra))?;
    }
    cfg["weights"] = json!(out_name);
    cfg["name"] = json!(format!("{}-{dtype}", cfg["name"].as_str().unwrap_or("student")));
    std::fs::write(dst.join("decide.json"), serde_json::to_string_pretty(&cfg)?)?;
    eprintln!("wrote {:?}", dst.join(out_name));
    Ok(())
}

fn bench(a: &HashMap<String, String>) -> Result<()> {
    let d = Decider::load(Path::new(need(a, "model")?), Some(1024))?;
    let reg = hone_decide::tasks::Registry::load()?;
    let task = reg.get("sensor_plausibility").unwrap();
    let ctxs: Vec<usize> = a.get("ctx").map(|s| s.as_str()).unwrap_or("200,500").split(',').map(|x| x.parse()).collect::<Result<_, _>>()?;
    let n: usize = a.get("n").map(|s| s.parse()).transpose()?.unwrap_or(5);
    let mut results = Vec::new();
    for target in ctxs {
        // grow a realistic sensor history until the context alone is ~target tokens
        let mut recent = Vec::new();
        let mut ctx;
        let mut i = 0;
        loop {
            ctx = json!({"sensor_id": "moto-gpower-baro", "owner": "josh", "kind": "barometer", "unit": "hPa",
                "physical_range": [300.0, 1100.0], "cell": "8a4d2f19c3b7e11", "epoch_seconds": 30,
                "recent": recent, "latest": {"epoch": 349400, "value": 915.42}});
            let ntok = d.encode(&hone_decide::pyjson::dumps(&ctx))?.len();
            if ntok >= target {
                break;
            }
            recent.push(json!({"epoch": 349000 + i * 2, "value": 915.0 + ((i * 37 % 17) as f64) / 10.0}));
            i += 1;
        }
        let rec = Record::choice(ctx, &task.question, task.candidates.0.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect())?;
        let mut lat = Vec::new();
        let mut toks = 0;
        for _ in 0..n {
            let r = d.decide(&rec)?;
            lat.push(r.latency_ms);
            toks = r.tokens;
        }
        lat.sort_by(|x, y| x.total_cmp(y));
        let hwm = std::fs::read_to_string("/proc/self/status").ok().and_then(|s| {
            s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        });
        results.push(json!({"context_tokens": target, "tokens_processed": toks, "candidates": 3,
            "latency_ms_median": lat[lat.len() / 2], "latency_ms_min": lat[0], "latency_ms_max": lat[lat.len() - 1],
            "peak_rss_mb": hwm.map(|k| k / 1024)}));
    }
    println!("{}", json!({"model": d.name(), "results": results}));
    Ok(())
}
