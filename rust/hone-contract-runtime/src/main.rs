/*!
# hone-contract-runtime

IPC server that executes WASM smart contracts for the HONE chain.
Receives JSON-RPC calls from the Node.js chain process over a Unix socket,
runs the contract in a sandboxed wasmtime environment, and returns results.

## IPC Protocol (JSON-RPC over Unix socket)

Request:
```json
{ "method": "contract_deploy" | "contract_call" | "contract_view", "params": { ... }, "id": 1 }
```

Response:
```json
{ "result": { ... }, "id": 1 }
```
or on error:
```json
{ "error": { "code": -32000, "message": "..." }, "id": 1 }
```
*/

use std::path::Path;
use base64::Engine as _;
use tokio::net::UnixListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use anyhow::Result;
use tracing::info;

use hone_contract_runtime::{
    execute_call, execute_view, execute_deploy,
    CallRequest, DeployRequest,
    state::{ContractState, StorageKv},
};

const SOCKET_PATH: &str = "/tmp/hone-contract-runtime.sock";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    if Path::new(SOCKET_PATH).exists() {
        std::fs::remove_file(SOCKET_PATH)?;
    }

    let listener = UnixListener::bind(SOCKET_PATH)?;
    info!("HONE Contract Runtime listening on {}", SOCKET_PATH);

    loop {
        let (mut socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = vec![0u8; 1 << 20]; // 1MB max request
            match socket.read(&mut buf).await {
                Ok(n) if n > 0 => {
                    let response = handle_request(&buf[..n]);
                    let _ = socket.write_all(response.as_bytes()).await;
                }
                _ => {}
            }
        });
    }
}

fn handle_request(data: &[u8]) -> String {
    let req: serde_json::Value = match serde_json::from_slice(data) {
        Ok(v) => v,
        Err(e) => return json_error(None, -32700, &format!("Parse error: {}", e)),
    };

    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(serde_json::Value::Null);

    let result = match method {
        "contract_call" => {
            match serde_json::from_value::<CallRequest>(params.clone()) {
                Ok(call_req) => {
                    let state = extract_state(&params);
                    let result = execute_call(call_req, state);
                    serde_json::to_value(result).unwrap_or_default()
                }
                Err(e) => return json_error(id.as_ref(), -32602, &format!("Invalid params: {}", e)),
            }
        }
        "contract_view" => {
            match serde_json::from_value::<CallRequest>(params.clone()) {
                Ok(call_req) => {
                    let state = extract_state(&params);
                    let result = execute_view(call_req, state);
                    serde_json::to_value(result).unwrap_or_default()
                }
                Err(e) => return json_error(id.as_ref(), -32602, &format!("Invalid params: {}", e)),
            }
        }
        "contract_deploy" => {
            match serde_json::from_value::<DeployRequest>(params) {
                Ok(deploy_req) => {
                    let result = execute_deploy(deploy_req, StorageKv::default());
                    serde_json::to_value(result).unwrap_or_default()
                }
                Err(e) => return json_error(id.as_ref(), -32602, &format!("Invalid params: {}", e)),
            }
        }
        _ => return json_error(id.as_ref(), -32601, &format!("Method not found: {}", method)),
    };

    serde_json::json!({ "result": result, "id": id }).to_string()
}

fn extract_state(params: &serde_json::Value) -> ContractState {
    let contract_id = params.get("contract_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let balance = params.get("contract_balance")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u128;
    let storage_raw = params.get("storage")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let mut storage = StorageKv::default();
    for (k, v) in storage_raw {
        if let Ok(key) = hex::decode(&k) {
            if let Some(val_b64) = v.as_str() {
                if let Ok(val) = base64::engine::general_purpose::STANDARD.decode(val_b64) {
                    storage.committed.insert(key, val);
                }
            }
        }
    }

    ContractState::new(contract_id, storage, balance)
}

fn json_error(id: Option<&serde_json::Value>, code: i32, msg: &str) -> String {
    serde_json::json!({
        "error": { "code": code, "message": msg },
        "id": id
    }).to_string()
}
