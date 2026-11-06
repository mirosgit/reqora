# Build Reqora (release) and install it for the current user:
#   %LOCALAPPDATA%\Programs\Reqora\reqora.exe plus a Start menu shortcut.
#
#   .\install.ps1              build + install
#   .\install.ps1 -NoBuild     install an already built target\release\reqora.exe
#
# If running scripts is disabled on your machine:
#   powershell -ExecutionPolicy Bypass -File .\install.ps1
param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$bin = 'target\release\reqora.exe'
$dest = Join-Path $env:LOCALAPPDATA 'Programs\Reqora'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Reqora.lnk'

if (-not $NoBuild) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'cargo not found. Install Rust first: https://rustup.rs'
    }
    # Use half of the CPU cores at low priority so the desktop stays responsive.
    $jobs = if ($env:JOBS) { $env:JOBS } else { [Math]::Max(1, [Math]::Floor([Environment]::ProcessorCount / 2)) }
    Write-Host "==> Building release binary ($jobs jobs, low priority)..."
    # Child processes inherit the below-normal priority class of this shell.
    $self = Get-Process -Id $PID
    $priority = $self.PriorityClass
    $self.PriorityClass = 'BelowNormal'
    try {
        cargo build --release -j $jobs
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit code $LASTEXITCODE)" }
    } finally {
        $self.PriorityClass = $priority
    }
}
if (-not (Test-Path $bin)) { throw "Binary $bin not found - build first." }

Write-Host "==> Installing into $dest"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
try {
    Copy-Item $bin (Join-Path $dest 'reqora.exe') -Force
} catch {
    throw "Could not replace $dest\reqora.exe. Close Reqora and run the script again. ($_)"
}
$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($shortcut)
$link.TargetPath = Join-Path $dest 'reqora.exe'
$link.WorkingDirectory = $dest
$link.Description = 'Fast, lightweight open-source API client'
$link.Save()

Write-Host "==> Done. Launch Reqora from the Start menu or run: $dest\reqora.exe"
