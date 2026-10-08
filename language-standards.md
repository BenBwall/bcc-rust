# Language standards

Configuration, CLI selection, predefined macros and shared extension diagnostics
now connect the lexer, preprocessor and explicit-frame parser. ISO syntax through
C23, the documented C2y subset, GNU extensions and independently enabled MSVC
extensions are implemented across these phases. This is a syntax-only front end:
mode selection is not a full conformance claim. No semantic analysis or code
generation is added.

## CLI reference

The CLI defaults to `gnu17`; `CompilerConfiguration::default()` remains strict
C99 with `ExtensionPolicy::Allow` to preserve library/test callers. `-std=VALUE`,
`-std VALUE`, `--std=VALUE`, and `--std VALUE` are accepted. Repeated `-std`
selections use the last value.

| Standard | Accepted strict spellings | Accepted GNU spellings | `__STDC_VERSION__` |
| --- | --- | --- | --- |
| C89/C90 | `c89`, `c90`, `iso9899:1990` | `gnu89`, `gnu90` | undefined |
| C95 amendment | `iso9899:199409` | no separate GNU95 mode | `199409L` |
| C99 | `c99`, `c9x`, `iso9899:1999`, `iso9899:199x` | `gnu99`, `gnu9x` | `199901L` |
| C11 | `c11`, `c1x`, `iso9899:2011` | `gnu11`, `gnu1x` | `201112L` |
| C17/C18 | `c17`, `c18`, `iso9899:2017`, `iso9899:2018` | `gnu17`, `gnu18` | `201710L` |
| C23 | `c23`, `c2x`, `iso9899:2024` | `gnu23`, `gnu2x` | `202311L` |
| C2y draft | `c2y` | `gnu2y` | `202400L` |

Strict modes define `__STRICT_ANSI__` as `1`; GNU modes leave it undefined.
MSVC flags do not affect either version or strictness. Existing `__STDC__`,
`__STDC_HOSTED__` (freestanding: `0`), date/time, and encoding macros remain.
`__GNUC__` and `_MSC_VER` are deliberately not defined.

`-pedantic` and `-Wpedantic` select Warn; `-pedantic-errors` selects Deny;
the default is Allow. Later policy flags win. Deny emits an error but keeps
classified tokens available for structured parser recovery. Existing specialized
preprocessor extensions retain their established recovery behavior.

MSVC groups are independently opt-in. `-fms-extensions` enables all ten groups;
`-fno-ms-extensions` disables them. Each group has `-fms-NAME` and
`-fno-ms-NAME`; later flags win, including umbrellas.

| NAME | Spellings / behavior | Owner |
| --- | --- | --- |
| `declspec` | `__declspec(...)` | parse-msvc |
| `int-types` | `__int8`, `__int16`, `__int32`, `__int64` | parse-msvc |
| `calling-conventions` | `__cdecl`, `__stdcall`, `__fastcall`, `__vectorcall`, `__thiscall` | parse-msvc |
| `type-qualifiers` | `__ptr32`, `__ptr64`, `__unaligned`, `__w64`, `__sptr`, `__uptr` | parse-msvc |
| `inline` | `__forceinline`; GNU `__inline` also available independently | parse-msvc |
| `seh` | `__try`, `__except`, `__finally`, `__leave` | parse-msvc |
| `asm` | `__asm`, `_asm` blocks | parse-msvc |
| `pragma` | `__pragma(...)` preprocessing operator | lexpp |
| `anonymous-structs` | anonymous tagged struct/union members | parse-msvc |
| `va-args` | empty `__VA_ARGS__` comma elision | lexpp |

Example: `bcc-rust -std=c89 -pedantic -fms-extensions -fno-ms-seh input.c`.
Invalid standard values produce the exact clang-style error and thirteen
notes (deprecated aliases are accepted but omitted from the notes).

## Configuration and shared seams

`src/configuration.rs` owns the value object. `CStandard` is ordered chronologically
(C89 and C90 are the same value; C95 distinguishes amendment 1). `LanguageMode`
pairs a standard with a GNU bit. `MsvcFeature` uses an independent ten-bit set.
All phases read the single configuration through `Context.configuration`.
The batch pipeline and included-file lexer already share that context.

`Feature::ALL` and `Feature::origin()` are the canonical feature vocabulary.
`configuration.accepts(feature)` is a precomputed bit test describing whether
the spelling/construct may be consumed, including extensions. `is_native(feature)`
is a second bit test describing native availability. Both describe the target
contract, **not implementation completion**. Constructors/builders recompute the
bitsets once; there is no per-token string matching or global configuration.
`ImplicitInt` is native before C99 and remains an extension candidate afterward;
`Trigraphs` ceases to be native in C23. These removed features have dedicated
derivation cases rather than monotone introduction checks.

