#!/usr/bin/env python3
"""Submit native Windows tests, benchmarks, and CI through GitHub Actions."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import uuid

REPOSITORY = "BenBwall/bcc-rust"
BRANCH = "ci/windows-runner"
WORKFLOW = "ci.yml"
DEFAULT_CHECKOUT = Path("/home/agent/workspace/bcc-rust/checkout")


def checkout_commit(requested=None):
    """Use the current worktree, or the sandbox checkout outside a repository."""
    if requested is None:
        current = subprocess.run(["git", "rev-parse", "--show-toplevel"],
                                 text=True, capture_output=True)
        checkout = Path(current.stdout.strip()) if current.returncode == 0 else DEFAULT_CHECKOUT
    else:
        checkout = Path(requested).expanduser()
    root = subprocess.run(["git", "-C", str(checkout), "rev-parse", "--show-toplevel"],
                          text=True, capture_output=True)
    if root.returncode:
        raise ValueError(f"No Git checkout found at {checkout}. Pass --checkout PATH or --sha FULL_SHA.")
    checkout = Path(root.stdout.strip())
    try:
        sha = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"],
                                      text=True, stderr=subprocess.PIPE).strip()
        status = subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"],
                                         text=True, stderr=subprocess.PIPE)
    except subprocess.CalledProcessError as error:
        raise ValueError(f"Cannot read checkout {checkout}: {error.stderr.strip()}") from error
    if status.strip():
        raise ValueError(f"Checkout {checkout} is dirty; commit/push first, "
                         "or explicitly select a pushed --sha.")
    return checkout, sha


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for suite in ("test", "bench", "ci", "torture"):
        command = commands.add_parser(suite)
        source = command.add_mutually_exclusive_group()
        source.add_argument("--sha", help="full pushed commit; works from any directory")
        source.add_argument("--checkout", metavar="PATH",
                            help="clean source checkout; defaults to the current worktree, "
                                 "or the sandbox checkout outside a repository")
        command.add_argument("--wait", action="store_true", help="wait and return the job's exit status")
    commands.add_parser("status")
    commands.add_parser("list")
    for operation in ("wait", "logs", "download"):
        command = commands.add_parser(operation)
        command.add_argument("run_id", type=int)
        if operation == "download":
            command.add_argument("--dir", required=True, help="artifact destination (prefer the shared mount)")
    # Everything after -- belongs to Cargo (or, for torture, to
    # scripts/run_gcc_torture_execute.py), without shell parsing/requoting.
    cli = sys.argv[1:]
    separator = cli.index("--") if "--" in cli else len(cli)
    cargo_arguments = cli[separator + 1:] if separator < len(cli) else []
    args = parser.parse_args(cli[:separator])
    if cargo_arguments and args.command not in ("test", "bench", "torture"):
        parser.error("Arguments after -- are supported only by test, bench and torture")
    if args.command == "status":
        return gh("api", f"repos/{REPOSITORY}/actions/runners", "--jq",
                  "{total_count,runners:[.runners[]|{name,status,busy,labels:[.labels[].name]}]}").returncode
    if args.command == "list":
        return gh("run", "list", "--workflow", WORKFLOW, "--branch", BRANCH, "--limit", "15").returncode
    if args.command == "wait":
        return gh("run", "watch", str(args.run_id), "--exit-status").returncode
    if args.command == "logs":
        return gh("run", "view", str(args.run_id), "--log").returncode
    if args.command == "download":
        return gh("run", "download", str(args.run_id), "--dir", args.dir).returncode
    sha = args.sha
    if not sha:
        try:
            checkout, sha = checkout_commit(args.checkout)
        except ValueError as error:
            parser.error(str(error))
        print(f"Source checkout: {checkout}\nSource commit: {sha}", file=sys.stderr)
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        parser.error("--sha must be a full lowercase 40-character commit SHA")
    visible = gh("api", f"repos/{REPOSITORY}/commits/{sha}", capture_output=True)
    if visible.returncode:
        sys.exit(f"Commit {sha} is not accessible on GitHub. "
                 "Push its branch before submitting Windows jobs, or select a pushed --sha.")
    request = uuid.uuid4().hex
    result = gh("workflow", "run", WORKFLOW, "--ref", BRANCH,
                "-f", f"commit_sha={sha}", "-f", f"suite={args.command}",
                "-f", f"cargo_args_json={json.dumps(cargo_arguments)}", "-f", f"request_id={request}")
    if result.returncode:
        return result.returncode
    # The request ID prevents concurrent agents from taking each other's runs.
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        response = gh("run", "list", "--workflow", WORKFLOW, "--branch", BRANCH, "--limit", "100",
                      "--json", "databaseId,displayTitle,url", capture_output=True)
        if response.returncode:
            sys.stderr.write(response.stderr)
            return response.returncode
        for run in json.loads(response.stdout):
            if request in run["displayTitle"]:
                print(json.dumps({"run_id": run["databaseId"], "url": run["url"],
                                  "commit": sha, "suite": args.command, "request_id": request}), flush=True)
                if args.wait:
                    return gh("run", "watch", str(run["databaseId"]), "--exit-status").returncode
                return 0
        time.sleep(2)
    sys.stderr.write(f"Submission accepted, but run ID lookup timed out. Request: {request}\n")
    return 1


def gh(*arguments, **options):
    environment = os.environ.copy()
    state = Path("/c/Users/benbw/Documents/GitRepo/bcc-rust/.git/windows-runner-state")
    if os.name == "posix" and state.is_dir():
        # Logs/artifact downloads remain usable when the sandbox overlay is full.
        for variable, directory in (("TMPDIR", "tmp"), ("XDG_CACHE_HOME", "cache")):
            location = state / directory
            location.mkdir(exist_ok=True)
            environment[variable] = str(location)
    environment["GH_NO_UPDATE_NOTIFIER"] = "1"
    return subprocess.run(["gh", *arguments, "--repo", REPOSITORY] if arguments[0] != "api"
                          else ["gh", *arguments], text=True, env=environment, **options)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
