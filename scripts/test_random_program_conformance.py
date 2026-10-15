"""Tests for the random-program conformance harness; python scripts/test_random_program_conformance.py."""

import argparse
import contextlib
import io
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

import conformance_common as common
import random_program_conformance as harness


SUM1 = "sum = 1" + chr(10)


def process(stdout="", stderr="", exit_code=0, timeout=False, seconds=0.01):
    return {"stdout": stdout, "stderr": stderr, "exit": None if timeout else exit_code, "timeout": timeout,
            "seconds": seconds}


class CompareRunTests(unittest.TestCase):
    def test_equal_output_passes_even_with_different_line_ends(self):
        reference = process("checksum = 1A\n")
        self.assertEqual(common.compare_run(process("checksum = 1A\r\n"), reference), ("pass", None))

    def test_different_output_is_wrong_output_with_the_first_difference(self):
        status, detail = common.compare_run(process("a\nchecksum = 2\n"), process("a\nchecksum = 1\n"))
        self.assertEqual(status, "wrong-output")
        self.assertIn("line 2", detail)

    def test_exit_status_difference_beats_output_difference(self):
        status, detail = common.compare_run(process("x", exit_code=0xC0000005), process("x"))
        self.assertEqual((status, detail), ("abnormal-exit", "exit status 0xC0000005, reference 0"))

    def test_timeout(self):
        self.assertEqual(common.compare_run(process(timeout=True), process("x"))[0], "timeout")

    def test_normalize_message_collapses_names_and_numbers(self):
        self.assertEqual(common.normalize_message("cannot find `foo_12` at 0x1F in line 77"),
                         "cannot find <name> at <n> in line <n>")


class ClassificationTests(unittest.TestCase):
    def setUp(self):
        self.work = Path(tempfile.mkdtemp(prefix="conformance-test-", dir=common.ROOT / "target"))
        self.addCleanup(shutil.rmtree, self.work, True)
        (self.work / "t.c").write_text("int main(void) { return 0; }\n", encoding="utf-8")

    def arm(self, name, compile_result, execute_result=None, reference=None):
        calls = [compile_result] + ([execute_result] if execute_result else [])
        with patch.object(common, "invoke", side_effect=calls):
            return common.run_arm(Path("bcc"), name, self.work / "t.c", self.work,
                                  deadlines={"compile": 1, "execute": 1, "interpret": 1},
                                  reference=reference or process("ok"), command_kwargs={})

    def test_native_arm_passes_when_the_program_prints_the_reference_output(self):
        (self.work / "program.exe").write_bytes(b"MZ")
        run = self.arm("A0", process(), process("ok"))
        self.assertEqual(run["status"], "pass")

    def test_native_arm_wrong_output_and_abnormal_exit(self):
        (self.work / "program.exe").write_bytes(b"MZ")
        self.assertEqual(self.arm("A1", process(), process("bad"))["status"], "wrong-output")
        (self.work / "program.exe").write_bytes(b"MZ")
        self.assertEqual(self.arm("A2", process(), process("ok", exit_code=3))["status"], "abnormal-exit")

    def test_refusals_crashes_and_missing_executable(self):
        refusal = process(stderr="error: not yet supported by lowering: vector types\n", exit_code=1)
        run = self.arm("A0", refusal)
        self.assertEqual((run["status"], run["constructs"]), ("lowering-unsupported", ["vector types"]))
        crash = process(stderr="thread 'main' panicked at src/x.rs:1:1\n", exit_code=101)
        self.assertEqual(self.arm("A0", crash)["status"], "compiler-crash")
        self.assertEqual(self.arm("A0", process())["status"], "compiler-crash")

    def test_interpreter_arm_compares_stdout_and_reports_traps(self):
        self.assertEqual(self.arm("interpret", process("ok"))["status"], "pass")
        self.assertEqual(self.arm("interpret", process("nope"))["status"], "wrong-output")
        trap = process(stderr="error: the interpreter stopped the program: out of bounds read (in @f)\n", exit_code=70)
        run = self.arm("interpret", trap)
        self.assertEqual((run["status"], run["detail"]), ("interpreter-trap", "out of bounds read"))
        self.assertEqual(self.arm("interpret", process(timeout=True))["status"], "timeout")


def fake_config(output, **overrides):
    values = dict(output=output, bcc=Path("bcc"), arms=["interpret", "A0"],
                  deadlines=harness.DEFAULT_DEADLINES, options={"csmith": [], "yarpgen": []},
                  system_includes=[], defines=[])
    values.update(overrides)
    return argparse.Namespace(**values)


