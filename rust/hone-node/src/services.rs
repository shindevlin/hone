use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use hone_types::{LedgerEntry, EPOCH_MS, NATIVE_TOKEN};

use crate::chain::Chain;
use crate::net::NetCmd;
use crate::utils::now_ms;

// 5 HONE for 30 days of a hosted clock node
pub const HOSTING_PRICE_HUNITS: u64 = 500_000_000;
// Duration in epochs: 30 days × 86400s/day ÷ 30s/epoch = 86,400 epochs
pub const HOSTING_DURATION_EPOCHS: u64 = 86_400;
// RocksDB key prefix for pending hosting requests
const HOSTING_REQUEST_PREFIX: &str = "hosting_req:";
// Published capability key
const HOSTING_LISTING_KEY: &str = "svc:hosting";

pub async fn run_service_node(
    chain:       Arc<Chain>,
    account:     String,
    genesis_ts:  u64,
    cmd_tx:      tokio::sync::mpsc::Sender<NetCmd>,
) {
    info!("service node started: account={}", account);

    // Advertise hosting capability in node meta so the network can discover it
    let listing = serde_json::json!({
        "node": account,
        "price_hunits": HOSTING_PRICE_HUNITS,
        "duration_epochs": HOSTING_DURATION_EPOCHS,
        "docker_available": check_docker(),
        "updated_ms": now_ms(),
    });
    let _ = chain.store.state_set(
        &format!("{}{}",  HOSTING_LISTING_KEY, account),
        &serde_json::to_vec(&listing).unwrap_or_default(),
    );

    let elapsed = now_ms().saturating_sub(genesis_ts);
    let mut last_epoch = elapsed / EPOCH_MS;

    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Process any pending hosting requests on every tick (not just epoch boundaries)
        process_hosting_requests(&chain, &account, &cmd_tx).await;

        let elapsed = now_ms().saturating_sub(genesis_ts);
        let epoch = elapsed / EPOCH_MS;
        if epoch <= last_epoch { continue; }
        last_epoch = epoch;

        let uptime_ms = now_ms().saturating_sub(genesis_ts);
        let container_hours = uptime_ms / 3_600_000;

        let entry = LedgerEntry::ServiceHeartbeat {
            node_id:         account.clone(),
            epoch,
            container_hours,
            signed_by:       account.clone(),
        };

        if let Err(e) = chain.apply_entry(&entry) {
            warn!("service: heartbeat failed epoch {}: {}", epoch, e);
            continue;
        }

        let envelope = serde_json::json!({"entry": entry});
        if let Ok(data) = serde_json::to_vec(&envelope) {
            let _ = cmd_tx.send(NetCmd::Broadcast {
                topic: "hone/entries",
                data,
            }).await;
        }

        info!("service: heartbeat epoch {} hours={}", epoch, container_hours);
    }
}

async fn process_hosting_requests(
    chain: &Arc<Chain>,
    service_account: &str,
    _cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
) {
    let prefix = format!("{}{}", HOSTING_REQUEST_PREFIX, service_account);
    let pairs = chain.store.state_scan_prefix(&prefix);

    for (key, val) in pairs {
        let Ok(mut req) = serde_json::from_slice::<serde_json::Value>(&val) else { continue };
        if req["status"].as_str() != Some("pending") { continue }

        let buyer = req["buyer"].as_str().unwrap_or("").to_string();
        let request_id = req["request_id"].as_str().unwrap_or("").to_string();
        if buyer.is_empty() || request_id.is_empty() { continue }

        let rid_short = if request_id.len() >= 8 { &request_id[..8] } else { &request_id };
        info!("[hosting] provisioning node for '{}' (request {})", buyer, rid_short);

        match provision_hosted_node(&buyer) {
            Ok(container_id) => {
                req["status"] = serde_json::json!("active");
                req["container_id"] = serde_json::json!(container_id);
                req["provisioned_ms"] = serde_json::json!(now_ms());
                let _ = chain.store.state_set(&key, &serde_json::to_vec(&req).unwrap_or_default());
                info!("[hosting] container started for '{}': {}", buyer, req["container_id"]);
            }
            Err(e) => {
                warn!("[hosting] failed to provision for '{}': {}", buyer, e);
                let _ = chain.store.credit(&buyer, NATIVE_TOKEN, HOSTING_PRICE_HUNITS);
                let _ = chain.store.debit(service_account, NATIVE_TOKEN, HOSTING_PRICE_HUNITS);
                req["status"] = serde_json::json!("refunded");
                req["error"] = serde_json::json!(e.to_string());
                let _ = chain.store.state_set(&key, &serde_json::to_vec(&req).unwrap_or_default());
            }
        }
    }
}

