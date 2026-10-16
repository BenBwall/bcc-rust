#!/usr/bin/env python3
"""Compare bcc-rust with Clang on randomly generated C programs.

Csmith and YARPGen write programs that avoid undefined behavior, so every
conforming compiler must produce the same output for them. For each seed this
script generates a program, builds it with the bundled Clang at -O0 (the
reference; a second build at -O2 must agree, or the program is discarded as
unreliable) and with bcc's five arms (the two interpreter arms and the native
arms A0, A1 and A2; see `conformance_common.ARMS`), runs them under deadlines,
and compares the output (Csmith prints a checksum, YARPGen a hash).

    python scripts/random_program_conformance.py build-generators
    python scripts/random_program_conformance.py run --output target/conformance/first \\
        --generators csmith,yarpgen --count 1000 --time-budget 7200 --jobs 20
    python scripts/random_program_conformance.py summarize target/conformance/first
    python scripts/random_program_conformance.py rerun target/conformance/first csmith 4242

Run after `cargo build --release`. Everything is written beneath this
worktree's target/. A run directory holds `manifest.json` (tool versions,
options, deadlines, bcc hash), `results.jsonl` (one row per program, appended as
the run proceeds, which makes the run resumable: start it again with the same
command and finished seeds are skipped), `summary.json`/`summary.md`, and
`failures/<generator>/<seed>/` with the source and the per-arm logs of every
program where an arm did something other than pass or refuse a construct.
Every compiled program is started by this script, which disables the Windows
error dialogs, never directly.
"""

import argparse
from collections import Counter, defaultdict
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time
import traceback

from conformance_common import (ARMS, BCC, CLANG, OUTCOMES, PLATFORMS, REFUSALS, ROOT,
                                bcc_command, classify, environment, invoke, normalize_message,
                                refusal_summary, remove_file, remove_tree, run_arm)
from run_gcc_torture import sha512

# The pinned generator sources are read-only inputs; BCC_GENERATOR_SOURCES overrides the location.
GENERATOR_SOURCES = Path(os.environ.get(
    "BCC_GENERATOR_SOURCES", r"C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\generators"))
BUILD = ROOT / "target/generators/build"
BIN = ROOT / "target/generators/bin"
CSMITH = BUILD / "csmith/src/csmith.exe"
YARPGEN = BUILD / "yarpgen/yarpgen.exe"

# Csmith options. Csmith's defaults produce programs bcc handles; each option here
# is recorded in the manifest with its reason. The first campaign started with
# the defaults, and the pilot showed which constructs bcc refuses; only options
# justified by a refusal of a construct that bcc rejects by design belong here.
CSMITH_OPTIONS = []
CSMITH_OPTION_REASONS = {}

# YARPGen: `--std=c` selects the C dialect. Array dimensions are capped because
# the default tests declare globals larger than 2 GiB, which neither the
# interpreter (it caps one global) nor a COFF image (relocations are 32-bit) can
# hold; Clang itself fails to link them. Pragmas are off because OpenMP and
# vectorizer pragmas are not part of the language under test.
YARPGEN_OPTIONS = ["--std=c", "--max-array-dims=2", "--emit-pragmas=none"]
YARPGEN_OPTION_REASONS = {
    "--std=c": "C dialect instead of C++",
    "--max-array-dims=2": "default tests declare globals over 2 GiB that no arm can build",
    "--emit-pragmas=none": "omp/clang loop pragmas are not under test",
}

DEFAULT_DEADLINES = {"generate": 60, "reference_compile": 120, "reference_run": 30,
                     "compile": 120, "execute": 30, "interpret": 120}
LOG_LIMIT = 40_000
DEFAULT_INTERPRET_STEP_LIMIT = 10_000_000_000


# ----------------------------------------------------------------------------
# Generators


