#!/usr/bin/env python3
"""Compare bcc-rust's optimizer with LLVM's on a corpus of C benchmarks.

Usage: python scripts/run.py compare_optimizers [options]

Run it from the repository root after `cargo build --release`. For every
benchmark in the corpus (default benches/c) and every arm it builds the
program once, checks its output, then times N interleaved repetitions and
writes a results file and a summary. The arms (middle-end.md, "Comparison
arms"):

    A0  bcc-rust --llvm-opt=O0                                     baseline
    N   bcc-rust --llvm-opt=O2 --llvm-codegen-only                  codegen only
    A1  bcc-rust --opt=bcc --llvm-opt=O2 --llvm-codegen-only       bcc only
    A2  bcc-rust --llvm-opt=O2                                     LLVM only
    A3  bcc-rust --opt=bcc --llvm-opt=O2                           bcc, then LLVM
    R   clang -O2 x.c                                              reference

A1/N isolates what the bcc optimizer adds on top of LLVM's code generator.

`--ablate leave-one-out` adds arm A1-all (the default pass list as one
explicit `--passes` list, repeated --ablate-rounds times) and one arm
A1-no-<pass> per pass; `--ablate prefix` adds A1-prefix-<k> for each prefix
of the list. `--passes` runs a list once, so these arms approximate the
default fixpoint pipeline by repeating the list; the arm A1 itself runs the
real fixpoint pipeline. The pass list comes from `bcc-rust
--print-default-passes`.

Checks. Programs print a checksum and return 0 only if it matches a constant
in the source. For each arm the harness requires exit status 0 and the same
stdout as the reference (arm R when present, otherwise the first arm that
passed) in a warm-up run and again in every timed run. Unless
`--no-interpreter-check` is given it also builds each program with
-DBENCH_REDUCED (a workload small enough for the bcc IR interpreter) and
requires every arm's binary to print what `bcc-rust --interpret` prints. An
arm that fails a check is reported and excluded from the statistics; it is
not hidden.

Measurements. `run_s` is wall time of the whole process (start-up included;
the summary reports the start-up floor of an empty program). `cpu_user_s` and
`cpu_sys_s` come from GetProcessTimes on Windows (a 15.6 ms clock tick, too
coarse for short runs) and from getrusage elsewhere. The summary uses CPU
time (user + system) on POSIX and wall time on Windows unless `--metric`
says otherwise. Arms are interleaved within each benchmark and repetition,
rotating the order each repetition. Windows has no CPU isolation here:
final numbers belong on native Linux (docs/research/
optimizer-comparison-methodology.md, section 4.2).

Output goes to a fresh directory (default target/optimizer-bench/<time>):
results.jsonl (one JSON object per line, appended as the run proceeds),
summary.md, and the built programs under bin/. `--summarize FILE` recomputes
the summary from a results.jsonl without running anything.
"""

import argparse
import datetime
import fnmatch
import json
import math
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess
import sys
import time

import crash_dialogs

ROOT = Path(__file__).resolve().parents[1]
EXE = ".exe" if os.name == "nt" else ""
STANDARD_ARMS = {
    "A0": ("--llvm-opt=O0",),
    "N": ("--llvm-opt=O2", "--llvm-codegen-only"),
    "A1": ("--opt=bcc", "--llvm-opt=O2", "--llvm-codegen-only"),
    "A2": ("--llvm-opt=O2",),
    "A3": ("--opt=bcc", "--llvm-opt=O2"),
    "R": None,  # clang -O2 on the C source
}
ARM_DESCRIPTIONS = {
    "A0": "bcc front end, no optimizer, clang -O0 code generation",
    "N": "unoptimized bcc IR, clang -O2 code generation with LLVM IR passes disabled",
    "A1": "bcc optimizer only (fixpoint pipeline), clang -O2 code generation",
    "A2": "LLVM -O2 only, on the unoptimized bcc IR",
    "A3": "bcc optimizer, then LLVM -O2",
    "R": "clang -O2 from the C source",
}
ABLATION_BASELINE = "A1-all"
REDUCED_DEFINE = "-DBENCH_REDUCED"
DEFAULT_REPS = 7
BOOTSTRAP_RESAMPLES = 2000
MAX_PASS_LIST = 32  # PassList::CAPACITY in src/optimizer/options.rs


