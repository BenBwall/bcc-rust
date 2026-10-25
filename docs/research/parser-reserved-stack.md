# Reserved native-stack follow-up

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

2026-10-22. Follow-up to [the recursive descent study](parser-recursive-descent.md).
Native stack code is `462812f9`; the build batch completed at `916047b8`
(subsequent source changes concern collection/analysis only). Per-run binary
hashes and options are retained in [the data directory](parser-reserved-stack-data/).
The default compiler/parser path is unchanged.

## Result and recommendation

**The large, demand-paged stack works and retains recursive descent throughput.**
With a 100 GiB usable reservation, Lua's parser takes 22.727 ms
release and 119.411 ms debug, 39.0% and 28.3% below the
feature-disabled production baseline in this batch. Its stack mapping uses
44 KiB resident after release Lua parsing and 168 KiB after debug Lua parsing.
Both builds pass all 56 nesting probes through 65,536 levels with no growth
checks or segment switches in recursive grammar calls.

This makes a reserved native worker a credible alternative to depth-triggered
hybrid fallback. Prefer this follow-up if keeping the recursive path simple is
worth the memory cost on extreme input. Keep the hybrid as the stronger option
for bounded native-stack use: 65,536 nested blocks need about 511 MiB of native
stack pages in debug, and the reserved variant is slower than the machine on
that workload. This is a speed/architecture option, not a general memory win.

Do not switch the production default yet. The ten existing malformed-recovery
mismatches remain, and the Windows implementation has not been compiled or run
here. A reserved stack removes depth-triggered fallback; retiring existing
expression/statement frames would still require extension/recovery parity.

## Implementation

The standalone executable adds `--reserve-stack BYTES`. It runs preparation,
parsing and optional inspection on a fresh native worker, then joins it.
Linux reserves the requested usable range plus two 64 KiB inaccessible guards
using `mmap(MAP_NORESERVE | MAP_STACK)`. `mprotect` makes the usable interval
writable without touching it. `pthread_attr_setstack` registers that interval
with pthreads; the kernel backs ordinary pages as recursion touches them.
The mapping never moves and is unmapped after successful join. A parent-owned
packet transfers the callback/result; panics are caught inside the native entry
point and resumed after joining. A failed join aborts rather than invalidating
a packet/stack that might still be in use.

The callback and result must be `Send`; preparation and arenas are created on
the worker. Existing non-`Send` arena/context values cannot simply be moved
across this boundary. Production integration would need to keep the relevant
compilation work inside the worker, or use a separately validated same-thread
stack-switching mechanism. This benchmark exercises the first arrangement.

The code attempts `MADV_NOHUGEPAGE`, but this host rejects THP advice with
`EINVAL`; it is best-effort. Actual anonymous huge-page backing is zero in all
recorded stack-memory samples. No custom signal handler handles page growth or
overflow. The supplied stack is an ordinary fixed native address range.

Windows has a counterpart using `CreateThread` with
`STACK_SIZE_PARAM_IS_A_RESERVATION`. The OS manages commit behind its moving
guard. `VirtualQuery` identifies the reservation and committed/protected regions.
That source path remains unvalidated here: only Linux/wasm Rust targets are
installed, and no Windows execution environment was used. Windows physical stack
residency is deliberately reported as unavailable (zero), not inferred from
commit. Linux measurements are not Windows measurements.

This differs from the arena's explicit commit frontier: Linux makes the usable
stack writable up front with `MAP_NORESERVE`. The host uses overcommit mode 0
and has no swap. Under strict overcommit policies no-reserve behavior differs;
reservation/setup can fail, touching pages can exhaust physical memory, and
crossing the bottom guard remains fatal. These are not parser resource diagnostics.
The benchmark returns setup errors, contains worker panics and leaves ordinary
parser limits unchanged; it does not establish graceful recovery from OS OOM.

- [Runner and allocation interval](../../examples/parser_study.rs)
- [Native worker protocol](../../src/compilation_stack.rs)
- [Linux mapping/guards](../../src/compilation_stack/linux.rs)
  and [Windows reservation](../../src/compilation_stack/windows.rs)
- [Untimed OS telemetry](../../src/compilation_stack/telemetry.rs)

## Method and reproduction

