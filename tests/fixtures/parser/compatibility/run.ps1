param(
    [string]$Gcc = "gcc.exe",
    [string]$Clang = (Join-Path $env:LIBCLANG_PATH "clang.exe")
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$gccVersion = (& $Gcc --version | Select-Object -First 1)
$clangVersion = (& $Clang --version | Select-Object -First 1)
if ($gccVersion -notmatch "13\.2\.0") {
    throw "Expected pinned GCC 13.2.0, found: $gccVersion"
}
if ($clangVersion -notmatch "19\.1\.5") {
    throw "Expected pinned Clang 19.1.5, found: $clangVersion"
}

$fixtures = Get-ChildItem -LiteralPath $root -Filter "*.c" | Sort-Object Name
foreach ($fixture in $fixtures) {
    & $Gcc -std=c99 -pedantic-errors -Wall -Wextra -Wno-unused-parameter -m64 -fmax-errors=20 -fsyntax-only $fixture.FullName
    $gccExit = $LASTEXITCODE
    & $Clang -std=c99 -pedantic-errors -Wall -Wextra -Wno-unused-parameter --target=x86_64-pc-windows-msvc -ferror-limit=20 -fsyntax-only $fixture.FullName
    $clangExit = $LASTEXITCODE
    [pscustomobject]@{
        Fixture = $fixture.Name
        GccExit = $gccExit
        ClangExit = $clangExit
    }
}
