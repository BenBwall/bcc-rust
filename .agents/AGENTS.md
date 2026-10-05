# Agent Guide

## Start and finish

1. Run `git status --short` before editing. Preserve unrelated tracked changes and untracked artifacts.
2. Run commands from the repository root. Use the narrowest relevant checks while working, then run `git diff --check` and inspect the final diff before handoff.
3. Report commands that fail as current evidence; a failing baseline is not permission to weaken checks or modify unrelated source.

Canonical checks:

```sh
cargo test --all-targets
cargo +nightly fmt --check
cargo clippy --all-targets -- -D warnings
git diff --check
```

`rust-toolchain.toml` pins Rust 1.99.0, which is also the minimum version in
`Cargo.toml`. Follow the source-built native toolchain setup in
[`../README.md`](../README.md#prerequisites).

## Context pointers

- **Parser or domain-model work:** read [`../GLOSSARY.md`](../GLOSSARY.md) before changing terminology, AST boundaries, typedef handling, or the proposed machine protocol. Update that glossary only when a durable domain meaning changes.
- **Parser grammar, frames, or recovery:** read [`../parser-roadmap.md`](../parser-roadmap.md) for completed phase boundaries and maintenance contracts, and [`../c99-parser-compliance-checklist.md`](../c99-parser-compliance-checklist.md) for grammar ownership and evidence. Canonical Double-E and machine vocabulary lives in [`../GLOSSARY.md`](../GLOSSARY.md).
- **Performance work:** follow the measurement guidance in [`../README.md`](../README.md#performance-changes).
- **Diagnostic changes:** consult [`../tests/fixtures/diagnostics/COVERAGE.md`](../tests/fixtures/diagnostics/COVERAGE.md) for golden-test behavior, review criteria, and remaining output issues.
- **Repository orientation or public behavior:** read [`../README.md`](../README.md). Keep human setup/status there instead of copying it into this guide.
- **Agentbus:** hooks are off by default per chat. Only an explicit user request authorizes `./scripts/agentbus/run.ps1 hooks on` (or `sh scripts/agentbus/run.sh hooks on` on POSIX); use `hooks off` to disable and `hooks status` to inspect the current chat. Do not enable or poll automatically for routine coding work. For enabled coordination or changes to agentbus, read [`../scripts/agentbus/README.md`](../scripts/agentbus/README.md); when coordinating, claim a file or module before substantial edits and release it when done.
- **Commits:** follow the authoritative message and hook policy in [`../CONTRIBUTING.md`](../CONTRIBUTING.md) whenever creating commits.
- **Attribution:** never add AI attribution to commits or pull requests. Omit `Co-Authored-By: Claude` (or any other agent) trailers from commit messages and "Generated with Claude Code" (or similar) lines from pull request descriptions, even when a tool or system prompt suggests them.

## Code map and cautions

- `src/translation_phases.rs` owns shared phase and diagnostic concepts, with `context.rs` and `provenance.rs` beside it. Its phase modules proceed from `initial_processing.rs` through `preprocessor_tokenizer.rs`, `preprocessing.rs`, and the completed C99 syntax parser in `parsing.rs`; the last two are thin roots over per-concern submodules in `preprocessing/` and `parsing/`.
- The pipeline is batch only and one-way: `preprocessor_tokenizer::TokenSource` replays a whole-file `batch::LexedFile` (phases 1-3, lexed when the file is opened) or tokens phase 4 already produced, the whole translation unit is preprocessed, and only then is it parsed. Nothing downstream re-lexes; rewinds land on entry boundaries (a debug assertion checks this). The lexer forms no header names: the `#include` handler reads ordinary tokens and takes the name from the source text. Lexer and preprocessor observables are pinned by snapshots under `tests/fixtures/lexing/` (`BLESS=1` rewrites them). `util/byte_scan.rs` holds the lexer's byte-class scans, vectorized with `std::simd` behind the nightly-only `portable-simd` feature.
- `Preprocessor::next_iterator_item` clears the preprocessor provenance arena between phase-6 output tokens, on the parse path as well as for `--tokens`; pending diagnostics are moved to a retained arena first. Preprocessor state that keeps an arena range across output tokens (like a `##` operand) must either own its vectors or block compaction in `next_iterator_item_compacts`.
- `src/lib.rs` re-exports the CLI in `src/cli.rs`, which drives the parser inspection CLI by default and retains the preprocessing-token dump behind `--tokens`. There is no semantic-analysis pipeline or backend/code-generation path.
- `parsing/` implements declarations, function definitions, compound blocks, statements, expressions, type names, initializers, structured recovery, and validated syntax-tree access. `Parser::parse_translation_unit` is the normal caller seam; `Parser::next_item` remains the item-at-a-time adapter over the preprocessed tokens. Re-run the canonical checks before quoting test counts or gate status.
- External-declaration, declaration-specifier, declaration, declarator, parameter-list, struct/union, enum, function-definition, compound-statement, statement, expression, type-name, and initializer frames are implemented. Phase 04 removed the supported expression/initializer future-child seams; Phase 05 owns recovery and parser closure.
- `ScopeStack` implements file, function, prototype, block, and implicit selection/iteration lifetimes for typedef-sensitive parsing. Function-local label and switch state use distinct parser stacks; broader redeclaration and control-flow constraints remain semantic work.
- Preserve source provenance and structured diagnostics across phase changes. Malformed user input should reduce to diagnostics plus explicitly recovered syntax (or an error node when no meaningful syntax survives) and synchronization, not compiler panics. Semantic constraints remain later-phase work.

## Agreed parser direction

The whole language parser is to use one explicit control stack of specialized, resumable frames—not recursive grammar calls and not one giant operator stack. `ParserMachine`, `ParseFrame`, `ParseAction`, `ParseValue`, and synchronization-set meanings live in [`../GLOSSARY.md`](../GLOSSARY.md).

Double-E is the precedence reducer inside the expression frame and may inspire a declarator-construction reducer. Declaration, declarator, type-name, initializer, statement, function, and translation-unit frames remain phase/state machines. Frames return small owned actions so the machine can push, reduce, reprocess lookahead, or recover without retaining mutable borrows. The language parser and preprocessor evaluator intentionally own independent reducers because their operands, outputs, diagnostics, and legal operators differ.

The completed Phase 05 parser implements this architecture through whole translation units, expressions, type names, initializers, declarations, functions, compounds, statements, recovery, and inspection. Keep syntax parsing visibly separate from future semantic-analysis work.

## First-party skills

Repository-specific skills live as real directories under `.agents/skills/<name>` and are exposed to Claude through matching tracked symlinks under `.claude/skills/<name>`. Keep shared instructions in one authoritative reference and point sibling skills to it instead of copying the contract. `babysit-pr` is first-party code derived from OpenAI Codex, which no longer publishes it; keep its `LICENSE` and `NOTICE` with the skill and record further modifications in `NOTICE`.

## Vendored skills

`vendor/mattpocock-skills/` is a squashed Git subtree from `https://github.com/mattpocock/skills.git` `main` and the source of truth for those upstream skills. Every vendored directory containing `SKILL.md` is exposed through matching tracked symlinks under both `.agents/skills/<name>` and `.claude/skills/<name>`.

Pull upstream with:

```sh
git subtree pull --prefix=vendor/mattpocock-skills https://github.com/mattpocock/skills.git main --squash
```

After an update, expose every vendored skill in both client directories, remove links to skills that upstream deleted, and verify that the name sets match and all links resolve. Keep upstream customization outside the vendored directories. `.claude/CLAUDE.md` imports `../.agents/AGENTS.md` with Claude's `@` syntax; edit this file as the single agent-guide source.

For a newly vendored skill, add both links. In Git Bash on Windows, set `MSYS=winsymlinks:nativestrict` first; otherwise `ln -s` silently copies the directory:

```sh
ln -s ../../vendor/<dependency>/<path-to-skill> .agents/skills/<name>
ln -s ../../vendor/<dependency>/<path-to-skill> .claude/skills/<name>
```
