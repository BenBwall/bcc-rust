#!/usr/bin/env python3
"""Run a repository script with the environment it needs.

Usage: python scripts/run.py <script> [args...]

<script> names a file in scripts/, with or without its .rs or .py extension.
Cargo scripts run under `cargo +nightly -Zscript` without the repository's
rustflags: .cargo/config.toml names the linker by a path relative to the
repository root, which a script package rooted in scripts/ cannot find.
"""

import os
from pathlib import Path
import subprocess
import sys

SCRIPTS = Path(__file__).resolve().parent


def resolve(name: str) -> Path:
    for candidate in (SCRIPTS / name, SCRIPTS / f"{name}.rs", SCRIPTS / f"{name}.py"):
        if candidate.is_file():
            return candidate
    sys.exit(f"run.py: no script named {name!r} in {SCRIPTS}")


def main() -> int:
    if len(sys.argv) < 2 or sys.argv[1] in ("-h", "--help"):
        print(__doc__.strip())
        names = sorted(p.name for p in SCRIPTS.iterdir() if p.suffix in (".rs", ".py") and p.name != "run.py")
        print("\nScripts:", *names, sep="\n  ")
        return 0 if len(sys.argv) >= 2 else 2
    script, args = resolve(sys.argv[1]), sys.argv[2:]
    environment = os.environ.copy()
    if script.suffix == ".rs":
        # An empty CARGO_ENCODED_RUSTFLAGS overrides every config rustflags entry.
        environment.pop("RUSTFLAGS", None)
        environment["CARGO_ENCODED_RUSTFLAGS"] = ""
        command = ["cargo", "+nightly", "-Zscript", str(script), *args]
    elif script.suffix == ".py":
        command = [sys.executable, str(script), *args]
    else:
        sys.exit(f"run.py: cannot run {script.name}; expected a .rs or .py script")
    try:
        return subprocess.run(command, env=environment).returncode
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
