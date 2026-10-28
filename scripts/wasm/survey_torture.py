#!/usr/bin/env python3
"""Survey GCC's c-torture execute tests through bcc's WebAssembly back end.

Each test is compiled with `--target=wasm32-wasip1 -o test.wasm` and run under
Node's WASI (scripts/wasm/run.mjs); exit status 0 passes (the tests call
abort() on failure). Compiler refusals are grouped by their message.

    python3 scripts/wasm/survey_torture.py --bcc target/release/bcc-rust \
        --source GCC_SRC/gcc/testsuite/gcc.c-torture/execute [--jobs 4]
"""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent.parent
RUNNER = ROOT / "scripts/wasm/run.mjs"
OPTIONS = re.compile(r'\{\s*dg-(?:additional-)?options\s+"([^"]*)"')


def survey(path, bcc, timeout, scratch):
    source = path.read_text(errors="replace")
    flags = []
    for match in OPTIONS.finditer(source):
        flags += [flag for flag in match.group(1).split() if flag.startswith(("-std=", "-D", "-U", "-f"))]
    output = Path(scratch) / (path.stem + ".wasm")
    command = [str(bcc), "--target=wasm32-wasip1", "-w", *flags, "-o", str(output), str(path)]
    try:
        compiled = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return path.name, "compile-timeout", ""
    if compiled.returncode != 0:
        text = compiled.stderr
        message = next((line for line in text.splitlines() if line.startswith("error")), text[:200])
        if "panicked" in text:
            return path.name, "compiler-crash", message
        if "WebAssembly back end does not support" in text:
            return path.name, "wasm-unsupported", re.sub(r" \(in function .*\)", "", message)
        if "undefined symbol" in text:
            return path.name, "undefined-symbol", message
        if "not yet supported" in text or "unsupported" in text:
            return path.name, "lowering-unsupported", message
        return path.name, "frontend-error", message
    try:
        ran = subprocess.run(["node", "--no-warnings", str(RUNNER), str(output)],
                             capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return path.name, "run-timeout", ""
    if ran.returncode == 0:
        return path.name, "pass", ""
    return path.name, "wrong-result", f"status {ran.returncode} {ran.stderr.strip()[:200]}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bcc", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--timeout", type=float, default=20)
    parser.add_argument("--json", type=Path, help="Write every result here")
    args = parser.parse_args()
    tests = sorted(args.source.glob("*.c"))
    with tempfile.TemporaryDirectory() as scratch, ThreadPoolExecutor(args.jobs) as pool:
        results = list(pool.map(lambda path: survey(path, args.bcc, args.timeout, scratch), tests))
    totals = Counter(kind for _, kind, _ in results)
    print(f"{len(results)} tests")
    for kind, count in totals.most_common():
        print(f"  {kind}: {count}")
    reasons = Counter((kind, message) for _, kind, message in results
                      if kind in ("wasm-unsupported", "undefined-symbol", "lowering-unsupported"))
    print("most common refusals:")
    for (kind, message), count in reasons.most_common(25):
        print(f"  {count:4} {kind}: {message}")
    wrong = [(name, message) for name, kind, message in results if kind in ("wrong-result", "compiler-crash", "run-timeout")]
    print(f"wrong results, crashes and timeouts ({len(wrong)}):")
    for name, message in wrong:
        print(f"  {name}: {message}")
    if args.json:
        args.json.write_text(json.dumps(results, indent=1))


if __name__ == "__main__":
    main()
