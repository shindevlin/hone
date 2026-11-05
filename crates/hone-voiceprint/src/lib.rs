//! # hone-voiceprint
//!
//! Recognises individual birds by voice so a user can name one ("Ruben", a Great Kiskadee) and
//! later see "Ruben is back, night 14".
//!
//! Per detection (after the human-speech gate and above a confidence floor) the Perch v2
//! embedding is folded into a 64-value int8 voiceprint ([`projection`]). Audio never reaches this
//! crate and nothing here can rebuild it. Voiceprints are clustered online per
//! (species, H3 res-7 cell). A cluster becomes a candidate individual only after it recurs on
//! `min_nights` nights with a stable centroid; then it gets a stable `bird_id`. Every match carries
//! a calibrated likelihood, capped below certainty. Splits and merges are deterministic; on a
//! merge the earliest name wins.
//!
//! Raw voiceprints stay on the device ([`Voiceprinter::save`]). Only bird ids and summary stats
//! leave it ([`Voiceprinter::report`]).

#[cfg(feature = "android")]
mod android;
pub mod gate;
pub mod projection;

use gate::{GateTable, SpeciesParams};
use projection::{cosine, sub, Projection, Voiceprint, DEFAULT_SEED, VP_DIM};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// What the camera listener (and any other detector) calls for each bird detection.
/// `t` is unix time in milliseconds (values below 1e11 are taken as seconds).
pub trait VoiceprintSink {
    fn on_detection(&mut self, species: &str, embedding: &[f32], t: u64, conf: f32);
}

/// Likelihoods are never shown as certain.
pub const MAX_LIKELIHOOD: f32 = 0.99;
/// Detections a population needs before its mean is trusted for centring.
pub const WARMUP: u64 = 20;
const SPLIT_MIN: usize = 6;
const MAX_NIGHT_STATS: usize = 120;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Local time zone offset in minutes (nights run local noon to noon).
    pub tz_min: i32,
    /// Detections below this Perch confidence are ignored.
    pub conf_floor: f32,
    /// Nights (with >= 2 calls) before a cluster can become a candidate individual.
    pub min_nights: u32,
    pub max_clusters_per_place: usize,
    pub max_members_per_cluster: usize,
    /// Unpromoted clusters not heard for this many nights are dropped.
    pub forget_after_nights: u32,
    pub seed: u64,
    pub gates: GateTable,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tz_min: 0,
            conf_floor: 0.6,
            min_nights: 3,
            max_clusters_per_place: 96,
            max_members_per_cluster: 128,
            forget_after_nights: 45,
            seed: DEFAULT_SEED,
            gates: GateTable::shipped(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct NightStat {
    n: u32,
    sum: Vec<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Member {
    night: u32,
    t: u64,
    vp: Voiceprint,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cluster {
    local: u64,
    nights: BTreeMap<u32, NightStat>,
    /// Aggregates of nights dropped from `nights` (bounded memory).
    old_n: u64,
    old_sum: Vec<f32>,
    old_nights: u32,
    members: VecDeque<Member>,
    first_night: u32,
    last_heard: u64,
    detections: u64,
    bird_id: Option<String>,
    name: Option<String>,
    named_at: Option<u64>,
    back_night: Option<u32>,
    back_likelihood: Option<f32>,
    /// Event::Nameable already sent (chain BIRD_OBSERVE).
    #[serde(default)]
    nameable_sent: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Population {
    species: String,
    cell7: String,
    mean_n: u64,
    mean_sum: Vec<f64>,
    pending: Vec<Member>,
    clusters: Vec<Cluster>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct State {
    v: u32,
    cell7: Option<String>,
    night: Option<u32>,
    next_local: u64,
    pops: BTreeMap<String, Population>,
    /// Ids absorbed by merges -> surviving id.
    aliases: BTreeMap<String, String>,
    #[serde(skip)]
    events: Vec<Event>,
}

/// One detection's outcome.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Match {
    /// Candidate individual this call most likely came from, if any.
    pub bird_id: Option<String>,
    /// Likelihood that the call is that bird (0..0.99), if a candidate matched.
    pub likelihood: Option<f32>,
    pub new_cluster: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A cluster recurred on enough nights with a stable centroid: it is now a candidate.
    Candidate { bird_id: String, species: String, nights_seen: u32, naming_allowed: bool },
    /// A candidate passed the naming gate for the first time: emit chain BIRD_OBSERVE {bird_id}
    /// now, so the chain records this device's seat as the first reporter.
    Nameable { bird_id: String, species: String, nights_seen: u32 },
    /// "Ruben is back, night 14".
    Back { bird_id: String, name: Option<String>, nights_seen: u32, likelihood: f32 },
    /// `from` was the same bird as `into`; `from` now resolves to `into`.
    Merged { from: String, into: String, dropped_name: Option<String> },
    /// A cluster held two birds; the part with the later first night became a new cluster.
    Split { from: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub bird_id: String,
    pub species: String,
    pub cell7: String,
    pub nights_seen: u32,
    pub first_night: u32,
    pub last_heard: u64,
    pub detections: u64,
    pub name: Option<String>,
    pub naming_allowed: bool,
    /// Calls assigned tonight.
    pub heard_tonight: u32,
    /// Likelihood behind tonight's "is back", if it was said.
    pub back_likelihood: Option<f32>,
}

#[derive(Debug, PartialEq)]
pub enum VpError {
    UnknownBird,
    NamingNotAllowed(String),
    BadName,
    BadCell,
    BadState(String),
    /// link(): the two ids are different species or places.
    NotSamePlace,
}

impl std::fmt::Display for VpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VpError::UnknownBird => write!(f, "no candidate with that bird id"),
            VpError::NamingNotAllowed(s) => write!(f, "naming is not offered for {s}"),
            VpError::BadName => write!(f, "name must be 1-24 letters, spaces, apostrophes or hyphens"),
            VpError::BadCell => write!(f, "not an H3 cell at resolution 7 or finer"),
            VpError::BadState(e) => write!(f, "saved state unreadable: {e}"),
            VpError::NotSamePlace => write!(f, "only birds of one species at one place can be linked"),
        }
    }
}
impl std::error::Error for VpError {}

/// Same rule as the chain's BIRD_NAME: 1-24 letters, spaces, apostrophes, hyphens.
pub fn valid_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=24).contains(&n) && name.chars().all(|c| c.is_alphabetic() || c == ' ' || c == '\'' || c == '-')
}

