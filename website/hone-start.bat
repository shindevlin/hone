@echo off
REM HONE Windows Starter - Self-Healing Edition
setlocal enabledelayedexpansion
title HONE Starter

echo.
echo   ######   ######## ######  ######  ######
echo   ##  ##      ##    ##      ##  ##  ##
echo   ######      ##    ##      ######  ##
echo   ##  ##      ##    ##      ##      ##
echo   ######      ##    ######  ##      ######
echo.
echo   Bitcoin Proof of Compute - Windows Starter
echo.

REM Retry counter for main loop
set RETRY_COUNT=0

:MAIN_LOOP
set /a RETRY_COUNT+=1
echo [HONE] Starting up... (attempt !RETRY_COUNT!)

REM ============================================================
REM Step 1: Verify docker is on PATH
REM ============================================================
where docker >/dev/null 2>&1
if errorlevel 1 goto TRY_LAUNCH_DOCKER
echo [HONE] docker command found.
goto CHECK_ENGINE

:TRY_LAUNCH_DOCKER
echo [HONE] Docker not on PATH yet. Waiting 60 seconds for Docker Desktop to finish starting...
timeout /t 60 /nobreak >nul
where docker >/dev/null 2>&1
if not errorlevel 1 goto CHECK_ENGINE

echo [HONE] Trying to launch Docker Desktop...
if exist "C:\Program Files\Docker\Docker\Docker Desktop.exe" (
    start "" "C:\Program Files\Docker\Docker\Docker Desktop.exe"
    echo [HONE] Docker Desktop launched. Waiting 10 minutes for it to fully start...
    for /L %%i in (1,1,120) do (
        timeout /t 5 /nobreak >nul
        where docker >/dev/null 2>&1
        if not errorlevel 1 goto CHECK_ENGINE
    )
)

echo [HONE] Docker Desktop not found locally. Downloading installer...
curl.exe -fSL -o "%TEMP%\DockerDesktopInstaller.exe" "https://desktop.docker.com/win/main/amd64/Docker%%20Desktop%%20Installer.exe"
if errorlevel 1 (
    echo [HONE] Download failed. Sleeping 60 seconds and retrying...
    timeout /t 60 /nobreak >nul
    goto MAIN_LOOP
)
echo [HONE] Running Docker Desktop installer silently...
"%TEMP%\DockerDesktopInstaller.exe" /quiet
echo [HONE] Installer finished. Waiting 60 seconds for first launch...
timeout /t 60 /nobreak >nul
goto MAIN_LOOP

:CHECK_ENGINE
REM ============================================================
REM Step 2: Verify docker engine is running
REM ============================================================
docker info >/dev/null 2>&1
if not errorlevel 1 goto HAVE_ENGINE

echo [HONE] Docker engine is not ready yet. Waiting up to 10 minutes...
for /L %%i in (1,1,120) do (
    timeout /t 5 /nobreak >nul
    docker info >/dev/null 2>&1
    if not errorlevel 1 goto HAVE_ENGINE
)
echo [HONE] Docker engine still not up after 10 minutes. Retrying from start...
timeout /t 30 /nobreak >nul
goto MAIN_LOOP

:HAVE_ENGINE
echo [HONE] Docker engine is running.

REM ============================================================
REM Step 3: Get working directory
REM ============================================================
if not exist "%USERPROFILE%\hone" mkdir "%USERPROFILE%\hone"
cd /d "%USERPROFILE%\hone"

REM ============================================================
REM Step 4: Download docker-compose.yml
REM ============================================================
if exist docker-compose.yml goto HAVE_COMPOSE
echo [HONE] Downloading docker-compose.yml...
:DL_COMPOSE_RETRY
curl.exe -fsSL -o docker-compose.yml https://honemesh.net/docker-compose.yml
if not errorlevel 1 goto HAVE_COMPOSE
echo [HONE] Could not download docker-compose.yml. Retrying in 15 seconds...
timeout /t 15 /nobreak >nul
goto DL_COMPOSE_RETRY
:HAVE_COMPOSE
echo [HONE] docker-compose.yml ready.

REM ============================================================
REM Step 5: Download + load image (with exponential backoff)
REM ============================================================
docker image inspect hone:latest >/dev/null 2>&1
if not errorlevel 1 goto HAVE_IMAGE