def generate(generator, seed, work, options):
    """Write the program for `seed` into `work`; return (sources, include dirs, error)."""
    work.mkdir(parents=True, exist_ok=True)
    if generator == "csmith":
        source = work / "test.c"
        result = invoke([str(CSMITH), "--seed", str(seed), *options, "-o", str(source)], 60,
                        environment(work), work)
        if result["exit"] != 0 or not source.is_file():
            return None, [], f"csmith failed: {result['stderr'][:300]}"
        runtime = GENERATOR_SOURCES / "csmith/runtime"
        if "--float" in options:
            # With floats Csmith passes `&float_object` to a `char *` parameter, a constraint
            # violation that Clang and GCC accept with a warning. A header beside the program
            # (found before the runtime's) casts that one argument; nothing else changes.
            (work / "csmith.h").write_text(
                f'#include "{(runtime / "csmith.h").as_posix()}"\n'
                "#define transparent_crc_bytes(ptr, nbytes, vname, vflag) "
                "transparent_crc_bytes((char *)(ptr), (nbytes), (vname), (vflag))\n", encoding="utf-8")
        return [source], [str(runtime), str(BUILD / "csmith/runtime")], None
    if generator == "yarpgen":
        result = invoke([str(YARPGEN), "-s", str(seed), *options, "-o", str(work)], 60,
                        environment(work), work)
        if result["exit"] != 0 or not (work / "func.c").is_file():
            return None, [], f"yarpgen failed: {result['stderr'][:300]}"
        # One translation unit: the interpreter takes a single file and the
        # generator's two files only communicate through init.h.
        source = work / "all.c"
        source.write_text('#include "driver.c"\n#include "func.c"\n', encoding="utf-8")
        return [source], [str(work)], None
    raise ValueError(generator)


# ----------------------------------------------------------------------------
# One program


def test_program(config, generator, seed):
    """Generate one program and compare every arm with the reference."""
    started = time.perf_counter()
    work = config.output / "work" / generator / str(seed)
    remove_tree(work)
    options = config.options[generator]
    row = {"generator": generator, "seed": seed, "status": "tested", "arms": {}}
    sources, generator_includes, error = generate(generator, seed, work, options)
    if error:
        row.update(status="generator-failure", detail=error)
        remove_tree(work)
        return row
    source = sources[0]
    row["source_bytes"] = sum(path.stat().st_size for path in work.glob("*.[ch]"))
    includes = generator_includes
    platform = PLATFORMS[getattr(config, "platform", "windows")]
    system = config.system_includes
    defines = config.defines
    deadlines = config.deadlines

    # All five arms share one front end, so a program the front end refuses is
    # refused by every arm: probe it once instead of building it five times.
    probe_dir = work / "probe"
    probe_dir.mkdir()
    probe = invoke(bcc_command(config.bcc, source, target=platform.target, includes=includes,
                               system_includes=system, defines=defines, extra=["--semantic-types"]),
                   deadlines["interpret"], environment(probe_dir), probe_dir)
    probed = classify(probe, "compile")
    if probed["status"] == "frontend-error":
        for arm in config.arms:
            row["arms"][arm] = {"status": "frontend-error", "detail": probed["detail"], "constructs": [],
                                "shared_front_end": True}
        row["defect_candidate"] = False
        row["seconds"] = round(time.perf_counter() - started, 2)
        remove_tree(work)
        return row

    # Reference: Clang -O0, cross-checked by Clang -O2 against undefined behavior.
    reference = None
    for opt in ("-O0", "-O2"):
        sub = work / ("ref" + opt)
        sub.mkdir()
        exe = sub / ("ref" + (".exe" if not platform.linux else ""))
        built = invoke(platform.reference_command(sources, exe, opt, includes + system, defines),
                       deadlines["reference_compile"], environment(sub), sub)
        ran = None
        if built["exit"] == 0 and exe.is_file():
            ran = invoke(platform.run_command(exe, deadlines["reference_run"]), deadlines["reference_run"] + 5,
                         environment(sub), sub)
        remove_file(exe)
        if ran is None or ran["timeout"] or ran["exit"] != 0 or not ran["stdout"].strip():
            row.update(status="reference-failure",
                       detail=f"clang {opt}: " + (built["stderr"][:300] if ran is None else
                                                  f"exit {ran['exit']} timeout {ran['timeout']}"))
            _keep(work, config, row)
            return row
        if reference is None:
            reference = ran
        elif ran["stdout"] != reference["stdout"]:
            row.update(status="reference-unstable",
                       detail="clang -O0 and -O2 print different output; the program is not reliable")
            _keep(work, config, row)
            return row
    row["reference"] = {"stdout_head": reference["stdout"][:200], "seconds": reference["seconds"]}

    command_kwargs = {"includes": includes, "system_includes": system, "defines": defines}
    evidence = {}
    for arm in config.arms:
        sub = work / arm
        sub.mkdir()
        run = run_arm(config.bcc, arm, source, sub, deadlines=deadlines, reference=reference,
                      command_kwargs={**command_kwargs, "extra": [
                          *command_kwargs.get("extra", []),
                          *([f"--interpret-step-limit={config.interpret_step_limit}"]
                            if not ARMS[arm]["native"] else [])]}, platform=platform)
        evidence[arm] = (run.pop("_compile"), run.pop("_execute", None))
        run["command"] = [part.replace(str(work), "<work>") for part in run["command"]]
        row["arms"][arm] = run
    row["seconds"] = round(time.perf_counter() - started, 2)
    candidate = any(run["status"] not in ("pass", *REFUSALS) for run in row["arms"].values())
    row["defect_candidate"] = candidate
    if any(run["status"] in ("wrong-output", "abnormal-exit") for run in row["arms"].values()):
        row["gcc"] = gcc_opinion(config, platform, sources, includes + system, defines, reference, work)
    if candidate:
        for arm, (compiled, executed) in evidence.items():
            _write_log(work / arm / "compile.log", compiled)
            if executed:
                _write_log(work / arm / "execute.log", executed)
        _write_log(work / "reference.log", {"stdout": reference["stdout"], "stderr": ""})
        _keep(work, config, row)
    else:
        remove_tree(work)
    return row


