//! Qwen3.5 hybrid text backbone (Gated DeltaNet linear attention + gated full
//! attention), the base of Open-Jev-2B/9B. Written from the transformers 5.10.2
//! reference (`modeling_qwen3_5.py`); text-only, so interleaved MRoPE reduces to
//! plain RoPE on the first `head_dim * partial_rotary_factor` dims.
//!
//! The delta rule runs token-by-token (the exact recurrence); HF's chunked
//! prefill is the same math up to float rounding.

use crate::weights::{attention, rms_norm, rope_tables, Emb, Lin, Weights};
use anyhow::{bail, Result};
use candle_core::{Device, Tensor};
use rayon::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct RopeParams {
    pub rope_theta: f64,
    #[serde(default = "one")]
    pub partial_rotary_factor: f64,
}
fn one() -> f64 {
    1.0
}

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub hidden_size: usize,
    pub intermediate_size: usize,
    pub num_hidden_layers: usize,
    pub num_attention_heads: usize,
    pub num_key_value_heads: usize,
    pub head_dim: usize,
    pub layer_types: Vec<String>,
    pub linear_conv_kernel_dim: usize,
    pub linear_key_head_dim: usize,
    pub linear_value_head_dim: usize,
    pub linear_num_key_heads: usize,
    pub linear_num_value_heads: usize,
    pub rms_norm_eps: f64,
    pub rope_parameters: RopeParams,
}

struct Full {
    q: Lin,
    k: Lin,
    v: Lin,
    o: Lin,
    qn: Tensor,
    kn: Tensor,
}

struct Linear {
    qkv: Lin,
    z: Lin,
    b: Lin,
    a: Lin,
    out: Lin,
    conv: Vec<f32>, // [conv_dim, K]
    dt_bias: Vec<f32>,
    a_neg_exp: Vec<f32>, // -exp(A_log)
    norm: Tensor,
}

enum Mixer {
    Full(Full),
    Linear(Linear),
}

struct Layer {
    ln1: Tensor,
    ln2: Tensor,
    mix: Mixer,
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
    rot: usize,
    dev: Device,
}

#[derive(Clone)]
enum LState {
    Kv(Option<(Tensor, Tensor)>),
    /// conv tail [conv_dim * (K-1)] (time-major), recurrent S [Hv * dk * dv]
    Rec { conv: Vec<f32>, s: Vec<f32> },
}

#[derive(Clone)]
pub struct State {
    pub pos: usize,
    layers: Vec<LState>,
}

fn vecf(w: &Weights, n: &str) -> Result<Vec<f32>> {
    Ok(w.f32(n)?.flatten_all()?.to_vec1::<f32>()?)
}

impl Model {
    pub fn load(cfg: Config, w: &Weights, dev: &Device, max_len: usize) -> Result<Self> {
        let mut layers = Vec::new();
        for i in 0..cfg.num_hidden_layers {
            let n = |s: &str| format!("layers.{i}.{s}");
            let mix = match cfg.layer_types[i].as_str() {
                "full_attention" => Mixer::Full(Full {
                    q: w.linear(&n("self_attn.q_proj.weight"))?,
                    k: w.linear(&n("self_attn.k_proj.weight"))?,
                    v: w.linear(&n("self_attn.v_proj.weight"))?,
                    o: w.linear(&n("self_attn.o_proj.weight"))?,
                    qn: w.f32(&n("self_attn.q_norm.weight"))?,
                    kn: w.f32(&n("self_attn.k_norm.weight"))?,
                }),
                "linear_attention" => Mixer::Linear(Linear {
                    qkv: w.linear(&n("linear_attn.in_proj_qkv.weight"))?,
                    z: w.linear(&n("linear_attn.in_proj_z.weight"))?,
                    b: w.linear(&n("linear_attn.in_proj_b.weight"))?,
                    a: w.linear(&n("linear_attn.in_proj_a.weight"))?,
                    out: w.linear(&n("linear_attn.out_proj.weight"))?,
                    conv: vecf(w, &n("linear_attn.conv1d.weight"))?,
                    dt_bias: vecf(w, &n("linear_attn.dt_bias"))?,
                    a_neg_exp: vecf(w, &n("linear_attn.A_log"))?.iter().map(|x| -x.exp()).collect(),
                    norm: w.f32(&n("linear_attn.norm.weight"))?,
                }),
                t => bail!("unknown layer type {t}"),
            };
            layers.push(Layer {
                ln1: w.f32(&n("input_layernorm.weight"))?,
                ln2: w.f32(&n("post_attention_layernorm.weight"))?,
                mix,
                gate: w.linear(&n("mlp.gate_proj.weight"))?,
                up: w.linear(&n("mlp.up_proj.weight"))?,
                down: w.linear(&n("mlp.down_proj.weight"))?,
            });
        }
        let rot = (cfg.head_dim as f64 * cfg.rope_parameters.partial_rotary_factor) as usize;
        let (cos, sin) = rope_tables(rot, cfg.rope_parameters.rope_theta, max_len, dev)?;
        Ok(Model {
            emb: w.embedding("embed_tokens.weight")?,
            norm: w.f32("norm.weight")?,
            layers,
            cos,
            sin,
            rot,
            dev: dev.clone(),
            cfg,
        })
    }

