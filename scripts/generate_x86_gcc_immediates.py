"""Measure GCC's limits on the immediates of the x86 builtins bcc's headers call.

`src/builtins/x86.rs` records Clang's limits (`generate_x86_builtins.py`).
GCC checks the same builtins with limits and words of its own when it
expands a call (`ix86_expand_builtin` and its helpers in GCC's
`i386-expand.cc`). This script calls each builtin of that registry that has
an immediate with every value of a probe set, compiles the calls with GCC at
`-O0`, and records, per immediate, the values GCC accepts and the message it
gives for the others and for an operand that is no constant. The result is `src/builtins/x86_gcc_immediates.rs`;
`semantic_analysis/builtins/x86_builtins.rs` applies it under GCC's rules.

Usage: python3 scripts/generate_x86_gcc_immediates.py [GCC driver and flags...]
The driver defaults to `gcc`. Pass the -B and -isystem options that select
GCC 14 when the installed driver is older.
"""
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "src/builtins/x86.rs"
OUT = ROOT / "src/builtins/x86_gcc_immediates.rs"
FEATURES = ["-mavx2", "-mfma", "-maes", "-mpclmul", "-msse4.2", "-mxsave", "-mpopcnt",
            "-mf16c", "-mbmi", "-mbmi2", "-mlzcnt", "-madx", "-mprfchw", "-mcrc32"]
# Every value from -1 to 256, then values far outside a byte.
VALUES = [-2147483648, -65536, -129] + list(range(-1, 257)) + [4096, 65536, 2147483647]
SCALARS = {"VOID": "void", "CHAR": "char", "SCHAR": "signed char", "UCHAR": "unsigned char",
           "SHORT": "short", "USHORT": "unsigned short", "INT": "int", "UINT": "unsigned int",
           "LONG": "long", "ULONG": "unsigned long", "LLONG": "long long",
           "ULLONG": "unsigned long long", "FLOAT": "float", "DOUBLE": "double"}
SIZES = {"char": 1, "signed char": 1, "unsigned char": 1, "short": 2, "unsigned short": 2,
         "int": 4, "unsigned int": 4, "long long": 8, "unsigned long long": 8, "float": 4,
         "double": 8}
ROW = re.compile(r'^\s*x86\(Id::(\w+), "(\w+)", (\w+), &\[([^\]]*)\], &\[([^\]]*)\]\),', re.M)


def c_type(name, typedefs):
    """The C spelling of a registry type alias, defining vector typedefs."""
    pointer = name.startswith("P_")
    rest = name[2:] if pointer else name
    constant = rest.startswith("C_")
    rest = rest[2:] if constant else rest
    vector = re.match(r"V(\d+)_(\w+)$", rest)
    if vector:
        scalar = SCALARS[vector[2]]
        base = "v%s_%s" % (vector[1], vector[2].lower())
        typedefs.setdefault(base, "typedef %s %s __attribute__((vector_size(%d)));"
                            % (scalar, base, int(vector[1]) * SIZES[scalar]))
    else:
        base = SCALARS[rest]
    return ("const " if constant else "") + base + (" *" if pointer else "")


def accepted_limits(accepted):
    """`(low, high, mask)` for an accepted set, as the registry's `imm` reads it."""
    if accepted == set(VALUES):
        return (-2147483648, 2147483647, 0)
    if not accepted:
        raise RuntimeError("no value accepted")
    low, high = min(accepted), max(accepted)
    if all(v in accepted for v in VALUES if low <= v <= high):
        return (low, high, 0)
    if high < 16:
        return (low, high, sum(1 << v for v in accepted))
    raise RuntimeError("accepted set is neither a range nor a mask: %s" % sorted(accepted))


