# Recursive descent parser study

Status: investigation in progress, 2026-10-21. Baseline source: `145898d4`.
All study code is opt-in; the default parser remains the explicit frame machine.

## Question and constraints

Compare phase 7 alone with a recursive expression/statement prototype using the
same immutable syntax nodes, translation-unit arena, provenance, and declaration
machinery. The current parser guarantees arbitrary nesting without native stack
growth, structured recovery, and resource diagnostics. A speed comparison cannot
silently trade these away. C99 translation floors are minimum capabilities, not
a justification for rejecting everything deeper.

## Method (planned and being recorded)

Run every Cargo operation from this worktree using:

```sh
python3 /home/agent/workspace/bcc-rust/triage-2026-10-16/swarm/run.py cargo <arguments>
```

Retain study binaries inside the same wrapper invocation that builds them;
shared target directories may be cleaned by sibling jobs. Long jobs are detached,
and results are saved before waiting. Use at least seven interleaved repetitions,
report minimum and variation, and separate parser time from preprocessing,
inspection, setup, destruction, and process peak RSS. Report global allocation
counts separately from arena allocation/high-water metrics: the compiler's
ordinary path intentionally makes zero global allocations.

Inputs will include checked-in C benchmarks, a larger pinned real-world source
if available, and generated nesting/chain/initializer stress cases. The harness
will preprocess each source once before the measured parser interval. Differential
checks will compare iterative syntax inspection including locations, plus
structured diagnostic output where exposed; matching root counts alone is not
an AST identity check.

## Evidence saved so far

- `parser-roadmap.md`, parser entry docs, `machine.rs`, `frame_pool.rs`,
  `limits.rs`, benchmark preparation/memory APIs and allocation tests read.
- Linux wrapper uses six build workers, two serialized build slots, dev/test
  debug symbols disabled, and release fat LTO with full debug symbols.
- `/proc/sys/kernel/perf_event_paranoid` is `2`; `perf` and `valgrind` were not
  initially on PATH. Counter availability is not established yet.
- Frame pools already recycle vectors/boxes by high-water concurrency; replacing
  them with heap allocation would not be a fair memory comparison.

Measurements, prototype coverage, checks, and recommendation will follow here.
