# Windows build (run on beastly): exe with tray, zip for install.ps1, and the Store MSIX.
#   powershell -File build-windows.ps1 -Tfl C:\hl\tfl\tensorflowlite_c.dll -Assets <dir> -Ffmpeg <ffmpeg.exe> -Out <dir>
# Store values (from Partner Center > Product identity) may be passed now or pasted later into
# AppxManifest.xml: -IdentityName, -Publisher (CN=...), -PublisherDisplayName, -AppName.
# Without them the MSIX builds with obvious placeholders (valid syntax, NOT submittable).
param([string]$Tfl, [string]$Assets, [string]$Ffmpeg, [string]$Out,
      [string]$IdentityName = 'HONE.BirdsListener.Placeholder', [string]$Publisher = 'CN=PLACEHOLDER-PUBLISHER-ID',
      [string]$PublisherDisplayName = 'PLACEHOLDER Publisher', [string]$AppName = 'HONE Birds Listener')
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$crate = Resolve-Path "$here\..\.."
$ver = (Select-String -Path "$crate\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$env:CARGO_TARGET_DIR = 'C:\hl\target'
Push-Location $crate
& "$env:USERPROFILE\.cargo\bin\cargo.exe" build --release --locked --features tray
if ($LASTEXITCODE) { throw 'cargo build failed' }
Pop-Location
$exe = 'C:\hl\target\release\hone-listener.exe'
New-Item -ItemType Directory -Force $Out | Out-Null
$stage = Join-Path $Out 'stage'; Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue; New-Item -ItemType Directory $stage | Out-Null
Copy-Item $exe, $Tfl, $Ffmpeg $stage
foreach ($f in 'perch_v2_8bit.tflite','class_names.json','names_en.json','NOTICE.txt') { Copy-Item (Join-Path $Assets $f) $stage }
Copy-Item $exe (Join-Path $Out "hone-listener-$ver-windows-x86_64.exe")
$zip = Join-Path $Out "hone-listener-$ver-windows-x86_64.zip"
Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path "$stage\*" -DestinationPath $zip
# MSIX
$m = Join-Path $Out 'msix'; Remove-Item -Recurse -Force $m -ErrorAction SilentlyContinue
Copy-Item -Recurse $stage $m
Copy-Item -Recurse "$here\Assets" "$m\Assets"
(Get-Content "$here\AppxManifest.xml" -Raw).Replace('__VERSION__', $ver).Replace('__PACKAGE_IDENTITY_NAME__', $IdentityName).Replace('__PUBLISHER_CN__', $Publisher).Replace('__PUBLISHER_DISPLAY_NAME__', $PublisherDisplayName).Replace('__APP_DISPLAY_NAME__', $AppName) | Set-Content -Encoding UTF8 "$m\AppxManifest.xml"
$makeappx = (Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\makeappx.exe' | Sort-Object FullName | Select-Object -Last 1).FullName
$msix = Join-Path $Out "HONE-Birds-Listener-$ver-x64.msix"
& $makeappx pack /o /d $m /p $msix
if ($LASTEXITCODE) { throw 'makeappx failed' }
Get-ChildItem $Out -File | Select-Object Name, Length
