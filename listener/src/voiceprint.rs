//! Individual birds by voice (crate `hone-voiceprint`, branch `voiceprint`).
//! Every Perch detection's embedding goes to one Voiceprinter, which clusters calls per species
//! and place. State (compact non-invertible voiceprints, no audio) is saved only in this
//! listener's data dir, owner-only, every 10 minutes and on shutdown.
//! Shown to the user ("is back", candidates) only for species whose naming gate is open
//! (eval/gates.json: today only Athene noctua). All other species are clustered silently
//! (shadow mode) for later evaluation. BIRD_OBSERVE is not sent or queued until the chain's
//! BIRD_CARDS feature is active.

use crate::config::{now_ms, state_dir};
use hone_voiceprint::{phrase, Candidate, Config as VpConfig, Event, Voiceprinter};
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};

/// The listener's detection hook (Perch embeddings go here and nowhere else).
pub trait VoiceprintSink: Send {
    fn on_detection(&mut self, species: &str, embedding: &[f32], t: u64, conf: f32);
}

/// Chain BIRD_CARDS (BIRD_OBSERVE/BIRD_NAME entries) is not active yet: nothing is sent or queued.
/// When it activates, every candidate with naming_allowed is eligible for BIRD_OBSERVE (the
/// crate's one-time Nameable events from before activation are not needed for that).
pub const BIRD_CARDS_ACTIVE: bool = false;

const FILE: &str = "voiceprints.bin";
const SAVE_EVERY_MS: u64 = 10 * 60_000;

pub struct Store {
    vp: Voiceprinter,
    dirty: bool,
    last_save: u64,
    /// "is back" lines for the user (gated species only), newest first.
    back: Vec<Value>,
}

fn path() -> std::path::PathBuf {
    state_dir().join(FILE)
}

impl Store {
    fn open() -> Self {
        let vp = std::fs::read(path()).ok().and_then(|b| Voiceprinter::load(VpConfig::default(), &b).ok())
            .unwrap_or_else(|| Voiceprinter::new(VpConfig::default()));
        Store { vp, dirty: false, last_save: now_ms(), back: Vec::new() }
    }

    fn take_events(&mut self) {
        for e in self.vp.drain_events() {
            match e {
                Event::Back { bird_id, name, nights_seen, likelihood } => {
                    if let Some(c) = self.vp.status(&bird_id).filter(|c| c.naming_allowed) {
                        self.back.insert(0, json!({"t": now_ms(), "bird_id": bird_id, "species": c.species, "name": name,
                            "nights_seen": nights_seen, "likelihood_pct": (likelihood * 100.0).round() as u32, "phrase": phrase(likelihood)}));
                        self.back.truncate(30);
                    }
                }
                Event::Nameable { .. } if BIRD_CARDS_ACTIVE => { /* BIRD_OBSERVE goes here once the chain has it */ }
                _ => {} // Candidate, Split, Merged, pre-activation Nameable: state only (shadow)
            }
        }
    }

    pub fn save(&mut self) -> Result<(), String> {
        write_private(&path(), &self.vp.save())?;
        self.dirty = false;
        self.last_save = now_ms();
        Ok(())
    }

    /// Candidates the user may see (naming gate open).
    pub fn visible(&self) -> Vec<Candidate> {
        self.vp.candidates().into_iter().filter(|c| c.naming_allowed).collect()
    }
}

impl VoiceprintSink for Store {
    fn on_detection(&mut self, species: &str, embedding: &[f32], t: u64, conf: f32) {
        hone_voiceprint::VoiceprintSink::on_detection(&mut self.vp, species, embedding, t, conf);
        self.dirty = true;
        self.take_events();
        if now_ms() - self.last_save > SAVE_EVERY_MS {
            if let Err(e) = self.save() {
                log::warn!("voiceprints: {e}");
            }
        }
    }
}

pub fn write_private(p: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.sync_all().ok();
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?; // %LOCALAPPDATA% is per-user already
    std::fs::rename(tmp, p).map_err(|e| e.to_string())
}

fn global() -> &'static Mutex<Store> {
    static S: OnceLock<Mutex<Store>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Store::open()))
}

/// The sink the analyzer holds: forwards to the shared store.
pub struct Shared;

impl VoiceprintSink for Shared {
    fn on_detection(&mut self, species: &str, embedding: &[f32], t: u64, conf: f32) {
        global().lock().unwrap().on_detection(species, embedding, t, conf);
    }
}

/// Place and time zone from the camera's declared location (cell) and the owner's clock.
pub fn configure(cell: Option<&str>, tz_min: i32) {
    let mut s = global().lock().unwrap();
    if let Some(c) = cell {
        if let Err(e) = s.vp.set_cell(c) {
            log::warn!("voiceprints: location: {e:?}");
        }
    }
    s.vp.set_tz_min(tz_min);
}

pub fn save_now() {
    let mut s = global().lock().unwrap();
    if s.dirty {
        if let Err(e) = s.save() {
            log::warn!("voiceprints: {e}");
        }
    }
}

