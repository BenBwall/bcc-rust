"""Shared helpers for the conformance scripts.

`random_program_conformance.py` (generated programs) and `run_program_corpus.py`
(real programs) build each program with the bundled Clang as the reference and
with bcc's arms, run the results, and compare the output. This module holds what
they share: the arm definitions, the command lines, the include directories a
MinGW-w64 target needs, and the outcome classification. It reuses the process
runner (Windows kill-on-close job objects, deadlines) and the compiler-output
classifier from `run_gcc_torture_execute.py`, which also turns on the Windows
error-dialog suppression that every compiled test program relies on.
"""

from collections import Counter
from pathlib import Path
import os
import re
import shutil
import subprocess
import time

import crash_dialogs
from run_gcc_torture import ANSI, ROOT
from run_gcc_torture_execute import TARGET, classify
from run_gcc_torture_execute import invoke as run_process

# Programs compiled by the arms inherit this; a faulting one must not open a dialog.
crash_dialogs.suppress()

CLANG = ROOT / "target/llvm/bin/clang.exe"
BCC = ROOT / "target/x86_64-pc-windows-gnu/release/bcc-rust.exe"

# The five arms of the conformance survey. `native` arms build an executable.
# A0 is bcc without any optimizer, A1 adds only the bcc optimizer in front of
# LLVM's code generator, A2 only LLVM's O2 pipeline (middle-end.md, "Comparison arms").
ARMS = {
    "interpret": {"flags": ["--interpret"], "native": False},
    "interpret-bcc": {"flags": ["--interpret", "--opt=bcc"], "native": False},
    "A0": {"flags": ["--llvm-opt=O0"], "native": True},
    "A1": {"flags": ["--opt=bcc", "--llvm-opt=O2", "--llvm-codegen-only"], "native": True},
    "A2": {"flags": ["--llvm-opt=O2"], "native": True},
}
NATIVE_ARMS = [name for name, arm in ARMS.items() if arm["native"]]

# Outcome classes of one arm on one program, most severe last.
OUTCOMES = ("pass", "frontend-error", "lowering-unsupported", "abi-unsupported", "link-error",
            "timeout", "interpreter-trap", "abnormal-exit", "wrong-output", "compiler-crash")
REFUSALS = ("frontend-error", "lowering-unsupported", "abi-unsupported")

# Some libraries guard an inline body with a macro; bcc cannot translate the
# MinGW-w64 inline bodies (x87 inline assembly, `__builtin_fabsf`), so both the
# reference and bcc see the out-of-line declarations only.
MINGW_DEFINES = ["__CRT__NO_INLINE"]


def invoke(command, timeout, environment, cwd):
    """Run a process under a deadline (see run_gcc_torture_execute.invoke).

    Windows occasionally refuses to put a freshly started process into its kill-on-close
    job under heavy load (the torture runner kills the process and raises OSError). The
    commands here have no side effects beyond their work directory, so run them again.
    """
    for attempt in range(4):
        try:
            return run_process(command, timeout, environment, cwd)
        except OSError as error:
            if attempt == 3 or "process job" not in str(error):
                raise
            time.sleep(0.25 * (attempt + 1))


def remove_tree(path):
    """Delete a directory tree, retrying while a just-killed process still holds a handle in it."""
    for _ in range(20):
        shutil.rmtree(path, ignore_errors=True)
        if not Path(path).exists():
            return
        time.sleep(0.25)


def remove_file(path):
    """Delete a file, tolerating the short lock Windows keeps on a just-run executable."""
    for _ in range(20):
        try:
            Path(path).unlink(missing_ok=True)
            return
        except PermissionError:
            time.sleep(0.25)


def mingw_include_dirs(gcc="gcc"):
    """The C library directories GCC would search, without GCC's private ones."""
    executable = shutil.which(gcc)
    if executable is None:
        raise RuntimeError("gcc (MinGW-w64) is not on PATH; set it up as in README.md#prerequisites")
    listing = subprocess.run([executable, "-xc", "-E", "-v", os.devnull], capture_output=True,
                             text=True, timeout=60).stderr
    listing = listing.split("#include <...> search starts here:", 1)[-1].split("End of search list.", 1)[0]
    reported = [Path(line.strip()).resolve() for line in listing.splitlines() if line.strip()]
    private = [path for path in reported
               if any(a.lower() == "lib" and b.lower() == "gcc" for a, b in zip(path.parts, path.parts[1:]))]
    return [path.as_posix() for path in reported if path not in private]


WSL_DISTRO = "Ubuntu"
WSL_SYSROOT = ROOT / "target/sysroots/wsl-ubuntu"


def to_wsl_path(path):
    """The WSL spelling of a Windows path (C:\\a\\b becomes /mnt/c/a/b)."""
    path = Path(path).resolve()
    return "/mnt/" + path.drive[0].lower() + "/" + "/".join(path.parts[1:])


def wsl_command(*words):
    return ["wsl.exe", "-d", WSL_DISTRO, "--", *words]


