"""Tests for the m4 subset in mini_m4.py; python -m unittest discover -s scripts -p 'test_mini_m4.py'."""

import unittest

from mini_m4 import MiniM4


def expand(text):
    return MiniM4().run(text)


class MiniM4Tests(unittest.TestCase):
    def test_text_without_macros_passes_through(self):
        self.assertEqual(expand("int x = 1; /* keep */\n"), "int x = 1; /* keep */\n")

    def test_dnl_discards_the_rest_of_the_line_and_its_newline(self):
        self.assertEqual(expand("a dnl comment\nb\n"), "a b\n")

    def test_define_with_parameters(self):
        self.assertEqual(expand("define(`twice',`$1+$1')twice(x)\n"), "x+x\n")
        self.assertEqual(expand("define(`pair',`[$2,$1]')pair(a,b)\n"), "[b,a]\n")

    def test_arguments_are_expanded_before_substitution(self):
        self.assertEqual(expand("define(`one',`1')define(`id',`$1')id(one)\n"), "1\n")

    def test_quotes_stop_expansion_and_are_stripped_once(self):
        self.assertEqual(expand("define(`x',`y')`x' x\n"), "x y\n")

    def test_ifelse_picks_the_matching_branch(self):
        self.assertEqual(expand("ifelse(a,a,yes,no) ifelse(a,b,yes,no) ifelse(a,b,yes)|\n"), "yes no |\n")
        self.assertEqual(expand("ifelse(a,b,1,c,c,2,3)\n"), "2\n")

    def test_hash_comments_are_not_expanded(self):
        self.assertEqual(expand("define(`x',`y')# x stays\nx\n"), "# x stays\ny\n")

    def test_nested_macro_in_a_macro_body_is_rescanned(self):
        text = "define(`inner',`<$1>')define(`outer',`inner($1)!')outer(v)\n"
        self.assertEqual(expand(text), "<v>!\n")

    def test_parameter_count_and_empty_missing_arguments(self):
        self.assertEqual(expand("define(`n',`$#:$2:')n(a)n(a,b,c)\n"), "1::3:b:\n")

    def test_generated_header_shape(self):
        # The shape Csmith's safe_math.m4 relies on: a macro that defines functions per type.
        text = ("define(`safe_op',`static int $1_$2(int a) { return a + $3; }')\n"
                "safe_op(add,s,1)\nsafe_op(sub,u,2)\n")
        output = expand(text)
        self.assertIn("static int add_s(int a) { return a + 1; }", output)
        self.assertIn("static int sub_u(int a) { return a + 2; }", output)
        self.assertNotIn("define(", output)


if __name__ == "__main__":
    unittest.main()
