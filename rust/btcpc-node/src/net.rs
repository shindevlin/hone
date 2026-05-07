//! btcpc-node P2P networking — libp2p 0.56
#![allow(dead_code)]
//!
//! Gossipsub topics:
//!   btcpc/blocks     — serialized block bytes (JSON envelope: {"epoch":N,"data_b64":"..."})
//!   btcpc/entries    — ledger entries (JSON)
//!   btcpc/seals      — epoch seals (JSON)
//!   btcpc/sync       — block sync requests/responses
//!   btcpc/consensus  — reward hash proposals (JSON: {epoch,rewards_hash,node_id})
//!
//! Public surface:
//!   Network::new(config, event_tx) -> (Network, NetworkHandle)
//!   Network::run()
//!   NetworkHandle::cmd_tx  (send NetCmd)
//!   NetworkEvent           (received on the broadcast channel)

use crate::config::Config;
use crate::discovery;
use crate::store::Store;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use libp2p::futures::StreamExt;
use libp2p::{
    gossipsub, identify, kad, mdns,
    kad::store::MemoryStore,
    noise, ping,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, SwarmBuilder,
};
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::fs;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Events emitted by the network layer and consumed by other tasks.
#[derive(Debug, Clone)]
pub enum NetworkEvent {
    /// Raw block bytes received from a peer.
    Block { epoch: u64, data: Vec<u8> },
    /// A single ledger entry received over gossip.
    Entry { entry: serde_json::Value },
    /// An EPOCH_SEAL received over gossip.
    EpochSeal { seal: serde_json::Value },
    /// A reward consensus proposal received from a peer clock node.
    ConsensusProposal { epoch: u64, rewards_hash: String, node_id: String },
    /// A new peer connection was established.
    PeerConnected { peer_id: String },
    /// An existing peer connection was closed.
    PeerDisconnected { peer_id: String },
    /// A `LinkGitRefUpdate` arrived over gossip — storage sync signal.
    /// Other nodes should fetch missing objects from `peer_http` if set.
    GitRefAvailable {
        repo_id: String,
        owner: String,
        repo_name: String,
        commit_hash: String,
        /// HTTP base URL of the peer that gossiped the update, e.g. "http://1.2.3.4:4242".
        peer_http: Option<String>,
    },
}

/// Commands that other tasks can send to the network layer.
pub enum NetCmd {
    /// Publish `data` on the given gossipsub topic name.
    Broadcast { topic: &'static str, data: Vec<u8> },
    /// Request block for `epoch` from `peer` (or any peer if None).
    RequestBlock { epoch: u32, peer: Option<String> },
}

/// Cheap clone handle used by other tasks to send commands to the network.
pub struct NetworkHandle {
    pub cmd_tx: mpsc::Sender<NetCmd>,
}

// ---------------------------------------------------------------------------
// Behaviour
// ---------------------------------------------------------------------------

#[derive(NetworkBehaviour)]
struct BtcpcBehaviour {
    gossipsub: gossipsub::Behaviour,
    kademlia:  kad::Behaviour<MemoryStore>,
    identify:  identify::Behaviour,
    ping:      ping::Behaviour,
    mdns:      mdns::tokio::Behaviour,
}

// ---------------------------------------------------------------------------
// Topic names
// ---------------------------------------------------------------------------

const TOPIC_BLOCKS:     &str = "btcpc/blocks";
const TOPIC_ENTRIES:    &str = "btcpc/entries";
const TOPIC_SEALS:      &str = "btcpc/seals";
const TOPIC_SYNC:       &str = "btcpc/sync";
const TOPIC_CONSENSUS:  &str = "btcpc/consensus";

// ---------------------------------------------------------------------------
// Network struct
// ---------------------------------------------------------------------------

pub struct Network {
    config:   Config,
    store:    Store,
    cmd_rx:   mpsc::Receiver<NetCmd>,
    event_tx: broadcast::Sender<NetworkEvent>,
    /// Maps PeerId string → derived HTTP base URL ("http://{ip}:4242").
    /// Populated from libp2p Identify responses; used for storage sync.
    peer_http_cache: Arc<parking_lot::Mutex<HashMap<String, String>>>,
}

impl Network {
    /// Create a new `Network` and the associated handle + event channel.
    ///
    /// Returns `(Network, NetworkHandle, broadcast::Receiver<NetworkEvent>)`.
    pub fn new(config: Config, store: Store) -> (Self, NetworkHandle, broadcast::Receiver<NetworkEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(256);
        let (event_tx, event_rx) = broadcast::channel(1024);

        let peer_http_cache = Arc::new(parking_lot::Mutex::new(HashMap::new()));
        let network = Self { config, store, cmd_rx, event_tx, peer_http_cache };
        let handle  = NetworkHandle { cmd_tx };
        (network, handle, event_rx)
    }

