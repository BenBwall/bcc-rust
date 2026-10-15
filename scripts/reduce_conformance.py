#!/usr/bin/env python3
"""Minimize a C program that bcc handles differently from Clang (delta reduction).

C-Reduce is not available, so this is a small stand-in: it first preprocesses the
program (Clang -E -P, so no other file is needed), then alternates two passes until
nothing shrinks or the time limit passes.

    lines    delete chunks of lines, from half of the file down to single lines
    groups   replace a balanced `( ... )` group, largest first, with `0` or `1`

A candidate counts only if it is still *interesting*:

 1. Clang -O0 builds it, and for the output-based kinds it runs and exits 0;
 2. unless --no-ub-check, Clang -O2 and MinGW GCC -O0/-O2 print the same output
    (removing code can introduce undefined behavior, so the reference set has
    to agree before the difference is blamed on bcc);
 3. the chosen bcc arm still ends with the same outcome (`--kind`), and for the
    refusal and crash kinds its message still contains `--message`.

    python scripts/reduce_conformance.py test.c --arm A2 --kind wrong-output \\
        -I csmith/runtime --output reduced.c

Everything stays beneath the output directory's scratch folder (target/).
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
from pathlib import Path
import shutil
import sys
import tempfile
import threading
import time

from conformance_common import (ARMS, BCC, CLANG, MINGW_DEFINES, OUTCOMES, ROOT, TARGET, clang_command,
                                environment, invoke, mingw_include_dirs, run_arm)

# Called with every accepted candidate so that an interrupted reduction leaves its best program behind.
ON_PROGRESS = lambda text: None  # noqa: E731

DEADLINES = {"compile": 60, "execute": 20, "interpret": 60, "reference_compile": 60, "reference_run": 20}
def preprocess(path, includes, system):
    """Expand the program with Clang's preprocessor so that it needs no other file."""
    sub = Path(tempfile.mkdtemp(prefix="preprocess-", dir=ROOT / "target"))
    try:
        command = [str(CLANG), f"--target={TARGET}", "-E", "-P", "-w", str(path)]
        for directory in [*includes, *system]:
            command += ["-I", str(directory)]
        for define in MINGW_DEFINES:
            command += ["-D", define]
        result = invoke(command, 120, environment(sub), sub)
        if result["exit"] != 0:
            sys.exit("clang -E failed: " + result["stderr"][:500])
        return result["stdout"]
    finally:
        shutil.rmtree(sub, ignore_errors=True)


class Oracle:
    def __init__(self, args, system_includes):
        self.args = args
        self.system = system_includes
        self.defines = MINGW_DEFINES
        self.cache = {}
        self.lock = threading.Lock()
        self.scratch = Path(tempfile.mkdtemp(prefix="reduce-", dir=ROOT / "target"))
        self.counter = 0
        self.tests = 0

    def _directory(self):
        with self.lock:
            self.counter += 1
            number = self.counter
        directory = self.scratch / str(number)
        directory.mkdir()
        return directory

    def __call__(self, text):
        key = hashlib.sha1(text.encode("utf-8", errors="replace")).hexdigest()
        with self.lock:
            if key in self.cache:
                return self.cache[key]
        result = self.evaluate(text)
        with self.lock:
            self.cache[key] = result
            self.tests += 1
        return result

    def build_and_run(self, work, name, compiler, opt, source, run=True):
        """Build the program with one reference compiler; return its run (or True when not run)."""
        args = self.args
        sub = work / name
        sub.mkdir()
        exe = sub / "ref.exe"
        if compiler == CLANG:
            command = clang_command(CLANG, [source], exe, opt=opt, includes=args.include + self.system,
                                    defines=self.defines)
        else:
            command = [compiler, opt, "-w", "-o", str(exe), str(source)]
            for directory in args.include:
                command += ["-I", str(directory)]
            for define in self.defines:
                command += ["-D", define]
        built = invoke(command, 60, environment(sub), sub)
        if built["exit"] != 0 or not exe.is_file():
            return None
        if not run:
            return True
        ran = invoke([str(exe)], 20, environment(sub), sub)
        if ran["timeout"] or ran["exit"] != 0 or not ran["stdout"].strip():
            return None
        return ran

    def evaluate(self, text):
        args = self.args
        work = self._directory()
        try:
            source = work / "reduced.c"
            source.write_text(text, encoding="utf-8")
            output_kind = args.kind in ("wrong-output", "abnormal-exit", "timeout", "interpreter-trap")
            # The cheap checks come first: Clang -O0 must accept (and for output kinds run) the
            # program and the bcc arm must still misbehave; only then are the undefined-behavior
            # cross-checks (Clang -O2, GCC -O0/-O2) worth their compile time.
            reference = self.build_and_run(work, "ref0", CLANG, "-O0", source, run=output_kind)
            if reference is None:
                return False
            if reference is True:
                reference = {"exit": 0, "stdout": "", "stderr": "", "timeout": False}
            sub = work / args.arm
            sub.mkdir()
            run = run_arm(args.bcc, args.arm, source, sub, deadlines=DEADLINES, reference=reference,
                          command_kwargs={"includes": args.include, "system_includes": self.system,
                                          "defines": self.defines})
            if run["status"] != args.kind:
                return False
            if args.message:
                haystack = (run.get("detail") or "") + " " + " ".join(run.get("constructs") or []) + " " + (
                    run["_compile"]["stderr"] if run.get("_compile") else "")
                if args.message not in haystack:
                    return False
            if output_kind and not args.no_ub_check:
                for index, (compiler, opt) in enumerate([(CLANG, "-O2"), ("gcc", "-O0"), ("gcc", "-O2")], 1):
                    other = self.build_and_run(work, f"ref{index}", compiler, opt, source)
                    if other is None or other["stdout"].strip() != reference["stdout"].strip():
                        return False
            return True
        finally:
            shutil.rmtree(work, ignore_errors=True)


