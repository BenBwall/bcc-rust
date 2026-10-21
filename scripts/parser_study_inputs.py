#!/usr/bin/env python3
"""Prepare reproducible real and synthetic phase-7 inputs inside local scratch."""

import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile
import urllib.request
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path(".scratch/parser-study/inputs"))
    parser.add_argument("--sqlite", action="store_true", help="download SQLite 3.50.4 amalgamation")
    parser.add_argument("--lua", action="store_true", help="download and preprocess Lua v5.4.8 onelua.c")
    parser.add_argument("--cc", default="gcc")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    manifest = {}

    def save(name, source, origin):
        path = args.output / name
        if isinstance(source, str):
            source = source.encode()
        path.write_bytes(source)
        manifest[name] = {"bytes": len(source), "sha256": hashlib.sha256(source).hexdigest(), "origin": origin}

    for path in sorted(Path("benches/c").glob("*.c")):
        command = [args.cc, "-std=gnu99", "-E", "-P", str(path)]
        result = subprocess.run(command, check=True, capture_output=True)
        save(path.stem + ".i", result.stdout, " ".join(command))

    # Preserve macro/include provenance in a separate differential fixture.
    save("provenance.c", "#define INC(x) ((x)+1)\n#include \"stddef.h\"\nint f(int x){return INC(x);}\n", "generated")
    save("extensions.c", """typedef int T;
struct S { int x; int y; };
int f(int x, int *p) {
  int a = (T)x + sizeof(T) + sizeof x + _Alignof(T);
  struct S s = (struct S){ .y = 2, .x = 1 };
  a += __extension__ ({ int v = 3; v + x; });
  if (a) { a = p[1]++; } else a = s.x;
  do { a--; } while (a > 4);
  for (int i=0; i<2; ++i) { if(i) continue; a += i; }
  while (x--) { switch(x) { case 0: break; default: a ^= x; } }
  goto end; end: return a ? p[0] : s.y;
}
""", "generated grammar differential fixture")
    save("precedence.c", """int f(int a, int b, int c, int *p) {
return (a += b = c), a ? b, c : a ? b : c, a || b && c | a ^ b & c == a < b << c + a * b / c % 2,
       ++*p + --a + a++ + b-- + !a + ~b + -c + +a;
}
""", "generated grammar differential fixture")
    save("malformed.c", "int f(void){ int a = ; if(1) return (2+); else return 3; } int following;\n", "generated recovery fixture")
    expressions = []
    declarations = []
    statements = []
    for index in range(4000):
        expressions.append(f"int expr{index}(int a,int b,int *p){{return (a+=b*3), (a<<2)+p[a&3] > b ? (int)(a||b&&p[0]) : (a^b)|(~b&7);}}\n")
        declarations.append(f"struct S{index}{{int a[4];unsigned b:3;}};typedef int (*F{index})(int);int x{index}[4]={{[2]=sizeof(struct S{index}),[0]=1}};\n")
        statements.append(f"int stmt{index}(int a){{int n=0;for(int i=0;i<a;++i){{if(i&1)continue;n+=i;}}while(n>100)n-=3;do{{n++;}}while(n<0);switch(a){{case 0:return n;default:break;}}return n;}}\n")
    save("expression-mix.c", "".join(expressions), "generated 4000 expression-heavy functions")
    save("declaration-mix.c", "".join(declarations), "generated 4000 aggregate/typedef/initializer groups")
    save("statement-mix.c", "".join(statements), "generated 4000 control-flow-heavy functions")
    save("alignment-cache.c", "int aligned __attribute__((aligned(8)));\n" + "".join(f"int a{index} = {index};\n" for index in range(50000)), "generated full-token alignment index plus 50000 scalar declarations")
    for depth in [16, 128, 1024]:
        save(f"alignment-nested-{depth}.c", "int x __attribute__((aligned(" + "sizeof(struct {int y __attribute__((aligned(" * depth + "8" + ")));})" * depth + ")));\n", f"generated nested alignment operands depth={depth}")
    for depth in [32, 127, 256, 1024, 4096, 16384]:
        fixtures = {
            "parentheses": "int f(void){return " + "(" * depth + "1" + ")" * depth + ";}\n",
            "unary": "int f(void){return " + "! " * depth + "1;}\n",
            "assignment": "int f(int a){return " + "a=" * depth + "1;}\n",
            "conditional": "int f(int a){return " + "a?1:" * depth + "2;}\n",
            "statements": "int f(int a){" + "if(a)" * depth + "return 1; return 0;}\n",
            "blocks": "int f(void){" + "{" * depth + "int x;" + "}" * depth + "return 0;}\n",
            "declarator": "int " + "(" * depth + "x" + ")" * depth + ";\n",
            "initializer": "int x = " + "{" * depth + "1" + "}" * depth + ";\n",
        }
        for kind, source in fixtures.items():
            save(f"{kind}-{depth}.c", source, f"generated depth={depth}")
    for size in [1000, 10000, 100000]:
        save(f"chain-{size}.c", "int f(int a){return " + "+".join(["a"] * size) + ";}\n", f"generated size={size}")
        save(f"initializer-list-{size}.c", "int x[]={" + ",".join(["1"] * size) + "};\n", f"generated size={size}")

    if args.lua:
        url = "https://codeload.github.com/lua/lua/tar.gz/refs/tags/v5.4.8"
        archive_path = args.output / "lua-5.4.8.tar.gz"
        if not archive_path.exists():
            with urllib.request.urlopen(url, timeout=120) as response:
                archive_path.write_bytes(response.read())
        payload = archive_path.read_bytes()
        # Pinned archive hash, observed from GitHub's v5.4.8 tag on 2026-10-21.
        digest = hashlib.sha256(payload).hexdigest()
        expected = "d85b70a65f43c5d2254944d58d625e822c8e2e10d9c6a3bd9b5b657e46376a19"
        if digest != expected:
            raise SystemExit(f"Lua archive checksum mismatch: {digest}")
        manifest[archive_path.name] = {"sha256": digest, "bytes": len(payload), "origin": url}
        with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
            archive.extractall(args.output, filter="data")
        command = [args.cc, "-std=gnu99", "-E", "-P", str(args.output / "lua-5.4.8/onelua.c")]
        result = subprocess.run(command, check=True, capture_output=True)
        save("lua.i", result.stdout, " ".join(command))
        # glibc exposes _Float128 prototypes, while bcc accepts the equivalent
        # reserved GNU spelling __float128. Preserve the untouched case above
        # and prepare an explicit compatibility variant for clean comparisons.
        command.insert(2, "-D_Float128=__float128")
        result = subprocess.run(command, check=True, capture_output=True)
        save("lua-supported.i", result.stdout, " ".join(command))

    if args.sqlite:
        url = "https://www.sqlite.org/2025/sqlite-amalgamation-3500400.zip"
        archive_path = args.output / "sqlite-amalgamation-3500400.zip"
        if not archive_path.exists():
            with urllib.request.urlopen(url, timeout=120) as response:
                archive_path.write_bytes(response.read())
        payload = archive_path.read_bytes()
        manifest[archive_path.name] = {"sha256": hashlib.sha256(payload).hexdigest(), "bytes": len(payload), "origin": url}
        with zipfile.ZipFile(io.BytesIO(payload)) as archive:
            source = archive.read("sqlite-amalgamation-3500400/sqlite3.c")
        save("sqlite3.c", source, url)
        command = [args.cc, "-std=gnu99", "-DSQLITE_OMIT_LOAD_EXTENSION", "-E", "-P", str(args.output / "sqlite3.c")]
        result = subprocess.run(command, check=True, capture_output=True)
        save("sqlite3.i", result.stdout, " ".join(command))

    manifest["_compiler"] = subprocess.check_output([args.cc, "--version"], text=True).splitlines()[0]
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Prepared {len(manifest) - 1} inputs in {args.output}")


if __name__ == "__main__":
    main()
