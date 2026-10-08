# Language standards

bcc-rust parses C89 through C23, a listed subset of the C2y draft, GNU
dialects, and independently enabled MSVC extensions. One configuration value,
CLI selection, predefined macros and a shared extension-diagnostic emitter
connect the lexer, preprocessor and explicit-frame parser. This is a
front end with an initial declaration semantic pass: selecting a mode is not
a conformance claim. [semantic-analysis.md](semantic-analysis.md) records the
C99 semantic subset and conservative extension boundaries; code generation is absent.

## Status and known gaps

Every mode, alias, policy flag and MSVC group below, and every row of the
feature matrix, is implemented through the batch pipeline. These limits remain:

- Type checking, constant evaluation outside preprocessing, attribute meaning,
  target assembly validation, ABI and calling-convention effects and code
  generation belong to later phases. Syntax alone cannot settle every GNU
  zero-length array or union cast: the policy diagnostic covers literal zero
  bounds (parenthesized too) and explicitly written union types, while general
  constant bounds and typedefs naming unions wait for analysis.
- Decimal type keywords parse, but the decimal floating suffixes `df`, `dd` and
  `dl` and decimal values are not implemented.
- Integer constants are limited to 64-bit magnitudes. A signed `wb` constant may
  take a 65-bit type for a 64-bit magnitude; larger magnitudes are diagnosed.
  Preprocessing expressions use a 64-bit intmax/uintmax model.
