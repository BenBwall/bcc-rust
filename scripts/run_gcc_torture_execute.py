#!/usr/bin/env python3
"""Survey existing GCC execute sources through bcc's interpreter and native arms.

No downloads or source rewrites. This is an exploratory single-file survey,
not DejaGnu: supported unconditional dg-options/dg-additional-options are
applied; target selectors and additional-source directives remain recorded.
Run after `cargo build --release`.
"""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from fnmatch import fnmatchcase
import json
import os
from pathlib import Path
import platform
import re
import signal
import shlex
import subprocess
import time

import crash_dialogs
from run_gcc_torture import ANSI, ERROR_HEADING, ROOT, VERSION, evidence, sha512


TARGET = "x86_64-w64-windows-gnu"
ARMS = {
    "interpret": ["--interpret"],
    "interpret-bcc": ["--interpret", "--opt=bcc"],
    "A0": ["--llvm-opt=O0"],
    "A1": ["--opt=bcc", "--llvm-opt=O2", "--llvm-codegen-only"],
}
CLASSES = ("pass", "frontend-error", "lowering-unsupported", "abi-unsupported",
           "compiler-crash", "interpreter-trap", "wrong-result", "timeout", "link-error")
LOWERING = re.compile(r"not yet supported by lowering: (.*)")
TRAP = re.compile(r"the interpreter stopped the program: (.*)")
PANIC = re.compile(r"panicked at|stack overflow|fatal runtime error")
DG_OPTIONS = re.compile(r'\{\s*dg-(options|additional-options)\s+"((?:\\.|[^"\\])*)"')
STANDARDS = set("c89 c90 iso9899:1990 iso9899:199409 c99 c9x iso9899:1999 "
                "iso9899:199x c11 c1x iso9899:2011 c17 c18 iso9899:2017 "
                "iso9899:2018 c23 c2x iso9899:2024 c2y gnu89 gnu90 gnu99 "
                "gnu9x gnu11 gnu1x gnu17 gnu18 gnu23 gnu2x gnu2y".split())
LANGUAGE_FLAGS = {"-fgnu89-inline", "-fno-gnu89-inline", "-w", "--suppress-warnings",
                  "-pedantic", "-Wpedantic", "-pedantic-errors", "-ffreestanding", "-fhosted",
                  "-nostdinc", "-nostdlibinc", "-nobuiltininc",
                  "--nostdinc", "--nostdlibinc", "--nobuiltininc",
                  "--no-repeated-specifier-warnings"}
MSVC_GROUPS = "extensions declspec int-types calling-conventions type-qualifiers inline seh asm pragma anonymous-structs va-args".split()
LANGUAGE_FLAGS.update(f"-{prefix}{group}" for prefix in ("fms-", "fno-ms-") for group in MSVC_GROUPS)
VALUE_FLAGS = {"-D", "-U", "-I", "-include", "-iquote", "-isystem", "-idirafter", "--sysroot",
               "--iquote", "--isystem", "--idirafter", "--include-directory", "--source-date-epoch"}


def source_options(source):
    """Apply bcc front-end flags, retaining unsupported tokens and selectors.

    dg-options replaces the base list; dg-additional-options appends to it.
    DejaGnu target/effective-target expressions are deliberately not evaluated.
    Back-end/output options cannot override the four survey arms.
    """
    base, additional, directives = [], [], []
    for match in DG_OPTIONS.finditer(source):
        # Find the closing outer brace without confusing quoted strings or
        # nested target selector braces for the end of the directive.
        depth, quoted, escaped, end = 1, False, False, match.end()
        while end < len(source) and depth:
            char = source[end]
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                quoted = not quoted
            elif not quoted:
                depth += (char == "{") - (char == "}")
            end += 1
        selector = source[match.end():end - 1].strip()
        record = {"directive": match[1], "text": match[2], "selector": selector,
                  "applied_options": [], "unsupported_options": []}
        directives.append(record)
        if depth or selector:
            record["deferred_reason"] = "unterminated directive" if depth else "target selector not evaluated"
            continue
        try:
            tokens = shlex.split(match[2])
        except ValueError:
            record["deferred_reason"] = "malformed option quoting"
            continue
        applied, unsupported = supported_options(tokens)
        record.update(applied_options=applied, unsupported_options=unsupported)
        if match[1] == "options":
            base = applied
        else:
            additional.extend(applied)
    return {"applied_options": base + additional,
            "unsupported_options": [flag for item in directives for flag in item["unsupported_options"]],
            "option_directives": directives}


