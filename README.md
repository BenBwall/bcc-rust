# bcc-rust

`bcc-rust` is an experimental Rust implementation of a C compiler front end targeting C99-era syntax. It is a development snapshot: the preprocessing pipeline is substantial, but the language parser is incomplete and there is no code-generation path.

## Current status

The current CLI entry point is a preprocessing inspection tool. It accepts a C source file or an input string, runs the initial-processing, preprocessing-token, and preprocessing stages, then prints the parser-facing tokens and diagnostics. It does not invoke the language parser or emit an object file or executable.

The repository does not currently build. Verified on 22 August 2026, `cargo test --all-targets` stops at 14 compile errors in [`src/translation_phases/parsing.rs`](src/translation_phases/parsing.rs), before any tests run. The immediate failures reflect unfinished parser and AST wiring, including a missing type-name model, missing arenas and diagnostics, incompatible index types, and incomplete control-flow returns. Declaration-specifier and declarator code exists, but expression, statement, initializer, function-definition, scope, recovery, and several AST paths remain unfinished.

This is not yet a production-ready or conforming C99 compiler. The strongest working area is lexing and preprocessing; the parser types show intended coverage, not completed support.

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

The build, test, and Clippy commands currently encounter the parser compile gate described above. Once that gate is repaired, the existing preprocessing CLI can be exercised with either input form:

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
| [`src/translation_phases/parsing.rs`](src/translation_phases/parsing.rs) | Contains the incomplete language parser, declaration/declarator work, AST shapes, indexes, and parser diagnostics. |
| [`src/translation_phases.rs`](src/translation_phases.rs) | Defines the shared phase interface, compilation context, source provenance, and diagnostic plumbing. |
| [`src/util/`](src/util/) | Provides project-specific arenas, interned strings, shared storage, queues, stacks, and vector slices. |
| [`src/lib.rs`](src/lib.rs) | Wires the current token-dump CLI to the preprocessor. |

## Parser direction

The chosen direction is a non-recursive parser for the whole C grammar. A proposed `ParserMachine` owns one explicit control stack of specialized, resumable grammar frames for translation units, declarations, declarators, type names, initializers, statements, and expressions. Double-E-style operator/operand reduction handles expression precedence inside an expression frame; declaration and statement frames remain their own state machines. Scope transitions, typedef lookup, owned frame results, and per-frame error recovery are explicit parts of the design.

That architecture is agreed direction, not implemented behavior. The in-tree preprocessor-expression evaluator is its strongest precedent and should be extracted and hardened as a reusable reduction core rather than coupled directly to language-AST operands. See the [project glossary](CONTEXT.md) for canonical terms and the [Double-E integration report](double-e-integration-report.html) for the point-in-time design study.

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
