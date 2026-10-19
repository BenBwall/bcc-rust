# M7 retained entity, symbol and function policy

Base: `codex/semantic-facts-m7` at `94212cd9`, based on M6's
`claude/semantic-facts-m6`. This implements migration row M7 of
[the design](../design/semantic-facts.md). Attribute validation, DLL storage,
Microsoft inline COMDAT policy, semantic overload identities and emitted ABI
names were already present at the base; this step preserves their producers.
The retained/legacy comparison is `9055bc01`; the consumer switch is
`ec0b9496`. Measurements below describe the latter revision.

## Result and choices

`semantic_analysis/facts/entities.rs` completes one retained record per C
entity after definitions, attributes and ABI names are final. Each record keeps
its linked binding and occurrence IDs, final type, definition and initializer
association, known function definition, symbol request and final body policy.
A separate retained `FunctionPolicy` keeps body permission and end policy per
function occurrence: a GNU extern-inline body and its replacement definition
share an entity but must not share an emission decision. Point-of-use types
remain on bindings; an open call never takes its type from the final entity
type. Initializer placement remains M8's responsibility.

Assembler labels are decoded and inherited once by semantic completion. A
request keeps its decoded name and source; invalid, conflicting, empty,
non-narrow, non-UTF-8, NUL-bearing and reserved-prefix labels keep their previous
lowering capability gate and provenance. An automatic register label also keeps
that refusal. Linked declarations share a request by semantic entity identity,
while static locals own theirs. Semantic completion does not merge entities
merely because they request the same IR symbol.

Lowering still reserves user names before generating internal storage, joins
requested IR symbols for reachability and completed object layouts, checks
colliding definitions and refuses aliases involving assembler labels. These are
symbol capability decisions. Private `always_inline` bodies continue to use
semantic entity identity, so glibc `__REDIRECT` aliases call the external symbol.
The Windows DLL and COMDAT rules continue to read their existing entity
attributes; neither name propagation nor finalization changes their state.

`FunctionEnd::MainZero` preserves the old C-identifier-only rule, including
assembler renaming and private inline copies. No new hosted or language-mode
condition is introduced. The record stores body permission; lowering still
chooses module reachability and allocates the private body's unique IR name.

Convention validation additionally retains the effective convention of each
written request (System V, Microsoft or unsupported), its provenance and its
owning function, including abstract function-pointer types. Ignored x86
conventions resolve to the target default. Unsupported requests keep their
existing refusal records, stage, ordering and text. Canonical C type identity is
unchanged for ignored/default requests; representing a supported nondefault
ABI is outside this migration.

The effective returns-twice contract combines the existing semantic attribute
with cataloged assembler requests. Normal declarations consume that retained
contract; private inline bodies preserve the previous attribute-only contract.
The same phase-neutral external catalog now exposes canonical library
signatures from builtin descriptors, including fortified entry points. Host
validation projects those signatures to IR scalar types without importing
semantic types. Runtime signature validation of textual IR remains independent
of semantic analysis.

Whole-object readonly storage and volatile access use canonical semantic type
predicates. Readonly storage remains distinct from the modifiable-lvalue test
for a record containing const members. VM classification is retained from type
interning, and natural member/value layout queries already belong to `Types`.

Deleted lowering reconstruction:

- assembler-label token decoding and linked/local name maps;
- the function-definition name set and open-external reconstruction;
- rebuilding definition and private-inline policy maps in `Unit::new`;
- the per-unit binding-to-initializer join;
- name-based closing-brace policy and the local readonly classifier;
- assembler-label returns-twice name classification.

Lowering attribute-token parsing had already been removed before M7. Host core,
fortified and math signature tables are replaced by shared descriptor queries;
math execution dispatch and runtime validation remain downstream.

## Coverage and validation

Before switching consumers, the shared lowering test helper compared the new
records against the unchanged lowerer for every lowerable fixture it exercised:
requested names and semantic identity, label refusals and provenance, open
externals, definition association, linked occurrences, private-inline permission,
readonly storage, returns-twice and function-end policy. A permanent policy
snapshot pins linked redeclarations, asm aliases, completed arrays, static locals,
asm-renamed `main`, inline bodies, overload ABI names and readonly objects on all
four targets. Transitional comparisons were removed with the old consumers.

The comparison caught the GNU inline replacement case: using only the final
entity definition would emit the earlier inline body. Policy is therefore
retained per body, with the entity separately recording its final definition.
This is covered by the existing replacement-body test and the policy snapshot.

The snapshot covers 14 sources on four targets (56 translation units).
Permanent tests additionally check convention resolution/refusal ownership,
readonly/volatile/VM predicates, explicit IR for asm-renamed `main`, and
provenance-bearing `MissingFact` for removed entity and function policy. The
initializer corruption test now removes the canonical entity association.
No missing required record falls back to a lowerer reconstruction.

