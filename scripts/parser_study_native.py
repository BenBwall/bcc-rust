#!/usr/bin/env python3
"""Submit/resume serial parser study jobs on the configured Windows Actions runner.

Requires explicit authorization to publish the source commit and dispatch jobs.
Never invokes Cargo locally, changes the dispatcher, or publishes a branch.
"""

import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
REPOSITORY = "BenBwall/bcc-rust"
FEATURE = "parser-recursive-descent-study"


def jobs():
    for profile in ("debug", "release"):
        release = ["--release"] if profile == "release" else []
        yield f"recovery-{profile}", release + [
            "--features", FEATURE, "--lib", "recursive_descent", "--", "--test-threads=1"]
        yield f"stack-{profile}", release + [
            "--example", "parser_stack_probe", "runner_stack_measurements", "--",
            "--ignored", "--nocapture", "--test-threads=1"]
        yield f"parser-{profile}", release + [
            "--features", FEATURE, "--example", "parser_study", "runner_parser_measurements",
            "--", "--ignored", "--nocapture", "--test-threads=1"]
    for profile in ("debug", "release"):
        release = ["--release"] if profile == "release" else []
        yield f"worker-{profile}", release + [
            "--example", "parser_stack_probe", "reserved_stack::tests::", "--", "--test-threads=1"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sha", required=True, help="exact already-pushed source SHA")
    parser.add_argument("--adopt-first", type=int, help="existing recovery-debug run ID")
    parser.add_argument("--output", type=Path,
                        default=ROOT / ".scratch/parser-study/windows/native")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    manifest = args.output / "runs.json"
    data = json.loads(manifest.read_text()) if manifest.exists() else {"sha": args.sha, "jobs": []}
    if data["sha"] != args.sha:
        raise SystemExit("output directory belongs to another source SHA")

    def save():
        manifest.write_text(json.dumps(data, indent=2) + "\n")

    for name, arguments in jobs():
        row = next((row for row in data["jobs"] if row["name"] == name), None)
        command = ["bcc-windows", "test", "--sha", args.sha, "--", *arguments]
        if row is None:
            if name == "recovery-debug" and args.adopt_first:
                submitted = {"run_id": args.adopt_first,
                             "url": f"https://github.com/{REPOSITORY}/actions/runs/{args.adopt_first}"}
            else:
                result = subprocess.run(command, cwd=ROOT, check=True, capture_output=True, text=True)
                submitted = json.loads(result.stdout.splitlines()[-1])
            row = {"name": name, "command": command, **submitted}
            data["jobs"].append(row)
            save()  # A resumed run must never submit a duplicate job.
        if row.get("downloaded"):
            continue
        run_id = str(row["run_id"])
        print(name, run_id, row["url"], flush=True)
        log = args.output / f"{name}-watch.log"
        with log.open("w") as stream:
            watched = subprocess.run(["bcc-windows", "wait", run_id], cwd=ROOT,
                                     stdout=stream, stderr=subprocess.STDOUT, check=False)
        status = subprocess.check_output([
            "gh", "run", "view", run_id, "--repo", REPOSITORY,
            "--json", "status,conclusion,jobs,url"], cwd=ROOT, text=True)
        row["workflow"] = json.loads(status)
        row["watch_exit"] = watched.returncode
        save()
        if row["workflow"]["status"] != "completed":
            raise SystemExit("watch ended before completion; resume using the same output directory")
        destination = args.output / run_id
        subprocess.run(["bcc-windows", "download", run_id, "--dir", str(destination)],
                       cwd=ROOT, check=True)
        results = list(destination.rglob("results.json"))
        if len(results) != 1:
            raise SystemExit(f"expected one runner result in {destination}")
        evidence = json.loads(results[0].read_text(encoding="utf-8-sig"))
        if evidence["commit"] != args.sha:
            raise SystemExit("runner artifact source SHA differs from requested SHA")
        row["result"] = evidence
        row["downloaded"] = True
        save()
        print(name, row["workflow"]["conclusion"], flush=True)
        workload = next((step for job in row["workflow"]["jobs"] for step in job["steps"]
                         if step["name"] == "Run requested Windows workload"), None)
        if not workload or workload["conclusion"] in ("skipped", "cancelled"):
            raise SystemExit("runner setup did not reach the workload; inspect artifacts before resuming")
    if any(row["workflow"]["conclusion"] != "success" for row in data["jobs"]):
        raise SystemExit("one or more native jobs failed; all artifacts were retained")


if __name__ == "__main__":
    main()
