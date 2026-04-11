@echo off
REM ===============================================================
REM BTCPC One-Click Starter for Windows
REM Usage: double-click this file.
REM Requires: Docker Desktop installed from https://docker.com
REM ===============================================================
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

REM --- Step 1: is docker on PATH? ---
where docker >/dev/null 2>&1
if errorlevel 1 (
    echo [ERROR] The "docker" command is not recognized.
    echo.
    echo This means one of:
    echo   1. Docker Desktop is not installed yet.
    echo      Get it from https://www.docker.com/products/docker-desktop/
    echo   2. Docker Desktop was just installed and this shell is stale.
    echo      Close this window and open a fresh one after Docker Desktop launches.
    echo   3. Docker Desktop is installed but not running.
    echo      Launch "Docker Desktop" from the Start menu and wait for the whale.
    echo.
    pause
    exit /b 1
)

REM --- Step 2: is the docker engine actually running? ---
docker info >/dev/null 2>&1
if errorlevel 1 (
    echo [ERROR] Docker is installed but the engine is not running.
    echo.
    echo Launch "Docker Desktop" from the Start menu. Wait until the
    echo whale icon in your system tray stops animating, then double-click
    echo this file again.
    echo.
    pause
    exit /b 1
)

echo [OK] Docker is running.
echo.

REM --- Step 3: grab docker-compose.yml if missing ---
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
    echo [OK] docker-compose.yml already exists
)
echo.

REM --- Step 3b: download + load BTCPC image if not present ---
docker image inspect btcpc:latest >/dev/null 2>&1
if errorlevel 1 (
    echo BTCPC image not found locally.
    echo Downloading ~200 MB image tarball from btcpc.net ...
    echo ^(first run only - subsequent launches are instant^)
    echo.
    curl.exe -fL -o btcpc-image.tar.gz https://btcpc.net/btcpc-image.tar.gz
    if errorlevel 1 (
        echo [ERROR] Could not download btcpc-image.tar.gz
        echo Check your internet connection and try again.
        pause
        exit /b 1
    )
    echo.
    echo Loading image into Docker ^(takes about a minute^)...
    docker load -i btcpc-image.tar.gz
    if errorlevel 1 (
        echo [ERROR] docker load failed. Tarball may be corrupt.
        echo Delete btcpc-image.tar.gz and try again.
        pause
        exit /b 1
    )
    echo [OK] BTCPC image loaded.
) else (
    echo [OK] BTCPC image already present
)
echo.

REM --- Step 4: existing user or new user? ---
echo.
echo ===============================================================
echo  Do you already have a BTCPC username?
echo ===============================================================
echo.
echo   [1] Yes, I already have one (from @btcpcbot on Telegram)
echo   [2] No, I need to create one
echo.
set /p USER_CHOICE="Enter 1 or 2: "

if "!USER_CHOICE!"=="2" (
    echo.
    echo Opening @btcpcbot in your default browser.
    echo In the bot, type:  /create your_name_here
    echo Save the 12-word phrase it gives you - we cannot recover it.
    echo Then come back here and enter the username you chose.
    echo.
    start https://t.me/btcpcbot
    echo.
    pause
    echo.
)

echo Enter your BTCPC username:
echo ^(lowercase, 3-20 chars, letters/numbers/hyphens only^)
echo.
set /p BTCPC_MINER="Your username: "

if "!BTCPC_MINER!"=="" (
    echo [ERROR] Username cannot be empty.
    pause
    exit /b 1
)

REM --- Step 5: start the stack ---
echo.
echo Starting BTCPC node as miner !BTCPC_MINER! ...
echo.

docker compose up -d
if errorlevel 1 (
    echo.
    echo [ERROR] docker compose up failed. Check the output above.
    pause
    exit /b 1
)

echo.
echo ===============================================================
echo  BTCPC is running in the background as !BTCPC_MINER!
echo.
echo  Check balance in Telegram via @btcpcbot /balance
echo  View logs:   docker compose logs -f btcpc
echo  Stop node:   docker compose stop
echo  Remove all:  docker compose down
echo ===============================================================
echo.

set /p SHOWLOG="Show live logs now? (y/n): "
if /i "!SHOWLOG!"=="y" (
    docker compose logs -f btcpc
)

pause
endlocal
