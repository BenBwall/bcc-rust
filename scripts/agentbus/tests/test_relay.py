"""Exercise the filesystem transport without Rust or the real agentbus database."""

from concurrent.futures import ThreadPoolExecutor
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location("agentbus_relay", Path(__file__).resolve().parents[1] / "relay.py")
relay = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(relay)


class RelayTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="agentbus relay ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.directory = self.root / "relay"
        self.database = self.root / "host only.db"
        self.stop = threading.Event()
        self.thread = threading.Thread(target=relay.serve, args=(
            self.directory, sys.executable, self.root, self.database,
            ["bcc-rust", "codex-bcc-rust"], self.stop,
        ))
        self.thread.start()
        self.addCleanup(self.shutdown)
        deadline = time.monotonic() + 5
        while not (self.directory / "heartbeat").is_file():
            if time.monotonic() > deadline:
                self.fail("relay did not start")
            time.sleep(0.02)

    def shutdown(self):
        self.stop.set()
        self.thread.join(timeout=5)
        self.assertFalse(self.thread.is_alive())

    def test_literal_unicode_arguments_input_output_exit_and_environment(self):
        script = (
            "import json, os, sys; "
            "sys.stdin.reconfigure(encoding='utf-8'); sys.stderr.reconfigure(encoding='utf-8'); "
            "print(json.dumps({'args': sys.argv[1:], 'stdin': sys.stdin.read(), "
            "'agent': os.environ.get('AGENTBUS_AGENT'), "
            "'session': os.environ.get('CODEX_THREAD_ID'), 'db': os.environ['AGENTBUS_DB']})); "
            "print('error é', file=sys.stderr); sys.exit(7)"
        )
        literal = ["a 'quoted' subject", "$literal; text", "é"]
        with patch.dict(os.environ, {"AGENTBUS_AGENT": "codex", "CODEX_THREAD_ID": "session-1"}):
            result = relay.exchange(self.directory, ["-c", script, *literal], "input é\n", timeout=5)
        observed = json.loads(result["stdout"])
        self.assertEqual(observed, {
            "args": literal, "stdin": "input é\n", "agent": "codex",
            "session": "session-1", "db": str(self.database),
        })
        self.assertEqual(result["returncode"], 7)
        self.assertEqual(result["stderr"], "error é\n")
        self.assertEqual(list((self.directory / "requests").iterdir()), [])
        self.assertEqual(list((self.directory / "responses").iterdir()), [])

    def test_waiting_hook_does_not_block_other_requests(self):
        started = self.root / "started"
        script = f"import pathlib,time; pathlib.Path({str(started)!r}).touch(); time.sleep(1); print('hook')"
        with ThreadPoolExecutor(max_workers=2) as clients:
            slow = clients.submit(relay.exchange, self.directory, ["-c", script], "", 5)
            deadline = time.monotonic() + 3
            while not started.exists():
                self.assertLess(time.monotonic(), deadline)
                time.sleep(0.02)
            fast = relay.exchange(self.directory, ["-c", "print('send')"], "", timeout=5)
            self.assertEqual(fast["stdout"], "send\n")
            self.assertFalse(slow.done())
            self.assertEqual(slow.result()["stdout"], "hook\n")

    def test_dead_host_fails_without_publishing_or_falling_back(self):
        self.shutdown()
        with self.assertRaisesRegex(OSError, "unavailable"):
            relay.exchange(self.directory, ["where"], "", timeout=1)
        self.assertEqual(list((self.directory / "requests").iterdir()), [])

    def test_stale_heartbeat_fails(self):
        relay.publish(self.directory / "heartbeat", {"timestamp": time.time() - 100, "instance": "old"})
        with self.assertRaisesRegex(OSError, "stopped"):
            relay.check_heartbeat(self.directory)

    def test_abandoned_request_does_not_execute(self):
        request = self.directory / "requests" / "cancelled.running"
        response = self.directory / "responses" / "cancelled.json"
        relay.publish(request, {"args": ["-c", "raise SystemExit('must not execute')"]})
        with patch.object(relay.subprocess, "run") as run:
            relay.respond(request, response, sys.executable, self.root, self.database)
        run.assert_not_called()
        self.assertFalse(request.exists())
        self.assertFalse(response.exists())

    def test_relay_autoselects_only_the_registered_sandboxes(self):
        config = {"sandboxes": ["bcc-rust", "codex-bcc-rust"]}
        directory = self.root / "common" / "agentbus" / "relay"
        directory.mkdir(parents=True)
        relay.publish(directory / "config.json", config)
        with patch.object(relay.subprocess, "check_output", return_value=str(self.root / "common")):
            for sandbox in ["bcc-rust", "codex-bcc-rust", "claude-bcc-rust", "other"]:
                with patch.dict(os.environ, {"SANDBOX_NAME": sandbox}, clear=True):
                    self.assertEqual(relay.relay_directory(self.root), directory if sandbox in config["sandboxes"] else None)

    def test_mcp_notifications_do_not_close_the_stream(self):
        lines = ['{"id":1}\n', '{"method":"notifications/initialized"}\n', '{"id":2}\n']
        results = [
            {"returncode": 0, "stdout": '{"id":1}\n', "stderr": ""},
            {"returncode": 0, "stdout": "", "stderr": ""},
            {"returncode": 0, "stdout": '{"id":2}\n', "stderr": ""},
        ]
        output = io.StringIO()
        with patch.object(sys, "stdin", io.StringIO("".join(lines))), patch.object(sys, "stdout", output):
            with patch.object(relay, "exchange", side_effect=results) as exchange:
                self.assertEqual(relay.client_main(self.directory, ["serve"]), 0)
        self.assertEqual([call.args[2] for call in exchange.call_args_list], lines)
        self.assertEqual(output.getvalue(), '{"id":1}\n{"id":2}\n')


if __name__ == "__main__":
    unittest.main()
