# Parser performance handoff prompt

Implement a series of measured performance improvements to the bcc-rust C99
syntax parser (`src/translation_phases/parsing.rs`) without changing its
observable output: syntax trees, diagnostics, recovery, and provenance must stay
equivalent. Work in the priority order below, measure every step against a
baseline, and stop to ask if a change would require altering a documented domain
concept or public parser behavior.

## Read first

1. `.agents/AGENTS.md` (repository rules, canonical checks, commit policy
   pointer) and `CONTRIBUTING.md` before committing.
2. `CONTEXT.md` — especially **Source vector**, **Context**, the parser machine
   vocabulary (`ParserMachine`, `ParseFrame`, `ParseAction`, `ParseValue`,
   synchronization sets). The explicit-stack frame architecture is an agreed
   design decision: do **not** convert the parser to recursive descent or
   introduce Rust call recursion over grammar nesting.
3. `benches/bench.rs`, the `benchmarking-internals` helpers at the bottom of
   `src/lib.rs`, and `gen_parser_mix()` in `build.rs` — the parser benchmark
   you will use.

Run `git status --short` first. Other work may be in progress in this checkout
(another agent was recently editing `build.rs`, `src/main.rs`, `parsing.rs`,
`preprocessing.rs`, and tests). Preserve all unrelated changes; if files you
need are mid-edit by someone else, stop and ask rather than overwrite.

## Benchmark setup and known blockers

Parser benchmarks (Criterion, group `Parser`):

```sh
cargo bench --features benchmarking-internals --bench bench -- Parser
```

- `Parser/one million lines`: 1,000,000 × `int iN = N;` (same input as the
  existing `Preprocessor` benchmark, so parse overhead = difference).
- `Parser/mixed C99 workload`: generated ~21.5 MB / 560K lines / 5.24M tokens
  with typedefs, structs with bit-fields, enums, designated initializers,
  function pointers, and functions containing loops, `switch`, casts, `sizeof`,
  and precedence-heavy expressions.
- Setup asserts zero diagnostics and 1,000,000 roots before timing.

Blockers observed when this was written (verify whether they still exist):

- `src/main.rs` contains `coz::progress!` under `benchmarking-internals`, but
  `coz` is not a dependency, and Cargo builds the binary for benches, so
  `cargo bench` fails. Ask the user how they want this fixed (add `coz` as an
  optional dependency, gate it behind its own feature, or remove it) rather
  than deciding unilaterally.
- A bindgen `--target` change in `build.rs` failed locally with
  `'mm_malloc.h' file not found` (MinGW headers). It may already be fixed.

Profiling: sampling profilers (flamegraph/blondie, samply) need admin on this
Windows machine. Use instrumentation instead: temporary `AtomicU64` counters and
`core::arch::x86_64::_rdtsc()` cycle attribution, kept out of committed code.
The machine is shared and noisy; compare variants with **minimum of ≥7
interleaved runs**, not single runs.

## Baseline measurements (from the investigation)

Release build, min-of-7, "drive" = the full `parse_translation_unit` loop
including lazily pulled preprocessing.

| Workload | Preprocess only | Drive (pp + parse) | Parse only (approx.) |
|---|---|---|---|
| Mixed C99 (5.24M tokens) | ~0.77 s | ~2.79 s | ~2.0 s (~400 ns/token) |
| One million lines (5M tokens) | ~1.2 s | ~2.85 s | ~1.65 s |

Criterion (full pipeline): Preprocessor 1.045 s (19.9 MiB/s); Parser 1M lines
3.80 s (5.5 MiB/s, noisy); Parser mixed 3.18 s (6.4 MiB/s).

Machine statistics:

- Mixed: 13.9M driver steps (~2.7 per token), 2.72M `Push`, 3.1M `Reprocess`,
  average frame-stack depth 5.7, max 15.
- 1M lines: 21M steps (4.2 per token), 5M `Push`, 5M `Reprocess`.
- `size_of`: `ParseFrame` 240, `ParseAction` 240, `FrameStep` 248,
  `ParseValue` 56, `Token` 40, `InitializerFrame` 240, `ExpressionFrame` 168,
  `DeclaratorFrame` 144, `StatementFrame` 128, `SourceVector` 32.
- Cycle split (mixed): preprocessor 27%, frame `step` bodies 44%, driver and
  everything else 29%. Costliest frames per step: FunctionDefinition ~1265
  cycles, CompoundStatement ~1035, Statement ~587, Expression ~382.
- `SyntaxTree::validate` is ~15 ms (irrelevant). The per-step resource-limit
  checks cost only ~1% when disabled.

## Work items, in priority order

### 1. Make provenance merging O(1) (largest measured win)

