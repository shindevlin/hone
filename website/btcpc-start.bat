@echo off
REM ─────────────────────────────────────────────────────────────
REM BTCPC One-Click Starter for Windows
REM Usage: double-click this file. It will:
REM   1. Check Docker Desktop is installed and running
REM   2. Download docker-compose.yml if missing
REM   3. Ask for your BTCPC username
REM   4. Start the BTCPC node and show logs
REM Requires: Docker Desktop installed from https://docker.com
REM ─────────────────────────────────────────────────────────────
setlocal enabledelayedexpansion
title BTCPC Starter

echo.
echo   ######   ######## ######  ######  ######
echo   ##  ##      ##    ##      ##  ##  ##
echo   ######      ##    ##      ######  ##
echo   ##  ##      ##    ##      ##      ##
echo   ######      ##    ######  ##      ######
echo.
echo   Bitcoin Proof of Compute - Windows Starter
echo.

REM ── Step 1: is docker on PATH? ──
where docker >nul 2>&1
if errorlevel 1 (
    echo [ERROR] The 'docker' command is not recognized.
    echo.
    echo This means one of the following:
    echo   1. Docker Desktop is not installed yet.
    echo      Download it from: https://www.docker.com/products/docker-desktop/
    echo   2. Docker Desktop was just installed and this shell is stale.
    echo      Close this window and open a fresh one after Docker Desktop launches.
    echo   3. Docker Desktop is installed but not running.
    echo      Launch 'Docker Desktop' from the Start menu and wait for the whale.
    echo.
    pause
    exit /b 1
)

REM ── Step 2: is the docker engine actually running? ──
docker info >nul 2>&1
if errorlevel 1 (
    echo [ERROR] Docker is installed but the engine is not running.
    echo.
    echo Launch 'Docker Desktop' from the Start menu. Wait until the whale icon
    echo in your system tray stops animating (2-3 minutes on first launch).
    echo Then double-click this file again.
    echo.
    pause
    exit /b 1
)

echo [OK] Docker is running.
echo.

REM ── Step 3: grab docker-compose.yml if missing ──
if not exist docker-compose.yml (
    echo Downloading docker-compose.yml from btcpc.net ...
    curl.exe -fsSL -o docker-compose.yml https://btcpc.net/docker-compose.yml
    if errorlevel 1 (
        echo [ERROR] Could not download docker-compose.yml
        echo Check your internet connection.
        pause
        exit /b 1
    )
    echo [OK] Downloaded docker-compose.yml
) else (
    echo [OK] docker-compose.yml already exists in this folder
)
echo.

REM ── Step 3b: download + load the BTCPC image if not present ──
docker image inspect btcpc:latest >nul 2>&1
if errorlevel 1 (
    echo BTCPC image not found locally. Downloading from btcpc.net ...
    echo This is a one-time ~200 MB download. First run may take 2-5 minutes.
    echo.
    curl.exe -fL -o btcpc-image.tar.gz --progress-bar https://btcpc.net/btcpc-image.tar.gz
    if errorlevel 1 (
        echo [ERROR] Could not download btcpc-image.tar.gz
        echo Check your internet connection and try again.
        pause
        exit /b 1
    )
    echo.
    echo Loading BTCPC image into Docker ^(this extracts ~800 MB, takes a minute^)...
    docker load -i btcpc-image.tar.gz
    if errorlevel 1 (
        echo [ERROR] docker load failed. The tarball may be corrupt.
        echo Delete btcpc-image.tar.gz and try again.
        pause
        exit /b 1
    )
    echo [OK] BTCPC image loaded.
    REM Keep the tarball around so subsequent re-loads are offline
) else (
    echo [OK] BTCPC image already present locally
)
echo.

REM ── Step 4: existing user or new user? ──
echo.
echo ─────────────────────────────────────────────────────────────
echo  Do you already have a BTCPC username?
echo ─────────────────────────────────────────────────────────────
echo.
echo   [1] Yes, I already have one (from @btcpcbot on Telegram)
echo   [2] No, I need to create one
echo.
set /p USER_CHOICE="Enter 1 or 2: "

if "!USER_CHOICE!"=="2" (
    echo.
    echo ─────────────────────────────────────────────────────────────
    echo  Let's get you a username via Telegram
    echo ─────────────────────────────────────────────────────────────
    echo.
    echo  Opening Telegram bot @btcpcbot in your browser now.
    echo  In Telegram:
    echo    1. Click 'Start' to begin the chat
    echo    2. Type:  /create your_name_here
    echo       (lowercase, 3-20 chars, letters/numbers/hyphens)
    echo    3. The bot will give you a 12-word backup phrase.
    echo       WRITE IT DOWN on paper. We cannot recover it if lost.
    echo    4. Come back here and enter the username you chose.
    echo.
    start https://t.me/btcpcbot
    echo.
    pause
    echo.
)

echo Enter your BTCPC username:
echo Use lowercase, 3-20 characters, letters/numbers/hyphens only.
echo.
set /p BTCPC_MINER="Your username: "

if "!BTCPC_MINER!"=="" (
    echo [ERROR] Username cannot be empty.
    pause
    exit /b 1
)

REM Basic sanity check — lowercase letters, digits, hyphens only
echo !BTCPC_MINER!| findstr /r "^[a-z0-9][a-z0-9-]*$" >nul
if errorlevel 1 (
    echo [ERROR] Invalid username format.
    echo Use only lowercase letters, digits, and hyphens.
    echo Start with a letter or digit. No spaces or special characters.
    pause
    exit /b 1
)

echo.
echo Starting BTCPC node as miner '!BTCPC_MINER!' ...
echo (first run downloads the container images -- takes 2-5 minutes)
echo.

REM ── Step 5: start the stack ──
docker compose up -d
if errorlevel 1 (
    echo [ERROR] docker compose up failed. Check the output above.
    pause
    exit /b 1
)

echo.
echo ─────────────────────────────────────────────────────────────
echo  BTCPC is running in the background as miner '!BTCPC_MINER!'
echo.
echo  Your mining rewards will credit to your on-chain account.
echo  Check your balance in Telegram with @btcpcbot /balance
echo.
echo  Commands you might want:
echo    docker compose logs -f btcpc     show live logs
echo    docker compose stop              stop the node
echo    docker compose down              stop and remove containers
echo    docker compose up -d             start again after stop
echo ─────────────────────────────────────────────────────────────
echo.

set /p SHOWLOG="Show live logs now? (y/n): "
if /i "!SHOWLOG!"=="y" (
    docker compose logs -f btcpc
)

pause
endlocal
