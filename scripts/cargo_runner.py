#!/usr/bin/env python3
"""Cargo `runner` for Windows targets: run a binary without crash dialogs.

.cargo/config.toml names this script as the runner, so `cargo test` and
`cargo run` start every binary through it. It sets the process error mode that
suppresses Windows Error Reporting dialogs (see crash_dialogs.py), which the
binary and every program it starts inherit, then runs the binary with the
given arguments and exits with its status.
"""

from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import crash_dialogs  # noqa: E402


def main() -> int:
    if len(sys.argv) < 2:
        sys.exit("cargo_runner.py: expected a program to run")
    crash_dialogs.suppress()
    try:
        return subprocess.call(sys.argv[1:])
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
