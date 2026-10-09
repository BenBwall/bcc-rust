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
`__STDC_HOSTED__`, `__DATE__`/`__TIME__`, and `__STDC_MB_MIGHT_NEQ_WC__` are
predefined in every mode. `__STDC_HOSTED__` is `1` in the default hosted
execution environment and `0` with `-ffreestanding`; `-fhosted` restores
hosted and the later flag wins. `CompilerConfiguration::default()` is hosted
as well.

Compiler identity follows Clang's approach. Every language mode, including
strict ISO modes, defines `__GNUC__` `4`, `__GNUC_MINOR__` `2` and
`__GNUC_PATCHLEVEL__` `1`, the GCC version Clang claims, and the
inline-semantics macro Clang defines: `__GNUC_GNU_INLINE__` before C99 and
`__GNUC_STDC_INLINE__` from C99 on. Only `__STRICT_ANSI__` distinguishes
strict modes from GNU modes in these macros. `-fms-extensions` defines
`_MSC_VER` `1933`, `_MSC_FULL_VER` `193300000`, `_MSC_BUILD` `1` and
`_MSC_EXTENSIONS` `1`: MSVC 19.33, Clang's default
`-fms-compatibility-version`. Only the umbrella flag claims MSVC; the
individual `-fms-NAME` groups do not, and a later `-fno-ms-extensions`
withdraws it. Both GNU and MSVC identity can therefore be claimed at once,
whereas Clang drops the GNU identity under `-fms-compatibility`. Every mode
defines `__bcc__` `1`, `__bcc_major__`, `__bcc_minor__`, `__bcc_patchlevel__`
and the string `__bcc_version__` from the package version. `__clang__`,
`__llvm__`, `__VERSION__` and `__GXX_ABI_VERSION` are never defined. Like
Clang's, these are ordinary macros that `#undef` may remove. OS identity
macros such as `__linux__` and `_WIN32` follow the selected target, below. The identity and target definitions are read from the synthetic
`<built-in>/predefined.h` before user input.

Reserved target-description macros follow the pinned LLVM 23.1.1 Clang for the
selected `--target` triple. The default remains `x86_64-unknown-linux-gnu`;
`x86_64-unknown-linux-musl`, `x86_64-w64-windows-gnu` (aliases
`x86_64-w64-mingw32` and `x86_64-pc-windows-gnu`) and
`x86_64-pc-windows-msvc` are supported. Unknown triples are CLI errors.
`CompilerConfiguration::with_target` selects the same contract for the library
pipeline. Target selection does not implicitly enable GNU or MSVC syntax groups.

For MinGW only, `__cdecl`, `__fastcall`, `__pascal`, `__stdcall`, `__thiscall`
and their single-underscore aliases expand to Clang's GNU calling-convention
attributes; `__declspec(a)` expands to `__attribute__((a))`. Both ISO and GNU
target tables retain these eleven definitions exactly. Linux does not define
them, and the MSVC keyword contract is unchanged. Function declarators,
function pointers, parameters, members and typedefs use the existing balanced
attribute parser. `dllimport`, `dllexport`, `noreturn`, `nothrow`, `selectany`,
`restrict`, `deprecated(...)`, `noinline`, `naked`, `allocate(...)`, `uuid(...)`
and ABI annotations such as `__ms_abi__` are accepted without backend effects.
`align(...)` and `aligned(...)` continue to mark the affected type/layout as
unanalyzed rather than assume its natural alignment. Clang itself ignores
some of these macro-expanded attributes with warnings; bcc's current attribute
pass preserves their syntax without applicability warnings.

Startup preprocessing uses `PreprocessingOption::{Define,Undefine,Include}`
in `src/configuration.rs`, borrowed by `Context.preprocessing_options`; the
scalar `CompilerConfiguration` remains lifetime-free and `Copy`. CLI spellings
are documented in README. The ordinary directive parser reads an arena-backed
`<command line>` source after predefined macros and before the main input,
reusing structured diagnostics, macro validation and builtin protections.
Definition values split at the first `=`. Both `-D` and `-U` truncate at the
first CR or LF; definition truncation follows GCC. Command-line definitions
enter the ordinary lexer at phase 3;
backslashes cannot splice the next option and trigraph spellings stay literal.
The measured library CLI adapter applies these options before its intervals,
so golden diagnostics exercise the same arena-backed path. All `-D`/`-U`
operations precede all forced includes, even if an
include appeared earlier on argv, matching GCC/Clang startup ordering. Forced
includes use working-directory quoted lookup; the synthetic frame does not
consume the include-nesting limit. `-imacros` is deferred because discarding
its expanded output requires a separate, explicit preprocessor output scope.

