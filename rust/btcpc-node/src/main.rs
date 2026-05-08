/*!
# btcpc-node

BTCPC sovereign chain node — single binary for networking, consensus,
state machine, block production, contract execution, and HTTP API.

## Environment Variables

    BTCPC_DATA_DIR        — RocksDB data directory (default: ~/.btcpc)
    BTCPC_ACCOUNT         — this node's account name
    BTCPC_NODE_ID         — libp2p node identity label
    BTCPC_API_PORT        — HTTP API port (default: 4242)
    BTCPC_P2P_PORT        — libp2p listen port (default: 6942)
    BTCPC_MINER               — "true" to enable mining
    BTCPC_CLOCK               — "true" to participate in clock consensus
    BTCPC_GENESIS_FILE        — path to genesis.json
    BTCPC_GENESIS_TIMESTAMP   — Unix ms timestamp for genesis block (MUST match on all nodes)
    BTCPC_LOG_LEVEL           — tracing filter (default: btcpc_node=info)
    BTCPC_BOOTSTRAP_PEERS     — comma-separated multiaddrs for DHT bootstrap
    BTCPC_CHAIN_ID            — "btcpc-1" (mainnet) or "btcpc-satoshi" (testnet)
    BTCPC_POSTING_KEY         — hex-encoded 32-byte ed25519 seed; node_id is derived from
                                the public key and all clock seals are signed with it
    BTCPC_STORAGE             — "true" to emit StorageHeartbeat each epoch (earns StorageReward)
    BTCPC_ETH_RPC             — Ethereum JSON-RPC endpoint for ENS resolution (default: cloudflare-eth.com)
    BTCPC_SERVICE             — "true" to emit ServiceHeartbeat each epoch (earns ServiceReward)
    BTCPC_SENSOR              — "true" to emit SensorDataCommit each epoch (earns SensorReward)
    BTCPC_MEMPOOL             — "true" to emit MempoolHeartbeat each epoch (earns MempoolReward)
*/

mod api;
mod chain;
mod clock;
mod config;
mod contracts;
mod discovery;
mod finalize;
mod genesis;
mod hardware;
mod inference;
mod inference_daemon;
mod miner;
mod reserved_names;
mod sensor;
mod services;
mod storage;
mod worker;
mod net;
mod sim;
mod store;
mod tx;
mod utils;
mod wallet;
mod ens;
mod linkgit_server;
mod storage_sync;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use anyhow::Result;
use tracing::{info, warn};
use btcpc_types::{
    LedgerEntry, block_reward_at, era, RECYCLE_ERA, RECYCLE_REWARD_RATE, RECYCLE_REWARD_DENOM,
    RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, TESTNET_CHAIN_ID, TESTNET_FUND_ACCOUNT,
    inference_score, clock_reward_at, testnet_reward_at,
    sensor_score, stake_weight, mempool_relay_score,
    ACTIVITY_RATIO_DENOM, MIN_ACTIVITY_RATIO_NUM,
    CALIBRATION_INFERENCE, CALIBRATION_STORAGE, CALIBRATION_SENSOR,
    CALIBRATION_VERIFIER, CALIBRATION_SERVICE, CALIBRATION_MEMPOOL,
    CALIBRATION_TRACKER, CALIBRATION_LINKGIT, CALIBRATION_LINKGIT_BUILD,
    CRITICAL_MASS_INFERENCE, CRITICAL_MASS_STORAGE, CRITICAL_MASS_SENSOR,
    CRITICAL_MASS_VERIFIER, CRITICAL_MASS_SERVICE, CRITICAL_MASS_MEMPOOL,
    CRITICAL_MASS_TRACKER, CRITICAL_MASS_LINKGIT, CRITICAL_MASS_LINKGIT_BUILD,
    MINE_GRACE_PERIOD_EPOCHS, MINE_GRACE_REWARD_BPS, MINE_GRACE_BENCHMARK_JOBS_PER_EPOCH,
    INFERENCE_PRUNE_WINDOW_EPOCHS,
    STORAGE_PROOF_NO_PROOF_BPS,
    SENSOR_LOCATION_BOOST_BPS, SENSOR_LOCATION_REQUIRED_EPOCH,
    EMA_ALPHA_NUM, EMA_ALPHA_DENOM,
    entry_weight, compute_next_base_fee, BASE_FEE_INITIAL_DREAMS, EPOCH_TARGET_WEIGHT_UNITS,
};

use chain::Chain;
use config::Config;
use contracts::ContractEngine;
use net::{NetCmd, NetworkEvent};
use store::Store;