`Context::report_extension(feature, spelling, source_vectors)` is the normal
shared emitter. `report_extension_since(spelling, FeatureOrigin, source_vectors)`
is the explicit-origin adapter. Native standard features emit nothing; otherwise
Allow emits nothing, Warn emits a warning, and Deny emits an error. Messages are
`'SPELLING' is a C11 extension`, `'SPELLING' is a GNU extension`, or
`'SPELLING' is an MSVC extension`. GNU/MSVC constructs remain non-ISO under
pedantic flags even when enabled. The diagnostic retains provenance through
preprocessor compaction and the ordinary FIFO renderer. Reserved aliases can
use their own origin independently of their canonical parser kind.

`KeywordTokenType::classify(id, configuration)` is the **only** classifier.
`ALL` contains canonical parser kinds/spellings; `ALIASES` follows them in the
reserved interner prefix. `KeywordClassification` returns the canonical kind,
original static spelling, and optional origin. Existing C99 keyword IDs remain
stable. Adding parser grammar should consume these kinds and should not add a
second spelling classifier. ISO/GNU/MSVC parser consumers use the canonical kinds
and retain original tokens for spelling and provenance. Unsupported or malformed
syntax follows the structured diagnostic and synchronization paths.

## Feature matrix

ISO-native columns below give revision availability: `Y` means native and
`-` means not native. GNU counterparts use the same ISO revision and additionally
mark GNU-origin rows as dialect-native; MSVC-origin rows become dialect-native
when their flag is enabled. The acceptance column describes extensions/gates separately:
`extension` accepts reserved or unambiguous syntax in older modes, `native` gates
the spelling, `GNU earlier` also accepts it in older GNU modes, and `MS flag`
requires its independent flag. `Trigraphs` is a special gate: only strict C89–C17.
Rows describe native availability and extension gates. Implementation status is
syntax-only: a `Y` does not promise semantic checks, target support, or code
generation. The sections below describe limits and handoffs for implemented rows.

The foundation's mode/flags, version macros, complete keyword spelling recognition,
and shared policy diagnostics are **implemented** for all modes. The lexical,
preprocessing and parser rows below record the integrated implementation.

