# HONE Birds camera listener installer for Windows 10/11 (no admin needed).
#   irm https://honemesh.net/app/listener/install.ps1 | iex
#   $env:HONE_REF='invitecode'; irm https://honemesh.net/app/listener/install.ps1 | iex
# Trust: version.json must carry a valid ed25519 signature (version.json.sig) by the pinned
# HONE listener release key, and the download must match its sha256 in that signed manifest,
# before anything is installed. HONE_VERIFIED_MANIFEST may point to a version.json a trusted
# caller (the HONE MCP) already verified. HONE_NONINTERACTIVE=1: never prompts.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# Windows PowerShell started from PowerShell 7 inherits a PSModulePath it cannot use.
if ($PSVersionTable.PSVersion.Major -lt 6) { $env:PSModulePath = [Environment]::GetEnvironmentVariable('PSModulePath', 'Machine') }
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$Base = 'https://honemesh.net/app/listener'
$ReleasePub = '27ef54588674c3391e660056d873b89136e75125b5a433de0747191baa94a10d'
$TaskName = 'HONE Listener'

function Fail($m) { Write-Host "HONE listener: $m" -ForegroundColor Red; throw "HONE listener: $m" }
function Hex($s) { $b = New-Object byte[] ($s.Length / 2); for ($i = 0; $i -lt $b.Length; $i++) { $b[$i] = [Convert]::ToByte($s.Substring(2 * $i, 2), 16) }; return ,$b }

# -- Ed25519 verify (RFC 8032 section 6 reference algorithm; .NET has no Ed25519) --
Add-Type -AssemblyName System.Numerics
$script:P = [Numerics.BigInteger]::Pow(2, 255) - 19
$script:Q = [Numerics.BigInteger]::Pow(2, 252) + [Numerics.BigInteger]::Parse('27742317777372353535851937790883648493')
function M([Numerics.BigInteger]$x) { $r = [Numerics.BigInteger]::Remainder($x, $script:P); if ($r.Sign -lt 0) { $r += $script:P }; return $r }
function Inv([Numerics.BigInteger]$x) { return [Numerics.BigInteger]::ModPow((M $x), $script:P - 2, $script:P) }
$script:D = M ([Numerics.BigInteger](-121665) * (Inv 121666))
$script:I = [Numerics.BigInteger]::ModPow(2, ($script:P - 1) / 4, $script:P)
function LE([byte[]]$b) { return New-Object Numerics.BigInteger (,[byte[]]($b + [byte]0)) }
function PAdd($p1, $p2) {
  $ta = M (($p1[1] - $p1[0]) * ($p2[1] - $p2[0])); $tb = M (($p1[1] + $p1[0]) * ($p2[1] + $p2[0]))
  $tc = M (2 * $p1[3] * $p2[3] * $script:D); $td = M (2 * $p1[2] * $p2[2])
  $te = $tb - $ta; $tf = $td - $tc; $tg = $td + $tc; $th = $tb + $ta
  return @((M ($te * $tf)), (M ($tg * $th)), (M ($tf * $tg)), (M ($te * $th)))
}
function PMul([Numerics.BigInteger]$s, $p) {
  $r = @([Numerics.BigInteger]::Zero, [Numerics.BigInteger]::One, [Numerics.BigInteger]::One, [Numerics.BigInteger]::Zero)
  while ($s.Sign -gt 0) { if (-not $s.IsEven) { $r = PAdd $r $p }; $p = PAdd $p $p; $s = [Numerics.BigInteger]::Divide($s, 2) }
  return $r
}
function RecoverX([Numerics.BigInteger]$y, [int]$sign) {
  if ($y -ge $script:P) { return $null }
  $x2 = M (($y * $y - 1) * (Inv ($script:D * $y * $y + 1)))
  if ($x2.IsZero) { if ($sign) { return $null } else { return [Numerics.BigInteger]::Zero } }
  $x = [Numerics.BigInteger]::ModPow($x2, ($script:P + 3) / 8, $script:P)
  if (-not (M ($x * $x - $x2)).IsZero) { $x = M ($x * $script:I) }
  if (-not (M ($x * $x - $x2)).IsZero) { return $null }
  if ([int](-not $x.IsEven) -ne $sign) { $x = $script:P - $x }
  return $x
}
function Decompress([byte[]]$s) {
  $y = LE $s; $sign = [int]([Numerics.BigInteger]::Divide($y, [Numerics.BigInteger]::Pow(2, 255)))
  $y = [Numerics.BigInteger]::Remainder($y, [Numerics.BigInteger]::Pow(2, 255))
  $x = RecoverX $y $sign; if ($null -eq $x) { return $null }
  return @($x, $y, [Numerics.BigInteger]::One, (M ($x * $y)))
}
function Test-Ed25519([byte[]]$pub, [byte[]]$msg, [byte[]]$sig) {
  if ($pub.Length -ne 32 -or $sig.Length -ne 64) { return $false }
  $ptA = Decompress $pub; if ($null -eq $ptA) { return $false }
  $Rs = [byte[]]$sig[0..31]; $ptR = Decompress $Rs; if ($null -eq $ptR) { return $false }
  $sc = LE ([byte[]]$sig[32..63]); if ($sc -ge $script:Q) { return $false }
  $sha = [Security.Cryptography.SHA512]::Create()
  $h = [Numerics.BigInteger]::Remainder((LE $sha.ComputeHash([byte[]]($Rs + $pub + $msg))), $script:Q)
  $gy = M (4 * (Inv 5)); $gx = RecoverX $gy 0; $G = @($gx, $gy, [Numerics.BigInteger]::One, (M ($gx * $gy)))
  $sB = PMul $sc $G; $hA = PAdd $ptR (PMul $h $ptA)
  return ((M ($sB[0] * $hA[2] - $hA[0] * $sB[2])).IsZero -and (M ($sB[1] * $hA[2] - $hA[1] * $sB[2])).IsZero)
}