# --- Statistics ------------------------------------------------------------


def geometric_mean(values):
    return math.exp(sum(math.log(v) for v in values) / len(values))


def percentile(sorted_values, fraction):
    """The linearly interpolated percentile of an ascending list."""
    position = fraction * (len(sorted_values) - 1)
    low = math.floor(position)
    high = math.ceil(position)
    return sorted_values[low] + (sorted_values[high] - sorted_values[low]) * (position - low)


def ratio_summary(arm_samples, baseline_samples, resamples, seed, statistic=statistics.median):
    """Geometric mean over benchmarks of statistic(arm) / statistic(baseline).

    Both arguments map benchmark -> list of positive samples. Only benchmarks
    present in both count. The interval is a percentile bootstrap that
    resamples the repetitions of both arms within each benchmark. Returns
    (point, low, high, count), or None when no benchmark is shared.
    """
    shared = sorted(set(arm_samples) & set(baseline_samples))
    if not shared:
        return None
    point = geometric_mean([statistic(arm_samples[b]) / statistic(baseline_samples[b]) for b in shared])
    rng = random.Random(seed)
    if resamples <= 0:
        return point, None, None, len(shared)
    estimates = []
    for _ in range(resamples):
        ratios = []
        for b in shared:
            arm = rng.choices(arm_samples[b], k=len(arm_samples[b]))
            base = rng.choices(baseline_samples[b], k=len(baseline_samples[b]))
            ratios.append(statistic(arm) / statistic(base))
        estimates.append(geometric_mean(ratios))
    estimates.sort()
    return point, percentile(estimates, 0.025), percentile(estimates, 0.975), len(shared)


# --- Tools and environment -------------------------------------------------


def find_bcc(explicit):
    if explicit:
        return Path(explicit)
    candidates = []
    for profile in ("release", "debug"):
        candidates += sorted((ROOT / "target").glob(f"*/{profile}/bcc-rust{EXE}"))
        candidates.append(ROOT / "target" / profile / f"bcc-rust{EXE}")
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    sys.exit("compare_optimizers: no bcc-rust binary found; run `cargo build --release` or pass --bcc")


def find_clang(explicit):
    path = Path(explicit) if explicit else ROOT / "target" / "llvm" / "bin" / f"clang{EXE}"
    if not path.is_file():
        sys.exit(f"compare_optimizers: no clang at {path}; pass --clang")
    return path


def default_target():
    machine = platform.machine().lower()
    arch = "aarch64" if machine in ("arm64", "aarch64") else "x86_64"
    if os.name == "nt":
        return f"{arch}-w64-windows-gnu"
    if sys.platform == "darwin":
        return f"{arch}-apple-darwin"
    return f"{arch}-unknown-linux-gnu"


def capture(command):
    try:
        return subprocess.run(command, capture_output=True, text=True, timeout=60).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


def compiler_commit():
    commit = capture(["git", "-C", str(ROOT), "rev-parse", "--short=12", "HEAD"]) or "unknown"
    dirty = subprocess.run(["git", "-C", str(ROOT), "diff", "--quiet", "HEAD"]).returncode != 0
    return commit + ("-dirty" if dirty else "")


def cpu_name():
    if os.name == "nt":
        try:
            import winreg

            with winreg.OpenKey(
                winreg.HKEY_LOCAL_MACHINE, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"
            ) as key:
                return winreg.QueryValueEx(key, "ProcessorNameString")[0].strip()
        except OSError:
            pass
    return platform.processor() or platform.machine()


def host_description():
    return (
        f"{platform.system()} {platform.release()} ({platform.version()}) {platform.machine()}; "
        f"{cpu_name()}; {os.cpu_count()} logical CPUs"
    )


def warn_if_binary_is_stale(bcc):
    newest = max((p.stat().st_mtime for p in (ROOT / "src").rglob("*.rs")), default=0)
    if newest > bcc.stat().st_mtime:
        print(f"warning: {bcc} is older than the sources in src/; rebuild before trusting results",
              file=sys.stderr)


