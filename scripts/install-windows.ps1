<#
.SYNOPSIS
  Install wp-executor as a Windows service via sc.exe.

.DESCRIPTION
  Registers the wp-executor binary as an auto-start Windows service that
  runs in the background. Must be run from an elevated PowerShell session.

.PARAMETER BinaryPath
  Absolute path to the wp-executor.exe binary. Defaults to .\target\release\wp-executor.exe
  if it exists, otherwise to the binary on PATH.

.PARAMETER ConfigPath
  Absolute path to the config.toml file. Defaults to %ProgramData%\wp-executor\config.toml.

.PARAMETER ServiceName
  Service name used by sc.exe. Defaults to "wp-executor".

.PARAMETER DisplayName
  Display name shown in services.msc. Defaults to "Setyenv Workflow Executor".

.EXAMPLE
  .\install-windows.ps1
  .\install-windows.ps1 -BinaryPath "C:\Program Files\wp-executor\wp-executor.exe"
#>
[CmdletBinding()]
param(
    [string]$BinaryPath,
    [string]$ConfigPath = (Join-Path $env:ProgramData "wp-executor\config.toml"),
    [string]$ServiceName = "wp-executor",
    [string]$DisplayName = "Setyenv Workflow Executor"
)

$ErrorActionPreference = "Stop"

function Assert-Elevated {
    $current = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($current)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "This script must be run from an elevated PowerShell session (Run as Administrator)."
    }
}

Assert-Elevated

if (-not $BinaryPath) {
    $local = Join-Path (Get-Location) "target\release\wp-executor.exe"
    if (Test-Path $local) {
        $BinaryPath = (Resolve-Path $local).Path
    } else {
        $cmd = Get-Command wp-executor -ErrorAction SilentlyContinue
        if ($cmd) {
            $BinaryPath = $cmd.Path
        } else {
            throw "wp-executor.exe not found. Pass -BinaryPath, place the binary in PATH, or build with 'cargo build --release'."
        }
    }
}

if (-not (Test-Path $BinaryPath)) {
    throw "Binary not found at $BinaryPath"
}

# Make sure the config dir + a stub config exist (service won't start without a real one).
$configDir = Split-Path -Parent $ConfigPath
if (-not (Test-Path $configDir)) {
    New-Item -ItemType Directory -Path $configDir -Force | Out-Null
}
if (-not (Test-Path $ConfigPath)) {
    Write-Warning "Config not found at $ConfigPath. Writing template — fill in base_url and bearer_token before starting the service."
    @"
# wp-executor config.toml — fill in the secrets BEFORE starting the service.
base_url = "https://CHANGE-ME.example.com"
bearer_token = "pfw_worker_<id>_<secret>"

# Optional (defaults shown):
# namespace = "wp-pfworkflow/v1"
# max_jobs_per_claim = 5
# lease_seconds = 60
# heartbeat_interval_seconds = 15
# idle_poll_seconds = 5
# default_job_timeout_seconds = 300
# sign_requests = true
# allowed_capabilities = ["shell.run", "fs.read", "fs.write", "fs.list", "http.request", "system.info"]
"@ | Set-Content -Path $ConfigPath -Encoding UTF8
    icacls $ConfigPath /inheritance:r /grant "BUILTIN\Administrators:(F)" "NT AUTHORITY\SYSTEM:(F)" | Out-Null
}

# Remove any existing service with the same name (idempotent install).
$existing = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
if ($existing) {
    if ($existing.Status -ne "Stopped") {
        Stop-Service -Name $ServiceName -Force -ErrorAction SilentlyContinue
    }
    & sc.exe delete $ServiceName | Out-Null
    Start-Sleep -Seconds 1
}

# binPath= must include both the exe and the args; quote the exe.
$binArgs = "`"$BinaryPath`" --config `"$ConfigPath`" run"
$createOutput = & sc.exe create $ServiceName binPath= $binArgs DisplayName= $DisplayName start= auto 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "sc.exe create failed: $createOutput"
}

& sc.exe description $ServiceName "Setyenv Workflow remote executor. Pulls jobs from a wp-pfworkflow site over the documented REST contract and runs them locally." | Out-Null

# Configure recovery: restart on failure with 5s/30s/60s backoff.
& sc.exe failure $ServiceName reset= 86400 actions= restart/5000/restart/30000/restart/60000 | Out-Null

Start-Service -Name $ServiceName

Write-Host ""
Write-Host "Installed Windows service '$ServiceName'."
Write-Host "  binary : $BinaryPath"
Write-Host "  config : $ConfigPath"
Write-Host "  status : $(Get-Service -Name $ServiceName | Select-Object -ExpandProperty Status)"
Write-Host ""
Write-Host "Useful commands:"
Write-Host "  Get-Service $ServiceName"
Write-Host "  Restart-Service $ServiceName"
Write-Host "  Get-EventLog -LogName Application -Source $ServiceName -Newest 50"
