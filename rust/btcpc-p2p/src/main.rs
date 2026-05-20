mod config;
mod ipc;
mod keys;
mod node;

use anyhow::Result;
use config::Config;
use ipc::{IpcCommand, IpcServer};
use node::Node;
use tokio::sync::{broadcast, mpsc};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("btcpc_p2p=info".parse()?)
                .add_directive("libp2p=warn".parse()?)
        )
        .init();

    let config = Config::from_env();

    info!("BTCPC P2P Sidecar v{}", env!("CARGO_PKG_VERSION"));
    info!("IPC socket : {}", config.ipc_socket);
    info!("P2P port   : {}", config.p2p_port);
    info!("Data dir   : {}", config.data_dir);

    // cmd channel: IPC server → Node (commands from Node.js)
    let (cmd_tx, cmd_rx) = mpsc::channel::<IpcCommand>(256);

    // push broadcast: Node → all connected IPC clients (events to Node.js)
    // capacity 1024: drops oldest if Node.js is slow (lagged warning logged)
    let (push_tx, _) = broadcast::channel(1024);

    let ipc_server = IpcServer::new(config.ipc_socket.clone(), cmd_tx, push_tx.clone());
    let p2p_node   = Node::new(config, cmd_rx, push_tx);

    let (r1, r2) = tokio::try_join!(
        tokio::spawn(async move { ipc_server.run().await }),
        tokio::spawn(async move { p2p_node.run().await }),
    )?;
    r1?;
    r2?;

    Ok(())
}
