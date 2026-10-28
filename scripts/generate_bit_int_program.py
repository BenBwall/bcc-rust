"""Generate the bit-precise integer execution fixture.

Writes tests/fixtures/programs/bit_int.c, a C23 program that runs every
operator of `_BitInt(N)` and `unsigned _BitInt(N)` for widths 1 (unsigned
only), 2, 7, 8, 15, 63, 64, 65, 127 and 128 over a table of operand pairs,
converts each type's values to and from every standard integer and real
floating type and to every other width, folds a sample of constant
expressions, and prints each result's bits in hexadecimal.
tests/integration/bit_int.rs compares bcc's output on every back end with
what the bundled Clang and the host GCC (14 or later) build of it prints.

Operands come from a table, read through a volatile object, so the
arithmetic happens at run time; the folded section writes the same
operations with constants. The values are each type's bounds, 0, 1, -1 and
pseudo-random patterns (a fixed seed). An operation whose behavior is
undefined (signed overflow, division by zero, a shift count outside the
width, a negative value shifted left, a floating value out of range) is
left out.

WebAssembly has no host compiler to compare with at test time, so
`--wasi-sdk PATH` also writes bit_int.wasm32.expected: what the program
built by that wasi-sdk's Clang for wasm32-wasip1 prints under Node
(scripts/wasm/run.mjs). Clang limits `_BitInt` to 128 bits there too.

Usage: python3 scripts/generate_bit_int_program.py [--wasi-sdk PATH]
"""
import argparse
import subprocess
import tempfile
from fractions import Fraction
from pathlib import Path
import random

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "tests" / "fixtures" / "programs" / "bit_int.c"

WIDTHS = [1, 2, 7, 8, 15, 63, 64, 65, 127, 128]

# Operators of the table, by code.
BINARY = [
    ("add", "a + b"),
    ("sub", "a - b"),
    ("mul", "a * b"),
    ("div", "a / b"),
    ("rem", "a % b"),
    ("and", "a & b"),
    ("or", "a | b"),
    ("xor", "a ^ b"),
    ("shl", "a << b"),
    ("shr", "a >> b"),
    ("cmp", None),
    ("add=", None),
    ("shl=", None),
]
UNARY = [
    ("neg", "-a"),
    ("not", "~a"),
    ("lnot", "!a"),
    ("inc", None),
    ("dec", None),
]

STANDARD = [
    ("bool", "bool", 1, False),
    ("char", "char", 8, True),
    ("sc", "signed char", 8, True),
    ("uc", "unsigned char", 8, False),
    ("short", "short", 16, True),
    ("ushort", "unsigned short", 16, False),
    ("int", "int", 32, True),
    ("uint", "unsigned", 32, False),
    ("long", "long", 64, True),
    ("ulong", "unsigned long", 64, False),
    ("ll", "long long", 64, True),
    ("ull", "unsigned long long", 64, False),
    ("i128", "__int128", 128, True),
    ("u128", "unsigned __int128", 128, False),
]
# Real floating types: tag, spelling, precision, largest exponent, and how
# the value prints (as `double` with %a for the narrow ones).
FLOATING = [
    ("f16", "_Float16", 11, 16),
    ("f", "float", 24, 128),
    ("d", "double", 53, 1024),
    ("ld", "long double", 64, 16384),
    ("f128", "__float128", 113, 16384),
]


class BitInt:
    def __init__(self, width, signed):
        self.width = width
        self.signed = signed
        self.tag = ("bs" if signed else "bu") + str(width)
        self.spelling = ("" if signed else "unsigned ") + f"_BitInt({width})"

    def minimum(self):
        return -(1 << (self.width - 1)) if self.signed else 0

    def maximum(self):
        return (1 << (self.width - 1)) - 1 if self.signed else (1 << self.width) - 1

    def fits(self, value):
        return self.minimum() <= value <= self.maximum()

    def wrap(self, value):
        value &= (1 << self.width) - 1
        if self.signed and value >> (self.width - 1):
            value -= 1 << self.width
        return value

    def literal(self, value):
        """A constant expression of this type with `value`."""
        if value == -(1 << 63):
            return f"({self.spelling})(-0x7fffffffffffffffLL - 1)"
        if -(1 << 63) < value < (1 << 64):
            return f"({self.spelling})({value}LL)" if value < 0 else f"({self.spelling}){value}ULL"
        pattern = value & ((1 << 128) - 1)
        high, low = pattern >> 64, pattern & ((1 << 64) - 1)
        return (
            f"({self.spelling})(((unsigned __int128){high:#x}ULL << 64) | {low:#x}ULL)"
        )


