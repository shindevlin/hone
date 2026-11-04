//! Enrolment probe (port of station/StationSetup.kt): find the camera's stream (ONVIF
//! GetStreamUri, then common paths incl. ESEE's), open ONLY its audio track and listen a few
//! seconds to rate it: codec, sample rate, bit rate, noise floor -> tier A/B/C.

use crate::audio;
use crate::discovery::{is_local_host, FoundCamera, Onvif, OnvifStream};
use crate::rtsp::{self, RtspAudioClient, RtspError};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize, Clone, Debug)]
pub struct ProbeResult {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub codec: String,
    pub rate: u32,
    pub tier: String,
    pub kbps: u32,
    pub noise_db: Option<f64>,
    pub seconds: u32,
    pub id_source: String,
    #[serde(skip)]
    pub id_value: String,
    pub model: Option<String>,
    pub video_tracks_skipped: usize,
}

pub fn candidates(cam: &FoundCamera, s: Option<&OnvifStream>) -> Vec<(u16, String)> {
    let mut out: Vec<(u16, String)> = Vec::new();
    if let Some(s) = s {
        out.push((s.port, s.path.clone()));
    }
    let mut ports: Vec<u16> = [cam.rtsp_port, s.map(|x| x.port), Some(554), Some(8554)].into_iter().flatten().collect();
    ports.dedup();
    for p in ports {
        for path in rtsp::FALLBACK_PATHS {
            if !out.contains(&(p, path.to_string())) {
                out.push((p, path.to_string()));
            }
        }
    }
    out
}

fn arp_mac(host: &str) -> Option<String> {
    let t = std::fs::read_to_string("/proc/net/arp").ok()?;
    t.lines().skip(1).map(|l| l.split_whitespace().collect::<Vec<_>>()).find(|c| c.len() >= 4 && c[0] == host && c[3] != "00:00:00:00:00:00").map(|c| c[3].to_lowercase())
}

pub fn connect(cam: &FoundCamera, user: &str, pass: &str, seconds: f64) -> Result<ProbeResult, RtspError> {
    if !is_local_host(&cam.host) {
        return Err(rtsp::err(403, "only cameras on your own network can be added"));
    }
    let mut onvif_stream = None;
    let mut hw = None;
    let mut model = cam.name.clone();
    if let Some(x) = &cam.onvif {
        if let Some(o) = Onvif::new(x, user, pass) {
            match o.stream_uri() {
                Ok(s) => onvif_stream = s.filter(|s| s.host == cam.host || is_local_host(&s.host)),
                Err(true) => return Err(rtsp::err(401, "wrong username or password")),
                Err(false) => {}
            }
            hw = o.hardware_id();
            if let Some(m) = o.model() {
                model = Some(m);
            }
        }
    }
    let mut no_audio: Option<RtspError> = None;
    let mut reached = false;
    let host = onvif_stream.as_ref().map(|s| s.host.clone()).unwrap_or(cam.host.clone());
    for (port, path) in candidates(cam, onvif_stream.as_ref()) {
        let mut c = RtspAudioClient::new(&host, port, &path, (!user.is_empty()).then_some(user), Some(pass), 5_000);
        let r = (|| {
            let m = c.open()?;
            if !["PCMA", "PCMU", "L16", "MPEG4-GENERIC"].contains(&m.encoding.as_str()) {
                return Err(rtsp::err(415, format!("audio codec not supported: {}", m.encoding)));
            }
            let mut samples: Vec<f32> = Vec::new();
            let t0 = std::time::Instant::now();
            let pcm = m.encoding != "MPEG4-GENERIC";
            c.stream_for(seconds, |p| {
                if pcm {
                    samples.extend(audio::decode(&m.encoding, p, m.channels))
                } else {
                    samples.push(0.0)
                }
            })?;
            let secs = t0.elapsed().as_secs_f64().max(1.0);
            if samples.is_empty() {
                return Err(rtsp::err(415, "no sound arrived from the camera"));
            }
            let rate = rtsp::rate_of(&m);
            let id = hw.clone().or_else(|| arp_mac(&cam.host).map(|m| ("mac".to_string(), m))).unwrap_or(("host".into(), cam.host.clone()));
            Ok(ProbeResult {
                host: host.clone(), port, path: path.clone(), codec: m.encoding.clone(), rate, tier: audio::tier(rate).into(),
                kbps: (c.bytes_in as f64 * 8.0 / secs / 1000.0) as u32,
                noise_db: if pcm { audio::noise_floor_db(&samples, rate) } else { None },
                seconds: secs as u32, id_source: id.0, id_value: id.1, model: model.clone(), video_tracks_skipped: c.video_tracks_skipped,
            })
        })();
        c.close();
        match r {
            Ok(p) => return Ok(p),
            Err(e) if e.code == 401 => return Err(e),
            Err(e) if e.code == 415 => {
                no_audio = Some(e);
                reached = true;
            }
            Err(e) if e.code != 0 => reached = true,
            Err(_) => {}
        }
    }
    Err(no_audio.unwrap_or_else(|| rtsp::err(if reached { 404 } else { 0 }, "camera stream not found")))
}

/// Stable station id: hash of the camera's hardware id and this listener's station key.
/// Never the raw MAC. Same formula as the Android app.
pub fn station_id(id_source: &str, id_value: &str, station_pub_hex: &str) -> String {
    hex::encode(Sha256::digest(format!("hone-station|{id_source}|{}|{station_pub_hex}", id_value.to_lowercase()).as_bytes()))[..16].to_string()
}

/// Copy shown after the probe (Android strings_onboarding.xml fr_tier_*).
pub fn tier_copy(t: &str) -> &'static str {
    match t {
        "A" => "Your camera hears everything from tiny songbirds to owls.",
        "B" => "Your camera hears most birds well.",
        _ => "Your camera hears low sounds well, like owls, doves, frogs and nightjars. Perfect for night listening.",
    }
}

pub fn friendly(e: &RtspError) -> &'static str {
    match e.code {
        401 => "That username or password didn't work. Try again.",
        403 => "Only cameras on your own Wi-Fi can be added.",
        415 => "This camera doesn't send sound. Check that its microphone is turned on in your camera app, then try again.",
        _ => "We couldn't connect to this camera. Make sure it is on and on the same Wi-Fi.",
    }
}
