use super::*;
use crate::gate::{GateTable, SpeciesParams};
use crate::projection::PERCH_DIM;

const CELL9: &str = "8966c2d6b6bffff"; // somewhere in El Salvador, res 9
const KISK: &str = "Pitangus sulphuratus";
const DAY: u64 = 86_400_000;
const T0: u64 = 1_790_000_000_000;

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
        (0..PERCH_DIM).map(|_| self.gauss() * s).collect()
    }
}

/// Perch-like: a big shared species direction, a smaller per-bird offset, per-call noise.
fn call(species: &[f32], bird: &[f32], r: &mut Rng) -> Vec<f32> {
    species.iter().zip(bird).map(|(s, b)| s + b + r.gauss() * 0.35).collect()
}

fn cfg(allowed: bool) -> Config {
    let mut g = GateTable::default();
    g.species.insert(
        KISK.into(),
        SpeciesParams {
            naming_allowed: allowed,
            join_tau: 0.45,
            back_tau: 0.6,
            k_min: 3,
            merge_tau: 0.85,
            split_tau: 0.2,
            logit_a: 12.0,
            logit_b: 0.45,
            source: "test".into(),
        },
    );
    Config { gates: g, ..Config::default() }
}

fn run(nights: u64, allowed: bool) -> (Voiceprinter, Vec<Event>) {
    let mut r = Rng(42);
    let sp = r.vec(1.0);
    let birds = [r.vec(0.6), r.vec(0.6)];
    let mut v = Voiceprinter::new(cfg(allowed));
    v.set_cell(CELL9).unwrap();
    let mut ev = Vec::new();
    for n in 0..nights {
        for k in 0..12u64 {
            let b = (k % 2) as usize;
            let t = T0 + n * DAY + k * 60_000;
            v.on_detection(KISK, &call(&sp, &birds[b], &mut r), t, 0.9);
        }
        ev.extend(v.drain_events());
    }
    (v, ev)
}

#[test]
fn two_birds_become_two_candidates_and_come_back() {
    let (v, ev) = run(6, true);
    let c = v.candidates();
    assert_eq!(c.len(), 2, "{c:?}");
    for b in &c {
        assert!(b.nights_seen >= 5, "{b:?}");
        assert!(v.is_back(&b.bird_id));
        let p = b.back_likelihood.unwrap();
        assert!(p > 0.5 && p <= MAX_LIKELIHOOD, "{p}");
        assert_eq!(b.cell7, cell7_of(CELL9).unwrap());
    }
    assert!(ev.iter().any(|e| matches!(e, Event::Back { .. })));
    assert!(ev.iter().filter(|e| matches!(e, Event::Candidate { .. })).count() == 2);
    assert_eq!(ev.iter().filter(|e| matches!(e, Event::Nameable { .. })).count(), 2);
}

#[test]
fn deterministic_and_stable_ids() {
    let (a, _) = run(5, true);
    let (b, _) = run(5, true);
    assert_eq!(a.save(), b.save());
    let ids: Vec<_> = a.candidates().into_iter().map(|c| c.bird_id).collect();
    let re = Voiceprinter::load(cfg(true), &a.save()).unwrap();
    let ids2: Vec<_> = re.candidates().into_iter().map(|c| c.bird_id).collect();
    assert_eq!(ids, ids2);
    assert!(ids.iter().all(|i| i.len() == 64 && i.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())));
}

#[test]
fn naming_gate() {
    let (mut v, ev) = run(5, false);
    assert!(!ev.iter().any(|e| matches!(e, Event::Nameable { .. })));
    let id = v.candidates()[0].bird_id.clone();
    assert_eq!(v.name(&id, "Ruben"), Err(VpError::NamingNotAllowed(KISK.into())));
    let (mut v, _) = run(5, true);
    let id = v.candidates()[0].bird_id.clone();
    assert_eq!(v.name(&id, ""), Err(VpError::BadName));
    assert_eq!(v.name(&id, "Ruben2"), Err(VpError::BadName));
    assert_eq!(v.name(&id, "abcdefghijklmnopqrstuvwxy"), Err(VpError::BadName));
    v.name(&id, "Ruben").unwrap();
    assert_eq!(v.status(&id).unwrap().name.as_deref(), Some("Ruben"));
    assert_eq!(v.name("nope", "x"), Err(VpError::UnknownBird));
}