/// Words for a likelihood. Never "certainly".
pub fn phrase(p: f32) -> &'static str {
    if p >= 0.9 {
        "very likely"
    } else if p >= 0.7 {
        "likely"
    } else if p >= 0.5 {
        "possibly"
    } else {
        "unsure"
    }
}

pub fn night_of(t_ms: u64, tz_min: i32) -> u32 {
    let m = (t_ms / 60_000) as i64 + tz_min as i64 - 720;
    m.div_euclid(1440).max(0) as u32
}

fn to_ms(t: u64) -> u64 {
    if t < 100_000_000_000 {
        t * 1000
    } else {
        t
    }
}

/// H3 cell (res >= 7, hex string) -> its res-7 parent.
pub fn cell7_of(cell: &str) -> Result<String, VpError> {
    use std::str::FromStr;
    let c = h3o::CellIndex::from_str(cell).map_err(|_| VpError::BadCell)?;
    c.parent(h3o::Resolution::Seven).map(|p| p.to_string()).ok_or(VpError::BadCell)
}

pub fn cell7_at(lat: f64, lon: f64) -> Result<String, VpError> {
    let ll = h3o::LatLng::new(lat, lon).map_err(|_| VpError::BadCell)?;
    Ok(ll.to_cell(h3o::Resolution::Seven).to_string())
}

