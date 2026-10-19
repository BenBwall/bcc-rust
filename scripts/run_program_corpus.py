#!/usr/bin/env python3
"""Build real programs with Clang and with bcc's arms, run fixed workloads, compare.

The corpus is SQLite (shell and a deterministic SQL script), Lua (the
interpreter running the scripts under scripts/program_corpus/lua) and zlib
(`example.c` and a round-trip driver over every level and strategy). Each
program is compiled one translation unit at a time to an object file with
Clang -O0 (the reference) and with bcc's native arms A0, A1 and A2
(`conformance_common.ARMS`), linked with the bundled Clang, and run under
deadlines. Where the program is a single self-contained translation unit once
its sources are included together, the interpreter arms run it too.

    python scripts/run_program_corpus.py --output target/conformance/programs
    python scripts/run_program_corpus.py --output target/conformance/programs-zlib --programs zlib

Run after `cargo build --release`. Output: `results.json`, `summary.md` and
`logs/<program>/<arm>/` (one compile log per translation unit, one log per run).
When bcc cannot compile some translation units of a program, they are replaced
by their Clang objects (a *mixed* build, recorded as such) so that the remaining
translation units are still tested; the first diagnostic of every refusal is kept.
Every compiled program is started by this script, which disables the Windows error
dialogs, never directly.
"""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import json
from pathlib import Path
import platform
import re
import shutil
import subprocess
import time

from conformance_common import (ARMS, BCC, CLANG, NATIVE_ARMS, PLATFORMS, ROOT, ANSI,
                                bcc_command, classify, compare_run, environment, invoke)
from run_gcc_torture import sha512

CORPUS = Path(r"C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\programs")
DATA = Path(__file__).resolve().parent / "program_corpus"
LUA_SCRIPTS = ["closures", "tables", "strings", "coroutines", "numeric", "mixed"]
DEADLINES = {"compile": 1800, "link": 300, "run": 300, "interpret": 900}

ZLIB_SOURCES = ["adler32.c", "compress.c", "crc32.c", "deflate.c", "gzclose.c", "gzlib.c", "gzread.c",
                "gzwrite.c", "infback.c", "inffast.c", "inflate.c", "inftrees.c", "trees.c", "uncompr.c",
                "zutil.c"]
LUA_SOURCES = ["lapi.c", "lcode.c", "lctype.c", "ldebug.c", "ldo.c", "ldump.c", "lfunc.c", "lgc.c",
               "llex.c", "lmem.c", "lobject.c", "lopcodes.c", "lparser.c", "lstate.c", "lstring.c",
               "ltable.c", "ltm.c", "lundump.c", "lvm.c", "lzio.c", "lauxlib.c", "lbaselib.c",
               "lcorolib.c", "ldblib.c", "liolib.c", "lmathlib.c", "loadlib.c", "loslib.c", "lstrlib.c",
               "ltablib.c", "lutf8lib.c", "linit.c"]

