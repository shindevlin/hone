/// Unix domain socket IPC server.
///
/// Node.js ↔ Rust protocol (newline-delimited JSON):
///
/// Node.js → Rust (commands):
///   { "method": "broadcast", "params": { "data": <json_msg> } }
///   { "method": "send",      "params": { "peer": "<peer_id>", "data": <json_msg> } }
///   { "method": "peers",     "params": {} }
///
/// Rust → Node.js (pushed events):
///   { "method": "message",          "params": { "from": "<peer_id>", "data": <json_msg> } }
///   { "method": "peer_connected",   "params": { "peer": "<peer_id>" } }
///   { "method": "peer_disconnected","params": { "peer": "<peer_id>" } }

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcMessage {
    pub method: String,
    pub params: Value,
}

pub enum IpcCommand {
    Broadcast { data: Value },
    Send { #[allow(dead_code)] peer: String, data: Value },
    GetPeers { reply_tx: tokio::sync::oneshot::Sender<Value> },
}

/// Shared push channel — broadcast from node to all connected IPC clients.
pub type PushSender = broadcast::Sender<IpcMessage>;

pub struct IpcServer {
    socket_path: String,
    cmd_tx: mpsc::Sender<IpcCommand>,
    push_tx: PushSender,
}

impl IpcServer {
    pub fn new(
        socket_path: String,
        cmd_tx: mpsc::Sender<IpcCommand>,
        push_tx: PushSender,
    ) -> Self {
        Self { socket_path, cmd_tx, push_tx }
    }

    pub async fn run(self) -> Result<()> {
        let path = Path::new(&self.socket_path);
        if path.exists() {
            std::fs::remove_file(path)?;
        }

        let listener = UnixListener::bind(path)?;
        info!("IPC server listening on {}", self.socket_path);

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let cmd_tx = self.cmd_tx.clone();
                    let push_rx = self.push_tx.subscribe();
                    tokio::spawn(handle_connection(stream, cmd_tx, push_rx));
                }
                Err(e) => warn!("IPC accept error: {}", e),
            }
        }
    }
}

async fn handle_connection(
    stream: UnixStream,
    cmd_tx: mpsc::Sender<IpcCommand>,
    mut push_rx: broadcast::Receiver<IpcMessage>,
) {
    let (read_half, write_half) = stream.into_split();
    let write_half = Arc::new(Mutex::new(write_half));
    let write_for_push = write_half.clone();

    // Task: push events from node → this client
    let push_task = tokio::spawn(async move {
        loop {
            match push_rx.recv().await {
                Ok(msg) => {
                    let line = match serde_json::to_string(&msg) {
                        Ok(s) => format!("{}\n", s),
                        Err(e) => { warn!("Push serialize error: {}", e); continue; }
                    };
                    let mut w = write_for_push.lock().await;
                    if w.write_all(line.as_bytes()).await.is_err() {
                        break; // client disconnected
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("IPC push lagged {} messages", n);
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Task: read commands from this client → node
    let mut reader = BufReader::new(read_half).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        let msg: IpcMessage = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => { warn!("IPC parse error: {}", e); continue; }
        };

        let response = dispatch_command(msg, &cmd_tx).await;
        let resp_line = format!("{}\n", serde_json::to_string(&response).unwrap_or_default());
        let mut w = write_half.lock().await;
        if w.write_all(resp_line.as_bytes()).await.is_err() {
            break;
        }
    }

    push_task.abort();
    debug!("IPC client disconnected");
}

async fn dispatch_command(msg: IpcMessage, cmd_tx: &mpsc::Sender<IpcCommand>) -> Value {
    match msg.method.as_str() {
        "broadcast" => {
            let data = msg.params["data"].clone();
            let _ = cmd_tx.send(IpcCommand::Broadcast { data }).await;
            serde_json::json!({ "ok": true })
        }
        "send" => {
            let peer = msg.params["peer"].as_str().unwrap_or("").to_string();
            let data = msg.params["data"].clone();
            let _ = cmd_tx.send(IpcCommand::Send { peer, data }).await;
            serde_json::json!({ "ok": true })
        }
        "peers" => {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            let _ = cmd_tx.send(IpcCommand::GetPeers { reply_tx }).await;
            let peers = reply_rx.await.unwrap_or(Value::Array(vec![]));
            serde_json::json!({ "ok": true, "peers": peers })
        }
        unknown => {
            warn!("Unknown IPC method: {}", unknown);
            serde_json::json!({ "ok": false, "error": format!("unknown method: {}", unknown) })
        }
    }
}
