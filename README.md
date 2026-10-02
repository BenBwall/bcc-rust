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

Diagnostics are rendered like `rustc`'s: a lowercase message, the
`file:line:column` location, the quoted source line with `^` under the problem
and `-` under related input, and `= note:` / `= help:` lines, which cite the
C99 clause (N1256) where a rule was broken. Messages quote source spellings, so
output never depends on interned-string handles, arena indices, or host
`long double` layout. An error at exactly the same place as the one before it
is folded into that error, and the run ends with a summary such as
`2 errors and 1 warning generated.` Color is used on terminals unless
`NO_COLOR` is set; `CLICOLOR_FORCE` forces it.

`--tokens` prints one line per token: its location, kind, source spelling, and
for constants the value and C type (for example
`` <input>:1:1: floating constant `1.5L` = 0x1.8p+0 (long double) ``).
`long double` values print as exact hexadecimal without the C library's
`printf`. `--syntax-tree` spells operators, qualifiers, storage classes, and
constants as C, and `--syntax-locations` names the file for nodes outside the
main source.

`Parser::parse_translation_unit` returns an ordered `ParsedTranslationUnit` and
validated, read-only `SyntaxTree`. Declarations, prototype-style and old-style
function definitions, blocks, every C99 statement family, expressions, type
names, and initializers run through one explicit heap-backed frame stack.
Malformed input retains repaired syntax where meaningful, produces a
provenance-only external error node for pure top-level garbage, and emits
structured FIFO diagnostics with recovery context.

Ordinary multi-character constants pack their UTF-8 execution bytes into a
signed 32-bit integer, most-significant byte first, retaining the final four
bytes. The same value is used in syntax nodes and `#if` expressions. This is
the implementation-defined choice for constants such as `'ab'`.

Parser resource accounting includes source provenance, with a default budget
of 40 million stored segments per translation unit. Extreme nesting can
produce a `SourceSegments` resource diagnostic; flat syntax lists collect
their provenance once instead of repeatedly copying their growing prefix.

Phase 05 is complete. The parser meets the parser-relevant C99 minimum
translation floors, diagnoses excluded extensions and invalid phase-7 input,
and has deterministic truncation/property coverage. Semantic analysis—including
type/lvalue constraints, constant-expression evaluation, initializer
current-object rules, and linkage—and code generation remain unimplemented.
This is not yet a production-ready or conforming C99 compiler.

## Prerequisites

- Rust 1.98.0, pinned in [`rust-toolchain.toml`](rust-toolchain.toml) and declared
  as the minimum supported version in [`Cargo.toml`](Cargo.toml).
- Clippy and Rust's `llvm-tools` component, both selected automatically by
  `rust-toolchain.toml`.
- Git, CMake 3.20 or newer, Ninja, Python, and a C++17 compiler for building
  Clang, libclang, and LLD from Rust's pinned LLVM sources. On Windows GNU,
  installed Visual Studio C++ Build Tools are preferred for this host-tool
  build; MinGW GCC/G++ can also bootstrap it. The first Cargo build compiles
  LLVM; later builds reuse the Ninja cache under ignored `target/llvm/`.
- For Windows GNU, MinGW headers. The build finds the installation through
  `gcc.exe` on `PATH`, or accepts `MINGW_ROOT` explicitly. GCC does not compile
  the C helper. For MSVC, the Visual Studio C headers and SDK are required.
- On Linux, the target's static C runtime development libraries.
- Nightly Rustfmt for the repository's unstable formatting options.

## Source submodules

[`vendor/rust`](vendor/rust) is a shallow, filtered submodule pinned to the
same commit as Rust 1.98.0 (`88d9e12ae178fab0fb5cc050a94da85685d449ea`). Its
LLVM submodule pins `52ed14fcd56afc30f9cccd8ca8ce237c2eef7e04` (LLVM 22.1.8).
Git pins both source revisions; the native build checks that Clang, libclang,
and the archiver report the same LLVM version as rustc.
Upgrading Rust requires updating the Rust submodule to the new compiler's
commit and checking out the LLVM revision that it pins.

