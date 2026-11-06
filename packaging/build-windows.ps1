# Build a portable .zip from the release binary. Runs on Windows, after `cargo build --release`:
#   .\packaging\build-windows.ps1   -> dist\reqora_<version>_windows_<arch>.zip
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$bin = 'target\release\reqora.exe'
if (-not (Test-Path $bin)) { throw 'Build first: cargo build --release' }
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }

$stage = Join-Path ([IO.Path]::GetTempPath()) ('reqora-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    Copy-Item -Path $bin, LICENSE, README.md -Destination $stage
    New-Item -ItemType Directory -Force -Path dist | Out-Null
    $out = "dist\reqora_${version}_windows_$arch.zip"
    Remove-Item $out -ErrorAction SilentlyContinue
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $out
    Write-Host "Built $out"
} finally {
    Remove-Item -Recurse -Force $stage
}