def gcc_opinion(config, platform, sources, includes, defines, reference, work):
    """What GCC -O0 prints for a program on which an arm disagreed with Clang.

    When GCC and Clang disagree the program leans on behavior the standard leaves open (or one
    of them has a bug), so the arm's difference is not evidence by itself.
    """
    sub = work / "gcc"
    sub.mkdir(exist_ok=True)
    exe = sub / ("gcc" if platform.linux else "gcc.exe")
    built = invoke(platform.gcc_command(sources, exe, includes, defines), config.deadlines["reference_compile"],
                   environment(sub), sub)
    if built["exit"] != 0 or not exe.is_file():
        return {"status": "build-failed", "detail": built["stderr"][:200]}
    ran = invoke(platform.run_command(exe, config.deadlines["reference_run"]), config.deadlines["reference_run"] + 5,
                 environment(sub), sub)
    remove_file(exe)
    return {"status": "agrees-with-clang" if ran["stdout"] == reference["stdout"] and ran["exit"] == 0
            else "differs-from-clang", "stdout_head": ran["stdout"][:120], "exit": ran["exit"]}


def _write_log(path, result):
    text = (result["stdout"] + ("\n--- stderr ---\n" + result["stderr"] if result["stderr"] else ""))
    path.write_text(text[:LOG_LIMIT], encoding="utf-8", errors="replace")


def _keep(work, config, row):
    """Move a failing program's source and logs to failures/, dropping binaries."""
    for pattern in ("**/*.exe", "**/*.o", "**/*.obj", "**/program", "**/ref"):
        for path in work.glob(pattern):
            if path.is_file():
                remove_file(path)
    target = config.output / "failures" / row["generator"] / str(row["seed"])
    target.parent.mkdir(parents=True, exist_ok=True)
    remove_tree(target)
    shutil.copytree(work, target, dirs_exist_ok=True, ignore_dangling_symlinks=True)
    remove_tree(work)
    row["evidence"] = target.relative_to(config.output).as_posix()


def guarded(config, generator, seed):
    """test_program, with a harness failure recorded as a row instead of ending the campaign."""
    try:
        return test_program(config, generator, seed)
    except Exception:  # noqa: BLE001 - the campaign must outlive one bad program
        remove_tree(config.output / "work" / generator / str(seed))
        return {"generator": generator, "seed": seed, "status": "harness-error", "arms": {},
                "detail": traceback.format_exc()[-1500:]}


# ----------------------------------------------------------------------------
# Campaign


def configuration(args):
    options = {"csmith": CSMITH_OPTIONS + args.csmith_option, "yarpgen": YARPGEN_OPTIONS + args.yarpgen_option}
    return {
        "generators": args.generators, "options": options,
        "option_reasons": {"csmith": CSMITH_OPTION_REASONS, "yarpgen": YARPGEN_OPTION_REASONS},
        "arms": args.arms, "target": PLATFORMS[args.platform].target,
        "defines": PLATFORMS[args.platform].defines() + args.define,
        "deadlines": args.deadlines, "seed_start": args.seed_start,
        "interpret_step_limit": args.interpret_step_limit,
        **({"platform": args.platform} if args.platform != "windows" else {}),
    }