impl Cluster {
    fn total(&self) -> (u64, [f32; VP_DIM]) {
        let mut s = [0f32; VP_DIM];
        let mut n = self.old_n;
        for i in 0..VP_DIM {
            s[i] = self.old_sum.get(i).copied().unwrap_or(0.0);
        }
        for ns in self.nights.values() {
            n += ns.n as u64;
            for i in 0..VP_DIM {
                s[i] += ns.sum[i];
            }
        }
        (n, s)
    }
    fn centroid(&self) -> [f32; VP_DIM] {
        let (n, mut s) = self.total();
        let d = n.max(1) as f32;
        for v in s.iter_mut() {
            *v /= d;
        }
        s
    }
    fn centroid_without(&self, night: u32) -> Option<[f32; VP_DIM]> {
        let (mut n, mut s) = self.total();
        if let Some(ns) = self.nights.get(&night) {
            n -= ns.n as u64;
            for i in 0..VP_DIM {
                s[i] -= ns.sum[i];
            }
        }
        if n == 0 {
            return None;
        }
        for v in s.iter_mut() {
            *v /= n as f32;
        }
        Some(s)
    }
    fn night_mean(&self, night: u32) -> Option<(u32, [f32; VP_DIM])> {
        let ns = self.nights.get(&night)?;
        let mut o = [0f32; VP_DIM];
        for i in 0..VP_DIM {
            o[i] = ns.sum[i] / ns.n.max(1) as f32;
        }
        Some((ns.n, o))
    }
    fn nights_seen(&self) -> u32 {
        self.old_nights + self.nights.len() as u32
    }
    fn add(&mut self, m: Member, max_members: usize) {
        let y = m.vp.to_f32();
        let ns = self.nights.entry(m.night).or_insert_with(|| NightStat { n: 0, sum: vec![0.0; VP_DIM] });
        ns.n += 1;
        for i in 0..VP_DIM {
            ns.sum[i] += y[i];
        }
        self.first_night = self.first_night.min(m.night);
        self.last_heard = self.last_heard.max(m.t);
        self.detections += 1;
        self.members.push_back(m);
        while self.members.len() > max_members {
            self.members.pop_front();
        }
        while self.nights.len() > MAX_NIGHT_STATS {
            let (k, ns) = self.nights.iter().next().map(|(k, v)| (*k, v.clone())).unwrap();
            self.nights.remove(&k);
            self.old_n += ns.n as u64;
            self.old_nights += 1;
            if self.old_sum.len() != VP_DIM {
                self.old_sum = vec![0.0; VP_DIM];
            }
            for i in 0..VP_DIM {
                self.old_sum[i] += ns.sum[i];
            }
        }
    }
    fn new(local: u64, m: &Member) -> Self {
        Cluster {
            local,
            nights: BTreeMap::new(),
            old_n: 0,
            old_sum: vec![0.0; VP_DIM],
            old_nights: 0,
            members: VecDeque::new(),
            first_night: m.night,
            last_heard: m.t,
            detections: 0,
            bird_id: None,
            name: None,
            named_at: None,
            back_night: None,
            back_likelihood: None,
            nameable_sent: false,
        }
    }
    /// Merge order: earliest name, then already a candidate (earliest first night), then
    /// earliest first night, then creation order.
    fn survivor_key(&self) -> (u64, u8, u32, u64) {
        (self.named_at.unwrap_or(u64::MAX), if self.bird_id.is_some() { 0 } else { 1 }, self.first_night, self.local)
    }
}

impl Population {
    fn mean(&self) -> [f32; VP_DIM] {
        let mut o = [0f32; VP_DIM];
        let d = self.mean_n.max(1) as f64;
        for i in 0..VP_DIM {
            o[i] = (self.mean_sum[i] / d) as f32;
        }
        o
    }
    /// Posterior over this population's candidates for a centred query vector.
    fn posterior(&self, q: &[f32; VP_DIM], p: &SpeciesParams) -> Option<(usize, f32)> {
        let m = self.mean();
        let mut best: Option<(usize, f64)> = None;
        let mut sum_odds = 0f64;
        for (i, c) in self.clusters.iter().enumerate() {
            if c.bird_id.is_none() {
                continue;
            }
            let s = cosine(q, &sub(&c.centroid(), &m));
            let ps = p.p_same(s).clamp(1e-4, 0.9999) as f64;
            let o = ps / (1.0 - ps);
            sum_odds += o;
            if best.map(|b| o > b.1).unwrap_or(true) {
                best = Some((i, o));
            }
        }
        best.map(|(i, o)| (i, ((o / (1.0 + sum_odds)) as f32).min(MAX_LIKELIHOOD)))
    }
}

pub struct Voiceprinter {
    cfg: Config,
    proj: Projection,
    st: State,
}

impl Voiceprinter {
    pub fn new(cfg: Config) -> Self {
        let proj = Projection::seeded(cfg.seed);
        Voiceprinter { cfg, proj, st: State { v: 1, ..Default::default() } }
    }

    /// Restores state written by [`save`](Self::save). Device-local only.
    pub fn load(cfg: Config, bytes: &[u8]) -> Result<Self, VpError> {
        let st: State = serde_json::from_slice(bytes).map_err(|e| VpError::BadState(e.to_string()))?;
        let proj = Projection::seeded(cfg.seed);
        Ok(Voiceprinter { cfg, proj, st })
    }

