#!/usr/bin/env python3
"""Compile each public libc header with Clang and bcc-rust, and compare them.

Run from the repository root after `cargo build --bin bcc-rust` and
`python scripts/fetch_libc_sysroots.py`. Every header becomes a one-line
translation unit. Clang runs first with the configuration's target flags; the
headers it rejects are recorded and excluded, and bcc compiles the rest
through a configurable argument template. Results, translation units, and logs
stay in target/. This measures header coverage, not conformance.
"""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from fnmatch import fnmatchcase
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import string
import subprocess
import time

from run_gcc_torture import ANSI, bcc_status


ROOT = Path(__file__).resolve().parents[1]
C17_HEADERS = [
    "assert.h", "complex.h", "ctype.h", "errno.h", "fenv.h", "float.h", "inttypes.h",
    "iso646.h", "limits.h", "locale.h", "math.h", "setjmp.h", "signal.h", "stdalign.h",
    "stdarg.h", "stdatomic.h", "stdbool.h", "stddef.h", "stdint.h", "stdio.h", "stdlib.h",
    "stdnoreturn.h", "string.h", "tgmath.h", "threads.h", "time.h", "uchar.h", "wchar.h",
    "wctype.h",
]
# C23 headers count only when the libc ships them.
C23_HEADERS = ["stdbit.h", "stdckdint.h"]
POSIX_HEADERS = [
    "aio.h", "alloca.h", "arpa/inet.h", "byteswap.h", "cpio.h", "dirent.h", "dlfcn.h",
    "endian.h", "err.h", "fcntl.h", "features.h", "fmtmsg.h", "fnmatch.h", "ftw.h",
    "getopt.h", "glob.h", "grp.h", "iconv.h", "ifaddrs.h", "langinfo.h", "libgen.h",
    "malloc.h", "monetary.h", "mqueue.h", "net/if.h", "netdb.h", "netinet/in.h",
    "netinet/ip.h", "netinet/tcp.h", "netinet/udp.h", "nl_types.h", "poll.h", "pthread.h",
    "pwd.h", "regex.h", "sched.h", "search.h", "semaphore.h", "spawn.h", "strings.h",
    "sys/epoll.h", "sys/eventfd.h", "sys/file.h", "sys/inotify.h", "sys/ioctl.h",
    "sys/ipc.h", "sys/mman.h", "sys/msg.h", "sys/param.h", "sys/prctl.h", "sys/random.h",
    "sys/resource.h", "sys/select.h", "sys/sem.h", "sys/sendfile.h", "sys/shm.h",
    "sys/socket.h", "sys/stat.h", "sys/statvfs.h", "sys/syscall.h", "sys/sysmacros.h",
    "sys/time.h", "sys/timerfd.h", "sys/times.h", "sys/types.h", "sys/uio.h", "sys/un.h",
    "sys/utsname.h", "sys/wait.h", "syslog.h", "tar.h", "termios.h", "unistd.h", "utime.h",
    "utmpx.h", "wordexp.h",
]
WINDOWS_HEADERS = [
    "windows.h", "winsock2.h", "ws2tcpip.h", "winbase.h", "winnt.h", "winuser.h",
    "wincrypt.h", "winreg.h", "shellapi.h", "shlobj.h", "objbase.h", "commctrl.h",
    "tlhelp32.h", "psapi.h", "intrin.h", "conio.h", "direct.h", "fcntl.h", "io.h",
    "malloc.h", "process.h", "share.h", "sys/stat.h", "sys/types.h", "sys/timeb.h",
    "sys/utime.h",
]
MINGW_POSIX_HEADERS = ["dirent.h", "getopt.h", "pthread.h", "strings.h", "sys/time.h", "unistd.h"]
COMBINED = "<c-standard>"
SENTINEL = "typedef int libc_header_survey_unit;\n"
DEFAULT_BCC_ARGS = '--std={std} "--isystem {dir}"'
PLACEHOLDERS = {"triple", "sysroot", "std", "config", "dir", "input"}
CLANG_DIAGNOSTIC = re.compile(r"^(.*?):(\d+):\d+: (fatal error|error|warning): (.*)$", re.MULTILINE)
BCC_FIRST_ERROR_LOCATION = re.compile(r"^[Ee]rror: .*\n\s*--> (.+):(\d+):\d+\s*$", re.MULTILINE)


