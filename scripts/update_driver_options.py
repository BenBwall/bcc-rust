"""Freeze the option names the GCC and Clang driver personalities accept.

The personalities accept, and ignore, the warning, code-generation and
machine options their compiler knows but bcc does not implement, and diagnose
the rest as that compiler does. GCC's names come from a GCC on PATH
(`gcc --help=...`); Clang's from the repository's pinned Clang
(`clang --autocomplete=...`). Run with --check to compare the frozen lists.
"""
import argparse
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "src" / "cli" / "personality" / "options"
CLANG = ROOT / "target" / "llvm" / "bin" / ("clang.exe" if sys.platform == "win32" else "clang")
OPTION = re.compile(r"^  (-[^\s,<]+)")


def gcc_options(gcc):
    names = set()
    for category in ("warnings", "common", "optimizers", "c", "target", "undocumented"):
        result = subprocess.run([gcc, f"--help={category}"], capture_output=True, text=True, check=True)
        for line in result.stdout.splitlines():
            match = OPTION.match(line)
            if match:
                names.add(match.group(1))
    return names


def clang_options():
    names = set()
    for prefix in ("-W", "-f", "-m"):
        result = subprocess.run([str(CLANG), f"--autocomplete={prefix}"], capture_output=True, text=True, check=True)
        for line in result.stdout.splitlines():
            name = line.split("\t", 1)[0].strip()
            if name.startswith(prefix) and len(name) > 2:
                names.add(name)
    return names


def render(names, prefix, source):
    selected = sorted(name for name in names if name.startswith(prefix) and len(name) > len(prefix))
    return f"# {source}; regenerate with scripts/update_driver_options.py\n" + "".join(f"{name}\n" for name in selected)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="compare instead of writing")
    parser.add_argument("--gcc", default="gcc", help="the GCC whose options to freeze")
    args = parser.parse_args()
    version = subprocess.run([args.gcc, "-dumpfullversion"], capture_output=True, text=True, check=True).stdout.strip()
    gcc = gcc_options(args.gcc)
    clang = clang_options()
    files = {}
    for prefix, kind in (("-W", "warnings"), ("-f", "flags"), ("-m", "machine")):
        files[f"gcc-{kind}.list"] = render(gcc, prefix, f"GCC {version} --help")
        files[f"clang-{kind}.list"] = render(clang, prefix, "pinned Clang --autocomplete")
    stale = []
    OUT.mkdir(parents=True, exist_ok=True)
    for name, text in files.items():
        path = OUT / name
        if args.check:
            old = path.read_text() if path.exists() else ""
            # The GCC version line may differ between hosts; compare names.
            if old.splitlines()[1:] != text.splitlines()[1:]:
                stale.append(name)
        else:
            path.write_text(text, newline="\n")
    if stale:
        print("stale:", ", ".join(stale))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
