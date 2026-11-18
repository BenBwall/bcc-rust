"""Run real build systems against compilers and compare what they conclude.

Each compiler is given as LABEL=COMMAND, the command split as a shell would
(`bcc=/path/bcc-rust --gcc-mode`, `gcc=gcc`). For every compiler the script
configures, builds and tests one small project with each build system it
finds on PATH, and prints, per build system, the values that identify the
compiler, side by side:

- CMake (Ninja when installed, and Unix Makefiles, with response files
  forced under Ninja): the compiler ID, version, frontend variant, default
  standard and extensions, compile features, implicit link libraries and
  directories, library architecture, linker ID, pointer size, depfile flags
  and the results of `check_c_source_compiles`, `check_c_compiler_flag` and
  `check_type_size`;
- Autoconf with Automake (and libtool, when `libtoolize` is on PATH): the
  `ac_cv_*`, `am_cv_*` and `lt_cv_*` cache values that `AC_PROG_CC`,
  `AC_PROG_CPP`, `AC_C_BIGENDIAN`, `AC_CHECK_SIZEOF`, `AC_SYS_LARGEFILE`,
  `AC_SEARCH_LIBS` and `LT_INIT` decide;
- Meson: the compiler and linker it names and the results of its checks.

`--windows` instead runs CMake's identification for a `cl`-style compiler
with a Windows toolchain file and static libraries only, which works off
Windows without the SDK; give `--archiver` an `llvm-ar` named `lib` or
`llvm-lib` there.

Usage:
    python scripts/build_system_probes.py --work build/probes \\
        gcc=gcc "bcc-gcc=target/debug/bcc-rust --gcc-mode"

`docs/design/driver-modes.md` ("Build systems") records the comparison for
each personality; `tests/integration/build_system_probes.rs` pins the probes
that decided it.
"""
import argparse
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys

LIBRARY = """\
#ifdef _WIN32
#define API __declspec(dllexport)
#else
#define API __attribute__((visibility("default")))
#endif
API int shared_add(int a, int b) { return a + b; }
"""
ARCHIVE = "int static_mul(int a, int b) { return a * b; }\n"
PROGRAM = """\
#include <math.h>
#include <stdio.h>
int shared_add(int, int);
int static_mul(int, int);
int main(void) {
    volatile double d = 2.0;
    int r = shared_add(2, 3) + static_mul(2, 3) + (int)sqrt(d * 8.0);
    printf("%d\\n", r);
    return r == 15 ? 0 : 1;
}
"""
CMAKE_VARIABLES = (
    "CMAKE_C_COMPILER_ID CMAKE_C_COMPILER_VERSION CMAKE_C_COMPILER_FRONTEND_VARIANT "
    "CMAKE_C_SIMULATE_ID CMAKE_C_SIMULATE_VERSION CMAKE_C_STANDARD_COMPUTED_DEFAULT "
    "CMAKE_C_EXTENSIONS_COMPUTED_DEFAULT CMAKE_C_COMPILE_FEATURES "
    "CMAKE_C_IMPLICIT_LINK_LIBRARIES CMAKE_C_IMPLICIT_LINK_DIRECTORIES "
    "CMAKE_C_LIBRARY_ARCHITECTURE CMAKE_C_COMPILER_LINKER_ID CMAKE_SIZEOF_VOID_P "
    "CMAKE_C_BYTE_ORDER CMAKE_C_COMPILER_ABI CMAKE_C_COMPILER_ARCHITECTURE_ID "
    "CMAKE_C_COMPILE_OPTIONS_PIC CMAKE_DEPFILE_FLAGS_C CMAKE_C_DEPFILE_FORMAT "
    "CMAKE_C_STANDARD_LATEST CMAKE_C_LINKER_DEPFILE_SUPPORTED MSVC_VERSION"
)
CMAKE_CHECKS = """\
include(CheckCSourceCompiles)
include(CheckCCompilerFlag)
include(CheckTypeSize)
check_c_source_compiles("int main(void){return __builtin_expect(1,1)-1;}" HAVE_BUILTIN_EXPECT)
check_c_source_compiles("#include <stdatomic.h>\\nint main(void){_Atomic int x=0; return atomic_load(&x);}" HAVE_STDATOMIC)
check_type_size("long double" SIZEOF_LONG_DOUBLE)
check_c_compiler_flag(-Wall HAVE_WALL)
check_c_compiler_flag(-Wno-bogus-warning HAVE_WNO_BOGUS)
check_c_compiler_flag(-fbogus-flag HAVE_FBOGUS)
check_c_compiler_flag(/W4 HAVE_W4)
check_c_compiler_flag(/bogus HAVE_SLASH_BOGUS)
check_c_compiler_flag(-fstack-protector-strong HAVE_SSP)
"""
CMAKE_CHECK_NAMES = (
    "HAVE_BUILTIN_EXPECT HAVE_STDATOMIC SIZEOF_LONG_DOUBLE HAVE_WALL HAVE_WNO_BOGUS "
    "HAVE_FBOGUS HAVE_W4 HAVE_SLASH_BOGUS HAVE_SSP"
)
AUTOCONF_KEYS = re.compile(
    r"^((?:ac_cv_c_compiler_gnu|ac_cv_prog_cc_c11|ac_cv_prog_cc_c99|ac_cv_prog_cc_g"
    r"|ac_cv_c_bigendian|ac_cv_sizeof_long|ac_cv_sys_file_offset_bits|ac_cv_search_sqrt"
    r"|ac_cv_c_inline|ac_cv_c_restrict|am_cv_CC_dependencies_compiler_type"
    r"|lt_cv_prog_compiler_pic|lt_cv_prog_compiler_pic_works|lt_cv_prog_compiler_c_o"
    r"|lt_cv_prog_compiler_static_works|lt_cv_prog_compiler_rtti_exceptions"
    r"|lt_cv_prog_gnu_ld|lt_cv_shlibpath_overrides_runpath|lt_cv_deplibs_check_method"
    r"|lt_cv_path_LD))=(.*)$"
)
LIBTOOL_KEYS = re.compile(r"^(sys_lib_search_path_spec|pic_flag|wl|link_static_flag)=(.*)$")


