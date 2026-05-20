use crate::config::Config;
use crate::models::LedgerEntry;
use crate::state::SharedState;
use anyhow::Result;
use serde_json::Value;
use std::io::BufRead;
use std::io::Write;
use std::path::Path;
use tracing::warn;

/// Append one ledger entry to pending-entries.jsonl and apply it to shared state.
pub fn persist(cfg: &Config, state: &SharedState, entry: &LedgerEntry) -> Result<()> {
    let line = serde_json::to_string(entry)? + "\n";
    let path = cfg.pending_entries_path();

    // Ensure parent dir exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;

    // Apply to in-memory state
    if let Ok(val) = serde_json::to_value(entry) {
        state.write().apply_entry(&val);
    }

    Ok(())
}

/// Load all commerce entries from pending-entries.jsonl into state at startup.
pub fn load_pending_entries(cfg: &Config, state: &SharedState) -> usize {
    let path = cfg.pending_entries_path();
    if !path.exists() {
        return 0;
    }
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => { warn!("Cannot open pending-entries.jsonl: {e}"); return 0; }
    };
    let mut count = 0;
    for line in std::io::BufReader::new(file).lines() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            _ => continue,
        };
        if let Ok(val) = serde_json::from_str::<Value>(&line) {
            state.write().apply_entry(&val);
            count += 1;
        }
    }
    count
}

/// Load commerce entries from a finalized block file at startup.
/// Walks data/blocks/*.bin files — these are JSON (text) block payloads.
pub fn load_block_files(cfg: &Config, state: &SharedState) -> usize {
    let blocks_dir = Path::new(&cfg.data_dir).join("blocks");
    if !blocks_dir.exists() {
        return 0;
    }
    let mut count = 0;
    let mut entries = match std::fs::read_dir(&blocks_dir) {
        Ok(e) => e.collect::<Vec<_>>(),
        Err(_) => return 0,
    };
    entries.sort_by_key(|e| {
        e.as_ref().ok().and_then(|de| de.file_name().to_str().map(str::to_string)).unwrap_or_default()
    });
    for entry in entries {
        let path = match entry { Ok(e) => e.path(), Err(_) => continue };
        if path.extension().and_then(|e| e.to_str()) != Some("bin") { continue; }
        // Block files are JSON stored in .bin — try parsing
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if let Ok(block) = serde_json::from_str::<Value>(&content) {
            if let Some(entries_arr) = block["payload"]["ledger_entries"].as_array() {
                for e in entries_arr {
                    state.write().apply_entry(e);
                    count += 1;
                }
            }
        }
    }
    count
}
