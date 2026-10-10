"""Guard CodeGraph's ancestor search from serving a parent of a nested worktree."""

import json
import os
import shutil
import subprocess
import sys


def fail(message):
    print(message, file=sys.stderr)
    return 1


def main():
    try:
        root = subprocess.check_output(
            ["git", "rev-parse", "--show-toplevel"],
            stdin=subprocess.DEVNULL, stderr=subprocess.PIPE,
            text=True, encoding="utf-8",
        ).strip()
    except (OSError, subprocess.CalledProcessError, UnicodeError):
        return fail("Cannot find the checkout root with git rev-parse")

    exe = shutil.which("codegraph")
    if exe is None:
        return fail("CodeGraph CLI not found on PATH; install codegraph")

    # Pass "." from the checkout root so batch shims never parse path metacharacters.
    try:
        result = subprocess.run(
            [exe, "status", "--json", "."], cwd=root, stdin=subprocess.DEVNULL,
            capture_output=True, text=True, encoding="utf-8", errors="replace",
        )
    except OSError:
        return fail("CodeGraph status failed; cannot verify this checkout's index")
    if result.returncode:
        return fail("CodeGraph status failed; cannot verify this checkout's index")
    try:
        status = json.loads(result.stdout)
    except ValueError:
        return fail("CodeGraph status did not return valid JSON")

    owns_index = False
    if isinstance(status, dict) and status.get("initialized") is True:
        project = status.get("projectPath")
        if isinstance(project, str) and project:
            try:
                owns_index = os.path.samefile(project, root)
            except (OSError, ValueError):
                pass
    if not owns_index:
        return fail(
            "This checkout has no CodeGraph index of its own; run codegraph init here"
        )

    try:
        return subprocess.run([exe, "serve", "--mcp", "--path", "."], cwd=root).returncode
    except OSError:
        return fail("CodeGraph serve failed to start")


if __name__ == "__main__":
    sys.exit(main())
