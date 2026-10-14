#!/usr/bin/env python3
"""Differential record-layout fuzzer: bcc against the platform compilers.

Each program declares random structures and unions (scalars, arrays, bit-fields
of every integer type including zero-width and unnamed ones, nested records,
explicitly aligned members, `#pragma pack` and, on the GNU targets,
`__attribute__((packed))`) and prints, for each record, its `sizeof` and
`_Alignof`, the byte image after storing all ones into each named member, and
for each named bit-field the value read back after storing a random value into
a record filled with 0x5a, followed by the record's bytes.

Targets and references (run after `cargo build --release`; each target has a
primary reference and Clang):

- `gnu`: x86_64-w64-windows-gnu, MinGW-w64 GCC (`gcc` on PATH, primary) and
  the bundled Clang.
- `msvc`: x86_64-pc-windows-msvc, Microsoft's `cl.exe` (found through
  `vswhere` and `vcvars64.bat`, primary) and the bundled Clang. `cl.exe` has
  no GNU attributes, so this flavour aligns members with `_Alignas` only.
- `linux`: x86_64-unknown-linux-gnu, GCC (primary) and Clang inside WSL; bcc's
  object file is linked inside WSL.

Each record is classified by whether the references agree and whom bcc
follows. A failure is a record on which bcc differs from agreeing references
(or from the only reference whose program ran), or a bcc program that did not
build or run. Where the references disagree bcc may follow either: on MinGW it
follows GCC for zero-width bit-fields, whose packed layouts Clang overlaps,
and Clang elsewhere (`semantic-analysis.md`, "Record layout").

Usage:

    python scripts/layout_fuzz.py --target gnu --programs 100 --records 50
    python scripts/layout_fuzz.py --target msvc --seed 7 --keep
    python scripts/layout_fuzz.py --fixture tests/fixtures/layout --seed 1 --records 60

`--fixture` writes the deterministic regression programs that
`pipeline::tests::middle_end::record_layout` runs in the interpreter: one
program per target, with the output lines of the records on which both
references agree.

The program uses no headers (`putchar` only), so no C library headers are needed.
Exit status: 0 without failures, 1 otherwise.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from collections import Counter
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import crash_dialogs  # noqa: E402

crash_dialogs.suppress()

ROOT = Path(__file__).resolve().parent.parent
BCC = ROOT / "target/x86_64-pc-windows-gnu/release/bcc-rust.exe"
CLANG = ROOT / "target/llvm/bin/clang.exe"
TRIPLES = {"gnu": "x86_64-w64-windows-gnu", "msvc": "x86_64-pc-windows-msvc",
           "linux": "x86_64-unknown-linux-gnu"}
# (spelling, bits, natural alignment in bytes)
TYPES = [("char", 8), ("signed char", 8), ("unsigned char", 8), ("short", 16),
         ("unsigned short", 16), ("int", 32), ("unsigned", 32), ("long long", 64),
         ("unsigned long long", 64), ("_Bool", 1)]
SIGNED = {"char", "signed char", "short", "int", "long long"}

PRELUDE = r"""int putchar(int);
static void ps(const char *s) { while (*s) putchar(*s++); }
static void pu(unsigned long long v) { char b[24]; int n = 0; do b[n++] = (char)('0' + v % 10); while (v /= 10); while (n) putchar(b[--n]); }
static void pi(long long v) { if (v < 0) { putchar('-'); pu(0ull - (unsigned long long)v); } else pu((unsigned long long)v); }
static void img(const void *p, unsigned long long n) { const unsigned char *b = (const unsigned char *)p; putchar(':'); while (n--) { putchar("0123456789abcdef"[*b >> 4]); putchar("0123456789abcdef"[*b & 15]); b++; } }
static void fill(void *p, int v, unsigned long long n) { unsigned char *b = (unsigned char *)p; while (n--) *b++ = (unsigned char)v; }
"""


class Generator:
    """Random records for one target flavour; `gnu` covers the GNU attribute spellings."""

    def __init__(self, seed, flavour, zero_width, rich):
        self.rng = random.Random(seed)
        self.flavour = flavour
        self.zero_width = zero_width
        self.rich = rich
        self.kinds = []
        self.sizes = []

    def record(self, index):
        rng, gnu = self.rng, self.flavour != "msvc"
        kind = "union" if rng.random() < 0.15 else "struct"
        self.kinds.append(kind)
        lines, fields = [], []
        for f in range(rng.randint(1, 8)):
            ty, bits = rng.choice(TYPES)
            name = f"f{f}"
            if ty == "_Bool" or rng.random() < 0.6:
                if rng.random() < self.zero_width:
                    lines.append(f"{ty} : 0;")
                    continue
                width = rng.randint(1, bits)
                attribute = ""
                if gnu and self.rich and rng.random() < 0.04:
                    attribute = f" __attribute__((aligned({rng.choice([1, 2, 4, 8, 16])})))"
                elif gnu and self.rich and rng.random() < 0.04:
                    attribute = " __attribute__((packed))"
                if rng.random() < 0.1:
                    lines.append(f"{ty} : {width}{attribute};")
                    continue
                lines.append(f"{ty} {name} : {width}{attribute};")
                if ty == "_Bool":
                    value = rng.randint(0, 1)
                elif ty in SIGNED:
                    value = rng.randint(-(1 << (width - 1)) + 1, (1 << (width - 1)) - 1)
                else:
                    value = rng.randint(0, (1 << width) - 1)
                fields.append(("bits", name, value))
                continue
            choice = rng.random()
            if choice < 0.15:
                lines.append(f"{ty} {name}[{rng.randint(1, 3)}];")
                fields.append(("array", name, None))
            elif self.rich and index > 0 and choice < 0.3:
                prev = rng.randrange(index)
                lines.append(f"{self.kinds[prev]} T{prev} {name};")
                fields.append(("record", name, None))
            else:
                attribute, prefix = "", ""
                if self.rich and rng.random() < 0.12:
                    natural = max(1, bits // 8)
                    if gnu:
                        attribute = f" __attribute__((aligned({rng.choice([1, 2, 4, 8, 16])})))"
                    else:
                        prefix = f"_Alignas({rng.choice([n for n in (1, 2, 4, 8, 16) if n >= natural])}) "
                elif gnu and self.rich and rng.random() < 0.05:
                    attribute = " __attribute__((packed))"
                lines.append(f"{prefix}{ty} {name}{attribute};")
                fields.append(("scalar", name, None))
        if not fields:
            lines.append("int fz;")
            fields.append(("scalar", "fz", None))
        packs = [None, None, 1, 2, 4, 8] + (["attr"] if gnu else [])
        pack = rng.choice(packs)
        after = ""
        if pack == "attr":
            after = " __attribute__((packed))"
        if gnu and self.rich and rng.random() < 0.05:
            after += f" __attribute__((aligned({rng.choice([1, 2, 4, 8, 16, 32])})))"
        text = f"#pragma pack(push, {pack})\n" if isinstance(pack, int) else ""
        text += f"{kind} T{index} {{ {' '.join(lines)} }}{after};\n"
        if isinstance(pack, int):
            text += "#pragma pack(pop)\n"
        return text, kind, fields

    def program(self, count):
        decls, functions = [], []
        for index in range(count):
            text, kind, fields = self.record(index)
            decls.append(text)
            tag = f"{kind} T{index}"
            body = [f'{tag} s; ps("T{index} "); pu(sizeof s); putchar(\' \'); pu(_Alignof({tag}));']
            for field, name, _ in fields:
                if field == "record":
                    store = f"fill(&s.{name}, 0xff, sizeof s.{name});"
                elif field == "array":
                    store = f"s.{name}[0] = -1;"
                else:
                    store = f"s.{name} = -1;"
                body.append(f'fill(&s, 0, sizeof s); {store} ps(" {name}"); img(&s, sizeof s);')
            for field, name, value in fields:
                if field == "bits":
                    body.append(f'fill(&s, 0x5a, sizeof s); s.{name} = {value}LL; ps(" {name}="); '
                                f'pi((long long)s.{name}); img(&s, sizeof s);')
            body.append("putchar('\\n');")
            functions.append(f"static void t{index}(void) {{\n    " + "\n    ".join(body) + "\n}\n")
        main = "int main(void) {\n" + "".join(f"    t{i}();\n" for i in range(count)) + "    return 0;\n}\n"
        return PRELUDE + "".join(decls) + "".join(functions) + main


def vcvars_environment():
    """The environment `vcvars64.bat` sets up for `cl.exe`, or None."""
    vswhere = Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / \
        "Microsoft Visual Studio/Installer/vswhere.exe"
    roots = []
    if vswhere.is_file():
        found = subprocess.run([str(vswhere), "-products", "*", "-latest", "-property", "installationPath"],
                               capture_output=True, text=True).stdout.strip()
        if found:
            roots.append(Path(found))
    for root in roots:
        script = root / "VC/Auxiliary/Build/vcvars64.bat"
        if script.is_file():
            out = subprocess.run(f'cmd /s /c ""{script}" >nul && set"', capture_output=True, text=True,
                                 shell=True).stdout
            return dict(line.split("=", 1) for line in out.splitlines() if "=" in line)
    return None


def wsl_path(path):
    path = Path(path).resolve().as_posix()
    return f"/mnt/{path[0].lower()}{path[2:]}"


def run(command, cwd, env=None, timeout=600):
    """Exit status, standard output and standard error; status None on timeout."""
    try:
        result = subprocess.run(command, cwd=cwd, env=env, capture_output=True, text=True,
                                timeout=timeout, errors="replace")
    except subprocess.TimeoutExpired:
        return None, "", "timeout"
    return result.returncode, result.stdout, result.stderr


def build_and_run(target, source, work, bcc, msvc_env):
    """Each compiler's output lines for one program, keyed by compiler name."""
    triple = TRIPLES[target]
    outputs = {}
    stem = source.stem
    runner = [sys.executable, str(ROOT / "scripts/cargo_runner.py")]

    def checked(step, command, env=None):
        code, out, err = run(command, work, env)
        if code != 0:
            raise RuntimeError(f"{step} failed ({code}): {(out + err)[:4000]}")
        return out

    def native(name, command, env=None):
        exe = work / f"{stem}_{name}.exe"
        output = [f"/Fe{exe}", f"/Fo{work / (stem + '_cl.obj')}"] if name == "cl" else ["-o", str(exe)]
        checked("compile", [*command, *output], env)
        return checked("run", [*runner, str(exe)])

    def wsl(name, compile_command):
        binary = f"/tmp/layout_fuzz_{os.getpid()}_{stem}_{name}"
        checked("compile", ["wsl.exe", "--", "sh", "-c", f"{compile_command} -o {binary}"])
        try:
            return checked("run", ["wsl.exe", "--", binary])
        finally:
            run(["wsl.exe", "--", "rm", "-f", binary], work)

    def attempt(name, action):
        try:
            outputs[name] = action()
        except RuntimeError as error:
            outputs[name] = f"error: {error}"

    if target == "linux":
        wsrc = wsl_path(source)
        for name in ("gcc", "clang"):
            attempt(name, lambda name=name: wsl(name, f"{name} -w -O0 '{wsrc}'"))
        obj = work / f"{stem}_bcc.o"

        def bcc_linux():
            checked("compile", [str(bcc), f"--target={triple}", "-w", "-c", str(source), "-o", str(obj)])
            return wsl("bcc", f"clang '{wsl_path(obj)}'")
        attempt("bcc", bcc_linux)
    elif target == "gnu":
        attempt("gcc", lambda: native("gcc", ["gcc", "-w", "-O0", str(source)]))
        attempt("clang", lambda: native("clang", [str(CLANG), f"--target={triple}", "-w", "-O0", str(source)]))
        attempt("bcc", lambda: native("bcc", [str(bcc), f"--target={triple}", "-w", str(source)]))
    else:
        cl = shutil.which("cl.exe", path=msvc_env.get("PATH", msvc_env.get("Path"))) or "cl.exe"
        attempt("cl", lambda: native("cl", [cl, "/nologo", "/std:c11", "/Od", "/w", str(source)], msvc_env))
        attempt("clang", lambda: native("clang", [str(CLANG), f"--target={triple}", "-w", "-O0", str(source)]))
        attempt("bcc", lambda: native("bcc", [str(bcc), f"--target={triple}", "-w", str(source)]))
    for leftover in work.glob(f"{stem}_*"):
        leftover.unlink(missing_ok=True)
    return {name: text.replace("\r", "").splitlines() for name, text in outputs.items()}