Same host/toolchain as the original study: Ryzen 9 5900X, 24 logical cores,
15.7 GiB RAM, Linux 7.0.14, Rust 1.99.0 / LLVM 23.1.1, GCC 15.2.0. Release
uses fat LTO, one codegen unit and `target-cpu=native`; debug is opt-level 0
with symbols disabled by the shared wrapper. The canonical check bundle ran
concurrently for part of collection. There is no CPU isolation/frequency control.

Run from `/home/agent/workspace/bcc-rust/worktrees/parser-recursive-descent-study`:

```sh
python3 scripts/parser_study_inputs.py --lua
python3 scripts/parser_study_cases.py
python3 /home/agent/workspace/bcc-rust/triage-2026-10-16/swarm/run.py sh scripts/parser_study_build.sh reserved
python3 scripts/parser_study_reserved.py prepare
python3 scripts/parser_study_reserved.py collect --profile release
python3 scripts/parser_study_reserved.py archive
python3 scripts/parser_study_reserved.py collect --profile debug
python3 scripts/parser_study_reserved.py archive
python3 /home/agent/workspace/bcc-rust/triage-2026-10-16/swarm/run.py sh scripts/parser_study_build.sh checks
cargo +nightly fmt --check
git diff --check
```

Detach long batches as in the original report. Every Cargo operation except
nightly fmt must stay inside the shared wrapper. The build batch copies study
and feature-disabled release/debug binaries to `.scratch/parser-study/reserved-*`
before releasing its slot; the collector never invokes Cargo.

The scripts contain exact files and options. `reserved:MODE` selects the native
reservation, defaulting to 100 GiB; `--reserve-stack BYTES` changes its size.
`--stack BYTES` applies only to ordinary, non-reserved labels. The 17 benchmark
C files and pinned preprocessed Lua are unchanged from the original report.
The eight extra depth-65,536 fixtures are deterministic; hashes are archived.
AST inspection is limited to small/moderate depths to avoid the previously
observed inspection-memory failure on huge, deeply indented provenance output.

Nine subprocess samples per shallow input/mode and seven per deep input/mode
rotate mode order. Preprocessing/token preparation, source loading, setup,
join, destruction and OS snapshots are outside the parser timer/allocation
counter. Memory-only batches take three fresh-process samples and enable
`--stack-memory`; timing batches leave smaps inspection disabled. Stack-address
sampling is enabled only for the separate nesting grid.

Opt-in `/proc/self/smaps` snapshots identify this stack's resident, swapped and
writable bytes, its `nr` flag and anonymous huge pages before/after parsing.
They include worker runtime frames and snapshot helper calls, so they are not
an exact parser-only stack peak. Pages remain mapped after unwinding, making
post-parse residency useful here. Per-thread minor/major fault deltas bracket
phase 7; RSS is whole-process `VmHWM`, including preparation, not a subtraction
of high-water marks. Arena commit excludes the native stack. Hardware PMU events
remain unavailable; no new instruction/cycle/cache numbers are claimed.

## Native throughput

Lua phase-7 time, nine samples. CV is population standard deviation / mean.
`baseline` disables the recursive study feature; `machine` uses the feature-
matched driver. Reserved machine is a control for worker placement.

| Mode | Release median ms | Minimum ms | CV % | Debug median ms | Minimum ms | CV % |
|---|---|---|---|---|---|---|
| baseline | 37.266 | 35.758 | 3.09 | 166.512 | 156.715 | 11.58 |
| machine | 37.659 | 35.595 | 3.43 | 168.707 | 160.741 | 7.80 |
| recursive | 23.338 | 21.963 | 4.40 | 116.729 | 114.577 | 3.00 |
| growing | 23.025 | 22.564 | 5.65 | 121.860 | 115.427 | 2.91 |
| hybrid:16 | 25.659 | 24.733 | 3.97 | 129.304 | 124.014 | 4.85 |
| reserved:machine | 37.622 | 36.155 | 3.90 | 163.451 | 159.581 | 5.37 |
| reserved:recursive | 22.727 | 22.321 | 4.30 | 119.411 | 112.787 | 8.20 |

