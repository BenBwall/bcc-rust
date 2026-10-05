#!/bin/sh
set -eu
RUSTFLAGS='' cargo run --quiet --manifest-path "$(dirname "$0")/Cargo.toml" -- "$@"