`Context::merge_vectors` in `src/translation_phases.rs` copies every
`SourceVector` of both inputs to the end of `context.source_vectors` whenever the
ranges are not adjacent. Parser frames merge provenance incrementally (per token
via `Parser::merge_source`, per child in compound statements, declarations,
etc. — ~99 call sites), and once a range has been copied it is never adjacent
again, so each later merge recopies the growing range: quadratic per construct.

Evidence: on the mixed input, 5.24M tokens produced **62.8M** source vectors
(~2 GB), with 5.5M merge calls of which 4.48M copied a total of 53.9M elements.
On 1M lines, all 6M merges copied (19M elements). Replacing the copy with
`return v1` (incorrect, upper bound only) reduced parse-only time by ~29% on the
mixed input and ~17% on 1M lines.

Goal: merged parser provenance should be constant size with no allocation, and
diagnostics must render the same locations and ranges as before (check
`get_source_vectors` consumers, the CLI diagnostic renderer in `src/lib.rs`, and
`inspect`/`--syntax-locations` output). Viable designs to evaluate:

- A span representation (first and last constituent source vectors, or start
  and end token provenance) for parser-constructed syntax, resolved lazily when
  rendering.
- Keeping `SourceVectors` but making merges of ordered, disjoint parser ranges
  produce a covering range without copying, if you can prove the covered
  indices are exactly the intended ones (beware macro-expanded tokens that
  carry several non-adjacent vectors and the entries earlier merges appended).

This changes the meaning of a documented concept, so update `CONTEXT.md`
(**Source vector** entry) in the same change and confirm the design with the
user before a large refactor. Add a regression test or benchmark assertion
proving source-vector growth is linear in token count (e.g. the vector table
after parsing the mixed input stays within a small constant factor of tokens).

### 2. Step frames in place and shrink `ParseFrame`

The driver (`Parser::drive`) pops the top frame by value, calls `step`, and
pushes it back; `ParseAction::Push` carries a whole child frame inside a
248-byte `FrameStep`. Restructure so the active frame is stepped through a
mutable reference: split the frame stack from the parser state that `step`
needs (for example, a `frames: Vec<ParseFrame>` alongside a separate
`ParserState` struct passed to `step`), so `frames.last_mut()` and
`&mut state` borrow disjointly. Reduce `ParseFrame` size by boxing cold or
rarely used payloads of the largest frames (`InitializerFrame` 240 B,
`ExpressionFrame` 168 B, `DeclaratorFrame` 144 B) without adding an allocation
to every common push. Keep `ParseAction` small (push a frame kind or constructor
data rather than a full frame if that helps). Recovery (`Recover` →
`recover()` + `merge_recovered_sources`) and the syntax checkpoint/restore on
resource failure must keep working.

### 3. Remove avoidable driver round trips

`Reprocess` accounts for 22–24% of steps. Let a frame continue through phase
changes within one `step` call when no child push, token consumption, or
reduction is needed, instead of returning to the driver. Consider fast paths
for very common shapes that currently push child frames, such as a plain
identifier declarator or a single-token primary/initializer expression, if
they keep one code path for diagnostics and recovery. Measure steps per token
before and after.

### 4. Make resource-limit accounting incremental (small)

Each step folds `retained_node_count()` over the whole frame stack twice, sums
22 arena lengths in `SyntaxStore::node_count()`, and takes a 22-field
`SyntaxStoreCheckpoint`. Track the retained and arena node totals
incrementally. Preserve exact limit semantics and the existing
resource-limit tests.

### 5. Minor items (only if cheap)

- `CompoundStatementFrame` interns `"__func__"` on every function body;
  intern it once.
- Each nested scope creates a `HashMap`; consider reusing scope maps.
- The preprocessor is 27–35% of total time; note findings but treat it as a
  separate task unless the user asks.

## Constraints

- No grammar recursion; preserve the frame machine, synchronization sets,
  structured diagnostics, recovery ownership, typedef scope handling, streaming
  `next_item`, and resource limits.
- Parser output must be unchanged: run the full test suite, including CLI
  snapshot tests and `tests/torture_regressions.rs` if present.
- Do not commit profiling instrumentation.
- Keep changes reviewable: one work item per commit, following
  `CONTRIBUTING.md` for message and hook policy. Commit only when the user asks.

## Verification for every item

```sh
cargo test --all-targets
cargo +nightly fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --lib --bench bench --features benchmarking-internals -- -D warnings
git diff --check
cargo bench --features benchmarking-internals --bench bench -- Parser
```

Report pre-existing failures as current evidence rather than working around
them. For each work item, report: Criterion results before/after for both
parser benchmarks, min-of-7 parse-only times, steps per token, source-vector
count after the mixed input, and peak memory if you can observe it.

## Final deliverable

A short report with a before/after table per work item, the design chosen for
provenance spans (and the `CONTEXT.md` change), anything deferred, and
remaining hotspots, including an updated estimate of the frame machine's
overhead relative to parse time.
