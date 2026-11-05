//! Perch embedding -> compact voiceprint.
//!
//! A voiceprint is `W · (e / |e|)` quantised to int8, where `W` is a fixed 64 x 1536 matrix.
//! Centring by the population mean happens later, at compare time: projection is linear, so
//! `W(x - mu) = Wx - W mu` and prints computed early stay comparable as the mean settles.
//!
//! Not invertible: 1,536 Perch dimensions (themselves a lossy summary of 5 s of audio, from which
//! audio cannot be rebuilt) are folded into 64 int8 numbers. There is no audio path back.

use serde::{Deserialize, Serialize};

pub const PERCH_DIM: usize = 1536;
pub const VP_DIM: usize = 64;
/// Seed of the shipped projection ("HONE"). Changing it changes every voiceprint and bird id.
pub const DEFAULT_SEED: u64 = 0x484F_4E45;

/// A voiceprint: 64 int8 values and one scale. Kept on the device only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Voiceprint {
    pub q: Vec<i8>,
    pub scale: f32,
}

impl Voiceprint {
    pub fn to_f32(&self) -> [f32; VP_DIM] {
        let mut o = [0f32; VP_DIM];
        for (i, v) in self.q.iter().enumerate().take(VP_DIM) {
            o[i] = *v as f32 * self.scale;
        }
        o
    }
    pub fn quantise(y: &[f32; VP_DIM]) -> Self {
        let m = y.iter().fold(0f32, |a, v| a.max(v.abs())).max(1e-12);
        let q = y.iter().map(|v| (v / m * 127.0).round().clamp(-127.0, 127.0) as i8).collect();
        Voiceprint { q, scale: m / 127.0 }
    }
}

pub struct Projection {
    w: Vec<f32>, // row-major VP_DIM x PERCH_DIM
}

impl Projection {
    /// Gaussian random projection from splitmix64 + Box-Muller. Same numbers as the Kotlin port
    /// and the Python evaluation (`scripts/eval.py::rp`).
    pub fn seeded(seed: u64) -> Self {
        let n = VP_DIM * PERCH_DIM;
        let mut w = Vec::with_capacity(n);
        let mut st = seed;
        let mut next = || {
            st = st.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = st;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        while w.len() < n {
            let u1 = ((next() >> 11) as f64 + 1.0) / 9_007_199_254_740_993.0;
            let u2 = (next() >> 11) as f64 / 9_007_199_254_740_992.0;
            let r = (-2.0 * u1.ln()).sqrt();
            let th = 2.0 * std::f64::consts::PI * u2;
            w.push((r * th.cos()) as f32);
            if w.len() < n {
                w.push((r * th.sin()) as f32);
            }
        }
        Projection { w }
    }

    /// A learned matrix (e.g. PCA), row-major VP_DIM x PERCH_DIM.
    pub fn from_matrix(w: Vec<f32>) -> Option<Self> {
        (w.len() == VP_DIM * PERCH_DIM).then_some(Projection { w })
    }

    /// Projects one Perch embedding. None if the length is wrong or the vector is all zero/NaN.
    pub fn project(&self, emb: &[f32]) -> Option<[f32; VP_DIM]> {
        if emb.len() != PERCH_DIM {
            return None;
        }
        let n = emb.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>().sqrt();
        if !n.is_finite() || n < 1e-9 {
            return None;
        }
        let mut y = [0f32; VP_DIM];
        for (r, yr) in y.iter_mut().enumerate() {
            let row = &self.w[r * PERCH_DIM..(r + 1) * PERCH_DIM];
            let mut acc = 0f64;
            for (a, b) in row.iter().zip(emb) {
                acc += (*a as f64) * (*b as f64);
            }
            *yr = (acc / n) as f32;
        }
        Some(y)
    }
}

pub(crate) fn sub(a: &[f32; VP_DIM], b: &[f32; VP_DIM]) -> [f32; VP_DIM] {
    let mut o = [0f32; VP_DIM];
    for i in 0..VP_DIM {
        o[i] = a[i] - b[i];
    }
    o
}

pub(crate) fn cosine(a: &[f32; VP_DIM], b: &[f32; VP_DIM]) -> f32 {
    let (mut ab, mut aa, mut bb) = (0f64, 0f64, 0f64);
    for i in 0..VP_DIM {
        ab += a[i] as f64 * b[i] as f64;
        aa += a[i] as f64 * a[i] as f64;
        bb += b[i] as f64 * b[i] as f64;
    }
    if aa < 1e-18 || bb < 1e-18 {
        return 0.0;
    }
    (ab / (aa.sqrt() * bb.sqrt())) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seeded_is_deterministic_and_gaussianish() {
        let a = Projection::seeded(DEFAULT_SEED);
        let b = Projection::seeded(DEFAULT_SEED);
        assert_eq!(a.w, b.w);
        let mean: f64 = a.w.iter().map(|v| *v as f64).sum::<f64>() / a.w.len() as f64;
        let var: f64 = a.w.iter().map(|v| (*v as f64 - mean).powi(2)).sum::<f64>() / a.w.len() as f64;
        assert!(mean.abs() < 0.01 && (var - 1.0).abs() < 0.02, "{mean} {var}");
        // golden values shared with the Kotlin port
        assert_eq!(format!("{:.6} {:.6} {:.6}", a.w[0], a.w[1], a.w[98303]), GOLDEN);
    }
    pub(crate) const GOLDEN: &str = "0.552339 -0.851627 -0.767298";

    #[test]
    fn quantise_roundtrip() {
        let mut y = [0f32; VP_DIM];
        for (i, v) in y.iter_mut().enumerate() {
            *v = (i as f32 - 30.0) / 7.0;
        }
        let q = Voiceprint::quantise(&y);
        assert!(cosine(&q.to_f32(), &y) > 0.9999);
    }
}
