"""Measure what GCC 14.2 and Clang 23 accept in an x86 `target` attribute.

Writes src/target/x86_features/table.rs, the table that
src/target/x86_features.rs reads: every feature name either compiler takes in
`target("...")` (and after `-m` where bcc implements the option), the features
each turns on in each compiler, and every processor `arch=` or `tune=` names.

Nothing is copied from either compiler's sources; each fact is what the
compiler does with a probe:

- A name is accepted when a function with `target("name")` compiles without
  GCC's error or Clang's -Wignored-attributes warning.
- Clang's implications are the `target-features` attribute of that function in
  its LLVM IR (`+name` entries). One that comes along only by default, as
  POPCNT does with SSE4.2, is told from one the feature needs by adding
  `no-implied`: the feature survives the first and is lost with the second.
- GCC's implications are its `-dM -E` macros for `-mname`, mapped back to
  features through each feature's own macro, and told apart the same way with
  `-mno-implied`.
- A processor's features are Clang's `target-features` for `arch=cpu` and
  GCC's macros for `-march=cpu`; its LLVM name is Clang's `target-cpu`.
- The feature an x86 builtin of src/builtins/x86.rs needs is the one Clang
  names when a call with operands of its signature stands in a function
  without it ("needs target feature").

Pass `--cc1` the `cc1` of GCC 14.2 for x86_64-linux-gnu (as
scripts/update_gcc_target_macros.py does) and `--clang` the pinned Clang.
`--check` compares instead of writing.
"""
import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "src/target/x86_features/table.rs"
TRIPLE = "x86_64-unknown-linux-gnu"

# Names both or either compiler accepts that are options rather than
# instruction sets: they take no bit. `sse4` and `no-sse4` are aliases that
# x86_features.rs resolves itself.
OPTIONS = {
    "align-stringops", "cld", "fancy-math-387", "ieee-fp", "inline-all-stringops",
    "inline-stringops-dynamically", "recip", "relax-cmpxchg-loop", "general-regs-only",
    "prefer-256-bit", "sse4",
}
# Always on for x86-64, so they take no bit either: x86_features.rs accepts
# them and refuses turning one off.
BASELINE = {"x87", "mmx", "fxsr", "sse", "sse2"}
# Macros whose names do not follow from the feature's.
MACRO = {"sahf": "__LAHF_SAHF__", "3dnow": "__3dNOW__", "3dnowa": "__3dNOW_A__", "apxf": "__APX_F__"}
GCC_MACRO = {"usermsr": "__USER_MSR__"}


def macro(feature, gcc):
    if gcc and feature in GCC_MACRO:
        return GCC_MACRO[feature]
    return MACRO.get(feature, "__" + re.sub(r"[.-]", "_", feature.upper()) + "__")


class Probe:
    def __init__(self, cc1, clang):
        self.cc1, self.clang = cc1, clang
        self.dir = Path(tempfile.mkdtemp(prefix="bcc-target-attribute-"))

    def attributes(self, strings):
        """Per string: GCC's first diagnostic, Clang's, and Clang's
        (features, cpu, tune) for the function."""
        source = self.dir / "probe.c"
        source.write_text("".join(
            '__attribute__((target("%s"))) void f%d(void) {}\n' % (s, i) for i, s in enumerate(strings)))
        gcc = subprocess.run([self.cc1, "-quiet", str(source), "-o", "/dev/null"], capture_output=True, text=True)
        clang = subprocess.run([self.clang, "--target=" + TRIPLE, "-S", "-emit-llvm", "-O0", str(source), "-o", "-"],
                               capture_output=True, text=True)
        def diagnostics(stderr):
            found = {}
            for m in re.finditer(r"probe\.c:(\d+):\d+: (?:error|warning): (.*)", stderr):
                found.setdefault(int(m[1]) - 1, m[2])
            return found
        gbad, cbad = diagnostics(gcc.stderr), diagnostics(clang.stderr)
        groups = dict(re.findall(r"^attributes #(\d+) = \{ (.*) \}$", clang.stdout, re.M))
        ir = {}
        for m in re.finditer(r"^define dso_local void @f(\d+)\(\) #(\d+)", clang.stdout, re.M):
            text = groups[m[2]]
            get = lambda key: (re.search(r'"%s"="([^"]*)"' % key, text) or [None, None])[1]
            ir[int(m[1])] = (get("target-features") or "", get("target-cpu"), get("tune-cpu"))
        return [(gbad.get(i), cbad.get(i), ir.get(i)) for i in range(len(strings))]

    def gcc_macros(self, options):
        out = subprocess.run([self.cc1, "-quiet", "-E", "-dM", *options, "-", "-o", "-"], input="",
                             capture_output=True, text=True)
        assert out.returncode == 0, (options, out.stderr)
        return set(re.findall(r"#define (__\w+__) 1\b", out.stdout))

    def gcc_options(self):
        out = subprocess.run([self.cc1, "--help=target", "-quiet"], input="", capture_output=True, text=True).stdout
        return set(re.findall(r"^  -m([a-z0-9][a-z0-9._-]*)", out, re.M))

    def gcc_cpus(self, kind):
        source = self.dir / "cpu.c"
        source.write_text('__attribute__((target("%s=bcc-unknown"))) void f(void) {}\n' % kind)
        out = subprocess.run([self.cc1, "-quiet", str(source), "-o", "/dev/null"], capture_output=True, text=True)
        m = re.search(r"valid arguments to .target\(\"%s=\"\). attribute are: (.*)" % kind, out.stderr)
        return set(m[1].split())

    def llvm_help(self):
        source = self.dir / "empty.c"
        source.write_text("int x;\n")
        out = subprocess.run([self.clang, "--target=" + TRIPLE, "-c", str(source), "-o", "/dev/null",
                              "-Xclang", "-target-feature", "-Xclang", "+help"], capture_output=True, text=True)
        text = out.stdout + out.stderr
        cpus, features = text.split("Available features for this target:")
        names = lambda part: set(re.findall(r"^\s+([a-z0-9][a-z0-9._-]*)\s+- ", part, re.M))
        return names(cpus), names(features)