# Each program: its root under the corpus, the library sources shared by all its
# executables, the executables (own sources and the workloads to run), and the
# defines. SQLite is built without threads and extension loading so that the
# program needs nothing from the platform beyond file and time services.
PROGRAMS = {
    "zlib": {
        "root": "zlib-1.3.2", "includes": ["."], "defines": [],
        "defines_linux": ["Z_HAVE_UNISTD_H"],
        "library": ZLIB_SOURCES,
        "executables": {
            "example": {"sources": ["test/example.c"],
                        "runs": [{"name": "example", "args": []}]},
            "roundtrip": {"sources": [str(DATA / "zlib_roundtrip.c")],
                          "runs": [{"name": "roundtrip", "args": []}]},
            "roundtrip_small": {"sources": [str(DATA / "zlib_roundtrip_small.c")],
                                "runs": [{"name": "roundtrip_small", "args": []}]},
        },
        # The gz*.c files and inflate.c both define a macro named COPY, so only the
        # in-memory API can share one translation unit; `example` needs gzopen.
        "single_translation_unit": {"library": [u for u in ZLIB_SOURCES if not u.startswith("gz")],
                                    "executables": ["roundtrip_small"]},
    },
    # lvm.c dispatches through a table of label addresses (GNU computed goto), which bcc refuses;
    # "lua-jumptable" records that, and "lua" builds with LUA_USE_JUMPTABLE=0 to test the rest.
    "lua-jumptable": {
        "root": "lua-5.4.9/src", "includes": ["."], "defines": [], "defines_linux": ["LUA_USE_LINUX"],
        "library": ["lvm.c"], "executables": {}, "single_translation_unit": None,
    },
    "lua": {
        "root": "lua-5.4.9/src", "includes": ["."], "defines": ["LUA_USE_JUMPTABLE=0"],
        "defines_linux": ["LUA_USE_LINUX"], "libs_linux": ["-ldl"],
        "library": LUA_SOURCES,
        "executables": {
            "lua": {"sources": ["lua.c"],
                    "runs": [{"name": script, "args": [f"{script}.lua"], "files": [str(DATA / "lua" / f"{script}.lua")]}
                             for script in LUA_SCRIPTS]},
        },
        "single_translation_unit": None,
    },
    # The shell and the math functions are the two parts bcc currently refuses (see the
    # report), so SQLite is built three ways: "sqlite-math" only records the refusal of the
    # math function table, "sqlite" is the amalgamation without it with the shell, and the
    # driver runs the same script through the C API so sqlite3.c is tested even when
    # shell.c is refused.
    "sqlite-math": {
        "root": "sqlite-amalgamation-3540000", "includes": ["."],
        "defines": ["SQLITE_THREADSAFE=0", "SQLITE_OMIT_LOAD_EXTENSION=1", "SQLITE_ENABLE_MATH_FUNCTIONS=1"],
        "library": ["sqlite3.c"], "executables": {}, "single_translation_unit": None,
    },
    "sqlite": {
        "root": "sqlite-amalgamation-3540000", "includes": ["."],
        "defines": ["SQLITE_THREADSAFE=0", "SQLITE_OMIT_LOAD_EXTENSION=1"],
        "library": ["sqlite3.c"],
        "executables": {
            "sqlite3": {"sources": ["shell.c"],
                        "runs": [{"name": "memory", "args": [":memory:", ".read sqlite_workload.sql"],
                                  "files": [str(DATA / "sqlite_workload.sql")]},
                                 {"name": "file", "args": ["workload.db", ".read sqlite_workload.sql"],
                                  "files": [str(DATA / "sqlite_workload.sql")]},
                                 {"name": "reopen", "args": ["workload.db",
                                  "SELECT count(*) FROM orders; PRAGMA integrity_check;"]}]},
            "sqlite_driver": {"sources": [str(DATA / "sqlite_driver.c")],
                              "runs": [{"name": "driver-memory", "args": [":memory:", "sqlite_workload.sql"],
                                        "files": [str(DATA / "sqlite_workload.sql")]},
                                       {"name": "driver-file", "args": ["driver.db", "sqlite_workload.sql"],
                                        "files": [str(DATA / "sqlite_workload.sql")]}]},
        },
        "single_translation_unit": None,
    },
}


PLATFORM = PLATFORMS["windows"]

# glibc's <assert.h> spells the function name __PRETTY_FUNCTION__ and <math.h> defines INFINITY and
# NAN with __builtin_inff()/__builtin_nanf(); bcc refuses both (see the report). A header forced
# into every translation unit of the *Linux* build, for Clang and bcc alike, defines them as macros
# first (it includes nothing, so feature-test macros such as SQLite's _GNU_SOURCE still work), which
# lets the rest of SQLite and Lua be tested. The refusals themselves are recorded in the report.
LINUX_SHIM = """#define __PRETTY_FUNCTION__ __func__
#define __builtin_inff() (3.40282347e+38F * 2.0F)
#define __builtin_nanf(tag) (0.0F / 0.0F)
#define __builtin_huge_val() (1.79769313486231571e+308 * 2.0)
"""


