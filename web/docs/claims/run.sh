#!/bin/sh
# Compile and run each probe here and each lesson with GCC, Clang and bcc at
# -O0 and -O2, printing what each program wrote, its exit status and the
# compiler's warnings with -Wall -Wextra. See ../claims-audit.md.
#
#   GCC="gcc -B/path/to/gcc-14/libexec/" CLANG=clang BCC=bcc-rust sh web/docs/claims/run.sh
#
# Leave a compiler's variable unset to skip it. Run from the repository root.
#
# STD picks the language standard, c17 by default, as results.txt was made;
# Part 20's results-part20.txt and later runs use STD=c99. EXTRA adds options
# after -Wall -Wextra, such as EXTRA=-Wshadow. Name probes or lessons, as in
# `STD=c99 sh web/docs/claims/run.sh c34_fallthrough c35_shadow`, to run only
# those; with no names it runs them all.
set -u
here=web/docs/claims
std=${STD:-c17}
extra=${EXTRA:-}
out=$(mktemp -d)
if [ "$#" -eq 0 ]; then
    set -- "$here"/*.c web/demos/*.c
else
    for name in "$@"; do
        if [ -f "$here/$name.c" ]; then
            set -- "$@" "$here/$name.c"
        elif [ -f "web/demos/$name.c" ]; then
            set -- "$@" "web/demos/$name.c"
        else
            echo "run.sh: no probe or lesson named $name" >&2
            exit 2
        fi
        shift
    done
fi
for src in "$@"; do
    name=$(basename "$src" .c)
    echo "=== $name"
    for compiler in GCC CLANG BCC; do
        eval "command=\${$compiler:-}"
        [ -n "$command" ] || continue
        for level in O0 O2; do
            exe="$out/$name-$compiler-$level"
            flags="-std=$std -$level -Wall -Wextra $extra"
            # shellcheck disable=SC2086
            warnings=$($command $flags "$src" -o "$exe" 2>&1 | grep -E 'warning|error' | sort -u)
            if [ -x "$exe" ]; then
                timeout 5 "$exe" >"$out/printed" 2>&1
                status=$?
                # timeout(1) reports a core dump where cores go to a pipe.
                printed=$(grep -v '^timeout: the monitored command dumped core' "$out/printed" | tr '\n' '|')
                [ -z "$printed" ] || printed="$printed "
                echo "[$compiler -$level] ${printed}exit=$status"
            else
                echo "[$compiler -$level] (did not compile)"
            fi
            [ -z "$warnings" ] || echo "$warnings" | sed 's/^/    /'
        done
    done
done