Reserved/raw medians differ by about -2.6% release and +2.3% debug, within timing
variation. These data establish no reliable speed difference between them, nor
a small ordering against stacker. Reserved worker launch medians are
89.7 us release and 97.0 us debug on Lua; join/cleanup is outside
parser time and was not independently timed. This setup cost matters more for
tiny translation units than for Lua. End-to-end subprocess durations are retained
but also include preparation and executable loading.

Other large input medians:

| Build / input | Baseline ms | Raw RD ms | Growing ms | Hybrid 16 ms | Reserved RD ms |
|---|---|---|---|---|---|
| release / expression-mix.c | 30.573 | 20.153 | 20.611 | 20.791 | 20.374 |
| release / statement-mix.c | 40.831 | 23.087 | 22.821 | 22.662 | 22.646 |
| release / declaration-mix.c | 28.124 | 23.025 | 23.991 | 23.153 | 22.865 |
| debug / expression-mix.c | 169.047 | 124.194 | 130.430 | 125.608 | 126.615 |
| debug / statement-mix.c | 224.186 | 147.188 | 151.374 | 147.926 | 148.209 |
| debug / declaration-mix.c | 137.671 | 126.764 | 131.362 | 128.791 | 126.829 |

Minima, variability, RSS, allocations and faults for every group are in
[summary.tsv](parser-reserved-stack-data/summary.tsv); raw samples remain in JSON.
Do not interpret small declaration-mode differences as stable speedups.

## Physical memory and deep nesting

All successful measurements make zero global allocator calls/bytes during phase 7.
The worker stack uses OS mappings, outside that counter. Lua arena growth stays
identical across algorithms; recursive scratch saves only 22,464 bytes.

| Build / mode | Peak RSS MiB | Scratch bytes | TU growth bytes |
|---|---|---|---|
| release / baseline | 33.61 | 616,016 | 15,989,008 |
| release / recursive | 33.62 | 593,552 | 15,989,008 |
| release / growing | 33.63 | 593,552 | 15,989,008 |
| release / hybrid:16 | 33.63 | 603,856 | 15,989,008 |
| release / reserved:recursive | 33.48 | 593,552 | 15,989,008 |
| debug / baseline | 38.80 | 616,016 | 15,989,008 |
| debug / recursive | 38.83 | 593,552 | 15,989,008 |
| debug / growing | 38.97 | 593,552 | 15,989,008 |
| debug / hybrid:16 | 38.96 | 603,856 | 15,989,008 |
| debug / reserved:recursive | 38.61 | 593,552 | 15,989,008 |

Physical pages in the **reserved recursive stack mapping**, three-sample medians:

| Input | Release before / after KiB | Debug before / after KiB |
|---|---|---|
| lua-supported.i | 20 / 44 | 48 / 168 |
| parentheses-16384.c | 16 / 11,284 | 48 / 31,272 |
| blocks-16384.c | 16 / 16,148 | 48 / 130,860 |
| blocks-65536.c | 16 / 64,532 | 48 / 523,308 |

Every sampled usable mapping is exactly 100 GiB writable, with `nr` set, zero
swap and zero anonymous huge-page bytes. A huge writable virtual range is not
resident memory. Linux stack-marker slopes from depth 4,096 to 65,536 remain:
parentheses 704 bytes/level release and 1,952 debug; statements 496/3,776;
blocks 1,008/8,176. These are lower bounds, excluding transient helper peaks.
Declarations/declarators and initializers still use the existing iterative frames.

Deep parser medians / whole-process peak RSS, seven samples. Machine, growing
and hybrid start on a 1 MiB standard worker; reserved RD uses its 100 GiB mapping.
These are workload-specific results, not a universal ordering:

| Build / input | Machine ms / RSS MiB | Growing ms / RSS MiB | Hybrid ms / RSS MiB | Reserved RD ms / RSS MiB |
|---|---|---|---|---|
| release / parentheses-65536.c | 24.922 / 34.26 | 35.792 / 58.31 | 24.004 / 32.42 | 30.260 / 58.83 |
| release / blocks-16384.c | 13.369 / 25.05 | 12.813 / 21.50 | 10.655 / 20.17 | 12.589 / 26.54 |
| release / blocks-65536.c | 52.153 / 91.75 | 51.980 / 77.61 | 43.808 / 72.81 | 53.532 / 97.83 |
| debug / parentheses-65536.c | 90.073 / 39.60 | 120.473 / 138.04 | 87.978 / 37.71 | 107.523 / 141.97 |
| debug / blocks-16384.c | 31.395 / 30.20 | 90.543 / 139.32 | 29.314 / 25.51 | 81.263 / 143.49 |
| debug / blocks-65536.c | 128.044 / 97.19 | 376.289 / 533.85 | 118.345 / 78.27 | 327.571 / 550.95 |

