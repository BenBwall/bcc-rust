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

## Initial native baseline (smoke, not final comparison)

The first release harness build completed. On Lua v5.4.8 `onelua.c`, GCC 15.2
preprocessing produces 693,358 bytes. One phase-7 smoke sample took 42.65 ms,
used 616,192 bytes of parse arena and 15,990,600 bytes of additional TU arena,
and made zero global allocations. Eight diagnostics require classification
before this is admitted as a clean corpus. A 16,384-parenthesis function parsed
in 6.96 ms with 8,130,712 bytes of parse-arena high water and zero diagnostics.
These single samples are checkpoint evidence, not the final estimates.

The official SQLite/Lua download domains required interactive network approval;
we did not wait for it. Lua's tagged public GitHub source is available. The
input generator pins archive SHA-256
`d85b70a65f43c5d2254944d58d625e822c8e2e10d9c6a3bd9b5b657e46376a19`.

`linux-perf` 7.0.0-38 reports **No supported events found. The instructions:u
event is not supported.** This VM does not expose the hardware PMU. Hardware
cycles, instructions, branch misses, and cache misses are unavailable; software
`task-clock`, faults, and context switches work. We will use explicitly labelled
Callgrind instruction/cache/branch simulation rather than invent hardware counts.

Valgrind 3.26 rejects each production 100 GiB arena reservation (`mmap` fails
with `EINVAL`), before parsing starts. The separate feature
`parser-study-small-regions` reserves 1 GiB per region **only for profiling**.
It keeps allocation and commit algorithms intact. Native time, RSS, and capacity
runs use the ordinary 100 GiB reservations.

Measurements, prototype coverage, checks, and recommendation will follow here.