SCALARS = {"Char": "char", "UnsignedChar": "unsigned char", "Short": "short",
           "UnsignedShort": "unsigned short", "Int": "int", "UnsignedInt": "unsigned int",
           "LongLong": "long long", "UnsignedLongLong": "unsigned long long", "Float": "float",
           "Double": "double", "Void": "void"}
SIZES = {"char": 1, "unsigned char": 1, "short": 2, "unsigned short": 2, "int": 4, "unsigned int": 4,
         "float": 4, "long long": 8, "unsigned long long": 8, "double": 8}


def builtin_features(probe):
    """The feature each builtin of src/builtins/x86.rs needs in Clang."""
    registry = (ROOT / "src/builtins/x86.rs").read_text()
    types = {}
    for name, kind, args in re.findall(r"const (\w+): T = T::(\w+)\((.*?)\);", registry):
        a = [x.strip() for x in args.split(",")]
        scalar = SCALARS[a[0][3:]]
        const = "const " if a[-1] == "true" else ""
        if kind == "Scalar":
            types[name] = (scalar, "{0}")
        elif kind == "Pointer":
            types[name] = ("%s%s *" % (const, scalar), "0")
        elif kind == "Vector":
            types[name] = ("%s __attribute__((vector_size(%d)))" % (scalar, int(a[1]) * SIZES[scalar]), "{0}")
        elif kind == "VectorPointer":
            types[name] = ("%s%s __attribute__((vector_size(%d))) *" % (const, scalar, int(a[1]) * SIZES[scalar]), "0")
    found = {}
    source = probe.dir / "builtin.c"
    for name, params, args in re.findall(r'x86\(Id::\w+, "(\w+)", \w+, &\[([^\]]*)\], &\[([^\]]*)\]\)', registry):
        params = [p.strip() for p in params.split(",") if p.strip()]
        operands = re.findall(r"P|imm\((-?[\d_]+), (-?[\d_]+), (\d+)\)", args)
        decls, values = [], []
        for index, (param, operand) in enumerate(zip(params, operands)):
            if operand[0]:
                low, mask = int(operand[0].replace("_", "")), int(operand[2])
                values.append(str(next((x for x in range(max(low, 0), 4096) if mask & (1 << (x % 16))), low)
                                  if mask else low))
            else:
                ty, init = types[param]
                decls.append("%s a%d = %s;" % (ty, index, init))
                values.append("a%d" % index)
        source.write_text("void f(void) { %s (void)%s(%s); }\n" % (" ".join(decls), name, ", ".join(values)))
        out = subprocess.run([probe.clang, "--target=" + TRIPLE, "-c", "-w", str(source), "-o", "/dev/null"],
                             capture_output=True, text=True)
        m = re.search(r"needs target feature (\S+)", out.stderr)
        assert out.returncode == 0 or m, (name, out.stderr)
        if m:
            found[name] = m[1]
    return found


def plus(features):
    return {f[1:] for f in features.split(",") if f.startswith("+")}


