//! Weight sources: safetensors (dense f32/bf16/f16) or GGUF (quantized, written by
//! `hone-decide quantize`). Both expose the same names.

use anyhow::{anyhow, bail, Context, Result};
use candle_core::quantized::{gguf_file, GgmlDType, QMatMul, QTensor};
use candle_core::{DType, Device, Module, Tensor};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub enum Weights {
    Dense(HashMap<String, Tensor>),
    Gguf(HashMap<String, Arc<QTensor>>),
}

impl Weights {
    pub fn open(path: &Path, dev: &Device) -> Result<Self> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext == "gguf" {
            let mut f = std::fs::File::open(path).with_context(|| format!("open {path:?}"))?;
            let c = gguf_file::Content::read(&mut f)?;
            let mut m = HashMap::new();
            let names: Vec<String> = c.tensor_infos.keys().cloned().collect();
            for n in names {
                let t = c.tensor(&mut f, &n, dev)?;
                m.insert(n, Arc::new(t));
            }
            Ok(Weights::Gguf(m))
        } else {
            let m = candle_core::safetensors::load(path, dev).with_context(|| format!("load {path:?}"))?;
            Ok(Weights::Dense(m))
        }
    }

    pub fn has(&self, name: &str) -> bool {
        match self {
            Weights::Dense(m) => m.contains_key(name),
            Weights::Gguf(m) => m.contains_key(name),
        }
    }

    /// Any tensor as dense f32.
    pub fn f32(&self, name: &str) -> Result<Tensor> {
        match self {
            Weights::Dense(m) => Ok(m.get(name).ok_or_else(|| anyhow!("missing tensor {name}"))?.to_dtype(DType::F32)?),
            Weights::Gguf(m) => {
                let q = m.get(name).ok_or_else(|| anyhow!("missing tensor {name}"))?;
                Ok(q.dequantize(&q.device())?)
            }
        }
    }

    pub fn linear(&self, name: &str) -> Result<Lin> {
        match self {
            Weights::Dense(_) => Ok(Lin(QMatMul::Tensor(self.f32(name)?))),
            Weights::Gguf(m) => {
                let q = m.get(name).ok_or_else(|| anyhow!("missing tensor {name}"))?;
                Ok(Lin(QMatMul::from_arc(q.clone())?))
            }
        }
    }

    pub fn embedding(&self, name: &str) -> Result<Emb> {
        if let Weights::Gguf(m) = self {
            let q = m.get(name).ok_or_else(|| anyhow!("missing tensor {name}"))?;
            if q.dtype() == GgmlDType::Q8_0 {
                let (rows, dim) = q.shape().dims2()?;
                return Ok(Emb::Q8 { data: q.data()?.into_owned(), rows, dim });
            }
        }
        Ok(Emb::Dense(self.f32(name)?))
    }
}

/// Linear layer without bias, dense or quantized.
pub struct Lin(pub QMatMul);
impl Lin {
    /// Quantized weights: candle's CPU q-matmul is a per-row vec_dot (slow for prefill),
    /// so for >= `deq_rows()` input rows the layer is dequantized into a transient f32
    /// matrix and run through gemm. Weights stay quantized at rest.
    pub fn fwd(&self, x: &Tensor) -> Result<Tensor> {
        if let QMatMul::QTensor(q) = &self.0 {
            let rows = x.elem_count() / x.dim(candle_core::D::Minus1)?;
            if rows >= deq_rows() {
                let w = q.dequantize(x.device())?;
                return Ok(x.matmul(&w.t()?)?);
            }
        }
        Ok(self.0.forward(x)?)
    }
}

fn deq_rows() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| std::env::var("HONE_DECIDE_DEQ_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(4))
}

/// Token embedding. Q8_0 rows are dequantized on lookup so the table stays small.
pub enum Emb {
    Dense(Tensor),
    Q8 { data: Vec<u8>, rows: usize, dim: usize },
}

