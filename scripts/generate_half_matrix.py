"""Generate the floating conversion matrix and its per-compiler expectations.

Writes tests/fixtures/programs/half_matrix.c, a C program that converts
values between every pair of the arithmetic types below through `+`, both
orders of `?:` and casts, and prints each result's bits, and
half_matrix.{gcc,clang}.expected, the lines the host GCC (13 or later) and
the bundled Clang print. A line the compiler's -O0 and -O2 builds print
differently becomes `? TAG`, which matches any value.
tests/integration/half_precision.rs compares bcc's output under each
identity with these files.

The values sit at the destinations' rounding ties and range limits: an
integer 2**top + 2**(top - p) + 1 is a tie at precision p that only its last
bit decides, and the half and binary32 values straddle binary16's range and
precision. Each operand appears as a volatile object (computed at run time),
a folded constant and, for the half types, an excess-precision expression.
Conversions from a floating value to an integer type it does not fit are
undefined and left out.

Usage: python3 scripts/generate_half_matrix.py [--clang PATH] [--gcc PATH]
"""
import argparse
from fractions import Fraction
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "tests" / "fixtures" / "programs"


class Type:
    def __init__(self, tag, spelling, kind, bits=0, signed=False, precision=0, max_exp=0, min_exp=0):
        self.tag = tag
        self.spelling = spelling
        self.kind = kind  # "int", "real" or "complex"
        self.bits = bits
        self.signed = signed
        self.precision = precision
        self.max_exp = max_exp
        self.min_exp = min_exp

    @property
    def half(self):
        return self.tag in ("bf16", "f16", "cf16")

    def maximum(self):
        if self.kind == "int":
            return 2 ** (self.bits - 1) - 1 if self.signed else 2 ** self.bits - 1
        return Fraction(2) ** self.max_exp * (1 - Fraction(1, 2 ** self.precision))


INTS = [
    Type("i32", "int", "int", 32, True),
    Type("u32", "unsigned", "int", 32, False),
    Type("i64", "long long", "int", 64, True),
    Type("u64", "unsigned long long", "int", 64, False),
    Type("i128", "__int128", "int", 128, True),
    Type("u128", "unsigned __int128", "int", 128, False),
]
REALS = [
    Type("bf16", "__bf16", "real", precision=8, max_exp=128, min_exp=-125),
    Type("f16", "_Float16", "real", precision=11, max_exp=16, min_exp=-13),
    Type("f32", "float", "real", precision=24, max_exp=128, min_exp=-125),
    Type("f64", "double", "real", precision=53, max_exp=1024, min_exp=-1021),
    Type("f80", "long double", "real", precision=64, max_exp=16384, min_exp=-16381),
    Type("f128", "__float128", "real", precision=113, max_exp=16384, min_exp=-16381),
]
COMPLEX = [
    Type("cf16", "_Complex _Float16", "complex", precision=11, max_exp=16, min_exp=-13),
    Type("cf32", "_Complex float", "complex", precision=24, max_exp=128, min_exp=-125),
    Type("cf64", "_Complex double", "complex", precision=53, max_exp=1024, min_exp=-1021),
]
TYPES = INTS + REALS + COMPLEX
PRECISIONS = (8, 11, 24, 53, 64, 113)


def integer_values(ty):
    """Ties at each narrower precision, decided by the last bit, and for a
    signed type their negations."""
    top = ty.bits - 2 if ty.signed else ty.bits - 1
    values = [2 ** top + 2 ** (top - p) + 1 for p in PRECISIONS if p < top - 1]
    values.append(65520)
    if ty.signed:
        values += [-value for value in values[:2]] + [-65520]
    return values


def real_values(ty):
    one = Fraction(1)
    candidates = {
        "bf16": [Fraction(2) ** 20, one + Fraction(1, 2 ** 7), Fraction(2) ** -133],
        "f16": [Fraction(65504), one + Fraction(1, 2 ** 10), Fraction(2) ** -24],
        "f32": [one + Fraction(1, 2 ** 8) + Fraction(1, 2 ** 23), Fraction(65520), Fraction(2) ** -25 * 3],
        "f64": [one + Fraction(1, 2 ** 24) + Fraction(1, 2 ** 52), Fraction(2) ** 100 + Fraction(2) ** 76],
        "f80": [one + Fraction(1, 2 ** 53) + Fraction(1, 2 ** 63)],
        "f128": [one + Fraction(1, 2 ** 64) + Fraction(1, 2 ** 112), one + Fraction(1, 2 ** 8) + Fraction(1, 2 ** 112)],
    }
    values = candidates[ty.tag if ty.kind == "real" else ty.tag[1:]]
    # The negations of the first two, and negative zero.
    return values + [-value for value in values[:2]] + [NEGATIVE_ZERO]