| Feature (`Feature` variant) | C89 | C95 | C99 | C11 | C17 | C23 | C2y | Acceptance | Status / owner |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Digraphs | - | Y | Y | Y | Y | Y | Y | C95+ / GNU earlier | implemented (lexpp) |
| LineComments | - | - | Y | Y | Y | Y | Y | GNU earlier | implemented (lexpp) |
| Trigraphs | Y | Y | Y | Y | Y | - | - | strict through C17 | implemented (lexpp) |
| UnicodeLiteralPrefixes | - | - | - | Y | Y | Y | Y | native | implemented (lexpp) |
| Utf8CharacterConstants | - | - | - | - | - | Y | Y | native | implemented (lexpp) |
| DigitSeparators | - | - | - | - | - | Y | Y | native | implemented (lexpp) |
| BitIntSuffixes | - | - | - | - | - | Y | Y | native | implemented (lexpp) |
| BinaryConstants | - | - | - | - | - | Y | Y | GNU earlier | implemented (lexpp) |
| Elifdef | - | - | - | - | - | Y | Y | GNU earlier | implemented (lexpp) |
| WarningDirective | - | - | - | - | - | Y | Y | GNU earlier | implemented (lexpp) |
| Embed | - | - | - | - | - | Y | Y | GNU earlier | implemented (lexpp) |
| HasInclude | - | - | - | - | - | Y | Y | GNU earlier | implemented (lexpp) |
| HasEmbed | - | - | - | - | - | Y | Y | GNU earlier | implemented (lexpp) |
| HasCAttribute | - | - | - | - | - | Y | Y | GNU earlier | implemented (lexpp) |
| VaOpt | - | - | - | - | - | Y | Y | GNU earlier | implemented (lexpp) |
| OctalPrefix | - | - | - | - | - | - | Y | native | implemented (lexpp) |
| DelimitedEscapes | - | - | - | - | - | - | Y | native | implemented (lexpp) |
| HexFloats | - | - | Y | Y | Y | Y | Y | extension | implemented (lexpp) |
| VariadicMacros | - | - | Y | Y | Y | Y | Y | extension | implemented (lexpp) |
| EmptyMacroArguments | - | - | Y | Y | Y | Y | Y | extension | implemented (lexpp) |
| Inline | - | - | Y | Y | Y | Y | Y | GNU earlier | implemented syntax and mode diagnostics (parse-std) |
| Restrict | - | - | Y | Y | Y | Y | Y | native | implemented syntax and mode diagnostics (parse-std) |
| Bool | - | - | Y | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| Complex | - | - | Y | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| Imaginary | - | - | Y | Y | Y | Y | Y | extension | implemented unsupported-type diagnostic (parse-std); imaginary semantics remain out of scope |
| ImplicitInt | Y | Y | - | - | - | - | - | extension | implemented syntax and mode diagnostics (parse-std) |
| MixedDeclarations | - | - | Y | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| ForDeclarations | - | - | Y | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| DesignatedInitializers | - | - | Y | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| CompoundLiterals | - | - | Y | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| FlexibleArrayMembers | - | - | Y | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| LongLong | - | - | Y | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| TrailingEnumComma | - | - | Y | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| Func | - | - | Y | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| StaticAssert | - | - | - | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| Generic | - | - | - | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| Alignas | - | - | - | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| Alignof | - | - | - | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| Noreturn | - | - | - | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| ThreadLocal | - | - | - | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| Atomic | - | - | - | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| AnonymousAggregates | - | - | - | Y | Y | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| C23Keywords | - | - | - | - | - | Y | Y | native | implemented syntax and mode diagnostics (parse-std) |
| Attributes | - | - | - | - | - | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| BitInt | - | - | - | - | - | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| DecimalTypes | - | - | - | - | - | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| EmptyInitializers | - | - | - | - | - | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| EnumUnderlyingType | - | - | - | - | - | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| AutoTypeInference | - | - | - | - | - | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| Constexpr | - | - | - | - | - | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| Nullptr | - | - | - | - | - | Y | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| Countof | - | - | - | - | - | - | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| IfSwitchDeclarations | - | - | - | - | - | - | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| NamedLoops | - | - | - | - | - | - | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| GenericTypeOperand | - | - | - | - | - | - | Y | extension | implemented syntax and mode diagnostics (parse-std) |
| CaseRanges | - | - | - | - | - | - | Y | extension | implemented ISO/GNU syntax and origin policy (parse-std, parse-gnu) |
| GnuAttribute | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| GnuAsm | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| GnuTypeof | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| ExtensionMarker | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| StatementExpressions | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| BuiltinVaArg | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| BuiltinOffsetof | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| BuiltinTypesCompatible | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| BuiltinChooseExpr | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| LabelsAsValues | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| LocalLabels | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| OmittedConditionalOperand | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| ZeroLengthArrays | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| Int128 | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| AutoType | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| GnuAlternateKeywords | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| RealImag | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| GnuDesignators | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| UnionCasts | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| EmptyStructs | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| NestedFunctions | - | - | - | - | - | - | - | extension (GNU native) | implemented syntax and mode diagnostics (parse-gnu) |
| ImaginaryConstants | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| DollarIdentifiers | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| NamedVariadicMacros | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| GnuVaArgs | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| IncludeNext | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| IdentDirective | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| Counter | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| HasAttribute | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| HasBuiltin | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| MsDeclspec | - | - | - | - | - | - | - | MS flag | implemented syntax, independent gates and policy diagnostics (parse-msvc) |
| MsIntTypes | - | - | - | - | - | - | - | MS flag | implemented syntax, independent gates and policy diagnostics (parse-msvc) |
| MsCallingConventions | - | - | - | - | - | - | - | MS flag | implemented syntax, independent gates and policy diagnostics (parse-msvc) |
| MsTypeQualifiers | - | - | - | - | - | - | - | MS flag | implemented syntax, independent gates and policy diagnostics (parse-msvc) |
| MsInline | - | - | - | - | - | - | - | MS flag | implemented syntax, independent gates and policy diagnostics (parse-msvc) |
| MsSeh | - | - | - | - | - | - | - | MS flag | implemented syntax, independent gates and policy diagnostics (parse-msvc) |
| MsAsm | - | - | - | - | - | - | - | MS flag | implemented syntax, independent gates and policy diagnostics (parse-msvc) |
| MsPragma | - | - | - | - | - | - | - | MS flag | implemented (lexpp) |
| MsAnonymousStructs | - | - | - | - | - | - | - | MS flag | implemented syntax, independent gates and policy diagnostics (parse-msvc) |
| MsVaArgs | - | - | - | - | - | - | - | MS flag | implemented (lexpp) |

The lexical workstream also owns exact prefix/suffix recognition, `//` treatment
in GNU89 pedantic mode, `#elifndef`, `#sccs`, `, ## __VA_ARGS__`, and predicates
for builtins represented by their grouped feature rows. Parser rows encompass
all positions of GNU attributes; basic/extended/goto asm and declarator asm labels;
GNU range/old-style designators; and syntax-only nested functions. The ISO C2y
subset is implemented by parse-std.

## Decisions and handoff

