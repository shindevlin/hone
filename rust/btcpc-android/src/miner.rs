//! Block miner for the Android micronode.
//! Writes blocks directly to the local sled store and broadcasts via gossipsub.

use std::sync::Arc;
use std::time::Duration;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use btcpc_types::{
    Block, BlockHeader, LedgerEntry, block_reward_at, epoch_duration_ms,
    era, RECYCLE_ERA, RECYCLE_FUND_ACCOUNT, RECYCLE_REWARD_RATE, RECYCLE_REWARD_DENOM,
    NATIVE_TOKEN, EPOCH_MS,
};

use crate::chain::Chain;
use crate::net::NetCmd;

pub async fn run_miner(
    chain:      Arc<Chain>,
    account:    String,
    genesis_ts: u64,
    cmd_tx:     tokio::sync::mpsc::Sender<NetCmd>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) {
    info!("miner: started, account={}", account);

    // Wait for genesis.
    let now = now_ms();
    if genesis_ts > now {
        let wait = genesis_ts - now;
        info!("miner: waiting {}s for genesis", wait / 1000);
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }

    let mut last_produced = chain.store.latest_epoch().unwrap_or(0);

    loop {
        let next_epoch = last_produced + 1;

        tokio::select! {
            _ = shutdown.recv() => break,
            _ = sleep_until_epoch(next_epoch, genesis_ts) => {}
        }

        if chain.store.has_block(next_epoch) {
            last_produced = next_epoch;
            continue;
        }

        match produce_block(&chain, &account, next_epoch) {
            Ok(block) => {
                // Broadcast block to peers.
                let mut payload = next_epoch.to_be_bytes().to_vec();
                payload.extend_from_slice(&block.to_bytes());
                let _ = cmd_tx.send(NetCmd::Broadcast {
                    topic: "btcpc/blocks",
                    data: payload,
                }).await;

                let mut cur = chain.current_epoch.write();
                if next_epoch > *cur { *cur = next_epoch; }
                last_produced = next_epoch;
            }
            Err(e) => warn!("miner: block {} failed: {}", next_epoch, e),
        }
    }

    info!("miner: stopped");
}

fn produce_block(chain: &Chain, miner: &str, epoch: u64) -> anyhow::Result<Block> {
    let prev_hash = chain.store.read_block(epoch.saturating_sub(1))?
        .and_then(|b| Block::from_bytes(&b))
        .map(|b| b.header.hash())
        .unwrap_or([0u8; 32]);

    let miner_id: [u8; 32] = Sha256::digest(miner.as_bytes()).into();

    let reward = if era(epoch) >= RECYCLE_ERA {
        let fund = chain.store.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN);
        ((fund as u128 * RECYCLE_REWARD_RATE) / RECYCLE_REWARD_DENOM) as u64
    } else {
        block_reward_at(epoch)
    };

    let work_value = compute_work(prev_hash, epoch);

    let entries = vec![
        LedgerEntry::Mine {
            miner: miner.to_owned(),
            epoch,
            work_value,
            block_hash: String::new(),
        },
        LedgerEntry::MineReward {
            miner: miner.to_owned(),
            amount: reward,
            epoch,
        },
    ];

    chain.apply_block_entries(&entries);

    let entry_hashes: Vec<String> = entries.iter().map(|e| e.hash()).collect();
    let tx_root = btcpc_types::merkle_root(&entry_hashes);

    let mut header = BlockHeader::new(epoch as u32, prev_hash, miner_id);
    header.merkle_root_transactions = tx_root;

    let payload = serde_json::json!({
        "ledger_entries": entries,
        "chain_id": chain.chain_id,
    });

    let block = Block { header, payload };
    chain.store.write_block(epoch, &block.to_bytes())?;

    info!("miner: block {} mined (reward {} dreams)", epoch, reward);
    Ok(block)
}

fn compute_work(prev_hash: [u8; 32], epoch: u64) -> u64 {
    let mut h = Sha256::new();
    h.update(prev_hash);
    h.update(epoch.to_le_bytes());
    let result = h.finalize();
    let mut zeros = 0u64;
    for byte in result.iter() {
        if *byte == 0 { zeros += 8; }
        else { zeros += byte.leading_zeros() as u64; break; }
    }
    zeros
}

async fn sleep_until_epoch(epoch: u64, genesis_ts: u64) {
    let start_ms = genesis_ts + epoch * EPOCH_MS;
    let now = now_ms();
    if start_ms > now {
        tokio::time::sleep(Duration::from_millis(start_ms - now)).await;
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
