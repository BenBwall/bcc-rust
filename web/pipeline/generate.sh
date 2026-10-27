#!/bin/sh
# Regenerate the precomputed LLVM IR for every lesson in web/demos/, at -O0
# (llvm.ll) and -O2 (llvm-O2.ll), with a copy of the source it came from. The
# site compiles every other stage live with bcc's in-browser compiler, which
# has no LLVM, so LLVM IR is the one stage the lessons carry precomputed.
#
# Usage, from the repository root:
#   BCC=path/to/bcc-rust sh web/pipeline/generate.sh           # rewrite
#   BCC=path/to/bcc-rust sh web/pipeline/generate.sh --check   # compare only
# BCC defaults to target/release/bcc-rust.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd -P)
bcc=${BCC:-$root/target/release/bcc-rust}
case $bcc in /*) ;; *) bcc=$(pwd -P)/$bcc ;; esac
[ -x "$bcc" ] || { echo "generate.sh: $bcc is not an executable bcc-rust; set BCC" >&2; exit 1; }
check=false
[ "${1:-}" = --check ] && check=true
out=$root/web/pipeline
if $check; then
    out=$(mktemp -d)
    trap 'rm -rf "$out"' EXIT
fi

cd "$root/web/demos"
for source in *.c; do
    stem=${source%.c}
    dir=$out/$stem
    mkdir -p "$dir"
    cp "$source" "$dir/source.c"
    "$bcc" --emit=llvm "$source" > "$dir/llvm.ll"
    "$bcc" -O2 --emit=llvm "$source" > "$dir/llvm-O2.ll"
done

if $check; then
    if ! diff -ru -x generate.sh -x README.md "$root/web/pipeline" "$out"; then
        echo 'generate.sh: web/pipeline/ is stale; run web/pipeline/generate.sh and commit the result.' >&2
        exit 1
    fi
    echo 'Pipeline outputs are up to date.'
else
    echo "Wrote the pipeline outputs to web/pipeline/."
fi
