# Inliner validation and measurements

Implemented on `claude/ir-inline`, 2026-10-11, based on `9c9ce0d9f477`.

## Policy and representation

`inline` is the first default pass. An arena-backed, explicit-stack Tarjan
traversal orders the original direct-call graph bottom-up and excludes recursive
SCCs as callees. Each function runs inline once, then the eight function passes
repeat to a fixpoint (up to eight rounds). Callers therefore copy cleaned-up
callees, and promotion sees both copied slots and formerly escaped caller slots.
Explicit pass lists retain their order and may request additional inline runs.

Instruction counts include terminators: leaves up to 30 instructions are eagerly
accepted, and other callees up to 100 instructions are accepted within the same
caller budget. Growth is bounded by the larger of the caller's initial size and
256 instructions, with a 2,000-instruction absolute cap. Every complete call-site
expansion consumes one bisect rewrite.

Expansion owns independent slots, blocks, parameters, instruction results, call
operand lists and wide constant bits. It splits the call's block and joins all
returns through a continuation parameter, omitted for void. Entry parameters map
to the original arguments. Symbol, signature and access-tag ids remain shared
within the module. Function definitions remain available for exported symbols,
address references and indirect calls.

Variadic functions, recursive SCCs and `va_start` bodies are refused. The IR has
no general returns-twice attribute: recognized builtin names, direct wrappers and
bodies taking those builtin addresses are guarded, matching the current lowering
refusal contract. Arbitrary indirect returns-twice metadata remains unsupported.
Inlining is pre-ABI. Aggregate-by-value signatures remain rejected by lowering;
ordinary pointers to caller-owned copies are preserved without adding ABI copies.

## Validation

All final commands exited successfully:

| Command | Result |
| --- | --- |
| `cargo test --all-targets` | 1,507 passed, 3 ignored across 17 test binaries |
| `cargo test --features benchmarking-internals --test allocation_count` | 25 passed |
| `cargo +nightly fmt` then `cargo +nightly fmt --check` | Passed |
| `cargo clippy --all-targets -- -D warnings` | Passed |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | Passed |
| `git diff --check` | Passed |
| `BCC_DIFFTEST_CASES=5000 cargo test --lib difftest` | No miscompiles; 4,406 kept, 594 discarded for undefined behavior |
| `cargo test --test bench_corpus` | All ten reduced corpus programs passed before/after optimization |
| `cargo build --release` | Passed |

The differential sweep had 123 poison-returning cases and 1,676,330 default-pipeline
rewrites; refinement checks passed. The native differential test also passed.
The generator now includes nested, multi-return helpers with slots on every seed,
so the single inline pass and default pipeline both exercise expansion.

Fourteen dedicated IR tests cover splitting, bottom-up forward calls, multiple
returns, void calls, address-taken caller slots, independent copied slots,
self/mutual recursion, refusal and growth limits, bisect prefixes, wide constants,
indirect-call operands, switches, callee loops, reverse block layout and a
2,000-function call chain. Wide floating constants are checked as exact textual
bits because the interpreter does not execute f80/f128. A C regression checks
nested calls and local-slot promotion under both Linux and Windows target layouts.

## Five-repetition comparison

The final comparison is recorded below after all builds and tests became idle.
An earlier comparison overlapped validation and is excluded from the final figures.

Command: `python scripts/run.py compare_optimizers --reps 5`, after
`cargo build --release`. Host: Windows 11 (10.0.26300) AMD64; AMD Ryzen 9 5900X 12-Core Processor; 24 logical CPUs. Target:
`x86_64-w64-windows-gnu`; bundled clang 23.1.1. The harness selected the release
binary. All five arms (A0, A1, A2, A3, R) completed all ten benchmarks, with
no exclusions or checksum/interpreter mismatches. There is no arm N on this
branch, so A1/N is unavailable.

Metric: whole-process Windows wall time, five interleaved repetitions per
cell. No CPU pinning or priority override; the empty-process floor was
32.1 ms. Builds and tests were idle for this final run.
These Windows measurements show trends; CPU-isolated native Linux remains
needed for precise generated-code conclusions, as the harness README explains.

| Benchmark | A1 median seconds | A2 median seconds | A1/A2 medians | A1/A2 minima |
| --- | --- | --- | --- | --- |
| bytecode | 0.322 | 0.293 | 1.098 | 1.124 |
| collatz | 0.355 | 0.343 | 1.034 | 0.995 |
| crc32 | 0.373 | 0.367 | 1.016 | 1.006 |
| hashtable | 0.261 | 0.243 | 1.073 | 1.061 |
| levenshtein | 0.227 | 0.228 | 0.992 | 1.006 |
| linkedlist | 0.511 | 0.457 | 1.118 | 0.978 |
| matmul | 0.330 | 0.321 | 1.028 | 0.982 |
| nqueens | 0.212 | 0.213 | 0.996 | 0.915 |
| sieve | 0.393 | 0.399 | 0.985 | 1.033 |
| sorts | 0.263 | 0.217 | 1.215 | 1.159 |
| Geometric mean (10 benchmarks) | | | 1.053 | 1.024 |

The harness's 95% bootstrap interval for the A1/A2 median geometric mean is
[1.006, 1.091] (2,000 resamples). Ratios below one favor A1. The requested
levenshtein ratio is 0.992 and hashtable is 1.073. These compare the complete
current bcc pipeline against LLVM O2.

Raw results: `target/optimizer-bench/20261011-143446/results.jsonl`.
Full harness summary: `target/optimizer-bench/20261011-143446/summary.md`.
Final timing log: `target/inline-measure-final.log`.
The earlier overlapping run is `target/optimizer-bench/20261011-142331/`
and is not used in the table above.

The whole-file commit partition is `target/commit-plan.md`. No staging or
commits were performed, and the LLVM junction was not modified.
