# Language standards

The foundation provides configuration, CLI selection, predefined version macros,
keyword classification, and shared extension diagnostics. The ISO parser now
implements phase-7 syntax through C23 and the C2y subset listed below, including
policy diagnostics for earlier modes. GNU phase-7 syntax is also implemented.
Lexical/preprocessing additions and MSVC grammar remain assigned to their
workstreams. This is a syntax-only front end;
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
second spelling classifier. ISO parser consumers use the canonical kinds and
retain original tokens for spelling and provenance. Unsupported vendor kinds
continue through diagnostic and synchronization paths until their owners land.

## Feature matrix

ISO-native columns below give revision availability: `Y` means native and
`-` means not native. GNU counterparts use the same ISO revision and additionally
mark GNU-origin rows as dialect-native; MSVC-origin rows become dialect-native
when their flag is enabled. The acceptance column describes extensions/gates separately:
`extension` accepts reserved or unambiguous syntax in older modes, `native` gates
the spelling, `GNU earlier` also accepts it in older GNU modes, and `MS flag`
requires its independent flag. `Trigraphs` is a special gate: only strict C89–C17.
Rows describe intended availability; pending behavior must not be inferred from
a `Y`. Existing C99 behavior is marked implemented with pending mode work where
necessary. Workstream owners update their own rows as behavior lands.

The foundation's mode/flags, version macros, complete keyword spelling recognition,
and shared policy diagnostic plumbing are **implemented** for all modes. All
ISO parser additions are implemented; lexical, preprocessing, and vendor parser
behavior below remains delegated.

| Feature (`Feature` variant) | C89 | C95 | C99 | C11 | C17 | C23 | C2y | Acceptance | Status / owner |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Digraphs | - | Y | Y | Y | Y | Y | Y | C95+ / GNU earlier | implemented C99 behavior; pending mode checks/diagnostics (lexpp) |
| LineComments | - | - | Y | Y | Y | Y | Y | GNU earlier | implemented C99 behavior; pending mode checks/diagnostics (lexpp) |
| Trigraphs | Y | Y | Y | Y | Y | - | - | strict through C17 | implemented C99 behavior; pending mode checks/diagnostics (lexpp) |
| UnicodeLiteralPrefixes | - | - | - | Y | Y | Y | Y | native | pending (lexpp) |
| Utf8CharacterConstants | - | - | - | - | - | Y | Y | native | pending (lexpp) |
| DigitSeparators | - | - | - | - | - | Y | Y | native | pending (lexpp) |
| BitIntSuffixes | - | - | - | - | - | Y | Y | native | pending (lexpp) |
| BinaryConstants | - | - | - | - | - | Y | Y | GNU earlier | implemented C99 behavior; pending mode checks/diagnostics (lexpp) |
| Elifdef | - | - | - | - | - | Y | Y | GNU earlier | pending (lexpp) |
| WarningDirective | - | - | - | - | - | Y | Y | GNU earlier | pending (lexpp) |
| Embed | - | - | - | - | - | Y | Y | GNU earlier | pending (lexpp) |
| HasInclude | - | - | - | - | - | Y | Y | GNU earlier | pending (lexpp) |
| HasEmbed | - | - | - | - | - | Y | Y | GNU earlier | pending (lexpp) |
| HasCAttribute | - | - | - | - | - | Y | Y | GNU earlier | pending (lexpp) |
| VaOpt | - | - | - | - | - | Y | Y | GNU earlier | pending (lexpp) |
| OctalPrefix | - | - | - | - | - | - | Y | native | pending (lexpp) |
| DelimitedEscapes | - | - | - | - | - | - | Y | native | pending (lexpp) |
| HexFloats | - | - | Y | Y | Y | Y | Y | extension | implemented C99 behavior; pending mode checks/diagnostics (lexpp) |
| VariadicMacros | - | - | Y | Y | Y | Y | Y | extension | implemented C99 behavior; pending mode checks/diagnostics (lexpp) |
| EmptyMacroArguments | - | - | Y | Y | Y | Y | Y | extension | implemented C99 behavior; pending mode checks/diagnostics (lexpp) |
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
| ImaginaryConstants | - | - | - | - | - | - | - | extension (GNU native) | pending (lexpp) |
| DollarIdentifiers | - | - | - | - | - | - | - | extension (GNU native) | pending (lexpp) |
| NamedVariadicMacros | - | - | - | - | - | - | - | extension (GNU native) | pending (lexpp) |
| GnuVaArgs | - | - | - | - | - | - | - | extension (GNU native) | pending (lexpp) |
| IncludeNext | - | - | - | - | - | - | - | extension (GNU native) | pending (lexpp) |
| IdentDirective | - | - | - | - | - | - | - | extension (GNU native) | pending (lexpp) |
| Counter | - | - | - | - | - | - | - | extension (GNU native) | pending (lexpp) |
| HasAttribute | - | - | - | - | - | - | - | extension (GNU native) | pending (lexpp) |
| HasBuiltin | - | - | - | - | - | - | - | extension (GNU native) | pending (lexpp) |
| MsDeclspec | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsIntTypes | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsCallingConventions | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsTypeQualifiers | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsInline | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsSeh | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsAsm | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsPragma | - | - | - | - | - | - | - | MS flag | pending (lexpp) |
| MsAnonymousStructs | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsVaArgs | - | - | - | - | - | - | - | MS flag | pending (lexpp) |

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
  spelling is independently reserved. `__pragma` is recognized as a keyword
  until lexpp implements consumption as an operator.
- GNU/MSVC policy diagnostics remain active in their enabled dialects under
  pedantic options. `_Imaginary` still reports the baseline unsupported-type
  diagnostic; classification does not claim imaginary type semantics.
- Phase A mechanically removes the old C99-only exhaustive matches around
  existing preprocessor extension policy. Their behavior is otherwise unchanged.
- Phase A introduced no lexer mode behavior, parser production, AST node or
  semantic rule. The ISO parser additions above build on its prepared seams. Avoid editing configuration, CLI, token classification, and
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