def tool_versions():
    def first_line(command):
        try:
            result = subprocess.run(command, capture_output=True, text=True, timeout=60)
            return (result.stdout or result.stderr).strip().splitlines()[0]
        except (OSError, subprocess.SubprocessError, IndexError):
            return "unavailable"
    return {
        "csmith": " / ".join(subprocess.run([str(CSMITH), "--version"], capture_output=True,
                                            text=True).stdout.splitlines()),
        "yarpgen": first_line([str(YARPGEN), "--version"]),
        "clang": first_line([str(CLANG), "--version"]),
        "gcc": first_line(["gcc", "--version"]),
        "python": platform.python_version(), "host": platform.platform(),
    }


def command_run(args):
    args.output = args.output.resolve()
    if not args.output.is_relative_to((ROOT / "target").resolve()):
        sys.exit("all output must be beneath this worktree's target/")
    args.bcc = args.bcc.resolve(strict=True)
    args.deadlines = {**DEFAULT_DEADLINES, **{key: value for key, value in (
        ("compile", args.compile_timeout), ("execute", args.execute_timeout),
        ("interpret", args.interpret_timeout)) if value is not None}}
    args.arms = args.arms or list(ARMS)
    args.generators = args.generators.split(",")
    config = configuration(args)
    fingerprint = {**config, "bcc_sha512": sha512(args.bcc)}
    args.output.mkdir(parents=True, exist_ok=True)
    manifest_path = args.output / "manifest.json"
    done = set()
    results_path = args.output / "results.jsonl"
    if manifest_path.exists():
        previous = json.loads(manifest_path.read_text(encoding="utf-8"))
        if previous["config"] != fingerprint and not args.allow_config_change:
            sys.exit("the existing run used a different configuration or bcc build; "
                     "choose a new --output or pass --allow-config-change")
        for line in results_path.read_text(encoding="utf-8").splitlines() if results_path.exists() else []:
            row = json.loads(line)
            if row["status"] != "harness-error":
                done.add((row["generator"], row["seed"]))
        manifest = previous
        manifest["resumed"] = manifest.get("resumed", 0) + 1
    else:
        manifest = {
            "utc": datetime.now(timezone.utc).isoformat(), "config": fingerprint,
            "bcc": str(args.bcc), "tools": tool_versions(),
            "git_head": subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True,
                                       text=True).stdout.strip(),
            "git_status": subprocess.run(["git", "status", "--short"], cwd=ROOT, capture_output=True,
                                         text=True).stdout,
            "jobs": args.jobs, "time_budget_seconds": args.time_budget,
        }
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    args.system_includes = PLATFORMS[args.platform].system_includes()
    args.options = config["options"]
    args.defines = config["defines"]
    # Interleave the generators so a time budget splits evenly between them.
    tasks = [(generator, seed) for seed in range(args.seed_start, args.seed_start + args.count)
             for generator in args.generators if (generator, seed) not in done]
    print(f"{len(tasks)} programs to test ({len(done)} already done) with {args.jobs} workers", flush=True)
    deadline = time.monotonic() + args.time_budget if args.time_budget else None
    started = time.monotonic()
    finished = 0
    counts = Counter()
    pending = set()
    queue = iter(tasks)
    exhausted = False
    with results_path.open("a", encoding="utf-8") as results, ThreadPoolExecutor(max_workers=args.jobs) as pool:
        try:
            while True:
                while not exhausted and len(pending) < args.jobs * 2 and (deadline is None or time.monotonic() < deadline):
                    task = next(queue, None)
                    if task is None:
                        exhausted = True
                        break
                    pending.add(pool.submit(guarded, args, *task))
                if deadline is not None and time.monotonic() >= deadline:
                    exhausted = True
                if not pending:
                    break
                complete, pending = wait(pending, return_when=FIRST_COMPLETED)
                for future in complete:
                    row = future.result()
                    results.write(json.dumps(row) + "\n")
                    results.flush()
                    finished += 1
                    counts[row["status"]] += 1
                    for run in row["arms"].values():
                        counts["arm:" + run["status"]] += 1
                    if finished % 25 == 0:
                        print(f"{finished}/{len(tasks)} in {time.monotonic() - started:.0f}s: "
                              f"{dict(sorted(counts.items()))}", flush=True)
        except KeyboardInterrupt:
            print("interrupted; finishing the programs in flight", flush=True)
            for future in pending:
                future.cancel()
    manifest["elapsed_seconds"] = round(manifest.get("elapsed_seconds", 0) + time.monotonic() - started, 1)
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    command_summarize(argparse.Namespace(directory=args.output))
    return 0


# ----------------------------------------------------------------------------
# Summary


