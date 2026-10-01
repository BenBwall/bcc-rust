"""Regression tests for the corpus runner's diagnostic classifier.

Run from the repository root with `python3 -m unittest scripts/test_run_gcc_torture.py`.
"""

import unittest

from run_gcc_torture import bcc_status


def result(stderr, exit_code=0):
    return {"exit": exit_code, "timeout": False, "stdout": "", "stderr": stderr}


class BccStatusTests(unittest.TestCase):
    def test_rendered_error_is_a_diagnostic(self):
        status = bcc_status(result(
            "warning: no newline at end of file\n"
            " --> a.c:1:11\n"
            "error: expected an expression, found `;`\n"
            " --> a.c:1:9\n"
            "  |\n"
            "1 | int c = ;\n"
            "  |         ^ expected an expression\n"
            "\n"
            "1 error and 1 warning generated.\n"
        ))
        self.assertEqual(status, {
            "status": "diagnostics", "errors": 1, "warnings": 1,
            "first_error": "expected an expression, found `;`",
        })

    def test_warning_only_output_is_accepted(self):
        status = bcc_status(result("warning: extra tokens\n --> a.c:1:1\n"))
        self.assertEqual((status["status"], status["warnings"]), ("accepted", 1))

    def test_capitalized_headings_remain_compatible(self):
        status = bcc_status(result("Error: Unknown token at 0:[SourceVector]\n"))
        self.assertEqual((status["status"], status["first_error"]),
                         ("diagnostics", "Unknown token"))

    def test_summary_line_is_not_a_diagnostic(self):
        status = bcc_status(result("2 errors generated.\n"))
        self.assertEqual((status["status"], status["errors"]), ("accepted", 0))


if __name__ == "__main__":
    unittest.main()
