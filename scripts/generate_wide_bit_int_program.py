"""Generate the wide bit-precise integer execution fixture.

Writes tests/fixtures/programs/bit_int_wide.c, a C23 program for x86-64
that runs every operator of `_BitInt(N)` and `unsigned _BitInt(N)` for
widths 129, 192, 255, 256, 257, 512, 1000 and 4096 over a table of operand
pairs, converts each type's values to and from every standard integer and
real floating type and to other widths, folds a sample of constant
expressions in static initializers, and prints each result. Width 65535,
which only GCC's BITINT_MAXWIDTH and Clang's hold and which Clang compiles
slowly, is built only with -DBIT_INT_65535. tests/integration/bit_int.rs
compares bcc's output on every back end with what the bundled Clang and the
host GCC (14 or later) build of it prints; generate_bit_int_program.py
covers widths 1 to 128.

Operands are limb images in constant tables, copied into objects at run
time, so the arithmetic happens at run time; the folded section writes the
same operations with constant operands. The values are each type's bounds,
0, 1, -1 and pseudo-random patterns of several magnitudes (a fixed seed).
An operation whose behavior is undefined (signed overflow, division by
zero, a shift count outside the width, a negative value shifted left, a
floating value out of range) is left out. A result of more than 512 bits
prints as its low and high limbs and a hash of all of them.

Usage: python3 scripts/generate_wide_bit_int_program.py
"""
from fractions import Fraction
from pathlib import Path
import random

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "tests" / "fixtures" / "programs" / "bit_int_wide.c"

WIDTHS = [129, 192, 255, 256, 257, 512, 1000, 4096]
HUGE = 65535

BINARY = [
    ("add", "a + b"),
    ("sub", "a - b"),
    ("mul", "a * b"),
    ("div", "a / b"),
    ("rem", "a % b"),
    ("and", "a & b"),
    ("or", "a | b"),
    ("xor", "a ^ b"),
    ("shl", "a << ({t})k"),
    ("shr", "a >> ({t})k"),
    ("shlk", "a << k"),
    ("shrk", "a >> k"),
    ("cmp", None),
    ("add=", None),
    ("mul=", None),
    ("shr=", None),
    ("cond", None),
]
UNARY = [
    ("neg", "-a"),
    ("not", "~a"),
    ("lnot", "!a"),
    ("inc", None),
    ("dec", None),
    ("postinc", None),
]

STANDARD = [
    ("bool", "bool", 1, False),
    ("char", "signed char", 8, True),
    ("uc", "unsigned char", 8, False),
    ("short", "short", 16, True),
    ("int", "int", 32, True),
    ("uint", "unsigned", 32, False),
    ("long", "long", 64, True),
    ("ull", "unsigned long long", 64, False),
    ("i128", "__int128", 128, True),
    ("u128", "unsigned __int128", 128, False),
]
NARROW = [(7, True), (64, False), (65, True), (128, False)]
FLOATING = [
    ("f16", "_Float16", 11, 16, "2"),
    ("f", "float", 24, 128, "4"),
    ("d", "double", 53, 1024, "8"),
    ("ld", "long double", 64, 16384, "10"),
    ("f128", "__float128", 113, 16384, "16"),
]


class BitInt:
    def __init__(self, width, signed):
        self.width = width
        self.signed = signed
        self.tag = ("bs" if signed else "bu") + str(width)
        self.spelling = ("" if signed else "unsigned ") + f"_BitInt({width})"
        self.unsigned_spelling = f"unsigned _BitInt({width})"
        self.limbs = (width + 63) // 64

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

    def image(self, value):
        """The limbs of `value` in memory, padding bits extended."""
        pattern = value & ((1 << (64 * self.limbs)) - 1)
        return [(pattern >> (64 * i)) & ((1 << 64) - 1) for i in range(self.limbs)]

    def constant(self, value):
        """A constant expression of this type with `value`, built from
        64-bit pieces in its unsigned type."""
        pattern = value & ((1 << self.width) - 1)
        pieces = []
        for i in range(self.limbs):
            piece = (pattern >> (64 * i)) & ((1 << 64) - 1)
            if piece:
                term = f"({self.unsigned_spelling}){piece:#x}ULL"
                pieces.append(f"{term} << {64 * i}" if i else term)
        if not pieces:
            return f"({self.spelling})0"
        return f"({self.spelling})({' | '.join(f'({p})' for p in pieces)})"


def truncating_division(a, b):
    quotient = abs(a) // abs(b)
    return quotient if (a < 0) == (b < 0) else -quotient


