# GCC torture corpus: bcc-rust parser survey

Run date: 2026-09-30. The initial survey exercised **3,878 unmodified GCC torture
sources** and found front-end failures, incorrect literal values, and stalled
recovery. The debugging follow-up below fixes the reduced failures and records
a complete rerun. Historical observations remain below for comparison. These
acceptance counts are not a C99 conformance score.

## Debugging follow-up

The final optimized rerun processed all 3,878 sources with eight workers and a
ten-second subprocess deadline in **261.432 seconds**. It returned **0**, with
**no crashes, abnormal exits, or timeouts**, on both original sources and the
selected GCC-preprocessed inputs.

| Profile | Inputs | Accepted | Error diagnostics | Timeouts | Abnormal exits |
| --- | ---: | ---: | ---: | ---: | ---: |
| Initial debug, original sources | 3,878 | 1,571 | 2,259 | 43 | 5 |
| Final release, original sources | 3,878 | 1,994 | 1,884 | 0 | 0 |
| Initial debug, header-free GCC output | 2,595 | 1,859 | 727 | 9 | 0 |
| Final release, header-free GCC output | 2,596 | 1,952 | 644 | 0 | 0 |

Raw acceptance increased by **423 files**. The final GCC oracle accepted 2,874
sources and rejected 1,004; its earlier `limits-exprparen.c` timeout completed
in this run, adding one file to the header-free subset. Debug and release are
different build profiles, so this table does not compare performance under
identical compiler flags. All reduced regressions also pass in the debug build.

| Failure | Root cause and resulting change |
| --- | --- |
| Repeated call/initializer diagnostics | A failed child returned at an owning grammar boundary, and the call frame retried the same token as another argument. Call recovery now preserves that boundary and the following declaration. |
| Tabs, vertical tabs, form feeds | Whitespace dispatch only recognized a space. All four non-newline whitespace forms now enter the whitespace tokenizer. |
| `Long`, `L92`, `LIM1` | Wide-literal dispatch returned after the first `L`. The fallback now consumes the complete identifier. |
| `0x89ABCDEF`, unsigned exponents, negative exponents | Number tokenization imposed floating grammar too early and wrote `+` for either exponent sign. Preprocessing numbers now retain their spelling, optional sign, and identifier characters until token conversion. `1e-3` produces `0.001`. |
| Escaped backslashes | String-like tokenization did not consume a complete escape pair. Closing quotes are now recognized after an escaped backslash. |
| Native `1.0L` failure | GCC used a 16-byte, 16-aligned `long double`, while libclang's default generated an 8-byte binding. Bindgen now receives Cargo's target and the GNU compiler's builtin include directory; the generated type matches the native ABI. |
| Ordinary multi-character constants | They now retain a signed 32-bit packed integer value in both syntax and preprocessing expressions. `'ab'` has value 24930 under the documented implementation-defined byte-packing policy. |
| Nested macro arguments | An argument was looked up in the callee's parameter map, causing self-expansion or leaving the wrong identifier. Arguments now retain the caller's substitution context. |
| Nested token pasting | The first token of a multi-token argument was pasted instead of its last token. Pasting now preserves preceding tokens and uses both operands' original argument context. |
| Self-referential macros | Active replacement lists were repeatedly expanded. Invocation frames now track disabled macro names; argument expansion retains its caller's disabled set. |
| Parenthesized object-like definitions | Definition lookahead discarded whitespace and incorrectly classified `#define VALUE (3)` as function-like. Function-like definitions now require an immediately adjacent `(`. |
| Large lists and adjacent strings | Repeated provenance merges copied each growing prefix. Lists now flatten their ordered source segments once on reduction; adjacent strings use the same collector. A 1,024-statement reduction previously stored 1,059,863 segments; the regression now requires fewer than 65,536 without losing its statements. |
| Deep case-label chains | Each action rescanned every pending frame to count retained syntax nodes. Push/pop now maintain that count, preserving the existing resource checks without repeatedly scanning the stack. The 30,000-label CLI reduction finishes within its five-second regression deadline. |

The allocation problem was verified with GDB on `limits-blockid.c`: an 8 GB
allocation failed in `Context::merge_vectors` during compound-statement source
accumulation. Removing growing-prefix copies fixed flat-list failures. Deeply
nested syntax can still retain overlapping provenance; a new default budget
of **40 million source segments** emits one structured resource diagnostic
and resets parser state. The existing 4,096-level parser stress checks remain
passing. This is a parser accounting threshold, not a strict process-memory
ceiling or a limit on every preprocessing allocation.

Representative final observations:

