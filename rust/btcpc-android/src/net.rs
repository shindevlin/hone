//! libp2p swarm for the Android micronode.
//! TCP transport only (no QUIC) — Android network stack is reliable with TCP.
//! Same gossipsub topics and Kademlia bootstrap logic as the desktop node.

use std::time::Duration;
use anyhow::Result;
use futures::StreamExt;
use libp2p::{
    gossipsub, identify, kad, noise, ping, tcp, yamux,
    swarm::{NetworkBehaviour, SwarmEvent},
    Multiaddr, PeerId, StreamProtocol,
    identity::Keypair,
};
use serde_json::Value;
use tokio::sync::{mpsc, broadcast};
use tracing::{info, warn};

// ── Commands and events ───────────────────────────────────────────────────────

pub enum NetCmd {
    Broadcast { topic: &'static str, data: Vec<u8> },
}

#[derive(Debug)]
pub enum NetworkEvent {
    Entry    { entry: Value },
    Block    { epoch: u64, data: Vec<u8> },
    EpochSeal { seal: Value },
    PeerConnected    { peer: PeerId },
    PeerDisconnected { peer: PeerId },
}

// ── Behaviour ─────────────────────────────────────────────────────────────────

#[derive(NetworkBehaviour)]
struct Behaviour {
    gossipsub: gossipsub::Behaviour,
    kad:       kad::Behaviour<kad::store::MemoryStore>,
    identify:  identify::Behaviour,
    ping:      ping::Behaviour,
}

// ── Swarm runner ──────────────────────────────────────────────────────────────

pub struct SwarmConfig {
    pub chain_id:        String,
    pub data_dir:        String,
    pub p2p_port:        u16,
    pub bootstrap_peers: Vec<String>,
}

pub async fn run_swarm(
    cfg: SwarmConfig,
    event_tx: mpsc::Sender<NetworkEvent>,
    mut cmd_rx: mpsc::Receiver<NetCmd>,
    mut shutdown: broadcast::Receiver<()>,
) -> Result<()> {
    // Load or generate identity key.
    let keypair = load_or_create_keypair(&cfg.data_dir);
    let local_peer_id = PeerId::from(&keypair.public());
    info!("net: local peer {}", local_peer_id);

    // Gossipsub config — same mesh parameters as desktop node.
    let gossipsub_cfg = gossipsub::ConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(10))
        .mesh_n(6)
        .mesh_n_low(4)
        .mesh_n_high(12)
        .build()
        .expect("gossipsub config");

    let message_id_fn = |msg: &gossipsub::Message| {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        msg.data.hash(&mut h);
        gossipsub::MessageId::from(h.finish().to_string())
    };

    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(keypair.clone()),
        gossipsub_cfg,
    ).expect("gossipsub");

    // Subscribe to all chain topics.
    for topic_str in &["btcpc/entries", "btcpc/seals", "btcpc/blocks"] {
        let topic = gossipsub::IdentTopic::new(*topic_str);
        gossipsub.subscribe(&topic).ok();
    }

    // Kademlia DHT for peer discovery.
    let kad_store = kad::store::MemoryStore::new(local_peer_id);
    let mut kad = kad::Behaviour::new(local_peer_id, kad_store);
    kad.set_mode(Some(kad::Mode::Server));

    // Identify so peers learn our observed address.
    let identify = identify::Behaviour::new(identify::Config::new(
        format!("/btcpc/{}/1.0.0", cfg.chain_id),
        keypair.public(),
    ));

    let ping = ping::Behaviour::default();

    let behaviour = Behaviour { gossipsub, kad, identify, ping };

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        .with_behaviour(|_| behaviour)?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    // Listen on all interfaces.
    let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", cfg.p2p_port)
        .parse().expect("listen addr");
    swarm.listen_on(listen_addr)?;

    // Dial bootstrap peers.
    for peer_addr in &cfg.bootstrap_peers {
        if let Ok(addr) = peer_addr.parse::<Multiaddr>() {
            swarm.dial(addr).ok();
        }
    }

    // Also fetch peers from Hive registry.
    if let Ok(hive_peers) = fetch_hive_peers(&cfg.chain_id).await {
        for addr in hive_peers {
            if let Ok(ma) = addr.parse::<Multiaddr>() {
                swarm.dial(ma).ok();
            }
        }
    }

    // ── Event loop ────────────────────────────────────────────────────────────
    loop {
        tokio::select! {
            _ = shutdown.recv() => break,

            // Outbound commands from other subsystems.
            Some(cmd) = cmd_rx.recv() => {
                let NetCmd::Broadcast { topic, data } = cmd;
                let t = gossipsub::IdentTopic::new(topic);
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(t, data) {
                    warn!("net: publish error on {}: {}", topic, e);
                }
            }

            // Swarm events.
            event = swarm.select_next_some() => {
                handle_event(event, &event_tx).await;
            }
        }
    }

    Ok(())
}

async fn handle_event(
    event: SwarmEvent<BehaviourEvent>,
    event_tx: &mpsc::Sender<NetworkEvent>,
) {
    match event {
        SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(
            gossipsub::Event::Message { message, .. }
        )) => {
            let topic = message.topic.to_string();
            let data  = message.data;

            if topic.contains("seals") {
                if let Ok(v) = serde_json::from_slice::<Value>(&data) {
                    let _ = event_tx.send(NetworkEvent::EpochSeal { seal: v }).await;
                }
            } else if topic.contains("blocks") {
                // Block gossip: first 8 bytes = epoch (big-endian), rest = block bytes.
                if data.len() > 8 {
                    let epoch = u64::from_be_bytes(data[..8].try_into().unwrap_or([0u8; 8]));
                    let _ = event_tx.send(NetworkEvent::Block {
                        epoch,
                        data: data[8..].to_vec(),
                    }).await;
                }
            } else if topic.contains("entries") {
                if let Ok(v) = serde_json::from_slice::<Value>(&data) {
                    let _ = event_tx.send(NetworkEvent::Entry { entry: v }).await;
                }
            }
        }
        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
            info!("net: connected to {}", peer_id);
            let _ = event_tx.send(NetworkEvent::PeerConnected { peer: peer_id }).await;
        }
        SwarmEvent::ConnectionClosed { peer_id, .. } => {
            let _ = event_tx.send(NetworkEvent::PeerDisconnected { peer: peer_id }).await;
        }
        _ => {}
    }
}

// ── Identity persistence ──────────────────────────────────────────────────────

fn load_or_create_keypair(data_dir: &str) -> Keypair {
    let path = format!("{}/p2p-identity.json", data_dir);
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(kp) = Keypair::from_protobuf_encoding(&bytes) {
            return kp;
        }
    }
    let kp = Keypair::generate_ed25519();
    if let Ok(bytes) = kp.to_protobuf_encoding() {
        let _ = std::fs::write(&path, &bytes);
    }
    kp
}

// ── Hive peer discovery ───────────────────────────────────────────────────────

async fn fetch_hive_peers(chain_id: &str) -> Result<Vec<String>> {
    let key = if chain_id == btcpc_types::TESTNET_CHAIN_ID {
        "btcpc_satoshi_peers"
    } else {
        "btcpc_peers"
    };

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "condenser_api.get_accounts",
        "params": [["btcpc"]],
        "id": 1,
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let resp: Value = client
        .post("https://api.hive.blog")
        .json(&body)
        .send().await?
        .json().await?;

    let peers = resp
        .pointer("/result/0/posting_json_metadata")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| serde_json::from_value::<Vec<String>>(v[key].clone()).ok())
        .unwrap_or_default();

    Ok(peers)
}