def load_rows(directory):
    path = directory / "results.jsonl"
    rows = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            row = json.loads(line)
            rows[(row["generator"], row["seed"])] = row
    return list(rows.values())


def signature(arm, run, row):
    """A key under which equal defects collide: status, normalized message, trap kind."""
    status = run["status"]
    if status in ("wrong-output", "abnormal-exit"):
        failing = sorted(name for name, other in row["arms"].items() if other["status"] == status)
        divergent = row.get("gcc", {}).get("status") == "differs-from-clang"
        return f"{status} in {'+'.join(failing)}" + (" (GCC differs from Clang)" if divergent else "")
    return f"{status}: {normalize_message(run.get('detail') or '')}"


def summarize(rows, manifest):
    generators = sorted({row["generator"] for row in rows})
    per_generator = {}
    for generator in generators:
        chosen = [row for row in rows if row["generator"] == generator]
        tested = [row for row in chosen if row["status"] == "tested"]
        per_generator[generator] = {
            "programs": len(chosen), "tested": len(tested),
            "discarded": dict(Counter(row["status"] for row in chosen if row["status"] != "tested")),
            "arms": {arm: dict(Counter(row["arms"][arm]["status"] for row in tested if arm in row["arms"]))
                     for arm in ARMS},
            "refusals": refusal_summary(tested)[:25],
        }
    groups = defaultdict(list)
    for row in rows:
        for arm, run in row["arms"].items():
            if run["status"] not in ("pass", *REFUSALS):
                groups[signature(arm, run, row)].append((row["generator"], row["seed"], arm))
    defects = [{"signature": key, "programs": len({(g, s) for g, s, _ in items}),
                "arms": sorted({arm for _, _, arm in items}),
                "examples": sorted({(g, s) for g, s, _ in items})[:8]}
               for key, items in groups.items()]
    defects.sort(key=lambda item: (-item["programs"], item["signature"]))
    return {"generators": per_generator, "defect_groups": defects}


def markdown(summary, manifest):
    lines = ["# Random program conformance summary", ""]
    lines.append(f"Target `{manifest['config']['target']}`; bcc `{manifest['bcc']}`; "
                 f"elapsed {manifest.get('elapsed_seconds', '?')} s.")
    for generator, data in summary["generators"].items():
        lines += ["", f"## {generator}", "",
                  f"{data['programs']} programs, {data['tested']} compared; "
                  f"discarded: {data['discarded'] or 'none'}.", "",
                  "| Outcome | " + " | ".join(ARMS) + " |", "| --- | " + " | ".join("---:" for _ in ARMS) + " |"]
        for outcome in OUTCOMES:
            lines.append(f"| {outcome} | " + " | ".join(str(data["arms"][arm].get(outcome, 0)) for arm in ARMS) + " |")
        lines += ["", "Refused constructs (programs):", ""]
        lines += [f"- {name}: {count}" for name, count in data["refusals"]] or ["None."]
    lines += ["", "## Defect signatures (programs affected)", ""]
    for item in summary["defect_groups"]:
        examples = ", ".join(f"{g}/{s}" for g, s in item["examples"])
        lines.append(f"- {item['signature']}: {item['programs']} programs, arms {', '.join(item['arms'])}; e.g. {examples}")
    return "\n".join(lines) + "\n"


