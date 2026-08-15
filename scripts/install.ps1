#requires -Version 5.1
<#!
.SYNOPSIS
Installs a checksum-verified super-mem release for the current Windows user.
#>
[CmdletBinding()]
param(
    [Parameter()]
    [ValidatePattern('^(latest|v?[0-9A-Za-z._+-]+)$')]
    [string]$Version = $(if ($env:SUPER_MEM_VERSION) { $env:SUPER_MEM_VERSION } else { 'latest' }),

    [Parameter()]
    [string]$InstallDir = $(if ($env:SUPER_MEM_INSTALL_DIR) { $env:SUPER_MEM_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\super-mem\bin' }),

    [Parameter()]
    [switch]$NoPathUpdate,

    [Parameter(DontShow)]
    [string]$DownloadBase = $(if ($env:SUPER_MEM_DOWNLOAD_BASE) { $env:SUPER_MEM_DOWNLOAD_BASE } else { 'https://github.com/Limme-swe/super-mem/releases/download' }),

    [Parameter(DontShow)]
    [string]$ReleaseApi = $(if ($env:SUPER_MEM_RELEASE_API) { $env:SUPER_MEM_RELEASE_API } else { 'https://api.github.com/repos/Limme-swe/super-mem/releases/latest' })
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3.0

function Get-DownloadFile {
    param(
        [Parameter(Mandatory)][string]$Uri,
        [Parameter(Mandatory)][string]$Destination
    )
    $parsed = [Uri]$Uri
    if ($parsed.IsFile) {
        Copy-Item -LiteralPath $parsed.LocalPath -Destination $Destination -Force
        return
    }
    $headers = @{
        Accept = 'application/octet-stream'
        'User-Agent' = 'super-mem-installer'
    }
    Invoke-WebRequest -Uri $Uri -Headers $headers -OutFile $Destination -UseBasicParsing
}

if (-not [Environment]::Is64BitOperatingSystem) {
    throw 'super-mem currently publishes only a Windows x86-64 release.'
}

if ($Version -eq 'latest') {
    $headers = @{
        Accept = 'application/vnd.github+json'
        'User-Agent' = 'super-mem-installer'
    }
    $release = Invoke-RestMethod -Uri $ReleaseApi -Headers $headers -UseBasicParsing
    if (-not $release.tag_name) {
        throw 'Latest release metadata did not contain tag_name.'
    }
    $Tag = [string]$release.tag_name
    $Version = $Tag -replace '^v', ''
} else {
    $Tag = if ($Version.StartsWith('v')) { $Version } else { "v$Version" }
    $Version = $Version -replace '^v', ''
}

if ($Version -notmatch '^[0-9A-Za-z._+-]+$') {
    throw "Unsafe release version: $Version"
}

$Target = 'x86_64-pc-windows-msvc'
$ArchiveName = "super-mem-v$Version-$Target.zip"
$ArchiveRoot = "super-mem-v$Version-$Target"
$Temporary = Join-Path ([IO.Path]::GetTempPath()) ("super-mem-install-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Temporary | Out-Null

try {
    $Archive = Join-Path $Temporary $ArchiveName
    $Checksums = Join-Path $Temporary 'SHA256SUMS'
    Write-Host "Downloading super-mem $Version for $Target..."
    Get-DownloadFile -Uri "$DownloadBase/$Tag/$ArchiveName" -Destination $Archive
    Get-DownloadFile -Uri "$DownloadBase/$Tag/SHA256SUMS" -Destination $Checksums

    $escapedName = [Regex]::Escape($ArchiveName)
    $line = Get-Content -LiteralPath $Checksums | Where-Object { $_ -match "^[0-9a-fA-F]{64}  $escapedName$" } | Select-Object -First 1
    if (-not $line) {
        throw "SHA256SUMS has no entry for $ArchiveName"
    }
    $Expected = ($line -split '\s+')[0].ToLowerInvariant()
    $Actual = (Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) {
        throw "Checksum verification failed for $ArchiveName"
    }

    $Extracted = Join-Path $Temporary 'extract'
    Expand-Archive -LiteralPath $Archive -DestinationPath $Extracted -Force
    $Source = Join-Path $Extracted "$ArchiveRoot\supermem.exe"
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        throw "Archive did not contain $ArchiveRoot\supermem.exe"
    }

    $InstallDir = [IO.Path]::GetFullPath($InstallDir)
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    $Destination = Join-Path $InstallDir 'supermem.exe'
    $Staged = Join-Path $InstallDir ('.supermem.new.' + [Guid]::NewGuid().ToString('N') + '.exe')
    Copy-Item -LiteralPath $Source -Destination $Staged -Force
    Move-Item -LiteralPath $Staged -Destination $Destination -Force

    if (-not $NoPathUpdate) {
        $UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $Entries = @($UserPath -split ';' | Where-Object { $_ })
        $AlreadyPresent = $Entries | Where-Object {
            try { [IO.Path]::GetFullPath($_).TrimEnd('\') -ieq $InstallDir.TrimEnd('\') } catch { $false }
        }
        if (-not $AlreadyPresent) {
            $NewUserPath = (@($InstallDir) + $Entries) -join ';'
            [Environment]::SetEnvironmentVariable('Path', $NewUserPath, 'User')
        }
        if (-not (($env:Path -split ';') -contains $InstallDir)) {
            $env:Path = "$InstallDir;$env:Path"
        }
    }

    $Reported = & $Destination --version
    if ($LASTEXITCODE -ne 0 -or ($Reported -notmatch [Regex]::Escape($Version))) {
        throw "Installed binary reported an unexpected version: $Reported"
    }
    Write-Host "Installed $Reported at $Destination"
    Write-Host 'Next step: supermem init'
} finally {
    Remove-Item -LiteralPath $Temporary -Recurse -Force -ErrorAction SilentlyContinue
}