#[tokio::main]
async fn main() -> Result<()> {
    let mut cfg = Config::from_env();

    // Prevent multiple instances on the same machine.
    // Port pre-check works everywhere including WSL↔Windows cross-instance conflicts,
    // since WSL2 and Windows share the localhost port namespace.
    {
        use std::net::TcpListener;
        let probe_port = cfg.api_port;
        match TcpListener::bind(("127.0.0.1", probe_port)) {
            Ok(_) => {} // port is free; the listener is immediately dropped
            Err(_) => {
                eprintln!("btcpc-node: port {} is already in use — another instance may be running", probe_port);
                std::process::exit(1);
            }
        }
    }
    // Unix file lock catches same-OS duplicate starts sharing the same data dir.
    #[cfg(unix)]
    let _lock_file = {
        use std::os::unix::io::AsRawFd;
        std::fs::create_dir_all(&cfg.data_dir)?;
        let lock_path = cfg.data_dir.join("node.lock");
        let lf = std::fs::OpenOptions::new()
            .create(true).write(true).open(&lock_path)?;
        let locked = unsafe { libc::flock(lf.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if locked != 0 {
            eprintln!("btcpc-node: another instance is already running (lock: {:?})", lock_path);
            std::process::exit(1);
        }
        lf // keep alive until process exits
    };

    // Load posting key. node_id stays as the account name for reward routing;
    // the derived pubkey goes into the seal's `pubkey` field for verification.
    let signing_key: Arc<Option<ed25519_dalek::SigningKey>> = Arc::new(
        cfg.posting_key.as_deref().and_then(load_signing_key)
    );
    let signing_pubkey_hex: Arc<Option<String>> = Arc::new(
        signing_key.as_ref().as_ref().map(|sk| hex::encode(sk.verifying_key().to_bytes()))
    );

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| cfg.log_level.parse().unwrap_or_default()),
        )
        .init();

    info!("btcpc-node starting — account={} chain={} data={:?}", cfg.account, cfg.chain_id, cfg.data_dir);

    // Wallet: restore / load / generate all chain keys (BTCPC, Bitcoin, Ethereum).
    let wallet_keys = wallet::init(&cfg.data_dir)?;
    // Keep ~/.btcpc/{account}.wallet.key in sync so TUI/CLI can find keys without knowing data_dir.
    wallet::backup_to_home(&cfg.account, &wallet_keys);

    // Open state database
    let db_path = cfg.data_dir.join("state");
    let store = Store::open(&db_path)?;

    // Network gets a clone of the store for peer-store persistence.
    let (network, net_handle, net_events) = net::Network::new(cfg.clone(), store.clone());

    let chain = Arc::new(Chain::new(store, cfg.node_id.clone(), cfg.chain_id.clone()));

    // Genesis
    genesis::init_genesis(&chain, cfg.genesis_file.as_deref(), cfg.genesis_timestamp)?;

    // One-time backfill: populate txglobal: from existing txhist: entries so historical
    // transactions appear in the explorer even before the txglobal index existed.
    backfill_txglobal(&chain);

    // Register this account on-chain (no-op if already exists) — links all public keys.
    if let Err(e) = wallet::register_account(&chain, &cfg.account, &wallet_keys) {
        warn!("wallet: account registration failed (non-fatal): {}", e);
    }

    // Hardware anti-sybil: detect GPU serial / machine-id, queue HardwareClaim.
    // The claim goes into the pending pool and is gossiped to peers.
    // At the next epoch seal, all nodes apply pending entries in the same deterministic
    // hash order — the first valid claim per fingerprint wins network-wide.
    let hw = hardware::detect();
    let hw_fingerprint_for_seal = hw.fingerprint.clone();
    let hw_account_for_seal = cfg.account.clone();
    if !hw.fingerprint.is_empty() {
        let hw_entry = LedgerEntry::HardwareClaim {
            account:     cfg.account.clone(),
            fingerprint: hw.fingerprint.clone(),
            hw_info:     hw.summary.clone(),
            epoch:       chain.current_epoch(),
            signed_by:   cfg.account.clone(),
        };
        // Queue for epoch-ordered application — do NOT apply directly.
        chain.push_pending(hw_entry.clone(), None);
        info!("[hardware] fingerprint {} queued for epoch commit ({})",
            &hw.fingerprint[..hw.fingerprint.len().min(16)], hw.summary);
    } else {
        info!("[hardware] no identifiable hardware found — skipping hw claim");
    }

    info!("chain state ready — latest epoch={}", chain.current_epoch());
    tokio::spawn(async move {
        if let Err(e) = network.run().await {
            tracing::error!("network error: {}", e);
        }
    });

    // ── Self-announce to Hive + btcpc.net (best-effort, fire-and-forget) ───────
    {
        let chain_id = cfg.chain_id.clone();
        let node_id = cfg.node_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            tokio::join!(
                discovery::announce_to_hive(&chain_id, &node_id),
                discovery::announce_to_btcpc_net(&chain_id, &node_id),
            );
        });
    }

    // ── Clock consensus ───────────────────────────────────────────────────────
    let clock = Arc::new(clock::ClockConsensus::new());
    {
        let clock_ref = clock.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(1_000)).await;
                clock_ref.tick();
            }
        });
    }

    // Wire: clock sealed events → chain epoch advancement + MineReward emission + consensus proposal
    {
        let mut sealed_rx = clock.subscribe();
        let chain_ref = chain.clone();
        let clock_ref = clock.clone();
        let node_id_c = cfg.node_id.clone();
        let cmd_tx_for_seal = net_handle.cmd_tx.clone();
        // Hardware conflict check: after each epoch seal we verify our fingerprint is still ours.
        let hw_fp  = hw_fingerprint_for_seal;
        let hw_acc = hw_account_for_seal;
        tokio::spawn(async move {
            loop {
                match sealed_rx.recv().await {
                    Ok(sealed) if sealed.sealed => {
                        let seal_hash = sealed.winner
                            .as_ref()
                            .map(|w| w.seal_hash.clone())
                            .unwrap_or_default();
                        let ts = sealed.winner
                            .as_ref()
                            .map(|w| w.timestamp)
                            .unwrap_or_else(now_ms);
                        let entry = LedgerEntry::EpochSeal {
                            node_id: node_id_c.clone(),
                            epoch: sealed.epoch,
                            timestamp: ts,
                            seal_hash: seal_hash.clone(),
                            signature: None,
                            node_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
                        };
                        if let Err(e) = chain_ref.apply_entry(&entry) {
                            warn!("clock seal apply failed (epoch {}): {}", sealed.epoch, e);
                        }

                        // ── Apply pending user entries for this epoch ─────────────────
                        // All nodes sort by sha256(entry_bytes) — same gossip set = same order.
                        // This is the commit boundary: entries are "on-chain" from here.
                        let sealed_epoch = sealed.epoch;
                        let pending = chain_ref.drain_pending_sorted();
                        if !pending.is_empty() {
                            let mut applied = 0usize;
                            for (e, sig) in &pending {
                                match tx::validate_and_apply(&chain_ref, e, sig.as_deref()) {
                                    Ok(_) => applied += 1,
                                    Err(err) => tracing::debug!(
                                        "[epoch {}] pending entry rejected: {}", sealed_epoch, err
                                    ),
                                }
                            }
                            info!("[epoch {}] committed {}/{} pending entries",
                                sealed_epoch, applied, pending.len());
                        }

                        // Hardware conflict check: if our fingerprint is now claimed by
                        // a different account (lost the ordering race), exit immediately.
                        if !hw_fp.is_empty() {
                            let claim_key = format!("hw_claim:{}", hw_fp);
                            if let Some(bytes) = chain_ref.store.state_get(&claim_key) {
                                let claimer = String::from_utf8_lossy(&bytes);
                                if claimer != hw_acc.as_str() {
                                    eprintln!(
                                        "btcpc-node: hardware conflict at epoch {} — fingerprint \
                                         claimed by '{}'. Only one BTCPC account per physical \
                                         machine is allowed.", sealed_epoch, claimer
                                    );
                                    std::process::exit(1);
                                }
                            }
                        }

                        // Sum entry weights for this epoch (T3-2): used to adjust base fee.
                        // Counts all submitted entries (including rejected) — bandwidth is bandwidth.
                        let epoch_total_weight: u64 = pending.iter()
                            .map(|(e, _)| entry_weight(e))
                            .sum();

                        // Release matured unbonding tokens and apply timelocked param changes.
                        chain_ref.drain_unbonding(sealed_epoch);
                        chain_ref.drain_pending_params(sealed_epoch);

                        // Dynamic base fee update (EIP-1559-style ±10%/epoch toward 50% target).
                        let current_base_fee: u64 = chain_ref.store.state_get("chain_param:base_fee")
                            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                            .and_then(|j| j["fee"].as_u64())
                            .unwrap_or(BASE_FEE_INITIAL_DREAMS);
                        let new_base_fee = compute_next_base_fee(
                            current_base_fee, epoch_total_weight, EPOCH_TARGET_WEIGHT_UNITS
                        );
                        let _ = chain_ref.store.state_set(
                            "chain_param:base_fee",
                            &serde_json::to_vec(&serde_json::json!({ "fee": new_base_fee }))
                                .unwrap_or_default(),
                        );
                        if new_base_fee != current_base_fee {
                            info!(
                                "clock: epoch {} base_fee {} → {} (epoch_weight={} target={})",
                                sealed_epoch, current_base_fee, new_base_fee,
                                epoch_total_weight, EPOCH_TARGET_WEIGHT_UNITS
                            );
                        }

                        // Count unique active verifiers over last VERIFIER_ACTIVITY_TRACK_EPOCHS.
                        // Stored as `active_verifier_count` for auto-scaling min_verifiers per job.
                        {
                            use btcpc_types::VERIFIER_ACTIVITY_TRACK_EPOCHS;
                            let window_start = sealed_epoch.saturating_sub(VERIFIER_ACTIVITY_TRACK_EPOCHS);
                            let mut unique: std::collections::HashSet<String> = std::collections::HashSet::new();
                            for ep in window_start..=sealed_epoch {
                                for (k, _) in chain_ref.store.state_scan_prefix(&format!("infer_verify:{}:", ep)) {
                                    // key format: "infer_verify:{epoch}:{verifier}"
                                    if let Some(verifier) = k.strip_prefix(&format!("infer_verify:{}:", ep)) {
                                        unique.insert(verifier.to_string());
                                    }
                                }
                            }
                            let count = unique.len() as u64;
                            let _ = chain_ref.store.state_set("active_verifier_count", &count.to_le_bytes());
                        }

                        // Refresh the registered clock node set so quorum uses the live list.
                        let reg_clocks = clock::registered_clock_nodes(&chain_ref.store);
                        // Persist the validator snapshot for this epoch — used by fork choice
                        // and external auditors to verify quorum claims against the registered set.
                        let _ = chain_ref.store.state_set(
                            &format!("epoch_validators:{}", sealed_epoch),
                            &serde_json::to_vec(&reg_clocks).unwrap_or_default(),
                        );
                        clock_ref.set_registered_clocks(reg_clocks);

                        // Epoch entropy: XOR all winning seal hashes → SHA-256.
                        let seal_hashes: Vec<&str> = sealed.signing_clocks.iter().map(|_| {
                            // Use the winner seal_hash if present; fall back to empty string.
                            sealed.winner.as_ref().map(|w| w.seal_hash.as_str()).unwrap_or("")
                        }).collect();
                        let entropy = clock::compute_epoch_entropy(&seal_hashes);
                        let _ = chain_ref.store.state_set(
                            &format!("epoch_entropy:{}", sealed_epoch),
                            entropy.as_bytes(),
                        );

                        // Comprehensive epoch reward distribution across all work types.
                        // Pass the signing_clocks list directly — the seal:{epoch}: store
                        // scan was dead (those keys were never written anywhere).
                        let signing_clocks = sealed.signing_clocks.clone();
                        emit_epoch_rewards(sealed_epoch, &signing_clocks, &chain_ref, &cmd_tx_for_seal).await;

                        // Persist the sealed block — written after rewards so all entries are indexed.
                        chain_ref.write_sealed_block(sealed_epoch, seal_hash.as_str(), ts);

                        // Compute reward hash and broadcast consensus proposal.
                        let rewards_hash = clock::compute_rewards_hash(sealed_epoch, &chain_ref.store);
                        let proposal = clock::RewardProposal {
                            epoch: sealed_epoch,
                            node_id: node_id_c.clone(),
                            rewards_hash: rewards_hash.clone(),
                        };
                        clock_ref.receive_reward_proposal(proposal);
                        if let Ok(data) = serde_json::to_vec(&serde_json::json!({
                            "epoch": sealed_epoch,
                            "node_id": node_id_c,
                            "rewards_hash": rewards_hash,
                        })) {
                            let _ = cmd_tx_for_seal.send(NetCmd::Broadcast {
                                topic: "btcpc/consensus",
                                data,
                            }).await;
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("clock sealed_rx lagged by {} events", n);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // Wire: finalized-epoch events → emit EpochFinalize entry
    {
        let mut finalized_rx = clock.subscribe_finalized();
        let chain_ref = chain.clone();
        let node_id_c = cfg.node_id.clone();
        let cmd_tx_fin = net_handle.cmd_tx.clone();
        tokio::spawn(async move {
            loop {
                match finalized_rx.recv().await {
                    Ok(fin) => {
                        let state_root = chain_ref.store.balance_merkle_root();
                        let entry = LedgerEntry::EpochFinalize {
                            epoch: fin.epoch,
                            node_id: node_id_c.clone(),
                            rewards_hash: fin.rewards_hash,
                            quorum: fin.quorum as u64,
                            sealed_by: vec![],
                            state_root,
                            timestamp: now_ms(),
                        };
                        if let Err(e) = chain_ref.apply_entry(&entry) {
                            warn!("EpochFinalize apply failed (epoch {}): {}", fin.epoch, e);
                        }
                        let envelope = serde_json::json!({"entry": entry});
                        if let Ok(data) = serde_json::to_vec(&envelope) {
                            let _ = cmd_tx_fin.send(NetCmd::Broadcast {
                                topic: "btcpc/entries",
                                data,
                            }).await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("finalized_rx lagged by {} events", n);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // Auto-register as a clock node on startup if not already registered.
    if cfg.is_clock {
        let reg_key = format!("clock_reg:{}", cfg.node_id);
        let already_registered = chain.store.state_get(&reg_key).is_some();
        if !already_registered {
            let min_stake: u64 = chain.store.state_get("chain_param:clock_min_stake")
                .and_then(|b| serde_json::from_slice::<u64>(&b).ok())
                .unwrap_or(5 * 10_000_000_000);
            let balance = chain.store.get_balance(&cfg.node_id, btcpc_types::NATIVE_TOKEN);
            if balance >= min_stake {
                let epoch = chain.current_epoch();
                let pubkey = signing_pubkey_hex.as_deref().map(|s| s.to_owned());
                let entry = btcpc_types::LedgerEntry::ClockNodeRegister {
                    node_id: cfg.node_id.clone(),
                    stake: min_stake,
                    epoch,
                    pubkey,
                    signature: None,
                };
                // Sign with posting key (accepted alongside active key for self-registration).
                let sig_hex: Option<String> = signing_key.as_ref().as_ref().and_then(|sk| {
                    crate::tx::canonical_signing_message(&entry, &chain.chain_id).ok().map(|msg| {
                        use ed25519_dalek::Signer;
                        hex::encode(sk.sign(msg.as_bytes()).to_bytes())
                    })
                });
                let sig_ref = sig_hex.as_deref();
                match crate::tx::validate_and_apply(&chain, &entry, sig_ref) {
                    Ok(hash) => {
                        info!("[clock] auto-registered '{}' as clock node (hash {})", cfg.node_id, hash);
                        broadcast_entry_desktop(&entry, &net_handle.cmd_tx).await;
                    },
                    Err(e)   => warn!(
                        "[clock] auto-register failed for '{}': {} — \
                        check BTCPC_POSTING_KEY is the private key seed (not the public key) \
                        matching the posting key registered on-chain for this account, \
                        or call POST /api/clock/register manually with a signed payload",
                        cfg.node_id, e
                    ),
                }
            } else {
                warn!("[clock] '{}' has {} dreams, needs {} to register as clock node",
                    cfg.node_id, balance, min_stake);
            }
        }
    }

    // Emit seals when this node is running as a clock peer
    if cfg.is_clock {
        let clock_ref = clock.clone();
        let cmd_tx = net_handle.cmd_tx.clone();
        let node_id_c = cfg.node_id.clone();
        let signing_key_c = Arc::clone(&signing_key);
        let signing_pubkey_c = Arc::clone(&signing_pubkey_hex);
        // Epoch is relative to genesis, not Unix epoch.
        // genesis_ts is guaranteed set (init_genesis would have errored otherwise).
        let genesis_ts = cfg.genesis_timestamp.unwrap_or(0);
        tokio::spawn(async move {
            let mut last_sent: u64 = 0;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                let now = now_ms();
                let elapsed = now.saturating_sub(genesis_ts);
                let epoch = elapsed / btcpc_types::EPOCH_MS;
                if epoch > last_sent {
                    last_sent = epoch;
                    let seal_hash = {
                        use sha2::Digest;
                        let mut h = sha2::Sha256::new();
                        h.update(epoch.to_le_bytes());
                        hex::encode(h.finalize())
                    };
                    // node_id is the account name (for reward routing).
                    // pubkey is the ed25519 verifying key hex (for signature verification).
                    let sig_hex = signing_key_c.as_ref().as_ref().map(|sk| {
                        use ed25519_dalek::Signer;
                        let msg = format!("seal:{}:{}:{}:{}", epoch, seal_hash, node_id_c, now);
                        hex::encode(sk.sign(msg.as_bytes()).to_bytes())
                    });
                    let seal = serde_json::json!({
                        "epoch_number": epoch,
                        "node_id": node_id_c,
                        "timestamp": now,
                        "seal_hash": seal_hash,
                        "signature": sig_hex,
                        "pubkey": *signing_pubkey_c,
                    });
                    // Self-ingest so we count toward quorum on single-node networks.
                    clock_ref.receive_seal(seal.clone());
                    if let Ok(data) = serde_json::to_vec(&seal) {
                        let _ = cmd_tx.send(NetCmd::Broadcast {
                            topic: "btcpc/seals",
                            data,
                        }).await;
                    }
                }
            }
        });
    }

    // ── Finalizer ─────────────────────────────────────────────────────────────
    {
        let chain_ref = chain.clone();
        tokio::spawn(async move {
            finalize::run_finalizer(chain_ref, 10).await;
        });
    }

    // ── Inference marketplace daemon ──────────────────────────────────────────
    {
        let chain_ref = chain.clone();
        tokio::spawn(async move {
            inference_daemon::run(chain_ref).await;
        });
    }

    // Compute binary SHA-256 once at startup for software attestation in Mine entries.
    let software_hash = {
        use sha2::{Sha256, Digest};
        let self_path = std::env::current_exe().unwrap_or_default();
        match std::fs::read(&self_path) {
            Ok(bytes) => format!("sha256:{}", hex::encode(Sha256::digest(&bytes))),
            Err(e) => {
                tracing::warn!("software attestation: could not hash binary: {}", e);
                String::new()
            }
        }
    };

    // ── Mining (legacy direct-inference loop) ────────────────────────────────
    // NOTE: BTCPC_MINER submits Mine entries each epoch via a local Ollama call.
    // This bypasses the inference marketplace. New deployments should use
    // BTCPC_WORKER=true instead, which bids on posted jobs and earns job fees.
    if cfg.is_miner {
        let chain_ref = chain.clone();
        let account = cfg.account.clone();
        let genesis_ts = cfg.genesis_timestamp.unwrap_or(0);
        let cmd_for_miner = net_handle.cmd_tx.clone();
        let sw_hash_for_miner = software_hash.clone();
        tokio::spawn(async move {
            miner::run_miner(chain_ref, account, genesis_ts, cmd_for_miner, sw_hash_for_miner).await;
        });
    }

    // Shared counter: git packs served to remote clients this epoch.
    // Incremented by git_upload_pack in api.rs; consumed by storage.rs each heartbeat.
    let git_serve_queries = Arc::new(AtomicUsize::new(0));

    // ── Storage node (StorageHeartbeat each epoch) ───────────────────────────
    // BTCPC_STORAGE=true: proves bytes on disk each epoch, earns StorageReward.
    if std::env::var("BTCPC_STORAGE").map(|v| v == "true" || v == "1").unwrap_or(false) {
        let chain_ref   = chain.clone();
        let account     = cfg.account.clone();
        let data_dir    = cfg.data_dir.clone();
        let genesis_ts  = cfg.genesis_timestamp.unwrap_or(0);
        let cmd_storage = net_handle.cmd_tx.clone();
        let git_queries = git_serve_queries.clone();
        tokio::spawn(async move {
            storage::run_storage_node(chain_ref, account, data_dir, genesis_ts, cmd_storage, git_queries).await;
        });
    }

    // ── Service node (ServiceHeartbeat each epoch) ───────────────────────────
    // BTCPC_SERVICE=true: proves active container-hours, earns ServiceReward.
    // Covers general service nodes; LinkGit storage nodes set this flag too.
    if std::env::var("BTCPC_SERVICE").map(|v| v == "true" || v == "1").unwrap_or(false) {
        let chain_ref   = chain.clone();
        let account     = cfg.account.clone();
        let genesis_ts  = cfg.genesis_timestamp.unwrap_or(0);
        let cmd_service = net_handle.cmd_tx.clone();
        tokio::spawn(async move {
            services::run_service_node(chain_ref, account, genesis_ts, cmd_service).await;
        });
    }

    // ── Sensor node (SensorDataCommit each epoch) ────────────────────────────
    // BTCPC_SENSOR=true: reads system sensors (CPU temp, uptime) and submits
    // SensorDataCommit each epoch, earning SensorReward.
    if std::env::var("BTCPC_SENSOR").map(|v| v == "true" || v == "1").unwrap_or(false) {
        let chain_ref  = chain.clone();
        let account    = cfg.account.clone();
        let genesis_ts = cfg.genesis_timestamp.unwrap_or(0);
        let cmd_sensor = net_handle.cmd_tx.clone();
        tokio::spawn(async move {
            sensor::run_sensor_node(chain_ref, account, genesis_ts, cmd_sensor).await;
        });
    }

    // ── Worker (inference marketplace participant) ────────────────────────────
    // BTCPC_WORKER=true: watches the chain for posted inference jobs, bids on
    // them, calls Ollama when awarded, and submits InferenceJobComplete.
    if std::env::var("BTCPC_WORKER").map(|v| v == "true" || v == "1").unwrap_or(false) {
        let chain_ref = chain.clone();
        let account = cfg.account.clone();
        let cmd_for_worker = net_handle.cmd_tx.clone();
        tokio::spawn(async move {
            worker::run_worker(chain_ref, account, cmd_for_worker).await;
        });
    }

    // ── Broadcast channel (entries → net gossip) ──────────────────────────────
    let (tx_broadcast, _) = tokio::sync::broadcast::channel::<api::GossipEntry>(256);

    // Forward newly-accepted entries to gossip peers.
    // Wrap as {"entry": <json>, "sig": <hex_or_null>} so receiving nodes can
    // re-verify signatures for accounts that have registered keys.
    // For InferenceJobPost and InferenceJobComplete, also carry the actual
    // input_text / output_text so remote verifiers can assess quality.
    {
        let mut net_rx = tx_broadcast.subscribe();
        let cmd_tx = net_handle.cmd_tx.clone();
        let chain_ref = chain.clone();
        tokio::spawn(async move {
            loop {
                match net_rx.recv().await {
                    Ok((entry, sig)) => {
                        if let Ok(entry_val) = serde_json::to_value(&entry) {
                            let mut envelope = serde_json::json!({
                                "entry": entry_val,
                                "sig": sig,
                            });
                            // Attach plaintext so remote verifiers can judge quality.
                            match &entry {
                                LedgerEntry::InferenceJobPost { job_id, .. } => {
                                    if let Some(t) = chain_ref.store.state_get(
                                        &format!("infer_input:{}", job_id)
                                    ).and_then(|b| String::from_utf8(b).ok()) {
                                        envelope["input_text"] = serde_json::Value::String(t);
                                    }
                                }
                                LedgerEntry::InferenceJobComplete { job_id, .. } => {
                                    if let Some(t) = chain_ref.store.state_get(
                                        &format!("infer_output:{}", job_id)
                                    ).and_then(|b| String::from_utf8(b).ok()) {
                                        envelope["output_text"] = serde_json::Value::String(t);
                                    }
                                }
                                _ => {}
                            }
                            if let Ok(data) = serde_json::to_vec(&envelope) {
                                let _ = cmd_tx.send(NetCmd::Broadcast {
                                    topic: "btcpc/entries",
                                    data,
                                }).await;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("net gossip tx lagged by {} entries", n);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // Shared peer count: updated by the net event loop, read by the API.
    let shared_peer_count = Arc::new(AtomicUsize::new(0));

    // Shared relay counter: incremented each time a gossip entry is accepted into
    // the pending pool.  Consumed by the mempool heartbeat daemon each epoch.
    let mempool_relay_counter = Arc::new(AtomicUsize::new(0));

    // ── Storage sync (LinkGit object replication) ─────────────────────────────
    let (sync_tx, sync_rx) = tokio::sync::mpsc::channel::<storage_sync::RefAvailable>(256);
    {
        let store_for_sync = Arc::new(chain.store.clone());
        tokio::spawn(storage_sync::run(store_for_sync, sync_rx));
    }

    // Apply incoming network events to chain state
    {
        let chain_ref = chain.clone();
        let clock_ref = clock.clone();
        let peer_count_ref = shared_peer_count.clone();
        let relay_counter_ref = mempool_relay_counter.clone();
        let mut events = net_events;
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(NetworkEvent::PeerConnected { .. }) => {
                        let n = peer_count_ref.fetch_add(1, Ordering::Relaxed) + 1;
                        clock_ref.update_peers(n);
                    }
                    Ok(NetworkEvent::PeerDisconnected { .. }) => {
                        let prev = peer_count_ref.load(Ordering::Relaxed);
                        let n = prev.saturating_sub(1);
                        peer_count_ref.store(n, Ordering::Relaxed);
                        clock_ref.update_peers(n);
                    }
                    Ok(NetworkEvent::Entry { entry }) => {
                        // Extract and store any plaintext attached to the gossip envelope
                        // so the local verifier can access it later.
                        if let Some(job_id) = entry.get("entry")
                            .and_then(|e| e.get("InferenceJobPost"))
                            .and_then(|v| v.get("job_id"))
                            .and_then(|v| v.as_str())
                        {
                            if let Some(t) = entry.get("input_text").and_then(|v| v.as_str()) {
                                let _ = chain_ref.store.state_set(
                                    &format!("infer_input:{}", job_id), t.as_bytes());
                            }
                        }
                        if let Some(job_id) = entry.get("entry")
                            .and_then(|e| e.get("InferenceJobComplete"))
                            .and_then(|v| v.get("job_id"))
                            .and_then(|v| v.as_str())
                        {
                            if let Some(t) = entry.get("output_text").and_then(|v| v.as_str()) {
                                let _ = chain_ref.store.state_set(
                                    &format!("infer_output:{}", job_id), t.as_bytes());
                            }
                        }

                        // Unwrap gossip envelope {"entry": ..., "sig": ...}.
                        // Fall back to treating the whole value as the entry (legacy/direct format).
                        let (entry_val, sig) = if let Some(inner) = entry.get("entry") {
                            let sig = entry.get("sig")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned);
                            (inner.clone(), sig)
                        } else {
                            (entry, None)
                        };
                        match tx::entry_from_json(&entry_val) {
                            Ok(e) => {
                                if tx::is_system_entry(&e) {
                                    // System entries (rewards, seals) are generated locally
                                    // on epoch seal — silently drop if received via gossip.
                                    tracing::trace!("ignoring gossiped system entry");
                                } else if matches!(e, btcpc_types::LedgerEntry::Mine { .. }) {
                                    // Mine entries must be in every node's DB before reward
                                    // hash is computed at epoch seal. Apply directly so the
                                    // reward consensus hash matches across all nodes.
                                    if let Err(err) = chain_ref.apply_entry(&e) {
                                        tracing::debug!("gossip Mine apply failed: {}", err);
                                    }
                                } else {
                                    // User entries go into the pending pool and are applied
                                    // at epoch seal in deterministic hash order across all nodes.
                                    chain_ref.push_pending(e, sig);
                                    relay_counter_ref.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            Err(e) => tracing::debug!("net entry parse error: {}", e),
                        }
                    }
                    Ok(NetworkEvent::Block { .. }) => {
                        // Miners no longer produce blocks — block gossip is ignored.
                    }
                    Ok(NetworkEvent::EpochSeal { seal }) => {
                        clock_ref.receive_seal(seal);
                    }
                    Ok(NetworkEvent::ConsensusProposal { epoch, rewards_hash, node_id }) => {
                        clock_ref.receive_reward_proposal(clock::RewardProposal {
                            epoch, rewards_hash, node_id,
                        });
                    }
                    Ok(NetworkEvent::GitRefAvailable { repo_id, owner, repo_name, commit_hash, peer_http }) => {
                        let _ = sync_tx.try_send(storage_sync::RefAvailable {
                            repo_id, owner, repo_name, commit_hash, peer_http,
                        });
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("net events lagged by {}", n);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // ── Contract engine ───────────────────────────────────────────────────────
    let contracts = Arc::new(ContractEngine::new(chain.clone()));

    // ── Testnet sim daemon ────────────────────────────────────────────────────
    if cfg.chain_id == TESTNET_CHAIN_ID {
        let chain_ref = chain.clone();
        tokio::spawn(async move {
            sim::run(chain_ref).await;
        });
    }

    // ── Inference verifier ────────────────────────────────────────────────────
    {
        let chain_ref = chain.clone();
        let account   = cfg.account.clone();
        let cmd_tx    = net_handle.cmd_tx.clone();
        tokio::spawn(async move {
            run_inference_verifier(chain_ref, account, cmd_tx).await;
        });
    }

    // ── Mempool heartbeat node ────────────────────────────────────────────────
    // BTCPC_MEMPOOL=true: reports entries relayed per epoch, earns MempoolReward.
    // Any running node already relays gossip — this just makes that work auditable.
    if std::env::var("BTCPC_MEMPOOL").map(|v| v == "true" || v == "1").unwrap_or(false) {
        let chain_ref    = chain.clone();
        let account      = cfg.account.clone();
        let genesis_ts   = cfg.genesis_timestamp.unwrap_or(0);
        let cmd_mempool  = net_handle.cmd_tx.clone();
        let relay_ctr    = mempool_relay_counter.clone();
        tokio::spawn(async move {
            run_mempool_node(chain_ref, account, genesis_ts, cmd_mempool, relay_ctr).await;
        });
    }

    // ── HTTP API ──────────────────────────────────────────────────────────────
    let ollama_url_for_model = std::env::var("OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned());
    let default_model = crate::miner::detect_model(&ollama_url_for_model).await
        .unwrap_or_default();

    tracing::info!("software_hash: {}", &software_hash[..software_hash.len().min(24)]);

    let app_state = api::AppState {
        chain: chain.clone(),
        contracts,
        tx_broadcast,
        faucet_claims:     Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        faucet_ip_claims:  Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        agent_rate:        Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        post_rate:         Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        chain_challenges:  Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        current_model:     Arc::new(tokio::sync::RwLock::new(default_model)),
        hw_fingerprint:    Arc::new(hw.fingerprint),
        hw_summary:        Arc::new(hw.summary),
        peer_count:        shared_peer_count,
        clock:             clock.clone(),
        software_hash:     Arc::new(software_hash),
        git_serve_queries: git_serve_queries.clone(),
        node_account:      Arc::new(cfg.account.clone()),
    };
    api::serve(app_state, cfg.api_port).await?;

    Ok(())
}

use utils::now_ms;

// ── Inference verifier ────────────────────────────────────────────────────────
//
// Periodically scans for completed inference jobs from OTHER nodes, re-runs
// the same prompt via Ollama using the same model, and submits a verdict.
// Only verifies jobs whose model matches the locally configured BTCPC_MODEL.

async fn run_inference_verifier(
    chain:   Arc<Chain>,
    account: String,
    cmd_tx:  tokio::sync::mpsc::Sender<NetCmd>,
) {
    use std::time::Duration;
    use sha2::{Digest, Sha256};

    let ollama_url = std::env::var("OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned());
    let local_model = std::env::var("BTCPC_MODEL")
        .unwrap_or_else(|_| "qwen2.5:0.5b".to_owned());

    // Peers to try when IO text is not stored locally (worker node has the actual output).
    let peer_apis: Vec<String> = std::env::var("BTCPC_PEER_APIS")
        .unwrap_or_else(|_| "http://192.168.68.72:4242,http://192.168.68.75:4242".to_owned())
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();

    let http = reqwest::Client::new();

    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;

        let epoch = chain.current_epoch();
        let jobs = chain.store.state_scan_prefix("infer_job:");

        for (_, val) in jobs {
            use inference::JobState;
            let job: JobState = match serde_json::from_slice(&val) {
                Ok(v) => v, Err(_) => continue,
            };

            // Only verify completed jobs from other nodes.
            if job.winner.as_deref() == Some(account.as_str()) { continue; }
            if job.status != inference::JobStatus::Completed { continue; }

            let job_id = &job.job_id;
            let verdict_key = format!("infer_verdict:{}:{}", job_id, account);
            if chain.store.state_get(&verdict_key).is_some() { continue; }

            // Fetch IO text: check local store first, then fall back to peer APIs.
            let (input_text, output_text) = {
                let local_in = chain.store.state_get(&format!("infer_input:{}", job_id))
                    .and_then(|b| String::from_utf8(b).ok());
                let local_out = chain.store.state_get(&format!("infer_output:{}", job_id))
                    .and_then(|b| String::from_utf8(b).ok());

                if local_in.is_some() && local_out.is_some() {
                    (local_in.unwrap(), local_out.unwrap())
                } else {
                    let mut fetched_in = local_in;
                    let mut fetched_out = local_out;
                    for peer in &peer_apis {
                        if fetched_in.is_some() && fetched_out.is_some() { break; }
                        let url = format!("{}/api/inference/job/{}/io", peer, job_id);
                        if let Ok(r) = http.get(&url).timeout(Duration::from_secs(10)).send().await {
                            if let Ok(body) = r.json::<serde_json::Value>().await {
                                if fetched_in.is_none() {
                                    fetched_in = body["input_text"].as_str().map(str::to_owned);
                                }
                                if fetched_out.is_none() {
                                    fetched_out = body["output_text"].as_str().map(str::to_owned);
                                }
                            }
                        }
                    }
                    match (fetched_in, fetched_out) {
                        (Some(i), Some(o)) => (i, o),
                        _ => continue,
                    }
                }
            };

            // Ask the local model to evaluate whether the work was done.
            let meta_prompt = format!(
                "Task: {}\nResponse: {}\n\n\
                Did the AI assistant adequately complete the task?\n\
                Reply with exactly one word: APPROVED, REJECTED, or REVIEW.",
                input_text.chars().take(512).collect::<String>(),
                output_text.chars().take(512).collect::<String>(),
            );

            let resp = http
                .post(format!("{}/api/generate", ollama_url))
                .timeout(Duration::from_secs(30))
                .json(&serde_json::json!({
                    "model":  local_model,
                    "prompt": meta_prompt,
                    "stream": false,
                    "options": { "num_predict": 20, "temperature": 0.0 },
                }))
                .send()
                .await;

            let verdict = match resp {
                Ok(r) if r.status().is_success() => {
                    match r.json::<serde_json::Value>().await {
                        Ok(body) => {
                            let text = body["response"].as_str().unwrap_or("").to_uppercase();
                            if text.contains("APPROVED") {
                                "approved"
                            } else if text.contains("REJECTED") || text.contains("REJECT") {
                                "rejected"
                            } else {
                                "review_required"
                            }
                        }
                        Err(_) => continue,
                    }
                }
                _ => continue,
            };

            // Estimate value_score from output length + model (true score comes via claim flow)
            let output_tokens = (output_text.split_whitespace().count() as u64).max(1);
            let value_score = btcpc_types::inference_score(output_tokens, 0, &job.model);

            let entry = LedgerEntry::InferenceJobVerify {
                job_id:      job_id.clone(),
                verifier:    account.clone(),
                verdict:     verdict.to_owned(),
                value_score,
                reason:      None,
                reveal_salt: None,
                epoch,
                signed_by:   account.clone(),
            };
            if let Err(e) = chain.apply_entry(&entry) {
                warn!("verifier: apply failed for job {}: {}", job_id, e);
                continue;
            }
            let envelope = serde_json::json!({ "entry": entry });
            if let Ok(data) = serde_json::to_vec(&envelope) {
                let _ = cmd_tx.send(NetCmd::Broadcast {
                    topic: "btcpc/entries", data,
                }).await;
            }
            let _ = chain.store.state_set(&verdict_key, verdict.as_bytes());
            info!("verifier: job {} → {}", job_id, verdict);

            break; // one verdict per cycle
        }
    }
}

// ── txglobal backfill ─────────────────────────────────────────────────────────

/// Scan txhist: and write any missing txglobal: entries.
/// Runs once at startup — safe to repeat, skips existing keys.
fn backfill_txglobal(chain: &Chain) {
    let all: Vec<(String, Vec<u8>)> = chain.store.state_scan_prefix("txhist:");
    let mut written = 0usize;
    for (key, val) in all {
        // key: txhist:{account}:{epoch:016x}:{role}:{hash}
        let parts: Vec<&str> = key.splitn(5, ':').collect();
        if parts.len() < 5 { continue; }
        let epoch_hex  = parts[2];
        let hash_prefix = parts[4];
        let global_key = format!("txglobal:{}:{}", epoch_hex, hash_prefix);
        if chain.store.state_get(&global_key).is_some() { continue; }
        if let Err(e) = chain.store.state_set(&global_key, &val) {
            tracing::warn!("backfill_txglobal: {}", e);
        } else {
            written += 1;
        }
    }
    if written > 0 {
        tracing::info!("txglobal backfill: {} entries written", written);
    }
}

// ── Epoch reward distribution ─────────────────────────────────────────────────

/// Distribute the epoch's reward pool: mandatory reserve split first, then
/// Layer D infrastructure base, then Layer B dynamic activity pools.
///
/// Layer A scalar and Layer C fee boost are computed here but the full EMA
/// state persistence comes in a follow-up; for now they fall back to 1.0.
async fn emit_epoch_rewards(
    epoch: u64,
    clock_sealers: &[String],
    chain: &Arc<Chain>,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
) {
    // Sweep expired pending token transfers → refund sender.
    sweep_expired_pending_transfers(epoch, chain);

    let raw_pool = if era(epoch) >= RECYCLE_ERA {
        let fund = chain.store.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN);
        ((fund as u128 * RECYCLE_REWARD_RATE) / RECYCLE_REWARD_DENOM) as u64
    } else {
        block_reward_at(epoch)
    };
    if raw_pool == 0 { return; }

    // ── Mandatory 2% reserve split (Layer D, always fires) ───────────────────
    // 1.5% → recycle fund  |  0.5% → testnet fund (perpetual top-up)
    let reserve_total  = (raw_pool as u128 * 2 / 100) as u64;
    let testnet_top_up = (raw_pool as u128 / 200) as u64;           // 0.5%
    let recycle_split  = reserve_total.saturating_sub(testnet_top_up); // 1.5%
    let activity_pool  = raw_pool.saturating_sub(reserve_total);

    if recycle_split > 0 {
        chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_split);
    }
    if testnet_top_up > 0 {
        chain.store.credit(TESTNET_FUND_ACCOUNT, NATIVE_TOKEN, testnet_top_up);
    }

    // ── Layer D: era-scaled clock reward (infrastructure base) ──────────────
    // Reward is scaled by uptime reputation: 0.5× (low uptime) → 1.0× (perfect).
    // Uptime is tracked as a sliding window (CLOCK_UPTIME_WINDOW epochs).
    // New nodes earn full reward for the first CLOCK_UPTIME_MIN_EPOCHS epochs.
    const CLOCK_UPTIME_WINDOW: u64 = 100;
    const CLOCK_UPTIME_MIN_EPOCHS: u64 = 10;

    let base_clock_reward = clock_reward_at(epoch);

    // Collect all registered clock nodes for the uptime window update.
    let registered_nodes: Vec<String> = chain.store.state_scan_prefix("clock_reg:")
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            // Only track nodes that still have stake (not slashed).
            if j["stake"].as_u64().unwrap_or(0) == 0 { return None; }
            j["node_id"].as_str().map(|s| s.to_owned())
        })
        .collect();

    let sealer_set: std::collections::HashSet<&str> =
        clock_sealers.iter().map(|s| s.as_str()).collect();

    // Update uptime window for every registered node; compute multiplier for sealers.
    for node_id in &registered_nodes {
        let uptime_key = format!("clock_uptime:{}", node_id);
        let prev: serde_json::Value = chain.store.state_get(&uptime_key)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(serde_json::json!({"seals": 0, "epochs": 0}));
        let mut seals  = prev["seals"].as_u64().unwrap_or(0);
        let mut epochs = prev["epochs"].as_u64().unwrap_or(0);

        epochs = (epochs + 1).min(CLOCK_UPTIME_WINDOW);
        let sealed = sealer_set.contains(node_id.as_str());
        if sealed {
            seals = (seals + 1).min(epochs);
        } else if epochs == CLOCK_UPTIME_WINDOW {
            // Steady-state: missed seal leaks one point from the window.
            seals = seals.saturating_sub(1);
        }

        let _ = chain.store.state_set(&uptime_key, &serde_json::to_vec(
            &serde_json::json!({ "seals": seals, "epochs": epochs })
        ).unwrap_or_default());
    }

    // Emit ClockReward for each sealer, scaled by uptime.
    for node_id in clock_sealers {
        if base_clock_reward == 0 { break; }
        let uptime_key = format!("clock_uptime:{}", node_id);
        let uptime: serde_json::Value = chain.store.state_get(&uptime_key)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(serde_json::json!({"seals": 1, "epochs": 1}));
        let seals  = uptime["seals"].as_u64().unwrap_or(1).max(1);
        let epochs = uptime["epochs"].as_u64().unwrap_or(1).max(1);
        let multiplier = if epochs < CLOCK_UPTIME_MIN_EPOCHS {
            1_000u64 // grace period: full reward, millipct units
        } else {
            500 + 500 * seals / epochs // 500–1000 (0.5× – 1.0×)
        };
        let scaled_reward = (base_clock_reward as u128 * multiplier as u128 / 1_000) as u64;
        if scaled_reward == 0 { continue; }
        let entry = LedgerEntry::ClockReward { node_id: node_id.clone(), amount: scaled_reward, epoch };
        if let Err(e) = chain.apply_entry(&entry) {
            warn!("clock: clock reward failed for {} epoch {}: {}", node_id, epoch, e);
            continue;
        }
        broadcast_entry_desktop(&entry, cmd_tx).await;
    }

    // ── Layer E: testnet operator rewards (from testnet fund) ────────────────
    let testnet_ops: Vec<(String, String)> = chain.store.state_scan_prefix("testnet_op:")
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            Some((j["mainnet_account"].as_str()?.to_owned(), j["testnet_node_id"].as_str()?.to_owned()))
        }).collect();

    if !testnet_ops.is_empty() {
        let per_op   = testnet_reward_at(epoch);
        let fund     = chain.store.get_balance(TESTNET_FUND_ACCOUNT, NATIVE_TOKEN);
        let needed   = per_op.saturating_mul(testnet_ops.len() as u64);
        let actual   = if needed > 0 { (fund.min(needed)) / testnet_ops.len() as u64 } else { 0 };
        for (mainnet_account, testnet_node_id) in &testnet_ops {
            if actual == 0 { break; }
            let entry = LedgerEntry::TestnetReward {
                mainnet_account: mainnet_account.clone(),
                testnet_node_id: testnet_node_id.clone(),
                amount: actual,
                epoch,
            };
            if let Err(e) = chain.apply_entry(&entry) {
                warn!("clock: testnet reward failed for {} epoch {}: {}", mainnet_account, epoch, e);
                continue;
            }
            broadcast_entry_desktop(&entry, cmd_tx).await;
        }
        info!("clock: epoch {} testnet: {} ops × {} dreams", epoch, testnet_ops.len(), actual);
    }

    if activity_pool == 0 { return; }

    // ── T3-4: Read EMA critical-mass targets from previous epoch ─────────────
    // EMA is updated AFTER reward distribution so it doesn't influence itself this epoch.
    // Static CRITICAL_MASS_* constants serve as cold-start seeds when no EMA exists yet.
    // A stored value of 0 (EMA decayed to zero during a long gap with no participants)
    // is treated identically to "not set" — the static default acts as a floor.
    let read_ema_cm = |pool: &str, default: u64| -> u64 {
        chain.store.state_get(&format!("ema_critical_mass:{}", pool))
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|j| j["ema"].as_u64())
            .filter(|&v| v > 0)
            .unwrap_or(default)
    };
    let cm_inference = read_ema_cm("inference", CRITICAL_MASS_INFERENCE);
    let cm_storage   = read_ema_cm("storage",   CRITICAL_MASS_STORAGE);
    let cm_sensor    = read_ema_cm("sensor",     CRITICAL_MASS_SENSOR);
    let cm_verifier  = read_ema_cm("verifier",   CRITICAL_MASS_VERIFIER);
    let cm_service   = read_ema_cm("service",    CRITICAL_MASS_SERVICE);
    let cm_mempool   = read_ema_cm("mempool",    CRITICAL_MASS_MEMPOOL);
    let cm_tracker   = read_ema_cm("tracker",    CRITICAL_MASS_TRACKER);
    let cm_linkgit   = read_ema_cm("linkgit",       CRITICAL_MASS_LINKGIT);
    let cm_build     = read_ema_cm("linkgit_build", CRITICAL_MASS_LINKGIT_BUILD);

    // ── Layer B: collect work contributors ────────────────────────────────────

    // Inference: score = output_tokens × hw_tier_weight × model_weight × stake_weight(stake)
    // Grace period (D8): unlinked Mine earns 20% during first MINE_GRACE_PERIOD_EPOCHS.
    // After grace period: unlinked Mine earns 0% (dropped from mines list entirely).
    // Linked Mine (job_id present): must have an approved InferenceJobVerify for the job.
    let in_grace_period = epoch < MINE_GRACE_PERIOD_EPOCHS;
    let mines: Vec<(String, u64)> = chain.store.state_scan_prefix(&format!("mine:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let out_toks = j["output_tokens"].as_u64().unwrap_or(0);
            let hw_tier  = j["hw_tier"].as_u64().unwrap_or(0) as u8;
            let model    = j["model"].as_str().unwrap_or("");
            let miner    = j["miner"].as_str()?.to_owned();
            let stake    = chain.store.get_stake(&miner);
            let sw       = stake_weight(stake);
            let raw_score = inference_score(out_toks, hw_tier, model).saturating_mul(sw);

            // Determine effective score based on job linkage.
            let job_id = j.get("job_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
            let effective_score = if let Some(jid) = job_id {
                // Linked Mine: job must be Verified by the verifier board, and miner must
                // be the awarded worker. Load job state directly — the old infer_verify key
                // lookup was keyed by verifier account, not job_id, so it was always false.
                let job_ok = crate::inference::get_job(&chain, jid)
                    .map(|job| {
                        job.status == crate::inference::JobStatus::Verified
                            || job.status == crate::inference::JobStatus::Paid
                    })
                    .unwrap_or(false);
                if job_ok { raw_score } else {
                    // Job not yet verified (pending board quorum) — grace period rules apply.
                    if in_grace_period {
                        raw_score * MINE_GRACE_REWARD_BPS / 10_000
                    } else {
                        return None; // drop entirely after grace period
                    }
                }
            } else {
                // Unlinked Mine.
                if in_grace_period {
                    raw_score * MINE_GRACE_REWARD_BPS / 10_000
                } else {
                    return None; // unlinked Mine earns nothing after grace period
                }
            };
            if effective_score == 0 { return None; }
            Some((miner, effective_score))
        }).collect();

    // Storage: score = bytes_proven + query_bonus (up to 2× for active query traffic).
    // Nodes that include a valid Merkle range proof earn full score (T4-1, D10).
    // Nodes without a proof earn STORAGE_PROOF_NO_PROOF_BPS (20%) of normal score.
    let storage_nodes: Vec<(String, u64)> = chain.store.state_scan_prefix(&format!("storage_beat:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let bytes   = j["bytes_proven"].as_u64().unwrap_or(0);
            let queries = j["query_count"].as_u64().unwrap_or(0);
            let bonus   = (bytes as u128 * queries.min(100) as u128 / 100) as u64;
            let raw     = bytes.saturating_add(bonus);
            let proof_valid = j["proof_valid"].as_bool().unwrap_or(false);
            let score = if proof_valid { raw } else { raw * STORAGE_PROOF_NO_PROOF_BPS / 10_000 };
            if score == 0 { return None; }
            Some((j["node_id"].as_str()?.to_owned(), score))
        }).collect();

    // Sensors: type-aware sensor_score() per individual sensor, then grouped by owner.
    // Sensors with a registered location earn a 1.3× boost (T4-3, SENSOR_LOCATION_BOOST_BPS).
    // After SENSOR_LOCATION_REQUIRED_EPOCH, unlocated sensors are skipped entirely.
    let sensor_nodes: Vec<(String, u64)> = {
        let mut by_owner: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for (_, v) in chain.store.state_scan_prefix(&format!("sensor_commit:{}:", epoch)) {
            let Ok(j) = serde_json::from_slice::<serde_json::Value>(&v) else { continue };
            let Some(owner) = j["owner"].as_str() else { continue };
            let sensor_id     = j["sensor_id"].as_str().unwrap_or("");
            let reading_count = j["reading_count"].as_u64().unwrap_or(0);
            let stype         = j["sensor_type"].as_str().unwrap_or("continuous");
            let has_location  = chain.store.get_meta(&format!("sensor:{}", sensor_id))
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|s| s.get("location").and_then(|l| l.as_str()).map(|l| !l.is_empty()))
                .unwrap_or(false);
            if SENSOR_LOCATION_REQUIRED_EPOCH > 0 && epoch >= SENSOR_LOCATION_REQUIRED_EPOCH && !has_location {
                continue; // location required after mainnet month 3; skip if absent
            }
            let base  = sensor_score(reading_count, stype);
            let score = if has_location {
                (base as u128 * SENSOR_LOCATION_BOOST_BPS as u128 / 10_000) as u64
            } else { base };
            *by_owner.entry(owner.to_owned()).or_default() += score;
        }
        // Coverage reports: dead-spot reports earn more; corroborated cells earn 1.5× bonus.
        // Scores merged into sensor_nodes pool so coverage reporters share the sensor reward pool.
        use btcpc_types::{COVERAGE_SCORE_SIGNAL, COVERAGE_SCORE_DEAD_SPOT, COVERAGE_CORROBORATION_BONUS_BPS};
        for (_, v) in chain.store.state_scan_prefix(&format!("coverage_report:{}:", epoch)) {
            let Ok(j) = serde_json::from_slice::<serde_json::Value>(&v) else { continue };
            let Some(reporter) = j["reporter"].as_str() else { continue };
            let is_dead = j["is_dead_spot"].as_bool().unwrap_or(true);
            let lat_q   = j["lat_q"].as_i64().unwrap_or(0);
            let lon_q   = j["lon_q"].as_i64().unwrap_or(0);
            let carrier = j["carrier_mcc_mnc"].as_str().unwrap_or("");
            let base    = if is_dead { COVERAGE_SCORE_DEAD_SPOT } else { COVERAGE_SCORE_SIGNAL };
            let cell_key = format!("coverage_cell:{}:{}:{}:{}", epoch, lat_q, lon_q, carrier);
            let corroborated = chain.store.state_get(&cell_key)
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|c| c["corroborated"].as_bool())
                .unwrap_or(false);
            let score = if corroborated {
                (base as u128 * COVERAGE_CORROBORATION_BONUS_BPS as u128 / 10_000) as u64
            } else { base };
            *by_owner.entry(reporter.to_owned()).or_default() += score;
        }
        by_owner.into_iter().collect()
    };

    // Verifiers: sum of value_scores from approved verdicts only; zero → excluded
    let verifiers: Vec<(String, u64)> = chain.store.state_scan_prefix(&format!("infer_verify:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let score = j["value_score_total"].as_u64().unwrap_or(0);
            if score == 0 { return None; }
            Some((j["verifier"].as_str()?.to_owned(), score))
        }).collect();

    // Services: score = container_hours
    let service_nodes: Vec<(String, u64)> = chain.store.state_scan_prefix(&format!("service_beat:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            Some((j["node_id"].as_str()?.to_owned(), j["container_hours"].as_u64().unwrap_or(0)))
        }).collect();

    // Mempool: score inversely weighted by latency, proportional to relay throughput
    let mempool_ops: Vec<(String, u64)> = chain.store.state_scan_prefix(&format!("mempool_beat:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let latency  = j["latency_ms"].as_u64().unwrap_or(u64::MAX);
            let relayed  = j["entries_relayed"].as_u64().unwrap_or(0);
            if relayed == 0 { return None; }
            Some((j["operator"].as_str()?.to_owned(), mempool_relay_score(relayed, latency)))
        }).collect();

    // Tracker coverage: BLE observer nodes that submitted sighting commits this epoch.
    // Score = total sightings (all tag types count equally) treated as "event" sensor.
    // AcousticVerified observers get a 1.5× boost (witness_id present in any claim they hold).
    let tracker_nodes: Vec<(String, u64)> = {
        let mut by_observer: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for (_, v) in chain.store.state_scan_prefix(&format!("tracker_sighting:{}:", epoch)) {
            let Ok(j) = serde_json::from_slice::<serde_json::Value>(&v) else { continue };
            let Some(observer) = j["observer_id"].as_str() else { continue };
            let count = j["airtag_count"].as_u64().unwrap_or(0)
                + j["android_fmd_count"].as_u64().unwrap_or(0)
                + j["tile_count"].as_u64().unwrap_or(0)
                + j["samsung_count"].as_u64().unwrap_or(0)
                + j["other_count"].as_u64().unwrap_or(0);
            if count == 0 { continue; }

            // T4-4: physical-presence gate. The observer must hold at least one
            // TrackerClaim that reached "Verified" status via TrackerAcousticProof.
            // Observers with only "Registered" claims earn zero sighting reward —
            // desk-based fake sightings are excluded from the reward pool.
            let observer_claims: Vec<serde_json::Value> = chain.store
                .state_scan_prefix(&format!("tracker_claim:{}:", observer))
                .into_iter()
                .filter_map(|(_, cv)| serde_json::from_slice(&cv).ok())
                .collect();
            let has_verified = observer_claims.iter()
                .any(|cr| cr["status"].as_str() == Some("Verified"));
            if !has_verified { continue; }

            // "event" sensor model: 100 points per event, capped at 20 events (2_000).
            let base_score = sensor_score(count, "event");
            // Acoustic witness multiplier: 3/2 if observer is a witness on any AcousticVerified claim.
            let has_acoustic_witness = chain.store.state_scan_prefix("tracker_claim:")
                .into_iter()
                .any(|(_, cv)| {
                    let Ok(cr) = serde_json::from_slice::<serde_json::Value>(&cv) else { return false };
                    cr["status"].as_str() == Some("AcousticVerified")
                        && cr["witness_id"].as_str() == Some(observer)
                });
            let score = if has_acoustic_witness { base_score * 3 / 2 } else { base_score };
            *by_observer.entry(observer.to_owned()).or_default() += score;
        }
        by_observer.into_iter().collect()
    };

    // LinkGit builders: accounts that pushed at least one ref update this epoch.
    let linkgit_builders: Vec<(String, u64)> = {
        let idx_key = format!("linkgit:builders:{}", epoch);
        let builders: Vec<String> = chain.store.state_get(&idx_key)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        builders.into_iter().filter_map(|builder| {
            let build_key = format!("linkgit:build:{}:{}", epoch, builder);
            let push_count = chain.store.state_get(&build_key)
                .and_then(|b| <[u8; 8]>::try_from(b.as_slice()).ok())
                .map(u64::from_le_bytes)
                .unwrap_or(0);
            if push_count == 0 { return None; }
            Some((builder, push_count))
        }).collect()
    };

    // LinkGit: repos that were fetched by remote IPs this epoch.
    // Each entry is (repo_id, owner, serve_count).
    let linkgit_repos: Vec<(String, String, u64)> = {
        let index_key = format!("linkgit:served_repos:{}", epoch);
        let repo_ids: Vec<String> = chain.store.state_get(&index_key)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        repo_ids.into_iter().filter_map(|repo_id| {
            let count_key = format!("linkgit:serve_count:{}:{}", epoch, repo_id);
            let serve_count = chain.store.state_get(&count_key)
                .and_then(|b| <[u8; 8]>::try_from(b.as_slice()).ok())
                .map(u64::from_le_bytes)
                .unwrap_or(0);
            if serve_count == 0 { return None; }
            let repo_json: serde_json::Value = chain.store
                .state_get(&format!("linkgit:repo:{}", repo_id))
                .and_then(|b| serde_json::from_slice(&b).ok())?;
            let owner = repo_json["owner"].as_str()?.to_owned();
            Some((repo_id, owner, serve_count))
        }).collect()
    };

    // ── Layer B: calibration-normalized utilization + activity-gating + scarcity ─
    let inference_total: u64 = mines.iter().map(|(_, s)| s).sum();
    let storage_total:   u64 = storage_nodes.iter().map(|(_, s)| s).sum();
    let sensor_total:    u64 = sensor_nodes.iter().map(|(_, s)| s).sum();
    let verify_total:    u64 = verifiers.iter().map(|(_, s)| s).sum();
    let service_total:   u64 = service_nodes.iter().map(|(_, s)| s).sum();
    let mempool_total:   u64 = mempool_ops.iter().map(|(_, s)| s).sum();
    let tracker_total:   u64 = tracker_nodes.iter().map(|(_, s)| s).sum();
    let linkgit_total:       u64 = linkgit_repos.iter().map(|(_, _, s)| s).sum();
    let linkgit_build_total: u64 = linkgit_builders.iter().map(|(_, s)| s).sum();

    // Normalize each pool's raw score by its calibration target → [0, 1.0]
    let norm = |work: u64, target: u64| -> u64 {
        if work == 0 { return 0; }
        ((work as u128 * ACTIVITY_RATIO_DENOM as u128) / target as u128)
            .min(ACTIVITY_RATIO_DENOM as u128) as u64
    };
    let util_inference = norm(inference_total, CALIBRATION_INFERENCE);
    let util_storage   = norm(storage_total,   CALIBRATION_STORAGE);
    let util_sensor    = norm(sensor_total,     CALIBRATION_SENSOR);
    let util_verify    = norm(verify_total,     CALIBRATION_VERIFIER);
    let util_service   = norm(service_total,    CALIBRATION_SERVICE);
    let util_mempool   = norm(mempool_total,    CALIBRATION_MEMPOOL);
    let util_tracker   = norm(tracker_total,    CALIBRATION_TRACKER);
    let util_linkgit   = norm(linkgit_total,       CALIBRATION_LINKGIT);
    let util_build     = norm(linkgit_build_total, CALIBRATION_LINKGIT_BUILD);
    let total_util     = util_inference + util_storage + util_sensor
                       + util_verify + util_service + util_mempool + util_tracker
                       + util_linkgit + util_build;

    // Activity ratio = average utilization across active pools
    let active_pool_count = [util_inference, util_storage, util_sensor,
                             util_verify, util_service, util_mempool, util_tracker,
                             util_linkgit, util_build]
        .iter().filter(|&&u| u > 0).count() as u64;
    let activity_ratio_num = if total_util == 0 {
        0
    } else {
        (total_util / active_pool_count).max(MIN_ACTIVITY_RATIO_NUM)
    };
    let gated_pool   = (activity_pool as u128 * activity_ratio_num as u128 / ACTIVITY_RATIO_DENOM as u128) as u64;
    let idle_recycle = activity_pool.saturating_sub(gated_pool);
    if idle_recycle > 0 {
        let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, idle_recycle);
    }

    // Budget allocation: proportional to each pool's normalized utilization
    let alloc_pool = |util: u64| -> u64 {
        if total_util == 0 { return 0; }
        (gated_pool as u128 * util as u128 / total_util as u128) as u64
    };

    let inference_pool_raw = alloc_pool(util_inference);
    let storage_pool_raw   = alloc_pool(util_storage);
    let sensor_pool_raw    = alloc_pool(util_sensor);
    let verify_pool_raw    = alloc_pool(util_verify);
    let service_pool_raw   = alloc_pool(util_service);
    let mempool_pool_raw   = alloc_pool(util_mempool);
    let tracker_pool_raw   = alloc_pool(util_tracker);
    let linkgit_pool_raw   = alloc_pool(util_linkgit);
    let build_pool_raw     = alloc_pool(util_build);

    // Scarcity divider: each pool pays at reduced rate if participant count < critical mass.
    // Remainder flows to recycle — prevents sparse networks from extracting full rewards.
    // critical_mass = 0 means EMA hasn't warmed up yet; treat as no gate (full payout).
    let payout_factor = |count: usize, critical_mass: u64| -> u64 {
        if critical_mass == 0 { return ACTIVITY_RATIO_DENOM; }
        ((count as u64).min(critical_mass) * ACTIVITY_RATIO_DENOM / critical_mass)
    };
    let apply_scarcity = |pool: u64, factor: u64| -> (u64, u64) {
        let effective = (pool as u128 * factor as u128 / ACTIVITY_RATIO_DENOM as u128) as u64;
        (effective, pool.saturating_sub(effective))
    };

    let (inference_pool, scatter_infer)  = apply_scarcity(inference_pool_raw, payout_factor(mines.len(),        cm_inference));
    let (storage_pool,   scatter_store)  = apply_scarcity(storage_pool_raw,   payout_factor(storage_nodes.len(), cm_storage));
    let (sensor_pool,    scatter_sensor) = apply_scarcity(sensor_pool_raw,    payout_factor(sensor_nodes.len(),  cm_sensor));
    let (verify_pool,    scatter_verify) = apply_scarcity(verify_pool_raw,    payout_factor(verifiers.len(),     cm_verifier));
    let (service_pool,   scatter_svc)   = apply_scarcity(service_pool_raw,    payout_factor(service_nodes.len(), cm_service));
    let (mempool_pool,   scatter_mem)   = apply_scarcity(mempool_pool_raw,    payout_factor(mempool_ops.len(),   cm_mempool));
    let (tracker_pool,   scatter_track) = apply_scarcity(tracker_pool_raw,    payout_factor(tracker_nodes.len(), cm_tracker));
    let (linkgit_pool,   scatter_git)   = apply_scarcity(linkgit_pool_raw,    payout_factor(linkgit_repos.len(),    cm_linkgit));
    let (build_pool,     scatter_build) = apply_scarcity(build_pool_raw,      payout_factor(linkgit_builders.len(), cm_build));

    let scarcity_recycle = scatter_infer + scatter_store + scatter_sensor
                         + scatter_verify + scatter_svc + scatter_mem + scatter_track
                         + scatter_git + scatter_build;
    if scarcity_recycle > 0 {
        let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, scarcity_recycle);
    }

    distribute_rewards_desktop(epoch, &mines, inference_pool, chain, cmd_tx, |miner, amount, ep| {
        LedgerEntry::MineReward { miner, amount, epoch: ep }
    }).await;
    distribute_rewards_desktop(epoch, &storage_nodes, storage_pool, chain, cmd_tx, |node_id, amount, ep| {
        LedgerEntry::StorageReward { node_id, amount, epoch: ep }
    }).await;
    distribute_rewards_desktop(epoch, &sensor_nodes, sensor_pool, chain, cmd_tx, |node_id, amount, ep| {
        LedgerEntry::SensorReward { node_id, amount, epoch: ep }
    }).await;
    distribute_rewards_desktop(epoch, &verifiers, verify_pool, chain, cmd_tx, |node_id, amount, ep| {
        LedgerEntry::VerifierReward { node_id, amount, epoch: ep }
    }).await;
    distribute_rewards_desktop(epoch, &service_nodes, service_pool, chain, cmd_tx, |node_id, amount, ep| {
        LedgerEntry::ServiceReward { node_id, amount, epoch: ep }
    }).await;
    distribute_rewards_desktop(epoch, &mempool_ops, mempool_pool, chain, cmd_tx, |operator, amount, ep| {
        LedgerEntry::MempoolReward { operator, amount, epoch: ep }
    }).await;
    distribute_rewards_desktop(epoch, &tracker_nodes, tracker_pool, chain, cmd_tx, |observer_id, amount, ep| {
        LedgerEntry::TrackerCoverageReward { observer_id, amount, epoch: ep }
    }).await;

    // LinkGit serve rewards: split linkgit_pool proportionally across repos by serve count.
    if linkgit_pool > 0 && !linkgit_repos.is_empty() {
        for (repo_id, owner, serve_count) in &linkgit_repos {
            let amount = (linkgit_pool as u128 * *serve_count as u128 / linkgit_total as u128) as u64;
            if amount == 0 { continue; }
            let entry = LedgerEntry::LinkGitServeReward {
                repo_id: repo_id.clone(),
                owner: owner.clone(),
                amount,
                serve_count: *serve_count,
                epoch,
            };
            match chain.apply_entry(&entry) {
                Ok(()) => broadcast_entry_desktop(&entry, cmd_tx).await,
                Err(e) => warn!("linkgit serve reward failed for {}: {}", repo_id, e),
            }
        }
    }

    // LinkGit build rewards: split build_pool proportionally by push count across builders.
    distribute_rewards_desktop(epoch, &linkgit_builders, build_pool, chain, cmd_tx, |builder, amount, ep| {
        let push_count = linkgit_builders.iter()
            .find(|(b, _)| b == &builder)
            .map(|(_, c)| *c)
            .unwrap_or(0);
        LedgerEntry::LinkGitBuildReward { builder, amount, push_count, epoch: ep }
    }).await;

    // ── T3-4: Update EMA critical-mass targets for next epoch ────────────────
    // alpha = EMA_ALPHA_NUM / EMA_ALPHA_DENOM ≈ 2/20161 ≈ 7-day smoothing window.
    let update_ema_cm = |pool: &str, current: u64, prev: u64| {
        let new_ema = (EMA_ALPHA_NUM * current + (EMA_ALPHA_DENOM - EMA_ALPHA_NUM) * prev) / EMA_ALPHA_DENOM;
        let _ = chain.store.state_set(
            &format!("ema_critical_mass:{}", pool),
            &serde_json::to_vec(&serde_json::json!({ "ema": new_ema })).unwrap_or_default(),
        );
    };
    update_ema_cm("inference", mines.len()        as u64, cm_inference);
    update_ema_cm("storage",   storage_nodes.len() as u64, cm_storage);
    update_ema_cm("sensor",    sensor_nodes.len()  as u64, cm_sensor);
    update_ema_cm("verifier",  verifiers.len()     as u64, cm_verifier);
    update_ema_cm("service",   service_nodes.len() as u64, cm_service);
    update_ema_cm("mempool",   mempool_ops.len()   as u64, cm_mempool);
    update_ema_cm("tracker",   tracker_nodes.len() as u64, cm_tracker);
    update_ema_cm("linkgit",       linkgit_repos.len()    as u64, cm_linkgit);
    update_ema_cm("linkgit_build", linkgit_builders.len() as u64, cm_build);

    // ── Subscription fee release ───────────────────────────────────────────────
    sweep_tracker_subscription_fees(epoch, chain, cmd_tx).await;

    let distributed  = inference_pool + storage_pool + sensor_pool
                     + verify_pool + service_pool + mempool_pool + tracker_pool
                     + linkgit_pool + build_pool;
    let remainder    = gated_pool.saturating_sub(distributed).saturating_sub(scarcity_recycle);
    if remainder > 0 {
        let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, remainder);
    }

    info!(
        "clock: epoch {} raw={} reserve={} gated={}({:.0}%) scarcity_recycle={} \
         [infer={}/{} store={}/{} sensor={}/{} verify={}/{} svc={}/{} mem={}/{} tracker={}/{} git={}/{} build={}/{}]",
        epoch, raw_pool, reserve_total, gated_pool,
        activity_ratio_num as f64 / ACTIVITY_RATIO_DENOM as f64 * 100.0,
        scarcity_recycle + idle_recycle,
        inference_pool, mines.len(),
        storage_pool, storage_nodes.len(),
        sensor_pool, sensor_nodes.len(),
        verify_pool, verifiers.len(),
        service_pool, service_nodes.len(),
        mempool_pool, mempool_ops.len(),
        tracker_pool, tracker_nodes.len(),
        linkgit_pool, linkgit_repos.len(),
        build_pool, linkgit_builders.len(),
    );

    // ── Protocol benchmark jobs (D8 grace period) ─────────────────────────────
    // Post MINE_GRACE_BENCHMARK_JOBS_PER_EPOCH jobs from __testnet_fund__ so miners
    // have real demand to serve during the 90-day grace period (D16: open model selection).
    if in_grace_period {
        for i in 0..MINE_GRACE_BENCHMARK_JOBS_PER_EPOCH {
            let job_id = format!("benchmark:{}:{}", epoch, i);
            // Only post if this job hasn't been posted yet (idempotent).
            let job_key = format!("job:{}", job_id);
            if chain.store.state_get(&job_key).is_some() { continue; }
            let max_fee: u64 = 1_000_000; // 0.0001 BTCPC — small fee from testnet fund
            let fund = chain.store.get_balance(TESTNET_FUND_ACCOUNT, NATIVE_TOKEN);
            if fund < max_fee { break; }
            let entry = LedgerEntry::InferenceJobPost {
                job_id: job_id.clone(),
                requester: TESTNET_FUND_ACCOUNT.to_owned(),
                model: "auto".to_owned(),
                mode: "solo".to_owned(),
                input_hash: format!("{:064x}", epoch * 1000 + i),
                max_fee,
                min_reputation: 0,
                bid_window_epochs: 2,
                deadline_epoch: epoch + 10,
                epoch,
                nonce: epoch * 1000 + i,
                signed_by: TESTNET_FUND_ACCOUNT.to_owned(),
                persist_on_fs: None,
                fs_fee: None,
                min_verifiers: None,
                node_fingerprint: None,
            };
            if let Err(e) = chain.apply_entry(&entry) {
                warn!("benchmark job {} failed: {}", job_id, e);
            } else {
                broadcast_entry_desktop(&entry, cmd_tx).await;
            }
        }
    }

    // ── Inference proof pruning (D1) ──────────────────────────────────────────
    // After InferenceJobPay + INFERENCE_PRUNE_WINDOW_EPOCHS, clear proof payload fields.
    // Verdict and reward records are kept forever; only the raw proof data is cleared.
    if epoch >= INFERENCE_PRUNE_WINDOW_EPOCHS {
        let prune_epoch = epoch - INFERENCE_PRUNE_WINDOW_EPOCHS;
        let prefix = format!("prune_ready:{}", prune_epoch);
        for (key, val) in chain.store.state_scan_prefix(&prefix) {
            let job_id = match std::str::from_utf8(&val).ok().filter(|s| !s.is_empty()) {
                Some(s) => s.to_owned(),
                None => continue,
            };
            let job_key = format!("job:{}", job_id);
            if let Some(raw) = chain.store.state_get(&job_key) {
                if let Ok(mut j) = serde_json::from_slice::<serde_json::Value>(&raw) {
                    // Null out payload fields but preserve verdict/reward structure.
                    j["output_hash"]   = serde_json::json!(null);
                    j["input_hash"]    = serde_json::json!(null);
                    j["result_hash"]   = serde_json::json!(null);
                    j["pruned_epoch"]  = serde_json::json!(epoch);
                    if let Ok(bytes) = serde_json::to_vec(&j) {
                        let _ = chain.store.state_set(&job_key, &bytes);
                    }
                }
            }
            let _ = chain.store.state_delete(&key);
        }
    }
}

async fn broadcast_entry_desktop(
    entry: &LedgerEntry,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
) {
    let envelope = serde_json::json!({"entry": entry});
    if let Ok(data) = serde_json::to_vec(&envelope) {
        let _ = cmd_tx.send(NetCmd::Broadcast { topic: "btcpc/entries", data }).await;
    }
}

fn sweep_expired_pending_transfers(epoch: u64, chain: &Arc<Chain>) {
    let prefix = "pending_transfer:";
    let entries = chain.store.state_scan_prefix(prefix);
    for (key, raw) in entries {
        let Ok(j) = serde_json::from_slice::<serde_json::Value>(&raw) else { continue };
        let expires = j["expires_epoch"].as_u64().unwrap_or(0);
        if expires > epoch { continue; }
        let from = j["from"].as_str().unwrap_or("").to_owned();
        let token = j["token"].as_str().unwrap_or("").to_owned();
        let amount = j["amount"].as_u64().unwrap_or(0);
        if from.is_empty() || token.is_empty() || amount == 0 { continue; }
        let _ = chain.store.state_delete(&key);
        let _ = chain.store.credit(&from, &token, amount);
    }
}

/// Release per-epoch subscription fees from escrow → observers (50%) + treasury (15%) + recycle (35%).
/// Called once per epoch seal after all other reward distributions.
async fn sweep_tracker_subscription_fees(
    epoch: u64,
    chain: &Arc<Chain>,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
) {
    let escrows = chain.store.state_scan_prefix("tracker_sub_escrow:");
    for (escrow_key, raw) in escrows {
        let Ok(mut rec) = serde_json::from_slice::<serde_json::Value>(&raw) else { continue };
        let expires  = rec["expires_epoch"].as_u64().unwrap_or(0);
        let start    = rec["start_epoch"].as_u64().unwrap_or(0);
        let fee      = rec["fee_per_epoch"].as_u64().unwrap_or(0);
        let escrowed = rec["total_escrowed"].as_u64().unwrap_or(0);
        let paid_out = rec["paid_out"].as_u64().unwrap_or(0);

        if expires < epoch || fee == 0 || paid_out >= escrowed { continue; }
        if epoch < start { continue; }

        let this_fee = fee.min(escrowed.saturating_sub(paid_out));
        if this_fee == 0 { continue; }

        // Split: 50% to observers who pushed data this epoch, 35% recycle, 15% treasury.
        let serial = match rec["serial_commitment"].as_str() { Some(s) => s.to_owned(), None => continue };
        let observer_cut = this_fee * 50 / 100;
        let treasury_cut = this_fee * 15 / 100;
        let recycle_cut  = this_fee.saturating_sub(observer_cut).saturating_sub(treasury_cut);

        // Find observers who pushed TrackerSightingData for this serial this epoch.
        let route_prefix = format!("tracker_route:{}:{:016x}", serial, epoch);
        let sighting_data: Vec<String> = chain.store.state_scan_prefix(&route_prefix)
            .into_iter()
            .filter_map(|(_, v)| {
                let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
                j["observer_id"].as_str().map(str::to_owned)
            })
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let n = sighting_data.len() as u64;
        if n > 0 {
            let per_observer = observer_cut / n;
            for obs in &sighting_data {
                if per_observer == 0 { break; }
                let entry = LedgerEntry::TrackerCoverageReward {
                    observer_id: obs.clone(),
                    amount: per_observer,
                    epoch,
                };
                if let Err(e) = chain.apply_entry(&entry) {
                    warn!("tracker sub fee: observer credit failed for {} epoch {}: {}", obs, epoch, e);
                    continue;
                }
                broadcast_entry_desktop(&entry, cmd_tx).await;
            }
        } else {
            // No observers pushed data — treat observer cut as recycle too.
            chain.store.credit(btcpc_types::RECYCLE_FUND_ACCOUNT, btcpc_types::NATIVE_TOKEN, observer_cut);
        }

        if treasury_cut > 0 {
            chain.store.credit("treasury", btcpc_types::NATIVE_TOKEN, treasury_cut);
        }
        if recycle_cut > 0 {
            chain.store.credit(btcpc_types::RECYCLE_FUND_ACCOUNT, btcpc_types::NATIVE_TOKEN, recycle_cut);
        }

        // Update escrow paid_out.
        rec["paid_out"] = serde_json::json!(paid_out + this_fee);
        let _ = chain.store.state_set(&escrow_key, &serde_json::to_vec(&rec).unwrap_or_default());
    }
}

async fn distribute_rewards_desktop<F>(
    epoch: u64,
    nodes: &[(String, u64)],
    pool: u64,
    chain: &Arc<Chain>,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
    make_entry: F,
) where
    F: Fn(String, u64, u64) -> LedgerEntry,
{
    if nodes.is_empty() || pool == 0 { return; }
    let total: u64 = nodes.iter().map(|(_, s)| s).sum();
    let n = nodes.len() as u64;
    for (account, score) in nodes {
        let amount = if total == 0 { pool / n }
            else { (pool as u128 * *score as u128 / total as u128) as u64 };
        if amount == 0 { continue; }
        let entry = make_entry(account.clone(), amount, epoch);
        if let Err(e) = chain.apply_entry(&entry) {
            warn!("clock: reward apply failed for {} epoch {}: {}", account, epoch, e);
            continue;
        }
        let envelope = serde_json::json!({"entry": entry});
        if let Ok(data) = serde_json::to_vec(&envelope) {
            let _ = cmd_tx.send(NetCmd::Broadcast { topic: "btcpc/entries", data }).await;
        }
    }
}

// ── Key helpers ───────────────────────────────────────────────────────────────

fn load_signing_key(hex_seed: &str) -> Option<ed25519_dalek::SigningKey> {
    let bytes = hex::decode(hex_seed.trim()).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(ed25519_dalek::SigningKey::from_bytes(&arr))
}

// ── Mempool heartbeat daemon ──────────────────────────────────────────────────

async fn run_mempool_node(
    chain:          Arc<Chain>,
    account:        String,
    genesis_ts:     u64,
    cmd_tx:         tokio::sync::mpsc::Sender<NetCmd>,
    relay_counter:  Arc<AtomicUsize>,
) {
    use btcpc_types::EPOCH_MS;
    info!("mempool node started: account={}", account);

    let elapsed = utils::now_ms().saturating_sub(genesis_ts);
    let mut last_epoch = elapsed / EPOCH_MS;

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        let elapsed = utils::now_ms().saturating_sub(genesis_ts);
        let epoch = elapsed / EPOCH_MS;
        if epoch <= last_epoch { continue; }
        last_epoch = epoch;

        // Consume the relay counter atomically — no double-counting across epochs.
        let entries_relayed = relay_counter.swap(0, Ordering::Relaxed) as u64;
        if entries_relayed == 0 {
            // No entries relayed this epoch — skip heartbeat; idle nodes don't earn.
            continue;
        }

        // P99 latency estimate: 200 ms represents a well-connected broadband relay node.
        // Measured propagation timing would require per-message timestamping; this is a
        // conservative honest estimate for a direct gossipsub participant.
        let propagation_latency_ms: u64 = 200;

        let entry = LedgerEntry::MempoolHeartbeat {
            operator:               account.clone(),
            epoch,
            propagation_latency_ms,
            entries_relayed,
            signed_by:              account.clone(),
        };

        if let Err(e) = chain.apply_entry(&entry) {
            warn!("mempool: heartbeat failed epoch {}: {}", epoch, e);
            continue;
        }

        let envelope = serde_json::json!({"entry": entry});
        if let Ok(data) = serde_json::to_vec(&envelope) {
            let _ = cmd_tx.send(NetCmd::Broadcast {
                topic: "btcpc/entries",
                data,
            }).await;
        }

        info!("mempool: heartbeat epoch {} relayed={} latency={}ms",
            epoch, entries_relayed, propagation_latency_ms);
    }
}
