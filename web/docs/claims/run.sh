#!/bin/sh
# Compile and run each probe here and each lesson with GCC, Clang and bcc at
# -O0 and -O2, printing what each program wrote, its exit status and the
# compiler's warnings with -Wall -Wextra. See ../claims-audit.md.
#
#   GCC="gcc -B/path/to/gcc-14/libexec/" CLANG=clang BCC=bcc-rust sh web/docs/claims/run.sh
#
# Leave a compiler's variable unset to skip it. Run from the repository root.
set -u
here=web/docs/claims
out=$(mktemp -d)
for src in "$here"/*.c web/demos/*.c; do
    name=$(basename "$src" .c)
    echo "=== $name"
    for compiler in GCC CLANG BCC; do
        eval "command=\${$compiler:-}"
        [ -n "$command" ] || continue
        for level in O0 O2; do
            exe="$out/$name-$compiler-$level"
            flags="-std=c17 -$level -Wall -Wextra"
            # shellcheck disable=SC2086
            warnings=$($command $flags "$src" -o "$exe" 2>&1 | grep -E 'warning|error' | sort -u)
            if [ -x "$exe" ]; then
                timeout 5 "$exe" >"$out/printed" 2>&1
                status=$?
                printed=$(tr '\n' '|' <"$out/printed")
                [ -z "$printed" ] || printed="$printed "
                echo "[$compiler -$level] ${printed}exit=$status"
            else
                echo "[$compiler -$level] (did not compile)"
            fi
            [ -z "$warnings" ] || echo "$warnings" | sed 's/^/    /'
        done
    done
done