def sha256(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def invoke(command, timeout, environment):
    start = time.perf_counter()
    try:
        result = subprocess.run(
            [str(item) for item in command], cwd=ROOT, env=environment,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout,
        )
        exit_code, stdout, stderr, timed_out = result.returncode, result.stdout, result.stderr, False
    except subprocess.TimeoutExpired as error:
        exit_code, stdout, stderr, timed_out = None, error.stdout or b"", error.stderr or b"", True
    return {
        "exit": exit_code, "seconds": round(time.perf_counter() - start, 4),
        "stdout": ANSI.sub("", stdout.decode("utf-8", errors="replace")),
        "stderr": ANSI.sub("", stderr.decode("utf-8", errors="replace")),
        "timeout": timed_out,
    }


def evidence(result, log):
    content = result["stdout"] + result["stderr"]
    if content:
        log.parent.mkdir(parents=True, exist_ok=True)
        log.write_text(content, encoding="utf-8")
    return {key: value for key, value in result.items() if key not in {"stdout", "stderr"}}


def relative_location(path, line, include_dirs):
    """Name a diagnostic's file relative to the include directory that holds it."""
    normalized = path.replace("\\", "/")
    for directory in include_dirs:
        prefix = str(directory).replace("\\", "/").rstrip("/") + "/"
        if normalized.lower().startswith(prefix.lower()):
            return f"{normalized[len(prefix):]}:{line}"
    return f"{Path(normalized).name}:{line}"


def clang_status(result, include_dirs):
    output = result["stdout"] + result["stderr"]
    diagnostics = CLANG_DIAGNOSTIC.findall(output)
    errors = [item for item in diagnostics if item[2] != "warning"]
    if result["timeout"]:
        status = "timeout"
    elif result["exit"] == 0:
        status = "accepted"
    else:
        status = "rejected"
    first = errors[0] if errors else None
    return {
        "status": status, "errors": len(errors), "warnings": len(diagnostics) - len(errors),
        "first_error": first[3] if first else None,
        "first_error_location": relative_location(first[0], first[1], include_dirs) if first else None,
    }


def bcc_result(result, include_dirs):
    """Classify bcc output; bcc exits 0 after C errors, so count rendered diagnostics."""
    status = bcc_status(result)
    location = BCC_FIRST_ERROR_LOCATION.search(result["stdout"] + result["stderr"])
    status["first_error_location"] = (relative_location(location[1], location[2], include_dirs)
                                      if location else None)
    return status


def template_fields(word):
    return {field for _, field, _, _ in string.Formatter().parse(word) if field is not None}


def check_template(template):
    words = shlex.split(template)
    for word in words:
        unknown = template_fields(word) - PLACEHOLDERS
        if unknown:
            raise ValueError(f"unknown placeholder {sorted(unknown)} in {word!r}")
    return words


def expand_template(template, values, include_dirs, source):
    """Expand a bcc argument template into arguments.

    Scalar placeholders fill in place; a word whose placeholder is empty (the
    MSVC configuration has no sysroot) is dropped. A word containing {dir}
    repeats once per include directory and is itself split into words first,
    so '--isystem {dir}' yields `--isystem`, `<dir>` for each directory.
    """
    arguments = []
    fields = {**values, "input": str(source)}
    for word in check_template(template):
        if "{dir}" in word:
            for directory in include_dirs:
                arguments.extend(part.format_map({**fields, "dir": str(directory)})
                                 for part in shlex.split(word))
        elif all(fields.get(name) for name in template_fields(word)):
            arguments.append(word.format_map(fields))
    if "{input}" not in template:
        arguments.append(str(source))
    return arguments


def version_key(name):
    return tuple(int(part) if part.isdigit() else 0 for part in re.split(r"[.]", name))


def linux_configuration(name, triple, sysroots, multiarch):
    sysroot = sysroots / name
    manifest = sysroot / "sysroot.json"
    if not manifest.exists():
        return None, f"missing {sysroot}; run scripts/fetch_libc_sysroots.py"
    include = sysroot / "usr/include"
    include_dirs = ([include / multiarch] if multiarch else []) + [include]
    return {
        "triple": triple, "sysroot": str(sysroot), "include_dirs": include_dirs,
        "header_roots": include_dirs, "extra_groups": {"posix": POSIX_HEADERS},
        "clang_flags": [f"--target={triple}", f"--sysroot={sysroot}"],
        "source": json.loads(manifest.read_text(encoding="utf-8")),
    }, None


def mingw_configuration(gcc, resource_include, environment):
    executable = shutil.which(gcc)
    if executable is None:
        return None, f"{gcc} is not on PATH"
    machine = invoke([executable, "-dumpmachine"], 30, environment)["stdout"].strip()
    if "mingw" not in machine:
        return None, f"{executable} targets {machine}, not MinGW-w64"
    verbose = invoke([executable, "-xc", "-E", "-v", os.devnull], 30, environment)["stderr"]
    listing = verbose.split("#include <...> search starts here:", 1)[-1]
    listing = listing.split("End of search list.", 1)[0]
    reported = [Path(line.strip()).resolve() for line in listing.splitlines() if line.strip()]
    # GCC's own lib/gcc/<triple>/<version>/include{,-fixed} play the role of
    # Clang's resource directory and bcc's built-in headers, so neither gets them.
    compiler_dirs = [path for path in reported
                     if any(a.lower() == "lib" and b.lower() == "gcc"
                            for a, b in zip(path.parts, path.parts[1:]))]
    include_dirs = [path for path in reported if path not in compiler_dirs]
    roots = [path for path in include_dirs if path.parent.name == machine] or include_dirs
    root = Path(executable).resolve().parent.parent
    return {
        "triple": "x86_64-w64-windows-gnu", "sysroot": str(root), "include_dirs": include_dirs,
        "header_roots": roots,
        "extra_groups": {"windows": WINDOWS_HEADERS, "mingw-posix": MINGW_POSIX_HEADERS},
        "clang_flags": ["--target=x86_64-w64-windows-gnu", "-nostdinc",
                        "-isystem", resource_include,
                        *[item for path in include_dirs for item in ("-isystem", path)]],
        "source": {
            "gcc": executable, "gcc_machine": machine,
            "gcc_version": invoke([executable, "--version"], 30, environment)["stdout"].splitlines()[0],
            "compiler_dirs_excluded": [str(path) for path in compiler_dirs],
        },
    }, None


def msvc_include_dirs(environment):
    """Return (directories, description) for the Visual Studio and UCRT headers."""
    if environment.get("INCLUDE"):
        dirs = [Path(item) for item in environment["INCLUDE"].split(os.pathsep) if item]
        return [path for path in dirs if path.is_dir()], {"from": "INCLUDE"}
    program_files = Path(environment.get("ProgramFiles(x86)", r"C:\Program Files (x86)"))
    vswhere = program_files / "Microsoft Visual Studio/Installer/vswhere.exe"
    if not vswhere.exists():
        return [], {"from": "vswhere", "error": f"missing {vswhere}"}
    query = invoke([vswhere, "-latest", "-products", "*", "-requires",
                    "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                    "-property", "installationPath"], 60, environment)
    installation = query["stdout"].strip().splitlines()
    if not installation:
        return [], {"from": "vswhere", "error": "no Visual Studio with the x64 C tools"}
    installation = Path(installation[0])
    version_file = installation / "VC/Auxiliary/Build/Microsoft.VCToolsVersion.default.txt"
    tools = version_file.read_text(encoding="utf-8").strip()
    vc_include = installation / "VC/Tools/MSVC" / tools / "include"
    kits_root = None
    try:
        import winreg
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE,
                            r"SOFTWARE\Microsoft\Windows Kits\Installed Roots") as key:
            kits_root = Path(winreg.QueryValueEx(key, "KitsRoot10")[0])
    except OSError:
        kits_root = program_files / "Windows Kits/10"
    sdks = sorted((path for path in (kits_root / "Include").glob("*")
                   if (path / "ucrt/stdio.h").exists()), key=lambda path: version_key(path.name))
    if not sdks:
        return [], {"from": "vswhere", "error": f"no UCRT headers under {kits_root / 'Include'}"}
    sdk = sdks[-1]
    dirs = [vc_include] + [sdk / part for part in ("ucrt", "shared", "um", "winrt")]
    return [path for path in dirs if path.is_dir()], {
        "from": "vswhere and the Windows Kits registry", "visual_studio": str(installation),
        "vc_tools_version": tools, "windows_sdk_version": sdk.name, "kits_root": str(kits_root),
    }


def msvc_configuration(resource_include, environment):
    include_dirs, source = msvc_include_dirs(environment)
    if not any((path / "stdio.h").exists() for path in include_dirs):
        return None, source.get("error", "no UCRT stdio.h in the MSVC include directories")
    roots = [path for path in include_dirs
             if path.name.lower() == "ucrt" or (path / "vcruntime.h").exists()]
    return {
        "triple": "x86_64-pc-windows-msvc", "sysroot": "", "include_dirs": include_dirs,
        "header_roots": roots or include_dirs, "extra_groups": {"windows": WINDOWS_HEADERS},
        "clang_flags": ["--target=x86_64-pc-windows-msvc", "-fms-extensions", "-fms-compatibility",
                        "-nostdinc", "-isystem", resource_include,
                        *[item for path in include_dirs for item in ("-isystem", path)]],
        "source": source,
    }, None


def configurations(args, resource_include, environment):
    return {
        "glibc-x86_64-linux": lambda: linux_configuration(
            "glibc-x86_64-linux", "x86_64-unknown-linux-gnu", args.sysroots, "x86_64-linux-gnu"),
        "musl-x86_64-linux": lambda: linux_configuration(
            "musl-x86_64-linux", "x86_64-unknown-linux-musl", args.sysroots, None),
        "mingw-w64": lambda: mingw_configuration(args.gcc, resource_include, environment),
        "msvc-ucrt": lambda: msvc_configuration(resource_include, environment),
    }


def locate(header, roots):
    for root in roots:
        if (root / header).is_file():
            return str(root / header)
    return None


def all_headers(roots):
    """Every .h beneath the header roots, skipping other roots and other multiarch triples."""
    found = set()
    for root in roots:
        for directory, subdirectories, files in os.walk(root):
            here = Path(directory)
            subdirectories[:] = sorted(
                name for name in subdirectories
                if here / name not in roots and not (here == root and "-linux-" in name))
            found.update((here / name).relative_to(root).as_posix()
                         for name in files if name.endswith(".h"))
    return sorted(found)


def header_rows(name, configuration, args):
    """List the (header, group, std) rows for one configuration."""
    roots = configuration["header_roots"]
    groups = {header: "c-standard" for header in C17_HEADERS}
    groups.update({header: "c23" for header in C23_HEADERS if locate(header, roots)})
    for group, headers in configuration["extra_groups"].items():
        for header in headers:
            groups.setdefault(header, group)
    if args.all_headers:
        for header in all_headers(roots):
            groups.setdefault(header, "all")
    rows = []
    for header, group in sorted(groups.items()):
        if args.match and not any(fnmatchcase(header, pattern) for pattern in args.match):
            continue
        modes = ["gnu17", "c17"] if group in {"c-standard", "c23"} else ["gnu17"]
        rows.extend({"config": name, "header": header, "std": std, "group": group,
                     "libc_path": locate(header, configuration["include_dirs"])} for std in modes)
    return rows


def translation_unit(includes):
    # A header such as stdalign.h declares nothing, and bcc rejects an empty
    # translation unit (C99 §6.9p1 requires an external declaration), so each
    # unit ends with one declaration of its own.
    return "".join(f"#include <{item}>\n" for item in includes) + SENTINEL


def survey_row(row, configuration, args, environment):
    name, header, std = row["config"], row["header"], row["std"]
    include_dirs = configuration["include_dirs"]
    stem = "_c_standard" if header == COMBINED else header
    source = args.output / "tu" / name / std / (stem + ".c")
    source.parent.mkdir(parents=True, exist_ok=True)
    includes = row.get("includes", [header])
    source.write_text(translation_unit(includes), encoding="utf-8")
    logs = args.output / "logs" / name / std
    clang = invoke([args.clang, *configuration["clang_flags"], f"-std={std}", "-fsyntax-only",
                    "-fno-color-diagnostics", source], args.timeout, environment)
    row["clang"] = {**clang_status(clang, include_dirs),
                    **evidence(clang, logs / "clang" / (stem + ".log"))}
    if row["clang"]["status"] != "accepted":
        row["excluded"] = "clang-rejected"
        row["bcc"] = None
        return row
    values = {"triple": configuration["triple"], "sysroot": configuration["sysroot"],
              "std": std, "config": name}
    template = args.bcc_args_for.get(name, args.bcc_args)
    command = [args.bcc, *expand_template(template, values, include_dirs, source)]
    bcc = invoke(command, args.timeout, environment)
    row["bcc"] = {**bcc_result(bcc, include_dirs), **evidence(bcc, logs / "bcc" / (stem + ".log"))}
    row["excluded"] = None
    return row


def summarize(rows):
    summary = {}
    for name in sorted({row["config"] for row in rows}):
        summary[name] = {}
        for std in ("gnu17", "c17"):
            selected = [row for row in rows if row["config"] == name and row["std"] == std]
            if not selected:
                continue
            headers = [row for row in selected if row["header"] != COMBINED]
            eligible = [row for row in headers if row["bcc"] is not None]
            combined = next((row for row in selected if row["header"] == COMBINED), None)
            summary[name][std] = {
                "headers": len(headers),
                "clang_accepted": len(eligible),
                "bcc_accepted": sum(row["bcc"]["status"] == "accepted" for row in eligible),
                "bcc_status": dict(Counter(row["bcc"]["status"] for row in eligible)),
                "groups": {group: {"clang_accepted": sum(row["group"] == group for row in eligible),
                                   "bcc_accepted": sum(row["group"] == group
                                                       and row["bcc"]["status"] == "accepted"
                                                       for row in eligible)}
                           for group in sorted({row["group"] for row in headers})},
                "clang_rejected": {row["header"]: row["clang"]["first_error"]
                                   for row in headers if row["bcc"] is None},
                "combined": None if combined is None else {
                    "includes": len(combined["includes"]),
                    "clang": combined["clang"]["status"],
                    "bcc": combined["bcc"]["status"] if combined["bcc"] else None,
                    "bcc_first_error": combined["bcc"]["first_error"] if combined["bcc"] else None,
                },
                "bcc_first_errors": Counter(row["bcc"]["first_error"] for row in eligible
                                            if row["bcc"]["first_error"]).most_common(15),
            }
    return summary


def print_summary(summary):
    print(f"\n{'configuration':<20} {'std':<6} {'headers':>7} {'clang ok':>8} {'bcc ok':>6} "
          f"{'combined (clang/bcc)':>22}")
    for name, modes in summary.items():
        for std, item in modes.items():
            combined = item["combined"]
            combined_text = "-" if combined is None else f"{combined['clang']}/{combined['bcc'] or '-'}"
            print(f"{name:<20} {std:<6} {item['headers']:>7} {item['clang_accepted']:>8} "
                  f"{item['bcc_accepted']:>6} {combined_text:>22}")
    for name, modes in summary.items():
        item = modes.get("gnu17") or next(iter(modes.values()))
        print(f"\n{name}: bcc status {item['bcc_status']}; Clang rejected "
              f"{len(item['clang_rejected'])}: {', '.join(sorted(item['clang_rejected'])) or '-'}")
        for message, count in item["bcc_first_errors"][:8]:
            print(f"  {count:>4}  {message}")


def compare(old_path, new_report):
    old = json.loads(Path(old_path).read_text(encoding="utf-8"))

    def accepted(report):
        return {(row["config"], row["std"], row["header"]): bool(row["bcc"])
                and row["bcc"]["status"] == "accepted" for row in report["rows"]}

    before, after = accepted(old), accepted(new_report)
    shared = sorted(before.keys() & after.keys())
    gained = [key for key in shared if after[key] and not before[key]]
    lost = [key for key in shared if before[key] and not after[key]]
    print(f"\nCompared with {old_path} ({len(shared)} shared rows; "
          f"{len(after.keys() - before.keys())} only in the new run, "
          f"{len(before.keys() - after.keys())} only in the old)")
    for title, keys in (("Newly accepted", gained), ("Newly rejected", lost)):
        print(f"{title}: {len(keys)}")
        for config, std, header in keys:
            print(f"  {config} {std} {header}")
    return {"old": str(old_path), "newly_accepted": gained, "newly_rejected": lost}


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="Template placeholders: {triple} {sysroot} {std} {config} {input} and {dir}. "
               "A word containing {dir} repeats for each include directory; a word whose "
               "placeholder is empty is dropped; the translation unit is appended unless "
               "{input} appears. Default: " + DEFAULT_BCC_ARGS)
    parser.add_argument("--bcc", type=Path, help="bcc-rust binary (defaults to Cargo's host-target debug build)")
    parser.add_argument("--clang", type=Path, help="Clang (defaults to target/llvm/bin/clang, then PATH)")
    parser.add_argument("--gcc", default="gcc", help="MinGW-w64 GCC that reports its include directories")
    parser.add_argument("--sysroots", type=Path, default=ROOT / "target/sysroots")
    parser.add_argument("--output", type=Path, default=ROOT / "target/libc-survey/results")
    parser.add_argument("--config", action="append", default=[],
                        help="Survey only this configuration (repeatable)")
    parser.add_argument("--match", action="append", default=[],
                        help="Select headers by glob (repeatable), e.g. 'sys/*'")
    parser.add_argument("--all-headers", action="store_true",
                        help="Also survey every .h under each configuration's libc include roots")
    parser.add_argument("--bcc-args", default=DEFAULT_BCC_ARGS, help="bcc argument template")
    parser.add_argument("--bcc-args-for", nargs=2, action="append", default=[],
                        metavar=("CONFIG", "TEMPLATE"), help="bcc argument template for one configuration")
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--compare", metavar="OLD_RESULTS_JSON",
                        help="After the run, list headers newly accepted or rejected since OLD")
    parser.add_argument("--against", metavar="NEW_RESULTS_JSON",
                        help="With --compare, compare OLD with this finished run instead of surveying")
    args = parser.parse_args()
    if args.against:
        if not args.compare:
            parser.error("--against requires --compare")
        compare(args.compare, json.loads(Path(args.against).read_text(encoding="utf-8")))
        return 0
    if args.jobs < 1 or args.timeout <= 0:
        parser.error("jobs and timeout must be positive")
    if args.bcc is None:
        rust_info = subprocess.check_output([os.environ.get("RUSTC", "rustc"), "-vV"], text=True)
        host = next(line.removeprefix("host: ") for line in rust_info.splitlines()
                    if line.startswith("host: "))
        args.bcc = ROOT / "target" / host / "debug" / ("bcc-rust.exe" if os.name == "nt" else "bcc-rust")
    if args.clang is None:
        bundled = ROOT / "target/llvm/bin" / ("clang.exe" if os.name == "nt" else "clang")
        args.clang = bundled if bundled.exists() else Path(shutil.which("clang") or "clang")
    args.bcc = args.bcc.resolve(strict=True)
    args.clang = args.clang.resolve(strict=True)
    args.sysroots = args.sysroots.resolve()
    args.output = args.output.resolve()
    try:
        for template in [args.bcc_args, *(item[1] for item in args.bcc_args_for)]:
            check_template(template)
    except ValueError as error:
        parser.error(str(error))
    if (args.output / "results.json").exists():
        parser.error("output already contains results; choose a new --output directory")
    environment = os.environ.copy()
    for variable in ("CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "OBJC_INCLUDE_PATH"):
        environment.pop(variable, None)
    resource_include = Path(invoke([args.clang, "-print-resource-dir"], 30, environment)["stdout"].strip()) / "include"
    available = configurations(args, resource_include, environment)
    unknown = {*args.config, *(name for name, _ in args.bcc_args_for)} - available.keys()
    if unknown:
        parser.error(f"unknown configuration {sorted(unknown)}; choose from {sorted(available)}")
    args.bcc_args_for = dict(args.bcc_args_for)
    selected, skipped, rows = {}, {}, []
    for name in args.config or sorted(available):
        configuration, reason = available[name]()
        if configuration is None:
            if args.config:
                parser.error(f"{name}: {reason}")
            skipped[name] = reason
            print(f"Skipping {name}: {reason}", flush=True)
            continue
        selected[name] = configuration
        rows.extend(header_rows(name, configuration, args))
    if not rows:
        parser.error("no headers selected")
    args.output.mkdir(parents=True, exist_ok=True)
    metadata = {
        "utc": datetime.now(timezone.utc).isoformat(),
        "bcc_binary": str(args.bcc), "bcc_binary_sha256": sha256(args.bcc),
        "clang": str(args.clang),
        "clang_version": invoke([args.clang, "--version"], 30, environment)["stdout"].splitlines()[0],
        "clang_resource_include": str(resource_include),
        "git_head": invoke(["git", "rev-parse", "HEAD"], 30, environment)["stdout"].strip(),
        "git_status": invoke(["git", "status", "--short"], 30, environment)["stdout"],
        "host": platform.platform(), "python_version": platform.python_version(),
        "jobs": args.jobs, "timeout_seconds": args.timeout, "match": args.match,
        "all_headers": args.all_headers, "skipped_configurations": skipped,
        "configurations": {
            name: {
                "triple": item["triple"], "sysroot": item["sysroot"],
                "include_dirs": [str(path) for path in item["include_dirs"]],
                "header_roots": [str(path) for path in item["header_roots"]],
                "clang_flags": [str(flag) for flag in item["clang_flags"]],
                "bcc_args_template": args.bcc_args_for.get(name, args.bcc_args),
                "source": item["source"],
            } for name, item in selected.items()
        },
    }
    start = time.perf_counter()
    finished = []
    print(f"Surveying {len(rows)} translation units in {', '.join(selected)} "
          f"with {args.jobs} workers; {args.output}", flush=True)
    with (args.output / "results.jsonl").open("w", encoding="utf-8") as progress:
        def run(batch):
            with ThreadPoolExecutor(max_workers=args.jobs) as pool:
                futures = [pool.submit(survey_row, row, selected[row["config"]], args, environment)
                           for row in batch]
                for future in as_completed(futures):
                    row = future.result()
                    finished.append(row)
                    progress.write(json.dumps(row) + "\n")
                    progress.flush()
                    if len(finished) % 100 == 0:
                        print(f"{len(finished)} done in {time.perf_counter() - start:.1f}s", flush=True)

        run(rows)
        # The combined unit holds the C standard headers Clang accepted one by one.
        combined = []
        for name in selected:
            for std in ("gnu17", "c17"):
                includes = sorted(row["header"] for row in finished
                                  if row["config"] == name and row["std"] == std
                                  and row["group"] in {"c-standard", "c23"} and row["bcc"] is not None)
                if includes:
                    combined.append({"config": name, "header": COMBINED, "std": std,
                                     "group": "combined", "libc_path": None, "includes": includes})
        run(combined)
    finished.sort(key=lambda row: (row["config"], row["std"], row["header"]))
    metadata["elapsed_seconds"] = round(time.perf_counter() - start, 3)
    report = {"metadata": metadata, "summary": summarize(finished), "rows": finished}
    print_summary(report["summary"])
    if args.compare:
        report["comparison"] = compare(args.compare, report)
    (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    # Rejections are the measurement; crashes and timeouts must stay visible.
    return int(any(row["clang"]["status"] == "timeout" or (
        row["bcc"] and row["bcc"]["status"] in {"crash", "process_failure", "timeout"})
        for row in finished))


if __name__ == "__main__":
    raise SystemExit(main())
