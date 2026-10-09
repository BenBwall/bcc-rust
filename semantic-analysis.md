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
`--semantic-types` uses the `--syntax-tree` conventions: one item per line,
lowercase `key=value` fields with inapplicable ones omitted, quoted values
other than plain words, and two-space indentation (a `[depth=N]` prefix past
32 levels). A `target` line precedes the scope tree, printed in creation
order with each scope nested under its parent; prototype scopes are omitted.
Within a scope, tag declarations come first (with members, bit offsets and
widths, and the names anonymous members contribute, at the tag's last
declaration; enums show their compatible type), then ordinary binding
occurrences in lexical order with C-like abstract type spelling, linkage,
object duration, parameter `static` minimums and available size/alignment.
Resolved type names, typed expressions and contextual conversions follow.
Expression ordinals follow deterministic child-before-parent traversal; no host
address is printed, and source locations are not yet shown. `--tokens`,
`--syntax-tree` and `--raw-syntax` stop
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
identities and completion cells, record members with byte/bit offsets and per-record member-name tables, binding
occurrences, semantic scope identities, resolved type names, and parameter
metadata (including array `static` minimums), tag declaration occurrences, typed
expression results and conversion records. A backend can retain this graph
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
namespace has function scope; GNU local labels additionally have lexical scope.
Each aggregate has a separate member namespace.
An anonymous structure or union member (C11 §6.7.2.1p13; an extension in C99
and GNU modes, and with MSVC anonymous structures also a tagged or typedef
record) is retained as an unnamed `Member` marked anonymous; its names join the
containing namespace, where duplicates are diagnosed, and initialization treats
it as a subobject.
Labels are reserved to the function statement stage and are not yet resolved.

`src/target.rs` is the target seam, selected through
`CompilerConfiguration::with_target` and the CLI's `--target`. The target flows
through the shared context to the preprocessor and type interner; no global
state or host `long` representation selects C types. The default GNU Linux
inspection label remains `x86_64-sysv-lp64` for snapshot compatibility.

| Triple | Model / long | long double size / align / precision | wchar / wint | size / ptrdiff / intmax | va_list |
| --- | --- | --- | --- | --- | --- |
| x86_64-unknown-linux-gnu | LP64 / 8 | 16 / 16 / x87 64-bit significand | int / unsigned int | unsigned long / long / long | one 24-byte, 8-aligned SysV record array |
| x86_64-unknown-linux-musl | LP64 / 8 | 16 / 16 / x87 64-bit significand | int / unsigned int | unsigned long / long / long | same SysV array |
| x86_64-w64-windows-gnu | LLP64 / 4 | 16 / 16 / x87 64-bit significand | unsigned short / unsigned short | unsigned long long / long long / long long | char pointer |
| x86_64-pc-windows-msvc | LLP64 / 4 | 8 / 8 / binary64 | unsigned short / unsigned short | unsigned long long / long long / long long | char pointer |

All four have 8-bit signed plain char, 1-byte bool/char, 2-byte short, 4-byte
int/float, 8-byte long-long/double/pointer, and natural scalar alignment.
Complex sizes are twice the real size, with real alignment. `int64_t` is long
on Linux and long long on Windows; the frozen macros also select exact/least/
fast unsigned aliases, intptr, uintmax and sig_atomic types. The embedded
`stdint.h` follows Clang's **freestanding** typedef choices; a hosted library
may choose different fast aliases in its own header (for example glibc).

Integer candidate selection and both semantic integer-evaluation paths use
target widths while keeping fundamental ranks distinct. Packed literal storage
can hold 64-bit values even for a 32-bit target long. Phase-4 intmax arithmetic
is 64-bit on every supported target. L-prefixed strings have int elements and
UTF-32 execution units on Linux, unsigned-short elements and UTF-16 execution
units on Windows. Source literal units remain intact for inspection;
`Context::wide_literal_units` supplies execution units, including surrogate
pairs, and semantic string bounds count those units. Linux numeric wide
character escapes are sign-extended from wchar_t before intmax preprocessing
arithmetic and semantic evaluation. The legacy nonnegative narrow-byte `#if`
choice is retained (C99 6.10.1p4 makes this implementation-defined). Non-BMP wide character
constants are rejected on Windows with a structured C99 §6.4.4.4p11 diagnostic.
MSVC long-double literals use binary64 conversion and constant operations round
to binary64; the existing native x87 bridge still carries values internally. Extended
operations temporarily select 64-bit x87 precision and restore the caller
control word, so Windows worker threads do not silently compute at 53 bits.
MSVC enum types and enumerators use 32-bit int, including truncation of wider
values. MinGW widens enum compatible types to long long; Linux retains the
existing long choices.
Windows stdarg operands are modifiable pointer lvalues (including va-copy's
source, matching Clang); SysV array decay retains existing behavior.

System V aggregate layout uses natural member alignment, tail padding, union maximum size,
and final flexible-array alignment with no elements included in the size. Bit-fields
allocate low-order bits first, may share bytes across differing integer base types,
and do not cross the allocation boundary of their declared base type. Unnamed
bit-fields never increase aggregate alignment: an unnamed zero-width field
advances to its base-type boundary, and in a union an unnamed bit-field occupies
only the bytes its width needs. Plain-int bit-fields are signed; integer bit-field types beyond the
C99-required int/unsigned-int/bool set are accepted as an implementation-defined
choice following the target ABI. GCC's flexible-array forms (in a union, as a
structure's only member, a flexible structure nested in a structure, and arrays
of flexible structures) keep their layout and are reported through the
`FlexibleArrayExtensions` policy. Packed/aligned/vendor attribute meaning is not
modeled; affected record layouts become unavailable instead of fabricated.
Windows records use Microsoft's bit-field layout on both MinGW and MSVC,
matching Clang's MinGW default `-mms-bitfields`. Consecutive nonzero fields share
an entire reserved storage unit only when their base-type sizes match and they
fit. A zero-width field affects alignment only immediately after a nonzero
bit-field. Union bit-fields occupy a full unit without increasing alignment;
zero-width union fields occupy no bytes. The C11 probe in
`tests/fixtures/targets/conformance.c` covers every scalar, ABI aliases, offsets,
mixed-type fields, exhausted units, zero-width fields, compact byte-field
records, and unions against all four Clang targets. Rust tests additionally
verify bit offsets, typedef identities, literal types and UTF-16 surrogate
values directly. `_Generic` remains syntax-only in sema: the C probe includes
Clang-checked `_Generic` assertions, and **direct Rust type assertions** supply
bcc's type evidence rather than claiming unmodeled generic assertions passed.
No packed/aligned attribute or pragma-pack semantics are added.

