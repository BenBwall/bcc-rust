#!/usr/bin/env python3
"""Download a pinned GCC corpus and exercise bcc-rust without executing C code.

Run from the repository root after `cargo build --bin bcc-rust`.
Downloaded sources, preprocessing output, and per-file evidence stay in target/.
This is a corpus survey, not GCC's DejaGnu runner or a C conformance score.
"""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from fnmatch import fnmatchcase
import hashlib
import json
import os
import platform
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import time
import urllib.request


VERSION = "15.2.0"
ARCHIVE_URL = f"https://ftp.gnu.org/gnu/gcc/gcc-{VERSION}/gcc-{VERSION}.tar.xz"
CHECKSUM_URL = f"https://gcc.gnu.org/pub/gcc/releases/gcc-{VERSION}/sha512.sum"
ARCHIVE_SHA512 = (
    "89047a2e07bd9da265b507b516ed3635adb17491c7f4f67cf090f0bd5b3fc7f2ee6e4cc4"
    "008beef7ca884b6b71dffe2bb652b21f01a702e17b468cca2d10b2de"
)
ROOT = Path(__file__).resolve().parents[1]
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
INCLUDE = re.compile(r"^\s*#\s*include\b", re.MULTILINE)
GCC_FLAGS = ["-std=c99", "-pedantic-errors", "-fmax-errors=5", "-fdiagnostics-color=never"]


