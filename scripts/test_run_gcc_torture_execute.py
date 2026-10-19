"""Classifier regressions; python -m unittest discover -s scripts -p 'test_run_gcc_torture*.py'."""

import unittest
import os
from pathlib import Path
import sys
import time
import tempfile
from types import SimpleNamespace
from unittest.mock import patch

from run_gcc_torture_execute import classify, disagreement, invoke, summary, source_options, supported_options, survey_file, ARMS


def result(stderr="", exit_code=0, timeout=False, stdout=""):
    return {"stderr": stderr, "stdout": stdout, "exit": exit_code, "timeout": timeout}


class SourceOptionsTests(unittest.TestCase):
    def test_supported_and_unsupported_options_are_separated(self):
        options = source_options('/* { dg-options "-fgnu89-inline -std=gnu89 -w -fwrapv" } */')
        self.assertEqual(options["applied_options"], ["-fgnu89-inline", "-std=gnu89", "-w"])
        self.assertEqual(options["unsupported_options"], ["-fwrapv"])

    def test_additional_options_and_replacement_preserve_order(self):
        options = source_options('''/* { dg-additional-options "-D X=1" }
            { dg-options "-std=c99" }
            { dg-options "-std=gnu17 -fgnu89-inline -fno-gnu89-inline" }
            { dg-additional-options "-U X" } */''')
        self.assertEqual(options["applied_options"],
                         ["-std=gnu17", "-fgnu89-inline", "-fno-gnu89-inline", "-D", "X=1", "-U", "X"])

    def test_selectors_are_deferred_without_replacing_unconditional_flags(self):
        options = source_options('''/* { dg-options "-w" }
            { dg-options "-std=c89" { target { { x86_64-*-* } && ia32 } } }
            { dg-additional-options "-DSIGNAL_SUPPRESS" { target { ! signal } } } */''')
        self.assertEqual(options["applied_options"], ["-w"])
        self.assertEqual(len(options["option_directives"]), 3)
        self.assertEqual(options["option_directives"][1]["deferred_reason"], "target selector not evaluated")

    def test_quoting_joined_operands_and_language_flags(self):
        options = source_options('/* { dg-options "-I\'dir with spaces\' -std c99 -DVALUE=7 -fms-extensions -pedantic" } */')
        self.assertEqual(options["applied_options"],
                         ["-Idir with spaces", "-std", "c99", "-DVALUE=7", "-fms-extensions", "-pedantic"])

    def test_invalid_values_missing_operands_and_arm_flags_are_unsupported(self):
        self.assertEqual(supported_options(["-std=unknown", "-std", "c++17", "-O2", "-c", "-I"]),
                         ([], ["-std=unknown", "-std", "c++17", "-O2", "-c", "-I"]))
        self.assertEqual(source_options('/* { dg-options "-w')["applied_options"], [])

    def test_survey_passes_and_records_flags_per_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "test.c"
            source.write_text('/* { dg-options "-fgnu89-inline -fwrapv" } */\nint main(void) { return 0; }',
                              encoding="utf-8")
            args = SimpleNamespace(output=root / "output", bcc=Path("bcc"), timeout=1)
            with patch("run_gcc_torture_execute.ARMS", {"interpret": ["--interpret"]}), \
                    patch("run_gcc_torture_execute.invoke", return_value={**result(), "seconds": 0}) as call:
                row = survey_file(source, root, args, {})
            self.assertEqual(row["applied_options"], ["-fgnu89-inline"])
            self.assertEqual(row["unsupported_options"], ["-fwrapv"])
            command = call.call_args.args[0]
            self.assertIn("-fgnu89-inline", command)
            self.assertNotIn("-fwrapv", command)
            self.assertEqual(row["arms"]["interpret"]["command"], command)


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

    def test_native_refusals_and_invalid_assembly(self):
        run = classify(result("error: not yet supported by the native back end: f64 values (in @half)\n", 1),
                       "compile")
        self.assertEqual((run["status"], run["constructs"]), ("native-unsupported", ["f64 values"]))
        stderr = "error: cannot build `test.exe`: clang failed:\nx.exe.bcc-1-0.s:12:3: error: invalid operand\n"
        self.assertEqual(classify(result(stderr, 1), "compile")["status"], "compiler-crash")

    def test_arm_selection_restricts_the_survey(self):
        import run_gcc_torture_execute as survey
        with patch("run_gcc_torture_execute.ARMS", dict(ARMS)):
            survey.select_arms(["B0", "interpret"])
            self.assertEqual(list(survey.ARMS), ["B0", "interpret"])
            self.assertEqual(survey.ARMS["B0"], ["--backend=native"])

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