`src/target.rs` is the single target seam: scalar/pointer layout, ABI aliases,
wide encoding, va-list representation, record rules and, for MSVC, the
Microsoft ABI's external emission of inline functions. Its `target/` module
contains frozen, ordinary macro definitions. GNU modes receive Clang's unreserved
OS spellings (`linux`, `unix`, `WIN32`, etc.) where Clang emits them; ISO modes
receive only the corresponding reserved forms. GNU Linux and musl share the
data model and the retained target macros; libc-specific distinctions come
from the selected headers, without inventing a musl identity macro. Windows triples supply Clang's MinGW or MSVC OS/ABI
names. This replaces the earlier policy of leaving OS identity undefined.
C99 §7.1.3p1 (printed p. 166; PDF p. 178) reserves implementation names without
changing language grammar. These definitions can be undefined or redefined;
required ISO builtins retain their existing protection.

Run `python scripts/update_target_macros.py --check` to compare all frozen
spellings against the repository's Clang (`-dM -E -x c`, ISO C11 and GNU C17).
The MSVC triple also passes `-fms-compatibility-version=19.33`, the version
bcc's `_MSC_VER` claims: Clang otherwise takes the host's installed Visual C++
version, and macros such as `__STDC_NO_THREADS__` would differ between hosts.
Omit `--check` only when deliberately refreshing the oracle. The canonical Rust
tests independently invoke Clang and compare sets of exact definitions, then
check that every exclusion is documented below. Existing language-mode
builtins are checked separately and are excluded from the target set; this
work does not change their registration or compiler-identity/hosted ownership.
The original 163-definition Linux fixture is retained and every one of its
spellings must still be present. Unsupported semantic/backend capabilities
must not be advertised merely because Clang advertises them.

### Embedded freestanding headers

All eleven public headers are discoverable in all language modes, and each behaves like
Clang's resource header of the same name, whose logic, not text, they follow.
In a hosted translation a header that Clang chains to the C library tests
`__has_include_next` and reads the library's header with `#include_next`
before supplying what remains; `-ffreestanding` skips the C library entirely.
The headers are searched after the `-I`,
`-isystem` and environment directories and before the C library's directories
(`--sysroot`) and `-idirafter`; quoted lookup additionally gives local and
`-iquote` headers priority. `-nostdinc` and `-nobuiltininc` remove the resource
directory. `__has_include` uses the same search.
This follows Clang resource-header availability; use of newer language syntax
still follows the configured extension policy.