    /// Drive the P2P swarm until the process exits, restarting on panic.
    ///
    /// Each restart gets a fresh swarm but the same keypair so our PeerId is
    /// stable. Commands flow from the real `cmd_rx` through a per-restart
    /// relay channel so external senders never block or need to know about
    /// restarts. A yamux-exploit crash therefore only kills the inner task;
    /// the outer loop absorbs it and brings the swarm back up after 3 s.
    pub async fn run(mut self) -> Result<()> {
        let keypair = load_or_create_keypair(&self.config.data_dir)?;
        let mut restart_count = 0u32;
        loop {
            // Per-restart relay: outer loop owns cmd_rx, inner task owns relay_rx.
            let (relay_tx, relay_rx) = mpsc::channel::<NetCmd>(256);
            let inner = Network {
                config:          self.config.clone(),
                store:           self.store.clone(),
                cmd_rx:          relay_rx,
                event_tx:        self.event_tx.clone(),
                peer_http_cache: self.peer_http_cache.clone(),
            };
            let kp = keypair.clone();
            let mut handle = tokio::spawn(async move { inner.run_inner(kp).await });

            // Forward real commands to the inner task; stop when inner exits.
            let join_result = loop {
                tokio::select! {
                    msg = self.cmd_rx.recv() => match msg {
                        Some(cmd) => { let _ = relay_tx.send(cmd).await; }
                        None => return Ok(()), // all cmd_tx senders dropped
                    },
                    result = &mut handle => break result,
                }
            };
            drop(relay_tx); // signal inner task the relay is gone

            match join_result {
                Ok(Ok(_))  => return Ok(()), // clean shutdown
                Ok(Err(e)) => {
                    warn!("P2P swarm error [restart #{}]: {}; retrying in 3s", restart_count, e);
                }
                Err(ref e) if e.is_panic() => {
                    warn!(
                        "P2P swarm panicked [restart #{}] — possible yamux exploit; \
                         restarting in 3s",
                        restart_count
                    );
                }
                Err(_) => return Ok(()), // task cancelled
            }
            restart_count += 1;
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }

    /// Inner swarm task — runs until clean exit or panic.  Called by `run()`.
    async fn run_inner(mut self, keypair: libp2p::identity::Keypair) -> Result<()> {
        // ------------------------------------------------------------------
        // 2. Build swarm
        // ------------------------------------------------------------------
        let mut swarm = SwarmBuilder::with_existing_identity(keypair.clone())
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_websocket(
                noise::Config::new,
                yamux::Config::default,
            )
            .await?
            .with_behaviour(|key| {
                // Gossipsub — content-hash message dedup
                let msg_id_fn = |msg: &gossipsub::Message| {
                    let mut h = DefaultHasher::new();
                    msg.data.hash(&mut h);
                    gossipsub::MessageId::from(h.finish().to_string())
                };

                let gossipsub_config = gossipsub::ConfigBuilder::default()
                    .heartbeat_interval(Duration::from_secs(10))
                    .validation_mode(gossipsub::ValidationMode::Permissive)
                    .message_id_fn(msg_id_fn)
                    .mesh_n(6)
                    .mesh_n_low(4)
                    .mesh_n_high(12)
                    // 10 MB — blocks can be large
                    .max_transmit_size(10 * 1024 * 1024)
                    .build()
                    .map_err(|e| anyhow::anyhow!(e))?;

                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )?;

                let mut kademlia = kad::Behaviour::new(
                    key.public().to_peer_id(),
                    MemoryStore::new(key.public().to_peer_id()),
                );
                kademlia.set_mode(Some(kad::Mode::Server));

                let identify = identify::Behaviour::new(identify::Config::new(
                    "/btcpc/1.0.0".to_string(),
                    key.public(),
                ));

                let ping = ping::Behaviour::new(
                    ping::Config::new()
                        .with_interval(Duration::from_secs(30))
                        .with_timeout(Duration::from_secs(30)),
                );

                let mdns = mdns::tokio::Behaviour::new(
                    mdns::Config::default(),
                    key.public().to_peer_id(),
                ).map_err(|e| anyhow::anyhow!("mDNS init failed: {}", e))?;

                Ok(BtcpcBehaviour { gossipsub, kademlia, identify, ping, mdns })
            })?
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(Duration::from_secs(60))
            })
            .build();