PRIMARY = {"gnu": "gcc", "msvc": "cl", "linux": "gcc"}


def compare(target, lines, source_text):
    """Classify each record line: agreement of the references and whether bcc follows the primary."""
    primary = PRIMARY[target]
    counts, failures = Counter(), []
    decls, previous = {}, ""
    for line in source_text.splitlines():
        if line.startswith(("struct T", "union T")):
            decls[line.split()[1]] = (previous + " " if previous.startswith("#pragma") else "") + line
        previous = line
    # A reference that crashes (Clang's MinGW layout of packed records with
    # zero-width bit-fields can overlap fields) loses only the later records.
    records = {name: {line.split()[0]: line for line in output if line.startswith("T")}
               for name, output in lines.items()}
    for name, declaration in decls.items():
        bcc = records["bcc"].get(name)
        reference, clang = records[primary].get(name), records["clang"].get(name)
        if bcc is None:
            counts["bcc-error"] += 1
            failures.append({"record": declaration, "outputs": {k: v[-3:] for k, v in lines.items()}})
            continue
        if reference is None or clang is None:
            only = reference if clang is None else clang
            if only is None:
                counts["references-failed"] += 1
                continue
            counts["one reference, match" if bcc == only else "one reference, mismatch"] += 1
            if bcc != only:
                failures.append({"record": declaration, "bcc": bcc, primary: reference, "clang": clang})
            continue
        agree = reference == clang
        if bcc == reference:
            counts["match" if agree else "refs-disagree, bcc follows " + primary] += 1
            continue
        if agree:
            counts["mismatch"] += 1
        else:
            counts["refs-disagree, bcc follows clang" if bcc == clang else "refs-disagree, bcc follows neither"] += 1
        failures.append({"record": declaration, "bcc": bcc, primary: reference, "clang": clang})
    return counts, failures


