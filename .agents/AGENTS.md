# Agent Guide

## Start and finish

1. Run `git status --short` before editing. Preserve unrelated tracked changes and untracked artifacts; the three root `*-report.html` files are research notes and must remain unless their removal is explicitly requested.
2. Run commands from the repository root. Use the narrowest relevant checks while working, then run `git diff --check` and inspect the final diff before handoff.
3. Report commands that fail as current evidence. The parser compile gate is known; a failing baseline is not permission to weaken checks or modify unrelated source.

Canonical checks:

```sh
cargo test --all-targets
cargo +nightly fmt --check
cargo clippy --all-targets -- -D warnings
git diff --check
```

`rust-toolchain.toml` pins Rust 1.98.0, which is also the minimum version in
`Cargo.toml`. `build.rs` additionally requires a native C compiler and
`libclang`; set `LIBCLANG_PATH` when discovery fails.

## Context pointers

- **Parser or domain-model work:** read [`../CONTEXT.md`](../CONTEXT.md) before changing terminology, AST boundaries, typedef handling, or the proposed machine protocol. Update that glossary only when a durable domain meaning changes.
- **Non-recursive parsing or Double-E work:** read [`../double-e-integration-report.html`](../double-e-integration-report.html), then revalidate its dated findings against [`../src/translation_phases/parsing.rs`](../src/translation_phases/parsing.rs) and [`../src/translation_phases/preprocessing.rs`](../src/translation_phases/preprocessing.rs).
- **Repository orientation or public behavior:** read [`../README.md`](../README.md). Keep human setup/status there instead of copying it into this guide.
- **Commits:** follow the authoritative message and hook policy in [`../CONTRIBUTING.md`](../CONTRIBUTING.md) whenever creating commits.
- **Historical parser or project estimates:** consult the root HTML reports as research notes, never as authority over current code or command output.

## Code map and cautions

- `src/translation_phases.rs` owns shared phase, provenance, context, and diagnostic concepts. Its phase modules proceed from `initial_processing.rs` through `preprocessor_tokenizer.rs`, `preprocessing.rs`, and the incomplete `parsing.rs`.
- `src/lib.rs` currently drives only the preprocessor and prints tokens. There is no parser invocation, semantic-analysis pipeline, or backend/code-generation path.
- `parsing.rs` now compiles and its Phase 02 declaration subset runs through `Parser::next_item`. Re-run the canonical checks before quoting test counts or gate status.
- External-declaration, declaration-specifier, declaration, declarator, parameter-list, struct/union, enum, and typed future-child frames are implemented. Type names, expressions, statements, initializers, and complete function definitions remain future work.
- `ScopeStack` implements file-scope typedef/ordinary-name classification and parser-visible prototype scopes needed by the migrated subset. Complete nested-scope and redeclaration handling remains future work.
- Preserve source provenance and structured diagnostics across phase changes. Malformed user input should reduce to diagnostics/error nodes and synchronization, not compiler panics.

## Agreed parser direction

The whole language parser is to use one explicit control stack of specialized, resumable frames—not recursive grammar calls and not one giant operator stack. `ParserMachine`, `ParseFrame`, `ParseAction`, `ParseValue`, and synchronization-set meanings live in [`../CONTEXT.md`](../CONTEXT.md).

Double-E is the precedence reducer inside the expression frame and may inspire a declarator-construction reducer. Declaration, declarator, type-name, initializer, statement, function, and translation-unit frames remain phase/state machines. Frames return small owned actions so the machine can push, reduce, reprocess lookahead, or recover without retaining mutable borrows. The existing preprocessor evaluator is the architectural precedent; extract a dialect-neutral core before sharing it with language-AST parsing, and add nested-conditional regression coverage while doing so.

The Phase 02 declaration subset implements this architecture. The remaining whole-language frames and shared expression reducer are intended design, not current behavior; keep current-status claims and proposed-design claims visibly separate.

## Vendored skills

`vendor/mattpocock-skills/` is a squashed Git subtree from `https://github.com/mattpocock/skills.git` `main` and the source of truth for those upstream skills. `vendor/openai-codex-skills/babysit-pr/` is a narrow snapshot of `.codex/skills/babysit-pr/` from `https://github.com/openai/codex.git`; its adjacent `.source.json` records the pinned revision. Every vendored directory containing `SKILL.md` is exposed through matching tracked symlinks under both `.agents/skills/<name>` and `.claude/skills/<name>`. The PR babysitter is additionally linked at `.codex/skills/babysit-pr` because its upstream commands invoke that installed path.

Pull upstream with:

```sh
git subtree pull --prefix=vendor/mattpocock-skills https://github.com/mattpocock/skills.git main --squash
```

Refresh the OpenAI snapshot from the repository and path recorded in `vendor/openai-codex-skills/.source.json`, replace only the `babysit-pr/` directory, and update the recorded revision in the same change. Invoke the repository-local `.agents/scripts/babysit_pr_watch.py` adapter for PR monitoring; it imports the pinned snapshot at runtime and owns repository-specific compatibility behavior without patching the vendored files.

After an update, expose every vendored skill in both client directories and verify that the name sets match and all links resolve. Keep upstream customization outside the vendored directories. `.claude/CLAUDE.md` imports `../.agents/AGENTS.md` with Claude's `@` syntax; edit this file as the single agent-guide source.

For a newly vendored skill, add both links:

```sh
ln -s ../../vendor/<dependency>/<path-to-skill> .agents/skills/<name>
ln -s ../../vendor/<dependency>/<path-to-skill> .claude/skills/<name>
```
