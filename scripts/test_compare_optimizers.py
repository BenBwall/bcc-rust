"""Tests for the optimizer comparison harness's statistics, arms and summary.

Run from the repository root with
`python -m unittest discover -s scripts -p "test_compare_optimizers.py"`.
"""

from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

import compare_optimizers as harness

PASSES = ["promote", "fold", "dce"]


def run_record(benchmark, arm, rep, seconds, ok=True):
    return {
        "kind": "run", "benchmark": benchmark, "arm": arm, "rep": rep, "compile_s": 1.0,
        "run_s": seconds, "cpu_user_s": None, "cpu_sys_s": None, "exit": 0, "timed_out": False,
        "output_ok": ok,
    }


def meta_record(arms):
    return {"kind": "meta", "arms": {a: {} for a in arms}, "reps": 3, "metric": "wall",
            "target": "t", "compiler_commit": "abc", "host": "h"}


class StatisticsTests(unittest.TestCase):
    def test_geometric_mean(self):
        self.assertAlmostEqual(harness.geometric_mean([1.0, 4.0]), 2.0)
        self.assertAlmostEqual(harness.geometric_mean([3.0]), 3.0)

    def test_percentile_interpolates(self):
        values = [1.0, 2.0, 3.0, 4.0, 5.0]
        self.assertEqual(harness.percentile(values, 0.0), 1.0)
        self.assertEqual(harness.percentile(values, 0.5), 3.0)
        self.assertAlmostEqual(harness.percentile(values, 0.125), 1.5)

    def test_identical_samples_give_a_unit_ratio(self):
        samples = {"a": [1.0, 1.1, 0.9], "b": [2.0, 2.2, 1.8]}
        point, low, high, count = harness.ratio_summary(samples, samples, 200, seed=1)
        self.assertAlmostEqual(point, 1.0)
        self.assertEqual(count, 2)
        self.assertLessEqual(low, 1.0)
        self.assertGreaterEqual(high, 1.0)

    def test_a_uniform_slowdown_has_a_tight_interval(self):
        base = {"a": [1.0, 1.0, 1.0], "b": [4.0, 4.0, 4.0]}
        slow = {"a": [2.0, 2.0, 2.0], "b": [8.0, 8.0, 8.0]}
        point, low, high, _ = harness.ratio_summary(slow, base, 100, seed=1)
        self.assertEqual((point, low, high), (2.0, 2.0, 2.0))

    def test_the_bootstrap_is_deterministic(self):
        a = {"x": [1.0, 1.5, 1.2, 1.1]}
        b = {"x": [1.0, 1.0, 1.3, 1.2]}
        self.assertEqual(harness.ratio_summary(a, b, 100, seed=5), harness.ratio_summary(a, b, 100, seed=5))

    def test_only_shared_benchmarks_count(self):
        arm = {"a": [1.0], "b": [1.0]}
        base = {"a": [2.0], "c": [1.0]}
        point, _, _, count = harness.ratio_summary(arm, base, 10, seed=1)
        self.assertEqual((point, count), (0.5, 1))
        self.assertIsNone(harness.ratio_summary({"a": [1.0]}, {"b": [1.0]}, 10, seed=1))


