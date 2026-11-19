"""Compare `bcc --msvc-mode`, and bcc run as `cl`, with the real `cl.exe`
under Wine, byte for byte, on the same command lines.

Each command line runs in a fresh copy of one directory of sources, once
with `cl.exe` (run by `wine64` in the environment msvc-wine's `msvcenv.sh`
sets, without msvc-wine's wrappers, which rewrite paths in the output) and
once with each bcc form, with INCLUDE and LIB in the host's form. The exit
status, the files written, stdout, stderr and the text of each `.i` file are
compared after the directory's path, in either form, becomes `<dir>` and
CRLF becomes LF.

Fetch the toolchain with msvc-wine (https://github.com/mstorsjo/msvc-wine),
accepting Microsoft's licence:

    python3 vsdownload.py --accept-license --major 17 --architecture x64 \\
        --host-arch x64 --only-host yes --with-atl no --with-mfc no \\
        --with-asan no --with-dia no --with-msbuild no --dest MSVC
    ./install.sh MSVC

Usage:
    python3 scripts/compare_cl_under_wine.py --msvc MSVC \\
        --bcc target/debug/bcc-rust --out build/cl-wine [--filter TEXT]

`wine64` (or `wine`) and, for the command lines that link, `lld-link` must
be on PATH. The script writes OUT/report.txt, every run side by side, and
OUT/summary.tsv, which of the five agree for each command line and form,
and prints the counts. `docs/design/driver-modes.md` ("Checked against
`cl.exe` under Wine") records a run.
"""
import argparse
import glob
import os
import shutil
import subprocess
import sys

FILES = {
    "a.c": "int b(void);\nint main(void) { return b() - 2; }\n",
    "b.c": "int b(void) { return 2; }\n",
    "pp.c": "#include \"h.h\"\n#ifndef VALUE\n#define VALUE 1\n#endif\nint pp = VALUE + H;\n",
    "h.h": "#define H 10\n",
    "inc.c": "#include \"deep.h\"\nint deep = DEEP;\n",
    "inc/deep.h": "#define DEEP 7\n",
    "unused.c": "int main(void) { int unused; return 0; }\n",
    "cast.c": "int f(int *p) { return (int)p; }\n",
    "bad.c": "int main(void) {\n    return missing;\n}\n",
    "syntax.c": "int f(void) {\n    int x = 1\n    int y = 2;\n    x = 3\n    y = x;\n    return x\n}\n",
    "implicit.c": "int main(void) { return g(1); }\nint g(int x) { return x - 1; }\n",
    "pointers.c": "int main(void) {\n    int *p = 0;\n    float *q = 0;\n    char **r = 0;\n    p = q;\n    p = r;\n    return 0;\n}\n",
    "initializer.c": "int main(void) {\n    int *p = 5;\n    return p != 0;\n}\n",
    "options.rsp": "/c a.c /link /SUBSYSTEM:CONSOLE\n/Foresponse.obj\n",
    "define.rsp": "/DVALUE=4\n/EP pp.c\n",
    # New inputs for this survey.
    "undef.c": "#ifdef GONE\nint gone = 1;\n#else\nint gone = 0;\n#endif\n",
    "show.c": "#include \"h.h\"\n#include \"nest.h\"\n#include \"h2.h\"\nint show = H + N;\n",
    "nest.h": "#include \"inc/deep.h\"\n#define N DEEP\n",
    "h2.h": "#ifndef H2\n#define H2\n#include \"nest.h\"\n#endif\n",
    "sys.c": "#include <stdio.h>\nint main(void) { printf(\"%d\\n\", 42); return 0; }\n",
    "error.c": "#error stop here\nint x;\n",
    "noinc.c": "#include \"nowhere.h\"\nint x;\n",
    "eof.c": "int f(void) {\n    return 0;\n",
    "redef.c": "int x;\nfloat x;\nint f(void) { return 1; }\nint f(void) { return 2; }\n",
    "member.c": "struct s { int a; };\nint f(struct s v) { return v.b; }\n",
    "args.c": "int g(int a, int b);\nint f(void) { return g(1); }\nint h(void) { return g(1, 2, 3); }\n",
    "lvalue.c": "int f(int a) { 1 = a; return *a; }\n",
    "conv.c": "int f(double d) { int i = d; short s = i; unsigned u = 1; if (i < u) return s; return i; }\n",
    "w4.c": "int f(int unused_param) {\n    int x;\n    int y = 3;\n    if (x = 2) return 1;\n    return 0;\n}\n",
    "returns.c": "int f(int a) { if (a) return 1; }\nvoid g(void) { return 1; }\nint h(void) { return; }\n",
    "fmt.c": "#include <stdio.h>\nint main(void) { printf(\"%d\\n\", \"s\"); return 0; }\n",
    "shift.c": "int f(void) { int a = 1 << 40; unsigned char c = 300; return a + c; }\n",
    "divzero.c": "int f(void) { return 1 / 0; }\n",
    "callnp.c": "int f(void) { int *p = 0; return p(); }\n",
    "main.c": "#include <stdio.h>\nint b(void);\nint main(void) { printf(\"b=%d\\n\", b()); return b(); }\n",
    "lines.c": "#define A 1\n#define B 2\nint a = A;\n\n\n\nint b = B;\n#if 0\nx\ny\nz\n#endif\nint c;\n#ifdef A\nint e;\n#else\nint f;\n#endif\n\n\n\n\n\n\n\n\n\n\nint g;\nint h = A +\n  B;\n#define M(x) x +\\\n  1\nint i = M(2);\n#define LAST 1\n",
    "sub/s.c": "int s;\n",
    "nested.c": "#if 1\n# if 0\nint no;\n# elif 1\nint yes;\n# endif\n#endif\n#include \"h2.h\"\nint n = N;\n",
}

