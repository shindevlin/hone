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
    let mut l = live().lock().unwrap();
    l.recent.push_front(d);
    l.recent.truncate(100);
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