class ArmTests(unittest.TestCase):
    def test_the_standard_arms_match_the_documented_flags(self):
        self.assertEqual(harness.STANDARD_ARMS["A0"], ("--llvm-opt=O0",))
        self.assertEqual(harness.STANDARD_ARMS["N"], ("--llvm-opt=O2", "--llvm-codegen-only"))
        self.assertEqual(harness.STANDARD_ARMS["A1"], ("--opt=bcc", "--llvm-opt=O2", "--llvm-codegen-only"))
        self.assertEqual(harness.STANDARD_ARMS["A2"], ("--llvm-opt=O2",))
        self.assertEqual(harness.STANDARD_ARMS["A3"], ("--opt=bcc", "--llvm-opt=O2"))
        self.assertIsNone(harness.STANDARD_ARMS["R"])

    def test_n_is_selected_by_default_and_can_be_selected_alone(self):
        self.assertEqual(harness.parse_arguments([]).arms, "A0,N,A1,A2,A3,B0,B1,R")
        self.assertEqual(harness.STANDARD_ARMS["B1"], ("--opt=bcc", "--backend=native"))
        self.assertEqual(harness.parse_arguments(["--arms", "N"]).arms, "N")

    def test_n_builds_unoptimized_ir_with_codegen_only_for_both_workloads(self):
        with tempfile.TemporaryDirectory() as directory:
            exe = Path(directory) / "x.exe"
            exe.touch()
            tools = harness.Tools(Path("bcc-rust"), Path("clang"), "triple", 30)
            completed = subprocess.CompletedProcess([], 0, b"", b"")
            for defines in ([], [harness.REDUCED_DEFINE]):
                with self.subTest(defines=defines), patch.object(
                    harness, "run_command", return_value=(1.0, completed)
                ) as run:
                    self.assertEqual(tools.build(harness.STANDARD_ARMS["N"], Path("x.c"), exe, defines),
                                     (1.0, None))
                    run.assert_called_once_with([
                        "bcc-rust", "x.c", "--target=triple", *defines, "-o", str(exe),
                        "--llvm-opt=O2", "--llvm-codegen-only",
                    ], 30)

    def test_n_is_recorded_in_json_metadata_checks_and_timed_runs(self):
        args = harness.parse_arguments(["--arms", "N", "--reps", "1"])
        tools = Mock(bcc=Path("bcc-rust"), clang=Path("clang"), target="triple")
        tools.build.return_value = (1.0, None)
        tools.interpret.return_value = (0, b"42\n")
        timer = Mock()
        timer.run.return_value = (0.5, None, None, 0, b"42\n", False)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            path = output / "results.jsonl"
            results = harness.Results(path, {})
            try:
                with patch.object(harness, "Timer", return_value=timer), patch.object(
                    harness, "capture", return_value="clang version"
                ), patch.object(harness, "startup_floor", return_value=0.01):
                    harness.run_benchmarks(args, tools, {"N": harness.STANDARD_ARMS["N"]},
                                           [Path("x.c")], output, results)
            finally:
                results.file.close()
            records = harness.load_records(path)
        self.assertEqual(records[0]["arms"]["N"]["flags"], ["--llvm-opt=O2", "--llvm-codegen-only"])
        n_records = [r for r in records if r.get("arm") == "N"]
        self.assertEqual([r["kind"] for r in n_records], ["interpreter_check", "verify", "run"])
        self.assertTrue(all(r["output_ok"] for r in n_records))
        self.assertEqual(n_records[-1]["run_s"], 0.5)

    def test_leave_one_out_drops_each_pass_in_every_round(self):
        arms = harness.ablation_arms("leave-one-out", PASSES, 2)
        self.assertEqual(list(arms), ["A1-all", "A1-no-promote", "A1-no-fold", "A1-no-dce"])
        self.assertIn("--passes=promote,fold,dce,promote,fold,dce", arms["A1-all"])
        self.assertIn("--passes=promote,dce,promote,dce", arms["A1-no-fold"])
        self.assertIn("--llvm-codegen-only", arms["A1-no-fold"])

    def test_prefix_arms_start_with_no_bcc_pass(self):
        arms = harness.ablation_arms("prefix", PASSES, 1)
        self.assertEqual(list(arms), ["A1-all", "A1-prefix-0", "A1-prefix-1", "A1-prefix-2"])
        self.assertEqual(arms["A1-prefix-0"], ("--llvm-opt=O2", "--llvm-codegen-only"))
        self.assertIn("--passes=promote,fold", arms["A1-prefix-2"])

    def test_too_many_rounds_exit(self):
        with self.assertRaises(SystemExit):
            harness.ablation_arms("leave-one-out", PASSES, harness.MAX_PASS_LIST)


class VerificationTests(unittest.TestCase):
    def test_the_reference_is_arm_r_when_it_passed(self):
        verified = {"A2": (None, 0, 0, b"a", False, None), "R": (None, 0, 0, b"r", False, None)}
        self.assertEqual(harness.reference_output(verified), b"r")

    def test_the_reference_skips_arms_that_failed(self):
        verified = {"A0": (None, 0, 1, b"bad", False, None), "A2": (None, 0, 0, b"good", False, None)}
        self.assertEqual(harness.reference_output(verified), b"good")
        self.assertIsNone(harness.reference_output({"A0": (None, 0, 1, b"x", False, None)}))

    def test_windows_line_ends_are_made_plain(self):
        self.assertEqual(harness.plain(b"a\r\nb\n"), b"a\nb\n")


