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

REM ── Step 4: ask for username ──
echo Enter your BTCPC username (from @btcpcbot on Telegram).
echo Use lowercase, 3-20 characters, letters/numbers/hyphens only.
echo If you don't have one yet, open Telegram and message @btcpcbot /create your_name first.
echo.
set /p BTCPC_MINER="Your username: "

if "!BTCPC_MINER!"=="" (
    echo [ERROR] Username cannot be empty.
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
