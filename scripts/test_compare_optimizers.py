"""Tests for the optimizer comparison harness's statistics, arms and summary.

Run from the repository root with
`python -m unittest discover -s scripts -p "test_compare_optimizers.py"`.
"""

import unittest

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
        self.assertEqual(harness.STANDARD_ARMS["A1"], ("--opt=bcc", "--llvm-opt=O2", "--llvm-codegen-only"))
        self.assertEqual(harness.STANDARD_ARMS["A2"], ("--llvm-opt=O2",))
        self.assertEqual(harness.STANDARD_ARMS["A3"], ("--opt=bcc", "--llvm-opt=O2"))
        self.assertIsNone(harness.STANDARD_ARMS["R"])

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
