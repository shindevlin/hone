//! Genesis block creation.
//!
//! Reads genesis.json if present:
//! {
//!   "accounts": {
//!     "alice": { "keys": { "posting": "02abc..." }, "balance": 0 },
//!     "bob":   { "public_key": "02abc..." }  // legacy: treated as posting key
//!   }
//! }
//! keys: role->pubkey map. public_key (legacy) is treated as the posting key.
//! balance: hunits (1 HONE = 10_000_000_000). Omit or 0 for no allocation.

use std::path::Path;
use anyhow::Result;
use tracing::info;
use hone_types::{Block, BlockHeader, LedgerEntry, NATIVE_TOKEN};

use crate::chain::Chain;
use crate::reserved_names;

pub fn init_genesis(chain: &Chain, genesis_file: Option<&Path>, genesis_timestamp: Option<u64>) -> Result<Block> {
    if chain.store.has_block(0) {
        let data = chain.store.read_block(0)?.expect("genesis exists");
        let block = Block::from_bytes(&data).expect("genesis parseable");
        info!("existing genesis: {}", block.header.hash_hex());

        // Enforce chain_id consistency. If not yet stamped (legacy DB), stamp it now.
        // If stamped and mismatching, refuse to start — prevents accidental cross-chain DB reuse.
        match chain.store.get_meta("chain_id") {
            Some(stored) => {
                let stored_id = String::from_utf8_lossy(&stored);
                if stored_id != chain.chain_id.as_str() {
                    // Allow one-time re-stamp when explicitly migrating a DB to a new chain_id.
                    // Set HONE_CHAIN_ID_MIGRATION=1 for a single restart, then remove it.
                    if std::env::var("HONE_CHAIN_ID_MIGRATION").is_ok() {
                        chain.store.set_meta("chain_id", chain.chain_id.as_bytes())?;
                        info!("chain_id migrated: '{}' → '{}'", stored_id, chain.chain_id);
                    } else {
                        eprintln!(
                            "hone-node: chain_id mismatch — this database was initialized as '{}' \
                             but HONE_CHAIN_ID='{}'. Use a separate HONE_DATA_DIR for each chain. \
                             To migrate deliberately, set HONE_CHAIN_ID_MIGRATION=1 for one restart.",
                            stored_id, chain.chain_id
                        );
                        std::process::exit(1);
                    }
                }
            }
            None => {
                // Legacy DB without stored chain_id — stamp it now.
                chain.store.set_meta("chain_id", chain.chain_id.as_bytes())?;
                info!("chain_id '{}' stamped into existing database", chain.chain_id);
            }
        }

        return Ok(block);
    }

    let ts = genesis_timestamp.ok_or_else(|| anyhow::anyhow!(
        "HONE_GENESIS_TIMESTAMP is not set.\n\
         All nodes must use the same timestamp to produce identical genesis blocks.\n\
         Set HONE_GENESIS_TIMESTAMP=<unix_ms> in the environment (e.g. 1746000000000)."
    ))?;

    let mut entries: Vec<LedgerEntry> = Vec::new();

    if let Some(path) = genesis_file {
        if path.exists() {
            let raw = std::fs::read_to_string(path)?;
            let cfg: serde_json::Value = serde_json::from_str(&raw)?;

            if let Some(accounts) = cfg.get("accounts").and_then(|v| v.as_object()) {
                for (account, val) in accounts {
                    let (keys, hunits) = if val.is_object() {
                        // Support new "keys" map or legacy "public_key" as shorthand for posting key.
                        let mut km: std::collections::BTreeMap<String, String> = val
                            .get("keys")
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                            .unwrap_or_default();
                        if km.is_empty() {
                            if let Some(pk) = val.get("public_key")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                            {
                                km.insert("posting".to_string(), pk.to_string());
                            }
                        }
                        let bal = val.get("balance").and_then(|v| v.as_u64()).unwrap_or(0);
                        (km, bal)
                    } else {
                        let hunits = val.as_u64().ok_or_else(|| anyhow::anyhow!(
                            "genesis: account '{}' must be an object or integer", account
                        ))?;
                        (std::collections::BTreeMap::new(), hunits)
                    };

                    entries.push(LedgerEntry::AccountCreate {
                        account:             account.clone(),
                        keys,
                        chain_proofs:        vec![],
                        epoch:               0,
                        funded_by:           None,
                        machine_fingerprint: None,
                    });
                    if hunits > 0 {
                        entries.push(LedgerEntry::GenesisAlloc {
                            account: account.clone(),
                            amount: hunits,
                            token: NATIVE_TOKEN.to_string(),
                        });
                    }
                }
            }
        }
    }

    // Register shindevlin's reserved namespace: ~1050 names (singles, doubles, popular worldwide).
    // All point to shindevlin's posting key so he can transfer them into the identity marketplace.
    let shindevlin_keys: std::collections::BTreeMap<String, String> = [
        ("posting".to_string(), reserved_names::SHINDEVLIN_POSTING_KEY.to_string()),
    ].into_iter().collect();

    for name in reserved_names::all_reserved() {
        // Skip names that are already registered via genesis.json (shindevlin itself, etc.)
        if chain.store.get_account(&name)?.is_some() {
            continue;
        }
        entries.push(LedgerEntry::AccountCreate {
            account:             name,
            keys:                shindevlin_keys.clone(),
            chain_proofs:        vec![],
            epoch:               0,
            funded_by:           None,
            machine_fingerprint: None,
        });
    }

    info!("reserved namespace: {} names registered to shindevlin", reserved_names::all_reserved().len());

    // Apply genesis entries
    for entry in &entries {
        chain.apply_entry(entry)?;
    }

    let entry_hashes: Vec<String> = entries.iter().map(|e| e.hash()).collect();
    let tx_root = hone_types::merkle_root(&entry_hashes);

    let mut header = BlockHeader::genesis(ts);
    header.merkle_root_transactions = tx_root;

    let payload = serde_json::json!({
        "ledger_entries": entries,
        "rewards": [],
        "output_hashes": [],
        "chain_id": chain.chain_id,
        "launch": {
            "proclamation": "HONE launched at noon, Los Angeles, 2026-07-04 12:00:00 PDT (America's 250th — freedom tech for a free people)",
            "timestamp_tz": "2026-07-04T12:00:00-07:00",
            "timestamp_utc": "2026-07-04T19:00:00Z",
            "genesis_ms": 1783191600000u64,
        },
    });

    let block = Block { header, payload };
    chain.store.write_block(0, &block.to_bytes())?;
    chain.store.set_meta("genesis_hash", block.header.hash_hex().as_bytes())?;
    chain.store.set_meta("chain_id", chain.chain_id.as_bytes())?;
    chain.store.set_meta("launch_proclamation", b"HONE - noon Los Angeles, 2026-07-04 (America's 250th)")?;

    // Seed governance council on-chain (D5). 2-of-3 required for parameter changes.
    let gov_keys: Vec<&str> = vec!["shindevlin", "natoshisakamoto", "josh"];
    let _ = chain.store.state_set("chain_param:governance_keys",
        &serde_json::to_vec(&gov_keys).unwrap_or_default());

    info!("genesis created: {}", block.header.hash_hex());
    Ok(block)
}