`tests/fixtures/semantic/layout-probe.c` uses Clang's Linux target static assertions
for scalar sizes, mixed-base bit-fields, zero-width and unnamed bit-fields, nested records, unions,
member offsets and flexible arrays. Clang accepted that probe.

Compatibility handles qualifiers, pointer targets, constant vs incomplete array
extents and prototype parameter types/ellipsis, and constructs composite derived
types iteratively. Unprototyped declarations impose default-promotion stability
when compared to prototypes; old-style definition parameter types are promoted
and checked against visible prototypes. An enumeration's compatible integer type
is implementation-defined (§6.7.2.2p4); this implementation follows GCC and Clang
on x86-64 System V: unsigned int when no member is negative, otherwise int, each
widening to the 64-bit type of the same signedness when a member needs it. The
tag records that type, which fixes its layout, promotion and integer conversions;
an enum/integer composite with exactly that type keeps the nominal enum identity.
Nominal identities distinguish independent enum tags. Enumeration constants
representable as int have type int; wider values are accepted as the C23
extension that GCC and Clang provide, take the type of their value, and are
reported through the extension policy. A set of members that no 64-bit type
holds is diagnosed.

## Binding, scope, linkage and recovery

Ordinary identifiers and tags have separate visible maps. An entry remembers its
previous visible binding; each scope owns an undo list. File, function, block,
prototype and implicit selection/iteration scopes are independent of parser scopes.
Bindings retain source occurrences rather than replacing earlier declarations.
Linked redeclarations form composite types, with a separate translation-unit
linkage map covering extern declarations across disjoint lexical scopes. A
redeclaration whose type is unanalyzed, such as GNU `__typeof__(f) f`, takes
the visible function's kind and keeps its type instead of conflicting with it.

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
missing expression slots are skipped. A nested list for a tag whose own list is
still open is a redefinition. A rejected definition (redefinition or kind
conflict) keeps the original tag's single completion and walks its list against
a fresh, uninstalled, unanalyzed tag, so the tags and enumerators it declares
remain visible and later uses do not cascade. Later valid declarations
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
operations and reports enumerators outside int through the extension policy.
Positive constant array bounds
use the same evaluator; runtime bounds at file scope/with linkage are rejected,
as are variably modified members (§6.7.2.1p8, §6.7.5.2p2) and `[*]` outside a
prototype that is not a definition (§6.7.5.2p4). Arrays and records larger than
PTRDIFF_MAX bytes exceed this implementation's object-size limit and are rejected
without dropping members.

Old-style definition parameter declarations are adjusted and promoted before
comparison with visible prototypes, and their completed signatures survive later
unprototyped declarations. Conditional ICE arms use their common integer type
without evaluating an unselected arm. Immediate floating-to-integer casts support
float, double and native long double, including parenthesized immediate constants.
Where an integer constant expression is required (enumerators, bit-field widths,
case labels, static assertions), signed overflow and exceptional evaluation
produce one primary diagnostic before dependent checks. An array bound with such
an operation is not an integer constant expression, so it declares a variable
length array (§6.7.5.2p4); file-scope, linked and static arrays then report the
variably modified type. As in GCC, a left shift of a nonnegative signed value
into (not past) the sign bit folds to its two's-complement result and is reported
through the `SignBitShifts` extension policy. Signed right shift is arithmetic;
narrowing integer casts use two's-complement truncation, following the selected
target.

Stage-1 boundaries, with their current disposition:

- Stage 2 types sizeof/alignof expression operands and evaluates their modeled
  constant sizes. VLA sizes remain runtime expressions.
- Extension-derived and inferred types, fixed-underlying enums, decimal/bit-precise
  types and layout-changing extensions are unanalyzed. MSVC `__int8`,
  `__int16`, `__int32` and `__int64` are not: as in Clang, they are `char`,
  `short`, `int` and `long long`, and `signed` or `unsigned` selects that
  variant. Only alignment
  specifiers, thread/constexpr storage, MSVC pointer-size modifiers and
  attributes naming `aligned`, `align`, `packed`, `mode`, `vector_size` or
  `ext_vector_type` make that one declaration's type unavailable; other
  attributes and the calling conventions that x86-64 ignores leave it intact. A
  tag's layout becomes unavailable only when its own definition or one of its
  members carries such an extension, never from a use site. C23-specific redeclaration/value rules beyond repeated typedefs
  are not fully modeled.
- Stage 2 implements initializer conversions, inferred array extents, and object
  completeness after an initializer. Parameter completeness in a
  function definition, named-parameter requirements and identifier-list definition
  constraints are now checked by Stage 3.
- Stage 3 implements label constraints, for-init declaration constraints, inline
  body restrictions and translation-unit completion of tentative definitions.

Unanalyzed types suppress dependent compatibility/layout diagnostics, including
member access through `.` or through a pointer to an unanalyzed type. Unsupported
constant-expression forms similarly suppress dependent enum/VLA diagnostics rather
than calling a valid unsupported constant a runtime bound. This is deliberately
conservative, and is not evidence that those expressions have been validated.

GNU size/alignment queries on void and function types use the one-byte GCC convention;
strict ISO modes retain the constraint diagnostics. Stage 2 evaluates supported
static assertions. Generic-selection and count operands remain structurally walked.

### Stage 2: Expressions and initializers

#### Retained expressions and conversions

