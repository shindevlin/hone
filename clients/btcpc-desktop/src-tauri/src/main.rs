// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::Manager;

mod hardware;
mod installer;

#[derive(Default)]
struct MinerState {
    process: Mutex<Option<Child>>,
}

#[derive(Serialize)]
struct StatusResponse {
    running: bool,
    pid: Option<u32>,
    btcpc_install_dir: String,
    has_node: bool,
    has_ollama: bool,
    has_btcpc: bool,
}

#[derive(Serialize, Deserialize)]
struct StartConfig {
    username: String,
    mode: String, // "miner" | "clock" | "both"
}

#[tauri::command]
fn get_status(state: tauri::State<MinerState>) -> StatusResponse {
    let process = state.process.lock().unwrap();
    let running = process.is_some();
    let pid = process.as_ref().map(|c| c.id());
    drop(process);

    StatusResponse {
        running,
        pid,
        btcpc_install_dir: installer::install_dir().to_string_lossy().to_string(),
        has_node: installer::has_command("node"),
        has_ollama: installer::has_command("ollama"),
        has_btcpc: installer::has_btcpc(),
    }
}

#[tauri::command]
fn detect_hardware() -> serde_json::Value {
    hardware::detect()
}

#[tauri::command]
async fn install_btcpc() -> Result<String, String> {
    installer::install().await.map_err(|e| e.to_string())
}

#[tauri::command]
fn start_node(
    config: StartConfig,
    state: tauri::State<MinerState>,
) -> Result<String, String> {
    let mut process = state.process.lock().unwrap();
    if process.is_some() {
        return Err("Node already running".to_string());
    }

    let install_dir = installer::install_dir();
    let bin = if config.mode == "clock" {
        install_dir.join("bin/btcpc-clock")
    } else {
        install_dir.join("bin/btcpc-mine")
    };

    let child = Command::new("node")
        .arg(&bin)
        .current_dir(&install_dir)
        .env("BTCPC_MINER", &config.username)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start node: {}", e))?;

    let pid = child.id();
    *process = Some(child);
    Ok(format!("Node started (pid {})", pid))
}

#[tauri::command]
fn stop_node(state: tauri::State<MinerState>) -> Result<String, String> {
    let mut process = state.process.lock().unwrap();
    if let Some(mut child) = process.take() {
        child.kill().map_err(|e| format!("Failed to stop: {}", e))?;
        Ok("Stopped".to_string())
    } else {
        Err("Node not running".to_string())
    }
}

#[tauri::command]
async fn fetch_balance(username: String) -> Result<serde_json::Value, String> {
    let url = format!("http://localhost:3000/api/wallet/balance/{}", username);
    let res = reqwest::get(&url)
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;
    Ok(res)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_process::init())
        .manage(MinerState::default())
        .invoke_handler(tauri::generate_handler![
            get_status,
            detect_hardware,
            install_btcpc,
            start_node,
            stop_node,
            fetch_balance
        ])
        .setup(|app| {
            // Hide window initially if minimized to tray
            #[cfg(desktop)]
            {
                let _ = app.get_webview_window("main").map(|w| w.show());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