def default_pass_list(bcc):
    result = subprocess.run([str(bcc), "--print-default-passes"], capture_output=True, text=True)
    names = [n for n in result.stdout.strip().split(",") if n]
    if result.returncode != 0 or not names:
        sys.exit("compare_optimizers: `bcc-rust --print-default-passes` failed; rebuild bcc-rust")
    return names


# --- Arms ------------------------------------------------------------------


def passes_arm_flags(passes):
    if passes:
        return ("--opt=bcc", f"--passes={','.join(passes)}", "--llvm-opt=O2", "--llvm-codegen-only")
    return ("--llvm-opt=O2", "--llvm-codegen-only")  # no bcc pass at all


def ablation_arms(mode, passes, rounds):
    """Ablation arms as an ordered {name: flags} mapping.

    Every arm repeats its list `rounds` times, since `--passes` runs a list
    once and the default pipeline runs to a fixpoint.
    """
    if rounds * len(passes) > MAX_PASS_LIST:
        sys.exit(f"compare_optimizers: --ablate-rounds {rounds} exceeds the {MAX_PASS_LIST}-pass list limit")
    arms = {ABLATION_BASELINE: passes_arm_flags(passes * rounds)}
    if mode == "leave-one-out":
        for skipped in passes:
            kept = [p for p in passes if p != skipped]
            arms[f"A1-no-{skipped}"] = passes_arm_flags(kept * rounds)
    elif mode == "prefix":
        for length in range(len(passes)):
            arms[f"A1-prefix-{length}"] = passes_arm_flags(passes[:length] * rounds)
    return arms


# --- Running programs ------------------------------------------------------


class Timer:
    """Runs a program and measures wall and CPU time."""

    def __init__(self, timeout, high_priority):
        self.timeout = timeout
        self.creation_flags = 0
        if os.name == "nt" and high_priority:
            self.creation_flags = 0x00000080  # HIGH_PRIORITY_CLASS, never REALTIME
        self._windows = _WindowsTimes() if os.name == "nt" else None

    def run(self, exe):
        """Returns (wall_s, user_s, sys_s, exit_code, stdout, timed_out)."""
        before = self._children_cpu()
        start = time.perf_counter()
        process = subprocess.Popen(
            [str(exe)], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            creationflags=self.creation_flags,
        )
        timed_out = False
        try:
            stdout, _ = process.communicate(timeout=self.timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            stdout, _ = process.communicate()
            timed_out = True
        wall = time.perf_counter() - start
        if self._windows:
            user, system = self._windows.process_times(process)
        else:
            after = self._children_cpu()
            user, system = after[0] - before[0], after[1] - before[1]
        return wall, user, system, None if timed_out else process.returncode, stdout, timed_out

    @staticmethod
    def _children_cpu():
        if os.name == "nt":
            return (0.0, 0.0)
        import resource

        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        return (usage.ru_utime, usage.ru_stime)


class _WindowsTimes:
    """GetProcessTimes through ctypes; absent values come back as None."""

    def __init__(self):
        import ctypes
        from ctypes import wintypes

        class FileTime(ctypes.Structure):
            _fields_ = [("low", wintypes.DWORD), ("high", wintypes.DWORD)]

        self.FileTime = FileTime
        self.kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel32.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(FileTime)] * 4
        self.kernel32.GetProcessTimes.restype = wintypes.BOOL
        self.ctypes = ctypes

    def process_times(self, process):
        created, exited, kernel, user = (self.FileTime() for _ in range(4))
        try:
            handle = int(process._handle)  # open until the Popen object is collected
        except (AttributeError, TypeError, ValueError):
            return None, None
        ok = self.kernel32.GetProcessTimes(
            handle, *(self.ctypes.byref(t) for t in (created, exited, kernel, user))
        )
        if not ok:
            return None, None
        seconds = lambda t: ((t.high << 32) | t.low) * 1e-7  # 100 ns units
        return seconds(user), seconds(kernel)


def pin_to_cpu(cpu):
    """Restricts this process, and the programs it starts, to one CPU."""
    if hasattr(os, "sched_setaffinity"):
        os.sched_setaffinity(0, {cpu})
    elif os.name == "nt":
        import ctypes

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.GetCurrentProcess.restype = ctypes.c_void_p
        kernel32.SetProcessAffinityMask.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
        if not kernel32.SetProcessAffinityMask(kernel32.GetCurrentProcess(), 1 << cpu):
            sys.exit(f"compare_optimizers: cannot pin to CPU {cpu}")
    else:
        sys.exit("compare_optimizers: --pin-cpu is not supported on this platform")


