// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::Manager;

mod hardware;
mod installer;

// ── UFL 3.4 Decentralized acceptance gate (UFL Section 9) ─────────────────
// Nothing that installs or runs HONE works until the user has accepted the
// license on the first-run screen. The record is the same file the Rust node
// reads (~/.hone/license-acceptance.json, or $HONE_DATA_DIR).
// See docs/LICENSE_ACCEPTANCE.md.

const LICENSE_REQUIRED: &str =
    "HONE's license has not been accepted. Accept it on the license screen first.";

fn license_accepted() -> bool {
    hone_license::require_acceptance(
        &hone_license::default_data_dir(),
        hone_license::Mode::Headless,
    )
    .is_ok()
}

#[derive(Serialize)]
struct LicenseTerms {
    accepted: bool,
    ruling: &'static str,
    spdx: &'static str,
    summary: String,
    text: &'static str,
}

#[tauri::command]
fn license_terms() -> LicenseTerms {
    LicenseTerms {
        accepted: license_accepted(),
        ruling: hone_license::RULING_LICENSE,
        spdx: hone_license::SPDX,
        summary: hone_license::summary(),
        text: hone_license::LICENSE_TEXT,
    }
}

#[tauri::command]
fn accept_license() -> Result<(), String> {
    let path = hone_license::record_path(&hone_license::default_data_dir());
    hone_license::write_record(&path, &hone_license::current_binary(), &hone_license::now_utc())
        .map_err(|e| format!("could not save the acceptance record at {}: {e}", path.display()))
}

#[tauri::command]
fn decline_license(app: tauri::AppHandle) {
    app.exit(hone_license::EXIT_NOT_ACCEPTED);
}

#[derive(Default)]
struct MinerState {
    process: Mutex<Option<Child>>,
}

#[derive(Serialize)]
struct StatusResponse {
    running: bool,
    pid: Option<u32>,
    hone_install_dir: String,
    has_node: bool,
    has_ollama: bool,
    has_hone: bool,
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
        hone_install_dir: installer::install_dir().to_string_lossy().to_string(),
        has_node: installer::has_command("node"),
        has_ollama: installer::has_command("ollama"),
        has_hone: installer::has_hone(),
    }
}

#[tauri::command]
fn detect_hardware() -> serde_json::Value {
    hardware::detect()
}

#[tauri::command]
async fn install_hone() -> Result<String, String> {
    if !license_accepted() {
        return Err(LICENSE_REQUIRED.to_string());
    }
    installer::install().await.map_err(|e| e.to_string())
}

#[tauri::command]
fn start_node(
    config: StartConfig,
    state: tauri::State<MinerState>,
) -> Result<String, String> {
    if !license_accepted() {
        return Err(LICENSE_REQUIRED.to_string());
    }
    let mut process = state.process.lock().unwrap();
    if process.is_some() {
        return Err("Node already running".to_string());
    }

    let install_dir = installer::install_dir();
    let bin = if config.mode == "clock" {
        install_dir.join("bin/hone-clock")
    } else {
        install_dir.join("bin/hone-mine")
    };

    let child = Command::new("node")
        .arg(&bin)
        .current_dir(&install_dir)
        .env("HONE_MINER", &config.username)
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
            install_hone,
            start_node,
            stop_node,
            fetch_balance,
            license_terms,
            accept_license,
            decline_license
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