def run(command, cwd, log, env=None, timeout=900):
    """Runs `command` in `cwd`, appending its output to `log`; returns its status."""
    with open(log, "a") as out:
        out.write(f"$ {shlex.join(command)}\n")
        out.flush()
        try:
            return subprocess.run(command, cwd=cwd, stdout=out, stderr=subprocess.STDOUT,
                                  env=env, timeout=timeout).returncode
        except (OSError, subprocess.TimeoutExpired) as error:
            out.write(f"{error}\n")
            return -1


def environment(compiler):
    env = dict(os.environ)
    env["CC"] = compiler
    env.pop("CFLAGS", None)
    env.pop("LDFLAGS", None)
    return env


def write_sources(directory):
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "lib.c").write_text(LIBRARY)
    (directory / "lib2.c").write_text(ARCHIVE)
    (directory / "main.c").write_text(PROGRAM)


def cmake(compiler, work, generator, windows, archiver):
    source = work / "cmake-src"
    write_sources(source)
    text = "cmake_minimum_required(VERSION 3.20)\nproject(probe C)\n"
    text += f"foreach(v {CMAKE_VARIABLES})\n  message(STATUS \"PROBE ${{v}}=${{${{v}}}}\")\nendforeach()\n"
    text += CMAKE_CHECKS
    text += f"foreach(v {CMAKE_CHECK_NAMES})\n  message(STATUS \"PROBE ${{v}}=${{${{v}}}}\")\nendforeach()\n"
    if windows:
        text += "add_library(objs STATIC lib.c lib2.c)\n"
    else:
        text += (
            "set(CMAKE_C_STANDARD 11)\nset(CMAKE_POSITION_INDEPENDENT_CODE ON)\n"
            "add_library(shlib SHARED lib.c)\nset_target_properties(shlib PROPERTIES SOVERSION 1)\n"
            "add_library(stlib STATIC lib2.c)\nadd_executable(app main.c)\n"
            "target_link_libraries(app shlib stlib m)\nenable_testing()\nadd_test(NAME app COMMAND app)\n"
        )
    (source / "CMakeLists.txt").write_text(text)
    build = work / f"cmake-{generator.replace(' ', '_')}"
    shutil.rmtree(build, ignore_errors=True)
    build.mkdir()
    log = build / "probe.log"
    command = ["cmake", "-G", generator, str(source), "-DCMAKE_BUILD_TYPE=RelWithDebInfo"]
    if generator == "Ninja":
        command.append("-DCMAKE_NINJA_FORCE_RESPONSE_FILE=ON")
    if windows:
        toolchain = build / "windows.cmake"
        toolchain.write_text(
            "set(CMAKE_SYSTEM_NAME Windows)\nset(CMAKE_SYSTEM_PROCESSOR AMD64)\n"
            "set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)\n"
            + (f"set(CMAKE_AR {archiver} CACHE FILEPATH \"\")\n" if archiver else "")
        )
        command.append(f"-DCMAKE_TOOLCHAIN_FILE={toolchain}")
    results = {}
    results["configure"] = run(command, build, log, environment(compiler))
    for line in log.read_text(errors="replace").splitlines():
        match = re.match(r"-- PROBE (\w+)=(.*)$", line)
        if match:
            results[match.group(1)] = match.group(2)
    if results["configure"] == 0:
        results["build"] = run(["cmake", "--build", ".", "-v"], build, log)
        if not windows and results["build"] == 0:
            results["test"] = run(["ctest", "--output-on-failure"], build, log, timeout=300)
    return results


