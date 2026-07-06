# BTCPC System Tray

Native system tray icon for each OS. Shows mining status, notifies on updates, and provides quick access to settings.

## Quick Start

```bash
# Start the tray icon (auto-detects your OS)
./bin/btcpc-tray &
```

## Per-OS Details

### Linux (GNOME/KDE/XFCE)

**Requires:** `yad` (GTK dialog tool)
```bash
sudo apt install yad
```

**What it does:**
- AppIndicator icon in the top panel
- Tooltip shows: epoch, total mined, block count
- Left-click: opens Settings page in browser
- Right-click menu: Settings, Explorer, Pause, Resume, Quit
- Desktop notifications via `notify-send` when updates are available

**Fallback:** If `yad` is not installed, runs in notification-only mode using `notify-send`.

**Autostart on login:**
```bash
cp bin/tray/btcpc-tray.desktop ~/.config/autostart/
```

### Mac (macOS)

**Requires:** Nothing — uses built-in `osascript` (AppleScript)

**What it does:**
- Menu bar item showing "BTCPC"
- Click menu: Settings, Explorer, Pause, Resume, Quit
- Native macOS notifications when updates are available

**Run:**
```bash
osascript bin/tray/mac-tray.applescript &
```

### Windows

**Requires:** Nothing — uses built-in PowerShell + .NET

**What it does:**
- System tray icon (orange circle)
- Double-click: opens Settings page
- Right-click menu: Settings, Explorer, Pause, Resume, Quit
- Balloon notifications for updates and status changes

**Run:**
```powershell
powershell -ExecutionPolicy Bypass -File bin\tray\windows-tray.ps1
```

**Autostart on login:** Add a shortcut to `shell:startup` folder.

## Settings Page

All platforms open `http://localhost:4242/settings` which provides:
- CPU slider (10-100%) — limits inference thread count
- GPU slider (0-100%) — limits GPU layer offloading (0 = CPU only)
- Full / Reduced / Pause / Auto mode buttons
- Live status display

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HONE_MAX_CPU` | `100` | Max CPU percentage for mining |
| `HONE_MAX_GPU` | `100` | Max GPU percentage (0 = CPU only) |
| `HONE_IDLE_THRESHOLD_MS` | `120000` | Idle time before full speed (2 min) |
| `HONE_REDUCED_HOURS` | (none) | Reduced hours schedule, e.g. `09:00-17:00` |

## How Auto Mode Works

1. Miner checks mouse/keyboard idle time every 30 seconds
2. If idle > 2 minutes → full speed mining
3. If user is active → reduced mining (1 work item per epoch)
4. Schedule overrides idle detection during configured hours
5. Manual override (pause/resume) overrides everything
