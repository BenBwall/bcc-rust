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

## Saved baseline repetitions

Initial feature-disabled harness, seven consecutive samples per input (not yet
the interleaved candidate comparison):

| Input | Minimum ms | Median ms | CV % | Parse arena bytes | TU growth bytes | Global allocations / bytes |
|---|---:|---:|---:|---:|---:|---:|
| lua.i | 36.514 | 42.772 | 9.9 | 616,192 | 15,990,600 | 0 / 0 |
| bytecode.i | 0.126 | 0.141 | 12.4 | 17,736 | 18,352 | 0 / 0 |
| mandel.i | 0.102 | 0.105 | 30.9 | 10,696 | 10,832 | 0 / 0 |
| parentheses-256.c | 0.149 | 0.157 | 7.9 | 131,224 | 12,792 | 0 / 0 |
| parentheses-16384.c | 6.295 | 7.696 | 7.4 | 8,130,712 | 786,936 | 0 / 0 |
| unary-256.c | 0.119 | 0.120 | 10.2 | 131,224 | 12,792 | 0 / 0 |
| assignment-256.c | 0.088 | 0.095 | 4.8 | 37,208 | 25,104 | 0 / 0 |
| conditional-256.c | 0.253 | 0.281 | 5.2 | 255,704 | 43,536 | 0 / 0 |
| statements-256.c | 0.193 | 0.197 | 6.9 | 190,296 | 25,216 | 0 / 0 |
| blocks-256.c | 0.222 | 0.225 | 3.2 | 419,224 | 19,096 | 0 / 0 |
| declarator-256.c | 0.145 | 0.151 | 6.7 | 128,008 | 14,480 | 0 / 0 |
| initializer-256.c | 0.189 | 0.194 | 8.2 | 292,824 | 24,800 | 0 / 0 |
| chain-10000.c | 1.275 | 1.306 | 5.0 | 4,824 | 960,432 | 0 / 0 |
| initializer-list-10000.c | 3.788 | 3.818 | 2.0 | 1,575,792 | 1,120,224 | 0 / 0 |

A software `cpu-clock:u` sample profile (499 Hz, 100 Lua preparations/parses)
completed with zero lost samples. It is a full-process sample, not a phase-7-only
count: attribution to the named phase-7 boundary or parser symbols is useful,
but shared context helpers and inlining prevent treating its percentages as an
exact parser-cost partition. Callgrind will supply the isolated call graph.

The next build/check jobs are queued behind shared wrapper locks. The prototype
now covers precedence-climbing expressions and recursive statements/blocks,
with retained frame fallbacks for types, declarations, initializers and extension
productions. It has not yet passed compilation or differential validation.

## Reproduction checkpoints

```sh
# Run from the assigned worktree. Every Cargo command goes through the wrapper.
python3 scripts/parser_study_inputs.py --lua
python3 scripts/parser_study_cases.py
python3 /home/agent/workspace/bcc-rust/triage-2026-10-16/swarm/run.py sh -c '
  cargo clippy --all-targets --features parser-recursive-descent-study -- -D warnings &&
  cargo build --release --features parser-recursive-descent-study --example parser_study &&
  cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/examples/parser_study" .scratch/parser-study/study-release &&
  cargo build --features parser-recursive-descent-study --example parser_study &&
  cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/debug/examples/parser_study" .scratch/parser-study/study-debug'
python3 scripts/parser_study.py diff --binary .scratch/parser-study/study-release --output .scratch/parser-study/diff.json
python3 scripts/parser_study.py bench --binary .scratch/parser-study/study-release --output .scratch/parser-study/release.json
python3 scripts/parser_study.py bench --binary .scratch/parser-study/study-debug --output .scratch/parser-study/debug.json
# Run crash probes in separate processes on an explicitly sized native stack.
python3 scripts/parser_study.py stack --binary .scratch/parser-study/study-debug --stack 1048576 --output .scratch/parser-study/debug-stack-1m.json
```

