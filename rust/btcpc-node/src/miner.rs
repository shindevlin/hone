//! Mining loop — produces a block every epoch.

use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use sha2::{Sha256, Digest};
use tracing::{info, warn};
use btcpc_types::{
    Block, BlockHeader, LedgerEntry, block_reward_at, epoch_duration_ms,
    era, RECYCLE_ERA, RECYCLE_FUND_ACCOUNT, RECYCLE_REWARD_RATE, RECYCLE_REWARD_DENOM, EPOCH_MS,
};

use crate::chain::Chain;
use crate::utils::now_ms;

/// Block reward for `epoch` — delegates to the emission schedule.
/// Re-exported here for call-site convenience; the canonical implementation
/// lives in `btcpc_types::emission::block_reward_at`.
pub fn block_reward(epoch: u64) -> u64 {
    block_reward_at(epoch)
}

pub async fn run_miner(chain: Arc<Chain>, account: String, genesis_ts: u64) {
    info!("miner started: account={}", account);

    // Wait until genesis time before producing any blocks.
    let now = now_ms();
    if genesis_ts > now {
        let wait = genesis_ts - now;
        info!("miner waiting {}s for genesis", wait / 1000);
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }

    // Base on actual stored blocks, not the in-memory epoch counter (which only
    // advances via EpochSeal).  This prevents the inflation bug where current_epoch
    // stays 0 and the miner keeps overwriting epoch 1.
    let mut last_produced: u64 = chain.store.latest_epoch().unwrap_or(0);

    loop {
        let next_epoch = last_produced + 1;

        // Sleep until the wall-clock boundary for next_epoch.
        tokio::time::sleep(wait_for_next_epoch(next_epoch, genesis_ts)).await;

        // Skip if another process already produced this block (e.g. sync filled it in).
        if chain.store.has_block(next_epoch) {
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
    let prev_hash = chain.store.read_block(prev_epoch)?
        .and_then(|b| Block::from_bytes(&b))
        .map(|b| b.header.hash())
        .unwrap_or([0u8; 32]);

    let miner_id = sha2::Sha256::digest(miner.as_bytes()).into();

    // Era 5+: pay miner from recycle fund (RECYCLE_REWARD_RATE/RECYCLE_REWARD_DENOM × balance).
    let reward = if era(epoch) >= RECYCLE_ERA {
        let fund_balance = chain.store.get_balance(RECYCLE_FUND_ACCOUNT, btcpc_types::NATIVE_TOKEN);
        ((fund_balance as u128 * RECYCLE_REWARD_RATE) / RECYCLE_REWARD_DENOM) as u64
    } else {
        block_reward(epoch)
    };

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
        "chain_id": chain.chain_id,
    });

    let block = Block { header, payload };
    chain.store.write_block(epoch, &block.to_bytes())?;

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

/// Sleep duration from now until the wall-clock start of `next_epoch`.
/// Epochs are numbered relative to genesis_ts; each lasts epoch_duration_ms(epoch).
fn wait_for_next_epoch(next_epoch: u64, genesis_ts: u64) -> Duration {
    // For simplicity use era-0 epoch duration for timing. Halving eras are very
    // long (years), so this is accurate for any foreseeable use.
    let epoch_start_ms = genesis_ts + next_epoch * EPOCH_MS;
    let now = now_ms();
    if epoch_start_ms > now {
        Duration::from_millis(epoch_start_ms - now)
    } else {
        Duration::ZERO
    }
}

