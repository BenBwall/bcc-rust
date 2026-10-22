# Recursive lowering on a compilation worker

2026-10-22. Follow-up to [the parser study](parser-recovery-windows.md).
The opt-in implementation is complete. Linux measurements and native Windows
debug validation are complete; native release validation is running.
The feature-disabled compiler retains its existing pipeline.

## Implementation boundary

`reserved-compilation-stack` runs the complete command-line invocation on one
100 GiB demand-paged worker: argument handling, input preparation, preprocessing,
parsing, semantic analysis, lowering, optimization, backend work and destruction.
The public `with_compilation_stack` callback is the corresponding library seam.
Arenas and context are created inside it; no `Send` implementation is added to
arena-backed compiler types. The tested native worker is shared by compiler and
parser probes rather than copied. Linux and Windows retain their original OS
implementations. Reservations remain finite and exhaustion remains fatal.

`recursive-lowering-study` opts into recursive lowering and the compilation
worker. Common expressions return an `Item` through native calls and reuse the
same conversion/operator builders. Compound statements iterate over siblings and
recurse into children. Other statements, calls, short-circuit expressions,
builtins, VLA actions and initialization reuse the existing task descriptions,
with recursive draining bounded to the current construct's pending-work floor.
The prototype retains their task and operand buffers; it does not replace every
continuation with a typed Rust function. Retained AST, type and IR storage stays
in arenas. Semantic analysis and optimizer worklists remain unchanged. The CLI
still uses the existing frame-machine parser; the recursive parser remains a
separate study feature and probe.

Ordinary library pipeline entry points retain task lowering even with the study
feature enabled; only the reserved-worker CLI and explicit study options select
recursion. The same helper supports task/recursive differential checks in
the same feature-enabled binary. Lowering tests compare complete printed IR,
function sources, retained call diagnostics and refusal diagnostics. Measurements
prepare the frontend outside the lowering timer and verify/inspect afterward.
Stack sampling runs separately from timing. Whole-worker wall time is also
reported, including preprocessing, analysis, setup/join and destruction.

## Validation method

- Existing lowering tests, including all target/extension/cleanup cases that
  use the shared test helper, compare task and recursive results.
- The standalone `lowering_study` example accepts source, mode and repetitions.
  Its opt-in native runner test rotates nine samples of each mode for two
  4,000-function mixes and the repository's 17 C benchmark sources. Unsupported
  or diagnosed inputs are retained as parity evidence and excluded from speed
  claims.
- Deep nonconstant expression chains and nested blocks at 256/4,096 levels
  measure native-address spans in separate runs.
- Check rustfmt, strict Clippy, default checks and native Windows execution.


## Reproduction

Run from `/home/agent/workspace/bcc-rust/worktrees/parser-recursive-descent-study`.
Every local Cargo invocation below is inside the shared two-slot wrapper. The
build script copies each executable before releasing its slot. Detach long
wrapper invocations and retain their complete logs.

```sh
python3 /home/agent/workspace/bcc-rust/triage-2026-10-16/swarm/run.py sh scripts/lowering_study_build.sh build
python3 /home/agent/workspace/bcc-rust/triage-2026-10-16/swarm/run.py sh scripts/lowering_study_build.sh checks
cargo +nightly fmt --check
python3 scripts/lowering_study_results.py .scratch/lowering-study/build.log --output .scratch/lowering-study/linux-summary.json
python3 scripts/lowering_study_width.py --output .scratch/lowering-study/width --statements 50000 --repeats 9

# Native GNU Windows source cohort, sequential self-hosted Actions jobs.
python3 scripts/parser_study_native.py --study lowering --sha 932f32a89c75682029e7f90b208b74960fc71e94 --output .scratch/lowering-study/windows

# Explicit file/mode runs, each using a fresh compilation worker.
.scratch/lowering-study/study-release benches/c/bits.c recursive 9 --inspect
```

The last command selects one real benchmark file; the measured
input list comes from that exact source tree, sorted by path. The opt-in runner
retains diagnosed/unsupported cases as differential evidence and excludes them
from timing comparisons. All timed lowering intervals assert zero global
allocation calls/bytes using thread-local allocator hooks.

The first lowering suite recorded 205 passes and one failure in an existing
range-initializer test: its thread-local copy-policy override did not follow the
compilation onto the new worker. The helper now explicitly carries the override
and replay/effect counters, and excludes the extra differential passes from
those counters. The failure was in test setup, not an IR mismatch. The corrected Linux debug
suite passes 206 tests (one existing ignored test); strict combined-feature
Clippy and the five-case focused example test also pass. Initial compile/Clippy failures are archived alongside the successful checks.

