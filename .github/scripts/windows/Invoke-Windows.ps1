[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Repository,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{40}$')][string]$ExpectedSha,
    [Parameter(Mandatory)][ValidateSet('ci', 'test', 'bench')][string]$Suite,
    [string]$CargoArgumentsJson = '[]',
    [Parameter(Mandatory)][string]$ResultsDirectory
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$Repository = (Resolve-Path -LiteralPath $Repository).Path
$ResultsDirectory = (Resolve-Path -LiteralPath $ResultsDirectory).Path
$arguments = ConvertFrom-Json -InputObject $CargoArgumentsJson -NoEnumerate
if ($arguments -isnot [array] -or @($arguments | Where-Object { $_ -isnot [string] }).Count) {
    throw 'Cargo arguments must be an array of strings.'
}
if ($Suite -eq 'ci' -and $arguments.Count) { throw 'CI runs its fixed canonical commands; use test/bench for arguments.' }
Set-Location -LiteralPath $Repository
$sha = (& git rev-parse HEAD | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $sha -ne $ExpectedSha) { throw 'Checkout does not match the requested exact commit.' }
$dirty = & git status --porcelain
if ($LASTEXITCODE -ne 0 -or $dirty) { throw "The requested source checkout must be clean: $dirty" }
foreach ($name in @('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET', 'CARGO_TARGET_DIR')) {
    Remove-Item "Env:$name" -ErrorAction SilentlyContinue
}
# Cargo versions/branches without a checked-in runner still receive crash-dialog
# suppression. The parent wrapper also covers every descendant process.
$wrapper = 'C:\bcc\runner\cargo_runner.py'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER = "python $wrapper"
$env:RUSTUP_TOOLCHAIN = '1.99.0-x86_64-pc-windows-gnu'
$env:PATH = "$env:MINGW_ROOT\bin;$env:PATH"
$env:CRITERION_HOME = Join-Path $ResultsDirectory 'criterion'
$checks = [Collections.Generic.List[object]]::new()
$started = [DateTime]::UtcNow.ToString('o')
$problem = $null
try {
    $commands = switch ($Suite) {
        'test' { ,@{ Name = 'tests'; Arguments = @('test') + $arguments } }
        'bench' { ,@{ Name = 'benchmarks'; Arguments = @('bench', '--features', 'benchmarking-internals') + $arguments } }
        'ci' {
            @{ Name = 'all-targets'; Arguments = @('test', '--all-targets', '--no-fail-fast') }
            @{ Name = 'allocation-count'; Arguments = @('test', '--features', 'benchmarking-internals', '--test', 'allocation_count') }
            @{ Name = 'clippy'; Arguments = @('clippy', '--all-targets', '--', '-D', 'warnings') }
            @{ Name = 'clippy-benchmarking'; Arguments = @('clippy', '--all-targets', '--features', 'benchmarking-internals', '--', '-D', 'warnings') }
            @{ Name = 'format'; Arguments = @('+nightly-x86_64-pc-windows-gnu', 'fmt', '--check') }
        }
    }
    foreach ($command in $commands) {
        $begin = [DateTime]::UtcNow.ToString('o')
        Write-Host "[$($command.Name)] cargo $($command.Arguments -join ' ')"
        & python $wrapper cargo @($command.Arguments) 2>&1 |
            Tee-Object -FilePath (Join-Path $ResultsDirectory "$($command.Name).log") | Out-Host
        $checks.Add(@{ name = $command.Name; arguments = $command.Arguments; exit_code = $LASTEXITCODE;
                      started_utc = $begin; finished_utc = [DateTime]::UtcNow.ToString('o') })
    }
    if ($Suite -eq 'ci') {
        & git diff --check 2>&1 | Tee-Object -FilePath (Join-Path $ResultsDirectory 'diff-check.log') | Out-Host
        $checks.Add(@{ name = 'diff-check'; arguments = @('diff', '--check'); exit_code = $LASTEXITCODE })
    }
} catch { $problem = $_.Exception.Message; Write-Host "::error::$problem" }
finally {
    $identity = (& git rev-parse 'HEAD^{tree}' | Out-String).Trim()
    [ordered]@{
        runner = $env:RUNNER_NAME; suite = $Suite; commit = $sha; tree = $identity
        windows = [Environment]::OSVersion.VersionString; architecture = [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
        rustc = (& rustc -vV | Out-String).Trim(); mingw_root = $env:MINGW_ROOT
        started_utc = $started; finished_utc = [DateTime]::UtcNow.ToString('o')
        checks = @($checks); problem = $problem
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $ResultsDirectory 'results.json')
}
if ($problem -or -not $checks.Count -or @($checks | Where-Object exit_code -NE 0).Count) { exit 1 }
exit 0
