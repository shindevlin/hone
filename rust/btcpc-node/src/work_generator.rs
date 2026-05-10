//! Synthetic work generator.
//!
//! When BTCPC_WORK_GENERATOR=true, self-posts InferenceJobPost entries each epoch
//! to keep the inference marketplace alive with real demand. Useful on small testnets
//! where organic demand hasn't formed yet.

use std::sync::Arc;
use std::time::Duration;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use btcpc_types::{LedgerEntry, NATIVE_TOKEN, EPOCH_MS};
use crate::chain::Chain;
use crate::net::NetCmd;
use crate::utils::now_ms;

const PROMPT_POOL: &[&str] = &[
    "Explain quantum entanglement in one sentence.",
    "What is the most efficient sorting algorithm and why?",
    "Describe the role of mitochondria in cellular respiration.",
    "How does the Bitcoin proof-of-work consensus mechanism work?",
    "What is the difference between supervised and unsupervised learning?",
    "Explain the CAP theorem in distributed systems.",
    "What causes the northern lights?",
    "How does HTTPS protect data in transit?",
    "What is the difference between TCP and UDP?",
    "Explain gradient descent in neural network training.",
    "What is a Merkle tree and why is it useful in blockchains?",
    "How does RSA encryption work at a high level?",
    "What is the significance of Shannon entropy?",
    "Describe the halting problem and its implications.",
    "What is zero-knowledge proof and where is it used?",
];

const JOB_FEE_DREAMS: u64 = 5_000; // 0.05 BTCPC per synthetic job

pub async fn run(
    chain: Arc<Chain>,
    account: String,
    genesis_ts: u64,
    cmd_tx: tokio::sync::mpsc::Sender<NetCmd>,
    signing_key: Arc<Option<ed25519_dalek::SigningKey>>,
) {
    info!("[work-gen] started for account={}", account);
    let mut last_epoch = 0u64;
    let mut prompt_idx = 0usize;

    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;

        let epoch = (now_ms().saturating_sub(genesis_ts)) / EPOCH_MS;
        if epoch <= last_epoch { continue; }
        last_epoch = epoch;

        // Check balance — skip if we'd go negative.
        if chain.get_balance(&account, NATIVE_TOKEN) < JOB_FEE_DREAMS {
            warn!("[work-gen] insufficient balance to post job, skipping epoch {}", epoch);
            continue;
        }

        let prompt = PROMPT_POOL[prompt_idx % PROMPT_POOL.len()];
        prompt_idx += 1;

        let job_id = hex::encode(Sha256::digest(
            format!("workgen:{}:{}:{}", account, epoch, prompt_idx).as_bytes()
        ));

        let input_hash = hex::encode(Sha256::digest(prompt.as_bytes()));
        let model = std::env::var("BTCPC_MODEL").unwrap_or_else(|_| "qwen2.5:0.5b".to_owned());

        let nonce: u64 = chain.store.state_get(&format!("nonce:{}", account))
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(0);

        let entry = LedgerEntry::InferenceJobPost {
            job_id: job_id.clone(),
            requester: account.clone(),
            model,
            mode: "solo".to_owned(),
            input_hash,
            max_fee: JOB_FEE_DREAMS,
            min_reputation: 0,
            bid_window_epochs: 2,
            deadline_epoch: epoch + 10,
            epoch,
            nonce,
            signed_by: account.clone(),
            persist_on_fs: None,
            fs_fee: None,
            min_verifiers: None,
            node_fingerprint: None,
        };

        let sig_hex = signing_key.as_ref().as_ref().map(|sk| {
            use ed25519_dalek::Signer;
            let bytes = serde_json::to_vec(&entry).unwrap_or_default();
            hex::encode(sk.sign(&bytes).to_bytes())
        });

        chain.push_pending(entry.clone(), sig_hex.clone());
        if let Ok(data) = serde_json::to_vec(&serde_json::json!({"entry": entry})) {
            let _ = cmd_tx.send(NetCmd::Broadcast { topic: "btcpc/entries", data }).await;
        }

        // Store prompt text so workers can fetch it.
        let _ = chain.store.state_set(
            &format!("infer_input:{}", job_id),
            prompt.as_bytes(),
        );

        info!("[work-gen] epoch {} — posted job {} (prompt_idx={})", epoch, &job_id[..12], prompt_idx);
    }
}