def autoconf(compiler, work):
    source = work / "autoconf"
    shutil.rmtree(source, ignore_errors=True)
    write_sources(source)
    libtool = shutil.which("libtoolize") is not None
    (source / "configure.ac").write_text(
        "AC_INIT([probe], [1.0])\nAC_CONFIG_AUX_DIR([build-aux])\nAC_CONFIG_MACRO_DIRS([m4])\n"
        "AM_INIT_AUTOMAKE([foreign subdir-objects])\nAC_PROG_CC\nAC_PROG_CPP\n"
        "AC_C_BIGENDIAN\nAC_CHECK_SIZEOF([long])\nAC_SYS_LARGEFILE\nAC_C_INLINE\nAC_C_RESTRICT\n"
        + ("LT_INIT([shared static])\n" if libtool else "AC_PROG_RANLIB\n")
        + "AC_SEARCH_LIBS([sqrt], [m])\nAC_CONFIG_FILES([Makefile])\nAC_OUTPUT\n"
    )
    if libtool:
        makefile = (
            "ACLOCAL_AMFLAGS = -I m4\nlib_LTLIBRARIES = libprobe.la\n"
            "libprobe_la_SOURCES = lib.c lib2.c\nbin_PROGRAMS = app\napp_SOURCES = main.c\n"
            "app_LDADD = libprobe.la\nTESTS = app\n"
        )
    else:
        makefile = (
            "ACLOCAL_AMFLAGS = -I m4\nlib_LIBRARIES = libprobe.a\nlibprobe_a_SOURCES = lib.c lib2.c\n"
            "bin_PROGRAMS = app\napp_SOURCES = main.c\napp_LDADD = libprobe.a\nTESTS = app\n"
        )
    (source / "Makefile.am").write_text(makefile)
    (source / "m4").mkdir()
    log = source / "probe.log"
    results = {"libtool": "yes" if libtool else "not installed"}
    if run(["autoreconf", "-fi"], source, log) != 0:
        results["autoreconf"] = "failed"
        return results
    results["configure"] = run(["./configure", f"CC={compiler}"], source, log)
    config = (source / "config.log").read_text(errors="replace") if (source / "config.log").exists() else ""
    for line in config.splitlines():
        match = AUTOCONF_KEYS.match(line)
        if match:
            results[match.group(1)] = match.group(2)
    script = source / "libtool"
    if script.exists():
        for line in script.read_text(errors="replace").splitlines():
            match = LIBTOOL_KEYS.match(line)
            if match and match.group(1) not in results:
                results[match.group(1)] = match.group(2)
    if results["configure"] == 0:
        results["build"] = run(["make", "V=1"], source, log)
        if results["build"] == 0:
            results["test"] = run(["make", "check"], source, log, timeout=300)
    return results


