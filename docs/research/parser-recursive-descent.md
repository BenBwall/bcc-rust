# Recursive descent parser study

> **Status (2026-10-25):** recursive descent is now bcc's only parser
> (`src/translation_phases/parsing/grammar/`); the frame machine, the study
> modes and their harness (`parser-recursive-descent-study`, `stacker`,
> `examples/parser_study`, `scripts/parser_study*`) were removed. The command
> line always runs on the reserved compilation stack, and nesting is bounded
> by `--max-nesting-depth`. This report is kept as the record of the
> measurements behind that decision; its commands need the source revision
> it names.

Follow-up: [whole-compilation worker and recursive lowering](recursive-lowering.md).

Follow-up: [recovery parity and Windows validation](parser-recovery-windows.md)
records all ten recovery fixes and completed native Windows GNU validation in
release/debug. A 100 GiB stack starts with 8 KiB writable; reserved RD reduces
native synthetic parser time by 30.5–46.7% release and 22.1–31.5% debug, with
worker launch excluded. The measurements and open blockers below describe the
original cohort; the follow-up supersedes its recovery/Windows status.

2026-10-21. Original baseline: `145898d4`. Study branch:
`codex/parser-recursive-descent-study`. Native binaries were built at `e8f95dd6`
(the parser prototype is `2ed14574`; final Linux harness code is `fee12e29`).
Later commits add analysis, inputs and collection scripts. Binary hashes and
sample observations are archived in [the data directory](parser-recursive-descent-data/).

A [2026-10-22 follow-up](parser-reserved-stack.md) implements and measures a
large, demand-paged native stack for recursive parsing. It finds a 39% release
Lua parser speedup with a 100 GiB reservation using 44 KiB of stack pages after
parsing, adding a reserved worker as an alternative to the hybrid recommendation
below. The original measurements and their interpretation remain recorded here.

## Recommendation

**Pursue a bounded hybrid if changing the parser architecture; retain the frame
machine as the shipping default until recovery parity is complete.** There is a
real speed opportunity: on clean Lua, recursive parsing reduces release parser
time by **36.1%** and hybrid depth 16 by **30.2%** against the feature-disabled
production baseline. Debug reductions are **32.8%** and **24.6%**. The hybrid
preserves an iterative path for arbitrary nesting and stays within the tested
1 MiB stack. This prototype is not ready to replace the default: ten malformed
corpus cases produce different recovered trees/provenance.

There is **no material real-input process-memory win**: all clean-Lua variants
peak around 33.6 MiB in release. Recursion moves working state to native stacks;
`stacker` makes extreme nesting safe in these tests but can greatly increase
debug memory. A large worker stack and a generic depth limit do not preserve
the current nesting guarantee. Independently, the one-line alignment-cache reuse
experiment saves 242,544 bytes of Lua scratch with identical inspected output.
Promote that optimization separately after normal review.

## Prototype and comparison boundary

The opt-in `parser-recursive-descent-study` feature implements precedence-climbing
expressions, prefix/postfix operators, every C99 precedence/associativity level,
casts, compound literals, both `sizeof` forms, alignment operators, and recursive
statements/blocks. GNU and later-language atom/statement productions fall back to
the existing extension frames. Declarations, declarators, type names and
initializers retain their existing frames. This is a meaningful mixed parser,
not a fully recursive declaration/declarator implementation.

It uses the same node constructors, TU arena, ordered provenance, typedef scopes,
label/switch state and pooled call/block collections. Operator definitions are
shared. The default pipeline always selects the original machine. No feature
in this study is enabled by default.

- [Prototype entry and policies](../../src/translation_phases/parsing/recursive_descent.rs)
- [Expressions](../../src/translation_phases/parsing/recursive_descent/expressions.rs)
  and [statements](../../src/translation_phases/parsing/recursive_descent/statements.rs)
- [Measurement seam](../../src/benchmarking/parser_study.rs) and
  [standalone executable](../../examples/parser_study.rs)
- [Source-cost audit](parser-frame-audit.md) and
  [grammar corpus](parser-study-coverage.md)

The feature-enabled machine has an extra driver seam and study frame fields.
Its timings are shown alongside a feature-disabled baseline; speed claims above
use the latter. On Lua, enabling the study increased the release machine median
from 35.608 to 36.813 ms. Treat smaller cross-mode differences cautiously.