def supported_options(tokens):
    """Whitelist spellings/values accepted by cli/{arguments,search}.rs."""
    applied, unsupported = [], []
    index = 0
    while index < len(tokens):
        flag = tokens[index]
        index += 1
        if flag in LANGUAGE_FLAGS:
            applied.append(flag)
        elif flag in {"-std", "--std"} or flag in VALUE_FLAGS:
            if index == len(tokens):
                unsupported.append(flag)
                continue
            value = tokens[index]
            index += 1
            if flag in {"-std", "--std"} and value not in STANDARDS:
                unsupported.extend([flag, value])
            else:
                applied.extend([flag, value])
        elif flag.startswith(("-std=", "--std=")):
            (applied if flag.split("=", 1)[1] in STANDARDS else unsupported).append(flag)
        elif flag.startswith(("-D", "-U", "-I")) and len(flag) > 2:
            applied.append(flag)
        elif any(flag.startswith(prefix + "=") for prefix in VALUE_FLAGS if prefix.startswith("--")):
            applied.append(flag)
        elif any(flag.startswith(prefix) and len(flag) > len(prefix)
                 for prefix in ("-iquote", "-isystem", "-idirafter")):
            applied.append(flag)
        else:
            unsupported.append(flag)
    return applied, unsupported


def main():
    crash_dialogs.suppress()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True,
                        help="Existing GCC release root, gcc.c-torture root, or execute directory")
    parser.add_argument("--bcc", type=Path, default=ROOT / "target/x86_64-pc-windows-gnu/release/bcc-rust.exe")
    parser.add_argument("--output", type=Path, default=ROOT / "target/compiler-corpus/execute")
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--timeout", type=float, default=10)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--match", action="append", default=[], help="Repeatable execute-relative glob")
    parser.add_argument("--recursive", action="store_true", help="Also include ieee/ and builtins/")
    args = parser.parse_args()
    if args.jobs < 1 or args.timeout <= 0 or (args.limit is not None and args.limit < 1):
        parser.error("jobs, timeout and limit must be positive")
    args.bcc = args.bcc.resolve(strict=True)
    args.source = args.source.resolve(strict=True)
    args.output = args.output.resolve()
    if not args.output.is_relative_to((ROOT / "target").resolve()):
        parser.error("all output must be beneath this worktree's target/")
    if args.output.exists():
        parser.error("choose a fresh --output directory")
    execute = source_directory(args.source)
    files = sorted(execute.rglob("*.c") if args.recursive else execute.glob("*.c"))
    files = [path for path in files if not args.match or any(
        fnmatchcase(path.relative_to(execute).as_posix(), pattern) for pattern in args.match)]
    if args.limit is not None:
        files = files[:args.limit]
    if not files:
        parser.error("no execute sources matched")
    args.output.mkdir(parents=True)
    environment = os.environ.copy()
    for variable in ("CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "OBJC_INCLUDE_PATH"):
        environment.pop(variable, None)
    environment["NO_COLOR"] = "1"
    if os.name == "nt":
        # Inherited by children: faults must terminate rather than open WER dialogs.
        import ctypes
        ctypes.windll.kernel32.SetErrorMode(0x0001 | 0x0002)
    metadata = {
        "utc": datetime.now(timezone.utc).isoformat(), "gcc_release": VERSION,
        "source": str(execute), "recursive": args.recursive,
        "bcc": str(args.bcc), "bcc_sha512": sha512(args.bcc), "target": TARGET,
        "arms": ARMS, "jobs": args.jobs, "timeout_seconds": args.timeout,
        "match": args.match, "limit": args.limit, "host": platform.platform(),
        "python_version": platform.python_version(),
        "git_head": invoke(["git", "rev-parse", "HEAD"], args.timeout, environment, ROOT)["stdout"].strip(),
        "git_status": invoke(["git", "status", "--short"], args.timeout, environment, ROOT)["stdout"],
    }
    (args.output / "manifest.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    rows = []
    started = time.perf_counter()
    print(f"Surveying {len(files)} execute files with {args.jobs} workers: {args.output}", flush=True)
    with (args.output / "results.jsonl").open("w", encoding="utf-8") as progress:
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = [pool.submit(survey_file, path, execute, args, environment) for path in files]
            for future in as_completed(futures):
                row = future.result()
                rows.append(row)
                progress.write(json.dumps(row) + "\n")
                progress.flush()
                if len(rows) % 100 == 0 or len(rows) == len(files):
                    counts = Counter(run["status"] for row in rows for run in row["arms"].values())
                    print(f"{len(rows)}/{len(files)} in {time.perf_counter() - started:.1f}s: {dict(counts)}", flush=True)
    rows.sort(key=lambda row: row["file"])
    metadata["elapsed_seconds"] = round(time.perf_counter() - started, 3)
    totals = summary(rows)
    metadata["source_options"] = {row["file"]: {key: row[key] for key in
                                  ("applied_options", "unsupported_options", "option_directives")}
                                  for row in rows}
    (args.output / "manifest.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    (args.output / "results.json").write_text(
        json.dumps({"metadata": metadata, "summary": totals, "files": rows}, indent=2) + "\n", encoding="utf-8")
    (args.output / "summary.md").write_text(markdown_summary(totals, metadata), encoding="utf-8")
    print(json.dumps({"counts": totals["counts"], "disagreements": len(totals["disagreements"]),
                      "crashes": len(totals["crashes"])}, indent=2), flush=True)
    return int(bool(totals["crashes"] or totals["disagreements"] or any(
        run["status"] in {"wrong-result", "timeout"} for row in rows for run in row["arms"].values())))


def classify(result, phase):
    """phase is interpret, compile, or execute; program exits are never crashes."""
    stderr = ANSI.sub("", result["stderr"])
    errors = ERROR_HEADING.findall(stderr)
    detail = errors[0] if errors else None
    status = "pass"
    constructs = []
    trap_kind = None
    if result["timeout"]:
        status = "timeout"
    elif phase == "execute":
        if result["exit"] != 0:
            status = "wrong-result"
    elif PANIC.search(stderr):
        status = "compiler-crash"
    elif match := TRAP.search(stderr):
        status = "interpreter-trap"
        detail = re.sub(r" \(in @.*", "", match[1])
        trap_kind = detail
    elif "not yet supported by lowering:" in stderr:
        status = "lowering-unsupported"
        constructs = sorted(set(LOWERING.findall(stderr)))
    elif "construct not supported by code generation: its type was not analyzed" in stderr:
        status = "lowering-unsupported"
        constructs = ["type not analyzed by semantic analysis"]
    elif re.search(r"not (?:yet )?supported by ABI lowering|unsupported by ABI lowering", stderr):
        status = "abi-unsupported"
    elif "internal error:" in stderr or "invalid IR" in stderr:
        status = "compiler-crash"
    elif "cannot build `" in stderr:
        # Invalid emitted LLVM IR is a compiler defect, not a link limitation.
        status = "compiler-crash" if re.search(
            r"\.ll:\d+:\d+: error:|LLVM ERROR:|PLEASE submit a bug report", stderr) else "link-error"
    elif errors:
        status = "frontend-error"
    elif result["exit"] != 0:
        status = "wrong-result" if phase == "interpret" and 0 < result["exit"] <= 255 else "compiler-crash"
    return {"status": status, "detail": detail, "constructs": constructs, "trap_kind": trap_kind}


def survey_file(path, execute, args, environment):
    relative = path.relative_to(execute).as_posix()
    source = path.read_text(encoding="utf-8", errors="replace")
    row = {"file": relative, "source_sha512": sha512(path),
           "dg_directives": re.findall(r"\{ dg-[^\n]*", source), **source_options(source), "arms": {}}
    for arm, flags in ARMS.items():
        work = args.output / "work" / relative / arm
        work.mkdir(parents=True)
        env = {**environment, "TMP": str(work), "TEMP": str(work), "TMPDIR": str(work)}
        exe = work / "test.exe"
        command = [str(args.bcc), str(path), f"--target={TARGET}", *row["applied_options"], *flags]
        phase = "interpret" if arm.startswith("interpret") else "compile"
        if phase == "compile":
            command += ["-o", str(exe)]
        result = invoke(command, args.timeout, env, work)
        run = {**classify(result, phase), "command": command,
               "compile": evidence(result, args.output / "logs" / relative / (arm + ".compile.log"))}
        if phase == "compile" and run["status"] == "pass":
            if not exe.is_file():
                run.update(status="compiler-crash", detail="compiler returned success without an executable")
            else:
                result = invoke([str(exe)], args.timeout, env, work)
                run.update(classify(result, "execute"))
                run["execute"] = evidence(result, args.output / "logs" / relative / (arm + ".execute.log"))
        row["arms"][arm] = run
    row["disagreement"] = disagreement(row["arms"])
    return row


def disagreement(arms):
    """Keep all status differences, including capability gaps, for review."""
    return len({run["status"] for run in arms.values()}) > 1


def summary(rows):
    return {
        "files": len(rows),
        "counts": {arm: {status: sum(row["arms"][arm]["status"] == status for row in rows)
                         for status in CLASSES} for arm in ARMS},
        # Count sources, rather than multiplying a shared refusal by four arms.
        "unsupported": Counter(construct for row in rows for construct in {
            item for run in row["arms"].values() for item in run["constructs"]}).most_common(20),
        "crashes": [{"file": row["file"], "arm": arm, "detail": run["detail"]}
                    for row in rows for arm, run in row["arms"].items() if run["status"] == "compiler-crash"],
        "disagreements": [{"file": row["file"], "arms": {arm: run["status"] for arm, run in row["arms"].items()},
                           "pass_vs_wrong": any(run["status"] == "pass" for run in row["arms"].values()) and
                           any(run["status"] == "wrong-result" for run in row["arms"].values())}
                          for row in rows if row["disagreement"]],
    }


def markdown_summary(totals, metadata):
    lines = [f"GCC {VERSION} execute survey", "", f"{totals['files']} sources; target `{TARGET}`; "
             f"{metadata['elapsed_seconds']} seconds. See manifest.json for compiler identity and flags.", "",
             "| Class | " + " | ".join(ARMS) + " |", "| --- | " + " | ".join("---:" for _ in ARMS) + " |"]
    for status in CLASSES:
        lines.append("| " + status + " | " + " | ".join(str(totals["counts"][arm][status]) for arm in ARMS) + " |")
    lines += ["", "Top unsupported constructs (source counts):", ""]
    lines += [f"- {construct}: {count}" for construct, count in totals["unsupported"]]
    lines += ["", "Compiler crashes:", ""]
    lines += [f"- `{item['file']}` {item['arm']}: {item['detail']}" for item in totals["crashes"]] or ["None."]
    lines += ["", "Arm disagreements (capability gaps included; pass/wrong-result marked):", ""]
    lines += [f"- `{item['file']}`: {item['arms']}" + (" **pass/wrong-result**" if item["pass_vs_wrong"] else "")
              for item in totals["disagreements"]] or ["None."]
    return "\n".join(lines) + "\n"


def source_directory(source):
    for candidate in (source / "gcc/testsuite/gcc.c-torture/execute", source / "execute", source):
        if candidate.is_dir() and next(candidate.glob("*.c"), None) is not None:
            return candidate
    raise ValueError(f"no execute directory at {source}")


def invoke(command, timeout, environment, cwd):
    """Capture evidence and kill descendants too when a compiler deadline expires."""
    started = time.perf_counter()
    process = subprocess.Popen(command, cwd=cwd, env=environment, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=os.name != "nt")
    close_job = windows_job(process) if os.name == "nt" else None
    timed_out = False
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        if os.name == "nt":
            close_job()
            close_job = None
        else:
            os.killpg(process.pid, signal.SIGKILL)
        process.kill()
        stdout, stderr = process.communicate()
    finally:
        if close_job is not None:
            close_job()
    return {"exit": None if timed_out else process.returncode, "timeout": timed_out,
            "seconds": round(time.perf_counter() - started, 4),
            "stdout": stdout.decode("utf-8", errors="replace"),
            "stderr": ANSI.sub("", stderr.decode("utf-8", errors="replace"))}


def windows_job(process):
    """A kill-on-close job also stops clang/linker descendants holding pipes.

    https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects
    """
    import ctypes as c
    from ctypes import wintypes as w

    class BasicLimits(c.Structure):
        _fields_ = [("process_time", c.c_int64), ("job_time", c.c_int64),
                    ("flags", w.DWORD), ("min_working_set", c.c_size_t),
                    ("max_working_set", c.c_size_t), ("processes", w.DWORD),
                    ("affinity", c.c_size_t), ("priority", w.DWORD), ("scheduling", w.DWORD)]

    class ExtendedLimits(c.Structure):
        _fields_ = [("basic", BasicLimits), ("io_counters", c.c_uint64 * 6),
                    ("process_memory", c.c_size_t), ("job_memory", c.c_size_t),
                    ("peak_process_memory", c.c_size_t), ("peak_job_memory", c.c_size_t)]

    kernel = c.WinDLL("kernel32", use_last_error=True)
    kernel.CreateJobObjectW.argtypes = [c.c_void_p, w.LPCWSTR]
    kernel.CreateJobObjectW.restype = w.HANDLE
    kernel.SetInformationJobObject.argtypes = [w.HANDLE, c.c_int, c.c_void_p, w.DWORD]
    kernel.AssignProcessToJobObject.argtypes = [w.HANDLE, w.HANDLE]
    kernel.CloseHandle.argtypes = [w.HANDLE]
    if process.poll() is not None:
        return lambda: None
    job = kernel.CreateJobObjectW(None, None)
    limits = ExtendedLimits()
    limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    if not job or not kernel.SetInformationJobObject(job, 9, c.byref(limits), c.sizeof(limits)) or not (
            kernel.AssignProcessToJobObject(job, int(process._handle))):
        error = c.get_last_error()
        if job:
            kernel.CloseHandle(job)
        # Tiny native tests can finish between Popen and job assignment.
        # Their process handle is still valid, but Windows denies assignment
        # of an already terminated process. Preserve its completed evidence.
        try:
            process.wait(timeout=0.1)
            return lambda: None
        except subprocess.TimeoutExpired:
            pass
        process.kill()
        process.communicate()
        raise OSError(error, f"cannot assign a Windows process job: {process.args}")
    return lambda: kernel.CloseHandle(job)


if __name__ == "__main__":
    raise SystemExit(main())
