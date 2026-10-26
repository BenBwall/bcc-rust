#!/usr/bin/env python3
"""Summarize lowering study log records without rerunning the measurements."""

import argparse
import json
from pathlib import Path
import re
import statistics


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("logs", nargs="+", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    rows = {kind: [] for kind in ("timing", "parity", "stack")}
    for path in args.logs:
        for line in path.read_text(encoding="utf-8-sig").splitlines():
            match = re.search(r"lowering_(timing|parity|stack) (.*)", line)
            if not match:
                continue
            record = dict(re.findall(r"(\w+)=([^\s]+)", match[2]))
            record = {key: int(value) if value.isdecimal() else value.replace("\\", "/")
                      for key, value in record.items()}
            record["log"] = str(path)
            rows[match[1]].append(record)
    aggregates = []
    for profile, name, recursive in sorted({
            (row["profile"], row["input"], row["recursive"]) for row in rows["timing"]}):
        samples = [row for row in rows["timing"] if
                   (row["profile"], row["input"], row["recursive"]) == (profile, name, recursive)]
        times = [row["ns"] for row in samples]
        totals = [row["total_ns"] for row in samples]
        aggregates.append({
            "profile": profile, "input": name, "recursive": recursive,
            "samples": len(samples), "median_ns": statistics.median(times),
            "min_ns": min(times), "max_ns": max(times),
            "cv_percent": 100 * statistics.pstdev(times) / statistics.mean(times),
            "total_median_ns": statistics.median(totals),
            "scratch_bytes": sorted({row["scratch_bytes"] for row in samples}),
            "ir_bytes": sorted({row["ir_bytes"] for row in samples}),
        })
    result = rows | {"aggregates": aggregates}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(f"{len(rows['timing'])} timings, {len(rows['parity'])} parity cases, {len(rows['stack'])} deep probes")
    for row in aggregates:
        if row["input"] in ("expressions", "statements"):
            print(row)


if __name__ == "__main__":
    main()