## Method and reproduction

Host: AMD Ryzen 9 5900X, 24 logical cores, 15.7 GiB physical RAM, Linux 7.0.14,
GCC 15.2.0, Rust 1.99.0 / LLVM 23.1.1. Release uses the repository's fat LTO,
`target-cpu=native`, one codegen unit and static CRT. Debug is opt-level 0 with
debug symbols disabled by the shared wrapper. Sibling builds/reviews remained
active. No CPU isolation or frequency control was imposed.

Wall time covers parser construction and phase 7 only. Source loading,
preprocessing/token collection, optional inspection, output, worker creation,
joining and arena destruction are outside it. Each subprocess creates fresh
arenas and preprocessing state. Global allocation hooks use the same phase
boundary. This extends the repository's `benchmarking-internals` preparation
and VM-accounting infrastructure instead of timing the normal CLI, whose
benchmarking entry point measures preprocessing.

Seven subprocess samples per input/mode rotate their order each round. Reported
central values are medians; minima and population CVs are retained in
[timing.tsv](parser-recursive-descent-data/timing.tsv), with every raw sample in
JSON. This is a custom isolated harness, not a Criterion confidence estimate.
Instruction/cache/branch counts come from one Callgrind simulation per case,
with two additional repetitions of each study mode on Lua. Two attempted
feature-disabled production controls failed in Valgrind as described below.
Hardware cycles are unavailable.

Run all commands from:

```sh
cd /home/agent/workspace/bcc-rust/worktrees/parser-recursive-descent-study
python3 scripts/parser_study_inputs.py --lua
python3 scripts/parser_study_cases.py

# Every Cargo operation is inside the shared wrapper. Each batch copies its
# executable before another checkout can clean the shared target directory.
python3 /home/agent/workspace/bcc-rust/triage-2026-10-16/swarm/run.py sh scripts/parser_study_build.sh baseline
python3 /home/agent/workspace/bcc-rust/triage-2026-10-16/swarm/run.py sh -c '
  sh scripts/parser_study_build.sh native &&
  sh scripts/parser_study_build.sh profile &&
  sh scripts/parser_study_build.sh alignment &&
  sh scripts/parser_study_build.sh checks'

sh scripts/parser_study_collect.sh native
sh scripts/parser_study_collect.sh alignment
sh scripts/parser_study_collect.sh profile
python3 scripts/parser_study_archive.py
cargo +nightly fmt --check
git diff --check
```

Detach long commands on the shared machine, for example:

```sh
setsid nohup sh -c 'sh scripts/parser_study_collect.sh native > .scratch/parser-study/collect-native.log 2>&1; echo EXIT $? >> .scratch/parser-study/collect-native.log' >/dev/null 2>&1 &
```

The scripts contain the exact feature sets, files and options for every main
batch. The default differential batch now also includes the combined-floor
fixture, so rerunning it produces 200 comparisons; the archived 196 comparisons
and separate four floor comparisons cover the same inputs. Supplemental final
checks were:

```sh
python3 scripts/parser_study.py diff --binary .scratch/parser-study/study-debug --inputs .scratch/parser-study/grammar --manifest .scratch/parser-study/grammar/manifest.json --modes machine recursive growing hybrid:16 --output .scratch/parser-study/final-grammar-debug.json
python3 scripts/parser_study.py diff --binary .scratch/parser-study/study-release --modes machine recursive growing hybrid:16 --files c99-floors.c --output .scratch/parser-study/final-floors-diff.json
# Here the runner's label "baseline" denotes the 1 GiB profiling binary.
python3 scripts/parser_study.py diff --binary .scratch/parser-study/study-release --baseline-binary .scratch/parser-study/study-profile --modes machine baseline --files lua-supported.i extensions.c malformed.c parentheses-256.c blocks-256.c --output .scratch/parser-study/final-profile-region-diff.json
python3 scripts/parser_study_profile.py --binary .scratch/parser-study/study-profile --files lua-supported.i --modes machine recursive growing hybrid:16 --repeats 2 --output .scratch/parser-study/final-profile-repeat.json
python3 scripts/parser_study_profile.py --binary .scratch/parser-study/alignment-profile --files alignment-cache.c alignment-nested-1024.c --modes machine --output .scratch/parser-study/final-profile-alignment-verify.json
```

