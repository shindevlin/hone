//! HONE voice gate, ported exactly from the Android app (android/voice-gate branch,
//! sound/VoiceGate.kt; reference tools/voice-gate/voice_gate.py). Finds voiced human speech
//! (pitch 70-500 Hz, energy below 1 kHz) in 100 ms blocks of one 5 s, 32 kHz frame.
//! Measured speech leak on the reference set: < 0.2 % (README in tools/voice-gate).
//!
//! The listener is stricter than the phone: ANY speech block drops the whole frame (only
//! the count is kept), and Perch's human classes are a second gate on what is left.

use std::f64::consts::PI;
use std::sync::OnceLock;

pub const SR: usize = 32000;
pub const BLOCK: usize = 3200;
pub const DEC: usize = 4;
pub const WIN: usize = 256;
pub const HOP: usize = 80;
pub const LAG_MIN: usize = 16;
pub const LAG_MAX: usize = 114;
pub const VOICED_R: f64 = 0.65;
pub const ABS_FLOOR_DB: f64 = -70.0;
pub const FLOOR_DB: f64 = 6.0;
pub const LOW_SHARE_DB: f64 = -8.0;
pub const LOUD_DB: f64 = -50.0;
pub const MIN_VOICED: usize = 3;
pub const MARGIN_BLOCKS: usize = 3;
pub const DROP_FRACTION: f64 = 0.5;
const NTAPS: usize = 63;

#[allow(dead_code)]
pub struct GateResult {
    pub mask: Vec<bool>,
    pub drop: bool,
    pub voiced: Vec<bool>,
    pub peak: Vec<f64>,
    pub energy_db: Vec<f64>,
}

impl GateResult {
    pub fn muted_blocks(&self) -> usize {
        self.mask.iter().filter(|m| **m).count()
    }
    /// Any speech at all in the frame.
    pub fn has_voice(&self) -> bool {
        self.mask.iter().any(|m| *m)
    }
}

fn taps() -> &'static [f64; NTAPS] {
    static T: OnceLock<[f64; NTAPS]> = OnceLock::new();
    T.get_or_init(|| {
        let mut h = [0f64; NTAPS];
        let fc = 1000.0 / SR as f64;
        for (i, hv) in h.iter_mut().enumerate() {
            let m = i as f64 - (NTAPS - 1) as f64 / 2.0;
            let a = 2.0 * fc * m;
            let sinc = if a == 0.0 { 1.0 } else { (PI * a).sin() / (PI * a) };
            *hv = 2.0 * fc * sinc * (0.54 - 0.46 * (2.0 * PI * i as f64 / (NTAPS - 1) as f64).cos());
        }
        let s: f64 = h.iter().sum();
        for hv in h.iter_mut() {
            *hv /= s;
        }
        h
    })
}

fn hann() -> &'static [f64; WIN] {
    static H: OnceLock<[f64; WIN]> = OnceLock::new();
    H.get_or_init(|| {
        let mut h = [0f64; WIN];
        for (i, v) in h.iter_mut().enumerate() {
            *v = 0.5 - 0.5 * (2.0 * PI * i as f64 / (WIN - 1) as f64).cos();
        }
        h
    })
}

