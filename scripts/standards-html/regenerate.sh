#!/bin/sh
# Regenerate every HTML edition in standards/ from its PDF.
#
#   sh scripts/standards-html/regenerate.sh [edition ...]
#
# Editions are c89 c99 c11 c17 c23; with none, all of them. Run from the
# repository root.
set -eu

# The repository's Cargo configuration links the compiler with LTO flags that
# this standalone tool does not use.
export CARGO_ENCODED_RUSTFLAGS= RUSTFLAGS=
cargo build --quiet --release --manifest-path scripts/standards-html/Cargo.toml

convert() {
  edition=$1 file=$2 title=$3
  shift 3
  case " $selected " in *" $edition "*) ;; *) return 0 ;; esac
  echo "standards/$file.html"
  cargo run --quiet --release --manifest-path scripts/standards-html/Cargo.toml -- \
    "standards/$file.pdf" "standards/$file.html" --title "$title" "$@"
}

selected=${*:-c89 c99 c11 c17 c23}
convert c89 c89-fips160 "C89 — ANSI X3.159-1989 (FIPS PUB 160)"
convert c99 c99-n1256 "C99 — ISO/IEC 9899:TC3 (WG14 N1256)"
convert c11 c11-n1570 "C11 — ISO/IEC 9899:2011 (WG14 N1570)"
convert c17 c17-n2310 "C17 — ISO/IEC 9899:2018 (WG14 N2310)" --diff-marks
convert c23 c23-n3220 "C23 — ISO/IEC 9899:2024 (WG14 N3220)"
