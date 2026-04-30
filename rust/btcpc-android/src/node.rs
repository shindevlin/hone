//! Full standalone micronode — wires together store, chain, clock, miner,
//! inference worker, and P2P swarm with no dependency on any remote API.

use std::sync::Arc;
use std::time::Duration;
use parking_lot::Mutex as PLMutex;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

use btcpc_types::{LedgerEntry, NATIVE_TOKEN, TESTNET_CHAIN_ID};

use crate::chain::Chain;
use crate::clock::{self, ClockConfig, ClockConsensus};
use crate::llm::LlmEngine;
use crate::miner;
use crate::net::{self, NetCmd, NetworkEvent, SwarmConfig};
use crate::store::Store;

pub struct NodeConfig {
    pub account:         String,
    pub posting_key:     String,
    pub chain_id:        String,
    pub genesis_ts:      u64,
    pub data_dir:        String,
    pub model_id:        String,
    pub model_dir:       String,
    pub bootstrap_peers: Vec<String>,
    pub p2p_port:        u16,
    pub is_miner:        bool,
    pub is_clock:        bool,
}

/// Handle held while the node is running.
pub struct NodeHandle {
    pub chain:       Arc<Chain>,
    pub status:      Arc<PLMutex<String>>,
    pub cmd_tx:      mpsc::Sender<NetCmd>,
    pub shutdown_tx: broadcast::Sender<()>,
}

pub async fn start(cfg: NodeConfig, status: Arc<PLMutex<String>>) -> anyhow::Result<NodeHandle> {
    std::fs::create_dir_all(&cfg.data_dir)?;

    *status.lock() = "Opening store…".to_owned();
    let store = Store::open(&format!("{}/state", cfg.data_dir))?;
    let chain = Arc::new(Chain::new(store, cfg.account.clone(), cfg.chain_id.clone()));

    // Genesis: ensure we have a starting balance entry if the store is fresh.
    init_genesis(&chain, &cfg);

    let (shutdown_tx, _) = broadcast::channel::<()>(4);
    let (cmd_tx, cmd_rx) = mpsc::channel::<NetCmd>(256);
    let (event_tx, mut event_rx) = mpsc::channel::<NetworkEvent>(256);

    // ── P2P swarm ─────────────────────────────────────────────────────────────
    {
        let swarm_cfg = SwarmConfig {
            chain_id:        cfg.chain_id.clone(),
            data_dir:        cfg.data_dir.clone(),
            p2p_port:        cfg.p2p_port,
            bootstrap_peers: cfg.bootstrap_peers.clone(),
        };
        let sd = shutdown_tx.subscribe();
        tokio::spawn(async move {
            if let Err(e) = net::run_swarm(swarm_cfg, event_tx, cmd_rx, sd).await {
                warn!("swarm error: {}", e);
            }
        });
    }

    // ── Clock consensus ───────────────────────────────────────────────────────
    let (clock_arc, _) = ClockConsensus::new();
    {
        let clock_cfg = ClockConfig {
            node_id:    format!("{}-android", cfg.account),
            chain_id:   cfg.chain_id.clone(),
            genesis_ts: cfg.genesis_ts,
            is_clock:   cfg.is_clock,
            quorum:     std::env::var("BTCPC_CLOCK_QUORUM")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(2),
        };
        let sd      = shutdown_tx.subscribe();
        let cmd     = cmd_tx.clone();
        let ch      = chain.clone();
        let clk     = clock_arc.clone();
        tokio::spawn(async move {
            clock::run(clock_cfg, clk, cmd, ch, sd).await;
        });
    }

    // ── Shared LLM engine (miner + inference worker both use it) ─────────────
    let llm = Arc::new(tokio::sync::Mutex::new(
        LlmEngine::new(&cfg.model_dir, status.clone())
    ));

    // ── Block miner (inference-based) ─────────────────────────────────────────
    if cfg.is_miner {
        let ch    = chain.clone();
        let acct  = cfg.account.clone();
        let gen   = cfg.genesis_ts;
        let cmd   = cmd_tx.clone();
        let sd    = shutdown_tx.subscribe();
        let llm_r = llm.clone();
        tokio::spawn(async move {
            miner::run_miner(ch, acct, gen, llm_r, cmd, sd).await;
        });
    }

    // ── Inference job worker ──────────────────────────────────────────────────
    {
        let acct    = cfg.account.clone();
        let chain_r = chain.clone();
        let cmd     = cmd_tx.clone();
        let gen_ts  = cfg.genesis_ts;
        let sd      = shutdown_tx.subscribe();
        let status2 = status.clone();
        let llm_r   = llm.clone();
        tokio::spawn(async move {
            run_inference_worker(llm_r, acct, chain_r, cmd, gen_ts, sd, status2).await;
        });
    }

    // ── Inference verifier ────────────────────────────────────────────────────
    {
        let acct    = cfg.account.clone();
        let chain_r = chain.clone();
        let cmd     = cmd_tx.clone();
        let sd      = shutdown_tx.subscribe();
        let llm_r   = llm.clone();
        tokio::spawn(async move {
            run_verifier(llm_r, acct, chain_r, cmd, sd).await;
        });
    }

    // ── Network event handler ─────────────────────────────────────────────────
    {
        let ch      = chain.clone();
        let clk     = clock_arc.clone();
        let mut sd  = shutdown_tx.subscribe();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = sd.recv() => break,
                    Some(ev) = event_rx.recv() => handle_net_event(ev, &ch, &clk),
                }
            }
        });
    }

    // ── Status updater ────────────────────────────────────────────────────────
    {
        let ch      = chain.clone();
        let acct    = cfg.account.clone();
        let st      = status.clone();
        let mut sd  = shutdown_tx.subscribe();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = sd.recv() => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
                let epoch = ch.current_epoch();
                let bal   = ch.store.get_balance(&acct, NATIVE_TOKEN);
                let btcpc = bal as f64 / 10_000_000_000.0;
                *st.lock() = format!("epoch {} | {:.4} BTCPC | {}", epoch, btcpc, acct);
            }
        });
    }

    *status.lock() = "Node running".to_owned();
    info!("micronode started: account={} chain={}", cfg.account, cfg.chain_id);

    Ok(NodeHandle {
        chain,
        status,
        cmd_tx,
        shutdown_tx,
    })
}

