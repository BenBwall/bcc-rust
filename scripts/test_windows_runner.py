"""Protocol regressions for the sandbox Windows submission client."""

import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("windows_runner", Path(__file__).with_name("windows-runner.py"))
client = importlib.util.module_from_spec(spec)
spec.loader.exec_module(client)


class SubmissionTests(unittest.TestCase):
    def test_github_cli_scoping_and_environment(self):
        with patch.object(client.subprocess, 'run') as run:
            client.gh('run', 'list')
            self.assertEqual(run.call_args.args[0], ['gh', 'run', 'list', '--repo', client.REPOSITORY])
            self.assertEqual(run.call_args.kwargs['env']['GH_NO_UPDATE_NOTIFIER'], '1')
            client.gh('api', f'repos/{client.REPOSITORY}/actions/runners')
            self.assertNotIn('--repo', run.call_args.args[0])

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

    def test_rejected_dispatch_does_not_report_a_submission(self):
        def github(*arguments, **options):
            return subprocess.CompletedProcess(arguments, 7 if arguments[0] == "workflow" else 0, "", "")

        output = io.StringIO()
        with patch.object(sys, "argv", ["bcc-windows", "test", "--sha", "a" * 40]), \
             patch.object(client, "gh", github), contextlib.redirect_stdout(output):
            self.assertEqual(client.main(), 7)
        self.assertEqual(output.getvalue(), "")


class CheckoutTests(unittest.TestCase):
    """Exercise discovery with real Git checkouts and linked worktrees."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="bcc-win-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name).resolve()
        self.home = self.root / "home"
        self.home.mkdir()
        self.checkout = self.root / "checkout"
        self.git("-c", "init.templateDir=", "init", "-q", str(self.checkout))
        (self.checkout / "source.txt").write_text("initial source\n", encoding="utf-8")
        self.commit(self.checkout)
        self.sha = self.git("-C", str(self.checkout), "rev-parse", "HEAD").strip()
        self.default = patch.object(client, "DEFAULT_CHECKOUT", self.checkout)
        self.default.start()
        self.addCleanup(self.default.stop)

    def git(self, *arguments):
        return subprocess.run(["git", *arguments], text=True, capture_output=True,
                              check=True).stdout

    def commit(self, checkout):
        self.git("-C", str(checkout), "add", "source.txt")
        self.git("-C", str(checkout), "-c", "user.name=Runner test",
                 "-c", "user.email=runner-test@example.invalid", "-c", "commit.gpgsign=false",
                 "-c", f"core.hooksPath={self.root / 'no-hooks'}", "commit", "-qm", "Test source")

    def test_ci_wait_from_home_submits_default_checkout_head(self):
        calls = []

        def github(*arguments, **options):
            calls.append(arguments)
            output = json.dumps([{"databaseId": 42, "displayTitle": "Windows ci home-request",
                                  "url": "correct"}]) if arguments[:2] == ("run", "list") else ""
            return subprocess.CompletedProcess(arguments, 0, output, "")

        output, errors = io.StringIO(), io.StringIO()
        with contextlib.chdir(self.home), \
             patch.object(sys, "argv", ["bcc-windows", "ci", "--wait"]), \
             patch.object(client, "gh", github), \
             patch.object(client.uuid, "uuid4", return_value=types.SimpleNamespace(hex="home-request")), \
             contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
            self.assertEqual(client.main(), 0)
        dispatch = next(call for call in calls if call[:2] == ("workflow", "run"))
        self.assertIn(f"commit_sha={self.sha}", dispatch)
        self.assertIn(("run", "watch", "42", "--exit-status"), calls)
        self.assertEqual(json.loads(output.getvalue())["commit"], self.sha)
        self.assertIn(str(self.checkout), errors.getvalue())
        self.assertNotIn("fatal:", errors.getvalue())

    def test_current_worktree_takes_precedence_over_default(self):
        worktree = self.root / "worktree"
        self.git("-C", str(self.checkout), "worktree", "add", "-q", "-b", "agent-work", str(worktree))
        (worktree / "source.txt").write_text("worktree source\n", encoding="utf-8")
        self.commit(worktree)
        sha = self.git("-C", str(worktree), "rev-parse", "HEAD").strip()
        nested = worktree / "nested"
        nested.mkdir()
        with contextlib.chdir(nested):
            self.assertEqual(client.checkout_commit(), (worktree, sha))
        self.assertNotEqual(sha, self.sha)

    def test_explicit_checkout_works_from_home(self):
        with contextlib.chdir(self.home):
            self.assertEqual(client.checkout_commit("../checkout"), (self.checkout, self.sha))

    def test_dirty_checkout_cannot_be_mistaken_for_tested_source(self):
        (self.checkout / "untracked.txt").write_text("uncommitted\n", encoding="utf-8")
        errors = io.StringIO()
        with contextlib.chdir(self.home), patch.object(sys, "argv", ["bcc-windows", "test"]), \
             patch.object(client, "gh") as github, contextlib.redirect_stderr(errors):
            with self.assertRaises(SystemExit) as failure:
                client.main()
        self.assertEqual(failure.exception.code, 2)
        github.assert_not_called()
        self.assertIn("is dirty", errors.getvalue())
        self.assertIn(str(self.checkout), errors.getvalue())

    def test_missing_default_checkout_explains_how_to_select_source(self):
        errors = io.StringIO()
        with contextlib.chdir(self.home), \
             patch.object(client, "DEFAULT_CHECKOUT", self.root / "missing"), \
             patch.object(sys, "argv", ["bcc-windows", "ci"]), \
             patch.object(client, "gh") as github, contextlib.redirect_stderr(errors):
            with self.assertRaises(SystemExit) as failure:
                client.main()
        self.assertEqual(failure.exception.code, 2)
        github.assert_not_called()
        self.assertIn("--checkout PATH or --sha FULL_SHA", errors.getvalue())
        self.assertNotIn("fatal:", errors.getvalue())

    def test_explicit_sha_works_from_home_without_a_checkout(self):
        def github(*arguments, **options):
            return subprocess.CompletedProcess(arguments, 7 if arguments[0] == "workflow" else 0, "", "")

        with contextlib.chdir(self.home), \
             patch.object(sys, "argv", ["bcc-windows", "test", "--sha", self.sha]), \
             patch.object(client, "checkout_commit") as discover, patch.object(client, "gh", github):
            self.assertEqual(client.main(), 7)
        discover.assert_not_called()

    def test_inaccessible_commit_is_rejected_before_submission(self):
        errors = io.StringIO()
        with contextlib.chdir(self.home), patch.object(sys, "argv", ["bcc-windows", "ci"]), \
             patch.object(client, "gh", return_value=subprocess.CompletedProcess([], 1, "", "")) as github, \
             contextlib.redirect_stderr(errors):
            with self.assertRaises(SystemExit) as failure:
                client.main()
        self.assertIn(self.sha, str(failure.exception))
        self.assertIn("Push its branch", str(failure.exception))
        github.assert_called_once_with("api", f"repos/{client.REPOSITORY}/commits/{self.sha}",
                                       capture_output=True)


if __name__ == "__main__":
    unittest.main()
