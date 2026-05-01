<#
.SYNOPSIS
  Uninstall the wp-executor Windows service installed by install-windows.ps1.

.PARAMETER ServiceName
  Service name. Defaults to "wp-executor".
#>
[CmdletBinding()]
param(
    [string]$ServiceName = "wp-executor"
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

$svc = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
if (-not $svc) {
    Write-Host "Service '$ServiceName' is not installed."
    exit 0
}

if ($svc.Status -ne "Stopped") {
    Stop-Service -Name $ServiceName -Force -ErrorAction SilentlyContinue
}
& sc.exe delete $ServiceName | Out-Null
Write-Host "Removed service '$ServiceName'."
Write-Host "Note: binary and config file (if any) were left in place."
