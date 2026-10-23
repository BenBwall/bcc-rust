# M8 one initializer plan producer

Base: `claude/semantic-facts-m6` at `94212cd9` (PR #73's head, containing
M1-M6). Branch `claude/semantic-facts-m8`. This implements migration row M8
of [the design](../design/semantic-facts.md) and inventory rows C18-C22.
Completion is recorded in [semantic-analysis.md](../../semantic-analysis.md)
("Initializer placement plans") and [middle-end.md](../../middle-end.md)
("Retained semantic interfaces").

## Result and choices

The current-object checker (`semantic_analysis/initializers.rs`) is the one
traversal that decides where an initializer's elements go. It now emits each
decision as it makes it, through `initializers/plan.rs`, into one retained
`InitializerPlan` per accepted initializer
(`facts/initializer_plans.rs`, queried as `sema.initializers.plan`). Automatic
and static lowering replay that plan (`lowering/initializers/plan.rs`); the
second current-object machine in lowering is gone.

Deleted from lowering:

- `Walker`, `leaves`'s brace walk, `Frame`, `Subobject` and the range
  `RangeCursor` product (`lowering/initializers.rs` shrinks from 1,224 to 613
  lines, and `plan.rs` adds 278);
- the field-name designator search over `Tag::fields` (`Walker::designate`)
  and `open_next`;
- braced-scalar and whole-copy rediscovery (`braced_scalar`,
  `Walker::braced_string`, `Walker::leaf`, `Walker::scalar` classification);
- the repeated-expression counting heuristic in `initialize_aggregate`
  (expression addresses counted over the leaves to choose remembered values);
- `Construct::Designator`, whose refusals only the walker produced.

Deleted from semantic analysis: `string_initializers` and
`SemanticTranslationUnit::string_initializes`, the M3-era shared string
classification whose only consumer was the walker; the plan's `String`
operation carries the decision.

Kept: `CoverageIndex` (the efficient reset-coverage index), `zero_gap`,
`initialize_zero/initialize/initialize_string`, static byte and relocation
serialization, and the `Leaf` vocabulary they consume. The automatic scalar
fast path in `declare_binding` (`T x = expr;` for a scalar `T`) still pushes
`Evaluate`/`Initialize` directly; it classifies the binding's representation,
not the initializer's placement.

Plan operations (`InitOperation`):

| Operation | Meaning |
| --- | --- |
| `Zero { destination, end, source }` | a brace pair around an aggregate; bytes no operation before `end` stores are zero, and earlier writes are replaced |
| `Select { destination, member, scope, source }` | an element enters member `member` of a union |
| `Store { value, destination, bit_field, source }` | a scalar gets a converted value |
| `Copy { value, destination, enclosing_unions }` | a structure or union gets an expression of its type |
| `String { literal, destination }` | a character or wide array gets a string literal |
| `Repeat { count, stride, length }` | a GNU range: the next `length` operations, stored at the range's first element, repeat `count` times `stride` bytes apart |
| `EmptyScalar { destination, source }` | `{}` around a scalar |
| `Unmodeled { destination, source }` | brace elision descends into a vector or an unanalyzable type |

Choices made within M8:

- **The checker's cursors carry plan positions.** Each `Current` cursor has
  its container's absolute offset, accumulated qualifiers and union chain
  (`Container`), computed from the member offsets and element sizes the
  checker already resolves. A field designator uses the checker's
  `member_indices` path; nothing searches fields by name again.
- **Ranges are repeats, not expansions.** One `Repeat` per range designator in
  a designation, opened at that designator's step: selections of unions above
  the range stay outside it, operations below it are stored at the range's
  first element, and nested ranges nest repeats. Lowering replays a repeat
  with an explicit cursor per element. This preserves the walker's order
  exactly: the walker re-ran a designation per range element, but its outer
  union selections were no-ops after the first element.
- **Union selections are explicit; resets are emission.** The plan records
  every `Select`. Emission keeps the walker's selection map keyed by absolute
  offset and union type, emits the reset `Zero` leaf when the member changes,
  and forgets the selections inside a whole `Copy` except those of the unions
  containing it (`enclosing_unions`). The selection state depends on the
  range element being emitted, so retaining resets would mean expanding
  ranges. The member chosen is never decided downstream.