def minus(features):
    return {f[1:] for f in features.split(",") if f.startswith("-")}


def measure(probe):
    llvm_cpus, llvm_features = probe.llvm_help()
    # GCC's help lists some negations, as `-mno-align-stringops`, which
    # the attribute reads as negated options.
    candidates = sorted(n for n in llvm_features | probe.gcc_options() if not n.startswith("no-"))
    verdicts = probe.attributes(candidates)
    accepted = {name: (g is None, c is None) for name, (g, c, _) in zip(candidates, verdicts)}
    isa = sorted(n for n, (g, c) in accepted.items() if (g or c) and n not in OPTIONS | BASELINE)
    clang_plus = {n: plus(ir[0]) for n, (_, c, ir) in zip(candidates, verdicts) if c is None}
    for name in OPTIONS | BASELINE:
        assert any(accepted[name]), name
    # Clang: needs or defaults.
    clang = {}
    pairs = [(n, d) for n in isa if accepted[n][1] for d in sorted((clang_plus[n] & set(isa)) - {n})]
    survived = probe.attributes(["%s,no-%s" % pair for pair in pairs])
    for (n, d), (_, bad, ir) in zip(pairs, survived):
        assert bad is None, (n, d, bad)
        needs, defaults = clang.setdefault(n, (set(), set()))
        (defaults if n in plus(ir[0]) else needs).add(d)
    # GCC: through the macros.
    base = probe.gcc_macros([])
    own = {n: macro(n, True) for n in isa if accepted[n][0]}
    gcc = {}
    for n in own:
        found = probe.gcc_macros(["-m" + n]) - base
        implied = {d for d, m in own.items() if m in found and d != n}
        needs, defaults = gcc.setdefault(n, (set(), set()))
        for d in sorted(implied):
            (defaults if own[n] in probe.gcc_macros(["-m" + n, "-mno-" + d]) else needs).add(d)
    # Processors.
    gcc_arch, gcc_tune = probe.gcc_cpus("arch"), probe.gcc_cpus("tune")
    names = sorted(llvm_cpus | gcc_arch | gcc_tune)
    arch = probe.attributes(["arch=" + c for c in names])
    tune = probe.attributes(["tune=" + c for c in names])
    cpus = []
    for name, (ga, ca, air), (gt, ct, tir) in zip(names, arch, tune):
        row = {"name": name, "gcc_arch": ga is None, "gcc_tune": gt is None, "clang_arch": ca is None,
               "clang_tune": ct is None, "llvm": None, "llvm_tune": None, "gcc": [], "clang": []}
        if ca is None:
            row["llvm"] = air[1]
            row["clang"] = sorted(plus(air[0]) & set(isa))
        if ct is None:
            row["llvm_tune"] = tir[2]
        elif name in llvm_cpus:
            row["llvm_tune"] = name
        if ga is None:
            found = probe.gcc_macros(["-march=" + name])
            row["gcc"] = sorted(d for d, m in own.items() if m in found)
        if any(row[k] for k in ("gcc_arch", "gcc_tune", "clang_arch", "clang_tune")):
            cpus.append(row)
    # Check `no-` against Clang's own reading: it turns off what needs the
    # feature, transitively.
    # An alias, as Clang's `apxf` is, stays on in this reading.
    real = [n for n in isa if accepted[n][1] and n in llvm_features]
    negations = probe.attributes(["no-" + n for n in real])
    for n, (_, bad, ir) in zip(real, negations):
        assert bad is None, (n, bad)
        expected = {n} | {f for f in clang if n in closure(clang, f) and f in llvm_features}
        if minus(ir[0]) != expected: print("negation", n, sorted(minus(ir[0]) - expected), sorted(expected - minus(ir[0])), file=sys.stderr)
    features = []
    for n in isa:
        features.append({
            "name": n, "llvm": n if n in llvm_features else None, "gcc": accepted[n][0], "clang": accepted[n][1],
            "macro": macro(n, not accepted[n][1]),
            "gcc_needs": sorted(gcc.get(n, (set(), set()))[0]), "gcc_defaults": sorted(gcc.get(n, (set(), set()))[1]),
            "clang_needs": sorted(clang.get(n, (set(), set()))[0]),
            "clang_defaults": sorted(clang.get(n, (set(), set()))[1]),
        })
    builtins = builtin_features(probe)
    for name, feature in builtins.items():
        assert feature in isa, (name, feature)
    return {"features": features, "cpus": cpus, "builtins": sorted(builtins.items())}


