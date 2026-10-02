# Parser performance continuation prompt

Continue the bcc-rust parser performance work. The full original brief is in
`parser-performance-handoff-prompt.md` at the repository root; read it first and
treat it as authoritative for goals, work-item order, constraints,
verification, and the final report. This document records what the previous
session established and decided, so do not re-ask those questions.

## Status at handoff (2026-09-30, ~21:30)

No parser work items have started. `src/translation_phases/parsing.rs`,
`src/translation_phases.rs`, and `GLOSSARY.md` have not been touched by the
previous session.

### Blocker: concurrent edits to `parsing.rs`

Another agent was editing `parsing.rs` during the previous session: its
`ParserLimits::default().source_segments` changed from `4_000_000` to
`40_000_000` mid-session, and `tests/torture_regressions.rs` was rewritten.
Last observed modification times: `parsing.rs` 20:41, `tests/torture_regressions.rs`
20:47. Before editing, run `git status --short`, check modification times
again, and ask the user to confirm the other agent has finished. If the files
are still changing, stop and ask.

The user decided to **build on top of the uncommitted working tree** (it
contains the other agent's work, including the `source_segments` resource
limit enforced in `Parser::drive` and a new `source_storage_exhaustion...`
test). Preserve all of it.

### Baseline evidence already observed

- With the 4M limit, `cargo test --all-targets` had 236 passed / 1 failed:
  `deeply_nested_expressions_use_heap_backed_parser_frames` exceeded
  `SourceSegments` because of quadratic provenance copying (work item 1). Rerun
  with the current 40M limit; it may now pass.
- The mixed benchmark was measured at ~62.8M source vectors, which exceeds even
  40M, so `bench_parser`'s zero-diagnostics setup assertion will likely fail
  until item 1 lands. To take a pre-change baseline, raise the limit only
  temporarily in an uncommitted measurement build, or measure with a temporary
  harness; do not commit that.
- `cargo clippy --lib --bench bench --features benchmarking-internals -- -D warnings`
  fails (seen on Linux) with three pre-existing errors in the uncommitted bench
  helpers in `src/lib.rs`: `must_use_candidate` on `preprocess_one_million`
  and `one_million_input_bytes`, and `large_include_file` on the
  one-million-lines `include_str!`. Report these as existing evidence; ask
  before fixing, since that code belongs to the other in-progress work.

## Decisions already made by the user

1. **`coz` build blocker: done.** `coz` is an optional dependency enabled by
   `benchmarking-internals = ["dep:coz"]`. Because `coz` 0.1.3 does not compile
   on Windows, it is declared under `[target.'cfg(unix)'.dependencies]`, and
   `src/main.rs` gates `coz::progress!` with `#[cfg(unix)]`. `src/lib.rs` has
   `#[cfg(all(unix, feature = "benchmarking-internals"))] use coz as _;` to
   silence `unused_crate_dependencies` on Linux. `src/main.rs` also gates the
   `ExitCode` import and discards `preprocess_one_million()`'s result with
   `_ =`. The Windows release build with the feature has no warnings.
   These changes are uncommitted and mixed with the other agent's diff in
   `Cargo.toml`, `Cargo.lock`, `src/main.rs`, and `src/lib.rs`.
2. **Provenance design for work item 1: a parser provenance log.** When the
   parser first consumes a token, copy its source vectors once into a
   parser-owned append-only arena, and tag `SourceVectors` (for example with the
   high bit of `start_index`) to say which arena a range indexes.
   `Context::get_source_vectors` dispatches on the tag, so the ~99 merge call
   sites and all consumers stay unchanged. Merges of ranges that are adjacent
   in that log (the normal consumption-order case) become O(1) range
   extensions; out-of-order merges (recovery, discarded ranges) may still copy
   into the log. Required properties:
   - Output must be byte-identical: CLI diagnostics print the **full list** of
     constituent vectors (`print_translation_error` and
     `format_parser_diagnostic_details` in `src/lib.rs`), and inspection with
     `--syntax-locations` uses the first vector's position
     (`parsing/inspection.rs`).
   - Copy each token only once, even though the same token is seen several
     times through `Reprocess` and lookahead (`TokenCursor::current`,
     `lookahead`, `consume` in `parsing.rs`).
   - The source-segment limit check currently counts only
     `context.source_vectors.0.len()`. Decide whether the log counts toward
     `ParserLimits::source_segments`, keep the resource-limit tests passing, and
     state the decision in the report.
   - Update the **Source vector** entry in `GLOSSARY.md` in the same change, and
     add a test that source-vector storage stays linear in token count.

## Profiling setup that works (WSL `coz`)

Sampling profilers need admin on Windows; causal profiling works in WSL Ubuntu.

- Ubuntu's `coz-profiler` 0.2.2 package is broken on this system (glibc 2.43):
  it aborts at startup with `Thread state not found` even for a trivial C
  program, and it cannot read DWARF 5. Use the upstream build instead:
  `~/coz-src/coz` (built from plasma-umass/coz `master` 256e3c3; no root needed).
  It writes `profile.jsonl`, not `profile.coz`.
- `coz` needs `perf_event_paranoid <= 1`; the default is 2. The user approved
  temporarily setting it with
  `wsl.exe -u root -e sh -c 'echo 1 >/proc/sys/kernel/perf_event_paranoid'`
  and restoring it to 2 afterwards. It is currently 2. Ask again before
  changing it in a new session.
- Profile a snapshot, not the Windows checkout:
  `rsync -a --delete --exclude target --exclude .git /mnt/c/Users/benbw/Documents/GitRepo/bcc-rust/ ~/coz-bcc/`,
  then `cargo build --release --features benchmarking-internals --bin bcc-rust`
  and `timeout -s INT 1800 ~/coz-src/coz run -s "%/coz-bcc/src/%" --- ./target/release/bcc-rust`.

### Results of the one completed run (preprocessor only)

The bench-mode `main` loops `preprocess_one_million()` with one progress point
per iteration, so this profiled the **preprocessor**, not the parser. 30
minutes, 222 experiments, 45 lines; the noise floor is about ±0.1-0.26.

- `preprocessor_tokenizer.rs:241` `context.string_cache.end_str()` (per-token
  spelling interning: hash, dedup probe, pop/truncate on hit): 62 experiments;
  ~+10% program speedup at 10-20% line speedup, ~+20-27% at 65-100%. The only
  robust signal.
- `preprocessor_tokenizer.rs:148` token dispatch (LTO-inlined scanners): ~+10%
  at 20%, plateauing around +15-25%.
- `preprocessing.rs:4020` `parse_number`: weak (5 experiments), ~+20% at 75-85%.
- `string_cache.rs:251` `get_impl` and `initial_processing.rs:72`: ≈0.

The preprocessor is outside the parser work items unless the user asks. To
profile the parser with `coz`, the bench-mode `main` would need to drive the
parse helpers with a progress point per external declaration; ask the user
before changing that.

## Next steps

1. Confirm with the user that the other agent is finished with `parsing.rs`.
2. Rerun `cargo test --all-targets` and the other canonical checks to
   establish the current baseline, and record the current failures.
3. Take pre-change measurements (min of ≥7 interleaved runs; Criterion where
   the setup assertions allow), including source-vector count and steps per
   token, using temporary uncommitted instrumentation.
4. Implement work item 1 with the design above, then items 2-5 in order, as
   the original brief describes. Commit only when the user asks, one work
   item per commit, following `CONTRIBUTING.md`.