def plain(data):
    """Output bytes with Windows line ends made plain, for comparisons."""
    return data.replace(b"\r\n", b"\n")


def run_command(command, timeout):
    """Runs a build command; returns (seconds, completed process or None)."""
    start = time.perf_counter()
    try:
        completed = subprocess.run(command, capture_output=True, timeout=timeout, stdin=subprocess.DEVNULL)
    except subprocess.TimeoutExpired:
        return time.perf_counter() - start, None
    return time.perf_counter() - start, completed


class Tools:
    def __init__(self, bcc, clang, target, build_timeout):
        self.bcc, self.clang, self.target, self.build_timeout = bcc, clang, target, build_timeout

    def build(self, arm_flags, source, exe, defines):
        """Builds `source` into `exe`; returns (seconds, error text or None)."""
        if arm_flags is None:
            command = [str(self.clang), "-O2", f"--target={self.target}", *defines, str(source), "-o", str(exe)]
        else:
            command = [str(self.bcc), str(source), f"--target={self.target}", *defines, "-o", str(exe), *arm_flags]
        seconds, completed = run_command(command, self.build_timeout)
        if completed is None:
            return seconds, "build timed out"
        if completed.returncode != 0 or not exe.is_file():
            text = (completed.stderr or completed.stdout).decode("utf-8", "replace").strip()
            return seconds, f"exit {completed.returncode}: {text[-600:]}"
        return seconds, None

    def interpret(self, source, defines):
        """Runs main in the bcc interpreter; returns (exit, stdout bytes) or None."""
        command = [str(self.bcc), str(source), f"--target={self.target}", *defines, "--interpret"]
        _, completed = run_command(command, self.build_timeout)
        if completed is None:
            return None
        return completed.returncode, plain(completed.stdout)


# --- The run ---------------------------------------------------------------


class Results:
    """Appends JSON Lines records and keeps them for the summary."""

    def __init__(self, path, common):
        self.records = []
        self.common = common
        self.file = open(path, "a", encoding="utf-8")

    def write(self, kind, **fields):
        record = {"kind": kind, **self.common, **fields}
        self.records.append(record)
        self.file.write(json.dumps(record) + "\n")
        self.file.flush()
        return record


def startup_floor(tools, work, timer):
    """Median wall time of an empty program, built with clang -O0."""
    source = work / "empty.c"
    source.write_text("int main(void) { return 0; }\n")
    exe = work / f"empty{EXE}"
    command = [str(tools.clang), "-O0", f"--target={tools.target}", str(source), "-o", str(exe)]
    _, completed = run_command(command, tools.build_timeout)
    if completed is None or completed.returncode != 0:
        return None
    timer.run(exe)
    return statistics.median(timer.run(exe)[0] for _ in range(15))