- C2y version is `202400L`, following the [Clang user manual](https://clang.llvm.org/docs/UsersManual.html#differences-between-various-standard-modes).
  GCC used `202500L` in its [initial C2y implementation](https://gcc.gnu.org/pipermail/gcc-patches/2024-June/654270.html).
  Draft versions are implementation choices, not published conformance claims.
- C2y scope is `_Countof`, octal prefixes, delimited escapes, case ranges,
  selection declarations, named loop control, and type-controlling `_Generic`.
  Further draft features need an explicit matrix row and supporting primary source.
- Reserved standard keywords stay recognized in older modes and are diagnosed
  by origin. C23 non-reserved aliases stay identifiers before C23; GNU `typeof`
  and `asm`, and GNU89 `inline`, are exceptions. `restrict` stays an identifier
  before C99 unless written with a reserved GNU alternate spelling.
- `_Alignas`/`alignas`, `_Static_assert`/`static_assert`, `_Bool`/`bool`, and
  `_Thread_local`/`thread_local` share kinds. Aliases retain original spelling
  for policy diagnostics; token contents preserve it for parser consumers.
- GNU alternate keywords map to existing C99 kinds when applicable. `__asm__`
  is GNU `Asm`; `__asm`/`_asm` are MSVC `MsAsm` gated by the MS asm flag.
  GNU `__inline` remains available with MSVC inline disabled because its GNU
  spelling is independently reserved. With its flag enabled, lexpp consumes
  `__pragma` as a preprocessing operator before keyword classification.
- GNU/MSVC policy diagnostics remain active in their enabled dialects under
  pedantic options. `_Imaginary` still reports the baseline unsupported-type
  diagnostic; classification does not claim imaginary type semantics.
- Phase A mechanically removes the old C99-only exhaustive matches around
  existing preprocessor extension policy. Their behavior is otherwise unchanged.
- Phase A introduced no lexer mode behavior, parser production, AST node, or
  semantic rule. Avoid editing configuration, CLI, token classification, and
  shared diagnostic surfaces in parallel later workstreams unless a missing seam
  is coordinated. Use the prepared feature vocabulary and keyword variants.
- Validation uses the existing pinned LLVM installation through an ignored
  worktree-local `target/llvm` junction; no tracked build setting is changed.

## ISO parser implementation and handoff

Evidence: [mode, policy, recovery, AST and inspection tests](src/translation_phases/parsing/tests/standards.rs),
[CLI mode tests](tests/cli.rs), [token-seam snapshot](tests/fixtures/lexing/iso_parser_token_seam.snap),
[policy diagnostic goldens](tests/fixtures/diagnostics/language/iso-c89-warning.stderr),
and [zero-global-allocation coverage](tests/allocation_count.rs).

| Revision | Implemented phase-7 surface | Later analysis boundary |
| --- | --- | --- |
| C89/C90/C95 | Native implicit int; C99-origin mixed blocks, for declarations, designated initializers, compound literals, flexible array members, long long, trailing enum comma, __func__, and unambiguous qualified/static/[*] array syntax follow Allow/Warn/Deny | Variable bounds need constant evaluation to distinguish VLAs from constant arrays; flexible-member position and object layout remain semantic |
| C99 | Existing full grammar; implicit int is retained with a policy diagnostic | Type/name/control-flow constraints remain as documented in the C99 checklist |
| C11/C17 | _Alignas expression/type operands, _Alignof types, _Atomic type/qualifier, _Generic associations, _Noreturn, _Static_assert, _Thread_local, anonymous untagged aggregates | Alignment validity, atomic eligibility, generic type compatibility/selection, assertion evaluation, storage-class combinations and aggregate layout |
| C23 | All [[...]] attribute positions with standard/vendor names and balanced arguments; bool/true/false, nullptr, constexpr, typeof/typeof_unqual, message-optional static_assert, alignas/alignof/thread_local aliases; labels before declarations and at block end; {}; _BitInt with signedness; fixed enum underlying specifier-qualifier lists; auto inferred type; unnamed definition parameters; decimal types; compound-literal storage classes | Attribute applicability/meaning, inferred types, width values, enum type legality, decimal support, literal object lifetime and removed old-style function constraints |
| C2y subset | _Countof unary expression or parenthesized type; type-controlling _Generic; if/switch declaration headers with optional following expression; case ranges; break/continue label | Array/type/count evaluation, selection conversion, range overlap, and named control-target resolution |

Reserved keywords are diagnosed by the foundation's token classifier, so parser
consumers do not duplicate keyword-origin diagnostics. Unambiguous grammar-only
features report through the shared extension emitter and keep their AST under
Deny. Non-reserved C23 aliases remain identifiers in earlier strict modes.
The ISO parser does not change lexer or preprocessing behavior.

New operands, assertions, generic selections, specifier additions and attributes
are immutable arena nodes. `ModernFrame` owns delimiters and delegates expression
and type children to the existing machine; no recursive grammar calls are added.
Node sizes are pinned. Inspection visits every new child, including compound-
literal storage, generic default arms, and fixed enum underlying types.

`AttributeSpecifier` retains a syntax discriminator, balanced original tokens,
provenance and recovery state. The GNU workstream adds `__attribute__` delimiter
entry paths while sharing its immutable representation and attachment points.
MSVC `__declspec` can add a further discriminator variant in its own workstream.

C2y grammar follows WG14 [N3388 selection declarations](https://open-std.org/jtc1/sc22/wg14/www/docs/n3388.htm),
[N3355 named loops](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n3355.htm),
[N3260 generic type operands](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n3260.pdf),
[N3370 case ranges](https://www.open-std.org/JTC1/SC22/WG14/www/docs/n3370.htm), and the draft _Countof grammar implemented in the
[GCC primary implementation patch](https://gcc.gnu.org/pipermail/gcc-patches/2025-May/683845.html).
The C2y subset intentionally excludes draft semantic analysis and lexer-owned
octal prefixes/delimited escapes.

## GNU phase-7 parser handoff

Reserved GNU spellings parse in every configured mode; the shared extension
policy chooses Allow/Warn/Deny without discarding the AST. Non-reserved `asm`,
`typeof`, and alternate keywords retain the foundation's mode gates. C23 `typeof`
and C2y case ranges retain their ISO status. Lexer and preprocessor behavior is
unchanged by this workstream.

GNU attributes use the same `AttributeSpecifier` and linked specifier-extension
representation as C23 attributes, distinguished by `AttributeSyntax::Gnu`.
Balanced argument tokens and original spellings remain available to analysis.
The parser accepts declaration, aggregate, enumerator, pointer, parameter,
nested declarator, array, bit-field, function and label attribute positions from
[GCC Attribute Syntax](https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html).

[GNU C Extensions](https://gcc.gnu.org/onlinedocs/gcc/C-Extensions.html) and
[GCC Extended Asm](https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html) specify the
vendor grammar. Assembly nodes preserve qualifiers/templates/constraints/clobbers
and parsed expression operands, symbolic names and goto labels. File assembly and
declarator assembly labels have explicit AST variants. Builtins preserve their
type/expression operands and offset member paths. Statement expressions, local
labels, label addresses, computed goto, omitted conditional operands, GNU
initializers and nested definitions also have inspectable syntax nodes.

`__extension__` suppresses pedantic extension diagnostics for its expression or
declaration, including a function body. Suppression restores at the owning frame
boundary and retains diagnostics from later unsuppressed occurrences, including
macro expansions. Nested functions isolate typedef, label and switch state.

These are syntax guarantees. Attribute application, target assembly constraints,
builtin evaluation, inferred types, label/control-target resolution, closure or
trampoline generation and object layout remain semantic/lowering work. Zero-length
array policy is diagnosed for literal zero bounds (including parentheses); general
constant-expression evaluation is deferred. Union-cast policy is diagnosed for
explicit union type names; resolving a typedef to a union is deferred. GNU empty
structures/unions remain complete syntax under all policies.

Evidence: `parsing::tests::gnu` covers grammar, AST ownership, provenance,
Allow/Warn/Deny across all revisions, every input prefix, following-declaration
recovery and deep nesting. CLI diagnostic goldens and the GNU token-seam snapshot
pin rendering and the unchanged phase-7 input. Allocation tests exercise the new
frame paths without global allocations.

## MSVC phase-7 parser handoff

All eight parser-owned groups are independently opt-in, regardless of ISO/GNU
mode. Disabled keyword spellings remain identifiers. Enabled reserved keywords
use the existing classifier's MSVC Allow/Warn/Deny diagnostics and retain their
syntax under Deny. Anonymous tagged members are a grammar-only extension and
report through the same policy emitter. The preprocessor implements `pragma` and
`va-args` before the parser consumes the resulting tokens.
No lexer/preprocessor behavior or keyword classification changed: all needed
parser-visible MSVC token kinds were already present.

- `__declspec(...)` shares `AttributeSpecifier`, attachment points and balanced
  argument tokens with ISO/GNU attributes; `AttributeSyntax::Msvc` distinguishes
  its single outer parenthesis. Space-separated and comma-separated modifiers,
  nested argument delimiters, spelling and provenance are retained.
- `__int8/16/32/64` retain their explicit width and optional signedness, including
  signed/unsigned on either side. Declaration and pointer qualifiers retain
  `__ptr32`, `__ptr64`, `__unaligned`, `__w64`, `__sptr` and `__uptr`; each pointer
  level owns its qualifier set. Calling conventions retain their source-backed
  declaration or declarator position, including nested function pointers,
  parameters and abstract declarators. `__forceinline` retains its modifier and
  inline property; GNU-reserved `__inline` stays available independently with
  the MS inline flag disabled, as established by the foundation.
- SEH owns `__try` guarded compounds, `__except` expression filters,
  `__finally` compounds and `__leave;`. Children use the existing machine's
  compound/expression frames and ordinary scope restoration. Missing handlers,
  bodies, filters and delimiters produce recovered syntax and preserve following
  input. Placement of `__leave` and handler semantics are later analysis.
- `__asm` / `_asm` brace blocks and single-line instructions have opaque arena
  token lists with balanced parentheses/brackets/braces. A repeated asm keyword
  separates single-line instructions. The enclosing C brace remains unconsumed.
  Line boundaries use macro invocation provenance and original source text,
  ignoring phase-2 backslash/trigraph splices, without changing lexing. Assembly
  semicolons remain opaque tokens; target instruction/comment interpretation is
  deferred. EOF and mismatched delimiters mark the assembly node recovered.
- Anonymous tagged struct/union definitions and references are accepted only
  with `-fms-anonymous-structs`. ISO untagged anonymous members retain their
  independent C11 status; resolving typedefs and promoting members are analysis.

Microsoft primary grammar references: [declspec](https://learn.microsoft.com/en-us/cpp/cpp/declspec),
[inline assembly](https://learn.microsoft.com/en-us/cpp/assembler/inline/asm),
[try-except](https://learn.microsoft.com/en-us/cpp/cpp/try-except-statement),
[try-finally](https://learn.microsoft.com/en-us/cpp/cpp/try-finally-statement), and
[anonymous class types](https://learn.microsoft.com/en-us/cpp/cpp/anonymous-class-types).
ABI selection, fixed-width target types, qualifier applicability, attributes,
SEH control transfer and assembly meaning remain outside this syntax workstream.

Evidence: `parsing::tests::msvc` covers each feature enabled alone and disabled
under the umbrella, ISO/GNU modes, all policy severities, immutable nodes,
provenance, every input prefix, recovery and deep SEH nesting. CLI tests pin all
sub-flags, inspection, enabled/disabled token snapshots and diagnostic goldens.
`node_sizes` pins the new arena nodes; the benchmarking-only MSVC parse seam
exercises valid and recovered paths under the zero-global-allocation check.

## Lexical and preprocessing implementation

Phase 1 trigraph replacement and phase 2 splicing use the same mode gate in the
main lexer, included files, terminal-splice diagnostics, and written header-name
reconstruction. Strict C89-C17 translate trigraphs; GNU modes and C23/C2y preserve
them. Strict C89 does not combine digraphs; C95 adds them. Strict C89/C95 retain
`//` as two `/` tokens. GNU89 accepts both features and reports their origins
under pedantic policy. Diagnostics are deferred with the batch token entries,
so skipped conditional groups do not report unused extension spellings.

Phase 3 recognizes `u`, `U`, and `u8` string prefixes from C11, `u`/`U` character
prefixes from C11, and `u8` character prefixes from C23. Earlier modes preserve
the identifier followed by a literal. C23 digit separators are part of
pp-numbers and must lie between digits of the constant's radix. C23 binary and
`wb`/`uwb` suffixes and C2y `0o`/`0O` octal prefixes convert to typed values.
Suffix recognition also follows mode gates when tokens are pasted. GNU binary
constants before C23 report the C23 origin; GNU imaginary `i`/`j` suffixes retain
the real component's precision and signedness, including either order with
ordinary floating suffixes. Dollar identifiers, including continuations, report
GNU policy diagnostics. C89 long-long suffixes and hexadecimal floating
constants report C99 policy diagnostics while retaining values.

C23 pp-number separator boundaries include identifier nondigits (ASCII,
Unicode and universal-character names), even when conversion must reject the
resulting constant. C23 permits universal names for basic/control characters
inside literals while retaining invalid-scalar checks; earlier modes retain the
C99/C11 low-code-point restriction.

Phase 5 preserves Unicode encoding in literal tokens and distinguishes numeric
escapes from source characters. UTF-8/UTF-16/UTF-32 character constants must fit
one code unit; numeric escapes have the selected code-unit range. C2y's scoped
delimited-escape support is `\x{...}` and `\o{...}`, with empty, unterminated,
invalid-radix, and out-of-range forms diagnosed. Phase 6 concatenates ordinary
strings with encoded strings while retaining the encoding, and diagnoses
incompatible nonordinary prefixes. Phase 7 adds encoding-bearing literal,
bit-precise integer, and imaginary constant token variants. The existing integer
magnitude ceiling remains 64 bits: `wb` can describe a signed 65-bit type for a
64-bit positive magnitude, but larger magnitudes still diagnose overflow. This
is token conversion, not arbitrary-precision arithmetic or type semantics.

C23 `#elifdef`/`#elifndef`, `#warning`, `#embed`, `__has_include`, `__has_embed`,
`__has_c_attribute`, and `__VA_OPT__` also work as earlier GNU extensions.
`#warning` always emits its message, with an additional policy diagnostic when
its spelling is an extension. `#embed` reads real files as 8-bit resource
elements, sharing include search paths, and implements `limit`, `prefix`,
`suffix`, `if_empty`, and their double-underscore aliases. Limits use the existing
integer preprocessor evaluator and reject negative values and `defined`.
Written quoted/angle names are read before macro replacement inside their
boundaries, including digraph-looking punctuation and dollar signs as header
characters; macro-produced names and builtin strings such as `__FILE__` are
expanded normally. Missing operands and missing closing quotes diagnose. Resource queries
return 0 for missing resources, 1 for found resources, and `__has_embed` returns
2 for an empty effective resource, including `limit(0)`. The three
`__STDC_EMBED_*__` constants are predefined when resource inclusion is enabled.
An unsupported qualified embed parameter returns 0 in `__has_embed`; a direct
`#embed` diagnoses it. Malformed parameters diagnose in both forms. Query
nesting is bounded at 64 to turn adversarial recursive operands into a diagnostic
instead of exhausting the native stack. Resource and C attribute queries are
restricted to preprocessing conditional expressions (C23 §6.10.2p11); their
names remain defined for `defined`, `#ifdef` and related macro tests. Invalid
operator openings preserve the following token or directive boundary.

`__has_c_attribute` returns 202311 for the seven standard C23 attribute names,
including double-underscore aliases, and 0 for unknown names. GNU
`__has_attribute` returns 1 for the syntax-supported set `unused`, `deprecated`,
`aligned`, `packed`, `noreturn`, `weak`, `section`, `visibility`, `format`,
`always_inline`, and `noinline`; `__has_builtin` returns 1 for
`__builtin_va_arg`, `__builtin_offsetof`, `__builtin_types_compatible_p`, and
`__builtin_choose_expr`. Other names return 0. These query tables describe the
intended syntax-front-end subset, not backend effects or a GCC/clang version.
Both queries are available through their reserved GNU spellings in strict modes
and report GNU policy diagnostics.

`__VA_OPT__` selects its inner replacement after variadic argument expansion;
inner substitution and pasting precede outer stringification or pasting.
Placemarkers survive until the outer paste is complete. Definition-time checks
reject use outside a variadic replacement, nested optional replacements,
missing parentheses, and `##` at either inner boundary. C23 permits omitted
variadic arguments; pre-C23 pedantic modes retain the existing omission
diagnostic. C89 fixed and variadic empty arguments and variadic definitions
report C99 origin. Named `args...` definitions normalize the name to the same
variadic machinery and report GNU origin.
Original named variadic parameter IDs remain in definition metadata so identical
redefinitions compare normalized bodies without losing parameter-name checks,
even when the variadic parameter is unused.

GNU `, ## __VA_ARGS__` elides a comma for an omitted variadic argument, preserves
it for an explicitly empty argument, and substitutes supplied arguments without
pasting a comma. For a macro with only `...`, an empty invocation elides in GNU
modes and retains the comma in strict modes, following the documented
[GCC distinction](https://gcc.gnu.org/onlinedocs/cpp/Variadic-Macros.html).
MSVC `-fms-va-args` independently elides a preceding comma when the variadic
argument expands to nothing, following the traditional
[MSVC behavior](https://learn.microsoft.com/en-us/cpp/preprocessor/variadic-macros?view=msvc-170).

GNU `#include_next` continues after the actual configured search entry that
provided the current header, even when the next directive changes quote/angle
form. A quoted include found next to its including file has no configured entry;
its `#include_next` starts at the first configured directory. This follows
[GCC's search-order description](https://gcc.gnu.org/onlinedocs/cpp/Wrapper-Headers.html).
`#ident`/`#sccs` require a string and are consumed as metadata directives; this
syntax-only frontend emits no object-file metadata. `__COUNTER__` starts at 0
for each translation unit and increments only when expanded. MSVC
`-fms-pragma` consumes `__pragma(...)` tokens and passes their payload to the
existing pragma handler, retaining source provenance. It is independent of
`-fms-va-args` and of the selected standard, consistent with the
[MSVC operator documentation](https://learn.microsoft.com/en-us/cpp/preprocessor/pragma-directives-and-the-pragma-keyword?view=msvc-170).

No keyword classification or parser grammar changed in lexpp. The small shared
changes are exhaustive literal-display arms in `src/cli.rs` and
`src/translation_phases/parsing/inspection.rs`, a mode-aware diagnostic measurement
adapter exported from `src/lib.rs`,
a context iterator exposing stable include search ranks, and activation of the
prepared extension emitter. The original measurement API retains its library
default mode. The adapter parses mode/policy flags before measured intervals;
CLI setup allocations are excluded as before. Tests cover all seven ISO
revisions and their GNU counterparts,
Allow/Warn/Deny policies, MSVC flags independently, written and expanded real
resource paths, include-next chains, optional replacement examples from C23,
and malformed/truncated input recovery. The diagnostic harness accepts optional
`.args` sidecars to render the selected mode and policy.

### Lexpp performance verification (8 October 2026)

Baseline: foundation commit `f73b9cd`. Feature implementation: `2ee1605`, then
named-variadic redefinition fix `d6b516a`. The final timing artifact is
`d6b516a` plus the two `#[inline(always)]` annotations on `Lexer::lex_token`
and `Lexer::lex_number`. The resource-header correction `228a5b9` affects
query/resource paths absent from these three benchmark inputs. No parser-only
performance claim is made: parser grammar and keyword classification are outside
this workstream.

Windows x86-64, Rust 1.99.0, native LLVM 23.1.1, static native C archive and
Rust/C fat LTO; release builds with `benchmarking-internals`, without
`portable-simd`. Both executables use the unchanged strict-C99 library benchmark
configuration. Seven interleaved baseline/current runs alternate their order.
Criterion parameters: 0.2 s warm-up, 0.5 s requested measurement, 10 samples;
the preprocessor group overrides this to 20 samples and Criterion extends short
measurement windows. Each cell below is the minimum **mean point estimate** of
the seven runs, followed by that run's 95% bootstrap confidence interval, in ms.
These are lexer-only and phases-1-through-6 timings, respectively.

| Workload | Baseline min mean [95% CI], ms | Lexpp min mean [95% CI], ms | Change |
| --- | ---: | ---: | ---: |
| Lexer / one million lines | 496.22 [491.56, 501.66] | 470.93 [464.77, 479.87] | -5.1% |
| Lexer / mixed c99 workload | 347.98 [346.38, 350.26] | 361.93 [360.26, 363.75] | +4.0% |
| Lexer / macro-heavy workload | 109.64 [109.10, 110.25] | 116.97 [116.52, 117.42] | +6.7% |
| Preprocessor / one million lines | 810.26 [800.85, 822.05] | 837.72 [820.95, 855.73] | +3.4% |
| Preprocessor / mixed c99 workload | 707.33 [704.78, 710.17] | 722.01 [718.84, 725.44] | +2.1% |
| Preprocessor / macro-heavy workload | 617.61 [614.72, 620.84] | 621.92 [620.04, 623.98] | +0.7% |

An earlier seven-run comparison without the annotations measured +9.2% mixed
and +10.8% macro-heavy lexer overhead. Symbol inspection showed that LLVM had
outlined these two hot methods after the additional feature arms enlarged them;
the baseline had inlined both. The annotations restore that property. The
remaining overhead is reported above, not treated as a parser improvement.

The memory harness reports identical rounded arena high-water marks for baseline
and the optimized lexpp build: PP arenas 145.9/144.3/55.0 MiB, expansion arenas
0.1/0.1/37.7 KiB, parse arenas 1.6/10.5/9.5 KiB, and TU arenas
238.4/181.0/147.1 MiB for the plain/mixed/macro workloads. Peak arena commits are
526.1/454.1/344.6 MiB; peak regions are 8/9/8 (800/900/800 GiB reserved).
OS peak commit and working set are unchanged to within 0.1 MiB: phases 1-6 commit
458.9/412.8/227.0 MiB and working set 480.2/434.7/233.9 MiB; phases 1-7 commit
528.0/455.9/346.1 MiB and working set 548.0/476.7/338.3 MiB.

Temporary counters in isolated benchmark copies (including the final resource
correction) count every iteration of the preprocessor driver's outer token loop
and sample its live preprocessing provenance before that iteration. They are not
part of the production changes. Output token counts, driver steps, and total
retained source segments match the baseline exactly:

| Input | Output tokens | Driver steps | Steps/token | Retained segments | Peak live PP vectors, baseline → lexpp |
| --- | ---: | ---: | ---: | ---: | ---: |
| one million lines | 5,000,000 | 6,000,001 | 1.200 | 5,000,001 | 1 → 1 |
| mixed C99 workload | 5,240,000 | 5,800,001 | 1.107 | 5,240,001 | 2 → 2 |
| macro-heavy workload | 3,260,000 | 6,100,012 | 1.871 | 3,300,001 | 194 → 311 |

The extra macro-definition preprocessing provenance is bounded by setup in this
input and does not grow across its repeated expansions. Ordered provenance,
macro/include locations, recovery, query-depth bounds and token values are
covered by snapshots, structured tests and rendered diagnostics. All six
canonical checks pass, including the eight feature-enabled allocation tests;
rendering every diagnostic golden continues to require zero global allocations.

## Integrated CLI verification

The integration branch merges `std/parse` before `std/lexpp`, preserving each
workstream's implementation and diagnostic coverage. The only textual merge
conflicts were this document's status matrix/implementation sections and the
diagnostic coverage document; both sets of completed behavior were retained.

[End-to-end tests](tests/language_cli.rs) run the compiled CLI through preprocessing,
parsing and syntax inspection for every accepted standard spelling. Representative
strict programs use native syntax under `-pedantic-errors`; GNU programs combine
named variadic macros, dollar identifiers, keyword aliases, inline, asm,
statement expressions and imaginary constants. MSVC programs combine macro-produced
`__pragma`, declspec, integer types, calling conventions, pointer qualifiers, SEH,
assembly and empty variadic comma elision in every ISO revision. Tests cover later
flag overrides, disabled gates, older-mode identifier preservation and malformed
cross-phase inputs with a following declaration.

C attribute queries report the syntax-recognized subset: all seven names can
reach retained `[[...]]` nodes, while unknown/vendor queries return zero.
Attribute meaning and applicability remain later analysis. `#embed` and
`__has_embed` use the same real resource; emitted bytes, prefix/suffix, limits and
empty-resource replacement reach ordinary initializer elements. Numeric and
character literal variants, including C23 bit-precise and C2y literals, reach
expression nodes without introducing semantic evaluation.

GNU imaginary integer values previously displayed only their component magnitude,
while floating imaginary values displayed `i`. Both the token dump and syntax
inspection now retain `i` consistently, including unsigned and long components.
The verified lexical snapshot and parser AST/inspection regression pin this
behavior. No feature gate, diagnostic severity, arena rule or canonical check
was relaxed during integration.

Integration validation (8 October 2026): `cargo test --all-targets` passes 763
tests, including all seven new CLI integration tests and the combined diagnostic
golden. `cargo test --features benchmarking-internals --test allocation_count`
passes all 11 tests. `cargo +nightly fmt --check`, both canonical Clippy commands
with `-D warnings`, and `git diff --check` pass. Work remains syntax-only;
semantic analysis and code generation are outside this integration.
