#!/bin/sh
# Compile each lesson in web/demos/ to WebAssembly with clang, for the
# WebAssembly view until bcc's own WebAssembly back end produces these
# modules in the browser. Run from the repository root:
#
#   CLANG=path/to/clang sh web/wasm/generate.sh           # rewrite
#   CLANG=path/to/clang sh web/wasm/generate.sh --check   # compare only
#
# CLANG must target wasm32 and find wasm-ld: the repository's pinned LLVM is
# built for x86-64 only, so use wasi-sdk's (https://github.com/WebAssembly/
# wasi-sdk; these were generated with wasi-sdk 34, clang 23). The modules
# are built at -O0 with DWARF line tables (-g), which the stepper reads to
# map each instruction to its C line, without the C library (the lessons
# call no library functions), and with every function exported so the
# stepper finds `main`. `-ffile-prefix-map` keeps the build directory out of
# the debug information, so the output does not depend on the checkout.
set -eu
: "${CLANG:?set CLANG to a clang that targets wasm32 (for example wasi-sdk's)}"
check=false
[ "${1:-}" = --check ] && check=true
cd "$(dirname "$0")"
status=0
scratch=$(mktemp -d)
for source in ../demos/*.c; do
    name=$(basename "$source" .c)
    (
        cd ../demos
        "$CLANG" --target=wasm32-unknown-unknown -nostdlib -O0 -g \
            -ffile-prefix-map="$PWD"=. \
            -Wl,--no-entry -Wl,--export-all \
            "$name.c" -o "$scratch/$name.wasm"
    )
    if $check; then
        if ! cmp -s "$scratch/$name.wasm" "$name.wasm"; then
            echo "web/wasm/$name.wasm is out of date" >&2
            status=1
        fi
    else
        cp "$scratch/$name.wasm" "$name.wasm"
    fi
done
exit $status