def run_benchmarks(args, tools, arms, benchmarks, output, results):
    bin_dir = output / "bin"
    bin_dir.mkdir()
    timer = Timer(args.timeout, args.high_priority)
    results.write("meta", **{
        "arms": {name: {"flags": list(flags) if flags is not None else ["clang", "-O2"]} for name, flags in arms.items()},
        "reps": args.reps, "target": tools.target, "metric": resolve_metric(args.metric),
        "bcc": str(tools.bcc), "clang": str(tools.clang),
        "clang_version": capture([str(tools.clang), "--version"]).splitlines()[0:1],
        "python": platform.python_version(), "cpu": cpu_name(),
        "startup_floor_s": startup_floor(tools, output, timer),
        "pinned_cpu": args.pin_cpu, "high_priority": args.high_priority,
        "started": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    })
    cells = {}  # (benchmark, arm) -> executable that passed every check
    for source in benchmarks:
        name = source.stem
        print(f"== {name}", flush=True)
        oracle = None
        if args.interpreter_check:
            oracle = tools.interpret(source, [REDUCED_DEFINE])
            ok = oracle is not None and oracle[0] == 0
            results.write("oracle", benchmark=name, exit=oracle[0] if oracle else None,
                          stdout=oracle[1].decode("utf-8", "replace") if oracle else None, output_ok=ok)
            if not ok:
                print(f"   warning: the interpreter did not run {name} cleanly "
                      f"(exit {oracle[0] if oracle else 'timeout'}); no interpreter check", flush=True)
                oracle = None
        verified = {}
        for arm, flags in arms.items():
            exe = bin_dir / f"{name}.{arm}{EXE}"
            compile_s, error = tools.build(flags, source, exe, [])
            if error:
                results.write("build_failed", benchmark=name, arm=arm, compile_s=compile_s, error=error)
                print(f"   {arm}: BUILD FAILED: {error.splitlines()[0] if error else ''}", flush=True)
                continue
            interpreter_ok = None
            if oracle is not None:
                interpreter_ok = check_reduced(tools, flags, source, bin_dir / f"{name}.{arm}.reduced{EXE}",
                                               oracle, timer, results, name, arm)
            wall, user, system, code, stdout, timed_out = timer.run(exe)  # warm-up and verification
            stdout = plain(stdout)
            verified[arm] = (exe, compile_s, code, stdout, timed_out, interpreter_ok)
        reference = reference_output(verified)
        for arm, (exe, compile_s, code, stdout, timed_out, interpreter_ok) in verified.items():
            matches = reference is not None and stdout == reference
            output_ok = code == 0 and matches and interpreter_ok is not False
            reason = None
            if timed_out:
                reason = "timeout"
            elif code != 0:
                reason = f"exit {code} (the program's own check failed or it crashed)"
            elif not matches:
                reason = "stdout differs from the reference"
            elif interpreter_ok is False:
                reason = "differs from the interpreter on the reduced workload"
            results.write("verify", benchmark=name, arm=arm, compile_s=compile_s, exit=code,
                          output_ok=output_ok, reason=reason, stdout=stdout.decode("utf-8", "replace").strip())
            print(f"   {arm}: compile {compile_s:.2f}s  {'ok' if output_ok else 'FAILED: ' + str(reason)}", flush=True)
            if output_ok:
                cells[(name, arm)] = (exe, compile_s, reference)
    time_cells(args, benchmarks, arms, cells, timer, results)


def check_reduced(tools, flags, source, exe, oracle, timer, results, name, arm):
    """Builds the reduced workload and compares it with the interpreter."""
    _, error = tools.build(flags, source, exe, [REDUCED_DEFINE])
    if error:
        results.write("interpreter_check", benchmark=name, arm=arm, output_ok=False, error=error)
        return False
    _, _, _, code, stdout, timed_out = timer.run(exe)
    ok = not timed_out and (code, plain(stdout)) == oracle
    results.write("interpreter_check", benchmark=name, arm=arm, output_ok=ok, exit=code,
                  stdout=stdout.decode("utf-8", "replace").strip())
    return ok


def reference_output(verified):
    """stdout of arm R if it passed its own check, else of the first arm that did."""
    order = ["R"] + [arm for arm in verified if arm != "R"]
    for arm in order:
        if arm in verified and verified[arm][2] == 0 and not verified[arm][4]:
            return verified[arm][3]
    return None


def time_cells(args, benchmarks, arms, cells, timer, results):
    arm_names = list(arms)
    for rep in range(args.reps):
        print(f"-- repetition {rep + 1}/{args.reps}", flush=True)
        for source in benchmarks:
            name = source.stem
            rotation = rep % len(arm_names)
            for arm in arm_names[rotation:] + arm_names[:rotation]:
                if (name, arm) not in cells:
                    continue
                exe, compile_s, reference = cells[(name, arm)]
                wall, user, system, code, stdout, timed_out = timer.run(exe)
                results.write("run", benchmark=name, arm=arm, rep=rep, compile_s=compile_s, run_s=wall,
                              cpu_user_s=user, cpu_sys_s=system, exit=code, timed_out=timed_out,
                              output_ok=code == 0 and plain(stdout) == reference)


# --- Summary ---------------------------------------------------------------


