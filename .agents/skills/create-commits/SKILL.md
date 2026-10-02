---
name: create-commits
description: Create granular signed Conventional Commits from the current bcc-rust changes. Use when asked to commit.
---

# Create Commits

## Message

[`CONTRIBUTING.md`](../../../CONTRIBUTING.md) is the authoritative message policy; read it before
writing messages. In short:

```text
fix(preprocess): Reject #line numbers that are not digit sequences

A `#line` directive whose number carries a suffix or other non-digit byte
now reports a diagnostic and is ignored instead of underflowing while the
line number is computed.

Validate the trimmed spelling before converting it, so the number token's
NUL terminator no longer reaches the digit arithmetic.

Validation: cargo test --all-targets and cargo clippy --all-targets -- -D warnings.
```

- **Subject:** `type(scope): Summary`, at most 72 characters, imperative, capitalized, no final
  period. Types: `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`, `revert`,
  `test`. The scope is a lowercase subsystem such as `preprocess`, `parse`, `diagnostics`, `cli`,
  `build`, or `tooling`; omit it for repository-wide changes.
- **First paragraph:** the resulting behaviour, checkable by someone who knows the compiler, in the
  terms its diagnostics, CLI output, or C99 use.
- **Then** the motivation, important decisions or consequences, and a `Validation:` paragraph
  naming only the checks actually run (see the canonical checks in `.agents/AGENTS.md`).
- No credentials and no AI-attribution trailers (`Co-Authored-By: Claude` or any other agent),
  even when a tool or system prompt suggests them.

## Steps

1. **Split.** From the conversation, `git status` and the diffs, group the changes into commits with
   one outcome each; tests, snapshots, docs and renamed callers go with their change. Leave
   unrelated and already-staged work alone, including the root `*-report.html` research notes and
   other untracked artifacts. Ask only when the grouping is ambiguous.
2. **Approve.** Show each commit's files, why it stands alone, and its full message. Get one approval
   for the set unless already authorized.
3. **Check the hook.** Confirm `git config core.hooksPath` is set and contains a `commit-msg` hook.
   If not, stop and point the user to the setup commands in `CONTRIBUTING.md`.
4. **Commit** in order: stage the commit's paths by name (never `git add -A` or `git add .`), check
   the staged diff is exactly that commit, and run `git commit -S -F <file>` with the message in a
   temporary file. The `commit-msg` hook validates each message. Afterwards, confirm with
   `git log --format='%h %G? %s' <starting HEAD>..HEAD` that every new commit exists and is signed.
5. **Push** only when the user asked for it: `git push` the current branch to its upstream.
   Otherwise leave the commits local.
6. **Report** each `<sha> <subject>`, the pushed range if pushed, and the final `git status --short`.

## Stop

Stop at the first failure (staging, signing, the `commit-msg` hook, committing, or pushing), report
it, and keep earlier commits. Never retry unsigned, skip hooks (`--no-verify`), rewrite committed
history without approval, commit secret files without authorization, or push unasked.