NAMES = [
    "_MSC_VER", "_MSC_FULL_VER", "_MSC_BUILD", "_MSC_EXTENSIONS", "_MSVC_TRADITIONAL",
    "_WIN32", "_WIN64", "_M_X64", "_M_AMD64", "_M_IX86", "_M_ARM64", "_M_FP_PRECISE",
    "_M_FP_FAST", "_M_FP_STRICT", "_MT", "_DLL", "_DEBUG", "_CHAR_UNSIGNED",
    "_INTEGRAL_MAX_BITS", "_ISO_VOLATILE", "__STDC__", "__STDC_VERSION__",
    "__STDC_HOSTED__", "__STDC_NO_ATOMICS__", "__STDC_NO_COMPLEX__", "__STDC_NO_THREADS__",
    "__STDC_NO_VLA__", "__AVX__", "__SSE2__", "__GNUC__", "__clang__", "__x86_64__",
    "__bcc__", "__STDC_UTF_16__", "__STDC_UTF_32__", "__STDC_IEC_559__", "_M_AMD64",
    "__DATE__", "__TIME__", "__FILE__", "__LINE__", "__COUNTER__", "__FUNCTION__",
    "_WCHAR_T_DEFINED", "_NATIVE_WCHAR_T_DEFINED", "__cplusplus", "_CPPRTTI", "_CPPUNWIND",
    "__STDC_SECURE_LIB__", "_MSC_FULL_VER", "__SIZEOF_POINTER__", "_PREFAST_",
    "__STDC_WANT_SECURE_LIB__", "_M_X64", "_VC_NODEFAULTLIB", "__TIMESTAMP__",
]
FILES["macros.c"] = "#define S(x) #x\n#define X(x) S(x)\n" + "".join(
    f"{n} S({n}) X({n})\n" for n in dict.fromkeys(NAMES) if n not in ("__DATE__", "__TIME__", "__TIMESTAMP__")
)

# (args, CL, _CL_)
CASES = []