def table(headers, rows):
    widths = [max(len(str(row[i])) for row in [headers] + rows) for i in range(len(headers))]
    def line(row):
        return "| " + " | ".join(str(cell).rjust(width) if i else str(cell).ljust(width)
                                  for i, (cell, width) in enumerate(zip(row, widths))) + " |"
    return "\n".join([line(headers), "|" + "|".join("-" * (w + 2) for w in widths) + "|"] + [line(r) for r in rows])


def metric_value(record, metric):
    if metric == "wall":
        return record["run_s"]
    if record.get("cpu_user_s") is None or record.get("cpu_sys_s") is None:
        return None
    return record["cpu_user_s"] + record["cpu_sys_s"]


def resolve_metric(requested):
    if requested != "auto":
        return requested
    return "wall" if os.name == "nt" else "cpu"


def fmt_ratio(value):
    return f"{value:.3f}"


def summarize(records, resamples, seed, baseline, metric):
    meta = next((r for r in records if r["kind"] == "meta"), {})
    metric = resolve_metric(meta.get("metric", "auto") if metric == "auto" else metric)
    samples = {}   # arm -> benchmark -> [metric]
    compile_times = {}   # arm -> benchmark -> compile_s
    for r in records:
        if r["kind"] == "run" and r["output_ok"]:
            value = metric_value(r, metric)
            if value is not None and value > 0:
                samples.setdefault(r["arm"], {}).setdefault(r["benchmark"], []).append(value)
                compile_times.setdefault(r["arm"], {})[r["benchmark"]] = r["compile_s"]
    arms = list(meta.get("arms", {})) or sorted(samples)
    arms = [a for a in arms if a in samples or any(
        r.get("arm") == a for r in records if r["kind"] in ("verify", "build_failed"))]
    benchmarks = sorted({b for per_bench in samples.values() for b in per_bench})
    reps = meta.get("reps", "?")
    lines = ["# Optimizer comparison", ""]
    first = records[0] if records else {}
    floor = meta.get("startup_floor_s")
    what = "user + system CPU time" if metric == "cpu" else "wall time of the whole process"
    lines += [
        f"- compiler commit: `{first.get('compiler_commit', '?')}`; target `{meta.get('target', '?')}`",
        f"- host: {first.get('host', '?')}",
        f"- clang: {' '.join(meta.get('clang_version') or [])}",
        f"- {reps} interleaved repetitions per cell; metric: {metric} ({what})",
        f"- start-up floor of an empty program: {floor * 1000:.1f} ms" if floor else "- start-up floor: unknown",
        "- CIs are 95% percentile bootstrap intervals over repetitions "
        f"({resamples} resamples); the geometric mean runs over the benchmarks both arms completed.",
    ]
    if isinstance(reps, int) and reps < 3:
        lines.append(f"- warning: {reps} repetition(s) are too few for meaningful intervals; use at least 7")
    lines.append("")

    def time_table(title, arm_subset):
        arm_subset = [a for a in arm_subset if a in samples]
        rows = []
        for b in benchmarks:
            row = [b]
            for a in arm_subset:
                s = samples[a].get(b)
                row.append(f"{statistics.median(s):.3f} ({min(s):.3f})" if s else "-")
            rows.append(row)
        return [f"## {title}", "", "Median (minimum) seconds per benchmark.", "",
                table(["benchmark"] + arm_subset, rows), ""]

    def ratio_tables(title, arm_subset, base):
        if base not in samples:
            return [f"## {title}", "", f"Baseline arm {base} has no valid measurements.", ""]
        arm_subset = [a for a in arm_subset if a in samples and a != base]
        rows = []
        for b in benchmarks:
            row = [b]
            for a in arm_subset:
                if b in samples[a] and b in samples[base]:
                    row.append(fmt_ratio(statistics.median(samples[a][b]) / statistics.median(samples[base][b])))
                else:
                    row.append("-")
            rows.append(row)
        out = [f"## {title}", "",
               f"Median over median, relative to {base}; below 1 is faster than {base}.", "",
               table(["benchmark"] + arm_subset, rows), ""]
        summary_rows = []
        for a in arm_subset:
            by_median = ratio_summary(samples[a], samples[base], resamples, seed)
            if by_median is None:
                continue
            by_min = ratio_summary(samples[a], samples[base], 0, seed, statistic=min)
            point, low, high, count = by_median
            shared = set(samples[a]) & set(samples[base])
            faster = sum(statistics.median(samples[a][b]) < statistics.median(samples[base][b]) for b in shared)
            summary_rows.append([a, count, fmt_ratio(point), f"[{fmt_ratio(low)}, {fmt_ratio(high)}]",
                                 fmt_ratio(by_min[0]), f"{faster}/{count}"])
        out += [f"Geometric mean of the ratios to {base}:", "",
                table(["arm", "benchmarks", "geomean", "95% CI", "geomean of minima", f"faster than {base}"],
                      summary_rows), ""]
        return out

    standard = [a for a in STANDARD_ARMS if a in arms]
    ablation = [a for a in arms if a not in STANDARD_ARMS]
    if standard:
        lines += time_table("Run time", standard)
        if any(a != baseline for a in standard):
            lines += ratio_tables(f"Ratios to {baseline}", standard, baseline)
        if "A1" in standard and "N" in standard:
            lines += ratio_tables("Bcc optimizer ratios (A1/N)", ["A1"], "N")
    if ablation:
        lines += time_table("Ablation run time", ablation)
        lines += ratio_tables(f"Ablation ratios to {ABLATION_BASELINE}", ablation, ABLATION_BASELINE)

    compile_rows = []
    for a in arms:
        if a in compile_times:
            values = list(compile_times[a].values())
            row = [a, f"{statistics.median(values):.2f}"]
            base = compile_times.get(baseline, {})
            shared = [b for b in compile_times[a] if b in base]
            row.append(fmt_ratio(geometric_mean([compile_times[a][b] / base[b] for b in shared])) if shared else "-")
            compile_rows.append(row)
    if compile_rows:
        lines += ["## Compile time", "",
                  "Whole-process wall time of one build: for the bcc arms the front end, optimizer and the clang "
                  "invocation on the LLVM IR; for R clang on the source. Median over benchmarks, and geometric mean "
                  f"of the per-benchmark ratio to {baseline}.", "",
                  table(["arm", "median seconds", f"geomean ratio to {baseline}"], compile_rows), ""]

    problems = []
    for r in records:
        if r["kind"] == "build_failed":
            problems.append([r["benchmark"], r["arm"], "build failed", r["error"].splitlines()[0][:100] if r["error"] else ""])
        elif r["kind"] == "verify" and not r["output_ok"]:
            problems.append([r["benchmark"], r["arm"], "check failed", r.get("reason") or ""])
        elif r["kind"] == "interpreter_check" and not r["output_ok"]:
            problems.append([r["benchmark"], r["arm"], "interpreter check failed", "reduced workload differs from --interpret"])
        elif r["kind"] == "oracle" and not r["output_ok"]:
            problems.append([r["benchmark"], "-", "no interpreter result", f"exit {r.get('exit')}"])
        elif r["kind"] == "run" and not r["output_ok"]:
            problems.append([r["benchmark"], r["arm"], f"run {r['rep']} wrong output", f"exit {r.get('exit')}"])
    lines += ["## Excluded or failed", ""]
    lines += [table(["benchmark", "arm", "what", "detail"], problems) if problems else "None.", ""]
    return "\n".join(lines)


