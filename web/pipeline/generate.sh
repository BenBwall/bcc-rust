#!/bin/sh
# Regenerate the precomputed compiler-pipeline views for every demo in
# web/demos/: tokens, syntax tree, bcc IR before and after ABI lowering, LLVM
# IR, native x86-64 assembly, and machine code (objdump of the native object).
# The site shows these for the bundled demos; LLVM IR is only available here,
# because the browser build of bcc has no LLVM.
#
# Usage, from the repository root:
#   BCC=path/to/bcc-rust sh web/pipeline/generate.sh           # rewrite
#   BCC=path/to/bcc-rust sh web/pipeline/generate.sh --check   # compare only
# BCC defaults to target/release/bcc-rust. objdump (GNU binutils) disassembles.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd -P)
bcc=${BCC:-$root/target/release/bcc-rust}
case $bcc in /*) ;; *) bcc=$(pwd -P)/$bcc ;; esac
[ -x "$bcc" ] || { echo "generate.sh: $bcc is not an executable bcc-rust; set BCC" >&2; exit 1; }
command -v objdump >/dev/null || { echo 'generate.sh: objdump is required' >&2; exit 1; }
check=false
[ "${1:-}" = --check ] && check=true
out=$root/web/pipeline
if $check; then
    out=$(mktemp -d)
    trap 'rm -rf "$out"' EXIT
fi
scratch=$(mktemp -d)
if $check; then
    trap 'rm -rf "$scratch" "$out"' EXIT
else
    trap 'rm -rf "$scratch"' EXIT
fi

cd "$root/web/demos"
for source in *.c; do
    stem=${source%.c}
    dir=$out/$stem
    mkdir -p "$dir"
    cp "$source" "$dir/source.c"
    # Each stage's diagnostics go to diagnostics.txt; a failing stage leaves
    # its view empty rather than stopping the run.
    : > "$dir/diagnostics.txt"
    : > "$scratch/$stem.err"
    stage() {
        file=$1
        shift
        "$bcc" "$@" "$source" > "$dir/$file" 2>> "$scratch/$stem.err" || true
    }
    # The token and syntax-tree views are printed on stderr.
    "$bcc" --tokens "$source" > "$dir/tokens.txt" 2>&1 || true
    "$bcc" --syntax-tree --syntax-locations "$source" > "$dir/syntax-tree.txt" 2>&1 || true
    stage ir.txt --emit=ir
    stage ir-abi.txt --emit=ir-abi
    stage llvm.ll --emit=llvm
    stage asm.s --emit=asm --backend=native
    "$bcc" --backend=native -c -o "$scratch/$stem.o" "$source" 2>> "$scratch/$stem.err" || true
    if [ -f "$scratch/$stem.o" ]; then
        # objdump's instruction lines are "  off:\tbytes\tinstruction"; keep
        # function labels and instructions, and drop alignment padding.
        objdump -d -w "$scratch/$stem.o" | awk -F'\t' '
            /^[0-9a-f]+ <.*>:$/ { sub(/^[0-9a-f]+ /, ""); print ""; print; next }
            NF >= 3 && $1 ~ /^ *[0-9a-f]+:$/ {
                offset = $1; gsub(/[ :]/, "", offset)
                bytes = $2; sub(/ +$/, "", bytes)
                text = $3; gsub(/  +/, " ", text)
                if (text ~ /(^| )nop/) next
                printf "%6s  %-24s %s\n", offset, bytes, text
            }' | sed '1{/^$/d}' > "$dir/machine-code.txt"
    else
        : > "$dir/machine-code.txt"
    fi
    # Every stage reports the same diagnostics; keep one copy of each line.
    if [ -f "$scratch/$stem.err" ]; then
        awk '!seen[$0]++' "$scratch/$stem.err" > "$dir/diagnostics.txt"
    fi
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
