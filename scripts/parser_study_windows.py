#!/usr/bin/env python3
"""Build and collect the actual reserved-stack module in an isolated Windows probe.

Run prepare normally; run build ONLY inside the shared swarm wrapper. Cross
builds require an unpacked matching Rust standard library and MinGW import
libraries. This does not cross-build the compiler or make Wine Windows evidence.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / ".scratch/parser-study/windows"


def prepare():
    BASE.mkdir(parents=True, exist_ok=True)
    original = tomllib.loads((ROOT / "Cargo.toml").read_text())
    text = '''[package]
name = "parser-stack-windows-probe"
version = "0.0.0"
edition = "2024"
description = "Reserved-stack portability probe"
license-file = %s
repository = "https://github.com/BenBwall/bcc-rust"
readme = %s
keywords = ["compiler", "benchmark"]
categories = ["development-tools"]
[workspace]
[[bin]]
name = "parser_stack_probe"
path = %s
[dependencies]
windows-sys = { version = "0.61.2", features = ["Win32_Foundation", "Win32_Security", "Win32_System_Memory", "Win32_System_Threading", "Win32_System_Diagnostics_Debug"] }
libc = "0.2"
[profile.release]
lto = "fat"
codegen-units = 1
''' % (json.dumps(str(ROOT / "LICENSE")), json.dumps(str(ROOT / "README.md")), json.dumps(str(ROOT / "examples/parser_stack_probe.rs")))
    for group, values in original["lints"].items():
        text += f"\n[lints.{group}]\n"
        for key, value in values.items():
            rendered = json.dumps(value) if isinstance(value, str) else "{" + ",".join(
                f"{k}={json.dumps(v)}" for k, v in value.items()) + "}"
            text += f"{json.dumps(key)} = {rendered}\n"
    (BASE / "Cargo.toml").write_text(text)


def build(args):
    target_dir = os.environ.get("CARGO_TARGET_DIR")
    if not target_dir:
        raise SystemExit("invoke build through the shared swarm wrapper from the worktree root")
    env = os.environ.copy()
    flags = ["--sysroot=" + str(args.sysroot.resolve())]
    if args.target.endswith("-gnu"):
        flags += ["-C", "linker=" + str(ROOT / "target/llvm/bin/ld.lld"),
                  "-C", "linker-flavor=ld.lld", "-L", "native=" + str(args.mingw.resolve())]
    for directory in sorted((BASE / "root/usr/lib/gcc/x86_64-w64-mingw32").glob("*")):
        flags += ["-L", "native=" + str(directory)]
    env["PATH"] = str(BASE / "root/usr/bin") + os.pathsep + env.get("PATH", "")
    env["RUSTFLAGS"] = " ".join(flags)
    common = ["--manifest-path", str(BASE / "Cargo.toml"), "--target", args.target, "--locked"]
    # prepare is deliberately separate so a build never changes inputs mid-run.
    for command in (["cargo", "clippy", *common, "--all-targets", "--", "-D", "warnings"],):
        print(shlex.join(command), flush=True)
        subprocess.run(command, env=env, cwd=ROOT, check=True)
    if args.target.endswith("-msvc"):
        return  # compile and lint only: no MSVC SDK/linker libraries on Linux
    for profile in ("debug", "release"):
        command = ["cargo", "build", *common]
        if profile == "release":
            command += ["--release"]
        subprocess.run(command, env=env, cwd=ROOT, check=True)
        binary = Path(target_dir) / args.target / profile / "parser_stack_probe.exe"
        shutil.copy2(binary, BASE / f"probe-{profile}.exe")
    completed = subprocess.run(["cargo", "test", *common, "--no-run", "--message-format=json"],
                               env=env, cwd=ROOT, check=True, capture_output=True, text=True)
    for line in completed.stdout.splitlines():
        message = json.loads(line)
        if message.get("executable") and message.get("profile", {}).get("test"):
            shutil.copy2(message["executable"], BASE / "probe-tests.exe")
    print(completed.stderr, end="", flush=True)


def collect(args):
    args.output.parent.mkdir(parents=True, exist_ok=True)
    runner = [sys.executable, str(ROOT / "scripts/cargo_runner.py")]
    if args.wine:
        runner += [str(args.wine.resolve())]
    env = os.environ.copy()
    if args.wine:
        env.update(WINEPREFIX=str(BASE / "wine-prefix"), WINEDEBUG="-all", WINEDLLOVERRIDES="mscoree,mshtml=")
        env["WINESERVER"] = str(args.wine.resolve().parent / "wineserver")
        env["LD_LIBRARY_PATH"] = str(args.wine.resolve().parents[1]) + os.pathsep + env.get("LD_LIBRARY_PATH", "")
    rows = []
    # Avoid claiming failure-reservation probes as normal successful samples.
    probes = [(1024**3 * 100, d) for d in (0, 256, 4096, 16384, 65536)]
    probes += [(1024**2, 0), (1024**2, 4096), (64 * 1024**2, 16384), (1024**3, 65536)]
    binaries = {p: BASE / f"probe-{p}.exe" for p in ("debug", "release")}
    metadata = {"runtime": "Wine compatibility layer" if args.wine else "native Windows",
                "binary_sha256": {p: hashlib.sha256(b.read_bytes()).hexdigest() for p, b in binaries.items()}}

    def execute(command, **fields):
        before = time.monotonic()
        try:
            result = subprocess.run(runner + list(map(str, command)), cwd=ROOT, env=env,
                                    capture_output=True, text=True, timeout=60)
            row = fields | {"command": runner + list(map(str, command)), "returncode": result.returncode,
                            "stdout": result.stdout, "stderr": result.stderr,
                            "elapsed_s": time.monotonic() - before}
        except subprocess.TimeoutExpired:
            row = fields | {"returncode": "timeout", "elapsed_s": time.monotonic() - before}
        rows.append(row)
        args.output.write_text(json.dumps({"metadata": metadata, "rows": rows}, indent=2) + "\n")
        print(fields, row["returncode"], flush=True)

    execute([BASE / "probe-tests.exe", "--test-threads=1"], kind="unit-tests")
    for reserve, depth in probes:
        for repeat in range(args.repeats):
            for profile, binary in binaries.items():
                execute([binary, reserve, depth], profile=profile, reserve=reserve, depth=depth, repeat=repeat)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    sub.add_parser("prepare")
    builder = sub.add_parser("build")
    builder.add_argument("--sysroot", type=Path, default=BASE / "sysroot")
    builder.add_argument("--mingw", type=Path, default=BASE / "root/usr/x86_64-w64-mingw32/lib")
    builder.add_argument("--target", default="x86_64-pc-windows-gnu", choices=["x86_64-pc-windows-gnu", "x86_64-pc-windows-msvc"])
    collector = sub.add_parser("collect")
    collector.add_argument("--wine", type=Path)
    collector.add_argument("--repeats", type=int, default=3)
    collector.add_argument("--output", type=Path, default=BASE / "results.json")
    args = parser.parse_args()
    if args.action == "prepare":
        prepare()
    elif args.action == "build":
        build(args)
    else:
        collect(args)


if __name__ == "__main__":
    main()