echo [HONE] HONE image not present. Downloading (~200 MB)...

set DL_ATTEMPT=0
:DL_IMAGE
set /a DL_ATTEMPT+=1
if !DL_ATTEMPT! gtr 5 goto DL_GIVE_UP

REM Calculate backoff delay: attempt 1=5s, 2=15s, 3=45s, 4=120s, 5=300s
if !DL_ATTEMPT!==1 set DL_WAIT=5
if !DL_ATTEMPT!==2 set DL_WAIT=15
if !DL_ATTEMPT!==3 set DL_WAIT=45
if !DL_ATTEMPT!==4 set DL_WAIT=120
if !DL_ATTEMPT!==5 set DL_WAIT=300

if !DL_ATTEMPT! gtr 1 (
    echo [HONE] Download attempt !DL_ATTEMPT!. Waiting !DL_WAIT! seconds...
    timeout /t !DL_WAIT! /nobreak >nul
)

if exist hone-image.tar.gz del /f hone-image.tar.gz
echo [HONE] Downloading image tarball (attempt !DL_ATTEMPT! of 5)...
curl.exe -fSL -o hone-image.tar.gz https://honemesh.net/hone-image.tar.gz
if errorlevel 1 (
    echo [HONE] Download failed. Will retry...
    goto DL_IMAGE
)

echo [HONE] Loading image into Docker...
docker load -i hone-image.tar.gz
if not errorlevel 1 goto LOAD_OK

echo [HONE] docker load failed. Removing tarball and retrying download...
del /f hone-image.tar.gz
set DL_ATTEMPT=0
set DL_WAIT=5
timeout /t 5 /nobreak >nul
curl.exe -fSL -o hone-image.tar.gz https://honemesh.net/hone-image.tar.gz
if errorlevel 1 (
    echo [HONE] Re-download also failed. Sleeping 60 seconds and restarting...
    timeout /t 60 /nobreak >nul
    goto MAIN_LOOP
)
docker load -i hone-image.tar.gz
if errorlevel 1 (
    echo [HONE] Second load attempt failed. Sleeping 60 seconds and restarting...
    timeout /t 60 /nobreak >nul
    goto MAIN_LOOP
)

:LOAD_OK
echo [HONE] HONE image loaded successfully.
goto HAVE_IMAGE

:DL_GIVE_UP
echo [HONE] Download failed after 5 attempts. Sleeping 5 minutes and restarting...
timeout /t 300 /nobreak >nul
goto MAIN_LOOP

:HAVE_IMAGE
echo [HONE] HONE image ready.

REM ============================================================
REM Step 6: Get username (guest fallback if empty)
REM ============================================================
if defined HONE_MINER goto HAVE_MINER
set MINER_ATTEMPTS=0
:ASK_MINER
set /a MINER_ATTEMPTS+=1
echo.
set /p HONE_MINER="Your HONE username (press Enter to get a guest name): "
if defined HONE_MINER (
    if "!HONE_MINER!"=="" goto USE_GUEST
    goto HAVE_MINER
)
if !MINER_ATTEMPTS! geq 3 goto USE_GUEST
goto ASK_MINER

:USE_GUEST
echo [HONE] No username entered. Generating a guest name...
for /f "delims=" %%g in ('powershell -c "[guid]::NewGuid().ToString().Substring(0,8)"') do set HONE_MINER=guest-%%g
echo [HONE] Mining as guest account: !HONE_MINER!

:HAVE_MINER
echo [HONE] Starting HONE node as miner: !HONE_MINER!

REM ============================================================
REM Step 7: docker compose up
REM ============================================================
set HONE_MINER=!HONE_MINER!
docker compose up -d
if not errorlevel 1 goto RUNNING

echo [HONE] docker compose up failed. Sleeping 30 seconds and retrying...
timeout /t 30 /nobreak >nul
goto MAIN_LOOP

:RUNNING
echo.
echo ===============================================================
echo  HONE is running as !HONE_MINER!
echo ===============================================================
echo.
docker ps --filter name=hone
echo.
echo  View logs:  docker compose logs -f hone
echo  Stop node:  docker compose stop
echo.
echo [HONE] Mining started. Check your balance in Telegram: @honebot /balance
echo.
endlocal
