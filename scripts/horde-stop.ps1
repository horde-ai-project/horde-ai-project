# Stop a Horde process started by horde-start.ps1 / horde-start.bat.
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$PidFile = if ($env:HORDE_PID_FILE) { $env:HORDE_PID_FILE } else { Join-Path $Root "horde.pid" }

if (-not (Test-Path $PidFile)) {
    Write-Host "No pid file at $PidFile"
    exit 0
}

$pidText = (Get-Content $PidFile -ErrorAction SilentlyContinue | Select-Object -First 1)
if (-not $pidText) {
    Remove-Item $PidFile -Force -ErrorAction SilentlyContinue
    Write-Host "Empty pid file removed."
    exit 0
}

$procId = [int]$pidText
$proc = Get-Process -Id $procId -ErrorAction SilentlyContinue
if ($proc) {
    Stop-Process -Id $procId -Force
    Write-Host "Stopped horde (pid $procId)"
} else {
    Write-Host "Process $procId not running"
}
Remove-Item $PidFile -Force -ErrorAction SilentlyContinue