class Platform:
    """Where a program is built and run: Windows (MinGW-w64 headers) or Linux in WSL.

    bcc itself always runs on Windows. On Linux it compiles for
    `x86_64-unknown-linux-gnu` against a copy of the WSL distribution's
    /usr/include (target/sysroots/wsl-ubuntu); the object files are linked and the
    programs run by WSL's own Clang, which is also the reference compiler there.
    """

    def __init__(self, name):
        self.name = name
        self.linux = name == "linux-wsl"
        self.target = "x86_64-unknown-linux-gnu" if self.linux else TARGET
        self.exe_name = "program" if self.linux else "program.exe"

    def system_includes(self):
        if self.linux:
            include = WSL_SYSROOT / "usr/include"
            return [(include / "x86_64-linux-gnu").as_posix(), include.as_posix()]
        return mingw_include_dirs()

    def defines(self):
        return [] if self.linux else list(MINGW_DEFINES)

    def reference_command(self, sources, output, opt, includes, defines):
        """Build the reference executable; Windows uses the bundled Clang, Linux WSL's."""
        if not self.linux:
            return clang_command(CLANG, sources, output, opt=opt, includes=includes, defines=defines)
        command = ["clang", opt, "-w", "-fno-strict-aliasing"]
        for directory in includes:
            command += ["-I", to_wsl_path(directory)]
        for define in defines:
            command += ["-D", define]
        command += [to_wsl_path(source) for source in sources] + ["-o", to_wsl_path(output), "-lm"]
        return wsl_command(*command)

    def gcc_command(self, sources, output, includes, defines):
        """A second opinion on the reference output: GCC -O0 (MinGW-w64 on PATH, or WSL's)."""
        command = ["gcc", "-O0", "-w"]
        convert = to_wsl_path if self.linux else str
        for directory in includes:
            command += ["-I", convert(directory)]
        for define in defines:
            command += ["-D", define]
        command += [convert(source) for source in sources] + ["-o", convert(output)]
        command += ["-lm"] if self.linux else []
        return wsl_command(*command) if self.linux else command

    def object_command(self, source, output, includes, defines, extra=()):
        """Compile one translation unit to an object with the reference compiler (Clang -O0)."""
        if not self.linux:
            command = [str(CLANG), f"--target={TARGET}", "-O0", "-w", "-fno-strict-aliasing", "-c", str(source),
                       "-o", str(output)]
            for directory in includes:
                command += ["-I", str(directory)]
            for define in defines:
                command += ["-D", define]
            return command + list(extra)
        command = ["clang", "-O0", "-w", "-fno-strict-aliasing", "-c", to_wsl_path(source), "-o", to_wsl_path(output)]
        for directory in includes:
            command += ["-I", to_wsl_path(directory)]
        for define in defines:
            command += ["-D", define]
        # -include takes a Windows path here, so convert the operand that follows each -include.
        words = list(extra)
        for index, word in enumerate(words):
            if word == "-include":
                words[index + 1] = to_wsl_path(words[index + 1])
        return wsl_command(*command, *words)

    def link_command(self, objects, output, libraries=("-lm",)):
        """Link object files with the bundled Clang (Windows) or WSL's Clang (Linux)."""
        if not self.linux:
            return [str(CLANG), f"--target={TARGET}", *map(str, objects), "-o", str(output), *libraries]
        return wsl_command("clang", *[to_wsl_path(o) for o in objects], "-o", to_wsl_path(output), *libraries)

    def run_command(self, exe, deadline, arguments=()):
        if not self.linux:
            return [str(exe), *arguments]
        # The kill inside WSL matters: stopping wsl.exe alone would leave the program running.
        return wsl_command("timeout", "-s", "KILL", str(int(deadline) + 2), to_wsl_path(exe), *arguments)


WINDOWS = Platform("windows")
LINUX = Platform("linux-wsl")
PLATFORMS = {platform.name: platform for platform in (WINDOWS, LINUX)}


def bcc_command(bcc, source, *, target=TARGET, arm_flags=(), includes=(), defines=(),
                system_includes=(), output=None, compile_only=False, extra=()):
    """The command line for one bcc invocation (arm flags last, like the survey)."""
    command = [str(bcc), str(source), f"--target={target}", "-w"]
    for directory in includes:
        command += ["-I", str(directory)]
    for directory in system_includes:
        command += ["-idirafter", str(directory)]
    for define in defines:
        command += ["-D", define]
    command += list(extra) + list(arm_flags)
    if output is not None:
        command += ["-o", str(output)]
        if compile_only:
            command.append("-c")
    return command


def clang_command(clang, sources, output, *, target=TARGET, opt="-O0", includes=(), defines=(),
                  extra=()):
    command = [str(clang), f"--target={target}", opt, "-w", "-fno-strict-aliasing"]
    for directory in includes:
        command += ["-I", str(directory)]
    for define in defines:
        command += ["-D", define]
    command += list(extra) + [str(source) for source in sources] + ["-o", str(output)]
    return command


def normalized(text):
    """Program output as compared: LF line ends, no trailing whitespace on the whole."""
    return text.replace("\r\n", "\n").rstrip()


