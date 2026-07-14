/*!
# hone-node

HONE sovereign chain node — single binary for networking, consensus,
state machine, block production, contract execution, and HTTP API.

## Environment Variables

    HONE_DATA_DIR        — RocksDB data directory (default: ~/.hone)
    HONE_ACCOUNT         — this node's account name
    HONE_NODE_ID         — libp2p node identity label
    HONE_API_PORT        — HTTP API port (default: 4242)
    HONE_P2P_PORT        — libp2p listen port (default: 6942)
    HONE_MINER               — "true" to enable mining
    HONE_CLOCK               — "true" to participate in clock consensus
    HONE_GENESIS_FILE        — path to genesis.json
    HONE_GENESIS_TIMESTAMP   — Unix ms timestamp for genesis block (MUST match on all nodes)
    HONE_LOG_LEVEL           — tracing filter (default: hone_node=info)
    HONE_BOOTSTRAP_PEERS     — comma-separated multiaddrs for DHT bootstrap
    HONE_ISOLATED            — "true" = no public discovery (no honemesh.net fetch/announce,
                                no DNS-seed fallback, ignore mDNS); dial only HONE_BOOTSTRAP_PEERS.
                                For self-contained N-clock consensus tests. NOT for production.
    HONE_SIM                 — "false" to disable the testnet synthetic-traffic sim daemon
    HONE_CHAIN_ID            — "hone" (mainnet) or "hone-testnet" (testnet)
    HONE_POSTING_KEY         — wallet secret: 12-word BIP-39 mnemonic OR 64-char hex (treated
                                as 32-byte BIP-39 entropy, NOT a raw ed25519 seed). The wallet
                                derives the posting role key at SLIP-10 m/44'/6942'/2'/0'; that
                                same key signs all clock seals, so signer == on-chain identity
    HONE_STORAGE             — "true" to emit StorageHeartbeat each epoch (earns StorageReward)
    HONE_ETH_RPC             — Ethereum JSON-RPC endpoint for ENS resolution (default: cloudflare-eth.com)
    HONE_SERVICE             — "true" to emit ServiceHeartbeat each epoch (earns ServiceReward)
    HONE_SENSOR              — "true" to emit SensorDataCommit each epoch (earns SensorReward)
    HONE_MEMPOOL             — "true" to emit MempoolHeartbeat each epoch (earns MempoolReward)
    HONE_SECRETS_PASSPHRASE  — AES-256-GCM passphrase for secrets.enc (default: hw_fingerprint)
    HONE_AUTO_UPDATE         — "1" to enable auto-update from HONE_UPDATE_URL
    HONE_UPDATE_URL          — URL to poll for binary updates (requires HONE_AUTO_UPDATE=1)
    HONE_ALERT_WEBHOOK       — HTTP POST URL for health alert notifications
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
mod inference_engine;
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
mod scientific;
mod cross_chain_finality;
mod system_contracts;
mod throttle;
mod ens;
mod linkgit_server;
mod storage_sync;
mod secret_store;
mod monitor;
mod updater;
mod totp;
mod work_generator;
mod ensemble;
mod slashing;
mod bridge;
mod oracle;
mod ton_relay;
mod session_market;
mod agent_session;
mod agent_task;
mod agent_tools;
mod agent_worker;
mod agent_registry;
mod vrf;
mod auction;
mod private_auth;
mod memory_service;
mod amber_pill;
mod blob_bandwidth;
mod blob_serve;
mod hive_replica;
mod peer_commerce;
mod gateway_resolver;
mod snapshot_replication;
mod phone_storage;
mod finetune;
mod computer_use;
mod rag;
mod hardware_probe;
mod udp_gossip;
mod tor;
mod nostr_transport;
#[cfg(feature = "matrix")]
mod matrix_transport;
mod i2p;
mod lorawan;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use anyhow::Result;
use tracing::{error, info, warn};
use hone_types::{
    LedgerEntry, block_reward_at, era, RECYCLE_ERA, RECYCLE_REWARD_RATE, RECYCLE_REWARD_DENOM,
    RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, TESTNET_CHAIN_ID, TESTNET_FUND_ACCOUNT, TREASURY_ACCOUNT,
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
    LAYER_A_SLOW_ALPHA_NUM, LAYER_A_SLOW_ALPHA_DENOM,
    LAYER_A_SCALAR_DENOM, LAYER_A_POOL_COUNT,
    layer_a_scalar,
    LAYER_C_FEE_BOOST_NUM, LAYER_C_FEE_BOOST_DENOM,
    entry_weight, compute_next_base_fee, BASE_FEE_INITIAL_HUNITS, EPOCH_TARGET_WEIGHT_UNITS,
};

use chain::Chain;
use config::Config;
use contracts::ContractEngine;
use net::{NetCmd, NetworkEvent};
use store::Store;

#[tokio::main]
async fn main() -> Result<()> {
    // Install ring as the rustls 0.23 crypto provider before any TLS code runs.
    // Required when multiple providers (ring + aws-lc-rs) are in the dep tree.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cfg = Config::from_env();

    // Prevent multiple instances on the same machine.
    // Port pre-check works everywhere including WSL↔Windows cross-instance conflicts,
    // since WSL2 and Windows share the localhost port namespace.
    {
        use std::net::TcpListener;
        let probe_port = cfg.api_port;
        match TcpListener::bind(("127.0.0.1", probe_port)) {
            Ok(_) => {} // port is free; the listener is immediately dropped
            Err(_) => {
                eprintln!("hone-node: port {} is already in use — another instance may be running", probe_port);
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
            eprintln!("hone-node: another instance is already running (lock: {:?})", lock_path);
            std::process::exit(1);
        }
        lf // keep alive until process exits
    };

    // Signing key is derived from the wallet's posting role key below, AFTER
    // wallet::init() — so the seal-signing identity is byte-identical to the
    // posting key registered on-chain. (Historically this parsed HONE_POSTING_KEY
    // as a raw ed25519 seed here, which produced a DIFFERENT key than the wallet's
    // SLIP-10 m/44'/6942'/2'/0' posting key — seals signed by an identity that
    // never matched genesis. See reports/DRYRUN_2CLOCK_2026-07-10.md, BUG 1.)

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| cfg.log_level.parse().unwrap_or_default()),
        )
        .init();

    info!("hone-node starting — account={} chain={} data={:?}", cfg.account, cfg.chain_id, cfg.data_dir);

    // Wallet: restore / load / generate all chain keys (HONE, Bitcoin, Ethereum).
    let mut wallet_keys = wallet::init(&cfg.data_dir)?;
    // Backfill ton_address if missing (async call to tonapi.io).
    wallet::ensure_ton_address(&cfg.data_dir, &mut wallet_keys).await;
    // Keep ~/.hone/{account}.wallet.key in sync so TUI/CLI can find keys without knowing data_dir.
    wallet::backup_to_home(&cfg.account, &wallet_keys);

    // Seal-signing key = the wallet's posting role key (SLIP-10 m/44'/6942'/2'/0').
    // This is the SAME key registered on-chain as the account's posting key, so
    // seals and clock self-registration are signed by the on-chain identity —
    // not a raw-seed reinterpretation of HONE_POSTING_KEY (BUG 1).
    let signing_key: Arc<Option<ed25519_dalek::SigningKey>> = Arc::new(
        load_signing_key(&wallet_keys.hone_private_key)
    );
    let signing_pubkey_hex: Arc<Option<String>> = Arc::new(
        signing_key.as_ref().as_ref().map(|sk| hex::encode(sk.verifying_key().to_bytes()))
    );

    // A clock with no usable signing key can seal in-memory but can never
    // self-register or produce verifiable seals matching its genesis identity.
    // Fail loud instead of silently booting a useless clock (BUG 1).
    if cfg.is_clock && signing_key.is_none() {
        error!(
            "[clock] no usable posting/signing key for account '{}' — refusing to start. \
             The wallet's posting key (hone_private_key) did not parse as a 32-byte ed25519 seed. \
             Check HONE_POSTING_KEY (12-word mnemonic or 64-char hex) and the wallet.key it produced.",
            cfg.account
        );
        std::process::exit(1);
    }

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

    // Resource throttling: automatically drop CPU/GPU usage when the operator
    // is actively using this PC, so HONE never makes the machine feel
    // unusable to its owner. HONE_THROTTLE_LOW_PERCENT sets the target
    // (10-50, default 30); set HONE_THROTTLE_DISABLED=1 to opt out entirely
    // (e.g. dedicated mining/server boxes with no interactive user).
    if std::env::var("HONE_THROTTLE_DISABLED").ok().as_deref() != Some("1") {
        let low_percent: u8 = std::env::var("HONE_THROTTLE_LOW_PERCENT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);
        throttle::spawn_watcher(low_percent);
    }

    // Secret store: encrypted local file + RocksDB runtime index.
    let secret_store_arc: Arc<secret_store::SecretStore> = Arc::new(
        secret_store::SecretStore::open(&cfg.data_dir, &hw.fingerprint, chain.store.clone())
            .unwrap_or_else(|e| {
                warn!("[secrets] init failed ({}), starting with empty store", e);
                secret_store::SecretStore::open(&cfg.data_dir, "fallback", chain.store.clone())
                    .expect("fallback secret store open")
            })
    );
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

    // Store server pubkey for agent session ECDH (node's own ed25519 verifying key).
    if let Some(sk) = signing_key.as_ref().as_ref() {
        let pubkey_hex = hex::encode(sk.verifying_key().to_bytes());
        let _ = chain.store.state_set("node:server_pubkey", pubkey_hex.as_bytes());
    }

    info!("chain state ready — latest epoch={}", chain.current_epoch());
    tokio::spawn(async move {
        if let Err(e) = network.run().await {
            tracing::error!("network error: {}", e);
        }
    });

    // ── Self-announce to Hive + honemesh.net (best-effort, fire-and-forget) ───────
    // Skipped in isolated mode — an isolated node must make no outbound discovery
    // calls, so it neither registers itself nor pulls in live-network peers.
    if !cfg.isolated {
        let chain_id = cfg.chain_id.clone();
        let node_id = cfg.node_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            tokio::join!(
                discovery::announce_to_hive(&chain_id, &node_id),
                discovery::announce_to_hone_net(&chain_id, &node_id),
            );
        });
    } else {
        info!("[isolated] HONE_ISOLATED=true — skipping public discovery announce");
    }

    // ── Clock consensus ───────────────────────────────────────────────────────
    let clock = Arc::new(clock::ClockConsensus::new());
    // Populate registered_clocks immediately from store so quorum is correct from epoch 1.
    clock.set_registered_clocks(clock::registered_clock_nodes(&chain.store, chain.current_epoch()));
    {
        let clock_ref = clock.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(1_000)).await;
                clock_ref.tick();
            }
        });
    }

    // ── LoRaWAN transport — Phase 8 transport cascade ────────────────────────
    // Must be started before the seal handler so the handle can be captured.
    // LoRaWAN peers do NOT count toward peer_count — libp2p-only hardline intact.
    let lorawan_handle =
        lorawan::start_lorawan(chain.clone(), net_handle.cmd_tx.clone()).await;

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
        let lorawan_for_seal = lorawan_handle.clone();
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
                                    Ok(_) => {
                                        applied += 1;
                                        // Forward accepted non-system entry hashes to LoRaWAN
                                        // devices on the next downlink window.
                                        if !tx::is_system_entry(e) {
                                            if let Some(ref lh) = lorawan_for_seal {
                                                if let Ok(hash_bytes) = hex::decode(e.hash()) {
                                                    if let Ok(arr) = hash_bytes.try_into() {
                                                        lh.queue_hash(arr);
                                                    }
                                                }
                                            }
                                        }
                                    }
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
                                        "hone-node: hardware conflict at epoch {} — fingerprint \
                                         claimed by '{}'. Only one HONE account per physical \
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

                        // Release matured unbonding tokens, apply timelocked param changes,
                        // and finalize any governance proposals whose voting window closed.
                        chain_ref.drain_unbonding(sealed_epoch);
                        chain_ref.drain_pending_params(sealed_epoch);
                        chain_ref.drain_governance_proposals(sealed_epoch);

                        // Dynamic base fee update (EIP-1559-style ±10%/epoch toward 50% target).
                        let current_base_fee: u64 = chain_ref.store.state_get("chain_param:base_fee")
                            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                            .and_then(|j| j["fee"].as_u64())
                            .unwrap_or(BASE_FEE_INITIAL_HUNITS);
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
                            use hone_types::VERIFIER_ACTIVITY_TRACK_EPOCHS;
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
                        let reg_clocks = clock::registered_clock_nodes(&chain_ref.store, sealed_epoch);
                        let reg_count = reg_clocks.len();
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

                        // REWARD FINALIZATION (BUG 6): rewards are NO LONGER emitted here at
                        // seal time off this node's LOCAL view of who sealed. Paying per-node
                        // at seal forks balances whenever two nodes cross the quorum threshold
                        // in different epochs (each pays a different set) — the fork we chased
                        // in DRYRUN_2CLOCK. Instead, emission is deferred to the finalized-epoch
                        // handler below, which fires only once this epoch's rewards_hash reaches
                        // quorum agreement across nodes, and pays from the DETERMINISTIC
                        // epoch_validators:{epoch} snapshot (identical on every node) — so all
                        // nodes emit byte-identical ClockReward and balances stay convergent.
                        // (reg_count kept for the diagnostic below.)
                        let quorum = clock::quorum();
                        if reg_count < quorum {
                            info!(
                                "[clock] epoch {}: {} registered clock(s) < quorum {} — sealing only, \
                                 rewards defer to finalization once quorum agrees",
                                sealed_epoch, reg_count, quorum
                            );
                        }

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
                                topic: "hone/consensus",
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
                        // BUG 6 FIX: emit this epoch's rewards HERE — only after its
                        // rewards_hash reached quorum agreement (that is what fired this
                        // event). Pay from the DETERMINISTIC epoch_validators:{epoch}
                        // snapshot (the store-derived registered clock set, persisted at
                        // seal and identical on every node) rather than any node's local
                        // signing_clocks view. Every node therefore emits byte-identical
                        // ClockReward for the epoch, so balances (and the state_root over
                        // them) stay convergent. Idempotency: emit_epoch_rewards writes
                        // ClockReward entries keyed by epoch; a finalize is fired once per
                        // epoch (state.finalized guard in receive_reward_proposal), so this
                        // runs once per epoch per node.
                        let sealers: Vec<String> = chain_ref.store
                            .state_get(&format!("epoch_validators:{}", fin.epoch))
                            .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
                            .unwrap_or_default();
                        if !sealers.is_empty() {
                            emit_epoch_rewards(fin.epoch, &sealers, &chain_ref, &cmd_tx_fin).await;
                        }

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
                                topic: "hone/entries",
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
    // Holds this clock's own signed ClockNodeRegister envelope so the seal loop can
    // re-announce it to peers that connect AFTER the one-shot boot broadcast (which
    // hits an empty gossip mesh and is dropped, never retried). Populated on a
    // successful auto-register below. See DRYRUN_2CLOCK — registration propagation.
    let self_register_envelope: Arc<Option<Vec<u8>>>;
    if cfg.is_clock {
        let reg_key = format!("clock_reg:{}", cfg.node_id);
        let already_registered = chain.store.state_get(&reg_key).is_some();
        let mut captured_envelope: Option<Vec<u8>> = None;
        if !already_registered {
            let min_stake: u64 = chain.store.state_get("chain_param:clock_min_stake")
                .and_then(|b| serde_json::from_slice::<u64>(&b).ok())
                .unwrap_or(5 * 10_000_000_000);
            let balance = chain.store.get_balance(&cfg.node_id, hone_types::NATIVE_TOKEN);
            let epoch = chain.current_epoch();
            // During the bootstrap grace window the apply-layer (chain.rs) accepts a
            // stake-0 clock registration with no minimum and no debit — the POW-genesis
            // deadlock-breaker. The startup guard must honor grace too, or a fresh
            // zero-balance founder clock bails here before ever submitting, and never
            // registers ("0 registered clocks"). See DRYRUN_2CLOCK_2026-07-10.md, BUG 2.
            let in_grace = epoch <= hone_types::CLOCK_BOOTSTRAP_GRACE_END_EPOCH;
            if in_grace || balance >= min_stake {
                // In grace, offer whatever balance we have (may be 0); otherwise full min_stake.
                let stake = if in_grace { balance.min(min_stake) } else { min_stake };
                let pubkey = signing_pubkey_hex.as_deref().map(|s| s.to_owned());
                let entry = hone_types::LedgerEntry::ClockNodeRegister {
                    node_id: cfg.node_id.clone(),
                    stake,
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
                        // Broadcast WITH signature so peers can validate + apply it
                        // (a sig-less ClockNodeRegister is rejected on the receive side).
                        broadcast_signed_entry(&entry, sig_ref, &net_handle.cmd_tx).await;
                        // Capture for periodic re-announce to late-joining peers.
                        captured_envelope = signed_entry_envelope(&entry, sig_ref);
                    },
                    Err(e)   => warn!(
                        "[clock] auto-register failed for '{}': {} — \
                        check HONE_POSTING_KEY is the private key seed (not the public key) \
                        matching the posting key registered on-chain for this account, \
                        or call POST /api/clock/register manually with a signed payload",
                        cfg.node_id, e
                    ),
                }
            } else {
                warn!("[clock] '{}' has {} hunits, needs {} to register as clock node \
                    (bootstrap grace ended at epoch {}, now epoch {})",
                    cfg.node_id, balance, min_stake,
                    hone_types::CLOCK_BOOTSTRAP_GRACE_END_EPOCH, epoch);
            }
        } else {
            // Already registered on-chain (e.g. relaunch on an existing data dir).
            // Re-sign a ClockNodeRegister from the stored record so we can still
            // re-announce ourselves to peers that have never seen our registration.
            if let Some(sk) = signing_key.as_ref().as_ref() {
                if let Some(rec) = chain.store.state_get(&reg_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                {
                    let entry = hone_types::LedgerEntry::ClockNodeRegister {
                        node_id: cfg.node_id.clone(),
                        stake:   rec["stake"].as_u64().unwrap_or(0),
                        epoch:   rec["registered_epoch"].as_u64().unwrap_or(0),
                        pubkey:  rec["pubkey"].as_str().map(|s| s.to_owned()),
                        signature: None,
                    };
                    let sig_hex = crate::tx::canonical_signing_message(&entry, &chain.chain_id)
                        .ok()
                        .map(|msg| { use ed25519_dalek::Signer; hex::encode(sk.sign(msg.as_bytes()).to_bytes()) });
                    captured_envelope = signed_entry_envelope(&entry, sig_hex.as_deref());
                }
            }
        }
        self_register_envelope = Arc::new(captured_envelope);
    } else {
        self_register_envelope = Arc::new(None);
    }

    // Emit seals when this node is running as a clock peer
    if cfg.is_clock {
        let clock_ref = clock.clone();
        let cmd_tx = net_handle.cmd_tx.clone();
        let node_id_c = cfg.node_id.clone();
        let signing_key_c = Arc::clone(&signing_key);
        let signing_pubkey_c = Arc::clone(&signing_pubkey_hex);
        let self_reg_c = Arc::clone(&self_register_envelope);
        // Epoch is relative to genesis, not Unix epoch.
        // genesis_ts is guaranteed set (init_genesis would have errored otherwise).
        let genesis_ts = cfg.genesis_timestamp.unwrap_or(0);
        tokio::spawn(async move {
            let mut last_sent: u64 = 0;
            let mut announced: u64 = 0; // epochs in which we re-announced our registration
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                let now = now_ms();
                let elapsed = now.saturating_sub(genesis_ts);
                let epoch = elapsed / hone_types::EPOCH_MS;
                if epoch > last_sent {
                    last_sent = epoch;

                    // Re-announce ourselves as a clock so peers that connected AFTER our
                    // one-shot boot broadcast still learn we exist (gossipsub does not
                    // replay to late joiners). This goes on the hone/clock-hello liveness
                    // topic, NOT hone/entries: a hello is applied WRITE-ONCE and never
                    // enters the pending pool, so repeating it every epoch cannot fork
                    // state_root — unlike the old re-broadcast that re-applied the
                    // registration with a grown stake. See DRYRUN_2CLOCK / BUG 6.
                    if let Some(hello_data) = self_reg_c.as_ref().as_ref() {
                        if announced < 20 || epoch % 20 == 0 {
                            announced += 1;
                            let _ = cmd_tx.send(NetCmd::Broadcast {
                                topic: "hone/clock-hello",
                                data: hello_data.clone(),
                            }).await;
                        }
                    }

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
                            topic: "hone/seals",
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

    // ── Cross-chain finality announcer ────────────────────────────────────────
    {
        let cc_module = Arc::new(cross_chain_finality::CrossChainFinalityModule::new(
            chain.clone(),
            cfg.data_dir.to_str().unwrap_or("."),
        ));
        let chain_id  = cfg.chain_id.clone();
        let account   = cfg.account.clone();
        tokio::spawn(async move {
            cross_chain_finality::run_announcer(cc_module, chain_id, account).await;
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

    // ── Hardware probe — auto-detect installed hardware, fill unset role flags ──
    // If an env var is explicitly set ("true"/"false"), that value wins.
    // If it is absent (not set at all), the probe result fills the gap.
    let capabilities = Arc::new(hardware_probe::probe(&cfg.data_dir).await);
    fn env_or_probe(var: &str, detected: bool) -> bool {
        match std::env::var(var) {
            Ok(v) => v == "true" || v == "1",
            Err(_) => detected,
        }
    }
    let enable_storage = env_or_probe("HONE_STORAGE", capabilities.disk_gb >= 10);
    let enable_service = env_or_probe("HONE_SERVICE", false);
    let enable_sensor  = env_or_probe("HONE_SENSOR",  capabilities.has_gnss);
    let enable_worker  = env_or_probe("HONE_WORKER",  capabilities.has_ollama);
    let enable_mempool = env_or_probe("HONE_MEMPOOL", false);
    info!(
        "roles — miner={} clock={} storage={} service={} sensor={} worker={} mempool={}",
        cfg.is_miner, cfg.is_clock, enable_storage, enable_service,
        enable_sensor, enable_worker, enable_mempool,
    );

    // ── UDP multicast gossip — Phase 2 transport ────────────────────────────
    // Propagates light entries (bids) to LAN peers in <1ms via 239.1.1.1:6944.
    // Does NOT count toward peer_count — the zero-peers gate is libp2p-only.
    let udp_handle = {
        let (udp_gossip, handle) = udp_gossip::UdpGossip::new(
            chain.clone(),
            net_handle.cmd_tx.clone(),
            cfg.chain_id.clone(),
        );
        tokio::spawn(async move { udp_gossip.run().await });
        handle
    };

    // ── Mining (legacy direct-inference loop) ────────────────────────────────
    // NOTE: HONE_MINER submits Mine entries each epoch via a local Ollama call.
    // This bypasses the inference marketplace. New deployments should use
    // HONE_WORKER=true instead, which bids on posted jobs and earns job fees.
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
    // HONE_STORAGE=true: proves bytes on disk each epoch, earns StorageReward.
    if enable_storage {
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
    // HONE_SERVICE=true: proves active container-hours, earns ServiceReward.
    // Covers general service nodes; LinkGit storage nodes set this flag too.
    if enable_service {
        let chain_ref   = chain.clone();
        let account     = cfg.account.clone();
        let genesis_ts  = cfg.genesis_timestamp.unwrap_or(0);
        let cmd_service = net_handle.cmd_tx.clone();
        tokio::spawn(async move {
            services::run_service_node(chain_ref, account, genesis_ts, cmd_service).await;
        });
    }

    // ── Sensor node (SensorDataCommit each epoch) ────────────────────────────
    // HONE_SENSOR=true: reads system sensors (CPU temp, uptime) and submits
    // SensorDataCommit each epoch, earning SensorReward.
    if enable_sensor {
        let chain_ref  = chain.clone();
        let account    = cfg.account.clone();
        let genesis_ts = cfg.genesis_timestamp.unwrap_or(0);
        let cmd_sensor = net_handle.cmd_tx.clone();
        tokio::spawn(async move {
            sensor::run_sensor_node(chain_ref, account, genesis_ts, cmd_sensor).await;
        });
    }

    // ── Worker (inference marketplace participant) ────────────────────────────
    // HONE_WORKER=true: watches the chain for posted inference jobs, bids on
    // them, calls Ollama when awarded, and submits InferenceJobComplete.
    if enable_worker {
        let chain_ref = chain.clone();
        let account = cfg.account.clone();
        let cmd_for_worker = net_handle.cmd_tx.clone();
        let udp_for_worker = udp_handle.clone();
        tokio::spawn(async move {
            worker::run_worker(chain_ref, account, cmd_for_worker, udp_for_worker).await;
        });
    }

    // ── TON wallet activation relay ───────────────────────────────────────────
    {
        let ton_cfg = crate::ton_relay::TonRelayConfig::from_env(&wallet_keys);
        if ton_cfg.enabled {
            let chain_c  = chain.clone();
            let cmd_tx_c = net_handle.cmd_tx.clone();
            tokio::spawn(async move {
                crate::ton_relay::run_ton_relay(chain_c, ton_cfg, cmd_tx_c).await;
            });
        }
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
                                    topic: "hone/entries",
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
        let gossip_genesis_ts = cfg.genesis_timestamp.unwrap_or(0);
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
                                } else if matches!(e, hone_types::LedgerEntry::Mine { .. }) {
                                    // Mine entries must be in every node's DB before reward
                                    // hash is computed at epoch seal. Apply directly so the
                                    // reward consensus hash matches across all nodes.
                                    if let Err(err) = chain_ref.apply_entry(&e) {
                                        tracing::debug!("gossip Mine apply failed: {}", err);
                                    }
                                } else if let hone_types::LedgerEntry::ClockNodeRegister { node_id, .. } = &e {
                                    // Clock registration is WRITE-ONCE. A clock re-announces
                                    // itself every epoch (via hone/clock-hello and, for older
                                    // peers, re-broadcast on hone/entries), but the on-chain
                                    // record must be applied exactly once — re-applying it
                                    // later re-records the (by-then grown) stake on some nodes
                                    // but not others, forking state_root. If already in the
                                    // store, drop; otherwise apply write-once now (NOT via the
                                    // pending pool, whose per-epoch ordering also diverged).
                                    // See DRYRUN_2CLOCK / BUG 6.
                                    let reg_key = format!("clock_reg:{}", node_id);
                                    if chain_ref.store.state_get(&reg_key).is_some() {
                                        tracing::trace!("clock-register already applied for '{}', ignoring re-announce", node_id);
                                    } else if let Err(err) = tx::validate_and_apply(&chain_ref, &e, sig.as_deref()) {
                                        tracing::debug!("gossip ClockNodeRegister apply failed: {}", err);
                                    } else {
                                        info!("[clock] applied peer registration for '{}' (write-once)", node_id);
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
                        // Drop seals from ghost peers that are more than 10 epochs stale.
                        let real_epoch = now_ms().saturating_sub(gossip_genesis_ts) / 30_000;
                        let stale = seal.get("epoch_number")
                            .and_then(|v| v.as_u64())
                            .map(|e| real_epoch > e + 10)
                            .unwrap_or(false);
                        if stale {
                            let seal_epoch = seal.get("epoch_number").and_then(|v| v.as_u64()).unwrap_or(0);
                            tracing::debug!("dropping stale seal epoch {} (real epoch {})", seal_epoch, real_epoch);
                        } else {
                            clock_ref.receive_seal(seal);
                        }
                    }
                    Ok(NetworkEvent::ClockHello { hello }) => {
                        // A clock liveness beacon: {"entry": <ClockNodeRegister>, "sig": ...}.
                        // Purely a propagation/liveness signal — it carries the sender's
                        // ORIGINAL signed registration so a late-joining peer can write it
                        // into its own store exactly once. It never re-applies (write-once
                        // guard below) and never enters the pending pool, so repeating it
                        // every epoch cannot fork state_root. See DRYRUN_2CLOCK / BUG 6.
                        let (inner, sig) = if let Some(e) = hello.get("entry") {
                            let s = hello.get("sig").and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty()).map(str::to_owned);
                            (e.clone(), s)
                        } else {
                            (hello.clone(), None)
                        };
                        if let Ok(e @ hone_types::LedgerEntry::ClockNodeRegister { .. }) = tx::entry_from_json(&inner) {
                            if let hone_types::LedgerEntry::ClockNodeRegister { node_id, .. } = &e {
                                let reg_key = format!("clock_reg:{}", node_id);
                                if chain_ref.store.state_get(&reg_key).is_none() {
                                    match tx::validate_and_apply(&chain_ref, &e, sig.as_deref()) {
                                        Ok(_)  => info!("[clock] registered peer clock '{}' from hello (write-once)", node_id),
                                        Err(err) => tracing::debug!("clock-hello apply failed for '{}': {}", node_id, err),
                                    }
                                }
                            }
                        }
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

    // Deploy built-in system contracts on first start (idempotent).
    system_contracts::ensure_system_contracts(&chain, &contracts);

    // ── Testnet sim daemon ────────────────────────────────────────────────────
    // Runs on testnet by default (synthetic traffic for demos). MUST be disable-able:
    // it seeds balances and posts transfers independently on each node, which forks
    // state between two clocks and makes a real 2-clock consensus dry-run impossible
    // (see reports/DRYRUN_2CLOCK_2026-07-10.md fork observation). Set HONE_SIM=false
    // on any testnet node used to validate the actual clock/consensus path.
    let sim_enabled = std::env::var("HONE_SIM")
        .map(|v| !(v == "false" || v == "0"))
        .unwrap_or(true);
    if cfg.chain_id == TESTNET_CHAIN_ID && sim_enabled {
        let chain_ref = chain.clone();
        tokio::spawn(async move {
            sim::run(chain_ref).await;
        });
    } else if cfg.chain_id == TESTNET_CHAIN_ID {
        info!("[sim] disabled via HONE_SIM=false — no synthetic testnet traffic");
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
    // HONE_MEMPOOL=true: reports entries relayed per epoch, earns MempoolReward.
    // Any running node already relays gossip — this just makes that work auditable.
    if enable_mempool {
        let chain_ref    = chain.clone();
        let account      = cfg.account.clone();
        let genesis_ts   = cfg.genesis_timestamp.unwrap_or(0);
        let cmd_mempool  = net_handle.cmd_tx.clone();
        let relay_ctr    = mempool_relay_counter.clone();
        tokio::spawn(async move {
            run_mempool_node(chain_ref, account, genesis_ts, cmd_mempool, relay_ctr).await;
        });
    }

    // ── Agent worker / verifier daemon ───────────────────────────────────────
    if std::env::var("HONE_AGENT_WORKER").map(|v| v == "true" || v == "1").unwrap_or(false)
        || std::env::var("HONE_AGENT_VERIFIER").map(|v| v == "true" || v == "1").unwrap_or(false)
    {
        let chain_ref = chain.clone();
        let account   = cfg.account.clone();
        let cmd_aw    = net_handle.cmd_tx.clone();
        let sk_ref    = signing_key.clone();
        tokio::spawn(async move {
            agent_worker::run(chain_ref, account, cmd_aw, sk_ref).await;
        });
    }

    // ── Work generator (synthetic inference demand) ───────────────────────────
    if std::env::var("HONE_WORK_GENERATOR").map(|v| v == "true" || v == "1").unwrap_or(false) {
        let chain_ref   = chain.clone();
        let account     = cfg.account.clone();
        let genesis_ts  = cfg.genesis_timestamp.unwrap_or(0);
        let cmd_wg      = net_handle.cmd_tx.clone();
        let sk_ref      = signing_key.clone();
        tokio::spawn(async move {
            work_generator::run(chain_ref, account, genesis_ts, cmd_wg, sk_ref).await;
        });
    }

    // ── Periodic sweepers (ensemble, session market, agent sessions, VRF) ───────
    {
        let chain_ref = chain.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let epoch = chain_ref.current_epoch();
                ensemble::sweep_expired(&chain_ref, epoch);
                session_market::sweep_expired(&chain_ref, epoch);
                agent_session::sweep_expired(&chain_ref, epoch);
                vrf::sweep_epoch(&chain_ref, epoch);
            }
        });
    }

    // ── Model healer: REMOVED ───────────────────────────────────────────────────
    // The old healer polled an external Ollama daemon (pull/generate/tags) to keep
    // a named model resident. Ollama has been retired as HONE's inference engine —
    // the node runs the model itself via the embedded candle backend, which needs
    // no daemon to heal. The module and its HONE_MODEL_HEALER wiring are gone.

    // ── Chain health monitor ──────────────────────────────────────────────────
    let health_monitor = Arc::new(monitor::HealthMonitor::new());
    {
        let chain_ref  = chain.clone();
        let monitor_ref = health_monitor.clone();
        tokio::spawn(async move { monitor::run(chain_ref, monitor_ref).await; });
    }

    // ── Auto-updater ──────────────────────────────────────────────────────────
    {
        let data_dir = cfg.data_dir.clone();
        tokio::spawn(async move { updater::run(data_dir).await; });
    }

    // ── HTTP API ──────────────────────────────────────────────────────────────
    // The active model is the one the embedded engine serves (HONE_MODEL), or the
    // external INFERENCE_URL model — no Ollama tag query.
    let default_model = crate::miner::detect_model().await.unwrap_or_default();

    tracing::info!("software_hash: {}", &software_hash[..software_hash.len().min(24)]);

    let onion_address = tor::start_hidden_service(&chain, cfg.api_port, cfg.p2p_port).await;

    // ── Nostr relay transport — Phase 4 transport cascade ────────────────────
    // Propagates entries through the Nostr relay network for open-internet reach.
    // Nostr relay peers do NOT count toward peer_count — libp2p-only hardline intact.
    let nostr_handle =
        nostr_transport::start_nostr(chain.clone(), net_handle.cmd_tx.clone(), cfg.chain_id.clone()).await;

    // If Nostr is active, subscribe to the broadcast channel and forward every
    // accepted entry into the Nostr relay pool.
    if let Some(ref nh) = nostr_handle {
        let nh_clone = nh.clone();
        let mut nostr_rx = tx_broadcast.subscribe();
        tokio::spawn(async move {
            loop {
                match nostr_rx.recv().await {
                    Ok((entry, _sig)) => nh_clone.send_entry(entry),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("nostr broadcast rx lagged by {} entries", n);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // ── Matrix room transport — Phase 5 transport cascade ─────────────────────
    // Propagates entries through a federated Matrix room for global reach.
    // Matrix peers do NOT count toward peer_count — libp2p-only hardline intact.
    // Behind the optional `matrix` feature (off by default) — see
    // docs/PLAN_FABLE5_STABILIZATION.md Phase 0 for why (rustc ICE via matrix-sdk).
    #[cfg(feature = "matrix")]
    {
        let matrix_handle =
            matrix_transport::start_matrix(chain.clone(), net_handle.cmd_tx.clone()).await;

        // If Matrix is active, subscribe to the broadcast channel and forward every
        // accepted entry into the Matrix room.
        if let Some(ref mh) = matrix_handle {
            let mh_clone = mh.clone();
            let mut mx_rx = tx_broadcast.subscribe();
            tokio::spawn(async move {
                loop {
                    match mx_rx.recv().await {
                        Ok((entry, _sig)) => mh_clone.send_entry(entry),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!("matrix broadcast rx lagged by {} entries", n);
                        }
                        Err(_) => break,
                    }
                }
            });
        }
    }

    // ── I2P datagram transport — Phase 6 transport cascade ───────────────────
    // Propagates entries through the I2P garlic-routed mesh via SAM bridge.
    // I2P peers do NOT count toward peer_count — libp2p-only hardline intact.
    let i2p_handle =
        i2p::start_i2p(chain.clone(), net_handle.cmd_tx.clone()).await;

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
        signing_key:       signing_key.clone(),
        scientific:        Arc::new(scientific::ScientificEngine::new(chain.clone())),
        cross_chain:       Arc::new(cross_chain_finality::CrossChainFinalityModule::new(
            chain.clone(),
            cfg.data_dir.to_str().unwrap_or("."),
        )),
        secret_store: secret_store_arc,
        health_monitor,
        blob_bandwidth: Arc::new(blob_bandwidth::BlobBandwidthMeter::new()),
        capabilities,
        onion_address: Arc::new(onion_address),
        nostr_handle,
        #[cfg(feature = "matrix")]
        matrix_handle,
        i2p_handle,
        lorawan_handle,
    };
    api::serve(app_state, cfg.api_port).await?;

    Ok(())
}

use utils::now_ms;

// ── Inference verifier ────────────────────────────────────────────────────────
//
// Periodically scans for completed inference jobs from OTHER nodes, re-runs
// the same prompt via Ollama using the same model, and submits a verdict.
// Only verifies jobs whose model matches the locally configured HONE_MODEL.

async fn run_inference_verifier(
    chain:   Arc<Chain>,
    account: String,
    cmd_tx:  tokio::sync::mpsc::Sender<NetCmd>,
) {
    use std::time::Duration;

    // The verifier re-evaluates other nodes' inference work with THIS node's own
    // embedded engine (or the sanctioned external INFERENCE_URL) — never a
    // separate Ollama daemon. Only jobs matching the locally selected model are
    // considered comparable.
    let local_model = crate::miner::detect_model().await.unwrap_or_default();

    // Peers to try when IO text is not stored locally (worker node has the actual output).
    let peer_apis: Vec<String> = std::env::var("HONE_PEER_APIS")
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

            use crate::inference_engine::{self, ChatRequest, Message};
            let eng_req = ChatRequest {
                model: local_model.clone(),
                messages: vec![Message { role: "user".to_owned(), content: meta_prompt }],
                max_tokens: 20,
            };
            let verdict = match inference_engine::chat(eng_req).await {
                Ok(resp) => {
                    let text = resp.content.to_uppercase();
                    if text.contains("APPROVED") {
                        "approved"
                    } else if text.contains("REJECTED") || text.contains("REJECT") {
                        "rejected"
                    } else {
                        "review_required"
                    }
                }
                Err(_) => continue,
            };

            // Estimate value_score from output length + model (true score comes via claim flow)
            let output_tokens = (output_text.split_whitespace().count() as u64).max(1);
            let value_score = hone_types::inference_score(output_tokens, 0, &job.model);

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
                    topic: "hone/entries", data,
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
    // Sweep expired agentic tasks → refund escrow.
    agent_task::sweep_expired(chain, epoch);

    let raw_pool = if era(epoch) >= RECYCLE_ERA {
        let fund = chain.store.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN);
        ((fund as u128 * RECYCLE_REWARD_RATE) / RECYCLE_REWARD_DENOM) as u64
    } else {
        block_reward_at(epoch)
    };
    if raw_pool == 0 { return; }

    // ── Layer A: long-term global network health scalar ───────────────────────
    // Read both EMA values from the PREVIOUS epoch (cold-start default = full health).
    // The scalar is applied to raw_pool before the 2% reserve split so the entire
    // epoch ceiling — including reserve contributions — scales with long-run health.
    // Damped tokens (raw_pool − adjusted_pool) go to the recycle fund immediately.
    let read_layer_a_ema = |key: &str| -> u64 {
        chain.store.state_get(key)
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|j| j["ema"].as_u64())
            .unwrap_or(LAYER_A_SCALAR_DENOM) // default: full health until EMAs warm up
    };
    let layer_a_fast_prev = read_layer_a_ema("layer_a:fast_ema");
    let layer_a_slow_prev = read_layer_a_ema("layer_a:slow_ema");
    let long_term_scalar  = layer_a_scalar(layer_a_fast_prev, layer_a_slow_prev);
    let adjusted_pool     = (raw_pool as u128 * long_term_scalar as u128 / LAYER_A_SCALAR_DENOM as u128) as u64;
    let layer_a_damped    = raw_pool.saturating_sub(adjusted_pool);
    if layer_a_damped > 0 {
        let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, layer_a_damped);
    }
    let raw_pool = adjusted_pool; // shadow: all downstream logic operates on the scaled pool

    // ── Mandatory 2% reserve split (Layer D, always fires) ───────────────────
    // 1.5% → recycle fund  |  0.4% → testnet fund  |  0.1% → treasury (DAO)
    let reserve_total  = (raw_pool as u128 * 2 / 100) as u64;
    let testnet_top_up = (raw_pool as u128 * 4 / 1000) as u64;          // 0.4%
    let treasury_split = (raw_pool as u128 / 1000) as u64;              // 0.1%
    let recycle_split  = reserve_total
        .saturating_sub(testnet_top_up)
        .saturating_sub(treasury_split);                                  // 1.5%
    let activity_pool  = raw_pool.saturating_sub(reserve_total);

    if recycle_split > 0 {
        let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_split);
    }
    if testnet_top_up > 0 {
        let _ = chain.store.credit(TESTNET_FUND_ACCOUNT, NATIVE_TOKEN, testnet_top_up);
    }
    if treasury_split > 0 {
        let _ = chain.store.credit(TREASURY_ACCOUNT, NATIVE_TOKEN, treasury_split);
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
        info!("clock: epoch {} testnet: {} ops × {} hunits", epoch, testnet_ops.len(), actual);
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
            // P2-C: blob payout tier multiplier (1×, 2×, 5× based on reported tier).
            // Tier thresholds: tier 2 requires ≥ 10 GB, tier 3 requires ≥ 100 GB.
            let tier = j["tier"].as_u64().unwrap_or(1);
            let bytes_gb = bytes / (1024 * 1024 * 1024);
            let effective_tier = match tier {
                3 if bytes_gb >= 100 => 3,
                2 if bytes_gb >= 10  => 2,
                _ => 1,
            };
            let tier_mul = match effective_tier { 3 => 5, 2 => 2, _ => 1 };
            Some((j["node_id"].as_str()?.to_owned(), score.saturating_mul(tier_mul)))
        }).collect();

    // Sensors: type-aware sensor_score() per individual sensor, then grouped by owner.
    // Sensors with a registered location earn a 1.3× boost (T4-3, SENSOR_LOCATION_BOOST_BPS).
    // After SENSOR_LOCATION_REQUIRED_EPOCH, unlocated sensors are skipped entirely.
    // gateway_map: sensor_account → gateway_account (for 60/40 split)
    let mut sensor_gateway_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
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
            // Record gateway association for reward split
            if let Some(gw) = j["gateway_account"].as_str() {
                sensor_gateway_map.insert(owner.to_string(), gw.to_string());
            }
        }
        // Coverage reports: dead-spot reports earn more; corroborated cells earn 1.5× bonus.
        // Scores merged into sensor_nodes pool so coverage reporters share the sensor reward pool.
        use hone_types::{COVERAGE_SCORE_SIGNAL, COVERAGE_SCORE_DEAD_SPOT, COVERAGE_CORROBORATION_BONUS_BPS};
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
    let gated_pool_base = (activity_pool as u128 * activity_ratio_num as u128 / ACTIVITY_RATIO_DENOM as u128) as u64;
    let idle_before_c   = activity_pool.saturating_sub(gated_pool_base);

    // ── Layer C: fee-driven boost ─────────────────────────────────────────────
    // Verified fee flows from epoch-1 unlock idle rewards that would otherwise
    // recycle.  Net-flow accounting removes circular payments (A→B→A → 0).
    // Cap = idle_before_c: redirects idle tokens, never creates new supply.
    let fee_boost = if epoch > 0 {
        let prev_epoch = epoch - 1;
        let mut directed: std::collections::BTreeMap<(String, String), u64> =
            std::collections::BTreeMap::new();
        for (_, val) in chain.store.state_scan_prefix(&format!("fee_flow:{}:", prev_epoch)) {
            let Ok(j) = serde_json::from_slice::<serde_json::Value>(&val) else { continue };
            let from   = j["from"].as_str().unwrap_or("").to_owned();
            let to     = j["to"].as_str().unwrap_or("").to_owned();
            let amount = j["amount"].as_u64().unwrap_or(0);
            if from.is_empty() || to.is_empty() || from == to || amount == 0 { continue; }
            *directed.entry((from, to)).or_default() += amount;
        }
        // Build canonical (min, max) pair set — each unordered pair counted once.
        let canonical: std::collections::BTreeSet<(String, String)> = directed.keys()
            .map(|(f, t)| if f <= t { (f.clone(), t.clone()) } else { (t.clone(), f.clone()) })
            .collect();
        let net_total: u128 = canonical.iter().map(|(a, b)| {
            let a_to_b = directed.get(&(a.clone(), b.clone())).copied().unwrap_or(0) as u128;
            let b_to_a = directed.get(&(b.clone(), a.clone())).copied().unwrap_or(0) as u128;
            if a_to_b >= b_to_a { a_to_b - b_to_a } else { b_to_a - a_to_b }
        }).sum();
        let boost_raw = ((net_total * LAYER_C_FEE_BOOST_NUM as u128) / LAYER_C_FEE_BOOST_DENOM as u128) as u64;
        boost_raw.min(idle_before_c)
    } else {
        0
    };

    let gated_pool   = gated_pool_base.saturating_add(fee_boost);
    let idle_recycle = idle_before_c.saturating_sub(fee_boost);
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
        (count as u64).min(critical_mass) * ACTIVITY_RATIO_DENOM / critical_mass
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
    // Sensor rewards: split 60/40 when gateway is registered, plain SensorReward otherwise.
    {
        let total_score: u64 = sensor_nodes.iter().map(|(_, s)| s).sum();
        for (node_id, score) in &sensor_nodes {
            if total_score == 0 || *score == 0 { continue; }
            let amount = (sensor_pool as u128 * *score as u128 / total_score as u128) as u64;
            if amount == 0 { continue; }
            let entry = if let Some(gateway) = sensor_gateway_map.get(node_id) {
                const SENSOR_BPS: u128 = 6_000;
                const GATEWAY_BPS: u128 = 4_000;
                let sensor_amount = (amount as u128 * SENSOR_BPS / 10_000) as u64;
                let gateway_amount = (amount as u128 * GATEWAY_BPS / 10_000) as u64;
                LedgerEntry::GatewayRewardSplit {
                    sensor_account: node_id.clone(),
                    gateway_account: gateway.clone(),
                    sensor_amount,
                    gateway_amount,
                    epoch,
                }
            } else {
                LedgerEntry::SensorReward { node_id: node_id.clone(), amount, epoch }
            };
            let _ = chain.apply_entry(&entry);
            if let Ok(data) = serde_json::to_vec(&serde_json::json!({"entry": entry})) {
                let _ = cmd_tx.send(crate::net::NetCmd::Broadcast { topic: "hone/entries", data }).await;
            }
        }
    }
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

    // ── Layer A: update EMAs with this epoch's total utilisation signal ───────
    // Signal = fraction of the theoretical maximum (all 9 pool types at 100%),
    // expressed in [0, LAYER_A_SCALAR_DENOM].  Normalising by the fixed pool
    // count (not active_pool_count) so a chain with only 3 active pool types
    // naturally settles below full scalar until the remaining pools organise.
    let util_signal = (total_util as u128 * LAYER_A_SCALAR_DENOM as u128
        / (LAYER_A_POOL_COUNT as u128 * ACTIVITY_RATIO_DENOM as u128)) as u64;
    let layer_a_fast_new = (EMA_ALPHA_NUM * util_signal
        + (EMA_ALPHA_DENOM - EMA_ALPHA_NUM) * layer_a_fast_prev)
        / EMA_ALPHA_DENOM;
    let layer_a_slow_new = (LAYER_A_SLOW_ALPHA_NUM * util_signal
        + (LAYER_A_SLOW_ALPHA_DENOM - LAYER_A_SLOW_ALPHA_NUM) * layer_a_slow_prev)
        / LAYER_A_SLOW_ALPHA_DENOM;
    let _ = chain.store.state_set("layer_a:fast_ema",
        &serde_json::to_vec(&serde_json::json!({ "ema": layer_a_fast_new })).unwrap_or_default());
    let _ = chain.store.state_set("layer_a:slow_ema",
        &serde_json::to_vec(&serde_json::json!({ "ema": layer_a_slow_new })).unwrap_or_default());

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
        "clock: epoch {} raw_pre_a={} scalar_a={:.3} adjusted={} reserve={} \
         gated={}({:.0}%) fee_boost={} scarcity_recycle={} layer_a_ema(f={},s={}) \
         [infer={}/{} store={}/{} sensor={}/{} verify={}/{} svc={}/{} mem={}/{} tracker={}/{} git={}/{} build={}/{}]",
        epoch, adjusted_pool.saturating_add(layer_a_damped),
        long_term_scalar as f64 / LAYER_A_SCALAR_DENOM as f64,
        raw_pool, reserve_total, gated_pool,
        activity_ratio_num as f64 / ACTIVITY_RATIO_DENOM as f64 * 100.0,
        fee_boost, scarcity_recycle + idle_recycle,
        layer_a_fast_new, layer_a_slow_new,
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
            let max_fee: u64 = 1_000_000; // 0.0001 HONE — small fee from testnet fund
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
                // Store the input text so the local worker can execute it.
                let prompt = format!("Summarise epoch {} of the HONE chain in one sentence.", epoch);
                let _ = chain.store.state_set(
                    &format!("infer_input:{}", job_id),
                    prompt.as_bytes(),
                );
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
        let _ = cmd_tx.send(NetCmd::Broadcast { topic: "hone/entries", data }).await;
    }
}

/// Broadcast an entry WITH its signature attached in the gossip envelope.
///
/// The plain `broadcast_entry_desktop` above sends `{"entry": ...}` with no `sig`.
/// That is fine for entries a peer will re-validate against on-chain keys, but
/// `ClockNodeRegister` from a key-less/self-auth account (or verified against the
/// account's posting key) REQUIRES the signature to be present in the envelope's
/// `"sig"` field — the receive path (main.rs NetworkEvent::Entry) pulls `sig` from
/// there and `validate_and_apply` rejects a ClockNodeRegister with no signature
/// ("signature required for ClockNodeRegister", tx.rs). Without this, a clock's
/// registration never applies on any *other* node, so peers never see it as
/// registered — the "registered_count stuck at 1" fork. See DRYRUN_2CLOCK.
fn signed_entry_envelope(entry: &LedgerEntry, sig_hex: Option<&str>) -> Option<Vec<u8>> {
    let mut envelope = serde_json::json!({"entry": entry});
    if let Some(s) = sig_hex.filter(|s| !s.is_empty()) {
        envelope["sig"] = serde_json::Value::String(s.to_owned());
    }
    serde_json::to_vec(&envelope).ok()
}

async fn broadcast_signed_entry(
    entry: &LedgerEntry,
    sig_hex: Option<&str>,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
) {
    if let Some(data) = signed_entry_envelope(entry, sig_hex) {
        let _ = cmd_tx.send(NetCmd::Broadcast { topic: "hone/entries", data }).await;
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
            let _ = chain.store.credit(hone_types::RECYCLE_FUND_ACCOUNT, hone_types::NATIVE_TOKEN, observer_cut);
        }

        if treasury_cut > 0 {
            let _ = chain.store.credit("treasury", hone_types::NATIVE_TOKEN, treasury_cut);
        }
        if recycle_cut > 0 {
            let _ = chain.store.credit(hone_types::RECYCLE_FUND_ACCOUNT, hone_types::NATIVE_TOKEN, recycle_cut);
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
            let _ = cmd_tx.send(NetCmd::Broadcast { topic: "hone/entries", data }).await;
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
    use hone_types::EPOCH_MS;
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
                topic: "hone/entries",
                data,
            }).await;
        }

        info!("mempool: heartbeat epoch {} relayed={} latency={}ms",
            epoch, entries_relayed, propagation_latency_ms);
    }
}