def tool_flags(program, root, system):
    includes = [str(root / path) for path in program["includes"]]
    defines = PLATFORM.defines() + program["defines"] + program.get("defines_linux" if PLATFORM.linux else "", [])
    return includes, defines


def shim_flags(program):
    """`-include` of the Linux shim header, when this build uses one."""
    return ["-include", program["_shim"]] if program.get("_shim") else []


def compile_unit(task):
    """Compile one translation unit to an object file with Clang or with a bcc arm."""
    arm, source, root, program, system, out_dir, log_dir = task
    includes, defines = tool_flags(program, root, system)
    source = Path(source) if Path(source).is_absolute() else root / source
    obj = out_dir / (source.stem + ".o")
    work = out_dir / ("work-" + source.stem)
    work.mkdir(parents=True, exist_ok=True)
    if arm == "clang":
        command = PLATFORM.object_command(source, obj, includes + [str(Path(source).parent)], defines,
                                          extra=shim_flags(program))
        result = invoke(command, DEADLINES["compile"], environment(work), work)
        status = {"status": "pass" if result["exit"] == 0 and obj.is_file() else "frontend-error",
                  "detail": None, "constructs": []}
        if status["status"] != "pass":
            status["detail"] = result["stderr"][:300]
    else:
        command = bcc_command(BCC_PATH, source, includes=includes + [str(Path(source).parent)],
                              system_includes=system, defines=defines, arm_flags=ARMS[arm]["flags"],
                              output=obj, compile_only=True, target=PLATFORM.target, extra=shim_flags(program))
        result = invoke(command, DEADLINES["compile"], environment(work), work)
        status = classify(result, "compile")
        if status["status"] == "pass" and not obj.is_file():
            status = {"status": "compiler-crash", "detail": "no object file produced", "constructs": []}
    log = log_dir / (source.stem + ".compile.log")
    log.parent.mkdir(parents=True, exist_ok=True)
    log.write_text((result["stdout"] + result["stderr"])[:60_000], encoding="utf-8", errors="replace")
    shutil.rmtree(work, ignore_errors=True)
    return {"source": source.name, "object": str(obj), "status": status["status"],
            "detail": status["detail"], "constructs": status["constructs"], "seconds": result["seconds"],
            "command": [part for part in command]}


BCC_PATH = BCC


def build_arm(arm, name, program, root, system, out_dir, log_dir, jobs):
    """Compile every translation unit of one program with one arm; link every executable."""
    out_dir.mkdir(parents=True, exist_ok=True)
    units = list(program["library"])
    for executable in program["executables"].values():
        units += executable["sources"]
    units = list(dict.fromkeys(units))
    tasks = [(arm, unit, root, program, system, out_dir, log_dir) for unit in units]
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        compiled = list(pool.map(compile_unit, tasks))
    by_source = {item["source"]: item for item in compiled}
    return compiled, by_source


def link(arm, name, program, executable_name, compiled, by_source, reference_objects, out_dir, log_dir):
    """Link one executable; objects bcc could not produce come from the Clang build."""
    executable = program["executables"][executable_name]
    sources = [Path(unit).name for unit in program["library"] + executable["sources"]]
    objects, substituted = [], []
    for source in dict.fromkeys(sources):
        item = by_source[source]
        if item["status"] == "pass":
            objects.append(item["object"])
        elif arm != "clang" and source in reference_objects:
            objects.append(reference_objects[source])
            substituted.append(source)
        else:
            return {"status": "not-built", "detail": f"{source}: {item['status']}", "substituted": substituted}
    exe = out_dir / (executable_name + ("" if PLATFORM.linux else ".exe"))
    work = out_dir / ("link-" + executable_name)
    work.mkdir(exist_ok=True)
    libraries = ["-lm", *(program.get("libs_linux", []) if PLATFORM.linux else [])]
    command = PLATFORM.link_command(objects, exe, libraries)
    result = invoke(command, DEADLINES["link"], environment(work), work)
    (log_dir / f"{executable_name}.link.log").write_text(result["stdout"] + result["stderr"], encoding="utf-8")
    shutil.rmtree(work, ignore_errors=True)
    if result["exit"] != 0 or not exe.is_file():
        return {"status": "link-error", "detail": ANSI.sub("", result["stderr"])[:300], "substituted": substituted}
    return {"status": "linked", "exe": str(exe), "substituted": substituted}


