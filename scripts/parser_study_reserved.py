#!/usr/bin/env python3
"""Collect and archive the demand-paged native-stack follow-up (no Cargo calls)."""

import argparse
import csv
import hashlib
import json
from pathlib import Path
import platform
import statistics
import subprocess
import sys


def prepare(scratch):
    inputs = scratch / "inputs"
    depth = 65536
    sources = {
        "parentheses": "int f(void){return " + "(" * depth + "1" + ")" * depth + ";}\n",
        "unary": "int f(void){return " + "! " * depth + "1;}\n",
        "assignment": "int f(int a){return " + "a=" * depth + "1;}\n",
        "conditional": "int f(int a){return " + "a?1:" * depth + "2;}\n",
        "statements": "int f(int a){" + "if(a)" * depth + "return 1; return 0;}\n",
        "blocks": "int f(void){" + "{" * depth + "int x;" + "}" * depth + "return 0;}\n",
        "declarator": "int " + "(" * depth + "x" + ")" * depth + ";\n",
        "initializer": "int x = " + "{" * depth + "1" + "}" * depth + ";\n",
    }
    for family, source in sources.items():
        (inputs / f"{family}-{depth}.c").write_text(source)
    manifest = {
        path.name: {"bytes": path.stat().st_size, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        for path in sorted(inputs.iterdir()) if path.suffix in [".c", ".i"]
    }
    (scratch / "reserved-inputs.json").write_text(json.dumps(manifest, indent=2) + "\n")
    host = {
        "platform": platform.platform(),
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
    }
    for key, path in [
        ("overcommit_memory", "/proc/sys/vm/overcommit_memory"),
        ("overcommit_ratio", "/proc/sys/vm/overcommit_ratio"),
        ("meminfo", "/proc/meminfo"),
        ("cpuinfo", "/proc/cpuinfo"),
    ]:
        if Path(path).exists():
            text = Path(path).read_text()
            host[key] = text.split("\n\n", 1)[0] if key == "cpuinfo" else text.strip()
    (scratch / "reserved-environment.json").write_text(json.dumps(host, indent=2) + "\n")


def collect(args):
    scratch = args.scratch
    profile = args.profile
    binary = scratch / f"reserved-{profile}"
    baseline = scratch / f"reserved-baseline-{profile}"
    if not binary.is_file() or not baseline.is_file():
        raise SystemExit("build and copy both study and baseline binaries first")

    def batch(name, action, modes, files=None, extra=(), repeats=7):
        command = [sys.executable, "scripts/parser_study.py", action,
                   "--binary", str(binary), "--inputs", str(scratch / "inputs"),
                   "--output", str(scratch / f"reserved-{profile}-{name}.json"),
                   "--repeats", str(repeats), "--modes", *modes]
        if "baseline" in modes:
            command += ["--baseline-binary", str(baseline)]
        if files:
            command += ["--files", *files]
        command += list(extra)
        print("Running " + " ".join(command), flush=True)
        subprocess.run(command, check=True)

    batch("timing", "bench", ["baseline", "machine", "recursive", "growing", "hybrid:16",
                               "reserved:machine", "reserved:recursive"],
          ["lua-supported.i", "expression-mix.c", "statement-mix.c", "declaration-mix.c"], repeats=9)
    batch("stdlib-100g", "bench", ["recursive"], ["lua-supported.i"],
          extra=["--stack", str(100 * 1024**3)], repeats=3)
    deep = ["parentheses-16384.c", "blocks-16384.c", "parentheses-65536.c", "blocks-65536.c",
            "statements-65536.c", "declarator-65536.c", "initializer-65536.c"]
    batch("deep", "bench", ["machine", "growing", "hybrid:16", "reserved:recursive"], deep,
          extra=["--stack", "1048576"])
    batch("memory", "bench", ["reserved:machine", "reserved:recursive"],
          ["lua-supported.i", "parentheses-16384.c", "blocks-16384.c", "blocks-65536.c"],
          extra=["--stack-memory"], repeats=3)
    families = ["parentheses", "unary", "assignment", "conditional", "statements", "blocks", "declarator", "initializer"]
    grid = [f"{family}-{depth}.c" for depth in [32, 127, 256, 1024, 4096, 16384, 65536] for family in families]
    batch("grid", "stack", ["reserved:recursive"], grid)
    batch("diff", "diff", ["machine", "recursive", "reserved:machine", "reserved:recursive"])
    batch("grammar", "diff", ["machine", "recursive", "reserved:machine", "reserved:recursive"],
          extra=["--inputs", str(scratch / "grammar"), "--manifest", str(scratch / "grammar/manifest.json")])
    # These deliberately include exhausted reservations. The runner records
    # subprocess signals instead of treating a collection exit as success.
    for size in [1024**2, 64 * 1024**2, 1024**3, 100 * 1024**3]:
        batch(f"size-{size}", "bench", ["reserved:recursive"], ["lua-supported.i", "blocks-16384.c"],
              extra=["--reserve-stack", str(size), "--stack-memory"], repeats=3)


def archive(args):
    output = args.output
    output.mkdir(parents=True, exist_ok=True)
    tables = []
    checks = []
    for path in sorted(args.scratch.glob("reserved-*.json")):
        data = json.loads(path.read_text())
        (output / path.name).write_text(json.dumps(data, separators=(",", ":")) + "\n")
        if "rows" not in data:
            continue
        groups = {}
        for row in data["rows"]:
            groups.setdefault((row["input"], row["mode"]), []).append(row)
        if data["metadata"]["action"] == "diff":
            for source in sorted({row["input"] for row in data["rows"]}):
                by_mode = {row["mode"]: row for row in data["rows"] if row["input"] == source}
                for mode in ["machine", "recursive"]:
                    left, right = by_mode[mode], by_mode["reserved:" + mode]
                    checks.append({"run": path.stem, "input": source, "mode": mode,
                                   "identical": left["returncode"] == right["returncode"] == 0
                                   and left.get("inspection_sha256") == right.get("inspection_sha256")})
        for (source, mode), rows in groups.items():
            good = [row for row in rows if row["returncode"] == 0 and row.get("metrics")]
            if not good:
                tables.append({"run": path.stem, "input": source, "mode": mode, "n": 0, "failed": len(rows)})
                continue
            values = [row["metrics"][0]["ns"] / 1e6 for row in good]
            med = lambda key: int(statistics.median(row["metrics"][0].get(key, 0) for row in good))
            summary = {
                "run": path.stem, "input": source, "mode": mode,
                "n": len(good), "failed": len(rows) - len(good),
                "minimum_ms": min(values), "median_ms": statistics.median(values),
                "cv_percent": statistics.pstdev(values) / statistics.mean(values) * 100,
                **{key: med(key) for key in ["rss_after", "parse_bytes", "tu_bytes", "commit_bytes", "global_calls",
                                           "global_bytes", "depth", "stack_span", "fallbacks", "minor_faults", "major_faults",
                                           "stack_reserved", "stack_rss_before", "stack_rss_after", "stack_swap_after",
                                           "stack_writable", "stack_noreserve", "stack_sampled", "stack_huge_bytes"]},
                "launch_median_ns": int(statistics.median(row.get("thread_launch_ns", 0) for row in good)),
            }
            tables.append(summary)
    keys = list(dict.fromkeys(key for row in tables for key in row))
    with (output / "summary.tsv").open("w") as stream:
        writer = csv.DictWriter(stream, fieldnames=keys, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        writer.writerows(tables)
    (output / "transport-identity.json").write_text(json.dumps(checks, indent=2) + "\n")
    validation = {}
    for path in sorted(args.scratch.glob("reserved-*.log")):
        # Keep validation logs and failure evidence, not verbose repeated
        # collection stdout already represented in raw observation JSON.
        if "build" in path.name or "checks" in path.name or "verification" in path.name:
            text = path.read_text()
            (output / path.name).write_text("\n".join(line.rstrip() for line in text.splitlines()).rstrip() + "\n")
            validation[path.name] = {
                "raw_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "complete": text.rstrip().endswith("EXIT 0"),
                "test_results": [line for line in text.splitlines() if line.startswith("test result:")],
            }
    (output / "validation.json").write_text(json.dumps(validation, indent=2) + "\n")
    print(f"Archived {len(tables)} groups, {sum(row['identical'] for row in checks)}/{len(checks)} transport identity checks")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "collect", "archive"])
    parser.add_argument("--scratch", type=Path, default=Path(".scratch/parser-study"))
    parser.add_argument("--profile", choices=["release", "debug"], default="release")
    parser.add_argument("--output", type=Path, default=Path("docs/research/parser-reserved-stack-data"))
    args = parser.parse_args()
    if args.action == "prepare":
        prepare(args.scratch)
    elif args.action == "collect":
        collect(args)
    else:
        archive(args)


if __name__ == "__main__":
    main()