class TestProgramTests(unittest.TestCase):
    """test_program with the generator and every subprocess replaced."""

    def setUp(self):
        self.output = Path(tempfile.mkdtemp(prefix="conformance-test-", dir=common.ROOT / "target"))
        self.addCleanup(shutil.rmtree, self.output, True)

    def generate(self, seed_work):
        def fake(generator, seed, work, options):
            work.mkdir(parents=True, exist_ok=True)
            (work / "test.c").write_text("int main(void) { return 0; }\n", encoding="utf-8")
            return [work / "test.c"], [], None
        return fake

    def scripted(self, o0=SUM1, o2=SUM1, reference_exit=0):
        """A fake `invoke`: the probe passes, Clang writes an executable, the runs print o0 then o2."""
        outputs = iter([o0, o2, o0])  # Clang -O0, Clang -O2, then GCC -O0 for a disagreeing arm

        def fake(command, timeout, environment, cwd):
            if "--semantic-types" in command:
                return process()
            if str(command[0]) in (str(common.CLANG), "gcc"):
                Path(command[command.index("-o") + 1]).write_bytes(b"MZ")
                return process()
            return process(next(outputs), exit_code=reference_exit)
        return fake

    def test_frontend_refusal_is_shared_by_all_arms_without_building_a_reference(self):
        refused = process(stderr="error: invalid operand types for comparison\n", exit_code=1)
        with patch.object(harness, "generate", self.generate(None)), \
                patch.object(harness, "invoke", side_effect=[refused]) as call:
            row = harness.test_program(fake_config(self.output), "csmith", 7)
        self.assertEqual(call.call_count, 1)
        self.assertTrue(all(run["status"] == "frontend-error" and run["shared_front_end"]
                            for run in row["arms"].values()))
        self.assertFalse(row["defect_candidate"])

    def test_unreliable_reference_discards_the_program(self):
        with patch.object(harness, "generate", self.generate(None)), \
                patch.object(harness, "invoke", self.scripted(o2=SUM1.replace("1", "2"))):
            row = harness.test_program(fake_config(self.output), "csmith", 8)
        self.assertEqual(row["status"], "reference-unstable")
        self.assertTrue((self.output / "failures/csmith/8").is_dir())

    def test_reference_failure_when_clang_cannot_run_the_program(self):
        with patch.object(harness, "generate", self.generate(None)), \
                patch.object(harness, "invoke", self.scripted(reference_exit=1)):
            row = harness.test_program(fake_config(self.output), "csmith", 9)
        self.assertEqual(row["status"], "reference-failure")

    def test_wrong_arm_output_keeps_evidence_and_passing_programs_leave_nothing(self):
        def arms(passing):
            def fake(bcc, arm, source, work, **kwargs):
                bad = arm == "A0" and not passing
                return {"status": "wrong-output" if bad else "pass", "detail": "line 1" if bad else None,
                        "constructs": [], "command": [str(work)], "compile_seconds": 0.1,
                        "_compile": process("compiled"), "_execute": process("ran")}
            return fake
        with patch.object(harness, "generate", self.generate(None)), \
                patch.object(harness, "invoke", self.scripted()), \
                patch.object(harness, "run_arm", arms(False)):
            row = harness.test_program(fake_config(self.output), "csmith", 10)
        self.assertTrue(row["defect_candidate"])
        self.assertEqual(row["gcc"]["status"], "agrees-with-clang")
        evidence = self.output / row["evidence"]
        self.assertTrue((evidence / "test.c").is_file())
        self.assertTrue((evidence / "A0/compile.log").is_file())
        with patch.object(harness, "generate", self.generate(None)), \
                patch.object(harness, "invoke", self.scripted()), \
                patch.object(harness, "run_arm", arms(True)):
            row = harness.test_program(fake_config(self.output), "csmith", 11)
        self.assertFalse(row["defect_candidate"])
        self.assertFalse((self.output / "failures/csmith/11").exists())
        self.assertFalse((self.output / "work/csmith/11").exists())


