[CmdletBinding()]
param([Parameter(Mandatory)][string]$Repository)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$Repository = (Resolve-Path -LiteralPath $Repository).Path
if (-not $IsWindows -or -not [Environment]::Is64BitProcess) { throw 'Native x64 Windows is required.' }
$env:RUSTUP_TOOLCHAIN = '1.99.0-x86_64-pc-windows-gnu'
$env:MINGW_ROOT = 'C:\Strawberry\c'
$env:PATH = "$env:MINGW_ROOT\bin;$env:PATH"
foreach ($name in @('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET', 'CARGO_TARGET_DIR',
                    'CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER')) {
    Remove-Item "Env:$name" -ErrorAction SilentlyContinue
}
foreach ($name in @('git', 'python', 'cmake', 'ninja', 'rustup', 'cargo', 'gcc', 'g++')) {
    Get-Command $name -ErrorAction Stop | Out-Null
}
$rustc = (& rustc -vV | Out-String)
if ($LASTEXITCODE -ne 0 -or $rustc -notmatch 'host: x86_64-pc-windows-gnu\s' -or
    $rustc -notmatch 'release: 1\.99\.0\s' -or $rustc -notmatch 'LLVM version: 23\.1\.1\s') {
    throw "Native Rust toolchain mismatch: $rustc"
}
# These installations were built and verified on this Windows host. Only share
# them with the exact source pin; Cargo outputs remain in this runner's checkout.
$prepared = 'C:\bcc\manual-checkout'
$rustPin = (& git -C $Repository rev-parse HEAD:vendor/rust | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve the requested Rust source pin.' }
if ($rustPin -eq 'b940084d7eb6a299eb4bfeb8e34901bc051e7ac4') {
    foreach ($entry in @(
        @{ Relative = 'target/llvm'; Stamp = '23.1.1' },
        @{ Relative = 'target/llvm-libs'; Stamp = '23.1.1-x86_64-pc-windows-gnu-static-x86-v1' },
        @{ Relative = 'vendor/rust'; Stamp = $null }
    )) {
        $destination = Join-Path $Repository $entry.Relative
        $source = Join-Path $prepared $entry.Relative
        if ($entry.Stamp -and (Get-Content -Raw -LiteralPath "$source/.installed-version") -ne $entry.Stamp) {
            throw "Prepared cache stamp mismatch: $source"
        }
        if (-not (Test-Path -LiteralPath $destination)) {
            New-Item -ItemType Directory -Force -Path (Split-Path $destination) | Out-Null
            New-Item -ItemType Junction -Path $destination -Target $source | Out-Null
        } elseif ($entry.Relative -eq 'vendor/rust' -and
                  @(Get-ChildItem -Force -LiteralPath $destination).Count -eq 0) {
            # Git creates an empty submodule placeholder. Only remove that empty
            # directory, after verifying it is inside this owned checkout.
            if (-not ([IO.Path]::GetFullPath($destination)).StartsWith($Repository + '\', [StringComparison]::OrdinalIgnoreCase)) {
                throw 'Source placeholder is outside the runner checkout.'
            }
            Remove-Item -LiteralPath $destination
            New-Item -ItemType Junction -Path $destination -Target $source | Out-Null
        }
    }
}
# New dependency repositories inherit Windows checkout settings without changing
# machine/global Git config. The matching pinned sources are checked for dirt.
$env:GIT_CONFIG_COUNT = '2'
$env:GIT_CONFIG_KEY_0 = 'core.longpaths'
$env:GIT_CONFIG_VALUE_0 = 'true'
$env:GIT_CONFIG_KEY_1 = 'core.autocrlf'
$env:GIT_CONFIG_VALUE_1 = 'false'
Write-Host $rustc
Write-Host "Source: $Repository; Rust pin: $rustPin; MinGW: $env:MINGW_ROOT"