        // ------------------------------------------------------------------
        // 3. Subscribe to topics
        // ------------------------------------------------------------------
        let t_blocks    = gossipsub::IdentTopic::new(TOPIC_BLOCKS);
        let t_entries   = gossipsub::IdentTopic::new(TOPIC_ENTRIES);
        let t_seals     = gossipsub::IdentTopic::new(TOPIC_SEALS);
        let t_sync      = gossipsub::IdentTopic::new(TOPIC_SYNC);
        let t_consensus = gossipsub::IdentTopic::new(TOPIC_CONSENSUS);

        swarm.behaviour_mut().gossipsub.subscribe(&t_blocks)?;
        swarm.behaviour_mut().gossipsub.subscribe(&t_entries)?;
        swarm.behaviour_mut().gossipsub.subscribe(&t_seals)?;
        swarm.behaviour_mut().gossipsub.subscribe(&t_sync)?;
        swarm.behaviour_mut().gossipsub.subscribe(&t_consensus)?;

        // ------------------------------------------------------------------
        // 4. Listeners
        // ------------------------------------------------------------------
        let listen_tcp: Multiaddr =
            format!("/ip4/0.0.0.0/tcp/{}", self.config.p2p_port).parse()?;
        swarm.listen_on(listen_tcp)?;

        // WebSocket transport — allows nodes behind NAT to connect via the
        // cloudflared tunnel at wss://p2p.btcpc.net (port 443 → local 4943).
        let ws_port = std::env::var("BTCPC_WS_PORT")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(4943u16);
        let listen_ws: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}/ws", ws_port).parse()?;
        swarm.listen_on(listen_ws)?;

        // ------------------------------------------------------------------
        // 5. Load stored peers + run discovery, then dial all
        // ------------------------------------------------------------------

        // Layer 1: peers from RocksDB peer store (instant, no network).
        let stored_peers = load_peer_store(&self.store);
        if !stored_peers.is_empty() {
            info!("peer store: {} cached peers", stored_peers.len());
        }
        for addr in &stored_peers {
            let _ = swarm.dial(addr.clone());
        }

        // Layer 2: Cloudflare DNS seeds (bootstrap_peers from config — already dialed below
        //           after discovery so they're all handled in one unified pass).

        // Layer 3: Hive + TON registry (10s timeout — best effort).
        let chain_id = self.config.chain_id.clone();
        let discovered = tokio::time::timeout(
            Duration::from_secs(10),
            discovery::fetch_all_peers(&chain_id),
        )
        .await
        .unwrap_or_default();

        // Dial config bootstrap peers + discovery peers together.
        let all_bootstrap: Vec<String> = self.config.bootstrap_peers.iter()
            .cloned()
            .chain(discovered.into_iter())
            .collect();

        for addr_str in &all_bootstrap {
            match addr_str.parse::<Multiaddr>() {
                Ok(addr) => {
                    info!("Dialing bootstrap peer: {}", addr);
                    let _ = swarm.dial(addr);
                }
                Err(e) => warn!("Invalid bootstrap addr '{}': {}", addr_str, e),
            }
        }

        // Announce own WS multiaddr to btcpc.net peer registry.
        // BTCPC_ANNOUNCE_ADDR overrides; otherwise builds from peer_id + p2p.btcpc.net.
        {
            let peer_id = swarm.local_peer_id().to_string();
            let chain_id = self.config.chain_id.clone();
            let node_id  = self.config.node_id.clone();
            let announce_addr = std::env::var("BTCPC_ANNOUNCE_ADDR").unwrap_or_else(|_| {
                format!("/dns4/p2p.btcpc.net/tcp/443/wss/p2p/{}", peer_id)
            });
            // Fire-and-forget; never blocks startup.
            tokio::spawn(async move {
                // Small delay so the swarm is fully initialised.
                tokio::time::sleep(Duration::from_secs(5)).await;
                discovery::announce_to_btcpc_net_addr(&announce_addr, &chain_id, &node_id).await;
            });
        }

        // Kick off Kademlia bootstrap (harmless if no peers yet).
        if let Err(e) = swarm.behaviour_mut().kademlia.bootstrap() {
            warn!("Kademlia bootstrap queued (expected on first start): {}", e);
        }

        info!(
            peer_id = %swarm.local_peer_id(),
            port    = self.config.p2p_port,
            "btcpc-node P2P running",
        );

        // ------------------------------------------------------------------
        // 6. Timers
        // ------------------------------------------------------------------
        let mut rebootstrap   = tokio::time::interval(Duration::from_secs(300));
        let mut peer_flush    = tokio::time::interval(Duration::from_secs(300));
        rebootstrap.tick().await; // discard immediate first tick
        peer_flush.tick().await;

        // In-memory peer cache: peer_id_str → listen addrs.
        // Populated by Identify events; flushed to RocksDB every 5 minutes.
        let mut peer_cache: HashMap<String, Vec<Multiaddr>> = HashMap::new();

        // ------------------------------------------------------------------
        // 7. Event loop
        // ------------------------------------------------------------------
        loop {
            tokio::select! {
                event = swarm.select_next_some() => {
                    self.handle_swarm_event(event, &mut swarm, &mut peer_cache);
                }
                Some(cmd) = self.cmd_rx.recv() => {
                    self.handle_cmd(cmd, &mut swarm,
                        &t_blocks, &t_entries, &t_seals, &t_sync, &t_consensus);
                }
                _ = rebootstrap.tick() => {
                    let connected = swarm.connected_peers().count();
                    if connected < 3 {
                        debug!(connected, "Re-bootstrapping Kademlia (low peer count)");
                        if let Err(e) = swarm.behaviour_mut().kademlia.bootstrap() {
                            debug!("Kademlia re-bootstrap: {}", e);
                        }
                    }
                }
                _ = peer_flush.tick() => {
                    save_peer_store(&self.store, &peer_cache);
                    debug!("peer store: flushed {} peer entries", peer_cache.len());
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Swarm event handler
    // -----------------------------------------------------------------------

    fn handle_swarm_event(
        &self,
        event: SwarmEvent<BtcpcBehaviourEvent>,
        swarm: &mut libp2p::Swarm<BtcpcBehaviour>,
        peer_cache: &mut HashMap<String, Vec<Multiaddr>>,
    ) {
        match event {
            // Listeners
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("Listening on {}", address);
            }

            // Peer lifecycle
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                info!("Peer connected: {}", peer_id);
                self.emit(NetworkEvent::PeerConnected { peer_id: peer_id.to_string() });
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                debug!("Peer disconnected: {} ({:?})", peer_id, cause);
                self.emit(NetworkEvent::PeerDisconnected { peer_id: peer_id.to_string() });
            }

            // Gossipsub message
            SwarmEvent::Behaviour(BtcpcBehaviourEvent::Gossipsub(
                gossipsub::Event::Message { message, propagation_source, .. }
            )) => {
                self.handle_gossip_message(&message, propagation_source.to_string());
            }

            // Identify — feed addresses into Kademlia, persist, and derive HTTP URL.
            SwarmEvent::Behaviour(BtcpcBehaviourEvent::Identify(
                identify::Event::Received { peer_id, info, .. }
            )) => {
                let cache_entry = peer_cache
                    .entry(peer_id.to_string())
                    .or_default();
                for addr in &info.listen_addrs {
                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                    if !cache_entry.contains(addr) {
                        cache_entry.push(addr.clone());
                    }
                }
                // Derive HTTP base URL from the first routable IP4 address.
                if let Some(http_url) = info.listen_addrs.iter()
                    .find_map(|a| multiaddr_to_http(a))
                {
                    self.peer_http_cache.lock().insert(peer_id.to_string(), http_url);
                }
            }

            // Kademlia bootstrap progress
            SwarmEvent::Behaviour(BtcpcBehaviourEvent::Kademlia(
                kad::Event::OutboundQueryProgressed { result, .. }
            )) => {
                if let kad::QueryResult::Bootstrap(Ok(kad::BootstrapOk { num_remaining, .. })) = result {
                    if num_remaining == 0 {
                        info!("Kademlia DHT bootstrap complete");
                    }
                }
            }

            // Ping
            SwarmEvent::Behaviour(BtcpcBehaviourEvent::Ping(
                ping::Event { peer, result, .. }
            )) => {
                if let Err(e) = result {
                    debug!("Ping failed for {}: {}", peer, e);
                }
            }

            // mDNS — local network peer discovery
            SwarmEvent::Behaviour(BtcpcBehaviourEvent::Mdns(
                mdns::Event::Discovered(peers)
            )) => {
                for (peer_id, addr) in peers {
                    info!("mDNS: discovered {} at {}", peer_id, addr);
                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                    let _ = swarm.dial(addr);
                }
            }
            SwarmEvent::Behaviour(BtcpcBehaviourEvent::Mdns(
                mdns::Event::Expired(peers)
            )) => {
                for (peer_id, _addr) in peers {
                    if !swarm.is_connected(&peer_id) {
                        swarm.behaviour_mut().kademlia.remove_peer(&peer_id);
                    }
                }
            }

            // Listener errors / close
            SwarmEvent::ListenerError { listener_id, error } => {
                warn!("Listener {:?} error: {}", listener_id, error);
            }
            SwarmEvent::ListenerClosed { listener_id, addresses, reason } => {
                warn!(
                    "Listener {:?} closed (addresses: {:?}, reason: {:?})",
                    listener_id, addresses, reason
                );
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                debug!("Outgoing connection error (peer {:?}): {}", peer_id, error);
            }

            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Gossip message dispatch
    // -----------------------------------------------------------------------

    fn handle_gossip_message(
        &self,
        message: &gossipsub::Message,
        source: String,
    ) {
        // Resolve topic string from hash — we know all four topics
        let topic_str = {
            let h = &message.topic;
            if      h == &gossipsub::IdentTopic::new(TOPIC_BLOCKS).hash()     { TOPIC_BLOCKS     }
            else if h == &gossipsub::IdentTopic::new(TOPIC_ENTRIES).hash()    { TOPIC_ENTRIES    }
            else if h == &gossipsub::IdentTopic::new(TOPIC_SEALS).hash()      { TOPIC_SEALS      }
            else if h == &gossipsub::IdentTopic::new(TOPIC_SYNC).hash()       { TOPIC_SYNC       }
            else if h == &gossipsub::IdentTopic::new(TOPIC_CONSENSUS).hash()  { TOPIC_CONSENSUS  }
            else {
                debug!("Message on unknown topic hash {:?}", h);
                return;
            }
        };

        let json: serde_json::Value = match serde_json::from_slice(&message.data) {
            Ok(v)  => v,
            Err(e) => {
                warn!("Non-JSON gossip on {} from {}: {}", topic_str, source, e);
                return;
            }
        };

        match topic_str {
            TOPIC_BLOCKS => {
                // {"epoch": N, "data_b64": "..."}
                let epoch = match json.get("epoch").and_then(|v| v.as_u64()) {
                    Some(e) => e,
                    None    => { warn!("btcpc/blocks: missing 'epoch' field"); return; }
                };
                let data_b64 = match json.get("data_b64").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None    => { warn!("btcpc/blocks: missing 'data_b64' field"); return; }
                };
                let data = match B64.decode(data_b64) {
                    Ok(b)  => b,
                    Err(e) => { warn!("btcpc/blocks: base64 decode error: {}", e); return; }
                };
                debug!("Received block epoch={} ({} bytes) from {}", epoch, data.len(), source);
                self.emit(NetworkEvent::Block { epoch, data });
            }

            TOPIC_ENTRIES => {
                debug!("Received entry from {}", source);

                // If this is a LinkGitRefUpdate, also emit a GitRefAvailable so
                // the storage sync loop can fetch any missing objects.
                if let Some(entry_obj) = json.get("entry").or(Some(&json)) {
                    if entry_obj.get("LinkGitRefUpdate").is_some()
                        || entry_obj.get("type").and_then(|t| t.as_str()) == Some("LinkGitRefUpdate")
                    {
                        let v = entry_obj.get("LinkGitRefUpdate").unwrap_or(entry_obj);
                        let repo_id     = v["repo_id"].as_str().unwrap_or("").to_owned();
                        let owner       = v["owner"].as_str().unwrap_or("").to_owned();
                        let repo_name   = v["repo_name"].as_str().unwrap_or("").to_owned();
                        let commit_hash = v["commit_hash"].as_str().unwrap_or("").to_owned();
                        if !repo_id.is_empty() && !commit_hash.is_empty() {
                            let peer_http = self.peer_http_cache.lock().get(&source).cloned();
                            self.emit(NetworkEvent::GitRefAvailable {
                                repo_id, owner, repo_name, commit_hash, peer_http,
                            });
                        }
                    }
                }

                self.emit(NetworkEvent::Entry { entry: json });
            }

            TOPIC_SEALS => {
                info!("Received epoch seal from {}", source);
                self.emit(NetworkEvent::EpochSeal { seal: json });
            }

            TOPIC_CONSENSUS => {
                let epoch = match json.get("epoch").and_then(|v| v.as_u64()) {
                    Some(e) => e,
                    None => { warn!("btcpc/consensus: missing 'epoch'"); return; }
                };
                let rewards_hash = match json.get("rewards_hash").and_then(|v| v.as_str()) {
                    Some(h) => h.to_owned(),
                    None => { warn!("btcpc/consensus: missing 'rewards_hash'"); return; }
                };
                let node_id = match json.get("node_id").and_then(|v| v.as_str()) {
                    Some(n) => n.to_owned(),
                    None => { warn!("btcpc/consensus: missing 'node_id'"); return; }
                };
                debug!("Received consensus proposal epoch={} hash={} from {}", epoch, rewards_hash, source);
                self.emit(NetworkEvent::ConsensusProposal { epoch, rewards_hash, node_id });
            }

            TOPIC_SYNC => {
                // {"type":"request","epoch":N} or {"type":"response","epoch":N,"data_b64":"..."}
                match json.get("type").and_then(|v| v.as_str()) {
                    Some("response") => {
                        let epoch = match json.get("epoch").and_then(|v| v.as_u64()) {
                            Some(e) => e,
                            None    => { warn!("btcpc/sync response: missing 'epoch'"); return; }
                        };
                        let data_b64 = match json.get("data_b64").and_then(|v| v.as_str()) {
                            Some(s) => s,
                            None    => { warn!("btcpc/sync response: missing 'data_b64'"); return; }
                        };
                        let data = match B64.decode(data_b64) {
                            Ok(b)  => b,
                            Err(e) => { warn!("btcpc/sync: base64 decode error: {}", e); return; }
                        };
                        debug!("Received sync response epoch={} from {}", epoch, source);
                        self.emit(NetworkEvent::Block { epoch, data });
                    }
                    Some("request") => {
                        // Peer wants a block; other tasks handle fulfilling it via Broadcast
                        debug!(
                            "Sync request epoch={:?} from {}",
                            json.get("epoch"),
                            source
                        );
                    }
                    other => {
                        warn!("btcpc/sync: unknown type {:?}", other);
                    }
                }
            }

            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // NetCmd handler
    // -----------------------------------------------------------------------

    fn handle_cmd(
        &self,
        cmd: NetCmd,
        swarm: &mut libp2p::Swarm<BtcpcBehaviour>,
        t_blocks:    &gossipsub::IdentTopic,
        t_entries:   &gossipsub::IdentTopic,
        t_seals:     &gossipsub::IdentTopic,
        t_sync:      &gossipsub::IdentTopic,
        t_consensus: &gossipsub::IdentTopic,
    ) {
        match cmd {
            NetCmd::Broadcast { topic, data } => {
                let ident_topic = match topic {
                    TOPIC_BLOCKS     => t_blocks.clone(),
                    TOPIC_ENTRIES    => t_entries.clone(),
                    TOPIC_SEALS      => t_seals.clone(),
                    TOPIC_SYNC       => t_sync.clone(),
                    TOPIC_CONSENSUS  => t_consensus.clone(),
                    other => {
                        warn!("NetCmd::Broadcast: unknown topic '{}'", other);
                        return;
                    }
                };
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(ident_topic, data) {
                    warn!("Gossipsub publish on '{}' failed: {}", topic, e);
                }
            }

            NetCmd::RequestBlock { epoch, peer: _ } => {
                // Publish a sync request; any peer with that block will respond via btcpc/sync
                let payload = match serde_json::to_vec(&serde_json::json!({
                    "type": "request",
                    "epoch": epoch,
                })) {
                    Ok(b)  => b,
                    Err(e) => { warn!("RequestBlock serialize error: {}", e); return; }
                };
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(t_sync.clone(), payload) {
                    warn!("Gossipsub sync request publish failed: {}", e);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Helper: emit a NetworkEvent (best-effort — dropped if no receivers)
    // -----------------------------------------------------------------------

    fn emit(&self, event: NetworkEvent) {
        // broadcast::Sender::send returns Err only if there are no receivers;
        // that is not an error condition — silently discard.
        let _ = self.event_tx.send(event);
    }
}

// ---------------------------------------------------------------------------
// Peer store — persist discovered multiaddrs across restarts
// ---------------------------------------------------------------------------

/// RocksDB meta key for the serialised peer address list.
const PEER_STORE_KEY: &str = "peer_addrs";

/// Maximum peers to persist (keeps the store small; Kademlia handles the rest).
const MAX_STORED_PEERS: usize = 100;

/// Load cached peer multiaddrs from RocksDB.  Returns an empty list on miss or error.
fn load_peer_store(store: &Store) -> Vec<Multiaddr> {
    store
        .get_meta(PEER_STORE_KEY)
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// Flush the in-memory peer cache to RocksDB.
/// Skips loopback / unspecified addresses — they're useless to other nodes.
fn save_peer_store(store: &Store, cache: &HashMap<String, Vec<Multiaddr>>) {
    let mut addrs: Vec<String> = cache
        .iter()
        .flat_map(|(peer_id, addrs)| {
            addrs.iter().filter_map(move |addr| {
                let s = addr.to_string();
                if s.contains("/127.0.0.1/")
                    || s.contains("/::1/")
                    || s.contains("/0.0.0.0/")
                {
                    return None;
                }
                Some(format!("{}/p2p/{}", s, peer_id))
            })
        })
        .collect();

    addrs.sort();
    addrs.dedup();
    addrs.truncate(MAX_STORED_PEERS);

    if let Ok(json) = serde_json::to_vec(&addrs) {
        let _ = store.set_meta(PEER_STORE_KEY, &json);
    }
}

// ---------------------------------------------------------------------------
// Key management — load or create an ED25519 keypair from data_dir/p2p-key
// ---------------------------------------------------------------------------

/// Stored key format (JSON, restricted 0600 permissions).
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredKey {
    private_key_hex: String,
    peer_id:         String,
}

fn load_or_create_keypair(data_dir: &std::path::Path) -> Result<libp2p::identity::Keypair> {
    use libp2p::identity::{self, Keypair};

    let path = data_dir.join("p2p-key");

    if path.exists() {
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read keypair from {}", path.display()))?;
        let stored: StoredKey = serde_json::from_str(&contents)
            .with_context(|| "Failed to parse p2p-key JSON")?;
        let bytes = hex::decode(&stored.private_key_hex)
            .with_context(|| "Failed to hex-decode p2p-key")?;
        let keypair = Keypair::ed25519_from_bytes(bytes)
            .with_context(|| "Failed to restore ED25519 keypair")?;
        info!(
            peer_id = %keypair.public().to_peer_id(),
            "Loaded existing P2P identity",
        );
        return Ok(keypair);
    }

    // First start — generate and persist
    let keypair   = identity::Keypair::generate_ed25519();
    let peer_id   = keypair.public().to_peer_id();

    let private_bytes = keypair
        .clone()
        .try_into_ed25519()
        .map_err(|_| anyhow::anyhow!("Not an ED25519 keypair"))?
        .secret()
        .as_ref()
        .to_vec();

    let stored = StoredKey {
        private_key_hex: hex::encode(&private_bytes),
        peer_id:         peer_id.to_string(),
    };

    fs::create_dir_all(data_dir)
        .with_context(|| format!("Failed to create data_dir {}", data_dir.display()))?;

    let json = serde_json::to_string_pretty(&stored)?;
    fs::write(&path, &json)
        .with_context(|| format!("Failed to write p2p-key to {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }

    info!(
        peer_id = %peer_id,
        path    = %path.display(),
        "Generated new P2P identity",
    );

    // Re-derive from saved bytes for a consistent return
    let keypair = Keypair::ed25519_from_bytes(private_bytes)
        .with_context(|| "Failed to restore generated keypair")?;
    Ok(keypair)
}

// ── Utility ───────────────────────────────────────────────────────────────────

/// Extract the HTTP base URL for a peer from one of their listen Multiaddrs.
///
/// Maps `/ip4/{ip}/tcp/{any_port}` → `"http://{ip}:4242"`.
/// Returns None for loopback, link-local, IPv6-only, or unrecognised formats.
fn multiaddr_to_http(addr: &Multiaddr) -> Option<String> {
    // Walk the multiaddr protocol components as text.
    let s = addr.to_string();
    // Expect /ip4/{ip}/tcp/{port}[/...]
    let parts: Vec<&str> = s.splitn(5, '/').collect();
    // parts[0] = "", parts[1] = "ip4", parts[2] = "{ip}", parts[3] = "tcp", parts[4] = ...
    if parts.get(1)? != &"ip4" { return None; }
    let ip = parts.get(2)?;
    // Skip loopback and private link-local (169.254.x.x)
    if *ip == "127.0.0.1" || ip.starts_with("169.254.") { return None; }
    Some(format!("http://{}:4242", ip))
}
