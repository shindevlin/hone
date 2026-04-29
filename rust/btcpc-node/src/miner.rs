//! Mining loop — produces a block every epoch.

use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use sha2::{Sha256, Digest};
use tracing::{info, warn};
use btcpc_types::{Block, BlockHeader, LedgerEntry, DREAMS_PER_BTCPC, EPOCH_MS};

use crate::chain::Chain;
use crate::utils::now_ms;

/// Block reward: 50 BTCPC per block (halving every 210_000 epochs like Bitcoin).
pub fn block_reward(epoch: u64) -> u64 {
    let halvings = epoch / 210_000;
    if halvings >= 64 { return 0; }
    (50 * DREAMS_PER_BTCPC) >> halvings
}

pub async fn run_miner(chain: Arc<Chain>, account: String) {
    info!("miner started: account={}", account);
    // Base on actual stored blocks, not the in-memory epoch counter (which only
    // advances via EpochSeal).  This prevents the inflation bug where current_epoch
    // stays 0 and the miner keeps overwriting epoch 1.
    let mut last_produced: u64 = chain.store.latest_epoch().unwrap_or(0) as u64;

    loop {
        let next_epoch = last_produced + 1;

        // Wait until the next epoch boundary
        let epoch_start = epoch_start_ms(next_epoch);
        let now = now_ms();
        if epoch_start > now {
            let wait = Duration::from_millis(epoch_start - now);
            tokio::time::sleep(wait).await;
        }

        // Skip if another process already produced this block (e.g. sync filled it in).
        if chain.store.has_block(next_epoch as u32) {
            last_produced = next_epoch;
            continue;
        }

        match produce_block(&chain, &account, next_epoch) {
            Ok(_) => {
                // Advance the shared epoch counter so other subsystems see progress.
                {
                    let mut current = chain.current_epoch.write();
                    if next_epoch > *current {
                        *current = next_epoch;
                    }
                }
                last_produced = next_epoch;
            }
            Err(e) => {
                warn!("block production failed (epoch {}): {}", next_epoch, e);
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn produce_block(chain: &Chain, miner: &str, epoch: u64) -> Result<Block> {
    let prev_epoch = epoch.saturating_sub(1);
    let prev_hash = chain.store.read_block(prev_epoch as u32)?
        .and_then(|b| Block::from_bytes(&b))
        .map(|b| b.header.hash())
        .unwrap_or([0u8; 32]);

    let miner_id = sha2::Sha256::digest(miner.as_bytes()).into();
    let reward = block_reward(epoch);

    let entries = vec![
        LedgerEntry::Mine {
            miner: miner.to_string(),
            epoch,
            work_value: compute_work(prev_hash, epoch),
            block_hash: String::new(), // filled after header creation
        },
        LedgerEntry::MineReward {
            miner: miner.to_string(),
            amount: reward,
            epoch,
        },
    ];

    // Apply entries to state
    chain.apply_block_entries(&entries);

    let entry_hashes: Vec<String> = entries.iter().map(|e| e.hash()).collect();
    let tx_root = btcpc_types::merkle_root(&entry_hashes);

    let mut header = BlockHeader::new(epoch as u32, prev_hash, miner_id);
    header.merkle_root_transactions = tx_root;

    let payload = serde_json::json!({
        "ledger_entries": entries,
        "rewards": [{ "miner": miner, "amount": reward, "epoch": epoch }],
        "compute_proofs": [],
    });

    let block = Block { header, payload };
    chain.store.write_block(epoch as u32, &block.to_bytes())?;

    info!("block {} mined: {} (reward {} dreams)", epoch, block.header.hash_hex(), reward);
    Ok(block)
}

fn compute_work(prev_hash: [u8; 32], epoch: u64) -> u64 {
    let mut h = Sha256::new();
    h.update(prev_hash);
    h.update(epoch.to_le_bytes());
    let result = h.finalize();
    // work_value = number of leading zero bits in the hash
    let mut leading_zeros = 0u64;
    for byte in result.iter() {
        if *byte == 0 {
            leading_zeros += 8;
        } else {
            leading_zeros += byte.leading_zeros() as u64;
            break;
        }
    }
    leading_zeros
}

fn epoch_start_ms(epoch: u64) -> u64 {
    epoch * EPOCH_MS
}

