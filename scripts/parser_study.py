#!/usr/bin/env python3
"""Run isolated, interleaved parser timings and AST/diagnostic comparisons."""

import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
import time


def parse_metrics(output):
    rows = []
    for line in output.splitlines():
        if line.startswith("sample="):
            rows.append({key: int(value) for key, value in (part.split("=", 1) for part in line.split())})
    return rows


def main():
    # Overflow is an expected result of explicit small-stack probes. Do not
    # retain core images containing the compiler's large reserved address space.
    try:
        import resource
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    except ImportError:
        pass
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["bench", "diff", "stack"])
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--inputs", type=Path, default=Path(".scratch/parser-study/inputs"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=7)
    parser.add_argument("--modes", nargs="+", default=["machine", "recursive", "growing", "hybrid:16", "limited:256"])
    parser.add_argument("--files", nargs="+")
    parser.add_argument("--stack", type=int)
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    scratch = args.output.parent / (args.output.stem + "-inspections")
    if args.action == "diff":
        scratch.mkdir(exist_ok=True)
    if args.files:
        files = [args.inputs / name for name in args.files]
    elif args.action == "bench":
        files = [args.inputs / name for name in ["lua-supported.i", "expression-mix.c", "statement-mix.c", "declaration-mix.c", "bytecode.i", "mandel.i", "parentheses-256.c", "unary-256.c", "assignment-256.c", "conditional-256.c", "statements-256.c", "blocks-256.c", "declarator-256.c", "initializer-256.c", "chain-10000.c", "initializer-list-10000.c"]]
    elif args.action == "diff":
        files = sorted(args.inputs.glob("*.i")) + [args.inputs / name for name in ["provenance.c", "extensions.c", "precedence.c", "malformed.c"]]
        files += [args.inputs / f"{kind}-{depth}.c" for depth in [32, 127, 256] for kind in ["parentheses", "unary", "assignment", "conditional", "statements", "blocks", "declarator", "initializer"]]
        files += [args.inputs / "chain-1000.c", args.inputs / "initializer-list-1000.c"]
    else:
        files = [args.inputs / f"{kind}-{depth}.c" for depth in [32, 127, 256, 1024, 4096, 16384] for kind in ["parentheses", "unary", "assignment", "conditional", "statements", "blocks", "declarator", "initializer"]]

    rows = []
    metadata = {"binary": str(args.binary), "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(), "action": args.action, "repeats": args.repeats, "modes": args.modes, "stack_bytes": args.stack}

    def save():
        args.output.write_text(json.dumps({"metadata": metadata, "rows": rows}, indent=2) + "\n")

    def execute(source, mode, repeat):
        command = [str(args.binary), str(source), "1", "--mode", mode]
        if args.stack:
            command += ["--stack", str(args.stack)]
        if args.action == "stack":
            command += ["--sample-stack"]
        destination = scratch / (source.name + "." + mode.replace(":", "-") + ".ast")
        if args.action == "diff":
            command += ["--inspect", str(destination)]
            destination.unlink(missing_ok=True)
        before = time.monotonic()
        try:
            completed = subprocess.run(command, capture_output=True, text=True, timeout=60)
            row = {"input": source.name, "mode": mode, "repeat": repeat, "returncode": completed.returncode, "elapsed_seconds": time.monotonic() - before, "metrics": parse_metrics(completed.stdout)}
            if completed.stderr:
                row["stderr"] = completed.stderr[:4000]
            if args.action == "diff" and completed.returncode == 0:
                row["inspection_sha256"] = hashlib.sha256(destination.read_bytes()).hexdigest()
                baseline = scratch / (source.name + ".machine.ast")
                row["identical"] = destination.read_bytes() == baseline.read_bytes() if baseline.exists() else None
                if row["identical"] is None:
                    row["comparison_error"] = "machine inspection unavailable"
                elif not row["identical"]:
                    # Bounded evidence: inspection can contain large, deeply
                    # indented trees. Retain full files in scratch for review.
                    with baseline.open() as left, destination.open() as right:
                        for line_number, (a, b) in enumerate(zip(left, right), 1):
                            if a != b:
                                row["first_difference"] = {"line": line_number, "machine": a[:500].rstrip(), "candidate": b[:500].rstrip()}
                                break
                # Successful candidate output is redundant with machine output.
                if row["identical"] and mode != "machine":
                    destination.unlink()
            print(f"{source.name} {mode} #{repeat}: rc={completed.returncode} " + (f"identical={row.get('identical')} " if args.action == "diff" else "") + str(row["metrics"]), flush=True)
            return row
        except subprocess.TimeoutExpired:
            return {"input": source.name, "mode": mode, "repeat": repeat, "returncode": "timeout"}

    for source in files:
        rounds = args.repeats if args.action == "bench" else 1
        modes = args.modes
        if args.action == "diff":
            modes = ["machine"] + [mode for mode in modes if mode != "machine"]
        for repeat in range(rounds):
            offset = repeat % len(modes) if args.action == "bench" else 0
            for mode in modes[offset:] + modes[:offset]:
                rows.append(execute(source, mode, repeat))
                save()
    if args.action == "bench":
        print("input mode n minimum_ms median_ms cv_percent", flush=True)
        for source in files:
            for mode in args.modes:
                values = [row["metrics"][0]["ns"] / 1e6 for row in rows if row["input"] == source.name and row["mode"] == mode and row["returncode"] == 0 and row["metrics"]]
                if values:
                    cv = statistics.pstdev(values) / statistics.mean(values) * 100
                    print(source.name, mode, len(values), f"{min(values):.4f}", f"{statistics.median(values):.4f}", f"{cv:.2f}", flush=True)


if __name__ == "__main__":
    main()
