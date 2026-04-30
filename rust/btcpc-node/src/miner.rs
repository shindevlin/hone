//! Inference mining loop — produces a block every epoch by running a local LLM.
//!
//! work_value = number of tokens generated during the epoch.
//! compute_proof = SHA-256 hex of the inference output text.
//!
//! If Ollama is unavailable for an epoch, the block is still produced with
//! work_value = 0 so the chain doesn't stall, but the miner earns less
//! reputation over time.

use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use sha2::{Sha256, Digest};
use tracing::{info, warn};
use btcpc_types::{
    Block, BlockHeader, LedgerEntry, block_reward_at,
    era, RECYCLE_ERA, RECYCLE_FUND_ACCOUNT, RECYCLE_REWARD_RATE, RECYCLE_REWARD_DENOM,
    NATIVE_TOKEN, EPOCH_MS,
};

use crate::chain::Chain;
use crate::utils::now_ms;

pub async fn run_miner(chain: Arc<Chain>, account: String, genesis_ts: u64) {
    info!("miner started: account={}", account);

    let now = now_ms();
    if genesis_ts > now {
        let wait = genesis_ts - now;
        info!("miner waiting {}s for genesis", wait / 1000);
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }

    let mut last_produced: u64 = chain.store.latest_epoch().unwrap_or(0);

    loop {
        let next_epoch = last_produced + 1;
        tokio::time::sleep(wait_for_next_epoch(next_epoch, genesis_ts)).await;

        if chain.store.has_block(next_epoch) {
            last_produced = next_epoch;
            continue;
        }

        // Run inference first — this is the proof of work.
        let prev_hash = chain.store.read_block(next_epoch.saturating_sub(1))
            .ok().flatten()
            .and_then(|b| Block::from_bytes(&b))
            .map(|b| b.header.hash())
            .unwrap_or([0u8; 32]);

        let (work_value, compute_proof) =
            run_inference(next_epoch, &account, prev_hash).await;

        match produce_block(&chain, &account, next_epoch, work_value, &compute_proof) {
            Ok(_) => {
                let mut cur = chain.current_epoch.write();
                if next_epoch > *cur { *cur = next_epoch; }
                last_produced = next_epoch;
            }
            Err(e) => warn!("block production failed (epoch {}): {}", next_epoch, e),
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ── Inference proof-of-work ───────────────────────────────────────────────────

async fn run_inference(epoch: u64, miner: &str, prev_hash: [u8; 32]) -> (u64, String) {
    let ollama_url = std::env::var("OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned());
    let model = std::env::var("BTCPC_MODEL")
        .unwrap_or_else(|_| "qwen2.5:0.5b".to_owned());

    // Epoch-seeded prompt — deterministic per (epoch, miner, chain state).
    let prompt = format!(
        "btcpc epoch {} miner {} prev {}",
        epoch, miner, &hex::encode(prev_hash)[..16]
    );

    let client = reqwest::Client::new();
    let result = client
        .post(format!("{}/api/generate", ollama_url))
        .timeout(Duration::from_secs(25))
        .json(&serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "options": { "num_predict": 64, "temperature": 0.0 },
        }))
        .send()
        .await;

    match result {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                let text   = body["response"].as_str().unwrap_or("");
                let tokens = body["eval_count"].as_u64().unwrap_or(0);
                let proof  = hex::encode(Sha256::digest(text.as_bytes()));
                info!("inference: epoch {} — {} tokens (model={})", epoch, tokens, model);
                return (tokens, proof);
            }
        }
        Ok(resp) => warn!("inference: Ollama {} for epoch {}", resp.status(), epoch),
        Err(e)   => warn!("inference: Ollama unreachable for epoch {}: {}", epoch, e),
    }

    (0, String::new())
}

// ── Block production ──────────────────────────────────────────────────────────

fn produce_block(
    chain:         &Chain,
    miner:         &str,
    epoch:         u64,
    work_value:    u64,
    compute_proof: &str,
) -> Result<Block> {
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

    let entries = vec![
        LedgerEntry::Mine {
            miner:      miner.to_owned(),
            epoch,
            work_value,
            block_hash: String::new(),
        },
        LedgerEntry::MineReward {
            miner:  miner.to_owned(),
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
        "rewards": [{ "miner": miner, "amount": reward, "epoch": epoch }],
        "compute_proofs": [{
            "worker":       miner,
            "output_hash":  compute_proof,
            "tokens":       work_value,
            "epoch":        epoch,
        }],
        "chain_id": chain.chain_id,
    });

    let block = Block { header, payload };
    chain.store.write_block(epoch, &block.to_bytes())?;

    info!("block {} mined: reward={} dreams, work={} tokens", epoch, reward, work_value);
    Ok(block)
}

fn wait_for_next_epoch(next_epoch: u64, genesis_ts: u64) -> Duration {
    let epoch_start_ms = genesis_ts + next_epoch * EPOCH_MS;
    let now = now_ms();
    if epoch_start_ms > now {
        Duration::from_millis(epoch_start_ms - now)
    } else {
        Duration::ZERO
    }
}