if ($env:HONE_SELFTEST_ED25519 -eq '1') {
  $ok = Test-Ed25519 (Hex 'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a') ([byte[]]@()) (Hex 'e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b')
  $bad = Test-Ed25519 (Hex 'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a') ([byte[]]@(1)) (Hex 'e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b')
  Write-Host "ed25519 selftest: valid=$ok tampered=$bad"; return
}

# -- manifest --
$Tmp = Join-Path ([IO.Path]::GetTempPath()) ('hone-listener-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
  if ($env:HONE_VERIFIED_MANIFEST -and (Test-Path $env:HONE_VERIFIED_MANIFEST)) {
    $manBytes = [IO.File]::ReadAllBytes($env:HONE_VERIFIED_MANIFEST)
    Write-Host 'Using the release manifest that was already checked against the HONE release key.'
  } else {
    try { $manBytes = (Invoke-WebRequest -UseBasicParsing "$Base/version.json").Content } catch { Fail "couldn't reach honemesh.net; check your internet and try again" }
    if ($manBytes -is [string]) { $manBytes = [Text.Encoding]::UTF8.GetBytes($manBytes) }
    try { $sigTxt = (Invoke-WebRequest -UseBasicParsing "$Base/version.json.sig").Content } catch { Fail "the release isn't signed yet, so nothing was installed" }
    if ($sigTxt -isnot [string]) { $sigTxt = [Text.Encoding]::ASCII.GetString($sigTxt) }
    $sigTxt = $sigTxt.Trim()
    $sig = if ($sigTxt -match '^[0-9a-fA-F]{128}$') { Hex $sigTxt } else { [Convert]::FromBase64String($sigTxt) }
    Write-Host 'Checking the release signature...'
    if (-not (Test-Ed25519 (Hex $ReleasePub) $manBytes $sig)) { Fail "version.json's signature didn't match the HONE release key; nothing was installed" }
    Write-Host 'Checked: the release is signed by HONE.'
  }
  $man = [Text.Encoding]::UTF8.GetString($manBytes) | ConvertFrom-Json
  if ($man.name -ne 'hone-listener') { Fail "the release manifest isn't for the HONE listener" }
  $ver = [string]$man.version
  $zip = "hone-listener-$ver-windows-x86_64.zip"
  $want = [string]$man.files.$zip.sha256
  if ($want -notmatch '^[0-9a-f]{64}$') { Fail "this release isn't available for Windows yet" }
  Write-Host "Downloading the HONE listener $ver..."
  $zipPath = Join-Path $Tmp $zip
  Invoke-WebRequest -UseBasicParsing "$Base/$zip" -OutFile $zipPath
  $fs = [IO.File]::OpenRead($zipPath); try { $got = (([Security.Cryptography.SHA256]::Create().ComputeHash($fs)) | ForEach-Object { $_.ToString('x2') }) -join '' } finally { $fs.Dispose() }
  if ($got -ne $want) { Fail "the download didn't match its signed checksum; nothing was installed" }
  Write-Host "Checked: $zip matches the signed checksum."

  # -- install (per user) --
  $State = Join-Path $env:LOCALAPPDATA 'HONE\Listener'
  $App = Join-Path $State 'app'
  New-Item -ItemType Directory -Force -Path $App | Out-Null
  Get-Process -Name 'hone-listener' -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$App*" } | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 500
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $za = [IO.Compression.ZipFile]::OpenRead($zipPath)
  try { foreach ($en in $za.Entries) { if ($en.Name) { $dst = Join-Path $App $en.FullName; New-Item -ItemType Directory -Force (Split-Path -Parent $dst) | Out-Null; [IO.Compression.ZipFileExtensions]::ExtractToFile($en, $dst, $true) } } } finally { $za.Dispose() }
  $exe = Join-Path $App 'hone-listener.exe'
  $ref = ([string]$env:HONE_REF).ToLower() -replace '[^a-z0-9._-]', ''
  if ($ref.Length -gt 32) { $ref = $ref.Substring(0, 32) }
  if ($ref) { Set-Content -Path (Join-Path $State 'ref') -Value $ref -Encoding ASCII }

  # start at sign-in: a per-user scheduled task (no admin), else the Run key
  $started = $false
  try {
    $act = New-ScheduledTaskAction -Execute $exe -Argument 'tray'
    $trg = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
    $set = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
    $pr = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
    Register-ScheduledTask -TaskName $TaskName -Action $act -Trigger $trg -Settings $set -Principal $pr -Force | Out-Null
    $started = $true
  } catch {
    Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'HONE Listener' -Value "`"$exe`" tray"
  }
  $lnk = Join-Path ([Environment]::GetFolderPath('Programs')) 'HONE Birds Listener.lnk'
  $sh = New-Object -ComObject WScript.Shell; $s = $sh.CreateShortcut($lnk); $s.TargetPath = $exe; $s.Arguments = 'open'; $s.Description = "Hear who's singing at your place"; $s.Save()
  if ($started) { Start-ScheduledTask -TaskName $TaskName } else { Start-Process -FilePath $exe -ArgumentList 'tray' -WindowStyle Hidden }
  Write-Host ''
  Write-Host 'The HONE listener is installed and starts when you sign in to Windows.'
  Write-Host 'Its setup page opens in your browser. It also sits in the system tray (near the clock).'
} finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
