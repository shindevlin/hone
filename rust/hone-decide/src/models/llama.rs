//! Llama-architecture backbone (SmolLM2 students). Returns the final-normed hidden
//! state of the last token; no LM head is loaded.

use crate::weights::{attention, rms_norm, rope_tables, Emb, Lin, Weights};
use anyhow::Result;
use candle_core::{Device, Tensor};
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub hidden_size: usize,
    pub intermediate_size: usize,
    pub num_hidden_layers: usize,
    pub num_attention_heads: usize,
    pub num_key_value_heads: usize,
    #[serde(default = "eps")]
    pub rms_norm_eps: f64,
    #[serde(default = "theta")]
    pub rope_theta: f64,
    #[serde(default = "maxpos")]
    pub max_position_embeddings: usize,
}
fn eps() -> f64 {
    1e-5
}
fn theta() -> f64 {
    10000.0
}
fn maxpos() -> usize {
    8192
}

struct Layer {
    ln1: Tensor,
    ln2: Tensor,
    q: Lin,
    k: Lin,
    v: Lin,
    o: Lin,
    gate: Lin,
    up: Lin,
    down: Lin,
}

pub struct Model {
    cfg: Config,
    emb: Emb,
    layers: Vec<Layer>,
    norm: Tensor,
    cos: Tensor,
    sin: Tensor,
    dev: Device,
}

/// Per-layer KV cache. Cloning is cheap (tensors are immutable and shared).
#[derive(Clone, Default)]
pub struct State {
    pub pos: usize,
    kv: Vec<Option<(Tensor, Tensor)>>,
}

impl Model {
    pub fn load(cfg: Config, w: &Weights, dev: &Device, max_len: usize) -> Result<Self> {
        let p = |s: &str| format!("model.{s}");
        let pick = |s: &str| if w.has(&p(s)) { p(s) } else { s.to_string() };
        let mut layers = Vec::new();
        for i in 0..cfg.num_hidden_layers {
            let n = |s: &str| pick(&format!("layers.{i}.{s}"));
            layers.push(Layer {
                ln1: w.f32(&n("input_layernorm.weight"))?,
                ln2: w.f32(&n("post_attention_layernorm.weight"))?,
                q: w.linear(&n("self_attn.q_proj.weight"))?,
                k: w.linear(&n("self_attn.k_proj.weight"))?,
                v: w.linear(&n("self_attn.v_proj.weight"))?,
                o: w.linear(&n("self_attn.o_proj.weight"))?,
                gate: w.linear(&n("mlp.gate_proj.weight"))?,
                up: w.linear(&n("mlp.up_proj.weight"))?,
                down: w.linear(&n("mlp.down_proj.weight"))?,
            });
        }
        let hd = cfg.hidden_size / cfg.num_attention_heads;
        let (cos, sin) = rope_tables(hd, cfg.rope_theta, max_len.min(cfg.max_position_embeddings), dev)?;
        Ok(Model {
            emb: w.embedding(&pick("embed_tokens.weight"))?,
            norm: w.f32(&pick("norm.weight"))?,
            layers,
            cos,
            sin,
            dev: dev.clone(),
            cfg,
        })
    }

    pub fn new_state(&self) -> State {
        State { pos: 0, kv: vec![None; self.cfg.num_hidden_layers] }
    }

    pub fn max_len(&self) -> usize {
        self.cos.dim(0).unwrap_or(0)
    }

