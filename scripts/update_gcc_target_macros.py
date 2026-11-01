"""Freeze GCC 14.2's predefined macros for bcc's GCC identity on Linux.

bcc claims GCC 14.2 under `--compiler-identity=gcc` (`--gcc-mode`). On the
Linux targets that identity reads these tables instead of the pinned Clang's
(src/target/*.h), since GCC's listing differs from Clang's in more than
spelling: GCC makes `int_fast16_t` and `int_fast32_t` `long` on glibc, and it
defines none of Clang's `__INTn_FMTd__`, `__INTn_C_SUFFIX__` or
`__CLANG_ATOMIC_*` families.

The tables come from GCC 14.2's own preprocessor, `cc1`, run directly so that
no distribution driver defaults (`-fPIE`, `-fstack-protector-strong`,
`-fcf-protection`) leak into them: pass `--cc1` the `cc1` of a GCC 14.2 for
x86_64-linux-gnu, such as Ubuntu's `cpp-14-x86-64-linux-gnu` package
(`usr/libexec/gcc/x86_64-linux-gnu/14/cc1`), which runs from an extracted
`.deb` without installing it. `-mmusl` gives the musl target's listing, as a
GCC configured for x86_64-linux-musl defines it.

Macros another part of bcc owns, and those that advertise something bcc does
not implement, are left out and listed with the reason in
src/target/gcc/macro-exclusions.tsv. `--check` compares instead of writing.
"""
import argparse
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
from update_target_macros import exclusion as clang_exclusion  # noqa: E402

GCC_VERSION = "14.2.0"
# The target's file prefix and the cc1 options that select its C library.
TARGETS = (("x86_64-unknown-linux-gnu", []), ("x86_64-unknown-linux-musl", ["-mmusl"]))
INTERCHANGE_PREFIXES = ("__FLT16_", "__BFLT16_", "__FLT32_", "__FLT64_", "__FLT32X_", "__FLT64X_", "__FLT128_")


def exclusion(name):
    # bcc lowers `=@ccCOND` flag outputs of x86 inline assembly.
    if name == "__GCC_ASM_FLAG_OUTPUTS__":
        return None
    if name.startswith(INTERCHANGE_PREFIXES) or name == "__FLT_EVAL_METHOD_TS_18661_3__":
        return "Interchange floating characteristics follow the types the identity provides (target/interchange_floats.rs)."
    if name in {"__GCC_IEC_559", "__GCC_IEC_559_COMPLEX", "__FLT_EVAL_METHOD__", "__SCHAR_WIDTH__", "__LONG_LONG_WIDTH__", "__STDC_UTF_16__", "__STDC_UTF_32__", "__NO_INLINE__"}:
        return "Follows the options and language mode; written by the GCC identity's macros."
    if name in {"__VERSION__", "__FLT_IS_IEC_60559__", "__DBL_IS_IEC_60559__", "__LDBL_IS_IEC_60559__"}:
        return "Part of the GCC identity claim, written beside the identity macros."
    if name.startswith("__DEC") and name != "__DECIMAL_DIG__" or name == "__DECIMAL_BID_FORMAT__":
        return "Decimal floating types are not implemented."
    if name == "__SIZEOF_FLOAT80__":
        return "The __float80 type is not implemented."
    if name == "__HAVE_SPECULATION_SAFE_VALUE":
        return "__builtin_speculation_safe_value is not implemented."
    if name in {"__ATOMIC_HLE_ACQUIRE", "__ATOMIC_HLE_RELEASE"}:
        return "Hardware lock elision memory-order bits are not implemented."
    return clang_exclusion(name)


def listing(cc1, mode, options):
    result = subprocess.run(
        [str(cc1), "-E", "-dM", "-quiet", "-nostdinc", f"-std={mode}", "-mtune=generic", "-march=x86-64", *options, "/dev/null"],
        text=True, capture_output=True, check=True,
    )
    return sorted(result.stdout.splitlines())


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--cc1", required=True, type=Path, help="GCC 14.2's cc1 for x86_64-linux-gnu")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    version = subprocess.run([str(args.cc1), "-E", "-dM", "-quiet", "/dev/null"], text=True, capture_output=True, check=True).stdout
    if f'#define __VERSION__ "{GCC_VERSION}' not in version:
        sys.exit(f"{args.cc1} is not GCC {GCC_VERSION}")
    excluded = {}
    outputs = {}
    for triple, options in TARGETS:
        for mode in ("c11", "gnu17"):
            kept = []
            for line in listing(args.cc1, mode, options):
                name = re.match(r"#define (\w+)", line)[1]
                reason = exclusion(name)
                if reason:
                    excluded[name] = reason
                else:
                    kept.append(line.rstrip())
            outputs[ROOT / f"src/target/gcc/{triple}-{mode}.h"] = "\n".join(kept) + "\n"
    outputs[ROOT / "src/target/gcc/macro-exclusions.tsv"] = "".join(f"{name}\t{reason}\n" for name, reason in sorted(excluded.items()))
    for path, contents in outputs.items():
        if args.check:
            assert path.read_text(encoding="utf-8") == contents, f"GCC macro drift: {path}"
        else:
            path.parent.mkdir(exist_ok=True)
            path.write_text(contents, encoding="utf-8", newline="\n")
    print(f"GCC {GCC_VERSION} macro contract: {len(TARGETS)} targets x 2 modes ({len(excluded)} documented exclusions)")


if __name__ == "__main__":
    main()
