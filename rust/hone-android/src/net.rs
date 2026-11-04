//! libp2p swarm for the Android micronode.
//! TCP transport only (no QUIC) — Android network stack is reliable with TCP.
//! Same gossipsub topics and Kademlia bootstrap logic as the desktop node.

use std::time::Duration;
use anyhow::Result;
use futures::StreamExt;
use libp2p::{
    core::ConnectedPoint, dcutr, gossipsub, identify, kad, mdns, multiaddr::Protocol, noise, ping, relay, tcp, yamux,
    swarm::{dial_opts::{DialOpts, PeerCondition}, NetworkBehaviour, SwarmEvent},
    Multiaddr, PeerId, StreamProtocol,
    identity::Keypair,
};

/// Public seeds the phone always dials (appended to the app's list): the TXT seed list
/// and the ferryvps seed over TCP. Phones use TCP only.
pub const DEFAULT_BOOTSTRAP: &[&str] = &[
    "/dnsaddr/bootstrap.honemesh.net",
    "/dns4/honemesh.net/tcp/6942/p2p/12D3KooWQnJPfAYbktWXv1zFoGCQAz4v7qWthLTdrD7ey5JPDCUt",
];
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
    ConsensusProposal { epoch: u64, rewards_hash: String, node_id: String },
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
    // Optional: Android 11+ restricts the netlink sockets mDNS needs, so a
    // failed mDNS init must not take down the whole swarm.
    mdns:      libp2p::swarm::behaviour::toggle::Toggle<mdns::tokio::Behaviour>,
    /// Circuit relay v2 client: reserve a slot on the public seed so other nodes can
    /// reach the phone, and DCUtR to upgrade relayed links to direct ones.
    relay_client: relay::client::Behaviour,
    dcutr:     dcutr::Behaviour,
}

// ── Swarm runner ──────────────────────────────────────────────────────────────

pub struct SwarmConfig {
    pub chain_id:        String,
    pub data_dir:        String,
    pub p2p_port:        u16,
    pub bootstrap_peers: Vec<String>,
    /// Observer mode: drop every outbound publish (listen-only).
    pub read_only:       bool,
    /// Witness clock: publish ONLY on hone/seals. Every other topic is dropped,
    /// so the phone can never gossip rewards, finalize or ledger entries.
    pub seals_only:      bool,
    /// Shared live peer count for the UI.
    pub peers:           std::sync::Arc<std::sync::atomic::AtomicU32>,
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
    // On-wire topic = "{chain_id}/{key}", exactly like hone-node (net.rs: t_seals etc.).
    // Without the chain prefix the phone joined topics no desktop node uses, so it
    // never received a message and its publishes failed with NoPeersSubscribedToTopic.
    for topic_str in &["hone/entries", "hone/seals", "hone/blocks", "hone/consensus"] {
        let topic = gossipsub::IdentTopic::new(format!("{}/{}", cfg.chain_id, topic_str));
        gossipsub.subscribe(&topic).ok();
    }

    // Kademlia DHT for peer discovery.
    let kad_store = kad::store::MemoryStore::new(local_peer_id);
    let mut kad = kad::Behaviour::new(local_peer_id, kad_store);
    kad.set_mode(Some(kad::Mode::Server));

    // Identify so peers learn our observed address.
    let identify = identify::Behaviour::new(identify::Config::new(
        format!("/hone/{}/1.0.0", cfg.chain_id),
        keypair.public(),
    ).with_push_listen_addr_updates(true));

    let ping = ping::Behaviour::default();