def load_records(path):
    with open(path, encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


# --- Command line ----------------------------------------------------------


def parse_arguments(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0], epilog="See the module docstring for details.")
    parser.add_argument("--corpus", type=Path, default=ROOT / "benches" / "c", help="directory of benchmark .c files")
    parser.add_argument("--match", action="append", default=[], metavar="GLOB",
                        help="only benchmarks whose name matches (repeatable)")
    parser.add_argument("--arms", default=",".join(STANDARD_ARMS), help="comma-separated arms, or 'none' (default: all)")
    parser.add_argument("--ablate", choices=["none", "leave-one-out", "prefix"], default="none",
                        help="add pass-ablation arms of A1 (default: none)")
    parser.add_argument("--ablate-rounds", type=int, default=2, metavar="N",
                        help="repeat the pass list N times in ablation arms (default 2)")
    parser.add_argument("--reps", type=int, default=DEFAULT_REPS, help=f"timed repetitions (default {DEFAULT_REPS})")
    parser.add_argument("--baseline", default="A2", help="arm the ratios are relative to (default A2)")
    parser.add_argument("--metric", choices=["auto", "wall", "cpu"], default="auto",
                        help="statistic input: cpu on POSIX, wall on Windows by default")
    parser.add_argument("--target", default=default_target(), help="--target triple (default: the host's)")
    parser.add_argument("--bcc", help="path to bcc-rust (default: target/*/release, then debug)")
    parser.add_argument("--clang", help="path to clang (default: target/llvm/bin/clang)")
    parser.add_argument("--output", type=Path, help="fresh output directory (default target/optimizer-bench/<time>)")
    parser.add_argument("--timeout", type=float, default=60.0, help="seconds per run of a program (default 60)")
    parser.add_argument("--build-timeout", type=float, default=300.0, help="seconds per build (default 300)")
    parser.add_argument("--no-interpreter-check", dest="interpreter_check", action="store_false",
                        help="skip the reduced-workload comparison with --interpret")
    parser.add_argument("--pin-cpu", type=int, metavar="N", help="pin the harness and its programs to CPU N")
    parser.add_argument("--high-priority", action="store_true", help="Windows: run programs at HIGH_PRIORITY_CLASS")
    parser.add_argument("--bootstrap", type=int, default=BOOTSTRAP_RESAMPLES, metavar="N", help="bootstrap resamples")
    parser.add_argument("--seed", type=int, default=20261011, help="bootstrap seed")
    parser.add_argument("--summarize", type=Path, metavar="RESULTS", help="summarize an existing results.jsonl and exit")
    parser.add_argument("--list", action="store_true", help="list benchmarks and arms, then exit")
    return parser.parse_args(argv)