def case(*args, cl=None, cl_after=None, nologo=True):
    a = list(args)
    if nologo and not (a and a[0].startswith("-")):
        a = ["/nologo"] + a
    CASES.append((a, cl, cl_after))


# Help, usage, banner
case(nologo=False)
case("/?", nologo=False)
case("/help", nologo=False)
case("/c", "a.c", nologo=False)
case("-c", "a.c", nologo=False)
case()
case("/c")
# Outputs
case("/c", "a.c")
case("/c", "a.c", "/Foout.obj")
case("/c", "a.c", "/Fo:out2.obj")
case("/c", "a.c", "/Fo:", "out3.obj")
case("/c", "a.c", "/Foout")
case("/c", "a.c", "b.c", "/Foobj\\")
case("/c", "a.c", "b.c", "/Foobj/")
case("/c", "a.c", "b.c", "/Foone.obj")
case("a.c", "b.c")
case("a.c", "b.c", "/Feprog.exe")
case("a.c", "b.c", "/Feprog")
case("a.c", "b.c", "/Fe:prog2.exe")
case("a.c", "b.c", "/Feobj\\")
case("a.c", "b.c", "/link", "/OUT:linked.exe")
case("a.c", "b.c", "/link", "/SUBSYSTEM:CONSOLE", "/DEBUG")
case("a.c", "b.c", "/link", "/nologo")
case("main.c", "b.c")
# Warning levels
for level in ["/W0", "/W1", "/W2", "/W3", "/W4", "/Wall", "/w"]:
    for f in ["implicit.c", "pointers.c", "cast.c", "unused.c", "w4.c", "conv.c"]:
        case(level, "/c", f)
case("/WX", "/c", "implicit.c")
case("/W3", "/WX", "/c", "implicit.c")
case("/WX", "/c", "pointers.c")
case("/WX", "/c", "cast.c")
case("/W4", "/WX", "/c", "unused.c")
case("/WX", "/c", "initializer.c")
case("/W4", "/wd4013", "/c", "implicit.c")
case("/W4", "/wd4101", "/c", "unused.c")
case("/we4013", "/c", "implicit.c")
case("/we4047", "/c", "pointers.c")
case("/we4101", "/c", "unused.c")
case("/w14013", "/c", "implicit.c")
case("/wd4047", "/c", "pointers.c")
case("/wd4133", "/c", "pointers.c")
case("/W4", "/wd4100", "/wd4189", "/c", "w4.c")
# Standards
for std in ["/std:c11", "/std:c17", "/std:clatest", "/std:c99", "/std:c89"]:
    case(std, "/c", "a.c")
case("/std:c11", "/EP", "macros.c")
case("/std:c17", "/EP", "macros.c")
case("/std:clatest", "/EP", "macros.c")
# Preprocessing
case("/E", "pp.c")
case("/EP", "pp.c")
case("/P", "pp.c")
case("/P", "/EP", "pp.c")
case("/P", "/Fipp-out.i", "pp.c")
case("/P", "/Fi:pp-out2.i", "pp.c")
case("/E", "pp.c", "a.c")
case("/EP", "pp.c", "a.c")
case("/DVALUE=3", "/EP", "pp.c")
case("/DVALUE#5", "/EP", "pp.c")
case("/D", "VALUE=6", "/EP", "pp.c")
case("/DVALUE", "/EP", "pp.c")
case("/DGONE", "/EP", "undef.c")
case("/DGONE", "/UGONE", "/EP", "undef.c")
case("/DGONE", "/U", "GONE", "/EP", "undef.c")
case("/Iinc", "/EP", "inc.c")
case("/I", "inc", "/EP", "inc.c")
case("/EP", "inc.c")
case("/E", "inc.c")
case("/EP", "macros.c")
case("/E", "macros.c")
case("/Zc:preprocessor", "/EP", "macros.c")
case("/MD", "/EP", "macros.c")
case("/MDd", "/EP", "macros.c")
case("/MTd", "/EP", "macros.c")
case("/LD", "/EP", "macros.c")
case("/Za", "/EP", "macros.c")
case("/J", "/EP", "macros.c")
case("/O2", "/EP", "macros.c")
case("/fp:fast", "/EP", "macros.c")
case("/fp:strict", "/EP", "macros.c")
case("/Zc:__STDC__", "/EP", "macros.c")
case("/E", "error.c")
case("/E", "noinc.c")
# Optimization and debug
for o in ["/O1", "/O2", "/Od", "/Ox", "/Os", "/Zi", "/Z7"]:
    case(o, "/c", "a.c")