| Source | Final original-source result | GCC-preprocessed result |
| --- | --- | --- |
| `compile/20001226-1.c` | Accepted in 0.218 s | Accepted |
| `compile/20010102-1.c` | Diagnostics | Accepted |
| `compile/20011217-2.c` | Bounded diagnostics in 0.102 s | Bounded diagnostics |
| `compile/limits-externalid.c` | Accepted in 0.977 s | Accepted |
| `compile/limits-fnargs.c` | Accepted in 0.054 s | Accepted |
| `compile/limits-idexternal.c` | Accepted in 0.039 s | Accepted |
| `compile/limits-caselabels.c` | Source-storage resource diagnostic in 2.332 s | Resource diagnostic |
| `compile/limits-declparen.c` | Source-storage resource diagnostic | Resource diagnostic |
| `compile/limits-structnest.c` | Source-storage resource diagnostic | Resource diagnostic |
| `compile/pr46534.c` | Accepted in 1.295 s | Not selected: GCC rejected this profile |

The resource diagnostics also occur for `limits-pointer.c` and, on original
input, `limits-exprparen.c`. Those large cases finish but are not accepted.
Other remaining diagnostics include GNU extensions, unavailable headers,
target-specific spellings, and unresolved front-end limitations. They have not
all been classified. A separate value check still exposes incorrect nested
stringification: `QUOTE(int)` with `QUOTE(s) -> QUOTE_(s)` and `QUOTE_(s) -> #s`
currently produces `"s"`. This remains separate preprocessing work; the macro
regressions added here cover substitution, pasting, and rescan termination.

Validation completed:

- `cargo test --all-targets`: **257 passed** (237 unit, 8 existing CLI, 12 torture regressions).
- `cargo test --release --test torture_regressions`: **12 passed**.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `git diff --check`: passed.
- `cargo +nightly fmt --check`: still fails on existing comment wrapping in `float_parsing.rs`, `initial_processing.rs`, `preprocessing.rs`, `last_entry.rs`, and `shared.rs`, plus concurrent import edits in `lib.rs`; the torture-fix code has no formatter differences.

Final evidence:

- [Complete final results, timings, binary checksum, and manifest](./target/compiler-corpus/fixed-release-complete/results.json)
- [Final incremental records](./target/compiler-corpus/fixed-release-complete/results.jsonl)
- [CLI reductions](./tests/torture_regressions.rs)
- [Parser storage and resource regressions](./src/translation_phases/parsing.rs)
- [Canonical test output](./target/torture-tests.log)
- [Optimized regression output](./target/torture-release-tests.log)

## Corpus and compiler practices

The chosen data is the pinned GCC 15.2.0 release's
`gcc/testsuite/gcc.c-torture/compile` and `execute` trees, recursively including
`execute/ieee` and `execute/builtins`. There are 1,988 compile sources and 1,890
execute sources. Supporting torture headers and harness files were extracted
unchanged alongside GCC's root license texts into ignored `target/` storage.

