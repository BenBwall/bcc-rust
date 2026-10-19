"""Tests for the delta reducer; python scripts/test_reduce_conformance.py."""

import contextlib
import io
import time
import unittest

import reduce_conformance as reducer


class GroupTests(unittest.TestCase):
    def test_groups_are_balanced_largest_first_and_skip_strings(self):
        text = 'f(a, (b + c)) + g(")")'
        spans = reducer.groups(text)
        texts = [text[start:end] for start, end in spans]
        self.assertEqual(texts[0], "(a, (b + c))")
        self.assertIn("(b + c)", texts)
        self.assertIn('(")")', texts)
        self.assertTrue(all(text[start:end].count("(") == text[start:end].count(")") or '")"' in text[start:end]
                            for start, end in spans))


class PassTests(unittest.TestCase):
    def oracle(self, needle):
        calls = []

        def interesting(text):
            calls.append(text)
            return needle in text
        interesting.calls = calls
        return interesting

    def test_line_pass_keeps_only_the_lines_the_predicate_needs(self):
        source = "".join(f"line {i}\n" for i in range(40))
        oracle = self.oracle("line 17")
        with contextlib.redirect_stderr(io.StringIO()):
            reduced = reducer.line_pass(oracle, source, 2, time.monotonic() + 30)
        self.assertEqual(reduced, "line 17\n")

    def test_group_pass_replaces_parenthesized_expressions_with_constants(self):
        source = "int x = f((1 + 2) * (3 + 4)) + g(5);\n"
        oracle = self.oracle("int x = ")
        with contextlib.redirect_stderr(io.StringIO()):
            reduced = reducer.group_pass(oracle, source, 2, time.monotonic() + 30)
        self.assertNotIn("1 + 2", reduced)
        self.assertNotIn("3 + 4", reduced)
        self.assertLess(len(reduced), len(source))

    def test_parallel_first_returns_the_first_interesting_candidate_in_order(self):
        found = reducer.parallel_first(lambda text: text.endswith("!"), ["a", "b!", "c!", "d"], 2)
        self.assertEqual(found, "b!")
        self.assertIsNone(reducer.parallel_first(lambda text: False, ["a", "b"], 2))


if __name__ == "__main__":
    unittest.main()
