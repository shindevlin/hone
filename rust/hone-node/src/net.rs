//! hone-node P2P networking — libp2p 0.56
#![allow(dead_code)]
//!
//! Gossipsub topics:
//!   hone/blocks     — serialized block bytes (JSON envelope: {"epoch":N,"data_b64":"..."})
//!   hone/entries    — ledger entries (JSON)
//!   hone/seals      — epoch seals (JSON)
//!   hone/sync       — block sync requests/responses
//!   hone/consensus  — reward hash proposals (JSON: {epoch,rewards_hash,node_id})
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
    autonat, core::ConnectedPoint, dcutr, gossipsub, identify, kad, mdns, relay, upnp,
    kad::store::MemoryStore,
    multiaddr::Protocol,
    noise, ping,
    swarm::{behaviour::toggle::Toggle, dial_opts::{DialOpts, PeerCondition}, NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, SwarmBuilder,
};
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
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
    /// A clock liveness beacon (`hone/clock-hello`) — signed proof a peer is a
    /// registered clock. Updates only the in-memory registered set; never state.
    ClockHello { hello: serde_json::Value },
    /// A reward consensus proposal received from a peer clock node.
    ConsensusProposal { epoch: u64, rewards_hash: String, node_id: String },
    /// Signed batch of catch-up reward votes for past unfinalized epochs
    /// (`{"type":"catchup","node_id","votes":[[epoch,hash],..],"sig"}` on hone/consensus).
    CatchupVotes { node_id: String, votes: Vec<(u64, String)>, sig: String, echo: bool },
    /// State-healer traffic on hone/consensus: a signed checkpoint attestation
    /// (`type: checkpoint`), a snapshot request (`checkpoint_req`) or a snapshot reply
    /// (`checkpoint_snap`). See state_healer.rs.
    StateCheckpoint { msg: serde_json::Value },
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
struct HoneBehaviour {
    gossipsub: gossipsub::Behaviour,
    kademlia:  kad::Behaviour<MemoryStore>,
    identify:  identify::Behaviour,
    ping:      ping::Behaviour,
    mdns:      mdns::tokio::Behaviour,
    // NAT traversal (networking only; no consensus effect).
    relay_client: relay::client::Behaviour,
    /// Circuit relay v2 server, only with HONE_RELAY_SERVER=1 (public seeds).
    relay_server: Toggle<relay::Behaviour>,
    dcutr:     dcutr::Behaviour,
    autonat:   autonat::Behaviour,
    /// UPnP-IGD port mapping on the home router; HONE_UPNP=0 turns it off.
    upnp:      Toggle<upnp::tokio::Behaviour>,
}

fn env_on(k: &str) -> bool { matches!(std::env::var(k).as_deref(), Ok("1") | Ok("true")) }
fn env_off(k: &str) -> bool { matches!(std::env::var(k).as_deref(), Ok("0") | Ok("false")) }

/// NAT-traversal state kept by the swarm task.
#[derive(Default)]
struct NatState {
    /// AutoNAT says our address is publicly reachable.
    public: bool,
    /// Relays we hold (or are requesting) a reservation on.
    reserved: HashSet<PeerId>,
    /// The address we dialed each outbound peer on (used as its relay address).
    dialed: HashMap<PeerId, Multiaddr>,
    /// LAN peers seen by mDNS: peer -> one dialable address.
    lan: HashMap<PeerId, Multiaddr>,
    /// Relays we hold a reservation on: relay -> its circuit base address
    /// (`.../p2p/<relay>/p2p-circuit`). Peers reserved on the same relay are reachable as
    /// `<base>/p2p/<peer>`, which is how two NAT'd nodes behind one seed first meet.
    relays: HashMap<PeerId, Multiaddr>,
}

/// Loopback / unspecified addresses are never useful to another host.
fn is_local_only(a: &Multiaddr) -> bool {
    a.iter().any(|p| match p {
        Protocol::Ip4(ip) => ip.is_loopback() || ip.is_unspecified(),
        Protocol::Ip6(ip) => ip.is_loopback() || ip.is_unspecified(),
        _ => false,
    })
}

/// `addr` ends with /p2p/<id>; append it if missing.
fn with_peer(addr: &Multiaddr, id: PeerId) -> Multiaddr {
    if addr.iter().any(|p| matches!(p, Protocol::P2p(_))) { addr.clone() } else { addr.clone().with(Protocol::P2p(id)) }
}

