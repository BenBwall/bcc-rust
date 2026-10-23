# Parser frame and recursion audit

Date: 2026-10-21. Baseline source: `9c11aca8167245020f4308697d0883152b49e3ca`.
Source references below describe that baseline unless explicitly marked as
subsequent study work. The study harness and experimental parser are described
in the main measurement report.

## Findings and evidence boundary

There is enough existing optimization in the explicit machine that a recursive
rewrite needs a measured advantage. The current driver steps frames in place,
reuses frame vectors and large boxes, maintains a running retained-node count,
and resolves `Continue` without returning to the driver. The best first
experiments are removing a redundant alignment-table copy, simplifying cursor
lookahead, eliminating duplicate type-name classification, and making
designation state lazy. None requires changing the grammar architecture.

This source audit records cost hypotheses rather than measured speedups.
The feature-gated alignment-table experiment is described at the end.
Measurements and validation are in [the study report](parser-recursive-descent.md).

Read guidance: [.agents/AGENTS.md](../../.agents/AGENTS.md),
[parser-roadmap.md](../../parser-roadmap.md), [GLOSSARY.md](../../GLOSSARY.md),
[README performance guidance](../../README.md#performance-changes),
[C99 checklist](../../c99-parser-compliance-checklist.md), and
[CONTRIBUTING.md](../../CONTRIBUTING.md#commit-messages). The guide explicitly
requires the whole-language parser to retain specialized resumable frames on
one explicit stack. A recursive prototype is a study, not an authorized change
to that production contract.

## What the machine actually pays for

- **Per driver action:** [parsing.rs:290](../../src/translation_phases/parsing.rs#L290)
  checks frame depth, takes the returned value and current token, replaces the
  `ArenaVec` stack descriptor with an empty descriptor, steps its last frame,
  restores the descriptor, updates retained-node counts, checks source/node
  budgets, and dispatches the action. This moves the vector descriptor, not
  all active frames. The empty `ArenaVec::new_in` allocates nothing. Repeated
  matches, bookkeeping and branches could cost CPU on very small productions;
  their importance under fat LTO is unknown.
- **Frame dispatch:** [machine.rs:154](../../src/translation_phases/parsing/machine.rs#L154)
  loops around `step_once`, matching the frame family for each transition.
  `Continue` already bypasses the outer driver; `Reprocess` returns there with
  unchanged input. A recursive implementation can remove some dispatch but
  adds native calls and suspended stack frames. It is not inherently cheaper.
- **Push/reduce:** `ParseAction::Push` owns a complete `ParseFrame`
  ([machine.rs:104](../../src/translation_phases/parsing/machine.rs#L104)).
  [push_frame/pop_frame](../../src/translation_phases/parsing/frame_pool.rs#L163)
  lend/reclaim storage and update retained-node counts. Reduction moves out a
  frame, then [reclaim_pooled](../../src/translation_phases/parsing/machine.rs#L474)
  clears and returns its vectors/boxes. These transitions can move a sizable
  enum even though ordinary token steps do not. Optimized assembly must settle
  whether those moves survive in each hot path.
- **Accounting:** [retained_node_count](../../src/translation_phases/parsing/machine.rs#L359)
  reads a small fixed number of lengths, not the whole stack or every list
  element. There is no current O(depth) rescan per action to remove. Checked
  arithmetic and budget checks preserve structured resource failure; removing
  them is not a valid performance comparison.
- **Tests versus production:** traces, action-budget assertions and syntax logs
  in the driver are `cfg(test)`. Their cost must not be attributed to release
  parsing. A test executable's memory is not a release parser-memory result.

### Sizes and storage

The 64-bit expectations in
[node_sizes.rs:100–126](../../src/translation_phases/parsing/tests/node_sizes.rs#L100)
pin `ParseFrame` at **152 bytes**, `ParseValue` at **40**, `Expression` at **48**,
`Statement` at **48**, `DeclarationSpecifiers` at **40**, and `Declarator` at
**24**. These are checked-in expectations, not freshly measured sizes. The test
does not pin `ParseAction`, `Option<ParseValue>`, individual frames, pool boxes,
or native call-frame size. Do not invent those sizes from the enum definitions;
Rust layout, padding, niches and optimizer spills matter.

A control-vector capacity of 4,096 entries would therefore occupy 622,592 bytes
(608 KiB) at the pinned layout, excluding vector metadata, pool allocations,
scope state and spare capacity. Grammar nesting is not necessarily control
depth: one nested construct can suspend several family frames. A small active
variant still occupies a full 152-byte slot. `ParseAction` must accommodate a
`Push(ParseFrame)` payload, but its exact layout needs separate inspection.

State contributing to that footprint:

- [ExpressionFrame:1661](../../src/translation_phases/parsing/frames/expression.rs#L1661)
  contains two arena-vector descriptors, phase/mode/boundary state and an
  optional call-state box. Call state is already lazy at
  [expression.rs:2019](../../src/translation_phases/parsing/frames/expression.rs#L2019).
  Its three argument/provenance vectors keep capacity when reclaimed.
- [DeclaratorFrame:731](../../src/translation_phases/parsing/frames/declarator.rs#L731)
  contains two vector descriptors and pending nested-declarator, array and
  provenance state. Its shared `chain_named` flag already avoids walking the
  nested declarator chain repeatedly.
- [DeclarationFrame:606](../../src/translation_phases/parsing/frames/declaration.rs#L606)
  combines pending specifiers with two vector descriptors.
  [CompoundStatementFrame:242](../../src/translation_phases/parsing/frames/compound_statement.rs#L242)
  has item/provenance vectors and scope/error state.
- [machine.rs:75](../../src/translation_phases/parsing/machine.rs#L75) already
  boxes Modern, GNU, MSVC, ParameterList and StructOrUnion frames. Pool boxes
  use `allocator_api2::Box<T, &Bump>`
  ([frame_pool.rs:149](../../src/translation_phases/parsing/frame_pool.rs#L149));
  do not assume they have the layout of a global-allocator `Box<T>`.

Boxing every remaining family might shrink the enum, but adds pointer chasing,
pool traffic and cold allocations. First identify the largest remaining inline
variant with a layout probe; test boxing that variant alone and
verify that the enum actually shrinks. Embedding small inline vectors may save
allocations while enlarging every control slot, an opposing tradeoff.

### Construction, allocation and provenance

[construction.rs:80](../../src/translation_phases/parsing/construction.rs#L80)
builds a token-diagnostic map by scanning pending preprocessing errors, copying
selected provenance and making FIFO queues. Clean inputs skip most of this;
extension-heavy headers can behave differently. Constructors also intern three
builtin typedefs, initialize scopes and create empty vector/pool descriptors.
They do not preallocate each pool's element storage. This work belongs in a
phase-7 benchmark that includes `Parser::from_preprocessed`.

The token cursor reads the already-preprocessed array by index, so arbitrary
lookahead does **not** allocate a new token buffer
([token_cursor.rs:32](../../src/translation_phases/parsing/token_cursor.rs#L32)).
Declaration/type classification may probe current typedef bindings, but
[ScopeStack::is_typedef:107](../../src/translation_phases/parsing/scope.rs#L107)
uses the innermost-name map plus the file-scope bitset, not a linear walk of
every enclosing scope. Any classifier cache must be invalidated on publication,
shadowing and scope exit, including transitions without token consumption.

Two lazy whole-input costs deserve their own workload:

- [retain_since:158](../../src/translation_phases/parsing/token_cursor.rs#L158)
  copies the **whole token array** into the TU arena once when retained
  attribute tokens first need it; later slices share that copy. This is linear
  and deliberate, but a tiny retained attribute on a large TU can trigger it.
  A recursive driver does not remove it. Changing token ownership to eliminate
  the copy is a wider lifetime/storage change, not a cheap frame optimization.
- [alignment_argument_closes:197](../../src/translation_phases/parsing/token_cursor.rs#L197)
  builds an N-element boolean table and a delimiter stack, then copies the
  table into a second arena allocation at line 243. The table construction is
  already linear and cached. Removing its final copy is the smallest concrete
  memory improvement found here.

[VecPool](../../src/translation_phases/parsing/frame_pool.rs#L113) reuses empty
allocations with a LIFO spare list; [BoxPool](../../src/translation_phases/parsing/frame_pool.rs#L134)
reuses whole state. This prevents one fresh allocation for every sequential
grammar occurrence. It retains high-water capacities, however: a large early
list can leave memory resident until parse-arena destruction. Repeated nested
lists and interleaved vector growth can consume more storage than final live
contents. [Bump::grow:667](../../src/util/memory/bump.rs#L667) grows only the
latest compatible allocation in place; otherwise it allocates and copies, with
the old allocation remaining. Deallocation can roll back only the latest block
([bump.rs:645](../../src/util/memory/bump.rs#L645)), so saying that every dropped
arena allocation is individually reclaimed would also be wrong.

[alloc_syntax_list:46](../../src/translation_phases/parsing/allocation.rs#L46)
copies accumulated lists into immutable TU storage and clears the scratch
vector. The copy and simultaneous scratch/final storage are real; the pool
avoids repeating the scratch allocation. Node storage, token/provenance storage
and parse working memory must be reported separately.

Provenance merges are not uniformly copying:
[merge_vectors:85](../../src/translation_phases/context/source_vectors.rs#L85)
shares adjacent ranges, handles recovery anchors and can extend a tail;
[merge_vector_list:134](../../src/translation_phases/context/source_vectors.rs#L134)
flattens noncontiguous lists once. Macro-fragmented or overlapping operator
ranges can still cause copies. Measure source-segment growth and copied bytes
before blaming the control stack. The historical
[GCC report](gcc-torture-parser-results.md) describes older growing-prefix and
stack-rescan bugs; it is not evidence that those bugs remain, and its old
40-million-segment default is superseded by current `ParserLimits`.

## Cheap experiments, in priority order

Each experiment should be a separate change against the same baseline, not a
bundle. Benefits listed here remain unmeasured.

1. **Keep the existing alignment table allocation.** At
   [token_cursor.rs:243](../../src/translation_phases/parsing/token_cursor.rs#L243),
   replace `Some(arena.alloc_slice_copy(&matches))` with
   `Some(matches.leak())`. `matches` already uses the parse arena and has its
   final length; `ArenaVec::leak` is an existing idiom in
   [bump.rs:204](../../src/util/memory/bump.rs#L204). This removes an N-byte
   boolean copy/allocation when the cache is built, without changing matches or
   their lifetime. The old table's allocation cannot roll back while the new
   copied table follows it. Compare a large input with an alignment attribute,
   malformed delimiter/semicolon cases, and a no-attribute control. Expect a
   storage reduction only when this cache is used; no universal speedup.
2. **Remove the redundant current-token lookup in arbitrary lookahead.** At
   [token_cursor.rs:145](../../src/translation_phases/parsing/token_cursor.rs#L145),
   try `self.at(index.checked_add(1)?)` instead of
   `self.at(0).and_then(|_| self.at(index.checked_add(1)?))`. If a later token is
   in bounds then the current token is in bounds; checked overflow and
   `abandon()`'s shortened end retain the same result. Test EOF, abandoned
   input, offset overflow, recovery scans and deep declarators. The optimizer
   may already remove the duplicate check; inspect generated code or compare
   instruction counts before claiming a CPU gain.
3. **Classify a parenthesized type prefix once.** The only callers of
   [parenthesized_compound_literal_follows:1894](../../src/translation_phases/parsing/frames/expression.rs#L1894)
   are the Countof/Alignof paths at baseline lines 247–249 and 288–289. Both
   already short-circuit on `parenthesized_type_name_follows`, which the helper
   repeats at line 1895. Remove that repeat and give the helper a name/comment
   stating its known-type-prefix precondition. This saves duplicate scope
   probes/prefix scans on those paths. Preserve storage-class handling and the
   matching-close scan; use sizeof/cast/compound-literal/alignof/countof
   regressions, including typedef shadowing. Expected benefit is narrow.
4. **Borrow designation state only when a designator starts.**
   [machine.rs:439–444](../../src/translation_phases/parsing/machine.rs#L439)
   eagerly calls `take_designation()` on every pushed initializer, although
   scalar initializers never use it. Remove that eager borrowing; acquire the
   box before the GNU-field, `.` and `[` designator branches in
   [initializer.rs:795](../../src/translation_phases/parsing/frames/initializer.rs#L795).
   Make the helper accept pools or add an explicit ensure call before
   `merge_designation_source`; its current
   [expect at line 898](../../src/translation_phases/parsing/frames/initializer.rs#L898)
   assumes eager provisioning. Reset, optional designation extraction,
   retained-node counting and reclaim already handle `None` in their ordinary
   paths. Audit every accessor and malformed designator path before changing
   that invariant. Compare scalar-only and deeply undesignated initializers
   with designated/GNU/range/recovery controls. Saving is mostly box-pool
   traffic and peak simultaneous state, not one global allocation per scalar.
5. **Hoist family dispatch out of the continuation loop.**
   [machine.rs:154–198](../../src/translation_phases/parsing/machine.rs#L154)
   could match the family once, then loop over that concrete frame's `step`
   until it returns something other than `Continue`, passing the child return
   only once. Preserve the existing continuation protocol and accounting
   boundaries. This is a small scheduling experiment; LTO may already achieve
   it and extra specialization may increase instruction-cache pressure.

Second-tier experiments: isolate a single normal-path `Reprocess` to
`Continue` conversion, or box one confirmed largest inline variant. Do not
bulk-convert recovery transitions: an outer driver boundary also runs resource
checks and test action-budget/trace logic. For example, Declarator AwaitNested
at baseline line 299 changes retained-node accounting, so skipping its boundary
requires proof that resource diagnostics and allocated partial syntax remain
equivalent. Preserve checked counts, recovery timing and caller-owned tokens.

## C99 floors and recursion choices

The normative basis recorded in the checklist is C99 §5.2.4.1p1 and footnote 13,
printed pp. 20–21 / PDF pp. 32–33: capability for at least one program
containing an instance of every listed limit, with encouragement to avoid fixed
limits. These numbers are floors, not recommended rejection thresholds.
Relevant syntax cases include 127 nested blocks; 12 combined pointer/array/
function derivations; 63 nested parenthesized declarators; 63 parenthesized
expressions; 127 definition parameters and call arguments; 511 block-scope and
4,095 external identifiers; 1,023 cases in one switch, members in one
struct/union, and enumerators; and 63 nested struct/union definitions.
Preprocessing also owns its conditional/include floors; changing the parser
does not satisfy those requirements.

Current [limits.rs:92](../../src/translation_phases/parsing/limits.rs#L92)
uses representation ceilings: `usize::MAX` roots/nodes/source segments and
`u32::MAX` frame depth. On failure it emits one diagnostic/error root and
iteratively reclaims frames and restores scopes
([limits.rs:35](../../src/translation_phases/parsing/limits.rs#L35)). OS memory
exhaustion remains outside that recovery guarantee. Do not present current
defaults as small protective grammar limits.

Existing regression sources exercise 4,096 parenthesized declarators
([tests/limits.rs:71](../../src/translation_phases/parsing/tests/limits.rs#L71)),
4,096 expression groups and 512 initializer braces
([tests/expressions.rs:1213](../../src/translation_phases/parsing/tests/expressions.rs#L1213)),
127 nested statements/blocks and parameters
([tests/statements.rs:1756](../../src/translation_phases/parsing/tests/statements.rs#L1756)),
large names/cases/members/enums and aggregate depth
([tests/limits.rs:680](../../src/translation_phases/parsing/tests/limits.rs#L680)),
and exact frame-limit boundaries
([tests/limits.rs:569](../../src/translation_phases/parsing/tests/limits.rs#L569)).
Their presence is evidence of the intended behavior, not a claim that this
audit reran them. Initializer stress depth 512 is a project regression, not a
separate numeric C99 initializer-nesting floor.

**Plain recursive descent:** native calls can specialize control flow and avoid
the universal enum slot, but native activations include return addresses,
saved registers, alignment, spills and locals. Rust provides no general
tail-call guarantee. Converting Double-E's operator/pointer/list loops into
recursive chains can make flat valid input consume unbounded native stack.
Keep those loops iterative in any fair prototype. Stack overflow generally
aborts rather than producing the parser's recoverable resource diagnostic.

**`stacker`:** stack growth needs checks at every unbounded recursive cycle,
including mixed expression/type-name/declarator/initializer/statement paths.
One outer `maybe_grow` call does not protect recursion occurring inside its
closure unless that recursion reaches another guarded boundary. Check cost,
slow-path stack allocation, switching, initialization and possible retained
segments must be counted. Native segments live outside parse-arena high-water
metrics; zero global allocations alone cannot prove zero stack allocation.
Red zones must leave room for the largest unguarded helper, diagnostics and
unwind paths. Validate platform/architecture support, static linking, unwind
behavior, sanitizer/Valgrind handling, and borrowed parser state on the pinned
dependency version. This audit did not fetch or validate stacker's source; it
was absent from the baseline dependencies. It is not automatically compatible
with the arena-only compiler allocation contract.

**One large worker-thread stack:** simple to prototype, but still finite and
platform-dependent. Include thread creation/join, TLS and stack reserve/commit
costs in an end-to-end result; label an already-started-worker measurement
separately. `RUST_MIN_STACK` does not resize an existing/main thread stack;
explicit worker stack configuration does not resize the caller. OS limits,
guard pages and compiler frame sizes affect usable capacity. A huge virtual
reservation is not the same as RSS, and committed stack bytes can dominate
deep inputs. Arena statistics omit that memory. Per-thread allocation counters
must be enabled on the worker: the existing allocation test explicitly counts
per thread ([allocation_count.rs:1](../../tests/allocation_count.rs#L1)).

**Grammar depth limits:** check before recursing, never after overflow. A
machine-frame ceiling, grammar nesting depth, operator depth and native call
depth are different quantities. A limit that accepts 127 blocks can still
reject valid combinations of nested declarators, types and expressions below
their individual floors. A small fixed cap also weakens current substantially
deeper behavior even if it clears the standard minima. Adding one requires an
explicit behavior decision, boundary tests and the existing stable failure
cleanup; it is not a free performance optimization.

**Hybrid:** preserve Double-E and flat list/pointer loops, and either retain
the machine for unbounded structural nesting or guard a shallow recursive fast
path and transfer to explicit continuations before exhaustion. Falling back
mid-production requires reifying pending state, scope/recovery ownership and
typed returns; it cannot simply restart at the current token. Reparse/rewind
can duplicate diagnostics, nodes, bindings and provenance. A single recursive
driver around existing frame `step`s is a useful scheduling ablation but still
constructs actions/frames and performs most of the existing work; it does not
measure a fully specialized recursive grammar implementation. Match semantics
and metric coverage before interpreting either result.

## Available tooling, corpora and preliminary measurement evidence

Local read-only observations: Linux x86-64, `rustc 1.99.0
(b940084d7 2026-09-28)`, GCC `15.2.0`, GNU binutils `2.46`, GNU `/usr/bin/time`
(its version string reports `UNKNOWN`). The shell stack soft limit was 8 MiB,
hard limit unlimited. `perf_event_paranoid` was 2; that setting alone does not
establish usable PMU events. The queried THP enabled sysfs path was absent, so
actual huge-page backing was not established. Repository VM code still
requests aligned 2 MiB commit increments and reserves 100 GiB per active region
([vm.rs:299](../../src/util/memory/vm.rs#L299)).

Valgrind was absent from the initial PATH scan and available later as
**3.26.0**, including Callgrind, Cachegrind, Massif, DHAT and Memcheck engines.
`callgrind_annotate --version` printed 3.26.0 but returned status 255;
`valgrind --version` and `callgrind_control --version` returned 0. Availability
does not establish that these engines can profile this program. Heaptrack, samply, flamegraph, hyperfine, coz,
rr, pahole, GDB and LLVM disassembly/DWARF tools were not found on PATH in the
initial scan. `readelf`/`objdump` are available for later binary inspection.
No target directory was initially present in this worktree or the checkout's
main root, and `CARGO_TARGET_DIR` was unset; build artifacts may appear as the
study proceeds.

Preliminary measurement observations (superseded by the final study report):

- Installed `linux-perf 7.0.0-38`; hardware instructions/cycles events are
  unavailable in this VM: `No supported events found. instructions:u not
  supported`. Software `task-clock` works. Do not report software task-clock as
  retired instructions or cycles.
- Default Callgrind aborts when a 100 GiB arena `mmap` is rejected with `EINVAL`.
  The study uses an opt-in `parser-study-small-regions` configuration with 1 GiB
  reservations solely for profiling. Keep native timing/RSS runs on the default
  100 GiB regions. Callgrind results from the 1 GiB configuration require that
  label and a successful correctness cross-check; they are a different memory
  configuration and instruction counts are simulated, not hardware PMU data.
- Acquired Lua **v5.4.8** `onelua` from a pinned GitHub archive; GCC 15
  preprocessing produced **693,358 bytes**. Preliminary baseline: about
  **42 ms** phase-7 time, **616,192 bytes** parse-arena use,
  **15,990,600 bytes** TU growth and **8 diagnostics**, still to inspect.
  These are preliminary observations, not this audit's measurements or
  a clean acceptance/performance conclusion. Record archive hash, preprocessing
  flags, input hash, features and repetitions in the measurement report.
- SQLite's official domain is approval-blocked; no SQLite timing is available
  from that attempt. Do not substitute a claimed SQLite benchmark result.

Immediately available tracked inputs include five parser-compatibility C files,
428 diagnostic C fixtures (many test other phases), 14 manual test-program C
files and 17 `benches/c` programs. These counts were filesystem observations,
not numbers of passing tests. Generated parser benchmarks cover one million
simple declarations, mixed C99, macro-heavy, expression-heavy (4,000 function
groups) and declaration-heavy (6,000 groups) workloads
([benchmarking/input.rs:18](../../src/benchmarking/input.rs#L18)). They are
reproducible starting points, but do not all exercise the alignment-table path.

The GCC torture downloader defaults to `target/compiler-corpus`; that directory
was absent at audit start. Its tracked survey report records old results, not a
currently rerun corpus. `scripts/run_program_corpus.py` also names zlib, Lua and
SQLite, but its default corpus path is a developer's Windows location. A script
mentioning a corpus does not establish Linux-local availability. Do not invoke
download/survey scripts merely to inspect what might be installed.

## Measurement handoff

Follow README's at-least-seven interleaved runs on a noisy machine, reporting
the minimum and Criterion estimates with commit, toolchain, target, features
and input hashes. Separate phase 7 from the full pipeline. Existing
[Parser only](../../benches/bench.rs#L88) performs preprocessing in untimed setup;
[PreparedParse::parse](../../src/benchmarking/prepared.rs#L44) includes parser
construction and summarization, with owning-arena destruction outside that
interval. Check local-result destruction boundaries when comparing the
study harness so the two intervals mean the same thing.

For each candidate retain roots, structured diagnostics/recovery and full
ordered provenance equivalence; inspection text is useful but should not be
the only check of diagnostic equivalence. Include valid inputs, truncated and
malformed continuations, macro/include locations, typedef shadowing, exact
resource boundaries and deep mixed-family nesting. Add a workload specifically
triggering alignment-table construction for candidate 1 and scalar/deep
undesignated initializers for candidate 4.

Report parse-arena high water, incremental TU growth, source-segment growth,
regions/reservation/commit and fresh-process peak RSS. For recursive variants
also report native stack use, stack growth/worker allocations and matched
thread setup policy. Pool bytes, stack bytes, node bytes and process RSS answer
different questions. Linux commit increments can hide a small byte saving in
RSS; phase-1–7 peak RSS minus phase-1–6 peak RSS is not a reliable parser-only
peak because the peaks can occur at different times.

The stock [memory harness](../../benches/memory.rs#L128) iterates
`BenchmarkInput::ALL`, not `PARSER_STRESS`; a memory result for expression-heavy,
declaration-heavy or Lua input therefore needs the additional
measurement seam. The `benchmarking-internals` executable/coz progress point
measures preprocessing, as README warns. Callgrind collection should bracket
only the intended phase-7 interval; whole-process data would include untimed
input preparation despite Criterion's clock boundaries. Default heap-only
profilers can miss VM-backed arenas and native stacks; use arena counters and
OS memory accounting, and label any page/stack profiling options explicitly.

Measurements and validation gates are recorded in the main study report.
The isolated alignment experiment changes only `token_cursor.rs`.

## Alignment table storage experiment

Candidate 1 is implemented behind `parser-study-leak-alignment`, alongside a
benchmark-only token-count accessor.

Changes in `token_cursor.rs` only:

- `Upstream::token_count()` is `pub(super)`, gated on
  `benchmarking-internals`, and returns `self.tokens.len()` without fields or
  counters. The `PreprocessedTranslationUnit::study_token_count()`
  wrapper can normalize profiling observations by the same phase-6 token count.
- In `alignment_argument_closes`, mutually exclusive `cfg` blocks assign
  `self.alignment_closures = Some(matches.leak())` when
  `parser-study-leak-alignment` is enabled and preserve
  `Some(arena.alloc_slice_copy(&matches))` otherwise. Table construction,
  delimiter matching, cache lifetime and the returned boolean are unchanged.

Compare the large alignment fixture with and without
the feature using its exact token count, parse-arena high water (`parse_bytes`),
TU growth, diagnostics and syntax/provenance output. The predicted eliminated
allocation is one N-byte boolean table; the exact high-water difference may
also reflect alignment and arena allocation ordering and must be measured.
Use a no-alignment control to confirm the experiment only affects cache users.

For Callgrind, both comparison arms must use the same 1 GiB
`parser-study-small-regions` configuration and phase-7 collection boundary;
change only `parser-study-leak-alignment`. Report simulated instructions and
cache/branch events actually collected, normalized per token where useful.
Retain default 100 GiB regions for the separate native timing/RSS comparison.
The [completed study](parser-recursive-descent.md#cheap-frame-machine-optimization)
records 242,544 bytes less Lua scratch and a small modeled instruction reduction.
Shared-host timing did not establish a speedup; the feature remains opt-in.