#[test]
fn below_floor_and_no_location_are_ignored() {
    let mut r = Rng(1);
    let e = r.vec(1.0);
    let mut v = Voiceprinter::new(cfg(true));
    assert!(v.observe(KISK, &e, T0, 0.99).is_none()); // no cell yet
    v.set_cell(CELL9).unwrap();
    assert!(v.observe(KISK, &e, T0, 0.1).is_none()); // below floor
    assert!(v.observe(KISK, &e[..10], T0, 0.99).is_none()); // wrong size
}

#[test]
fn merge_earliest_name_wins() {
    let (mut v, _) = run(5, true);
    let c = v.candidates();
    let (a, b) = (c[0].bird_id.clone(), c[1].bird_id.clone());
    v.name(&b, "Ruben").unwrap();
    v.name(&a, "Later").unwrap();
    // make b named earlier, then force a merge by lowering merge_tau
    for p in v.st.pops.values_mut() {
        for cl in p.clusters.iter_mut() {
            if cl.bird_id.as_deref() == Some(&b) {
                cl.named_at = Some(1);
            }
        }
    }
    v.cfg.gates.species.get_mut(KISK).unwrap().merge_tau = -1.0;
    v.cfg.gates.species.get_mut(KISK).unwrap().split_tau = -1.0;
    v.maintain();
    let ev = v.drain_events();
    assert!(ev.contains(&Event::Merged { from: a.clone(), into: b.clone(), dropped_name: Some("Later".into()) }), "{ev:?}");
    assert_eq!(v.resolve(&a), b);
    assert_eq!(v.status(&a).unwrap().name.as_deref(), Some("Ruben"));
    assert_eq!(v.candidates().len(), 1);
}

#[test]
fn split_two_birds_forced_together() {
    // join everything into one cluster, then let maintain() split it
    let mut c = cfg(true);
    c.gates.species.get_mut(KISK).unwrap().join_tau = -1.0;
    let mut r = Rng(9);
    let sp = r.vec(1.0);
    let birds = [r.vec(0.8), r.vec(0.8)];
    let mut v = Voiceprinter::new(c);
    v.set_cell(CELL9).unwrap();
    for n in 0..3u64 {
        for k in 0..20u64 {
            v.on_detection(KISK, &call(&sp, &birds[(k % 2) as usize], &mut r), T0 + n * DAY + k * 1000, 0.9);
        }
    }
    let before: usize = v.st.pops.values().map(|p| p.clusters.len()).sum();
    v.maintain();
    let after: usize = v.st.pops.values().map(|p| p.clusters.len()).sum();
    assert!(before <= 2);
    assert_eq!(after, 2);
}

#[test]
fn report_fits_and_holds_no_prints() {
    let (v, _) = run(6, true);
    let r = v.report(4096);
    assert!(r.len() <= 4096);
    assert!(!r.contains("\"q\"") && !r.contains("scale") && !r.contains("cell"));
    let j: serde_json::Value = serde_json::from_str(&r).unwrap();
    assert_eq!(j["birds"].as_array().unwrap().len(), 2);
    let small = v.report(60);
    assert!(small.len() <= 60 || serde_json::from_str::<serde_json::Value>(&small).unwrap()["birds"].as_array().unwrap().is_empty());
}

#[test]
fn shipped_gate_table_parses() {
    let g = GateTable::shipped();
    assert!(!g.naming_allowed("Not a bird"));
}

#[test]
fn night_boundary_is_local_noon() {
    // 2026-11-04 11:59 UTC and 12:01 UTC with tz 0 fall in different nights
    let a = 1_793_793_540_000u64; // 11:59
    let b = a + 2 * 60_000;
    assert_eq!(night_of(b, 0), night_of(a, 0) + 1);
    // same instant, a night 6 h west (tz -360) is still the earlier night
    assert_eq!(night_of(b, -360), night_of(a, 0));
}

#[test]
fn user_link_keeps_earliest_name() {
    let (mut v, _) = run(5, true);
    let c = v.candidates();
    let (a, b) = (c[0].bird_id.clone(), c[1].bird_id.clone());
    v.name(&a, "Ruben").unwrap();
    v.drain_events();
    let into = v.link(&b, &a).unwrap();
    assert_eq!(into, a);
    assert_eq!(v.resolve(&b), a);
    assert_eq!(v.candidates().len(), 1);
    assert!(matches!(v.drain_events()[0], Event::Merged { .. }));
    assert_eq!(v.link(&a, "nope"), Err(VpError::UnknownBird));
}