    /// Full state including raw voiceprints. Keep it on the device; never upload it.
    pub fn save(&self) -> Vec<u8> {
        serde_json::to_vec(&self.st).expect("state serialises")
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Where the listener is: any H3 cell of resolution >= 7.
    pub fn set_cell(&mut self, cell: &str) -> Result<(), VpError> {
        self.st.cell7 = Some(cell7_of(cell)?);
        Ok(())
    }
    pub fn set_location(&mut self, lat: f64, lon: f64) -> Result<(), VpError> {
        self.st.cell7 = Some(cell7_at(lat, lon)?);
        Ok(())
    }
    pub fn set_tz_min(&mut self, tz_min: i32) {
        self.cfg.tz_min = tz_min;
    }

    /// Takes one detection. Returns None when the detection is ignored (below the confidence
    /// floor, no location yet, bad embedding) or while the place is still warming up.
    pub fn observe(&mut self, species: &str, embedding: &[f32], t: u64, conf: f32) -> Option<Match> {
        if !(conf >= self.cfg.conf_floor) {
            return None;
        }
        let cell7 = self.st.cell7.clone()?;
        let t = to_ms(t);
        let y = self.proj.project(embedding)?;
        let vp = Voiceprint::quantise(&y);
        let night = night_of(t, self.cfg.tz_min);
        if self.st.night.map(|n| night > n).unwrap_or(true) {
            if self.st.night.is_some() {
                self.maintain();
            }
            self.st.night = Some(night);
        }
        let key = format!("{species}|{cell7}");
        let params = self.cfg.gates.get(species);
        let pop = self.st.pops.entry(key.clone()).or_insert_with(|| Population {
            species: species.to_string(),
            cell7: cell7.clone(),
            mean_n: 0,
            mean_sum: vec![0.0; VP_DIM],
            pending: Vec::new(),
            clusters: Vec::new(),
        });
        let yq = vp.to_f32();
        pop.mean_n += 1;
        for i in 0..VP_DIM {
            pop.mean_sum[i] += yq[i] as f64;
        }
        let m = Member { night: night_of(t, self.cfg.tz_min), t, vp };
        if pop.mean_n < WARMUP {
            pop.pending.push(m);
            return None;
        }
        let mut queue = std::mem::take(&mut pop.pending);
        queue.push(m);
        let mut last = None;
        for m in queue {
            last = Some(self.assign(&key, m, &params));
        }
        last
    }

    fn assign(&mut self, key: &str, m: Member, params: &SpeciesParams) -> Match {
        let max_members = self.cfg.max_members_per_cluster;
        let max_clusters = self.cfg.max_clusters_per_place;
        let min_nights = self.cfg.min_nights;
        let tonight = self.st.night.unwrap_or(m.night);
        let naming = self.cfg.gates.naming_allowed(&self.st.pops[key].species);
        let mut next_local = self.st.next_local;
        let pop = self.st.pops.get_mut(key).unwrap();
        let mean = pop.mean();
        let q = sub(&m.vp.to_f32(), &mean);
        let mut best: Option<(usize, f32)> = None;
        for (i, c) in pop.clusters.iter().enumerate() {
            let s = cosine(&q, &sub(&c.centroid(), &mean));
            if best.map(|b| s > b.1).unwrap_or(true) {
                best = Some((i, s));
            }
        }
        let (idx, new_cluster) = match best {
            Some((i, s)) if s >= params.join_tau => (i, false),
            _ => {
                if pop.clusters.len() >= max_clusters {
                    // drop the weakest non-candidate: fewest nights, then oldest
                    if let Some(w) = pop
                        .clusters
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| c.bird_id.is_none())
                        .min_by_key(|(_, c)| (c.nights_seen(), c.last_heard, c.local))
                        .map(|(i, _)| i)
                    {
                        pop.clusters.remove(w);
                    }
                }
                next_local += 1;
                pop.clusters.push(Cluster::new(next_local, &m));
                (pop.clusters.len() - 1, true)
            }
        };
        let night = m.night;
        let had_night = pop.clusters[idx].nights.contains_key(&night);
        pop.clusters[idx].add(m, max_members);
        let mut events = Vec::new();

        // A new night for this cluster: maybe it is now a candidate.
        if !had_night || pop.clusters[idx].bird_id.is_none() {
            if let Some(id) = promote(pop, idx, params, min_nights, tonight) {
                events.push(Event::Candidate {
                    bird_id: id,
                    species: pop.species.clone(),
                    nights_seen: pop.clusters[idx].nights_seen(),
                    naming_allowed: naming,
                });
            }
        }

        // "is back": tonight's calls, together, against the bird's earlier nights.
        let c = &pop.clusters[idx];
        if c.bird_id.is_some() && c.back_night != Some(night) && night == tonight {
            if let (Some((n, agg)), Some(prev)) = (c.night_mean(night), c.centroid_without(night)) {
                if n >= params.k_min && cosine(&sub(&agg, &mean), &sub(&prev, &mean)) >= params.back_tau {
                    if let Some((j, lik)) = pop.posterior(&sub(&agg, &mean), params) {
                        if j == idx {
                            let c = &mut pop.clusters[idx];
                            c.back_night = Some(night);
                            c.back_likelihood = Some(lik);
                            events.push(Event::Back {
                                bird_id: c.bird_id.clone().unwrap(),
                                name: c.name.clone(),
                                nights_seen: c.nights_seen(),
                                likelihood: lik,
                            });
                        }
                    }
                }
            }
        }

        let (bird_id, likelihood) = match pop.posterior(&q, params) {
            Some((j, lik)) if j == idx => (pop.clusters[idx].bird_id.clone(), Some(lik)),
            _ => (None, None),
        };
        nameable(pop, naming, &mut events);
        self.st.next_local = next_local;
        self.st.events.extend(events);
        Match { bird_id, likelihood, new_cluster }
    }