def command_summarize(args):
    directory = args.directory.resolve()
    manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
    summary = summarize(load_rows(directory), manifest)
    (directory / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    (directory / "summary.md").write_text(markdown(summary, manifest), encoding="utf-8")
    print(markdown(summary, manifest), flush=True)
    return 0


def command_rerun(args):
    """Regenerate one program and rerun every arm in a scratch directory."""
    directory = args.directory.resolve()
    manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
    config = manifest["config"]
    scratch = ROOT / "target/conformance-rerun" / f"{args.generator}-{args.seed}"
    shutil.rmtree(scratch, ignore_errors=True)
    namespace = argparse.Namespace(
        output=scratch, bcc=Path(manifest["bcc"]), arms=config["arms"], deadlines=config["deadlines"],
        options=config["options"], defines=config["defines"],
        interpret_step_limit=config.get("interpret_step_limit", 100_000_000),
        platform=config.get("platform", "windows"))
    namespace.system_includes = PLATFORMS[namespace.platform].system_includes()
    row = test_program(namespace, args.generator, args.seed)
    print(json.dumps(row, indent=2))
    return 0


# ----------------------------------------------------------------------------
# Generator builds


def command_build(args):
    """Build Csmith and YARPGen under target/generators with MinGW g++ and CMake.

    Csmith needs the m4 macro processor to generate its safe-math headers; none
    exists on a stock Windows host, so CMake is pointed at a wrapper around
    `scripts/mini_m4.py`. Csmith calls the one-argument MinGW `mkdir` with two
    arguments, which a force-included header papers over (the sources under
    target/compiler-corpus stay untouched). Both generators link statically:
    the MSYS and Strawberry MinGW runtime DLLs on PATH conflict at start-up.
    """
    BIN.mkdir(parents=True, exist_ok=True)
    root = ROOT.as_posix()
    (BIN / "m4.cmd").write_text(f'@python "{(ROOT / "scripts/mini_m4.py")}"\n', encoding="utf-8")
    (BIN / "csmith_win_compat.h").write_text(
        "/* Force-included when building Csmith on MinGW, whose mkdir takes one argument. */\n"
        "#include <sys/stat.h>\n#include <io.h>\n#include <direct.h>\n"
        "#define mkdir(path, mode) mkdir(path)\n", encoding="utf-8")
    common = ["-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_CXX_COMPILER=g++",
              "-DCMAKE_C_COMPILER=gcc", "-DCMAKE_EXE_LINKER_FLAGS=-static"]
    steps = [
        ("csmith", [*common, f"-DCMAKE_CXX_FLAGS=-include {root}/target/generators/bin/csmith_win_compat.h -w",
                    f"-DM4={root}/target/generators/bin/m4.cmd", str(GENERATOR_SOURCES / "csmith")]),
        ("yarpgen", [*common, str(GENERATOR_SOURCES / "yarpgen")]),
    ]
    for name, configure in steps:
        build = BUILD / name
        build.mkdir(parents=True, exist_ok=True)
        subprocess.run(["cmake", *configure], cwd=build, check=True)
        subprocess.run(["ninja"], cwd=build, check=True)
    print(json.dumps(tool_versions(), indent=2))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("build-generators", help="build Csmith and YARPGen beneath target/generators").set_defaults(
        function=command_build)
    run = sub.add_parser("run", help="generate programs and compare the arms (resumable)")
    run.add_argument("--output", type=Path, required=True, help="run directory beneath target/")
    run.add_argument("--generators", default="csmith,yarpgen")
    run.add_argument("--count", type=int, default=500, help="programs per generator")
    run.add_argument("--seed-start", type=int, default=1)
    run.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 4) - 4))
    run.add_argument("--time-budget", type=float, help="stop starting programs after this many seconds")
    run.add_argument("--bcc", type=Path, default=BCC)
    run.add_argument("--arms", nargs="+", choices=list(ARMS))
    run.add_argument("--platform", choices=list(PLATFORMS), default="windows",
                     help="windows: MinGW-w64 target, programs run natively; "
                          "linux-wsl: x86_64-unknown-linux-gnu, objects linked and programs run by WSL's Clang")
    run.add_argument("--define", action="append", default=[], help="extra -D for every compiler")
    run.add_argument("--csmith-option", action="append", default=[], help="extra Csmith option (repeatable)")
    run.add_argument("--yarpgen-option", action="append", default=[], help="extra YARPGen option (repeatable)")
    run.add_argument("--compile-timeout", type=float)
    run.add_argument("--execute-timeout", type=float)
    run.add_argument("--interpret-timeout", type=float)
    run.add_argument("--interpret-step-limit", type=int, default=DEFAULT_INTERPRET_STEP_LIMIT,
                     help="interpreter instruction limit (default 10000000000; 0 is unlimited)")
    run.add_argument("--allow-config-change", action="store_true")
    run.set_defaults(function=command_run)
    summarize_parser = sub.add_parser("summarize", help="recompute summary.md from results.jsonl")
    summarize_parser.add_argument("directory", type=Path)
    summarize_parser.set_defaults(function=command_summarize)
    rerun = sub.add_parser("rerun", help="regenerate one program and run every arm again")
    rerun.add_argument("directory", type=Path)
    rerun.add_argument("generator", choices=["csmith", "yarpgen"])
    rerun.add_argument("seed", type=int)
    rerun.set_defaults(function=command_rerun)
    args = parser.parse_args()
    if args.command == "run" and (args.jobs < 1 or args.count < 1):
        parser.error("jobs and count must be positive")
    if args.command == "run" and args.interpret_step_limit < 0:
        parser.error("interpret-step-limit must be nonnegative")
    return args.function(args)


if __name__ == "__main__":
    raise SystemExit(main())