pub fn analyze(x: &[f32]) -> GateResult {
    let taps = taps();
    let hann = hann();
    let half = (NTAPS - 1) / 2;
    let ny = x.len().div_ceil(DEC);
    let mut y = vec![0f64; ny];
    for (q, yv) in y.iter_mut().enumerate() {
        let i = q * DEC;
        let mut acc = 0.0;
        for (j, t) in taps.iter().enumerate() {
            let k = i as isize + half as isize - j as isize;
            if k >= 0 && (k as usize) < x.len() {
                acc += t * x[k as usize] as f64;
            }
        }
        *yv = acc;
    }
    let n = if ny >= WIN { 1 + (ny - WIN) / HOP } else { 0 };
    let mut peak = vec![0f64; n];
    let mut edb = vec![0f64; n];
    let mut share = vec![0f64; n];
    let mut f = [0f64; WIN];
    for i in 0..n {
        let off = i * HOP;
        let mean: f64 = y[off..off + WIN].iter().sum::<f64>() / WIN as f64;
        let mut e = 0.0;
        for k in 0..WIN {
            let v = (y[off + k] - mean) * hann[k];
            f[k] = v;
            e += v * v;
        }
        edb[i] = 10.0 * (e / WIN as f64 + 1e-20).log10();
        let mut best = f64::NEG_INFINITY;
        for l in LAG_MIN..=LAG_MAX {
            let mut ac = 0.0;
            for k in 0..WIN - l {
                ac += f[k] * f[k + l];
            }
            let r = ac / (e + 1e-20) * (WIN as f64 / (WIN - l).max(1) as f64);
            if r > best {
                best = r;
            }
        }
        peak[i] = best;
        let start = i * HOP * DEC;
        let mut fe = 0.0;
        for k in 0..WIN * DEC {
            let v = x[(start + k).min(x.len() - 1)] as f64;
            fe += v * v;
        }
        share[i] = edb[i] - 10.0 * (fe / (WIN * DEC) as f64 + 1e-20).log10();
    }
    let noise_floor = if n == 0 {
        0.0
    } else {
        let mut s = edb.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pos = 0.1 * (n - 1) as f64;
        let lo = pos.floor() as usize;
        let hi = (lo + 1).min(n - 1);
        s[lo] + (pos - lo as f64) * (s[hi] - s[lo])
    };
    let voiced: Vec<bool> = (0..n)
        .map(|i| {
            peak[i] >= VOICED_R
                && edb[i] >= ABS_FLOOR_DB
                && (edb[i] >= noise_floor + FLOOR_DB || edb[i] >= LOUD_DB)
                && share[i] >= LOW_SHARE_DB
        })
        .collect();
    let n_blocks = x.len() / BLOCK;
    let hops_per_block = BLOCK / DEC / HOP;
    let core: Vec<bool> = (0..n_blocks)
        .map(|b| (b * hops_per_block..((b + 1) * hops_per_block).min(n)).filter(|h| voiced[*h]).count() >= MIN_VOICED)
        .collect();
    let mut mask = vec![false; n_blocks];
    for b in 0..n_blocks {
        if core[b] {
            for m in mask.iter_mut().take((b + MARGIN_BLOCKS).min(n_blocks - 1) + 1).skip(b.saturating_sub(MARGIN_BLOCKS)) {
                *m = true;
            }
        }
    }
    let drop = n_blocks > 0 && mask.iter().filter(|m| **m).count() as f64 / n_blocks as f64 > DROP_FRACTION;
    GateResult { mask, drop, voiced, peak, energy_db: edb }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bird(t: f64) -> f64 {
        if (t * 4.0) % 1.0 < 0.5 { 0.05 * (2.0 * PI * 3000.0 * t).sin() } else { 0.0 }
    }
    fn stack(t: f64, f0: f64) -> f64 {
        (1..=8).map(|k| (0.1 / k as f64) * (2.0 * PI * k as f64 * f0 * t).sin()).sum()
    }
    fn synth() -> Vec<f32> {
        (0..5 * SR)
            .map(|i| {
                let t = i as f64 / SR as f64;
                let voice = if (2.0..3.0).contains(&t) { stack(t, 140.0) } else { 0.0 };
                (bird(t) + voice) as f32
            })
            .collect()
    }

    /// Golden values from tools/voice-gate/gen_vectors.py (same as VoiceGateTest.kt).
    #[test]
    fn matches_python_reference() {
        let r = analyze(&synth());
        let expected: Vec<bool> = (0..50).map(|i| (17..=32).contains(&i)).collect();
        assert_eq!(r.mask, expected);
        assert!(!r.drop);
        assert_eq!(r.voiced.iter().filter(|v| **v).count(), 99);
        let hops = [0usize, 100, 210, 250, 290, 400, 496];
        let peak = [1.040886778164853, 1.0408867781649183, 0.9228970135198005, 0.922868344385425, 0.922864085565105, 1.0408867781629487, 0.0];
        let edb = [-89.79277637580643, -89.79277637580506, -25.712545642441263, -25.711693068017382, -25.71218559823333, -89.79277637542879, -200.0];
        for (k, h) in hops.iter().enumerate() {
            assert!((peak[k] - r.peak[*h]).abs() < 1e-6, "peak at hop {h}: {}", r.peak[*h]);
            assert!((edb[k] - r.energy_db[*h]).abs() < 1e-6, "energy at hop {h}: {}", r.energy_db[*h]);
        }
    }

    #[test]
    fn bird_only_is_kept() {
        let f: Vec<f32> = (0..5 * SR).map(|i| bird(i as f64 / SR as f64) as f32).collect();
        let r = analyze(&f);
        assert_eq!(r.muted_blocks(), 0);
        assert!(!r.has_voice());
    }

    #[test]
    fn continuous_voice_drops_the_frame() {
        let f: Vec<f32> = (0..5 * SR).map(|i| stack(i as f64 / SR as f64, 130.0) as f32).collect();
        assert!(analyze(&f).drop);
    }
}