| Header | Contents and mode policy |
| --- | --- |
| `float.h` | C99 floating limits; C11 adds true minima, subnormal and decimal-digit macros. `FLT_ROUNDS` is the default round-to-nearest value `1`; `FLT_EVAL_METHOD` is `0`. Runtime changes to the rounding environment require backend support. Hosted with `__MINGW32__` or `_MSC_VER` defined, it first reads the C library's `<float.h>` and then replaces its characteristics, as Clang does; it defines GCC's guard `_FLOAT_H___` first, so MinGW-w64's header does not look for GCC's. |
| `iso646.h` | The eleven C alternative operator macros. |
| `limits.h` | Target-derived signed/unsigned limits; long-long limits from C99 onward. Hosted, it first reads the C library's `<limits.h>` for its POSIX and other additions, defining `_GCC_LIMITS_H_` in GNU modes so glibc does not look for GCC's header, then replaces the integer limits with the target's, as Clang does. `MB_LEN_MAX` is defined only if the C library did not (glibc 16, musl 4, MSVC 5); the fallback is `4`, since bcc's literals are UTF-8 and 4 bytes cover every stateless encoding. Clang's fallback is `1`. |
| `stdarg.h` | `va_list` and the four `va_*` macros in every mode, plus GNU `__gnuc_va_list` and `__va_copy`. Like Clang's, it may be included repeatedly; `__need___va_list`, `__need_va_list`, `__need_va_arg`, `__need___va_copy` and `__need_va_copy` request one part, and each part keeps its conventional guard (`__GNUC_VA_LIST`, `_VA_LIST`). The intrinsic type is an array of one opaque 24-byte, 8-aligned SysV record on Linux, and `char *` on Windows. |
| `stdbool.h` | `__bool_true_false_are_defined`; `bool`, `true`, `false` macros before C23. C23 uses language keywords. |
| `stddef.h` | `size_t`, `ptrdiff_t`, `wchar_t`, `NULL`, `offsetof`; C11 adds `max_align_t`, and `__STDC_WANT_LIB_EXT1__` adds `rsize_t`. Like Clang's, it may be included repeatedly: `__need_size_t`, `__need_ptrdiff_t`, `__need_wchar_t`, `__need_NULL`, `__need_wint_t`, `__need_rsize_t`, `__need_max_align_t` and `__need_offsetof` request one part, as glibc's headers do. Each type keeps its conventional guard (`_SIZE_T`, `_PTRDIFF_T`, `_WCHAR_T`, `_WINT_T`, `_RSIZE_T`), so a definition the C library made first is kept; a requested `NULL` is always restored to `((void *)0)`. As in Clang, `_MSC_EXTENSIONS` also defines vcruntime.h's `_WCHAR_T_DEFINED` guard with `wchar_t`, and `max_align_t` is `double` on the MSVC target (`_M_X64` without `__MINGW32__`), otherwise the GCC-style record, independently of MSVC extension flags. |
| `stdint.h` | All 8/16/32/64 exact, least and fast types, pointer and maximum types, corresponding limits and constant macros; `SIG_ATOMIC`, `SIZE`, `PTRDIFF`, `WCHAR`, `WINT` limits. Hosted, a C library `<stdint.h>` replaces all of these, as with Clang. |
| `stdalign.h` | Like Clang: `alignas`, `alignof`, and the two indicator macros when `__STDC_VERSION__` exists and precedes C23; empty in C89 and C23/C2y. |
| `stdatomic.h` | C11 atomic typedefs, `memory_order`, `atomic_flag`, initialization, fences, lock-free queries and all generic operation macros. Hosted, it defers to a following system header except with `_MSC_VER` in C mode, matching the pinned Clang resource header. C17 marks `ATOMIC_VAR_INIT` deprecated through `#pragma clang deprecated`; `_CLANG_DISABLE_CRT_DEPRECATION_WARNINGS` suppresses that marker. C23 removes `ATOMIC_VAR_INIT` and adds `atomic_char8_t` and `ATOMIC_CHAR8_T_LOCK_FREE`. No `__STDC_NO_ATOMICS__` is defined or interpreted, as in Clang. |
| `stdnoreturn.h` | Like Clang: `noreturn` and `__noreturn_is_defined` in every mode, retained in C23 despite deprecation. |
| `mm_malloc.h` | Not ISO C; GCC and Clang ship it, and MinGW-w64's `<malloc.h>` includes it. Like Clang's, it includes `<stdlib.h>` and defines `static __inline__` `_mm_malloc` and `_mm_free`: through `__mingw_aligned_malloc` for MinGW-w64, `_aligned_malloc` for the MSVC runtime (from its `<malloc.h>`, unless that defines `_mm_malloc` as a macro), and `posix_memalign`, which it declares, elsewhere. An alignment of 1 uses `malloc`, and a smaller power of two is raised to a pointer's alignment. The x86 intrinsic headers (`x86intrin.h`, `emmintrin.h`, `immintrin.h`, `cpuid.h` and their family) need vector types and target builtins and are not provided. |