fn provision_hosted_node(buyer_account: &str) -> anyhow::Result<String> {
    if !check_docker() {
        anyhow::bail!("Docker not available on this service node");
    }

    let safe_name = buyer_account
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect::<String>();
    let container_name = format!("hone-hosted-{}", safe_name);

    // Remove any stale container with this name
    let _ = std::process::Command::new("docker")
        .args(["rm", "-f", &container_name])
        .output();

    let api_port = find_free_port(14242, 14342).unwrap_or(14242).to_string();
    let p2p_port = find_free_port(16942, 17042).unwrap_or(16942).to_string();
    let vol_name = format!("hone-hosted-{}", safe_name);

    let out = std::process::Command::new("docker")
        .args([
            "run", "-d",
            "--name", &container_name,
            "--restart", "unless-stopped",
            "-e", &format!("HONE_ACCOUNT={}", buyer_account),
            "-e", "HONE_CLOCK=true",
            "-e", "HONE_MINER=false",
            "-e", "HONE_STORAGE=false",
            "-e", "HONE_CHAIN_ID=hone",
            "-e", &format!("HONE_API_PORT={}", api_port),
            "-e", &format!("HONE_P2P_PORT={}", p2p_port),
            "-e", "HONE_ANNOUNCE_ADDR=",
            "-p", &format!("{}:4242", api_port),
            "-p", &format!("{}:6942", p2p_port),
            "-v", &format!("{}:/data", vol_name),
            "ghcr.io/hone-network/hone-node:latest",
        ])
        .output()?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("docker run failed: {}", stderr.trim());
    }

    let container_id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(container_id)
}

fn check_docker() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn find_free_port(start: u16, end: u16) -> Option<u16> {
    for port in start..=end {
        if std::net::TcpListener::bind(("0.0.0.0", port)).is_ok() {
            return Some(port);
        }
    }
    None
}

/// Called by the API to place a hosting request after payment is verified.
pub fn place_hosting_request(
    chain: &Chain,
    buyer: &str,
    service_node: &str,
    request_id: &str,
) -> anyhow::Result<()> {
    let req = serde_json::json!({
        "request_id": request_id,
        "buyer": buyer,
        "service_node": service_node,
        "price_hunits": HOSTING_PRICE_HUNITS,
        "duration_epochs": HOSTING_DURATION_EPOCHS,
        "status": "pending",
        "created_ms": now_ms(),
    });
    let key = format!("{}{}{}", HOSTING_REQUEST_PREFIX, service_node, request_id);
    chain.store.state_set(&key, &serde_json::to_vec(&req)?)?;
    Ok(())
}

/// Returns all hosting listings advertised by service nodes.
pub fn list_hosting_nodes(chain: &Chain) -> Vec<serde_json::Value> {
    chain.store.state_scan_prefix(HOSTING_LISTING_KEY)
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice(&v).ok())
        .collect()
}

/// Returns all hosted nodes for a given buyer.
pub fn list_hosted_nodes(chain: &Chain, buyer: &str) -> Vec<serde_json::Value> {
    chain.store.state_scan_prefix(HOSTING_REQUEST_PREFIX)
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<serde_json::Value>(&v).ok())
        .filter(|v| v["buyer"].as_str() == Some(buyer))
        .collect()
}
