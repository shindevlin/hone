//! Inference mining loop — submits a Mine entry every epoch by running a local LLM.
//!
//! work_value = number of tokens generated during the epoch.
//! output_hash = SHA-256 hex of the inference output text (binding commitment, not a reproducibility proof).
//!
//! Miners no longer produce blocks or self-award MineReward. Clock nodes
//! emit MineReward entries pro-rata after epoch seal quorum is reached.
//!
//! Job linkage (T2-1): each epoch the miner first checks for an awarded inference
//! job. If one is found, it runs inference on the job prompt, submits
//! InferenceJobComplete, then submits Mine { job_id } for full reward. Without a
//! real job the Mine is unlinked and earns only the grace-period rate (20%).
#![allow(dead_code)]

#![allow(dead_code)]
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
    software_hash: String,
) {
    info!("miner started: account={}", account);

    let now = now_ms();
    if genesis_ts > now {
        let wait = genesis_ts - now;
        info!("miner waiting {}s for genesis", wait / 1000);
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }

    // Start from the current time-based epoch so Mine entries land where the
    // clock seals, not sequentially from 0 after a state wipe.
    let elapsed = now_ms().saturating_sub(genesis_ts);
    let time_epoch = elapsed / EPOCH_MS;
    let mut last_produced: u64 = time_epoch.saturating_sub(1);

    loop {
        let next_epoch = last_produced + 1;
        tokio::time::sleep(wait_for_next_epoch(next_epoch, genesis_ts)).await;

        let ollama_url = std::env::var("OLLAMA_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_owned());
        let model_id = match detect_model(&ollama_url).await {
            Some(m) => m,
            None => {
                warn!("miner: no Ollama model available at {} — skipping epoch {}", ollama_url, next_epoch);
                last_produced = next_epoch;
                continue;
            }
        };
        let hw_tier_val = std::env::var("BTCPC_HW_TIER")
            .ok().and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(1);
        let model_hash = fetch_model_digest(&ollama_url, &model_id).await;
        let start_ms = now_ms();

        // Check for an awarded inference job. Jobs in the "awarded" state are ready
        // for the miner to work on; "completed" means we already submitted a result.
        let pending_job = find_awarded_job(&chain, &account);

        let (input_tokens, output_tokens, output_hash, linked_job_id) =
            if let Some(ref jid) = pending_job {
                // Run inference for the real job. The prompt is derived from job_id + input_hash
                // so it's deterministic (actual off-chain data is fetched in future work, D1).
                let prompt = job_prompt(&chain, jid);
                let (inp, out, hash) = run_inference_prompt(next_epoch, &account, &prompt).await;
                info!("miner: running awarded job '{}' (epoch {})", jid, next_epoch);
                (inp, out, hash, Some(jid.clone()))
            } else {
                // No real job — run benchmark inference. Earns grace-period rate only.
                let (inp, out, hash) = run_inference(next_epoch, &account).await;
                (inp, out, hash, None)
            };

        let latency_ms = now_ms().saturating_sub(start_ms);

        // Slow models (e.g. 26B params) may take longer than one epoch (30 s).
        // Use whichever epoch is current AT SUBMISSION TIME so the mine record
        // lands in the right reward scan bucket — not the epoch we started targeting.
        let elapsed_at_submit = now_ms().saturating_sub(genesis_ts);
        let submit_epoch = (elapsed_at_submit / EPOCH_MS).max(next_epoch);

        // If we have a linked job, submit InferenceJobComplete before Mine.
        if let Some(ref jid) = linked_job_id {
            let complete = LedgerEntry::InferenceJobComplete {
                job_id:      jid.clone(),
                worker:      account.clone(),
                result_hash: output_hash.clone(),
                latency_ms,
                epoch:       submit_epoch,
                signed_by:   account.clone(),
            };
            if let Err(e) = chain.apply_entry(&complete) {
                warn!("miner: InferenceJobComplete failed for job '{}': {}", jid, e);
            } else {
                broadcast(&cmd_tx, &complete).await;
                info!("miner: submitted InferenceJobComplete for job '{}' (epoch {})", jid, submit_epoch);
            }
        }

        // Build and submit Mine entry.
        let entry = LedgerEntry::Mine {
            miner:         account.clone(),
            epoch:         submit_epoch,
            model:         model_id,
            input_tokens,
            output_tokens,
            tool_calls:    0,
            hw_tier:       hw_tier_val,
            output_hash:   output_hash.clone(),
            job_id:        linked_job_id,
            node_version:  Some(env!("CARGO_PKG_VERSION").to_owned()),
            software_hash: if software_hash.is_empty() { None } else { Some(software_hash.clone()) },
            model_hash,
        };
        if let Err(e) = chain.apply_entry(&entry) {
            warn!("miner: apply Mine failed (epoch {}): {}", submit_epoch, e);
        } else {
            broadcast(&cmd_tx, &entry).await;
        }

        {
            let mut cur = chain.current_epoch.write();
            if submit_epoch > *cur { *cur = submit_epoch; }
        }
        last_produced = submit_epoch;

        info!("miner: submitted Mine entry epoch {} out={} in={} tokens", submit_epoch, output_tokens, input_tokens);

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ── Job discovery ─────────────────────────────────────────────────────────────

/// Return the first job_id this miner has been awarded but not yet completed.
fn find_awarded_job(chain: &Chain, miner: &str) -> Option<String> {
    let prefix = format!("mine_queue:{}:", miner);
    chain.store.state_scan_prefix(&prefix)
        .into_iter()
        .find_map(|(k, v)| {
            if v.as_slice() == b"awarded" {
                k.strip_prefix(&prefix).map(|jid| jid.to_owned())
            } else {
                None
            }
        })
}

/// Build a deterministic prompt for a job. Falls back to job_id hash until
/// off-chain input fetch (D1/btcpc-fs) is implemented.
fn job_prompt(chain: &Chain, job_id: &str) -> String {
    let input_hash = crate::inference::get_job(chain, job_id)
        .and_then(|j| j.result_hash.or(Some(j.input_hash)))
        .unwrap_or_else(|| job_id.to_owned());
    format!("btcpc inference job {} input {}", job_id, input_hash)
}

// ── Inference proof-of-work ───────────────────────────────────────────────────

async fn run_inference(epoch: u64, miner: &str) -> (u64, u64, String) {
    let prompt = format!("btcpc epoch {} miner {}", epoch, miner);
    run_inference_prompt(epoch, miner, &prompt).await
}

/// Returns whatever model Ollama has installed. Prefers BTCPC_MODEL env var;
/// falls back to the first model reported by /api/tags.
pub async fn detect_model(ollama_url: &str) -> Option<String> {
    if let Ok(m) = std::env::var("BTCPC_MODEL") {
        if !m.trim().is_empty() { return Some(m.trim().to_owned()); }
    }
    let client = reqwest::Client::new();
    if let Ok(resp) = client
        .get(format!("{}/api/tags", ollama_url))
        .timeout(Duration::from_secs(5))
        .send().await
    {
        if let Ok(body) = resp.json::<serde_json::Value>().await {
            if let Some(name) = body["models"].as_array()
                .and_then(|a| a.first())
                .and_then(|m| m["name"].as_str().or(m["model"].as_str()))
            {
                return Some(name.to_owned());
            }
        }
    }
    None
}

/// Fetch the SHA-256 digest Ollama assigned to a model via /api/show.
/// Returns the digest string (e.g. "sha256:abc123...") or None.
pub async fn fetch_model_digest(ollama_url: &str, model: &str) -> Option<String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/show", ollama_url))
        .timeout(Duration::from_secs(10))
        .json(&serde_json::json!({ "name": model }))
        .send().await.ok()?;
    let body: serde_json::Value = resp.json().await.ok()?;
    body["digest"].as_str().map(str::to_owned)
}

async fn run_inference_prompt(epoch: u64, miner: &str, prompt: &str) -> (u64, u64, String) {
    let ollama_url = std::env::var("OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned());
    let model = match detect_model(&ollama_url).await {
        Some(m) => m,
        None => {
            warn!("miner: no Ollama model available — skipping epoch {} inference", epoch);
            return (0, 0, String::new());
        }
    };

    let client = reqwest::Client::new();
    let result = client
        .post(format!("{}/api/generate", ollama_url))
        .timeout(Duration::from_secs(60))
        .json(&serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "keep_alive": "0s",
            "options": { "num_predict": 64, "temperature": 0.0 },
        }))
        .send()
        .await;

    match result {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                let text         = body["response"].as_str().unwrap_or("");
                let output_toks  = body["eval_count"].as_u64().unwrap_or(0);
                let input_toks   = body["prompt_eval_count"].as_u64().unwrap_or(0);
                let proof        = hex::encode(Sha256::digest(text.as_bytes()));
                info!("inference: epoch {} miner {} — in={} out={} tokens (model={})", epoch, miner, input_toks, output_toks, model);
                return (input_toks, output_toks, proof);
            }
        }
        Ok(resp) => warn!("inference: Ollama {} for epoch {}", resp.status(), epoch),
        Err(e)   => warn!("inference: Ollama unreachable for epoch {}: {}", epoch, e),
    }

    (0, 0, String::new())
}

fn model_and_hw() -> (String, u8) {
    let model = std::env::var("BTCPC_MODEL")
        .unwrap_or_else(|_| "qwen2.5:0.5b".to_owned());
    let hw_tier = std::env::var("BTCPC_HW_TIER")
        .ok().and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(1); // default: cpu-only desktop
    (model, hw_tier)
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

async fn broadcast(cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>, entry: &LedgerEntry) {
    let envelope = serde_json::json!({"entry": entry});
    if let Ok(data) = serde_json::to_vec(&envelope) {
        let _ = cmd_tx.send(NetCmd::Broadcast {
            topic: "btcpc/entries",
            data,
        }).await;
    }
}