`SemanticTranslationUnit.expressions` is an arena vector of `ExpressionInfo`
records. Each record borrows its immutable syntax expression and retains its type,
value category, resolved binding where applicable, bit-field width, address
eligibility, integer/floating constant value and constant-expression class.
The analyzer's scratch map from syntax identity to vector index gives expected
O(1) operand lookup. The retained vector is preferable to a retained hash map:
it gives deterministic inspection and a compact backend iteration surface without
preserving host-address hashing or scratch capacity. A backend needing random
lookup can build an arena index from the retained expression references.

Value categories distinguish lvalues, their modifiable subset, function
designators and rvalues (§6.3.2.1p1, printed p. 46, PDF p. 58). Const aggregate
members, including array elements, prevent modification; bit-fields retain enough
information for promotion and address/sizeof constraints. Parentheses preserve
the underlying category. Ordinary identifiers resolve through the declaration
binding model. A function's `__func__` is modeled as a static const character array.
Calls to undeclared names create an unprototyped int-returning function only in
C89/C95 and GNU modes, using shared removed-feature policy. Strict C99 and later
diagnose undeclared identifiers. Undeclared reserved `__builtin_*` names are GCC
builtins in every mode: they are bound as opaque functions without a
diagnostic, and calls to them have unknown results, except that
`__builtin_constant_p` folds to whether its operand is an arithmetic constant.

In GNU modes, as in GCC, an arithmetic constant that is not an integer constant
expression (§6.6p6) is folded where an integer constant expression is required
and reported through the `ConstantFolding` extension policy. This includes the
classic `offsetof` macro: the address of a member or element reached from an
integer-valued pointer constant carries that integer plus the byte offset, and
converting it to an integer type yields an arithmetic constant. Reading a
static-duration, const, non-volatile integer object with a constant initializer
also folds to that value. Strict modes
keep the integer-constant-expression constraint, so such an array bound is
variable length.

`conversions` retains the syntax expression, destination type and conversion
kind: lvalue conversion, array/function decay, arithmetic conversion, assignment
conversion or default argument promotion (§6.3, pp. 42-49, PDF pp. 54-61).
These are contextual operations, separate from the expression's original category;
sizeof and unary address operands therefore keep their unconverted identities.
Compound assignments also retain their arithmetic/pointer operation type before
the final conversion to the left operand's type. The backend must evaluate that
left operand once (§6.5.16.2p3, p. 93, PDF p. 105).

Integer promotions preserve enum and bit-field rules. Usual arithmetic conversions
preserve integer rank even where LP64 long and long long have equal widths, and
preserve real/complex component precision. Assignment conversion checks immediate
pointed-to qualifier inclusion and exact compatibility at deeper pointer levels;
it supports object/void pointers, null pointer constants and pointer-to-bool.
Function prototypes enforce argument count and assignment-compatible arguments;
unprototyped and variadic arguments receive default promotions. Core unary,
postfix, binary, conditional, assignment, comma, cast and compound-literal
constraints use the same helpers (§6.5, pp. 69-94, PDF pp. 81-106).

Expression completion runs on the existing explicit work stack after children
and type names. Failed operands yield `Unknown` and suppress dependent diagnostics.
Recovery taint and unmodeled extension owners likewise yield unknown results;
walking their children is not a claim that their extension semantics were checked.
Each completed record retains a field table (`Tag::fields`) that resolves every
name in its member namespace, including names contributed by anonymous members,
to the member path, the qualifiers of the anonymous members on that path and the
byte/bit offset from the record's start. Member access and designators use it
through a scratch index with expected O(1) lookup; const-member queries cache
explicit postorder results rather than rescanning aggregate trees for every use.

#### Constant expressions

Typed expression folding shares `integer.rs` arithmetic with the Stage-1 ICE
evaluator. Eligibility remains separate from a folded value: arbitrary folded
integer expressions are not automatically ICEs (§6.6p6, p. 95, PDF p. 107).
ICE validation covers enumerators, bounds, bit-fields, designators, case labels
and supported static assertions. Case expressions retain evaluated values;
duplicate case checking and conversion to the switch type belong to Stage 3.
Unevaluated sizeof operands and unselected logical/conditional arms follow the
§6.6p3 exceptions. Strict ICE floating operands require immediate integer casts.

Arithmetic constant expressions and address constants govern static-duration
initializers (§6.6p7-9, pp. 95-96, PDF pp. 107-108; §6.7.8p4, p. 125, PDF p. 137).
Addresses track static storage/function/string/compound-literal designations,
including members, subscripts, casts and integer offsets, without reading stored
object values. This is constant eligibility, not backend relocation lowering:
syntax, binding identity and retained conversions preserve the operands needed
to lower a symbol plus offset. As §6.6p10 permits and GCC does, the difference
of two addresses within one object (the same binding, identical string
literals, or integer-valued pointer constants) folds to an arithmetic
constant. Scalar initializers additionally apply assignment
conversion. Automatic aggregate copies may use compatible record expressions.

Finite floating arithmetic uses the existing padding-free native `LongDouble`
carrier, with a small C bridge for arithmetic, comparison and rounding. The
configured GNU x86-64 host's x87 representation carries extended values;
MSVC target values convert and round in binary64. Precision control is explicit
inside the native bridge, including on Windows worker threads.
Operands round to the common component type before arithmetic/comparison, and
results round to the expression type. Integer arithmetic uses checked signed
128-bit operations and modular unsigned 128-bit operations; the linker retains
the late compiler-builtin division objects before LTO.
Exceptional constant evaluation produces one primary diagnostic.

#### Initializer current objects

`initializers.rs` implements §6.7.8p17-22 (pp. 126-128, PDF pp. 138-140) with
explicit list/value continuations and immutable arena cursor paths. Each cursor
identifies an array element or named record member and its containing cursor;
brace elision descends and subsequent elements advance or unwind that path.
Designators reset the current path relative to the containing brace pair.
Unions initialize one selected member; unnamed bit-fields and flexible-array
members do not consume ordinary initializer positions. Excess elements,
incompatible scalar values and invalid designators have structured diagnostics.
Narrow/wide strings initialize their matching character arrays, with optional
braces and the permitted omitted terminator in an exactly sized array.

