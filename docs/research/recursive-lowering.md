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

The default helper remains available for task/recursive differential checks in
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