def run_workloads(arm, executable_name, executable, exe, out_dir, log_dir, reference_runs):
    """Run each workload of an executable; compare with the reference arm's runs."""
    runs = {}
    work = out_dir / ("run-" + executable_name)
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir()
    for run in executable["runs"]:
        for file in run.get("files", []):
            shutil.copy(file, work)
        result = invoke(PLATFORM.run_command(exe, DEADLINES["run"], run["args"]), DEADLINES["run"] + 5,
                        environment(work), work)
        record = {"exit": result["exit"], "seconds": result["seconds"], "timeout": result["timeout"],
                  "stdout_bytes": len(result["stdout"]), "stdout_head": result["stdout"][:300],
                  "stderr_head": result["stderr"][:300]}
        (log_dir / f"{run['name']}.run.log").write_text(
            result["stdout"][:200_000] + ("\n--- stderr ---\n" + result["stderr"] if result["stderr"] else ""),
            encoding="utf-8", errors="replace")
        if arm == "clang":
            record["status"] = "reference" if result["exit"] == 0 and not result["timeout"] else "reference-failure"
            record["_stdout"] = result["stdout"]
        else:
            reference = reference_runs.get(run["name"])
            if reference is None or reference["status"] != "reference":
                record["status"], record["detail"] = "no-reference", None
            else:
                record["status"], record["detail"] = compare_run(
                    result, {"exit": reference["exit"], "stdout": reference["_stdout"]})
        runs[run["name"]] = record
    shutil.rmtree(work, ignore_errors=True)
    return runs


def amalgam(program, executable, root, directory):
    """One translation unit holding the library sources and the executable's sources.

    A program's own quoted headers are inlined at their first inclusion only:
    some (zlib's inftrees.h and inflate.h) have no include guard and would
    redefine their types when several source files are joined.
    """
    path = directory / "amalgam.c"
    seen = set()
    include = re.compile(r'^\s*#\s*include\s+"([^"]+)"')

    def expand(file):
        out = []
        for line in file.read_text(encoding="utf-8", errors="replace").splitlines():
            match = include.match(line)
            candidate = (file.parent / match[1]) if match else None
            if candidate is not None and candidate.is_file() and (root.resolve() in candidate.resolve().parents or DATA.resolve() in candidate.resolve().parents):
                if candidate.resolve() not in seen:
                    seen.add(candidate.resolve())
                    out.extend(expand(candidate))
            else:
                out.append(line)
        return out

    lines = []
    for unit in program["single_translation_unit"]["library"]:
        lines += expand(root / unit)
    for source in executable["sources"]:
        lines += expand(Path(source) if Path(source).is_absolute() else root / source)
    path.write_text(chr(10).join(lines) + chr(10), encoding="utf-8")
    return path


