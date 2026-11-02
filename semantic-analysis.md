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
Resolved type names, typed expressions and contextual conversions follow;
conversions are listed by expression ordinal, each expression's in the order
they were applied.
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

`SemanticTranslationUnit.errors` counts the error-severity diagnostics semantic
analysis reported; warnings do not count. `Context::error_count()` counts
error-severity diagnostics from every phase, pending or already drained by a
reporter, and excludes withdrawn or suppressed ones. Lowering calls
`SemanticTranslationUnit::lowerable(&Context)`, which holds only when both are
zero. Because an unmodeled extension can still yield an unanalyzed type without
a diagnostic, lowering also rejects any type for which `Types::unanalyzed`
holds.

## Storage and traversal

The translation-unit arena (`'tu`) retains canonical type nodes, nominal tag
identities and completion cells, record members with byte/bit offsets and per-record member-name tables, binding
occurrences, semantic scope identities, resolved type names, and parameter
metadata (including array `static` minimums), tag declaration occurrences, typed
expression results and conversion records. A backend can retain this graph
without retaining a semantic analyzer. The syntax tree remains separately available.
`semantic_analysis` re-exports the retained vocabulary to the crate: the type
graph (`Types`, `TypeId`, `TypeKind`, `Tag`, `TagKind`, `Member`, `Field`,
`FieldPath`, `Parameter`, `ArrayBound`, `Layout`, `Scalar`), expression records
(`ExpressionInfo`, `ValueCategory`, `Conversion`, `ConversionKind`,
`ConstantClass`, `Integer`, `Floating`) and declarations (`Binding`,
`BindingKind`, `Linkage`, `Duration`, `Scope`, `ScopeKind`, `Definition`,
`DefinitionKind`). The retained `Types` answers `kind`, `tag`, `layout`,
`non_atomic` and `unanalyzed` without the working interner.

