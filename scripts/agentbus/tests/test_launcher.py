"""Launcher checks that never build agentbus or interact with its database."""

import importlib.util
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
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[3]


class LauncherTests(unittest.TestCase):
    def test_cargo_gets_literal_arguments_and_isolated_flags(self):
        with tempfile.TemporaryDirectory(prefix="agentbus checkout ") as temporary:
            checkout = Path(temporary)
            launcher = checkout / "scripts" / "agentbus" / "run.py"
            launcher.parent.mkdir(parents=True)
            shutil.copyfile(ROOT / "scripts/agentbus/run.py", launcher)
            spec = importlib.util.spec_from_file_location("agentbus_launcher", launcher)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            nested = checkout / "nested working directory"
            nested.mkdir()
            arguments = ["send", "--subject", "a 'quoted' subject", "$literal; text", "é"]
            environment = {
                "RUSTFLAGS": "-Clinker-plugin-lto",
                "CARGO_ENCODED_RUSTFLAGS": "-Clinker-plugin-lto\x1f-Clto",
                "AGENTBUS_DB": "custom path/bus.db",
                "AGENTBUS_AGENT": "reviewer",
                "PATH": os.environ.get("PATH", ""),
            }
            original_cwd = Path.cwd()
            try:
                os.chdir(nested)
                with patch.dict(os.environ, environment, clear=True):
                    with patch.object(module.subprocess, "run") as cargo:
                        cargo.return_value.returncode = 37
                        with patch.object(sys, "argv", [str(launcher), *arguments]):
                            self.assertEqual(module.main(), 37)
                        command, = cargo.call_args.args
                        self.assertEqual(command, [
                            "cargo", "run", "--quiet", "--manifest-path",
                            str(launcher.with_name("Cargo.toml")), "--", *arguments,
                        ])
                        expected_environment = dict(environment)
                        expected_environment.update(RUSTFLAGS="", CARGO_ENCODED_RUSTFLAGS="")
                        # No shell, cwd, or stdio override: Cargo receives literal
                        # arguments and inherits the caller's streams/directory.
                        self.assertEqual(cargo.call_args.kwargs, {"env": expected_environment})
                        self.assertEqual(dict(os.environ), environment)
                        self.assertEqual(Path.cwd(), nested)
            finally:
                os.chdir(original_cwd)

    def test_client_entries_find_the_checkout_from_a_nested_directory(self):
        configurations = []
        with (ROOT / ".codex/config.toml").open("rb") as source:
            codex = tomllib.load(source)["mcp_servers"]["agentbus"]
        configurations.append((codex["args"], codex["env"]))
        claude = json.loads((ROOT / ".mcp.json").read_text())["mcpServers"]["agentbus"]
        configurations.append((claude["args"], claude["env"]))
        self.assertEqual(codex["command"], "python")
        self.assertEqual(claude["command"], "python")
        for filename in [".codex/hooks.json", ".claude/settings.json"]:
            hooks = json.loads((ROOT / filename).read_text())["hooks"]
            for groups in hooks.values():
                for group in groups:
                    for hook in group["hooks"]:
                        command = shlex.split(hook["command"])
                        self.assertEqual(command[0], "python")
                        configurations.append((command[1:], {}))

        with tempfile.TemporaryDirectory(prefix="agentbus checkout ") as temporary:
            checkout = Path(temporary).resolve()
            subprocess.run(["git", "init", "--quiet", str(checkout)], check=True)
            nested = checkout / "nested working directory"
            nested.mkdir()
            launcher = checkout / "scripts/agentbus/run.py"
            launcher.parent.mkdir(parents=True)
            # A stand-in observes which checkout was selected without running
            # Cargo, enabling hooks, polling messages, or opening the database.
            launcher.write_text(
                "import json, os, sys\n"
                "print(json.dumps({'launcher': __file__, 'args': sys.argv[1:], "
                "'cwd': os.getcwd(), 'agent': os.environ.get('AGENTBUS_AGENT')}))\n"
            )
            for arguments, configured_env in configurations:
                with self.subTest(arguments=arguments):
                    result = subprocess.run(
                        [sys.executable, *arguments], cwd=nested,
                        env={**os.environ, **configured_env},
                        text=True, capture_output=True, check=True,
                    )
                    observed = json.loads(result.stdout)
                    self.assertEqual(Path(observed["launcher"]), launcher)
                    self.assertEqual(observed["args"], arguments[2:])
                    self.assertEqual(Path(observed["cwd"]), nested)
                    if configured_env:
                        self.assertEqual(observed["agent"], configured_env["AGENTBUS_AGENT"])


if __name__ == "__main__":
    unittest.main()
