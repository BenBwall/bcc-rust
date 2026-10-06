use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{
        Command,
        Stdio,
    },
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

use serde_json::{
    Value,
    json,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Harness {
    dir: PathBuf,
    db:  PathBuf,
}
impl Harness {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "agentbus-rust-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Self {
            db: dir.join("bus.db"),
            dir,
        }
    }

    fn run(
        &self,
        args: &[&str],
        input: &str,
        extra_env: &[(&str, &str)],
    ) -> (bool, String, String) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agentbus"));
        command
            .args(args)
            .env("AGENTBUS_DB", &self.db)
            .env_remove("CODEX_THREAD_ID")
            .env_remove("CLAUDE_CODE_SESSION_ID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in extra_env {
            command.env(key, value);
        }
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        (
            output.status.success(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    }

    fn ok(&self, args: &[&str], input: &str) -> String {
        let (success, stdout, stderr) = self.run(args, input, &[]);
        assert!(success, "{args:?}: {stderr}");
        stdout
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(unix)]
#[test]
fn database_paths_with_url_characters_use_the_exact_file() {
    for name in ["bus%20.db", "bus?mode=ro.db", "bus#archive.db", "bus\\archive.db"] {
        let mut h = Harness::new();
        h.db = h.dir.join(name);
        h.ok(&["send", "--from", "codex", "--to", "claude", "saved"], "");
        assert!(h.db.is_file(), "database missing at {name:?}");
        assert!(h.ok(&["inbox", "--agent", "claude"], "").contains("saved"));
    }
}

#[test]
fn disabled_hook_does_not_open_database_or_consume_messages() {
    let h = Harness::new();
    let payload = json!({"hook_event_name":"Stop","session_id":"idle"}).to_string();
    assert_eq!(h.ok(&["hook", "--agent", "claude"], &payload), "");
    assert!(!h.db.exists());
    h.ok(&["send", "--from", "codex", "--to", "claude", "queued"], "");
    assert_eq!(h.ok(&["hook", "--agent", "claude"], &payload), "");
    h.ok(&["hooks", "on", "--session", "idle"], "");
    let output = h.ok(&["hook", "--agent", "claude"], &payload);
    assert_eq!(
        serde_json::from_str::<Value>(&output).unwrap()["decision"],
        "block"
    );
    assert!(output.contains("queued"));
    h.ok(&["hooks", "off", "--session", "idle"], "");
    assert_eq!(h.ok(&["hook", "--agent", "claude"], &payload), "");
}

#[test]
fn mcp_tools_share_cli_messages_and_threads() {
    let h = Harness::new();
    h.ok(
        &[
            "send",
            "--from",
            "codex",
            "--to",
            "claude",
            "--subject",
            "review",
            "first",
        ],
        "",
    );
    let requests=[
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inbox","arguments":{}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"send","arguments":{"to":"codex","body":"reply","reply_to":1}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"ack","arguments":{"ids":[1]}}}),
    ].map(|v|v.to_string()).join("\n")+"\n";
    let (success, output, error) = h.run(&["serve"], &requests, &[("AGENTBUS_AGENT", "claude")]);
    assert!(success, "{error}");
    let responses: Vec<Value> = output
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(responses.len(), 5);
    assert_eq!(responses[1]["result"]["tools"].as_array().unwrap().len(), 7);
    let inbox: Value = serde_json::from_str(
        responses[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(inbox["messages"][0]["body"], "first");
    let reply: Value = serde_json::from_str(
        responses[3]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(reply["thread"], 1);
    assert_eq!(h.ok(&["log", "-n", "2"], "").matches("--- #").count(), 2);
}

#[test]
fn claims_and_session_selection_validate_inputs() {
    let h = Harness::new();
    let (success, _, error) = h.run(&["hooks", "on"], "", &[]);
    assert!(!success);
    assert!(error.contains("pass --session"));
    assert!(!h.db.exists());
    let (success, output, _) = h.run(&["hooks", "on"], "", &[("CODEX_THREAD_ID", "working")]);
    assert!(success);
    assert!(output.contains("hooks on"));
    let (success, output, _) = h.run(&["hooks", "status"], "", &[("CODEX_THREAD_ID", "working")]);
    assert!(success);
    assert!(output.contains("hooks on"));
    let request=json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"claim","arguments":{"resource":"src/lib.rs","note":"work"}}}).to_string()+"\n";
    let (_, output, _) = h.run(&["serve"], &request, &[("AGENTBUS_AGENT", "claude")]);
    assert!(output.contains("src/lib.rs"));
    let (_, output, _) = h.run(&["serve"], &request, &[("AGENTBUS_AGENT", "codex")]);
    assert!(output.contains("isError\":true"));
}

#[test]
fn broadcasts_have_separate_receipts_and_hooks_deliver_once() {
    let h = Harness::new();
    h.ok(&["send", "--from", "claude", "--to", "all", "notice"], "");
    h.ok(&["hooks", "on", "--session", "work"], "");
    let payload = json!({"hook_event_name":"PostToolUse","session_id":"work"}).to_string();
    let first = h.ok(&["hook", "--agent", "codex"], &payload);
    assert!(first.contains("notice"));
    assert_eq!(h.ok(&["hook", "--agent", "codex"], &payload), "");
    assert!(h.ok(&["inbox", "--agent", "codex"], "").contains("notice"));
    assert!(h.ok(&["inbox", "--agent", "gemini"], "").contains("notice"));
    assert_eq!(h.ok(&["inbox", "--agent", "claude"], ""), "");
}

#[test]
fn stop_limit_and_afk_keepalive_preserve_pending_messages() {
    let h = Harness::new();
    h.ok(&["hooks", "on", "--session", "work"], "");
    let stop = json!({"hook_event_name":"Stop","session_id":"work"}).to_string();
    for index in 0..5 {
        h.ok(
            &[
                "send",
                "--from",
                "codex",
                "--to",
                "claude",
                &format!("round {index}"),
            ],
            "",
        );
        let output = h.ok(&["hook", "--agent", "claude"], &stop);
        assert_eq!(
            serde_json::from_str::<Value>(&output).unwrap()["decision"],
            "block"
        );
    }
    h.ok(&["send", "--from", "codex", "--to", "claude", "held"], "");
    assert_eq!(h.ok(&["hook", "--agent", "claude"], &stop), "");
    let prompt = json!({"hook_event_name":"UserPromptSubmit","session_id":"work"}).to_string();
    assert!(
        h.ok(&["hook", "--agent", "claude"], &prompt)
            .contains("held")
    );
    h.ok(&["afk", "on", "--hours", "1"], "");
    let keepalive = h.ok(&["hook", "--agent", "claude", "--idle-wait", "0"], &stop);
    assert!(keepalive.contains("keep-alive"));
    h.ok(&["afk", "off"], "");
}

#[test]
fn disabling_a_session_releases_an_afk_wait() {
    let h = Harness::new();
    h.ok(&["hooks", "on", "--session", "work"], "");
    h.ok(&["afk", "on", "--hours", "1"], "");
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentbus"))
        .args(["hook", "--agent", "claude", "--idle-wait", "4"])
        .env("AGENTBUS_DB", &h.db)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            json!({"hook_event_name":"Stop","session_id":"work"})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    h.ok(&["hooks", "off", "--session", "work"], "");
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn concurrent_hooks_deliver_a_message_only_once() {
    let h = Harness::new();
    h.ok(&["hooks", "on", "--session", "work"], "");
    h.ok(
        &["send", "--from", "codex", "--to", "claude", "one delivery"],
        "",
    );
    let payload = json!({"hook_event_name":"PostToolUse","session_id":"work"}).to_string();
    let (first, second) = std::thread::scope(|scope| {
        let one = scope.spawn(|| h.ok(&["hook", "--agent", "claude"], &payload));
        let two = scope.spawn(|| h.ok(&["hook", "--agent", "claude"], &payload));
        (one.join().unwrap(), two.join().unwrap())
    });
    assert_eq!(
        usize::from(first.contains("one delivery")) + usize::from(second.contains("one delivery")),
        1
    );
}