case("/Zi", "a.c", "b.c")
# showIncludes
case("/showIncludes", "/c", "show.c")
case("/showIncludes", "/EP", "show.c")
case("/showIncludes", "/c", "sys.c")
case("/showIncludes", "/c", "noinc.c")
# Unknown and odd options
case("/c", "/bogus", "a.c")
case("/c", "-bogus", "a.c")
case("/c", "/Zs", "a.c")
case("/c", "a.c", "h.h")
case("/c", "/Tch.h")
case("/c", "a.c", "/Fe")
# Command files and environment
case("@options.rsp")
case("@define.rsp")
case("/c", "@missing.rsp", "a.c")
case("a.c", cl="/c /Foenvironment.obj")
case("/Fofirst.obj", "a.c", cl="/c", cl_after="/Folast.obj")
case("/c", "pp.c", cl="/DVALUE#2", cl_after="/Fodefined.obj")
case("/EP", "pp.c", cl="/DVALUE=8")
case("/EP", "pp.c", cl_after="/DVALUE=9")
case("/c", "a.c", cl="/nologo", nologo=False)
# Overriding options (D9025)
for pair in [("/W3", "/W4"), ("/w", "/W4"), ("/W0", "/w"), ("/W4", "/Wall"), ("/O1", "/O2"), ("/O2", "/Ox"),
             ("/Od", "/O2"), ("/O1", "/Os"), ("/Os", "/Ot"), ("/MD", "/MT"), ("/MT", "/MTd"), ("/std:c11", "/std:c17"),
             ("/Zi", "/Z7"), ("/WX", "/WX-"), ("/DX", "/UX"), ("/UX", "/DX"), ("/DX=1", "/DX=2"), ("/Fofirst.obj", "/Folast.obj"),
             ("/Zp4", "/Zp8"), ("/GS", "/GS-"), ("/Gy", "/Gy-"), ("/fp:fast", "/fp:precise")]:
    case(*pair, "/c", "a.c")
case("/D", "X", "/U", "X", "/c", "a.c")
# Preprocessor output layout
case("/E", "lines.c")
case("/EP", "lines.c")
case("/P", "lines.c")
case("/E", "nested.c")
case("/EP", "nested.c")
case("/E", "show.c")
case("/E", "undef.c")
case("/P", "error.c")
case("/P", "noinc.c")
case("/P", "/showIncludes", "show.c")
case("/Zs", "/showIncludes", "show.c")
case("/EP", "/W3", "implicit.c")
case("/c", "sub/s.c")
case("/Za", "/EP", "macros.c")
case("/Zc:__STDC__", "/EP", "macros.c")
# Diagnostics
for f in [
    "bad.c", "syntax.c", "implicit.c", "pointers.c", "initializer.c", "cast.c",
    "error.c", "noinc.c", "eof.c", "redef.c", "member.c", "args.c", "lvalue.c",
    "returns.c", "fmt.c", "shift.c", "divzero.c", "callnp.c",
]:
    case("/c", f)
    case("/W4", "/c", f)
case("/c", "/diagnostics:column", "bad.c")
case("/c", "/diagnostics:caret", "bad.c")
case("/c", "/diagnostics:classic", "bad.c")
case("/c", "missing.c")
case("/c", "a.c", "missing.c")
case("/c", "bad.c", "missing.c")
case("bad.c", "b.c")
case("/LD", "b.c")


