"""Protocol regressions for the sandbox Windows submission client."""

import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import types
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("windows_runner", Path(__file__).with_name("windows-runner.py"))
client = importlib.util.module_from_spec(spec)
spec.loader.exec_module(client)


class SubmissionTests(unittest.TestCase):
    def test_argument_boundaries_and_concurrent_run_identity(self):
        calls = []

        def github(*arguments, **options):
            calls.append(arguments)
            if arguments[:2] == ("run", "list"):
                return subprocess.CompletedProcess(arguments, 0, json.dumps([
                    {"databaseId": 99, "displayTitle": "another agent", "url": "other"},
                    {"databaseId": 42, "displayTitle": "Windows test request-123", "url": "correct"},
                ]), "")
            return subprocess.CompletedProcess(arguments, 0, "", "")

        output = io.StringIO()
        cargo = ["--test", "a space;$(literal)", "--", "--exact", "unicode-å"]
        with patch.object(sys, "argv", ["bcc-windows", "test", "--sha", "a" * 40, "--", *cargo]), \
             patch.object(client, "gh", github), \
             patch.object(client.uuid, "uuid4", return_value=types.SimpleNamespace(hex="request-123")), \
             contextlib.redirect_stdout(output):
            self.assertEqual(client.main(), 0)
        dispatch = next(call for call in calls if call[:2] == ("workflow", "run"))
        payload = next(value for value in dispatch if value.startswith("cargo_args_json="))
        self.assertEqual(json.loads(payload.partition("=")[2]), cargo)
        self.assertEqual(json.loads(output.getvalue())["run_id"], 42)

    def test_wait_returns_failed_windows_job_status(self):
        def github(*arguments, **options):
            if arguments[:2] == ("run", "list"):
                return subprocess.CompletedProcess(arguments, 0, json.dumps([
                    {"databaseId": 42, "displayTitle": "Windows ci request-123", "url": "correct"},
                ]), "")
            return subprocess.CompletedProcess(arguments, 1 if arguments[:2] == ("run", "watch") else 0, "", "")

        with patch.object(sys, "argv", ["bcc-windows", "ci", "--sha", "a" * 40, "--wait"]), \
             patch.object(client, "gh", github), \
             patch.object(client.uuid, "uuid4", return_value=types.SimpleNamespace(hex="request-123")), \
             contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(client.main(), 1)

    def test_dirty_checkout_cannot_be_mistaken_for_tested_source(self):
        with patch.object(sys, "argv", ["bcc-windows", "test"]), \
             patch.object(client.subprocess, "check_output", side_effect=["a" * 40, " M src/lib.rs"]), \
             patch.object(client, "gh") as github, contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as failure:
                client.main()
            self.assertEqual(failure.exception.code, 2)
            github.assert_not_called()

    def test_rejected_dispatch_does_not_report_a_submission(self):
        def github(*arguments, **options):
            return subprocess.CompletedProcess(arguments, 7 if arguments[0] == "workflow" else 0, "", "")

        output = io.StringIO()
        with patch.object(sys, "argv", ["bcc-windows", "test", "--sha", "a" * 40]), \
             patch.object(client, "gh", github), contextlib.redirect_stdout(output):
            self.assertEqual(client.main(), 7)
        self.assertEqual(output.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
