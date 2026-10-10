"""Exercise both MCP client entries without running the real CodeGraph CLI."""

import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[1]


class CodeGraphLauncherTests(unittest.TestCase):
    def setUp(self):
        with (ROOT / ".codex/config.toml").open("rb") as source:
            codex = tomllib.load(source)["mcp_servers"]["codegraph"]
        claude = json.loads((ROOT / ".mcp.json").read_text())["mcpServers"]["codegraph"]
        self.configurations = {"codex": codex, "claude": claude}

        temporary = tempfile.TemporaryDirectory(prefix="codegraph checkout ")
        self.addCleanup(temporary.cleanup)
        self.parent = Path(temporary.name).resolve()
        self.checkout = self.parent / "worktree"
        self.checkout.mkdir()
        subprocess.run(["git", "init", "--quiet", str(self.checkout)], check=True)
        self.nested = self.checkout / "nested working directory"
        self.nested.mkdir()
        launcher = self.checkout / "scripts/codegraph_mcp.py"
        launcher.parent.mkdir()
        shutil.copyfile(ROOT / "scripts/codegraph_mcp.py", launcher)
        # An interrupted init directory must not be enough to start the server.
        (self.checkout / ".codegraph").mkdir()

        self.bin = self.parent / "bin"
        self.bin.mkdir()
        stand_in = self.bin / "stand_in_cli.py"
        stand_in.write_text(
            "import json, os, pathlib, sys\n"
            "record = pathlib.Path(os.environ['CODEGRAPH_RECORD'])\n"
            "observed = {'argv': sys.argv[1:], 'cwd': os.getcwd()}\n"
            "if sys.argv[1:3] == ['status', '--json']:\n"
            "    observed['stdin'] = sys.stdin.read()\n"
            "    record.with_suffix('.status').write_text(json.dumps(observed))\n"
            "    print(os.environ['CODEGRAPH_STATUS_OUTPUT'])\n"
            "    print('status stderr must stay private', file=sys.stderr)\n"
            "    sys.exit(int(os.environ['CODEGRAPH_STATUS_EXIT']))\n"
            "record.write_text(json.dumps(observed))\n"
            "print('MCP stream')\n"
            "sys.exit(int(os.environ['CODEGRAPH_SERVE_EXIT']))\n",
            encoding="utf-8", newline="\n",
        )
        (self.bin / "codegraph.cmd").write_text(
            f'@echo off\n"{sys.executable}" "%~dp0stand_in_cli.py" %*\n',
            encoding="utf-8", newline="\n",
        )
        wrapper = self.bin / "codegraph"
        wrapper.write_text(
            f"#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(stand_in))} \"$@\"\n",
            encoding="utf-8", newline="\n",
        )
        wrapper.chmod(0o755)
        git = Path(shutil.which("git")).resolve()
        if os.name == "nt":
            # Git for Windows needs its adjacent DLLs; npm shims live elsewhere.
            self.git_path = str(git.parent)
        else:
            # Isolate PATH even if CodeGraph is installed alongside Git.
            (self.bin / "git").symlink_to(git)
            self.git_path = str(self.bin)
        self.record = self.parent / "serve.json"

    def check_clients(self, output, error=None, status_exit=0, missing=False):
        if missing:
            (self.bin / "codegraph").unlink()
            (self.bin / "codegraph.cmd").unlink()
        environment = {
            **os.environ,
            "PATH": os.pathsep.join([str(self.bin), self.git_path]),
            "CODEGRAPH_STATUS_OUTPUT": output,
            "CODEGRAPH_STATUS_EXIT": str(status_exit),
            "CODEGRAPH_SERVE_EXIT": "37",
            "CODEGRAPH_RECORD": str(self.record),
        }
        if missing:
            self.assertIsNone(shutil.which("codegraph", path=environment["PATH"]))
        for name, configuration in self.configurations.items():
            with self.subTest(client=name):
                self.record.unlink(missing_ok=True)
                self.record.with_suffix(".status").unlink(missing_ok=True)
                result = subprocess.run(
                    [sys.executable, *configuration["args"]], cwd=self.nested,
                    env=environment, text=True, capture_output=True,
                    input="MCP client input must not reach status\n",
                )
                if error:
                    self.assert_refused(result, error)
                else:
                    self.assertEqual(result.returncode, 37, result.stderr)
                    self.assertEqual(result.stdout, "MCP stream\n")
                    self.assertEqual(result.stderr, "")
                    observed = json.loads(self.record.read_text())
                    self.assertEqual(observed["argv"][:3], ["serve", "--mcp", "--path"])
                    self.assertEqual(len(observed["argv"]), 4)
                    self.assertEqual(Path(observed["argv"][3]), self.checkout)
                    self.assertEqual(Path(observed["cwd"]), self.nested)
                if missing:
                    self.assertFalse(self.record.with_suffix(".status").exists())
                if not missing:
                    observed = json.loads(self.record.with_suffix(".status").read_text())
                    self.assertEqual(observed["argv"][:2], ["status", "--json"])
                    self.assertEqual(Path(observed["argv"][2]), self.checkout)
                    self.assertEqual(Path(observed["cwd"]), self.nested)
                    self.assertEqual(observed["stdin"], "")

    def assert_refused(self, result, message):
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.record.exists())
        self.assertEqual(result.stdout, "")
        self.assertEqual(len(result.stderr.splitlines()), 1)
        self.assertIn(message, result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_client_entries_are_identical(self):
        codex, claude = self.configurations.values()
        self.assertEqual(codex["command"], "python")
        self.assertEqual(claude["command"], codex["command"])
        self.assertEqual(claude["args"], codex["args"])
        self.assertEqual(claude["type"], "stdio")
        self.assertIs(claude["alwaysLoad"], True)

    def test_uninitialized_checkout_is_refused(self):
        output = json.dumps({"initialized": False, "projectPath": str(self.checkout)})
        self.check_clients(output, "no CodeGraph index of its own; run codegraph init here")

    def test_ancestor_index_is_refused(self):
        output = json.dumps({"initialized": True, "projectPath": str(self.parent)})
        self.check_clients(output, "no CodeGraph index of its own; run codegraph init here")

    def test_own_index_starts_serve_and_propagates_exit_code(self):
        # Directory names are the resolver's concern, including WSL/overrides.
        (self.checkout / ".codegraph").rmdir()
        (self.checkout / ".codegraph-wsl").mkdir()
        output = json.dumps({"initialized": True, "projectPath": str(self.checkout)})
        self.check_clients(output)

    def test_missing_cli_is_refused_without_a_traceback(self):
        self.check_clients("", "CodeGraph CLI not found on PATH", missing=True)

    def test_failed_status_is_refused(self):
        self.check_clients("not JSON", "CodeGraph status failed", status_exit=9)

    def test_non_json_status_is_refused(self):
        self.check_clients("not JSON", "CodeGraph status did not return valid JSON")

    def test_invalid_status_fields_are_refused(self):
        for status in [[], {}, {"initialized": "true", "projectPath": str(self.checkout)},
                       {"initialized": True, "projectPath": None}]:
            with self.subTest(status=status):
                self.check_clients(json.dumps(status), "no CodeGraph index of its own")


if __name__ == "__main__":
    unittest.main()
