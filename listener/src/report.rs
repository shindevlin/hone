//! The fixed-schema 15-minute station report (the Android station report, station/Stations.kt
//! closeWindow, plus blocks_muted_voice from the voice-gate build), signed with the station key
//! in the canonical envelope. At most 4 KB signed; species are trimmed (weakest first) to fit.
//! Integers only, so the canonical bytes are the same in every language: noise levels are
//! whole dBFS.

use crate::canonical;
use crate::perch::MODEL_ID;
use crate::pipeline::Win;
use serde_json::{json, Value};

pub const MAX_BYTES: usize = 4096;

#[derive(Clone, Debug)]
pub struct Report {
    pub station: String,
    pub tier: String,
    pub window: u64,
    /// The report fields (the signed payload is canonical::encode(&payload)).
    pub payload: Value,
    /// {"payload","key":"seat","pub","sig"} as canonical JSON, <= MAX_BYTES.
    pub signed: String,
}

fn pct(sorted: &[f64], q: f64) -> Value {
    if sorted.is_empty() {
        return Value::Null;
    }
    json!(sorted[((1.0 - q) * (sorted.len() - 1) as f64) as usize].round() as i64)
}

pub fn payload(station: &str, tier: &str, w: &Win) -> Value {
    let mut sorted = w.block_db.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut sp: Vec<(&String, &(f32, u32))> = w.species.iter().collect();
    sp.sort_by(|a, b| b.1 .0.partial_cmp(&a.1 .0).unwrap());
    json!({
        "v": 1, "source": "station", "station": station, "tier": tier, "window": w.index, "model": MODEL_ID,
        "listen_seconds": w.frames * 5, "frames": w.frames, "frames_discarded_human": w.human,
        "blocks_muted_voice": w.voice_blocks,
        "species": sp.iter().map(|(n, (c, f))| json!({"name": n, "conf_q8": (c * 255.0) as i64, "frames": f})).collect::<Vec<_>>(),
        "noise": {"unit": "dBFS", "l90": pct(&sorted, 0.90), "l50": pct(&sorted, 0.50)},
    })
}

pub fn sign(mut p: Value) -> Result<(Value, String), String> {
    let key = crate::secrets::station_key()?;
    loop {
        let env = canonical::envelope(&canonical::encode(&p), "seat", &key);
        let s = canonical::encode(&env);
        if s.len() <= MAX_BYTES {
            return Ok((p, s));
        }
        let arr = p["species"].as_array_mut().ok_or("no species list")?;
        if arr.pop().is_none() {
            return Err("report too large".into());
        }
    }
}

pub fn build(station: &str, tier: &str, w: &Win) -> Result<Report, String> {
    let (payload, signed) = sign(payload(station, tier, w))?;
    Ok(Report { station: station.into(), tier: tier.into(), window: w.index, payload, signed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_is_capped_and_schema_complete() {
        std::env::set_var("HONE_LISTENER_STATE", std::env::temp_dir().join("hl-test-shared"));
        std::env::set_var("HONE_LISTENER_FILE_SECRETS", "1");
        let mut w = Win { index: 1993200, frames: 180, human: 7, voice_blocks: 120, ..Default::default() };
        for i in 0..400 {
            w.species.insert(format!("Genusname speciesname{i:04}"), (0.5 + (i as f32) / 1000.0, 2));
        }
        w.block_db = vec![-60.0, -50.0, -40.0];
        let r = build("0123456789abcdef", "C", &w).unwrap();
        assert!(r.signed.len() <= MAX_BYTES, "{}", r.signed.len());
        let p = &r.payload;
        for k in ["v", "source", "station", "tier", "window", "model", "listen_seconds", "frames", "frames_discarded_human", "species", "noise"] {
            assert!(p.get(k).is_some(), "{k}");
        }
        assert_eq!(p["frames_discarded_human"], 7);
        assert_eq!(p["noise"]["l90"], -60);
        // strongest kept first
        assert_eq!(p["species"][0]["name"], "Genusname speciesname0399");
        let env: Value = serde_json::from_str(&r.signed).unwrap();
        assert_eq!(env["key"], "seat");
    }
}