class SummaryTests(unittest.TestCase):
    def control_records(self):
        records = [meta_record(["N", "A1", "A2"])]
        for benchmark, n, a1, a2 in [("fast", 2.0, 1.0, 0.5), ("slow", 8.0, 2.0, 1.0)]:
            for rep in range(3):
                for arm, seconds in [("N", n), ("A1", a1), ("A2", a2)]:
                    records.append(run_record(benchmark, arm, rep, seconds))
        return records

    def test_n_has_baseline_ratios_and_a_separate_a1_over_n_comparison(self):
        text = harness.summarize(self.control_records(), 50, 1, "A2", "wall")
        baseline, control = text.split("## Bcc optimizer ratios (A1/N)")
        self.assertIn("## Ratios to A2", baseline)
        self.assertIn("| N", baseline)
        self.assertIn("4.000", baseline)
        self.assertIn("8.000", baseline)
        self.assertIn("0.500", control)
        self.assertIn("0.250", control)
        self.assertIn("0.354", control)  # geometric mean of 0.5 and 0.25
        self.assertIn("[0.354, 0.354]", control)
        self.assertIn("2/2", control)

    def test_a1_over_n_does_not_depend_on_the_selected_baseline(self):
        for baseline in ("A1", "N", "R"):
            with self.subTest(baseline=baseline):
                text = harness.summarize(self.control_records(), 50, 1, baseline, "wall")
                control = text.split("## Bcc optimizer ratios (A1/N)")[1]
                self.assertIn("relative to N", control)
                self.assertIn("0.354", control)

    def test_a1_over_n_uses_only_shared_valid_measurements(self):
        records = self.control_records()
        for r in records:
            if r.get("arm") == "N" and r.get("benchmark") == "slow":
                r["output_ok"] = False
        text = harness.summarize(records, 50, 1, "A2", "wall")
        control = text.split("## Bcc optimizer ratios (A1/N)")[1].split("## Compile time")[0]
        self.assertIn("| slow", control)
        self.assertIn("| slow      |     - |", control)
        self.assertIn("| A1  |          1 |   0.500 |", control)

    def test_a_failed_n_is_reported_without_a_ratio(self):
        records = [r for r in self.control_records() if r.get("arm") != "N"]
        records.append({"kind": "build_failed", "benchmark": "fast", "arm": "N",
                        "error": "build timed out"})
        text = harness.summarize(records, 50, 1, "A2", "wall")
        self.assertIn("Baseline arm N has no valid measurements.", text)
        self.assertIn("build timed out", text)

    def records(self):
        records = [meta_record(["A2", "A3"])]
        for rep, (a2, a3) in enumerate([(1.0, 0.5), (1.1, 0.6), (0.9, 0.55)]):
            records.append(run_record("fast", "A2", rep, a2))
            records.append(run_record("fast", "A3", rep, a3))
        records.append(run_record("slow", "A2", 0, 2.0))
        records.append(run_record("slow", "A3", 0, 100.0, ok=False))
        records.append({"kind": "verify", "benchmark": "slow", "arm": "A3", "output_ok": False,
                        "reason": "stdout differs from the reference"})
        return records

    def test_the_summary_reports_ratios_and_excludes_bad_runs(self):
        text = harness.summarize(self.records(), 100, 1, "A2", "wall")
        self.assertIn("## Ratios to A2", text)
        self.assertNotIn("A1/N", text)  # historical or subset results without N
        self.assertIn("| fast", text)
        self.assertIn("0.550", text)  # median of A3 over median of A2 on "fast"
        # "slow" has no valid A3 sample, so only "fast" counts.
        self.assertIn("| A3  |          1 |", text)
        self.assertIn("stdout differs from the reference", text)
        self.assertIn("run 0 wrong output", text)

    def test_ablation_arms_get_their_own_baseline(self):
        records = [meta_record(["A1-all", "A1-no-dce"])]
        for rep in range(3):
            records.append(run_record("b", "A1-all", rep, 1.0))
            records.append(run_record("b", "A1-no-dce", rep, 1.5))
        text = harness.summarize(records, 50, 1, "A2", "wall")
        self.assertIn("## Ablation ratios to A1-all", text)
        self.assertIn("1.500", text)


if __name__ == "__main__":
    unittest.main()