def defined(ty, name, a, b):
    if name in ("add", "add="):
        return not ty.signed or ty.fits(a + b)
    if name == "sub":
        return not ty.signed or ty.fits(a - b)
    if name in ("mul", "mul="):
        return not ty.signed or ty.fits(a * b)
    if name in ("div", "rem"):
        return b != 0 and (not ty.signed or ty.fits(truncating_division(a, b)))
    if name in ("shl", "shlk"):
        if not 0 <= b < ty.width:
            return False
        return not ty.signed or (a >= 0 and ty.fits(a << b))
    if name in ("shr", "shrk", "shr="):
        return 0 <= b < ty.width
    if name == "neg":
        return not ty.signed or ty.fits(-a)
    if name in ("inc", "postinc"):
        return not ty.signed or ty.fits(a + 1)
    if name == "dec":
        return not ty.signed or ty.fits(a - 1)
    return True


def values(ty, rng, few=False):
    chosen = [0, 1, ty.maximum(), ty.minimum()]
    if ty.signed:
        chosen.append(-1)
    sizes = [ty.width, ty.width // 2] if few else [ty.width, ty.width, ty.width // 2, 70, 3]
    for bits in sizes:
        value = rng.getrandbits(bits)
        if ty.signed and rng.getrandbits(1):
            value = -value
        chosen.append(ty.wrap(value))
    unique = []
    for value in chosen:
        if ty.fits(value) and value not in unique:
            unique.append(value)
    return unique


def generate():
    rng = random.Random(0xB17_1DE)
    lines = [
        "/* Generated by scripts/generate_wide_bit_int_program.py; do not edit. */",
        "#include <stdio.h>",
        "#include <string.h>",
        "",
        "typedef unsigned long long limb;",
        "typedef unsigned __int128 u128;",
        "static int line;",
        "",
        "/* Prints the low `bits` bits of the limbs at p: every limb up to 512",
        " * bits, else the low and high limbs and an FNV-1a hash of all. */",
        "static void show(const void *p, int bits) {",
        "    int n = (bits + 63) / 64;",
        "    limb top = ((const limb *)p)[n - 1];",
        "    if (bits % 64) top &= (1ULL << bits % 64) - 1;",
        "    printf(\"%d\", line++);",
        "    if (bits <= 512) {",
        "        printf(\" %llx\", top);",
        "        for (int i = n - 2; i >= 0; i--) printf(\" %016llx\", ((const limb *)p)[i]);",
        "    } else {",
        "        limb hash = 0xcbf29ce484222325ULL;",
        "        for (int i = 0; i < n; i++) {",
        "            limb v = i == n - 1 ? top : ((const limb *)p)[i];",
        "            for (int k = 0; k < 8; k++) hash = (hash ^ (v >> 8 * k & 0xff)) * 0x100000001b3ULL;",
        "        }",
        "        printf(\" top %llx low %016llx hash %016llx\", top, ((const limb *)p)[0], hash);",
        "    }",
        "    printf(\"\\n\");",
        "}",
        "#define SHOW(type, bits, x) do { type show_value = (x); show(&show_value, bits); } while (0)",
        "static void p128(u128 v) {",
        "    printf(\"%d %llx:%016llx\\n\", line++, (unsigned long long)(v >> 64), (unsigned long long)v);",
        "}",
        "static void bytes(const void *p, int n) {",
        "    const unsigned char *b = p;",
        "    printf(\"%d \", line++);",
        "    for (int i = n - 1; i >= 0; i--) printf(\"%02x\", b[i]);",
        "    printf(\"\\n\");",
        "}",
        "",
    ]
    types = []
    for width in WIDTHS:
        types.append((BitInt(width, True), False))
        types.append((BitInt(width, False), False))
    huge = [(BitInt(HUGE, True), True), (BitInt(HUGE, False), True)]
    sections = []
    for ty, is_huge in types + huge:
        operands = values(ty, rng, few=is_huge)
        entries = []
        for code, (name, _) in enumerate(BINARY):
            pairs = [(a, b) for a in operands for b in operands]
            if is_huge and name in ("mul", "div", "rem", "mul="):
                pairs = pairs[:: max(1, len(pairs) // 6)]
            for a, b in pairs:
                # A shift count is the value of `k`, converted to the
                # shifted type for the operators that take one of it.
                k = 0
                if name in ("shl", "shr", "shr=", "shlk", "shrk"):
                    k = abs(b) % ty.width
                    b = 0
                    if not defined(ty, name, a, k):
                        continue
                elif not defined(ty, name, a, b):
                    continue
                entries.append((code, operands.index(a), operands.index(b), k))
        for offset, (name, _) in enumerate(UNARY):
            for a in operands:
                if defined(ty, name, a, 0):
                    entries.append((len(BINARY) + offset, operands.index(a), 0, 0))
        section = emit_type(ty, operands, entries)
        if is_huge:
            section = ["#ifdef BIT_INT_65535"] + section + ["#endif"]
        lines.extend(section)
        sections.append((ty, is_huge, operands))
    lines.extend(conversions(sections, rng))
    lines.extend(folded(sections, rng))
    lines.extend(EXTRAS.splitlines())
    lines.append("int main(void) {")
    for ty, is_huge, _ in sections:
        call = f"    run_{ty.tag}();"
        lines.append(f"#ifdef BIT_INT_65535\n{call}\n#endif" if is_huge else call)
    lines.append("    conversions();")
    lines.append("    folded();")
    lines.append("    extras();")
    lines.append("    return 0;")
    lines.append("}")
    OUTPUT.write_text("\n".join(lines) + "\n")


def emit_type(ty, operands, entries):
    t, s = ty.tag, ty.spelling
    lines = [f"typedef {s} {t};"]
    for index, value in enumerate(operands):
        limbs = ", ".join(f"{limb:#x}" for limb in ty.image(value))
        lines.append(f"static const limb {t}_v{index}[{ty.limbs}] = {{{limbs}}};")
    lines.append(f"static const limb *const {t}_v[] = {{")
    lines.append("    " + ", ".join(f"{t}_v{i}" for i in range(len(operands))))
    lines.append("};")
    for column, pick in (("op", 0), ("a", 1), ("b", 2)):
        lines.append(f"static const unsigned short {t}_{column}[] = {{")
        lines.append("    " + ", ".join(str(entry[pick]) for entry in entries))
        lines.append("};")
    lines.append(f"static const unsigned {t}_k[] = {{")
    lines.append("    " + ", ".join(str(entry[3]) for entry in entries))
    lines.append("};")
    lines.append(f"static void run_{t}(void) {{")
    lines.append(f"    printf(\"{t} sizeof %zu alignof %zu\\n\", sizeof({t}), _Alignof({t}));")
    lines.append(f"    for (unsigned i = 0; i < sizeof {t}_op / sizeof {t}_op[0]; i++) {{")
    lines.append(f"        {t} a, b, r = 0, s = 0;")
    lines.append(f"        memcpy(&a, {t}_v[{t}_a[i]], sizeof a);")
    lines.append(f"        memcpy(&b, {t}_v[{t}_b[i]], sizeof b);")
    lines.append(f"        volatile unsigned k = {t}_k[i];")
    lines.append(f"        switch ({t}_op[i]) {{")
    for code, (name, expression) in enumerate(BINARY):
        if name == "cmp":
            body = (
                "r = (a < b) | (a <= b) << 1 | (a > b) << 2 | (a >= b) << 3"
                " | (a == b) << 4 | (a != b) << 5"
            )
        elif name == "add=":
            body = "r = a; r += b"
        elif name == "mul=":
            body = "r = a; r *= b"
        elif name == "shr=":
            body = f"r = a; r >>= ({t})k"
        elif name == "cond":
            body = "r = a ? b : a; s = a < b ? a : b; r ^= s"
        else:
            body = f"r = {expression.format(t=t)}"
        lines.append(f"        case {code}: {body}; break;")
    for offset, (name, expression) in enumerate(UNARY):
        code = len(BINARY) + offset
        if name == "inc":
            body = "r = a; r++"
        elif name == "dec":
            body = "r = a; --r"
        elif name == "postinc":
            body = "r = a; s = r++; r ^= s"
        else:
            body = f"r = {expression}"
        lines.append(f"        case {code}: {body}; break;")
    lines.append("        }")
    lines.append(f"        show(&r, {ty.width});")
    lines.append("    }")
    lines.append("}")
    lines.append("")
    return lines


def conversions(sections, rng):
    lines = ["static void conversions(void) {"]
    plain = [ty for ty, is_huge, _ in sections if not is_huge]
    for ty, is_huge, operands in sections:
        t = ty.tag
        body = [f"    printf(\"conversions {t}\\n\");"]
        others = plain if not is_huge else [o for o in plain if o.width in (129, 4096)]
        for index, value in enumerate(operands):
            body.append(f"    {{ {t} v; memcpy(&v, {t}_v{index}, sizeof v);")
            for _, spelling, width, _ in STANDARD:
                body.append(f"      p128((u128)({spelling})v);")
            for width, signed in NARROW:
                spelling = ("" if signed else "unsigned ") + f"_BitInt({width})"
                body.append(f"      p128((u128)({spelling})v);")
            for tag, spelling, _, _, size in FLOATING:
                # Clang 23 converts an unsigned value with its top bit set
                # to x87 and binary128 as if it were negative.
                if tag in ("ld", "f128") and not ty.signed and value >> (ty.width - 1):
                    continue
                body.append(f"      {{ {spelling} f = v; bytes(&f, {size}); }}")
            for other in others:
                body.append(f"      SHOW({other.spelling}, {other.width}, ({other.spelling})v);")
            if is_huge:
                body.append(f"      SHOW({ty.spelling}, {ty.width}, ({ty.spelling})({BitInt(HUGE, not ty.signed).spelling})v);")
            body.append("    }")
        # From the standard integer types.
        for tag, spelling, width, signed in STANDARD:
            if tag == "bool":
                continue
            source = BitInt(width, signed)
            for value in (source.minimum(), -1 if signed else 1, source.maximum()):
                body.append(
                    f"    {{ volatile {spelling} v = ({spelling}){literal_of(value)};"
                    f" SHOW({ty.spelling}, {ty.width}, v); }}"
                )
        # From the floating types, values in range only, whole and
        # truncated.
        for value in floating_values(ty, rng):
            for tag, spelling, precision, max_exponent, _ in FLOATING:
                if not representable(value, precision, max_exponent):
                    continue
                body.append(
                    f"    {{ volatile {spelling} v = ({spelling}){float_literal(value)};"
                    f" SHOW({ty.spelling}, {ty.width}, v); }}"
                )
        if is_huge:
            body = ["#ifdef BIT_INT_65535"] + body + ["#endif"]
        lines.extend(body)
    lines.append("}")
    lines.append("")
    return lines


def folded(sections, rng):
    """Constant expressions in static initializers, which must fold."""
    lines = ["static void folded(void) {"]
    for ty, is_huge, operands in sections:
        if is_huge:
            continue
        lines.append(f"    printf(\"folded {ty.tag}\\n\");")
        picks = [(a, b) for a in operands for b in operands]
        rng.shuffle(picks)
        count = 0
        for a, b in picks:
            for name, expression in BINARY + UNARY:
                if name in ("shl", "shr"):
                    expression = "a << b" if name == "shl" else "a >> b"
                if expression is None or "k" in expression:
                    continue
                operand_b = (b % ty.width if b >= 0 else 0) if name in ("shl", "shr") else b
                if not defined(ty, name, a, operand_b):
                    continue
                if rng.random() > 0.25:
                    continue
                text = expression.replace("a", "{a}").replace("b", "{b}").format(
                    a=f"({ty.constant(a)})", b=f"({ty.constant(operand_b)})"
                )
                lines.append(
                    f"    {{ static const {ty.spelling} folded = ({ty.spelling})({text});"
                    f" show(&folded, {ty.width}); }} /* {name} */"
                )
                count += 1
            if count > 24:
                break
        # Conversions of constants to narrower and wider types and floats.
        for value in operands[:4]:
            constant = ty.constant(value)
            lines.append(f"    {{ static const unsigned long long folded = (unsigned long long){constant}; p128(folded); }}")
            lines.append(f"    {{ static const double folded = {constant}; bytes(&folded, 8); }}")
            lines.append(f"    {{ static const long double folded = {constant}; bytes(&folded, 10); }}")
            wider = BitInt(ty.width + 77, not ty.signed)
            lines.append(
                f"    {{ static const {wider.spelling} folded = {constant};"
                f" show(&folded, {wider.width}); }}"
            )
    lines.append("}")
    lines.append("")
    return lines


# Values that cross statements and conditionals, and a function whose
# hundreds of 65535-bit results need their stack slots shared.
EXTRAS = r"""
static _BitInt(300) chain(_BitInt(300) a, _BitInt(300) b, int n) {
    _BitInt(300) r = a;
    for (int i = 0; i < n; i++) {
        _BitInt(300) t = r * 3 + b;
        r = i % 2 ? t - a : t / 2;
    }
    return n > 2 ? r + a * b : r - b;
}

#ifdef BIT_INT_65535
#define STEP(k) r = (r + a * (k)) / (b + (k)) + (a >> (k)) - (b << (k));
#define TEN(k) STEP(k) STEP(k + 1) STEP(k + 2) STEP(k + 3) STEP(k + 4) \
    STEP(k + 5) STEP(k + 6) STEP(k + 7) STEP(k + 8) STEP(k + 9)
static _BitInt(65535) deep(_BitInt(65535) a, _BitInt(65535) b) {
    _BitInt(65535) r = a;
    TEN(0) TEN(10) TEN(20) TEN(30) TEN(40) TEN(50) TEN(60) TEN(70)
    TEN(80) TEN(90) TEN(100) TEN(110) TEN(120) TEN(130) TEN(140)
    return r;
}
#endif

#define WIDTH(x) _Generic((x), unsigned _BitInt(136): 136, _BitInt(128): 128, \
    _BitInt(126): 126, _BitInt(209): 209, unsigned _BitInt(200): 200, default: 0)

struct flags {
    _Bool b;
    int i;
    long long l;
};

static void extras(void) {
    printf("extras\n");
    /* Folded wide values whose low 128 bits are zero are not zero. */
    struct flags f = {(_BitInt(200))1 << 128, (int)((_BitInt(200))1 << 130), 5};
    _Bool t = (_BitInt(200))1 << 150;
    static _Bool st = (_BitInt(200))1 << 150;
    static struct flags sf = {(_BitInt(200))1 << 128, (int)((_BitInt(200))1 << 130), 5};
    printf("%d %d %lld %d %d %d %d\n", f.b, f.i, f.l, t, st, sf.b, sf.i);
    /* wb constants of more than 64 bits, in every base. */
    printf("%d %d %d %d %d\n", WIDTH(0xfedcba9876543210fedcba9876543210ffuwb),
           WIDTH(123456789012345678901234567890123456789wb),
           WIDTH(0x8000000000000000000000000000000000000000000000000000wb),
           WIDTH(0b11111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111uwb),
           WIDTH(0377777777777777777777777777777777777777777wb));
    SHOW(unsigned _BitInt(136), 136, 0xfedcba9876543210fedcba9876543210ffuwb);
    SHOW(_BitInt(128), 128, -123456789012345678901234567890123456789wb);
    SHOW(_BitInt(209), 209, 0x8000000000000000000000000000000000000000000000000000wb - 1);
    SHOW(_BitInt(126), 126, 0377777777777777777777777777777777777777777wb);
    volatile int n = 5;
    _BitInt(300) a = (_BitInt(300))n << 200, b = (_BitInt(300))n * 1234567;
    SHOW(_BitInt(300), 300, chain(a, b, n));
    SHOW(_BitInt(300), 300, chain(-a, b, 2));
    _BitInt(300) values[4];
    for (int i = 0; i < 4; i++)
        values[i] = a * i - b;
    SHOW(_BitInt(300), 300, values[0] + values[1] * values[2] - values[3]);
#ifdef BIT_INT_65535
    _BitInt(65535) big = (_BitInt(65535))n << 65000, small = (_BitInt(65535))n << 300;
    SHOW(_BitInt(65535), 65535, deep(big, small));
#endif
}
"""


def literal_of(value):
    if value == -(1 << 63):
        return "(-0x7fffffffffffffffLL - 1)"
    if value == -(1 << 127):
        return "(-((__int128)0x7fffffffffffffffLL << 64) - ((__int128)0xffffffffffffffffULL) - 1)"
    if -(1 << 63) < value < (1 << 64):
        return f"({value}LL)" if value < 0 else f"{value}ULL"
    pattern = value & ((1 << 128) - 1)
    high, low = pattern >> 64, pattern & ((1 << 64) - 1)
    return f"(((unsigned __int128){high:#x}ULL << 64) | {low:#x}ULL)"


def floating_values(ty, rng):
    """Whole values with at most 11 significant bits, which every format
    holding their magnitude holds exactly, and values with a fraction."""
    result = [0, 1, 3 << (ty.width - 4), ty.maximum() // 5]
    if ty.signed:
        result += [-1, -(5 << (ty.width - 6))]
    exact = []
    for value in result:
        if value != 0:
            shift = max(0, abs(value).bit_length() - 11)
            magnitude = (abs(value) >> shift) << shift
            value = magnitude if value > 0 else -magnitude
        if ty.fits(value) and value not in exact:
            exact.append(value)
    fractions = [Fraction(7, 4), Fraction(-1023, 8)] if ty.signed else [Fraction(7, 4)]
    return exact + fractions


def representable(value, precision, max_exponent):
    return abs(value) < 2 ** (max_exponent - 1)


def float_literal(value):
    if value == 0:
        return "0.0"
    if isinstance(value, Fraction):
        return f"({value.numerator}.0 / {value.denominator})"
    exponent = abs(value).bit_length() - 1
    mantissa = Fraction(abs(value), 2**exponent)
    sign = "-" if value < 0 else ""
    fraction = int((mantissa - 1) * 2**12)
    # A long double constant: x87's range holds every width's values.
    return f"{sign}0x1.{fraction:03x}p{exponent}L"


if __name__ == "__main__":
    generate()
