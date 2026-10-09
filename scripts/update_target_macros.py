"""Freeze the pinned Clang target contract; never invokes a system Clang."""
import argparse
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
TRIPLES = ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl", "x86_64-w64-windows-gnu", "x86_64-pc-windows-msvc")


def exclusion(name):
    if name.startswith(("__clang", "__GNUC")) or name in {"__VERSION__", "__llvm__", "_MSC_VER", "_MSC_FULL_VER", "_MSC_BUILD", "_MSC_EXTENSIONS", "__STDC_HOSTED__"}:
        return "Compiler identity/version or hosted-mode contract owned separately."
    if name in {"__STDC__", "__STDC_VERSION__", "__STRICT_ANSI__"} or name.startswith("__STDC_EMBED_"):
        return "Existing language-mode preprocessor builtin; not a target definition."
    if name in {"__STDC_UTF_16__", "__STDC_UTF_32__"}:
        return "Encoded literal syntax exists, but its semantic types and storage are not implemented."
    if name.startswith(("__FLT16_", "__BF16_")):
        return "Half-precision floating types and arithmetic are not implemented."
    if name in {"__GXX_ABI_VERSION", "__GXX_TYPEINFO_EQUALITY_INLINE"}:
        return "C++ ABI and type-info semantics are not implemented."
    if name == "__GCC_HAVE_DWARF2_CFI_ASM":
        return "DWARF CFI assembly and unwind code generation are not implemented."
    if name.startswith(("__MEMORY_SCOPE_", "__OPENCL_MEMORY_SCOPE_")):
        return "Scoped atomic operations and memory scopes are not implemented."
    if name.startswith("__FPCLASS_"):
        return "Floating classification intrinsics are not implemented."
    if name in {"__BITINT_MAXWIDTH__"}:
        return "Extended integer syntax exists but these widths lack semantic types."
    if name in {"__MMX__", "__SSE__", "__SSE2__", "__SSE_MATH__", "__SSE2_MATH__", "__FXSR__", "__GCC_ASM_FLAG_OUTPUTS__", "__GCC_CONSTRUCTIVE_SIZE", "__GCC_DESTRUCTIVE_SIZE", "__SEG_FS", "__SEG_GS", "__seg_fs", "__seg_gs"}:
        return "Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented."
    if name in {"__PIC__", "__pic__", "__PIE__", "__pie__", "__code_model_small__", "__NO_INLINE__", "__NO_MATH_ERRNO__", "__tune_k8__"}:
        return "Code generation, relocation and optimization policy are not implemented."
    if name in {"__CONSTANT_CFSTRINGS__", "__OBJC_BOOL_IS_BOOL"}:
        return "Objective-C and CoreFoundation string intrinsics are not implemented."
    if name in {"__PRAGMA_REDEFINE_EXTNAME", "_MSVC_CONSTEXPR_ATTRIBUTE"}:
        return "The advertised pragma/attribute semantics are not implemented."
    if name == "_MSVC_TRADITIONAL":
        return "Traditional Microsoft preprocessing is not implemented; bcc uses its conforming preprocessor."
    if name in {"_M_FP_CONTRACT", "_M_FP_PRECISE"}:
        return "MS floating code-generation modes are not implemented."
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    clang = ROOT / "target/llvm/bin/clang.exe"
    if not clang.exists():
        clang = ROOT / "target/llvm/bin/clang"
    excluded = {}
    outputs = {}
    for triple in TRIPLES:
        c23 = subprocess.run([str(clang), f"--target={triple}", "-std=c23", "-dM", "-E", "-x", "c", "-"], input="", text=True, capture_output=True, check=True)
        char8 = "\n".join(line for line in c23.stdout.splitlines() if re.match(r"#define __(?:CLANG|GCC)_ATOMIC_CHAR8_T_LOCK_FREE ", line)) + "\n"
        char8_path = ROOT / f"src/target/{triple}-atomic-c23.h"
        outputs[char8_path] = char8
        for mode in ("c11", "gnu17"):
            result = subprocess.run([str(clang), f"--target={triple}", f"-std={mode}", "-dM", "-E", "-x", "c", "-"], input="", text=True, capture_output=True, check=True)
            kept = []
            for line in result.stdout.splitlines():
                name = re.match(r"#define (\w+)", line)[1]
                reason = exclusion(name)
                if reason:
                    excluded[name] = reason
                else:
                    kept.append(line.rstrip())
            outputs[ROOT / f"src/target/{triple}-{mode}.h"] = "\n".join(kept) + "\n"
    outputs[ROOT / "src/target/macro-exclusions.tsv"] = "".join(f"{name}\t{reason}\n" for name, reason in sorted(excluded.items()))
    for path, contents in outputs.items():
        if args.check:
            assert path.read_text(encoding="utf-8") == contents, f"Clang macro drift: {path}"
        else:
            path.write_text(contents, encoding="utf-8", newline="\n")
    print(f"Clang macro contract: {len(TRIPLES)} triples x 2 modes match ({len(excluded)} documented exclusions)")


if __name__ == "__main__":
    main()