- **Values belong to the occurrence.** `Store` and `Copy` refer to an
  `InitValueId`; a value inside a repeat with more than one element is
  `repeated`. Automatic emission evaluates such a value at its first store,
  snapshots and remembers it, and reuses it after; source-order evaluation is
  unchanged.
- **Completion at the end.** The root array of unknown size and the object's
  own flexible array member have incomplete types while the checker runs;
  their `Zero` and `String` operations are patched with the completed root
  type and the flexible member's written type (qualified as the object) when
  it finishes, as the walker received them. Another flexible member's empty
  list stores nothing, as before.
- **Same refusals.** Lowering checks each operation's destination
  representation where the walker did: a vector or binary128 store, a vector
  brace pair, `{}` around a scalar (`Unsupported(Initializer)`) and elision
  into a vector (`Unmodeled`) refuse with the walker's kind; a missing plan is
  a provenance-bearing `MissingFact`. Rejected and unmodeled initializers
  retain no plan.
- **`check_initializer` stays one function**, with the static-constant check,
  the final type completion, the elision step's plan bookkeeping and the
  completion choice moved into helpers to stay within Clippy's 500-line
  limit. Diagnostics, their order and the overrides recorder are unchanged.

## Equivalence evidence (before deleting the walker)

Commit `4d78995e` added a debug-build comparison inside lowering: every
initializer lowered by a debug build expanded its plan and asserted that the
leaves equal the walker's (kind, offset, size, type with qualifiers and
alignment, zero scope end and scope index, bit range, and expression or
literal identity), or that both refuse with the same error and provenance.
It ran in its own arena, so the lowering arena budgets tests assert were
unchanged. The consumer switch (`843bb4f0`) kept the same comparison with
the roles reversed.

| Run | Result |
| --- | --- |
| `cargo test --all-targets --no-fail-fast` with the comparison (walker consuming) | 2,368 passed, 0 failed, 7 ignored, no disagreement |
| Same after the consumer switch (plan consuming, walker compared) | 2,368 passed, 0 failed, 7 ignored, no disagreement |
| Debug binary, `--emit=ir`, 3,886 GCC 15.2.0 `gcc.c-torture` sources plus the 504 `.c` files under `tests/` and `docs/` | 3,298 lowered (367 of them with a braced initializer), 1,091 stopped at an existing diagnostic or refusal, 1 timeout (`compile/limits-caselabels.c`, a 60 s debug-build limit on a switch with many labels); 0 disagreements, 0 panics |

No golden was reblessed in any M8 commit. The torture sources were taken from
the pinned `gcc-15.2.0.tar.xz` in the main checkout's corpus cache and run
with a scratch driver, not `scripts/run_gcc_torture.py`, so this is not a
matched execute survey (see residual items).

The existing H7/H11 coverage passes unchanged on the plan: replacement and
union reset tests (`lowering/tests/review.rs`, including the 20,000-switch and
nested-overlapping-union resource tests with their scratch budgets), braced
strings and the pointer-array braced string, range initializers and their
once-only evaluation, bit-field reinitialization, flexible array member
storage (`the_last_flexible_array_member_initializer_sets_the_storage`),
function-designator `void *` relocations, the sqlite function table, nullptr
constant casts, aggregate reinitialization (`middle_end.rs`), packing and
alignment, static bytes and relocations, and the recovery and diagnostic
goldens.

## Plan sizes

`size_of` on the 64-bit hosts (`plan_records_stay_small`):

| Record | Bytes |
| --- | ---: |
| `InitOperation` | 56 |
| `Subobject` (offset and qualified type) | 24 |
| `InitValue` | 16 |
| `InitializerPlan` | 48 |

A plan costs 48 bytes plus 56 per operation and 16 per value, plus one map
entry keyed by the initializer address. `T x = e;` is one `Store` and one
value: 120 bytes. Measured with the ignored
`measure_retained_plan_sizes` test over the same torture sources:

| Corpus | Units | Plans | Operations | Values | Repeats | Largest plan | Record bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| GCC 15.2.0 `gcc.c-torture` (each file its own unit, errors included) | 3,886 | 8,137 | 127,239 | 116,980 | 2,090 | 100,002 operations | 9,387,640 |

One source dominates: `compile/limits-externdecl.c`, whose macros expand to a
written 100,000-element initializer, has the 100,002-operation plan, whose
operation records alone are 5.6 MB of the 9.4 MB. The other 3,885 units share
the remaining 27,237 operations, 3.3 per plan. Record bytes exclude the plan
map's hash table and arena alignment.