The complete Linux check script passes: default all-target tests, all 31
allocation tests, strict default/benchmark/study Clippy, the feature-enabled
full library suite, the parser differential example and shared worker tests.
Rustfmt, Python byte-compilation, shell syntax, archive decoding, research
Markdown links and `git diff --check` also pass. Full test counts and commands
are retained in `recursive-lowering-data/linux-checks.log.gz` and the build
script, rather than inferred from filtered harness counts.

The shared worker's `CreateThread` binding requires `Win32_Security`. The
compilation-stack feature now explicitly enables that dependency feature for
normal Windows builds, rather than relying on the test-only dependency to
provide it. Native source cohort `932f32a8` already receives it from its test
configuration; this later declaration does not change its compiled test code.

## Linux measurements

Rust source cohort `932f32a8`, Rust 1.99.0 / LLVM 23.1.1 on the same shared
Ryzen 9 5900X Linux host as the parser study. The feature-matched task and
recursive modes use identical IR builders, frontend and reservation policy.
These are nine rotating samples in one test process, with a fresh 1 GiB worker
and fresh arenas per sample. There is no CPU isolation; shared builds ran during
collection. CV is population standard deviation divided by mean.

| Profile/input | Task median ms | Recursive median ms | Task range ms / CV % | Recursive range ms / CV % |
| --- | ---: | ---: | --- | --- |
| release / expressions | 69.216 | 64.481 | 63.769–77.326 / 6.47 | 60.184–68.680 / 4.17 |
| release / statements | 101.422 | 96.807 | 91.856–109.022 / 6.29 | 91.341–104.946 / 4.34 |
| debug / expressions | 626.830 | 700.345 | 491.809–707.785 / 13.14 | 507.813–747.149 / 13.66 |
| debug / statements | 1007.462 | 939.039 | 824.045–1334.743 / 15.63 | 841.685–1141.212 / 11.14 |

Release lowering uses 6.8% less time for expressions and 4.6% less for statements.
Debug expressions use 11.7% more time; debug statements use 6.8% less. Whole-worker
median reductions are 3.9% / 2.9% release; debug expressions take 3.8% more and
statements 6.3% less. Those deltas sit amid substantial variation. These results
do not establish a general speed advantage, especially in debug.

The 17 real C files are small: release task medians range from 0.093 to
0.287 ms and recursive medians from 0.095 to 0.256 ms. Individual files move
in both directions, with population CVs up to 50.6%. Their full per-input
medians/ranges/CVs are archived; these short intervals do not support a broad
throughput claim. The larger synthetic mixes above amortize more fixed costs.

Each profile completes all 342 timing samples, 19 parity inputs and four deep
probes. Every input is clean and supported; none was excluded. Printed IR,
function sources, call diagnostics and refusal output match. Every measured
lowering interval has zero global allocation calls/bytes.

| Input | Task scratch B, release / debug | Recursive scratch B, release / debug | IR arena B (both) |
| --- | ---: | ---: | ---: |
| Expressions | 14,639 / 15,872 | 13,359 / 14,592 | 18,264,688 |
| Statements | 27,904 / 29,896 | 26,688 / 28,680 | 28,376,672 |

The scratch high-water mark includes spare capacity/alignment and survives
per-function resets. The many-function mixes save only 1,216–1,280 scratch bytes;
retained IR size is identical.

| Native stack shape | Release B/level | Debug B/level | Release span at 4,096 | Debug span at 4,096 |
| --- | ---: | ---: | ---: | ---: |
| Nonconstant `a+a+...` | 672 | 8,016 | 2,753,513 B | 32,836,112 B |
| Nested compounds | 464 | 1,024 | 1,901,472 B | 4,196,880 B |

Slopes use entry-address differences between 256 and 4,096 source levels, with
separate instrumented runs. They are not exact native peak measurements. Debug
expression traversal is stack-heavy even though nested compounds are small.

### Wide function

A separate fresh-process cohort rotates nine samples per mode/profile for one
function containing 50,000 `a+=1;` statements. Both modes use the same 100 GiB
worker; `/usr/bin/time` measures process peak RSS. Input and binary SHA-256 hashes
are archived. The runner creates the source in scratch, so no huge fixture is
committed. Compilation preparation/destruction is outside lowering time.

