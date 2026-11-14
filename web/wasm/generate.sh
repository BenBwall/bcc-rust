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
# map each instruction to its C line, without the C library, and with every
# function exported so the stepper finds `main`. A lesson that includes
# <stdio.h> calls the C library, so it is built as a WASI command instead,
# linked with a copy of wasi-sdk's wasi-libc whose debug information is
# stripped: the stepper would otherwise read the library's own line tables as
# lines of the lesson. `-ffile-prefix-map` keeps the build directory out of
# the debug information, so the output does not depend on the checkout.
#
# Each module ends with a `bcc.lesson` custom section holding FLAGS, a
# newline, and the lesson's source, which the site's tests compare with the
# lessons (web/src/wasm/lessons.rs), so a stale module fails CI without a
# wasm32 clang.
set -eu
: "${CLANG:?set CLANG to a clang that targets wasm32 (for example wasi-sdk's)}"
# Keep in step with LESSON_FLAGS and COMMAND_FLAGS in web/src/wasm/lessons.rs.
BARE_FLAGS='--target=wasm32-unknown-unknown -nostdlib -O0 -g -Wl,--no-entry -Wl,--export-all'
COMMAND_FLAGS='--target=wasm32-wasip1 -O0 -g'
check=false
[ "${1:-}" = --check ] && check=true
cd "$(dirname "$0")"

# Write $1 as an unsigned LEB128.
leb() {
    n=$1
    while :; do
        byte=$((n % 128))
        n=$((n / 128))
        [ "$n" -gt 0 ] && byte=$((byte + 128))
        printf "\\$(printf '%03o' "$byte")"
        [ "$n" -gt 0 ] || break
    done
}

status=0
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
# wasi-sdk's sysroot for wasm32-wasip1, its libraries copied with their debug
# information stripped.
bin=$(dirname "$CLANG")
sysroot=$scratch/wasi-sysroot
mkdir -p "$sysroot/include" "$sysroot/lib"
cp -R "$bin/../share/wasi-sysroot/include/wasm32-wasip1" "$sysroot/include/"
cp -R "$bin/../share/wasi-sysroot/lib/wasm32-wasip1" "$sysroot/lib/"
for library in "$sysroot"/lib/wasm32-wasip1/*.a "$sysroot"/lib/wasm32-wasip1/*.o; do
    "$bin/llvm-strip" --strip-debug "$library"
done
for source in ../demos/*.c; do
    name=$(basename "$source" .c)
    if grep -qx '#include <stdio.h>' "$source"; then
        FLAGS=$COMMAND_FLAGS
        LIBRARY="--sysroot=$sysroot -ffile-prefix-map=$sysroot=/wasi-sysroot"
    else
        FLAGS=$BARE_FLAGS
        LIBRARY=
    fi
    (
        cd ../demos
        # shellcheck disable=SC2086 # FLAGS and LIBRARY hold several arguments.
        "$CLANG" $FLAGS $LIBRARY -ffile-prefix-map="$PWD"=. "$name.c" -o "$scratch/$name.wasm"
    )
    { printf '%s\n' "$FLAGS"; cat "$source"; } > "$scratch/$name.lesson"
    size=$(wc -c < "$scratch/$name.lesson")
    {
        printf '\000'
        leb $((1 + 10 + size))
        leb 10
        printf 'bcc.lesson'
        cat "$scratch/$name.lesson"
    } >> "$scratch/$name.wasm"
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