### Inputs

All 17 checked-in `benches/c/*.c` are preprocessed using
`gcc -std=gnu99 -E -P`. Lua v5.4.8 `onelua.c` comes from
`https://codeload.github.com/lua/lua/tar.gz/refs/tags/v5.4.8`, pinned to SHA-256
`d85b70a65f43c5d2254944d58d625e822c8e2e10d9c6a3bd9b5b657e46376a19`.
The untouched preprocessed Lua has eight baseline diagnostics for unsupported
glibc `_Float128` spellings. The clean comparison explicitly adds
`-D_Float128=__float128` during GCC preprocessing, using the supported GNU type
spelling; the untouched input remains in the differential corpus. Clean Lua has
241,784 parser tokens and 2,403 roots. Input bytes, hashes, GCC version and exact
preprocessing commands are in [inputs.json](parser-recursive-descent-data/inputs.json).
SQLite was not available from the attempted official-domain download; no SQLite
result is claimed.

Synthetic inputs cover parenthesis/unary/assignment/conditional nesting,
statements, blocks, declarators and initializers through depth 16,384; operator
chains and initializer lists through 100,000 elements; 4,000-function expression
and statement mixes; 4,000 aggregate/typedef/initializer declaration groups;
and alignment-cache workloads. Depths 16,000/16,500 bracket capacity cliffs.
The grammar corpus has 150 cases: 87 valid, 21 extension and 42 malformed,
including 16 seeded grammar-generated cases. Benchmark configuration is C99,
GNU off, extensions Allow. Source generation is deterministic.

## Native throughput

Median phase-7 milliseconds, seven samples each. All selected large inputs
parse without diagnostics. Complete minima, CVs and all other stress cases are
in the archived timing table.

### Release

| Input | Production | Study machine | Recursive | Growing | Hybrid 16 |
|---|---|---|---|---|---|
| lua-supported.i | 35.608 | 36.813 | 22.765 | 22.888 | 24.841 |
| expression-mix.c | 30.264 | 31.179 | 20.785 | 21.110 | 20.357 |
| statement-mix.c | 40.066 | 40.683 | 22.557 | 23.012 | 22.586 |
| declaration-mix.c | 24.498 | 25.001 | 22.777 | 22.562 | 22.503 |

### Debug

| Input | Production | Study machine | Recursive | Growing | Hybrid 16 |
|---|---|---|---|---|---|
| lua-supported.i | 174.633 | 171.425 | 117.420 | 122.763 | 131.710 |
| expression-mix.c | 177.672 | 174.721 | 128.628 | 132.433 | 133.112 |
| statement-mix.c | 223.485 | 232.641 | 148.065 | 151.017 | 147.355 |
| declaration-mix.c | 133.900 | 139.422 | 128.779 | 130.418 | 131.351 |

Lua minima and timing variability:

| Mode | Release minimum ms | Release CV % | Debug minimum ms | Debug CV % |
|---|---|---|---|---|
| baseline | 34.845 | 1.89 | 164.248 | 5.17 |
| machine | 35.422 | 2.73 | 164.280 | 4.69 |
| recursive | 22.231 | 5.26 | 115.862 | 1.61 |
| growing | 22.383 | 1.41 | 117.839 | 1.92 |
| hybrid:16 | 24.519 | 1.41 | 126.250 | 2.18 |

The largest outlier is the debug declaration/hybrid group (CV 35.45%); its
median is 131.351 ms and minimum 125.376 ms. Do not infer a small speed ordering
between those declaration modes. Release Lua CVs are 1.4-5.3%; the architecture
speed difference is much larger. These are parser-only improvements, not an
end-to-end compiler speedup.

## Instructions, branches, caches and profile findings

`perf stat -e instructions,cycles,branches,branch-misses,cache-misses true`
reports `No supported events found. The instructions:u event is not supported.`
The VM exposes software counters but no hardware PMU. No hardware cycle,
instruction, branch-miss or cache-miss numbers were measured.