Work counters (`review_steps[3]`, one step per checker task, designator path
step and elision level) and operation counts, from
`tests/initializer_plans.rs`:

| Input | Operations | Work bound asserted |
| --- | ---: | ---: |
| `n` nested brace pairs, `n` = 128, 512 | `n + 1` | `4n` |
| one element eliding `n` levels | 2 | `4n` |
| `n` sparse designators in a 2^20-element static array, `n` = 16, 64 | `n + 1` | `4n + 4` |
| `n` pairs of overlapping ranges, one over all 2^20 elements | `4n + 1` | `8n + 4` |

Plan construction never depends on an array's extent or a range's length;
emission scales with the stores and bytes it produces. The
`-Winitializer-overrides` recorder keeps its separate 65,536-element range
cap.

## Peak memory for local-heavy files (issue #92)

M8 as first merged raised peak RSS on the M7 review's "N functions × 50
initialized locals" generator by about 15%. The cause was not the retained
plans but the checker's working storage: the analyzer's scratch arena lives
for the whole translation unit, and `check_initializer` built its work stack,
override recorder, leaf cache and, new in M8, the plan builder's five vectors
afresh for every initializer. The plan builder's vectors and the work stack,
whose entries grew to carry plan sites, left about 780 bytes of dead scratch
per `int v = e;`. The analyzer now keeps one set of checker buffers and clears
it per initializer; plans are copied to the translation-unit arena as before.

Peak RSS (`ru_maxrss`, debug builds through the swarm wrapper, Linux,
2-3 runs each, spread under 1%):

| Input | M6 | main before M8 (c74dfb0c) | M8 producer alone (2bc00d11) | main (c1f72977) | reused buffers |
| --- | ---: | ---: | ---: | ---: | ---: |
| 2k × 50 locals | 526 MiB | 529 MiB | 603 MiB | 605 MiB | 535 MiB (+1.7%) |
| 4k × 50 locals, `--emit=ir` | 1025 MiB | 1030 MiB | | 1181 MiB | 1042 MiB (+1.6%) |
| 8k × 50 locals | 2022 MiB | 2031 MiB | | 2336 MiB | 2055 MiB (+1.6%) |

Percentages are against M6. The rest of the difference is the retained plans
themselves: one `Store`, one value and a map entry per local. The library
test `scalar_initializers_reuse_the_checker_storage` bounds the scratch bytes
each further initialized local costs at 1,800; it measures 1,539 with the
buffers reused and 2,275 at c1f72977.

## Final checks

Run through the shared swarm wrapper on Linux at the final tree (the M8
commits plus this note and the measurement test); logs are in the swarm
scratch directory, not retained in the repository.

| Command | Result |
| --- | --- |
| `cargo +nightly fmt --check` | PASS |
| `cargo clippy --all-targets -- -D warnings` | PASS |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | PASS |
| `cargo test --all-targets --no-fail-fast` | PASS: 2,368 passed, 0 failed, 8 ignored across 30 binaries; the library phase passes 2,149 with 5 ignored (the new ignored test is the plan-size measurement) |
| `cargo test --features benchmarking-internals --test allocation_count` | PASS: 31 tests |
| `git diff --check` | PASS |

`BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` was not run separately
(its default cases ran in the library phase): the generator emits bcc IR, not
C, so it never reaches initializer lowering.

Line counts (`git diff --numstat 94212cd9..HEAD -- src`):

| Category | Added | Deleted | Files |
| --- | ---: | ---: | ---: |
| Compiler | 1,473 | 867 | 11 |
| Tests | 435 | 1 | 3 |

The compiler change exceeds the 800-1,400 estimate (2,340 lines touched): the
plan vocabulary and its documentation (186 lines), the builder (502) and the
emission replay (278) are new, and `check_initializer` grew helpers to stay
under Clippy's function-length limit; net growth is 606 lines.

## Residual items

- No matched GCC execute survey was run for M8 (the design asks for one at
  M8). The scratch comparison above lowered the torture sources without
  executing them.
- `declare_binding` keeps its scalar fast path; routing it through the plan
  would remove one representation check but add a plan lookup per scalar
  local.
- Static emission still maps `Store` values through `static_scalar` and
  `address_constant`; retained destination constants and symbolic addresses
  are M9.
- Plans are retained for every accepted initializer, including compound
  literals in unevaluated operands, which lowering never reads.