Reserved `__builtin_va_arg`, `__builtin_va_start`, `__builtin_va_end`,
`__builtin_va_copy`, `__builtin_va_list` and `__builtin_offsetof` support the
standard headers even with `-pedantic-errors`. The parser uses `GnuFrame` for
intrinsic expression operands; semantic analysis checks va-list operands,
va-arg result types and variadic-function context. Runtime pairing, argument
availability and promoted argument/type agreement remain backend/runtime work.
An `offsetof` member/index path yields a `size_t` integer constant; bit-fields
and invalid paths diagnose.

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
| Generic | - | - | - | Y | Y | Y | Y | extension | parser and sema |
| Alignas | - | - | - | Y | Y | Y | Y | extension | parser |
| Alignof | - | - | - | Y | Y | Y | Y | extension | parser |
| Noreturn | - | - | - | Y | Y | Y | Y | extension | parser |
| ThreadLocal | - | - | - | Y | Y | Y | Y | extension | parser |
| Atomic | - | - | - | Y | Y | Y | Y | extension | parser and sema |
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
| WideEnumerators | - | - | - | - | - | Y | Y | extension | semantic analysis |
| Countof | - | - | - | - | - | - | Y | extension | parser |
| IfSwitchDeclarations | - | - | - | - | - | - | Y | extension | parser |
| NamedLoops | - | - | - | - | - | - | Y | extension | parser |
| GenericTypeOperand | - | - | - | - | - | - | Y | extension | parser |
| CaseRanges | - | - | - | - | - | - | Y | extension | parser |
| GnuAttribute | - | - | - | - | - | - | - | extension (GNU native) | parser; vector_size/aligned sema |
| GnuAsm | - | - | - | - | - | - | - | extension (GNU native) | parser |
| GnuTypeof | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| ExtensionMarker | - | - | - | - | - | - | - | extension (GNU native) | parser |
| StatementExpressions | - | - | - | - | - | - | - | extension (GNU native) | parser |
| BuiltinVaArg | - | - | - | - | - | - | - | reserved intrinsic in all modes | parser/sema |
| BuiltinOffsetof | - | - | - | - | - | - | - | reserved intrinsic in all modes | parser/sema |
| BuiltinTypesCompatible | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| VectorBuiltins | - | - | - | - | - | - | - | extension (GNU native) | shuffle/convert/bitcast and x86 signatures |
| BuiltinChooseExpr | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| LabelsAsValues | - | - | - | - | - | - | - | extension (GNU native) | parser |
| LocalLabels | - | - | - | - | - | - | - | extension (GNU native) | parser |
| OmittedConditionalOperand | - | - | - | - | - | - | - | extension (GNU native) | parser |
| ZeroLengthArrays | - | - | - | - | - | - | - | extension (GNU native) | parser |
| Float128 | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| Int128 | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| AutoType | - | - | - | - | - | - | - | extension (GNU native) | parser |
| GnuAlternateKeywords | - | - | - | - | - | - | - | extension (GNU native) | parser |
| RealImag | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| GnuDesignators | - | - | - | - | - | - | - | extension (GNU native) | parser |
| UnionCasts | - | - | - | - | - | - | - | extension (GNU native) | parser |
| EmptyStructs | - | - | - | - | - | - | - | extension (GNU native) | parser |
| ExtraSemicolons | - | - | - | - | - | - | - | extension (GNU native) | parser |
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
| SignBitShifts | - | - | - | - | - | - | - | extension (GNU native) | semantic analysis |
| FlexibleArrayExtensions | - | - | - | - | - | - | - | extension (GNU native) | semantic analysis |
| ConstantFolding | - | - | - | - | - | - | - | extension (GNU native) | semantic analysis |
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
query predicates for builtins grouped under one row; `IncludeNext` also covers
`__has_include_next`. GNU parser rows cover every
GNU attribute position, basic, extended and goto assembly with declarator
assembly labels, GNU range and old-style designators, and syntax-only nested
functions.

## ISO syntax

| Revision | Phase-7 syntax | Left to later analysis |
| --- | --- | --- |
| C89/C90/C95 | Native implicit int; C99-origin mixed blocks, for declarations, designated initializers, compound literals, flexible array members, long long, trailing enum comma, `__func__`, and unambiguous qualified/static/`[*]` array syntax follow Allow/Warn/Deny | Variable bounds need constant evaluation to distinguish VLAs from constant arrays; flexible-member position and object layout |
| C99 | Full C99 grammar; implicit int is retained with a removed-feature policy diagnostic | Type/name/control-flow constraints listed in the C99 checklist |
| C11/C17 | `_Alignas` expression/type operands, `_Alignof` types (parenthesized expressions as a GNU extension), `_Atomic` type/qualifier, `_Generic` associations, `_Noreturn`, `_Static_assert`, `_Thread_local`, anonymous untagged aggregates; `_Alignas` in a plain type name is a constraint error | Alignment validity, storage-class combinations and aggregate layout; atomic eligibility and generic selection are analyzed |
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