Valgrind 3.26 rejects the production 100 GiB arena reservation with `EINVAL`.
Only the profiling binaries enable `parser-study-small-regions` (1 GiB per
region); native timing/RSS/capacity uses 100 GiB. A cross-binary inspection check
matches native and small-region output on Lua, extensions, malformed input and
deep expression/block cases. Both attempts to profile the feature-disabled
production control terminated with SIGILL: Valgrind cannot decode the native
AMD SSE4a `EXTRQ` instruction (`66 0f 78 c0 18 00`). The
[disassembly](parser-recursive-descent-data/sse4a-proof.log) and
[failure log](parser-recursive-descent-data/production-valgrind-failure.log)
are archived; no partial counts from those attempts are used. Consequently
the model comparisons below use the feature-matched study machine and candidates;
the separate production native timings remain valid. The profiling model uses
32 KiB/8-way/64-byte L1 instruction and data caches and a 64 MiB direct-mapped
last-level cache.
These model events must not be presented as physical processor misses. The two
additional release-Lua repetitions have identical event totals within each mode;
compared with the original run, instruction counts remain identical and branch
miss counts differ by at most three events.

The exact phase boundary is a non-inlined function:

```sh
valgrind --tool=callgrind --collect-atstart=no   --toggle-collect='*parser_study_phase7*' --cache-sim=yes --branch-sim=yes   --callgrind-out-file=.scratch/parser-study/lua.callgrind   .scratch/parser-study/study-profile .scratch/parser-study/inputs/lua-supported.i 1 --mode machine
callgrind_annotate --auto=no --inclusive=no .scratch/parser-study/lua.callgrind
```

The profile runner rejects empty collection and saves self/inclusive annotations.
Clean-Lua **simulated** event totals (M = million, K = thousand):

| Build | Mode | Instructions M | Branches M | Branch misses M | L1 I misses K | L1 D misses K | LL misses K |
|---|---|---|---|---|---|---|---|
| release | machine | 316.055 | 35.153 | 2.997 | 1472.8 | 772.4 | 470.1 |
| release | recursive | 175.680 | 20.885 | 1.564 | 1150.7 | 844.0 | 469.9 |
| release | growing | 178.038 | 20.992 | 1.549 | 1183.4 | 857.2 | 470.0 |
| release | hybrid:16 | 202.919 | 23.740 | 1.795 | 1361.7 | 840.2 | 470.3 |
| debug | machine | 1420.112 | 105.478 | 4.725 | 18321.8 | 1431.3 | 444.2 |
| debug | recursive | 960.619 | 71.238 | 2.858 | 11105.2 | 2232.8 | 444.9 |
| debug | growing | 973.944 | 72.274 | 2.961 | 11380.7 | 2250.4 | 444.9 |
| debug | hybrid:16 | 1049.538 | 77.988 | 3.218 | 12839.7 | 2207.3 | 445.2 |

Additional inputs and exact events are in
[events.tsv](parser-recursive-descent-data/events.tsv). Release and debug Lua
self-cost annotations are retained alongside the raw JSON, with out-of-line
call counts in [calls.tsv](parser-recursive-descent-data/calls.tsv). The release
recursive parser executes 44.4% fewer model instructions than the study machine, while
L1 data misses increase 9.3%. Last-level misses are nearly unchanged: AST and
provenance storage still have to be written. This is principally a control and
bookkeeping reduction, not evidence of universally better cache behavior.

On release Lua, out-of-line call counts are:

| Operation | Machine | Recursive |
|---|---:|---:|
| Frame dispatch (`step_once`) | 624,990 | 200,463 |
| Retained-node counting | 1,277,764 | 400,398 |
| Push frame | 122,838 | 44,796 |
| Reclaim pooled frame | 122,838 | 44,796 |
| Expression construction | 106,427 | 106,427 |
| Provenance merge helper | 200,502 | 200,502 |
| Out-of-line vector-pool helpers | 20,964 | 20,964 |

The machine takes 2.585 frame-dispatch calls per token versus 0.829 recursive.
Selected self-cost slices are 8.84% in the main driver source, 8.79% in expression
stepping, 6.16% in `step_once`, 4.76% in retained-node counting and 1.34% in
pooled reclamation. Bump allocation is 1.97% and the out-of-line expression
constructor 1.18%. LLVM attributes inlined work to several source files, so
these slices are not a complete additive cost partition. Driver-side moves,
checks, cursor operations and pool lending also occur in inlined slices.