At 65,536 blocks, raw debug stack residency is roughly 511 MiB even though the
reservation itself is cheap. The hybrid is about 78 MiB whole-process RSS there
versus 551 MiB reserved RD. Frame sizes, not virtual reservation size, dominate
this difference. Stacker can use a different live frame layout/segment footprint;
its measured process peak is retained above rather than assumed equal to raw
recursion. The earlier capacity-cliff warning still applies to arena comparisons.
A reused worker would retain touched stack pages unless explicitly reclaimed;
these samples use fresh workers and release the mapping after join.

## Reservation boundaries and output identity

Three trials per reservation/workload:

| Reservation | Release Lua | Release 16K blocks | Debug Lua | Debug 16K blocks |
|---|---|---|---|---|
| 1 MiB | accepted 3/3 | SIGSEGV 3/3 | accepted 3/3 | SIGSEGV 3/3 |
| 64 MiB | accepted 3/3 | accepted 3/3 | accepted 3/3 | SIGSEGV 3/3 |
| 1 GiB | accepted 3/3 | accepted 3/3 | accepted 3/3 | accepted 3/3 |
| 100 GiB | accepted 3/3 | accepted 3/3 | accepted 3/3 | accepted 3/3 |

There are nine expected guard-exhaustion SIGSEGV subprocesses in these sweeps.
All other reserved parses succeed, including both 56-case grids through depth
65,536. A 1 GiB reservation suffices for the 16,384-block probe in both builds;
the larger grid uses 100 GiB. Larger reservations add virtual headroom without
proportional residency. This is bounded by both
address space and available physical memory, not an unlimited-memory guarantee.

A standard Rust worker requested with `--stack 107374182400` fails to start in
all six release/debug attempts (`EAGAIN`, resource temporarily unavailable).
The explicit no-reserve mapping succeeds at the same usable size. Requesting
a large ordinary worker is therefore not equivalent on this host.

- Broad corpus: all 200 runs match the machine in each build.
- Grammar corpus: 150 cases in each build; moving either machine or recursive
  parsing to the reserved stack preserves its complete rendered AST, ordered
  provenance and diagnostics. Together these are **800/800 transport checks**.
- All 108 valid/extension grammar cases match across algorithms. The same ten
  malformed cases differ from the production machine, exactly as before this
  stack change; no recovery mismatch is silently relabeled as a match.

The 15 nonzero subprocess exits are fully accounted for: nine guard-exhaustion
SIGSEGVs and six standard-worker setup errors. Raw return codes/stderr remain
archived. Full deep AST rendering was not attempted; acceptance/allocation/stack
checks at 65,536 do not prove deep structural identity.

## Validation and next decisions

The successful wrapped build and canonical check batches both end in `EXIT 0`.
All three new worker tests pass: lazy reservation with borrowed callback/result,
exact panic propagation after join, and invalid reservation rejection. Strict
Clippy passes for the study feature, default, benchmarking and combined study
features. Canonical tests pass: 2,304 library, three additional integration,
254 main integration, 31 allocation-count and six focused recursive tests.
Nine canonical tests remain ignored. Nightly rustfmt, Python/shell syntax and
`git diff --check` pass.

The first build caught five corrected Clippy issues. The first worker test run
failed on unsupported optional THP advice; it now passes with backing measured
explicitly. Failed and successful validation logs, exit status, test lines and
raw-log hashes are retained in [validation.json](parser-reserved-stack-data/validation.json).

The next architectural decision is whether to accept potentially large debug
stack residency in exchange for the simpler reserved-worker approach. If yes,
validate the native Windows path and close the existing recovery/extension gaps
before integrating a worker ownership boundary into the compiler. If strict
native-stack/memory bounds on adversarial input matter more, keep the bounded
hybrid. The separately measured alignment-table optimization remains independent.
No default pipeline change, push or PR was performed.