    /// Splits, merges, candidate promotion and forgetting. Runs by itself at each night change;
    /// callers may also run it (e.g. once a day). Deterministic for a given state.
    pub fn maintain(&mut self) {
        let keys: Vec<String> = self.st.pops.keys().cloned().collect();
        let tonight = self.st.night.unwrap_or(0);
        for key in keys {
            let species = self.st.pops[&key].species.clone();
            let params = self.cfg.gates.get(&species);
            let naming = self.cfg.gates.naming_allowed(&species);
            let mut events = Vec::new();
            let mut aliases = Vec::new();
            let mut next_local = self.st.next_local;
            {
                let pop = self.st.pops.get_mut(&key).unwrap();
                if pop.mean_n < WARMUP {
                    continue;
                }
                // splits
                let mut i = 0;
                while i < pop.clusters.len() {
                    if let Some(newc) = try_split(pop, i, &params, &mut next_local) {
                        if let Some(id) = &pop.clusters[i].bird_id {
                            events.push(Event::Split { from: id.clone() });
                        }
                        pop.clusters.push(newc);
                    }
                    i += 1;
                }
                // merges, closest pair first
                loop {
                    let m = pop.mean();
                    let cents: Vec<_> = pop.clusters.iter().map(|c| sub(&c.centroid(), &m)).collect();
                    let mut best: Option<(usize, usize, f32)> = None;
                    for a in 0..cents.len() {
                        for b in a + 1..cents.len() {
                            let s = cosine(&cents[a], &cents[b]);
                            let one_candidate = pop.clusters[a].bird_id.is_some() != pop.clusters[b].bird_id.is_some();
                            let tau = if one_candidate { params.back_tau.max(params.join_tau) } else { params.merge_tau };
                            if s >= tau && best.map(|x| s > x.2).unwrap_or(true) {
                                best = Some((a, b, s));
                            }
                        }
                    }
                    let Some((a, b, _)) = best else { break };
                    let (keep, gone) =
                        if pop.clusters[a].survivor_key() <= pop.clusters[b].survivor_key() { (a, b) } else { (b, a) };
                    let g = pop.clusters.remove(gone);
                    let keep = if gone < keep { keep - 1 } else { keep };
                    if let (Some(from), Some(into)) = (&g.bird_id, &pop.clusters[keep].bird_id) {
                        aliases.push((from.clone(), into.clone()));
                        events.push(Event::Merged { from: from.clone(), into: into.clone(), dropped_name: g.name.clone() });
                    }
                    absorb(&mut pop.clusters[keep], g, self.cfg.max_members_per_cluster);
                }
                // promotions and forgetting
                for i in 0..pop.clusters.len() {
                    if let Some(id) = promote(pop, i, &params, self.cfg.min_nights, tonight) {
                        events.push(Event::Candidate {
                            bird_id: id,
                            species: species.clone(),
                            nights_seen: pop.clusters[i].nights_seen(),
                            naming_allowed: naming,
                        });
                    }
                }
                nameable(pop, naming, &mut events);
                let forget = self.cfg.forget_after_nights;
                let tz = self.cfg.tz_min;
                pop.clusters.retain(|c| c.bird_id.is_some() || night_of(c.last_heard, tz) + forget >= tonight);
            }
            self.st.next_local = next_local;
            for (f, t) in aliases {
                // re-point anything that pointed at the absorbed id
                for v in self.st.aliases.values_mut() {
                    if *v == f {
                        *v = t.clone();
                    }
                }
                self.st.aliases.insert(f, t);
            }
            self.st.events.extend(events);
        }
    }

    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.st.events)
    }

    /// Follows merges to the surviving id.
    pub fn resolve(&self, bird_id: &str) -> String {
        self.st.aliases.get(bird_id).cloned().unwrap_or_else(|| bird_id.to_string())
    }

    fn find(&self, bird_id: &str) -> Option<(&Population, &Cluster)> {
        let id = self.resolve(bird_id);
        self.st.pops.values().find_map(|p| p.clusters.iter().find(|c| c.bird_id.as_deref() == Some(&id)).map(|c| (p, c)))
    }

    fn candidate(&self, p: &Population, c: &Cluster) -> Candidate {
        let tonight = self.st.night.unwrap_or(0);
        Candidate {
            bird_id: c.bird_id.clone().unwrap_or_default(),
            species: p.species.clone(),
            cell7: p.cell7.clone(),
            nights_seen: c.nights_seen(),
            first_night: c.first_night,
            last_heard: c.last_heard,
            detections: c.detections,
            name: c.name.clone(),
            naming_allowed: self.cfg.gates.naming_allowed(&p.species),
            heard_tonight: c.nights.get(&tonight).map(|n| n.n).unwrap_or(0),
            back_likelihood: if c.back_night == Some(tonight) { c.back_likelihood } else { None },
        }
    }

    /// Candidate individuals, most recently heard first.
    pub fn candidates(&self) -> Vec<Candidate> {
        let mut v: Vec<Candidate> = self
            .st
            .pops
            .values()
            .flat_map(|p| p.clusters.iter().filter(|c| c.bird_id.is_some()).map(move |c| (p, c)))
            .map(|(p, c)| self.candidate(p, c))
            .collect();
        v.sort_by(|a, b| b.last_heard.cmp(&a.last_heard).then(a.bird_id.cmp(&b.bird_id)));
        v
    }

    pub fn status(&self, bird_id: &str) -> Option<Candidate> {
        self.find(bird_id).map(|(p, c)| self.candidate(p, c))
    }

    /// True when the bird was recognised tonight (enough calls, matching its earlier nights).
    pub fn is_back(&self, bird_id: &str) -> bool {
        let tonight = self.st.night;
        self.find(bird_id).map(|(_, c)| tonight.is_some() && c.back_night == tonight).unwrap_or(false)
    }

    pub fn nights_seen(&self, bird_id: &str) -> Option<u32> {
        self.find(bird_id).map(|(_, c)| c.nights_seen())
    }

    pub fn last_heard(&self, bird_id: &str) -> Option<u64> {
        self.find(bird_id).map(|(_, c)| c.last_heard)
    }

    /// Local label for a candidate. Only for species that passed the naming gate. The chain
    /// BIRD_NAME entry takes the same bird_id once that feature is active.
    pub fn name(&mut self, bird_id: &str, name: &str) -> Result<(), VpError> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(VpError::BadName);
        }
        let id = self.resolve(bird_id);
        let now = self.st.night.map(|n| n as u64).unwrap_or(0);
        for p in self.st.pops.values_mut() {
            if let Some(c) = p.clusters.iter_mut().find(|c| c.bird_id.as_deref() == Some(&id)) {
                if !self.cfg.gates.naming_allowed(&p.species) {
                    return Err(VpError::NamingNotAllowed(p.species.clone()));
                }
                c.name = Some(name.to_string());
                // first naming time is what "earliest name wins" compares; renaming keeps it
                if c.named_at.is_none() {
                    c.named_at = Some(c.last_heard.max(now));
                }
                return Ok(());
            }
        }
        Err(VpError::UnknownBird)
    }

    /// The user says two candidates are one bird ("this is also Ruben"). Merged by the usual rule
    /// (earliest name, then earlier candidate, then earlier first night); returns the surviving id.
    pub fn link(&mut self, a: &str, b: &str) -> Result<String, VpError> {
        let (a, b) = (self.resolve(a), self.resolve(b));
        if a == b {
            return Ok(a);
        }
        let max_members = self.cfg.max_members_per_cluster;
        let key = self
            .st
            .pops
            .iter()
            .find(|(_, p)| p.clusters.iter().any(|c| c.bird_id.as_deref() == Some(&a)))
            .map(|(k, _)| k.clone())
            .ok_or(VpError::UnknownBird)?;
        let pop = self.st.pops.get_mut(&key).unwrap();
        let ia = pop.clusters.iter().position(|c| c.bird_id.as_deref() == Some(&a)).unwrap();
        let Some(ib) = pop.clusters.iter().position(|c| c.bird_id.as_deref() == Some(&b)) else {
            let known = self.st.pops.values().any(|p| p.clusters.iter().any(|c| c.bird_id.as_deref() == Some(&b)));
            return Err(if known { VpError::NotSamePlace } else { VpError::UnknownBird });
        };
        let (keep, gone) = if pop.clusters[ia].survivor_key() <= pop.clusters[ib].survivor_key() { (ia, ib) } else { (ib, ia) };
        let g = pop.clusters.remove(gone);
        let keep = if gone < keep { keep - 1 } else { keep };
        let into = pop.clusters[keep].bird_id.clone().unwrap();
        let from = g.bird_id.clone().unwrap();
        let dropped = g.name.clone();
        absorb(&mut pop.clusters[keep], g, max_members);
        for v in self.st.aliases.values_mut() {
            if *v == from {
                *v = into.clone();
            }
        }
        self.st.aliases.insert(from.clone(), into.clone());
        self.st.events.push(Event::Merged { from, into: into.clone(), dropped_name: dropped });
        Ok(into)
    }

    /// Summary for the 4 KB device report: bird ids and counts only, no voiceprints, no names,
    /// no cell (the report carries its own coarse cell). Trimmed to `max_bytes`.
    pub fn report(&self, max_bytes: usize) -> String {
        let tonight = self.st.night.unwrap_or(0);
        let mut birds: Vec<serde_json::Value> = self
            .candidates()
            .into_iter()
            .map(|c| {
                let mut j = serde_json::json!({
                    "id": c.bird_id,
                    "sp": c.species,
                    "nights": c.nights_seen,
                    "last_night": night_of(c.last_heard, self.cfg.tz_min),
                    "tonight": c.heard_tonight,
                });
                if let Some(p) = c.back_likelihood {
                    j["back_q8"] = serde_json::json!((p * 255.0).round() as u32);
                }
                j
            })
            .collect();
        loop {
            let s = serde_json::json!({"v": 1, "night": tonight, "birds": birds}).to_string();
            if s.len() <= max_bytes || birds.is_empty() {
                return s;
            }
            birds.pop();
        }
    }
}