def run_interpreter(arm, name, program, root, system, out_dir, log_dir, reference_runs):
    """Interpret each executable of a single-translation-unit program."""
    results = {}
    includes, defines = tool_flags(program, root, system)
    for executable_name in program["single_translation_unit"]["executables"]:
        executable = program["executables"][executable_name]
        directory = out_dir / executable_name
        directory.mkdir(parents=True, exist_ok=True)
        source = amalgam(program, executable, root, directory)
        runs = {}
        for run in executable["runs"]:
            work = directory / ("run-" + run["name"])
            work.mkdir(exist_ok=True)
            for file in run.get("files", []):
                shutil.copy(file, work)
            command = bcc_command(BCC_PATH, source, includes=includes + [str(root / "test"), str(root)],
                                  system_includes=system, defines=defines, arm_flags=ARMS[arm]["flags"],
                                  target=PLATFORM.target)
            # Arguments after `--` are not supported by the interpreter CLI: the hosted entry
            # gives `main` an empty argument vector, so workloads that need arguments are skipped.
            result = invoke(command, DEADLINES["interpret"], environment(work), work)
            classified = classify(result, "interpret")
            record = {"seconds": result["seconds"], "exit": result["exit"],
                      "stdout_head": result["stdout"][:300], "stderr_head": ANSI.sub("", result["stderr"])[:600]}
            (log_dir / f"{executable_name}.{run['name']}.interpret.log").write_text(
                result["stdout"][:200_000] + "\n--- stderr ---\n" + result["stderr"][:20_000], encoding="utf-8",
                errors="replace")
            if run["args"]:
                record.update(status="not-attempted", detail="the interpreter gives main no arguments")
            elif classified["status"] not in ("pass", "wrong-result"):
                record.update(status=classified["status"], detail=classified["detail"],
                              constructs=classified["constructs"])
            else:
                reference = reference_runs.get(run["name"])
                if reference is None or reference["status"] != "reference":
                    record.update(status="no-reference")
                else:
                    record["status"], record["detail"] = compare_run(
                        result, {"exit": reference["exit"], "stdout": reference["_stdout"]})
            runs[run["name"]] = record
            shutil.rmtree(work, ignore_errors=True)
        results[executable_name] = runs
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--programs", default=",".join(PROGRAMS))
    parser.add_argument("--arms", nargs="+", default=NATIVE_ARMS + ["interpret", "interpret-bcc"],
                        choices=list(ARMS))
    parser.add_argument("--corpus", type=Path, default=CORPUS)
    parser.add_argument("--jobs", type=int, default=12)
    parser.add_argument("--bcc", type=Path, default=BCC)
    parser.add_argument("--platform", choices=list(PLATFORMS), default="windows",
                        help="windows (MinGW-w64, run natively) or linux-wsl (glibc headers, objects linked and run in WSL)")
    parser.add_argument("--no-mixed", action="store_true", help="do not substitute Clang objects for refused units")
    args = parser.parse_args()
    args.output = args.output.resolve()
    if not args.output.is_relative_to((ROOT / "target").resolve()):
        parser.error("--output must be beneath this worktree's target/")
    global BCC_PATH
    BCC_PATH = args.bcc.resolve(strict=True)
    global PLATFORM
    PLATFORM = PLATFORMS[args.platform]
    args.output.mkdir(parents=True, exist_ok=True)
    if PLATFORM.linux:
        shim = args.output / "linux_shim.h"
        shim.write_text(LINUX_SHIM, encoding="utf-8")
        for program in PROGRAMS.values():
            program["_shim"] = str(shim)
    system = PLATFORM.system_includes()
    args.output.mkdir(parents=True, exist_ok=True)
    started = time.perf_counter()
    manifest = {
        "utc": datetime.now(timezone.utc).isoformat(), "bcc": str(BCC_PATH), "bcc_sha512": sha512(BCC_PATH),
        "clang": subprocess.run([str(CLANG), "--version"], capture_output=True, text=True).stdout.splitlines()[0],
        "target": PLATFORM.target, "platform": args.platform, "arms": args.arms, "deadlines": DEADLINES, "corpus": str(args.corpus),
        "host": platform.platform(), "python": platform.python_version(),
        "git_head": subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip(),
        "programs": {name: {"defines": PROGRAMS[name]["defines"], "library": PROGRAMS[name]["library"]}
                     for name in args.programs.split(",")},
    }
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    results = []
    for name in args.programs.split(","):
        program = PROGRAMS[name]
        root = args.corpus / program["root"]
        print(f"== {name}", flush=True)
        reference_objects, reference_runs = {}, {}
        for arm in ["clang"] + [a for a in args.arms if ARMS[a]["native"]]:
            out_dir = args.output / "build" / name / arm
            log_dir = args.output / "logs" / name / arm
            log_dir.mkdir(parents=True, exist_ok=True)
            began = time.perf_counter()
            compiled, by_source = build_arm(arm, name, program, root, system, out_dir, log_dir, args.jobs)
            record = {"program": name, "arm": arm,
                      "units": [{key: value for key, value in item.items() if key not in ("object", "command")}
                                for item in compiled],
                      "executables": {}}
            if arm == "clang":
                reference_objects = {item["source"]: item["object"] for item in compiled if item["status"] == "pass"}
            for executable_name, executable in program["executables"].items():
                linked = link(arm, name, program, executable_name, compiled, by_source,
                              {} if args.no_mixed else reference_objects, out_dir, log_dir)
                entry = {"link": {key: value for key, value in linked.items() if key != "exe"}, "runs": {}}
                if linked["status"] == "linked":
                    entry["runs"] = run_workloads(arm, executable_name, executable, linked["exe"], out_dir, log_dir,
                                                  reference_runs)
                    if arm == "clang":
                        reference_runs.update(entry["runs"])
                record["executables"][executable_name] = entry
            record["seconds"] = round(time.perf_counter() - began, 1)
            print(f"  {arm}: " + ", ".join(f"{e}: {x['link']['status']} "
                  f"{dict(Counter(r['status'] for r in x['runs'].values()))}"
                  for e, x in record["executables"].items()), flush=True)
            results.append(record)
        if program["single_translation_unit"]:
            for arm in [a for a in args.arms if not ARMS[a]["native"]]:
                log_dir = args.output / "logs" / name / arm
                log_dir.mkdir(parents=True, exist_ok=True)
                interpreted = run_interpreter(arm, name, program, root, system, args.output / "build" / name / arm,
                                              log_dir, reference_runs)
                results.append({"program": name, "arm": arm, "executables": {
                    e: {"runs": runs} for e, runs in interpreted.items()}})
                print(f"  {arm}: " + ", ".join(f"{e}: {dict(Counter(r['status'] for r in runs.values()))}"
                                               for e, runs in interpreted.items()), flush=True)
    for record in results:
        for entry in record["executables"].values():
            for run in entry["runs"].values():
                run.pop("_stdout", None)
    manifest["elapsed_seconds"] = round(time.perf_counter() - started, 1)
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    (args.output / "results.json").write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")
    text = markdown(results, manifest)
    (args.output / "summary.md").write_text(text, encoding="utf-8")
    print(text)
    return 0


