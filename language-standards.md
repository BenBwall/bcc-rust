# Language standards and implementation status

Phase A provides configuration, CLI selection, predefined version macros,
keyword classification, and shared extension diagnostics. This is a syntax-only
front end: selecting a mode does **not** yet implement that mode's full grammar
or lexical rules by itself. The lexpp workstream implements the lexical and
preprocessing rows below; parser rows retain their separate owners. The completed
C99 parser remains the baseline. No semantic analysis or code generation is added.

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
second spelling classifier. Unsupported kinds currently enter existing diagnostic
and synchronization paths; Phase A does not add grammar or AST nodes.

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
and shared policy diagnostic plumbing are **implemented** for all modes. Lexical
and preprocessing behavior is implemented by lexpp; parser behavior below is
tracked independently.

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
| Inline | - | - | Y | Y | Y | Y | Y | GNU earlier | implemented C99 behavior; pending mode checks/diagnostics (parse-std) |
| Restrict | - | - | Y | Y | Y | Y | Y | native | implemented C99 behavior; pending mode checks/diagnostics (parse-std) |
| Bool | - | - | Y | Y | Y | Y | Y | extension | implemented C99 behavior; pending mode checks/diagnostics (parse-std) |
| Complex | - | - | Y | Y | Y | Y | Y | extension | implemented C99 behavior; pending mode checks/diagnostics (parse-std) |
| Imaginary | - | - | Y | Y | Y | Y | Y | extension | implemented unsupported-type diagnostic; pending (parse-std) |
| ImplicitInt | Y | Y | - | - | - | - | - | extension | pending (parse-std) |
| MixedDeclarations | - | - | Y | Y | Y | Y | Y | extension | implemented C99 behavior; pending mode checks/diagnostics (parse-std) |
| ForDeclarations | - | - | Y | Y | Y | Y | Y | extension | implemented C99 behavior; pending mode checks/diagnostics (parse-std) |
| DesignatedInitializers | - | - | Y | Y | Y | Y | Y | extension | implemented C99 behavior; pending mode checks/diagnostics (parse-std) |
| CompoundLiterals | - | - | Y | Y | Y | Y | Y | extension | implemented C99 behavior; pending mode checks/diagnostics (parse-std) |
| FlexibleArrayMembers | - | - | Y | Y | Y | Y | Y | extension | implemented C99 behavior; pending mode checks/diagnostics (parse-std) |
| LongLong | - | - | Y | Y | Y | Y | Y | extension | literal suffix diagnostics implemented (lexpp); type-specifier mode work pending (parse-std) |
| TrailingEnumComma | - | - | Y | Y | Y | Y | Y | extension | implemented C99 behavior; pending mode checks/diagnostics (parse-std) |
| Func | - | - | Y | Y | Y | Y | Y | extension | implemented C99 behavior; pending mode checks/diagnostics (parse-std) |
| StaticAssert | - | - | - | Y | Y | Y | Y | extension | pending (parse-std) |
| Generic | - | - | - | Y | Y | Y | Y | extension | pending (parse-std) |
| Alignas | - | - | - | Y | Y | Y | Y | extension | pending (parse-std) |
| Alignof | - | - | - | Y | Y | Y | Y | extension | pending (parse-std) |
| Noreturn | - | - | - | Y | Y | Y | Y | extension | pending (parse-std) |
| ThreadLocal | - | - | - | Y | Y | Y | Y | extension | pending (parse-std) |
| Atomic | - | - | - | Y | Y | Y | Y | extension | pending (parse-std) |
| AnonymousAggregates | - | - | - | Y | Y | Y | Y | extension | pending (parse-std) |
| C23Keywords | - | - | - | - | - | Y | Y | native | pending (parse-std) |
| Attributes | - | - | - | - | - | Y | Y | extension | pending (parse-std) |
| BitInt | - | - | - | - | - | Y | Y | extension | pending (parse-std) |
| DecimalTypes | - | - | - | - | - | Y | Y | extension | pending (parse-std) |
| EmptyInitializers | - | - | - | - | - | Y | Y | extension | pending (parse-std) |
| EnumUnderlyingType | - | - | - | - | - | Y | Y | extension | pending (parse-std) |
| AutoTypeInference | - | - | - | - | - | Y | Y | extension | pending (parse-std) |
| Constexpr | - | - | - | - | - | Y | Y | extension | pending (parse-std) |
| Nullptr | - | - | - | - | - | Y | Y | extension | pending (parse-std) |
| Countof | - | - | - | - | - | - | Y | extension | pending (parse-std) |
| IfSwitchDeclarations | - | - | - | - | - | - | Y | extension | pending (parse-std) |
| NamedLoops | - | - | - | - | - | - | Y | extension | pending (parse-std) |
| GenericTypeOperand | - | - | - | - | - | - | Y | extension | pending (parse-std) |
| CaseRanges | - | - | - | - | - | - | Y | extension | pending (parse-gnu) |
| GnuAttribute | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| GnuAsm | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| GnuTypeof | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| ExtensionMarker | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| StatementExpressions | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| BuiltinVaArg | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| BuiltinOffsetof | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| BuiltinTypesCompatible | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| BuiltinChooseExpr | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| LabelsAsValues | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| LocalLabels | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| OmittedConditionalOperand | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| ZeroLengthArrays | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| Int128 | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| AutoType | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| GnuAlternateKeywords | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| RealImag | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| GnuDesignators | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| UnionCasts | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| EmptyStructs | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| NestedFunctions | - | - | - | - | - | - | - | extension (GNU native) | pending (parse-gnu) |
| ImaginaryConstants | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| DollarIdentifiers | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| NamedVariadicMacros | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| GnuVaArgs | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| IncludeNext | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| IdentDirective | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| Counter | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| HasAttribute | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| HasBuiltin | - | - | - | - | - | - | - | extension (GNU native) | implemented (lexpp) |
| MsDeclspec | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsIntTypes | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsCallingConventions | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsTypeQualifiers | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsInline | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsSeh | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsAsm | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsPragma | - | - | - | - | - | - | - | MS flag | implemented (lexpp) |
| MsAnonymousStructs | - | - | - | - | - | - | - | MS flag | pending (parse-msvc) |
| MsVaArgs | - | - | - | - | - | - | - | MS flag | implemented (lexpp) |

The lexical workstream also owns exact prefix/suffix recognition, `//` treatment
in GNU89 pedantic mode, `#elifndef`, `#sccs`, `, ## __VA_ARGS__`, and predicates
for builtins represented by their grouped feature rows. Parser rows encompass
all positions of GNU attributes; basic/extended/goto asm and declarator asm labels;
GNU range/old-style designators; syntax-only nested functions; and the C2y subset.

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
boundaries; macro-produced names are expanded normally. Resource queries
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