def closure(table, name):
    seen, work = set(), [name]
    while work:
        n = work.pop()
        for d in table.get(n, (set(), set()))[0]:
            if d not in seen:
                seen.add(d)
                work.append(d)
    return seen


def constant(name):
    return {"3dnow": "THREE_DNOW", "3dnowa": "THREE_DNOW_A"}.get(name, re.sub(r"[.-]", "_", name.upper()))


def bits(names):
    return " | ".join(constant(n) for n in names) or "0"


def render(data, versions):
    features, cpus = data["features"], data["cpus"]
    assert len(features) <= 128, len(features)
    lines = [
        "//! Generated by `scripts/measure_x86_target_attribute.py` from probes of",
        "//! %s and %s; do not edit." % versions,
        "//!",
        "//! Every x86 feature either compiler takes in a `target` attribute, with the",
        "//! features each compiler turns on with it: those it needs, which turning",
        "//! them off takes along, and those it brings only by default, which an",
        "//! explicit choice outranks. Then every processor `arch=` or `tune=` names,",
        "//! with the features each compiler gives it and LLVM's names for it.",
        "//! GCC manual, \"x86 Function Attributes\"; Clang Attributes Reference,",
        "//! \"target\".",
        "",
        "#![expect(",
        "    dead_code,",
        "    reason = \"Code names some features by their constants; the table holds all.\"",
        ")]",
        "",
        "use super::{",
        "    Feature,",
        "    Processor,",
        "};",
        "",
    ]
    for index, f in enumerate(features):
        lines.append("pub(super) const %s: u128 = 1 << %d;" % (constant(f["name"]), index))
    lines += ["", "/// In bit order: entry `n` is the feature with bit `n`.", "#[rustfmt::skip]",
              "pub(super) const FEATURES: [Feature; %d] = [" % len(features)]
    for f in features:
        lines.append("    Feature { name: %s, llvm: %s, macro_name: %s, gcc: %s, clang: %s, needs: [%s, %s], defaults: [%s, %s] }," % (
            json.dumps(f["name"]), "Some(%s)" % json.dumps(f["llvm"]) if f["llvm"] else "None",
            json.dumps(f["macro"]), str(f["gcc"]).lower(), str(f["clang"]).lower(),
            bits(f["gcc_needs"]), bits(f["clang_needs"]), bits(f["gcc_defaults"]), bits(f["clang_defaults"])))
    lines += ["];", "", "/// The processors, by name.", "#[rustfmt::skip]",
              "pub(super) const PROCESSORS: [Processor; %d] = [" % len(cpus)]
    for c in cpus:
        lines.append("    Processor { name: %s, llvm: %s, llvm_tune: %s, arch: [%s, %s], tune: [%s, %s], features: [%s, %s] }," % (
            json.dumps(c["name"]), "Some(%s)" % json.dumps(c["llvm"]) if c["llvm"] else "None",
            "Some(%s)" % json.dumps(c["llvm_tune"]) if c["llvm_tune"] else "None",
            str(c["gcc_arch"]).lower(), str(c["clang_arch"]).lower(), str(c["gcc_tune"]).lower(),
            str(c["clang_tune"]).lower(), bits(c["gcc"]), bits(c["clang"])))
    builtins = data["builtins"]
    lines += ["];", "", "/// The x86 builtins that need a feature in Clang, by name.", "#[rustfmt::skip]",
              "pub(super) const BUILTINS: [(&str, u128); %d] = [" % len(builtins)]
    for name, feature in builtins:
        lines.append("    (%s, %s)," % (json.dumps(name), constant(feature)))
    lines += ["];", ""]
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--cc1", required=True, help="GCC 14.2's cc1 for x86_64-linux-gnu")
    parser.add_argument("--clang", required=True, help="the pinned Clang")
    parser.add_argument("--check", action="store_true", help="compare with the table instead of writing it")
    args = parser.parse_args()
    probe = Probe(args.cc1, args.clang)
    gcc_version = subprocess.run([args.cc1, "-version", "-", "-o", "/dev/null"], input="",
                                 capture_output=True, text=True).stderr
    clang_version = subprocess.run([args.clang, "--version"], capture_output=True, text=True).stdout
    versions = ("GCC " + re.search(r"version (\d+\.\d+)", gcc_version)[1],
                "Clang " + re.search(r"version (\d+\.\d+\.\d+)", clang_version)[1])
    text = render(measure(probe), versions)
    if args.check:
        if OUT.read_text() != text:
            print("%s is out of date" % OUT.relative_to(ROOT), file=sys.stderr)
            return 1
        return 0
    OUT.parent.mkdir(exist_ok=True)
    OUT.write_text(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
