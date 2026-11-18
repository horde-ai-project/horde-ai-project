# Start the Horde API binary (localhost operator UI at /ui/).
# Loads .env from the repo root when present. Does not print secrets.
param(
    [switch]$OpenBrowser
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$PidFile = if ($env:HORDE_PID_FILE) { $env:HORDE_PID_FILE } else { Join-Path $Root "horde.pid" }
$LogFile = if ($env:HORDE_LOG_FILE) { $env:HORDE_LOG_FILE } else { Join-Path $Root "horde.log" }

function Import-DotEnv {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return }
    Get-Content $Path | ForEach-Object {
        $line = $_.Trim()
        if (-not $line -or $line.StartsWith("#")) { return }
        $idx = $line.IndexOf("=")
        if ($idx -lt 1) { return }
        $name = $line.Substring(0, $idx).Trim()
        $value = $line.Substring($idx + 1).Trim()
        if (($value.StartsWith('"') -and $value.EndsWith('"')) -or ($value.StartsWith("'") -and $value.EndsWith("'"))) {
            $value = $value.Substring(1, $value.Length - 2)
        }
        Set-Item -Path "Env:$name" -Value $value
    }
}

Import-DotEnv (Join-Path $Root ".env")

if (-not $env:HORDE_PASSPHRASE) {
    Write-Error "HORDE_PASSPHRASE is required. Copy .env.example to .env and set it."
}

if (-not $env:HORDE_DATA_DIR) { $env:HORDE_DATA_DIR = "./horde-data" }
if (-not $env:HORDE_BIND) { $env:HORDE_BIND = "127.0.0.1:8787" }
if (-not $env:HORDE_UI_DIR) { $env:HORDE_UI_DIR = (Join-Path $Root "ui") }

if (Test-Path $PidFile) {
    $old = (Get-Content $PidFile -ErrorAction SilentlyContinue | Select-Object -First 1)
    if ($old) {
        $proc = Get-Process -Id ([int]$old) -ErrorAction SilentlyContinue
        if ($proc) {
            Write-Error "Horde already running (pid $old). Stop with scripts/horde-stop.ps1"
        }
    }
    Remove-Item $PidFile -Force -ErrorAction SilentlyContinue
}

$release = Join-Path $Root "target/release/horde.exe"
$debug = Join-Path $Root "target/debug/horde.exe"
$bin = $null
if (Test-Path $release) { $bin = $release }
elseif (Test-Path $debug) { $bin = $debug }
else {
    Write-Host "Building horde-api (debug)…"
    cargo build -p horde-api
    $bin = $debug
}

$proc = Start-Process -FilePath $bin -WorkingDirectory $Root -RedirectStandardOutput $LogFile -RedirectStandardError $LogFile -PassThru -WindowStyle Hidden
Set-Content -Path $PidFile -Value $proc.Id
Write-Host "Started horde (pid $($proc.Id))"
Write-Host "  bind:  http://$($env:HORDE_BIND)/"
Write-Host "  UI:    http://$($env:HORDE_BIND)/ui/"
Write-Host "  log:   $LogFile"
Write-Host "Copy the studio API key from the log into the operator UI."

$shouldOpen = $OpenBrowser -or $env:HORDE_OPEN_BROWSER -eq "1" -or $env:HORDE_OPEN_BROWSER -eq "true"
if ($shouldOpen) {
    Start-Process "http://$($env:HORDE_BIND)/ui/"
}
