#!/bin/sh
# Regenerate the precomputed LLVM IR for every lesson in web/demos/, at -O0
# (llvm.ll) and -O2 (llvm-O2.ll), with a copy of the source it came from and
# where each of its lines came from (llvm.lines, llvm-O2.lines). The site
# compiles every other stage live with bcc's in-browser compiler, which has no
# LLVM, so LLVM IR is the one stage the lessons carry precomputed.
#
# A .lines file reads the debug locations `-g1` adds to the same IR. It has one
# line per line of the .ll file: `-` for a line that is not an instruction, `0`
# for an instruction with no source line, `L` for one from source line L, and
# `L@C` for one from line L inlined for the call on line C.
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
located=$(mktemp)
if $check; then
    out=$(mktemp -d)
    trap 'rm -rf "$out" "$located"' EXIT
else
    trap 'rm -f "$located"' EXIT
fi

# Write where each line of the IR "$1" came from, given the same IR with
# locations in "$2". Refuses IR that the locations changed, and a program whose
# locations are in more than one file, which the site could not place.
locations() {
    if [ "$(grep -c '!DIFile' "$2")" -ne 1 ]; then
        echo "generate.sh: $2 has locations in other files" >&2
        return 1
    fi
    awk -v plain="$1" '
        FNR == NR {
            if ($0 ~ /^![0-9]+ = (distinct )?!DILocation\(/) {
                id = $0; sub(/^!/, "", id); sub(/ .*/, "", id)
                at = $0; sub(/.*[(]line: /, "", at); sub(/,.*/, "", at)
                line[id] = at
                if ($0 ~ /inlinedAt: !/) {
                    at = $0; sub(/.*inlinedAt: !/, "", at); sub(/[,)].*/, "", at)
                    outer[id] = at
                }
            }
            next
        }
        /^!/ { exit }
        {
            text = $0
            id = ""
            if (text ~ /!dbg ![0-9]+/) {
                id = text; sub(/.*!dbg !/, "", id); sub(/[^0-9].*/, "", id)
            }
            sub(/,? !dbg ![0-9]+/, "", text)
            if ((getline expected < plain) <= 0 || expected != text) {
                print "generate.sh: the locations changed the IR: " text > "/dev/stderr"
                failed = 1
                exit 1
            }
            count++
            if ($0 !~ /^  [^ ;]/) {
                out[count] = "-"
            } else if (id == "" || line[id] == 0) {
                out[count] = "0"
            } else if (id in outer) {
                call = id
                while (call in outer) call = outer[call]
                out[count] = line[id] "@" line[call]
            } else {
                out[count] = line[id]
            }
            # A switch writes its location on the line that closes its cases.
            if (text ~ / \[$/) {
                open = count
            } else if (open && text ~ /^  \]/) {
                out[open] = out[count]
                out[count] = "-"
                open = 0
            }
        }
        END {
            if (failed) exit 1
            for (each = 1; each <= count; each++) print out[each]
            while ((getline expected < plain) > 0) print "-"
        }
    ' "$2" "$2"
}

cd "$root/web/demos"
for source in *.c; do
    stem=${source%.c}
    dir=$out/$stem
    mkdir -p "$dir"
    cp "$source" "$dir/source.c"
    "$bcc" --emit=llvm "$source" > "$dir/llvm.ll"
    "$bcc" -g1 --emit=llvm "$source" > "$located"
    locations "$dir/llvm.ll" "$located" > "$dir/llvm.lines"
    "$bcc" -O2 --emit=llvm "$source" > "$dir/llvm-O2.ll"
    "$bcc" -O2 -g1 --emit=llvm "$source" > "$located"
    locations "$dir/llvm-O2.ll" "$located" > "$dir/llvm-O2.lines"
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
