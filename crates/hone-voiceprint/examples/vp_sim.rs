//! Replays a labelled stream through the real Voiceprinter, for evaluation.
//!
//! usage: vp_sim SPECIES CELL GATES.json < stream.bin > out.jsonl
//! stream.bin: repeated records of  u32 label | u64 t_ms | f32 conf | 1536 x f32  (little endian)
//! out.jsonl: one line per record {"i","label","bird","lik"}, then {"events":[..]} and
//! {"candidates":[..]} lines. Labels never enter the Voiceprinter.
use hone_voiceprint::{gate::GateTable, Config, Voiceprinter};
use std::io::{Read, Write};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (species, cell, gates) = (&a[1], &a[2], &a[3]);
    let g: GateTable = serde_json::from_str(&std::fs::read_to_string(gates).unwrap()).unwrap();
    let mut v = Voiceprinter::new(Config { gates: g, ..Config::default() });
    v.set_cell(cell).unwrap();
    let mut buf = Vec::new();
    std::io::stdin().read_to_end(&mut buf).unwrap();
    let rec = 4 + 8 + 4 + 1536 * 4;
    let out = std::io::stdout();
    let mut o = out.lock();
    let mut events = Vec::new();
    for (i, r) in buf.chunks_exact(rec).enumerate() {
        let label = u32::from_le_bytes(r[0..4].try_into().unwrap());
        let t = u64::from_le_bytes(r[4..12].try_into().unwrap());
        let conf = f32::from_le_bytes(r[12..16].try_into().unwrap());
        let e: Vec<f32> = r[16..].chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        let m = v.observe(species, &e, t, conf);
        let (bird, lik) = m.map(|m| (m.bird_id, m.likelihood)).unwrap_or((None, None));
        for ev in v.drain_events() {
            events.push(serde_json::json!({"i": i, "night": hone_voiceprint::night_of(t, 0), "ev": ev}));
        }
        writeln!(o, "{}", serde_json::json!({"i": i, "label": label, "t": t, "bird": bird, "lik": lik})).unwrap();
    }
    v.maintain();
    for ev in v.drain_events() {
        events.push(serde_json::json!({"i": null, "ev": ev}));
    }
    writeln!(o, "{}", serde_json::json!({"events": events})).unwrap();
    writeln!(o, "{}", serde_json::json!({"candidates": v.candidates()})).unwrap();
}