The semantic working arena (`'s`) owns continuations, value stacks, interning
lookup, visible-binding lookup, linkage lookup and scope restoration lists. It
is dropped before returning `SemanticTranslationUnit<'tu>`. Retained values have
no destruction requirements. Assembler labels are decoded and inherited in
semantic completion; lowering receives finalized entity and symbol requests.
Attribute and calling-convention meanings are retained as symbol facts and
capability refusals (see [GNU entity attributes](#gnu-entity-attributes-and-cleanup-regions-2026-10-14)).
Collections use the repository's arena-backed
`ArenaMap`/`ArenaVec`; no compiler global allocator collections are introduced.

One explicit `Work` stack composes specifier resolution, declarator construction,
parameter lists, nested record/enum definitions, integer evaluation, initializers
and statement discovery. Parenthesized declarators never recurse in Rust.
Expression discovery/evaluation and scope traversal are also iterative.
String initialization retains the destination element type by expression syntax
identity, so lowering shares the whole-array classification for narrow and wide
literals, including optional braces and arrays completed from the string
(C99 §6.7.8p14-15, printed p. 126, PDF p. 138).
Compatibility/composite construction and type inspection have their own explicit
postorder stacks. Parameter/member lists accumulate in scratch cons cells, then
copy once into retained storage. Ordinary/tag lookup is expected O(1) per lookup;
scope exit costs the number of introduced entries.

The intended cost is linear in visited syntax plus retained output, with these
explicit exceptions: compatibility/composite checking can revisit shared typedef
graphs on repeated redeclarations; layout queries strip shared array derivations;
qualified array typedef construction traverses those derivations. These operations
are linear in the queried type graph, not necessarily just newly written syntax.
Conditional integer models and ICE operand eligibility are cached by syntax identity; deep conditional chains do not repeatedly scan their tails. No native-stack depth limit is imposed by semantic traversal; the emission model's fold evaluator and `noreturn` type walk recurse instead, bounded at 64 levels, and leave deeper forms unfolded or returning (see the emission model below).

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
Anonymous-member paths share retained index chains: each enclosing record
prepends one node, so deep nesting does not copy every descendant path.
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
units on Windows. `u8`, `u` and `U` strings have `char` (C23: `unsigned char`,
`char8_t`), `unsigned short` (`char16_t`) and `unsigned int` (`char32_t`)
elements with UTF-8, UTF-16 and UTF-32 units on every target
(`Context::encoded_literal_units`), complete arrays of unknown size, and
initialize only arrays of their own element type (C11 §6.4.5p6, §6.7.9p15);
`u` and `U` character constants have `char16_t` and `char32_t` type, and a
`u8` constant `char8_t`. Source literal units remain intact for inspection;
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
`FlexibleArrayExtensions` policy. Supported GNU packed/aligned attributes retain
their layouts; unmodeled vendor attributes make affected layouts unavailable.
A union that qualifies for GNU `transparent_union` records `Tag::transparent`;
a prototyped call whose argument does not have the union type converts it to
the first member it fits, as Clang selects one, and records an assignment
conversion to that member followed by a `TransparentUnion` conversion, which
lowering turns into a union temporary holding the value. The attribute written
elsewhere is ignored with Clang's warnings: on a union specifier that does not
define the union, on a structure or enumeration, on an object, function,
parameter or member, and among the specifiers of a declaration that declares
only a tag.
Windows records use Microsoft's bit-field layout on both MinGW and MSVC,
matching Clang's MinGW default `-mms-bitfields`. Consecutive nonzero fields share
an entire reserved storage unit only when their base-type sizes match and they
fit. A zero-width field affects layout only immediately after a nonzero
bit-field, where it ends that field's whole unit (see "Record layout
references" below). Union bit-fields occupy a full unit without increasing
alignment; zero-width union fields occupy no bytes, except that on MSVC one
after a nonzero bit-field reserves its type's size. The C11 probe in
`tests/fixtures/targets/conformance.c` covers every scalar, ABI aliases, offsets,
mixed-type fields, exhausted units, zero-width fields, compact byte-field
records, and unions against all four Clang targets. Rust tests additionally
verify bit offsets, typedef identities, literal types and UTF-16 surrogate
values directly. `_Generic` now resolves associations in sema (see the atomic section below).
The original target probe also includes **direct Rust type assertions** supplying
bcc's type evidence rather than claiming unmodeled generic assertions passed.
Packed/aligned attributes and translation-ordered pragma packing are retained
layout facts (see the storage and member-place alignment contract below).

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

### Storage, natural and member-place alignment

`packing::states` interprets `#pragma pack` once from macro-replaced operands and
output-token ordinals retained by preprocessing. The parser retains the opening
brace ordinal on each record definition; semantic analysis queries that state
and stores `Tag::max_member_alignment`. Record completion owns the resulting
member offsets, allocation units, alignment and size, and ABI projection consumes
those facts. This avoids source-location joins and downstream pragma walks.
The opening state follows bundled Clang even if a directive occurs inside the
body; the changed state still applies to subsequent definitions.

The stack supports reset, numeric caps (0, 1, 2, 4, 8, 16), unnamed/named pushes
and pops, optional pop caps, and `show`. Setting/resetting the cap does not
change saved entries; a named pop removes its matching entry and every entry
above it, and a missing name leaves the stack intact. Malformed forms are
warnings and leave the state unchanged. Empty pops, named pops with an alignment,
unterminated pushes, and `show` use Clang-compatible warnings, independently of
language-extension policy. Includes, `_Pragma` and `__pragma` use the same state.
State construction uses arenas and an explicit stack; each definition performs
a binary search over directive boundaries, rather than replaying the directives.

GNU layout caps explicit member alignment; MSVC ordinary members retain an
explicit higher alignment. Record alignment attributes can still raise the final
alignment. System V value bit-fields under a nonzero cap may cross their natural
allocation boundary and packed union fields occupy their written bits. Zero-width
fields retain the System V natural boundary. Microsoft fields retain complete
declared storage units; active zero-width fields follow the distinct MinGW/MSVC
rules. These are implementation-defined choices under C99 §6.7.2.1p10-12
(printed pp. 102-103; PDF pp. 114-115), checked against all three Clang targets.
See [GCC Structure-Layout Pragmas](https://gcc.gnu.org/onlinedocs/gcc/Structure-Layout-Pragmas.html),
[MSVC pack](https://learn.microsoft.com/en-us/cpp/preprocessor/pack), and
[Clang pragma semantics](https://clang.llvm.org/doxygen/SemaAttr_8cpp_source.html).

`Member::explicit_alignment` alone requests a new System V value-bit-field
boundary. Typedef-inherited alignment affects record alignment without splitting
adjacent fields. MinGW ignores inherited typedef alignment on bit-fields; MSVC
and System V retain it, following bundled Clang. Packed placement and zero-width
boundaries retain their separate target rules. C99 §6.7.2.1p10 (printed p. 102;
PDF p. 114) leaves allocation-unit alignment unspecified; the GNU
[aligned attribute](https://gcc.gnu.org/onlinedocs/gcc/Common-Type-Attributes.html)
supplies the extension contract.

`Types::storage_alignment` is the retained query for a declaration's storage
requirement, including variable array derivations. `Types::layout` supplies the
same requirement for complete fixed objects; `Types::natural_alignment` ignores
storage overrides through arrays and atomic derivations while preserving record
packing. Variable arrays preserve the same outermost override precedence as fixed
array layout, including GNU typedefs that lower alignment. ABI projection uses
natural/value layout rather than declaration
alignment to classify a value.

Member typing retains `ExpressionInfo::place_alignment` from the containing
place and resolved offset, including nested packed aggregates. Zero-width
bit-fields apply explicit member alignment to the boundary: System V keeps the
natural base-type minimum without raising the record alignment; MinGW honors
explicit boundaries even without a preceding bit-field; MSVC requires a preceding
value bit-field. Packed MSVC value-field placement remains distinct from an
explicit zero-width boundary. These rules are checked against bundled Clang.

Lowering owns allocated storage guarantees and refines them using the retained
member facts. Conversions describe value types and cannot strengthen a backing
place. See [Memory facts](middle-end.md#memory-facts) for the one alignment owner.
C99 6.7.2.1p12, p. 103 (PDF p. 115); C11 6.7.5p6, p. 127 (PDF p. 145);
GNU [aligned/packed attributes](https://gcc.gnu.org/onlinedocs/gcc/Common-Type-Attributes.html)
and System V AMD64 ABI 3.2.3.

### Record layout references

Record layout is checked against the platform compilers, not only bundled
Clang: MinGW-w64 GCC and Clang for `x86_64-w64-windows-gnu`, Microsoft's
`cl.exe` and Clang for `x86_64-pc-windows-msvc`, and GCC and Clang (WSL) for
Linux. `scripts/layout_fuzz.py` generates random records (scalars, arrays,
bit-fields of every integer type including zero-width and unnamed ones, nested
records, `aligned`/`_Alignas` members, `#pragma pack` and `packed`) and compares
each record's size, alignment, member byte images and bit-field read-back. Its
deterministic programs in `tests/fixtures/layout/` run in the interpreter and
natively (`pipeline::tests::middle_end::record_layout`), and
`semantic_analysis::tests::record_layout` pins the cases below.

- **Zero-width bit-fields (#55).** In Microsoft layout a zero-width field after
  a nonzero one ends that field's allocation unit (C99 §6.7.2.1p11): the next
  member starts after the whole unit, aligned for the zero-width field's type.
  MSVC caps that alignment with `#pragma pack`. MinGW GCC caps it too, and in a
  `packed` record still raises the record's alignment while placing the next
  member without padding. Clang's MinGW layout ignores the pack cap on
  zero-width fields and then lays such packed records out with overlapping
  fields (conformance finding D1), so the MinGW target follows GCC there.
- **MSVC required alignment.** `_Alignas` (or `aligned`) on a member, a typedef's
  alignment, or such a member inside a nested record or array of records is a
  requirement `#pragma pack` cannot lower: the containing record is aligned to
  it and so is every record containing that one (`Tag::required_alignment`).
  MinGW does not propagate it.
- **System V aligned bit-fields.** Without a pack, a bit-field that would
  straddle a unit of its alignment starts the next unit; otherwise an `aligned`
  request no larger than the pack aligns its start. `#pragma pack` overrides
  `packed` on a bit-field, which keeps its pack-capped declared alignment.

Where the references disagree on MinGW outside zero-width fields, bcc keeps
Clang's layout: GCC aligns unions to their bit-fields' types and does not let
an `aligned` bit-field in a packed record raise the record's alignment, and on
Linux GCC honors `aligned` bit-fields above the pack. The 2026-10-14 run
(seeds 2000-2109, 5,500 records per target) found no record on which bcc
differs from agreeing references. MSVC: all 5,500 match `cl.exe` and Clang.
Linux: 5,435 match both; on 65 GCC and Clang disagree and bcc follows Clang.
MinGW: 4,848 match both; on 238 bcc follows GCC (zero-width fields), on 343
Clang, and 3 mix the two policies; 68 records came after a Clang-built
program crashed on an overlapping layout, and the 3 of those that differ
from GCC were checked by hand to follow Clang's (non-crashing) layout.

## Binding, scope, linkage and recovery

Ordinary identifiers and tags have separate visible maps. An entry remembers its
previous visible binding; each scope owns an undo list. File, function, block,
prototype and implicit selection/iteration scopes are independent of parser scopes.
Bindings retain source occurrences rather than replacing earlier declarations.
Linked redeclarations form composite types only from visible prior declarations
(§6.2.7p4), with a separate translation-unit linkage map checking compatibility
across disjoint lexical scopes. Incompatible occurrences remain inspectable but
do not replace accepted bindings in either lookup map. A
redeclaration whose type is unanalyzed, such as GNU `__typeof__(f) f`, takes
the visible function's kind and keeps its type instead of conflicting with it.

Extern inherits a visible internal/external linkage (§6.2.2p4); a visible automatic
object does not confer linkage. File static is internal; other file objects and
functions are external unless inheriting prior function linkage. Internal/external
mixes are diagnosed (§6.2.2p7). Typedefs, enumerators, members and automatic objects
have no linkage. Objects at file scope, with linkage, or block static have static
duration; parameters/other local objects have automatic duration. Functions and
non-object bindings have no object duration. Such an object declared with
`_Thread_local`, `thread_local` or GNU `__thread` has `Duration::Thread` instead
(C11 §6.2.4p4), the retained fact lowering turns into a thread-local global; its
initializer is constant like a static one (C11 §6.7.9p4), and an inline
definition rejects a modifiable one (C11 §6.7.4p3). The parser rejects thread
storage beside a storage class other than `static`/`extern` (§6.7.1p2);
semantic analysis rejects it on functions (§6.7.1p4) and parameters
(§6.7.6.3p2), on a block-scope object without `static` or `extern`, and on one
declaration of an object but not another (§6.7.1p3). Same-scope no-linkage duplicates are
rejected. A typedef repeated with the same, not variably modified, type is
accepted (C11 §6.7p3); before C11, GNU modes included, it is a C11 extension
under the pedantic policy, as in GCC and Clang.

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
- Extension-derived types, fixed-underlying enums, decimal
  types and layout-changing extensions are unanalyzed. MSVC `__int8`,
  `__int16`, `__int32` and `__int64` are not: as in Clang, they are `char`,
  `short`, `int` and `long long`, and `signed` or `unsigned` selects that
  variant. Only alignment
  specifiers, constexpr storage, MSVC pointer-size modifiers and
  attributes naming `align`, `packed`, `mode` or `ext_vector_type`
  (and non-vector `aligned`) make that one declaration's type unavailable; other
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
O(1) operand lookup. When analysis ends, the map is copied at its final size
into the translation-unit arena, so lowering looks a node up with
`SemanticTranslationUnit::expression_index` or `expression_info` in expected
O(1). The vector keeps deterministic order for inspection and iteration; the
map is used only for lookup, so host addresses never reach output.

Value categories distinguish lvalues, their modifiable subset, function
designators and rvalues (§6.3.2.1p1, printed p. 46, PDF p. 58). Const aggregate
members, including array elements, prevent modification; bit-fields retain enough
information for promotion and address/sizeof constraints. Parentheses preserve
the underlying category. Ordinary identifiers resolve through the declaration
binding model. A function's `__func__` is modeled as a static const character array;
`__FUNCTION__` names the same object and `__PRETTY_FUNCTION__` a second one holding the signature.
Calls to undeclared names are accepted only in C89/C95 and GNU modes, using
shared removed-feature policy. Known standard library names receive the complete
prototype from the existing 24-entry library-alias table, with target `size_t`,
const input pointees and ellipsis. They also warn once per implicit binding,
naming the function and type, as bundled Clang does. Other names keep the
unprototyped int-returning type. Explicit declarations keep their written types;
this path neither replaces them nor adds builtin folding or attributes. Strict
C99 and later diagnose undeclared identifiers. This compatibility extension
avoids mismatched calls under C99 §6.5.2.2p6,p9 (pp. 71-72; PDF pp. 83-84);
§7.1.4p2 (p. 167; PDF p. 179) permits library declarations without headers.
The C89 implicit declaration rule remains §3.3.2.2 (p. 41; PDF p. 55). Undeclared reserved `__builtin_*` names are GCC
builtins in every mode: they are bound as opaque functions without a
diagnostic, and calls to them have unknown results, except that
`__builtin_constant_p` uses the shared Clang-like query evaluator: arithmetic
constants, string starts and supported constant aggregate elements can fold;
unknown queries emit zero. This value also drives static initializer data and
runtime lowering.

In GNU modes, as in GCC, an arithmetic constant that is not an integer constant
expression (§6.6p6) is folded where an integer constant expression is required
and reported through the `ConstantFolding` extension policy. This includes the
classic `offsetof` macro: the address of a member or element reached from an
integer-valued pointer constant carries that integer plus the byte offset, and
converting it to an integer type yields an arithmetic constant. Reading a
static-duration, const, non-volatile integer object with a constant initializer
also folds to that value. Absolute pointer-to-integer casts and address
differences retain a separate non-ICE marker in every language mode, matching
Clang's additional constant forms (§6.6p10). File-scope arrays, static local
arrays, members and compound literals fold these bounds with
`-Wgnu-folding-constant`; automatic local arrays and other abstract type names
retain VLA bounds. Consequently a sizeof of such a type name is runtime-valued
and cannot size a file-scope array. Integer casts preserve the non-ICE marker,
including narrowing and `_Bool` conversions. Function declarations retain
variably modified return bounds and reject them under §6.7.5.2p2.
Switch cases retain the same folded values that ICE evaluation accepted,
including their dispatch edges. Other folded arithmetic in strict modes keeps
the integer-constant-expression constraint.

`conversions` retains the syntax expression, destination type and conversion
kind: lvalue conversion, array/function decay, arithmetic conversion, assignment
conversion or default argument promotion (§6.3, pp. 42-49, PDF pp. 54-61).
A stable counting sort at the end of analysis groups the records by expression
ordinal, so `conversions_of(index)` and `expression_conversions(node)` return
one contiguous run in application order. Records whose operand has no retained
result, which only recovered syntax produces, follow the last run.
Truth-value contexts record no conversion beyond lvalue conversion or decay:
the operands of `!`, `&&` and `||` and controlling expressions keep their own
type, and lowering compares them with zero (§6.5.3.3p5, printed p. 79, PDF
p. 91).
These are contextual operations, separate from the expression's original category;
sizeof and unary address operands therefore keep their unconverted identities.
Compound assignments also retain their arithmetic/pointer operation type before
the final conversion to the left operand's type. The backend must evaluate that
left operand once (§6.5.16.2p3, p. 93, PDF p. 105).

Integer promotions preserve enum and bit-field rules. Usual arithmetic conversions
use `ExpressionInfo::promotion_width`, separately from the `bit_field` designation
used by address/size constraints. Comma forwards the right operand's width;
simple/compound assignment and prefix updates forward the left/updated operand's
width. Parentheses and designation-preserving selections retain it. Conditional
arithmetic results use the already-promoted common type. Casts and postfix updates
have ordinary declared-type results, matching Clang. `sizeof` and `_Generic`
therefore observe their standard unpromoted types and designation constraints.
Narrow extended integer bit-fields also follow Clang's width-based promotion.
This compatibility choice implements C99 §6.3.1.1p2 (printed pp. 42-43; PDF
pp. 54-55) for value-producing wrappers, where GCC and Clang preserve the field's
promotion behavior. All arithmetic/comparison/default-argument consumers use the
same promotion helper; constant folding and lowering consume its retained operand
conversions. No downstream expression walk re-infers the width. Arithmetic types
preserve integer rank even where LP64 long and long long have equal widths, and
preserve real/complex component precision. Assignment conversion checks immediate
pointed-to qualifier inclusion and exact compatibility at deeper pointer levels;
it supports object/void pointers, null pointer constants and pointer-to-bool.
Function prototypes enforce argument count and assignment-compatible arguments;
unprototyped and variadic arguments receive default promotions. Core unary,
postfix, binary, conditional, assignment, comma, cast and compound-literal
constraints use the same helpers (§6.5, pp. 69-94, PDF pp. 81-106).

Issue #46 (2026-10-14) adds Clang 23 pointer constraint classification in
`expressions/pointers.rs`. `assignment_conversion` first checks the standard
constraint and then classifies its violation once; assignment, initialization,
ordinary prototype arguments and return share that adapter. Intrinsic-specific
contracts retain strict `assignment_compatible`. Configuration diagnostic policy
selects warning/error/suppression with Clang group names, defaults and parent
group controls. Warnings retain ordinary conversion records and do not increment
semantic error counters or taint initializer recovery. Errors block lowering.
The qualifier classifier walks pointer/array derivations iteratively and does
not weaken unrelated terminal types, function signatures or array bounds.
The default/severity table and flag semantics are in `language-standards.md`.
Global `-Werror` is also applied to existing diagnostics from every phase at the
shared rendering/counting seam, so error counting blocks lowering before output.
This changes no call/designation/tail/member facts or downstream lowering.

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
`MemberFacts` retains the owning tag, field index, qualified access type,
guaranteed place alignment and bit-field width. `member_facts` and
`selected_field` use the normalized designation and expression indices;
neither query peels wrappers or recovers the owning record from base syntax.
Parentheses, `__extension__`, generic and choose-expr selections preserve the
member identity. The former relative field index in `ExpressionInfo` is deleted.

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
A thread-local object's address is not an address constant (C11 §6.6p9).
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
Unions initialize one selected member; unnamed bit-fields do not consume
ordinary initializer positions. As a GNU extension (`FlexibleArrayInitializers`),
a static object's own list may give its flexible array member elements: the
binding records a `FlexibleArray` (`SemanticTranslationUnit::flexible_array`)
with the member's completed array type and an array type with room for every
element any initializer of it writes, and its storage size
(`flexible_object_size`), the larger of the structure's size and the completed
member's end, while its type and `sizeof` stay the structure's. A later list or
string for the whole member overrides the earlier ones (§6.7.8p19), so the
object ends with the last one's extent as in Clang; an element brace elision
reaches replaces only that element. Lowering writes the initializer into a
buffer of the written extent and cuts it to the storage size. An empty list may
initialize any flexible array member, as in Clang; any other initializer of
one, in an automatic object, a compound literal or a nested structure, is an
error. A designator naming the member takes only a brace-enclosed list or a
string literal ("flexible array requires brace-enclosed initializer"), and a
designator into its elements is an error at that designator ("designator into
flexible array member subobject"), both with Clang's note at the member.
`initializers/overrides.rs` records which subobjects each initializer has
written and reports Clang's `-Winitializer-overrides` warnings (group
`initializer-overrides`): a replaced subobject at the new initializer, a write
inside a subobject an expression initialized whole at the designator that
enters it (for every such write), and a replaced union member at the element,
each noting the earlier initializer. Elements of a GNU range share their
initializer's history; a range of more than 65,536 elements records only its
last element. Excess elements,
incompatible scalar values and invalid designators have structured diagnostics.
Narrow/wide strings initialize their matching character arrays, with optional
braces and the permitted omitted terminator in an exactly sized array.

#### Initializer placement plans

**M8 is complete.** The same traversal emits the placement it accepts:
`facts/initializer_plans.rs` retains one `InitializerPlan` per accepted
initializer of an object definition, automatic declaration or compound
literal, keyed by the initializer's syntax identity
(`SemanticTranslationUnit::initializers.plan`); `initializers/plan.rs` builds
it while the checker's cursors move. Each cursor carries its container's
absolute offset, accumulated qualifiers and enclosing unions, so no layout is
searched again. A plan lists, in source order:

- `Zero` for each brace pair around an aggregate, with the exclusive index
  where its list ends (omitted bytes are zero, §6.7.8p21, and the pair
  replaces earlier writes, §6.7.8p19);
- `Select` whenever an element enters a union member (designated, positional
  or by brace elision), with the scope of the list containing the element;
- `Store` (scalar, with its bit range), `Copy` (a whole structure or union,
  §6.7.8p13, with the union types that contain it) and `String`;
- `Repeat { count, stride, length }` for a GNU range designator, one per range
  in a designation, over the operations of its element stored at the range's
  first element; operations before the range step (selections of containing
  unions) stay outside, so nested ranges repeat inside each other;
- `EmptyScalar` for `{}` around a scalar and `Unmodeled` where brace elision
  descends into a vector or an unanalyzable type, which lowering refuses.

Values are `InitValueId`s of the initializer occurrence; a value inside a
repeat with more than one element is marked `repeated`, so it is evaluated
once. A root array of unknown size and the object's own flexible array member
get their completed (respectively written) array types when the checker
finishes; another flexible array member's empty list stores nothing. Rejected
or unmodeled initializers retain no plan. A plan's size follows the written
initializer and its designators, never an array's extent or a range's length
(lowering copies a range's first element into the rest where it can; see
`middle-end.md`);
`tests/initializer_plans.rs` pins plan shapes and counts the producer's work
(`review_steps[3]`) for deep, sparse and overlapping-range initializers. See
the [M8 receipt](docs/research/semantic-facts-m8.md).

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
  count queries, attribute-derived types and newer-standard
  special values remain conservative unknowns. Core operator checks do not
  implement GNU void/function-pointer arithmetic.
- Floating constants follow the IEC 60559 arithmetic of Annex F, as GCC and
  Clang do on this target: overflow and division by zero give infinities or
  NaNs, negation keeps the sign of zero, and a NaN compares unordered. These
  are valid arithmetic constants; only converting a nonfinite value to an
  integer type is diagnosed. Complex multiplication and division use the native
  C arithmetic bridge at the common real precision, including its recovery
  behavior for huge/tiny values and infinities. Mixed real/complex expressions
  preserve their operand domains. Full Annex G conformance and floating
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

`SemanticTranslationUnit.functions` retains one `FunctionRecord` per function
definition in traversal order, nested GNU definitions and recovered ones
included: the `FunctionDefinition` syntax, its binding, its result type, its
function scope and the body bindings of its parameters in declarator order (the
identifier list for an old-style definition). Unnamed parameters and repeated
names have no binding; a lone `void` has no entry. An old-style parameter's
binding keeps its declared type while the binding of the function holds the
composite type with promoted parameter types, so lowering converts each
argument on entry (§6.9.1p10, printed p. 142, PDF p. 154).

The record retains original parameter types, source-ordered parameter-site typeof
and new-bound actions, and prototype/body parameter binding pairs when replaying
the definition's prototype scope. Runtime bounds use those prototype bindings;
lowering aliases them to the initialized body parameters in work proportional
to the definition's parameters. The 10,000-function regression bounds alias
visits directly, independently of machine speed and unrelated bindings.

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
may shadow it; the function's outer block may not redefine it. GCC's
`__FUNCTION__` and `__PRETTY_FUNCTION__` are not declared at entry: lookup that
finds no declaration resolves them (`predefined.rs`), `__FUNCTION__` to the
`__func__` object and `__PRETTY_FUNCTION__` to an object made at its first use
in the function, holding the signature as Clang prints it in C, with typedef
names as written. The object is installed in the function scope, so later uses
find it without another build. Outside any function all three warn at each use
and share one file-scope object (an empty string, or `top level`). Hosted `main`
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
addresses count as references and make their label a possible computed-goto
target (see [Labels as values](#labels-as-values-2026-10-14)).
Duplicate labels are diagnosed immediately, and unresolved references after
the translation unit has been walked. The parser retains ownership of the
duplicate-default diagnostic, so it is emitted once. Case/default placement
and case ICE requirements are checked semantically. Case values are converted
to the promoted control type; GNU inclusive ranges use the same conversion.
Every case label is retained, including an empty GNU range for fallthrough. Only
nonempty intervals receive dispatch transitions and enter the sorted case index.
Duplicate values and overlaps
are checked in O(n log n), and empty converted ranges produce a warning.

#### Retained control and lifetime facts

Semantic analysis owns one execution-region tree per function, separate from
lexical name visibility. `FunctionRecord::control` identifies its function/root;
`SemanticTranslationUnit::control` retains typed region, statement, label, jump,
loop and switch identities. Regions distinguish lexical scopes, variably modified
declarations and GNU statement expressions. Each region records its lexical owner,
parent, function, provenance and preorder/subtree interval. VM typedefs and pointers
protect entry without allocating storage; automatic VLA objects additionally carry
cleanup obligations. Nearest VM, statement-expression and cleanup links are retained.

The ordinary semantic work stack produces these facts while visiting signatures,
bodies and all expression contexts: evaluated typeof operands and array bounds,
runtime sizeof, initializers, conditions, arguments and nested statement expressions.
There is no second syntax walk. Definition parameter expressions use their function's
root during signature analysis. Nested definitions have independent roots. Loop and
switch contexts apply only to their bodies; header jumps resolve to an enclosing
body context, and invalid header jumps retain the existing diagnostics.

Resolved labels carry identity and definition points. Goto, break, continue and
return facts record source, resolved destination and compact left/entered region
runs. Statements record normal-exit transitions. Switch records retain the promoted
control type, source-ordered cases, inclusive converted endpoints, sorted case
indices, default and no-match transitions. Cases are sorted once for overlap checks;
lowering consumes those endpoints without converting them again.

After traversal, iterative heavy-light indexing takes O(R) work/storage. Each
transition/LCA and first-entered-VM query takes O(log R), with no ancestor scan
per query and no recursion. Preorder containment checks are O(1). Forward labels
resolve before goto diagnostics. Switch diagnostics retain their discovery order;
their secondary VM labels are finalized to the outermost entered declaration.
Entering statement expressions or VM scope reads this same graph, preserving text,
order and secondary labels. GNU nonlocal jumps retain the existing conservative
semantic handling and remain unsupported by lowering. A computed goto is a
jump to `ControlTarget::IndirectGoto(function)` with an empty transition.

GNU statement-expression tails consume their expression statement's converted
value: arrays/functions decay, lvalues become values and bit-field identity does
not escape. Empty blocks and other endings yield void. Aggregate results are copied
before an exit releases VLA storage. Internal jumps preserve enclosing lifetimes;
outward jumps release only the regions they leave. C99 §6.2.4p6, p. 32 (PDF p. 44),
§6.8.4.2p2, p. 134 (PDF p. 146), §6.8.6.1p1, p. 137 (PDF p. 149),
§6.8.6.2p1/§6.8.6.3p1, p. 138 (PDF p. 150); GNU extension:
[GCC Statement Exprs](https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html).

Deep generated inputs count construction and query work rather than elapsed time.
The M1 [validation receipt](docs/research/semantic-facts-m1.md) records retained
bytes, native oracle limits and fresh matched corpus evidence. Superseded control
and effect chronology is [archived](docs/research/semantic-control-pre-m1-history.md).

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
implicit `__func__` and `__PRETTY_FUNCTION__` bindings appear through its ordinary
binding output. No
statement inspection format was added. Earlier source occurrences and expression
types retain their point-of-use types; synthesized tentative definitions identify
the final composite binding needed by lowering.

Internal-linkage expression uses require a definition, except operands of
constant `sizeof` results (§6.9p3, printed p. 140, PDF p. 152). Persistent
sizeof contexts are resolved after typing, so runtime VLA-size uses still count.
Used undefined static functions are errors; unused undefined declarations warn.
Resolved entity uses are retained in `SemanticTranslationUnit.references` with
their owning function definition or object initializer. Parameter type-site
uses belong to the function whose entry executes them, even before its binding
is installed. The producer excludes constant `sizeof`, `alignof` and
non-VM `typeof` operands after typing, using retained typeof execution facts;
linked redeclarations keep their binding identities. Each reference also
records whether Clang's code generation at `-O0` skips the code it appears in
(`dead`), which approximates Clang's insertion state rather than run-time
reachability: a skipped reference creates no LLVM value, so it ranks no
`always_inline` function in Clang's creation order and selects no DLL import,
and entity reachability instead reads whether lowering omits the reference.
Static initializer references remain live after jumps when the declaration is
emitted. Code is skipped in the arm a constant
condition does not take, in a generic selection's controlling operand and
unselected associations, after a jump or a loop without an exit, and until a
label. One evaluator stands for Clang's `ConstantFoldsToSimpleInteger` in
every condition the flag folds (`if`, `switch`, scalar `?:`, `&&`, `||`), which
must have an integer or enumerated type, as `EvaluateAsInt` requires: an
integer constant, a comma whose left operand evaluates, a statement expression
of side-effect-free expression statements, a `const` non-volatile object that
is not weak with a constant initializer (converted to its type; the address of
an automatic, thread-local or `dllimport` object is no constant there, nor a
`?:` operand Clang selects), a scalar inside such an array or
structure whose initializer evaluates (read from its initializer plan), an
element of a string literal, the address of a non-weak object or function
(compared within one object in bytes, and distinct from a string literal
unless one may be one past its end while the other is at its start, or the
object is an array of unknown or zero size, as Clang's evaluator refuses),
`__builtin_expect`, `__builtin_strlen`, `__builtin_abs`, the count bit builtins
and byte swaps, and `__builtin_constant_p` as Clang's evaluator computes
them (1 only for an arithmetic operand that folds or a pointer to a literal's
first element) (a volatile read, also through `*`, `[]`, `.` or `->`, is a side effect),
but no call, assignment, volatile or other object read, and no condition
holding a label. Semantic analysis's own constant is trusted except around
`__builtin_constant_p` and weak entities, whose GNU-mode fold Clang's
evaluator does not make, and floating arithmetic with a NaN result, which it
refuses as undefined; a `sizeof`, `alignof` or `_Countof` operand does not
count, as only its type is read. An entity is weak for a body when `#pragma
weak` names it or a declaration, at file or block scope and before any
definition of the entity, writes `weak` or `weak_import`: before the end of
the body for a function Clang emits at its definition (a used, constructor
or destructor body, or a nondiscardable external definition under the
inline rules and declarations seen so far), and anywhere in the unit for
a deferred body. The pragmas and the ends of the
external declarations are compared by output-token ordinal. The evaluator
recurses through operands and object initializers, so it follows them at
most 64 levels deep, which an unoptimized build runs within a 1 MiB stack;
deeper conditions are left unfolded, emitted as without the evaluator, and
`noreturn` types are followed as far. Loops instead test the value IR emission produces. Two forms
take more than the statement walk (`emission/`). A `switch` whose
controlling expression folds emits only the statements Clang's
`CollectStatementsForCase` gathers from the matching case, else `default`
(§6.8.4.2p4-5, printed p. 134, PDF p. 146). Analysis still runs in source
order: the body is analyzed as unfolded until a label is selected, which then
marks the references recorded since the body began as skipped and the object
definitions and statement facts there as elided, except the blocks and case
labels leading to the selected label and statements holding an ordinary
label; a `default` is
selected tentatively only when collection from every later case succeeds.
A later matching case replaces it and elides the default region too.
Otherwise, or past 64 later labels, the switch stays unfolded, which may
retain more code than Clang. A retained generated-control fact directs
lowering straight to the chosen label when collection succeeded, so unselected
case labels, also those after the selected `break`, are never lowered; only a
tentative `default` keeps the case labels after its `break`, since a later case
may still replace it. An `if` with an elided arm lowers only its selected arm.
Every lowering consumer of a conditional or logical operator, as a value, as a
branch condition or in an effects-only recipe, reads the same operand elision
facts before ordinary constant shortcuts. The common operand of GNU `x ?: y`
is always emitted, as Clang binds it before folding. Conditional elision waits
for the result type: Clang's aggregate and complex visitors emit both arms.
VM composite pointer results conservatively retain both operands, preserving
main's behavior until their bound selection is repaired. Branch simplification
operands live in sparse retained facts, keeping expression records at 160 bytes.
In an `if` branch
condition, the recursive logical emitter instead simplifies only `1 && X`,
`X && 1`, `0 || X` and `X || 0`. Nested `0 && X` and `1 || X` retain
both operands unless the whole condition folds. Branch operands record their
regions so a later simplification omits references, definitions and statements
together; lowering reads the retained operand decision. Constant-query integers
take Clang's constant-context value, so semantic integer facts, case labels,
static assertions, static initializer data, standalone values and loop
decisions agree: a comma whose left operand is not constant, and an element
read through a non-`const` array compound literal, are not constant there.
Function code still yields one for both, as in Clang, whose non-constant fold
reads the compound literal and whose `llvm.is.constant` sees the comma's
constant result: the expression records that its emitted value differs, so
lowering evaluates the operators around such a query instead of taking their
constant. Control folding accepts the array read but leaves a discarded-comma
query unknown, since `ConstantFoldsToSimpleInteger` cannot discard its left
operand. Nested queries also stay unknown in control contexts when evaluating
the inner query fails; the constant-context nested-query shortcut is not a
control fold. Loop folding also treats the condition as unknown, and `__builtin_choose_expr` and static initializer data
select by the constant-context value. A discarded comma counts only before an
arithmetic constant; a pointer result is never constant in function code. An unknown query still
emits zero while the control fold can remain unknown.
String-address queries require each integer cast to have exactly the pointer width;
widening an integer after a narrowing cast does not recover the address.
When lowering omits a folded condition, its executable references are unlowered
too; nested static declarations keep their initializer-data references. After
selection, labels inside the selected statements resume nothing, and the code
after the switch follows the selected statements' end. When Clang's collection
fails (a label or declaration it would skip, a `break` inside a collected
statement other than a block, a case range after the match, a case label it
cannot reach through blocks and labels), the switch is emitted unfolded, as
Clang does. Labels and breaks in statement expressions count, as Clang's
`ContainsLabel` and `containsBreak` walk expressions. An expression statement
ending in a call that does not return skips what follows until a label
(`EmitStmt` erases the block without predecessors the call left): a call to
a function declared `noreturn` (GNU, `gnu::` and `__declspec` spellings,
function type attributes, or `_Noreturn` and standard `[[noreturn]]`,
declaration attributes, C11 §6.7.4p8), to `__builtin_unreachable` or
`__builtin_abort`, to a hosted library function Clang's builtin table marks
(`abort`, `exit`, `_Exit`, in GNU modes `_exit`, and `longjmp` or
`siglongjmp` after their buffer typedef), or through any callee whose type
carries the function type attribute: pointers, arrays, members, call results,
casts, typedefs and `__typeof__`, and a conditional when both operands do. A
written type attribute belongs to the function derivation nearest the declared
name, or, after a `*`, to the function type that pointer points to; one a
typedef name or `__typeof__` brings belongs below the declarator's function
derivations, so `NP (*gp)(void)` returns a `noreturn` pointer rather than not
returning. The rest of the same expression is still emitted. Code generation
still visits a block, a label or a declaration after such a call or a jump,
but not a statement it skips whole (any other statement there, which holds no
label), the arm or operand a folded condition skips, or what follows a folded
switch's selected statements; a block-scope static object declared in code it
visits is emitted with its initializer (`EmitStaticVarDecl`), so that
initializer's references are not skipped, and they count as made by the
function where it declares the object. An expression whose paths join in
a block with predecessors ends that state: a `&&`, `||` or scalar or `void`
`?:` whose condition does not fold (unless the `?:` selects between constants,
which a `void` operand is not), a complex `?:` or an ignored-result aggregate `?:`, a product of two
floating complex values, an atomic update `atomicrmw` cannot perform (an atomic
`_Bool` decrement among them), an atomic compare-and-exchange, an atomic
builtin whose memory order is not a constant (unless a library call copies its
object), a `__builtin_fpclassify` that does not evaluate, and a System V
`va_arg` of a type passed in registers (its members judged by their types'
natural alignment, as the ABI classifies them, with no class for an array of no
elements or an unnamed bit-field). A call that does not return changes nothing
in an unevaluated or skipped operand, the operand `__builtin_choose_expr` does
not select among them, nor in an operand of `__builtin_constant_p`, the
object-size builtins and `__builtin_classify_type`, but a join in an operand
Clang emits still counts: a variably modified `sizeof` or `typeof` operand, and
an operand of `__builtin_constant_p` or the object-size builtins without side
effects.
Where Clang emits an operand after
one this analysis visits later (the left operand of a scalar or compound
assignment, or on the Microsoft ABI the arguments before a call's last), a join
in it ends the state too. These rules read the expression's type, so
they run once it is analyzed. A statement
expression's value statement is emitted even after a jump, and one whose body
ends skipped ends its expression statement's code only where it is emitted
itself: not in a skipped or unevaluated operand. A declaration's
initializer does not end the code after it. Neither does a block-scope object
or typedef of a variably modified type, although Clang's
`EmitVariablyModifiedType` ensures an insertion point there and emits the
array sizes and the code after them: the rule needs to know that Clang reaches
the declaration at all, which conditions only Clang folds, the unselected
operand of `__builtin_choose_expr` and a switch without a match can hide, so
such code stays skipped after a jump, as before #113. After a call that does
not return, a declaration that may be variably modified still ahead in the
enclosing blocks keeps the code after the call emitted instead, as it is
without the model of such calls. A typedef name counts as variably modified
there when a block-scope typedef of the function may give it such a type, so
that typedefs coming into scope later cannot change a kept answer. Floating
object reads and explicit casts use the same target rounding as semantic
constant evaluation, including narrowing, underflow and complex-to-real
conversion; unsupported formats stay unfolded. Labels that prevent omission
also preserve the arm's insertion effects and any merge. VLA bounds in
declarations, typedefs and `sizeof` type operands take part in label
collection. Conditional values, including GNU omitted-middle forms, convert to
the result type before further folding. `__builtin_constant_p` checks only
selected generic/choose operands for side effects, walks statement expressions
and honors direct pure/const calls; its operands and those of
`__builtin_classify_type` create no emitted reference. Aggregate read coverage
uses a sorted endpoint sweep, and constant-array `strlen` results are memoized
with their evaluation-depth and weak-horizon dependencies. Literal lengths
reached through constant pointers reuse the decoded literal and offset.
Bit-field and union reads, GNU ranges in `const` initializers and the
unselected operand of `__builtin_choose_expr` remain unmodeled. Distinct-object
address comparisons such as `&x1 == &x2` also remain unfolded. Constant queries still miss `(long)"a" + 1 - 1`, `(long)"a" - 0`,
`(char *)8`, `!(long)"a"`, `(long)"a" == 0`, `(long)"a" != 0` and `__func__[0]`.
These gaps can leave code emitted that Clang skips. For a pure condition
built from constants, addresses and nonvolatile const-object reads whose
value remains unknown, optional local-static retention falls back to the
required/`used` closure in the affected if, value-folding conditional, logical or
foldable-switch region. Deferred definition contexts keep ignored aggregate
and complex conditional visitors and neighboring declarations independent.
This is a retention capability boundary, not a new value or branch fold;
required references and `used` storage remain checked. Parameters and
ordinary runtime loads do not trigger it. The VLA-after-jump gap above
instead omits code Clang revives.
Scalar conditionals omit an unselected constant arm. Aggregate and complex
visitors emit both arms, including member bases, arguments, assignments and
return values. Clang can bypass the aggregate visitor for a constant automatic
initializer; that decision waits for the complete typed initializer plan. A
dynamic sibling member prevents the bypass. A separate potential-constant
initializer scan only limits optional retention when a pure condition needs
an unmodeled fold; it never chooses an arm. Required-only retention propagates
through helper references. A certain path to the same helper restores its
optional locals, and required/used entities stay reachable on either path. Fully
overwritten automatic initializer values use that same required-only policy.
A memoized backwards scan compresses written bit intervals and processes each
interval once; unknown union/range operations conservatively retain values.
It does not change the pre-existing evaluation of overridden initializers. A sparse discarded-result context
follows parentheses, comma
results, selected-expression wrappers, statement-expression value tails and
void casts; expression statements and for initializers/steps start it. Arm
omission is resolved after typing, so nonvoid statement-expression tails stay
available to lowering and omitted tails cannot leak insertion state.
`functions.elided` omits lowering traversal and storage collection
for whole skipped statements and operands; there is no separate `unemitted`
state. A missing insertion point (`dead`) alone still permits simple
statements to collect storage. In `0 && ({ L:; static int a; 1; })`, the
label prevents elision and preserves the static even though the operand is
not evaluated. A constant-query operand has one deferred emission decision, shared by
references, nested definitions and joins. After the ordinary constant fold
fails, Clang emits a real arithmetic operand without side effects, retaining
all its referents; other operands are unevaluated. Within a static or thread-local initializer, the query emits only
its integer value: its operand declarations and all their references are omitted.
Either way its operand is still used in an expression, so an internal
identifier it names needs a definition (C99 §6.9p3, p. 140; PDF p. 152).
A dead automatic initializer holding a label gives Clang an insertion point
(`EmitAutoVarInit`), so the statements after its declaration are emitted.
Declarations after a folded switch's selected break are elided just like its
statements. A void statement-expression tail skipped after a jump retroactively
elides its definitions, statements and references, and lowering supplies its
void result without visiting the tail.
O0-only local-static retention is best effort for unsupported constructs.
A single retry boundary spans the complete output preparation: lowering,
optimization and inline resolution, ABI lowering, verification, backend
support, native frame/encoding and object construction, linking, and
interpreter global/relocation loading. Any refusal retries once without
unreferenced, non-`used` locals and the entities only their initializers
reach. Text and retained-call diagnostics are buffered until an attempt is
accepted. Interpreter preparation loads storage without running constructors
or user instructions; execution happens once, after the retry boundary.
Semantic diagnostics, including emitted BSS initializer checks, still run
before this capability fallback. Required and `used` entities retain their
support checks and diagnostics. The native backend also omits unused internal
TLS objects in named sections it cannot support, and allows an unread internal
initializer to hold a returns-twice function's address. Incoming global
references from instructions, data and aliases are collected once per module
and shared by native support and storage emission.
Static creation-order provenance indexes current writers by bit position,
snapshots only overlapping source runs for range copies, and skips replay for
owners with no emitted symbol references. The module-item planner
joins those identities by emitted symbol and computes the closure of external
definitions with an explicit worklist. Semantic definition constraints still
apply to every analyzed definition, including bodies omitted from emission.
Retained assembler-label declarators and attribute capability refusals also
identify their enclosing function definition. Lowering reports a refusal or
label error only when that definition is emitted; file declarations remain
checked. A `cleanup(f)` variable records a use of `f` by its body. Symbol
aliases are resolved before the planner joins linked entities.
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
  graph; the computed-goto target set is retained per function.
- GNU statement-expression result types, intrinsic signatures, local-label
  nonlocal-jump mechanics, other attributes and calling-convention
  effects remain conservative. The five requested GNU statement forms retain
  parser-owned extension policy; accepted opaque constructs are not certified.
- The existing parameter-inspection metadata still uses source provenance;
  definition binding and K&R comparisons use suffix identity. Lowering reaches
  a definition's parameter bindings through its `FunctionRecord`; other
  parameter sites, such as nested prototypes, have no retained index.
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
Unsigned high-half patterns use `u128` arithmetic and formatting. A `_BitInt`
wider than 128 bits whose value `i128` does not hold names its exact
two's-complement limbs in the analyzer's interned table
(`semantic_analysis/wide_integers.rs`), which every folding site reaches
through `WideIntegers` and which `SemanticTranslationUnit` keeps for lowering
(`integer_limbs`, `integer_float_bits`) and inspection. Its division relies on
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
long-double precision. `_Generic` gained semantic selection in the
binary128/type-generic work documented below. Exact
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
encoded u/U literal semantic types, target instruction lowering and a portable
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

## C11 atomics (2026-10-09)

`builtins/atomics.rs` owns the type-generic intrinsic classification and checking.
`TypeKind::Atomic(TypeId)` preserves identity independently of cv/restrict
qualifiers. Eligibility is checked after the work stack resolves the operand
type. Compatibility descends through atomic nodes with explicit continuations.
Atomic object layout follows the pinned Clang's default x86-64 targets:
sizes up to 16 bytes round up to a power of two with at least that alignment;
larger objects keep the underlying layout. No instructions or runtime
synchronization are emitted.

The Clang C11, GNU atomic and legacy sync builtins share argument checking but
retain their distinct pointer contracts and arities. C11 fetch-add/sub accept
atomic integer, pointer and float/double values; bitwise fetches require
integers and min/max accept integers or float/double. GNU scalar forms exclude
atomic pointees; GNU generic forms accept complete ordinary object types.
Legacy sync forms accept integer/pointer objects and their ignored variadic
arguments. Memory-order constants are checked where Clang checks them, with
structured warning diagnostics suppressed in system headers. Nonconstant
orders are accepted. Lock-free queries preserve Clang's distinction between
compile-time `always_lock_free` and runtime queries that cannot be folded.

`generic.rs` selects a compatible association or default and propagates the
selected expression. The existing integer evaluator now consumes that result
without evaluating the controlling expression or unselected associations.
Atomic casts remain non-ICEs; atomic size/alignment assertions are evaluated.
Unit tests check type identities and every modeled call's retained type.

The embedded header is adapted from the pinned Clang header with its
Apache-2.0 WITH LLVM-exception license retained. `#pragma clang deprecated`
single-name markers supply C17 `ATOMIC_VAR_INIT` warnings, and are cleared on
undefinition or replacement. C23 removes that macro and adds char8 atomics.
The installed MSVC 14.44 toolchain supplies `stdatomic.h`, which delegates in C
to `vcruntime_c11_stdatomic.h` and requires C11 with atomics enabled. Despite
that newer C implementation, the pinned Clang resource header still bypasses
the MSVC header in C mode, so bcc does too. The Windows SDK UCRT directory
itself has no competing `stdatomic.h` in this installation.

The shared target probe exercises C11/C17/C23 for all four triples, with
aggregate/scalar layout assertions, `_Generic` result checks, pointer/floating
fetches and all three builtin families. Allocation coverage includes the
header, type construction, calls, generic selection and every golden diagnostic.

The `target/libc-survey/atomic/results.json` comparison with `gnuc` has 448
shared rows: **8 newly accepted, 0 newly rejected**. `stdatomic.h` is newly
accepted for glibc, musl and MinGW in C17/GNU17; musl's two combined standard
header units also become accepted. MSVC's two `stdatomic.h` rows remain accepted.
The remaining first errors are glibc's unsupported `<tgmath.h>` type
combination, MinGW's missing `<tgmath.h>` and 13 MinGW headers needing
`<x86intrin.h>`. Those sibling features are outside this change.

## Binary128 and type-generic math (2026-10-09)

The scalar graph includes real and complex IEEE binary128 with arithmetic
rank above long double. Explicit `__float128` declarations are accepted on
Linux GNU, Linux musl and MinGW (16-byte size/alignment; complex 32/16), and
rejected on MSVC. Clang nevertheless accepts `q` literals and a `typeof`
type derived from them on MSVC; bcc preserves that distinction. The target
macros mirror the pinned Clang exactly: `__SIZEOF_FLOAT128__` on the GNU
triples, `__FLOAT128__` on Linux, no `__FLT128_*__` inventory and no MSVC
capability advertisement. GCC 4.2.1 identity remains unchanged (bcc later also
claimed Clang 23.1.1).

Native `_Float128` and `f128`/`F128` literals are deliberately not added:
Clang 23.1.1 rejects them in every tested mode (C11/C17/GNU17/C23/GNU23).
The reachable glibc path supplies `_Float32`, `_Float64`, `_Float32x` and
`_Float64x` as typedefs of the existing standard types. A private embedded
`bits/floatn.h` uses `include_next` and adjusts glibc's old-GCC binary128
capability decision, providing its `_Float128` and complex typedefs, suffix
macro and old-compiler builtin aliases. (Since 2026-10-15 bcc claims Clang,
glibc's own `bits/floatn.h` takes its Clang path and the bridge is gone; see
[language-standards.md](language-standards.md).) System headers stay read-only.
GNU-source math and stdlib probes cover quad sqrt/pow/fabs, complex sin,
`strtof128`, binary128 infinity/NaN initializers, fabs/copy-sign builtins,
and the standard float/double/long-double selections. Clang accepts the
`f128` builtin names and rejects the old GCC `q` builtin names; the bridge
keeps the native names for fabs and copy-sign. Their binary128 results
retain constant eligibility and, since 2026-10-27, fold to their bits. The bridge
requires ordinary resource-before-libc search order; explicit earlier
`-isystem` directories can bypass it.

`binary128.rs` converts literals without any host float: retain all digits
in arena-backed base-2^32 integers, represent decimal input as a rational
scaled by powers of two/five, and divide/round once at 113 significand bits
(or the fixed subnormal quantum 2^-16494). Ties round to even. Zero, the
minimum subnormal, half-ULP boundaries, maximum finite value, overflow and
underflow have bit-exact unit coverage. An independent batch of 250 decimal
and 100 hexadecimal literals matches Clang IR constant bits exactly. Token storage remains packed at the
existing alignment and all compiler allocation stays in arenas.

This first implemented a limited constant-evaluation fallback in which
binary128 arithmetic, comparisons and casts were not numerically folded.
Since 2026-10-27 they fold exactly; see
[Binary128 arithmetic and folding](#binary128-arithmetic-and-folding-2026-10-27).
The x87 helper and Rust's unstable native `f128` are still not used.

GNU `typeof` preserves operand type without array/function decay;
`typeof_unqual` removes top qualifiers, qualifiers carried by array elements and
a top-level atomic wrapper (C23 §6.7.3.6p5). `__builtin_types_compatible_p`
ignores the same top-level and array-element qualifiers, `__builtin_choose_expr` requires an ICE and transfers the selected
expression's type/lvalue/constant eligibility, and `_Generic` checks distinct
complete non-variably-modified association types and selects the matching or
default expression. `__builtin_classify_type` accepts one expression (also
through a parenthesized name) and folds Clang's integer/bool/pointer/real/
complex/record classes without evaluating it. Real/imaginary components
preserve complex lvalues, qualifiers and static-address eligibility; a real
operand's imaginary part is zero but keeps the operand's constant-expression
restrictions. Selections record the selected operand, so address constants,
GNU-folded address differences and vector-lane address restrictions survive
`_Generic` and `__builtin_choose_expr`. A failed choose condition suppresses the
dependent ICE diagnostic, and math builtin dispatch yields to a shadowing
declaration. The ordinary explicit work stack visits all syntax operands.

The reachable old-GCC glibc `tgmath.h` path needs these primitives rather than
`__builtin_tgmath`, which the pinned Clang rejects. The embedded LLVM 23.1.1
resource `tgmath.h` now exercises arithmetic promotion overloads, real/complex
selection and an unavailable variadic fallback. Resource-header regressions
compare C17, GNU17 and C23 on all four targets against pinned Clang. Its
binary128 calls remain ambiguous in both compilers because the resource
header does not declare a binary128 promotion overload.
Overload semantic identities remain separate from emitted ABI names. A final
source-order reachability pass assigns anonymous and repeated local tag ordinals
at definition signatures and emitted references, skipping unused declarations
and discarded static bodies. Eager external bodies interleave with signatures;
reachable internal bodies and alias targets follow deferred traversal. Alias
requests retain stable semantic identities, then resolve their strings against
the finalized emitted names, including chains through overloaded aliases.
ELF/MinGW use Itanium names, while MSVC uses Clang's Microsoft C linkage names,
including shared name/argument backreferences and sparse written function-type
metadata for nested parameter qualifiers and array adjustment. This metadata
follows declarators and typedef bindings without changing canonical type
identity. Bidirectional BCC/Clang multi-TU regressions compile and link all four
targets and execute on a native host. Unique `enable_if` functions use Clang's
ABI spelling, including plain C names for unmarked functions. Microsoft names
omit enable predicates; unmarked C names likewise cannot distinguish predicates.
Only actual symbol collisions use distinct private semantic names to retain
both bodies. Clang cannot represent those colliding APIs interoperably. Its
pinned Microsoft mangler also crashes for anonymous records introduced only in
a prototype; bcc uses a deterministic private spelling for that unshareable type.
The implementation also repairs the glibc macro pattern `cast function ##
f128`: a multi-token paste operand is pasted at its final token before its
prefix is rescanned, protected by a reduced preprocessor regression.

Diagnostics cover unsupported target spellings, extension policy and
suppression, literal ranges, unsupported suffixes, invalid generic selections
and non-ICE choose conditions. The shared Clang/bcc target fixture uses actual
`_Static_assert` and `_Generic` evaluation on all four triples; GNU-source
glibc integration is checked in both C17 and GNU17 when the sysroot is present.
Allocation coverage includes exact conversion, unfolded arithmetic and error
paths.

## GNU vectors and x86 SIMD resources (2026-10-09)

The canonical type graph now retains a vector's arithmetic element, lane count
and alignment. `vector_size` accepts integer/real scalar elements and positive
multiples of their size. Clang also accepts non-power-of-two lane counts: their
object size and natural alignment round to a power of two, while their lane
count remains unchanged. Explicit `aligned(N)` and bare `__aligned__` on vector
types override alignment; `may_alias`, `target` and `min_vector_width` parse and
leave type identity intact. These contracts extend C99; the primary references
are [GCC Vector Extensions](https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html)
and [Clang Vectors and Extended Vectors](https://clang.llvm.org/docs/LanguageExtensions.html#vectors-and-extended-vectors).

Vectors support function parameters/results, arrays/records, brace
initialization, lvalue subscripting, casts between equal-bit-width vectors, unary arithmetic and
integer complement, element-wise arithmetic/bitwise/shifts/comparisons, and
compound assignments. Comparison masks contain signed integers of each lane's
width and natural alignment. The conversions between vectors, the scalar
operands that splat, increments, vector element addresses and `?:` follow the
claimed identity's compiler (2026-10-29; language-standards.md, "GNU vector
values"): the Clang identities use Clang's default lax vector conversions,
under which vectors with equal bit widths convert implicitly, including mixed
element types, and a scalar as large as the vector reinterprets its bits; the
GCC identity accepts only compatible lane types, except for a comparison's
opaque mask (`ExpressionInfo::opaque_vector`). Padded object size alone does
not make three-lane and four-lane vectors convertible. Logical operators and
vector conditions are rejected in C. The operators record their operands'
lvalue conversions, so lowering reads them as any other operand.

Vector operands may be discarded with a void cast, selected by a scalar
conditional with compatible vector alternatives, and copied by a single braced
initializer. Binary128 scalars splat at 113-bit precision without passing
constants through long double; complex elements are rejected. Integer-only
operators check both operands, and shifts require equal lane counts. Value
operations remove atomic wrappers while assignment checks preserve operand
categories. Attribute scanning skips argument identifiers. Builtins propagate
unknown operands without cascades and retain constraints through parenthesized
callees; two-operand shuffles preserve the first vector type, x86 immediates
convert to parameter types before constraint checks, and elementwise min/max
reject Boolean operands.

`__builtin_shufflevector`, `__builtin_convertvector`, `__builtin_bit_cast` and
the overloaded elementwise/nontemporal builtins used by the resources have
operand-dependent typing. The generated registry shard `src/builtins/x86.rs` declares 384 x86 builtins (two population-count calls merge into scalar descriptors) from
canonical Clang call signatures, with constant-operand positions and immediate
ranges (including gather scale choices). `scripts/generate_x86_builtins.py`
derives the catalog from names called by the shipped headers, queries Clang's
AST with discovery calls, independently probes constant arguments and ranges,
and compiles a valid signature assertion/call for every entry on all four
supported targets. This avoids handwritten per-instruction semantic branches
and does not need the missing Clang builtin definition sources.

The embedded resources adapt Clang 23.1.1's 3DNow, AES/PCLMUL, AVX/AVX2,
population-count, CPUID and MSVC intrin/intrin0/ADC headers, retaining their
Apache-2.0 WITH LLVM-exception notices; the MMX through SSE4.2 and CRC32
headers are bcc's own plain-C implementations since 2026-10-30
(language-standards.md, "x86 intrinsic headers"). The adaptations remove
half/bfloat typedefs, omit Clang module-building conditions, spell GNU asm
unambiguously when MS extensions are enabled, and give MSVC inline forward
declarations the linkage of their definitions. bcc's umbrella
headers include this supported subset. Baseline predefined features remain
x86-64/SSE2; target-attributed bodies and AVX/AVX2 calls are checked without
backend feature gating, matching Clang's syntax-only acceptance. No instruction
selection or execution is promised. MSVC intrinsic resources forward to the
libc on non-MSVC targets through include_next. Twenty-eight MSVC compiler-provided
interlocked and ISO volatile interfaces are declared with separately Clang-verified
signatures so replacing intrin0.h preserves the native C17 stdatomic.h path.
The prefetch builtin checks its pointer and optional constant operands.

Known boundaries: AVX-512, AMX, newer x86 instruction families, _Float16/bfloat
vector typedefs and ext_vector_type are not shipped. `vector_size` arguments
are parsed and analyzed as integer constant expressions, as `aligned` ones are
(2026-10-29); the iterative integer-literal/enum/arithmetic reducer remains for
an argument analysis did not keep.
GNU alignment operands use semantic constant evaluation; declaration alignment
on pointers and arrays does not change their pointee or element type.
sizeof/alignof vectors and constant scalar lane initializers evaluate normally,
and a vector compound literal initializes a static vector as a record literal
initializes a record; vector arithmetic/shuffles/casts do not retain lane
constants, so their results are not folded in static initializers or
subsequent subscripting. This is a
constant-evaluation gap, not an opaque type-checking fallback.

The final libc survey in `target/libc-survey/vector/results.json` compares 448
rows with the `gnuc` baseline: zero newly accepted and zero newly rejected
headers. All thirteen MinGW missing `x86intrin.h` first errors clear, exposing
later failures: twelve reach the anonymous-union member parser at
`objidl.h:9984`, and `intrin.h` reaches the unmodeled GNU atomic pointer return
at `psdk_inc/intrin-impl.h:1737`. The MSVC C17 `stdatomic.h` and combined-header
paths remain accepted.

## MinGW Windows header compatibility

Type-only record declarations are accepted with Clang's
`declaration does not declare anything` warning, promoted to an error by
`-pedantic-errors`. Tagged records, enums, bare typedefs and scalars introduce
no member storage or promoted names. Untagged definitions remain C11 anonymous
members; tagged and typedef record members require the explicitly enabled MS
anonymous-structures feature. This clears `objidl.h:9984` and the two typedef
declarations at `mstcpip.h:265-266` without editing MinGW headers.

GNU `extern inline` declarations with `gnu_inline` (or GNU89 inline semantics)
permit a later static declaration or replacement definition, as Clang's
`canRedefineFunction` does. Type compatibility still applies, both bodies are
checked, and an ordinary replacement definition ends the redefinition
permission. GNU inline bodies are excluded from the C99 inline-definition
restrictions. This resolves MinGW's external `__cpuidex` followed by the
resource `cpuid.h` static definition; Clang includes both definitions too.

`CompilerConfiguration::gnu89_inline()` now owns the effective emission mode:
GNU89 before C99, C99 from C99 on, overridden by `-f[no-]gnu89-inline`.
GNU extern-inline bodies with external linkage are retained as
`DefinitionKind::Inline` immediately, per body, so a later ordinary declaration
cannot promote them to external definitions. Plain GNU inline bodies are
`DefinitionKind::Function`. C99 inline-only classification still waits for all
file-scope declarations; a declaration with `extern` or without `inline`
provides the external body. The per-function `gnu_inline` attribute overrides
the global mode. Lowering consumes these retained facts for both interpreter
and LLVM emission; neither backend independently interprets syntax or flags.
C99 §6.7.4p6-7 (pp. 112-113; PDF pp. 124-125) and §6.9p5 (p. 140;
PDF p. 152); GNU extension: [GCC Inline](https://gcc.gnu.org/onlinedocs/gcc/Inline.html).

`gnu_inline` is retained only from a declaration that itself specifies inline
and precedes a unique function body, matching Clang's ignored-attribute rules.
A previously valid attribute can still be inherited by later declarations.
GNU extern-inline bodies that inherit internal linkage satisfy the required
definition check (C99 §6.9p3), while retaining Clang's permission to replace
the body. The regressions in
`src/translation_phases/semantic_analysis/tests/mingw_inline.rs` cover these cases and the
duplicate-definition and linkage errors from invalid or late attributes.

All four x86-64 targets now advertise Clang's baseline `__MMX__`, `__SSE__`,
`__SSE2__`, `__SSE_MATH__` and `__SSE2_MATH__` macros. The previously excluded
macros selected MinGW's fallback union typedefs for `__m64` and the `__m128`
family even after loading the vector resource headers. These were genuinely
different types, so typedef identity and linkage diagnostics are not weakened.
The restored macros describe front-end feature support, without code generation.

The `mingw` survey compares 448 rows against `vector-stacked`: 13 newly accepted
GNU17 MinGW headers, zero newly rejected headers in either mode. Individual
MinGW coverage is 57/58 Clang-accepted headers in GNU17 and 27/28 in the C17
standard-header subset. Separate probes accept all 13 affected Windows and
intrinsic headers in both modes. That survey predates the embedded `tgmath.h`
and arithmetic/complex overload-selection implementation described above; its
remaining-header counts are historical rather than a current implementation
limit.


### Common GCC scalar builtin coverage (2026-10-12)

The neutral registry in `src/builtins.rs` owns fixed GCC scalar signatures,
target-dependent parameter widths, fixed-width type identities, operand rules,
constant constraints and operation/effect identities. `builtins/common.rs`
checks these descriptors, with dedicated overflow and floating handlers and
conservative constant/object-size folds. Lowering consumes checked instances.
`GnuBuiltins` reports these GNU-only calls through the common extension policy; prefetch now belongs to this family rather than vector
builtins. Statement expressions retain the final expression statement's converted
value type but never discard the block's effects by folding its tail. New focused
semantic tests exercise signatures, rejected arities and types, ICE overflow
predicates, and denied-extension diagnostics. See the GCC intrinsic contract
section in [middle-end.md](middle-end.md) for conservative decisions and IR
semantics. Unmodeled names still retain opaque types and bounded refusal.

### PR #41 round-1 corrections (2026-10-13)

Intrinsic typing now uses the resolved binding's implementation-origin flag.
Every user object, function, typedef or enumerator declaration suppresses named
intrinsic dispatch; implementation-created intrinsic bindings remain distinct
from C89 implicit ordinary function declarations. This also covers vector,
atomic, binary128 and x86 immediate checks. Expression-only varargs spellings
(`__builtin_va_start`, `__builtin_va_end`, `__builtin_va_copy`) are ordinary
identifier calls and share the header operand checker. `__builtin_va_arg`
retains its reserved type-operand grammar, matching the bundled Clang probe.
C99 6.2.1p4 (printed pp. 29-30, PDF pp. 41-42) governs shadowing.

Unbound plain `alloca` is recognized only in GNU modes. Every user declaration
of that name, including a compatible prototype, definition or assembler label,
uses the ordinary-call path. The reserved `__builtin_alloca` remains available
in strict modes when unshadowed. Hosted mode alone enables implicit ordinary
library prototypes; explicit `__builtin_` library aliases keep their GCC
contract in freestanding mode too. C99 5.1.2.1p1 (printed p. 11, PDF p. 23)
and [GCC Library Builtins](https://gcc.gnu.org/onlinedocs/gcc/Library-Builtins.html)
bound these environment and naming choices.

Mode-selected GNU89 extern-inline bodies use the existing target emission rule:
MSVC retains them; Windows GNU and Linux omit them. Explicit `gnu_inline`
still omits them on all three targets, as independently checked against Clang
for GNU89 and C99 with `-fgnu89-inline`. The MSVC body uses the same IR emission
contract already used for that target's C99 inline functions. See
[GCC Inline](https://gcc.gnu.org/onlinedocs/gcc/Inline.html) and C99 6.7.4p6
(printed pp. 112-113, PDF pp. 124-125).

### PR #41 round-2 corrections (2026-10-13)

The scalar builtin execution table now covers conversion-changing operands
for all 18 fixed-signature overflow builtins: signed/unsigned add, subtract
and multiply at int, long and long-long widths. Volatile inputs keep runtime
conversion paths observable, while a literal uadd case checks truncation of
0x100000000ULL to unsigned int before adding one. Long-width probes adapt to
LP64 and LLP64. Each assertion checks the overflow flag and stored value.
The contract is [GCC Integer Overflow Builtins](https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html).

A mixed switch combines ordinary labels, an inclusive 10 ... 20 range and
default, checking both endpoints, adjacent values, an ordinary case falling
through into the range body, and another falling through into default.
Both coverage programs share the existing pre/post-ABI interpreter stages,
bcc optimization settings and four native LLVM arms, including the optional
Linux native runner. Bundled Clang independently accepts and executes both
programs at -O0 and -O2.
The new cases also pass in all interpreter stages on Linux GNU and Windows
GNU and all four Windows native arms; no overflow or switch implementation
change was needed. Linux native execution was skipped because WSL is
unavailable in this environment.

Validation passed: all-target tests, feature-enabled allocation counting
(26 tests), nightly format checking, all-target Clippy in debug, release and
benchmarking-internals configurations with -D warnings, and git diff --check.
The final all-target run includes the backward jump into a closed sibling
expression regression. The plain CLI was rebuilt after feature-enabled checks.

### PR #41 round-5 constant-argument audit (2026-10-13)

Object-size modes now convert their ICE value to the target's `int` before
checking 0 through 3. Both object-size spellings fold from that converted
mode; lowering consumes the retained folded result instead of inspecting the
original argument again. Signed narrowing uses the existing target integer
truncation/sign-extension rule. The red semantic regression rejected
4294967296ULL before this repair. Accepted and rejected wide constants are
covered on all four targets, including unsigned 128-bit high-half values.
The execution table covers all four converted modes in both spellings before
and after optimization/ABI lowering and in the native arms. The reference
program passes bundled Clang at O0/O2, with its expected conversion warnings.
A query whose operand names no object semantic analysis can see (a pointer
parameter, say), and whose evaluation could not do anything GCC's
unevaluated operand would not (`deferrable_operand`: no side effects, reads
through pointers, atomic or volatile reads, calls, or variably modified
types, whose bounds a cast or `sizeof` evaluates), is no longer folded to the unknown answer in a function:
`ObjectSize::Deferred` makes it a non-constant expression, as in Clang, and
lowering emits an IR `object_size` that the middle end answers after
inlining (`middle-end.md`, "Object sizes"); it records the pointer
parameter its operand is computed from by pointer arithmetic and casts
alone, if any. Direct calls also retain, per pointer argument, the
closest-subobject size it names or that parameter
(`FunctionExtensions::subobject_arguments`), and assignments and increments
record the parameters they write (`written_parameters`), from which lowering
decides which subobject hints hold.
See [GCC Object Size Checking](https://gcc.gnu.org/onlinedocs/gcc/Object-Size-Checking.html)
and C99 6.5.2.2p7 (printed p. 72, PDF p. 84), 6.3.1.3p1-3
(printed p. 43, PDF p. 55).

The audit also found the fixed `size_t` alignment parameter was checked before
conversion. It now shares the converted-integer checker: 128-bit values that
convert to 16 are valid; values that convert to zero or three remain invalid.
Its optional offset accepts runtime values and has no immediate range check;
lowering already evaluates its recorded conversion and effects. Installed GCC
accepts and executes the wide-alignment program at O0/O2. Bundled Clang instead
checks the original alignment and rejects it; this repair follows the task's
specified parameter-conversion rule and the fixed GCC signature. See
[GCC Other Builtins](https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html).

The remaining audited paths need no narrowing repair:

- Prefetch rw/locality follow `...`, with no fixed `int` parameter. Checking
  their original wide ICE value is intentional; implicit narrowing would
  wrongly accept 4294967296ULL as rw or 4294967299ULL as locality. Bundled
  Clang rejects both. New audit regressions retain those constraints.
- Expect-with-probability already rounds to the parameter's `double` before
  checking its endpoints. Expect's other operands and fixed-signature bit,
  byte-swap, allocation and overflow inputs use recorded parameter conversions.
- Choose-expr tests ICE truth without an `int` parameter conversion; constant-p
  tests constantness without evaluating its operand. Wide nonzero conditions
  and wide constant queries are pinned by new audit assertions.
- Generic overflow operands retain their own types; overflow-p's third operand
  supplies a type/bit-field width, not a converted value. Existing fixed-signature
  overflow execution coverage checks conversions at int/long/long-long widths;
  new audit assertions distinguish wide and narrow predicate destinations.
- Va-start/end/copy validate list storage and va-arg validates a type operand;
  none has an integer/floating constant parameter. Offsetof indices and shuffle
  lanes have their own integer-expression constraints, not fixed `int` formals.
- X86 immediates already cast to the prototype parameter before range checking.
  Atomic lock-free size queries already cast to target `size_t` before folding.

The alloca coverage now pairs undeclared strict C99 rejection with GNU99
intrinsic lowering and strict C99 acceptance of reserved __builtin_alloca.
The strict test asserts the undeclared-identifier diagnostic and the pipeline's
UnitHasErrors refusal, on Linux GNU and Windows GNU targets. No binding change
was necessary. Bundled Clang rejects strict plain alloca and accepts the
reserved spelling; its GNU99 mode lowers plain alloca when the modern
implicit-function-declaration diagnostic is disabled.

Validation passed: `cargo test --all-targets` (1,604 library tests passed,
three existing ignores, and every integration target passed),
`cargo test --features benchmarking-internals --test allocation_count`
(26 tests), nightly formatting and format checking, all-target Clippy with
warnings denied in debug, release and benchmarking-internals configurations,
and `git diff --check`. Object-size and alignment regressions each failed
before their corresponding fix. A missing final newline in the new alloca
fixture was corrected before the successful full-suite rerun.
Linux native execution was skipped because WSL returned access denied; all
four target semantic checks and all interpreter stages on both targets passed.
The plain CLI was rebuilt after all feature-enabled checks. Shared LLVM
junctions were preserved, and nothing was staged or committed. Whole-file
commit groups are recorded in `target/commit-plan.md`.

### PR #41 round-6 review repairs (2026-10-13)

Reserved builtin identifiers acquire their semantic binding before expression
reduction, including inside compile-time selections. Bottom-up designation facts
retain the leaf and binding through parentheses, `__extension__`, `_Generic`
and `__builtin_choose_expr`. Scalar intrinsic dispatch uses that binding in both
semantic checking and lowering; user declarations and local function pointers
keep ordinary-call behavior. `CallFacts` retains directness once at the call site,
including all conservative indirect cases.

Intrinsic-only builtin designators outside a call now report Clang's "builtin
functions must be directly called" diagnostic instead of producing external
references. An arena-backed linear pass marks transparent child edges and
callee uses, then diagnoses the outermost invalid designator use. Library
builtin aliases retain addressability. See
[GCC Other Builtins](https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html)
and [Library Builtins](https://gcc.gnu.org/onlinedocs/gcc/Library-Builtins.html);
C99 6.5.1p5 (printed p. 69, PDF p. 81) preserves parenthesized designation.

Statement completion forwards the immediate ordinary, case/default, case-range
or attributed child's value-tail result in constant work. Lowering keeps the
retained outer tail statement on its task stack with a value-preserving flag: each label enters its usual jump
target before the converted tail is evaluated. C99 6.8.1p4 (printed p. 132,
PDF p. 144) and
[GCC Statement Exprs](https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html)
support ordinary labeled tails. Direct case/default tails would require an
outer switch to enter a statement expression, which GCC and Clang reject;
semantic regressions preserve that diagnostic while checking tail typing.
Execution tests therefore use legal internal case/default dispatch and jumps
from those arms to an ordinary labeled value tail.

Object-size modes, expect probabilities, assume-aligned alignments, and
prefetch read/write/locality operands have requirement-specific diagnostics
and goldens. Fixed-parameter checks retain round 5's converted-value rules;
prefetch ICEs retain their original width, and optional alignment offsets
remain allowed at runtime. Incompatible argument types emit only conversion
errors, without an additional value-constraint error. Both new goldens retain
following declarations and exact source spellings.

The focused `Pass::Inline` regression calls a small dynamic-alloca function
from a loop, asserts its call survives, and checks literal return values and
at most 512 retained interpreter byte cells for 1 and 10,000 iterations.
The original exclusion is retained: inlining would extend each allocation's
lifetime to the caller's return. See
[GCC Stack Allocation](https://gcc.gnu.org/onlinedocs/gcc/Stack-Allocation.html).

Round-6 validation passed on Windows: `cargo test --all-targets` (1,611
library tests passed, three existing ignores, and every integration target),
all 26 feature-enabled allocation tests, nightly formatting and format checking,
all-target Clippy with warnings denied in debug, release and feature-enabled
configurations, and `git diff --check`. The plain CLI was rebuilt last.
Removing the dynamic-alloca inliner exclusion made the new call-retention
assertion fail; restoring it passed both iteration counts. Both execution
fixtures returned zero with GCC and Clang at O0/O2. Linux native execution
was unavailable because WSL returned access denied. Shared LLVM junctions
were preserved, and nothing was staged or committed. The whole-file commit
plan is `target/commit-plan.md`; check logs are under `target/round6/`.


## Retained occurrence, parameter and runtime-size correspondence

`CorrespondenceFacts` retains source-ordered `DeclarationFacts` with typed
`OccurrenceId` and `EntityId`, accepted binding, initializer association,
prototype parameter facts and new declarator-bound actions. Named declarators
are copied values: their arena-owned suffix-list identity indexes the occurrence,
including parenthesized suffix aliases. No copied-value address or source-vector
comparison identifies an occurrence. Rejected declarations keep an occurrence
with no accepted binding/entity. The initializer association is retained now;
subobject placement plans belong to the initializer-plan migration.

`Parameter` retains its prototype binding and checked type-site actions during
parameter-list construction. `FunctionRecord::parameter_facts` keeps prototype
and body bindings, declared and incoming/promoted types, the original type before
array/function adjustment, and entry actions in parameter order. Unnamed and
invalid entries retain their ordinal with absent bindings. K&R declaration-list
actions retain their written order separately from identifier-list incoming
argument order. Bound expressions and the body use the same parameter storage
through the retained alias pairs. Inspection reads static minima by binding ID.
Correspondence construction and consumption visit parameters and occurrences;
there is no per-function translation-unit binding scan.

`TypeNameFacts` gives each written type name a deterministic `TypeNameId`,
resolved type and size-query record. Both producer lookup and retained queries
use syntax identity. `RuntimeBound` retains the typed integer expression, its
required size_t conversion, written evaluation site and saved-extent dependencies.
Distinct derivations keep distinct bound IDs, even when their expression spelling
or element type agrees.
Declaration, parameter, cast and type-operand sites retain the ordered actions
that execute; typedef uses reuse saved extents. Dependency reduction includes
size and stride uses in embedded statement expressions.

`SizeFacts` interns a shared `SizeFormula` DAG of byte constants and products
with constant extents or `BoundId`s. Finalization visits canonical types once,
after nominal tag layouts complete, sharing each child formula. Cached constant
sizes preserve fixed-size emission. Allocation, runtime `sizeof`, subscript,
pointer arithmetic and increment strides query this same graph. Lowering assigns
SSA extent variables and emits formula products; it does not rediscover bounds
or reconstruct products from types.

`SizeQueryFacts` records a formula and exactly which operand and written bound
or typeof actions execute. Constant queries suppress both operand and type-site
effects, including VM pointers with fixed pointer size. Runtime VLA queries
execute the retained actions and, for expression operands, the retained operand.
Missing required facts in a lowerable modeled node produce `MissingFact` with
provenance. The control graph still rejects entry into VM declaration regions;
VM typedef/pointer protection does not allocate stack storage.
C99 §6.7.5.2p5, p. 117 (PDF p. 129), §6.9.1p7,10, p. 142 (PDF p. 154),
§6.5.3.4p2, p. 80 (PDF p. 92), and §6.5.6p8, p. 83 (PDF p. 95).
Validation and corpus evidence: [M2 receipt](docs/research/semantic-facts-m2.md).

## Retained resolved expressions, calls, tails and members

**M3 is complete.** `ResolvedFacts` retains one `Designation` per expression
ordinal and sparse call, member and statement-expression records. Expression
completion copies one immediate selected child's leaf/binding and conversion
summary. Parentheses, `__extension__`, `_Generic` and `__builtin_choose_expr`
therefore cost O(1) per edge, without recursion or repeated chain walks.
Conversions on the outer node remain separate from conversions on its selected
children. Scratch conversion summaries track count, first kind and final type
while the existing contextual conversion records are produced.

`CallFacts` records `Callee::Direct` with entity and binding, `Indirect` with
the callee expression ordinal, or `Builtin` with its optional named binding,
designation-preserving directness and checked instance. It retains the point-of-use
function type, fixed prefix, prototype/ellipsis status and each argument's expression ordinal
and converted type. Argument ordinals reference the existing conversion runs;
later composite binding types never replace the written call's type. M4
retains checked scalar/library/varargs instances; M6 registered the remaining
families, whose calls retain no instance. Overloaded builtins may lack an
ordinary function type; ordinary lowerable calls require it.
M7 retains the entity's known definition; `unprototyped_external` combines it
with the binding's point-of-use type. Returns-twice and unsupported-builtin
capability refusals are unchanged.

GNU implicit function declarations still precede identifier typing. Discovery
forwards callee context through immediate parentheses/extension children on the
ordinary work stack; it does not walk a chain before every reduction. Compile-time
selections reuse completed designation facts. Function pointers, casts, address
and dereference operators, comma/conditional expressions, converted block values,
extra outer conversions and conversion-changing children stay indirect.

`StatementExpressionFacts` retains the block, outer labeled tail, optional value
expression, prefix length and converted result type. Statement continuations
forward one child's tail result; block reduction inspects only its last immediate
item. Empty/declaration/nested-compound endings yield void. Labels execute before
the converted tail value, and aggregate snapshots still precede lifetime cleanup.
The retained expression-effects record summarizes the block's statements.

Lowering's linear address-use scan and builtin dispatch read retained callee and
designation records. Call scheduling and completion share `CallFacts`; member
access reads `MemberFacts`; GNU block scheduling reads its retained tail/prefix.
The old chain-walking helpers, repeated direct-callee checks and owning-record
recovery are deleted. Constant-time selection/tail helpers are semantic-private.
Missing required facts report `MissingFact` with the written node's provenance,
including member references in static address initializers.
C99 §6.5.1p5, p. 69 (PDF p. 81), §6.5.2.2p6-7, pp. 71-72
(PDF pp. 83-84), §6.5.2.3p3-4, p. 73 (PDF p. 85), and §6.8.1p4,
p. 132 (PDF p. 144); C11/GNU selections and GNU statement expressions.
Validation and corpus evidence: [M3 receipt](docs/research/semantic-facts-m3.md).

## Retained builtin contracts and checked instances

**M4 is complete.** `src/builtins.rs` is a phase-neutral, static descriptor index.
The scalar and library shards own exact spellings, append-only `BuiltinId`s,
signatures expressed in target scalar/size/pointer patterns, closed type and
operation rules, operand conversion/evaluation/constant constraints, feature and
naming policy, effect classes and library equivalence. The generated sorted name
index maps one spelling to one ID; `scripts/generate_builtin_index.py` preserves
IDs when rows move or new entries are added, writes the file as rustfmt formats
it, and with `--check` (run in CI's formatting job) fails when the index has
drifted from the shards. Descriptors contain no syntax,
semantic types, IR opcodes, checker/lowerer callbacks or diagnostic messages.
Constraint identities adapt to the existing `SemanticErrorKind` catalog.

Lookup first respects a visible user binding. `BindingOrigin` then distinguishes
`User`, `Intrinsic(BuiltinId)`, `ImplicitOrdinary` and unknown reserved
`OpaqueIntrinsic` bindings.
Unknown reserved names keep opaque signatures. Plain implicit alloca is GNU-only;
ordinary implicit library names are hosted-only. Reserved library aliases keep
their signatures in freestanding mode. The fabs/fabsf/fabsl aliases share the
registry's double/float/long-double contracts; host implementations remain backend
capabilities.

Named scalar, library and varargs calls and keyword varargs/offsetof retain an
arena-owned `BuiltinCallFacts` in M3's `Callee::Builtin`. Keyword forms have no
named binding. Instances retain descriptor ID, checked operation, result and
operation types, result folds, effects and normalized operands. Each operand
retains its converted type/constants, bit-field precision and evaluation mode:
value, storage address, adjusted list pointer, effects only, unevaluated or type.
System V pointer parameters read an adjusted list pointer; Win64 cursors retain
storage-address uses. Lowering's address-use scan consumes the retained binding
uses. No synthetic `Builtin` AST is allocated for ordinary varargs calls in either
phase, and lowering does not decode scalar spellings or reconstruct operand shape.
IR emission and varargs ABI expansion keep their existing algorithms.

Fixed-parameter constants are checked after target conversion; probability rounds
to double before finite endpoint checks. Prefetch constraints retain original ICE
width, including 128-bit immediates. Assignment compatibility precedes dependent
constant diagnostics. Dedicated overflow, floating and varargs handlers keep their
type/precision/layout rules. Existing semantic/diagnostic/IR goldens are unchanged.

Exhaustive scalar/library agreement tests cover all current targets,
strict/GNU modes and hosted/freestanding environments. M5 later consumed the
registry's effect classes in the retained expression effects; M6 (below)
registered the remaining families; M7 retains the effective external-symbol
and returns-twice policy.

C99 §6.5.2.2p7, p. 72 (PDF p. 84), §6.2.1p4, pp. 29-30
(PDF pp. 41-42), §7.15.1, pp. 249-251 (PDF pp. 261-263),
and §7.1.4, pp. 166-167 (PDF pp. 178-179); scalar intrinsics are GNU
extensions. Validation and corpus evidence: [M4 receipt](docs/research/semantic-facts-m4.md).

### Builtin families in the registry

**M6 is complete.** Every builtin spelling and keyword form has exactly one
registry descriptor: the atomic shard (already registered with atomic
execution), the generated `src/builtins/x86.rs` shard, and the handwritten
`vector.rs` and `type_generic.rs` shards, beside the scalar, library, math and
checked shards. There is no other recognizer: `builtins/names.rs`, the
`LegacyIntrinsic` origin and the family name predicates are deleted.
`__has_builtin` reads the registry directly (keyword forms where the keyword
is active, ordinary calls by naming policy) and never consults semantic
analysis or a local binding.

Family descriptors name a closed `TypeRule` (`X86Signature`, `VectorShuffle`,
`VectorOverload(op)`, `ConvertVector`, `BitCast`, `TypesCompatible`,
`ChooseExpr`, `TypeClassify`, `Math128(rule)`, `Atomic`) and the dedicated
handler keeps its algorithm. `common_builtin_call` is the single named-call
entry: it reads the binding's descriptor once and dispatches on the rule;
`type_builtin` does the same for keyword forms through `registry::keyword_descriptor`,
the adapter from the one keyword classifier. Overload, binary128 and
classification handlers take their operation from the descriptor payload, not
from the spelling. The x86 signature is projected from symbolic
`Vector`/`VectorPointer` signature types; each immediate is an argument rule with
`ConstantConstraint::Immediate(low, high, mask)`, checked after prototype
conversion and reported through the `Immediate` diagnostic identity. Families
whose calls have no single C type use the `Opaque` signature, so their
undeclared bindings keep the unprototyped unknown-result type they had.

Family operations are `BuiltinOperation::Semantic(_)` (classification, type
compatibility, constant selection and binary128 primitives: checked and folded
or selected here) or `Unsupported(X86 | Vector)`. Neither retains a
`BuiltinCallFacts` instance (`BuiltinOperation::retains_instance`), so their
effect summary stays `UNKNOWN` and lowering keeps its earlier refusal (an
unsupported builtin, or the opaque binding's unanalyzed type); a keyword form's spelling that is an identifier because its keyword is
disabled stays an opaque reserved name. Two header calls of the x86 generator,
`__builtin_popcount` and `__builtin_popcountll`, are merged into the scalar
descriptor after a test checks that the signatures agree.

`tests/fixtures/semantic/builtin-names.snap` pins every recognized spelling's
`__has_builtin` answer in four configurations and the binding origin, type and
diagnostics of an undeclared call; the snapshot keeps its own names under test.
The reserved builtin type names (`__builtin_va_list`, `__int128_t`) still have
no type descriptor. Validation: [M6 receipt](docs/research/semantic-facts-m6.md).

## 2026-10-13: Torture representation types and constant forms

GNU `packed`, `aligned` and scalar `mode` now retain concrete types and layouts.
Packed enums select the smallest compatible integer representation. Alignment
belongs to the completed declarator, including arrays and pointers; aligned
typedefs can weaken their alignment, while ordinary object declarations cannot.
Record and member packing retain the original member types for ABI classification
and lower the guaranteed alignment of member accesses and initializer stores.
`_Alignas` operands are analyzed before declaration completion, with C11
§6.7.5p2-6 constraints (printed p. 127; PDF p. 145). GNU forms continue to obey
the existing extension policy, including `Deny` in GNU dialects.

GNU union casts require an exactly compatible member type and produce a union
rvalue. GNU range designators require ordered nonnegative integer constant
endpoints within bounds, infer incomplete array bounds from the upper endpoint,
and retain once-only expression evaluation. Existing `typeof`, type compatibility
and `offsetof` queries consume these retained types without a separate feature
implementation. Floating sign, infinity and sign-copy builtins retain signatures,
operand constraints and constant results, preserving signed zero and NaN payloads.

Decisions: use explicit arena-backed work stacks and retained type-handle
alignment, existing scalar/memory IR, and the shared byte/relocation initializer
representation. Do not add stack introspection, nonlocal jumps, argument replay,
label relocation, inline assembly, vector or complex lowering here. The first
four require backend/interpreter control or ABI subsystems; aggregate varargs
and VLA lowering remain owned by the separate worktree. The dated corpus report
in `docs/research/gcc-torture-execute-results.md` contains the exact inventory,
counts, implementation references and validation evidence.

### PR #42 round-1 layout and alignment repairs (2026-10-13)

System V packed bit-fields honor explicit member alignment, including a
one-byte request that starts a new byte. Packed union bit-fields occupy only
the bytes their bits require. Microsoft allocation units remain unchanged;
bundled Clang gives the aligned packed example size/alignment 4/4 on MinGW
and 4/1 on MSVC, which ignores that member alignment. The Linux result is 8/4.
These choices follow the [GCC packed and aligned attribute sections](https://gcc.gnu.org/onlinedocs/gcc/Common-Attributes.html)
and C99 §6.7.2.1p10 (printed p. 102; PDF p. 114).

Packed fields may straddle their declared storage slot. Lowering accesses
exactly their occupied bytes and uses a sufficiently wide integer for their
values; a 128-bit field reaching a seventeenth byte is assembled in value
coordinates. Static initializers merge those bits without discarding neighboring
fields, as required by C99 §6.7.8p19 (printed p. 127; PDF p. 139).

Recognized `aligned` operands retain parsed expression syntax and use semantic
ICE evaluation for `sizeof`, alignment queries, enum constants, casts and
arithmetic (C99 §6.6p6, printed p. 95; PDF p. 107). Raw attribute tokens and
provenance remain available; expression syntax and frame pools use arenas.
`vector_size` keeps its existing token reducer. Multiple `_Alignas` specifiers
combine into the strictest request before comparison with the original natural
alignment. That comparison, and type operands, use size-independent array
alignment for VLAs and VLA typedefs (C11 §6.7.5p4,p6, printed p. 127;
PDF p. 145).

Regressions in `src/lowering/tests/layout.rs` cover layout queries, interpreter
execution on all four targets and native LLVM execution with and without its
optimization passes. Semantic tests cover weakened VLA alignment, constraint
diagnostics, following-declaration recovery and deeply nested alignment operands.
Each reported finding was reproduced by a failing regression before its repair.
Bundled Clang checked all four target triples. GCC 13.2 with
`-mno-ms-bitfields` checked System V record rules at O0/O2 on the Windows host;
Linux-native GCC execution was unavailable because WSL access was denied.

## Type-site execution and aggregate alignment

`Types::aggregate_value_layout` reads the completed nominal tag layout. Storage
alignment retained on a declaration or aligned typedef does not change the
aggregate argument's ABI layout, including variadic calls and `va_arg`.
`Member::explicit_alignment` retains alignment written on the member's own
attributes or `_Alignas`; inherited typedef alignment remains on its type.
GNU packed layout uses only explicit member alignment to override packing.
MSVC preserves Clang's inherited-alignment policy for ordinary packed members
and its existing rule ignoring explicit alignment on packed bit-fields.

`BoundSiteFacts` retains the base/resolved type and required `typeof` operand
evaluations on each declaration and type-name syntax identity. The semantic
`TypeofDone` continuation determines VM status from the canonical type graph,
without array decay. Nested type operands compose already-retained site facts.
Non-VM expression operands retain an empty evaluation list. Typedef uses have
no original operand action: their extents are remembered, as required by
C99 §6.7.5.2p5 (p. 117; PDF p. 129).

Written sites retain only newly introduced runtime-bound identities, excluding
inherited bounds from an evaluated typeof operand or typedef. Lowering executes
the operand effects once before those new-bound actions. This preserves GCC's specifier-before-bound
ordering even where Clang chooses the opposite order. `sizeof` type queries
retain their evaluation mode separately: a constant `sizeof` suppresses type
operand effects, including `typeof` of a pointer to VLA. Runtime VLA size queries
execute the retained operand list.
The new tables are arena-owned and keyed by syntax identity; provenance remains
for diagnostics. Existing runtime-bound identities and extent SSA storage are
unchanged. The retained occurrence, parameter and size records above complete
the bound-action and formula interface.

Address-taking uses `FunctionRecord::parameter_aliases` to share the memory
requirement between prototype and body bindings before parameter storage is
chosen (C99 §6.9.1p10, p. 142; PDF p. 154). Bound expressions and body
expressions then address the same parameter object.

## Shared expression effects and entry actions

**M5 is complete.** `facts/effects.rs` produces one four-byte `ExpressionEffects`
record per expression ordinal, in child-before-parent order, when
`retain_expression` runs. It holds two contexts: reading the value and only
designating it (address-only and effects-only uses). Each context carries an
`EffectSet` of `WRITE`, `CALL`, `VOLATILE`, `CONTROL`, `LIFETIME` and
`UNKNOWN`, and a recipe kind. An empty set means the evaluation may be omitted;
it is not an optimizer promise that arithmetic cannot trap or be poison. Each
summary combines the retained summaries of the node's immediate children once,
so production is O(1) per operand edge with no recursion or suffix walk.

- Assignments and increments write; a store to a volatile lvalue is also a
  volatile access. Ordinary and indirect calls are `CALL`.
- A volatile read is `VOLATILE` only in the value context; `&v`, `&*vp` and
  `&p->vm` designate without access. Member bases and address-of operands use
  the designation context.
- `_Generic` and `__builtin_choose_expr` forward their retained selection only.
- Checked builtins use the registry instance: the descriptor's
  `BuiltinEffectClass` (pure/query none, write, control, allocate as
  `LIFETIME`, library as `CALL`) plus the operands their evaluation modes
  evaluate. Unevaluated and type operands contribute nothing; effects-only
  operands contribute their value context. Temporary M6 bridges are `UNKNOWN`.
- Constant `sizeof`, and `typeof` outside a VM type site, contribute nothing. A
  VLA `sizeof` takes its retained size-query operand and new bound expressions;
  a cast takes its `expression_actions` typeof operands and new bounds.
- A GNU statement expression opens a body summary; each completed statement
  adds its own expressions, loops and jumps add `CONTROL`, and declarations and
  nested definitions add `UNKNOWN`. The block closes the summary.
- Compound literals and unmodeled forms are conservatively effectful.

ICE eligibility remains separate. The existing rule still clears constant
eligibility when a cast's evaluated type-site actions are effectful, and an
overflow predicate folds only when its third operand's value summary is empty.

The same postorder retains effects-only recipes once the node's ICE state is
final. `EffectRecipe::None` and `EvaluateValue` or `StatementBlock` of the node
itself live in the dense record. `Sequence` (the node's own type-site actions,
then child uses) and `Conditional` (a condition evaluated once as a branch and
optional arm uses) live in a sparse map keyed by ordinal and context. A use
names a child ordinal and context. Short circuits and conditionals keep a
branch only when an arm's summary is non-empty; omitted conditionals branch on
their saved condition. ICE nodes execute nothing; volatile reads evaluate.
`SemanticTranslationUnit::effect_summary`/`effect_recipe` are the queries. A
child without a record makes the recipe unavailable, and lowering reports that
missing fact. The old per-node boolean purity pair and the lowering's
operator/volatility classification are deleted.
C99 §5.1.2.3p2, p. 13 (PDF p. 25), §6.7.3p6, p. 109 (PDF p. 121),
§6.5.3.4p2, p. 80 (PDF p. 92), §6.5.13p4-§6.5.15p4, pp. 89-90 (PDF pp. 101-102);
[GCC Statement Exprs](https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html).
Validation and corpus evidence: [M5 receipt](docs/research/semantic-facts-m5.md).

`FunctionRecord::parameter_facts` keeps each prototype-style entry action with
its original type and binding correspondence; `parameter_sites` keeps K&R
shared declaration actions in written order. Lowering initializes incoming parameters and
aliases first, then runs each site's typeof operand evaluations and newly introduced
bounds before the body. Prototype-only declarations do not execute entry actions.
C99 §6.9.1p10, p. 142 (PDF p. 154), §6.5.3.4p2, p. 80 (PDF p. 92);
[GCC Typeof](https://gcc.gnu.org/onlinedocs/gcc/Typeof.html) and
[GCC Integer Overflow Builtins](https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html).

## GNU entity attributes and cleanup regions (2026-10-14)

This is the attribute part of semantic-facts migration M7: semantic analysis
validates attributes once and lowering consumes retained facts. The attribute
pass lives in `attributes.rs` and `attributes/`:

- **Name pass.** `check_attribute_list` visits each attribute specifier once
  (keyed by its arena identity) from every position the work stack reaches:
  declaration specifiers, declarators and pointer levels (abstract ones
  included), record, enum, member and enumerator attributes, and attributed
  statements. `catalog.rs` gives each name a rule; unknown names and dropped
  meanings warn, conventions are checked against the target, and meanings
  code generation cannot honor become `SymbolFacts::refusals`, which lowering
  reports as unsupported constructs at their provenance. Lowering's former
  calling-convention token parsing is deleted.
- **Entity pass.** After a declaration binds (and for a function definition
  when its binding is installed), `declaration_attributes` applies the entity
  attributes of its specifier chain, declarator, pointer levels and grouped
  declarators to a `Subject` (function, object, typedef, parameter or member)
  with Clang's applicability diagnostics. Weak, section, used, visibility and
  function alignment merge into `EntityAttributes` keyed by `EntityId`; the
  correspondence retains the linked-name entity map so lowering can find a
  binding's entity (`SemanticTranslationUnit::entity_attributes`).
- **Completion.** `finish_attributes` runs after translation-unit definitions
  are final: alias requests resolve through chains to a definition by C name
  (`SymbolFacts::aliases`), with target, kind, cycle and type checks; alias
  declarations never become tentative definitions. Function definitions with
  constructor or destructor requests are registered in definition order
  (`SymbolFacts::init_functions`); parameters are refused.
- **Cleanup regions.** `cleanup(f)` is validated at the declaration and opened
  by a `DeclarationEnd` continuation after the declarator's initializer, as a
  `RegionKind::CleanupDeclaration` region with `CleanupKind::Call { variable,
  function }`. It joins the M1 region tree additively: `nearest_cleanup` now
  links both VLA and call cleanups, so every retained transition (normal
  exits, jumps, returns, switch dispatch) lists the calls to make, innermost
  first; `nearest_cleanup_declaration` and `Ancestry::first_entered_cleanup`
  (the VM query, generalized over the link) find entered cleanup scopes in
  O(log R). `attributes/jumps.rs` reports gotos and switch cases entering a
  cleanup scope; the computed-goto check below reports indirect gotos that
  could enter or leave one. Lowering marks cleanup variables address-taken and emits the
  calls from the same transitions that release VLA storage.

On ELF targets, named BSS sections are checked before lowering, independently
of the selected backend. `target/sections.rs` classifies BSS and TLS linkonce
families for both semantic analysis and native emission. Externally emitted
and explicitly used function definitions receive `BssFunction`; omitted static
and inline-only bodies remain valid. After entity completion supplies inherited sections,
`initializers/bss.rs` walks retained initializer operations backwards and
reports each emitted object's first surviving nonzero or relocated leaf at its
initializer. Semantic checks and lowering share entity reachability: evaluated
references from emitted functions and object initializers retain internal
objects, while references owned only by omitted functions do not. Unused
function-local static and thread-local objects are retained with every emitted
function when `CompilerConfiguration::optimization().0` is false, exactly when
`__OPTIMIZE__` is undefined (the default, `-O0`, native B0 and `--opt=none`
without another enabled optimizer). This models Clang O0 within the emission
evaluator's supported forms; static duration is
C99 §6.2.4p3, p. 32; PDF p. 44. Locals of omitted static or C99 inline-only
bodies remain omitted (§6.7.4p6, pp. 112-113; PDF pp. 124-125), including
`used` locals. With optimization enabled, only referenced or `used` locals of
emitted bodies are retained. Constant-untaken arms omitted as a whole by
Clang's code generation retain no local objects, even `used` ones. The shared
walk distinguishes a missing insertion point (`dead`) from omission of a whole
statement or operand (`elided`). Clang `EmitSimpleStmt` still visits declarations,
compounds, attributed statements and labels (including case/default) after a
jump or noreturn call. A non-simple statement without a label is skipped there,
including its nested declarations; labels inside GNU statement expressions count.
Folded switches elide unselected statements, unselected case labels included,
and keep only the blocks and labels leading to the selected label; the selected
label resumes emission before its facts are captured. Unevaluated sizeof/typeof/alignof operands, generic control
and unselected associations, unselected choose-expression arms and folded-away
label-free conditional/logical operands retain no local definitions. Runtime VLA operands
remain evaluated. Declaration contexts are finalized after typing, together
with reference contexts. Lowering consumes the same operand and statement
elision facts (C99 §6.8.4.1p2, pp. 133-134; PDF pp. 145-146).
Function-to-local edges and each initializer's outgoing references share the
same worklist, so a newly retained local retains its internal referents
transitively. Only unlowered references retain neither a function nor its
locals. Dead automatic initializers without labels and dead cleanup
registrations are omitted by sema and lowering together. Retained local storage
is defined independently of the executable expression walk, including statics
inside skipped operands. Lowering checks omitted internal referents before
creating a declaration. This decision feeds BSS validation, LLVM, both native object paths and
the interpreter. When optimizing, a reached inline-only function also retains
its `used` locals, matching Clang's optional-body section diagnostics even on
native and interpreter pipelines. Semantic checks still precede optimization:
a constant-read local such as `return b[0]` can be rejected where Clang O1/O2
folds the read and removes the object. This known difference does not attempt
to model LLVM's optimizer. Optimized optional copies of C99 inline-only bodies
retain their existing lowering policy.
A fixed-depth interval radix tree records later writes; scalar GNU ranges remain periodic queries and one coverage interval, independent of their
extent. Assignment conversion, negative floating zero, overwritten initializers,
union selections and bit-fields affect the final contents. Complex ranges use
sparse replay with a work budget linear in the written plan; budget exhaustion
conservatively defers to backend final-content validation. Nonzero aggregate,
string and bit-field GNU ranges commonly exhaust this budget from roughly
32–128 elements onward; nested ranges and partly overwritten pointer ranges
can do so too. These refusals lose the early source label, but cannot emit
invalid BSS data. Unmodeled initializers retain their existing capability
refusals; native emission also checks raw IR, and LLVM error diagnostics prevent
output from being written.

The constructor/destructor priority operand is parsed as an expression, as
`aligned`'s is, so the ordinary constant evaluator checks it. GNU extension:
GCC manual, "Common Function Attributes" and "Common Variable Attributes";
Clang Attributes Reference. C99 §6.2.2p2, p. 30 (PDF p. 42); §6.2.4p5, p. 32
(PDF p. 44); §6.8.6.1p1, p. 137 (PDF p. 149).

**M7 entity and symbol completion.** `facts/entities.rs` finalizes entities
after definitions, DLL/inline attributes and overload ABI names. The retained
record is sparse over functions and linked/static-duration objects; automatic
objects and parameters have no record. It joins binding and occurrence IDs, final type, definition and
initializer association and known function definition. Per-occurrence
`FunctionPolicy` keeps body permission and end policy so a GNU inline body and
its replacement definition can differ. Each binding keeps its point-of-use
type; open-call policy queries that type and the entity's known definition.
Lowering decides reachability and generated symbol names, and keys private inline bodies by semantic identity even when another
entity's asm request names the same external symbol.

Function values of GNU extern-inline fortify wrappers retain the external
symbol; direct calls can still use the private inline body. For Clang-style
`pass_object_size` overloads, value resolution selects the sole addressable
declaration before an explicit function-pointer cast changes its type (C99
§6.3.2.3p8, p. 48; PDF p. 60). Static relocations consume that same retained
selection as runtime identifiers, including glibc `getcwd`/`read` wrappers and
SQLite's generic syscall-pointer table (issue #81).

Assembler labels retain decoded names and provenance, inherited by entity
identity. Invalid requests retain capability refusals with the same lowering
stage, text and function-emission filtering. Colliding IR definitions and
aliases involving asm labels remain downstream refusals. `FunctionEnd` preserves
the C-identifier-only `main` zero-result rule, including asm-renamed and private
bodies; hosted and language-mode refinements are separate behavior changes.
Convention occurrences retain the resolved target convention or unsupported
request, source and owning function, including abstract callable type sites.
Existing convention diagnostics and default C type identity are unchanged.

The entity's effective returns-twice fact combines existing
`EntityAttributes::returns_twice` with assembler requests through
`builtins/external.rs`. That neutral catalog also exposes library signatures
from the registry for independent host/textual-IR validation. `Types` provides
whole-object readonly storage, retained VM classification and natural layout
queries; `TypeId::is_volatile` is the canonical volatile-access predicate.
Initializer placement and symbolic constant decomposition remain M8/M9 work.
Validation and measurements: [M7 receipt](docs/research/semantic-facts-m7.md).

#### Labels as values (2026-10-14)

GNU `&&label` ([GCC Labels as Values](https://gcc.gnu.org/onlinedocs/gcc/Labels-as-Values.html))
types as a `void *` rvalue address constant (`ConstantClass::Address`) and
resolves through the ordinary label lookup, so GNU local labels keep their
lexical identity. Outside a function it is `LabelAddressOutsideFunction`.
The first `&&` of a label numbers it among its function's address-taken labels:
`ControlFacts::address_taken(function)` lists them in that order,
`LabelFacts::address` holds each position, and `ControlFacts::label_address`
maps every `&&label` expression to its `LabelId`. Lowering uses the position as
the IR label number, so static initializers lowered before the body agree.

`goto *e` records a jump to `ControlTarget::IndirectGoto(function)` with an
empty transition (like Clang, it releases no VLA storage it leaves) and
converts `e` to `const void *` under simple-assignment constraints
(`InvalidComputedGoto` otherwise; C99 §6.5.16.1p1, p. 92; PDF p. 104). After
the unit is walked, each computed goto is checked against its function's
address-taken labels as Clang's jump checker does: none at all is
`IndirectGotoWithoutLabels`; a target inside a statement expression or the
scope of a variably modified identifier that the goto is outside of is
`IndirectJumpIntoStatementExpression` or `IndirectJumpIntoVariableScope`
(C99 §6.8.6.1p1, p. 137; PDF p. 149), reported once per goto at its first
such target with the region's start as secondary label. As in Clang, whose
jump checker treats a GNU cleanup scope as one an indirect jump may neither
enter nor leave, a target inside a cleanup variable's scope that the goto is
outside of, or any address-taken label outside the innermost cleanup scope
around the goto, is `IndirectJumpCleanupScope`. Each function's labels inside
a protecting region, and the first and last of its labels in region
preorder, are collected once, so the work is O(gotos + labels) plus one check
per goto and protected label; plain dispatch tables need no pairwise check
at all.

`&&a - &&b` for two labels of one function is `ConstantClass::LabelDifference`:
a constant with no folded value whose two labels `ControlFacts::label_difference`
retains. It survives parentheses and conversions to an integer of 32 to 64
bits no wider than the converted value, which static initializers accept
unless the destination is a bit-field, matching Clang; `_Bool`, narrower and
128-bit types, a narrowed difference widened again and arithmetic on it are not
constants. GNU `void *` arithmetic
([GCC Pointer Arith](https://gcc.gnu.org/onlinedocs/gcc/Pointer-Arith.html),
`Feature::VoidPointerArithmetic`) uses element size 1 for `+`, `-`, increments,
compound assignment and subscripts, and in address constants and constant
differences; a `void` subscript is an rvalue that is never read, and `sizeof`
of `void` is 1 in every mode.


## C99 real math builtin contracts (issue #47)

The neutral registry now covers the 57 C99 real math families in float, double
and long-double precision, including output-pointer functions, target-width
integer results and `nexttoward`'s long-double second argument. Existing `fabs`,
`floor`, classification and sign-copy operations retain their implementations;
the new aliases lower to C libm symbols. Hosted Linux executables link libm
through both LLVM and native back ends.

`__builtin_inf*`, `__builtin_huge_val*` and literal-tag `__builtin_nan*` produce
arithmetic constants of their declared precision. Numeric NaN tags retain their
payload bits; dynamic tags retain normal call conversions and evaluate once.
The typed `isnanf`/`isnanl` aliases use the existing classification operation.
Clang's left/right rotate builtins for 8, 16, 32 and 64 bits retain unsigned
contracts and reduce counts modulo the width for constant and runtime operands.

All f/d/l signatures are semantic contracts on all four targets. The interpreter
executes binary32/64 and MSVC's binary64 long double; its existing binary80
execution refusal remains. LLVM executes GNU binary80 math through libm, while
the native x86-64 backend retains its binary32/64 support boundary.
C99 §7.12p3-5, pp. 212-213 (PDF pp. 224-225), §7.12.4-§7.12.13,
pp. 218-239 (PDF pp. 230-251), §7.12.11.2p2, p. 237 (PDF p. 249);
GCC Library Builtins and Clang Language Extensions (rotate builtins).

## Diagnostic parity follow-up (#49 and #54)

`TypeKind::Nullptr` retains the C23 null pointer type independently of ordinary
pointers. Its layout and lowering representation match pointers; constraints
admit pointer and Boolean conversions and reject integer/arithmetic misuse.
Nullptr literals have a constant null address and no effects. GNU function
pointer arithmetic uses byte stride, including subtraction.

The assignment conversion classifier accepts a separate diagnostic site while
retaining the conversion at the operand. Root declaration initializers use the
identifier, assignments use the operator, and diagnostics from expanded syntax
report at the invocation; tokens written as macro arguments keep their own
locations. Parser diagnostic folding still uses spelling provenance. New
null/pointer warning groups retain ordinary
conversion facts. K&R calls check the parameter types retained by a visible
definition without replacing the default argument promotions. `nonnull`
attributes retain pointer parameter positions in entity facts and merge them
over redeclarations; later direct calls check constant null arguments.

Predefined signature typedef sugar is looked up at the function declaration's
scope rather than the first use's active body scope. C89 `__func__` follows
Clang's reserved-identifier behavior without a C99 extension diagnostic.

## Binary128 arithmetic and folding (2026-10-27)

`crate::binary128` computes IEEE binary128 in software: addition,
subtraction, multiplication, division, `fmodf128`, square root and
comparison, conversions to and from every integer width, binary32, binary64
and the x87 format, and the exact `<math.h>` operations (rounding to
integers, scaling, decomposition). Each operation unpacks its operands,
computes the exact result with integers (or with a sticky bit for an inexact
quotient or root) and rounds once to nearest even at 113 bits, denormalizing
first below the smallest normal. NaNs follow libgcc's soft-float routines on
x86-64 (`config/i386/sfp-machine.h`): of two NaNs the one with the larger
fraction wins, a tie going to the first operand of `+` and `*` and to the
second of `-` and `/`; the result keeps its sign and is quieted, an invalid
operation gives the negative default NaN, and conversions keep the top of a
payload. `__extendxftf2` ignores the x87 integer bit, so unnormals and
pseudo-denormals convert as their fields say. A differential run of 200,000
random operand pairs (subnormals, NaNs, infinities and values near one)
through every operation and conversion matched libgcc and glibc
(`sqrtf128`, `fmodf128`) bit for bit; unit tests keep fixed vectors.

Semantic analysis folds binary128 constants with it. `ExpressionInfo` keeps
a folded floating value in one byte-aligned slot that holds either a
standard value in the host carrier or a binary128 one (real and imaginary
components, 32 bytes), read through `floating()` and `binary128()`; a
constant has one floating type, so the record stays 160 bytes. Whenever a
record is retained (`constrain_constant`) its value is made to belong to its
type: a floating constant keeps no integer value, and only a binary128 one
keeps a binary128 value. An operator whose operand was converted, such as a
conditional with an integer arm and a binary128 or `double` one, therefore
cannot fold later operators with the unconverted integer; `(1 ? N + 1 :
0.0q) == (1 ? N : 0.0q)` for `N` = 2^113 is 1, as in GCC and Clang. Folding:

- a `q`/`Q`/`f128` literal is its parsed value, an imaginary literal its
  imaginary part;
- unary `+` and `-`, binary `+ - * /`, comparisons (an integer result, false
  for an unordered pair except `!=`), conditionals, the comma operator,
  `__real__`/`__imag__`, `__builtin_complex` and the
  `__builtin_{inf,huge_val,nan,nans,fabs,copysign}f128` builtins fold; a
  mixed real and complex operation computes only the components the real
  operand takes part in (C99 Annex G.5.1);
- `convert_binary128` converts any arithmetic constant to binary128 exactly
  (integers round to nearest) and a binary128 value to a standard floating
  type with one rounding, as `__trunctfdf2` rounds, never through the x87
  carrier; to an integer type it truncates, and `(int)1.5q` is an integer
  constant expression like any cast of a floating constant (§6.6p6);
- static initializers convert a floating constant to an integer object of
  the destination's width and signedness, so `unsigned __int128` takes
  values from 2^127 up;
- `__builtin_nanf128` and `__builtin_nansf128` keep the low 111 bits of a
  tag wider than 64 bits, as GCC and Clang do; glibc's run-time `nanf128`
  saturates its payload at 64 bits instead;
- the product and quotient of two complex values, which `__multc3` and
  `__divtc3` compute with their own infinity recovery, are not folded:
  `unfolded_binary128` now marks only those and what is computed from them,
  and keeps them out of integer constant expressions.

Lowering emits a folded binary128 value as `fconst.f128` (or converts it to
the destination format for a static initializer), and static initializers
read it through `float_parsing::bits::binary128_bits`. A folded invalid
operation gives the run-time result, the negative default NaN; the pinned
Clang folds `0.0q/0.0q` through LLVM's `APFloat` to a positive NaN, so the
sign of such a statically folded NaN differs from Clang's.

## Half-precision folding (2026-10-27)

`Scalar::Float16` is `_Float16`, IEEE binary16 (2 bytes, alignment 2), a
distinct type ranked below `float`, so `h + 1` is a `_Float16` and `h + f`
a `float`; default argument promotions leave it alone, as GCC and Clang do.
`Scalar::ComplexFloat16` is `_Complex _Float16` (4 bytes, alignment 2), and
`Scalar::BFloat16` is `__bf16`, bfloat16 (2 bytes, alignment 2), which
ranks below `_Float16`, so `b + h` is a `_Float16`. `crate::half_precision`
converts every format to binary16 rounding once, and an `f16` constant reads
its spelling rounded to odd at binary128's precision, so both its binary16
value and its value rounded once to `float` are exact (`HalfConstant`).

Neither compiler computes in binary16 on x86-64, and they keep the excess
`float` precision in different places, so folding follows the identity
(`Analyzer::half_in_float`, `CompilerConfiguration::half_in_float`):

- GCC (`-fexcess-precision=standard`, its only behaviour for `_Float16`):
  every `_Float16` value is folded at `float` precision, constants
  included, and only a conversion to `_Float16` rounds it. An integer
  operand converts to `float`; `?:` and the comma operator keep the
  excess; a conversion to `float`, `double` or an integer converts the
  excess value.
- Clang: a `_Float16` operation's folded value is its constant evaluator's,
  rounded to binary16 after each operation, which `static_assert`, enum
  constants and static initializers read; `(int)(60000.0f16 * 2 / 4)` is
  `INT_MAX` there. Its function code keeps a chain of `+`, `-`, `*`, `/`
  and unary `+`/`-` in `float` and rounds at the chain's root, so such an
  operator is a `runtime_query` and lowering evaluates it. Every other use
  reads the rounded value (`floating_value`).

`half_operand` gives the value an operand contributes to an operator, and
`half_result` the operator's folded value. Lowering's `half.rs` applies the
same rules to the values it computes.

`__bf16` and `_Complex _Float16` follow both rules too (`half_format`;
verified against GCC 15 and the pinned Clang). With Clang a `__bf16` operand
of a `_Float16` operation reads its value rounded to bfloat16 and then to
binary16, and GCC its excess; the complex type's folded parts round
together. GCC's `bf16`/`BF16` constants (`FloatTokenType::BFloat16`) are
read like `f16` ones and have GCC's "non-standard suffix" extension
diagnostic; Clang has no such suffix. `_Complex __bf16` is refused for both
identities (`ComplexBFloat16`), as both compilers refuse it.

An integer constant converts to a floating type with one rounding from its
exact value (`converted_floating`), not through the 64-bit x87 carrier
`floating_value` holds it in, so a 128-bit integer whose last bit decides
a tie folds as GCC and Clang fold it; binary128 constants convert to the
16-bit types with one rounding too.

`Scalar::Fp16` is Clang's `__fp16` (Clang identities only): binary16
storage whose values `promote` to `float` before any operation, as integers
promote to `int`, and to `double` as a variadic or unprototyped argument.
Assignment, `++` and compound assignment keep the type and round once. A
parameter or result of type `__fp16` is refused (`Fp16Parameter`,
`Fp16Return`, Clang's "did you forget * ?" errors), as is `_Complex __fp16`
(`ComplexFp16`). GCC provides the type only on ARM, so the other identities
keep the name an identifier.

## Type-generic bit builtins (2026-10-29)

The type-generic bit builtins (`src/builtins/bits.rs`) have the closed type
rule `TypeRule::GenericBit` and the operation
`BuiltinOperation::GenericBit(GenericBit)`, and retain a checked instance.
`semantic_analysis/builtins/bits.rs` checks the operand by the identity's
rules (`CompilerConfiguration::bit_builtin_rules`), converts a result for
zero to `int` and leaves a rotation count its own type, and records the
operation type whose width the call computes at: the operand's lvalue type,
or an `unsigned _BitInt(1)` for Clang's `bool`, or a `_BitInt` of a
narrower bit-field's width under GCC's rules. The result is `int`,
`unsigned int`, `bool` or the operand's type. Calls of constant operands
fold at any width (`generic_bit_value`, through
`WideIntegers::bit_operation` for the limbs of a value of more than 128
bits), as does the emission evaluator's condition folding. The rules, the
probed compiler behaviour and the deviations are in
[language-standards.md](language-standards.md#type-generic-bit-builtins-and-stdbith-2026-10-29).

## Inferred declarations (2026-10-31)

A C23 `auto` or GNU `__auto_type` declarator (`inference.rs`) reverses the
usual order: `Work::InferDeclarator` checks the declaration's form and
analyzes the initializer first, under the same elision, dead-code and
initializer-value state `Work::InitializeDeclaration` sets, but with no
reference owner, since no binding exists yet. `Work::InferType` reads the
analyzed expression's type after lvalue conversion and decay (Clang keeps
`_Atomic`) and matches Clang's pointer and function derivations;
`Work::Qualify` adds the written qualifiers, and the ordinary
`Work::DeclarationDeclarator` and `Work::Bind` steps type and bind the
declarator. `Work::InferInitialization` then records the initializer and
queues `Work::InitializeDone` without analyzing it again. References made
while a static or thread-local object's initializer was analyzed are given
to the bound object (`Analyzer::claim_references`), so entity emission
keeps what they name, and `Analyzer::initializing_static` answers from the
pending declaration's specifiers until it is bound. A use of the name in its
own initializer is checked against the stack of pending inferences
(`Analyzer::inferred_self_reference`): a binding made inside the initializer
may shadow it; an outer one, or none, is the self-reference C23 forbids,
which only GCC's `__auto_type` allows. `language-standards.md` ("Type
inference") lists each identity's constraints.
