"""Tests for the shared conformance helpers; python scripts/test_conformance_common.py."""

from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

import conformance_common as common
import random_program_conformance as harness


class InvokeRetryTests(unittest.TestCase):
    def test_a_refused_job_assignment_is_retried(self):
        answers = [OSError(5, "cannot assign a Windows process job: ['x']"), {"exit": 0}]

        def fake(*arguments):
            answer = answers.pop(0)
            if isinstance(answer, Exception):
                raise answer
            return answer
        with patch.object(common, "run_process", fake), patch.object(common.time, "sleep"):
            self.assertEqual(common.invoke(["x"], 1, {}, Path(".")), {"exit": 0})

    def test_other_os_errors_are_not_retried(self):
        with patch.object(common, "run_process", side_effect=OSError(2, "no such file")) as call:
            with self.assertRaises(OSError):
                common.invoke(["x"], 1, {}, Path("."))
        self.assertEqual(call.call_count, 1)

    def test_the_real_runner_captures_output_and_a_deadline(self):
        with tempfile.TemporaryDirectory(dir=common.ROOT / "target") as directory:
            work = Path(directory)
            done = common.invoke([common_python(), "-c", "print('hi')"], 30, common.environment(work), work)
            self.assertEqual((done["exit"], done["stdout"].strip(), done["timeout"]), (0, "hi", False))
            slow = common.invoke([common_python(), "-c", "import time; time.sleep(30)"], 0.5,
                                 common.environment(work), work)
            self.assertTrue(slow["timeout"])


def common_python():
    import sys
    return sys.executable


class CommandTests(unittest.TestCase):
    def test_bcc_command_orders_flags_and_marks_compile_only(self):
        command = common.bcc_command(Path("bcc"), Path("a.c"), arm_flags=["--llvm-opt=O2"], includes=["inc"],
                                     system_includes=["sys"], defines=["X=1"], output=Path("a.o"),
                                     compile_only=True)
        self.assertEqual(command, ["bcc", "a.c", f"--target={common.TARGET}", "-w", "-I", "inc", "-idirafter",
                                   "sys", "-D", "X=1", "--llvm-opt=O2", "-o", "a.o", "-c"])

    def test_clang_command(self):
        command = common.clang_command(Path("clang"), [Path("a.c"), Path("b.c")], Path("out.exe"),
                                       opt="-O2", includes=["inc"], defines=["Y"])
        self.assertEqual(command[:3], ["clang", f"--target={common.TARGET}", "-O2"])
        self.assertEqual(command[-4:], ["a.c", "b.c", "-o", "out.exe"])
        self.assertEqual(command[command.index("-D") + 1], "Y")

    def test_the_arms_cover_both_interpreters_and_three_native_pipelines(self):
        self.assertEqual(list(common.ARMS), ["interpret", "interpret-bcc", "A0", "A1", "A2"])
        self.assertEqual(common.NATIVE_ARMS, ["A0", "A1", "A2"])
        self.assertEqual(common.ARMS["A1"]["flags"], ["--opt=bcc", "--llvm-opt=O2", "--llvm-codegen-only"])

    def test_environment_confines_temporary_files_and_drops_include_variables(self):
        with patch.dict("os.environ", {"CPATH": "x", "C_INCLUDE_PATH": "y"}):
            env = common.environment(Path("work"))
        self.assertNotIn("CPATH", env)
        self.assertNotIn("C_INCLUDE_PATH", env)
        self.assertEqual(env["TMP"], "work")

    def test_remove_file_waits_out_a_transient_lock(self):
        path = Path(tempfile.mkdtemp(dir=common.ROOT / "target")) / "locked.exe"
        self.addCleanup(shutil.rmtree, path.parent, True)
        path.write_bytes(b"MZ")
        real = Path.unlink
        attempts = []

        def flaky(self, missing_ok=False):
            attempts.append(1)
            if len(attempts) < 3:
                raise PermissionError(5, "Access is denied")
            return real(self, missing_ok=missing_ok)
        with patch.object(Path, "unlink", flaky), patch.object(common.time, "sleep"):
            common.remove_file(path)
        self.assertFalse(path.exists())
        self.assertEqual(len(attempts), 3)


class PlatformTests(unittest.TestCase):
    def test_windows_paths_become_wsl_paths(self):
        self.assertEqual(common.to_wsl_path(r"C:\Users\me\x.c"), "/mnt/c/Users/me/x.c")

    def test_run_command_wraps_linux_programs_in_a_kill_timeout(self):
        self.assertEqual(common.WINDOWS.run_command("p.exe", 10, ["a"]), ["p.exe", "a"])
        command = common.LINUX.run_command(r"C:\work\p", 10, ["a"])
        self.assertEqual(command[:4], ["wsl.exe", "-d", common.WSL_DISTRO, "--"])
        self.assertEqual(command[4:9], ["timeout", "-s", "KILL", "12", "/mnt/c/work/p"])
        self.assertEqual(command[-1], "a")

    def test_linux_commands_convert_every_path(self):
        reference = common.LINUX.reference_command([r"C:\w\a.c"], r"C:\w\ref", "-O2", [r"C:\inc"], ["X=1"])
        self.assertEqual(reference[4:6], ["clang", "-O2"])
        self.assertIn("/mnt/c/inc", reference)
        self.assertIn("/mnt/c/w/a.c", reference)
        self.assertEqual(reference[-3:], ["-o", "/mnt/c/w/ref", "-lm"])
        obj = common.LINUX.object_command(r"C:\w\a.c", r"C:\w\a.o", [], [], extra=["-include", r"C:\w\shim.h"])
        self.assertEqual(obj[-2:], ["-include", "/mnt/c/w/shim.h"])
        link = common.LINUX.link_command([r"C:\w\a.o"], r"C:\w\p", ["-ldl"])
        self.assertEqual(link[-4:], ["/mnt/c/w/a.o", "-o", "/mnt/c/w/p", "-ldl"][-4:])

    def test_the_two_platforms_differ_in_target_and_defines(self):
        self.assertEqual(common.WINDOWS.target, common.TARGET)
        self.assertEqual(common.LINUX.target, "x86_64-unknown-linux-gnu")
        self.assertEqual(common.WINDOWS.defines(), ["__CRT__NO_INLINE"])
        self.assertEqual(common.LINUX.defines(), [])
        self.assertEqual(sorted(common.PLATFORMS), ["linux-wsl", "windows"])

    def test_gcc_second_opinion_command(self):
        command = common.WINDOWS.gcc_command([Path("a.c")], Path("g.exe"), ["inc"], ["X"])
        self.assertEqual(command[:3], ["gcc", "-O0", "-w"])
        self.assertEqual(command[-2:], ["-o", "g.exe"])
        self.assertEqual(common.LINUX.gcc_command([Path("a.c")], Path("g"), [], [])[4], "gcc")


class GuardedTests(unittest.TestCase):
    def test_a_harness_failure_becomes_a_row(self):
        output = Path(tempfile.mkdtemp(dir=common.ROOT / "target"))
        self.addCleanup(shutil.rmtree, output, True)
        config = type("Config", (), {"output": output})()
        with patch.object(harness, "test_program", side_effect=RuntimeError("boom")):
            row = harness.guarded(config, "csmith", 5)
        self.assertEqual((row["status"], row["seed"]), ("harness-error", 5))
        self.assertIn("boom", row["detail"])


if __name__ == "__main__":
    unittest.main()
