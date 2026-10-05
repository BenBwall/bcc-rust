$env:RUSTFLAGS = ' '
cargo run --quiet --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml') -- @args
exit $LASTEXITCODE
