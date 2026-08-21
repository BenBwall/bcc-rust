$ErrorActionPreference = "Stop"

$root = git rev-parse --show-toplevel
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$source = Join-Path $root ".githooks\commit-msg.rs"
$sourceHash = git hash-object -- $source
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$relativeHooksPath = "bcc-rust-hooks/windows-$sourceHash"
$gitCommonDir = git -C $root rev-parse --path-format=absolute --git-common-dir
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
$hooksPath = Join-Path $gitCommonDir $relativeHooksPath

New-Item -ItemType Directory -Force -Path $hooksPath | Out-Null
$hook = Join-Path $hooksPath "commit-msg.exe"
rustc --edition=2024 $source -o $hook
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

git -C $root config --local core.hooksPath $hooksPath
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

Write-Output "Configured the bcc-rust commit-message hook."