def main(argv=None):
    crash_dialogs.suppress()
    args = parse_arguments(argv)
    if args.summarize:
        print(summarize(load_records(args.summarize), args.bootstrap, args.seed, args.baseline, args.metric))
        return 0
    benchmarks = sorted(args.corpus.glob("*.c"))
    if args.match:
        benchmarks = [b for b in benchmarks if any(fnmatch.fnmatchcase(b.stem, g) for g in args.match)]
    if not benchmarks:
        sys.exit(f"compare_optimizers: no benchmarks in {args.corpus}")
    requested = [] if args.arms == "none" else [a.strip() for a in args.arms.split(",") if a.strip()]
    unknown = [a for a in requested if a not in STANDARD_ARMS]
    if unknown:
        sys.exit(f"compare_optimizers: unknown arm(s) {', '.join(unknown)}; known: {', '.join(STANDARD_ARMS)}")
    if args.list:
        print("benchmarks:", *(b.stem for b in benchmarks))
        for arm in STANDARD_ARMS:
            print(f"{arm}: {ARM_DESCRIPTIONS[arm]}")
        return 0
    bcc, clang = find_bcc(args.bcc), find_clang(args.clang)
    warn_if_binary_is_stale(bcc)
    arms = {a: STANDARD_ARMS[a] for a in requested}
    if args.ablate != "none":
        arms.update(ablation_arms(args.ablate, default_pass_list(bcc), args.ablate_rounds))
    if "R" in arms and args.baseline == "A2" and "A2" not in arms:
        print("warning: baseline A2 is not among the arms; ratios will be empty", file=sys.stderr)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    output = args.output or ROOT / "target" / "optimizer-bench" / stamp
    if output.exists() and any(output.iterdir()):
        sys.exit(f"compare_optimizers: {output} is not empty; use a fresh directory")
    output.mkdir(parents=True, exist_ok=True)
    if args.pin_cpu is not None:
        pin_to_cpu(args.pin_cpu)
    tools = Tools(bcc, clang, args.target, args.build_timeout)
    results = Results(output / "results.jsonl", {"compiler_commit": compiler_commit(), "host": host_description()})
    try:
        run_benchmarks(args, tools, arms, benchmarks, output, results)
    finally:
        results.file.close()
    summary = summarize(results.records, args.bootstrap, args.seed, args.baseline, args.metric)
    (output / "summary.md").write_text(summary + "\n", encoding="utf-8")
    print()
    print(summary)
    print(f"\nResults: {output / 'results.jsonl'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