class NegativeZero(Fraction):
    """-0.0, which a Fraction cannot tell from 0."""


NEGATIVE_ZERO = NegativeZero(0)


def hexfloat(value, suffix):
    """An exact hexadecimal spelling of a dyadic rational."""
    if isinstance(value, NegativeZero):
        return f"-0x0p0{suffix}"
    sign = "-" if value < 0 else ""
    value = abs(value)
    numerator, denominator = value.numerator, value.denominator
    exponent = 0
    while denominator > 1:
        denominator //= 2
        exponent -= 1
    return f"{sign}0x{numerator:x}p{exponent}{suffix}"


def constant(ty, value, imag=None):
    """A folded constant of type `ty`: a cast of an exact literal."""
    if ty.kind == "int":
        if value < 0:
            return f"(-{constant(ty, -value)})"
        if value >= 2 ** 63:
            high, low = divmod(value, 2 ** 64)
            return f"(({ty.spelling})((((unsigned __int128){high:#x}ull) << 64) | {low:#x}ull))"
        return f"(({ty.spelling}){value:#x}ull)"
    literal = hexfloat(value, "Q")
    if ty.kind == "complex" and imag is not None:
        part = TYPE_BY_TAG[{"cf16": "f16", "cf32": "f32", "cf64": "f64"}[ty.tag]].spelling
        imag_literal = hexfloat(imag, "Q")
        return f"__builtin_complex(({part}){literal}, ({part}){imag_literal})"
    # A folded complex constant is a cast of a real literal, which GCC
    # folds as it parses it unless the literal is negated.
    return f"(({ty.spelling}){literal})"


TYPE_BY_TAG = {ty.tag: ty for ty in TYPES}


def operands(ty):
    """(label, declaration, expression, exact value) for each operand of `ty`."""
    result = []
    values = integer_values(ty) if ty.kind == "int" else real_values(ty)
    for index, value in enumerate(values):
        imag = Fraction(1, 2) if ty.kind == "complex" else None
        name = f"v_{ty.tag}_{index}"
        result.append((f"{ty.tag}#{index}.var", f"volatile {ty.spelling} {name} = {constant(ty, value, imag)};", name, value))
        result.append((f"{ty.tag}#{index}.const", None, constant(ty, value), value))
        if ty.half and value > 0:
            result.append((f"{ty.tag}#{index}.excess", None, f"({name} + zero_{ty.tag})", value))
    return result


def representable(ty, value):
    """Whether converting `value` to the integer type `ty` is defined: its
    truncation is in range (C99 6.3.1.4 paragraph 1)."""
    truncated = int(value)
    minimum = -(2 ** (ty.bits - 1)) if ty.signed else 0
    return minimum <= truncated <= ty.maximum()