Before deletion, host comparisons checked every core/fortified table entry
and compared the descriptor on exercised math paths. The focused comparison
runs passed 182 lowering tests (one ignored) and 89 interpreter tests. The
final full suite additionally covers the real-math family. Existing malformed
textual-IR tests retain runtime signature-mismatch checks before host effects;
returns-twice optimizer-wrapper and native frame exclusions still pass.

## Measurements

The 64-bit policy snapshot measures `size_of`, rather than inferring layouts:

| Record | Bytes |
| --- | ---: |
| `Entity` | 136 |
| `FunctionPolicy` | 2 |
| `SymbolRequest` | 24 |
| `ConventionFacts` | 32 |

Entity membership adds one `usize` per binding and occurrence, function policy
is sparse by body occurrence, and VM classification adds one byte per canonical
type. These are payload sizes, excluding map capacity and arena growth waste.
The vendor-declarator and owner arrays and lowerer's definition, known-function,
private-inline and symbol-name maps are gone. Finalization makes linear passes
over bindings, occurrences, definitions, requests and function bodies; it does
not revisit syntax trees or scan a TU per function.

No retained-arena peak, process working set or controlled timing comparison was
measured. The allocation gate checks global allocation calls, not semantic
residency or performance. No matched GCC torture affected-source subset was
run: this work has no prepared baseline executable and corpus. The design
reserves full surveys for M1/M6/M8/M10; M7 still lacks its requested targeted
survey evidence. Existing IR and diagnostic goldens, the independent old/new
comparisons and execution suites provide equivalence evidence, but do not replace that survey.

Source line counts (`git diff --numstat 94212cd9..ec0b9496`, excluding documentation):

| Category | Added | Deleted | Files |
| --- | ---: | ---: | ---: |
| Compiler, including colocated tests | 854 | 480 | 22 |
| Dedicated tests | 276 | 3 | 3 |
| Policy snapshot | 273 | 0 | 1 |

The handwritten change exceeds the 400–700 estimate: completing linked
membership and initializer associations and retaining per-body policy add a
399-line producer, alongside the convention/object contract and host projection.
The transitional comparison was committed separately, then deleted with the
legacy consumers. Counts above describe the final net change, not that temporary
instrumentation.

## Final checks

All build/test/Clippy commands ran through the required shared swarm wrapper
on Linux with pinned Clang/LLD 23.1.1. Logs live under
`/c/Users/benbw/Documents/GitRepo/bcc-rust/.git/overnight-20261019/scratch/m7/`.

| Command | Result | Log |
| --- | --- | --- |
| `cargo test --all-targets --no-fail-fast` | PASS: 2,366 passed, 0 failed, 7 ignored across 30 binaries; library: 2,147 passed, 4 ignored | `gate-all-1.log` |
| `cargo test --features benchmarking-internals --test allocation_count` | PASS: 31 passed, 0 failed, 0 ignored | `gate-allocation-1.log` |
| `cargo clippy --all-targets -- -D warnings` | PASS | `gate-clippy-2.log` |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | PASS | `gate-clippy-feature-1.log` |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | PASS: 10 passed, 0 failed, 3 ignored | `gate-difftest-1.log` |
| `cargo +nightly fmt --check` | PASS | `gate-fmt-final.log` |
| `git diff --check` | PASS | `gate-diff-check.log` |

The switched focused suites pass 184 lowering tests (one ignored) and 89
interpreter tests. The initial ordinary Clippy gate rejected four distinct
lints (`doc_markdown`, `match_same_arms`, test `disallowed_types`, and
`assert_is_empty`); they were corrected before the final checks. No allocation
abort occurred. Existing goldens were unchanged; only the new policy snapshot
was written during the comparison stage. Windows DLL import/export interop,
Microsoft inline coalescing, private-inline redirection and overload mangling
regressions pass.

## Residual items for M8–M10

- M8 produces initializer placement plans; M7 only retains each entity's
  defining initializer association. The initializer walker is unchanged.
- M9 retains converted symbolic addresses and remaining operation facts.
  Address-constant decomposition and scalar instruction/ABI projection remain
  in lowering; target and natural aggregate layout helpers remain shared.
- M10 can remove raw parsed-unit roots once remaining initializer/constant
  consumers migrate, tighten traversal visibility and add the architecture
  checks and current-contract documentation from the design's guardrails.
- Family builtin instances and unsupported x86/vector lowering remain as M6
  recorded. Supporting nondefault conventions or aliases involving asm labels
  is a separately evidenced backend capability change.
