#!/usr/bin/env python3
"""Archive compact raw study observations and readable timing/profile tables."""

import argparse
import csv
import hashlib
import json
from pathlib import Path
import re
import shutil
import statistics


def call_counts(path):
    lines = path.read_text().splitlines()
    names = {}
    for line in lines:
        match = re.match(r"c?fn=\((\d+)\) (.*)", line)
        if match:
            names[match[1]] = match[2]
    fragments = ["ParseFrame>::step_once", "retained_node_count", "reclaim_pooled", "push_frame", "store_expression", "merge_vectors", "VecPool"]
    counts = dict.fromkeys(fragments, 0)
    current = ""
    for line in lines:
        match = re.match(r"cfn=\((\d+)\)", line)
        if match:
            current = names[match[1]]
        if line.startswith("calls="):
            for fragment in fragments:
                if fragment in current:
                    counts[fragment] += int(line.split()[0][6:])
    return counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scratch", type=Path, default=Path(".scratch/parser-study"))
    parser.add_argument("--output", type=Path, default=Path("docs/research/parser-recursive-descent-data"))
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    for name, path in [
        ("environment.json", args.scratch / "environment.json"),
        ("inputs.json", args.scratch / "inputs/manifest.json"),
        ("grammar.json", args.scratch / "grammar/manifest.json"),
    ]:
        shutil.copyfile(path, args.output / name)
    timing = []
    events = []
    calls = []
    for path in sorted(args.scratch.glob("final-*.json")):
        data = json.loads(path.read_text())
        groups = {}
        for row in data["rows"]:
            groups.setdefault((row["input"], row["mode"]), []).append(row)
            if "events" in row:
                events.append({"run": path.stem, "input": row["input"], "mode": row["mode"], "repeat": row["repeat"], **row["events"]})
                raw = next((item.split("=", 1)[1] for item in row["command"] if item.startswith("--callgrind-out-file=")), None)
                if raw and Path(raw).exists():
                    row["calls"] = call_counts(Path(raw))
                    calls.append({"run": path.stem, "input": row["input"], "mode": row["mode"], "repeat": row["repeat"], **row["calls"]})
                    if row["input"] == "lua-supported.i" and row["mode"] in ["machine", "recursive"] and row["repeat"] == 0:
                        annotation = Path(raw).with_suffix(".self.txt")
                        if annotation.exists():
                            shutil.copyfile(annotation, args.output / (path.stem + "-" + annotation.name))
        (args.output / path.name).write_text(json.dumps(data, separators=(",", ":")) + "\n")
        if data.get("metadata", {}).get("action") != "bench":
            continue
        for (name, mode), rows in groups.items():
            good = [row for row in rows if row["returncode"] == 0 and row["metrics"]]
            if not good:
                continue
            values = [row["metrics"][0]["ns"] / 1e6 for row in good]
            metric = good[0]["metrics"][0]
            timing.append({
                "run": path.stem, "input": name, "mode": mode,
                "n": len(good), "failed": len(rows) - len(good),
                "minimum_ms": f"{min(values):.6f}",
                "median_ms": f"{statistics.median(values):.6f}",
                "maximum_ms": f"{max(values):.6f}",
                "cv_percent": f"{statistics.pstdev(values) / statistics.mean(values) * 100:.3f}",
                **{key: metric[key] for key in ["tokens", "diagnostics", "parse_bytes", "tu_bytes", "commit_bytes", "global_calls", "global_bytes", "depth", "fallbacks"]},
                "median_rss_bytes": int(statistics.median(row["metrics"][0]["rss_after"] for row in good)),
            })
    for name, rows in [("timing.tsv", timing), ("events.tsv", events), ("calls.tsv", calls)]:
        if rows:
            with (args.output / name).open("w") as output:
                writer = csv.DictWriter(output, fieldnames=list(rows[0]), delimiter="\t", lineterminator="\n")
                writer.writeheader()
                writer.writerows(rows)
    print(f"Archived {len(timing)} timing groups and {len(events)} simulated counter samples")
    build_log = args.scratch / "build-final-bundle-3.log"
    if build_log.exists():
        text = build_log.read_text()
        validation = {
            "build_log_sha256": hashlib.sha256(build_log.read_bytes()).hexdigest(),
            "complete": text.rstrip().endswith("EXIT 0"),
            "test_results": [line for line in text.splitlines() if line.startswith("test result:")],
            "checks_revision": (args.scratch / "checks-revision").read_text().strip() if (args.scratch / "checks-revision").exists() else None,
        }
        (args.output / "validation.json").write_text(json.dumps(validation, indent=2) + "\n")


if __name__ == "__main__":
    main()