// ── Genesis ───────────────────────────────────────────────────────────────────

fn init_genesis(chain: &Chain, cfg: &NodeConfig) {
    // If we already have blocks, genesis was already applied.
    if chain.store.latest_epoch().is_some() { return; }
    // Set genesis timestamp in meta so the miner/clock can reference it.
    let _ = chain.store.set_meta("genesis_ts", &cfg.genesis_ts.to_be_bytes());
}

// ── Network event handler ─────────────────────────────────────────────────────

fn handle_net_event(ev: NetworkEvent, chain: &Chain, clock: &ClockConsensus) {
    match ev {
        NetworkEvent::EpochSeal { seal } => {
            clock.receive_seal(seal);
        }
        NetworkEvent::Block { .. } => {
            // Miners no longer produce blocks — this event is no-op.
        }
        NetworkEvent::Entry { entry } => {
            // Cache plaintext from the gossip envelope so the verifier can assess quality.
            if let Some(job_id) = entry.pointer("/entry/InferenceJobPost/job_id")
                .and_then(|v| v.as_str())
            {
                if let Some(t) = entry.get("input_text").and_then(|v| v.as_str()) {
                    let _ = chain.store.set_meta(&format!("infer_input:{}", job_id), t.as_bytes());
                }
            }
            if let Some(job_id) = entry.pointer("/entry/InferenceJobComplete/job_id")
                .and_then(|v| v.as_str())
            {
                if let Some(t) = entry.get("output_text").and_then(|v| v.as_str()) {
                    let _ = chain.store.set_meta(&format!("infer_output:{}", job_id), t.as_bytes());
                }
            }

            let entry_val = entry.get("entry").cloned().unwrap_or(entry);
            if let Ok(e) = serde_json::from_value::<LedgerEntry>(entry_val) {
                let _ = chain.apply_entry(&e);
            }
        }
        _ => {}
    }
}

// ── Inference worker ──────────────────────────────────────────────────────────

async fn run_inference_worker(
    llm: Arc<tokio::sync::Mutex<LlmEngine>>,
    account: String,
    chain: Arc<Chain>,
    cmd_tx: mpsc::Sender<NetCmd>,
    genesis_ts: u64,
    mut shutdown: broadcast::Receiver<()>,
    status: Arc<PLMutex<String>>,
) {
    // Wait for genesis before starting.
    let wait = genesis_ts.saturating_sub(now_ms());
    if wait > 0 {
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }

    loop {
        tokio::select! {
            _ = shutdown.recv() => break,
            _ = tokio::time::sleep(Duration::from_secs(20)) => {}
        }

        // Make sure model is present.
        if !llm.lock().await.ensure_ready().await { continue; }

        // Look for an awarded inference job we should complete.
        let epoch = chain.current_epoch();
        let jobs = chain.store.scan_prefix("infer_job:");
        for (_, val) in jobs {
            let job: serde_json::Value = match serde_json::from_slice(&val) {
                Ok(v) => v, Err(_) => continue,
            };
            let winner  = job["winner"].as_str().unwrap_or("");
            let status_ = job["status"].as_str().unwrap_or("");
            if winner != account || status_ != "awarded" { continue; }

            let job_id = match job["job_id"].as_str() {
                Some(id) => id.to_owned(), None => continue,
            };
            let prompt = job["input_hash"].as_str().unwrap_or("hello").to_owned();

            *status.lock() = format!("Running inference job {}…", &job_id[..8]);

            match llm.lock().await.generate(&prompt, 256).await {
                Ok(output) => {
                    use sha2::{Digest, Sha256};
                    let result_hash = hex::encode(Sha256::digest(output.as_bytes()));
                    // Store output so verifiers on this node can assess quality.
                    let _ = chain.store.set_meta(
                        &format!("infer_output:{}", job_id), output.as_bytes());
                    let entry = LedgerEntry::InferenceJobComplete {
                        job_id:      job_id.clone(),
                        worker:      account.clone(),
                        result_hash: result_hash.clone(),
                        latency_ms:  0,
                        epoch,
                        signed_by:   account.clone(),
                    };
                    let _ = chain.apply_entry(&entry);
                    // Include output_text in gossip so remote verifiers can assess.
                    let envelope = serde_json::json!({
                        "entry": entry,
                        "output_text": output,
                    });
                    if let Ok(data) = serde_json::to_vec(&envelope) {
                        let _ = cmd_tx.send(NetCmd::Broadcast {
                            topic: "btcpc/entries", data,
                        }).await;
                    }
                    tracing::info!("inference: job {} completed", job_id);
                }
                Err(e) => warn!("inference: job {} failed: {}", job_id, e),
            }
        }
    }
}

