# BTCPC Node — Windows Native Setup Script
# Run as Administrator in PowerShell
# Works for both native Windows installs and WSL2 setups

param(
    [switch]$WSL,       # Pass -WSL if the node is running inside WSL2
    [int]$Port = 6942,
    [int]$TestnetPort = 6943
)

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "BTCPC Node — Windows Setup" -ForegroundColor Cyan
Write-Host "==========================" -ForegroundColor Cyan
Write-Host ""

# Detect environment
if ($WSL) {
    Write-Host "Mode: WSL2 (node running inside WSL, Ollama on Windows)" -ForegroundColor Yellow
    Write-Host ""

    # Get WSL IP
    try {
        $wslIp = (wsl hostname -I).Trim().Split()[0]
        Write-Host "WSL IP detected: $wslIp" -ForegroundColor Green
    } catch {
        Write-Host "ERROR: Could not get WSL IP. Is WSL running?" -ForegroundColor Red
        exit 1
    }

    # Add port proxies (Windows -> WSL)
    Write-Host "Adding port proxies (Windows -> WSL)..."
    netsh interface portproxy delete v4tov4 listenport=$Port listenaddress=0.0.0.0 2>$null
    netsh interface portproxy delete v4tov4 listenport=$TestnetPort listenaddress=0.0.0.0 2>$null
    netsh interface portproxy add v4tov4 listenport=$Port listenaddress=0.0.0.0 connectport=$Port connectaddress=$wslIp
    netsh interface portproxy add v4tov4 listenport=$TestnetPort listenaddress=0.0.0.0 connectport=$TestnetPort connectaddress=$wslIp
    Write-Host "Port proxies set." -ForegroundColor Green

    Write-Host ""
    Write-Host "Port proxy table:" -ForegroundColor Cyan
    netsh interface portproxy show all

    Write-Host ""
    Write-Host "IMPORTANT: In your WSL node, set OLLAMA_URL to the Windows host IP:" -ForegroundColor Yellow
    $windowsIp = (Get-NetIPAddress -AddressFamily IPv4 |
        Where-Object { $_.InterfaceAlias -notlike "*Loopback*" -and $_.InterfaceAlias -notlike "*WSL*" } |
        Select-Object -First 1).IPAddress
    Write-Host "  export OLLAMA_URL=http://${windowsIp}:11434" -ForegroundColor White

} else {
    Write-Host "Mode: Native Windows (node and Ollama both running on Windows)" -ForegroundColor Yellow
    Write-Host "No port proxy needed — Ollama is already at localhost:11434." -ForegroundColor Green
    Write-Host ""
}

# Firewall rules (needed for both modes)
Write-Host "Adding Windows Firewall rules for P2P port $Port..."

# Remove existing rules with same name to avoid duplicates
Remove-NetFirewallRule -DisplayName "BTCPC P2P $Port" -ErrorAction SilentlyContinue
Remove-NetFirewallRule -DisplayName "BTCPC P2P $TestnetPort" -ErrorAction SilentlyContinue

New-NetFirewallRule -DisplayName "BTCPC P2P $Port" `
    -Direction Inbound -Protocol TCP -LocalPort $Port -Action Allow | Out-Null
New-NetFirewallRule -DisplayName "BTCPC P2P $TestnetPort" `
    -Direction Inbound -Protocol TCP -LocalPort $TestnetPort -Action Allow | Out-Null

Write-Host "Firewall rules added." -ForegroundColor Green

# Show LAN IP for router port forwarding
Write-Host ""
Write-Host "Your LAN IP (for router port forwarding if needed):" -ForegroundColor Cyan
Get-NetIPAddress -AddressFamily IPv4 |
    Where-Object { $_.InterfaceAlias -notlike "*Loopback*" -and $_.InterfaceAlias -notlike "*WSL*" } |
    Select-Object IPAddress, InterfaceAlias |
    Format-Table -AutoSize

Write-Host "If peers outside your home network need to connect, forward port $Port TCP" -ForegroundColor Yellow
Write-Host "from your router to the LAN IP above." -ForegroundColor Yellow

Write-Host ""
Write-Host "Setup complete." -ForegroundColor Green
Write-Host "Verify your node is reachable: curl http://localhost:4242/api/node/info" -ForegroundColor Cyan
Write-Host ""