    /// Feed `ids` after whatever `st` already holds; returns the final-normed hidden of the last token.
    pub fn forward(&self, st: &mut State, ids: &[u32]) -> Result<Vec<f32>> {
        let c = &self.cfg;
        let n = ids.len();
        let (h, kvh) = (c.num_attention_heads, c.num_key_value_heads);
        let hd = c.hidden_size / h;
        if st.pos + n > self.max_len() {
            anyhow::bail!("input length {} exceeds max_length {}", st.pos + n, self.max_len());
        }
        let cos = self.cos.narrow(0, st.pos, n)?;
        let sin = self.sin.narrow(0, st.pos, n)?;
        let mut x = self.emb.lookup(ids, &self.dev)?;
        for (li, l) in self.layers.iter().enumerate() {
            let r = x.clone();
            let y = rms_norm(&x, &l.ln1, c.rms_norm_eps, false)?;
            let q = l.q.fwd(&y)?.reshape((n, h, hd))?.transpose(0, 1)?.contiguous()?;
            let k = l.k.fwd(&y)?.reshape((n, kvh, hd))?.transpose(0, 1)?.contiguous()?;
            let v = l.v.fwd(&y)?.reshape((n, kvh, hd))?.transpose(0, 1)?.contiguous()?;
            let q = candle_nn::rotary_emb::rope(&q.unsqueeze(0)?, &cos, &sin)?.squeeze(0)?;
            let k = candle_nn::rotary_emb::rope(&k.unsqueeze(0)?, &cos, &sin)?.squeeze(0)?;
            let (k, v) = match &st.kv[li] {
                Some((pk, pv)) => (Tensor::cat(&[pk, &k], 1)?, Tensor::cat(&[pv, &v], 1)?),
                None => (k, v),
            };
            st.kv[li] = Some((k.clone(), v.clone()));
            let a = attention(&q, &k, &v, 1.0 / (hd as f64).sqrt())?;
            x = (r + l.o.fwd(&a)?)?;
            let r = x.clone();
            let y = rms_norm(&x, &l.ln2, c.rms_norm_eps, false)?;
            let m = (candle_nn::ops::silu(&l.gate.fwd(&y)?)? * l.up.fwd(&y)?)?;
            x = (r + l.down.fwd(&m)?)?;
        }
        st.pos += n;
        let last = x.narrow(0, n - 1, 1)?;
        let last = rms_norm(&last, &self.norm, c.rms_norm_eps, false)?;
        Ok(last.squeeze(0)?.to_vec1::<f32>()?)
    }

    /// Run several continuations of the same cached prefix in one batch (right padded).
    /// All linear layers see one [B*n, H] matrix, so quantised weights are dequantised once
    /// per layer instead of once per candidate. Returns each sequence's last hidden state.
    pub fn forward_batch(&self, st: &State, seqs: &[&[u32]]) -> Result<Vec<Vec<f32>>> {
        let c = &self.cfg;
        let b = seqs.len();
        let n = seqs.iter().map(|s| s.len()).max().unwrap_or(0);
        let (h, kvh) = (c.num_attention_heads, c.num_key_value_heads);
        let hd = c.hidden_size / h;
        if n == 0 || st.pos + n > self.max_len() {
            anyhow::bail!("input length {} exceeds max_length {}", st.pos + n, self.max_len());
        }
        let mut ids = Vec::with_capacity(b * n);
        for s in seqs {
            ids.extend_from_slice(s);
            ids.extend(std::iter::repeat(0u32).take(n - s.len()));
        }
        let cos = self.cos.narrow(0, st.pos, n)?;
        let sin = self.sin.narrow(0, st.pos, n)?;
        let mut x = self.emb.lookup(&ids, &self.dev)?;
        for (li, l) in self.layers.iter().enumerate() {
            let r = x.clone();
            let y = rms_norm(&x, &l.ln1, c.rms_norm_eps, false)?;
            let (q, k, v) = (l.q.fwd(&y)?, l.k.fwd(&y)?, l.v.fwd(&y)?);
            let mut outs = Vec::with_capacity(b);
            for i in 0..b {
                let heads = |t: &Tensor, nh: usize| -> Result<Tensor> {
                    Ok(t.narrow(0, i * n, n)?.reshape((n, nh, hd))?.transpose(0, 1)?.contiguous()?)
                };
                let qi = candle_nn::rotary_emb::rope(&heads(&q, h)?.unsqueeze(0)?, &cos, &sin)?.squeeze(0)?;
                let ki = candle_nn::rotary_emb::rope(&heads(&k, kvh)?.unsqueeze(0)?, &cos, &sin)?.squeeze(0)?;
                let vi = heads(&v, kvh)?;
                let (ki, vi) = match &st.kv[li] {
                    Some((pk, pv)) => (Tensor::cat(&[pk, &ki], 1)?, Tensor::cat(&[pv, &vi], 1)?),
                    None => (ki, vi),
                };
                outs.push(attention(&qi, &ki, &vi, 1.0 / (hd as f64).sqrt())?);
            }
            let a = Tensor::cat(&outs, 0)?;
            x = (r + l.o.fwd(&a)?)?;
            let r = x.clone();
            let y = rms_norm(&x, &l.ln2, c.rms_norm_eps, false)?;
            let m = (candle_nn::ops::silu(&l.gate.fwd(&y)?)? * l.up.fwd(&y)?)?;
            x = (r + l.down.fwd(&m)?)?;
        }
        let mut res = Vec::with_capacity(b);
        for (i, s) in seqs.iter().enumerate() {
            let last = rms_norm(&x.narrow(0, i * n + s.len() - 1, 1)?, &self.norm, c.rms_norm_eps, false)?;
            res.push(last.squeeze(0)?.to_vec1::<f32>()?);
        }
        Ok(res)
    }

    pub fn hidden_size(&self) -> usize {
        self.cfg.hidden_size
    }
}