def newest(pattern):
    """The last of the paths `pattern` matches, by name."""
    matches = sorted(glob.glob(pattern))
    if not matches:
        sys.exit(f"nothing matches {pattern}")
    return matches[-1]


class Toolchain:
    """The msvc-wine installation and the bcc binary compared with it."""

    def __init__(self, msvc, bcc):
        self.bcc = os.path.abspath(bcc)
        msvc = os.path.abspath(msvc)
        vc = newest(os.path.join(msvc, "VC", "Tools", "MSVC", "*"))
        kits = os.path.join(msvc, "Windows Kits", "10")
        include = newest(os.path.join(kits, "Include", "10.*"))
        version = os.path.basename(include)
        lib = os.path.join(kits, "Lib", version)
        self.cl_exe = os.path.join(vc, "bin", "Hostx64", "x64", "cl.exe")
        self.wine = shutil.which("wine64") or shutil.which("wine") or "/usr/lib/wine/wine64"
        windows = lambda path: "z:" + path.replace("/", "\\")
        self.cl_include = ";".join(
            windows(path)
            for path in [
                os.path.join(vc, "include"),
                os.path.join(include, "shared"),
                os.path.join(include, "ucrt"),
                os.path.join(include, "um"),
            ]
        )
        self.cl_lib = ";".join(
            windows(path)
            for path in [
                os.path.join(vc, "lib", "x64"),
                os.path.join(lib, "ucrt", "x64"),
                os.path.join(lib, "um", "x64"),
            ]
        )
        self.bcc_include = os.pathsep.join(
            [
                os.path.join(vc, "include"),
                os.path.join(include, "shared"),
                os.path.join(include, "ucrt"),
                os.path.join(include, "um"),
            ]
        )
        self.bcc_lib = ";".join(
            [
                os.path.join(vc, "lib", "x64"),
                os.path.join(lib, "ucrt", "x64"),
                os.path.join(lib, "um", "x64"),
            ]
        )
        # bcc run by the name `cl`.
        self.named = None

    def environment(self, kind, variables):
        env = dict(os.environ)
        for name in ("CL", "_CL_"):
            env.pop(name, None)
        env.update({name: value for name, value in variables.items() if value is not None})
        if kind == "cl":
            env.update(INCLUDE=self.cl_include, LIB=self.cl_lib, LIBPATH=self.cl_lib, WINEDEBUG="-all")
        else:
            env.update(INCLUDE=self.bcc_include, LIB=self.bcc_lib)
        return env

    def command(self, kind, args):
        if kind == "cl":
            return [self.wine, self.cl_exe, *args]
        if kind == "bcc-cl":
            return [self.named, *args]
        return [self.bcc, "--msvc-mode", *args]


def listing(directory):
    names = set()
    for root, _, files in os.walk(directory):
        for name in files:
            names.add(os.path.relpath(os.path.join(root, name), directory))
    return names


def run(toolchain, kind, directory, args, variables):
    if os.path.isdir(directory):
        shutil.rmtree(directory)
    for subdirectory in ("inc", "obj", "sub"):
        os.makedirs(os.path.join(directory, subdirectory))
    for name, text in FILES.items():
        with open(os.path.join(directory, name), "w", newline="\n") as file:
            file.write(text)
    before = listing(directory)
    process = subprocess.run(
        toolchain.command(kind, args),
        cwd=directory,
        env=toolchain.environment(kind, variables),
        capture_output=True,
        timeout=300,
    )
    # `cl /Zi` writes a program database, which bcc does not.
    created = sorted(
        name for name in listing(directory) - before if not name.lower().endswith((".pdb", ".ilk"))
    )
    contents = {}
    for name in created:
        if name.endswith(".i"):
            with open(os.path.join(directory, name), "rb") as file:
                contents[name] = file.read()
    return {
        "status": process.returncode,
        "created": created,
        "stdout": process.stdout,
        "stderr": process.stderr,
        "contents": contents,
        "dir": directory,
    }