def types():
    result = []
    for width in WIDTHS:
        if width >= 2:
            result.append(BitInt(width, True))
        result.append(BitInt(width, False))
    return result


def values(ty, rng):
    """The operand values of a type: its bounds, small values and random
    patterns."""
    chosen = [0, 1, ty.maximum(), ty.minimum()]
    if ty.signed:
        chosen.append(-1)
    for _ in range(2):
        chosen.append(ty.wrap(rng.getrandbits(ty.width)))
    unique = []
    for value in chosen:
        if ty.fits(value) and value not in unique:
            unique.append(value)
    return unique


def truncating_division(a, b):
    quotient = abs(a) // abs(b)
    return quotient if (a < 0) == (b < 0) else -quotient


def defined(ty, name, a, b):
    """Whether the operation is defined for these operands."""
    if name in ("add", "add="):
        return not ty.signed or ty.fits(a + b)
    if name == "sub":
        return not ty.signed or ty.fits(a - b)
    if name == "mul":
        return not ty.signed or ty.fits(a * b)
    if name in ("div", "rem"):
        return b != 0 and (not ty.signed or ty.fits(truncating_division(a, b)))
    if name in ("shl", "shl="):
        if not 0 <= b < ty.width:
            return False
        return not ty.signed or (a >= 0 and ty.fits(a << b))
    if name == "shr":
        return 0 <= b < ty.width
    if name == "neg":
        return not ty.signed or ty.fits(-a)
    if name == "inc":
        return not ty.signed or ty.fits(a + 1)
    if name == "dec":
        return not ty.signed or ty.fits(a - 1)
    return True


def print_value(expression, width):
    """A statement printing the bits of `expression`, an integer of at
    most `width` bits, as 64-bit limbs from the top."""
    return f"P{(width + 63) // 64}({expression})"


