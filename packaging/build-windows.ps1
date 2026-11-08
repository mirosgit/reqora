# Package the release binary for Windows. Runs on Windows, after `cargo build --release`:
#   .\packaging\build-windows.ps1
#     -> dist\reqora_<version>_windows_<arch>.zip      portable: unzip and run
#     -> dist\Reqora-Setup-<version>-x64.exe           installer (needs Inno Setup 6.3+: winget install JRSoftware.InnoSetup)
#   .\packaging\build-windows.ps1 -NoInstaller          only the .zip
param([switch]$NoInstaller)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$bin = 'target\release\reqora.exe'
if (-not (Test-Path $bin)) { throw 'Build first: cargo build --release' }
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }
New-Item -ItemType Directory -Force -Path dist | Out-Null

$stage = Join-Path ([IO.Path]::GetTempPath()) ('reqora-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    Copy-Item -Path $bin, LICENSE, README.md -Destination $stage
    $out = "dist\reqora_${version}_windows_$arch.zip"
    Remove-Item $out -ErrorAction SilentlyContinue
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $out
    Write-Host "Built $out"
} finally {
    Remove-Item -Recurse -Force $stage
}

if ($NoInstaller) { exit 0 }
$iscc = @(
    (Get-Command iscc -ErrorAction SilentlyContinue).Source,
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) {
    Write-Warning 'Inno Setup not found, so no installer was built. Install it with: winget install JRSoftware.InnoSetup'
    exit 0
}
& $iscc /Q "/DAppVersion=$version" packaging\windows\reqora.iss
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed (exit code $LASTEXITCODE)" }
Write-Host "Built dist\Reqora-Setup-$version-x64.exe"