def meson(compiler, work):
    source = work / "meson-src"
    write_sources(source)
    (source / "meson.build").write_text(
        "project('probe', 'c', default_options: ['c_std=gnu11'])\ncc = meson.get_compiler('c')\n"
        "message('PROBE id=' + cc.get_id())\nmessage('PROBE version=' + cc.version())\n"
        "message('PROBE linker=' + cc.get_linker_id())\n"
        "message('PROBE argument_syntax=' + cc.get_argument_syntax())\n"
        "message('PROBE has_bogus=' + cc.has_argument('-fbogus-flag').to_string())\n"
        "message('PROBE sizeof_long=' + cc.sizeof('long').to_string())\n"
        "m = cc.find_library('m', required: false)\n"
        "sh = shared_library('shlib', 'lib.c')\nst = static_library('stlib', 'lib2.c')\n"
        "app = executable('app', 'main.c', link_with: [sh, st], dependencies: [m])\ntest('app', app)\n"
    )
    build = work / "meson-build"
    shutil.rmtree(build, ignore_errors=True)
    log = work / "meson.log"
    log.unlink(missing_ok=True)
    meson_program = os.environ.get("MESON", "meson")
    results = {"configure": run([meson_program, "setup", str(build), str(source)], work, log,
                                environment(compiler))}
    for line in log.read_text(errors="replace").splitlines():
        match = re.match(r"Message: PROBE (\w+)=(.*)$", line)
        if match:
            results[match.group(1)] = match.group(2)
        elif "ERROR:" in line:
            results["error"] = line.split("ERROR:", 1)[1].strip()
    if results["configure"] == 0:
        results["build"] = run([meson_program, "compile", "-C", str(build)], work, log)
        if results["build"] == 0:
            results["test"] = run([meson_program, "test", "-C", str(build)], work, log, timeout=300)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("compilers", nargs="+", help="LABEL=COMMAND")
    parser.add_argument("--work", default="build/build-system-probes", help="scratch directory")
    parser.add_argument("--windows", action="store_true", help="CMake identification of cl-style compilers")
    parser.add_argument("--archiver", help="the lib.exe stand-in for --windows")
    args = parser.parse_args()
    work = Path(args.work).resolve()
    systems = {}
    for spec in args.compilers:
        label, _, command = spec.partition("=")
        words = shlex.split(command)
        if words and os.sep in words[0]:
            words[0] = str(Path(words[0]).resolve())
        compiler = shlex.join(words)
        directory = work / label
        directory.mkdir(parents=True, exist_ok=True)
        print(f"probing {label}: {compiler}", file=sys.stderr)
        generators = ["Ninja"] if shutil.which("ninja") else []
        if not args.windows:
            generators.append("Unix Makefiles")
        if shutil.which("cmake"):
            for generator in generators:
                systems.setdefault(f"CMake ({generator})", {})[label] = cmake(
                    compiler, directory, generator, args.windows, args.archiver)
        if args.windows:
            continue
        if shutil.which("autoreconf"):
            systems.setdefault("Autoconf", {})[label] = autoconf(compiler, directory)
        if shutil.which(os.environ.get("MESON", "meson")):
            systems.setdefault("Meson", {})[label] = meson(compiler, directory)
    labels = [spec.partition("=")[0] for spec in args.compilers]
    for system, by_label in systems.items():
        print(f"\n## {system}\n")
        keys = []
        for results in by_label.values():
            keys.extend(key for key in results if key not in keys)
        for key in keys:
            values = [str(by_label.get(label, {}).get(key, "")) for label in labels]
            marker = " " if len(set(values)) == 1 else "*"
            print(f"{marker} {key}")
            for label, value in zip(labels, values):
                print(f"    {label}: {value}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