Sparse patterns in [`build_support/rust.sparse`](build_support/rust.sparse)
retain Rust's LLVM gitlink, submodule metadata, and license files.
[`build_support/llvm.sparse`](build_support/llvm.sparse) retains the LLVM,
Clang, LLD, supporting build sources, and LLD's required libunwind header;
tests, documentation, and examples are excluded except for small CMake
entrypoints required by configuration.
Git stores sparse-checkout settings locally, so prepare a fresh checkout with
these PowerShell commands:

```powershell
git clone --filter=blob:none --no-checkout --depth 1 --branch 1.98.0 https://github.com/rust-lang/rust.git vendor/rust
Get-Content build_support/rust.sparse | git -C vendor/rust sparse-checkout set --no-cone --stdin
git submodule update --init --checkout vendor/rust
git -C vendor/rust submodule init src/llvm-project
git init vendor/rust/src/llvm-project
git -C vendor/rust/src/llvm-project remote add origin https://github.com/rust-lang/llvm-project.git
git -C vendor/rust/src/llvm-project config remote.origin.promisor true
git -C vendor/rust/src/llvm-project config remote.origin.partialclonefilter blob:none
$llvmCommit = git -C vendor/rust rev-parse HEAD:src/llvm-project
git -C vendor/rust/src/llvm-project fetch --depth 1 --filter=blob:none origin $llvmCommit
Get-Content build_support/llvm.sparse | git -C vendor/rust/src/llvm-project sparse-checkout set --no-cone --stdin
git -C vendor/rust/src/llvm-project checkout --detach $llvmCommit
git submodule absorbgitdirs vendor/rust
```

## Build, test, and inspect

Run checks from the repository root:

```sh
cargo build
cargo test --all-targets
cargo +nightly fmt --check
cargo clippy --all-targets -- -D warnings
```

Cargo explicitly targets the host so build scripts and proc macros do not
receive the executable's LTO flags. Artifacts are consequently under
`target/<host-triple>/debug/` or `target/<host-triple>/release/`.
Explicit `embed-bitcode=yes` keeps this configuration compatible with Cargo's
debug and test profiles as well as release builds.
`RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` override Cargo configuration, even
when empty; unset them for the standard build. The native build rejects
overrides that remove linker LTO.

### Native C, static linking, and cross-language LTO