| Profile/mode | Lowering median ms | Range ms | CV % | Scratch B | Peak RSS median KiB |
| --- | ---: | --- | ---: | ---: | ---: |
| release / machine | 84.104 | 75.095–92.336 | 6.93 | 25,018,238 | 219,548 |
| release / recursive | 77.500 | 70.590–101.692 | 11.57 | 20,824,190 | 219,396 |
| debug / machine | 285.561 | 250.721–350.327 | 11.26 | 27,318,377 | 226,688 |
| debug / recursive | 300.789 | 267.107–478.510 | 22.21 | 23,124,329 | 226,580 |

Recursion removes 4,194,048 requested scratch bytes by iterating compound siblings
instead of retaining them as pending tasks. IR requests remain 11,536,992 B.
Peak process RSS is essentially unchanged: it includes the larger frontend
working set. Release lowering is 7.9% lower; debug is 5.3% higher with 22.2% CV
for recursion. This is a scratch-storage improvement, not a measured process
memory win.


## Native Windows measurements

The self-hosted `bcc-win-x64` runner executes native
`x86_64-pc-windows-gnu`, Rust 1.99.0 / LLVM 23.1.1, Windows build 26300.
These timings compare modes on the Windows host and should not be compared
across machines to the Linux timings. The test suite and harness use the same
source cohort `932f32a89c75682029e7f90b208b74960fc71e94`, exact commands,
rotating samples and 1 GiB workers described above.

| Profile/input | Task median ms | Recursive median ms | Task range ms / CV % | Recursive range ms / CV % |
| --- | ---: | ---: | --- | --- |
| debug / expressions | 557.419 | 583.747 | 544.702–600.283 / 2.81 | 561.287–610.277 / 2.43 |
| debug / statements | 934.042 | 942.666 | 881.439–955.794 / 2.67 | 895.208–986.167 / 2.62 |

Debug recursion uses 4.7% more lowering time for expressions and 0.9% more for
statements. The 206 lowering tests pass (one existing ignored), as do all 19
corpus parity inputs, 342 timing samples and four deep probes. No measured
lowering interval allocates globally. Native release jobs remain in progress.

Windows debug address-span slopes are 12,592 B/expression level and
1,296 B/compound level; at 4,096 levels, the spans are 51,580,201 B and
5,311,785 B respectively. These exceed the ordinary Windows 1 MiB stack and
complete on the reserved worker. This is native execution, not cross-compilation
or Wine. MSVC, other operating systems and Windows physical working-set costs
are outside this validation. As with Linux, these are address samples rather than exact stack peak
or physical working-set measurements. The Windows stack implementation and its
lazy-commit/100 GiB probes are also covered by the prior
[reserved-stack Windows study](parser-recovery-windows.md).

## Whole-compilation CLI checks

The feature-only debug CLI build (without `benchmarking-internals`) successfully
emits IR and LLVM, interprets a compound assignment with status 42, interprets
an optimized loop with status 45, and emits IR for 4,096 nested blocks and a
4,096-operator nonconstant expression. Each command has empty stderr. The CLI
binary hash, exact arguments, exit statuses and output hashes are archived in
`recursive-lowering-data/linux-cli.json.gz`. These exercise the actual command
entry and backend/interpreter paths on the compilation worker, beyond the
lowering-only harness.

## Recommendation

Keep both features opt-in. The shared worker implements the intended ownership
boundary now: a synchronous caller submits a whole compilation, the worker owns
its arenas/context until completion, and only a `Send` result crosses back.
Library embedders choose their reservation size through `with_compilation_stack`;
the CLI currently uses a fixed 100 GiB reservation. An OS reservation/thread
creation error is returned, while exhausting a running stack remains fatal.
No overflow handler, moving stack, automatic retry or parser fallback is added.

The recursive lowering slice is a useful experiment, but these results do not
justify switching its default. Its release benefit is modest and debug can
regress. Removing the compound sibling task list provides a reproducible scratch
saving on wide functions; consider bringing that traversal improvement to the
explicit lowerer separately. That optimization has not been independently
implemented or timed here. A full recursive rewrite should first replace the
remaining task descriptions with typed return values, then repeat these same
parity checks and debug measurements. Semantic analysis and optimizer worklists
need their own profiles before any rewrite.

Native address span, arena requests, OS commit and process RSS describe different
resources. The worker makes a large virtual reservation cheap; touched pages,
page faults and finite exhaustion still matter. This study does not establish
whole-compiler speedups, and the earlier parser's speedups do not transfer to
lowering. Hardware performance counters were unavailable on this host in the
parser study; no instruction/cycle/cache claim is made for this follow-up.
