#!/usr/bin/env python3
"""Summarize the downloaded native Windows parser cohort without rerunning it."""

import argparse
import json
from pathlib import Path
import re
import statistics


def fields(line):
    return {key: int(value) if value.isdecimal() else value.replace("\\", "/")
            for key, value in re.findall(r"(\w+)=([^\s]+)", line)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path,
                        default=Path(".scratch/parser-study/windows/native/runs.json"))
    parser.add_argument("--output", type=Path,
                        default=Path(".scratch/parser-study/windows/native/summary.json"))
    args = parser.parse_args()
    manifest = json.loads(args.input.read_text())
    summary = {"sha": manifest["sha"], "jobs": []}
    for job in manifest["jobs"]:
        if not job.get("downloaded"):
            continue
        root = args.input.parent / str(job["run_id"])
        logs = list(root.rglob("tests.log"))
        if len(logs) != 1:
            raise ValueError(f"expected one tests.log in {root}")
        text = logs[0].read_text(encoding="utf-8-sig")
        result = {"name": job["name"], "run_id": job["run_id"], "url": job["url"],
                  "conclusion": job["workflow"]["conclusion"], "identity": job["result"],
                  "test_results": re.findall(r"test result:.*", text)}
        if job["name"].startswith("parser-"):
            result.update(timing=[], stacks=[], parity_checks=0)
            pending = None
            for line in text.splitlines():
                if "runner_parser kind=parity identical=1" in line:
                    result["parity_checks"] += 1
                if "runner_parser kind=timing " in line or "runner_parser kind=stack " in line:
                    pending = fields(line)
                    if pending["kind"] == "stack":
                        pending["fixture_depth"] = pending["depth"]
                elif line.startswith("sample=") and pending:
                    record = pending | fields(line)
                    result["timing" if pending["kind"] == "timing" else "stacks"].append(record)
                    pending = None
            result["aggregates"] = []
            groups = sorted({(row["input"], row["mode"]) for row in result["timing"]})
            for name, mode in groups:
                rows = [row for row in result["timing"] if (row["input"], row["mode"]) == (name, mode)]
                times = [row["ns"] for row in rows]
                result["aggregates"].append({
                    "input": name, "mode": mode, "samples": len(rows),
                    "median_ns": statistics.median(times), "min_ns": min(times), "max_ns": max(times),
                    "population_cv_percent": 100 * statistics.pstdev(times) / statistics.mean(times),
                    "tokens": sorted({row["tokens"] for row in rows}),
                    "diagnostics": sorted({row["diagnostics"] for row in rows}),
                    "global_calls": sorted({row["global_calls"] for row in rows}),
                    "global_bytes": sorted({row["global_bytes"] for row in rows}),
                    "parse_bytes": sorted({row["parse_bytes"] for row in rows}),
                    "tu_bytes": sorted({row["tu_bytes"] for row in rows}),
                })
            result["stack_slopes"] = {}
            for name in ("parentheses", "blocks"):
                rows = [row for row in result["stacks"] if row["input"] == name]
                if len(rows) == 2:
                    rows.sort(key=lambda row: row["fixture_depth"])
                    result["stack_slopes"][name] = (
                        (rows[1]["stack_span"] - rows[0]["stack_span"])
                        / (rows[1]["fixture_depth"] - rows[0]["fixture_depth"]))
        elif job["name"].startswith("stack-"):
            result["probes"] = [fields(line) for line in text.splitlines()
                                if "runner_stack reserve=" in line]
            result["overflows"] = [line[line.index("runner_stack_overflow "):]
                                   for line in text.splitlines() if "runner_stack_overflow " in line]
        summary["jobs"].append(result)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(summary, indent=2) + "\n")
    for job in summary["jobs"]:
        print(job["name"], job["conclusion"], *job["test_results"])


if __name__ == "__main__":
    main()
