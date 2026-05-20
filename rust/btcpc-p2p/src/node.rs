use crate::config::Config;
use crate::ipc::{IpcCommand, IpcMessage, PushSender};
use anyhow::Result;
use futures::StreamExt;
use libp2p::{
    gossipsub, identify, kad,
    kad::store::MemoryStore,
    noise, ping,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, SwarmBuilder,
};
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

const BTCPC_TOPIC: &str = "btcpc";

// Bootstrap peers — always dialed on start for eclipse protection
const BOOTSTRAP_PEERS: &[&str] = &[
    "/dns4/node1.btcpc.network/tcp/6942",
    "/dns4/node2.btcpc.network/tcp/6942",
];

#[derive(NetworkBehaviour)]
struct BtcpcBehaviour {
    gossipsub: gossipsub::Behaviour,
    kademlia: kad::Behaviour<MemoryStore>,
    identify:  identify::Behaviour,
    ping:      ping::Behaviour,
}

pub struct Node {
    config:  Config,
    cmd_rx:  mpsc::Receiver<IpcCommand>,
    push_tx: PushSender,
}

impl Node {
    pub fn new(config: Config, cmd_rx: mpsc::Receiver<IpcCommand>, push_tx: PushSender) -> Self {
        Self { config, cmd_rx, push_tx }
    }