[`build_support/native.rs`](build_support/native.rs) compiles the C helper with
the `cc` crate and Clang `-flto=full` into a static archive of LLVM bitcode.
[`build_support/llvm.rs`](build_support/llvm.rs) uses the `cmake` crate and
LLVM's `install-distribution` target to build only Clang, its resource headers,
libclang, `llvm-ar`, and LLD from Rust's exact LLVM source revision. Only the
host's backend is enabled. Rust uses
`-C linker-plugin-lto -C lto=fat`, allowing the final linker to optimize across
the Rust/C boundary. The [Rust LTO documentation](https://doc.rust-lang.org/rustc/linker-plugin-lto.html)
describes this combination and why matching LLVM versions matters.

Cargo selects the source-built linker directly from `target/llvm/bin/`.
The C compiler, archiver, libclang, and linker all come from that pinned
build; no setup script or linker wrapper is needed. The shared LLVM cache is
locked while CMake builds and installs the tools, so
independent Cargo builds cannot write its outputs simultaneously.
Rust libraries and the native C helper are statically linked into the executable.
On Windows GNU, Windows' system DLLs—including `msvcrt.dll`—remain OS
dependencies; no LLVM, libclang, Rust, GCC, or pthread DLL needs to accompany
the executable. Linux uses the
matching Clang/LLD driver and requests a static CRT; MSVC also requests a static
CRT. The complete build and executable verification have been exercised on
`x86_64-pc-windows-gnu`; Linux and MSVC configurations require validation on
their respective hosts.

libclang is loaded only by the binding generator during the build. It is not
linked into the resulting executable. Rust's `llvm-tools` component supplies
optional symbol and disassembly tools for inspecting the executable.

Float and double conversions go through the C helper as well. On MinGW it
selects the C99 conversion routines explicitly, so hex floats keep working
without relying on the linker's choice of a legacy Windows `strtod` symbol.

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

Use `--iquote <directory>` (`-q`) and `--isystem <directory>` (`-s`) to add include search paths. Header lookup follows GCC and Clang: `#include "name"` looks beside the including file (the working directory for `--input`), then each `--iquote` directory; both forms then search `CPATH`, each `--isystem` directory, and `C_INCLUDE_PATH`. The working directory is never searched implicitly. The environment variables use the platform path separator, and, as in GCC, an empty element names the working directory. A missing header's diagnostic lists every directory searched. `__DATE__` and `__TIME__` are fixed once per translation unit and honor `SOURCE_DATE_EPOCH` for reproducible output. Files under [`test-programs/`](test-programs/) are useful manual inspection inputs, but they are not an automated conformance suite.

### External torture corpus

`cargo test --test torture_regressions` runs the reduced regressions found in
the corpus, including literal values, macro expansion, native long-double
conversion, and bounded recovery. The parser unit tests also check that long
syntax lists retain source provenance with linear storage growth.

After `cargo build --bin bcc-rust`, run
`python scripts/run_gcc_torture.py --output target/compiler-corpus/new-run`
to download and checksum-verify the pinned GCC 15.2.0 torture corpus, then survey
its compile and execute sources through this front end. Use a fresh output
directory for each run. `--limit 30` runs a smoke sample; `--match` selects
corpus-relative globs. Sources and detailed results stay in ignored `target/`.
The runner reports crashes/timeouts with a nonzero exit status and does not
execute C programs. It is a corpus survey rather than a conformance gate.
See the [compiler corpus research](compiler-test-corpus-research.md) and
[recorded parser results](gcc-torture-parser-results.md) for comparison profiles,
known discrepancies, and interpretation limits.

## Pipeline and code map

| Area | Role |
| --- | --- |
| [`src/translation_phases/initial_processing.rs`](src/translation_phases/initial_processing.rs) | Normalizes source characters, line endings, trigraphs, escaped newlines, and comments. |
| [`src/translation_phases/preprocessor_tokenizer.rs`](src/translation_phases/preprocessor_tokenizer.rs) | Produces preprocessing tokens while retaining source provenance. |
| [`src/translation_phases/preprocessing.rs`](src/translation_phases/preprocessing.rs) | Handles macros, directives, includes, conditional preprocessing, literals, and conversion to parser-facing tokens. It owns the preprocessor-expression evaluator and its values and diagnostics. |
| [`src/translation_phases/parsing.rs`](src/translation_phases/parsing.rs) | Contains the explicit parser driver; declaration, function-definition, statement, expression, type-name, initializer, declarator, and tag frames; syntax stores and scopes; and parser diagnostics. |
| [`src/translation_phases.rs`](src/translation_phases.rs) | Defines the shared translation-phase interface, compilation context, source provenance, and diagnostic plumbing. |
| [`src/util/`](src/util/) | Provides project-specific arenas, interned strings, shared storage, queues, stacks, and vector slices. |
| [`src/diagnostics.rs`](src/diagnostics.rs) | Builds diagnostics (message, labelled source ranges, notes, help) and renders them as annotated source snippets. Each phase's error type explains itself through a `ToDiagnostic` implementation. |
| [`src/lib.rs`](src/lib.rs) | Wires the inspection CLI to the parser by default and to the token dump with `--tokens`, and reports diagnostics. |

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