The profile supports reducing grammar scheduling rather than replacing arenas
or pools. Pooling already makes scratch bounded by concurrency/maximum list
size, and both candidates perform the same expression/provenance work. Cursor
lookahead and boundary classification remain costs; the source audit identifies
redundant checks/type-prefix classification as further unmeasured candidates.

## Memory and stack use

Release clean-Lua memory, seven-sample median RSS. Arena numbers are deterministic:

| Mode | Scratch bytes | TU growth bytes | Additional peak arena commit bytes | Peak RSS MiB | Global calls / bytes |
|---|---|---|---|---|---|
| baseline | 616,016 | 15,989,008 | 18,931,712 | 33.59 | 0 / 0 |
| machine | 616,016 | 15,989,008 | 18,931,712 | 33.62 | 0 / 0 |
| recursive | 593,552 | 15,989,008 | 18,915,328 | 33.64 | 0 / 0 |
| growing | 593,552 | 15,989,008 | 18,915,328 | 33.68 | 0 / 0 |
| hybrid:16 | 603,856 | 15,989,008 | 18,915,328 | 33.62 | 0 / 0 |

All these variants make zero global allocator calls/bytes during parsing. Arena
and stack mappings are OS allocations outside that counter. Recursive scratch
saves only 22,464 bytes on Lua, about 0.14% of scratch plus TU growth, before
including its native stack. AST/TU growth is identical.

RSS uses `/proc/self/status` `VmHWM`, outside the timer and allocation interval.
Linux `getrusage` can inherit the Python parent's peak across fork/exec. A parent
holding 128 MiB caused the old harness to report 146.5 MB for a tiny parse; the
corrected executable reports about 3.1 MB. RSS includes preparation and process
startup; subtracting before/after high-water marks is not a parser-only RSS peak.
Arena commit excludes native/stacker stacks. No peak Windows commit was measured.

### Native stack slopes and exhaustion

Address sampling is disabled for timing. The following are measured stack-marker
slopes, lower bounds that exclude transient helper peaks. Expression/statement
slopes use depths 32/127; block slopes use 256/1024 on a 64 MiB worker. Declarator
and initializer nesting remains iterative in this prototype.

| Nesting family | Release bytes / level | Debug bytes / level |
|---|---|---|
| parentheses | 704 | 1,952 |
| unary | 704 | 1,952 |
| assignment | 704 | 1,008 |
| conditional | 704 | 1,008 |
| statements | 496 | 3,776 |
| blocks | 1,008 | 8,176 |

On a 1 MiB worker, release raw recursion passes 1,024 parentheses but fails at
4,096; it passes 256 blocks but fails at 1,024. Debug passes 256 parentheses
but fails at 1,024, and fails at exactly 127 total blocks (`blocks-126.c`).
The explicit machine, growing and hybrid complete every 48-case stress grid
through depth 16,384 in both builds. These are tested depths, not a proof of
unlimited allocation capacity.

C99 5.2.4.1's 63 parenthesized expressions/declarators and 127 nested blocks
are capability floors, not recommended rejection limits. A generic recursion
counter differs from grammar depth: a block enters statement and compound calls.
`c99-floors.c` combines 127 total blocks and 63 parentheses:

| Mode, 1 MiB worker | Release combined C99 floors | Debug combined C99 floors |
|---|---|---|
| machine | accepted | accepted |
| recursive | accepted | stack overflow |
| growing | accepted | accepted |
| hybrid:16 | accepted | accepted |
| limited:64 | resource diagnostic | resource diagnostic |
| limited:256 | resource diagnostic | stack overflow |
| limited:512 | accepted | stack overflow |

A limit of 64 even rejects the isolated 63-parenthesis case (66 grammar entries).
A limit of 256 accepts the isolated release block floor but rejects their combined
case; 512 permits it in release and still overflows in debug. These tests rule
out using a small generic counter alone while honoring the stated C99 floors.
The terminal prototype reuses the production resource diagnostic but does not
prove equivalent semantics for every frame/node/provenance limit.

### Stack-exhaustion options measured

- **`stacker::maybe_grow`:** 64 KiB red zone, 1 MiB segments, at recursive grammar
  entry. Lockfile resolves stacker 0.1.25 and psm 0.1.32. On release Lua, model
  instructions increase 1.34% over raw recursion; native median increases 0.54%,
  within timing variation. Debug native Lua increases 4.55%. It passes the 1 MiB
  stress grids but retains large live stacks during extreme debug nesting.