    pub fn hidden_size(&self) -> usize {
        self.cfg.hidden_size
    }

    pub fn max_len(&self) -> usize {
        self.cos.dim(0).unwrap_or(0)
    }

    pub fn new_state(&self) -> State {
        let c = &self.cfg;
        let conv_dim = 2 * c.linear_num_key_heads * c.linear_key_head_dim + c.linear_num_value_heads * c.linear_value_head_dim;
        let layers = c
            .layer_types
            .iter()
            .map(|t| {
                if t == "full_attention" {
                    LState::Kv(None)
                } else {
                    LState::Rec {
                        conv: vec![0.0; conv_dim * (c.linear_conv_kernel_dim - 1)],
                        s: vec![0.0; c.linear_num_value_heads * c.linear_key_head_dim * c.linear_value_head_dim],
                    }
                }
            })
            .collect();
        State { pos: 0, layers }
    }

    pub fn forward(&self, st: &mut State, ids: &[u32]) -> Result<Vec<f32>> {
        let c = &self.cfg;
        let n = ids.len();
        if st.pos + n > self.max_len() {
            bail!("input length {} exceeds max_length {}", st.pos + n, self.max_len());
        }
        let mut x = self.emb.lookup(ids, &self.dev)?;
        for (li, l) in self.layers.iter().enumerate() {
            let r = x.clone();
            let y = rms_norm(&x, &l.ln1, c.rms_norm_eps, true)?;
            let m = match (&l.mix, &mut st.layers[li]) {
                (Mixer::Full(f), LState::Kv(kv)) => self.full(f, &y, st.pos, kv)?,
                (Mixer::Linear(la), LState::Rec { conv, s }) => self.linear(la, &y, conv, s)?,
                _ => bail!("state/layer mismatch"),
            };
            x = (r + m)?;
            let r = x.clone();
            let y = rms_norm(&x, &l.ln2, c.rms_norm_eps, true)?;
            let g = (candle_nn::ops::silu(&l.gate.fwd(&y)?)? * l.up.fwd(&y)?)?;
            x = (r + l.down.fwd(&g)?)?;
        }
        st.pos += n;
        let last = rms_norm(&x.narrow(0, n - 1, 1)?, &self.norm, c.rms_norm_eps, true)?;
        Ok(last.squeeze(0)?.to_vec1::<f32>()?)
    }

    fn full(&self, f: &Full, y: &Tensor, pos: usize, kv: &mut Option<(Tensor, Tensor)>) -> Result<Tensor> {
        let c = &self.cfg;
        let (n, _) = y.dims2()?;
        let (h, kvh, hd) = (c.num_attention_heads, c.num_key_value_heads, c.head_dim);
        let qg = f.q.fwd(y)?.reshape((n, h, 2 * hd))?;
        let q = qg.narrow(2, 0, hd)?.contiguous()?;
        let gate = qg.narrow(2, hd, hd)?.contiguous()?.reshape((n, h * hd))?;
        let q = rms_norm(&q, &f.qn, c.rms_norm_eps, true)?.transpose(0, 1)?.contiguous()?;
        let k = rms_norm(&f.k.fwd(y)?.reshape((n, kvh, hd))?, &f.kn, c.rms_norm_eps, true)?
            .transpose(0, 1)?
            .contiguous()?;
        let v = f.v.fwd(y)?.reshape((n, kvh, hd))?.transpose(0, 1)?.contiguous()?;
        let cos = self.cos.narrow(0, pos, n)?;
        let sin = self.sin.narrow(0, pos, n)?;
        let rope = |t: &Tensor| -> Result<Tensor> {
            let rot = t.narrow(2, 0, self.rot)?.contiguous()?.unsqueeze(0)?;
            let rot = candle_nn::rotary_emb::rope(&rot, &cos, &sin)?.squeeze(0)?;
            let pass = t.narrow(2, self.rot, hd - self.rot)?;
            Ok(Tensor::cat(&[&rot, &pass], 2)?.contiguous()?)
        };
        let (q, k) = (rope(&q)?, rope(&k)?);
        let (k, v) = match kv.as_ref() {
            Some((pk, pv)) => (Tensor::cat(&[pk, &k], 1)?, Tensor::cat(&[pv, &v], 1)?),
            None => (k, v),
        };
        *kv = Some((k.clone(), v.clone()));
        let a = attention(&q, &k, &v, 1.0 / (hd as f64).sqrt())?;
        let a = (a * candle_nn::ops::sigmoid(&gate)?)?;
        f.o.fwd(&a)
    }