An incomplete outer array records the greatest initialized top-level index and
completes its canonical type before following expressions resolve its binding
(§6.7.8p22). This includes nested designators and brace elision. Recovered or
unmodeled range initialization yields an unknown completed result rather than
a fabricated extent or a dependent incomplete-object error.

Statement expression sites, return operands and loop iteration/initialization
expressions are typed. Selection/iteration conditions require scalar type and
switch operands require integer type (§6.8.4-§6.8.5, pp. 133-137, PDF pp. 145-149).
Stage 3 reuses assignment compatibility and retains assignment conversions for
return expressions.

#### Boundaries carried forward

- Stage 3 implements return conversions, labels/gotos, case dispatch,
  function-definition parameter rules, inline-body restrictions and
  tentative-definition completion.
- Other GNU builtins, statement-expression results, union casts, range initializers,
  generic selections, count queries, attribute-derived types and newer-standard
  special values remain conservative unknowns. Core operator checks do not
  implement GNU void/function-pointer arithmetic.
- Floating constants follow the IEC 60559 arithmetic of Annex F, as GCC and
  Clang do on this target: overflow and division by zero give infinities or
  NaNs, negation keeps the sign of zero, and a NaN compares unordered. These
  are valid arithmetic constants; only converting a nonfinite value to an
  integer type is diagnosed. Annex G complex special cases and floating
  exception flags are not modeled. Floating evaluation also depends on the
  configured native x87 bridge rather than a portable software target-float
  engine.
- Initializer traversal validates and completes types; it does not emit a
  flattened store plan or materialize implicit zero-filled object bytes. Those
  are backend lowering responsibilities, recoverable from syntax and types.

### Stage 3: Statements and functions

#### Function derivation and body bindings

`functions.rs` identifies the first derivation outward from the identifier,
walking grouping and pointer/suffix constructors iteratively. A definition must
specify its own function derivation rather than obtain a function type solely
from a typedef (§6.9.1p2, printed p. 141, PDF p. 153). Parameter lookup uses the
immutable defining suffix's syntax identity. It does not use the enclosing
declarator's provenance, which may describe several different function suffixes.
Thus `int (*fp(int a))(int b)` binds `a` in its body, and a grouped
`int (g(int a))` retains the same binding and redeclaration rules as `g`.
K&R signature comparison uses that identical suffix and labels the previous
declaration on a mismatch.

Prototype definitions require named, complete adjusted parameters, with the sole
unnamed `void` exception (§6.9.1p5 and §6.7.5.3p4, printed pp. 141 and 118,
PDF pp. 153 and 130). C23 permits unnamed parameters. Earlier-mode pedantic
diagnostics already emitted by the parser are not duplicated by sema; sema
enforces the missing-name constraint when that parser policy is silent, including
typedef spellings that the parser cannot resolve. `[*]` is excluded from definition
parameters while nested function prototypes retain prototype scope. Definitions
check storage classes, complete return object types and declaration-list
consistency. Missing K&R declarations bind `int` before the body: native in C89/C95,
a constraint diagnostic in strict C99 and later, and shared implicit-int extension
policy in GNU modes. This prevents accidental lookup of a same-named global.

`__func__` is a function-entry binding for a static const character array,
including before its first use (§6.4.2.2p1, printed p. 52, PDF p. 64). Its
retained synthesized definition records the function-name string. Inner blocks
may shadow it; the function's outer block may not redefine it. Hosted `main`
signatures outside the portable `int (void)` and `int (int, char **)` forms,
including internal linkage, receive an implementation warning. With
`-ffreestanding` the startup function is implementation-defined (§5.1.2.1p1),
so `main` is not checked. GNU nested
definitions have lexical bindings and no translation-unit linkage; their
statement/return state is independent of the enclosing function.

#### Statements and jump scope

`statements.rs` composes continuations with the existing work stack. It checks
scalar conditions, integer switch conditions and promotions, expression
statements, loop/switch placement of `break` and `continue`, and for-init object
and storage constraints (§6.8, printed pp. 131-139, PDF pp. 143-151). Returns
use Stage 2 assignment compatibility and retain conversion to the unqualified
result type (§6.8.6.4p3, printed p. 139, PDF p. 151). Missing values in non-void
functions are errors in strict C99 and later, warnings in C89/C95 and GNU modes.
Returning a void expression from a void function uses GNU extension policy;
other expressions in void returns violate the constraint.

Each associated selection/iteration substatement has its own block scope,
whether it is a compound or a single expression (§6.8.4p3 and §6.8.5p5, printed
pp. 133 and 135, PDF pp. 145 and 147). Enum/tag declarations in a then-body
therefore cannot leak into an else-body or a do-while condition. These scope
continuations use the same explicit work stack as expression typing.

Each function has a separate label map. GNU `__label__` declarations install
lexically scoped label identities, also available to nested functions. Label
addresses count as references; computed-goto destinations remain opaque.
Duplicate labels are diagnosed immediately, and unresolved references after
the translation unit has been walked. The parser retains ownership of the
duplicate-default diagnostic, so it is emitted once. Case/default placement
and case ICE requirements are checked semantically. Case values are converted
to the promoted control type; GNU inclusive ranges use the same conversion.
Cases are collected, then sorted once per switch. Duplicate values and overlaps
are checked in O(n log n), and empty converted ranges produce a warning.

A **variable scope path** is a persistent chain of declarations of variably
modified identifiers, including typedefs and pointers to VLAs. Scope exit
restores the previous chain. Labels, gotos and switch entry retain snapshots
without borrowing a live scope stack. Iterative path comparison diagnoses jumps
that enter a target declaration's scope (§6.8.6.1p1 and §6.8.4.2p2, printed
pp. 137 and 134, PDF pp. 149 and 146). Forward labels are checked after their
definitions are known. GNU nonlocal jumps through an enclosing local label are
kept conservative rather than applying ordinary same-function VLA rules.

#### Translation-unit definitions

