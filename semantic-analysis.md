# Semantic analysis roadmap

This document records the semantic analysis phase: its retained interfaces,
implementation choices, validation evidence and staged roadmap. It is a staged front end, not a C99 conformance claim.
The syntax boundary remains the one in [parser-roadmap.md](parser-roadmap.md).

## Purpose and phase placement

C99 translation phase 7 (§5.1.1.2p1, printed p. 10, PDF p. 22) includes both
syntax and semantics. The batch pipeline first completes preprocessing and
parsing. `pipeline::analyze_translation_unit` then consumes the immutable
`ParsedTranslationUnit`. The parser retains its own typedef-name classification;
semantic lookup is independent, with no feedback to parser frames.

The default CLI and the measured diagnostic adapter invoke semantic analysis.
`--semantic-types` reports declarations in deterministic lexical traversal order,
with a nominal-tag declaration table followed by ordinary binding occurrences,
C-like abstract type spelling, scope, binding category, linkage, duration,
and available size/alignment. `--tokens`, `--syntax-tree` and `--raw-syntax` stop
at their existing phases and retain their previous output contracts.

Diagnostics enter the existing pending FIFO after preprocessing and parser
errors. The existing reporter continues ordering parser source runs and folding
errors at identical provenance. Semantic errors retain discovery order (including
nested specifiers before their containing declaration), appended after parser
errors. Their primary labels point to syntax provenance, names come from source
spellings, and redeclaration errors include a previous-declaration label. They
use `SemanticErrorKind` and `ToDiagnostic`, with C99 notes, rather than strings
as error identities. No new rendering/normalization path exists.

## Storage and traversal

The translation-unit arena (`'tu`) retains canonical type nodes, nominal tag
identities and completion cells, record members with byte/bit offsets, binding
occurrences, semantic scope identities, resolved type names, and parameter
metadata (including array `static` minimums), and tag declaration occurrences. A backend can retain this graph
without retaining a semantic analyzer. The syntax tree remains separately available.

The semantic working arena (`'s`) owns continuations, value stacks, interning
lookup, visible-binding lookup, linkage lookup and scope restoration lists. It
is dropped before returning `SemanticTranslationUnit<'tu>`. Retained values have
no destruction requirements. Collections use the repository's arena-backed
`ArenaMap`/`ArenaVec`; no compiler global allocator collections are introduced.

One explicit `Work` stack composes specifier resolution, declarator construction,
parameter lists, nested record/enum definitions, integer evaluation, initializers
and statement discovery. Parenthesized declarators never recurse in Rust.
Expression discovery/evaluation and scope traversal are also iterative.
Compatibility/composite construction and type inspection have their own explicit
postorder stacks. Parameter/member lists accumulate in scratch cons cells, then
copy once into retained storage. Ordinary/tag lookup is expected O(1) per lookup;
scope exit costs the number of introduced entries.

The intended cost is linear in visited syntax plus retained output, with these
explicit exceptions: compatibility/composite checking can revisit shared typedef
graphs on repeated redeclarations; layout queries strip shared array derivations;
qualified array typedef construction traverses those derivations. These operations
are linear in the queried type graph, not necessarily just newly written syntax.
Conditional integer models and ICE operand eligibility are cached by syntax identity; deep conditional chains do not repeatedly scan their tails. No native-stack depth limit is imposed by semantic traversal.

## Type graph and target data model

`TypeId` pairs an unqualified interned node index with a qualifier bitset. Scalar,
pointer, array and function shapes are hash-consed using immediate child identities;
hashing never recursively descends a type. Distinct tagged types are nominal even
when their layouts match. Tag completion updates only retained member/layout cells,
so pointers formed before a record definition keep the same identity.

The graph represents all C99 scalar types including the three complex types,
qualified pointers, constant/incomplete/variable/prototype-star arrays, prototype
and unprototyped functions, ellipsis, and nominal struct/union/enum types. Function
parameter identities use adjusted, top-level-unqualified types; declared parameter
bindings retain their qualifiers. Array typedef qualification reaches its element
type (§6.7.3p8). Function typedef qualifiers warn about the C99 undefined behavior and are ignored; pointer qualifiers remain meaningful. Prototype parameter/tag scope is separate from file scope; a
function definition installs its parameter bindings and parameter-list tags and
enumerators in the outer body scope. `ScopeKind::Function` names that semantic
owner: ordinary names there have C99 block scope, while the distinct label
namespace has function scope and awaits Stage 3. Each aggregate has a separate member namespace.
Labels are reserved to the function statement stage and are not yet resolved.

