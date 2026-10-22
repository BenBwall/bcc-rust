# Recursive lowering on a compilation worker

2026-10-22. Follow-up to [the parser study](parser-recovery-windows.md).
Implementation and measurement are in progress; no lowering speed claim is
established yet. The feature-disabled compiler retains its existing pipeline.

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
in arenas. Semantic analysis and optimizer worklists remain unchanged.

Ordinary library pipeline entry points retain task lowering even with the study
feature enabled; only the reserved-worker CLI and explicit study options select
recursion. The same helper supports task/recursive differential checks in
the same feature-enabled binary. Lowering tests compare complete printed IR,
function sources, retained call diagnostics and refusal diagnostics. Measurements
prepare the frontend outside the lowering timer and verify/inspect afterward.
Stack sampling runs separately from timing. Whole-worker wall time is also
reported, including preprocessing, analysis, setup/join and destruction.

## Validation plan

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

Exact commands and measured results will be added after validation.

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
Clippy and the five-case focused example test also pass. Initial compile/Clippy failures are retained in scratch
logs alongside the successful checks.

The shared worker's `CreateThread` binding requires `Win32_Security`. The
compilation-stack feature now explicitly enables that dependency feature for
normal Windows builds, rather than relying on the test-only dependency to
provide it. Native source cohort `932f32a8` already receives it from its test
configuration; this later declaration does not change its compiled test code.