- **64 MiB worker:** seven Lua samples record median launch latency 112.3 us in
  release and 139.9 us in debug; preparation/parsing run inside the worker, and
  launch/join are excluded from parser time. Release passes all tested worker
  depths. Debug still overflows at 16,384 blocks (4,096 passes). A fixed large
  stack buys headroom, not the existing arbitrary-nesting guarantee. The borrowed
  arena/AST API would also need a deliberate worker ownership boundary.
- **Depth limits:** tables above show both successful terminal diagnostics and
  failure to meet combined floors or to prevent debug overflow. A separately
  budgeted grammar policy could be designed, but is not established by these
  generic limits and would reduce the current accepted nesting range.
- **Hybrid depth 16:** recurses shallowly then suspends recursion for the entire
  selected subtree. Precedence-bounded expression frames retain operator
  ownership. It passes both stress grids and C99 probes; sampled stack span in
  the combined-floor debug case is 61,424 bytes, versus growing's per-segment
  span of 956,336 bytes (934 KiB). Its Lua median costs 9.12% over raw recursion
  in release, while retaining a 30.2% advantage over production. The model
  instruction cost is 15.50% above raw recursion, reflecting 2,390 Lua fallback entries.

Growth-heavy parsing on a 1 MiB worker, **without** stack sampling, seven rotating
samples. Values are median parser ms / process peak RSS MiB:

| Build | Input | Machine ms / RSS MiB | Growing ms / RSS MiB | Hybrid ms / RSS MiB |
|---|---|---|---|---|
| release | parentheses-16384.c | 8.738 / 10.62 | 11.374 / 16.47 | 7.598 / 10.13 |
| release | blocks-16384.c | 18.372 / 25.14 | 17.485 / 21.52 | 13.773 / 20.17 |
| debug | parentheses-16384.c | 27.372 / 15.79 | 36.798 / 40.31 | 25.026 / 15.18 |
| debug | blocks-16384.c | 37.798 / 30.21 | 122.486 / 139.16 | 37.527 / 25.58 |

The 16,384-point hybrid memory saving is partly a capacity cliff. At 16,000 blocks
machine/hybrid scratch is 14,120,344/14,120,152 bytes; at 16,500 it is
26,751,256/26,751,064. At 16,384 it is 26,740,120/16,778,456: removing shallow
frames narrowly avoids a vector doubling. Parentheses show the same effect.
Do not generalize the isolated 10 MB hybrid saving into a per-level advantage.

## AST identity, recovery and diagnostics

The structural check includes node order/labels, locations, ordered source
segments, expression operator/delimiter provenance, recovered markers and rendered
diagnostics. It excludes addresses/arena handles and is not a proof about every
unrendered private field. Successful output hashes are retained; differing full
inspections remain in scratch. Collection commands can exit zero while recording
mismatches, so inspect their JSON and printed summaries.

- Broad release corpus: **196/196** runs identical (17 C benchmarks, both Lua
  variants, provenance/extensions/precedence/recovery fixtures, stress through
  depth 256 and width 1,000).
- Large expression/statement/declaration mixtures and 10K initializers:
  **16/16** identical; combined C99-floor corpus: **4/4** identical.
- Grammar corpus in release and debug: all **108 valid/extension cases** match
  in recursive/growing/hybrid modes. Of 42 malformed cases, **32 match and 10
  differ** in each candidate mode. None crashes in these grammar comparisons.

The ten remaining cases are `bad-stray-operand-group`, `bad-mismatched-close`,
`bad-missing-call-comma`, `bad-if-empty-condition`, `bad-if-stray-operand`,
`bad-if-missing-body`, `bad-while-header-brace`, `bad-do-missing-while`,
`bad-for-extra-clause` and `bad-switch-condition` (all `.c`). Examples include
representing an empty condition as an error expression instead of a missing slot,
shortening the source range after a stray operand/missing comma, and different
recovery marking/continuation after a broken control header. Recovery after a
consumed recursive prefix cannot simply rewind into the frame parser.