def main():
    gcc = sys.argv[1:] or ["gcc"]
    rows = []
    for m in ROW.finditer(REGISTRY.read_text()):
        rules = [r.strip() for r in re.split(r",\s*(?![^()]*\))", m[5])]
        immediates = [j for j, r in enumerate(rules) if r.startswith("imm(")]
        if immediates:
            rows.append((m[1], m[2], m[3], [p.strip() for p in m[4].split(",")], immediates))
    typedefs = {}
    probes = []
    for index, (_, name, result, params, immediates) in enumerate(rows):
        declarations = ", ".join("%s p%d" % (c_type(p, typedefs), j) for j, p in enumerate(params))
        lines, cases = [], {}
        for j in immediates:
            # Each value of the probe set, then the parameter, no constant.
            for value in VALUES + [None]:
                args = [("p%d" % k if value is None else str(value)) if k == j
                        else "1" if k in immediates else "p%d" % k for k in range(len(params))]
                call = "%s(%s)" % (name, ", ".join(args))
                # GCC drops an unused call before it expands it, and checks
                # the immediate only then, so each probe returns the result.
                returned = "void" if result == "VOID" else c_type(result, typedefs)
                body = "%s;" % call if result == "VOID" else "return (%s)%s;" % (returned, call)
                lines.append("%s probe_%d_%d(%s) { %s }" % (returned, j, len(lines), declarations, body))
                cases[len(lines)] = (j, value)
        probes.append((lines, cases))
    prefix = sorted(typedefs.values())
    absent = set()
    rejected = {}
    with tempfile.TemporaryDirectory() as scratch:
        path = Path(scratch) / "immediates.c"
        for index, (lines, cases) in enumerate(probes):
            # One builtin at a time: GCC expands no function, and so checks
            # no immediate, once the translation unit has another error.
            path.write_text("\n".join(prefix + lines) + "\n")
            run = subprocess.run([*gcc, *FEATURES, "-O0", "-fmax-errors=0", "-w", "-S", "-o",
                                  "/dev/null", str(path)], capture_output=True, text=True)
            for m in re.finditer(r"immediates\.c:(\d+):\d+: error: (.*)", run.stderr):
                j, value = cases[int(m[1]) - len(prefix)]
                message = m[2].replace("\u2018", "'").replace("\u2019", "'")
                if "implicit declaration" in message or "incompatible" in message:
                    absent.add(index)
                    break
                rejected.setdefault((index, j), {})[value] = message
    entries = []
    for index, (identity, name, _, _, immediates) in enumerate(rows):
        if index in absent:
            continue
        for j in immediates:
            refused = rejected.get((index, j), {})
            messages = sorted(set(refused.values()))
            assert len(messages) == 1, (name, j, messages)
            refused.pop(None)
            low, high, mask = accepted_limits(set(VALUES) - set(refused))
            entries.append((identity, j, low, high, mask, messages[0] if messages else ""))
    rows_text = "\n".join('    (Id::%s, %d, %s, %s, %d, "%s"),' % (identity, j, f"{low:_}", f"{high:_}", mask, message)
                          for identity, j, low, high, mask, message in entries)
    OUT.write_text(HEADER + rows_text + "\n];\n")
    print("Measured %d immediates of %d builtins; GCC lacks %d builtins."
          % (len(entries), len(rows) - len(absent), len(absent)))


HEADER = '''//! Generated by `scripts/generate_x86_gcc_immediates.py` from GCC 14.2;
//! do not edit. Regenerate after `scripts/generate_x86_builtins.py`.
//!
//! GCC's limits on the immediates of the x86 builtins in `x86.rs`, which
//! differ from Clang's there: GCC checks a builtin's immediate when it
//! expands the call, with the words of `ix86_expand_builtin` and its helpers
//! (GCC's `i386-expand.cc`). Each row is a builtin, the index of its
//! immediate operand, the values GCC accepts as the registry's `imm` reads
//! them (`[low, high]` and, with a nonzero mask, a set bit of `mask`), and
//! GCC's message for the others and for an operand that is no constant. A
//! builtin GCC lacks has no row.
//!
//! GNU extension: GCC manual, "x86 Built-in Functions".
//! <https://gcc.gnu.org/onlinedocs/gcc/x86-Built-in-Functions.html>
use super::BuiltinId as Id;

/// GCC's limits on operand `operand` of builtin `id`, and its message for a
/// value outside them or no constant; `None` for a builtin GCC lacks.
pub(crate) fn gcc_immediate(id: Id, operand: usize) -> Option<(i32, i32, u16, &'static str)> {
    GCC_IMMEDIATES
        .iter()
        .find(|row| row.0 == id && usize::from(row.1) == operand)
        .map(|&(_, _, low, high, mask, message)| (low, high, mask, message))
}

/// `(builtin, operand, low, high, mask, message)`.
#[rustfmt::skip]
const GCC_IMMEDIATES: &[(Id, u8, i32, i32, u16, &str)] = &[
'''

if __name__ == "__main__":
    main()
