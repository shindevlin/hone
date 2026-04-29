//! Mining loop — produces a block every epoch.

use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use sha2::{Sha256, Digest};
use tracing::{info, warn};
use btcpc_types::{Block, BlockHeader, LedgerEntry, DREAMS_PER_BTCPC, EPOCH_MS};

use crate::chain::Chain;

/// Block reward: 50 BTCPC per block (halving every 210_000 epochs like Bitcoin).
pub fn block_reward(epoch: u64) -> u64 {
    let halvings = epoch / 210_000;
    if halvings >= 64 { return 0; }
    (50 * DREAMS_PER_BTCPC) >> halvings
}

pub async fn run_miner(chain: Arc<Chain>, account: String) {
    info!("miner started: account={}", account);
    loop {
        let epoch = chain.current_epoch();
        let next_epoch = epoch + 1;

        // Wait until the next epoch boundary
        let epoch_start = epoch_start_ms(next_epoch);
        let now_ms = now_ms();
        if epoch_start > now_ms {
            let wait = Duration::from_millis(epoch_start - now_ms);
            tokio::time::sleep(wait).await;
        }

        if let Err(e) = produce_block(&chain, &account, next_epoch) {
            warn!("block production failed (epoch {}): {}", next_epoch, e);
        }

        // Small sleep to avoid tight loop on clock skew
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

    info!("block {} mined: {} (reward {} sat)", epoch, block.header.hash_hex(), reward);
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