def markdown(results, manifest):
    lines = ["# Program corpus summary", "",
             f"Target `{manifest['target']}`, {manifest['clang']}; bcc `{manifest['bcc']}`; "
             f"{manifest.get('elapsed_seconds', '?')} s.", "",
             "| Program | Arm | Translation units | Build | Runs |", "| --- | --- | --- | --- | --- |"]
    for record in results:
        units = record.get("units")
        unit_text = ""
        if units:
            counts = Counter(unit["status"] for unit in units)
            unit_text = ", ".join(f"{count} {status}" for status, count in counts.items())
        for executable, entry in record["executables"].items():
            link_status = entry.get("link", {}).get("status", "interpreted")
            substituted = entry.get("link", {}).get("substituted")
            if substituted:
                link_status += f" (mixed: Clang objects for {', '.join(substituted)})"
            runs = Counter(run["status"] for run in entry["runs"].values())
            lines.append(f"| {record['program']}:{executable} | {record['arm']} | {unit_text} | {link_status} | "
                         f"{dict(runs) or 'none'} |")
    lines += ["", "## Details", ""]
    for record in results:
        for unit in record.get("units", []):
            if unit["status"] != "pass":
                lines.append(f"- {record['program']} {record['arm']} `{unit['source']}`: {unit['status']}: "
                             f"{(unit['detail'] or '')[:200]}")
        for executable, entry in record["executables"].items():
            for name, run in entry["runs"].items():
                if run["status"] not in ("pass", "reference"):
                    lines.append(f"- {record['program']}:{executable} {record['arm']} run `{name}`: "
                                 f"{run['status']}: {(run.get('detail') or run.get('stderr_head') or '')[:200]}")
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    raise SystemExit(main())