// ── Inference verifier ────────────────────────────────────────────────────────
//
// Receives the original request and the submitted work, then asks the on-device
// model to judge whether the task was adequately completed.
// Verdicts: "approved" | "rejected" | "review_required"
// - approved      → payment flows
// - rejected      → no payment (clear failure)
// - review_required → opens claim window; only requester or worker may claim

async fn run_verifier(
    llm:      Arc<tokio::sync::Mutex<LlmEngine>>,
    account:  String,
    chain:    Arc<Chain>,
    cmd_tx:   mpsc::Sender<NetCmd>,
    mut shutdown: broadcast::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = shutdown.recv() => break,
            _ = tokio::time::sleep(Duration::from_secs(60)) => {}
        }

        if !llm.lock().await.ensure_ready().await { continue; }

        let epoch = chain.current_epoch();
        let jobs = chain.store.scan_prefix("infer_job:");

        for (_, val) in jobs {
            let job: serde_json::Value = match serde_json::from_slice(&val) {
                Ok(v) => v, Err(_) => continue,
            };

            let status_ = job["status"].as_str().unwrap_or("");
            let worker   = job["winner"].as_str().unwrap_or("");
            let model    = job["model"].as_str().unwrap_or("");

            if worker == account || status_ != "complete" { continue; }
            if !model.is_empty() && !model.contains("qwen2.5-0.5b") { continue; }

            let job_id = match job["job_id"].as_str() { Some(s) => s.to_owned(), None => continue };

            let verdict_key = format!("infer_verdict:{}:{}", job_id, account);
            if chain.store.get_meta(&verdict_key).is_some() { continue; }

            // Need actual request and output text to assess quality.
            let input_text = match chain.store.get_meta(&format!("infer_input:{}", job_id))
                .and_then(|b| String::from_utf8(b).ok())
            {
                Some(t) => t,
                None => continue,
            };
            let output_text = match chain.store.get_meta(&format!("infer_output:{}", job_id))
                .and_then(|b| String::from_utf8(b).ok())
            {
                Some(t) => t,
                None => continue,
            };

            let meta_prompt = format!(
                "Task: {}\nResponse: {}\n\n\
                Did the AI assistant adequately complete the task?\n\
                Reply with exactly one word: APPROVED, REJECTED, or REVIEW.",
                input_text.chars().take(512).collect::<String>(),
                output_text.chars().take(512).collect::<String>(),
            );

            match llm.lock().await.generate(&meta_prompt, 20).await {
                Ok(verdict_text) => {
                    let upper = verdict_text.to_uppercase();
                    let verdict = if upper.contains("APPROVED") {
                        "approved"
                    } else if upper.contains("REJECTED") || upper.contains("REJECT") {
                        "rejected"
                    } else {
                        "review_required"
                    };

                    let entry = LedgerEntry::InferenceJobVerify {
                        job_id:    job_id.clone(),
                        verifier:  account.clone(),
                        verdict:   verdict.to_owned(),
                        reason:    None,
                        epoch,
                        signed_by: account.clone(),
                    };
                    let _ = chain.apply_entry(&entry);
                    let envelope = serde_json::json!({ "entry": entry });
                    if let Ok(data) = serde_json::to_vec(&envelope) {
                        let _ = cmd_tx.send(NetCmd::Broadcast {
                            topic: "btcpc/entries", data,
                        }).await;
                    }
                    let _ = chain.store.set_meta(&verdict_key, verdict.as_bytes());
                    tracing::info!("verifier: job {} → {}", job_id, verdict);
                }
                Err(e) => warn!("verifier: meta-eval failed for job {}: {}", job_id, e),
            }

            // One verification per cycle — don't starve the miner.
            break;
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
