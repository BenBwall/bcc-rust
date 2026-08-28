# bcc-rust

`bcc-rust` is an experimental Rust implementation of a C compiler front end
targeting C99 syntax. Its non-recursive language parser is complete for the
declared syntax scope, but semantic analysis and code generation are not yet
implemented.

## Current status

The CLI accepts a C source file or an input string, runs preprocessing and the
language parser, and prints diagnostics. Pass `--syntax-tree` for a stable,
source-oriented tree, `--syntax-locations` to add locations, `--raw-syntax` for
arena debugging, or `--tokens` for parser-facing preprocessing tokens. Normal
operation does not dump internal arenas. The CLI does not emit an object file
or executable.

`Parser::parse_translation_unit` returns an ordered `ParsedTranslationUnit` and
validated, read-only `SyntaxTree`. Declarations, prototype-style and old-style
function definitions, blocks, every C99 statement family, expressions, type
names, and initializers run through one explicit heap-backed frame stack.
Malformed input retains repaired syntax where meaningful, produces a
provenance-only external error node for pure top-level garbage, and emits
structured FIFO diagnostics with recovery context.

Phase 05 is complete. The parser meets the parser-relevant C99 minimum
translation floors, diagnoses excluded extensions and invalid phase-7 input,
and has deterministic truncation/property coverage. Semantic analysis—including
type/lvalue constraints, constant-expression evaluation, initializer
current-object rules, and linkage—and code generation remain unimplemented.
This is not yet a production-ready or conforming C99 compiler.

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

These are the canonical Phase 05 gates. Exercise the parser with either input
form, and opt into the desired inspection view:

```sh
cargo run -- --input 'typedef int T;
T *value;'
cargo run -- --syntax-tree --syntax-locations test-programs/test.c
cargo run -- --raw-syntax --input 'int value = 1;'
cargo run -- test-programs/test.c
cargo run -- --tokens --input '#define N 3
N + 1'
```

Use `--iquote <directory>` (`-q`) and `--isystem <directory>` (`-s`) to add include search paths. `CPATH` and `C_INCLUDE_PATH` are also read by the CLI. Files under [`test-programs/`](test-programs/) are useful manual inspection inputs, but they are not an automated conformance suite.

## Pipeline and code map

| Area | Role |
| --- | --- |
| [`src/translation_phases/initial_processing.rs`](src/translation_phases/initial_processing.rs) | Normalizes source characters, line endings, trigraphs, escaped newlines, and comments. |
| [`src/translation_phases/preprocessor_tokenizer.rs`](src/translation_phases/preprocessor_tokenizer.rs) | Produces preprocessing tokens while retaining source provenance. |
| [`src/translation_phases/preprocessing.rs`](src/translation_phases/preprocessing.rs) | Handles macros, directives, includes, conditional preprocessing, literals, and conversion to parser-facing tokens. It owns the preprocessor-expression evaluator and its values and diagnostics. |
| [`src/translation_phases/parsing.rs`](src/translation_phases/parsing.rs) | Contains the explicit parser driver; declaration, function-definition, statement, expression, type-name, initializer, declarator, and tag frames; syntax stores and scopes; and parser diagnostics. |
| [`src/translation_phases.rs`](src/translation_phases.rs) | Defines the shared translation-phase interface, compilation context, source provenance, and diagnostic plumbing. |
| [`src/util/`](src/util/) | Provides project-specific arenas, interned strings, shared storage, queues, stacks, and vector slices. |
| [`src/lib.rs`](src/lib.rs) | Wires the inspection CLI to the parser by default and to the token dump with `--tokens`. |

## Parser direction

The language parser uses one explicit control stack of specialized, resumable
frames. `Parser` owns the buffered cursor, frame stack, typed child result,
syntax stores, scope and label state, recovery state, and resource ceilings.

`ExpressionFrame` owns Double-E-style operator/operand reduction alongside
`TypeNameFrame` and `InitializerFrame`, and all supported statement and
declaration expression sites contain parsed handles. The preprocessor evaluator
retains its independent reducer. Phase 05 closes the parser through structured
diagnostics, cross-family recovery, syntax provenance, compatibility fixtures,
translation floors, and opt-in inspection. See the [project glossary](CONTEXT.md)
for canonical terms.

## Parser roadmap

All five syntax-parser phases are complete. Phase 05 audited the full grammar
and closed cross-family recovery, provenance, inspection, phase-7 totality,
diagnostics, and translation limits.

The authoritative phase boundaries and the distinction between
parser-complete and compiler-complete are in the
[language parser roadmap](parser-roadmap.md). The completed implementation brief
is available as the [Phase 03 Markdown plan](phase-03-statements-and-function-definitions-plan.md)
and its [HTML companion](phase-03-statements-and-function-definitions-plan.html).
The merged Phase 04 scope is defined by the
[Phase 04 Markdown plan](phase-04-expressions-and-type-names-plan.md) and its
[HTML companion](phase-04-expressions-and-type-names-plan.html).
Phase 05 closure is defined by the
[Phase 05 plan](phase-05-recovery-and-c99-parser-closure-plan.md) and the
[C99 parser compliance checklist](c99-parser-compliance-checklist.md).

## Further reading

- [`CONTRIBUTING.md`](CONTRIBUTING.md) — commit-message format and hook setup.
- [`CONTEXT.md`](CONTEXT.md) — canonical compiler-domain and parser vocabulary.
- [`parser-roadmap.md`](parser-roadmap.md) — authoritative Phase 01–05 language-parser sequence and exit gates.
- [`phase-03-statements-and-function-definitions-plan.md`](phase-03-statements-and-function-definitions-plan.md) — completed Phase 03 implementation brief and acceptance criteria.
- [`phase-03-statements-and-function-definitions-codex-prompt.md`](phase-03-statements-and-function-definitions-codex-prompt.md) — archived task prompt used to implement Phase 03.
- [`phase-04-expressions-and-type-names-plan.md`](phase-04-expressions-and-type-names-plan.md) — completed merged Phase 04 implementation plan for expressions, type names, initializers, and expression-dependent declarations.
- [`phase-04-expressions-type-names-and-initializers-codex-prompt.md`](phase-04-expressions-type-names-and-initializers-codex-prompt.md) — ready-to-use Codex implementation prompt for merged Phase 04.
- [`.agents/AGENTS.md`](.agents/AGENTS.md) — compact operational guidance for coding agents; `.claude/CLAUDE.md` imports the same file.
- [`project-status-report.html`](project-status-report.html) — point-in-time repository assessment.
- [`parser-status-report.html`](parser-status-report.html) — detailed parser audit.
- [`double-e-integration-report.html`](double-e-integration-report.html) — feasibility and architecture study for the non-recursive parser.
- [The Double-E Method](https://erikeidt.github.io/The-Double-E-Method.html) — the algorithm description referenced by the architecture study.

The HTML reports are local research notes dated 21 August 2026. Revalidate their claims against the current source and command output before relying on them.

## License

See [`LICENSE`](LICENSE).
