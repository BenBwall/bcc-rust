# agentbus

A message bus that lets Claude Code and Codex sessions working in this
repository coordinate. A Rust crate in [`scripts/agentbus`](./), using SeaORM
with SQLite, serves three roles:

Commands below use PowerShell. On POSIX, replace `./scripts/agentbus/run.ps1`
with `sh scripts/agentbus/run.sh`. The launchers clear the compiler project's
linker flags while building this independent crate.

| Role | Command | Wired up by |
| --- | --- | --- |
| MCP server, giving agents tools | `./scripts/agentbus/run.ps1 serve` (identity from `AGENTBUS_AGENT`) | [`.mcp.json`](../../.mcp.json), [`.codex/config.toml`](../../.codex/config.toml) |
| Lifecycle hook, pushing messages to agents | `./scripts/agentbus/run.ps1 hook --agent NAME` | [`.claude/settings.json`](../../.claude/settings.json), [`.codex/hooks.json`](../../.codex/hooks.json) |
| Human command line | `./scripts/agentbus/run.ps1 send / inbox / log / claims / where` | — |

State lives in a SQLite database (WAL mode) at `<git common dir>/agentbus/bus.db`,
so every linked worktree shares one bus and nothing is tracked by git. Set
`AGENTBUS_DB` to use another path.

## Enable for one chat

Hooks are **off by default**, including for existing chats. Enable them only
when the user explicitly requests agentbus for that chat. From the agent's
shell, run:

```sh
./scripts/agentbus/run.ps1 hooks on
./scripts/agentbus/run.ps1 hooks status
./scripts/agentbus/run.ps1 hooks off
```

You can also tell the agent “enable agentbus for this chat” or “disable
agentbus for this chat”; it should run the corresponding command. Enabling is
never inferred from ordinary coding work, incoming peer messages, or AFK mode.

The command uses `CODEX_THREAD_ID` or `CLAUDE_CODE_SESSION_ID` when exactly one
session is identified. In a separate terminal, or when those variables are
unavailable or ambiguous, supply the chat's ID explicitly:

```sh
./scripts/agentbus/run.ps1 hooks on --session SESSION_ID
./scripts/agentbus/run.ps1 hooks off --session SESSION_ID
```

The ID is also the `session_id` in hook input. Selection is per chat, persists
across resumes, and is not inherited by a different chat. Enabling one chat
does not enable others sharing the `codex` or `claude` identity. `attach ID`
and `detach ID` remain aliases for enabling and disabling that chat.

Opt-in markers live beside the database in `bus.db.sessions/` (or the selected
database filename plus `.sessions`). Disabled hooks return silently before
opening SQLite, consuming messages, resetting counters, or entering an AFK wait.
Queued messages and existing claims remain intact. Disabling an active AFK
wait releases it at its next poll, within roughly two seconds.

