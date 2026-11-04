//! G.711 decoders, the streaming linear resampler and the station quality tier.
//! Ported from the Android app (station/Audio.kt); behaviour kept identical.

pub fn alaw(b: u8) -> i32 {
    let a = (b as i32) ^ 0x55;
    let mut t = (a & 0x0F) << 4;
    let seg = (a & 0x70) >> 4;
    t = match seg {
        0 => t + 8,
        1 => t + 0x108,
        _ => (t + 0x108) << (seg - 1),
    };
    if a & 0x80 != 0 { t } else { -t }
}

pub fn ulaw(b: u8) -> i32 {
    let u = (b as i32) ^ 0xFF;
    let mut t = ((u & 0x0F) << 3) + 0x84;
    t <<= (u & 0x70) >> 4;
    if u & 0x80 != 0 { 0x84 - t } else { t - 0x84 }
}

/// RTP payload -> mono floats in [-1, 1].
pub fn decode(encoding: &str, payload: &[u8], channels: usize) -> Vec<f32> {
    match encoding {
        "PCMA" => payload.iter().map(|&b| alaw(b) as f32 / 32768.0).collect(),
        "PCMU" => payload.iter().map(|&b| ulaw(b) as f32 / 32768.0).collect(),
        "L16" => {
            let ch = channels.max(1);
            let n = payload.len() / 2 / ch;
            (0..n)
                .map(|i| {
                    let mut s = 0f32;
                    for c in 0..ch {
                        let k = (i * ch + c) * 2;
                        s += i16::from_be_bytes([payload[k], payload[k + 1]]) as f32 / 32768.0;
                    }
                    s / ch as f32
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Streaming linear resampler (continuous across calls), as LinearResampler in Audio.kt.
pub struct LinearResampler {
    in_rate: u32,
    out_rate: u32,
    step: f64,
    t: f64,
    prev: f32,
}

impl LinearResampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        Self { in_rate, out_rate, step: in_rate as f64 / out_rate as f64, t: 0.0, prev: 0.0 }
    }

    pub fn process(&mut self, x: &[f32]) -> Vec<f32> {
        if self.in_rate == self.out_rate || x.is_empty() {
            return x.to_vec();
        }
        let cap = (((x.len() as f64 - self.t) / self.step).max(0.0) as usize) + 2;
        let mut out = Vec::with_capacity(cap);
        while self.t < x.len() as f64 && out.len() < cap {
            let i = self.t as usize;
            let f = (self.t - i as f64) as f32;
            let a = if i == 0 { self.prev } else { x[i - 1] };
            out.push(a + (x[i] - a) * f);
            self.t += self.step;
        }
        self.t -= x.len() as f64;
        self.prev = *x.last().unwrap();
        out
    }
}

/// A: >= 32 kHz wideband; B: 16-24 kHz; C: 8 kHz (G.711), low-band species only.
pub fn tier(rate: u32) -> &'static str {
    if rate >= 32_000 {
        "A"
    } else if rate >= 16_000 {
        "B"
    } else {
        "C"
    }
}

/// 10th-percentile level of 100 ms blocks, dBFS (the quiet background).
pub fn noise_floor_db(x: &[f32], rate: u32) -> Option<f64> {
    let block = (rate as usize / 10).max(1);
    let mut levels: Vec<f64> = (0..x.len() / block)
        .map(|b| {
            let e: f64 = x[b * block..(b + 1) * block].iter().map(|&v| (v as f64) * (v as f64)).sum();
            10.0 * (e / block as f64 + 1e-12).log10()
        })
        .collect();
    if levels.is_empty() {
        return None;
    }
    levels.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Some((levels[(levels.len() as f64 * 0.1) as usize] * 10.0).round() / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g711_known_values() {
        assert_eq!(alaw(0xD5), 8);
        assert_eq!(alaw(0x55), -8);
        assert_eq!(ulaw(0xFF), 0);
        assert_eq!(ulaw(0x00), -32124);
        assert_eq!(ulaw(0x80), 32124);
    }

    #[test]
    fn resampler_rate() {
        let mut r = LinearResampler::new(8000, 32000);
        let mut n = 0;
        for _ in 0..10 {
            n += r.process(&vec![0.1f32; 800]).len();
        }
        assert!((n as i64 - 32000).abs() <= 4, "{n}");
        assert_eq!(tier(8000), "C");
        assert_eq!(tier(16000), "B");
        assert_eq!(tier(48000), "A");
    }
}