def parallel_first(oracle, candidates, jobs):
    """Return the first candidate (in order) that is interesting, testing in batches."""
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        for start in range(0, len(candidates), jobs):
            batch = candidates[start:start + jobs]
            for candidate, result in zip(batch, pool.map(oracle, batch)):
                if result:
                    return candidate
    return None


def line_pass(oracle, text, jobs, deadline):
    """Delete chunks of lines, from half of the file down to single lines.

    Each round tests up to `jobs` chunks in parallel against the same text and keeps the first
    that still reproduces; the scan then resumes at that position (the lines after the deleted
    chunk moved up into it), so a deletion never makes the scan start over.
    """
    lines = text.splitlines(keepends=True)
    size = max(1, len(lines) // 2)
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        while size >= 1 and time.monotonic() < deadline:
            position = 0
            while position < len(lines) and time.monotonic() < deadline:
                starts = list(range(position, len(lines), size))[:jobs]
                candidates = ["".join(lines[:start] + lines[start + size:]) for start in starts]
                hit = next((index for index, ok in enumerate(pool.map(oracle, candidates)) if ok), None)
                if hit is None:
                    position = starts[-1] + size
                    continue
                lines = candidates[hit].splitlines(keepends=True)
                position = starts[hit]
                ON_PROGRESS(candidates[hit])
                print(f"  lines: {len(lines)} left (chunk {size})", file=sys.stderr, flush=True)
            size //= 2
    return "".join(lines)


def groups(text):
    """Spans of balanced parenthesis groups, outside strings, largest first."""
    stack, spans = [], []
    quote = None
    index = 0
    while index < len(text):
        char = text[index]
        if quote:
            if char == "\\":
                index += 1
            elif char == quote:
                quote = None
        elif char in "\"'":
            quote = char
        elif char == "(":
            stack.append(index)
        elif char == ")" and stack:
            spans.append((stack.pop(), index + 1))
        index += 1
    spans.sort(key=lambda span: span[0] - span[1])
    return spans


def group_pass(oracle, text, jobs, deadline):
    progress = True
    while progress and time.monotonic() < deadline:
        progress = False
        # Only statements in function bodies matter; skip lines with preprocessor text.
        spans = [span for span in groups(text) if span[1] - span[0] > 3
                 and "#" not in text[text.rfind("\n", 0, span[0]) + 1:span[0]]]
        candidates = []
        for start, end in spans[:400]:
            for replacement in ("0", "1"):
                candidates.append(text[:start] + replacement + text[end:])
        found = parallel_first(oracle, candidates, jobs)
        if found is not None:
            text = found
            ON_PROGRESS(found)
            progress = True
            print(f"  groups: {len(text.splitlines())} lines, {len(text)} bytes", file=sys.stderr, flush=True)
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source", type=Path)
    parser.add_argument("--arm", choices=list(ARMS), required=True)
    parser.add_argument("--kind", choices=[o for o in OUTCOMES if o != "pass"], required=True)
    parser.add_argument("--message", help="substring the bcc diagnostic must keep containing")
    parser.add_argument("-I", dest="include", action="append", type=Path, default=[])
    parser.add_argument("--bcc", type=Path, default=BCC)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--jobs", type=int, default=12)
    parser.add_argument("--time-limit", type=float, default=900)
    parser.add_argument("--no-ub-check", action="store_true")
    parser.add_argument("--no-preprocess", action="store_true", help="reduce the source text as written")
    args = parser.parse_args()
    args.output = args.output.resolve()
    if not args.output.is_relative_to((ROOT / "target").resolve()):
        parser.error("--output must be beneath this worktree's target/")
    args.bcc = args.bcc.resolve(strict=True)
    args.source = args.source.resolve(strict=True)
    args.include = [path.resolve() for path in args.include]
    system = mingw_include_dirs()
    text = args.source.read_text(encoding="utf-8", errors="replace") if args.no_preprocess else preprocess(
        args.source, args.include, system)
    oracle = Oracle(args, system)
    try:
        if not oracle(text):
            sys.exit("the starting program is not interesting under the given arm/kind/message")
        args.output.parent.mkdir(parents=True, exist_ok=True)
        partial = args.output.with_name(args.output.name + ".partial")
        globals()["ON_PROGRESS"] = lambda current: partial.write_text(current, encoding="utf-8")
        deadline = time.monotonic() + args.time_limit
        previous = None
        while previous != text and time.monotonic() < deadline:
            previous = text
            text = line_pass(oracle, text, args.jobs, deadline)
            text = group_pass(oracle, text, args.jobs, deadline)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text, encoding="utf-8")
        print(f"wrote {args.output} ({len(text.splitlines())} lines, {oracle.tests} candidates tested)")
    finally:
        shutil.rmtree(oracle.scratch, ignore_errors=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
