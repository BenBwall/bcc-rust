#!/usr/bin/env python3
"""Collect phase-7 Callgrind models, explicitly excluding input preparation."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def main():
    # A profiler may reject a CPU instruction. Do not dump the compiler's
    # large reserved address space when the tool raises SIGILL.
    try:
        import resource
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    except ImportError:
        pass
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--inputs", type=Path, default=Path(".scratch/parser-study/inputs"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--modes", nargs="+", default=["machine", "recursive", "growing", "hybrid:16"])
    parser.add_argument("--files", nargs="+", default=["lua-supported.i", "expression-mix.c", "statement-mix.c"])
    parser.add_argument("--repeats", type=int, default=1)
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    directory = args.output.parent / args.output.stem
    directory.mkdir(exist_ok=True)
    report = {
        "model": "Callgrind simulated instructions, cache and branch events; no hardware cycles",
        "binary": str(args.binary),
        "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        "rows": [],
    }
    for name in args.files:
        source = args.inputs / name
        for mode in args.modes:
            for repeat in range(args.repeats):
                stem = f"{name}.{mode.replace(':', '-')}.{repeat}"
                output = directory / (stem + ".callgrind")
                command = [
                    "valgrind", "--tool=callgrind", "--collect-atstart=no",
                    "--toggle-collect=*parser_study_phase7*",
                    "--cache-sim=yes", "--branch-sim=yes",
                    "--callgrind-out-file=" + str(output),
                    str(args.binary), str(source), "1", "--mode", mode,
                ]
                with (directory / (stem + ".log")).open("w") as log:
                    completed = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
                row = {"input": name, "mode": mode, "repeat": repeat, "command": command, "returncode": completed.returncode}
                if completed.returncode == 0:
                    lines = output.read_text().splitlines()
                    events = next(line.split()[1:] for line in lines if line.startswith("events:"))
                    totals = next(line.split()[1:] for line in reversed(lines) if line.startswith("totals:"))
                    row["events"] = dict(zip(events, map(int, totals)))
                    assert row["events"].get("Ir", 0) > 0, "empty phase-7 collection"
                    with (directory / (stem + ".self.txt")).open("w") as annotation:
                        subprocess.run(["callgrind_annotate", "--auto=no", "--inclusive=no", "--threshold=95", str(output)], stdout=annotation, check=True)
                    with (directory / (stem + ".inclusive.txt")).open("w") as annotation:
                        subprocess.run(["callgrind_annotate", "--auto=no", "--inclusive=yes", "--threshold=95", str(output)], stdout=annotation, check=True)
                report["rows"].append(row)
                args.output.write_text(json.dumps(report, indent=2) + "\n")
                print(stem, row.get("events", completed.returncode), flush=True)


if __name__ == "__main__":
    main()
