//! Live status shared by the pipeline, the setup page and the tray.

use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, OnceLock};

#[derive(Serialize, Clone, Default, Debug)]
pub struct StationLive {
    pub running: bool,
    pub reachable: Option<bool>,
    pub last_error: Option<String>,
    pub frames: u64,
    /// Frames dropped because a voice was heard (only this count is kept).
    pub voices_removed: u64,
    pub birds: u64,
    pub last_frame_ms: u64,
    pub busy_dropped: u64,
    /// Recent sound level per 100 ms block, dBFS rounded (muted speech = None), for the lane.
    pub levels: VecDeque<Option<i16>>,
    /// Last bird on this source: (Latin name, ms).
    pub last_bird: Option<(String, u64)>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Detection {
    pub t: u64,
    pub station: String,
    pub name: String,
    pub conf: u32,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct Live {
    pub stations: BTreeMap<String, StationLive>,
    pub recent: VecDeque<Detection>,
    pub reports_made: u64,
    pub last_report_ms: Option<u64>,
    pub last_report_bytes: Option<usize>,
    /// Bird map (indexer) upload state.
    pub map_state: Option<String>,
    /// Chain state shown to the user, e.g. "waiting for your listener to be registered".
    pub chain_state: Option<String>,
    pub chain_queued: usize,
    pub chain_accepted: u64,
    pub update_state: Option<String>,
    pub model_error: Option<String>,
    pub voices_day: (u64, u64),
    pub since_ms: u64,
}

pub fn live() -> &'static Mutex<Live> {
    static L: OnceLock<Mutex<Live>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(Live::default()))
}

pub fn station(id: &str, f: impl FnOnce(&mut StationLive)) {
    let mut l = live().lock().unwrap();
    f(l.stations.entry(id.to_string()).or_default());
}

pub fn detection(d: Detection) {
    lifelist_add(&d.name, d.t);
    let mut l = live().lock().unwrap();
    if let Some(s) = l.stations.get_mut(&d.station) {
        s.last_bird = Some((d.name.clone(), d.t));
    }
    l.recent.push_front(d);
    l.recent.truncate(100);
}

pub const LEVELS: usize = 150; // 15 s

pub fn levels(id: &str, blocks: &[Option<f64>]) {
    station(id, |s| {
        for b in blocks {
            s.levels.push_back(b.map(|v| v.round().clamp(-120.0, 0.0) as i16));
        }
        while s.levels.len() > LEVELS {
            s.levels.pop_front();
        }
    });
}

/// Voices removed today (local day of this computer's clock, UTC fallback): (day, count).
pub fn voice_removed() {
    let day = crate::config::now_ms() / 86_400_000;
    let mut l = live().lock().unwrap();
    if l.voices_day.0 != day {
        l.voices_day = (day, 0);
    }
    l.voices_day.1 += 1;
}

// ── species heard here, with first-heard time (names and times only, no audio) ──
fn lifelist_path() -> std::path::PathBuf {
    crate::config::state_dir().join("lifelist.json")
}

pub fn lifelist() -> &'static Mutex<BTreeMap<String, u64>> {
    static L: OnceLock<Mutex<BTreeMap<String, u64>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(std::fs::read_to_string(lifelist_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()))
}

fn lifelist_add(name: &str, t: u64) {
    let mut m = lifelist().lock().unwrap();
    if !m.contains_key(name) {
        m.insert(name.to_string(), t);
        let _ = std::fs::write(lifelist_path(), serde_json::to_vec(&*m).unwrap_or_default());
    }
}

pub fn set(f: impl FnOnce(&mut Live)) {
    f(&mut live().lock().unwrap());
}

pub fn snapshot() -> Live {
    live().lock().unwrap().clone()
}

/// One-line status for the tray tooltip.
pub fn summary(listening: bool) -> String {
    let l = snapshot();
    if let Some(e) = &l.model_error {
        return format!("Problem: {e}");
    }
    if !listening {
        return "Paused".into();
    }
    let birds: u64 = l.stations.values().map(|s| s.birds).sum();
    let voices: u64 = l.stations.values().map(|s| s.voices_removed).sum();
    let up = l.stations.values().any(|s| s.running && s.reachable == Some(true));
    let head = if up { "Listening" } else { "Can't reach the camera. Trying again soon." };
    let mut s = format!("{head} · {birds} birds heard · {voices} voices deleted");
    if let Some(c) = &l.chain_state {
        s.push_str(&format!(" · {c}"));
    }
    s
}
