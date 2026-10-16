#requires -Version 5.1
<#
.SYNOPSIS
Move prevalabs-mono and bcc-rust work into their existing Docker sandboxes.
.DESCRIPTION
Defaults to inspection only. -Apply copies and verifies all working state before
replacing the Windows checkouts with clean, independent origin/main clones.
Close editors, agents, build tools and Git clients before using -Apply.
#>
[CmdletBinding()]
param(
    [string]$RepoRoot = (Join-Path $env:USERPROFILE 'Documents\GitRepo'),
    [string]$PrevalabsPath,
    [string]$BccRustPath,
    [string]$SandboxRoot = '/home/agent/workspace',
    [switch]$Apply,
    [switch]$KeepHostBackups,
    [switch]$SkipBuildCaches
)

$ErrorActionPreference = 'Stop'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'Run this script on the Windows host, not inside a sandbox.'
}
Get-Command git -ErrorAction Stop | Out-Null
Get-Command sbx -ErrorAction Stop | Out-Null
$python = Get-Command python -ErrorAction SilentlyContinue
if (-not $python) { throw 'Python 3.10 or newer is required on the Windows host.' }
$helper = Join-Path $PSScriptRoot 'migrate.py'
if (-not (Test-Path -LiteralPath $helper)) { throw "Missing helper: $helper" }
if ($Apply) {
    # Keep the runner outside the directories that it will replace.
    $savedTools = Join-Path $RepoRoot ('.sandbox-migration\tools-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $savedTools -Force | Out-Null
    foreach ($file in @('Move-WorkToSandboxes.ps1', 'migrate.py', 'README.md')) {
        $source = Join-Path $PSScriptRoot $file
        if (Test-Path -LiteralPath $source) { Copy-Item -LiteralPath $source -Destination $savedTools }
    }
    $helper = Join-Path $savedTools 'migrate.py'
    Write-Host "Saved migration tools: $savedTools"
}
$arguments = @($helper, 'host', '--repo-root', $RepoRoot, '--sandbox-root', $SandboxRoot)
if ($PrevalabsPath) { $arguments += @('--prevalabs-path', $PrevalabsPath) }
if ($BccRustPath) { $arguments += @('--bcc-rust-path', $BccRustPath) }
if ($Apply) { $arguments += '--apply' }
if ($KeepHostBackups) { $arguments += '--keep-host-backups' }
if ($SkipBuildCaches) { $arguments += '--skip-build-caches' }
& $python.Source @arguments
if ($LASTEXITCODE -ne 0) { throw "Migration stopped (exit $LASTEXITCODE). Read the error and recovery paths above." }
