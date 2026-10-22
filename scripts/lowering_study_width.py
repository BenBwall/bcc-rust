#!/usr/bin/env python3
"""Fresh-process measurements of wide lowering, using already copied binaries."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path(".scratch/lowering-study/width"))
    parser.add_argument("--statements", type=int, default=50000)
    parser.add_argument("--repeats", type=int, default=9)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    source = args.output / "wide.c"
    source.write_text("int f(int a){\n" + "a+=1;\n" * args.statements + "return a;}\n")
    result = {"statements": args.statements,
              "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(), "samples": []}
    destination = args.output / "results.json"
    for profile in ("release", "debug"):
        binary = Path(f".scratch/lowering-study/study-{profile}")
        result[f"{profile}_binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
        for repeat in range(args.repeats):
            for offset in range(2):
                mode = ("machine", "recursive")[(repeat+offset) % 2]
                command = ["/usr/bin/time", "-f", "rss_kib=%M", str(binary), str(source), mode]
                run = subprocess.run(command, capture_output=True, text=True, check=True)
                record = {key: int(value) if value.isdecimal() else value
                          for key, value in re.findall(r"(\w+)=([^\s]+)", run.stdout.splitlines()[0])}
                record.update(profile=profile, repeat=repeat,
                              rss_kib=int(re.search(r"rss_kib=(\d+)", run.stderr)[1]))
                assert record["errors"] == record["diagnostics"] == 0
                result["samples"].append(record)
                destination.write_text(json.dumps(result, indent=2) + "\n")
                print(profile, repeat, mode, record["ns"], record["scratch_bytes"], flush=True)


if __name__ == "__main__":
    main()