- C2y is the subset listed under [Decisions](#decisions), not every current or
  future draft change.
- The unified extension policy reports reserved GNU and MSVC constructs that
  GCC or Clang sometimes exempt from pedantic diagnostics. Only `-pedantic`,
  `-Wpedantic` and `-pedantic-errors` select a policy; other warning options,
  including `-Wno-pedantic`, are not implemented.
- `__extension__` suppresses keyword, grammar and numeric-constant extension
  diagnostics within its owning declaration or expression. Lexical diagnostics
  (dollar identifiers, `//` comments, digraphs), directives and macro
  definitions stay outside its scope.
- An extension diagnostic raised in a macro expansion may underline the
  replacement in the macro definition, and a conditional-expression diagnostic
  may keep only the definition location; no expansion backtrace is rendered.
  Preprocessing diagnostics precede parser diagnostics by pipeline design.
- The added syntax costs parser time and retained memory; see
  [Performance](#performance).

## CLI reference

The CLI defaults to `gnu17`; `CompilerConfiguration::default()` remains strict
C99 with `ExtensionPolicy::Allow` for library and test callers. `-std=VALUE`,
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
MSVC flags affect neither version nor strictness. `__STDC__`,
`__STDC_HOSTED__` (freestanding: `0`), `__DATE__`/`__TIME__`, and
`__STDC_MB_MIGHT_NEQ_WC__` are predefined in every mode. `__GNUC__` and `_MSC_VER` are deliberately not
defined.

`-pedantic` and `-Wpedantic` select Warn; `-pedantic-errors` selects Deny;
the default is Allow. Later policy flags win. Deny emits an error but keeps
classified tokens available for structured parser recovery. Preprocessor
extensions with their own diagnostics keep their established recovery.

MSVC groups are independently opt-in. `-fms-extensions` enables all ten groups;
`-fno-ms-extensions` disables them. Each group has `-fms-NAME` and
`-fno-ms-NAME`; later flags win, including umbrellas.

| NAME | Spellings / behavior | Phase |
| --- | --- | --- |
| `declspec` | `__declspec(...)` | parser |
| `int-types` | `__int8`, `__int16`, `__int32`, `__int64` | parser |
| `calling-conventions` | `__cdecl`, `__stdcall`, `__fastcall`, `__vectorcall`, `__thiscall` | parser |
| `type-qualifiers` | `__ptr32`, `__ptr64`, `__unaligned`, `__w64`, `__sptr`, `__uptr` | parser |
| `inline` | `__forceinline`; GNU `__inline` also available independently | parser |
| `seh` | `__try`, `__except`, `__finally`, `__leave` | parser |
| `asm` | `__asm`, `_asm` blocks | parser |
| `pragma` | `__pragma(...)` preprocessing operator | preprocessor |
| `anonymous-structs` | anonymous tagged struct/union members | parser |
| `va-args` | empty `__VA_ARGS__` comma elision | preprocessor |

Example: `bcc-rust -std=c89 -pedantic -fms-extensions -fno-ms-seh input.c`.
Invalid standard values produce the exact clang-style error and thirteen
notes (deprecated aliases are accepted but omitted from the notes).

## Configuration model

`src/configuration.rs` owns the value object. `CStandard` is ordered
chronologically (C89 and C90 are the same value; C95 distinguishes amendment 1).
`LanguageMode` pairs a standard with a GNU bit. `MsvcFeature` uses an
independent ten-bit set. All phases read the single configuration through
`Context.configuration`.

`Feature::ALL` and `Feature::origin()` are the canonical feature vocabulary.
`configuration.accepts(feature)` is a precomputed bit test describing whether
the spelling or construct may be consumed, including as an extension;
`is_native(feature)` is a second bit test describing native availability. Both
describe the target contract, **not implementation completion**. Constructors
and builders recompute the bitsets once; there is no per-token string matching
or global configuration. `ImplicitInt` has a `FeatureOrigin::Removed` origin:
native before C99 and reported afterward as a C89 feature removed in C99.
`Trigraphs` ceases to be native in C23 through a dedicated derivation case.

`Context::report_extension(feature, spelling, source_vectors)` in
`translation_phases/extension.rs` is the shared emitter;
`report_extension_since(spelling, FeatureOrigin, source_vectors)` is its
explicit-origin form. Native standard features emit nothing; otherwise Allow
emits nothing, Warn emits a warning, and Deny emits an error. Messages are
`'SPELLING' is a C11 extension`, `'SPELLING' is a C89 feature removed in C99`,
`'SPELLING' is a GNU extension`, or `'SPELLING' is an MSVC extension`.
GNU/MSVC constructs remain non-ISO under pedantic flags even when enabled. The
diagnostic retains provenance through preprocessor compaction and the ordinary
FIFO renderer.

`KeywordTokenType::classify(id, configuration)` is the **only** keyword
classifier. `ALL` lists the canonical parser kinds; an explicit alias table
gives each alternate spelling its keyword, mode gate and diagnostic origin, and
its spellings (`ALIASES`) follow `ALL` in the reserved interner prefix.
`KeywordClassification` returns the canonical kind, original static spelling,
and optional origin. C99 keyword IDs are stable. New grammar consumes these
kinds rather than adding a second spelling classifier, and parser consumers keep
the original tokens for spelling and provenance.

## Feature matrix

ISO columns give revision availability: `Y` means native and `-` means not
native. GNU modes use the same ISO revision and additionally make GNU-origin rows
native; MSVC-origin rows become native when their flag is enabled. The
acceptance column describes the gate: `extension` accepts reserved or
unambiguous syntax in older modes with a policy diagnostic, `native` accepts the
spelling only where it is native, `GNU earlier` also accepts it in older GNU
modes, and `MS flag` requires its independent flag. `Trigraphs` is special: it
is accepted only in strict C89 through C17. Every row is implemented with its
mode gate and policy diagnostics (`Imaginary` reports an unsupported type); a
`Y` promises no semantic checks, target support, or code generation.

| Feature (`Feature` variant) | C89 | C95 | C99 | C11 | C17 | C23 | C2y | Acceptance | Phase |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Digraphs | - | Y | Y | Y | Y | Y | Y | C95+ / GNU earlier | lexer/preprocessor |
| LineComments | - | - | Y | Y | Y | Y | Y | GNU earlier | lexer/preprocessor |
| Trigraphs | Y | Y | Y | Y | Y | - | - | strict through C17 | lexer/preprocessor |
| UnicodeLiteralPrefixes | - | - | - | Y | Y | Y | Y | native | lexer/preprocessor |
| Utf8CharacterConstants | - | - | - | - | - | Y | Y | native | lexer/preprocessor |
| DigitSeparators | - | - | - | - | - | Y | Y | native | lexer/preprocessor |
| BitIntSuffixes | - | - | - | - | - | Y | Y | native | lexer/preprocessor |
| BinaryConstants | - | - | - | - | - | Y | Y | GNU earlier | lexer/preprocessor |
| Elifdef | - | - | - | - | - | Y | Y | GNU earlier | lexer/preprocessor |
| WarningDirective | - | - | - | - | - | Y | Y | GNU earlier | lexer/preprocessor |
| Embed | - | - | - | - | - | Y | Y | GNU earlier | lexer/preprocessor |
| HasInclude | - | - | - | - | - | Y | Y | GNU earlier | lexer/preprocessor |
| HasEmbed | - | - | - | - | - | Y | Y | GNU earlier | lexer/preprocessor |
| HasCAttribute | - | - | - | - | - | Y | Y | GNU earlier | lexer/preprocessor |
| VaOpt | - | - | - | - | - | Y | Y | GNU earlier | lexer/preprocessor |
| OctalPrefix | - | - | - | - | - | - | Y | native | lexer/preprocessor |
| DelimitedEscapes | - | - | - | - | - | - | Y | native | lexer/preprocessor |
| HexFloats | - | - | Y | Y | Y | Y | Y | extension | lexer/preprocessor |
| VariadicMacros | - | - | Y | Y | Y | Y | Y | extension | lexer/preprocessor |
| EmptyMacroArguments | - | - | Y | Y | Y | Y | Y | extension | lexer/preprocessor |
| Inline | - | - | Y | Y | Y | Y | Y | GNU earlier | parser |
| Restrict | - | - | Y | Y | Y | Y | Y | native | parser |
| Bool | - | - | Y | Y | Y | Y | Y | extension | parser |
| Complex | - | - | Y | Y | Y | Y | Y | extension | parser |
| Imaginary | - | - | Y | Y | Y | Y | Y | extension | parser (unsupported-type diagnostic) |
| ImplicitInt | Y | Y | - | - | - | - | - | extension | parser |
| MixedDeclarations | - | - | Y | Y | Y | Y | Y | extension | parser |
| ForDeclarations | - | - | Y | Y | Y | Y | Y | extension | parser |
| DesignatedInitializers | - | - | Y | Y | Y | Y | Y | extension | parser |
| CompoundLiterals | - | - | Y | Y | Y | Y | Y | extension | parser |
| FlexibleArrayMembers | - | - | Y | Y | Y | Y | Y | extension | parser |
| LongLong | - | - | Y | Y | Y | Y | Y | extension | parser |
| TrailingEnumComma | - | - | Y | Y | Y | Y | Y | extension | parser |
| Func | - | - | Y | Y | Y | Y | Y | extension | parser |
| StaticAssert | - | - | - | Y | Y | Y | Y | extension | parser |
| Generic | - | - | - | Y | Y | Y | Y | extension | parser |
| Alignas | - | - | - | Y | Y | Y | Y | extension | parser |
| Alignof | - | - | - | Y | Y | Y | Y | extension | parser |
| Noreturn | - | - | - | Y | Y | Y | Y | extension | parser |
| ThreadLocal | - | - | - | Y | Y | Y | Y | extension | parser |
| Atomic | - | - | - | Y | Y | Y | Y | extension | parser |
| AnonymousAggregates | - | - | - | Y | Y | Y | Y | extension | parser |
| C23Keywords | - | - | - | - | - | Y | Y | native | parser |
| Attributes | - | - | - | - | - | Y | Y | extension | parser |
| BitInt | - | - | - | - | - | Y | Y | extension | parser |
| DecimalTypes | - | - | - | - | - | Y | Y | extension | parser |
| EmptyInitializers | - | - | - | - | - | Y | Y | extension | parser |
| EnumUnderlyingType | - | - | - | - | - | Y | Y | extension | parser |
| AutoTypeInference | - | - | - | - | - | Y | Y | extension | parser |
| Constexpr | - | - | - | - | - | Y | Y | extension | parser |
| Nullptr | - | - | - | - | - | Y | Y | extension | parser |
| Countof | - | - | - | - | - | - | Y | extension | parser |
| IfSwitchDeclarations | - | - | - | - | - | - | Y | extension | parser |
| NamedLoops | - | - | - | - | - | - | Y | extension | parser |
| GenericTypeOperand | - | - | - | - | - | - | Y | extension | parser |
| CaseRanges | - | - | - | - | - | - | Y | extension | parser |
| GnuAttribute | - | - | - | - | - | - | - | extension (GNU native) | parser |
| GnuAsm | - | - | - | - | - | - | - | extension (GNU native) | parser |
| GnuTypeof | - | - | - | - | - | - | - | extension (GNU native) | parser |
| ExtensionMarker | - | - | - | - | - | - | - | extension (GNU native) | parser |
| StatementExpressions | - | - | - | - | - | - | - | extension (GNU native) | parser |
| BuiltinVaArg | - | - | - | - | - | - | - | extension (GNU native) | parser |
| BuiltinOffsetof | - | - | - | - | - | - | - | extension (GNU native) | parser |
| BuiltinTypesCompatible | - | - | - | - | - | - | - | extension (GNU native) | parser |
| BuiltinChooseExpr | - | - | - | - | - | - | - | extension (GNU native) | parser |
| LabelsAsValues | - | - | - | - | - | - | - | extension (GNU native) | parser |
| LocalLabels | - | - | - | - | - | - | - | extension (GNU native) | parser |
| OmittedConditionalOperand | - | - | - | - | - | - | - | extension (GNU native) | parser |
| ZeroLengthArrays | - | - | - | - | - | - | - | extension (GNU native) | parser |
| Int128 | - | - | - | - | - | - | - | extension (GNU native) | parser |
| AutoType | - | - | - | - | - | - | - | extension (GNU native) | parser |
| GnuAlternateKeywords | - | - | - | - | - | - | - | extension (GNU native) | parser |
| RealImag | - | - | - | - | - | - | - | extension (GNU native) | parser |
| GnuDesignators | - | - | - | - | - | - | - | extension (GNU native) | parser |
| UnionCasts | - | - | - | - | - | - | - | extension (GNU native) | parser |
| EmptyStructs | - | - | - | - | - | - | - | extension (GNU native) | parser |
| NestedFunctions | - | - | - | - | - | - | - | extension (GNU native) | parser |
| ImaginaryConstants | - | - | - | - | - | - | - | extension (GNU native) | lexer/preprocessor |
| DollarIdentifiers | - | - | - | - | - | - | - | extension (GNU native) | lexer/preprocessor |
| NamedVariadicMacros | - | - | - | - | - | - | - | extension (GNU native) | lexer/preprocessor |
| GnuVaArgs | - | - | - | - | - | - | - | extension (GNU native) | lexer/preprocessor |
| IncludeNext | - | - | - | - | - | - | - | extension (GNU native) | lexer/preprocessor |
| IdentDirective | - | - | - | - | - | - | - | extension (GNU native) | lexer/preprocessor |
| Counter | - | - | - | - | - | - | - | extension (GNU native) | lexer/preprocessor |
| HasAttribute | - | - | - | - | - | - | - | extension (GNU native) | lexer/preprocessor |
| HasBuiltin | - | - | - | - | - | - | - | extension (GNU native) | lexer/preprocessor |
| MsDeclspec | - | - | - | - | - | - | - | MS flag | parser |
| MsIntTypes | - | - | - | - | - | - | - | MS flag | parser |
| MsCallingConventions | - | - | - | - | - | - | - | MS flag | parser |
| MsTypeQualifiers | - | - | - | - | - | - | - | MS flag | parser |
| MsInline | - | - | - | - | - | - | - | MS flag | parser |
| MsSeh | - | - | - | - | - | - | - | MS flag | parser |
| MsAsm | - | - | - | - | - | - | - | MS flag | parser |
| MsPragma | - | - | - | - | - | - | - | MS flag | lexer/preprocessor |
| MsAnonymousStructs | - | - | - | - | - | - | - | MS flag | parser |
| MsVaArgs | - | - | - | - | - | - | - | MS flag | lexer/preprocessor |

Lexer and preprocessor rows also cover exact prefix and suffix recognition,
`//` in GNU89 pedantic mode, `#elifndef`, `#sccs`, `, ## __VA_ARGS__`, and the
query predicates for builtins grouped under one row. GNU parser rows cover every
GNU attribute position, basic, extended and goto assembly with declarator
assembly labels, GNU range and old-style designators, and syntax-only nested
functions.

## ISO syntax

| Revision | Phase-7 syntax | Left to later analysis |
| --- | --- | --- |
| C89/C90/C95 | Native implicit int; C99-origin mixed blocks, for declarations, designated initializers, compound literals, flexible array members, long long, trailing enum comma, `__func__`, and unambiguous qualified/static/`[*]` array syntax follow Allow/Warn/Deny | Variable bounds need constant evaluation to distinguish VLAs from constant arrays; flexible-member position and object layout |
| C99 | Full C99 grammar; implicit int is retained with a removed-feature policy diagnostic | Type/name/control-flow constraints listed in the C99 checklist |
| C11/C17 | `_Alignas` expression/type operands, `_Alignof` types (parenthesized expressions as a GNU extension), `_Atomic` type/qualifier, `_Generic` associations, `_Noreturn`, `_Static_assert`, `_Thread_local`, anonymous untagged aggregates; `_Alignas` in a plain type name is a constraint error | Alignment validity, atomic eligibility, generic type compatibility/selection, assertion evaluation, storage-class combinations and aggregate layout |
| C23 | All `[[...]]` attribute positions with standard/vendor names and balanced arguments; `bool`/`true`/`false`, `nullptr`, `constexpr`, `typeof`/`typeof_unqual`, message-optional `static_assert`, `alignas`/`alignof`/`thread_local` aliases; labels before declarations and at block end; `{}`; `_BitInt` with signedness; fixed enum underlying specifier-qualifier lists; `auto` type inference, alone or beside another storage class except `typedef`; unnamed definition parameters; ellipsis-only prototypes; empty parameter lists as prototypes; rejected identifier-list declarators; decimal type keywords; compound-literal storage classes | Attribute applicability/meaning, inferred types, width values, enum type legality, decimal literal support and literal object lifetime |
| C2y subset | `_Countof` unary expression or parenthesized type; type-controlling `_Generic`; `if`/`switch` declaration headers with an optional following expression; case ranges; `break`/`continue` label | Array/type/count evaluation, selection conversion, range overlap, and named control-target resolution |

The keyword classifier reports reserved keywords, so parser consumers do not
duplicate keyword-origin diagnostics. Unambiguous grammar-only features report
through the shared emitter and keep their AST under Deny. Non-reserved C23
aliases remain identifiers in earlier strict modes.

New operands, assertions, generic selections, specifier additions and
attributes are immutable arena nodes. `ModernFrame` owns their delimiters and
delegates expression and type children to the machine stack; no grammar
recursion is added. `parsing/tests/node_sizes.rs` pins node sizes. Inspection
visits every new child, including compound-literal storage, generic default
arms, and fixed enum underlying types.

`AttributeSpecifier` is shared by `[[...]]`, GNU `__attribute__((...))` and
MSVC `__declspec(...)`; `AttributeSyntax` distinguishes them. Each keeps its
balanced original tokens, provenance and recovery state, and attaches at the
same points. An attribute after `*` belongs to that pointer level.

C2y grammar follows WG14 [N3388 selection declarations](https://open-std.org/jtc1/sc22/wg14/www/docs/n3388.htm),
[N3355 named loops](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n3355.htm),
[N3260 generic type operands](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n3260.pdf),
[N3370 case ranges](https://www.open-std.org/JTC1/SC22/WG14/www/docs/n3370.htm), and the draft `_Countof` grammar implemented in the
[GCC primary implementation patch](https://gcc.gnu.org/pipermail/gcc-patches/2025-May/683845.html).
The C2y octal prefixes and delimited escapes are lexical; see
[Lexical and preprocessing behavior](#lexical-and-preprocessing-behavior).

## GNU syntax

Reserved GNU spellings parse in every mode; the extension policy chooses
Allow/Warn/Deny without discarding the AST. Non-reserved `asm` and `typeof` are
keywords only in GNU modes, except that `typeof` is native from C23; before C23
it reports a GNU origin. C2y case ranges keep their ISO origin.

GNU attributes accept declaration, aggregate, enumerator, pointer, parameter,
nested declarator, array, bit-field, function and label positions from
[GCC Attribute Syntax](https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html).

[GNU C Extensions](https://gcc.gnu.org/onlinedocs/gcc/C-Extensions.html) and
[GCC Extended Asm](https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html) specify
the vendor grammar. `GnuFrame` owns assembly, builtins and `__label__`
declarations. Assembly nodes keep typed qualifiers, the template literal,
constraint and clobber literals, parsed operand expressions, symbolic names and
goto labels; file assembly and declarator assembly labels have explicit AST
variants. Builtins keep their type/expression operands and `__builtin_offsetof`
member paths. Statement expressions, local labels, label addresses, computed
goto, omitted conditional operands, GNU initializers and nested definitions
also have inspectable syntax nodes.

`__extension__` suppresses extension diagnostics for keyword, grammar and
numeric-constant tokens within its expression or declaration, including a
function body or a parameter. Suppression ends at the owning frame boundary;
later unsuppressed occurrences, including ones from macro expansions, keep their
diagnostics. Nested functions isolate typedef, label and switch state. GNU empty
structures and unions remain complete syntax under every policy.

## MSVC syntax

The eight parser-owned groups are independently opt-in in every ISO and GNU
mode. Disabled keyword spellings remain identifiers. Enabled reserved keywords
use the classifier's MSVC Allow/Warn/Deny diagnostics and keep their syntax
under Deny. Anonymous tagged members are a grammar-only extension reported
through the same emitter. The preprocessor implements `pragma` and `va-args`
before the parser sees the resulting tokens.

- `__declspec(...)` uses the shared `AttributeSpecifier` with
  `AttributeSyntax::Msvc` and a single outer parenthesis. Space- and
  comma-separated modifiers, nested argument delimiters, spelling and provenance
  are retained.
- `__int8/16/32/64` keep their explicit width and optional signedness, including
  `signed`/`unsigned` on either side. Declaration and pointer qualifiers keep
  `__ptr32`, `__ptr64`, `__unaligned`, `__w64`, `__sptr` and `__uptr`; each
  pointer level owns its qualifier set. Calling conventions keep their
  declaration or declarator position, including nested function pointers,
  parameters and abstract declarators. `__forceinline` keeps its modifier and
  inline property.
- `MsvcFrame` owns SEH: `__try` guarded compounds, `__except` expression
  filters, `__finally` compounds and `__leave;`. Children use the machine's
  compound and expression frames and ordinary scope restoration. Missing
  handlers, bodies, filters and delimiters produce recovered syntax and preserve
  following input. `__leave` placement and handler semantics are later analysis.
- `MsvcFrame` also owns `__asm`/`_asm` brace blocks and single-line instructions
  as opaque arena token lists with balanced parentheses, brackets and braces. A
  repeated asm keyword separates single-line instructions, and the enclosing C
  brace stays unconsumed. Line boundaries come from macro-invocation provenance
  and the original source text, ignoring phase-2 splices. Phase 7 converts every
  pp-number before parsing, so the assembly withdraws the C constant-conversion
  errors and numeric extension diagnostics of its own tokens (MASM `0FFh`).
  Assembly semicolons stay opaque tokens. EOF and mismatched delimiters mark
  the node recovered.
- Anonymous tagged struct/union definitions and references, and members naming
  only a typedef, are accepted only with `-fms-anonymous-structs`. ISO untagged
  anonymous members keep their independent C11 status; whether a typedef names
  an aggregate, and member promotion, are analysis.

Microsoft grammar references: [declspec](https://learn.microsoft.com/en-us/cpp/cpp/declspec),
[inline assembly](https://learn.microsoft.com/en-us/cpp/assembler/inline/asm),
[try-except](https://learn.microsoft.com/en-us/cpp/cpp/try-except-statement),
[try-finally](https://learn.microsoft.com/en-us/cpp/cpp/try-finally-statement), and
[anonymous class types](https://learn.microsoft.com/en-us/cpp/cpp/anonymous-class-types).

## Lexical and preprocessing behavior

Phase 1 trigraph replacement and phase 2 splicing use the same mode gate in the
main lexer, included files, terminal-splice diagnostics, and written header-name
reconstruction. Strict C89-C17 translate trigraphs; GNU modes and C23/C2y
preserve them. Strict C89 does not combine digraphs; C95 adds them. Strict
C89/C95 lex `//` as two `/` tokens. GNU89 accepts both features and reports
their origins under pedantic policy. Diagnostics are deferred with the batch
token entries, so skipped conditional groups do not report extension spellings.

Phase 3 recognizes `u`, `U`, and `u8` string prefixes and `u`/`U` character
prefixes from C11, and `u8` character prefixes from C23. Earlier modes keep the
identifier followed by a literal. C23 digit separators are part of pp-numbers
and must lie between digits of the constant's radix. C23 binary constants,
`wb`/`uwb` suffixes and C2y `0o`/`0O` octal prefixes convert to typed values.
Suffix recognition follows the mode gates for pasted tokens too. GNU binary
constants before C23 report the C23 origin. GNU imaginary `i`/`j` suffixes keep
the real component's precision and signedness, in either order with ordinary
floating suffixes; an integer takes no `f` suffix. Dollar identifiers, including
continuations, report GNU policy diagnostics, except inside a written
`#include <...>` name. C89 long-long suffixes and hexadecimal floating constants
report C99 policy diagnostics while keeping their values.

A C23 `'` continues a pp-number only before a digit or an ASCII nondigit, even
when conversion must reject the resulting constant; a universal character name,
another character or `$` after it starts a character constant instead. C23
permits universal names for basic and control characters inside literals while
keeping invalid-scalar checks; earlier modes keep the C99/C11 low-code-point
restriction.

Phase 5 preserves Unicode encoding in literal tokens and distinguishes numeric
escapes from source characters. UTF-8/UTF-16/UTF-32 character constants must fit
one code unit; numeric escapes have the selected code-unit range. C2y delimited
escapes are `\x{...}` and `\o{...}`, with empty, unterminated, invalid-radix,
and out-of-range forms diagnosed. Phase 6 concatenates ordinary strings with
encoded strings while keeping the encoding, and diagnoses incompatible
nonordinary prefixes. Phase 7 adds encoding-bearing literal, bit-precise
integer, and imaginary constant token variants.

C23 `#elifdef`/`#elifndef`, `#warning`, `#embed`, `__has_include`,
`__has_embed`, `__has_c_attribute`, and `__VA_OPT__` also work as earlier GNU
extensions. `#warning` always emits its message, plus a policy diagnostic when
its spelling is an extension. `#embed` reads real files as 8-bit resource
elements through the include search paths, and implements `limit`, `prefix`,
`suffix`, `if_empty`, and their double-underscore aliases. Limits use the
integer preprocessor evaluator in their own expansion frame and reject negative values and `defined`. Written quoted/angle names are read before
macro replacement, including digraph-looking punctuation and dollar signs as
header characters; macro-produced names and builtin strings such as `__FILE__`
expand normally. Missing operands and missing closing quotes are diagnosed.
Resource queries return 0 for missing resources, 1 for found resources, and
`__has_embed` returns 2 for an empty effective resource, including `limit(0)`.
The three `__STDC_EMBED_*__` constants are predefined when resource inclusion is
enabled. An unsupported qualified embed parameter returns 0 in `__has_embed`; a
direct `#embed` diagnoses it. Malformed parameters are diagnosed in both forms.
Query nesting is bounded at 64, so adversarial recursive operands produce a
diagnostic instead of exhausting the native stack. Resource and C attribute
queries are restricted to preprocessing conditional expressions (C23
§6.10.2p11); their names remain defined for `defined`, `#ifdef` and related
tests. Invalid operator openings preserve the following token or directive
boundary.

`__has_c_attribute` returns 202311 for the seven standard C23 attributes and
`_Noreturn`, including double-underscore forms, and 0 for other names. GNU
`__has_attribute` returns 1 for `unused`, `deprecated`, `aligned`, `packed`,
`noreturn`, `weak`, `section`, `visibility`, `format`, `always_inline`, and
`noinline`; `__has_builtin` returns 1 for `__builtin_va_arg`,
`__builtin_offsetof`, `__builtin_types_compatible_p`, and
`__builtin_choose_expr`. Other names return 0. These tables describe the
syntax-front-end subset, not backend effects or a GCC/Clang version. Both GNU
queries are available through their reserved spellings in strict modes and
report GNU policy diagnostics.

`__VA_OPT__` selects its inner replacement after variadic argument expansion;
inner substitution and pasting precede outer stringification or pasting. The
result is rescanned with the rest of the replacement but never substituted
again (C23 §6.10.5.1p7). Placemarkers survive until
the outer paste is complete. Definition-time checks reject use outside a
variadic replacement, nested optional replacements, missing parentheses, and
`##` at either inner boundary. C23 permits omitted variadic
arguments; pre-C23 pedantic modes keep the omission diagnostic. C89 fixed and
variadic empty arguments and variadic definitions report C99 origin. Named
`args...` definitions keep the original parameter name, report GNU origin, and
must be last. Their captures use the same machinery, but `__VA_ARGS__` does not
denote the named argument. Identical redefinitions compare original token
spellings and parameter names, even when the variadic parameter is unused.

GNU `, ## __VA_ARGS__` elides a comma for an omitted variadic argument, keeps
it for an explicitly empty argument, and substitutes supplied arguments without
pasting a comma. For a macro with only `...`, an empty invocation elides in GNU
modes and keeps the comma in strict modes, following the documented
[GCC distinction](https://gcc.gnu.org/onlinedocs/cpp/Variadic-Macros.html).
MSVC `-fms-va-args` independently elides a preceding comma when the variadic
argument expands to nothing, following the traditional
[MSVC behavior](https://learn.microsoft.com/en-us/cpp/preprocessor/variadic-macros?view=msvc-170).

GNU `#include_next` continues after the configured search entry that provided
the current header, even when the next directive changes quote/angle form. A
quoted include found next to its including file has no configured entry; its
`#include_next` starts at the first configured directory, following
[GCC's search-order description](https://gcc.gnu.org/onlinedocs/cpp/Wrapper-Headers.html).
`#ident`/`#sccs` require a string and are consumed as metadata directives; no
object-file metadata is emitted. `__COUNTER__` starts at 0 for each translation
unit and increments only when expanded. MSVC `-fms-pragma` consumes
`__pragma(...)` and passes its payload to the pragma handler with source
provenance, independent of `-fms-va-args` and the selected standard,
consistent with the
[MSVC operator documentation](https://learn.microsoft.com/en-us/cpp/preprocessor/pragma-directives-and-the-pragma-keyword?view=msvc-170).

## Decisions

- C2y `__STDC_VERSION__` is `202400L`, following the
  [Clang user manual](https://clang.llvm.org/docs/UsersManual.html#differences-between-various-standard-modes).
  GCC used `202500L` in its
  [initial C2y implementation](https://gcc.gnu.org/pipermail/gcc-patches/2024-June/654270.html).
  Draft versions are implementation choices, not conformance claims.
- C2y scope is `0o` octal prefixes and `\x{}`/`\o{}` delimited escapes in the
  lexer, and `_Countof`, case ranges, selection declarations, named loop
  control, and type-controlling `_Generic` in the parser. Further draft
  features need an explicit matrix row and a supporting primary source.
- Reserved standard keywords stay recognized in older modes and are diagnosed
  by origin. C23 non-reserved aliases stay identifiers before C23; GNU `typeof`
  and `asm`, and GNU89 `inline`, are exceptions. `restrict` stays an identifier
  before C99 unless written with a reserved GNU alternate spelling.
- `_Alignas`/`alignas`, `_Static_assert`/`static_assert`, `_Bool`/`bool`, and
  `_Thread_local`/`thread_local` share kinds. Aliases keep their original
  spelling for policy diagnostics, and token contents preserve it for parser
  consumers.
- GNU alternate keywords map to existing C99 kinds where they exist. `__asm__`
  and reserved `__asm` accept GNU assembly in every mode, and file-scope
  `__asm` is always GNU. With MS assembly enabled, a statement `__asm` followed
  by `(`, `volatile`, `inline` or `goto` selects GNU
  grammar and anything else selects MSVC block or line grammar; `_asm` stays
  gated by the MS assembly flag. GNU `__inline` remains available with MSVC
  inline disabled because its GNU spelling is independently reserved. With its
  flag enabled, the preprocessor consumes `__pragma` as an operator before
  keyword classification.
- GNU/MSVC policy diagnostics stay active in their enabled dialects under
  pedantic options. `_Imaginary` reports the baseline unsupported-type
  diagnostic; classification does not claim imaginary type semantics.
- Implicit int is a removed C89 feature, not a GNU extension: C99 and later
  report `'implicit int' is a C89 feature removed in C99`.
- Semantic calls to undeclared functions follow `ImplicitFunctionDeclaration`,
  another removed C89 feature. C89/C95 create an unprototyped int-returning
  declaration; GNU modes retain that behavior with the configured pedantic
  policy. Strict C99 and later report an undeclared identifier. Ordinary
  non-call identifier lookup never creates an implicit declaration.
- `__STRICT_ANSI__` is an implementation macro outside §6.10.8's protection, so
  `#undef` and `#define` apply to it as to any macro, as in GCC. The GNU builtins
  `__COUNTER__`, `__has_attribute`, and `__has_builtin` may be redefined or
  undefined with a warning; ISO predefined macros and C23 query names stay
  protected.
- Because `e sign` may follow `' nondigit` in the pp-number grammar,
  `0x1'e+1` is one invalid pp-number, as GCC lexes it.

## Testing evidence

- Parser: `parsing::tests::standards`, `gnu`, `msvc` and `dialect_regressions`
  cover each feature in every revision, enabled and disabled gates, all policy
  severities, immutable nodes, provenance, every input prefix, recovery with a
  following declaration, and deep nesting. `node_sizes` pins the new nodes.
- Lexer and preprocessor: `preprocessing::tests::language_modes` and the
  regression modules beside it; token snapshots under `tests/fixtures/lexing/`
  pin the mode-dependent token streams, including the ISO, GNU and MSVC parser
  seams.
- CLI: [`tests/cli.rs`](tests/cli.rs) covers flag parsing and overrides;
  [`tests/language_cli.rs`](tests/language_cli.rs) runs every accepted
  standard spelling through preprocessing, parsing and syntax inspection, with
  GNU and MSVC programs, disabled gates and malformed cross-phase input.
- Diagnostics: golden inputs under `tests/fixtures/diagnostics/` take optional
  `.args` sidecars that select mode and policy; see its `COVERAGE.md`.
- Allocation: [`tests/allocation_count.rs`](tests/allocation_count.rs) checks
  that ISO, GNU, MSVC and pedantic-suppression parsing, and diagnostic
  rendering, make no global allocations.

## Performance

Measured 8 October 2026 against `main` at `938c0c2`: Windows x86-64, Rust
1.99.0, native LLVM 23.1.1 with fat LTO, `benchmarking-internals` without
`portable-simd`, unchanged strict-C99 benchmark inputs, seven interleaved
Criterion `--quick` runs, minimum point estimates. Treat the small sample set
as indicative.

| Pipeline | Change vs `main` |
| --- | ---: |
| Lexer only | -2.5% to +2.4% |
| Phases 1-6 | +1.0% to +4.8% |
| Full pipeline, phases 1-7 | +4.0% to +9.2% |
| Parser only | +8.9% to +16.2% |

Parser driver steps per token are unchanged. Retained syntax grows the
translation-unit arena by 3-15 MiB on the three large workloads; preprocessing
and expansion arenas are unchanged. The main parser causes identified so far are
general `TokenType` equality used for scalar keyword and operator tests,
`ParseFrame` widening from 144 to 176 bytes, and `Declaration` growing from 56
to 72 bytes. Fixes are pending.