    pub async fn run(mut self) -> Result<()> {
        let keypair = crate::keys::load_or_create(&self.config.data_dir)?;

        let mut swarm = SwarmBuilder::with_existing_identity(keypair.clone())
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_quic()
            .with_behaviour(|key| {
                // Gossipsub: content-hash message dedup (same as Node.js seen-message logic)
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
                    // 10MB max — inference payloads can be large
                    .max_transmit_size(10 * 1024 * 1024)
                    .build()
                    .map_err(|e| anyhow::anyhow!(e))?;

                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )?;

                // Kademlia: DHT peer discovery — eliminates Cloudflare relay dependency
                let mut kademlia = kad::Behaviour::new(
                    key.public().to_peer_id(),
                    MemoryStore::new(key.public().to_peer_id()),
                );
                kademlia.set_mode(Some(kad::Mode::Server));

                // Identify: exchange protocol version + listen addresses with peers
                let identify = identify::Behaviour::new(identify::Config::new(
                    "/btcpc/1.0.0".to_string(),
                    key.public(),
                ));

                let ping = ping::Behaviour::new(
                    ping::Config::new()
                        .with_interval(Duration::from_secs(30))
                        .with_timeout(Duration::from_secs(30)),
                );

                Ok(BtcpcBehaviour { gossipsub, kademlia, identify, ping })
            })?
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(Duration::from_secs(60))
            })
            .build();

        // Subscribe to the BTCPC gossip topic
        let topic = gossipsub::IdentTopic::new(BTCPC_TOPIC);
        swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

        // TCP + QUIC listeners
        let listen_tcp: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", self.config.p2p_port).parse()?;
        swarm.listen_on(listen_tcp)?;
        let listen_quic: Multiaddr = format!("/ip4/0.0.0.0/udp/{}/quic-v1", self.config.p2p_port).parse()?;
        swarm.listen_on(listen_quic)?;

        // Dial bootstrap peers
        for addr_str in BOOTSTRAP_PEERS {
            if let Ok(addr) = addr_str.parse::<Multiaddr>() {
                info!("Dialing bootstrap: {}", addr);
                let _ = swarm.dial(addr);
            }
        }
        for peer_str in &self.config.seed_peers {
            if let Ok(addr) = peer_str.parse::<Multiaddr>() {
                let _ = swarm.dial(addr);
            }
        }

        // Start Kademlia bootstrap (will retry automatically)
        if let Err(e) = swarm.behaviour_mut().kademlia.bootstrap() {
            warn!("Kademlia bootstrap queued (expected on first start): {}", e);
        }

        info!("BTCPC P2P node running — peer ID: {}", swarm.local_peer_id());
        info!("Port: {} (TCP + QUIC)", self.config.p2p_port);

        loop {
            tokio::select! {
                event = swarm.select_next_some() => {
                    self.handle_swarm_event(event, &mut swarm).await;
                }
                Some(cmd) = self.cmd_rx.recv() => {
                    self.handle_command(cmd, &mut swarm, &topic).await;
                }
            }
        }
    }

    async fn handle_swarm_event(
        &self,
        event: SwarmEvent<BtcpcBehaviourEvent>,
        swarm: &mut libp2p::Swarm<BtcpcBehaviour>,
    ) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("Listening on {}", address);
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                info!("Connected: {}", peer_id);
                let _ = self.push_tx.send(IpcMessage {
                    method: "peer_connected".to_string(),
                    params: serde_json::json!({ "peer": peer_id.to_string() }),
                });
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                debug!("Disconnected: {} ({:?})", peer_id, cause);
                let _ = self.push_tx.send(IpcMessage {
                    method: "peer_disconnected".to_string(),
                    params: serde_json::json!({ "peer": peer_id.to_string() }),
                });
            }
            SwarmEvent::Behaviour(BtcpcBehaviourEvent::Gossipsub(
                gossipsub::Event::Message { propagation_source, message, .. }
            )) => {
                match serde_json::from_slice::<Value>(&message.data) {
                    Ok(data) => {
                        // Deliver to Node.js chain process
                        let _ = self.push_tx.send(IpcMessage {
                            method: "message".to_string(),
                            params: serde_json::json!({
                                "from": propagation_source.to_string(),
                                "data": data
                            }),
                        });
                    }
                    Err(e) => warn!("Non-JSON gossip from {}: {}", propagation_source, e),
                }
            }
            SwarmEvent::Behaviour(BtcpcBehaviourEvent::Identify(
                identify::Event::Received { peer_id, info, .. }
            )) => {
                // Feed discovered addresses into Kademlia routing table
                for addr in info.listen_addrs {
                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                }
            }
            SwarmEvent::Behaviour(BtcpcBehaviourEvent::Kademlia(
                kad::Event::OutboundQueryProgressed { result, .. }
            )) => {
                if let kad::QueryResult::Bootstrap(Ok(kad::BootstrapOk { num_remaining, .. })) = result {
                    if num_remaining == 0 {
                        info!("Kademlia DHT bootstrap complete");
                    }
                }
            }
            SwarmEvent::Behaviour(BtcpcBehaviourEvent::Ping(
                ping::Event { peer, result, .. }
            )) => {
                if let Err(e) = result {
                    debug!("Ping failed for {}: {}", peer, e);
                }
            }
            SwarmEvent::ListenerError { listener_id, error } => {
                error!("Listener {:?} error: {}", listener_id, error);
            }
            SwarmEvent::ListenerClosed { listener_id, addresses, reason } => {
                warn!("Listener {:?} closed (addresses: {:?}, reason: {:?})", listener_id, addresses, reason);
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                debug!("Outgoing connection error (peer {:?}): {}", peer_id, error);
            }
            _ => {}
        }
    }

    async fn handle_command(
        &self,
        cmd: IpcCommand,
        swarm: &mut libp2p::Swarm<BtcpcBehaviour>,
        topic: &gossipsub::IdentTopic,
    ) {
        match cmd {
            IpcCommand::Broadcast { data } => {
                let bytes = match serde_json::to_vec(&data) {
                    Ok(b) => b,
                    Err(e) => { warn!("Serialize error: {}", e); return; }
                };
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), bytes) {
                    warn!("Gossipsub publish error: {}", e);
                }
            }
            IpcCommand::Send { data, .. } => {
                // Gossipsub is broadcast-only; unicast handled by Node.js filtering on nodeId
                let bytes = match serde_json::to_vec(&data) {
                    Ok(b) => b,
                    Err(e) => { warn!("Serialize error: {}", e); return; }
                };
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), bytes) {
                    warn!("Gossipsub publish (directed) error: {}", e);
                }
            }
            IpcCommand::GetPeers { reply_tx } => {
                let peers: Vec<Value> = swarm
                    .connected_peers()
                    .map(|p| serde_json::json!({ "peer_id": p.to_string() }))
                    .collect();
                let _ = reply_tx.send(Value::Array(peers));
            }
        }
    }
}
