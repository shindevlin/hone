//! Genesis block creation.
//!
//! Reads genesis.json if present:
//! {
//!   "accounts": { "alice": 1000000000, "bob": 500000000 },
//!   "timestamp": 1234567890000
//! }
//! Amounts are in dreams (1 BTCPC = 10_000_000_000 dreams).

use std::path::Path;
use anyhow::Result;
use tracing::info;
use btcpc_types::{Block, BlockHeader, LedgerEntry, NATIVE_TOKEN};

use crate::chain::Chain;

pub fn init_genesis(chain: &Chain, genesis_file: Option<&Path>, genesis_timestamp: Option<u64>) -> Result<Block> {
    if chain.store.has_block(0) {
        let data = chain.store.read_block(0)?.expect("genesis exists");
        let block = Block::from_bytes(&data).expect("genesis parseable");
        info!("existing genesis: {}", block.header.hash_hex());
        return Ok(block);
    }

    let ts = genesis_timestamp.ok_or_else(|| anyhow::anyhow!(
        "BTCPC_GENESIS_TIMESTAMP is not set.\n\
         All nodes must use the same timestamp to produce identical genesis blocks.\n\
         Set BTCPC_GENESIS_TIMESTAMP=<unix_ms> in the environment (e.g. 1746000000000)."
    ))?;

    let mut entries: Vec<LedgerEntry> = Vec::new();

    if let Some(path) = genesis_file {
        if path.exists() {
            let raw = std::fs::read_to_string(path)?;
            let cfg: serde_json::Value = serde_json::from_str(&raw)?;

            if let Some(accounts) = cfg.get("accounts").and_then(|v| v.as_object()) {
                for (account, amount) in accounts {
                    let dreams = amount.as_u64().ok_or_else(|| anyhow::anyhow!(
                        "genesis: account '{}' amount must be a u64 integer (got {})",
                        account, amount
                    ))?;
                    entries.push(LedgerEntry::AccountCreate {
                        account: account.clone(),
                        public_key: None,
                        epoch: 0,
                    });
                    entries.push(LedgerEntry::GenesisAlloc {
                        account: account.clone(),
                        amount: dreams,
                        token: NATIVE_TOKEN.to_string(),
                    });
                }
            }
        }
    }

    // Apply genesis entries
    for entry in &entries {
        chain.apply_entry(entry)?;
    }

    let entry_hashes: Vec<String> = entries.iter().map(|e| e.hash()).collect();
    let tx_root = btcpc_types::merkle_root(&entry_hashes);

    let mut header = BlockHeader::genesis(ts);
    header.merkle_root_transactions = tx_root;

    let payload = serde_json::json!({
        "ledger_entries": entries,
        "rewards": [],
        "compute_proofs": [],
        "chain_id": chain.chain_id,
    });

    let block = Block { header, payload };
    chain.store.write_block(0, &block.to_bytes())?;
    chain.store.set_meta("genesis_hash", block.header.hash_hex().as_bytes())?;

    info!("genesis created: {}", block.header.hash_hex());
    Ok(block)
}
