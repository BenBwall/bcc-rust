"""Tests for agentbus. Run with: python -m unittest discover tools/agentbus"""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import agentbus  # noqa: E402

SCRIPT = Path(__file__).resolve().parent / "agentbus.py"


class BusTestCase(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.db_path = Path(self.directory.name) / "bus.db"
        self.bus = agentbus.Bus(self.db_path)

    def tearDown(self):
        self.bus.close()
        self.directory.cleanup()


class MessageTests(BusTestCase):
    def test_direct_message_is_delivered_once_and_acked(self):
        sent = self.bus.send("codex", "claude", "parser.rs is mine for now")
        taken, remaining = self.bus.take_undelivered("claude")
        self.assertEqual([m["id"] for m in taken], [sent["id"]])
        self.assertEqual(remaining, 0)
        self.assertEqual(self.bus.take_undelivered("claude"), ([], 0))
        self.assertEqual(len(self.bus.inbox("claude")), 1)
        self.bus.ack("claude", [sent["id"]])
        self.assertEqual(self.bus.inbox("claude"), [])

    def test_message_is_invisible_to_other_agents(self):
        self.bus.send("codex", "claude", "hello")
        self.assertEqual(self.bus.inbox("other"), [])
        with self.assertRaises(agentbus.BusError):
            self.bus.ack("other", [1])

    def test_broadcast_reaches_everyone_but_sender_with_separate_receipts(self):
        sent = self.bus.send("claude", "all", "rebasing main")
        self.assertEqual(self.bus.inbox("claude"), [])
        self.bus.ack("codex", [sent["id"]])
        self.assertEqual(self.bus.inbox("codex"), [])
        self.assertEqual(len(self.bus.inbox("gemini")), 1)

    def test_replies_share_the_root_thread(self):
        root = self.bus.send("claude", "codex", "question")
        reply = self.bus.send("codex", "claude", "answer", reply_to=root["id"])
        again = self.bus.send("claude", "codex", "follow-up", reply_to=reply["id"])
        self.assertEqual(again["thread_id"], root["id"])
        self.assertEqual([m["id"] for m in self.bus.thread(again["id"])], [1, 2, 3])

    def test_rejects_invalid_messages(self):
        for sender, recipient, body in [
            ("claude", "claude", "self"),
            ("claude", "codex", "  "),
            ("all", "codex", "spoofed broadcast sender"),
            ("Bad Name", "codex", "x"),
        ]:
            with self.subTest(sender=sender, recipient=recipient):
                with self.assertRaises(agentbus.BusError):
                    self.bus.send(sender, recipient, body)
        with self.assertRaises(agentbus.BusError):
            self.bus.send("claude", "codex", "x", reply_to=99)


class ClaimTests(BusTestCase):
    def test_claims_are_exclusive_renewable_and_releasable(self):
        self.bus.claim("claude", "src/cli.rs", "splitting args")
        with self.assertRaises(agentbus.BusError):
            self.bus.claim("codex", "src/cli.rs")
        with self.assertRaises(agentbus.BusError):
            self.bus.release("codex", "src/cli.rs")
        self.bus.claim("claude", "src/cli.rs", "renewed")
        self.assertEqual(self.bus.claims()[0]["note"], "renewed")
        self.assertTrue(self.bus.release("claude", "src/cli.rs"))
        self.assertFalse(self.bus.release("claude", "src/cli.rs"))
        self.bus.claim("codex", "src/cli.rs")

    def test_expired_claim_can_be_taken(self):
        self.bus.claim("claude", "build.rs", ttl_minutes=1)
        self.bus.db.execute("UPDATE claims SET expires_at = '2000-01-01T00:00:00Z'")
        self.assertEqual(self.bus.claims(), [])
        self.assertEqual(self.bus.claim("codex", "build.rs")["agent"], "codex")


class HookTests(BusTestCase):
    def hook(self, event, **fields):
        return agentbus.run_hook(self.bus, "claude", {"hook_event_name": event, **fields})

    def test_session_start_orients_even_without_messages(self):
        self.bus.claim("codex", "src/lib.rs", "refactor")
        output = self.hook("SessionStart")
        context = output["hookSpecificOutput"]["additionalContext"]
        self.assertIn("you are `claude`", context)
        self.assertIn("src/lib.rs: codex", context)

    def test_context_events_inject_new_messages_once(self):
        self.assertIsNone(self.hook("PostToolUse"))
        self.bus.send("codex", "claude", "please review #2")
        output = self.hook("PostToolUse")
        self.assertEqual(output["hookSpecificOutput"]["hookEventName"], "PostToolUse")
        self.assertIn("please review #2", output["hookSpecificOutput"]["additionalContext"])
        self.assertIsNone(self.hook("UserPromptSubmit"))

    def test_stop_blocks_for_new_messages_and_is_bounded(self):
        self.assertIsNone(self.hook("Stop"))
        for round_number in range(agentbus.MAX_STOP_BLOCKS):
            self.bus.send("codex", "claude", f"round {round_number}")
            output = self.hook("Stop", stop_hook_active=round_number > 0)
            self.assertEqual(output["decision"], "block")
        self.bus.send("codex", "claude", "one too many")
        self.assertIsNone(self.hook("Stop", stop_hook_active=True))
        # The held message arrives with the next user prompt, which resets the bound.
        output = self.hook("UserPromptSubmit")
        self.assertIn("one too many", output["hookSpecificOutput"]["additionalContext"])

    def test_injection_is_limited_and_truncated(self):
        for index in range(agentbus.HOOK_MESSAGE_LIMIT + 2):
            self.bus.send("codex", "claude", "x" * (agentbus.HOOK_BODY_LIMIT + 10))
        context = self.hook("PostToolUse")["hookSpecificOutput"]["additionalContext"]
        self.assertIn("[2 more undelivered", context)
        self.assertIn("truncated", context)

    def test_unknown_events_do_not_consume_messages(self):
        self.bus.send("codex", "claude", "hi")
        self.assertIsNone(self.hook("PreToolUse"))
        self.assertEqual(len(self.bus.take_undelivered("claude")[0]), 1)


class FakeTime:
    """A clock that advances only when the hook sleeps, running `on_sleep` each time."""

    def __init__(self, on_sleep=None):
        self.now = 0.0
        self.sleeps = 0
        self.on_sleep = on_sleep

    def clock(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds
        self.sleeps += 1
        if self.on_sleep:
            self.on_sleep(self.sleeps)


class AfkTests(BusTestCase):
    def afk_on(self, agent=agentbus.EVERY_AGENT):
        self.bus.set_afk(agentbus.now() + agentbus.timedelta(hours=1), agent)

    def stop(self, fake, idle_wait=60, agent="claude"):
        return agentbus.run_hook(
            self.bus, agent, {"hook_event_name": "Stop"}, idle_wait, fake.sleep, fake.clock
        )

    def test_stop_waits_for_a_message_then_continues_with_it(self):
        self.afk_on()
        fake = FakeTime(lambda n: n == 3 and self.bus.send("codex", "claude", "your turn"))
        output = self.stop(fake)
        self.assertEqual(output["decision"], "block")
        self.assertIn("your turn", output["reason"])
        self.assertEqual(fake.sleeps, 3)

    def test_idle_wait_ends_in_a_keepalive_turn(self):
        self.afk_on()
        output = self.stop(FakeTime(), idle_wait=60)
        self.assertEqual(output["decision"], "block")
        self.assertIn("keep-alive", output["reason"])

    def test_turning_afk_off_releases_a_waiting_agent(self):
        self.afk_on()
        fake = FakeTime(lambda n: n == 2 and self.bus.set_afk(None))
        self.assertIsNone(self.stop(fake, idle_wait=3600))
        self.assertEqual(fake.sleeps, 2)

    def test_afk_replaces_the_per_prompt_cap_with_an_hourly_rate(self):
        self.afk_on()
        for index in range(agentbus.MAX_STOP_BLOCKS + 3):
            self.bus.send("codex", "claude", f"message {index}")
            self.assertIn(f"message {index}", self.stop(FakeTime())["reason"])
        self.bus.db.execute(
            "UPDATE agent_state SET value = ? WHERE agent = 'claude' AND key = 'afk_continuations'",
            (json.dumps([agentbus.iso(agentbus.now())] * agentbus.AFK_CONTINUATIONS_PER_HOUR),),
        )
        self.bus.send("codex", "claude", "throttled")
        output = self.stop(FakeTime(), idle_wait=10)
        self.assertIn("keep-alive", output["reason"])
        self.assertEqual(len(self.bus.take_undelivered("claude")[0]), 1)

    def test_agent_specific_settings(self):
        self.afk_on()
        self.bus.set_afk(None, "codex")
        self.assertIsNone(self.bus.afk_until("codex"))
        self.assertIsNotNone(self.bus.afk_until("claude"))
        self.bus.set_afk(None)
        self.assertEqual(self.bus.afk_settings(), [])
        self.afk_on("codex")
        self.assertIsNone(self.bus.afk_until("claude"))
        self.bus.set_afk(agentbus.now() - agentbus.timedelta(minutes=1), "codex")
        self.assertIsNone(self.bus.afk_until("codex"))

    def test_attended_stop_is_unchanged_when_afk_is_off(self):
        fake = FakeTime()
        self.assertIsNone(self.stop(fake, idle_wait=3600))
        self.assertEqual(fake.sleeps, 0)

    def test_orientation_announces_afk(self):
        self.afk_on()
        output = agentbus.run_hook(self.bus, "claude", {"hook_event_name": "SessionStart"})
        self.assertIn("AFK mode is on", output["hookSpecificOutput"]["additionalContext"])


class SessionTests(BusTestCase):
    def test_detached_session_hooks_do_nothing(self):
        self.bus.set_afk(agentbus.now() + agentbus.timedelta(hours=1))
        self.bus.send("codex", "claude", "for the working session")
        self.bus.detach_session("idle")
        fake = FakeTime()
        for event in ("SessionStart", "PostToolUse", "Stop"):
            payload = {"hook_event_name": event, "session_id": "idle"}
            self.assertIsNone(agentbus.run_hook(self.bus, "claude", payload, 60, fake.sleep, fake.clock))
        self.assertEqual(fake.sleeps, 0)
        payload = {"hook_event_name": "PostToolUse", "session_id": "working"}
        output = agentbus.run_hook(self.bus, "claude", payload)
        self.assertIn("for the working session", output["hookSpecificOutput"]["additionalContext"])
        self.assertTrue(self.bus.attach_session("idle"))
        self.assertFalse(self.bus.is_detached("idle"))

    def test_redeliver_requeues_only_unacked_messages(self):
        first = self.bus.send("codex", "claude", "misdelivered")
        second = self.bus.send("codex", "claude", "handled")
        self.bus.take_undelivered("claude")
        self.bus.ack("claude", [second["id"]])
        self.assertEqual(self.bus.redeliver("claude", [first["id"], second["id"]]), [first["id"]])
        taken, _ = self.bus.take_undelivered("claude")
        self.assertEqual([m["id"] for m in taken], [first["id"]])


class ProcessTests(unittest.TestCase):
    """Exercise the real entry points the agents launch."""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.env = {**os.environ, "AGENTBUS_DB": str(Path(self.directory.name) / "bus.db")}

    def tearDown(self):
        self.directory.cleanup()

    def run_script(self, *args, stdin="", agent=None):
        env = dict(self.env)
        if agent:
            env["AGENTBUS_AGENT"] = agent
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            input=stdin.encode("utf-8"),
            capture_output=True,
            env=env,
            check=True,
        ).stdout.decode("utf-8")

    def test_mcp_server_round_trip(self):
        requests = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "send", "arguments": {"to": "claude", "body": "hi"}}},
            {"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "send", "arguments": {"to": "codex", "body": "self"}}},
            {"jsonrpc": "2.0", "id": 5, "method": "bogus"},
        ]
        output = self.run_script("serve", stdin="".join(json.dumps(r) + "\n" for r in requests), agent="codex")
        responses = {r["id"]: r for r in map(json.loads, output.splitlines())}
        self.assertEqual(sorted(responses), [1, 2, 3, 4, 5])
        self.assertEqual(responses[1]["result"]["protocolVersion"], "2025-06-18")
        self.assertIn("send", [t["name"] for t in responses[2]["result"]["tools"]])
        self.assertFalse(responses[3]["result"]["isError"])
        self.assertTrue(responses[4]["result"]["isError"])
        self.assertEqual(responses[5]["error"]["code"], -32601)

    def test_hook_reads_stdin_and_writes_json(self):
        self.run_script("send", "--from", "codex", "--to", "claude", "check the tests")
        output = self.run_script("hook", "--agent", "claude", stdin=json.dumps({"hook_event_name": "Stop"}))
        self.assertEqual(json.loads(output)["decision"], "block")
        self.assertEqual(self.run_script("hook", "--agent", "claude", stdin="not json"), "")


class LocationTests(unittest.TestCase):
    def test_linked_worktree_resolves_to_the_shared_common_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            common = root / "main" / ".git"
            private = common / "worktrees" / "feature"
            private.mkdir(parents=True)
            (private / "commondir").write_text("../..\n")
            worktree = root / "feature" / "tools"
            worktree.mkdir(parents=True)
            (root / "feature" / ".git").write_text(f"gitdir: {private}\n")
            self.assertEqual(agentbus.git_common_dir(worktree), common.resolve())


if __name__ == "__main__":
    unittest.main()