The default parser's recovery, arena/construction API and representation-bound
limits remain the shipping contract. A production hybrid needs full parity for
these cases, mixed recursive/iterative cleanup and resource limits, extension
policies and typedef restoration; the current study is deliberately opt-in.
Maintaining both grammar implementations adds ongoing differential-testing cost.
No native Windows build was measured: platform stacker support, ABI frame sizes
and Windows' 1 MiB main stack need direct validation before adoption.

## Cheap frame-machine optimization

`parser-study-leak-alignment` changes only retention of the completed bool table
from `arena.alloc_slice_copy(&matches)` to `matches.leak()`. It removes a redundant
N-byte copy and allows the temporary delimiter stack's last arena allocation to
be reclaimed. Parse/commit allocation ordering explains savings beyond N bytes.
The experiment stays off by default.

| Input | Original scratch bytes | Reused-table scratch bytes | Saved bytes | Median ms original / reuse |
|---|---|---|---|---|
| alignment-cache.c | 504,160 | 254,048 | 250,112 | 64.896 / 63.650 |
| alignment-nested-16.c | 81,480 | 78,120 | 3,360 | 0.148 / 0.148 |
| alignment-nested-128.c | 656,040 | 629,272 | 26,768 | 0.830 / 0.799 |
| alignment-nested-1024.c | 5,252,520 | 5,038,488 | 214,032 | 25.920 / 19.968 |
| lua-supported.i | 616,016 | 373,472 | 242,544 | 46.205 / 44.706 |
| declaration-mix.c | 5,680 | 5,680 | 0 | 32.958 / 37.351 |

The ordinary declaration control has identical scratch. Whole-process memory
savings are much smaller than scratch percentages: Lua RSS falls about 0.15 MiB.
Timing CVs in this later shared-load batch are 10-40%, and the no-alignment
control also shifts. The medians above are observations, not proof of a speedup.
The model instruction count falls from 433,669,573 to 433,419,461 on the
50K-declaration case (250,112 instructions, 0.058%), and from 191,337,613 to
191,323,402 at depth 1,024 (14,211 instructions, 0.007%). A repeat of the reuse
measurements reproduces those totals. This supports a small copy/bookkeeping
reduction, not the large speedups suggested by the noisy medians.

Full syntax/provenance checks pass on the 50K-declaration case, alignment depths
16/128 and Lua. At depth 1,024, both unchanged-machine and reuse variants were
OS-killed during untimed full provenance rendering after successful parse-only
measurements. This is an inspection-memory limitation, not a successful deep
identity check or a parser-phase crash. The failed rows remain archived. The
reproduction batch retains timing/allocation at 1,024 but omits that expensive
inspection. Do not re-run it on a memory-constrained shared host.

Other audit candidates—cursor checks, duplicate type-prefix classification,
lazy designation-state borrowing and frame/action layout—remain unmeasured.
Do not assign them predicted speedups or remove recovery/resource checks to
make the machine faster.

## Validation and remaining owner decisions

The final wrapped build/check bundle completed successfully (`EXIT 0`):

- `cargo test --all-targets`: 2,304 library tests, three additional integration
  tests and 254 main integration tests passed; nine tests were ignored.
- Allocation-count integration tests: 31 passed.
- Focused recursive-parser tests: six passed.
- Strict Clippy passed for default, `benchmarking-internals`, the study feature,
  and all study/alignment/profiling features together.
- Nightly rustfmt, Python syntax, shell syntax and `git diff --check` passed.

[validation.json](parser-recursive-descent-data/validation.json) records the
bundle revision, log hash and exact test result lines. Expected raw-recursion
stack-overflow subprocesses are retained in the stack tables. The unresolved
results are the ten malformed recovery cases, two deep inspection OS kills,
and two unavailable production Valgrind controls; none is reported as a pass.
Hardware cycles and native Windows behavior remain unmeasured.

The architectural choice is whether a roughly 30% real-input parser speed gain
justifies maintaining recursive and iterative grammar paths and closing the
recovery gaps. If yes, depth-16 hybrid is the best follow-up; retain the machine
for fallback and Windows stack insurance. If that maintenance cost is unwanted,
keep the machine and pursue isolated optimizations from the audit. The small
alignment-table change is independently reviewable. Choose a native Windows CI
probe and a recovery/resource-limit acceptance corpus before making a default
parser change. No push, PR or pipeline switch was performed in this study.
