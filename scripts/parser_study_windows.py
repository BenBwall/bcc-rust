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



def setup():
    """Unpack pinned cross tools and Wine locally; never install system-wide."""
    downloads = BASE / "downloads"
    downloads.mkdir(parents=True, exist_ok=True)
    pins = {
        "wine64": "10.0~repack-12ubuntu1", "libwine": "10.0~repack-12ubuntu1",
        "wine-common": "10.0~repack-12ubuntu1", "libz-mingw-w64": "1.3.1+dfsg-2",
        "mingw-w64-x86-64-dev": "13.0.0-2ubuntu1", "mingw-w64-common": "13.0.0-2ubuntu1",
        "gcc-mingw-w64-x86-64-posix": "13.2.0-6ubuntu1+26.1",
        "gcc-mingw-w64-x86-64-posix-runtime": "13.2.0-6ubuntu1+26.1",
        "binutils-mingw-w64-x86-64": "2.45.90.20260125-1ubuntu1+13.3",
        "libunwind8": "1.8.3-0ubuntu1",
    }
    manifest = []

    def download(url, destination, digest=None):
        if not destination.exists() or digest and hashlib.sha256(destination.read_bytes()).hexdigest() != digest:
            subprocess.run(["curl", "--fail", "--location", "--retry", "2", "--max-time", "180",
                            "--output", str(destination), url], check=True)
        actual = hashlib.sha256(destination.read_bytes()).hexdigest()
        if digest and actual != digest:
            raise ValueError("download checksum mismatch: " + str(destination))
        return actual

    for package, version in pins.items():
        selector = package + ("=" + version if version else "")
        metadata = subprocess.check_output(["apt-cache", "show", selector], text=True).split("\n\n")[0]
        fields = dict(line.split(": ", 1) for line in metadata.splitlines() if ": " in line and not line.startswith(" "))
        url = "https://archive.ubuntu.com/ubuntu/" + fields["Filename"]
        destination = downloads / Path(fields["Filename"]).name
        digest = download(url, destination, fields["SHA256"])
        subprocess.run(["dpkg-deb", "-x", str(destination), str(BASE / "root")], check=True)
        manifest.append({"package": package, "version": fields["Version"], "url": url, "sha256": digest})
    for target in ("x86_64-pc-windows-gnu", "x86_64-pc-windows-msvc"):
        name = f"rust-std-1.99.0-{target}"
        url = f"https://static.rust-lang.org/dist/{name}.tar.xz"
        checksum = downloads / (name + ".sha256")
        download(url + ".sha256", checksum)
        archive = downloads / (name + ".tar.xz")
        digest = download(url, archive, checksum.read_text().split()[0])
        subprocess.run(["tar", "-xf", str(archive), "-C", str(BASE)], check=True)
        shutil.copytree(BASE / name / ("rust-std-" + target) / "lib", BASE / "sysroot/lib", dirs_exist_ok=True)
        manifest.append({"target": target, "url": url, "sha256": digest})
    # Debian's data prefix includes wine, and Wine appends another wine/nls.
    nested = BASE / "root/usr/share/wine/wine"
    nested.mkdir(parents=True, exist_ok=True)
    if not (nested / "nls").exists():
        (nested / "nls").symlink_to("../nls")
    # Local extraction skips package installation's integration of native DLLs.
    system = BASE / "wine-prefix-2/drive_c/windows/system32"
    system.mkdir(parents=True, exist_ok=True)
    for source in (BASE / "root/usr/lib/x86_64-linux-gnu/wine/x86_64-windows").iterdir():
        if source.suffix in (".dll", ".exe") and not (system / source.name).exists():
            (system / source.name).symlink_to(source)
    shutil.copy2(BASE / "root/usr/x86_64-w64-mingw32/lib/zlib1.dll", system / "zlib1.dll")
    (BASE / "setup-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


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
        flags += ["-C", "linker=" + str(BASE / "root/usr/bin/x86_64-w64-mingw32-gcc-posix"),
                  "-C", "link-self-contained=no",
                  "-C", "target-feature=+crt-static", "-L", "native=" + str(args.mingw.resolve())]
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
        env.update(WINEPREFIX=str(BASE / "wine-prefix-2"), WINEDEBUG="-all",
                   WINEDLLPATH=str(args.wine.resolve().parent), WINEDLLOVERRIDES="mscoree,mshtml=")
        env["WINESERVER"] = str(args.wine.resolve().parent / "wineserver")
        env["LD_LIBRARY_PATH"] = os.pathsep.join([str(args.wine.resolve().parents[1]),
                                                          str(args.wine.resolve().parent / "x86_64-unix"),
                                                          env.get("LD_LIBRARY_PATH", "")])
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
        metrics = next((line for line in row.get("stdout", "").splitlines() if line.startswith("reserve=")), None)
        if metrics:
            row["metrics"] = {key: int(value) for key, value in (part.split("=", 1) for part in metrics.split())}
        rows.append(row)
        args.output.write_text(json.dumps({"metadata": metadata, "rows": rows}, indent=2) + "\n")
        print(fields, row["returncode"], flush=True)

    execute([BASE / "probe-tests.exe", "--test-threads=1"], kind="unit-tests")
    for reserve, depth in probes:
        for repeat in range(args.repeats):
            for profile, binary in binaries.items():
                execute([binary, reserve, depth], profile=profile, reserve=reserve, depth=depth, repeat=repeat,
                        expected_success=not (reserve == 1024**2 and depth == 4096
                                              or reserve == 64 * 1024**2 and depth == 16384))
    failures = []
    for row in rows:
        if row.get("expected_success", True):
            m = row.get("metrics", {})
            good = row["returncode"] == 0
            if row.get("kind") != "unit-tests":
                good &= (m.get("actual", 0) >= row["reserve"] and m.get("before", row["reserve"]) < 4 * 1024**2
                         and m.get("span", row["reserve"]) < m.get("actual", 0)
                         and "passed=3" in row.get("stdout", ""))
                if row["depth"] > 0:
                    good &= m.get("deepest", 0) > m.get("before", 0)
            if not good:
                failures.append(row)
        elif row["returncode"] in (0, "timeout"):
            failures.append(row)
    if failures:
        raise SystemExit(f"{len(failures)} unexpected probe outcomes; inspect {args.output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    sub.add_parser("setup")
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
    if args.action == "setup":
        setup()
    elif args.action == "prepare":
        prepare()
    elif args.action == "build":
        build(args)
    else:
        collect(args)


if __name__ == "__main__":
    main()
