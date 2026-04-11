# BTCPC Windows Starter (PowerShell)
# Usage:
#   1. Open PowerShell (Start menu -> type PowerShell -> Enter)
#   2. Paste this one-liner and press Enter:
#        irm https://btcpc.net/btcpc-start.ps1 | iex
# That downloads and runs this script with nothing to install.

$ErrorActionPreference = "Stop"

function Say($msg, $color = "White") { Write-Host $msg -ForegroundColor $color }
function Hdr($msg) { Say "" ; Say ("=" * 65) "Yellow" ; Say "  $msg" "Yellow" ; Say ("=" * 65) "Yellow" ; Say "" }
function Err($msg) { Say "[ERROR] $msg" "Red" }
function Ok($msg) { Say "[OK] $msg" "Green" }

Hdr "BTCPC - Bitcoin Proof of Compute - Starter"

# Step 1: docker on PATH?
try {
    $null = Get-Command docker -ErrorAction Stop
    Ok "docker command found"
} catch {
    Err "The docker command is not on your PATH."
    Say ""
    Say "Fix options:"
    Say "  1. Install Docker Desktop from docker.com then REBOOT your PC"
    Say "     (reboot is important - PATH only updates on new sessions)"
    Say "  2. If Docker Desktop is installed, launch it from the Start menu"
    Say "     and wait for the whale icon in your system tray."
    Say ""
    Say "Your current PATH:"
    Say $env:PATH
    Read-Host "Press Enter to exit"
    exit 1
}

# Step 2: docker engine running?
docker info *> $null
if ($LASTEXITCODE -ne 0) {
    Err "Docker command works but engine is not running."
    Say ""
    Say "Launch Docker Desktop from the Start menu. Wait until the"
    Say "whale icon in your tray stops animating, then run this again."
    Read-Host "Press Enter to exit"
    exit 1
}
Ok "docker engine is running"

# Step 3: working directory
$workDir = Join-Path $env:USERPROFILE "btcpc"
if (-not (Test-Path $workDir)) {
    New-Item -ItemType Directory -Path $workDir | Out-Null
}
Set-Location $workDir
Ok "Working in $workDir"

# Step 4: docker-compose.yml
if (-not (Test-Path "docker-compose.yml")) {
    Say "Downloading docker-compose.yml..."
    try {
        Invoke-WebRequest -Uri "https://btcpc.net/docker-compose.yml" -OutFile "docker-compose.yml" -UseBasicParsing
    } catch {
        Err "Could not download docker-compose.yml"
        Say $_.Exception.Message
        Read-Host "Press Enter to exit"
        exit 1
    }
    Ok "downloaded docker-compose.yml"
} else {
    Ok "docker-compose.yml already present"
}

# Step 5: btcpc image
docker image inspect btcpc:latest *> $null
if ($LASTEXITCODE -ne 0) {
    Say ""
    Say "BTCPC image not found locally."
    Say "Downloading ~200 MB image tarball (first run only)..."
    try {
        $ProgressPreference = "Continue"
        Invoke-WebRequest -Uri "https://btcpc.net/btcpc-image.tar.gz" -OutFile "btcpc-image.tar.gz" -UseBasicParsing
    } catch {
        Err "Could not download btcpc-image.tar.gz"
        Say $_.Exception.Message
        Read-Host "Press Enter to exit"
        exit 1
    }
    Say ""
    Say "Loading image into Docker (about a minute)..."
    docker load -i btcpc-image.tar.gz
    if ($LASTEXITCODE -ne 0) {
        Err "docker load failed. Delete btcpc-image.tar.gz and try again."
        Read-Host "Press Enter to exit"
        exit 1
    }
    Ok "BTCPC image loaded"
} else {
    Ok "BTCPC image already present"
}

# Step 6: ask existing vs new username
Hdr "Do you already have a BTCPC username?"
Say "  [1] Yes, I already have one (from @btcpcbot on Telegram)"
Say "  [2] No, I need to create one"
Say ""
$choice = Read-Host "Enter 1 or 2"

if ($choice -eq "2") {
    Say ""
    Say "Opening @btcpcbot in your default browser."
    Say "In the bot, type:  /create your_name_here"
    Say "Save the 12-word phrase - we cannot recover it if lost."
    Start-Process "https://t.me/btcpcbot"
    Say ""
    Read-Host "Press Enter after creating your username"
}

# Step 7: username
do {
    $miner = Read-Host "Enter your BTCPC username"
    if ($null -eq $miner) { $miner = "" }
    $miner = $miner.Trim().ToLower()
    if ($miner -notmatch "^[a-z0-9][a-z0-9-]{2,19}$") {
        Err "Invalid format. Use 3-20 lowercase letters, digits, or hyphens."
        $miner = ""
    }
} while (-not $miner)

# Step 8: start
Say ""
Say "Starting BTCPC node as miner $miner ..." "Yellow"
Say ""
$env:BTCPC_MINER = $miner
docker compose up -d
if ($LASTEXITCODE -ne 0) {
    Err "docker compose up failed. See output above."
    Read-Host "Press Enter to exit"
    exit 1
}

Hdr "BTCPC is running as $miner"
Say "Containers running:"
docker ps --filter "name=btcpc"
Say ""
Say "Check balance in Telegram:  @btcpcbot /balance"
Say "View live logs:             docker compose logs -f btcpc"
Say "Stop node:                  docker compose stop"
Say "Start again:                docker compose up -d"
Say "Remove everything:          docker compose down"
Say ""

$showLogs = Read-Host "Show live logs now? (y/n)"
if ($showLogs -match "^[yY]") {
    docker compose logs -f btcpc
}

Read-Host "Press Enter to exit"