def sha512(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha512").hexdigest()


def prepare_corpus(cache):
    archive = cache / f"gcc-{VERSION}.tar.xz"
    cache.mkdir(parents=True, exist_ok=True)
    if not archive.exists():
        temporary = archive.with_suffix(".download")
        print(f"Downloading {ARCHIVE_URL}", flush=True)
        with urllib.request.urlopen(ARCHIVE_URL, timeout=60) as response:
            with temporary.open("wb") as output:
                shutil.copyfileobj(response, output)
        temporary.replace(archive)
    if sha512(archive) != ARCHIVE_SHA512:
        raise RuntimeError(f"Checksum mismatch for {archive}; remove it and retry")
    corpus = cache / f"gcc-{VERSION}" / "gcc" / "testsuite" / "gcc.c-torture"
    marker = cache / f"gcc-{VERSION}" / ".corpus-extracted"
    if not marker.exists():
        prefix = f"gcc-{VERSION}/gcc/testsuite/gcc.c-torture/"
        licenses = {f"gcc-{VERSION}/COPYING", f"gcc-{VERSION}/COPYING3"}
        print("Extracting torture sources, harness metadata, and license notices", flush=True)
        # Copy only regular files beneath the named prefix; never follow archive links.
        with tarfile.open(archive, "r|xz") as source:
            for member in source:
                if not member.isfile():
                    continue
                if not (member.name.startswith(prefix) or member.name in licenses):
                    continue
                relative = PurePosixPath(member.name)
                if relative.is_absolute() or ".." in relative.parts:
                    raise RuntimeError(f"Unsafe archive member: {member.name}")
                destination = cache.joinpath(*relative.parts)
                destination.parent.mkdir(parents=True, exist_ok=True)
                with source.extractfile(member) as content, destination.open("wb") as output:
                    shutil.copyfileobj(content, output)
        marker.write_text(ARCHIVE_SHA512 + "\n", encoding="utf-8")
    return corpus


def invoke(command, timeout, environment):
    start = time.perf_counter()
    try:
        result = subprocess.run(
            [str(item) for item in command], cwd=ROOT, env=environment,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout,
        )
        return {
            "exit": result.returncode,
            "seconds": round(time.perf_counter() - start, 4),
            "stdout": result.stdout.decode("utf-8", errors="replace"),
            "stderr": ANSI.sub("", result.stderr.decode("utf-8", errors="replace")),
            "timeout": False,
        }
    except subprocess.TimeoutExpired as error:
        return {
            "exit": None, "seconds": round(time.perf_counter() - start, 4),
            "stdout": (error.stdout or b"").decode("utf-8", errors="replace"),
            "stderr": ANSI.sub("", (error.stderr or b"").decode("utf-8", errors="replace")),
            "timeout": True,
        }


def evidence(result, log):
    content = result["stdout"] + result["stderr"]
    if content:
        log.parent.mkdir(parents=True, exist_ok=True)
        log.write_text(content, encoding="utf-8")
    return {key: value for key, value in result.items() if key not in {"stdout", "stderr"}}


# Rendered diagnostics open with a lowercase `error:` or `warning:` heading.
# Capitalized headings are accepted for logs from earlier builds.
ERROR_HEADING = re.compile(r"^[Ee]rror: (.*)$", re.MULTILINE)
WARNING_HEADING = re.compile(r"^[Ww]arning: (.*)$", re.MULTILINE)


def bcc_status(result):
    output = result["stdout"] + result["stderr"]
    errors = ERROR_HEADING.findall(output)
    warnings = WARNING_HEADING.findall(output)
    if result["timeout"]:
        status = "timeout"
    elif re.search(r"panicked at|stack overflow|fatal runtime error", output):
        status = "crash"
    elif result["exit"] != 0:
        status = "process_failure"
    elif errors:
        status = "diagnostics"
    else:
        status = "accepted"
    # Source-vector details vary; retain full diagnostics separately in logs.
    messages = [message.split(" at ", 1)[0] for message in errors]
    return {"status": status, "errors": len(errors), "warnings": len(warnings),
            "first_error": messages[0] if messages else None}


def survey_file(path, corpus, output, args, environment):
    relative = path.relative_to(corpus).as_posix()
    source = path.read_text(encoding="utf-8", errors="replace")
    row = {"file": relative, "source_sha512": sha512(path),
           "has_include": bool(INCLUDE.search(source))}
    bcc = invoke([args.bcc, path], args.timeout, environment)
    row["raw"] = {**bcc_status(bcc), **evidence(bcc, output / "logs/raw" / (relative + ".log"))}
    gcc = invoke([args.gcc, *GCC_FLAGS, "-fsyntax-only", path], args.timeout, environment)
    row["gcc_c99"] = evidence(gcc, output / "logs/gcc" / (relative + ".log"))
    # Header-free inputs avoid GNU/Windows implementation syntax from host headers.
    # Preserve source content: -E -P expands real GCC macros, removing line markers.
    if gcc["exit"] == 0 and not row["has_include"]:
        preprocessed = output / "preprocessed" / (relative + ".i")
        preprocessed.parent.mkdir(parents=True, exist_ok=True)
        cpp = invoke([args.gcc, *GCC_FLAGS, "-E", "-P", path], args.timeout, environment)
        row["preprocessing"] = evidence(
            {**cpp, "stdout": ""}, output / "logs/preprocessing" / (relative + ".log")
        )
        if cpp["exit"] == 0:
            preprocessed.write_text(cpp["stdout"], encoding="utf-8")
            parsed = invoke([args.bcc, preprocessed], args.timeout, environment)
            row["preprocessed"] = {
                **bcc_status(parsed),
                **evidence(parsed, output / "logs/preprocessed" / (relative + ".log")),
            }
    return row


def summary(rows):
    eligible = [row for row in rows if row["gcc_c99"]["exit"] == 0]
    isolated = [row for row in rows if "preprocessed" in row]
    return {
        "files": len(rows),
        "suites": dict(Counter(row["file"].split("/")[0] for row in rows)),
        "raw": dict(Counter(row["raw"]["status"] for row in rows)),
        "raw_with_warnings": sum(bool(row["raw"]["warnings"]) for row in rows),
        "gcc_c99_accepted": len(eligible),
        "gcc_c99_rejected": sum(row["gcc_c99"]["exit"] not in {0, None} for row in rows),
        "gcc_c99_timeouts": sum(row["gcc_c99"]["timeout"] for row in rows),
        "gcc_c99_accepted_raw": dict(Counter(row["raw"]["status"] for row in eligible)),
        "header_free_gcc_c99_preprocessed_files": len(isolated),
        "preprocessed": dict(Counter(row["preprocessed"]["status"] for row in isolated)),
        "preprocessed_with_warnings": sum(bool(row["preprocessed"]["warnings"]) for row in isolated),
        "preprocessed_diagnostic_files": [row["file"] for row in isolated
                                          if row["preprocessed"]["status"] != "accepted"],
        "raw_first_errors": Counter(row["raw"]["first_error"] for row in rows
                                    if row["raw"]["first_error"]).most_common(20),
        "preprocessed_first_errors": Counter(row["preprocessed"]["first_error"] for row in isolated
                                             if row["preprocessed"]["first_error"]).most_common(20),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bcc", type=Path, help="Parser binary (defaults to Cargo's host-target debug build)")
    parser.add_argument("--gcc", default="gcc")
    parser.add_argument("--cache", type=Path, default=ROOT / "target/compiler-corpus")
    parser.add_argument("--output", type=Path, default=ROOT / "target/compiler-corpus/results")
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--timeout", type=float, default=10)
    parser.add_argument("--limit", type=int, help="Survey the first N sorted files, for a smoke run")
    parser.add_argument("--match", action="append", default=[],
                        help="Select a corpus-relative glob (repeatable), e.g. 'compile/limits-*'")
    args = parser.parse_args()
    if args.bcc is None:
        rust_info = subprocess.check_output([os.environ.get("RUSTC", "rustc"), "-vV"], text=True)
        host = next(line.removeprefix("host: ") for line in rust_info.splitlines()
                    if line.startswith("host: "))
        args.bcc = ROOT / "target" / host / "debug" / ("bcc-rust.exe" if os.name == "nt" else "bcc-rust")
    if args.jobs < 1 or args.timeout <= 0 or (args.limit is not None and args.limit < 1):
        parser.error("jobs, timeout, and limit must be positive")
    args.bcc = args.bcc.resolve(strict=True)
    args.cache = args.cache.resolve()
    args.output = args.output.resolve()
    if (args.output / "results.json").exists():
        parser.error("output already contains results; choose a new --output directory")
    environment = os.environ.copy()
    for variable in ("CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "OBJC_INCLUDE_PATH"):
        environment.pop(variable, None)
    corpus = prepare_corpus(args.cache)
    files = sorted(path for suite in ("compile", "execute") for path in (corpus / suite).rglob("*.c"))
    if args.match:
        files = [path for path in files if any(fnmatchcase(path.relative_to(corpus).as_posix(), pattern)
                                              for pattern in args.match)]
    if args.limit is not None:
        files = files[:args.limit]
    if not files:
        parser.error("no corpus files matched")
    args.output.mkdir(parents=True, exist_ok=True)
    metadata = {
        "utc": datetime.now(timezone.utc).isoformat(), "archive": ARCHIVE_URL,
        "checksum_source": CHECKSUM_URL, "archive_sha512": ARCHIVE_SHA512,
        "bcc_binary_sha512": sha512(args.bcc), "bcc_binary": str(args.bcc),
        "gcc_flags": GCC_FLAGS, "jobs": args.jobs, "timeout_seconds": args.timeout,
        "limit": args.limit, "match": args.match, "corpus": str(corpus),
        "host": platform.platform(), "python_version": platform.python_version(),
        "gcc_version": invoke([args.gcc, "--version"], args.timeout, environment)["stdout"].splitlines()[0],
        "gcc_target": invoke([args.gcc, "-dumpmachine"], args.timeout, environment)["stdout"].strip(),
        "git_head": invoke(["git", "rev-parse", "HEAD"], args.timeout, environment)["stdout"].strip(),
        "git_status": invoke(["git", "status", "--short"], args.timeout, environment)["stdout"],
    }
    rows = []
    start = time.perf_counter()
    print(f"Surveying {len(files)} files with {args.jobs} workers; {args.output}", flush=True)
    with (args.output / "results.jsonl").open("w", encoding="utf-8") as progress:
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = [pool.submit(survey_file, path, corpus, args.output, args, environment) for path in files]
            for future in as_completed(futures):
                row = future.result()
                rows.append(row)
                progress.write(json.dumps(row) + "\n")
                progress.flush()
                if len(rows) % 250 == 0 or len(rows) == len(files):
                    states = Counter(item["raw"]["status"] for item in rows)
                    print(f"{len(rows)}/{len(files)} in {time.perf_counter() - start:.1f}s: {dict(states)}", flush=True)
    rows.sort(key=lambda row: row["file"])
    metadata["elapsed_seconds"] = round(time.perf_counter() - start, 3)
    report = {"metadata": metadata, "summary": summary(rows), "files": rows}
    (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    console_summary = {key: value for key, value in report["summary"].items()
                       if key != "preprocessed_diagnostic_files"}
    print(json.dumps(console_summary, indent=2), flush=True)
    # Expected dialect/semantic rejections do not fail this exploratory survey.
    # Crashes, process failures, and timeouts must be made visible to automation.
    return int(any(row[mode]["status"] in {"crash", "process_failure", "timeout"}
                   for row in rows for mode in ("raw", "preprocessed") if mode in row)
               or any(row["gcc_c99"]["exit"] is None for row in rows)
               or any("preprocessing" in row and row["preprocessing"]["exit"] != 0 for row in rows))


if __name__ == "__main__":
    raise SystemExit(main())
