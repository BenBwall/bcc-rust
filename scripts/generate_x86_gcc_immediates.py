"""Measure GCC's limits on the immediates of the x86 builtins bcc's headers call.

`src/builtins/x86.rs` records Clang's limits (`generate_x86_builtins.py`).
GCC checks the same builtins with limits and words of its own when it
expands a call (`ix86_expand_builtin` and its helpers in GCC's
`i386-expand.cc`). This script calls each builtin of that registry that has
an immediate with every value of a probe set, compiles the calls with GCC at
`-O0`, and records, per immediate, the values GCC accepts and the message it
gives for the others and for an operand that is no constant. The result is `src/builtins/x86_gcc_immediates.rs`;
`semantic_analysis/builtins/x86_builtins.rs` applies it under GCC's rules.

Usage: python3 scripts/generate_x86_gcc_immediates.py [--check] [GCC driver and flags...]
The driver defaults to `gcc-14`; the script refuses a compiler whose
`__GNUC__` is not 14. Pass the -B and -isystem options that select GCC 14's
`cc1` and headers when the installed driver is older. `--check` compares
the result with the checked-in file instead of writing it.

A builtin GCC lacks, or whose GCC form takes other operand types than the
registry's Clang signature (GCC's `__builtin_ia32_palignr128` takes two
vectors of 64-bit lanes and a count in bits), gets no row; the script lists
both kinds in the generated file. An error that is neither of these nor one
of GCC's immediate messages stops the script.
"""
import argparse
import re
import subprocess
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
# GCC's words for an immediate it refuses (`ix86_expand_builtin` and its
# helpers in GCC's `i386-expand.cc`).
IMMEDIATE = re.compile(r"(the \w+(?: to last)? argument must be an? \d+-bit immediate"
                       r"|selector must be an integer constant in the range \[0, \d+\])$")
MISSING = "implicit declaration of function"
INCOMPATIBLE = re.compile(r"incompatible type for argument \d+ of")
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
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true",
                        help="fail if src/builtins/x86_gcc_immediates.rs differs")
    parser.add_argument("gcc", nargs=argparse.REMAINDER, help="GCC 14 driver and flags")
    options = parser.parse_args()
    gcc = options.gcc or ["gcc-14"]
    version = gcc_version(gcc)
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
    missing, incompatible = [], []
    rejected = {}
    with tempfile.TemporaryDirectory() as scratch:
        path = Path(scratch) / "immediates.c"
        for index, (lines, cases) in enumerate(probes):
            name = rows[index][1]
            # One builtin at a time: GCC expands no function, and so checks
            # no immediate, once the translation unit has another error.
            path.write_text("\n".join(prefix + lines) + "\n")
            run = subprocess.run([*gcc, *FEATURES, "-O0", "-fmax-errors=0", "-w", "-S", "-o",
                                  "/dev/null", str(path)], capture_output=True, text=True)
            errors = []
            for line in run.stderr.splitlines():
                m = re.match(r".*immediates\.c:(\d+):\d+: error: (.*)", line)
                if m:
                    errors.append((int(m[1]) - len(prefix), m[2].replace("\u2018", "'")
                                   .replace("\u2019", "'")))
                elif re.search(r"\b(error|fatal error|internal compiler error):", line):
                    raise SystemExit("GCC failed on the probe of %s:\n%s" % (name, run.stderr))
            if run.returncode != 0 and not errors:
                raise SystemExit("GCC failed on the probe of %s:\n%s" % (name, run.stderr))
            messages = [message for _, message in errors]
            if any(MISSING in message for message in messages):
                missing.append(name)
                continue
            if any(INCOMPATIBLE.match(message) for message in messages):
                incompatible.append(name)
                continue
            for line, message in errors:
                if line not in cases or not IMMEDIATE.match(message):
                    raise SystemExit("unexpected GCC error on the probe of %s: %s"
                                     % (name, message))
                j, value = cases[line]
                rejected.setdefault((index, j), {})[value] = message
    entries = []
    for index, (identity, name, _, _, immediates) in enumerate(rows):
        if name in missing or name in incompatible:
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
    text = (HEADER.replace("{version}", version).replace("{missing}", listing(missing))
            .replace("{incompatible}", listing(incompatible)) + rows_text + "\n];\n")
    if options.check:
        if OUT.read_text() != text:
            raise SystemExit("%s differs from GCC %s's limits; run "
                             "scripts/generate_x86_gcc_immediates.py" % (OUT.relative_to(ROOT), version))
    else:
        OUT.write_text(text)
    print("Measured %d immediates of %d builtins with GCC %s; GCC lacks %d builtins and takes %d "
          "with other operand types (%s)." % (len(entries), len(rows) - len(missing) - len(incompatible),
                                               version, len(missing), len(incompatible),
                                               ", ".join(incompatible)))


def gcc_version(gcc):
    """`__GNUC__.__GNUC_MINOR__` of the compiler `gcc` runs, which must be 14."""
    try:
        run = subprocess.run([*gcc, "-E", "-P", "-x", "c", "-"], input="__GNUC__ __GNUC_MINOR__\n",
                             capture_output=True, text=True)
    except OSError as error:
        raise SystemExit("cannot run %s: %s" % (" ".join(gcc), error)) from error
    numbers = run.stdout.split()
    if run.returncode != 0 or len(numbers) != 2 or not all(n.isdigit() for n in numbers):
        raise SystemExit("cannot run %s:\n%s" % (" ".join(gcc), run.stderr))
    if numbers[0] != "14":
        raise SystemExit("%s is GCC %s; the table records GCC 14's limits"
                         % (" ".join(gcc), ".".join(numbers)))
    return ".".join(numbers)


def listing(names):
    """`names` as the generated file's comment lists them, wrapped."""
    lines, line = [], "//!"
    for word in ["`%s`%s" % (name, "," if k + 1 < len(names) else ".")
                 for k, name in enumerate(names)] or ["none."]:
        if len(line) + 1 + len(word) > 79:
            lines.append(line)
            line = "//!"
        line += " " + word
    return "\n".join(lines + [line])


HEADER = '''//! Generated by `scripts/generate_x86_gcc_immediates.py` from GCC {version};
//! do not edit. Regenerate after `scripts/generate_x86_builtins.py`.
//!
//! GCC's limits on the immediates of the x86 builtins in `x86.rs`, which
//! differ from Clang's there: GCC checks a builtin's immediate when it
//! expands the call, with the words of `ix86_expand_builtin` and its helpers
//! (GCC's `i386-expand.cc`). Each row is a builtin, the index of its
//! immediate operand, the values GCC accepts as the registry's `imm` reads
//! them (`[low, high]` and, with a nonzero mask, a set bit of `mask`), and
//! GCC's message for the others and for an operand that is no constant.
//!
//! A builtin GCC lacks has no row and keeps Clang's limits:
{missing}
//!
//! Nor has a builtin whose GCC form takes other operand types than Clang's,
//! which the probe cannot call with the registry's signature; it too keeps
//! Clang's limits:
{incompatible}
//!
//! GNU extension: GCC manual, "x86 Built-in Functions".
//! <https://gcc.gnu.org/onlinedocs/gcc/x86-Built-in-Functions.html>
use super::BuiltinId as Id;

/// GCC's limits on operand `operand` of builtin `id`, and its message for a
/// value outside them or no constant; `None` for a builtin without a row.
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
