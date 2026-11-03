//! Decider: tokenizer + backbone + scalar head + calibration temperature.

use crate::models::{llama, qwen35, Model};
use crate::prompt::{softmax, Kind, Record};
use crate::weights::Weights;
use anyhow::{bail, Context, Result};
use candle_core::Device;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokenizers::Tokenizer;

#[derive(Deserialize, Clone, Debug)]
pub struct DecideCfg {
    pub arch: String,
    #[serde(default)]
    pub name: String,
    pub weights: String,
    #[serde(default = "head_file")]
    pub head: String,
    #[serde(default = "config_file")]
    pub config: String,
    #[serde(default = "tok_file")]
    pub tokenizer: String,
    pub prompt_prefix: String,
    pub prompt_suffix: String,
    #[serde(default = "max_len")]
    pub max_length: usize,
    #[serde(default = "yes")]
    pub prefix_cache: bool,
}
fn head_file() -> String {
    "head.safetensors".into()
}
fn config_file() -> String {
    "config.json".into()
}
fn tok_file() -> String {
    "tokenizer.json".into()
}
fn max_len() -> usize {
    4096
}
fn yes() -> bool {
    true
}

pub struct Head {
    pub w: Vec<f32>,
    pub b: f32,
    pub temperature: f32,
}

impl Head {
    pub fn load(path: &Path) -> Result<Self> {
        let m = candle_core::safetensors::load(path, &Device::Cpu)?;
        let get = |k: &str| -> Result<Vec<f32>> {
            Ok(m.get(k).with_context(|| format!("head file lacks {k}"))?.flatten_all()?.to_vec1::<f32>()?)
        };
        let t = get("temperature")?;
        Ok(Head { w: get("head.weight")?, b: get("head.bias")?[0], temperature: t[0] })
    }
    pub fn score(&self, h: &[f32]) -> f32 {
        self.w.iter().zip(h).map(|(a, b)| a * b).sum::<f32>() + self.b
    }
}

pub struct Decider {
    pub cfg: DecideCfg,
    pub dir: PathBuf,
    model: Model,
    head: Head,
    tok: Tokenizer,
}

pub struct Decision {
    pub keys: Vec<String>,
    pub logits: Vec<f32>,
    pub probs: Vec<f32>,
    pub tokens: usize,
    pub latency_ms: f64,
}

impl Decider {
    pub fn load(dir: &Path, max_length: Option<usize>) -> Result<Self> {
        let dev = Device::Cpu;
        let mut cfg: DecideCfg = serde_json::from_slice(&std::fs::read(dir.join("decide.json")).context("decide.json")?)?;
        if let Some(m) = max_length {
            cfg.max_length = m;
        }
        let raw = std::fs::read(dir.join(&cfg.config)).context("config.json")?;
        let w = Weights::open(&dir.join(&cfg.weights), &dev)?;
        let model = match cfg.arch.as_str() {
            "llama" => Model::Llama(llama::Model::load(serde_json::from_slice(&raw)?, &w, &dev, cfg.max_length)?),
            "qwen35" => Model::Qwen35(qwen35::Model::load(serde_json::from_slice(&raw)?, &w, &dev, cfg.max_length)?),
            a => bail!("unknown arch {a}"),
        };
        drop(w);
        let head = Head::load(&dir.join(&cfg.head))?;
        if head.w.len() != model.hidden_size() {
            bail!("head size {} != hidden size {}", head.w.len(), model.hidden_size());
        }
        let tok = Tokenizer::from_file(dir.join(&cfg.tokenizer)).map_err(|e| anyhow::anyhow!("tokenizer: {e}"))?;
        Ok(Decider { cfg, dir: dir.to_path_buf(), model, head, tok })
    }

    pub fn name(&self) -> &str {
        &self.cfg.name
    }

    pub fn temperature(&self) -> f32 {
        self.head.temperature
    }

    pub fn encode(&self, user: &str) -> Result<Vec<u32>> {
        let text = format!("{}{}{}", self.cfg.prompt_prefix, user, self.cfg.prompt_suffix);
        let e = self.tok.encode(text, true).map_err(|e| anyhow::anyhow!("encode: {e}"))?;
        Ok(e.get_ids().to_vec())
    }

    /// Raw head logits per candidate (noul: [0, s]), plus total tokens processed.
    pub fn logits(&self, rec: &Record) -> Result<(Vec<f32>, usize)> {
        let seqs: Vec<Vec<u32>> = rec.candidate_prompts().iter().map(|p| self.encode(p)).collect::<Result<_>>()?;
        let longest = seqs.iter().map(|s| s.len()).max().unwrap_or(0);
        if longest > self.cfg.max_length {
            bail!("Input length {longest} exceeds max_length={}; no silent truncation", self.cfg.max_length);
        }
        let mut scores = Vec::with_capacity(seqs.len());
        let mut tokens = 0;
        if self.cfg.prefix_cache && seqs.len() > 1 {
            let lcp = common_prefix(&seqs);
            let mut base = self.model.new_state();
            // feed the shared prefix in chunks: same result (causal + cache), lower peak memory
            for chunk in seqs[0][..lcp].chunks(prefill_chunk()) {
                self.model.forward(&mut base, chunk)?;
            }
            tokens += lcp;
            let tails: Vec<&[u32]> = seqs.iter().map(|s| &s[lcp..]).collect();
            for (t, h) in tails.iter().zip(self.model.forward_many(&base, &tails)?) {
                tokens += t.len();
                scores.push(self.head.score(&h));
            }
        } else {
            for s in &seqs {
                let mut st = self.model.new_state();
                let h = self.model.forward(&mut st, s)?;
                tokens += s.len();
                scores.push(self.head.score(&h));
            }
        }
        if rec.kind == Kind::Noul {
            scores = vec![0.0, scores[0]];
        }
        Ok((scores, tokens))
    }

    pub fn decide(&self, rec: &Record) -> Result<Decision> {
        let t0 = Instant::now();
        let (logits, tokens) = self.logits(rec)?;
        let probs = softmax(&logits, self.head.temperature);
        Ok(Decision {
            keys: rec.answer_keys.clone(),
            logits,
            probs,
            tokens,
            latency_ms: t0.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

fn prefill_chunk() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| std::env::var("HONE_DECIDE_CHUNK").ok().and_then(|v| v.parse().ok()).unwrap_or(128))
}

/// Exact token LCP, leaving at least one token per sequence to score.
pub fn common_prefix(seqs: &[Vec<u32>]) -> usize {
    let limit = seqs.iter().map(|s| s.len()).min().unwrap_or(1).saturating_sub(1);
    let first = &seqs[0];
    (0..limit).find(|&i| seqs[1..].iter().any(|s| s[i] != first[i])).unwrap_or(limit)
}
