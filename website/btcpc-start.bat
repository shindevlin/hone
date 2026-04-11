@echo off
REM BTCPC Windows Starter
REM Diagnostic version - pauses at every step so errors are visible
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

echo [DEBUG] Starting checks...
echo [DEBUG] PATH = %PATH%
echo.

REM --- Check 1: docker on PATH ---
echo Checking for docker command...
where docker
if errorlevel 1 goto NO_DOCKER
echo [OK] docker command found
echo.

REM --- Check 2: docker engine running ---
echo Checking docker engine...
docker info >nul 2>&1
if errorlevel 1 goto NO_ENGINE
echo [OK] docker engine running
echo.

REM --- Download compose file ---
if exist docker-compose.yml goto HAVE_COMPOSE
echo Downloading docker-compose.yml ...
curl.exe -fsSL -o docker-compose.yml https://btcpc.net/docker-compose.yml
if errorlevel 1 goto NO_COMPOSE
echo [OK] Downloaded docker-compose.yml
goto COMPOSE_DONE
:HAVE_COMPOSE
echo [OK] docker-compose.yml already present
:COMPOSE_DONE
echo.

REM --- Download + load image ---
docker image inspect btcpc:latest >nul 2>&1
if not errorlevel 1 goto HAVE_IMAGE
echo BTCPC image not present. Downloading ~200 MB tarball...
curl.exe -fL -o btcpc-image.tar.gz https://btcpc.net/btcpc-image.tar.gz
if errorlevel 1 goto NO_DOWNLOAD
echo Loading image into docker...
docker load -i btcpc-image.tar.gz
if errorlevel 1 goto NO_LOAD
echo [OK] BTCPC image loaded
goto IMAGE_DONE
:HAVE_IMAGE
echo [OK] BTCPC image already present
:IMAGE_DONE
echo.

REM --- Ask for username ---
echo.
echo Do you already have a BTCPC username?
echo   [1] Yes, I already have one
echo   [2] No, I need to create one via Telegram @btcpcbot
echo.
set /p CHOICE="Enter 1 or 2: "
if "!CHOICE!"=="2" (
    echo Opening @btcpcbot in your browser...
    start https://t.me/btcpcbot
    echo After creating your username in Telegram, come back here.
    pause
)

echo.
set /p MINER="Your BTCPC username: "
if "!MINER!"=="" goto BAD_NAME

REM --- Start ---
echo.
echo Starting BTCPC node as miner !MINER! ...
set BTCPC_MINER=!MINER!
docker compose up -d
if errorlevel 1 goto NO_START

echo.
echo ===============================================================
echo  BTCPC is running in the background as !MINER!
echo ===============================================================
echo.
docker ps --filter name=btcpc
echo.
echo  View logs:  docker compose logs -f btcpc
echo  Stop node:  docker compose stop
echo.
set /p LOGS="Show live logs now? (y/n): "
if /i "!LOGS!"=="y" docker compose logs -f btcpc
goto END

:NO_DOCKER
echo.
echo [ERROR] The docker command is not on your PATH.
echo.
echo Common causes:
echo  1. Docker Desktop is not installed. Get it from docker.com
echo  2. Docker Desktop is installed but you need to sign out and
echo     sign back in (or reboot) so Windows picks up the new PATH.
echo  3. Docker Desktop is installed but not launched.
echo     Start it from your Start menu and wait for the whale icon.
echo.
echo Your current PATH is shown above this error message.
echo If it does not contain a Docker folder, cause 2 is likely.
goto END

:NO_ENGINE
echo.
echo [ERROR] Docker command exists but the engine is not running.
echo Launch Docker Desktop from the Start menu. Wait for the whale.
goto END

:NO_COMPOSE
echo [ERROR] Could not download docker-compose.yml
echo Check your internet connection.
goto END

:NO_DOWNLOAD
echo [ERROR] Could not download btcpc-image.tar.gz
echo Check your internet connection and try again.
goto END

:NO_LOAD
echo [ERROR] docker load failed. Tarball may be corrupt.
echo Delete btcpc-image.tar.gz and try again.
goto END

:BAD_NAME
echo [ERROR] Username cannot be empty.
goto END

:NO_START
echo [ERROR] docker compose up failed. See output above.
goto END

:END
echo.
echo ---------------------------------------------------------------
echo Press any key to close this window.
pause >nul
endlocal
