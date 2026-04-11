# BTCPC Windows Starter (PowerShell)
# Usage: open PowerShell, then run:
#     irm https://btcpc.net/btcpc-start.ps1 | iex

# IMPORTANT: do NOT set $ErrorActionPreference = "Stop" globally.
# It causes native commands that exit non-zero (like docker image
# inspect on a missing image) to throw and kill the script before
# the $LASTEXITCODE check runs.
$ErrorActionPreference = "Continue"
$PSNativeCommandUseErrorActionPreference = $false

function Say($msg, $color = "White") { Write-Host $msg -ForegroundColor $color }
function Hdr($msg) { Say "" ; Say ("=" * 65) "Yellow" ; Say "  $msg" "Yellow" ; Say ("=" * 65) "Yellow" ; Say "" }
function Err($msg) { Say "[ERROR] $msg" "Red" }
function Ok($msg) { Say "[OK] $msg" "Green" }

Hdr "BTCPC - Bitcoin Proof of Compute - Starter"

# Step 1: docker on PATH?
$dockerCmd = Get-Command docker -ErrorAction SilentlyContinue
if (-not $dockerCmd) {
    Err "The docker command is not on your PATH."
    Say ""
    Say "Fix options:"
    Say "  1. Install Docker Desktop from docker.com then REBOOT."
    Say "  2. If Docker Desktop is installed, launch it from the Start menu"
    Say "     and wait for the whale icon in your system tray."
    Say ""
    Say "Your current PATH:"
    Say $env:PATH
    Read-Host "Press Enter to exit"
    exit 1
}
Ok "docker command found"

# Step 2: docker engine running?
$null = docker info 2>&1
if ($LASTEXITCODE -ne 0) {
    Err "Docker command works but engine is not running."
    Say "Launch Docker Desktop and wait for the whale icon, then re-run."
    Read-Host "Press Enter to exit"
    exit 1
}
Ok "docker engine is running"

# Step 3: working directory
$workDir = Join-Path $env:USERPROFILE "btcpc"
if (-not (Test-Path $workDir)) { New-Item -ItemType Directory -Path $workDir | Out-Null }
Set-Location $workDir
Ok "Working in $workDir"

# Step 4: docker-compose.yml
if (-not (Test-Path "docker-compose.yml")) {
    Say "Downloading docker-compose.yml..."
    try {
        Invoke-WebRequest -Uri "https://btcpc.net/docker-compose.yml" -OutFile "docker-compose.yml" -UseBasicParsing -ErrorAction Stop
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

# Step 5: check for btcpc image via docker images, not image inspect
$imagesList = docker images --format "{{.Repository}}:{{.Tag}}" 2>$null
$imagePresent = $false
if ($imagesList -is [array]) {
    $imagePresent = $imagesList -contains "btcpc:latest"
} elseif ($imagesList -is [string]) {
    $imagePresent = ($imagesList -eq "btcpc:latest") -or ($imagesList -like "*btcpc:latest*")
}

if (-not $imagePresent) {
    Say ""
    Say "BTCPC image not found locally."
    Say "Downloading ~200 MB image tarball (first run only)..."
    if (Test-Path "btcpc-image.tar.gz") {
        Say "Removing stale btcpc-image.tar.gz from previous attempt..."
        Remove-Item "btcpc-image.tar.gz" -Force
    }
    try {
        $ProgressPreference = "Continue"
        Invoke-WebRequest -Uri "https://btcpc.net/btcpc-image.tar.gz" -OutFile "btcpc-image.tar.gz" -UseBasicParsing -ErrorAction Stop
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
        Err "docker load failed. See output above."
        Read-Host "Press Enter to exit"
        exit 1
    }
    $imagesAfter = docker images --format "{{.Repository}}:{{.Tag}}" 2>$null
    $loadedOk = $false
    if ($imagesAfter -is [array]) {
        $loadedOk = $imagesAfter -contains "btcpc:latest"
    } elseif ($imagesAfter -is [string]) {
        $loadedOk = ($imagesAfter -eq "btcpc:latest") -or ($imagesAfter -like "*btcpc:latest*")
    }
    if (-not $loadedOk) {
        Err "docker load reported success but btcpc:latest is still missing."
        Say "Run docker images to see what was actually loaded."
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
    Say "Opening @btcpcbot in your browser."
    Say "Type:  /create your_name_here"
    Say "Save the 12-word phrase."
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
$env:BTCPC_MINER = $miner
docker compose up -d
if ($LASTEXITCODE -ne 0) {
    Err "docker compose up failed. See output above."
    Read-Host "Press Enter to exit"
    exit 1
}

Hdr "BTCPC is running as $miner"
Say "Containers:"
docker ps --filter "name=btcpc"
Say ""
Say "Check balance in Telegram:  @btcpcbot /balance"
Say "View logs:  docker compose logs -f btcpc"
Say "Stop:       docker compose stop"
Say ""

$showLogs = Read-Host "Show live logs now? (y/n)"
if ($showLogs -match "^[yY]") { docker compose logs -f btcpc }

Read-Host "Press Enter to exit"