def generate():
    rng = random.Random(0xB17)
    lines = [
        "/* Generated by scripts/generate_bit_int_program.py; do not edit. */",
        "#include <stdio.h>",
        "#include <float.h>",
        "#include <string.h>",
        "",
        "/* WebAssembly has neither _Float16 nor __float128, and a binary128",
        " * long double; x86-64's x87 long double has 10 value bytes. */",
        "#ifdef __wasm__",
        "#define HALF(x)",
        "#define QUAD(x)",
        "#else",
        "#define HALF(x) x",
        "#define QUAD(x) x",
        "#endif",
        "#define LONG_DOUBLE_BYTES (LDBL_MANT_DIG == 64 ? 10 : (int)sizeof(long double))",
        "",
        "typedef unsigned __int128 u128;",
        "static int line;",
        "static void p1(unsigned long long v) { printf(\"%d %llx\\n\", line++, v); }",
        "static void p2(u128 v) {",
        "    printf(\"%d %llx:%016llx\\n\", line++, (unsigned long long)(v >> 64), (unsigned long long)v);",
        "}",
        "#define P1(x) p1((unsigned long long)(x))",
        "#define P2(x) p2((u128)(x))",
        "static void bits(const void *p, int n) {",
        "    const unsigned char *b = p;",
        "    printf(\"%d \", line++);",
        "    for (int i = n - 1; i >= 0; i--) printf(\"%02x\", b[i]);",
        "    printf(\"\\n\");",
        "}",
        "",
    ]
    all_types = types()
    tables = {}
    for ty in all_types:
        operands = values(ty, rng)
        entries = []
        for code, (name, _) in enumerate(BINARY):
            for a in operands:
                for b in operands:
                    if name in ("shl", "shr", "shl="):
                        b = (b % ty.width) if b >= 0 else 0
                    if defined(ty, name, a, b):
                        entries.append((code, a, b))
        for offset, (name, _) in enumerate(UNARY):
            for a in operands:
                if defined(ty, name, a, 0):
                    entries.append((len(BINARY) + offset, a, 0))
        tables[ty.tag] = (operands, entries)
        t, s = ty.tag, ty.spelling
        lines.append(f"typedef {s} {t};")
        lines.append(f"static const {t} {t}_a[] = {{")
        lines.extend(f"    {ty.literal(a)}," for _, a, _ in entries)
        lines.append("};")
        lines.append(f"static const {t} {t}_b[] = {{")
        lines.extend(f"    {ty.literal(b)}," for _, _, b in entries)
        lines.append("};")
        lines.append(f"static const unsigned char {t}_op[] = {{")
        lines.append("    " + ", ".join(str(code) for code, _, _ in entries))
        lines.append("};")
        lines.append(f"static void run_{t}(void) {{")
        lines.append(f"    printf(\"{t} sizeof %zu alignof %zu\\n\", sizeof({t}), _Alignof({t}));")
        lines.append(f"    for (unsigned i = 0; i < sizeof {t}_op; i++) {{")
        lines.append(f"        volatile {t} va = {t}_a[i], vb = {t}_b[i];")
        lines.append(f"        {t} a = va, b = vb, r = 0;")
        lines.append(f"        switch ({t}_op[i]) {{")
        for code, (name, expression) in enumerate(BINARY):
            if name == "cmp":
                body = (
                    "r = (a < b) | (a <= b) << 1 | (a > b) << 2 | (a >= b) << 3"
                    " | (a == b) << 4 | (a != b) << 5"
                    if ty.width >= 7
                    else "r = (a < b) ^ (a == b)"
                )
            elif name == "add=":
                body = "r = a; r += b"
            elif name == "shl=":
                body = "r = a; r <<= b"
            else:
                body = f"r = {expression}"
            lines.append(f"        case {code}: {body}; break;")
        for offset, (name, expression) in enumerate(UNARY):
            code = len(BINARY) + offset
            if name == "inc":
                body = "r = a; r++"
            elif name == "dec":
                body = "r = a; --r"
            else:
                body = f"r = {expression}"
            lines.append(f"        case {code}: {body}; break;")
        lines.append("        }")
        lines.append(f"        {print_value('r', ty.width)};")
        lines.append("    }")
        lines.append("}")
        lines.append("")
    # Conversions to and from the standard types, and between widths.
    lines.append("static void conversions(void) {")
    for ty in all_types:
        operands, _ = tables[ty.tag]
        t = ty.tag
        lines.append(f"    printf(\"conversions {t}\\n\");")
        for value in operands:
            lines.append(f"    {{ volatile {t} v = {ty.literal(value)};")
            for _, spelling, width, _ in STANDARD:
                lines.append(f"      {print_value(f'({spelling})v', width)};")
            for _, spelling, _, _ in FLOATING:
                lines.append(
                    guard(spelling, f"{{ {spelling} f = v; bits(&f, {floating_bytes(spelling)}); }}", "      ")
                )
            for other in all_types:
                lines.append(f"      {print_value(f'({other.spelling})v', other.width)};")
            lines.append("    }")
        # From the standard integer types.
        for tag, spelling, width, signed in STANDARD:
            if tag == "bool":
                continue
            source = BitInt(width, signed)
            for value in (source.minimum(), -1 if signed else 1, source.maximum()):
                lines.append(
                    f"    {{ volatile {spelling} v = ({spelling}){literal_of(value, width, signed)};"
                    f" {print_value(f'({ty.spelling})v', ty.width)}; }}"
                )
        # From the floating types, values in range only.
        for value in floating_values(ty, rng):
            for tag, spelling, precision, max_exponent in FLOATING:
                if not representable(value, precision, max_exponent):
                    continue
                lines.append(
                    guard(
                        spelling,
                        f"{{ volatile {spelling} v = ({spelling}){float_literal(value)};"
                        f" {print_value(f'({ty.spelling})v', ty.width)}; }}",
                        "    ",
                    )
                )
    lines.append("}")
    lines.append("")
    # Folded constants: the same operations on constant operands.
    lines.append("static void folded(void) {")
    for ty in all_types:
        operands, entries = tables[ty.tag]
        lines.append(f"    printf(\"folded {ty.tag}\\n\");")
        for code, a, b in entries[:: max(1, len(entries) // 24)]:
            name = (BINARY + UNARY)[code][0]
            expression = (BINARY + UNARY)[code][1]
            if expression is None:
                continue
            text = expression.replace("a", "{a}").replace("b", "{b}").format(
                a=f"({ty.literal(a)})", b=f"({ty.literal(b)})"
            )
            lines.append(f"    {print_value(f'({ty.spelling})({text})', ty.width)}; /* {name} */")
    lines.append("}")
    lines.append("")
    lines.append(EXTRAS)
    lines.append("int main(void) {")
    for ty in all_types:
        lines.append(f"    run_{ty.tag}();")
    lines.append("    conversions();")
    lines.append("    folded();")
    lines.append("    extras();")
    lines.append("    return 0;")
    lines.append("}")
    OUTPUT.write_text("\n".join(lines) + "\n")


def floating_bytes(spelling):
    return {
        "_Float16": "2",
        "float": "4",
        "double": "8",
        "long double": "LONG_DOUBLE_BYTES",
        "__float128": "16",
    }[spelling]


def guard(spelling, statement, indent):
    """`statement`, left out where the target lacks the floating type."""
    wrapper = {"_Float16": "HALF", "__float128": "QUAD"}.get(spelling)
    return f"{indent}{wrapper}({statement})" if wrapper else f"{indent}{statement}"


def literal_of(value, width, signed):
    if value == -(1 << 63):
        return "(-0x7fffffffffffffffLL - 1)"
    if -(1 << 63) < value < (1 << 64):
        return f"({value}LL)" if value < 0 else f"{value}ULL"
    pattern = value & ((1 << 128) - 1)
    high, low = pattern >> 64, pattern & ((1 << 64) - 1)
    return f"(((unsigned __int128){high:#x}ULL << 64) | {low:#x}ULL)"


def floating_values(ty, rng):
    """Values exactly representable in binary16 and in range of `ty`, then
    larger ones that need more precision."""
    candidates = [0, 1, ty.maximum(), ty.minimum(), ty.maximum() // 3]
    if ty.signed:
        candidates.append(-1)
    result = []
    for value in candidates:
        # Truncate to 11 significant bits, toward zero, so that the value
        # is exact in every floating type that holds its magnitude.
        if value != 0:
            shift = max(0, abs(value).bit_length() - 11)
            magnitude = (abs(value) >> shift) << shift
            value = magnitude if value > 0 else -magnitude
        if ty.fits(value) and value not in result:
            result.append(value)
    return result


def representable(value, precision, max_exponent):
    return abs(value) < 2 ** (max_exponent - 1)


def float_literal(value):
    if value == 0:
        return "0.0"
    exponent = abs(value).bit_length() - 1
    mantissa = Fraction(abs(value), 2**exponent)
    sign = "-" if value < 0 else ""
    # Exact hexadecimal: 1.xxx * 2^exponent with 11 significant bits.
    fraction = int((mantissa - 1) * 2**12)
    return f"{sign}0x1.{fraction:03x}p{exponent}"


EXTRAS = r"""
/* Types of the usual arithmetic conversions and of constants. */
#define NAME(x) _Generic((x), \
    _BitInt(2): "_BitInt(2)", unsigned _BitInt(1): "unsigned _BitInt(1)", \
    unsigned _BitInt(2): "unsigned _BitInt(2)", _BitInt(3): "_BitInt(3)", \
    _BitInt(7): "_BitInt(7)", unsigned _BitInt(7): "unsigned _BitInt(7)", \
    _BitInt(8): "_BitInt(8)", _BitInt(10): "_BitInt(10)", \
    _BitInt(32): "_BitInt(32)", unsigned _BitInt(32): "unsigned _BitInt(32)", \
    _BitInt(33): "_BitInt(33)", unsigned _BitInt(33): "unsigned _BitInt(33)", \
    _BitInt(40): "_BitInt(40)", unsigned _BitInt(40): "unsigned _BitInt(40)", \
    _BitInt(64): "_BitInt(64)", unsigned _BitInt(64): "unsigned _BitInt(64)", \
    _BitInt(65): "_BitInt(65)", _BitInt(128): "_BitInt(128)", \
    unsigned _BitInt(128): "unsigned _BitInt(128)", \
    int: "int", unsigned: "unsigned", long: "long", unsigned long: "unsigned long", \
    long long: "long long", unsigned long long: "unsigned long long", \
    __int128: "__int128", unsigned __int128: "unsigned __int128", \
    float: "float", double: "double", long double: "long double", \
    default: "other")

struct fields {
    _BitInt(40) a : 10;
    unsigned _BitInt(40) b : 10;
    _BitInt(8) c : 3;
    unsigned _BitInt(65) d : 65;
    _BitInt(128) e : 100;
    unsigned _BitInt(1) f : 1;
};

static int classify(_BitInt(65) x) {
    switch (x) {
    case 0: return 0;
    case 1: return 1;
    case -1: return 2;
    case (_BitInt(65))1 << 63: return 3;
    case -((_BitInt(65))1 << 63) * 2: return 4;
    default: return 5;
    }
}

static int small(unsigned _BitInt(7) x) {
    switch (x) {
    case 0: return 10;
    case 127: return 11;
    case 64: return 12;
    default: return 13;
    }
}

static void extras(void) {
    printf("types\n");
    printf("%s\n", NAME((_BitInt(8))0 + (char)0));
    printf("%s\n", NAME((_BitInt(8))0 + (_BitInt(8))0));
    printf("%s\n", NAME((_BitInt(7))0 + (unsigned _BitInt(7))0));
    printf("%s\n", NAME((_BitInt(32))0 + 0));
    printf("%s\n", NAME((_BitInt(32))0 + 0U));
    printf("%s\n", NAME((unsigned _BitInt(32))0 + 0));
    printf("%s\n", NAME((_BitInt(33))0 + 0U));
    printf("%s\n", NAME((unsigned _BitInt(33))0 + 0L));
    printf("%s\n", NAME((_BitInt(64))0 + 0L));
    printf("%s\n", NAME((_BitInt(64))0 + 0UL));
    printf("%s\n", NAME((unsigned _BitInt(64))0 + 0LL));
    printf("%s\n", NAME((_BitInt(65))0 + 0ULL));
    printf("%s\n", NAME((_BitInt(128))0 + (__int128)0));
    printf("%s\n", NAME((unsigned _BitInt(128))0 + (__int128)0));
    printf("%s\n", NAME((_BitInt(128))0 + (unsigned __int128)0));
    printf("%s\n", NAME((_BitInt(40))0 + (unsigned _BitInt(33))0));
    printf("%s\n", NAME((_BitInt(33))0 + (unsigned _BitInt(40))0));
    printf("%s\n", NAME((_BitInt(10))0 * 1.0f));
    printf("%s\n", NAME((_BitInt(128))0 + 1.0));
    printf("%s\n", NAME((_BitInt(8))1 << 3L));
    printf("%s\n", NAME(-(_BitInt(8))1));
    printf("%s\n", NAME(~(unsigned _BitInt(7))1));
    printf("%s\n", NAME(1 ? (_BitInt(8))1 : (_BitInt(10))2));
    printf("%s %s %s %s %s\n", NAME(3wb), NAME(1uwb), NAME(0wb), NAME(0uwb), NAME(-1wb));
    printf("%s %s\n", NAME(127wb), NAME(0x7fffffffffffffffwb));
    printf("%s\n", NAME(0xffffffffffffffffuwb));

    printf("literals\n");
    P2(0xffffffffffffffffwb);
    P2(-0x8000000000000000wb);
    P1(255uwb + 1uwb);

    printf("bit-fields\n");
    struct fields s = {0};
    volatile int k = 1;
    s.a = 511; s.b = 1023; s.c = -4; s.d = -1; s.e = -3; s.f = 1;
    P1(s.a + s.a); P1(s.b + s.b); P1(s.c); P2(s.d); P2(s.e); P1(s.f);
    s.a += k; s.b += k; s.c -= k; s.d += k; s.e *= 7; s.f += k;
    P1(s.a); P1(s.b); P1(s.c); P2(s.d); P2(s.e); P1(s.f);
    printf("%zu\n", sizeof(struct fields));

    printf("switch\n");
    volatile _BitInt(65) cases[] = {0, 1, -1, (_BitInt(65))1 << 63, -((_BitInt(65))1 << 63) * 2, 7};
    for (int i = 0; i < 6; i++) printf("%d\n", classify(cases[i]));
    volatile unsigned _BitInt(7) smalls[] = {0, 127, 64, 3};
    for (int i = 0; i < 4; i++) printf("%d\n", small(smalls[i]));

    printf("overflow\n");
    {
        _BitInt(8) r8;
        unsigned _BitInt(7) u7;
        _BitInt(65) r65;
        unsigned _BitInt(128) u128v;
        volatile _BitInt(8) x = 100, y = 27;
        int o = __builtin_add_overflow(x, y, &r8);
        printf("%d ", o); P1(r8);
        o = __builtin_add_overflow(x, (_BitInt(8))28, &r8);
        printf("%d ", o); P1(r8);
        o = __builtin_sub_overflow((unsigned _BitInt(7))3, (unsigned _BitInt(7))4, &u7);
        printf("%d ", o); P1(u7);
        o = __builtin_mul_overflow((_BitInt(65))1 << 32, (_BitInt(65))1 << 31, &r65);
        printf("%d ", o); P2(r65);
        o = __builtin_mul_overflow((_BitInt(65))1 << 32, (_BitInt(65))1 << 32, &r65);
        printf("%d ", o); P2(r65);
        o = __builtin_add_overflow(x, 1000, &r8);
        printf("%d ", o); P1(r8);
        o = __builtin_mul_overflow((unsigned _BitInt(128))-1, 3, &u128v);
        printf("%d ", o); P2(u128v);
    }

    printf("printf\n");
    {
        volatile _BitInt(65) w = -12345;
        volatile unsigned _BitInt(15) h = 32767;
        printf("%lld %u %d\n", (long long)w, (unsigned)h, (int)(_BitInt(7))-3);
    }
}
"""


def wasm_expectation(sdk):
    """Builds the program with wasi-sdk's Clang and records its output."""
    sdk = Path(sdk)
    with tempfile.TemporaryDirectory() as scratch:
        module = Path(scratch) / "bit_int.wasm"
        subprocess.run(
            [
                sdk / "bin" / "clang",
                "--target=wasm32-wasip1",
                f"--sysroot={sdk / 'share' / 'wasi-sysroot'}",
                "-std=c23",
                "-O0",
                "-w",
                OUTPUT,
                "-o",
                module,
            ],
            check=True,
        )
        run = subprocess.run(
            ["node", "--no-warnings", ROOT / "scripts" / "wasm" / "run.mjs", module],
            check=True,
            capture_output=True,
        )
    OUTPUT.with_name("bit_int.wasm32.expected").write_bytes(run.stdout)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--wasi-sdk", help="a wasi-sdk directory, to write the wasm32 expectation")
    arguments = parser.parse_args()
    generate()
    if arguments.wasi_sdk:
        wasm_expectation(arguments.wasi_sdk)