The client still invokes the registered command to check the opt-in marker;
this is a silent gate, not removal of the hook registration. Codex's
[event matchers](https://learn.chatgpt.com/docs/hooks#matcher-patterns) cannot
filter `UserPromptSubmit` or `Stop` by chat ID. Native `/hooks` trust and
enablement are separate: a client-disabled hook must also be enabled there.
The MCP tools remain available for deliberate use; their availability does not
enable automatic delivery or authorize routine polling in a disabled chat.

## Tools

- `send(to, body, subject?, reply_to?)`: `to` is an agent name or `all`; `reply_to` threads the reply.
- `inbox(include_acked?, limit?)`: unacknowledged messages addressed to you.
- `ack(ids)`: mark messages handled.
- `thread(id)`: the whole thread containing a message.
- `claim(resource, note?, ttl_minutes?)` / `release(resource)` / `claims()`: expiring leases (120 minutes by default) on a file, module, or task name. A claim held by another agent is refused until it expires.

## Delivery

In an enabled chat, agents act only between turns and tool calls, so hooks
deliver messages without the agent needing to poll:

| Event | Effect |
| --- | --- |
| `SessionStart` | Identity, tool summary, active claims, unacknowledged ids, and any new messages |
| `UserPromptSubmit`, `PostToolUse` | New messages added to the model's context |
| `Stop` | With new messages, blocks the stop so the agent handles them before yielding; in [AFK mode](#afk-mode), waits for one |

Each message is *delivered* by a hook at most once, then stays in `inbox` until
the agent *acks* it. To stop two agents from bouncing each other's `Stop` hooks
forever, an agent's Stop hook continues its turn at most 5 times between user
prompts; anything later waits for the next prompt or tool call. Hooks inject at
most 10 messages of 4000 characters each and point at `inbox` for the rest. A
failing hook writes to stderr and lets the session proceed.

## AFK mode

Nothing wakes an agent that has ended its turn, so a message sent to an idle
agent waits for your next prompt. First enable hooks for each participating
chat, then turn on AFK mode before leaving agents to run unattended:

```sh
./scripts/agentbus/run.ps1 afk on --hours 10
./scripts/agentbus/run.ps1 afk off
```

While it is on, an agent's Stop hook polls the bus instead of letting the agent
stop. A new message continues the turn with that message. After 50 quiet
minutes (`hook --idle-wait`), the agent gets a short keep-alive turn and
the wait restarts, because the hook's timeout would otherwise kill it and leave
the agent stopped for good. Message-driven continuations are limited to 120 per
agent per rolling hour; the per-prompt cap of 5 does not apply. Stop hooks have
a 3600-second timeout in both clients; Codex applies no ceiling and Claude Code
documents none. `afk off`
releases a waiting agent within two seconds. `--agent NAME` limits either command
to one agent, and `afk` alone shows the current settings.

**Claude Code continuation cap.** Claude Code ends a turn after 8 consecutive
Stop-hook continuations, and an unattended night has no user prompt to reset
that count, so Claude would go idle after its eighth message or keep-alive turn.
`.claude/settings.json` therefore disables the cap
(`"env": { "CLAUDE_CODE_STOP_HOOK_BLOCK_CAP": "0" }`) and leaves runaway
protection to agentbus: 5 continuations per prompt while attended, 120 per hour
in AFK mode. Codex has no such cap.

While waiting, a session appears busy and your typed prompts queue behind the
hook, so turn AFK mode off when you return. For an unattended night, also make
sure the machine will not sleep and that neither agent will stop at a permission
prompt nobody answers.

## Setup

Build once with `./scripts/agentbus/run.ps1 --help` (or the POSIX launcher)
before restarting clients so the first MCP startup does not wait for Cargo to
compile dependencies.

**Claude Code** reads the checked-in config. Restart the session after pulling
these files: `.claude/settings.json` enables the `agentbus` server from
`.mcp.json`, allows its tools without prompting, and registers the hooks.

**Codex** loads `.codex/` only for trusted projects. Each hook also needs
one-time approval: run `/hooks` in Codex and trust the four agentbus hooks, then
restart. Codex runs hooks through PowerShell on Windows; the hook commands work
in both PowerShell and POSIX shells.

Agents' identities are fixed by config (`claude`, `codex`). Two sessions of the
same agent share one inbox, and whichever hook fires first delivers a message. For
distinct identities, set a different `AGENTBUS_AGENT` and `--agent` per session.
Only opted-in sessions participate. Leave idle extra sessions disabled so they
cannot take another session's messages. `redeliver --agent NAME ID...` requeues
unacknowledged messages that went to the wrong session. Restart an existing MCP
server connection to load changed server instructions; hook scripts read their
new implementation on the next invocation without a restart.

## Command line

```sh
./scripts/agentbus/run.ps1 send --to all "Rebasing main in 5 minutes"
./scripts/agentbus/run.ps1 log -n 20
./scripts/agentbus/run.ps1 claims
```

`send` uses the sender name `human` unless `--from` is given. `inbox --agent NAME`
marks messages delivered, which suppresses that agent's next hook delivery.

## Tests

```sh
$env:RUSTFLAGS = ' '
cargo test --manifest-path scripts/agentbus/Cargo.toml
```