The runner saves JSON after every sample and rotates mode order across timing
repetitions. Phase-7 hooks surround only parser construction and parsing; global
allocator counters exclude source loading, preparation, thread creation, output,
inspection and destruction. A separate worker's launch latency is printed when
`--stack BYTES` is used. Repeating a sample makes a fresh context and preprocesses
once outside that sample's timer; it does not replay a consumed context or retain
an earlier AST. The preprocessed `.i` inputs have already expanded system headers
and real macros once with GCC; the harness still tokenizes/collects these before
the phase-7 boundary. Macro/include provenance is tested separately on original C.

Candidate comparison uses `machine` and recursive modes from the same feature
binary; the extra study dispatch branch may affect the feature-enabled machine.
Feature-disabled baseline results are retained to expose that overhead. Do not
treat the experimental mode as the production pipeline.

The inspection check includes syntax shape/order/labels, node source segments,
expression operator/delimiter source segments, recovery markers, and rendered
diagnostics. Addresses and arena handles are excluded. This is an observable
structural differential check, not a proof of equality for every unrendered
private field. The dedicated corpus has 150 cases (87 valid, 21 extension, 42
malformed), documented in [parser-study-coverage.md](parser-study-coverage.md).

See [parser-frame-audit.md](parser-frame-audit.md) for source-cost hypotheses,
existing pooling/layout constraints, recovery costs, and normative C99 floors.

Checkpoint: the first broad debug differential completed all 192 comparisons
without crashes or inspection differences. The separate 150-case grammar corpus
is exposing malformed-condition recovery differences; these remain under review.
Linux RSS collection is being changed from inherited `getrusage` high-water marks
to `/proc/self/status` `VmHWM`, outside the measured interval.

A first explicit 1 MiB debug worker probe already reproduces the stack concern:
recursive parsing aborts on 127 nested blocks, and `limited:256` aborts before
its check can protect that case. The explicit machine, `growing`, and `hybrid:16`
survive all generated depths through 16,384 in this preliminary binary. Final
stack tables will be rerun on the final build. The timing runner now supports
`--baseline-binary` and `--alignment-binary`, allowing these binaries to rotate
in the same interleaved run instead of comparing different load windows.

The generated block depth excludes the function's outer compound. Added depth
126 to test exactly 127 total blocks, and depth 63 for C99's parenthesized
expression/declarator floors. A generic recursion-entry limit is not a C grammar
nesting limit: each nested compound adds statement and compound calls.

Final source checkpoint: `2ed14574` fixes punctuation diagnostics and one recovered
assignment marker, and saves nine focused recovery regressions. The native,
profile, alignment, and canonical-check bundle is running via one shared-wrapper
slot. `scripts/parser_study_profile.py` saves the exact Callgrind command, binary
hash, simulated event totals, and self/inclusive callgraph annotations. Empty
phase-7 collection is treated as an error.

### Preliminary debug comparison (superseded by final tables below)

Seven rotating repetitions, feature-enabled binary from before the final
recovery-text and stack-sampling fixes; opt-level 0, no debug symbols. These
are complete measurements, retained as a checkpoint, not the final release
comparison. Times are medians in milliseconds; CV is population standard
deviation divided by mean. No stack sampling or inspection during timing.

| Input | Machine ms (CV %) | Recursive ms (CV %) | Growing ms (CV %) | Hybrid 16 ms (CV %) |
|---|---:|---:|---:|---:|
| Clean Lua | 306.196 (5.26) | 210.218 (3.89) | 212.756 (1.66) | 233.601 (2.24) |
| Expression mix | 327.688 (6.19) | 225.991 (7.10) | 232.275 (4.67) | 218.880 (8.37) |
| Statement mix | 412.823 (6.36) | 261.855 (1.65) | 266.188 (2.23) | 259.604 (1.99) |
| Declaration mix | 256.690 (2.06) | 238.480 (1.94) | 231.191 (6.10) | 235.422 (8.85) |

