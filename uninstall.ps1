# Remove Reqora installed by install.ps1. Your data (collections, history) is kept
# in %APPDATA%\Reqora unless you pass -Purge.
param([switch]$Purge)
$ErrorActionPreference = 'Stop'

$dest = Join-Path $env:LOCALAPPDATA 'Programs\Reqora'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Reqora.lnk'
$data = Join-Path $env:APPDATA 'Reqora'

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path $dest, $shortcut
if ($Purge) {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -Path $data
    Write-Host 'Removed app and data.'
} else {
    Write-Host "Removed app. Data kept in $data (use -Purge to delete)."
}
