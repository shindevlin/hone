# HONE Desktop

One-click mining for [Bitcoin Proof of Compute](https://hone.net). Native desktop app built with Tauri (Rust + HTML).

## What it does

- Auto-detects your hardware (CPU, GPU, RAM)
- Recommends miner or clock node mode
- Installs HONE node code with one click
- Manages the miner process (start, stop, status)
- Shows your balance and mining stats
- System tray icon, runs in background

## Build

Requires:
- Rust 1.70+ (`rustup`)
- Node.js 20+
- Platform deps:
  - **macOS:** nothing extra (Xcode Command Line Tools)
  - **Linux:** `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev`
  - **Windows:** WebView2 Runtime (preinstalled on Windows 11)
- See full list: https://tauri.app/start/prerequisites/

```bash
npm install
npm run dev    # development
npm run build  # produces .dmg / .msi / .AppImage
```

## Architecture

- **`src-tauri/`** — Rust backend with Tauri commands
  - `main.rs` — entry point, command handlers
  - `hardware.rs` — system hardware detection
  - `installer.rs` — clones/updates HONE repo, runs npm install
- **`src/`** — HTML/JS frontend
  - `index.html` — main UI

## Tauri commands

| Command | Purpose |
|---------|---------|
| `get_status` | Check if installed and running |
| `detect_hardware` | CPU/GPU/RAM info |
| `install_hone` | Clone repo + npm install |
| `start_node` | Spawn the miner/clock process |
| `stop_node` | Kill the running process |
| `fetch_balance` | Query the local API for balance |

## License

MIT — same as HONE core.