def normalize(data, directory):
    """`data` as text, with LF line ends and the run's directory as
    `<dir>`, in the host's form (`<dir>/h.h` for `cl`'s `<dir>\\h.h`)."""
    text = data.decode("utf-8", "replace").replace("\r\n", "\n")
    windows = "Z:" + directory.replace("/", "\\")
    for form in (windows, windows.lower(), "z:" + directory.replace("/", "\\"), directory):
        text = text.replace(form + "\\", "<dir>\\").replace(form + "/", "<dir>/").replace(form, "<dir>")
    # `#line` doubles the backslashes.
    doubled = windows.replace("\\", "\\\\")
    for form in (doubled, doubled.lower()):
        text = text.replace(form + "\\\\", "<dir>\\")
    return text.replace("<dir>\\", "<dir>/")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--msvc", required=True, help="the msvc-wine installation")
    parser.add_argument("--bcc", required=True, help="the bcc-rust binary")
    parser.add_argument("--out", required=True, help="the directory for the runs and reports")
    parser.add_argument("--filter", help="only the command lines that contain this text")
    options = parser.parse_args()
    toolchain = Toolchain(options.msvc, options.bcc)
    out = os.path.abspath(options.out)
    os.makedirs(out, exist_ok=True)
    named = os.path.join(out, "bin")
    os.makedirs(named, exist_ok=True)
    toolchain.named = os.path.join(named, "cl")
    if os.path.lexists(toolchain.named):
        os.remove(toolchain.named)
    os.symlink(toolchain.bcc, toolchain.named)
    runs = os.path.join(out, "runs")
    agree = {"bcc-cl": 0, "bcc": 0}
    total = 0
    with open(os.path.join(out, "report.txt"), "w") as report, open(
        os.path.join(out, "summary.tsv"), "w"
    ) as summary:
        summary.write("index\tcommand\tform\tstatus\tfiles\tstdout\tstderr\tcontents\n")
        for index, (args, before, after) in enumerate(CASES):
            label = " ".join(args)
            if before:
                label = f"CL={before} " + label
            if after:
                label = f"_CL_={after} " + label
            if options.filter and options.filter not in label:
                continue
            total += 1
            variables = {"CL": before, "_CL_": after}
            results = {
                kind: run(toolchain, kind, os.path.join(runs, f"{index}-{kind}"), args, variables)
                for kind in ("cl", "bcc-cl", "bcc")
            }
            report.write(f"\n==== [{index}] {label}\n")
            for kind, result in results.items():
                report.write(f"{kind}: status {result['status']}, created {result['created']}\n")
                for stream in ("stdout", "stderr"):
                    for line in normalize(result[stream], result["dir"]).splitlines():
                        report.write(f"{kind} {stream}| {line}\n")
                for name, text in result["contents"].items():
                    for line in normalize(text, result["dir"]).splitlines():
                        report.write(f"{kind} {name}| {line}\n")
            cl = results["cl"]
            for kind in ("bcc-cl", "bcc"):
                result = results[kind]
                same = {
                    field: normalize(cl[field], cl["dir"]) == normalize(result[field], result["dir"])
                    for field in ("stdout", "stderr")
                }
                contents = {
                    name: normalize(text, cl["dir"]) for name, text in cl["contents"].items()
                } == {name: normalize(text, result["dir"]) for name, text in result["contents"].items()}
                checks = [
                    cl["status"] == result["status"],
                    cl["created"] == result["created"],
                    same["stdout"],
                    same["stderr"],
                    contents,
                ]
                agree[kind] += all(checks)
                summary.write(f"{index}\t{label}\t{kind}\t" + "\t".join(map(str, checks)) + "\n")
            for result in results.values():
                shutil.rmtree(result["dir"], ignore_errors=True)
    print(f"{total} command lines; all five agree for {agree['bcc-cl']} as cl, {agree['bcc']} as --msvc-mode")


if __name__ == "__main__":
    main()