GCC uses these historical regressions across optimization configurations.
LLVM also imports GCC torture execution tests, with exclusions for unsupported
GNU features, target dependencies, and other assumptions. Clang separately uses
Parser/Sema tests with expected diagnostics, while TinyCC uses execution and
preprocessor comparisons. The survey adapts GCC sources to the current
bcc-rust front end; it does not run DejaGnu, compile the C programs into
executables, or check their runtime output. See the
[compiler corpus research](./compiler-test-corpus-research.md) for the broader
comparison and primary sources. [GCC test-suite documentation](https://gcc.gnu.org/onlinedocs/gccint/C-Tests.html),
[LLVM torture harness](https://raw.githubusercontent.com/llvm/llvm-test-suite/cde6a9c353752cc4050561e2b004f5d26e9e109f/SingleSource/Regression/C/gcc-c-torture/execute/CMakeLists.txt)

The release archive is
[`gcc-15.2.0.tar.xz`](https://ftp.gnu.org/gnu/gcc/gcc-15.2.0/gcc-15.2.0.tar.xz).
Its SHA-512 was checked against GCC's
[published checksum](https://gcc.gnu.org/pub/gcc/releases/gcc-15.2.0/sha512.sum):

```text
89047a2e07bd9da265b507b516ed3635adb17491c7f4f67cf090f0bd5b3fc7f2ee6e4cc4008beef7ca884b6b71dffe2bb652b21f01a702e17b468cca2d10b2de
```

## Initial survey observations

The debug CLI was built from the working tree at commit
`0165497c79a1fd577ec85df4327a8f7a0e7f4f60`. Pre-existing CLI edits in
`src/lib.rs`, `src/main.rs`, and `tests/cli.rs` were present and preserved.
The survey took 274.547 seconds with eight worker threads and a ten-second
deadline per subprocess on Windows x86-64.

| Original source inputs | Accepted, possibly with warnings | Error diagnostics | Timed out | Abnormal process exit |
| --- | ---: | ---: | ---: | ---: |
| `compile` (1,988) | 942 | 1,022 | 19 | 5 |
| `execute` (1,890) | 629 | 1,237 | 24 | 0 |
| **All 3,878 files** | **1,571** | **2,259** | **43** | **5** |

296 files produced warnings. A bcc-rust exit code of zero currently also occurs
when it emits syntax/preprocessing errors, so the runner strips ANSI colors and
classifies the printed diagnostic severity instead of equating exit zero with
acceptance. Abnormal exits and timeouts are recorded separately.

The comparison compiler was GCC 13.2.0, MinGW-w64 x86_64-ucrt-posix-seh.
Its syntax-check command used:

```text
gcc -std=c99 -pedantic-errors -fmax-errors=5 -fdiagnostics-color=never -fsyntax-only FILE
```

GCC accepted 2,873 sources, rejected 1,004, and timed out on
`compile/limits-exprparen.c`. Of the sources GCC accepted, bcc-rust accepted
1,446 raw sources, emitted errors for 1,392, timed out on 32, and terminated
abnormally on three.

For GCC-accepted files without `#include` directives, the runner also used
`gcc -std=c99 -pedantic-errors -E -P FILE` and passed that separate output to
bcc-rust. This removes line markers, expands GCC's actual macros, and avoids
host-header syntax. It still traverses bcc-rust's tokenizer and preprocessing
token conversion; it is not direct injection into the language parser.

| Header-free, GCC-preprocessed inputs | Count |
| --- | ---: |
| Total | 2,595 |
| Accepted, possibly with warnings | 1,859 |
| Error diagnostics | 727 |
| Timed out | 9 |

91 preprocessed inputs produced warnings. The change in acceptance is evidence
that preprocessing/tokenization contributes substantially to the raw failures.
It does not identify every cause or prove the resulting syntax trees are correct.

## Initial verified small discrepancies

The following **pre-fix observations** were checked with GCC 13.2.0 and Clang
19.1.5 under `-std=c99 -pedantic-errors -fsyntax-only`, and with freshly built
debug and release bcc-rust binaries. Both comparison compilers accepted every
ordinary-C99 example in this table.

| C input | Observed bcc-rust behavior | Representative upstream source / owner |
| --- | --- | --- |
| `int f(void) {` followed by a line beginning with a tab and `return 0;` | Reports `Unknown token`; an embedded tab in `int\tf(void)` also fails | `compile/20000105-1.c`; tokenizer whitespace dispatch |
| `unsigned value = 0x89ABCDEF;` | Reports missing exponent sign and splits a valid hexadecimal integer | `compile/20000326-1.c`; preprocessing-number tokenization |
| `double value = 1e3;` and `double value = 0x1p3;` | Requires an exponent sign even though it is optional | `compile/irreducible-loop.c`; preprocessing-number tokenization |
| `double value = 1e-3;` | Accepts it, but `--syntax-tree` renders `Float(Double(1000.0))` rather than `0.001` | Exponent tokenization writes `+` for either sign; `1e+3` also renders `1000.0` |
| `int Long;` | Splits the identifier after its initial `L` and reports a declaration error | `compile/930126-1.c` uses `L92`, `L15`, etc.; wide-string/identifier dispatch |
| `int value = '\\';` | Misreads the closing quote and reports a newline in the character literal | `compile/pr12578.c`; string-like escape handling |
| `long double value = 1.0L;` | Debug build reports an invalid floating literal; release build exits with `0xC0000005` on this host | `compile/20080628-1.c`; native long-double conversion boundary |
| `int value = 'ab';` | Reports that multi-character literals are unsupported | `compile/20031203-1.c`; current preprocessing limitation |

The initial `1.0L` release failure reproduced on three additional invocations.
Its ABI cause and fix are recorded in the follow-up above. The initial
multi-character rejection was a reported limitation; the follow-up documents
the selected implementation-defined value policy.

Control inputs `int value1;`, `float value = 1.0f;`, and a C99 `.x = 0`
compound-literal designator were accepted by both bcc-rust builds. These narrow
the observed failures to the spellings or recovery paths above.

## Initial recovery and scale failures

`compile/20011217-2.c` contains a GNU old-style field designator. Unsupported
syntax may be rejected, but the preprocessed input still exceeded 30 seconds in
the optimized build and emitted 127,606 errors, repeatedly pointing at the same
identifier. A smaller malformed input removes the GNU marker entirely:

```c
void f(void) { g((int){x: 0}); }
```

GCC and Clang diagnose this input; both debug and release bcc-rust exceeded a
two-second deadline. The useful regression expectation is bounded recovery and
diagnostics, not acceptance. A C99 designator control with `.x = 0` completes.

Seven original timeout/abnormal-exit cases were rerun with the optimized build,
four workers, and a 30-second per-process deadline. These are follow-up
observations, not replacements for the full-run totals:

| Original input | Optimized raw run | Optimized GCC-preprocessed run |
| --- | --- | --- |
| `compile/20001226-1.c` | Timeout | Abnormal exit |
| `compile/20010102-1.c` | Timeout | Accepted |
| `compile/20011217-2.c` | Timeout | Timeout with repeated diagnostics |
| `compile/limits-externalid.c` | Timeout | Accepted |
| `compile/limits-fnargs.c` | Abnormal exit | Abnormal exit |
| `compile/limits-idexternal.c` | Abnormal exit | Accepted |
| `compile/limits-stringlit.c` | Accepted in 1.133 seconds | Not selected: GCC rejected the source under this profile |

The full-run abnormal exits all returned `3221226505` (`0xC0000409`) and no
captured diagnostics. This establishes a fatal process failure, not its cause.
Some scale tests greatly exceed C99's minimum translation limits; exceeding
those floors is not by itself a conformance bug. A deadline also does not prove
an infinite loop. The repeated-diagnostic reproducer is stronger evidence of
a recovery/progress problem than the large-input timeouts alone.

## Reproduction and evidence

Build and run from the repository root; Python uses only its standard library:

```powershell
cargo build --bin bcc-rust
python scripts/run_gcc_torture.py --output target/compiler-corpus/new-run
```

Use a fresh output directory for each run. `--limit 30` gives a smoke run;
repeatable `--match` arguments select corpus-relative globs. For example:

```powershell
cargo build --release --bin bcc-rust
python scripts/run_gcc_torture.py --bcc target/release/bcc-rust.exe --timeout 30 --match 'compile/20011217-2.c' --output target/compiler-corpus/recovery-rerun
```

The runner verifies the pinned SHA-512, preserves original source content and
per-file hashes, clears ambient include-search variables for both tools, records
binary hashes and Git state, and writes incremental JSONL, complete JSON,
preprocessed sources, and nonempty diagnostic logs under `target/`. Raw runs
have no configured system-header paths; `Header not found` is an environment
limitation, not evidence of a language-parser bug. Local include fixtures are
retained. GCC's DejaGnu options, target skips, and multi-file test contracts are
preserved in the corpus but are not interpreted by this survey.

The survey returns a nonzero status for crashes/process failures, timeouts,
or a failed external preprocessing subprocess. Ordinary diagnostics do not fail
the survey, because it intentionally includes extension and semantic cases.
The full survey and optimized retry both returned 1 due to their recorded
failures. This runner is exploratory evidence collection, not a pass/fail
replacement for the repository's tests.

Evidence from this run:

- [Full per-file results and manifest](./target/compiler-corpus/results/results.json)
- [Incremental full-run results](./target/compiler-corpus/results/results.jsonl)
- [Optimized retry results](./target/compiler-corpus/release-retry/results.json)
- [Reduced probe sources and observations](./target/compiler-corpus/probes/results.json)
- [Negative-exponent syntax tree](./target/compiler-corpus/probes/negative-exponent.tree.log)
- [Runner](./scripts/run_gcc_torture.py)

For the initial investigation, `cargo build --bin bcc-rust`,
`cargo build --release --bin bcc-rust`, and `cargo test --test cli` passed
(eight CLI tests). Compiler fixes and their validation are recorded in the
debugging follow-up above.

## Interpretation and next use

The reduced recovery, literal, identifier, whitespace, native ABI, macro, and
scale failures now have regression coverage. Future corpus work should classify
the remaining diagnostics and check accepted syntax values, including the
nested stringification limitation recorded above.

Many other failures are expected from GNU attributes/assembly, reserved
keywords/builtins, missing target definitions and headers, or tests requiring
non-C99 modes. Even GCC acceptance with `-pedantic-errors` does not exclude
reserved GNU forms, and `-fsyntax-only` also checks semantic constraints.
Those cases need review before becoming parser expectations. None of the
aggregate accepted counts proves AST correctness, semantic correctness,
optimization behavior, or runtime correctness. [GCC alternate keywords](https://gcc.gnu.org/onlinedocs/gcc/Alternate-Keywords.html),
[Clang stage selection](https://clang.llvm.org/docs/CommandGuide/clang.html#stage-selection-options)
