"""Classifier regressions; python -m unittest discover -s scripts -p 'test_run_gcc_torture*.py'."""

import unittest
import os
from pathlib import Path
import sys
import time

from run_gcc_torture_execute import classify, disagreement, invoke, summary, ARMS


def result(stderr="", exit_code=0, timeout=False, stdout=""):
    return {"stderr": stderr, "stdout": stdout, "exit": exit_code, "timeout": timeout}


class ExecuteClassifierTests(unittest.TestCase):
    def test_pass_and_warning(self):
        self.assertEqual(classify(result("warning: old-style declaration\n"), "compile")["status"], "pass")

    def test_frontend_errors_even_with_zero_exit(self):
        for code in (0, 1):
            self.assertEqual(classify(result("error: undeclared identifier\n", code), "compile")["status"],
                             "frontend-error")

    def test_lowering_collects_constructs(self):
        run = classify(result("error: not yet supported by lowering: variable length arrays\n"
                              "error: not yet supported by lowering: computed goto\n", 1), "compile")
        self.assertEqual(run["status"], "lowering-unsupported")
        self.assertEqual(run["constructs"], ["computed goto", "variable length arrays"])

    def test_abi_failure(self):
        self.assertEqual(classify(result("error: va_arg of type f128 is not yet supported by ABI lowering for this target\n", 1), "compile")["status"],
                         "abi-unsupported")

    def test_opaque_type_is_a_lowering_limit(self):
        run = classify(result("error: construct not supported by code generation: its type was not analyzed\n", 1),
                       "compile")
        self.assertEqual(run["status"], "lowering-unsupported")
        self.assertEqual(run["constructs"], ["type not analyzed by semantic analysis"])

    def test_panics_override_diagnostics(self):
        self.assertEqual(classify(result("error: syntax\nthread 'main' panicked at x.rs:42\n", 101), "interpret")["status"],
                         "compiler-crash")

    def test_internal_error_is_a_crash(self):
        self.assertEqual(classify(result("error: internal error: lowering is missing a semantic fact: name\n", 1),
                                  "compile")["status"], "compiler-crash")

    def test_trap_kind_and_location(self):
        run = classify(result("error: the interpreter stopped the program: undefined behaviour: branch on poison "
                              "(in @main at inst4)\n", 70), "interpret")
        self.assertEqual(run["status"], "interpreter-trap")
        self.assertEqual(run["trap_kind"], "undefined behaviour: branch on poison")

    def test_program_status_70_is_not_a_trap(self):
        self.assertEqual(classify(result(exit_code=70), "interpret")["status"], "wrong-result")

    def test_abort_and_native_fault_are_wrong_results(self):
        for phase, code in (("interpret", 134), ("execute", 3), ("execute", 3221225477)):
            self.assertEqual(classify(result(exit_code=code), phase)["status"], "wrong-result")

    def test_native_program_diagnostic_text_is_not_a_compiler_failure(self):
        self.assertEqual(classify(result("error: test output\n", 0), "execute")["status"], "pass")
        self.assertEqual(classify(result(stdout="error: test output\n"), "interpret")["status"], "pass")

    def test_abnormal_compiler_exit(self):
        for code in (101, 3221225477):
            self.assertEqual(classify(result(exit_code=code), "compile")["status"], "compiler-crash")

    def test_timeout_precedes_partial_output(self):
        self.assertEqual(classify(result("thread 'main' panicked at x.rs\n", None, True), "compile")["status"], "timeout")

    def test_timeout_stops_descendants_holding_output_pipes(self):
        child = "import time; time.sleep(3)"
        parent = ("import subprocess,sys,time; subprocess.Popen([sys.executable, '-c', "
                  + repr(child) + "]); time.sleep(3)")
        started = time.perf_counter()
        run = invoke([sys.executable, "-c", parent], 0.3, os.environ, Path.cwd())
        self.assertTrue(run["timeout"])
        self.assertIsNone(run["exit"])
        self.assertLess(time.perf_counter() - started, 2.5)

    def test_link_error_and_invalid_llvm(self):
        for stderr, expected in (("error: cannot build `test.exe`: undefined symbol: foo\n", "link-error"),
                                 ("error: cannot build `test.exe`: x.ll:10:2: error: invalid phi\n", "compiler-crash")):
            self.assertEqual(classify(result(stderr, 1), "compile")["status"], expected)

    def test_summary_keeps_capability_disagreements_and_source_counts(self):
        arms = {arm: classify(result("error: not yet supported by lowering: this builtin\n", 1), "compile")
                for arm in ARMS}
        self.assertFalse(disagreement(arms))
        totals = summary([{"file": "a.c", "arms": arms, "disagreement": False}])
        self.assertEqual(totals["unsupported"], [("this builtin", 1)])
        arms["A0"] = classify(result(), "compile")
        self.assertTrue(disagreement(arms))
        arms["A1"] = classify(result(exit_code=3), "execute")
        totals = summary([{"file": "a.c", "arms": arms, "disagreement": True}])
        self.assertTrue(totals["disagreements"][0]["pass_vs_wrong"])


if __name__ == "__main__":
    unittest.main()
