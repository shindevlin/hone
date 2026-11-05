# hone-voiceprint

Individual birds by voice: name one ("Ruben", a Great Kiskadee) and later see
"Ruben is back, night 14". Spec: white paper §2d (sightings, coverage, rewards).

Evaluation, per-species numbers and the naming gate: [eval/RESULTS.md](eval/RESULTS.md).

## Pipeline

1. **Input.** One Perch v2 detection: species, the 1,536-float embedding of the 5 s frame, time,
   confidence. The caller has already dropped frames with any human class (speech gate); this
   crate also ignores anything below `conf_floor` (0.6).
2. **Voiceprint.** `W · e/|e|` with a fixed seeded 64 x 1536 Gaussian projection, int8 plus one
   scale (68 bytes). Centring by the place's running mean happens at compare time (linear, so old
   prints stay comparable). Not invertible: no audio can be rebuilt from it, and no audio is kept.
3. **Clustering.** Online, per (species, H3 res-7 cell). A call joins the closest cluster if the
   centred cosine is at least `join_tau`, else starts a new one. The first 20 calls of a place only
   warm up its mean.
4. **Candidates.** A cluster becomes a candidate individual after `min_nights` (3) nights with at
   least 2 calls each, each night's mean matching the centroid (`back_tau`). Then it gets
   `bird_id = SHA-256(species, cell res 7, sign bits of the centred centroid, first night)`, 64 hex,
   frozen.
5. **Is back.** Once per night, when at least `k_min` calls tonight, together, match the bird's
   earlier nights (`back_tau`) and the bird is the most likely candidate. The likelihood is a
   calibrated logistic of the cosine, normalised against the other candidates at that place, capped
   at 0.99. `phrase()` gives "very likely / likely / possibly / unsure".
6. **Maintenance** (each night change, deterministic): splits (2-means on kept members; the part
   heard first keeps the id and name), merges (centroids >= `merge_tau`; earliest name wins, then
   earlier candidate, then earlier first night; the absorbed id resolves to the survivor), forgetting
   unpromoted clusters after 45 quiet nights.
7. **Gate.** Per-species table `eval/gates.json`. Species not listed: clustered, never offered
   for naming. Night = local noon to noon.

## API

```rust
use hone_voiceprint::{Config, Voiceprinter, VoiceprintSink, Event};

let mut vp = Voiceprinter::new(Config { tz_min: -360, ..Config::default() });
vp.set_cell("8966c2d6b6bffff")?;                     // any H3 cell res >= 7, or set_location(lat, lon)
vp.on_detection("Pitangus sulphuratus", &emb, t_ms, conf);   // VoiceprintSink (listener)
let m = vp.observe(species, &emb, t_ms, conf);       // same, returns Option<Match{bird_id, likelihood, new_cluster}>
for e in vp.drain_events() { /* Candidate, Nameable, Back, Merged, Split */ }
vp.candidates();            // Vec<Candidate{bird_id, species, cell7, nights_seen, first_night, last_heard,
                            //   detections, name, naming_allowed, heard_tonight, back_likelihood}>
vp.is_back(&id); vp.nights_seen(&id); vp.last_heard(&id); vp.status(&id); vp.resolve(&id);
vp.name(&id, "Ruben")?;     // local label; Err(NamingNotAllowed) for gated-off species
vp.link(&a, &b)?;        // user: "this is also Ruben"; returns the survivor (earliest name wins)
vp.report(4096);            // {"v":1,"night":N,"birds":[{"id","sp","nights","last_night","tonight","back_q8"}]}
let bytes = vp.save();      // full state incl. raw voiceprints: device only
let vp = Voiceprinter::load(cfg, &bytes)?;
```

`t` is unix ms (values below 1e11 are read as seconds).

## Events and the chain (branch seats-cards)

| Event | Integration does |
|---|---|
| `Nameable{bird_id}` (first time a candidate passes the naming gate) | submit `BIRD_OBSERVE {bird_id}` with the seat key; offer "Name this bird" |
| `Back{bird_id, name, nights_seen, likelihood}` | show "Ruben is back, night 14 (very likely, 92%)" |
| `Merged{from, into, dropped_name}` | re-point UI; file BIRD_MERGE per eval/MERGE_AUTHORITY.md |
| `Candidate`, `Split` | informational |

Names: same rule as BIRD_NAME (1-24 letters, spaces, apostrophes, hyphens). The naming account is
the device owner's account.

## Builds

- Library + tests: `cargo test --release`
- Listener (Rust): depend by path `hone-voiceprint = { path = "../crates/hone-voiceprint" }`,
  hold a `Voiceprinter`, pass it as `&mut dyn VoiceprintSink`.
- Android: `clients/hone-android-native/tools/build-voiceprint.sh` (cargo-ndk, arm64-v8a) puts
  `libhone_voiceprint.so` in jniLibs; Kotlin wrapper `network.hone.app.sound.Voiceprints`.
- JNI host check: `tools/jni-smoke.sh`.
- Evaluation replay of the real crate: `cargo build --release --example vp_sim`, then
  `eval/scripts/sim.py (paths in the scripts point at ~/voiceprint-eval on beastly)`.
