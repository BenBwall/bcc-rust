# bcc-rust

`bcc-rust` is an experimental Rust implementation of a C compiler front end targeting C99-era syntax. It is a development snapshot: the preprocessing pipeline is substantial, but the language parser is incomplete and there is no code-generation path.

## Current status

The current CLI entry point is a preprocessing inspection tool. It accepts a C source file or an input string, runs the initial-processing, preprocessing-token, and preprocessing stages, then prints the parser-facing tokens and diagnostics. It does not invoke the language parser or emit an object file or executable.

The repository now builds, and `Parser::next_item` runs a non-recursive declaration subset through preprocessing and an explicit frame stack. The migrated subset covers declaration specifiers, comma-separated declarators without implemented initializers, pointer qualifiers, parenthesized/array/function/abstract/K&R declarators, parameter lists and variadics, struct/union members, enums, and the file-scope typedef/ordinary-name classification needed by later declarations. Union nodes preserve `union`, enum slices use the enumerator arena, and malformed migrated paths recover with source-backed diagnostics.

Expression-dependent array bounds, bit-field widths, explicit enum values, initializers, and function bodies currently cross typed future-child seams, emit a stable unsupported diagnostic, and recover at the owning delimiter. Expressions, statements, complete function definitions, type-name parsing, nested scopes, semantic analysis, and code generation remain unimplemented. This is not yet a production-ready or conforming C99 compiler.

## Prerequisites

- Rust 1.98.0, pinned in [`rust-toolchain.toml`](rust-toolchain.toml) and declared
  as the minimum supported version in [`Cargo.toml`](Cargo.toml).
- Clippy for the default Rust toolchain.
- A native C toolchain for compiling [`float_parsing.c`](float_parsing.c).
- `libclang`, used by `bindgen` in [`build.rs`](build.rs). Set `LIBCLANG_PATH` if it is not discoverable automatically.
- Nightly Rustfmt for the repository's unstable formatting options.

## Build, test, and inspect

Run checks from the repository root:

```sh
cargo build
cargo test --all-targets
cargo +nightly fmt --check
cargo clippy --all-targets -- -D warnings
```

`cargo check`, formatting, and all-target tests pass for the Phase 02 implementation. The repository-wide strict-Clippy command is an attribution gate for this phase: it still exposes pre-existing lint debt in generated bindings, preprocessing, and utilities that Phase 02 deliberately leaves unchanged. The parser uses compile-time entrypoint signature guards, item-scoped dead-code expectations for explicitly deferred syntax, and narrowly justified structural Clippy expectations where the C grammar makes the representation intentional. The existing preprocessing CLI can be exercised with either input form:

```sh
cargo run -- --input '#define N 3
N + 1'
cargo run -- test-programs/define.c
```

Use `--iquote <directory>` (`-q`) and `--isystem <directory>` (`-s`) to add include search paths. `CPATH` and `C_INCLUDE_PATH` are also read by the CLI. Files under [`test-programs/`](test-programs/) are useful manual preprocessing inputs; they are not an automated conformance suite.

## Pipeline and code map

| Area | Role |
| --- | --- |
| [`src/translation_phases/initial_processing.rs`](src/translation_phases/initial_processing.rs) | Normalizes source characters, line endings, trigraphs, escaped newlines, and comments. |
| [`src/translation_phases/preprocessor_tokenizer.rs`](src/translation_phases/preprocessor_tokenizer.rs) | Produces preprocessing tokens while retaining source provenance. |
| [`src/translation_phases/preprocessing.rs`](src/translation_phases/preprocessing.rs) | Handles macros, directives, includes, conditional preprocessing, literals, and conversion to parser-facing tokens. It also contains the in-tree two-stack preprocessor-expression evaluator. |
| [`src/translation_phases/parsing.rs`](src/translation_phases/parsing.rs) | Contains the explicit parser driver, declaration/declarator/tag frames, syntax stores, file-scope name classification, typed future-child seams, and parser diagnostics. |
| [`src/translation_phases.rs`](src/translation_phases.rs) | Defines the shared phase interface, compilation context, source provenance, and diagnostic plumbing. |
| [`src/util/`](src/util/) | Provides project-specific arenas, interned strings, shared storage, queues, stacks, and vector slices. |
| [`src/lib.rs`](src/lib.rs) | Wires the current token-dump CLI to the preprocessor. |

## Parser direction

The language parser now uses one explicit control stack of specialized, resumable frames. `Parser` owns the buffered cursor, frame stack, typed child result, syntax stores, file-scope name classes, and recovery state. The Phase 02 frames implement declaration, declarator, parameter, struct/union, and enum grammar without recursive parser calls; a private trace characterizes token ownership and frame depth.

The remaining architecture is still staged. A later `ExpressionFrame` will use Double-E-style operator/operand reduction, and later initializer/statement/type-name frames will replace the current typed future-child seams. The preprocessor evaluator remains unchanged and is not coupled to the language parser in this phase. See the [project glossary](CONTEXT.md) for canonical terms and the [Double-E integration report](double-e-integration-report.html) for the point-in-time design study.

## Further reading

- [`CONTRIBUTING.md`](CONTRIBUTING.md) — commit-message format and hook setup.
- [`CONTEXT.md`](CONTEXT.md) — canonical compiler-domain vocabulary, including clearly marked proposed parser terms.
- [`.agents/AGENTS.md`](.agents/AGENTS.md) — compact operational guidance for coding agents; `.claude/CLAUDE.md` imports the same file.
- [`project-status-report.html`](project-status-report.html) — point-in-time repository assessment.
- [`parser-status-report.html`](parser-status-report.html) — detailed parser audit.
- [`double-e-integration-report.html`](double-e-integration-report.html) — feasibility and architecture study for the non-recursive parser.
- [The Double-E Method](https://erikeidt.github.io/The-Double-E-Method.html) — the algorithm description referenced by the architecture study.

The HTML reports are local research notes dated 21 August 2026. Revalidate their claims against the current source and command output before relying on them.

## License

See [`LICENSE`](LICENSE).