// ---------------------------------------------------------------------------
// Internal broadcast routing keys — all callers use these exact strings, and
// the receive-side dispatch (below) matches against them, so they must stay in
// lockstep. The on-wire gossipsub topic is "{chain_id}/{key}", so a rename of
// chain_id (e.g. hone → hone) alone splits the mesh: mainnet, testnet, and
// any prior-named chain never share a topic even across the same physical peers.
const TOPIC_BLOCKS:     &str = "hone/blocks";
const TOPIC_ENTRIES:    &str = "hone/entries";
const TOPIC_SEALS:      &str = "hone/seals";
const TOPIC_SYNC:       &str = "hone/sync";
const TOPIC_CONSENSUS:  &str = "hone/consensus";
// Clock liveness beacon — a signed "I am a registered clock" announcement that
// updates only the IN-MEMORY registered set on receivers. It is NOT a ledger entry
// and never touches chain state, so re-announcing every epoch (for late-joining
// peers) cannot fork state_root. On-chain registration stays a write-once
// ClockNodeRegister. See DRYRUN_2CLOCK / BUG 6.
const TOPIC_CLOCK_HELLO: &str = "hone/clock-hello";

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
            // QUIC: UDP transport, encrypted natively, punches NAT better than TCP.
            // with_dns() bridges OtherTransportPhase → WebsocketPhase so both coexist.
            .with_quic()
            .with_dns()?
            .with_websocket(
                noise::Config::new,
                yamux::Config::default,
            )
            .await?
            .with_relay_client(noise::Config::new, yamux::Config::default)?
            .with_behaviour(|key, relay_client| {
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

                let mut roles = Vec::new();
                if self.config.is_clock { roles.push("clock".to_string()); }
                if self.config.is_miner { roles.push("miner".to_string()); }
                if matches!(std::env::var("DECIDE_JURY_SHADOW").as_deref(), Ok("1") | Ok("true")) {
                    roles.push("decide".to_string());
                }
                let agent = crate::peer_ident::agent_version(
                    &key.public().to_peer_id().to_string(), &self.config.account, roles);
                let identify = identify::Behaviour::new(identify::Config::new(
                    "/hone/1.0.0".to_string(),
                    key.public(),
                ).with_agent_version(agent).with_push_listen_addr_updates(true));

                let ping = ping::Behaviour::new(
                    ping::Config::new()
                        .with_interval(Duration::from_secs(30))
                        .with_timeout(Duration::from_secs(30)),
                );

                let mdns = mdns::tokio::Behaviour::new(
                    mdns::Config::default(),
                    key.public().to_peer_id(),
                ).map_err(|e| anyhow::anyhow!("mDNS init failed: {}", e))?;

                let pid = key.public().to_peer_id();
                // Relay server: small limits for a small VPS. Circuits coordinate a direct
                // (DCUtR) upgrade; gossip between NAT'd nodes otherwise flows through the
                // seed's own gossipsub.
                let relay_server = env_on("HONE_RELAY_SERVER").then(|| relay::Behaviour::new(pid, relay::Config {
                    max_reservations: 64,
                    max_reservations_per_peer: 2,
                    reservation_duration: Duration::from_secs(3600),
                    max_circuits: 16,
                    max_circuits_per_peer: 2,
                    max_circuit_duration: Duration::from_secs(300),
                    max_circuit_bytes: 4 << 20,
                    ..Default::default()
                }));
                let autonat = autonat::Behaviour::new(pid, autonat::Config {
                    // Lab networks use private addresses; production keeps global-only.
                    only_global_ips: !env_on("HONE_AUTONAT_ALLOW_PRIVATE"),
                    ..Default::default()
                });
                let upnp = (!env_off("HONE_UPNP") && !self.config.isolated).then(upnp::tokio::Behaviour::default);
                Ok(HoneBehaviour {
                    gossipsub, kademlia, identify, ping, mdns,
                    relay_client,
                    relay_server: Toggle::from(relay_server),
                    dcutr: dcutr::Behaviour::new(pid),
                    autonat,
                    upnp: Toggle::from(upnp),
                })
            })?
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(Duration::from_secs(60))
            })
            .build();

        // ------------------------------------------------------------------
        // 3. Subscribe to topics — on-wire name = "{chain_id}/{key}"
        //    e.g. "hone/hone/entries" vs "hone-testnet/hone/entries"
        //    so mainnet and testnet never share a gossip mesh.
        // ------------------------------------------------------------------
        let cid = &self.config.chain_id;
        let t_blocks    = gossipsub::IdentTopic::new(format!("{}/{}", cid, TOPIC_BLOCKS));
        let t_entries   = gossipsub::IdentTopic::new(format!("{}/{}", cid, TOPIC_ENTRIES));
        let t_seals     = gossipsub::IdentTopic::new(format!("{}/{}", cid, TOPIC_SEALS));
        let t_sync      = gossipsub::IdentTopic::new(format!("{}/{}", cid, TOPIC_SYNC));
        let t_consensus = gossipsub::IdentTopic::new(format!("{}/{}", cid, TOPIC_CONSENSUS));
        let t_clock_hello = gossipsub::IdentTopic::new(format!("{}/{}", cid, TOPIC_CLOCK_HELLO));

        swarm.behaviour_mut().gossipsub.subscribe(&t_blocks)?;
        swarm.behaviour_mut().gossipsub.subscribe(&t_entries)?;
        swarm.behaviour_mut().gossipsub.subscribe(&t_seals)?;
        swarm.behaviour_mut().gossipsub.subscribe(&t_sync)?;
        swarm.behaviour_mut().gossipsub.subscribe(&t_consensus)?;
        swarm.behaviour_mut().gossipsub.subscribe(&t_clock_hello)?;

        // ------------------------------------------------------------------
        // 4. Listeners
        // ------------------------------------------------------------------
        // Isolated mode binds LOOPBACK only, so other nodes on the host/LAN cannot
        // dial in — an isolated N-clock test must not be reachable by (or reach) any
        // node outside its explicit HONE_BOOTSTRAP_PEERS set. Production binds all
        // interfaces so peers can connect from anywhere.
        let bind_ip = if self.config.isolated { "127.0.0.1" } else { "0.0.0.0" };

        let listen_tcp: Multiaddr =
            format!("/ip4/{}/tcp/{}", bind_ip, self.config.p2p_port).parse()?;
        swarm.listen_on(listen_tcp)?;

        // WebSocket transport — allows nodes behind NAT to connect via the
        // cloudflared tunnel at wss://p2p.honemesh.net (port 443 → local 4943).
        let ws_port = std::env::var("HONE_WS_PORT")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(4943u16);
        let listen_ws: Multiaddr = format!("/ip4/{}/tcp/{}/ws", bind_ip, ws_port).parse()?;
        swarm.listen_on(listen_ws)?;

        // QUIC transport — UDP, encrypted natively, mobile-friendly, punches NAT better than TCP.
        // Shares the same port number as TCP but on UDP — no conflict, different stacks.
        // Override with HONE_QUIC_PORT if the UDP port needs to differ from p2p_port.
        let quic_port: u16 = std::env::var("HONE_QUIC_PORT")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(self.config.p2p_port);
        let listen_quic: Multiaddr = format!("/ip4/{}/udp/{}/quic-v1", bind_ip, quic_port).parse()?;
        if let Err(e) = swarm.listen_on(listen_quic) {
            warn!("QUIC listener failed (UDP/{} unavailable): {}", quic_port, e);
        }

        // HONE_EXTERNAL_ADDRS: addresses this node is known to be reachable on (a public
        // seed sets its own IP / DNS name); advertised via identify and in relay reservations.
        for a in std::env::var("HONE_EXTERNAL_ADDRS").unwrap_or_default().split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match a.parse::<Multiaddr>() {
                Ok(m) => { info!("external address {}", m); swarm.add_external_address(m); }
                Err(e) => warn!("HONE_EXTERNAL_ADDRS '{}': {}", a, e),
            }
        }

        // ------------------------------------------------------------------
        // 5. Load stored peers + run discovery, then dial all
        // ------------------------------------------------------------------

        // In-memory peer cache: peer_id_str → listen addrs.
        // Populated by Identify events; flushed to RocksDB every 5 minutes.
        let mut peer_cache: HashMap<String, Vec<Multiaddr>> = HashMap::new();
        let mut nat = NatState::default();

        // Let the listeners bind before the first dials: an outbound TCP dial then reuses
        // the listen port, so the NAT mapping a peer observes (and DCUtR punches to) is the
        // real one rather than an ephemeral port. Bounded at 2 s.
        {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            let mut bound = 0usize;
            while bound < 2 {
                tokio::select! {
                    ev = swarm.select_next_some() => {
                        if matches!(ev, SwarmEvent::NewListenAddr { .. }) { bound += 1; }
                        self.handle_swarm_event(ev, &mut swarm, &mut peer_cache, &mut nat);
                    }
                    _ = tokio::time::sleep_until(deadline) => break,
                }
            }
        }

        // Layer 1: peers from RocksDB peer store (instant, no network).
        let stored_peers = load_peer_store(&self.store);
        if !stored_peers.is_empty() {
            info!("peer store: {} cached peers", stored_peers.len());
        }
        for addr in &stored_peers {
            dial_once(&mut swarm, addr.clone());
        }

        // Layer 2: Cloudflare DNS seeds (bootstrap_peers from config — already dialed below
        //           after discovery so they're all handled in one unified pass).

        // Layer 3: Hive + TON registry (10s timeout — best effort).
        // Isolated mode makes NO outbound discovery calls, so the node cannot pull
        // in live-network peers and stays confined to HONE_BOOTSTRAP_PEERS.
        let discovered: Vec<String> = if self.config.isolated {
            info!("[isolated] skipping public peer discovery (Hive/honemesh.net)");
            Vec::new()
        } else {
            let chain_id = self.config.chain_id.clone();
            tokio::time::timeout(
                Duration::from_secs(10),
                discovery::fetch_all_peers(&chain_id),
            )
            .await
            .unwrap_or_default()
        };

        // Dial config bootstrap peers + discovery peers together.
        let all_bootstrap: Vec<String> = self.config.bootstrap_peers.iter()
            .cloned()
            .chain(discovered.into_iter())
            .collect();

        for addr_str in &all_bootstrap {
            match addr_str.parse::<Multiaddr>() {
                Ok(addr) => {
                    info!("Dialing bootstrap peer: {}", addr);
                    // Known to Kademlia too, so peer fill retries the seed's other
                    // addresses (e.g. TCP when UDP/QUIC is blocked) if this dial fails.
                    if let Some(Protocol::P2p(id)) = addr.iter().last() {
                        swarm.behaviour_mut().kademlia.add_address(&id, addr.clone());
                    }
                    dial_once(&mut swarm, addr);
                }
                Err(e) => warn!("Invalid bootstrap addr '{}': {}", addr_str, e),
            }
        }

        // Announce own WS multiaddr to honemesh.net peer registry.
        // HONE_ANNOUNCE_ADDR overrides; otherwise builds from peer_id + p2p.honemesh.net.
        // Skipped in isolated mode — no outbound registry calls.
        if !self.config.isolated {
            let peer_id = swarm.local_peer_id().to_string();
            let chain_id = self.config.chain_id.clone();
            let node_id  = self.config.node_id.clone();
            let announce_addr = std::env::var("HONE_ANNOUNCE_ADDR").unwrap_or_else(|_| {
                format!("/dns4/p2p.honemesh.net/tcp/443/wss/p2p/{}", peer_id)
            });
            // Fire-and-forget; never blocks startup.
            tokio::spawn(async move {
                // Small delay so the swarm is fully initialised.
                tokio::time::sleep(Duration::from_secs(5)).await;
                discovery::announce_to_hone_net_addr(&announce_addr, &chain_id, &node_id).await;
            });
        }

        // Kick off Kademlia bootstrap (harmless if no peers yet).
        if let Err(e) = swarm.behaviour_mut().kademlia.bootstrap() {
            warn!("Kademlia bootstrap queued (expected on first start): {}", e);
        }

        info!(
            peer_id = %swarm.local_peer_id(),
            port    = self.config.p2p_port,
            "hone-node P2P running",
        );

        // ------------------------------------------------------------------
        // 6. Timers
        // ------------------------------------------------------------------
        let mut rebootstrap   = tokio::time::interval(Duration::from_secs(300));
        let mut peer_flush    = tokio::time::interval(Duration::from_secs(300));
        rebootstrap.tick().await; // discard immediate first tick
        peer_flush.tick().await;
        // Gossip watchdog: a peer we are connected to but whose topic subscriptions we
        // never learned (its hello went out on a duplicate connection that then closed)
        // is invisible to gossipsub forever, so publishes fail NoPeersSubscribedToTopic
        // and the mesh partitions. Drop such a peer so the redial exchanges hellos again.
        let mut gossip_watch = tokio::time::interval(Duration::from_secs(20));
        gossip_watch.tick().await;
        let mut gossip_strikes: HashMap<PeerId, u8> = HashMap::new();
        let mut gossip_redial: HashSet<PeerId> = HashSet::new();
        // Peer fill: dial peers Kademlia knows (direct or relay-circuit addresses) so NAT'd
        // nodes meet and DCUtR can upgrade; also redial mDNS LAN peers not connected.
        let mut peer_fill = tokio::time::interval(Duration::from_secs(30));
        peer_fill.tick().await;


        // ------------------------------------------------------------------
        // 7. Event loop
        // ------------------------------------------------------------------
        loop {
            tokio::select! {
                event = swarm.select_next_some() => {
                    self.handle_swarm_event(event, &mut swarm, &mut peer_cache, &mut nat);
                }
                Some(cmd) = self.cmd_rx.recv() => {
                    self.handle_cmd(cmd, &mut swarm,
                        &t_blocks, &t_entries, &t_seals, &t_sync, &t_consensus, &t_clock_hello);
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
                _ = peer_fill.tick() => {
                    peer_fill_tick(&mut swarm, &nat);
                }
                _ = gossip_watch.tick() => {
                    gossip_watchdog(&mut swarm, &peer_cache, &mut gossip_strikes, &mut gossip_redial);
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
        event: SwarmEvent<HoneBehaviourEvent>,
        swarm: &mut libp2p::Swarm<HoneBehaviour>,
        peer_cache: &mut HashMap<String, Vec<Multiaddr>>,
        nat: &mut NatState,
    ) {
        match event {
            // Listeners
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("Listening on {}", address);
            }

            // Peer lifecycle
            SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                let relayed = endpoint.get_remote_address().iter().any(|p| matches!(p, Protocol::P2pCircuit));
                info!("Peer connected: {}{}", peer_id, if relayed { " (relayed)" } else { "" });
                if let ConnectedPoint::Dialer { address, .. } = &endpoint {
                    if !relayed { nat.dialed.insert(peer_id, address.clone()); }
                }
                crate::peer_ident::connected(&peer_id.to_string(), &endpoint.get_remote_address().to_string());
                self.emit(NetworkEvent::PeerConnected { peer_id: peer_id.to_string() });
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, num_established, .. } => {
                debug!("Peer disconnected: {} ({:?})", peer_id, cause);
                if num_established == 0 { crate::peer_ident::disconnected(&peer_id.to_string()); }
                self.emit(NetworkEvent::PeerDisconnected { peer_id: peer_id.to_string() });
            }

            // Gossipsub message
            SwarmEvent::Behaviour(HoneBehaviourEvent::Gossipsub(
                gossipsub::Event::Message { message, propagation_source, .. }
            )) => {
                self.handle_gossip_message(&message, propagation_source.to_string());
            }

            // Identify — feed addresses into Kademlia, persist, and derive HTTP URL.
            SwarmEvent::Behaviour(HoneBehaviourEvent::Identify(
                identify::Event::Received { peer_id, info, .. }
            )) => {
                let pid = peer_id.to_string();
                let v = crate::peer_ident::verify(&pid, &info.agent_version,
                    |a| crate::peer_ident::chain_keys(&self.store, a));
                if let Some(line) = crate::peer_ident::identified(&pid, &info.agent_version, v) {
                    info!("{}", line);
                }
                let cache_entry = peer_cache
                    .entry(peer_id.to_string())
                    .or_default();
                for addr in info.listen_addrs.iter().filter(|a| !is_local_only(a)) {
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
                // A peer offering circuit relay v2: unless we are publicly reachable (or a
                // relay ourselves), reserve a slot on it so NAT'd peers can reach us and
                // DCUtR can punch a direct connection. At most 2 relays.
                let is_relay = info.protocols.iter().any(|p| *p == relay::HOP_PROTOCOL_NAME);
                if is_relay && !nat.public && !env_on("HONE_RELAY_SERVER")
                    && !nat.reserved.contains(&peer_id) && nat.reserved.len() < 2
                {
                    if let Some(a) = nat.dialed.get(&peer_id) {
                        let circuit = with_peer(a, peer_id).with(Protocol::P2pCircuit);
                        match swarm.listen_on(circuit.clone()) {
                            Ok(_) => {
                                info!("relay: requesting reservation on {}", circuit);
                                nat.reserved.insert(peer_id);
                                nat.relays.insert(peer_id, circuit);
                            }
                            Err(e) => warn!("relay: listen on {} failed: {}", circuit, e),
                        }
                    }
                }
            }

            SwarmEvent::Behaviour(HoneBehaviourEvent::RelayClient(e)) => match e {
                relay::client::Event::ReservationReqAccepted { relay_peer_id, renewal, .. } => {
                    if !renewal { info!("relay: reservation accepted by {}", relay_peer_id); }
                }
                other => debug!("relay client: {:?}", other),
            },
            SwarmEvent::Behaviour(HoneBehaviourEvent::RelayServer(e)) => match e {
                relay::Event::ReservationReqAccepted { src_peer_id, renewed, .. } if !renewed =>
                    info!("relay server: reservation for {}", src_peer_id),
                relay::Event::CircuitReqAccepted { src_peer_id, dst_peer_id } =>
                    info!("relay server: circuit {} -> {}", src_peer_id, dst_peer_id),
                other => debug!("relay server: {:?}", other),
            },
            SwarmEvent::Behaviour(HoneBehaviourEvent::Dcutr(e)) => match &e.result {
                Ok(_) => info!("dcutr: direct connection to {} (hole punch ok)", e.remote_peer_id),
                Err(err) => info!("dcutr: hole punch to {} failed, staying relayed: {}", e.remote_peer_id, err),
            },
            SwarmEvent::Behaviour(HoneBehaviourEvent::Autonat(autonat::Event::StatusChanged { old, new })) => {
                info!("autonat: {:?} -> {:?}", old, new);
                nat.public = matches!(new, autonat::NatStatus::Public(_));
            }
            SwarmEvent::Behaviour(HoneBehaviourEvent::Upnp(e)) => match e {
                e @ upnp::Event::NewExternalAddr { .. } => info!("upnp: {:?}", e),
                other => debug!("upnp: {:?}", other),
            },
            SwarmEvent::ExternalAddrConfirmed { address } => {
                info!("external address confirmed: {}", address);
            }
            SwarmEvent::ListenerClosed { addresses, .. }
                if addresses.iter().any(|a| a.iter().any(|p| matches!(p, Protocol::P2pCircuit))) =>
            {
                // Reservation lost (relay restarted / went away): allow a new one.
                for a in &addresses {
                    let relay_id = a.iter().take_while(|p| !matches!(p, Protocol::P2pCircuit))
                        .filter_map(|p| if let Protocol::P2p(id) = p { Some(id) } else { None }).last();
                    if let Some(id) = relay_id {
                        nat.reserved.remove(&id);
                        nat.relays.remove(&id);
                        info!("relay: reservation on {} closed", id);
                    }
                }
            }

            // Kademlia bootstrap progress
            SwarmEvent::Behaviour(HoneBehaviourEvent::Kademlia(
                kad::Event::OutboundQueryProgressed { result, .. }
            )) => {
                if let kad::QueryResult::Bootstrap(Ok(kad::BootstrapOk { num_remaining, .. })) = result {
                    if num_remaining == 0 {
                        info!("Kademlia DHT bootstrap complete");
                    }
                }
            }

            // Ping
            SwarmEvent::Behaviour(HoneBehaviourEvent::Ping(
                ping::Event { peer, result, .. }
            )) => {
                if let Err(e) = result {
                    debug!("Ping failed for {}: {}", peer, e);
                }
            }

            // mDNS — local network peer discovery
            SwarmEvent::Behaviour(HoneBehaviourEvent::Mdns(
                mdns::Event::Discovered(peers)
            )) => {
                // Isolated mode ignores mDNS-discovered peers so the node cannot pick
                // up other hone nodes on the LAN (e.g. a dev node on the same host) —
                // it dials only the explicitly-listed HONE_BOOTSTRAP_PEERS.
                if self.config.isolated {
                    debug!("[isolated] ignoring {} mDNS-discovered peer(s)", peers.len());
                } else {
                    let mut best: HashMap<PeerId, Multiaddr> = HashMap::new();
                    for (peer_id, addr) in peers {
                        info!("mDNS: discovered {} at {}", peer_id, addr);
                        swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                        let plain_tcp = addr.iter().any(|p| matches!(p, Protocol::Tcp(_)))
                            && !addr.iter().any(|p| matches!(p, Protocol::Ws(_) | Protocol::Wss(_)));
                        if plain_tcp || !best.contains_key(&peer_id) { best.insert(peer_id, addr); }
                    }
                    // One dial per peer on its LAN TCP address, with `Disconnected` rather
                    // than `DisconnectedAndNotDialing`: a hung dial to another address of the
                    // same peer (stale tailscale / WAN) must not block the LAN dial. That left
                    // grouchly and beastly unlinked on 2026-11-04.
                    for (peer_id, addr) in best {
                        nat.lan.insert(peer_id, addr.clone());
                        if swarm.is_connected(&peer_id) { continue; }
                        info!("mDNS: dialing {} at {}", peer_id, addr);
                        let _ = swarm.dial(
                            DialOpts::peer_id(peer_id)
                                .addresses(vec![addr])
                                .condition(PeerCondition::Disconnected)
                                .build(),
                        );
                    }
                }
            }
            SwarmEvent::Behaviour(HoneBehaviourEvent::Mdns(
                mdns::Event::Expired(peers)
            )) => {
                for (peer_id, _addr) in peers {
                    nat.lan.remove(&peer_id);
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
        // Resolve topic string from hash — match against this node's chain-scoped topics.
        // A message on a different chain_id's topics will simply fall through to the
        // unknown-topic branch and be silently dropped.
        let cid = &self.config.chain_id;
        let topic_str = {
            let h = &message.topic;
            if      h == &gossipsub::IdentTopic::new(format!("{}/{}", cid, TOPIC_BLOCKS)).hash()     { TOPIC_BLOCKS     }
            else if h == &gossipsub::IdentTopic::new(format!("{}/{}", cid, TOPIC_ENTRIES)).hash()    { TOPIC_ENTRIES    }
            else if h == &gossipsub::IdentTopic::new(format!("{}/{}", cid, TOPIC_SEALS)).hash()      { TOPIC_SEALS      }
            else if h == &gossipsub::IdentTopic::new(format!("{}/{}", cid, TOPIC_SYNC)).hash()       { TOPIC_SYNC       }
            else if h == &gossipsub::IdentTopic::new(format!("{}/{}", cid, TOPIC_CONSENSUS)).hash()  { TOPIC_CONSENSUS  }
            else if h == &gossipsub::IdentTopic::new(format!("{}/{}", cid, TOPIC_CLOCK_HELLO)).hash() { TOPIC_CLOCK_HELLO }
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
                    None    => { warn!("hone/blocks: missing 'epoch' field"); return; }
                };
                let data_b64 = match json.get("data_b64").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None    => { warn!("hone/blocks: missing 'data_b64' field"); return; }
                };
                let data = match B64.decode(data_b64) {
                    Ok(b)  => b,
                    Err(e) => { warn!("hone/blocks: base64 decode error: {}", e); return; }
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

            TOPIC_CLOCK_HELLO => {
                debug!("Received clock-hello from {}", source);
                self.emit(NetworkEvent::ClockHello { hello: json });
            }

            TOPIC_CONSENSUS if matches!(json.get("type").and_then(|v| v.as_str()),
                Some("checkpoint") | Some("checkpoint_req") | Some("checkpoint_snap")) => {
                self.emit(NetworkEvent::StateCheckpoint { msg: json });
            }

            TOPIC_CONSENSUS if json.get("type").and_then(|v| v.as_str()) == Some("catchup") => {
                let node_id = json.get("node_id").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let sig = json.get("sig").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let votes: Vec<(u64, String)> = json.get("votes")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                if node_id.is_empty() || sig.is_empty() || votes.is_empty() {
                    warn!("hone/consensus: malformed catchup from {}", source);
                    return;
                }
                debug!("Received {} catch-up votes from {} ({})", votes.len(), node_id, source);
                let echo = json.get("echo").and_then(|v| v.as_bool()).unwrap_or(false);
                self.emit(NetworkEvent::CatchupVotes { node_id, votes, sig, echo });
            }

            TOPIC_CONSENSUS => {
                let epoch = match json.get("epoch").and_then(|v| v.as_u64()) {
                    Some(e) => e,
                    None => { warn!("hone/consensus: missing 'epoch'"); return; }
                };
                let rewards_hash = match json.get("rewards_hash").and_then(|v| v.as_str()) {
                    Some(h) => h.to_owned(),
                    None => { warn!("hone/consensus: missing 'rewards_hash'"); return; }
                };
                let node_id = match json.get("node_id").and_then(|v| v.as_str()) {
                    Some(n) => n.to_owned(),
                    None => { warn!("hone/consensus: missing 'node_id'"); return; }
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
                            None    => { warn!("hone/sync response: missing 'epoch'"); return; }
                        };
                        let data_b64 = match json.get("data_b64").and_then(|v| v.as_str()) {
                            Some(s) => s,
                            None    => { warn!("hone/sync response: missing 'data_b64'"); return; }
                        };
                        let data = match B64.decode(data_b64) {
                            Ok(b)  => b,
                            Err(e) => { warn!("hone/sync: base64 decode error: {}", e); return; }
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
                        warn!("hone/sync: unknown type {:?}", other);
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
        swarm: &mut libp2p::Swarm<HoneBehaviour>,
        t_blocks:    &gossipsub::IdentTopic,
        t_entries:   &gossipsub::IdentTopic,
        t_seals:     &gossipsub::IdentTopic,
        t_sync:      &gossipsub::IdentTopic,
        t_consensus: &gossipsub::IdentTopic,
        t_clock_hello: &gossipsub::IdentTopic,
    ) {
        match cmd {
            NetCmd::Broadcast { topic, data } => {
                let ident_topic = match topic {
                    TOPIC_BLOCKS     => t_blocks.clone(),
                    TOPIC_ENTRIES    => t_entries.clone(),
                    TOPIC_SEALS      => t_seals.clone(),
                    TOPIC_SYNC       => t_sync.clone(),
                    TOPIC_CONSENSUS  => t_consensus.clone(),
                    TOPIC_CLOCK_HELLO => t_clock_hello.clone(),
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
                // Publish a sync request; any peer with that block will respond via hone/sync
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

/// Dial `addr` unless we are already connected to (or dialing) the peer it names.
/// Stored peers carry one address per interface; dialing them all at once opened
/// several parallel connections per peer, which is what loses gossipsub hellos.
fn dial_once(swarm: &mut libp2p::Swarm<HoneBehaviour>, addr: Multiaddr) {
    let pid = addr.iter().find_map(|p| match p {
        libp2p::multiaddr::Protocol::P2p(id) => Some(id),
        _ => None,
    });
    let _ = match pid {
        Some(id) => swarm.dial(
            DialOpts::peer_id(id)
                .addresses(vec![addr])
                .condition(PeerCondition::DisconnectedAndNotDialing)
                .build(),
        ),
        None => swarm.dial(addr),
    };
}

/// Every 30 s: redial mDNS LAN peers that are not connected, and dial up to 4
/// Kademlia-known peers we are not connected to (their addresses may be relay circuit
/// addresses), up to 24 connections. Two NAT'd nodes that share only a seed meet this
/// way, and DCUtR then upgrades the relayed link to a direct one when it can.
fn peer_fill_tick(swarm: &mut libp2p::Swarm<HoneBehaviour>, nat: &NatState) {
    for (pid, addr) in &nat.lan {
        if !swarm.is_connected(pid) {
            debug!("peer fill: LAN redial {}", pid);
            let _ = swarm.dial(DialOpts::peer_id(*pid).addresses(vec![addr.clone()])
                .condition(PeerCondition::Disconnected).build());
        }
    }
    if swarm.connected_peers().count() >= 24 { return; }
    let me = *swarm.local_peer_id();
    let mut todo: Vec<(PeerId, Vec<Multiaddr>)> = Vec::new();
    for bucket in swarm.behaviour_mut().kademlia.kbuckets() {
        for e in bucket.iter() {
            let pid = *e.node.key.preimage();
            if pid != me { todo.push((pid, e.node.value.iter().cloned().collect())); }
        }
    }
    let mut n = 0;
    for (pid, addrs) in todo {
        if n >= 4 { break; }
        if swarm.is_connected(&pid) || nat.relays.contains_key(&pid) { continue; }
        let mut addrs: Vec<Multiaddr> = addrs.into_iter().filter(|a| !is_local_only(a)).collect();
        // Also try it through every relay we are reserved on (it likely shares the seed).
        for base in nat.relays.values() {
            let c = base.clone().with(Protocol::P2p(pid));
            if !addrs.contains(&c) { addrs.push(c); }
        }
        if addrs.is_empty() { continue; }
        info!("peer fill: dialing {} ({} address(es))", pid, addrs.len());
        let _ = swarm.dial(DialOpts::peer_id(pid).addresses(addrs)
            .condition(PeerCondition::DisconnectedAndNotDialing).build());
        n += 1;
    }
}

/// Peers gossipsub knows no topics for after two checks (20-40 s) are disconnected
/// and redialed on the next check from their identified addresses.
fn gossip_watchdog(
    swarm: &mut libp2p::Swarm<HoneBehaviour>,
    peer_cache: &HashMap<String, Vec<Multiaddr>>,
    strikes: &mut HashMap<PeerId, u8>,
    redial: &mut HashSet<PeerId>,
) {
    for pid in std::mem::take(redial) {
        if swarm.is_connected(&pid) { continue; }
        if let Some(addrs) = peer_cache.get(&pid.to_string()) {
            info!("gossip watchdog: redialing {}", pid);
            let _ = swarm.dial(
                DialOpts::peer_id(pid)
                    .addresses(addrs.clone())
                    .condition(PeerCondition::DisconnectedAndNotDialing)
                    .build(),
            );
        }
    }
    let subscribed: HashSet<PeerId> = swarm.behaviour().gossipsub.all_peers()
        .filter(|(_, t)| !t.is_empty())
        .map(|(p, _)| *p)
        .collect();
    let connected: Vec<PeerId> = swarm.connected_peers().cloned().collect();
    strikes.retain(|p, _| connected.contains(p));
    for pid in connected {
        // Only hone peers (identified with listen addrs) are expected to gossip.
        if subscribed.contains(&pid) || !peer_cache.contains_key(&pid.to_string()) {
            strikes.remove(&pid);
            continue;
        }
        let n = strikes.entry(pid).or_insert(0);
        *n += 1;
        if *n >= 2 {
            warn!("gossip watchdog: connected to {} but no topic subscriptions known; reconnecting", pid);
            strikes.remove(&pid);
            let _ = swarm.disconnect_peer_id(pid);
            redial.insert(pid);
        }
    }
}

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