Declaration occurrences remain distinct from entity definition state. Linked
entities accumulate definitions, tentative declarations, their latest composite
type and file-scope inline/extern status (§6.9-§6.9.2, printed pp. 140-143,
PDF pp. 152-155). Repeated definitions are rejected. At translation-unit end,
tentative definitions without explicit definitions become zero-initialized
definitions. Incomplete external arrays become one-element arrays with a warning;
incomplete internal tentative types are rejected at their declaration. Other
uncompleted tentative object types are rejected at translation-unit end.

`SemanticTranslationUnit.definitions` retains deterministic binding-index order
for explicit objects/functions, inline-only bodies, completed tentative objects
and implicit function-name arrays. The existing inspection schema is retained;
implicit `__func__` bindings appear through its ordinary binding output. No
statement inspection format was added. Earlier source occurrences and expression
types retain their point-of-use types; synthesized tentative definitions identify
the final composite binding needed by lowering.

Internal-linkage expression uses require a definition, except operands of
constant `sizeof` results (§6.9p3, printed p. 140, PDF p. 152). Persistent
sizeof contexts are resolved after typing, so runtime VLA-size uses still count.
Used undefined static functions are errors; unused undefined declarations warn.
Inline restrictions are deferred until all file-scope declarations are known:
a later non-inline or `extern` declaration can make the body an external
definition (§6.7.4p3,p6, printed p. 112, PDF p. 124). Inline-only external bodies
cannot define modifiable static objects or reference internal identifiers,
including in sizeof operands. Static inline and ordinary external definitions
are exempt. On the `x86_64-pc-windows-msvc` target, as in MSVC and Clang, the
Microsoft ABI emits every inline function as a discardable external definition,
so no body is an inline definition and these restrictions do not apply; the
UCRT's `__inline` functions keep modifiable static locals. Const qualification is checked on the object (or array element);
a const aggregate member does not protect the aggregate's other members.
Unmodeled `__builtin_*` calls receive opaque function results rather
than invented implicit-int return types; explicit visible prototypes still govern
known calls. Opaque results suppress dependent return-conversion errors.

#### Remaining backend and quality work

- Optional fallthrough/missing-return warnings and unused-label warnings are
  intentionally deferred. There is no CFG or claim of reachability analysis;
  infinite loops, noreturn attributes, exhaustive switches and goto cycles
  therefore receive no speculative fallthrough warnings.
- A backend still needs labels and branch targets, switch dispatch lowering,
  runtime VLA allocation/extent evaluation, automatic lifetime cleanup, return
  value lowering and a calling convention. The syntax and conversion graph remain
  available, but statement constraints do not provide an executable control-flow
  graph or computed-goto target set.
- GNU statement-expression result types, intrinsic signatures, local-label
  nonlocal-jump mechanics, GNU89 inline semantics, attributes and calling-convention
  effects remain conservative. The five requested GNU statement forms retain
  parser-owned extension policy; accepted opaque constructs are not certified.
- The existing parameter-inspection metadata still uses source provenance;
  definition binding and K&R comparisons use suffix identity. A backend requiring
  arbitrary parameter-site lookup should index the immutable suffixes itself.
- Initializer materialization, implicit object bytes, relocations and backend
  emitted-symbol selection remain lowering work. Definition records distinguish
  inline-only bodies from externally provided definitions.

## Validation

Unit tests cover type interning, compatibility and composite types, layout,
integer/floating constant evaluation, expressions/conversions, current objects,
scopes, linkage, and tags. Deep-input tests check
that traversal stays iterative: 100,000 pointer derivations, 100,000
compatibility derivations, 10,000 nested blocks, 100,000 expression parentheses,
100,000 unary operators and 100,000 additions. Layout and expression-sizeof expectations were
checked against Clang `_Static_assert`s for `x86_64-unknown-linux-gnu`. Each
semantic diagnostic has a rendered golden fixture, and `tests/allocation_count.rs`
checks that declarations, expressions, initializers and their diagnostics allocate
only from arenas. The same harness requires every ordinary and parser-stress
benchmark input to remain diagnostic-free through `sema`.

The `Semantic analysis` Criterion group runs phases 1-7 plus semantic
analysis over the same inputs as `Parser` and `Parser only`; the difference
from `Parser` is the cost of analysis.