`src/target.rs` is the target seam. `TargetLayout::LP64` describes x86-64 System V:
8-bit bytes, signed plain char, 1-byte bool/char, 2-byte short, 4-byte int/float,
8-byte long/long-long/double/pointer, 16-byte/16-aligned long double; complex sizes
are twice the real size with real alignment. `size_t` is unsigned long,
`ptrdiff_t` is long, and `wchar_t` is int. Integer token candidate selection now
consults this layout; phase-4 intmax/uintmax evaluation retains its established
64-bit model. The parser-facing packed literal variants are still the LP64 carriers.
This is a target description, not an assertion about Windows host `long`.
No LLP64 behavior or target-selection CLI is added.

Aggregate layout uses natural member alignment, tail padding, union maximum size,
and final flexible-array alignment with no elements included in the size. Bit-fields
allocate low-order bits first, may share bytes across differing integer base types,
and do not cross the allocation boundary of their declared base type. An unnamed
zero-width field advances to its base-type boundary without increasing aggregate
alignment. Plain-int bit-fields are signed; integer bit-field types beyond the
C99-required int/unsigned-int/bool set are accepted as an implementation-defined
choice following the target ABI. Packed/aligned/vendor attribute meaning is not
modeled; affected record layouts become unavailable instead of fabricated.
`tests/fixtures/semantic/layout-probe.c` uses Clang's Linux target static assertions
for scalar sizes, mixed-base bit-fields, zero-width fields, nested records, unions,
member offsets and flexible arrays. Clang accepted that probe.

Compatibility handles qualifiers, pointer targets, constant vs incomplete array
extents and prototype parameter types/ellipsis, and constructs composite derived
types iteratively. Unprototyped declarations impose default-promotion stability
when compared to prototypes; old-style definition parameter types are promoted
and checked against visible prototypes. This implementation selects int as the compatible
integer type for C99 enums, preserving the nominal enum identity when forming an enum/int composite. Nominal identities distinguish independent enum tags.

## Binding, scope, linkage and recovery

Ordinary identifiers and tags have separate visible maps. An entry remembers its
previous visible binding; each scope owns an undo list. File, function, block,
prototype and implicit selection/iteration scopes are independent of parser scopes.
Bindings retain source occurrences rather than replacing earlier declarations.
Linked redeclarations form composite types, with a separate translation-unit
linkage map covering extern declarations across disjoint lexical scopes.

Extern inherits a visible internal/external linkage (§6.2.2p4); a visible automatic
object does not confer linkage. File static is internal; other file objects and
functions are external unless inheriting prior function linkage. Internal/external
mixes are diagnosed (§6.2.2p7). Typedefs, enumerators, members and automatic objects
have no linkage. Objects at file scope, with linkage, or block static have static
duration; parameters/other local objects have automatic duration. Functions and
non-object bindings have no object duration. Same-scope no-linkage duplicates are
rejected; identical repeated typedefs are accepted in C11 and later modes and GNU modes.

Recovered declarations/functions retain useful bindings/types but suppress new
semantic diagnostics while their recovered subtree is analyzed. Error roots and
missing expression slots are skipped. Tag redefinition/kind errors retain the
original tag rather than attempting a second completion. Later valid declarations
continue. This favors avoiding cascades over diagnosing every constraint inside
repaired syntax; narrower recovery taint is a possible future refinement.

## Staged roadmap

### Stage 1: Types and declarations

Implemented foundations include the type graph, explicit LP64 layout, tag
forward declaration/completion, enums/bit-fields/constant bounds, typedef expansion,
declarator/parameter adjustment, declaration/storage/qualifier/inline constraints,
ordinary/tag scopes, linkage/duration and redeclaration composites, iterative body
declaration discovery and CLI inspection. Syntax specifier-combination validation
remains parser-owned because the parser already preserves a validated multiset.

The integer evaluator implements integer/character constants and enumerators,
unary integer operators, integer binary operators, short-circuit logical and
conditional evaluation, integer casts and sizeof/alignof of modeled type names.
It carries widths/signedness, promotes small integers, diagnoses exceptional/overflow
operations and constrains C99 enumerators to int. Positive constant array bounds
use the same evaluator; runtime bounds at file scope/with linkage are rejected.

Old-style definition parameter declarations are adjusted and promoted before
comparison with visible prototypes, and their completed signatures survive later
unprototyped declarations. Conditional ICE arms use their common integer type
without evaluating an unselected arm. Immediate floating-to-integer casts support
float, double and native long double, including parenthesized immediate constants.
Signed overflow and exceptional evaluation produce one primary diagnostic before
dependent array/enumerator checks. Signed right shift is arithmetic; narrowing
integer casts use two's-complement truncation, following the selected target.

Conservative boundaries and later work:

- sizeof/alignof expression operands are not typed yet; only type-name operands
  contribute evaluated ICE values in this stage. This follows the requested
  Stage-1 subset; expression typing belongs to Stage 2.
