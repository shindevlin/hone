//! Inference mining loop — submits a Mine entry every epoch by running a local LLM.
//!
//! work_value = number of tokens generated during the epoch.
//! compute_proof = SHA-256 hex of the inference output text.
//!
//! Miners no longer produce blocks or self-award MineReward. Clock nodes
//! emit MineReward entries pro-rata after epoch seal quorum is reached.

use std::sync::Arc;
use std::time::Duration;
use sha2::{Sha256, Digest};
use tracing::{info, warn};
use btcpc_types::{LedgerEntry, EPOCH_MS};

use crate::chain::Chain;
use crate::net::NetCmd;
use crate::utils::now_ms;

pub async fn run_miner(
    chain:   Arc<Chain>,
    account: String,
    genesis_ts: u64,
    cmd_tx:  tokio::sync::mpsc::Sender<NetCmd>,
) {
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

        // Run inference — this is the proof of work.
        let (work_value, compute_proof) =
            run_inference(next_epoch, &account).await;

        // Build Mine entry and apply locally.
        let entry = LedgerEntry::Mine {
            miner:      account.clone(),
            epoch:      next_epoch,
            work_value,
            block_hash: compute_proof.clone(),
        };
        if let Err(e) = chain.apply_entry(&entry) {
            warn!("miner: apply Mine failed (epoch {}): {}", next_epoch, e);
        }

        // Broadcast Mine entry as gossip — clock nodes will emit MineReward.
        let envelope = serde_json::json!({"entry": entry});
        if let Ok(data) = serde_json::to_vec(&envelope) {
            let _ = cmd_tx.send(NetCmd::Broadcast {
                topic: "btcpc/entries",
                data,
            }).await;
        }

        {
            let mut cur = chain.current_epoch.write();
            if next_epoch > *cur { *cur = next_epoch; }
        }
        last_produced = next_epoch;

        info!("miner: submitted Mine entry epoch {} work={} tokens", next_epoch, work_value);

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ── Inference proof-of-work ───────────────────────────────────────────────────

async fn run_inference(epoch: u64, miner: &str) -> (u64, String) {
    let ollama_url = std::env::var("OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned());
    let model = std::env::var("BTCPC_MODEL")
        .unwrap_or_else(|_| "qwen2.5:0.5b".to_owned());

    let prompt = format!("btcpc epoch {} miner {}", epoch, miner);

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

fn wait_for_next_epoch(next_epoch: u64, genesis_ts: u64) -> Duration {
    let epoch_start_ms = genesis_ts + next_epoch * EPOCH_MS;
    let now = now_ms();
    if epoch_start_ms > now {
        Duration::from_millis(epoch_start_ms - now)
    } else {
        Duration::ZERO
    }
}