def program():
    lines = [
        "/* Generated by scripts/generate_half_matrix.py; do not edit. */",
        "#include <stdio.h>",
        "#include <string.h>",
        "volatile int one = 1;",
    ]
    for ty in TYPES:
        if ty.half or ty.kind == "complex" and ty.tag == "cf16":
            lines.append(f"volatile {ty.spelling} zero_{ty.tag} = 0;")
    lines += [
        "static void bytes(const char *tag, const void *p, unsigned n) {",
        "    const unsigned char *b = p;",
        "    printf(\"%s \", tag);",
        "    for (unsigned i = n; i-- > 0;)",
        "        printf(\"%02x\", b[i]);",
        "    printf(\"\\n\");",
        "}",
    ]
    for ty in TYPES:
        size = {"i32": 4, "u32": 4, "i64": 8, "u64": 8, "i128": 16, "u128": 16, "bf16": 2, "f16": 2,
                "f32": 4, "f64": 8, "f80": 10, "f128": 16}.get(ty.tag)
        if ty.kind == "complex":
            part = {"cf16": 2, "cf32": 4, "cf64": 8}[ty.tag]
            lines += [
                f"static void p_{ty.tag}(const char *tag, {ty.spelling} v) {{",
                "    unsigned char b[32];",
                "    memcpy(b, &v, sizeof v);",
                f"    bytes(tag, b, {part});",
                f"    bytes(tag, b + {part}, {part});",
                "}",
            ]
        else:
            lines += [
                f"static void p_{ty.tag}(const char *tag, {ty.spelling} v) {{",
                "    unsigned char b[16];",
                "    memcpy(b, &v, sizeof v);",
                f"    bytes(tag, b, {size});",
                "}",
            ]
    declarations = []
    bodies = []
    all_operands = {ty.tag: operands(ty) for ty in TYPES}
    for ty in TYPES:
        for _, declaration, _, _ in all_operands[ty.tag]:
            if declaration:
                declarations.append(declaration)
    calls = []
    for source in TYPES:
        for index, (label, _, expression, value) in enumerate(all_operands[source.tag]):
            # One function per operand keeps each function small.
            function = f"from_{source.tag}_{index}"
            calls.append(f"    {function}();")
            body = [f"static void {function}(void) {{"]
            for target in TYPES:
                # A cast to every floating type, and to every integer type a
                # floating value converts to defined.
                if target.kind != "int" or source.kind != "int" and representable(target, value):
                    body += emit(f"({target.tag}){label}", target, f"({target.spelling}){expression}")
                # `+` and both orders of `?:` with the first operand of the
                # other type; the result has the common type.
                partner = all_operands[target.tag][0]
                if source.kind != "int" or target.kind != "int":
                    result = result_type(source, target)
                    if result is None:
                        continue
                    body += emit(f"{label}+{partner[0]}", result, f"{expression} + {partner[2]}")
                    body += emit(f"{label}?{partner[0]}", result, f"one ? {expression} : {partner[2]}")
                    body += emit(f"{partner[0]}?{label}", result, f"one ? {partner[2]} : {expression}")
            body.append("}")
            bodies.append("\n".join(body))
    lines += declarations
    lines += bodies
    lines.append("int main(void) {")
    lines += calls
    lines += ["    return 0;", "}"]
    return "\n".join(lines) + "\n"


def emit(tag, ty, expression):
    """Statements printing `expression` of type `ty`: its bits, and for a
    half type also its value widened to a wider type, which shows the
    excess precision a compiler keeps where an argument would round it."""
    lines = [f"    p_{ty.tag}(\"{tag}\", {expression});"]
    if ty.tag in ("bf16", "f16"):
        lines.append(f"    p_f80(\"{tag}~\", (long double)({expression}));")
    elif ty.tag == "cf16":
        lines.append(f"    p_cf64(\"{tag}~\", (_Complex double)({expression}));")
    return lines


RANK = {"bf16": 0, "f16": 1, "f32": 2, "f64": 3, "f80": 4, "f128": 5}


def result_type(a, b):
    """The usual arithmetic conversions' type for a floating operand."""
    def real_tag(ty):
        if ty.kind == "int":
            return None
        return {"cf16": "f16", "cf32": "f32", "cf64": "f64"}.get(ty.tag, ty.tag)
    tags = [t for t in (real_tag(a), real_tag(b)) if t]
    real = max(tags, key=RANK.__getitem__)
    if a.kind == "complex" or b.kind == "complex":
        complex_tag = {"f16": "cf16", "f32": "cf32", "f64": "cf64"}.get(real)
        if complex_tag is None:
            return None
        return TYPE_BY_TAG[complex_tag]
    return TYPE_BY_TAG[real]


def run(compiler, source, optimization):
    with tempfile.TemporaryDirectory() as directory:
        executable = Path(directory) / "matrix"
        subprocess.run([*compiler, "-std=gnu17", optimization, "-ffp-contract=off", "-w", str(source),
                        "-o", str(executable)], check=True)
        return subprocess.run([str(executable)], check=True, capture_output=True, text=True).stdout.splitlines()


def stable(compiler, source):
    """The -O0 build's lines, each one the -O2 build prints differently
    replaced by `? TAG`, which matches any value."""
    unoptimized = run(compiler, source, "-O0")
    optimized = run(compiler, source, "-O2")
    assert len(unoptimized) == len(optimized)
    return [line if line == other else "? " + line.split(" ")[0] for line, other in zip(unoptimized, optimized)]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--clang", default=str(ROOT / "target" / "llvm" / "bin" / "clang"))
    parser.add_argument("--gcc", default="gcc")
    arguments = parser.parse_args()
    source = FIXTURES / "half_matrix.c"
    source.write_text(program())
    for name, compiler in (("clang", [arguments.clang]), ("gcc", [arguments.gcc])):
        lines = stable(compiler, source)
        (FIXTURES / f"half_matrix.{name}.expected").write_text("\n".join(lines) + "\n")
        wildcards = sum(line.startswith("? ") for line in lines)
        print(f"{name}: {len(lines)} lines, {wildcards} unstable")


if __name__ == "__main__":
    main()