class SummaryTests(unittest.TestCase):
    def rows(self):
        def row(seed, **arms):
            return {"generator": "csmith", "seed": seed, "status": "tested", "arms": {
                name: {"status": status, "detail": detail, "constructs": constructs}
                for name, (status, detail, constructs) in arms.items()}}
        passing = ("pass", None, [])
        return [
            row(1, A0=passing, A1=passing),
            row(2, A0=("wrong-output", "line 3", []), A1=("wrong-output", "line 3", [])),
            row(3, A0=passing, A1=("wrong-output", "line 9", [])),
            row(4, A0=("lowering-unsupported", "x", ["vector types"]), A1=("lowering-unsupported", "x", ["vector types"])),
            row(5, A0=("compiler-crash", "panicked at foo.rs:12:3", []), A1=("compiler-crash", "panicked at foo.rs:99:1", [])),
        ]

    def test_signatures_group_equal_defects(self):
        groups = {item["signature"]: item for item in harness.summarize(self.rows(), {})["defect_groups"]}
        self.assertEqual(groups["wrong-output in A0+A1"]["programs"], 1)
        self.assertEqual(groups["wrong-output in A1"]["programs"], 1)
        self.assertEqual(groups["compiler-crash: panicked at foo.rs:<n>:<n>"]["programs"], 1)
        self.assertNotIn("lowering-unsupported", " ".join(groups))

    def test_a_gcc_disagreement_is_marked_in_the_signature(self):
        rows = self.rows()
        rows[1]["gcc"] = {"status": "differs-from-clang"}
        signatures = {item["signature"] for item in harness.summarize(rows, {})["defect_groups"]}
        self.assertIn("wrong-output in A0+A1 (GCC differs from Clang)", signatures)

    def test_refusals_are_counted_per_program(self):
        data = harness.summarize(self.rows(), {})["generators"]["csmith"]
        self.assertEqual(data["refusals"], [("vector types", 1)])
        self.assertEqual(data["arms"]["A1"]["wrong-output"], 2)

    def test_markdown_lists_every_outcome_row(self):
        manifest = {"config": {"target": "t"}, "bcc": "b"}
        text = harness.markdown(harness.summarize(self.rows(), manifest), manifest)
        for outcome in common.OUTCOMES:
            self.assertIn(f"| {outcome} |", text)


class ResumeTests(unittest.TestCase):
    def setUp(self):
        self.output = Path(tempfile.mkdtemp(prefix="conformance-test-", dir=common.ROOT / "target")) / "run"
        self.addCleanup(shutil.rmtree, self.output.parent, True)
        self.bcc = self.output.parent / "bcc.exe"
        self.bcc.write_bytes(b"fake")

    def arguments(self, count):
        return argparse.Namespace(
            output=self.output, generators="csmith,yarpgen", count=count, seed_start=1, jobs=2, time_budget=None,
            bcc=self.bcc, arms=None, platform="windows", define=[], csmith_option=[], yarpgen_option=[],
            compile_timeout=None, execute_timeout=None, interpret_timeout=None, allow_config_change=False)

    def run_campaign(self, count):
        seen = []

        def fake_test(config, generator, seed):
            seen.append((generator, seed))
            return {"generator": generator, "seed": seed, "status": "tested",
                    "arms": {"A0": {"status": "pass", "detail": None, "constructs": []}}}
        with patch.object(harness, "test_program", fake_test), \
                patch.object(common.Platform, "system_includes", return_value=[]), \
                patch.object(harness, "tool_versions", return_value={}), \
                contextlib.redirect_stdout(io.StringIO()):
            harness.command_run(self.arguments(count))
        return seen

    def test_a_second_run_skips_finished_seeds_and_extends_the_count(self):
        first = self.run_campaign(2)
        self.assertEqual(sorted(first), [("csmith", 1), ("csmith", 2), ("yarpgen", 1), ("yarpgen", 2)])
        second = self.run_campaign(3)
        self.assertEqual(sorted(second), [("csmith", 3), ("yarpgen", 3)])
        rows = [json.loads(line) for line in (self.output / "results.jsonl").read_text().splitlines()]
        self.assertEqual(len(rows), 6)
        self.assertEqual(json.loads((self.output / "manifest.json").read_text())["resumed"], 1)
        self.assertTrue((self.output / "summary.md").is_file())

    def test_changed_configuration_is_refused(self):
        self.run_campaign(1)
        arguments = self.arguments(1)
        arguments.csmith_option = ["--no-pointers"]
        with patch.object(common.Platform, "system_includes", return_value=[]), \
                self.assertRaises(SystemExit):
            harness.command_run(arguments)


if __name__ == "__main__":
    unittest.main()
