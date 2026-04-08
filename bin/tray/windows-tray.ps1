# BTCPC Windows System Tray
# Uses .NET NotifyIcon for native Windows tray support
# No dependencies — PowerShell + .NET are built into Windows
#
# Run: powershell -ExecutionPolicy Bypass -File bin\tray\windows-tray.ps1
# Or: .\bin\btcpc-tray

$ExplorerUrl = "http://localhost:4242"
$SettingsUrl = "$ExplorerUrl/settings"

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

# Create the tray icon
$notifyIcon = New-Object System.Windows.Forms.NotifyIcon
$notifyIcon.Text = "BTCPC Miner"
$notifyIcon.Visible = $true

# Use a simple icon (orange circle)
$bitmap = New-Object System.Drawing.Bitmap(16, 16)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$brush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(247, 147, 26))
$graphics.FillEllipse($brush, 1, 1, 14, 14)
$notifyIcon.Icon = [System.Drawing.Icon]::FromHandle($bitmap.GetHicon())

# Context menu
$contextMenu = New-Object System.Windows.Forms.ContextMenuStrip

$settingsItem = $contextMenu.Items.Add("Settings")
$settingsItem.Add_Click({ Start-Process $SettingsUrl })

$explorerItem = $contextMenu.Items.Add("Explorer")
$explorerItem.Add_Click({ Start-Process $ExplorerUrl })

$contextMenu.Items.Add("-")

$pauseItem = $contextMenu.Items.Add("Pause Mining")
$pauseItem.Add_Click({
    Invoke-WebRequest -Uri $SettingsUrl -Method POST -Body "action=pause" -UseBasicParsing | Out-Null
    $notifyIcon.ShowBalloonTip(3000, "BTCPC", "Mining paused", [System.Windows.Forms.ToolTipIcon]::Info)
})

$resumeItem = $contextMenu.Items.Add("Resume Mining")
$resumeItem.Add_Click({
    Invoke-WebRequest -Uri $SettingsUrl -Method POST -Body "action=auto" -UseBasicParsing | Out-Null
    $notifyIcon.ShowBalloonTip(3000, "BTCPC", "Mining resumed (auto)", [System.Windows.Forms.ToolTipIcon]::Info)
})

$contextMenu.Items.Add("-")

$quitItem = $contextMenu.Items.Add("Quit")
$quitItem.Add_Click({
    $notifyIcon.Visible = $false
    [System.Windows.Forms.Application]::Exit()
})

$notifyIcon.ContextMenuStrip = $contextMenu

# Double-click opens settings
$notifyIcon.Add_DoubleClick({ Start-Process $SettingsUrl })

# Show startup balloon
$notifyIcon.ShowBalloonTip(5000, "BTCPC Miner", "Mining active. Right-click for options.", [System.Windows.Forms.ToolTipIcon]::Info)

# Timer for status updates and update checks
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 30000  # 30 seconds

$timer.Add_Tick({
    try {
        $stats = Invoke-RestMethod -Uri "$ExplorerUrl/api/stats" -TimeoutSec 5
        $notifyIcon.Text = "BTCPC - Epoch $($stats.current_epoch) | $([math]::Round($stats.total_mined)) mined"

        # Check for updates
        $headers = @{ "x-bot-key" = $env:BOT_API_KEY }
        $update = Invoke-RestMethod -Uri "http://localhost:3100/api/bot/update-status" -Headers $headers -TimeoutSec 5
        if ($update.pending) {
            $notifyIcon.ShowBalloonTip(10000, "BTCPC Update Available",
                "v$($update.update.version) ready. Update to keep earning.",
                [System.Windows.Forms.ToolTipIcon]::Warning)
        }
    } catch {
        $notifyIcon.Text = "BTCPC - Offline"
    }
})

$timer.Start()

# Run the message loop (keeps tray icon alive)
[System.Windows.Forms.Application]::Run()