impl VoiceprintSink for Voiceprinter {
    fn on_detection(&mut self, species: &str, embedding: &[f32], t: u64, conf: f32) {
        let _ = self.observe(species, embedding, t, conf);
    }
}

/// bird_id = SHA-256(species, res-7 cell, quantised centroid, first night) as 64 lowercase hex,
/// the format the chain's BIRD_OBSERVE / BIRD_NAME entries take.
pub fn bird_id(species: &str, cell7: &str, centred_centroid: &[f32; VP_DIM], first_night: u32) -> String {
    let mut bits = [0u8; VP_DIM / 8];
    for (i, v) in centred_centroid.iter().enumerate() {
        if *v > 0.0 {
            bits[i / 8] |= 1 << (i % 8);
        }
    }
    let mut h = Sha256::new();
    h.update(b"hone-voiceprint/v1\0");
    h.update(species.as_bytes());
    h.update(b"\0");
    h.update(cell7.as_bytes());
    h.update(b"\0");
    h.update(bits);
    h.update(first_night.to_le_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn promote(pop: &mut Population, i: usize, p: &SpeciesParams, min_nights: u32, _tonight: u32) -> Option<String> {
    let c = &pop.clusters[i];
    if c.bird_id.is_some() {
        return None;
    }
    let m = pop.mean();
    let cent = sub(&c.centroid(), &m);
    // nights with at least two calls; the last `min_nights` of them must each match the centroid
    let good: Vec<u32> = c.nights.iter().filter(|(_, ns)| ns.n >= 2).map(|(k, _)| *k).collect();
    if (good.len() as u32) < min_nights {
        return None;
    }
    for n in good.iter().rev().take(min_nights as usize) {
        let (_, nm) = c.night_mean(*n)?;
        if cosine(&sub(&nm, &m), &cent) < p.back_tau {
            return None;
        }
    }
    // looks like a bird we already know: it will be absorbed at the next maintain(), no new id
    let tau = p.back_tau.max(p.join_tau);
    if pop.clusters.iter().any(|o| o.bird_id.is_some() && cosine(&sub(&o.centroid(), &m), &cent) >= tau) {
        return None;
    }
    let mut id = bird_id(&pop.species, &pop.cell7, &cent, c.first_night);
    let mut k = 1;
    while pop.clusters.iter().any(|o| o.bird_id.as_deref() == Some(&id)) {
        id = bird_id(&format!("{}#{k}", pop.species), &pop.cell7, &cent, c.first_night);
        k += 1;
    }
    pop.clusters[i].bird_id = Some(id.clone());
    Some(id)
}

fn nameable(pop: &mut Population, naming: bool, events: &mut Vec<Event>) {
    if !naming {
        return;
    }
    for c in pop.clusters.iter_mut() {
        if let (Some(id), false) = (&c.bird_id, c.nameable_sent) {
            c.nameable_sent = true;
            events.push(Event::Nameable { bird_id: id.clone(), species: pop.species.clone(), nights_seen: c.nights_seen() });
        }
    }
}

fn absorb(keep: &mut Cluster, g: Cluster, max_members: usize) {
    for (k, ns) in g.nights {
        let e = keep.nights.entry(k).or_insert_with(|| NightStat { n: 0, sum: vec![0.0; VP_DIM] });
        e.n += ns.n;
        for i in 0..VP_DIM {
            e.sum[i] += ns.sum[i];
        }
    }
    keep.old_n += g.old_n;
    keep.old_nights += g.old_nights;
    for i in 0..VP_DIM.min(g.old_sum.len()) {
        keep.old_sum[i] += g.old_sum[i];
    }
    let mut mem: Vec<Member> = keep.members.drain(..).chain(g.members).collect();
    mem.sort_by_key(|m| m.t);
    let skip = mem.len().saturating_sub(max_members);
    keep.members = mem.into_iter().skip(skip).collect();
    keep.first_night = keep.first_night.min(g.first_night);
    keep.last_heard = keep.last_heard.max(g.last_heard);
    keep.detections += g.detections;
    keep.nameable_sent |= g.nameable_sent && keep.bird_id.is_some();
    if keep.name.is_none() {
        keep.name = g.name;
        keep.named_at = g.named_at;
    }
}

/// 2-means on a cluster's kept members. Deterministic seeds: the member farthest from the
/// centroid, then the member farthest from that one.
fn try_split(pop: &mut Population, i: usize, p: &SpeciesParams, next_local: &mut u64) -> Option<Cluster> {
    let m = pop.mean();
    let c = &pop.clusters[i];
    if c.members.len() < 2 * SPLIT_MIN {
        return None;
    }
    let xs: Vec<[f32; VP_DIM]> = c.members.iter().map(|mm| sub(&mm.vp.to_f32(), &m)).collect();
    let cent = sub(&c.centroid(), &m);
    let far = |from: &[f32; VP_DIM]| {
        (0..xs.len()).min_by(|a, b| cosine(&xs[*a], from).partial_cmp(&cosine(&xs[*b], from)).unwrap().then(a.cmp(b))).unwrap()
    };
    let a0 = far(&cent);
    let b0 = far(&xs[a0]);
    let (mut ca, mut cb) = (xs[a0], xs[b0]);
    let mut lab = vec![false; xs.len()];
    for _ in 0..10 {
        for (k, x) in xs.iter().enumerate() {
            lab[k] = cosine(x, &cb) > cosine(x, &ca);
        }
        let mean_of = |want: bool| {
            let mut s = [0f32; VP_DIM];
            let mut n = 0;
            for (k, x) in xs.iter().enumerate() {
                if lab[k] == want {
                    n += 1;
                    for d in 0..VP_DIM {
                        s[d] += x[d];
                    }
                }
            }
            (n, s)
        };
        let (na, sa) = mean_of(false);
        let (nb, sb) = mean_of(true);
        if na == 0 || nb == 0 {
            return None;
        }
        ca = sa;
        cb = sb;
    }
    let na = lab.iter().filter(|l| !**l).count();
    let nb = lab.len() - na;
    if na < SPLIT_MIN || nb < SPLIT_MIN || cosine(&ca, &cb) >= p.split_tau {
        return None;
    }
    let nights_of = |want: bool| -> BTreeSet<u32> {
        c.members.iter().zip(&lab).filter(|(_, l)| **l == want).map(|(mm, _)| mm.night).collect()
    };
    let (sa, sb) = (nights_of(false), nights_of(true));
    if sa.len() < 2 || sb.len() < 2 {
        return None;
    }
    // the part heard first keeps the id and name; ties go to the larger part, then part A
    let first_a = (sa.iter().next().copied(), std::cmp::Reverse(na));
    let first_b = (sb.iter().next().copied(), std::cmp::Reverse(nb));
    let move_label = if first_a <= first_b { true } else { false };
    let c = &mut pop.clusters[i];
    let mut moved = Vec::new();
    let mut kept = VecDeque::new();
    for (mm, l) in c.members.drain(..).zip(lab) {
        if l == move_label {
            moved.push(mm);
        } else {
            kept.push_back(mm);
        }
    }
    c.members = kept;
    for mm in &moved {
        if let Some(ns) = c.nights.get_mut(&mm.night) {
            let y = mm.vp.to_f32();
            ns.n = ns.n.saturating_sub(1);
            for d in 0..VP_DIM {
                ns.sum[d] -= y[d];
            }
        }
    }
    c.nights.retain(|_, ns| ns.n > 0);
    c.detections = c.detections.saturating_sub(moved.len() as u64);
    *next_local += 1;
    let mut nc = Cluster::new(*next_local, &moved[0]);
    for mm in moved {
        nc.add(mm, usize::MAX);
    }
    Some(nc)
}

#[cfg(test)]
mod tests;
