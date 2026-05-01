@echo off
:: BTCPC Node — Windows launcher
:: Double-click this file. Requires: Docker Desktop + Ollama installed on Windows.

:: Make Ollama listen on all interfaces so the Docker container can reach it.
:: This only affects the current session; set it permanently in System env vars if preferred.
set OLLAMA_HOST=0.0.0.0

:: Restart Ollama with the new binding if it is already running.
tasklist /fi "imagename eq ollama.exe" 2>nul | find /i "ollama.exe" >nul
if not errorlevel 1 (
    echo Restarting Ollama with OLLAMA_HOST=0.0.0.0 ...
    taskkill /im ollama.exe /f >nul 2>&1
    timeout /t 2 /nobreak >nul
)
start "" ollama serve

echo Waiting for Ollama to start...
timeout /t 3 /nobreak >nul

echo Starting BTCPC node...
docker compose up -d

echo.
echo Done. Node is running. Check logs with: docker compose logs -f btcpc-node
pause