    let mdns = match mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id) {
        Ok(m) => Some(m),
        Err(e) => {
            warn!("net: mDNS unavailable ({}), continuing without LAN discovery", e);
            None
        }
    };
    let mdns = libp2p::swarm::behaviour::toggle::Toggle::from(mdns);

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        // Android has no /etc/resolv.conf, so the system resolver config cannot be read:
        // use a fixed public resolver for /dns4 and /dnsaddr seeds.
        .with_dns_config(libp2p::dns::ResolverConfig::cloudflare(), libp2p::dns::ResolverOpts::default())
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(|key, relay_client| Behaviour {
            gossipsub, kad, identify, ping, mdns, relay_client,
            dcutr: dcutr::Behaviour::new(key.public().to_peer_id()),
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    // Listen on all interfaces.
    let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", cfg.p2p_port)
        .parse().expect("listen addr");
    // Non-fatal: if the OS refuses the listener (e.g. interface enumeration is
    // blocked on newer Android), outbound dials to bootstrap peers still work.
    if let Err(e) = swarm.listen_on(listen_addr) {
        warn!("net: listen failed ({}), running outbound-only", e);
    }

    // Dial bootstrap peers: the app's list, then the public seeds.
    let mut boot: Vec<String> = cfg.bootstrap_peers.clone();
    for d in DEFAULT_BOOTSTRAP { if !boot.iter().any(|b| b == d) { boot.push(d.to_string()); } }
    for peer_addr in &boot {
        if let Ok(addr) = peer_addr.parse::<Multiaddr>() {
            if let Some(Protocol::P2p(id)) = addr.iter().last() {
                swarm.behaviour_mut().kad.add_address(&id, addr.clone());
                let _ = swarm.dial(DialOpts::peer_id(id).addresses(vec![addr])
                    .condition(PeerCondition::DisconnectedAndNotDialing).build());
            } else {
                swarm.dial(addr).ok();
            }
        }
    }
    // Relay state: the address we dialed each peer on, and the relay we reserved on.
    let mut dialed: std::collections::HashMap<PeerId, Multiaddr> = std::collections::HashMap::new();
    let mut reserved: Option<PeerId> = None;
    let mut mdns_dialed: std::collections::HashMap<PeerId, std::time::Instant> = std::collections::HashMap::new();

    // Fetch peers from Hive and honemesh.net concurrently.
    let (hive_peers, hone_net_peers) = tokio::join!(
        fetch_hive_peers(&cfg.chain_id),
        fetch_hone_net_peers(&cfg.chain_id),
    );
    for peers in [hive_peers.unwrap_or_default(), hone_net_peers.unwrap_or_default()] {
        for addr in peers {
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
                if cfg.read_only || (cfg.seals_only && topic != "hone/seals") {
                    tracing::debug!("net: publish not allowed in this mode, dropped {} ({} bytes)", topic, data.len());
                    continue;
                }
                let t = gossipsub::IdentTopic::new(format!("{}/{}", cfg.chain_id, topic));
                match swarm.behaviour_mut().gossipsub.publish(t, data) {
                    Ok(_) => info!("net: published on {}", topic),
                    Err(e) => warn!("net: publish error on {}: {}", topic, e),
                }
            }

            // Swarm events.
            event = swarm.select_next_some() => {
                // Keep the UI peer count current (count distinct peers, not connections).
                match &event {
                    SwarmEvent::ConnectionEstablished { num_established, .. } if num_established.get() == 1 => {
                        cfg.peers.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    SwarmEvent::ConnectionClosed { num_established: 0, .. } => {
                        let _ = cfg.peers.fetch_update(
                            std::sync::atomic::Ordering::Relaxed,
                            std::sync::atomic::Ordering::Relaxed,
                            |n| Some(n.saturating_sub(1)),
                        );
                    }
                    _ => {}
                }
                // Handle mDNS here so we can dial discovered addresses: one dial per peer
                // on its LAN address, skipped only if already connected (a hung dial to
                // another address of the same peer must not block it).
                if let SwarmEvent::Behaviour(BehaviourEvent::Mdns(
                    mdns::Event::Discovered(peers)
                )) = &event {
                    let mut by_peer: std::collections::HashMap<PeerId, Vec<Multiaddr>> = std::collections::HashMap::new();
                    for (peer_id, addr) in peers.clone() {
                        info!("net: mDNS local peer {} at {}", peer_id, addr);
                        swarm.behaviour_mut().kad.add_address(&peer_id, addr.clone());
                        let tcp = addr.iter().any(|p| matches!(p, Protocol::Tcp(_)))
                            && !addr.iter().any(|p| matches!(p, Protocol::Ws(_) | Protocol::Wss(_) | Protocol::QuicV1));
                        if tcp { by_peer.entry(peer_id).or_default().push(addr); }
                    }
                    // All of a peer's LAN TCP addresses in ONE dial (the swarm keeps the first
                    // that connects), at most every 20 s per peer, so mDNS bursts do not open
                    // parallel connections.
                    for (peer_id, addrs) in by_peer {
                        if swarm.is_connected(&peer_id) { continue; }
                        let now = std::time::Instant::now();
                        if mdns_dialed.get(&peer_id).map_or(false, |t| now.duration_since(*t) < Duration::from_secs(20)) { continue; }
                        mdns_dialed.insert(peer_id, now);
                        let _ = swarm.dial(DialOpts::peer_id(peer_id).addresses(addrs)
                            .condition(PeerCondition::Disconnected).build());
                    }
                }
                // Remember how we reached each peer; reserve a relay slot on the first
                // relay-capable peer we dialed (the public seed).
                if let SwarmEvent::ConnectionEstablished { peer_id, endpoint: ConnectedPoint::Dialer { address, .. }, .. } = &event {
                    if !address.iter().any(|p| matches!(p, Protocol::P2pCircuit)) { dialed.insert(*peer_id, address.clone()); }
                }
                if let SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received { peer_id, info, .. })) = &event {
                    if reserved.is_none() && info.protocols.iter().any(|p| *p == relay::HOP_PROTOCOL_NAME) {
                        if let Some(a) = dialed.get(peer_id) {
                            let base = if a.iter().any(|p| matches!(p, Protocol::P2p(_))) { a.clone() } else { a.clone().with(Protocol::P2p(*peer_id)) };
                            let circuit = base.with(Protocol::P2pCircuit);
                            match swarm.listen_on(circuit.clone()) {
                                Ok(_) => { info!("net: relay reservation requested on {}", circuit); reserved = Some(*peer_id); }
                                Err(e) => warn!("net: relay listen failed: {}", e),
                            }
                        }
                    }
                }
                if let SwarmEvent::ListenerClosed { addresses, .. } = &event {
                    if addresses.iter().any(|a| a.iter().any(|p| matches!(p, Protocol::P2pCircuit))) { reserved = None; }
                }
                if let SwarmEvent::Behaviour(BehaviourEvent::Dcutr(e)) = &event {
                    match &e.result {
                        Ok(_) => info!("net: dcutr direct connection to {}", e.remote_peer_id),
                        Err(err) => info!("net: dcutr to {} failed, staying relayed: {}", e.remote_peer_id, err),
                    }
                }
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
            } else if topic.contains("consensus") {
                if let Ok(v) = serde_json::from_slice::<Value>(&data) {
                    if let (Some(epoch), Some(rewards_hash), Some(node_id)) = (
                        v.get("epoch").and_then(|e| e.as_u64()),
                        v.get("rewards_hash").and_then(|h| h.as_str()).map(str::to_owned),
                        v.get("node_id").and_then(|n| n.as_str()).map(str::to_owned),
                    ) {
                        let _ = event_tx.send(NetworkEvent::ConsensusProposal { epoch, rewards_hash, node_id }).await;
                    }
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

// ── honemesh.net peer discovery ──────────────────────────────────────────────────

async fn fetch_hone_net_peers(chain_id: &str) -> Result<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let resp: serde_json::Value = client
        .get("https://honemesh.net/api/peers/bootstrap")
        .query(&[("chain_id", chain_id)])
        .send().await?
        .json().await?;

    let peers = resp
        .get("peers")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default();

    Ok(peers)
}

// ── Hive peer discovery ───────────────────────────────────────────────────────

async fn fetch_hive_peers(chain_id: &str) -> Result<Vec<String>> {
    let key = if chain_id == hone_types::TESTNET_CHAIN_ID {
        "hone_satoshi_peers"
    } else {
        "hone_peers"
    };

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "condenser_api.get_accounts",
        "params": [["hone"]],
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Live smoke test (needs internet): with an EMPTY app bootstrap list the phone core
    /// reaches the public seed through the built-in /dns4 default, read-only.
    /// cargo test --release -- --ignored public_seed_reachable --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn public_seed_reachable() {
        let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).try_init();
        let dir = std::env::temp_dir().join(format!("hone-android-smoke-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let peers = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cfg = SwarmConfig {
            chain_id: "hone".into(), data_dir: dir.to_string_lossy().into(), p2p_port: 0,
            bootstrap_peers: vec![], read_only: true, seals_only: false, peers: peers.clone(),
        };
        let (etx, mut erx) = mpsc::channel(1024);
        let (_ctx, crx) = mpsc::channel(8);
        let (stx, srx) = broadcast::channel(1);
        let h = tokio::spawn(run_swarm(cfg, etx, crx, srx));
        let mut seals = 0;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        while tokio::time::Instant::now() < deadline {
            if let Ok(Some(NetworkEvent::EpochSeal { .. })) = tokio::time::timeout(Duration::from_secs(1), erx.recv()).await { seals += 1; }
        }
        let _ = stx.send(());
        let _ = h.await;
        let n = peers.load(std::sync::atomic::Ordering::Relaxed);
        println!("peers={} seals={}", n, seals);
        assert!(n >= 1, "no peer reached through the public seed");
        assert!(seals >= 1, "no seal gossip received");
    }
}