The exact 127-total-block C99 probe (`blocks-126.c`) also aborts in recursive
mode on a 1 MiB debug stack. This is Linux evidence with an explicitly sized
worker, not a Windows measurement. Portability, unwind behavior and stacker
support on Windows still need a native-platform validation before deployment.

The grammar run can now use `--manifest .scratch/parser-study/grammar/manifest.json`
with `--inputs .scratch/parser-study/grammar`. The runner reports mismatches
explicitly and retains their inspections. A completed study data-collection
command may contain mismatches; its exit status alone is not a correctness gate.

## Interpretation guardrails

- The prototype is a slice of a real parser, not an alternate lexer or a toy
  arithmetic evaluator. It shares construction, provenance, scopes, lists and
  the operator table. Types, declarations, declarators, initializers and GNU/
  later-language atom productions retain their existing frames. Measured wins
  apply to that architecture, not to an unimplemented fully recursive compiler.
- A recursive grammar entry is not a C nesting level. Each compound introduces
  statement and compound entries; precedence climbing also has temporary RHS
  calls. A depth-limit policy must preserve every C99 minimum *in combination*,
  not just accept one isolated 63-parenthesis input.
- Arena bytes exclude native stack. Global allocator hooks exclude OS stack
  mappings and arena `mmap`; zero global calls does not mean zero memory work.
  `VmHWM` includes startup, preprocessing and live token/provenance buffers. Its
  before/after difference is not a parser-only RSS peak. Arena growth/commit and
  separately sampled native stack explain the phase-specific storage.
- `--sample-stack` measures address spans at grammar entries. Slopes are lower
  bounds on peak stack use, omit transient helper calls, and are collected
  separately from throughput. Stack-growth segments are compared only within
  the same segment; their spans cannot be summed into a contiguous stack size.
- The 64 MiB worker encloses preparation and parsing because arenas/context are
  created inside that worker. Reported parse time excludes launch, preparation,
  join and destruction; separate launch latency is recorded. This is not proof
  that the existing borrowed arena/AST API can move between arbitrary threads.
- Hybrid fallback suspends recursion for the whole selected subtree, using
  precedence-bounded expression frames to preserve operator ownership. It keeps
  two grammar implementations and therefore adds maintenance and differential
  testing obligations. Stack growth adds `stacker`/`psm` platform support and
  native-stack switching to those obligations.
- The terminal prototype limit reuses the existing frame-resource diagnostic,
  but counts recursive entries. It does not establish identical semantics for
  all production frame/node/provenance limits or error recovery. The default
  parser's representation bounds and cleanup remain authoritative.

`c99-floors.c` combines exactly 127 blocks with 63 parenthesized expressions;
final floor probes also compare limits 64, 256 and 512. This distinguishes a
limit that merely passes isolated floors from one that permits their combination.
Even a larger entry limit cannot protect a smaller native stack by itself.

RSS validation passed: with a Python parent holding a live 128 MiB buffer, the
old `getrusage` binary reported 146,518,016 bytes for a tiny parse; the final
`VmHWM` baseline reported 2,879,488 bytes before parsing and 3,076,096 after.
The final native RSS tables will use only the corrected binaries.

Final release batch (seven interleaved samples) is complete: clean Lua medians
are 35.608 ms production baseline, 36.813 ms study machine, 22.765 ms recursive,
22.888 ms growing, and 24.841 ms hybrid 16. This is a 36.1% recursive and 30.2%
hybrid time reduction against production. CVs are 1.89%, 2.73%, 5.26%, 1.41%
and 1.41%, respectively. Peak RSS is approximately 33.6 MiB in every mode;
there is no meaningful process-memory reduction on this real input.

On the release 1 MiB worker, raw recursion overflows by 4,096 parentheses or
1,024 nested blocks. Machine, growing and hybrid survive all 48 generated
kind/depth combinations through 16,384. A limit of 256 rejects the combined
C99-floor probe; limit 512 accepts it in release. Debug validation and isolated
counter collection remain in progress.
