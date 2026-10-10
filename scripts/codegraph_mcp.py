"""Guard CodeGraph's ancestor search from serving a parent of a nested worktree.

Without arguments, serve MCP for this checkout's own index or refuse. With
`prompt-hook`, run CodeGraph's Claude Code prompt hook against that index, or
add nothing to the prompt when the checkout has no index of its own.
"""

import json
import os
import shutil
import subprocess
import sys


class Refused(Exception):
    pass


def own_index():
    """Return the CLI and checkout root once CodeGraph reports that root's index."""
    try:
        root = subprocess.check_output(
            ["git", "rev-parse", "--show-toplevel"],
            stdin=subprocess.DEVNULL, stderr=subprocess.PIPE,
            text=True, encoding="utf-8",
        ).strip()
    except (OSError, subprocess.CalledProcessError, UnicodeError):
        raise Refused("Cannot find the checkout root with git rev-parse")

    exe = shutil.which("codegraph")
    if exe is None:
        raise Refused("CodeGraph CLI not found on PATH; install codegraph")

    # Pass "." from the checkout root so batch shims never parse path metacharacters.
    try:
        result = subprocess.run(
            [exe, "status", "--json", "."], cwd=root, stdin=subprocess.DEVNULL,
            capture_output=True, text=True, encoding="utf-8", errors="replace",
        )
    except OSError:
        raise Refused("CodeGraph status failed; cannot verify this checkout's index")
    if result.returncode:
        raise Refused("CodeGraph status failed; cannot verify this checkout's index")
    try:
        status = json.loads(result.stdout)
    except ValueError:
        raise Refused("CodeGraph status did not return valid JSON")

    if isinstance(status, dict) and status.get("initialized") is True:
        project = status.get("projectPath")
        if isinstance(project, str) and project:
            try:
                if os.path.samefile(project, root):
                    return exe, root
            except (OSError, ValueError):
                pass
    raise Refused("This checkout has no CodeGraph index of its own; run codegraph init here")


def serve():
    try:
        exe, root = own_index()
    except Refused as refusal:
        print(refusal, file=sys.stderr)
        return 1
    try:
        return subprocess.run([exe, "serve", "--mcp", "--path", "."], cwd=root).returncode
    except OSError:
        print("CodeGraph serve failed to start", file=sys.stderr)
        return 1


def prompt_hook():
    # A prompt must never wait on, or be reported against, a missing index.
    try:
        exe, root = own_index()
        return subprocess.run([exe, "prompt-hook"], cwd=root).returncode
    except (Refused, OSError):
        return 0


def main(arguments):
    if arguments == []:
        return serve()
    if arguments == ["prompt-hook"]:
        return prompt_hook()
    print("usage: codegraph_mcp.py [prompt-hook]", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