impl Emb {
    pub fn lookup(&self, ids: &[u32], dev: &Device) -> Result<Tensor> {
        match self {
            Emb::Dense(w) => {
                let idx = Tensor::new(ids, w.device())?;
                Ok(w.index_select(&idx, 0)?)
            }
            Emb::Q8 { data, rows, dim } => {
                let blocks = dim / 32;
                let row_bytes = blocks * 34;
                let mut out = Vec::with_capacity(ids.len() * dim);
                for &id in ids {
                    let id = id as usize;
                    if id >= *rows {
                        bail!("token id {id} out of range");
                    }
                    let r = &data[id * row_bytes..(id + 1) * row_bytes];
                    for b in 0..blocks {
                        let blk = &r[b * 34..(b + 1) * 34];
                        let d = half_to_f32(u16::from_le_bytes([blk[0], blk[1]]));
                        for j in 0..32 {
                            out.push(d * (blk[2 + j] as i8) as f32);
                        }
                    }
                }
                Ok(Tensor::from_vec(out, (ids.len(), *dim), dev)?)
            }
        }
    }
}

fn half_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1f) as u32;
    let frac = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if frac == 0 {
            sign << 31
        } else {
            // subnormal
            let mut e = 127 - 15 + 1;
            let mut f = frac;
            while f & 0x400 == 0 {
                f <<= 1;
                e -= 1;
            }
            (sign << 31) | ((e as u32) << 23) | ((f & 0x3ff) << 13)
        }
    } else if exp == 31 {
        (sign << 31) | (0xff << 23) | (frac << 13)
    } else {
        (sign << 31) | ((exp + 127 - 15) << 23) | (frac << 13)
    };
    f32::from_bits(bits)
}

pub fn rms_norm(x: &Tensor, w: &Tensor, eps: f64, one_plus: bool) -> Result<Tensor> {
    let var = x.sqr()?.mean_keepdim(candle_core::D::Minus1)?;
    let y = x.broadcast_div(&(var + eps)?.sqrt()?)?;
    let w = if one_plus { (w + 1.0)? } else { w.clone() };
    Ok(y.broadcast_mul(&w)?)
}

/// cos/sin tables [max_pos, dim/2] for rotate-half RoPE.
pub fn rope_tables(dim: usize, theta: f64, max_pos: usize, dev: &Device) -> Result<(Tensor, Tensor)> {
    let inv: Vec<f32> = (0..dim / 2).map(|i| (1.0 / theta.powf((2 * i) as f64 / dim as f64)) as f32).collect();
    let mut c = Vec::with_capacity(max_pos * dim / 2);
    let mut s = Vec::with_capacity(max_pos * dim / 2);
    for p in 0..max_pos {
        for &f in &inv {
            let a = p as f32 * f;
            c.push(a.cos());
            s.push(a.sin());
        }
    }
    Ok((Tensor::from_vec(c, (max_pos, dim / 2), dev)?, Tensor::from_vec(s, (max_pos, dim / 2), dev)?))
}

/// Causal attention for `n` new queries over `l` keys (the last `n` keys are the new ones).
/// q [h, n, d], k/v [kvh, l, d] -> [n, h*d]
pub fn attention(q: &Tensor, k: &Tensor, v: &Tensor, scale: f64) -> Result<Tensor> {
    let (h, n, d) = q.dims3()?;
    let (kvh, l, _) = k.dims3()?;
    let rep = h / kvh;
    let expand = |t: &Tensor| -> Result<Tensor> {
        if rep == 1 {
            return Ok(t.clone());
        }
        Ok(t.unsqueeze(1)?.broadcast_as((kvh, rep, l, d))?.reshape((h, l, d))?)
    };
    let (k, v) = (expand(k)?.contiguous()?, expand(v)?.contiguous()?);
    let att = (q.contiguous()?.matmul(&k.t()?)? * scale)?;
    let past = l - n;
    let mut mask = vec![0f32; n * l];
    for i in 0..n {
        for j in (past + i + 1)..l {
            mask[i * l + j] = f32::NEG_INFINITY;
        }
    }
    let mask = Tensor::from_vec(mask, (n, l), q.device())?;
    let att = candle_nn::ops::softmax_last_dim(&att.broadcast_add(&mask)?)?;
    let o = att.matmul(&v)?; // [h, n, d]
    Ok(o.transpose(0, 1)?.reshape((n, h * d))?)
}