pub fn name(bird_id: &str, name: &str) -> Result<(), String> {
    let mut s = global().lock().unwrap();
    s.vp.name(bird_id, name).map_err(|e| match e {
        hone_voiceprint::VpError::NamingNotAllowed(_) => "This bird can't be named yet.".to_string(),
        _ => "Use 1 to 24 letters for the name.".to_string(),
    })?;
    s.dirty = true;
    s.save()
}

/// For the page: visible individuals and recent "is back" lines (gated species only).
pub fn view(common: &dyn Fn(&str) -> Option<String>) -> Value {
    let s = global().lock().unwrap();
    let birds: Vec<Value> = s.visible().iter().map(|c| json!({
        "bird_id": c.bird_id, "species": c.species, "common": common(&c.species), "name": c.name,
        "nights_seen": c.nights_seen, "heard_tonight": c.heard_tonight, "last_heard": c.last_heard,
        "is_back": s.vp.is_back(&c.bird_id),
        "back_pct": c.back_likelihood.map(|p| (p * 100.0).round() as u32), "back_phrase": c.back_likelihood.map(phrase),
    })).collect();
    let back: Vec<Value> = s.back.iter().map(|b| {
        let mut b = b.clone();
        b["common"] = json!(common(b["species"].as_str().unwrap_or("")));
        b
    }).collect();
    json!({"birds": birds, "back": back})
}

/// Rough offset from longitude, used until the owner's browser reports the real one.
pub fn tz_from_lon(lon: f64) -> i32 {
    ((lon / 15.0).round() as i32 * 60).clamp(-720, 840)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) >> 11) as f64 / 9_007_199_254_740_992.0
        }
        fn gauss(&mut self) -> f32 {
            let (a, b) = (self.next().max(1e-12), self.next());
            ((-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()) as f32
        }
        fn vec(&mut self, s: f32) -> Vec<f32> {
            (0..1536).map(|_| self.gauss() * s).collect()
        }
    }

    /// Two birds of each species calling 12 times a night (alternating) for 8 nights at one place.
    fn feed(st: &mut Store) {
        let mut r = Rng(42);
        const T0: u64 = 1_790_000_000_000;
        let owl_sp = r.vec(1.0);
        let owls = [r.vec(0.6), r.vec(0.6)];
        let kisk_sp = r.vec(1.0);
        let kisks = [r.vec(0.6), r.vec(0.6)];
        for n in 0..8u64 {
            for k in 0..12u64 {
                let t = T0 + n * 86_400_000 + k * 60_000;
                let b = (k % 2) as usize;
                for (sp, s, bird) in [("Athene noctua", &owl_sp, &owls[b]), ("Pitangus sulphuratus", &kisk_sp, &kisks[b])] {
                    let e: Vec<f32> = s.iter().zip(bird).map(|(x, y)| x + y + r.gauss() * 0.35).collect();
                    st.on_detection(sp, &e, t, 0.9);
                }
            }
        }
    }

    fn fresh() -> Store {
        let mut vp = Voiceprinter::new(VpConfig::default());
        vp.set_cell("8966c2d6b6bffff").unwrap();
        Store { vp, dirty: false, last_save: now_ms(), back: Vec::new() }
    }

    #[test]
    fn only_gated_species_are_shown_others_are_shadow() {
        let mut st = fresh();
        feed(&mut st);
        let all = st.vp.candidates();
        assert!(all.iter().any(|c| c.species == "Athene noctua"), "{all:?}");
        assert!(all.iter().any(|c| c.species == "Pitangus sulphuratus"), "shadow clustering still runs: {all:?}");
        let vis = st.visible();
        assert!(!vis.is_empty() && vis.iter().all(|c| c.species == "Athene noctua"), "{vis:?}");
        assert!(st.back.iter().all(|b| b["species"] == "Athene noctua"), "{:?}", st.back);
        assert!(!BIRD_CARDS_ACTIVE);
    }

    #[test]
    fn saves_owner_only_and_reloads() {
        let mut st = fresh();
        feed(&mut st);
        let dir = std::env::temp_dir().join(format!("hl-vp-{}", std::process::id()));
        let p = dir.join(FILE);
        write_private(&p, &st.vp.save()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let back = Voiceprinter::load(VpConfig::default(), &std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(back.candidates(), st.vp.candidates());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn naming_gate_is_enforced() {
        let mut st = fresh();
        feed(&mut st);
        let kisk = st.vp.candidates().into_iter().find(|c| c.species == "Pitangus sulphuratus").unwrap();
        assert!(st.vp.name(&kisk.bird_id, "Ruben").is_err());
        let owl = st.visible().into_iter().next().unwrap();
        assert!(st.vp.name(&owl.bird_id, "Hoot").is_ok());
    }

    #[test]
    fn tz_guess() {
        assert_eq!(tz_from_lon(-89.24), -360);
        assert_eq!(tz_from_lon(0.0), 0);
    }
}
