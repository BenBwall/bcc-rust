param(
    [string[]]$Sandboxes = @('bcc-rust'),
    [switch]$AtLogon
)

$ErrorActionPreference = 'Stop'
$Sandboxes = @($Sandboxes | ForEach-Object { $_ -split ',' })
$checkout = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$relayScript = Join-Path $PSScriptRoot 'relay.py'
function Test-RelayAlive($heartbeat) {
    if (-not (Test-Path $heartbeat)) { return $false }
    $state = Get-Content -Raw $heartbeat | ConvertFrom-Json
    if (-not $state.pid -or ([DateTimeOffset]::UtcNow.ToUnixTimeSeconds() - $state.timestamp) -ge 15) { return $false }
    $process = Get-CimInstance Win32_Process -Filter "ProcessId = $($state.pid)"
    return $null -ne $process -and $process.CommandLine.Contains($relayScript)
}
Push-Location $checkout
try {
    $common = git rev-parse --path-format=absolute --git-common-dir
    if ($LASTEXITCODE -ne 0) { throw 'Cannot locate the Git common directory' }
    $directory = Join-Path $common 'agentbus/relay'
    New-Item -ItemType Directory -Force -Path $directory | Out-Null

    $heartbeat = Join-Path $directory 'heartbeat'
    $alive = Test-RelayAlive $heartbeat
    if (-not $alive) {
        # Match run.py's isolation from the compiler's linker/LTO flags.
        $oldFlags = $env:RUSTFLAGS
        $oldEncoded = $env:CARGO_ENCODED_RUSTFLAGS
        try {
            $env:RUSTFLAGS = ''
            $env:CARGO_ENCODED_RUSTFLAGS = ''
            $artifacts = cargo build --quiet --manifest-path scripts/agentbus/Cargo.toml --message-format=json
            if ($LASTEXITCODE -ne 0) { throw 'Agentbus build failed' }
        } finally {
            $env:RUSTFLAGS = $oldFlags
            $env:CARGO_ENCODED_RUSTFLAGS = $oldEncoded
        }
        $executable = ($artifacts | ForEach-Object { $_ | ConvertFrom-Json } |
            Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'agentbus' -and $_.executable } |
            Select-Object -Last 1).executable
        if (-not $executable) { throw 'Cargo did not report an agentbus executable' }
        $arguments = @(
            ('"' + $relayScript + '"'),
            '--directory', ('"' + $directory + '"'),
            '--executable', ('"' + $executable + '"'),
            '--database', ('"' + (Join-Path $common 'agentbus/bus.db') + '"')
        )
        foreach ($sandbox in $Sandboxes) { $arguments += @('--sandbox', $sandbox) }
        Start-Process -FilePath (Get-Command python).Source -ArgumentList $arguments -WindowStyle Hidden `
            -WorkingDirectory $checkout -RedirectStandardOutput (Join-Path $directory 'host.stdout.log') `
            -RedirectStandardError (Join-Path $directory 'host.stderr.log')
        $deadline = [DateTime]::UtcNow.AddSeconds(20)
        do {
            Start-Sleep -Milliseconds 100
            $alive = Test-RelayAlive $heartbeat
            if ($alive) { break }
        } while ([DateTime]::UtcNow -lt $deadline)
        if (-not $alive) { throw "Relay failed to start; see $directory/host.stderr.log" }
    }
    if ($AtLogon) {
        $startup = [Environment]::GetFolderPath('Startup')
        $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path $startup 'Agentbus-bcc-rust.lnk'))
        $shortcut.TargetPath = (Get-Command pwsh).Source
        $shortcut.Arguments = '-NoProfile -WindowStyle Hidden -File "' + $PSCommandPath + '" -Sandboxes ' + ($Sandboxes -join ',')
        $shortcut.WorkingDirectory = $checkout
        $shortcut.Save()
    }
    Write-Output "Agentbus host relay is running: $directory"
} finally {
    Pop-Location
}