`__int128` and `unsigned __int128` now have real semantic integer types on
all four x86-64 targets: 16-byte size/alignment and rank above `long long`.
Reserved builtin typedef names `__int128_t` and `__uint128_t` are present
without a header and are not macros. `__SIZEOF_INT128__` is 16 in every
frozen target snapshot, regenerated by `scripts/update_target_macros.py`.
The existing GNU extension policy diagnoses the keyword under `-pedantic`;
`__extension__` suppresses it. Typedef spellings do not add keyword diagnostics.
Literals still select standard types through `long long`; there is no 128-bit
suffix, matching [GCC's integer extension](https://gcc.gnu.org/onlinedocs/gcc/_005f_005fint128.html).
Preprocessor arithmetic remains the target's 64-bit intmax/uintmax model
(C99 §6.10.1p4, printed p. 148; PDF p. 160).

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
An extra `;` between external declarations or between members, such as the
one an empty macro like the Windows SDK's `DEFINE_ENUM_FLAG_OPERATORS(T);`
leaves behind in C, declares nothing; GCC and Clang accept it in every mode and
report it only when pedantic. A translation unit holding only such a `;` is not
empty, as in Clang.

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
integer preprocessor evaluator in their own expansion frame, allow conditional
queries, and reject negative values and `defined`. Written quoted/angle names
are read before macro replacement, including digraph-looking punctuation and
dollar signs as header characters; macro-produced names and builtin strings such as `__FILE__`
expand normally. Missing operands and missing closing quotes are diagnosed.
Resource queries return 0 for missing resources, 1 for found resources, and
`__has_embed` returns 2 for an empty effective resource, including `limit(0)`.
The three `__STDC_EMBED_*__` constants are predefined when resource inclusion is
enabled. An unsupported qualified embed parameter returns 0 in `__has_embed`; a
direct `#embed` diagnoses it. Malformed parameters, including mismatched
parentheses, brackets and braces, are diagnosed in both forms.
Query nesting is bounded at 64, so adversarial recursive operands produce a
diagnostic instead of exhausting the native stack. Resource and C attribute
queries are restricted to preprocessing conditional expressions and embed
limits (C23 §6.10.2p11, §6.10.4.2p3); their names remain defined for `defined`,
`#ifdef` and related tests. Invalid operator openings preserve the following
token or directive boundary.

`__has_c_attribute` returns 202311 for the seven standard C23 attributes and
`_Noreturn`, including double-underscore forms, and 0 for other names. GNU
`__has_attribute` returns 1 for `unused`, `deprecated`, `aligned`, `packed`,
`noreturn`, `weak`, `section`, `visibility`, `format`, `always_inline`, and
`noinline`; `__has_builtin` returns 1 for `__builtin_va_arg`,
`__builtin_va_start`, `__builtin_va_end`, `__builtin_va_copy`,
`__builtin_offsetof`, `__builtin_types_compatible_p`, and
`__builtin_choose_expr`, `__builtin_classify_type`, and the modeled `__c11_atomic_*`, `__atomic_*` and `__sync_*` names. Other names return 0. These tables describe the
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
the current header, even when the next directive changes quote/angle form, as
[GCC's search-order description](https://gcc.gnu.org/onlinedocs/cpp/Wrapper-Headers.html)
explains. Each opening of a header carries its own entry, so a header reached
through two entries continues after each. The rest follows Clang. A header
found beside its includer takes the includer's entry. The primary source file
has none, nor do a header found by an absolute path and a header found beside
either: there the directive warns (Clang's `-Winclude-next-outside-header` and
`-Winclude-next-absolute-path`) and searches exactly as `#include` would. GCC
instead starts a relative header's search at the first `-iquote` directory
without a warning. Both warnings are ordinary warnings, so they are withheld in
system headers. GNU `__has_include_next(header)` reports whether that
`#include_next` would find the header, with the same start and warnings. Like
`__has_include` it is restricted to conditional expressions, and it reports
the `IncludeNext` GNU origin under pedantic policy; in a system header, such
as the resource headers, that diagnostic is withheld (see
[System headers](#system-headers)).
`#ident`/`#sccs` require a string and are consumed as metadata directives; no
object-file metadata is emitted. `__COUNTER__` starts at 0 for each translation
unit and increments only when expanded. MSVC `-fms-pragma` consumes
`__pragma(...)` and passes its payload to the pragma handler with source
provenance, independent of `-fms-va-args` and the selected standard,
consistent with the
[MSVC operator documentation](https://learn.microsoft.com/en-us/cpp/preprocessor/pragma-directives-and-the-pragma-keyword?view=msvc-170).

## System headers

A header is a **system header** when it was found through a system directory,
following GCC's classification ("System Headers" in the GCC preprocessor
manual): an `-isystem` directory, `C_INCLUDE_PATH`, the resource directory, the
C library's `--sysroot` directories, or an `-idirafter` directory. A header
found beside a system header (a `"…"` include from it) is one too. A header
found through `-iquote`, `-I`, `CPATH` or beside a user file, one named by an
absolute path, and the primary source file are not. `#pragma GCC
system_header` (also through `_Pragma`) makes the rest of the current header
a system header; in the primary source file it is ignored with a warning, as
in GCC and Clang. A `#line` directive gives the following lines a new file
identity that is not a system header.

Inside a system header, warnings and extension diagnostics are withheld, the
latter at every policy level: under `-pedantic-errors` an extension in a system
header is not reported, matching Clang, where such diagnostics are warnings
promoted to errors and are not emitted from system headers. Policy-governed
preprocessor diagnostics (macro redefinitions, `__VA_ARGS__` and `__VA_OPT__`
outside a variadic macro, empty variadic arguments, the `#if` comma operator,
`defined` produced by a function-like macro, a backslash in a quoted header
name) count as extensions. Errors, such as syntax errors, constraint violations
that are errors by default, and `#error`, are always reported.

The diagnostic's location decides: its first source vector, which for a token
produced by macro expansion is where the token is spelled in the macro's
replacement list. This matches Clang's rule for its spelling location. A
diagnostic in user code about something a system header declared or defined,
such as a redefinition of the header's macro, is reported, while a construct
that a system header's macro spells, like a `long long` it supplies to C89
code, is not. Suppression happens when the diagnostic is reported, so withheld
diagnostics are not counted.

## Decisions

- Macro redefinitions that change the form, parameters or replacement list are
  warnings, and errors under `-pedantic-errors`, as in GCC and Clang
  (`-Wmacro-redefined`); C99 §6.10.3p2 requires only a diagnostic. The new
  definition replaces the old one. A change of form is reported once.
- A `defined` operator that macro replacement produces in `#if` is undefined
  behavior (C99 §6.10.1p4). As in GCC and Clang, it is evaluated once the
  replacement list's parameters are replaced and its `##` operators applied,
  and its operand is not macro-replaced, so MinGW-w64's
  `defined(__INTRINSIC_DEFINED_ ## name)` works. It is diagnosed as Clang's
  `-Wexpansion-to-defined` is: from an object-like macro or a macro argument it
  is a warning under every policy; from a function-like replacement list,
  including a `defined` formed by `##`, it is a policy-governed extension. The
  diagnostic is placed at the outermost invocation, so it is withheld inside a
  system header but reported where a user file expands the system macro, as in
  GCC and Clang.
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
- Hosted translation: [`tests/hosted_cli.rs`](tests/hosted_cli.rs) covers the
  execution environment, the header search order and its options, resource
  headers chained to the fake glibc-like library under
  `tests/fixtures/hosted/`, `mm_malloc.h` against POSIX-, MinGW-w64- and
  MSVC-like runtimes, compiler identity per mode against Clang, and the
  system-header classification of every search group.
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

## Target macro exclusions

Each name omitted from the pinned Clang target dump is listed individually.
The first two categories identify separate ownership, rather than unsupported
C features. `__STDC__`, version/strictness and embed constants remain in the
existing language-mode builtin mechanism. Target macros do not emulate the
Clang compiler identity. Feature promises below require further semantic or
backend work before they can be enabled.

| Macro | Reason |
| --- | --- |
| `_MSC_BUILD` | Compiler identity/version or hosted-mode contract owned separately. |
| `_MSC_EXTENSIONS` | Compiler identity/version or hosted-mode contract owned separately. |
| `_MSC_FULL_VER` | Compiler identity/version or hosted-mode contract owned separately. |
| `_MSC_VER` | Compiler identity/version or hosted-mode contract owned separately. |
| `_MSVC_CONSTEXPR_ATTRIBUTE` | The advertised pragma/attribute semantics are not implemented. |
| `_MSVC_TRADITIONAL` | Traditional Microsoft preprocessing is not implemented; bcc uses its conforming preprocessor. |
| `_M_FP_CONTRACT` | MS floating code-generation modes are not implemented. |
| `_M_FP_PRECISE` | MS floating code-generation modes are not implemented. |
| `__BITINT_MAXWIDTH__` | Extended integer syntax exists but these widths lack semantic types. |
| `__CONSTANT_CFSTRINGS__` | Objective-C and CoreFoundation string intrinsics are not implemented. |
| `__FLT16_DECIMAL_DIG__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_DENORM_MIN__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_DIG__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_EPSILON__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_HAS_DENORM__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_HAS_INFINITY__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_HAS_QUIET_NAN__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_MANT_DIG__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_MAX_10_EXP__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_MAX_EXP__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_MAX__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_MIN_10_EXP__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_MIN_EXP__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_MIN__` | Half-precision floating types and arithmetic are not implemented. |
| `__FLT16_NORM_MAX__` | Half-precision floating types and arithmetic are not implemented. |
| `__FPCLASS_NEGINF` | Floating classification intrinsics are not implemented. |
| `__FPCLASS_NEGNORMAL` | Floating classification intrinsics are not implemented. |
| `__FPCLASS_NEGSUBNORMAL` | Floating classification intrinsics are not implemented. |
| `__FPCLASS_NEGZERO` | Floating classification intrinsics are not implemented. |
| `__FPCLASS_POSINF` | Floating classification intrinsics are not implemented. |
| `__FPCLASS_POSNORMAL` | Floating classification intrinsics are not implemented. |
| `__FPCLASS_POSSUBNORMAL` | Floating classification intrinsics are not implemented. |
| `__FPCLASS_POSZERO` | Floating classification intrinsics are not implemented. |
| `__FPCLASS_QNAN` | Floating classification intrinsics are not implemented. |
| `__FPCLASS_SNAN` | Floating classification intrinsics are not implemented. |
| `__FXSR__` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__GCC_ASM_FLAG_OUTPUTS__` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__GCC_CONSTRUCTIVE_SIZE` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__GCC_DESTRUCTIVE_SIZE` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__GCC_HAVE_DWARF2_CFI_ASM` | DWARF CFI assembly and unwind code generation are not implemented. |
| `__GNUC_MINOR__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__GNUC_PATCHLEVEL__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__GNUC_STDC_INLINE__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__GNUC__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__GXX_ABI_VERSION` | C++ ABI and type-info semantics are not implemented. |
| `__GXX_TYPEINFO_EQUALITY_INLINE` | C++ ABI and type-info semantics are not implemented. |
| `__MEMORY_SCOPE_CLUSTR` | Scoped atomic operations and memory scopes are not implemented. |
| `__MEMORY_SCOPE_DEVICE` | Scoped atomic operations and memory scopes are not implemented. |
| `__MEMORY_SCOPE_SINGLE` | Scoped atomic operations and memory scopes are not implemented. |
| `__MEMORY_SCOPE_SYSTEM` | Scoped atomic operations and memory scopes are not implemented. |
| `__MEMORY_SCOPE_WRKGRP` | Scoped atomic operations and memory scopes are not implemented. |
| `__MEMORY_SCOPE_WVFRNT` | Scoped atomic operations and memory scopes are not implemented. |
| `__MMX__` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__NO_INLINE__` | Code generation, relocation and optimization policy are not implemented. |
| `__NO_MATH_ERRNO__` | Code generation, relocation and optimization policy are not implemented. |
| `__OBJC_BOOL_IS_BOOL` | Objective-C and CoreFoundation string intrinsics are not implemented. |
| `__OPENCL_MEMORY_SCOPE_ALL_SVM_DEVICES` | Scoped atomic operations and memory scopes are not implemented. |
| `__OPENCL_MEMORY_SCOPE_DEVICE` | Scoped atomic operations and memory scopes are not implemented. |
| `__OPENCL_MEMORY_SCOPE_SUB_GROUP` | Scoped atomic operations and memory scopes are not implemented. |
| `__OPENCL_MEMORY_SCOPE_WORK_GROUP` | Scoped atomic operations and memory scopes are not implemented. |
| `__OPENCL_MEMORY_SCOPE_WORK_ITEM` | Scoped atomic operations and memory scopes are not implemented. |
| `__PIC__` | Code generation, relocation and optimization policy are not implemented. |
| `__PIE__` | Code generation, relocation and optimization policy are not implemented. |
| `__PRAGMA_REDEFINE_EXTNAME` | The advertised pragma/attribute semantics are not implemented. |
| `__SEG_FS` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__SEG_GS` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__SSE2_MATH__` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__SSE2__` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__SSE_MATH__` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__SSE__` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__STDC_EMBED_EMPTY__` | Existing language-mode preprocessor builtin; not a target definition. |
| `__STDC_EMBED_FOUND__` | Existing language-mode preprocessor builtin; not a target definition. |
| `__STDC_EMBED_NOT_FOUND__` | Existing language-mode preprocessor builtin; not a target definition. |
| `__STDC_HOSTED__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__STDC_UTF_16__` | Encoded literal syntax exists, but its semantic types and storage are not implemented. |
| `__STDC_UTF_32__` | Encoded literal syntax exists, but its semantic types and storage are not implemented. |
| `__STDC_VERSION__` | Existing language-mode preprocessor builtin; not a target definition. |
| `__STDC__` | Existing language-mode preprocessor builtin; not a target definition. |
| `__STRICT_ANSI__` | Existing language-mode preprocessor builtin; not a target definition. |
| `__VERSION__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__clang__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__clang_literal_encoding__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__clang_major__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__clang_minor__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__clang_patchlevel__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__clang_version__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__clang_wide_literal_encoding__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__code_model_small__` | Code generation, relocation and optimization policy are not implemented. |
| `__llvm__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__pic__` | Code generation, relocation and optimization policy are not implemented. |
| `__pie__` | Code generation, relocation and optimization policy are not implemented. |
| `__seg_fs` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__seg_gs` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__tune_k8__` | Code generation, relocation and optimization policy are not implemented. |

## Atomic front-end support (2026-10-09)

`_Atomic(T)` and qualifier syntax share a distinct canonical atomic type. C11
and later are native; earlier revisions use the existing extension policy.
The type specifier rejects qualified, atomic, array, function and incomplete
operands. Qualifier syntax rejects arrays, functions and incomplete objects,
but can qualify an existing qualified or atomic typedef, as Clang does.
Lvalue conversion removes atomicity; assignments and increments retain their
ordinary C value result. Atomic casts retain their atomic type and do not
qualify as integer constant expressions.

The three builtin families are type generic, with eligible object pointers,
value/buffer conversions and result types checked. Constant invalid operation
orders are warnings; fences accept any integer-convertible order. Failure
orders cannot release, but need not be weaker than the success order in the
pinned Clang. GNU generic buffers and compare-exchange expected buffers warn
when their conversions discard qualifiers. Only lock-free queries fold:
1/2/4/8-byte operations and the zero-size query are lock-free on the four
default targets; 16-byte `is_lock_free` remains runtime. An `always_lock_free`
query with a constant size can fold false. Pointer type alignment and typed
null pointers affect GNU lock-free queries, as in Clang.

The macro oracle now includes memory orders, supported lock-free promises and
legacy compare-and-swap widths. Its C23 additions have separate per-target
files because MSVC omits GCC spellings. Memory-scope extensions remain excluded.
`python scripts/update_target_macros.py --check` checks the C11/GNU17 contract
and C23 atomic additions against the pinned Clang.

`_Generic` now resolves associations and propagates the selected expression's
type, value category and constant value. This makes the atomic result-type
probes effective in bcc. Incomplete/void generic associations (a C2y extension
in Clang) remain outside the supported subset; void builtin results are
checked directly by semantic unit tests.

## Binary128 and type-generic math (2026-10-09)

`__float128`, `_Complex __float128` and `q`/`Q` floating suffixes are GNU
extensions in every ISO mode, governed by `Float128` and suppressed by
`__extension__` or system-header provenance. Linux GNU, Linux musl and MinGW
use IEEE binary128 (16 bytes, alignment 16), distinct from x87 long double,
with greater arithmetic rank. Complex binary128 occupies 32 bytes with
alignment 16. MSVC rejects the explicit type spelling, matching the pinned Clang 23.1.1,
but still accepts binary128 literals and their internal types through `typeof`.
Clang's target definitions now retain `__SIZEOF_FLOAT128__` on the three GNU
triples and `__FLOAT128__` on Linux. Clang defines no `__FLT128_*__` macros;
none are invented. The macro oracle has 94 documented exclusions.

The pinned Clang rejects native `_Float128`, `_Float32`, `_Float64`,
`_Float32x`, `_Float64x`, and `f128`/`F128` suffixes in C11, C17, GNU17,
C23 and GNU23. bcc follows that contract: glibc supplies its `_FloatN`
typedefs. The private resource `bits/floatn.h` chains to glibc and bridges
its GCC-4.2 version decision to the actual binary128 capability, enabling
`__HAVE_FLOAT128` and the distinct type without changing compiler identity
or editing the sysroot. Its complex typedef avoids the unmodeled GNU `mode`
attribute. Explicit `-isystem` directories still precede resources and can
bypass this bridge; normal `--sysroot` and `-idirafter` ordering uses it.

`__typeof__`, compatible-type tests, choose-expression selection, generic
selection, type classification, and real/imaginary components now have
semantic types and constant eligibility. The reachable old-GCC glibc
`tgmath.h` path uses these primitives, not `__builtin_tgmath`; the pinned
Clang rejects that builtin. No resource `tgmath.h` is supplied for MinGW.
Clang's resource implementation requires `overloadable` function resolution,
which remains unimplemented, and is ambiguous for binary128 calls.

Binary128 literals are parsed exactly with arena-backed integer rational
conversion and one IEEE round-to-nearest-even step. Binary128 arithmetic,
comparisons and casts are retained as non-ICE arithmetic constants without
numerical folding. Static arithmetic initializers are accepted, but an
integer constant expression requiring binary128 evaluation is unavailable.
The x87 helper is never used to approximate binary128. See
[semantic-analysis.md](semantic-analysis.md#binary128-and-type-generic-math-2026-10-09)
for verification and limits.

## GNU vectors and x86 SIMD resources (2026-10-09)

GNU vector semantics and x86 resource coverage follow [GCC Vector Extensions](https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html) and [Clang Language Extensions](https://clang.llvm.org/docs/LanguageExtensions.html); see semantic-analysis.md for the supported instruction families and constant-evaluation boundaries.
