#!/usr/bin/env python3
"""Record the bundled Clang's pointer diagnostics across targets and dialects."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import subprocess

import crash_dialogs

ROOT = Path(__file__).resolve().parent.parent
MODES = "c89 gnu89 iso9899:199409 c99 gnu99 c11 gnu11 c17 gnu17 c23 gnu23 c2y gnu2y".split()
TARGETS = "x86_64-unknown-linux-gnu x86_64-w64-windows-gnu x86_64-pc-windows-msvc".split()
CASES = {
    "distinct-equality": "int f(int *p, unsigned *q) { return p == q; }",
    "distinct-order": "int f(int *p, float *q) { return p < q; }",
    "object-assignment": "void f(char *p, float *q) { p = q; }",
    "object-initialization": "void f(float *q) { char *p = q; }",
    "object-argument": "void g(char *); void f(float *q) { g(q); }",
    "object-return": "char *f(float *q) { return q; }",
    "pointer-sign": "void f(int *p, unsigned *q) { p = q; }",
    "char-sign": "void f(char *p, signed char *q) { p = q; }",
    "nested-sign": "void f(int **p, unsigned **q) { p = q; }",
    "discard-qualifiers": "void f(int *p, const int *q) { p = q; }",
    "nested-qualifiers": "void f(const int **p, int **q) { p = q; }",
    "nested-discard-qualifiers": "void f(int **p, const int **q) { p = q; }",
    "pointer-qualifiers": "void f(int **p, int *const *q) { p = q; }",
    "function-assignment": "void f(int (*p)(int), int (*q)(double)) { p = q; }",
    "function-object": "void f(int (*p)(int), int *q) { p = q; }",
    "function-void": "void f(int (*p)(int), void *q) { p = q; }",
    "integer-pointer": "void f(int *p, int q) { p = q; }",
    "pointer-integer": "void f(int p, int *q) { p = q; }",
    "pointer-integer-equality": "int f(int *p, int q) { return p == q; }",
    "pointer-integer-order": "int f(int *p, int q) { return p < q; }",
    "pointer-zero-order": "int f(int *p) { return p < 0; }",
    "function-order": "int f(int (*p)(int), int (*q)(int)) { return p < q; }",
    "function-distinct-order": "int f(int (*p)(int), int (*q)(double)) { return p < q; }",
    "function-equality": "int f(int (*p)(int), int (*q)(double)) { return p == q; }",
    "void-order": "int f(int *p, void *q) { return p < q; }",
}


def probe(clang, command, case, target, mode):
    result = subprocess.run([str(clang), *command], capture_output=True, text=True, check=False)
    return dict(case=case, target=target, mode=mode, command=[str(clang), *command],
                exit_code=result.returncode, stderr=result.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--flags-only', action='store_true')
    args = parser.parse_args()
    crash_dialogs.suppress()
    output = ROOT / "target/pointer-diagnostics"
    output.mkdir(parents=True, exist_ok=True)
    clang = ROOT / "target/llvm/bin/clang.exe"
    tasks = []
    for name, source in CASES.items():
        path = output / f"{name}.c"
        path.write_text(source + "\n", encoding="utf-8")
        if args.flags_only:
            continue
        for target in TARGETS:
            for mode in MODES:
                command = [f"--target={target}", f"-std={mode}",
                           "-fsyntax-only", "-fno-color-diagnostics", str(path)]
                tasks.append((command, name, target, mode))
    if args.flags_only:
        for name, group in [
            ('distinct-equality', 'compare-distinct-pointer-types'),
            ('object-assignment', 'incompatible-pointer-types'),
            ('pointer-sign', 'pointer-sign'),
            ('nested-qualifiers', 'incompatible-pointer-types-discards-qualifiers'),
            ('function-assignment', 'incompatible-function-pointer-types'),
            ('integer-pointer', 'int-conversion'),
            ('pointer-integer-equality', 'pointer-integer-compare'),
            ('function-order', 'ordered-compare-function-pointers'),
        ]:
            for flags in [[], ['-Werror'], ['-pedantic-errors'], [f'-W{group}'],
                          [f'-Werror={group}'], [f'-Wno-{group}'],
                          [f'-Wno-{group}', f'-W{group}'],
                          ['-Werror', f'-Wno-error={group}'],
                          [f'-Wno-error={group}', '-Werror'],
                          ['-w', '-Werror'],
                          ['-Werror', f'-Wno-{group}', f'-W{group}']]:
                command = [f'--target={TARGETS[1]}', '-std=gnu17', '-fsyntax-only',
                           '-fno-color-diagnostics', *flags, str(output / f'{name}.c')]
                tasks.append((command, name, TARGETS[1], 'gnu17'))
        for flags in [
            ['-Wno-compare-distinct-pointer-types', '-Werror=ordered-compare-function-pointers'],
            ['-Wno-ordered-compare-function-pointers', '-Werror=compare-distinct-pointer-types'],
        ]:
            name = 'function-distinct-order'
            command = [f'--target={TARGETS[1]}', '-std=gnu17', '-fsyntax-only',
                       '-fno-color-diagnostics', *flags, str(output / f'{name}.c')]
            tasks.append((command, name, TARGETS[1], 'gnu17'))
    with ThreadPoolExecutor(max_workers=8) as pool:
        rows = list(pool.map(lambda task: probe(clang, *task), tasks))
    version = subprocess.run([str(clang), "--version"], capture_output=True, text=True,
                             check=True).stdout
    (output / ("flags.json" if args.flags_only else "matrix.json")).write_text(
        json.dumps(dict(version=version, rows=rows), indent=2) + "\n", encoding="utf-8")
    for name in CASES:
        messages = sorted({line.split(": ", 1)[1] for row in rows if row["case"] == name
                           for line in row["stderr"].splitlines()
                           if ": warning:" in line or ": error:" in line})
        print(name, " | ".join(messages) or "silent")


if __name__ == "__main__":
    main()