def job(arguments, seed, msvc_env):
    flavour = arguments.target
    generator = Generator(seed, flavour, arguments.zero_width, not arguments.plain)
    work = arguments.output / arguments.target
    source = work / f"lf_{seed}.c"
    text = generator.program(arguments.records)
    source.write_text(text, encoding="utf-8")
    lines = build_and_run(arguments.target, source, work, arguments.bcc, msvc_env)
    counts, failures = compare(arguments.target, lines, text)
    if not failures and not arguments.keep:
        source.unlink(missing_ok=True)
    return seed, counts, failures


def write_fixture(arguments, msvc_env):
    """One deterministic program per target, with the output lines of the
    records on which both references agree (bcc's policy where they differ
    is pinned by the semantic tests instead)."""
    directory = arguments.fixture
    directory.mkdir(parents=True, exist_ok=True)
    work = arguments.output / "fixture"
    work.mkdir(parents=True, exist_ok=True)
    for target in ("gnu", "msvc", "linux"):
        generator = Generator(arguments.seed, target, arguments.zero_width, not arguments.plain)
        text = generator.program(arguments.records)
        source = work / f"layout_{target}.c"
        source.write_text(text, encoding="utf-8")
        lines = build_and_run(target, source, work, arguments.bcc, msvc_env)
        counts, _ = compare(target, lines, text)
        print(target, dict(counts))
        clang = set(lines["clang"])
        expected = [line for line in lines[PRIMARY[target]] if line.startswith("T") and line in clang]
        (directory / f"layout_{target}.c").write_text(text, encoding="utf-8", newline="\n")
        (directory / f"layout_{target}.expected").write_text(
            "\n".join(expected) + "\n", encoding="utf-8", newline="\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--target", choices=sorted(TRIPLES), default="gnu")
    parser.add_argument("--bcc", type=Path, default=BCC)
    parser.add_argument("--output", type=Path, default=ROOT / "target/layout-fuzz")
    parser.add_argument("--seed", type=int, default=1, help="first program seed")
    parser.add_argument("--programs", type=int, default=1)
    parser.add_argument("--records", type=int, default=50, help="records per program")
    parser.add_argument("--zero-width", type=float, default=0.15, help="probability of a zero-width bit-field")
    parser.add_argument("--plain", action="store_true", help="no nested records or alignment attributes")
    parser.add_argument("--jobs", type=int, default=6)
    parser.add_argument("--keep", action="store_true", help="keep sources that matched")
    parser.add_argument("--show", type=int, default=20, help="differing records to print")
    parser.add_argument("--fixture", type=Path, help="write the deterministic test programs here")
    arguments = parser.parse_args()
    arguments.bcc = arguments.bcc.resolve()
    arguments.output = arguments.output.resolve()
    msvc_env = None
    if arguments.target == "msvc" or arguments.fixture:
        msvc_env = vcvars_environment()
        if msvc_env is None:
            sys.exit("layout_fuzz.py: cl.exe not found (Visual Studio Build Tools)")
    if arguments.fixture:
        write_fixture(arguments, msvc_env)
        return 0
    (arguments.output / arguments.target).mkdir(parents=True, exist_ok=True)
    totals, failures = Counter(), []
    with ThreadPoolExecutor(arguments.jobs) as pool:
        futures = [pool.submit(job, arguments, seed, msvc_env)
                   for seed in range(arguments.seed, arguments.seed + arguments.programs)]
        for future in as_completed(futures):
            seed, counts, found = future.result()
            totals.update(counts)
            failures.extend(dict(item, seed=seed) for item in found)
    report = arguments.output / f"{arguments.target}-report.json"
    report.write_text(json.dumps({"totals": totals, "failures": failures}, indent=1), encoding="utf-8")
    for item in failures[:arguments.show]:
        print(json.dumps(item, indent=1))
    print(f"{arguments.target}: {sum(totals.values())} records:", dict(totals))
    print(f"report: {report}")
    # Failures: bcc differs from agreeing references or from the only
    # reference that ran, or bcc's program failed. Where the references
    # disagree, bcc may follow either (it follows the primary for zero-width
    # bit-fields and Clang elsewhere); the counts report which.
    bad = sum(totals[k] for k in ("mismatch", "one reference, mismatch", "bcc-error"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