Integer constant evaluation stores 128-bit patterns plus width and signedness.
Unsigned high-half patterns use `u128` arithmetic and formatting. Its division relies on
the linker flags in `.cargo/config.toml` that keep `compiler_builtins`'
`__divti3`/`__modti3` across linker-plugin LTO (see the README's LTO section).

### Diagnostic survey and triage

All 14 `.c` files in `test-programs/` completed without crashes or timeouts.
Comparing default compilation with syntax-only inspection found zero new semantic
errors; the 23 existing preprocessing/syntax errors were unchanged. This small,
mostly preprocessor-oriented set is not sufficient evidence of C99 conformance.

The 21 new semantic diagnostic fixtures produce 30 semantic errors and one
function-qualifier warning. Linux Clang
with `-std=c99 -pedantic-errors` also rejects every fixture. Sample triage:

| Input | Classification and independent check |
| --- | --- |
| `sema-duplicate.c` | Genuine: repeated automatic declaration and automatic followed by same-scope extern; Clang rejects both. |
| `sema-enum-range.c` | Genuine under `-pedantic-errors`: values outside int are the C23 extension Clang also reports, and Clang also rejects a negative member beside one that needs unsigned long. |
| `sema-member.c` | Genuine: void member and a flexible array before the last member; Clang rejects both. |
| `sema-flexible-extension.c` | Genuine under `-pedantic-errors`: the four GNU flexible-array forms violate §6.7.2.1p2 or p16; Clang reports the same four as extensions. |
| `sema-overflow.c` | Genuine C99 ICE constraint; Clang accepts folding only as a GNU extension and rejects it in strict mode. |
| GNU literal zero array in existing policy fixture | Initial sema rejection was a false positive; fixed to use shared parser/extension policy without duplicate diagnostics. Computed zero uses the same policy. |
| `sema-function-qualifier.c` | Genuine undefined behavior under C99; emitted as a warning, while strict Clang promotes its ignored-qualifier extension warning to an error. |
| Old-style prototype and body parameter types | Positive promoted short-to-int and float-to-double examples accepted by Clang and retained as regressions. |

The 12 retained syntax torture regressions pass. The Stage 3 survey below also
exercises the complete semantic CLI.

The defensive `UnknownTypedef` kind is reached only when the parser's typedef
classification and semantic scopes disagree. Well-formed source did reach it
while a nested-declarator function definition bound the wrong parameter list
(`typedef int b; int (*fp(int a))(int b) { b x = a; }`); Stage 3 binds the
defining list, and no other well-formed path is known. Its constructed resolver
unit test replaces a source golden; the other Stage 1 kinds have rendered source
goldens.

Stage 2 adds 28 diagnostic fixtures, covering every new semantic kind and the
shared implicit-function extension policy; the existing overflow diagnostic now
also covers arithmetic initializer evaluation. Linux-target Clang rejects every
negative fixture under its matching standard with `-pedantic-errors`.
The ordinary survey still has 14 files, 23 preprocessing/syntax errors and zero
additional semantic errors. The positive shared `expression-sizeof-probe.c`
checks inferred arrays, compound literals, strings, promotions, complex sizes and
pointers through both sema and Clang static assertions. This corpus and the unit
tests establish regression protection, not general conformance.

Stage 3 adds 36 diagnostic fixtures for 30 new semantic kinds and the shared
extension policy: 39 rendered errors and five warnings. Positive and negative
unit tests cover each rule; explicit work-stack tests use 10,000 nested
if/while/block statements and 10,000 chained cases. Allocation tests cover the
new paths, synthesized definitions, invalid statements and deep nesting. All
canonical checks pass with `CARGO_BUILD_JOBS=8` and the configured fat LTO.
The ordinary 14-file survey matches an archived Stage 2 CLI exactly: 23 existing
preprocessing/syntax errors and no additional diagnostics or crashes. Linux-target
Clang was run for every file and each new golden. Clang accepts the incomplete
tentative-array fixture with the same one-element warning; it warns rather than
errors on the modifiable-static inline fixture, and needs an optional warning
flag for the unused static declaration. Sema's severities follow the Stage 3
contract: inline restrictions are errors and unused undefined static functions
warn by default.

The GCC 15.2.0 torture corpus comparison uses `scripts/run_gcc_torture.py`, six
workers and the cached corpus without modifying it. The baseline CLI is built
from a `git archive` export of Stage 2 commit
`d06ebdf33c1e54f9781e1a7c193f6e5ec903da77`, with a separate target directory and
a copied LLVM toolchain. Both builds use plain `cargo build --bin bcc-rust` and
the same configured linker-plugin fat LTO. Rebuild the plain CLI after the
feature-enabled allocation checks: `benchmarking-internals` replaces its entry
point with a benchmark loop. GCC 13.2.0 targets Windows x86-64;
independent Clang checks target `x86_64-unknown-linux-gnu`.

| Survey measure | Stage 2 baseline | Stage 3 |
| --- | ---: | ---: |
| Raw inputs surveyed | 3,878 | 3,878 |
| Raw accepted / diagnosed | 3,069 / 809 | 3,119 / 759 |
| GCC C99 pedantic accepted | 2,874 | 2,874 |
| GCC-accepted raw inputs accepted / diagnosed by bcc | 2,327 / 547 | 2,352 / 522 |
| Header-free GCC C99 preprocessed inputs | 2,596 | 2,596 |
| Preprocessed accepted / diagnosed | 2,514 / 82 | 2,560 / 36 |
| Newly rejected GCC-accepted raw / preprocessed inputs | — | 0 / 0 |
| Crashes, timeouts and process failures | 0 | 0 |

The first comparison exposed five dependent return-conversion false positives:
`compile/pr37669.c`, `compile/pr38343-2.c`, `compile/pr46360.c`,
`compile/pr65873.c` and `execute/20030323-1.c`. Their pointer-returning GNU
intrinsics (`__builtin_strdup`, `__builtin_stpcpy`, `__builtin_strncpy`,
`__builtin___memcpy_chk`, `__builtin_return_address`) had been assigned invented
implicit-int results. Opaque intrinsic signatures fix those return diagnostics;
reduced positive regressions retain the behavior, and a genuine integer-to-pointer
return still diagnoses. Clang confirms the pointer returns; the unreduced GCC
sources also include a GCC-only va-arg-pack intrinsic and an old-style `main`
spelling that this Clang rejects independently. The final complete survey has no
new GCC-accepted rejections. The 36 remaining preprocessed rejections already
occurred in Stage 2 and are outside this stage's new statement/function rules.
Evidence is retained in `target/survey-stage2`, `target/survey-stage3` (initial
triage), `target/survey-stage3-cli`, and `target/sema3-final-triage.json`.

## Freestanding resources and intrinsic boundary

Embedded freestanding headers and reserved target-description macros are now
available through the ordinary preprocessing pipeline. The implementation and
mode decisions are in [language-standards.md](language-standards.md). Varargs
operations retain typed GNU builtin syntax; semantic analysis resolves
`__builtin_va_list` as an array of one opaque complete SysV record (24 bytes,
alignment 8) on Linux, and as `char *` on Windows. Ordinary parameter adjustment
and array decay apply; Windows operands must be modifiable lvalues. Intrinsics
check va-list operands, complete object result types for `va_arg`, and variadic
function context for `va_start`; `va_start`, `va_end` and `va_copy` return void.
`offsetof` resolves field/index paths iteratively and retains a `size_t` ICE.
No varargs instruction lowering, runtime pairing or argument-availability
checks are implied. `FLT_ROUNDS` describes the default round-to-nearest state;
observing dynamic rounding-state changes remains backend work.

The shared `tests/fixtures/freestanding/conformance.c` probe checks every
header, ABI layouts, integer limits and constant helpers in bcc and Linux-target
Clang. Strict C11 validates portable integer checks; GNU17 additionally validates
floating-value comparisons (Clang folds these as an extension to ICE rules).
Every emitted target macro matches the pinned Clang `-dM -E` definition exactly,
and CLI tests compare expanded values with live Clang in every language mode.
Allocation coverage exercises all resources and invalid varargs paths.

The raw GCC torture comparison with `target/survey-stage3-final/results.json`
accepts 3,565 of 3,878 sources (448 newly accepted, two newly rejected),
with no crashes or timeouts. GCC C99 pedantic acceptance is unchanged at 2,874;
bcc accepts 2,757 of those, up from 2,352, with no newly rejected strict-valid
sources. Header-free preprocessed acceptance rises from 2,560 to 2,565.

Both new raw rejections are independently GCC GNU17-valid and expose the
existing lack of GNU global register variables when `__x86_64__` now selects
the architecture-specific branch:

| Source | Triage |
| --- | --- |
| `compile/20041119-1.c` | File-scope `register unsigned int reg __asm("r14")`; GCC GNU17 accepts, Clang Linux rejects the unsuitable global register, bcc reports file-scope register storage. |
| `execute/pr51447.c` | File-scope `register void *ptr asm("rbx")`; GCC GNU17 accepts, Clang rejects the register and nested function, bcc reports file-scope register storage. |

These are documented GNU-extension gaps, not reasons to hide the target's
architecture macros. The semantic-review work owns broader declaration rules;
this change does not alter global-register semantics. Full evidence, including
per-file logs, delta lists and independent GNU-mode triage, is retained under
`target/survey-freestanding/`.


## Target-selection verification (2026-10-09)

The pinned Clang 23.1.1 accepts the shared C11 target probe for all four
canonical triples. bcc accepts the same probe, with direct Rust checks for
canonical typedef/literal types, bit offsets, UTF-16 surrogate values and
long-double precision supplementing syntax-only `_Generic` handling. Exact
macro comparisons pass for all four triples in both ISO C11 and GNU C17,
minus the 139 individually documented exclusions in language-standards.md.
Aliases and invalid/missing CLI values are covered. No existing snapshot was
changed, and every definition in the original 163-macro fixture is preserved.
The scalar/layout/macro oracle is Clang's freestanding resource contract;
this does not claim integration with installed glibc, musl, MinGW or UCRT.

Canonical checks passed unmodified with `CARGO_BUILD_JOBS=8`: all-target tests
(977 tests across their binaries), allocation counting (16 tests), nightly
format checking, both all-target clippy commands with `-D warnings`, and
`git diff --check`. The plain CLI build was restored after the feature test;
the benchmarking feature's executable is intentionally a benchmark loop.
Complete output is retained in `target/target-*.log` and
`target/TARGET_VALIDATION.md`.

The requested default-target survey ran over all 3,878 sources using the
unmodified command/harness and baseline `target/survey-r2/run/results.json`.
Its exact no-change requirement is **not met**, because it conflicts with the
new target-macro contract:

- Raw acceptance changes from 3,598 to 3,581; diagnostics change from 280 to
  297. The 17 affected `execute/builtins/*-chk-lib.c` sources (including
  `builtins/lib/chk.c`) now enter `#ifdef __unix__` and include `sys/types.h`,
  which this freestanding resource set does not provide.
- `execute/memchr-1.c` stays accepted but loses two undefined-macro warnings
  because `__WCHAR_WIDTH__` is now defined. Raw inputs with warnings change
  from 91 to 90.
- All 2,596 header-free preprocessed results, including diagnostic/warning
  counts, are identical: 2,588 accepted, eight diagnosed. GCC C99 acceptance
  remains 2,874 with zero timeouts. All other raw results are identical.
- A source wrapper that undefines only `__unix__` for the 17 includes, or only
  `__WCHAR_WIDTH__` for memchr, reproduces each baseline status, error count,
  warning count and first error. This isolates every delta to new macros.

The complete per-file comparison and control evidence are in
`target/survey-target/comparison.json` and `target/survey-target/source-controls/`.
The survey baseline and harness were left unchanged. Hiding required macros,
inventing a partial libc `sys/types.h`, or weakening the survey would not
resolve this conflict within the target layer. Hosted include integration is
owned by the separate branch and remains to be validated after merging.

Review decisions: target selection does not imply syntax-extension flags;
MSVC enums use Clang's 32-bit int/truncation rule; Microsoft bit-field rules
apply to both Windows targets; the native x87 bridge saves/restores precision
control on Windows worker threads. Packed/aligned attributes, pragma-pack,
`_Generic` semantic selection, encoded u/U literal semantic types, target instruction lowering and a portable
software implementation of extended floating arithmetic remain existing gaps.
No resource header, hosted-mode or compiler-identity implementation was changed.

## GNU 128-bit integers (2026-10-09)

The target scalar graph now includes signed and unsigned `__int128`, each
16 bytes with alignment 16 on Linux GNU, Linux musl, MinGW and MSVC x86-64.
Rank is above `long long`; integer promotions, usual arithmetic conversions,
assignment/cast/Boolean/pointer conversions, and implementation-defined
integer bit-fields use the shared scalar rules (C99 §6.3.1.1-3, pp. 42-43;
PDF pp. 54-55; §6.3.1.8, pp. 44-45; PDF pp. 56-57;
§6.7.2.1p4, p. 101; PDF p. 113). Clang probes confirm that signed/unsigned
128-bit fields of width below 32 promote to int; at width 32 the result is
int/unsigned int respectively; wider fields retain the 128-bit base type.

The parser seeds `__int128_t`/`__uint128_t` as reserved builtin typedef names,
using the same header-independent seam as `__builtin_va_list`; semantic
resolution maps them to canonical scalars. They do not enter the macro dump.
`__SIZEOF_INT128__` 16 is enabled in all eight target macro snapshots and
removed from the exclusion generator, table, and documentation. The macro
oracle compares these snapshots with the pinned Clang for each triple/mode.
There is no 128-bit integer literal suffix or new literal candidate, and
`intmax_t`/`uintmax_t` and `#if` remain 64-bit.

Decision: constant representation is a 128-bit two's-complement pattern plus
width/signedness, without enlarging the retained record or allocating a wider
number. The `i128` field is interpreted as signed only for signed values;
unsigned operations explicitly reinterpret it as `u128`. Masks use a right
shift of `u128::MAX`, never a shift by 128. Checked mathematical accessors
protect array bounds, designators, offsets and enum ranges. Display uses
unsigned decimal for the full unsigned range, including conversion to floating
constants. Signed arithmetic checks overflow; unsigned arithmetic wraps.
Shifts validate the mathematical count before shifting, signed right shifts
are arithmetic, and GNU sign-bit left shifts keep the existing extension
policy. Both signed minimum / -1 and signed minimum % -1 are exceptional,
as are division by zero and signed minimum negation. Floating-to-integer
conversion accepts the unsigned high half and signed minimum, while rejecting
out-of-range values. Integer-to-pointer constants truncate to pointer width.
Switch interval ordering uses a sign-biased key for unsigned patterns, so
ranges crossing bit 127 are ordered and overlapping cases diagnosed correctly.

Decision: retain the existing enumeration ABI ceiling of 64 bits, even when
an ICE computes a 128-bit value. Wider enum values get `EnumeratorRange` with
no dependent cascade. This is an implementation boundary, not a claim that
all GCC/Clang wide-enum extensions are modeled; pinned Clang itself warns and
truncates an unsigned 128-bit maximum enumerator to unsigned long long.

Known gap: GNU `mode`/`__mode__` attributes (QI/HI/SI/DI/TI/word/pointer) remain
opaque balanced tokens and make affected types unavailable, just like other
unmodeled representation attributes. Modeling them cleanly requires an
attribute semantic layer that distinguishes names, arguments, placement and
conflicts; this change does not add an ad hoc token scan. The MinGW fallback
`typedef int __int128 __attribute__((__mode__(TI)));` is still invalid when
`__SIZEOF_INT128__` is manually undefined: `__int128` is already a keyword,
and pinned Clang rejects that exact fallback too. Normal MinGW inclusion
avoids the branch with the now-correct predefined size macro. No packed/aligned,
pragma-pack, vector, atomic, backend or code-generation support is claimed.

`tests/fixtures/targets/int128.c` is shared by semantic unit tests, CLI tests,
allocation coverage, and pinned Clang `--target=<triple> -fsyntax-only` checks.
It asserts scalar/struct/union sizes, alignments and offsets, target-specific
bit-field storage, full-width arithmetic, Boolean conversion and floating casts.
Rust tests additionally inspect canonical types, ranks, bit-field promotion,
all shift counts 64-127, signed/unsigned limits, narrowing and switch ranges.
The three `sema-int128-*` goldens cover overflow/exceptional arithmetic,
bit-field/bound/enum constraints, duplicate high-half cases and pedantic policy,
with following declarations retained. Survey and canonical validation evidence
is recorded below.

### Int128 validation and survey evidence

All six canonical commands passed unmodified with `CARGO_BUILD_JOBS=8`:
`cargo test --all-targets` (998 tests),
`cargo test --features benchmarking-internals --test allocation_count`
(18 tests, including valid/invalid int128 paths with zero global allocations),
`cargo +nightly fmt --check`, both all-target clippy commands with
`-D warnings`, and `git diff --check`. The plain executable was restored with
`cargo build --bin bcc-rust` before both surveys. Verbatim results/full logs
are in `target/INT128_CHECKS.md` and `target/int128-check-*.log`.
Clang accepts `tests/fixtures/targets/int128.c` using each canonical triple
and `-std=c11 -fsyntax-only`; the target macro regeneration/equality checks
match all four triples in both modes with 138 documented exclusions.

The requested libc survey command completed 440 individual translation units
plus eight combined probes; all 448 rows match the supplied `targeted` baseline
by identity. There are zero newly accepted or newly rejected headers. MinGW
GNU17 remains 7/58 Clang-accepted headers, and C17 remains 6/28. The 34 GNU17
headers formerly stopped by the `int __int128` fallback now get past it and
stop at `_mingw.h:580`, `void __cdecl __debugbreak(void);`. This is an existing
calling-convention keyword gate, independent of integer support: a standalone
probe reproduces the error, and `-fms-extensions` makes that probe pass, as does
pinned Clang. The requested MinGW survey arguments intentionally lack that
flag; the command was not changed to inflate acceptance. The other libc
configurations retain their previous accepted counts and first errors.

Current MinGW GNU17 top first errors:

| Count | First error |
| ---: | --- |
| 34 | Expected declarator punctuation before `__debugbreak` (`__cdecl` gate). |
| 13 | Expected `)` to close `defined(`, found `##`. |
| 2 | Cannot find `mm_malloc.h`. |
| 1 | Cannot find `stdatomic.h`. |
| 1 | Cannot find `tgmath.h`. |

MinGW C17 still has 20 VARARGS-not-implemented first errors, plus the two missing
standard headers. Evidence and exact deltas are in
`target/libc-survey/int128/results.json`, `target/int128-libc-survey.log` and
`target/int128-mingw-cdecl-triage.json`. These results expose the next blocker;
they do not claim an increase in accepted libc headers.

The GCC 15.2 torture survey compares 3,878 files with `target/survey-target`:
raw acceptance is 3,581 -> 3,584 (+3), and the 2,596 header-free GCC-C99-valid
preprocessed inputs improve from 2,588 -> 2,592 (+4). The three bit-field files
`compile/bitfield-1.c`, `compile/bitfield-endian-1.c` and
`compile/bitfield-endian-2.c` improve in both forms; `execute/pr98474.c` additionally
improves after preprocessing. All four formerly had unknown `__uint128_t`.
There are zero newly rejected inputs (including GCC-valid inputs), crashes,
process failures or timeouts. GCC reference classifications are unchanged.
The cached corpus was copied into this worktree before running the survey;
all generated sources, logs and evidence stayed under this worktree's target/.
Exact comparison: `target/int128-gcc-comparison.json`; full survey:
`target/survey-int128/results.json`. Ordered exact-file commit partitions are
in `target/INT128_COMMITS.md`; no commits or pushes were made.
