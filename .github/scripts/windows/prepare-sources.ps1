# Prepare the exact source gitlinks, using the repository's sparse patterns.
# This downloads sources; it does not build or run compiler test programs.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Repository)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$Repository = (Resolve-Path -LiteralPath $Repository).Path

function Invoke-Git {
    param([string[]]$Arguments)
    & git @Arguments
    if ($LASTEXITCODE -ne 0) { throw "git failed ($LASTEXITCODE): $($Arguments -join ' ')" }
}

function Get-GitCommit {
    param([string]$Directory, [string]$Object)
    $value = (& git -C $Directory rev-parse $Object | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $value -notmatch '^[0-9a-f]{40}$') {
        throw "Cannot resolve exact Git object $Object in $Directory"
    }
    return $value
}

function Get-SparseSource {
    param([string]$Directory, [string]$Remote, [string]$Commit, [string]$Patterns)
    if (Test-Path -LiteralPath (Join-Path $Directory '.git')) {
        $existing = Get-GitCommit $Directory 'HEAD'
        if ($existing -ne $Commit) {
            throw "Existing $Directory is at $existing; expected $Commit. Use a fresh checkout."
        }
        $dirty = & git -C $Directory status --porcelain
        if ($LASTEXITCODE -ne 0 -or $dirty) { throw "Existing source checkout is dirty: $Directory" }
        Write-Host "Reusing exact source checkout: $Directory ($Commit)"
        return
    }
    if ((Test-Path -LiteralPath $Directory) -and
        @(Get-ChildItem -LiteralPath $Directory -Force).Count -ne 0) {
        throw "Refusing to initialize nonempty directory: $Directory"
    }
    New-Item -ItemType Directory -Force -Path $Directory | Out-Null
    Invoke-Git -Arguments @('-C', $Directory, 'init', '--quiet')
    Invoke-Git -Arguments @('-C', $Directory, 'remote', 'add', 'origin', $Remote)
    Invoke-Git -Arguments @('-C', $Directory, 'config', 'remote.origin.promisor', 'true')
    Invoke-Git -Arguments @('-C', $Directory, 'config', 'remote.origin.partialclonefilter', 'blob:none')
    Get-Content -LiteralPath $Patterns | & git -C $Directory sparse-checkout set --no-cone --stdin
    if ($LASTEXITCODE -ne 0) { throw "Cannot set sparse patterns for $Directory" }
    Invoke-Git -Arguments @('-C', $Directory, 'fetch', '--depth', '1', '--filter=blob:none', 'origin', $Commit)
    Invoke-Git -Arguments @('-C', $Directory, 'checkout', '--detach', $Commit)
    if ((Get-GitCommit $Directory 'HEAD') -ne $Commit) { throw 'Source commit mismatch' }
}

$rustCommit = Get-GitCommit $Repository 'HEAD:vendor/rust'
$rustDirectory = Join-Path $Repository 'vendor/rust'
Get-SparseSource $rustDirectory 'https://github.com/rust-lang/rust.git' $rustCommit `
    (Join-Path $Repository 'build_support/rust.sparse')
$llvmCommit = Get-GitCommit $rustDirectory 'HEAD:src/llvm-project'
Get-SparseSource (Join-Path $rustDirectory 'src/llvm-project') `
    'https://github.com/rust-lang/llvm-project.git' $llvmCommit `
    (Join-Path $Repository 'build_support/llvm.sparse')
[ordered]@{
    repository_commit = Get-GitCommit $Repository 'HEAD'
    rust_source_commit = $rustCommit
    llvm_source_commit = $llvmCommit
} | ConvertTo-Json