    fn linear(&self, la: &Linear, y: &Tensor, conv_tail: &mut Vec<f32>, s: &mut Vec<f32>) -> Result<Tensor> {
        let c = &self.cfg;
        let (n, _) = y.dims2()?;
        let (hk, hv, dk, dv, kk) = (
            c.linear_num_key_heads,
            c.linear_num_value_heads,
            c.linear_key_head_dim,
            c.linear_value_head_dim,
            c.linear_conv_kernel_dim,
        );
        let key_dim = hk * dk;
        let cd = 2 * key_dim + hv * dv;
        let qkv = la.qkv.fwd(y)?.flatten_all()?.to_vec1::<f32>()?; // [n, cd]
        let z = la.z.fwd(y)?; // [n, hv*dv]
        let b = la.b.fwd(y)?.flatten_all()?.to_vec1::<f32>()?; // [n, hv]
        let a = la.a.fwd(y)?.flatten_all()?.to_vec1::<f32>()?;

        // causal depthwise conv over [tail ++ new], then SiLU
        let tl = kk - 1;
        let mut seq = Vec::with_capacity((tl + n) * cd);
        seq.extend_from_slice(conv_tail);
        seq.extend_from_slice(&qkv);
        let mut mixed = vec![0f32; n * cd];
        mixed.par_chunks_mut(cd).enumerate().for_each(|(t, row)| {
            for ch in 0..cd {
                let mut acc = 0f32;
                for j in 0..kk {
                    acc += la.conv[ch * kk + j] * seq[(t + j) * cd + ch];
                }
                row[ch] = acc / (1.0 + (-acc).exp());
            }
        });
        conv_tail.copy_from_slice(&seq[n * cd..]);

        let rep = hv / hk;
        let scale = 1.0 / (dk as f32).sqrt();
        let l2 = |v: &[f32]| -> Vec<f32> {
            let ss: f32 = v.iter().map(|x| x * x).sum();
            let inv = 1.0 / (ss + 1e-6).sqrt();
            v.iter().map(|x| x * inv).collect()
        };
        // per value-head recurrence, heads in parallel
        let mut out = vec![0f32; n * hv * dv];
        let mut outs: Vec<Vec<f32>> = vec![Vec::new(); hv];
        s.par_chunks_mut(dk * dv).zip(outs.par_iter_mut()).enumerate().for_each(|(h, (sh, oh))| {
            let kh = h / rep;
            oh.resize(n * dv, 0.0);
            let mut kv_mem = vec![0f32; dv];
            for t in 0..n {
                let row = &mixed[t * cd..(t + 1) * cd];
                let q = l2(&row[kh * dk..(kh + 1) * dk]);
                let k = l2(&row[key_dim + kh * dk..key_dim + (kh + 1) * dk]);
                let v = &row[2 * key_dim + h * dv..2 * key_dim + (h + 1) * dv];
                let beta = 1.0 / (1.0 + (-b[t * hv + h]).exp());
                let xa = a[t * hv + h] + la.dt_bias[h];
                let sp = if xa > 20.0 { xa } else { xa.exp().ln_1p() };
                let g = (la.a_neg_exp[h] * sp).exp();
                for e in sh.iter_mut() {
                    *e *= g;
                }
                kv_mem.iter_mut().for_each(|m| *m = 0.0);
                for i in 0..dk {
                    let ki = k[i];
                    let srow = &sh[i * dv..(i + 1) * dv];
                    for j in 0..dv {
                        kv_mem[j] += srow[j] * ki;
                    }
                }
                let o = &mut oh[t * dv..(t + 1) * dv];
                o.iter_mut().for_each(|m| *m = 0.0);
                for i in 0..dk {
                    let ki = k[i];
                    let qi = q[i] * scale;
                    let srow = &mut sh[i * dv..(i + 1) * dv];
                    for j in 0..dv {
                        srow[j] += ki * (v[j] - kv_mem[j]) * beta;
                        o[j] += srow[j] * qi;
                    }
                }
            }
        });
        for (h, oh) in outs.iter().enumerate() {
            for t in 0..n {
                out[(t * hv + h) * dv..(t * hv + h + 1) * dv].copy_from_slice(&oh[t * dv..(t + 1) * dv]);
            }
        }
        let core = Tensor::from_vec(out, (n * hv, dv), &self.dev)?;
        let z = z.reshape((n * hv, dv))?;
        let normed = (rms_norm(&core, &la.norm, c.rms_norm_eps, false)? * candle_nn::ops::silu(&z)?)?;
        la.out.fwd(&normed.reshape((n, hv * dv))?)
    }
}
