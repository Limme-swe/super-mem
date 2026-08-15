#requires -Version 5.1
<#!
.SYNOPSIS
Removes the supermem executable for the current user while preserving memories by default.
#>
[CmdletBinding()]
param(
    [Parameter()]
    [string]$InstallDir = $(if ($env:SUPER_MEM_INSTALL_DIR) { $env:SUPER_MEM_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\super-mem\bin' }),

    [Parameter()]
    [switch]$PurgeData,

    [Parameter()]
    [switch]$Yes,

    [Parameter()]
    [switch]$KeepPath,

    [Parameter()]
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3.0

if ($PurgeData -and -not $Yes) {
    throw '-PurgeData requires -Yes because memories cannot be recovered.'
}

$InstallDir = [IO.Path]::GetFullPath($InstallDir)
$Binary = Join-Path $InstallDir 'supermem.exe'
$DataDir = Join-Path $env:LOCALAPPDATA 'super-mem'

function Remove-Safely {
    param(
        [Parameter(Mandatory)][string]$LiteralPath,
        [Parameter(Mandatory)][string]$Label,
        [switch]$Recurse
    )
    if (-not (Test-Path -LiteralPath $LiteralPath)) {
        Write-Host "$Label not found: $LiteralPath"
        return
    }
    if ($DryRun) {
        Write-Host "Would remove $Label`: $LiteralPath"
        return
    }
    Remove-Item -LiteralPath $LiteralPath -Force -Recurse:$Recurse
    Write-Host "Removed $Label`: $LiteralPath"
}

Remove-Safely -LiteralPath $Binary -Label 'binary'

if (-not $KeepPath) {
    $UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $Entries = @($UserPath -split ';' | Where-Object { $_ })
    $Filtered = @($Entries | Where-Object {
        try { [IO.Path]::GetFullPath($_).TrimEnd('\') -ine $InstallDir.TrimEnd('\') } catch { $true }
    })
    if ($Filtered.Count -ne $Entries.Count) {
        if ($DryRun) {
            Write-Host "Would remove install directory from the user PATH: $InstallDir"
        } else {
            [Environment]::SetEnvironmentVariable('Path', ($Filtered -join ';'), 'User')
            Write-Host "Removed install directory from the user PATH: $InstallDir"
        }
    }
}

if ($PurgeData) {
    $ResolvedData = [IO.Path]::GetFullPath($DataDir).TrimEnd('\')
    $ResolvedLocal = [IO.Path]::GetFullPath($env:LOCALAPPDATA).TrimEnd('\')
    $ExpectedPrefix = $ResolvedLocal + [IO.Path]::DirectorySeparatorChar
    if ($ResolvedData -eq $ResolvedLocal -or -not $ResolvedData.StartsWith($ExpectedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing unsafe data directory: $ResolvedData"
    }
    Remove-Safely -LiteralPath $ResolvedData -Label 'data directory' -Recurse
} else {
    Write-Host "Memory data was preserved at: $DataDir"
    if ($env:SUPER_MEM_DB) {
        Write-Host "Custom SUPER_MEM_DB was not removed: $env:SUPER_MEM_DB"
    }
}
