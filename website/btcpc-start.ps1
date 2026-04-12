# BTCPC Windows Starter (PowerShell) - Self-Healing Edition
# Usage: [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; irm https://btcpc.net/btcpc-start.ps1 | iex

$ErrorActionPreference = "Continue"
$PSNativeCommandUseErrorActionPreference = $false
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

function Say($msg, $color = "White") { Write-Host $msg -ForegroundColor $color }
function Ok($msg)  { Say "[BTCPC] $msg" "Green" }
function Info($msg) { Say "[BTCPC] $msg" "Yellow" }

Say ""
Say "  ######   ######## ######  ######  ######" "Yellow"
Say "  ##  ##      ##    ##      ##  ##  ##" "Yellow"
Say "  ######      ##    ##      ######  ##" "Yellow"
Say "  ##  ##      ##    ##      ##      ##" "Yellow"
Say "  ######      ##    ######  ##      ######" "Yellow"
Say ""
Say "  Bitcoin Proof of Compute - Windows Starter" "Yellow"
Say ""

$keepRetrying = $true
$loopCount = 0

do {
    $loopCount++
    Info "Starting up... (attempt $loopCount)"

    # ----------------------------------------------------------------
    # Step 1: Verify docker is on PATH
    # ----------------------------------------------------------------
    $dockerCmd = Get-Command docker -ErrorAction SilentlyContinue
    if (-not $dockerCmd) {
        Info "Docker not found. Checking if Docker Desktop is installed..."
        $ddPath = "C:\Program Files\Docker\Docker\Docker Desktop.exe"
        if (Test-Path $ddPath) {
            Info "Launching Docker Desktop silently..."
            Start-Process $ddPath -WindowStyle Hidden -ErrorAction SilentlyContinue
        } else {
            Info "Docker Desktop not installed. Downloading installer (~500 MB)..."
            $dlUrl = "https://desktop.docker.com/win/main/amd64/Docker%20Desktop%20Installer.exe"
            $dlDest = "$env:TEMP\DockerDesktopInstaller.exe"
            try {
                Invoke-WebRequest -Uri $dlUrl -OutFile $dlDest -UseBasicParsing -ErrorAction Stop
                Info "Installing Docker Desktop silently..."
                Start-Process $dlDest -ArgumentList "/quiet" -Wait -ErrorAction SilentlyContinue
            } catch {
                Info "Could not download Docker Desktop. Retrying in 60 seconds..."
                Start-Sleep 60
                continue
            }
        }
        Info "Waiting up to 10 minutes for Docker Desktop to start..."
        $waited = 0
        while ($waited -lt 600) {
            Start-Sleep 5
            $waited += 5
            $dockerCmd = Get-Command docker -ErrorAction SilentlyContinue
            if ($dockerCmd) { break }
        }
        if (-not $dockerCmd) {
            Info "Docker still not ready. Retrying from start in 30 seconds..."
            Start-Sleep 30
            continue
        }
    }
    Ok "docker command found"

    # ----------------------------------------------------------------
    # Step 2: Verify docker engine is running
    # ----------------------------------------------------------------
    $null = docker info 2>&1
    if ($LASTEXITCODE -ne 0) {
        Info "Docker engine not running yet. Trying to start Docker Desktop..."
        Start-Process "Docker Desktop" -WindowStyle Hidden -ErrorAction SilentlyContinue
        Info "Waiting up to 10 minutes for Docker engine..."
        $waited = 0
        while ($waited -lt 600) {
            Start-Sleep 5
            $waited += 5
            $null = docker info 2>&1
            if ($LASTEXITCODE -eq 0) { break }
        }
        if ($LASTEXITCODE -ne 0) {
            Info "Docker engine still not ready. Retrying from start in 30 seconds..."
            Start-Sleep 30
            continue
        }
    }
    Ok "docker engine is running"

    # ----------------------------------------------------------------
    # Step 3: Working directory
    # ----------------------------------------------------------------
    $workDir = Join-Path $env:USERPROFILE "btcpc"
    if (-not (Test-Path $workDir)) { New-Item -ItemType Directory -Path $workDir | Out-Null }
    Set-Location $workDir
    Ok "Working in $workDir"

    # ----------------------------------------------------------------
    # Step 4: docker-compose.yml
    # ----------------------------------------------------------------
    if (-not (Test-Path "docker-compose.yml")) {
        Info "Downloading docker-compose.yml..."
        $dlOk = $false
        while (-not $dlOk) {
            try {
                Invoke-WebRequest -Uri "https://btcpc.net/docker-compose.yml" -OutFile "docker-compose.yml" -UseBasicParsing -ErrorAction Stop
                $dlOk = $true
            } catch {
                Info "Could not download docker-compose.yml. Retrying in 15 seconds..."
                Start-Sleep 15
            }
        }
        Ok "docker-compose.yml downloaded"
    } else {
        Ok "docker-compose.yml already present"
    }

    # ----------------------------------------------------------------
    # Step 5: Download + load BTCPC image (exponential backoff)
    # ----------------------------------------------------------------
    $imagesList = docker images --format "{{.Repository}}:{{.Tag}}" 2>$null
    $imagePresent = $false
    if ($imagesList -is [array]) {
        $imagePresent = $imagesList -contains "btcpc:latest"
    } elseif ($imagesList -is [string]) {
        $imagePresent = ($imagesList -eq "btcpc:latest") -or ($imagesList -like "*btcpc:latest*")
    }

    if (-not $imagePresent) {
        Info "BTCPC image not found locally. Downloading (~200 MB)..."
        $delays = @(5, 15, 45, 120, 300)
        $dlAttempt = 0
        $dlSuccess = $false
        while (-not $dlSuccess -and $dlAttempt -lt 5) {
            if ($dlAttempt -gt 0) {
                $wait = $delays[$dlAttempt - 1]
                Info "Download attempt $($dlAttempt + 1). Waiting $wait seconds..."
                Start-Sleep $wait
            }
            $dlAttempt++
            if (Test-Path "btcpc-image.tar.gz") { Remove-Item "btcpc-image.tar.gz" -Force }
            Info "Downloading image tarball (attempt $dlAttempt of 5)..."
            try {
                $ProgressPreference = "SilentlyContinue"
                Invoke-WebRequest -Uri "https://btcpc.net/btcpc-image.tar.gz" -OutFile "btcpc-image.tar.gz" -UseBasicParsing -ErrorAction Stop
                $dlSuccess = $true
            } catch {
                Info "Download failed: $($_.Exception.Message)"
            }
        }
        if (-not $dlSuccess) {
            Info "Download failed after 5 attempts. Sleeping 5 minutes and retrying..."
            Start-Sleep 300
            continue
        }

        Info "Loading image into Docker (about a minute)..."
        docker load -i btcpc-image.tar.gz
        if ($LASTEXITCODE -ne 0) {
            Info "docker load failed. Removing tarball and retrying download..."
            if (Test-Path "btcpc-image.tar.gz") { Remove-Item "btcpc-image.tar.gz" -Force }
            try {
                $ProgressPreference = "SilentlyContinue"
                Invoke-WebRequest -Uri "https://btcpc.net/btcpc-image.tar.gz" -OutFile "btcpc-image.tar.gz" -UseBasicParsing -ErrorAction Stop
                docker load -i btcpc-image.tar.gz
                if ($LASTEXITCODE -ne 0) {
                    Info "Second load attempt failed. Restarting in 60 seconds..."
                    Start-Sleep 60
                    continue
                }
            } catch {
                Info "Re-download also failed. Restarting in 60 seconds..."
                Start-Sleep 60
                continue
            }
        }

        $imagesAfter = docker images --format "{{.Repository}}:{{.Tag}}" 2>$null
        $loadedOk = $false
        if ($imagesAfter -is [array]) {
            $loadedOk = $imagesAfter -contains "btcpc:latest"
        } elseif ($imagesAfter -is [string]) {
            $loadedOk = ($imagesAfter -eq "btcpc:latest") -or ($imagesAfter -like "*btcpc:latest*")
        }
        if (-not $loadedOk) {
            Info "Image load reported success but btcpc:latest is missing. Restarting..."
            Start-Sleep 30
            continue
        }
        Ok "BTCPC image loaded"
    } else {
        Ok "BTCPC image already present"
    }

    # ----------------------------------------------------------------
    # Step 6: Username (guest fallback if empty)
    # ----------------------------------------------------------------
    $miner = $env:BTCPC_MINER
    if (-not $miner) {
        Say ""
        Say "Enter your BTCPC username, or press Enter for a guest name." "Cyan"
        $miner = Read-Host "Your BTCPC username"
        if ($null -eq $miner) { $miner = "" }
        $miner = $miner.Trim().ToLower()
    }
    if (-not $miner -or $miner -eq "") {
        $miner = "guest-" + [guid]::NewGuid().ToString().Substring(0, 8)
        Info "No username entered. Mining as guest account: $miner"
    }
    $env:BTCPC_MINER = $miner
    Ok "Mining as: $miner"

    # ----------------------------------------------------------------
    # Step 7: docker compose up
    # ----------------------------------------------------------------
    Info "Starting BTCPC node as $miner ..."
    docker compose up -d
    if ($LASTEXITCODE -ne 0) {
        Info "docker compose up failed. Retrying in 30 seconds..."
        Start-Sleep 30
        continue
    }

    Say ""
    Say ("=" * 65) "Green"
    Say "  BTCPC is running as $miner" "Green"
    Say ("=" * 65) "Green"
    Say ""
    docker ps --filter "name=btcpc"
    Say ""
    Say "  Check balance:  @btcpcbot /balance on Telegram" "Cyan"
    Say "  View logs:      docker compose logs -f btcpc" "Cyan"
    Say "  Stop node:      docker compose stop" "Cyan"
    Say ""
    Ok "Mining started. This window can stay open to show logs."
    docker compose logs -f btcpc

    $keepRetrying = $false

} while ($keepRetrying)