- Extension-derived and inferred types, fixed-underlying enums, decimal/bit-precise
  types and attribute/calling-convention effects are unanalyzed. Attribute-bearing
  declaration/pointer/declarator types and affected aggregate layouts become
  unavailable. C23-specific redeclaration/value rules beyond repeated typedefs
  are not fully modeled.
- Initializer conversions, array extent inference from an initializer, and object
  completeness after an initializer await Stage 2. Parameter completeness in a
  function definition, named-parameter requirements and identifier-list definition
  constraints belong to the function-definition checks of Stage 3.
- Label bindings/constraints, for-init storage constraints, inline body restrictions
  and translation-unit completion of tentative definitions await Stage 3.

Unanalyzed types suppress dependent compatibility/layout diagnostics. Unsupported
constant-expression forms similarly suppress dependent enum/VLA diagnostics rather
than calling a valid unsupported constant a runtime bound. This is deliberately
conservative, and is not evidence that those expressions have been validated.

GNU size/alignment queries on void and function types use the one-byte GCC convention; strict ISO modes retain the constraint diagnostics. Later-standard assertion, generic-selection and count operands are walked structurally without evaluating their additional semantic rules.

### Stage 2: Expressions and initializers

Complete typed expression results, lvalues, conversions (§6.3), all operator
constraints (§6.5), integer constant expressions (§6.6) and initialization/current-object traversal
(§6.7.8). Use the existing work stack rather than recursive visitors. Preserve
canonical types needed for backend lowering, while dropping expression scratch.

### Stage 3: Statements and functions

Implement statement constraints (§6.8), return checking, function label namespace,
goto targets, switch/case/default bookkeeping, external definitions and tentative
definitions (§6.9). Complete old-style definition constraints and inline-body restrictions with the
full expression/binding model.

## Validation

Unit tests cover type interning, compatibility and composite types, layout,
integer constant evaluation, scopes, linkage, and tags. Deep-input tests check
that traversal stays iterative: 100,000 pointer derivations, 100,000
compatibility derivations, and 10,000 nested blocks. Layout expectations were
checked against Clang `_Static_assert`s for `x86_64-unknown-linux-gnu`. Each
semantic diagnostic has a rendered golden fixture, and `tests/allocation_count.rs`
checks that declaration analysis allocates only from arenas.

The `Semantic analysis` Criterion group runs phases 1-7 plus declaration
analysis over the same inputs as `Parser` and `Parser only`; the difference
from `Parser` is the cost of analysis.

Integer constant evaluation holds values in `i128` but never divides two
`i128`s. Under the configured linker-plugin fat LTO, `i128` division lowers to
`__divti3`/`__modti3`, which `ld.lld` fetches from `compiler_builtins` only
after LTO has discarded the `rust_eh_personality` that their unwind tables
reference, and the link fails. Operands are at most 64 bits wide, so division
uses `u64` magnitudes.

### Diagnostic survey and triage

All 14 `.c` files in `test-programs/` completed without crashes or timeouts.
Comparing default compilation with syntax-only inspection found zero new semantic
errors; the 24 existing preprocessing/syntax errors were unchanged. This small,
mostly preprocessor-oriented set is not sufficient evidence of C99 conformance.

The 21 new semantic diagnostic fixtures produce 30 semantic errors and one
function-qualifier warning. Linux Clang
with `-std=c99 -pedantic-errors` also rejects every fixture. Sample triage:

| Input | Classification and independent check |
| --- | --- |
| `sema-duplicate.c` | Genuine: repeated automatic declaration and automatic followed by same-scope extern; Clang rejects both. |
| `sema-enum-range.c` | Genuine C99 int representability constraint; Clang identifies the incremented value as requiring a C23 extension. |
| `sema-member.c` | Genuine: void member and nesting a flexible-array structure; Clang rejects both. |
| `sema-overflow.c` | Genuine C99 ICE constraint; Clang accepts folding only as a GNU extension and rejects it in strict mode. |
| GNU literal zero array in existing policy fixture | Initial sema rejection was a false positive; fixed to use shared parser/extension policy without duplicate diagnostics. Computed zero uses the same policy. |
| `sema-function-qualifier.c` | Genuine undefined behavior under C99; emitted as a warning, while strict Clang promotes its ignored-qualifier extension warning to an error. |
| Old-style prototype and body parameter types | Positive promoted short-to-int and float-to-double examples accepted by Clang and retained as regressions. |

The GCC torture corpus survey (`scripts/run_gcc_torture.py`) has not yet been
run with semantic analysis enabled; the 12 retained torture regressions pass.

The defensive `UnknownTypedef` kind cannot be reached by deliberately well-formed
source through the parser's typedef classification. Its constructed resolver unit
test replaces a source golden; the other 21 kinds have rendered source goldens.
