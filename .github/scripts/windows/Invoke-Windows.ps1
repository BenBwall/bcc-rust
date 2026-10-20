[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Repository,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{40}$')][string]$ExpectedSha,
    [Parameter(Mandatory)][ValidateSet('ci', 'test', 'bench', 'torture')][string]$Suite,
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
        'torture' { ,@{ Name = 'release-build'; Arguments = @('build', '--release') } }
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
    if ($Suite -eq 'torture' -and -not @($checks | Where-Object exit_code -NE 0).Count) {
        # Arguments go to scripts/run_gcc_torture_execute.py, except
        # `--repeat N`, which runs the same survey N times in sequence.
        $repeat = 1
        $survey = [Collections.Generic.List[string]]::new()
        for ($index = 0; $index -lt $arguments.Count; $index++) {
            if ($arguments[$index] -ceq '--repeat' -and $index + 1 -lt $arguments.Count) {
                $repeat = [int]$arguments[++$index]
            } else { $survey.Add($arguments[$index]) }
        }
        if ($repeat -lt 1 -or $repeat -gt 20) { throw '--repeat must be between 1 and 20.' }
        foreach ($reserved in @('--source', '--output', '--bcc')) {
            if ($survey -ccontains $reserved) { throw "The torture suite supplies $reserved itself." }
        }
        # The pinned GCC release is downloaded and checksum-verified once into
        # the persistent checkout's target directory.
        $prepare = 'import sys; from pathlib import Path; sys.path.insert(0, "scripts"); ' +
                   'import run_gcc_torture as t; print(t.prepare_corpus(Path("target/compiler-corpus")))'
        $corpus = (& python -c $prepare 2>&1 | Tee-Object -FilePath (Join-Path $ResultsDirectory 'corpus.log') |
                   Select-Object -Last 1 | Out-String).Trim()
        $checks.Add(@{ name = 'corpus'; arguments = @('prepare_corpus'); exit_code = $LASTEXITCODE })
        if ($LASTEXITCODE -eq 0) {
            for ($round = 1; $round -le $repeat; $round++) {
                $name = "torture-$round"
                $output = Join-Path $Repository "target/compiler-corpus/windows-$env:GITHUB_RUN_ID-$env:GITHUB_RUN_ATTEMPT-$round"
                $command = @('scripts/run_gcc_torture_execute.py', '--source', $corpus, '--output', $output) + $survey
                $begin = [DateTime]::UtcNow.ToString('o')
                Write-Host "[$name] python $($command -join ' ')"
                & python @command 2>&1 | Tee-Object -FilePath (Join-Path $ResultsDirectory "$name.log") | Out-Host
                $surveyExit = $LASTEXITCODE
                $kept = Join-Path $ResultsDirectory $name
                New-Item -ItemType Directory -Path $kept | Out-Null
                foreach ($file in @('manifest.json', 'results.json', 'summary.md')) {
                    $path = Join-Path $output $file
                    if (Test-Path -LiteralPath $path) { Copy-Item -LiteralPath $path -Destination $kept }
                }
                # The survey exits 1 when a program fails or times out, which is
                # what this suite measures; only a missing result fails the job.
                $status = if (Test-Path -LiteralPath (Join-Path $output 'results.json')) { 0 } else { [Math]::Max(1, $surveyExit) }
                $checks.Add(@{ name = $name; arguments = $command; survey_exit_code = $surveyExit; exit_code = $status
                               started_utc = $begin; finished_utc = [DateTime]::UtcNow.ToString('o') })
            }
        }
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