def environment(work):
    """A clean environment whose temporary files stay in the work directory."""
    env = os.environ.copy()
    for variable in ("CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "OBJC_INCLUDE_PATH"):
        env.pop(variable, None)
    env.update(NO_COLOR="1", TMP=str(work), TEMP=str(work), TMPDIR=str(work))
    return env


def compare_run(result, reference):
    """Classify one program execution against the reference execution."""
    if result["timeout"]:
        return "timeout", "the program did not finish before the deadline"
    if result["exit"] != reference["exit"]:
        return "abnormal-exit", f"exit status {describe_exit(result['exit'])}, reference {describe_exit(reference['exit'])}"
    if normalized(result["stdout"]) != normalized(reference["stdout"]):
        return "wrong-output", first_difference(reference["stdout"], result["stdout"])
    return "pass", None


def describe_exit(code):
    if code is None:
        return "none"
    if code > 0x7FFFFFFF:
        return f"0x{code:08X}"
    return str(code)


def first_difference(expected, actual):
    expected, actual = normalized(expected).splitlines(), normalized(actual).splitlines()
    for index in range(max(len(expected), len(actual))):
        want = expected[index] if index < len(expected) else "<end of output>"
        got = actual[index] if index < len(actual) else "<end of output>"
        if want != got:
            return f"line {index + 1}: expected {want[:120]!r}, got {got[:120]!r}"
    return "outputs differ"


def run_arm(bcc, arm, source, work, *, deadlines, reference, command_kwargs, program_args=(),
            platform=WINDOWS):
    """Build (or interpret) one program with one arm and compare it with the reference.

    Returns a dict with `status`, `detail`, `constructs`, the commands and the
    timings. Raw outputs are returned under the private keys `_compile` and
    `_execute` so the caller can store them as evidence.
    """
    spec = ARMS[arm]
    env = environment(work)
    exe = work / platform.exe_name
    obj = work / "program.o"
    command_kwargs = {**command_kwargs, "target": platform.target}
    if platform.linux and spec["native"]:
        # bcc cannot link for Linux on this host: it writes an object, WSL's Clang links it.
        command = bcc_command(bcc, source, arm_flags=spec["flags"], output=obj, compile_only=True,
                              **command_kwargs)
    else:
        command = bcc_command(bcc, source, arm_flags=spec["flags"], output=exe if spec["native"] else None,
                              **command_kwargs)
    result = invoke(command, deadlines["compile" if spec["native"] else "interpret"], env, work)
    run = {"command": command, "compile_seconds": result["seconds"], "_compile": result}
    phase = "compile" if spec["native"] else "interpret"
    if phase == "interpret":
        # The interpreter prints the program's output and exits with its status;
        # a trap or a refusal is told apart from a program exit by the message.
        classified = classify(result, "interpret")
        if classified["status"] in ("wrong-result",):
            classified = {"status": "pass", "detail": None, "constructs": [], "trap_kind": None}
        run.update(classified)
        if run["status"] == "pass":
            run["status"], run["detail"] = compare_run(result, reference)
        elif run["status"] == "timeout":
            run["detail"] = "the interpreter did not finish before the deadline"
        run["_execute"] = None
        return run
    run.update(classify(result, "compile"))
    if run["status"] == "pass" and platform.linux:
        if not obj.is_file():
            run.update(status="compiler-crash", detail="compiler returned success without an object file")
        else:
            linked = invoke(platform.link_command([obj], exe), deadlines["compile"], env, work)
            run["_compile"] = {**result, "stderr": result["stderr"] + linked["stderr"],
                               "stdout": result["stdout"] + linked["stdout"]}
            if linked["exit"] != 0:
                run.update(status="link-error", detail=normalize_message(linked["stderr"].strip()[-200:]))
        remove_file(obj)
    if run["status"] == "pass":
        if not exe.is_file():
            run.update(status="compiler-crash", detail="compiler returned success without an executable")
        else:
            executed = invoke(platform.run_command(exe, deadlines["execute"], program_args),
                              deadlines["execute"] + 5, env, work)
            run["execute_seconds"] = executed["seconds"]
            run["_execute"] = executed
            run["status"], run["detail"] = compare_run(executed, reference)
    else:
        run["_execute"] = None
    if run["status"] == "timeout" and run.get("detail") is None:
        run["detail"] = "the compiler did not finish before the deadline"
    remove_file(exe)
    return run


def refusal_summary(rows):
    """Frequency of refused constructs, one count per program (not per arm)."""
    counter = Counter()
    for row in rows:
        seen = set()
        for run in row["arms"].values():
            if run["status"] in REFUSALS:
                seen.update(run.get("constructs")
                            or [normalize_message(run.get("detail") or run["status"])])
        counter.update(seen)
    return counter.most_common()


def normalize_message(message):
    """Collapse identifiers and numbers so that equal defects share a signature."""
    message = ANSI.sub("", message or "")
    message = re.sub(r"`[^`]*`|'[^']*'|\"[^\"]*\"", "<name>", message)
    message = re.sub(r"\b0x[0-9a-fA-F]+\b|\b\d+\b", "<n>", message)
    return message.strip()[:160]
