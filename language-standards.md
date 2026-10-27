# Language standards

bcc-rust parses C89 through C23, a listed subset of the C2y draft, GNU
dialects, and independently enabled MSVC extensions. One configuration value,
CLI selection, predefined macros and a shared extension-diagnostic emitter
connect the lexer, preprocessor and recursive descent parser. This is a
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
  `-Wpedantic` and `-pedantic-errors` select a policy. Pointer diagnostic group
  controls and global `-Werror` are supported as described below;
  `-Wno-pedantic` and other unrelated warning groups are not implemented.
- `__extension__` suppresses keyword, grammar and numeric-constant extension
  diagnostics within its owning declaration or expression. Lexical diagnostics
  (dollar identifiers, `//` comments, digraphs), directives and macro
  definitions stay outside its scope.
- A preprocessing extension diagnostic raised in a macro expansion may
  underline the replacement in the macro definition. Parser and semantic
  extension diagnostics and warning-group diagnostics point at the outermost
  invocation instead, as Clang does; no expansion backtrace is rendered.
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
inline-semantics macro Clang defines: `__GNUC_GNU_INLINE__` in GNU89 inline
mode and `__GNUC_STDC_INLINE__` in C99 inline mode, each with value `1` and
the other absent. Before C99 (including C95), GNU89 inline is the default;
from C99 on, C99 inline is the default. `-fgnu89-inline` and
`-fno-gnu89-inline` override this choice independently of `-std`, with the
last inline flag winning. bcc permits explicitly selecting C99 inline even
in a pre-C99 mode; GCC diagnoses that combination. `-funsigned-char` (or
`-fno-signed-char`) makes plain `char` unsigned on every target, and
`-fsigned-char` (or `-fno-unsigned-char`) signed, the last flag winning, as in
GCC and Clang (C99 §6.2.5p15 leaves the choice to the implementation; every
bcc target defaults to signed). Plain `char` stays a distinct type; semantic
analysis, constant folding and lowering take its signedness from
`CompilerConfiguration::layout`, character constants such as `'\xff'` take its
value range, and `__CHAR_UNSIGNED__` is defined, with `_CHAR_UNSIGNED` on the
MSVC target as Clang defines it there. `#if` reads a one-byte narrow
character constant the same way (§6.10.1p4 lets it), so `#if '\xff' < 0` is
true exactly where plain `char` is signed, as in GCC and Clang; wide constants
keep their `wchar_t` value. `-ffloatn-types` adds GCC 7's `_FloatN` keywords,
constants, macros and builtins (see
[Interchange floating types](#interchange-floating-types-2026-10-26)); it is
on by default only under the `gcc` compiler identity, below, and off
otherwise, as for Clang. Only `__STRICT_ANSI__` distinguishes
strict modes from GNU modes in these macros. On the MSVC target,
explicitly enabling `-fms-extensions` defines
`_MSC_VER` `1933`, `_MSC_FULL_VER` `193300000`, `_MSC_BUILD` `1` and
`_MSC_EXTENSIONS` `1`: MSVC 19.33, Clang's default
`-fms-compatibility-version`. As in Clang, Linux and MinGW-w64 targets never
claim MSVC, even with `-fms-extensions`, so MinGW-w64 headers keep their GNU
paths (`__CRT_INLINE` stays `extern inline` with `gnu_inline`). Only the umbrella flag claims MSVC; the
individual `-fms-NAME` groups do not, and a later `-fno-ms-extensions`
withdraws it. Both GNU and MSVC identity can therefore be claimed at once,
whereas Clang drops the GNU identity under `-fms-compatibility`. Every mode
defines `__bcc__` `1`, `__bcc_major__`, `__bcc_minor__`, `__bcc_patchlevel__`
and the string `__bcc_version__` from the package version. Every mode also
claims the pinned Clang exactly as it identifies itself for the selected
target: `__clang__` `1`, `__clang_major__` `23`, `__clang_minor__` `1`,
`__clang_patchlevel__` `1`, `__clang_version__`, `__VERSION__`, `__llvm__`
and the narrow and wide literal encodings (`__clang_wide_literal_encoding__`
is `"UTF-16"` on the Windows targets). These are frozen with the target
definitions below, so the macro oracle checks them against the pinned Clang.
Headers therefore take their Clang paths, such as MinGW-w64's
`extern inline __attribute__((__gnu_inline__))` `__CRT_INLINE` and glibc's
`__glibc_clang_prereq` decisions; code that must tell bcc from Clang tests
`__bcc__`. `__GXX_ABI_VERSION` is never defined. Like Clang's, these are
ordinary macros that `#undef` may remove. OS identity
macros such as `__linux__` and `_WIN32` follow the selected target, below. The identity and target definitions are read from the synthetic
`<built-in>/predefined.h` before user input.

`--compiler-identity` claims another compiler than Clang, as the driver
personalities ([docs/design/driver-modes.md](docs/design/driver-modes.md)) do.
`gcc` defines `__GNUC__` `14`, `__GNUC_MINOR__` `2`, `__GNUC_PATCHLEVEL__`
`0`, `__VERSION__` `"14.2.0"` and GCC's execution character set names, and no
Clang macro (`__clang__` and its versions, `__llvm__`, the literal
encodings). It also defines GCC's `__FLT_IS_IEC_60559__`,
`__DBL_IS_IEC_60559__` and `__LDBL_IS_IEC_60559__` (`1`, as GCC 13 and
later), and `__GCC_IEC_559` and `__GCC_IEC_559_COMPLEX` as GCC computes them
from the options: `2`, or `0` under `-fno-signed-zeros`, `-freciprocal-math`
or an effective `-fassociative-math` (one with `-fno-signed-zeros` and
`-fno-trapping-math`), and the complex one also under limited or Fortran
complex arithmetic: `-f[no-]cx-fortran-rules` decides that alone when given,
so `-fcx-fortran-rules -fno-cx-limited-range` still gives `0` and
`-fno-cx-fortran-rules -fcx-limited-range` `2`, as in GCC 15, and
`-f[no-]cx-limited-range` decides only otherwise. Those options, `-f[no-]trapping-math` and
`-f[no-]math-errno` also define `__NO_SIGNED_ZEROS__`,
`__RECIPROCAL_MATH__`, `__ASSOCIATIVE_MATH__`, `__NO_TRAPPING_MATH__` and
`__NO_MATH_ERRNO__` there, and `-fno-fast-math` and
`-fno-unsafe-math-optimizations` restore them as in GCC
(`CompilerConfiguration::with_relaxed_floating_point`). bcc's code keeps
IEC 60559 semantics under all of them, which they permit; the other
identities accept them with no effect, since the pinned Clang's only
counterpart, `__NO_MATH_ERRNO__`, has a target-dependent default bcc's macro
tables leave out. Because GCC 7 and later
name the TS 18661-3 types as keywords, glibc's headers then stop declaring
them, so this identity turns on `-ffloatn-types` unless `-fno-floatn-types`
is given; with that, it defines `_Float32`, `_Float64`, `_Float32x`,
`_Float64x` and `_Float128` as macros for the standard types of the same
representation (`float`, `double`, `double`, `long double`, `__float128`)
instead, which are not distinct types. glibc's GCC fortify
wrappers for the `printf` family, `open` and `syslog` forward their arguments
with `__builtin_va_arg_pack`, which lowering expands at each call, so under
this identity they reach the checked functions. `msvc` (`cl.exe`) removes the
GNU and Clang macros and the inline-semantics macro, and defines `_MT` and
`_MSVC_TRADITIONAL` `0` (bcc's preprocessor is conforming, like `cl
/Zc:preprocessor`); `_MSC_VER` still needs `-fms-extensions` on the MSVC
target, which the `cl` personality selects. `clang-cl` removes the GNU macros
and keeps Clang's. `tcc` removes the GNU and Clang macros and defines
`__TINYC__` `927` (TCC 0.9.27). Every identity keeps `__bcc__`, the ISO
macros and the target's definitions, as clang-cl keeps `__x86_64__`.

Several options change rules the mode otherwise fixes. `-ftrigraphs` and
`-fno-trigraphs` (also GCC's `-trigraphs`) turn phase 1 trigraph replacement
on or off whatever the mode; the `cl` personality turns it off, as `cl`
replaces trigraphs only under `/Zc:trigraphs`. `-fwrapv` makes signed integer
`+`, `-`, `*`, `<<`, negation and `++`/`--` wrap in two's complement, so
lowering marks none of them `nsw`; pointer arithmetic is unaffected, as in
GCC. `-fpack-struct=N` (1, 2, 4, 8 or 16) sets the packing in effect before
any `#pragma pack`, which `#pragma pack()` restores, as GCC documents and
`cl /ZpN` does. `-fpack-struct` without a value instead packs every
structure and union as the `packed` attribute does (an `aligned` member keeps
its alignment) and ignores `#pragma pack` with GCC's warning, as GCC 15.2
does. `-fno-pack-struct` undoes both. `-fno-builtin` keeps LLVM from
treating any library function as a builtin (no `sqrt` instruction, no
`memset` formed from a loop, no `printf` turned into `puts`);
`-fno-builtin-NAME` does so for NAME, `-fbuiltin` undoes `-fno-builtin` but
keeps the named exclusions, as in GCC, and
`-ffreestanding` implies `-fno-builtin`, as in Clang. `-fvisibility=hidden`
(or `protected`, `internal` as hidden, `default`) gives that visibility to
the definitions with linkage that name none; declarations keep the default,
as in GCC.

Claiming Clang changes the paths the C libraries take. In glibc,
`sys/cdefs.h` drops its pre-C11 `_Noreturn` and `_Static_assert` fallbacks
(bcc accepts both keywords in every mode, as Clang does), marks socket
address structures `may_alias` (bcc's optimizer makes no type-based alias
assumptions) and turns `__glibc_macro_warning` into `#pragma GCC warning`;
`math.h` uses the type-generic classification builtins, `complex.h` defines
`CMPLX` with `__builtin_complex`, and `bits/floatn.h` enables binary128, as
described below. With `_FORTIFY_SOURCE` and `__OPTIMIZE__` (defined when
optimization is enabled), glibc selects Clang's fortify wrappers, which need the
`overloadable`, `pass_object_size`, `pass_dynamic_object_size`, `diagnose_if`,
`enable_if` and `unavailable` attributes. `__has_attribute` reports them as
Clang does. Semantic analysis retains overload identities and call selection,
hidden object-size parameters, and deferred call diagnostics; lowering expands
the hidden arguments and evaluates each written argument once. Object-size
inference handles fixed objects, interior pointers and modeled dynamic bounds.
Unmodeled allocation/conditional/compound-literal origins remain unknown, as do
runtime member and nested-array offsets in subobject modes 1 and 3. Unsupported
predicate shapes diagnose rather than being discarded. These contracts cover
the exercised glibc level-2/3 fortify wrappers and Clang C arithmetic,
real/complex, pointer-qualification and variadic-fallback overload selection.
Code that presents as GCC (GCC's version macros without Clang's) gets glibc's
GCC wrappers instead, which forward their anonymous arguments with
`__builtin_va_arg_pack ()` and count them with `__builtin_va_arg_pack_len ()`.
Lowering gives each direct call of a function using them a copy specialized for
the call's promoted anonymous argument types, folds the copy's branches on the
length at every optimization level, and refuses any use no call resolves
(`lowering/argument_pack.rs`).
Overload symbols use Clang's Itanium ABI on ELF and MinGW and its Microsoft ABI
on MSVC, including written nested-function qualifiers and shared argument/name
backreferences. Anonymous and repeated local tags receive ABI ordinals as
definition signatures and emitted references are processed, independently of
semantic overload identity. Bidirectional BCC/Clang
multi-translation-unit tests compile and link all four targets, executing on the
native host. A unique unmarked `enable_if` function keeps its C symbol; a marked
one keeps the target's Clang symbol. Clang's Microsoft ABI omits `enable_if`
predicates, and unmarked C predicates also omit overload distinctions. When
distinct entities actually collide, bcc preserves their bodies under distinct
private spellings; those colliding APIs cannot interoperate with Clang's symbol
scheme. Pinned Clang crashes for an anonymous record introduced only in a
Microsoft prototype, so bcc gives that unshareable type a deterministic private
name.
In MinGW-w64, `__CRT_INLINE` becomes
`extern inline` with `gnu_inline`, `setjmp` passes `__builtin_frame_address (0)`,
`intrin.h` always includes the x86 intrinsic headers, the printf and strftime
format archetypes become Clang's, and `intsafe.h` defines its functions as
`FORCEINLINE` bodies over the overflow builtins. bcc keeps each such
`always_inline` GNU inline body as a private definition that direct calls
reach and the inliner expands, so `intsafe.h` needs no external definition; a
body with inline assembly, as in `intrin-impl.h`, still calls the external
function. The UCRT's `corecrt.h` uses
`_Static_assert` for `_STATIC_ASSERT` and wraps itself in `#pragma clang
diagnostic`, which bcc ignores.

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
attribute parser. Each macro-expanded attribute follows the
[attribute status](#gnu-entity-attributes-and-attribute-status-2026-10-14)
table: `dllimport`, `dllexport`, `noreturn`, `nothrow`, `selectany` and
`noinline` are accepted, `__ms_abi__` is the target's own convention, `naked`
is refused, and `restrict`, `allocate(...)` and `uuid(...)` warn as unknown, as
bundled Clang's MinGW mode does, as does `align(...)`, which Clang ignores
there; with `-fms-extensions`, `__declspec` is the Microsoft keyword instead
(Clang defines it as itself) and `align(N)` is GNU `aligned(N)`. GNU
`aligned(...)` retains the requested declaration/type alignment; GNU `packed`
and scalar `mode` retain their layouts and representation types. GNU
`transparent_union` on a union definition or a typedef naming a union lets a
prototyped call pass an argument of a member's type as the union, selecting
the member as Clang does; a union with no member, a floating or vector first
member, or a member of another size or a stricter alignment ignores the
attribute with Clang's warning, as do the positions where it names no union
definition. On System V such an argument takes its first member's registers
and extension, and the whole union's stack slot once they are taken; Win64
passes the whole union.

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

Char8 atomic lock-free definitions are selected by `Feature::C23Keywords`,
whose acceptance is native C23/C2y only. Clang 23.1.1 defines
`__CLANG_ATOMIC_CHAR8_T_LOCK_FREE` as `2` on all four targets and
`__GCC_ATOMIC_CHAR8_T_LOCK_FREE` as `2` on the Linux and MinGW targets in C23
and GNU23, with neither in C17/GNU17. No other atomic lock-free macro is added
by C23. The GCC 13.2.0 `gcc` and `x86_64-w64-mingw32-gcc` on PATH reject the
`c23`/`gnu23` option names; their supported `c2x`/`gnu2x` modes define the GCC
char8 macro as `2`, while C17/GNU17 omit it. Thus they agree on availability
for the shared GCC spelling. GCC has no Clang spelling and cannot probe the
other Clang target triples. Frozen macro data is retained; no GNU17 char8
capability is advertised. C23 §7.17.1p4 (printed p. 293; PDF p. 306) specifies
the corresponding `ATOMIC_CHAR8_T_LOCK_FREE` header macro.

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
(the host's when compiling for the host, or `--sysroot`) and `-idirafter`; quoted lookup additionally gives local and
`-iquote` headers priority. `-nostdinc` and `-nobuiltininc` remove the resource
directory. `__has_include` uses the same search.
This follows Clang resource-header availability; use of newer language syntax
still follows the configured extension policy.

| Header | Contents and mode policy |
| --- | --- |
| `float.h` | C99 floating limits; C11 adds true minima, subnormal and decimal-digit macros. `FLT_ROUNDS` is the default round-to-nearest value `1`; `FLT_EVAL_METHOD` is `0`. Runtime changes to the rounding environment require backend support. Hosted with `__MINGW32__` or `_MSC_VER` defined, it first reads the C library's `<float.h>` and then replaces its characteristics, as Clang does; it defines GCC's guard `_FLOAT_H___` first, so MinGW-w64's header does not look for GCC's. |
| `iso646.h` | The eleven C alternative operator macros. |
| `limits.h` | Target-derived signed/unsigned limits; long-long limits from C99 onward. `CHAR_MIN` and `CHAR_MAX` are `0` and `UCHAR_MAX` when `__CHAR_UNSIGNED__` is defined (`-funsigned-char`), as in Clang's header. Hosted, it first reads the C library's `<limits.h>` for its POSIX and other additions, defining `_GCC_LIMITS_H_` in GNU modes so glibc does not look for GCC's header, then replaces the integer limits with the target's, as Clang does. `MB_LEN_MAX` is defined only if the C library did not (glibc 16, musl 4, MSVC 5); the fallback is `4`, since bcc's literals are UTF-8 and 4 bytes cover every stateless encoding. Clang's fallback is `1`. |
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
standard headers even with `-pedantic-errors`. The parser uses `parse_gnu` for
intrinsic expression operands; semantic analysis checks va-list operands,
va-arg result types and variadic-function context. Runtime pairing, argument
availability and promoted argument/type agreement remain backend/runtime work.
An `offsetof` member/index path yields a `size_t` integer constant; bit-fields
and invalid paths diagnose.

`#pragma clang deprecated(NAME)` also applies to user-defined macros. Its
optional message is a sequence of adjacent narrow string literals, decoded
like Clang's. Warnings point at the outermost replacement invocation (or the
argument being prescanned), with related expansion and pragma locations;
system-header suppression follows that use location. The mark survives
redefinitions, including identical ones, and is cleared by `#undef`.
Evaluated `#ifdef`, `#ifndef`, `defined`, `#elifdef` and `#elifndef` tests also
warn. Skipped conditional groups do not. This is implementation-defined
pragma behavior under C99 §6.10.6p1, rather than a GNU language-mode feature.

`-pedantic` and `-Wpedantic` select Warn; `-pedantic-errors` selects Deny;
the default is Allow. Later policy flags win. Deny emits an error but keeps
classified tokens available for structured parser recovery. Preprocessor
extensions with their own diagnostics keep their established recovery.

`-w` (also `--suppress-warnings`) suppresses warning rendering and warning
counts in every CLI view. Errors and extension-policy errors remain visible.

Inline emission follows [GCC's Inline manual](https://gcc.gnu.org/onlinedocs/gcc/Inline.html)
and [its dialect options](https://gcc.gnu.org/onlinedocs/gcc/C-Dialect-Options.html#index-fgnu89-inline).
GNU89 plain `inline` definitions emit an external body, while GNU89 `extern
inline` bodies provide no external definition even if another declaration
omits `inline`. `static inline` retains internal linkage in both modes.
In C99 mode, any file-scope declaration with `extern` or without `inline`
makes the inline body external (C99 §6.7.4p6-7, pp. 112-113; PDF pp. 124-125;
§6.9p5, p. 140; PDF p. 152). Block-scope declarations do not decide this.
The already parsed `gnu_inline`/`__gnu_inline__` attribute selects GNU89
semantics per function even with `-fno-gnu89-inline`; it does not change the
translation-unit predefined macro. `__has_attribute(gnu_inline)` is `1`.
On the Microsoft target, ordinary external inline bodies remain callable
definitions, but unused bodies can be omitted. Inline declarations propagate
forward. An explicit `extern` before the body, including a linked block-scope
declaration, requires its emission. The first explicit file-scope `extern`
after the body also requires emission with `-fms-extensions`. In C99 inline
mode, a later file-scope declaration without `inline`, or with `extern`, can
require emission if all preceding file-scope declarations specified `inline`
without `extern`. Calls, address references, `used`, constructors and destructors
keep the body and its dependencies. An applicable `dllexport` at or before the
body keeps an ordinary MSVC inline definition; an attribute introduced only
after the body remains ignored. Reached MSVC inline bodies use Clang's COMDAT
linkage: `linkonce_odr`, or `weak_odr` once an explicit `extern` or
`dllexport` requests the body, so units that each emit one link to a single
copy. On MinGW only `dllexport` makes an inline body an external `weak_odr`
definition, which may then reference internal functions and define
modifiable static objects. An imported inline body defines nothing, so an
alias may not target it. Explicit `gnu_inline` retains its separate GNU
rules.

MSVC `__forceinline` (Windows SDK `FORCEINLINE`) is `inline` plus
`always_inline` on every target, as in Clang: the definition takes the
inline linkage above (`linkonce_odr`, or `weak_odr` with `extern`, on MSVC;
a C99 inline definition elsewhere, whose `always_inline` body reaches LLVM
as Clang's `available_externally` definition, so a recursive one calls the
external definition at `-O0` as Clang's does), `static __forceinline` stays
internal, and `extern __forceinline` defines the external symbol. `noinline`
still prevails, and `__forceinline` on a redeclaration after the body is
ignored with Clang's warning.

BCC's target selection leaves Microsoft compatibility opt-in; Clang enables
it by default for the MSVC target. Compare inline emission with matching
compatibility flags on both compilers.

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

The typed adapter, `Context::report_extension_diagnostic`, retains the phase's
specific message, labels, notes and help. It shares native-feature suppression
and system-header handling with the generic emitter; non-accepted features
remain hard errors. `DiagnosticPolicy` and `policy_severity` also govern constraint
and quality diagnostics: variadic marker misuse, macro redefinition and empty
member declarations warn by default and become errors under Deny. Object-like
macro-generated defined remains a warning under Deny, matching Clang's
-Wexpansion-to-defined; function-like expansion uses the normal extension policy.
Strict C99+ implicit old-style parameters retain their hard-error baseline.
Unnamed definition parameters are native in C23; the earlier semantic fallback
now warns by default and is promoted under Deny, matching Clang and avoiding
duplicate reports from the parser.

`KeywordClassification` also records whether the written assembly introducer is
the ambiguous reserved __asm alias. Grammar owners use that metadata to choose
GNU parentheses or MSVC instructions/blocks. __has_builtin queries the builtin
registry (`src/builtins.rs`): a keyword form is advertised where `classify`
makes its spelling a keyword, an ordinary-call spelling by its naming policy.

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
| OmittedVariadicArguments | - | - | - | - | - | Y | Y | extension (MS exception) | preprocessor |
| PreprocessorComma | - | - | - | - | - | - | - | extension | preprocessor |
| MacroExpandedDefined | - | - | - | - | - | - | - | extension (object-like warning baseline) | preprocessor |
| QuotedHeaderBackslash | - | - | - | - | - | - | - | extension | preprocessor |
| UnnamedDefinitionParameters | - | - | - | - | - | Y | Y | extension (warning fallback) | parser/semantic analysis |
| InterchangeFloatTypes | - | - | - | - | - | Y | Y | `gcc` identity or `-ffloatn-types` | lexer/parser/semantic analysis |
| EmptyMacroArguments | - | - | Y | Y | Y | Y | Y | extension | lexer/preprocessor |
| Inline | - | - | Y | Y | Y | Y | Y | GNU earlier | parser |
| Restrict | - | - | Y | Y | Y | Y | Y | native | parser |
| Bool | - | - | Y | Y | Y | Y | Y | extension | parser |
| Complex | - | - | Y | Y | Y | Y | Y | extension | parser |
| Imaginary | - | - | Y | Y | Y | Y | Y | extension | parser (unsupported-type diagnostic) |
| ImplicitInt | Y | Y | - | - | - | - | - | extension | parser |
| MixedDeclarations | - | - | Y | Y | Y | Y | Y | extension | parser |
| ForDeclarations | - | - | Y | Y | Y | Y | Y | extension | parser |
| ForNonVariableDeclarations | - | - | - | - | - | Y | Y | extension | semantic analysis |
| ArrayParameterSyntax | - | - | Y | Y | Y | Y | Y | extension | parser |
| DesignatedInitializers | - | - | Y | Y | Y | Y | Y | extension | parser |
| CompoundLiterals | - | - | Y | Y | Y | Y | Y | extension | parser |
| FlexibleArrayMembers | - | - | Y | Y | Y | Y | Y | extension | parser |
| LongLong | - | - | Y | Y | Y | Y | Y | extension | parser |
| TrailingEnumComma | - | - | Y | Y | Y | Y | Y | extension | parser |
| OldStyleFunctionDeclarators | Y | Y | Y | Y | Y | - | - | native through C17 | parser |
| StaticAssert | - | - | - | Y | Y | Y | Y | extension | parser |
| Generic | - | - | - | Y | Y | Y | Y | extension | parser and sema |
| Alignas | - | - | - | Y | Y | Y | Y | extension | parser, sema, lowering |
| Alignof | - | - | - | Y | Y | Y | Y | extension | parser |
| Noreturn | - | - | - | Y | Y | Y | Y | extension | parser |
| ThreadLocal | - | - | - | Y | Y | Y | Y | extension | parser, sema, lowering |
| Atomic | - | - | - | Y | Y | Y | Y | extension | parser, sema, lowering, interpreter, LLVM |
| AnonymousAggregates | - | - | - | Y | Y | Y | Y | extension | parser |
| TypedefRedefinition | - | - | - | Y | Y | Y | Y | extension | semantic analysis |
| C23Keywords | - | - | - | - | - | Y | Y | native | preprocessor/parser |
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
| CaseRanges | - | - | - | - | - | - | Y | GNU earlier | parser |
| GnuAttribute | - | - | - | - | - | - | - | extension (GNU native) | parser; sema attribute pass; packed/aligned/scalar mode, transparent_union, cleanup, constructor/destructor, alias, weak, section, used, visibility lowering; vector_size sema |
| GnuAsm | - | - | - | - | - | - | - | extension (GNU native) | parser; sema statement checks; LLVM and native lowering |
| GnuTypeof | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| ExtensionMarker | - | - | - | - | - | - | - | extension (GNU native) | parser |
| StatementExpressions | - | - | - | - | - | - | - | extension (GNU native) | parser |
| BuiltinVaArg | - | - | - | - | - | - | - | reserved intrinsic in all modes | parser/sema |
| BuiltinOffsetof | - | - | - | - | - | - | - | reserved intrinsic in all modes | parser/sema |
| BuiltinTypesCompatible | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| VectorBuiltins | - | - | - | - | - | - | - | extension (GNU native) | shuffle/convert/bitcast and x86 signatures |
| BuiltinChooseExpr | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| LabelsAsValues | - | - | - | - | - | - | - | extension (GNU native) | parser, sema, lowering |
| LocalLabels | - | - | - | - | - | - | - | extension (GNU native) | parser, sema, lowering |
| OmittedConditionalOperand | - | - | - | - | - | - | - | extension (GNU native) | parser |
| OneVoidConditional | - | - | - | - | - | - | - | extension (GNU native) | semantic analysis |
| ConstantCompoundLiteral | - | - | - | - | - | - | - | extension (GNU native) | sema, lowering |
| ZeroLengthArrays | - | - | - | - | - | - | - | extension (GNU native) | parser |
| Float128 | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| Int128 | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| AutoType | - | - | - | - | - | - | - | extension (GNU native) | parser |
| GnuAlternateKeywords | - | - | - | - | - | - | - | extension (GNU native) | parser |
| AlignofExpression | - | - | - | - | - | - | - | extension (GNU native) | parser |
| VoidExpressionReturn | - | - | - | - | - | - | - | GNU only | semantic analysis |
| RealImag | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| GnuDesignators | - | - | - | - | - | - | - | extension (GNU native) | parser, sema, lowering |
| UnionCasts | - | - | - | - | - | - | - | extension (GNU native) | parser, sema, lowering |
| EmptyStructs | - | - | - | - | - | - | - | extension (GNU native) | parser |
| ExtraSemicolons | - | - | - | - | - | - | - | extension (GNU native) | parser |
| NestedFunctions | - | - | - | - | - | - | - | extension (GNU native) | parser |
| ImaginaryConstants | - | - | - | - | - | - | - | extension (GNU native) | lexer/preprocessor |
| EscapeCharacter | - | - | - | - | - | - | - | extension (GNU native) | lexer/preprocessor |
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
| FlexibleArrayInitializers | - | - | - | - | - | - | - | extension (GNU native) | semantic analysis, lowering |
| ConstantFolding | - | - | - | - | - | - | - | extension (GNU native) | semantic analysis |
| VoidPointerArithmetic | - | - | - | - | - | - | - | extension (GNU native) | semantic analysis, lowering |
| SameTypeCasts | - | - | - | - | - | - | - | extension (GNU native) | semantic analysis |
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

The following vocabulary entries retain the established diagnostic contracts:

- `OldStyleFunctionDeclarators` has origin `Removed { since: C89, removed: C23 }`.
  Identifier-list (K&R) declarators are accepted without an extension diagnostic
  through C17, in ISO and GNU modes. C23/C2y reject them under every policy,
  including GNU dialects, with the existing hard prototype-list error. Parsing
  retains the identifier-list shape for recovery. C99 §6.7.5p1 (printed p. 114;
  PDF p. 126) contains this grammar; C23 §6.7.7.1p1 (printed pp. 126-127;
  PDF pp. 139-140) removes it. The specialized error is retained pending the
  later diagnostic-helper follow-up.
- `OmittedVariadicArguments` has C23 origin and is accepted in every mode.
  Before C23, Allow is silent, Warn emits the existing `MissingVariadicArgument`
  warning, and Deny emits that specialized error, for ISO and GNU dialects.
  `MsVaArgs` exempts omitted arguments from that diagnostic independently of
  the revision. C23/C2y are silent. This covers direct calls, nested replacement
  rescanning and preprocessing-expression expansion. C99 §6.10.3p4 (printed
  p. 151; PDF p. 163) requires more arguments than named parameters;
  C23 §6.10.5p4 (printed p. 178; PDF p. 191) requires at least as many.
- `__func__` has its predefined declaration in C99
  (§6.4.2.2p1, printed p. 52; PDF p. 64). Like Clang 23.1.1, bcc also
  accepts this reserved identifier silently in C89, including `-pedantic-errors`.
  `__FUNCTION__` and
  `__PRETTY_FUNCTION__` are the GNU spellings: they are not a feature row,
  because Clang 23.1.1 reserves both in every `-std` and says nothing about them
  even under `-pedantic`, so bcc accepts them silently in every mode, ISO,
  GNU and every extension policy. Each names a `static const char` array. In C,
  `__FUNCTION__` is the same object as `__func__` (the function's name), and
  `__PRETTY_FUNCTION__` holds the signature as Clang prints it (`void f(int)`,
  `int (*g(void))(char)`; GCC prints only the name in C). A definition
  without a prototype prints `()`, and a typedef name appears as written for
  a parameter or result (`size_t`), but not for an adjusted array
  parameter. Known differences from Clang are an unnamed `struct` parameter
  (`struct (unnamed)`, where Clang adds a source position) and a variable
  length array behind a pointer (`[*]`, where Clang prints the bound
  expression). Outside a function all three are an empty string
  (`top level` for the pretty form) behind the default warning
  `predefined identifier is only valid inside function`, Clang's
  `-Wpredefined-identifier-outside-function`. Clang makes `__FUNCTION__` and
  `__PRETTY_FUNCTION__` keywords, so declaring them is a syntax error there;
  bcc uses a visible declaration of the same spelling in preference.
  The objects are static, so address equality holds as in Clang: all uses of
  `__func__` and `__FUNCTION__` in one function are one object, and so are all
  uses of `__PRETTY_FUNCTION__`; the two objects never compare equal, and no
  object equals a string literal (Clang may merge equal constants, which C
  leaves unspecified). Source: [GCC function names](https://gcc.gnu.org/onlinedocs/gcc/Function-Names.html).
- `ArrayParameterSyntax` has C99 origin and accepts array qualifiers, `static`
  bounds and `[*]` in every mode. C89/C95 report the existing C99-origin
  spellings under Warn/Deny, including GNU modes; C99 and later are silent.
  Parser `__extension__` suppression applies. C99 §6.7.5p1 (printed p. 114;
  PDF p. 126) specifies all three forms.
- `SameTypeCasts` has GNU origin. A cast of a structure or union to its own
  type, qualifiers aside (`(struct S)s`, through typedefs too), gives the
  operand's value as an rvalue, as GCC accepts it ("ISO C forbids casting
  nonscalar to the same type" under -Wpedantic; C99 §6.5.4p2 requires a scalar
  or void type). Warn/Deny report it, except a cast spelled `(union ...)`,
  which keeps the parser's `UnionCasts` report. A cast to another aggregate
  type stays invalid, the result is not an lvalue, and it is no constant
  initializer of a static object, as in GCC.
- `ConstantCompoundLiteral` has GNU origin. A compound literal of a static
  object's own type as its whole initializer, possibly parenthesized, is
  accepted as if its braced list were written in its place, when every value
  the literal stores is a constant its destination accepts: a structure or
  union (GCC and Clang), or a scalar of a compatible type, qualifiers aside
  (`static int x = (int){3};`, GCC only; `static double d = (int){3};` and
  `(int){3} + 1` stay non-constant, as in GCC). Lowering copies the bytes of
  the literal's own initializer. Warn/Deny report "'compound literal as a
  constant initializer' is a GNU extension" where GCC says "initializer
  element is not constant [-Wpedantic]". Source: [GCC compound
  literals](https://gcc.gnu.org/onlinedocs/gcc/Compound-Literals.html).
- `AlignofExpression` has GNU origin and accepts an expression operand in
  every mode, parenthesized or, as GCC accepts it, not (`_Alignof x`,
  `_Alignof (a)[1]`): every spelling takes `__alignof__`'s operand, a
  parenthesized type name or a unary expression, like `sizeof`. ISO
  `_Alignof` and C23 `alignof` spellings report GNU origin under
  Warn/Deny, including GNU dialects; Allow is silent. Reserved GNU aliases
  retain their exemption, and parser suppression still applies. The source is
  [GCC's alignment extension](https://gcc.gnu.org/onlinedocs/gcc/Alignment.html).
  Keyword availability and spelling checks are independent and unchanged.
- `TypedefRedefinition` has C11 origin. Redefining a typedef to the identical,
  non-variably-modified type is accepted in every mode, reported as
  `typedef redefinition` with C11 origin under Warn/Deny before C11 (GNU
  included), and silent under Allow or from C11 onward. Different types and
  variably modified typedefs retain their constraint errors. C11 §6.7p3
  (printed p. 108; PDF p. 126) specifies this exception.
- `ForNonVariableDeclarations` has C23 origin. A for-initializer declaration
  without declarators is accepted in every mode and reported with the existing
  `non-variable declaration in 'for' loop` spelling under Warn/Deny before
  C23, including GNU modes. Allow and C23/C2y are silent. Existing storage-class
  and other semantic constraints remain in force. C99 §6.8.5p3 (printed p. 135;
  PDF p. 147) restricts this position to automatic objects; C23 §6.8.6.1p1-2
  (printed p. 155; PDF p. 168) has declaration syntax without that restriction.
- `VoidExpressionReturn` has GNU origin and requires a GNU dialect. Returning
  a void expression from a void function reports the existing GNU-origin
  `return with a void expression` diagnostic under Warn/Deny and is silent
  under Allow. Every ISO revision retains `VoidReturnValue` as a hard error.
  Returning a non-void value is still an error in GNU modes. C99 §6.8.6.4p1
  (printed p. 139; PDF p. 151) forbids return expressions in void functions;
  [GCC's `c_finish_return`](https://gnu.googlesource.com/gcc/+/refs/tags/basepoints/gcc-13/gcc/c/c-typeck.cc)
  implements the void-expression exception.

`CaseRanges` retains C2y origin ([WG14 N3370](https://www.open-std.org/JTC1/SC22/WG14/www/docs/n3370.htm)),
extending C99 §6.8.1p1 (printed p. 131; PDF p. 143). Like `BinaryConstants`
and `VaOpt`, acceptance is native or GNU: strict pre-C2y modes emit a hard
syntax diagnostic regardless of extension policy or `__extension__` suppression.
They still parse and retain the range and following input. GNU pre-C2y modes
are silent under Allow and report **C2y** origin under Warn/Deny; parser
suppression applies to that policy diagnostic. C2y is silent. There is no
individual GNU feature CLI switch; `-std=gnu*` enables earlier case ranges.
Pinned Clang 23.1.1 accepts them with a C2y warning in both `-std=c17 -pedantic`
and `-std=gnu17 -pedantic`, and silently under `-std=c2y`. bcc's strict rejection
is the maintainer-selected availability gate.

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
| C11/C17 | `_Alignas` expression/type operands, `_Alignof` types (expression operands as a GNU extension), `_Atomic` type/qualifier, `_Generic` associations, `_Noreturn`, `_Static_assert`, `_Thread_local`, anonymous untagged aggregates; `_Alignas` in a plain type name is a constraint error | Alignment validity, storage-class combinations and aggregate layout; atomic eligibility and generic selection are analyzed |
| C23 | All `[[...]]` attribute positions with standard/vendor names and balanced arguments; `bool`/`true`/`false`, `nullptr`, `constexpr`, `typeof`/`typeof_unqual`, message-optional `static_assert`, `alignas`/`alignof`/`thread_local` aliases; labels before declarations and at block end; `{}`; `_BitInt` with signedness; fixed enum underlying specifier-qualifier lists; `auto` type inference, alone or beside another storage class except `typedef`; unnamed definition parameters; ellipsis-only prototypes; empty parameter lists as prototypes; rejected identifier-list declarators; decimal type keywords; compound-literal storage classes | Attribute applicability/meaning, inferred types, width values, enum type legality, decimal literal support and literal object lifetime |
| C2y subset | `_Countof` unary expression or parenthesized type; type-controlling `_Generic`; `if`/`switch` declaration headers with an optional following expression; case ranges; `break`/`continue` label | Array/type/count evaluation, selection conversion, range overlap, and named control-target resolution |

The keyword classifier reports reserved keywords, so parser consumers do not
duplicate keyword-origin diagnostics. Unambiguous grammar-only features report
through the shared emitter and keep their AST under Deny. Non-reserved C23
aliases remain identifiers in earlier strict modes.

New operands, assertions, generic selections, specifier additions and
attributes are immutable arena nodes. `parse_modern` owns their delimiters and
delegates expression and type children to their productions. `parsing/tests/node_sizes.rs` pins node sizes. Inspection
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

`__thread` is a reserved GNU alternate spelling of `_Thread_local`, a keyword
in every mode, as in GCC and Clang ([GCC Thread-Local
Storage](https://gcc.gnu.org/onlinedocs/gcc/Thread-Local.html)). Under the
unified policy `-pedantic` reports it as a GNU extension, which Clang does
not; Clang's separate pedantic warning for `__thread` written before `static`
or `extern` is not reproduced. The three spellings mean the same storage
class, but `__thread` beside `_Thread_local` or `thread_local` is an error and
the same one repeated is a repeated-specifier warning, as in Clang. With
`static` or `extern` only (and `auto` from C23), thread storage obeys C11
§6.7.1p2-4 (C23 (N3220) §6.7.2p2-3): parameters and functions reject it, a
block-scope object needs `static` or `extern`, and every declaration of the
object agrees. A thread-local object's address is no address constant (C11
§6.6p9), and the objects become thread-local globals. C23 storage-class
specifiers in a compound literal parse, but lowering refuses a `static` one in
a function and any `thread_local` one by name, rather than giving it automatic
or shared storage.
MSVC `__declspec(thread)` (on MinGW, where `__declspec(a)` expands to
`__attribute__((a))`, the attribute `thread`) is the same storage duration
under an attribute's spelling: analysis gives the object thread storage and
applies the function, parameter, block-scope and agreement constraints to it,
so it is never shared silently. The parser's combination checks do not see it.

GNU attributes accept declaration, aggregate, enumerator, pointer, parameter,
nested declarator, array, bit-field, function and label positions from
[GCC Attribute Syntax](https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html).

[GNU C Extensions](https://gcc.gnu.org/onlinedocs/gcc/C-Extensions.html) and
[GCC Extended Asm](https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html) specify
the vendor grammar. `parse_gnu` owns assembly, builtins and `__label__`
declarations. Assembly nodes keep typed qualifiers, the template literal,
constraint and clobber literals, parsed operand expressions, symbolic names and
goto labels; file assembly and declarator assembly labels have explicit AST
variants. Semantic analysis checks statement-level assembly (constraints,
operand lvalues and constants, clobbers, template references and goto
labels) and retains it for lowering; see
[middle-end.md](middle-end.md#gnu-inline-assembly-2026-10-26). Builtins keep their type/expression operands and `__builtin_offsetof`
member paths. Statement expressions, local labels, label addresses, computed
goto, omitted conditional operands, GNU initializers and nested definitions
also have inspectable syntax nodes.

`__extension__` suppresses extension diagnostics for keyword, grammar and
numeric-constant tokens within its expression or declaration, including a
function body or a parameter. Suppression ends at the owning production's boundary;
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
  inline property; semantic analysis adds `always_inline`.
- `parse_msvc` owns SEH: `__try` guarded compounds, `__except` expression
  filters, `__finally` compounds and `__leave;`. Children use the ordinary
  compound and expression productions and scope restoration. Missing
  handlers, bodies, filters and delimiters produce recovered syntax and preserve
  following input. `__leave` placement and handler semantics are later analysis.
- `parse_msvc` also owns `__asm`/`_asm` brace blocks and single-line instructions
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
and out-of-range forms diagnosed. An unknown escape such as `\j` or `\%`
stands for its character after the warning "unknown escape sequence `\j`
[-Wunknown-escape-sequence]", in every mode, strict ISO included, as GCC and
Clang do: C99 requires only a diagnostic (§6.4.4.4 footnote 65) and §6.11.4
leaves non-lowercase escapes to extensions. `-pedantic-errors` makes it an
error, as Clang's extension warning is; `-Wno-unknown-escape-sequence`
silences it and `-Werror=unknown-escape-sequence` promotes it, and a system
header withholds it. Phase 6 concatenates ordinary strings with
encoded strings while keeping the encoding, and diagnoses incompatible
nonordinary prefixes. Phase 7 adds encoding-bearing literal, bit-precise
integer, and imaginary constant token variants. Semantic analysis types the
encoded literals (see `semantic-analysis.md`, "Type graph and target data
model") and lowering emits their UTF-8, UTF-16 and UTF-32 units.

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
`_Noreturn`, including double-underscore forms. GNU `__has_attribute(name)`
and `__has_c_attribute(gnu::name)` or `(clang::name)` answer from the
attribute catalog exactly as bundled Clang does for the selected target
(`external_source_symbol` is 20230206), including attributes that bcc
refuses or only accepts, and 0 for other names and vendors; see
[the attribute status](#gnu-entity-attributes-and-attribute-status-2026-10-14).
`__has_builtin` returns 1 for `__builtin_va_arg`, x86 `__rdtsc` (and
Microsoft's `__debugbreak` with Microsoft extensions),
`__builtin_va_start`, `__builtin_va_end`, `__builtin_va_copy`,
`__builtin_offsetof`, `__builtin_types_compatible_p`, and
`__builtin_choose_expr`, `__builtin_classify_type`, the modeled `__c11_atomic_*`,
`__atomic_*` and `__sync_*` names, the [common GCC scalar intrinsic families](#common-gcc-scalar-intrinsics-2026-10-12),
the generated x86 builtins, the vector shuffle, conversion, bit-cast,
elementwise and non-temporal builtins, and the binary128 primitives
(`__builtin_inff128` and its family), whatever their lowering status.
Unmodeled names return 0. Clang's `__has_feature` and `__has_extension` share
`__has_attribute`'s gate: `c_alignas`, `c_alignof`, `c_atomic`,
`c_generic_selections`, `c_static_assert` and `c_thread_local` are features
where the C11 feature is native and extensions wherever bcc accepts it, as in
Clang; `attribute_deprecated_with_message` and `attribute_overloadable` are features
in every mode. `overloadable_unmarked` is an extension but not a feature, as
in Clang. Every other name returns 0. These tables describe the
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

`#pragma pack`, `_Pragma("pack(...)")` and enabled `__pragma(pack(...))` retain
translation order for semantic record layout. Numeric caps, reset, nested
push/pop, identifiers, optional pop alignment and `show` are supported on all
targets. Malformed directives warn and preserve state; unknown pragmas retain
the existing ignore policy. `#pragma weak name` (and `name = target`) does not
make `name` weak in emitted code, but semantic analysis keeps the name, so the
values Clang's evaluator would not fold for a weak entity stay unfolded where
the dead-code model reads them (`semantic-analysis.md`). Packing is implementation-defined pragma behavior
under C99 §6.10.6p1, not a GNU/MSVC language-mode gate. Semantic analysis owns
the cap, bit-field placement and explicit-alignment interaction; see
[the retained layout contract](semantic-analysis.md#storage-natural-and-member-place-alignment),
[GCC's pragma documentation](https://gcc.gnu.org/onlinedocs/gcc/Structure-Layout-Pragmas.html),
and [MSVC pack](https://learn.microsoft.com/en-us/cpp/preprocessor/pack).

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
identity that is not a system header. A GNU line marker (`# 12 "file.c" 1 3`),
the form `-E` writes, is read as `#line 12 "file.c"`. Flag 3 makes the
following lines a system header until the next line marker in the same
physical file, which ends the region unless it has flag 3 too, as GCC and
Clang read preprocessed output; the other flags are ignored, and a `#line`
directive does not end the region. The flags must come in GCC's order, one of
`1` or `2`, then `3`, then `4` (only after `3`): the first token out of that
order is an error ("invalid flag"), and it and the flags after it are ignored,
so `1 2 3` or `1 wat 3` makes no system header. A header included from a
system header, a marked region or the part after `#pragma GCC system_header`
included, is a system header too however it is found (beside the includer,
through `-I`, or by an absolute path), as GCC takes the including position's
status. A header that is a system header by its
directory or pragma stays one. So `-E` output compiled again withholds its
system headers' warnings, also from ordinary source. Already
preprocessed input (`.i`, `-x cpp-output`) is read as GCC's
`-fpreprocessed` reads it: phases 1 and 2 replace no trigraph and delete no
line splice (CR and CRLF still end lines), no macro, builtin or not, is
replaced, and only line markers, `#pragma`, `#ident`, `#sccs`, `#define` and
`#undef` whose `#` starts the line are directives; any other directive is an
error. `#pragma GCC warning` and `#pragma GCC
error` report their macro-replaced string operand (optionally parenthesized,
adjacent literals concatenated) as a warning or an error, as GCC and Clang do;
any other operand is an error. glibc's `__glibc_macro_warning`, which bcc
reaches as Clang does, uses the warning form for deprecated macros.

Inside a system header, warnings, extension diagnostics and warning-group
diagnostics are withheld, the latter two at every severity, except the
message of a `#warning` directive, which GCC and Clang also emit there
(`#pragma GCC warning` stays withheld, as in GCC): under
`-pedantic-errors` an extension in a system header is not reported, and
neither is an `-Wincompatible-pointer-types`, `-Wint-conversion` or
`-Wincompatible-function-pointer-types` error, matching Clang, where such
diagnostics are warnings that default to or are promoted to errors and are
not emitted from system headers. Policy-governed
preprocessor diagnostics (macro redefinitions, `__VA_ARGS__` and `__VA_OPT__`
outside a variadic macro, empty variadic arguments, the `#if` comma operator,
`defined` produced by a function-like macro, a backslash in a quoted header
name) count as extensions. Errors, such as syntax errors, constraint violations
that are errors by default, and `#error`, are always reported.

The diagnostic's expansion location decides, as in Clang: for a token that a
macro expansion produced, where the outermost invocation begins, and
otherwise the token itself. The preprocessor records that location for every
output token it expands from a macro (`Context::record_expansion_start`), and
the record lasts for the whole translation unit, so semantic analysis reads
it too. A diagnostic in user code about something a system header declared or
defined, such as a redefinition of the header's macro, is reported, and so is
a construct that a system header's macro supplies to user code, like a
`long long` in C89 code or `SYS_SET(ip)` assigning an `unsigned *`, which is
reported at the user's invocation; a user macro expanded inside a system
header is withheld. Tokens of a macro argument take the invocation too, where
Clang would point at the argument's spelling; that changes only the column.
Preprocessing diagnostics keep their own locations. Suppression happens when
the diagnostic is reported, so withheld diagnostics are not counted.

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
  `_Thread_local`/`thread_local`/GNU `__thread` share kinds. Aliases keep their original
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
  another removed C89 feature. C89/C95 and GNU modes create a declaration
  under the configured pedantic policy. Known library names use the complete
  library prototype and emit a warning naming the function and type, as Clang
  does; all other names retain unprototyped int returns. Explicit declarations
  keep the user's type. Strict C99 and later report an undeclared identifier.
  Ordinary non-call identifier lookup never creates an implicit declaration.
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
- CLI: [`tests/integration/cli.rs`](tests/integration/cli.rs) covers flag
  parsing and overrides;
  [`tests/integration/language_cli.rs`](tests/integration/language_cli.rs) runs
  every accepted standard spelling through preprocessing, parsing and syntax
  inspection, with GNU and MSVC programs, disabled gates and malformed
  cross-phase input.
- Diagnostics: golden inputs under `tests/fixtures/diagnostics/` take optional
  `.args` sidecars that select mode and policy; see its `COVERAGE.md`.
- Hosted translation:
  [`tests/integration/hosted_cli.rs`](tests/integration/hosted_cli.rs) covers the
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
existing language-mode builtin mechanism. Clang's own identity macros
(`__clang__` and its version, `__llvm__`, `__VERSION__` and the literal
encodings) are not excluded: bcc claims the pinned Clang, so they are frozen
with each target. Feature promises below require further semantic or
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
| `__OPTIMIZE__` | Defined by the identity macro builder when optimization is enabled. |
| `__OPTIMIZE_SIZE__` | Defined by the identity macro builder for `-Os` and `-Oz`. |
| `__NO_INLINE__` | Defined by the identity macro builder when optimization is disabled. |
| `__NO_MATH_ERRNO__` | Code generation, relocation and optimization policy are not implemented. |
| `__OBJC_BOOL_IS_BOOL` | Objective-C and CoreFoundation string intrinsics are not implemented. |
| `__OPENCL_MEMORY_SCOPE_ALL_SVM_DEVICES` | Scoped atomic operations and memory scopes are not implemented. |
| `__OPENCL_MEMORY_SCOPE_DEVICE` | Scoped atomic operations and memory scopes are not implemented. |
| `__OPENCL_MEMORY_SCOPE_SUB_GROUP` | Scoped atomic operations and memory scopes are not implemented. |
| `__OPENCL_MEMORY_SCOPE_WORK_GROUP` | Scoped atomic operations and memory scopes are not implemented. |
| `__OPENCL_MEMORY_SCOPE_WORK_ITEM` | Scoped atomic operations and memory scopes are not implemented. |
| `__PIC__` | Relocation-model macros follow -fpic, -fpie and -fno-pic; defined beside the identity macros. |
| `__PIE__` | Relocation-model macros follow -fpic, -fpie and -fno-pic; defined beside the identity macros. |
| `__PRAGMA_REDEFINE_EXTNAME` | The advertised pragma/attribute semantics are not implemented. |
| `__SEG_FS` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__SEG_GS` | Vector/CPU intrinsics, inline assembly and address-space semantics are not implemented. |
| `__STDC_EMBED_EMPTY__` | Existing language-mode preprocessor builtin; not a target definition. |
| `__STDC_EMBED_FOUND__` | Existing language-mode preprocessor builtin; not a target definition. |
| `__STDC_EMBED_NOT_FOUND__` | Existing language-mode preprocessor builtin; not a target definition. |
| `__STDC_HOSTED__` | Compiler identity/version or hosted-mode contract owned separately. |
| `__STDC_UTF_16__` | Encoded literal syntax exists, but its semantic types and storage are not implemented. |
| `__STDC_UTF_32__` | Encoded literal syntax exists, but its semantic types and storage are not implemented. |
| `__STDC_VERSION__` | Existing language-mode preprocessor builtin; not a target definition. |
| `__STDC__` | Existing language-mode preprocessor builtin; not a target definition. |
| `__STRICT_ANSI__` | Existing language-mode preprocessor builtin; not a target definition. |
| `__code_model_small__` | Code generation, relocation and optimization policy are not implemented. |
| `__pic__` | Relocation-model macros follow -fpic, -fpie and -fno-pic; defined beside the identity macros. |
| `__pie__` | Relocation-model macros follow -fpic, -fpie and -fno-pic; defined beside the identity macros. |
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

As of 2026-10-16, atomic objects and all modeled builtin spellings execute in
the interpreter and LLVM backend. Aggregate operations use the platform
libatomic runtime; the native backend explicitly refuses atomic instructions.
See [atomic execution](middle-end.md#atomic-execution) for ordering, runtime
requirements and the extended-float interpreter limit.

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
none are invented. The macro oracle has 82 documented exclusions.

The pinned Clang rejects native `_Float128`, `_Float32`, `_Float64`,
`_Float32x`, `_Float64x`, and `f128`/`F128` suffixes in C11, C17, GNU17,
C23 and GNU23. bcc follows that contract by default: glibc supplies its
`_FloatN` typedefs, so the names must stay identifiers. `-ffloatn-types`
(`CompilerConfiguration::with_interchange_float_types`, see
[Interchange floating types](#interchange-floating-types-2026-10-26)) gives
GCC 7's contract instead, and the `gcc` identity turns it on. Because bcc claims Clang, glibc's own `bits/floatn.h` enables
`__HAVE_FLOAT128` through `__glibc_clang_prereq (3, 9)` and declares the
complex type as `_Complex float __attribute__ ((__mode__ (__TC__)))`. The GNU
`mode` attribute selects real (`SF`, `DF`, `XF`, `TF`, `KF`) and complex
(`SC`, `DC`, `XC`, `TC`, `KC`) floating modes as Clang resolves them: `T`
and `K` name binary128 on the targets that spell `__float128`, and `X` the
x87 format where `long double` uses it. The earlier private `bits/floatn.h`
bridge, which compensated for the GCC 4.2.1 claim without Clang's, is gone.

`__typeof__`, compatible-type tests, choose-expression selection, generic
selection, type classification, and real/imaginary components now have
semantic types and constant eligibility.

`typeof(expression)` retains intrinsic typedef and record alignment while
excluding alignment attached only to an object's, member's or parameter's
storage; GNU `__alignof__(expression)` still observes that storage alignment.
Equal arithmetic operands retain their aligned typedef type, subject to the
usual promotions. A later object redeclaration supplies its typedef-derived
alignment, while explicit object alignment requests remain inherited.

Lowering diagnostics for unmodeled types retain the actual construct through
typedefs, arrays and record members: `_BitInt`, `constexpr`, decimal types,
inferred declarations, fixed-underlying enums, MSVC pointer modifiers or
`ext_vector_type`. They no longer list unrelated possible causes.

The reachable old-GCC glibc
`tgmath.h` path uses these primitives, not `__builtin_tgmath`; the pinned
Clang rejects that builtin. The embedded LLVM 23.1.1 resource `tgmath.h`
uses arithmetic promotion overloads, real/complex selection and an unavailable
variadic fallback. It is checked against pinned Clang in C17, GNU17 and C23
on all four targets. Its binary128 calls retain Clang's ambiguous-overload
diagnostic; the header does not define a binary128 promotion overload. The
resource and its LLVM license are distributed together under `src/headers/`.
bcc changes it in three places. Where the interchange floating types are on
(`__FLT32_MANT_DIG__` under `__bcc__`), `_Float32`, `_Float64`, `_Float32x`,
`_Float64x` and their complex types promote to the standard type with the
same format, so a call gives the value GCC's `_FloatN` function gives, with
the standard type as its type; `_Float128` stays refused like `__float128`,
and `nexttoward` refuses a `_FloatN` first argument, as GCC's tgmath does
(the C library has no such function). And in every mode `conj` of a real
argument returns imaginary part -0, the conjugate of the +0 a real number
has as a complex one (§7.22p6, §7.3.9.3), as glibc and GCC give; Clang's
header returns the argument unchanged, with +0.

Binary128 literals are parsed exactly with arena-backed integer rational
conversion and one IEEE round-to-nearest-even step. Binary128 arithmetic,
comparisons and casts are retained as non-ICE arithmetic constants without
numerical folding. Static arithmetic initializers are accepted, but an
integer constant expression requiring binary128 evaluation is unavailable.
The x87 helper is never used to approximate binary128. See
[semantic-analysis.md](semantic-analysis.md#binary128-and-type-generic-math-2026-10-09)
for verification and limits.

## Interchange floating types (2026-10-26)

`-ffloatn-types` (and `-fno-floatn-types`; the last wins) makes `_Float32`,
`_Float64`, `_Float128`, `_Float32x` and `_Float64x` keywords, as GCC 7 and
later have them in every C mode, with the C23 Annex H constant suffixes
`f32`, `f64`, `f128`, `f32x` and `f64x`, with `f` or `F` but only a lowercase
`x`, as GCC requires (§H.5.1-H.5.2). It is
off by default because glibc's `bits/floatn.h` declares these names as
typedefs for the Clang identity bcc claims by default. The `gcc` identity
(`--compiler-identity=gcc`, which the GCC personality selects), which makes
glibc expect the keywords, turns it on unless `-fno-floatn-types` is given;
`CompilerConfiguration::interchange_float_types` is the explicit choice, or
else whether the identity is GCC's. The `Feature` row is
`InterchangeFloatTypes`: origin C23, so `-pedantic` before C23 warns
"'_Float32' is a C23 extension" where GCC says "ISO C does not support the
'_Float32' type before C23".

Each type is distinct (§H.2.2p3, §H.2.4p2) and has the representation of a
type bcc already lowers: binary32 (`float`), binary64 (`double`, for
`_Float64` and `_Float32x`), the target's `long double` for `_Float64x`, and
binary128 for `_Float128`, which is the same type as `__float128`, as in GCC.
`_Generic`, `__builtin_types_compatible_p` and the usual arithmetic
conversions see the distinct types: within one precision an interchange type
is chosen over `long double`, then `double`, then the extended type
(§H.4.3p1), so `_Float32 + float` is `_Float32` and `_Float32x + double` is
`double`. The classes follow the target's formats: on the MSVC target, where
`long double` is binary64, `_Float64 + long double` is `_Float64` and
`_Float32x + long double` is `long double`, as GCC gives with
`-mlong-double-64`. Default argument promotions apply only to `float`, as in GCC.
`_Complex` combines with each in either order. On the MSVC target, whose
`long double` is binary64 and which has no binary128 type, `_Float64x` and
`_Float128` are refused as unsupported.

With the switch, the `__FLT32_*__`, `__FLT64_*__`, `__FLT32X_*__`,
`__FLT64X_*__` and `__FLT128_*__` characteristics macros are predefined
with GCC 15's x86-64 values (`target/interchange_floats.rs`), omitting the
types the target lacks, with `__FLT_EVAL_METHOD_TS_18661_3__` `0`, which
glibc's `<math.h>` prefers to `__FLT_EVAL_METHOD__`. A personality test
compares them with the host GCC's `-dM -E` (GCC 13 or later) when one is
installed. GCC defines no `__STDC_IEC_60559_TYPES__` for x86-64 and bcc
none either. The constant builtins glibc's GCC paths call
exist: `__builtin_huge_valfN`, `__builtin_inffN`, `__builtin_nanfN` and
`__builtin_nansfN` for `f32`, `f64`, `f32x` and `f64x` (the `f128` ones
already exist). The `nans` family, `__builtin_nans` included, gives a
signaling NaN of the result's format with the tag's payload, bit for bit as
GCC does; a conversion that cannot keep a NaN's payload makes it quiet. Without the switch these builtins are unknown to calls and
`__has_builtin`.

Verified against real glibc 2.43 headers (`<math.h>`, `<complex.h>`,
`<stdlib.h>`, `<float.h>`, with and without `_GNU_SOURCE`) under an emulated
GCC 14 identity (`-U__clang__ -U__GNUC__ -D__GNUC__=14 ...`) in C11, GNU17
and C23, and programs using `HUGE_VAL_F32`, `sqrtf32`, `strtof64` and the
builtins print what GCC 15's builds print (`signbit` returns another
nonzero value). Limits: the resource `<float.h>` is Clang's and knows
none of these types (no `FLT32_MAX`), so a GCC identity still needs GCC's
own `<float.h>` macros; the resource `<tgmath.h>` takes them with the
standard type's functions and result types (above), and refuses
`_Float128`; GNU vector element types, imaginary `_FloatN` constants such as
`1.0f32i`, `__builtin_*f128` math beyond the existing primitives, and
binary128 arithmetic in lowering remain unsupported.

## GNU vectors and x86 SIMD resources (2026-10-09)

GNU vector semantics and x86 resource coverage follow [GCC Vector Extensions](https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html) and [Clang Language Extensions](https://clang.llvm.org/docs/LanguageExtensions.html); see semantic-analysis.md for the supported instruction families and constant-evaluation boundaries.


### Common GCC scalar intrinsics (2026-10-12)

`GnuBuiltins` is a GNU-origin feature in the existing configuration vocabulary.
The semantic checker reports it for ordinary-call bit counts/swaps, expectation
and alignment hints, trap/unreachable/prefetch, constant/object-size queries,
overflow operations and stack allocation. `__has_builtin` recognizes the
implemented reserved names through the shared classifier; generic unsupported
builtin names remain false. `alloca` is also recognized in GNU mode, retaining
ordinary local/internal names. GNU syntax for case ranges, omitted conditional
operands and statement expressions continues to use its existing parser gates;
C2y case ranges retain their ISO gate. The lowering/backend contracts and
conservative query choices are documented in [middle-end.md](middle-end.md).

### PR #41 round-1 mode corrections (2026-10-13)

Plain `alloca` requires a GNU mode and an unshadowed implementation binding;
user declarations and definitions remain ordinary calls in every mode.
`__builtin_alloca` is available in strict modes when unshadowed. Freestanding
compilation keeps C89 implicit `int ()` for ordinary library spellings, while
explicit reserved library aliases retain their prototypes. The expression-only
varargs spellings are ordinary identifiers with binding-aware intrinsic
checking; `__builtin_va_arg` keeps its reserved type-operand syntax. GNU89
extern-inline mode follows the MSVC body's existing target rule unless an
explicit `gnu_inline` attribute requests GNU omission. Semantic diagnostics
now enforce the GCC prohibition on entering statement expressions while
preserving internal and outward jumps.

## 2026-10-13: GNU representations and initializer support

`packed`, `aligned`, scalar `mode`, union casts and range designators now have
semantic and lowering support under the existing GNU extension policy. `_Alignas`
has C11 constraints and completed-declarator storage alignment. Retained layouts
also support existing `typeof`, `__builtin_types_compatible_p`, `__builtin_offsetof`
and expression-alignment queries. `__builtin_signbit[f/l]`, `__builtin_isinf[f/l]`,
`__builtin_copysign[f/l]` and `__builtin_snprintf` are binding-aware builtins.
The type-generic `__builtin_isnan`, `__builtin_isfinite`, `__builtin_isnormal`,
`__builtin_isinf_sign` and `__builtin_fpclassify`, which glibc's `<math.h>`
selects for Clang, fold constants and lower to ordered comparisons.
`__builtin_complex`, which glibc's C11 `CMPLX` uses for Clang, takes a real and
an imaginary part of one real floating type, without promotion, and folds two
constants into a complex constant. It evaluates both operands once and lowers
supported F32/F64/F80 complex values through pair-valued expressions, component
lvalues, conversions, arithmetic, object initialization and ABI boundaries.
F32/F64 execute in the interpreter, LLVM and native backend; x87 complex
arithmetic uses LLVM and retains the existing native/interpreter F80 boundary.
Real and complex binary128 lowering remain explicitly unsupported. The
interpreter supports complex component extraction functions; full complex libm
coverage is not implied.
`__builtin_frame_address` and `__builtin_return_address` take a constant level
and lower level 0 (MinGW-w64's Clang `setjmp` passes the frame address);
`__builtin_extract_return_addr` is the identity.

GCC contracts: [common attributes](https://gcc.gnu.org/onlinedocs/gcc/Common-Attributes.html),
[union casts](https://gcc.gnu.org/onlinedocs/gcc/Cast-to-Union.html),
[designated initializers](https://gcc.gnu.org/onlinedocs/gcc/Designated-Inits.html),
and [other builtins](https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html).
ISO constraints and arithmetic/address constant classification remain those of
C99 §6.6 and §6.7.8; standard alignment follows C11 §6.7.5. GNU range expressions
execute once for the entire selected range. Vector execution, inline assembly
and nested functions remain outside this change; label relocations
arrived with labels as values below.

PR #42 round-1 repairs complete explicit alignment of packed System V
bit-fields, occupied-bit sizing of packed unions and lowering/static packing
of fields spanning more than their declared slot. Microsoft bit-field units
follow bundled Clang, including its MinGW/MSVC alignment difference.
GNU `aligned` uses semantic integer constant evaluation; `vector_size` retains
its token reducer. C11 §6.7.5p4,p6 combines multiple `_Alignas` requests before
checking natural alignment, including runtime-sized arrays and VLA typedefs.
See [the semantic repair record](semantic-analysis.md#pr-42-round-1-layout-and-alignment-repairs-2026-10-13)
and [GCC's packed/aligned sections](https://gcc.gnu.org/onlinedocs/gcc/Common-Attributes.html).

## GNU entity attributes and attribute status (2026-10-14)

Semantic analysis validates every attribute once and retains its meaning as
facts; lowering consumes the facts and never reads attribute tokens
([semantic-analysis.md](semantic-analysis.md#gnu-entity-attributes-and-cleanup-regions-2026-10-14)).
Each name in an attribute list, in any position (declarations, type names,
pointer levels, members, tags, enumerators and statements), is looked up once
in `semantic_analysis/attributes/catalog.rs`, under the spellings bundled
Clang knows for the target (`catalog/spellings.rs`, read from the pinned
Clang): GCC's attributes take `[[gnu::...]]` and Clang's own `[[clang::...]]`,
so `[[clang::weak]]` and `[[gnu::overloadable]]` are unknown;
GNU and `gnu::` `dllimport`, `dllexport` and `guard` exist only on the
Windows targets and `ifunc` only for glibc. Non-Windows
`__declspec(dllimport)` and `__declspec(dllexport)` warn under
`-Wignored-attributes`; their GNU spellings remain unknown, and attribute
availability queries remain zero. Unknown names warn as Clang's `-Wunknown-attributes`
("unknown attribute ignored"); a vendor prefix other than `gnu::` or
`clang::` is unknown, as in Clang. GCC attributes Clang lacks that bcc
accepts (`noipa`, `access`, `optimize` and others in the table) keep their
GNU and `gnu::` spellings, while `__has_attribute` answers Clang's 0 for
them. Unknown-attribute and dropped-attribute warnings follow
`-Wno-unknown-attributes` and `-Wno-ignored-attributes`, respectively, and
the matching `-Werror=` options. Warnings are withheld inside system headers,
so library headers stay quiet.

An attribute that a redeclaration adds after the entity's function body or
initialized definition is ignored with Clang's "attribute declaration must
precede definition" warning, unless a declaration up to the definition
already wrote it; type attributes (layouts, conventions, `noreturn`) still
apply, a late `alias` is the alias-definition error, and a tentative
definition completes only at the end of the unit. String arguments may carry
an encoding prefix, which Clang's `-Winvalid-unevaluated-string` warning
reports and ignores. `constructor()` takes the default priority, and too many
arguments are an argument error, not a syntax error.

Implemented meanings, with Clang's applicability diagnostics:

- `cleanup(f)` on an automatic variable calls `f(&var)` on every exit from its
  scope (fallthrough, `break`, `continue`, `return`, `goto`, the end of a
  statement expression), innermost variable first and interleaved with VLA
  storage release, after the return or tail value is computed. `f` must be a
  function taking one parameter that `&var` converts to; discarded qualifiers
  and `void *` are accepted, as Clang does. Elsewhere it warns as ignored. A
  goto or switch case that enters the variable's scope, and an indirect goto
  that enters or leaves it, are errors, as in Clang. `longjmp` does not run
  cleanups. For a returned structure, GCC's value semantics apply: Clang's
  named return value optimization lets a cleanup modify the returned copy,
  bcc's and GCC's results do not see that change.
- `constructor`/`destructor` with an optional integer constant priority
  (default 65535) register a function definition: `llvm.global_ctors` and
  `llvm.global_dtors` in the LLVM back end; the interpreter runs constructors
  before `main` in increasing priority (ties in definition order) and
  destructors after `main` returns or `exit`, in decreasing priority (ties in
  reverse order), not after `abort`. Priorities outside 0-65535 are errors and
  0-100 warn as reserved, as GCC does (bundled Clang accepts both silently).
  A constructor or destructor with parameters is refused.
- `alias("target")` makes a declaration another name for a definition of the
  same translation unit, resolved through alias chains (an LLVM global alias).
  Clang's errors apply: the target must be defined (by its emitted symbol name),
  an alias cannot be a definition (a non-`static` file-scope object without `extern`
  is a tentative definition), functions alias functions and objects objects,
  and cycles are rejected; differing function types warn
  (`-Wattribute-alias`). Overload aliases remain distinct and can be exported
  from Windows DLLs; target strings use the target's Itanium or Microsoft name.
  Repeated alias definitions diagnose. Block-scope aliases validate their
  arguments and participate in redefinition checks but neither emit an alias
  nor resolve its target, as in Clang.
  A target or alias with an assembler label is refused.
  A thread-local alias of a thread-local object is a `thread_local` LLVM
  alias, as Clang emits it; an alias whose storage duration differs from its
  target's, which Clang accepts and emits as an alias of the wrong storage,
  is refused.
- `weak` gives definitions weak linkage and declarations `extern_weak`
  linkage, so an undefined weak function or non-thread-local object has a
  null address on every target: bundled Clang emits the same LLVM linkage for COFF, where LLD
  resolves it as a weak external (checked by linking with Clang-compiled
  units on x86_64-w64-windows-gnu and, under WSL, x86_64-unknown-linux-gnu).
  Internal or no linkage is an error. Thread-local objects keep their thread
  storage: a weak definition is a weak `thread_local` global and a weak
  declaration an `extern_weak` one, as in Clang. Natively the address of an
  undefined weak thread-local object on ELF is computed from the thread's TLS
  block with offset zero, with Clang as with bcc. The interpreter models
  that shared nonnull thread base, so different unresolved weak TLS names
  have equal addresses, while ordinary undefined weak symbols remain null.
  The unresolved TLS address has no initialized object storage; a data
  access reports the missing definition. COFF cannot link an unresolved
  weak TLS address: LLD rejects its section-relative relocation to an
  absolute symbol. The bcc inliner never inlines a weak
  definition; the interpreter resolves a weak declaration to its default
  host function when the host provides one, else to null.
- `section("name")`, `used`, `visibility("default"|"hidden"|"internal"|
  "protected")` and `aligned(N)` on functions are emitted on the symbol;
  `used` is `llvm.compiler.used` on ELF and `llvm.used` on COFF, as Clang
  emits it. `section` and `alias` on an automatic variable, a typedef, a
  member or a parameter are errors, as in Clang; `used` there warns.
- `noinline` and `noipa` keep the bcc inliner from expanding a function and
  mark it `noinline` for LLVM (bcc has no other interprocedural pass, so
  `noipa` means the same; LLVM's own interprocedural passes do not see it,
  as Clang, which does not know `noipa`, does not mark it either).
  `always_inline` asks the inliner to expand every direct call it can copy,
  regardless of size, and LLVM to as well; an `always_inline` GNU or C99
  inline body that supplies no external definition becomes a private copy
  that direct calls reach.
- A goto in a cleanup variable's own initializer (a statement expression)
  may jump to a label in the variable's scope, as Clang allows; the cleanup
  then runs when the scope ends.
  Visibility on an entity with internal linkage is accepted silently and has
  no effect, as in Clang.

| Status | Attributes |
| --- | --- |
| Implemented (entity) | `noinline`, `noipa`, `always_inline`, diagnostic `nonnull`, `alias`, `cleanup`, `constructor`, `destructor`, `section` (and MSVC `allocate`), `used`, `visibility`, `weak`, `aligned` on functions, `selectany`, `returns_twice` (the same retained fact the external-symbol catalog gives `setjmp`, `_setjmp`, `__sigsetjmp`, `vfork` and their MinGW-w64 spellings; the native back end refuses calls to such functions) |
| Implemented (call diagnostics) | GNU `error("message")`, `warning("message")`, including `gnu::` spelling; emitted direct calls after bcc and LLVM IR optimization, with `-Wattribute-warning` control |
| Implemented (call contracts) | `overloadable`, `pass_object_size`, `pass_dynamic_object_size`, `diagnose_if`, `enable_if`, `unavailable`; unsupported predicate shapes diagnose and unmodeled object-size origins remain unknown |
| Implemented (representation) | `aligned`, `packed`, scalar `mode`, `vector_size`, `transparent_union`; `gnu_inline`; `ms_struct` on Windows targets, `gcc_struct` on Linux targets |
| Calling conventions | `cdecl`, `regparm`: the x86-64 C convention. `stdcall`, `fastcall`, `thiscall`: accepted on Windows, warning elsewhere; `pascal`: warning everywhere (as Clang). `ms_abi` on Windows and `sysv_abi` on Linux are the default; the opposite one, `vectorcall` (and the `__vectorcall` keyword), `regcall`, `preserve_*`, `swift*`, `intel_ocl_bicc`, `interrupt`, `no_caller_saved_registers` and the other non-x86 conventions are refused |
| Refused by lowering (dropping would change behavior) | `ifunc`, `weakref`, `naked`, `cpu_dispatch`, `cpu_specific`, `target_version`, `address_space`, Swift ABI parameter attributes, `internal_linkage`, the foreign record layout (`gcc_struct` on Windows, `ms_struct` on Linux) |
| Dropped with a warning | `common` (bcc emits ordinary definitions), `target`, `target_clones` (the default version runs), `weak_import` (still weak where the dead-code model folds), `copy` |
| Supported | `dllimport` and `dllexport`: Clang's diagnostics, `__imp_name` references, export directives, and inline-body retention with COMDAT linkage |
| Accepted, nothing to honor | hints, diagnostics and optimization promises: `access`, `alloc_align`, `alloc_size`, `artificial`, `assume`, `assume_aligned`, `cold`, `const`, `counted_by`, `deprecated`, `designated_init`, `fallthrough`, `flag_enum`, `flatten`, `format`, `format_arg`, `hot`, `leaf`, `malloc`, `maybe_unused`, `may_alias` (no type-based alias analysis), `musttail`, `no_*` instrumentation and sanitizer controls, `noclone`, `nocommon`, `nonstring`, `noreturn` (its only effect is on which code counts as emitted; see `semantic-analysis.md`), `nothrow`, `optimize`, `pure`, `retain`, `returns_nonnull`, `sentinel`, `simd`, `tls_model`, `unused`, `warn_unused_result`, the thread-safety capability attributes, and the C23 standard attributes; see the catalog for the full list |
| Unknown (warning, as in bundled Clang) | every other name, including GCC-only `noinit`, `symver`, `persistent` and `scalar_storage_order` (ignored in either byte order, as bundled Clang does), MSVC `restrict`, `uuid`, `allocate` and `align` through MinGW's `__declspec` macro, a name in the other vendor scope, and target-specific names on other targets |

GNU `error` and `warning` require one unevaluated string argument and apply
only to functions. The first declaration supplies the contract; unannotated
redeclarations inherit it, conflicting kinds are errors, and a changed message
warns while retaining the first. Native code generation and interpretation
diagnose evaluated identifier calls (including parenthesized identifiers) that
remain after the selected bcc passes and, for LLVM object/executable output,
the actual LLVM IR pass pipeline. Diagnostics are reported before object
emission. Address references, pointer calls, unevaluated
operands, unreachable blocks and discarded internal bodies do not diagnose.
Syntax/semantic-only and IR-only output modes defer these code-generation
checks. `warning` is controlled by `-Wattribute-warning`, including per-group
error promotion; `error` remains an error. These contracts are distinct from
`diagnose_if`, which is checked during semantic analysis. LLVM source-marker
retention follows pinned Clang, including suppression when LLVM merges distinct
call sites and discards their source markers.

`__declspec` modifiers use the names Clang supports there: `align(N)` is
Clang's `aligned` attribute (it sets an object's or typedef's alignment, also
below the natural one and replacing the alignment a typedef carries, raises a
record's and cannot lower a member's; after a record body, directly or behind
other specifiers as in `} const __declspec(align(1)) name`, every
`__declspec` is a declaration specifier, so it describes the declared object
and not the record), `allocate` is `section` (also on function-local statics),
`selectany` makes an external data definition `weak_odr` in a comdat of its
own name, as Clang emits it, so units that each define it link to one object
(on a function or data without external linkage it is Clang's error; the
native back end emits it in a COMDAT section; a `selectany` declaration without an initializer,
`extern` included, defines its object zero-initialized, as Clang does, so a
later initialized declaration is a redefinition and the type must be
complete), `thread` is thread storage
duration (also in the GNU spelling MinGW's `__declspec` macro produces; see
the thread-local storage notes), `naked` is refused, `noinline` is the GNU
attribute, and `dllimport` and `dllexport` are accepted on Windows and
warn as ignored on other targets. Their applicability and redeclaration
rules follow Clang's: a redeclaration adding one warns, or is an error after
an evaluated use (a constant `sizeof` operand is no use), and one dropping
`dllimport` drops it. Code generation exports what the declarations end
exporting (an `/EXPORT` or `-export` linker directive) and imports, through
`__imp_name`, what the unit's last evaluated reference in emitted code named
imported, or, when no emitted code references it, what any evaluated reference
in a body lowering emits named imported: lowering still emits some code Clang
skips, such as the code after a call that does not return, and a reference
there links only through the import it was written with. `dllexport` also retains MSVC inline
bodies as described above.
`noreturn` ends the code Clang emits after a call in an expression statement,
which decides that last reference (`semantic-analysis.md`); `nothrow`,
`deprecated`, `restrict`, `noalias`, `novtable`, `uuid` and similar
declaration annotations are accepted. Space-separated modifiers, as in
`__declspec(selectany allocate("s"))`, are one list. Other names, such as the
GNU `__declspec(weak)`, warn as Clang's unsupported `__declspec` attributes.

## Windows headers, intrinsics and macro rescanning (2026-10-16)

Checked against MinGW-w64 13.0.0 and Microsoft's published `win32metadata`
headers for `x86_64-w64-windows-gnu` (C99, cross-target; no Windows program
was run):

- A UTF-8 byte-order mark that begins a source file is skipped, as Clang and
  GCC do (Microsoft's `icu.h`); one elsewhere is a stray character.
- A forward reference to an enum tag, as in WinRT's `typedef enum E E;`
  before `enum E { ... }`, is accepted in every mode as GCC and Clang accept
  it, reported only under `-pedantic` as a GNU extension; the type is
  incomplete until its list.
- When a function-like macro call reads past the end of the replacement
  that supplied its name, that replacement's macro may expand again inside
  the call's replacement, as GCC and Clang choose (C11 §6.10.3.4p4 allows
  either); `TraceLoggingProvider.h` counts arguments that way.
- `__rdtsc` is Clang's x86 builtin in every target and mode
  (`unsigned long long __rdtsc(void)`, `llvm.x86.rdtsc`); the interpreter
  gives an increasing counter and the native back end refuses it.
  `__debugbreak` is the Microsoft breakpoint with Microsoft extensions
  (`llvm.debugtrap`, `int3`); the interpreter ends the program as an
  undebugged breakpoint does.
  `__annotation(L"...", ...)`, which `TraceLoggingProvider.h` writes, is
  Microsoft's debug-information annotation with Microsoft extensions: its
  arguments must be wide string literals and it does not execute (bcc emits
  no CodeView annotation).

## GNU labels as values (2026-10-14)

`&&label` and `goto *expression`
([GCC Labels as Values](https://gcc.gnu.org/onlinedocs/gcc/Labels-as-Values.html))
keep their parser-owned `LabelsAsValues` diagnostics: silent in GNU dialects,
`'label address'` and `'computed goto'` GNU-origin extension reports under
Warn/Deny in ISO modes, as Clang's `-Wgnu-label-as-value`. Semantic analysis
and lowering then follow Clang: `&&label` is a `void *` address constant of
the enclosing function, `goto *e` converts `e` to `const void *` by simple
assignment, and a computed goto is an error when its function takes no label
address, when the operand is not convertible, or when one of its possible
targets lies inside a statement expression or the scope of a variably
modified identifier that the goto is outside of. `&&label` outside a function
is an error. `&&a - &&b` for labels of one function is a link-time constant
in static initializers, converted to an integer of 32 to 64 bits that is no
wider than the value it converts. Clang rejects other widths (`_Bool`, `short`,
`__int128`), widening a narrowed difference, bit-field destinations and
arithmetic on the difference, and so does bcc. GNU `__label__` local labels now lower as ordinary labels.

`VoidPointerArithmetic` is a new GNU-origin feature, accepted in every mode
([GCC Pointer Arith](https://gcc.gnu.org/onlinedocs/gcc/Pointer-Arith.html)):
addition, subtraction, increments, compound assignment and subscripts on
`void *` use an element size of 1, also in address constants and constant
differences, and report `'arithmetic on a pointer to void'` or
`'subscript of a pointer to void'` under Warn/Deny, like Clang's
`-Wgnu-pointer-arith`. `p[i]` is a `void` rvalue that is never read; `&p[i]`
is its address. `sizeof` of `void` is 1 in every mode (`'sizeof of void'`
under Warn/Deny, as Clang's `-Wpointer-arith`); `sizeof` of a function type
stays GNU-only. Function-pointer arithmetic is not accepted.

## Pointer diagnostic defaults (2026-10-14, issue #46)

The oracle is bundled Clang **23.1.1**, Rust LLVM revision
`1b9c0d5ff9bbe7634aead059efe6b11a7eeba145`, with `-fsyntax-only` and no warning flags.
`python scripts/probe_pointer_diagnostics.py` retains source, argv, status and
complete stderr in `target/pointer-diagnostics/matrix.json`. All 25 probes
have identical default severity, group and wording on Linux GNU, Windows GNU
and Windows MSVC, in `c89`, `gnu89`, `iso9899:199409`, `c99`, `gnu99`, `c11`,
`gnu11`, `c17`, `gnu17`, `c23`, `gnu23`, `c2y`, `gnu2y` (975 invocations).

| Probe | Clang group | Default | Exact Clang wording |
| --- | --- | --- | --- |
| distinct-equality | `compare-distinct-pointer-types` | **warning** | `comparison of distinct pointer types ('int *' and 'unsigned int *')` |
| distinct-order | `compare-distinct-pointer-types` | **warning** | `comparison of distinct pointer types ('int *' and 'float *')` |
| object-assignment | `incompatible-pointer-types` | **error** | `incompatible pointer types assigning to 'char *' from 'float *'` |
| object-initialization | `incompatible-pointer-types` | **error** | `incompatible pointer types initializing 'char *' with an expression of type 'float *'` |
| object-argument | `incompatible-pointer-types` | **error** | `incompatible pointer types passing 'float *' to parameter of type 'char *'` |
| object-return | `incompatible-pointer-types` | **error** | `incompatible pointer types returning 'float *' from a function with result type 'char *'` |
| pointer-sign | `pointer-sign` | **warning** | `assigning to 'int *' from 'unsigned int *' converts between pointers to integer types with different sign` |
| char-sign | `pointer-sign` | **warning** | `assigning to 'char *' from 'signed char *' converts between pointers to integer types where one is of the unique plain 'char' type and the other is not` |
| nested-sign | `incompatible-pointer-types` | **error** | `incompatible pointer types assigning to 'int **' from 'unsigned int **'` |
| discard-qualifiers | `incompatible-pointer-types-discards-qualifiers` | **warning** | `assigning to 'int *' from 'const int *' discards qualifiers` |
| nested-qualifiers | `incompatible-pointer-types-discards-qualifiers` | **warning** | `assigning to 'const int **' from 'int **' discards qualifiers in nested pointer types` |
| nested-discard-qualifiers | `incompatible-pointer-types-discards-qualifiers` | **warning** | `assigning to 'int **' from 'const int **' discards qualifiers in nested pointer types` |
| pointer-qualifiers | `incompatible-pointer-types-discards-qualifiers` | **warning** | `assigning to 'int **' from 'int *const *' discards qualifiers` |
| function-assignment | `incompatible-function-pointer-types` | **error** | `incompatible function pointer types assigning to 'int (*)(int)' from 'int (*)(double)'` |
| function-object | `incompatible-pointer-types` | **error** | `incompatible pointer types assigning to 'int (*)(int)' from 'int *'` |
| function-void | `pedantic when requested` | **silent** | `No default diagnostic` |
| integer-pointer | `int-conversion` | **error** | `incompatible integer to pointer conversion assigning to 'int *' from 'int'; take the address with &` |
| pointer-integer | `int-conversion` | **error** | `incompatible pointer to integer conversion assigning to 'int' from 'int *'; dereference with *` |
| pointer-integer-equality | `pointer-integer-compare` | **warning** | `comparison between pointer and integer ('int *' and 'int')` |
| pointer-integer-order | `no named group` | **warning** | `ordered comparison between pointer and integer ('int *' and 'int')` |
| pointer-zero-order | `pedantic when requested` | **silent** | `No default diagnostic` |
| function-order | `ordered-compare-function-pointers` | **warning** | `ordered comparison of function pointers ('int (*)(int)' and 'int (*)(int)')` |
| function-distinct-order | `ordered-compare-function-pointers`, then `compare-distinct-pointer-types` | **two warnings** | `ordered comparison of function pointers ('int (*)(int)' and 'int (*)(double)')`; `comparison of distinct pointer types ('int (*)(int)' and 'int (*)(double)')` |
| function-equality | `compare-distinct-pointer-types` | **warning** | `comparison of distinct pointer types ('int (*)(int)' and 'int (*)(double)')` |
| void-order | `compare-distinct-pointer-types` | **warning** | `comparison of distinct pointer types ('int *' and 'void *')` |

`-Wdiscarded-qualifiers` is a GCC spelling, not a Clang or bcc alias. bcc
warns about unsupported warning groups and continues, as Clang does.
Use `-Wincompatible-pointer-types-discards-qualifiers` instead.
Clang warns about the option itself with
`unknown warning option '-Wdiscarded-qualifiers'; did you mean '-Wignored-qualifiers'?`
in `[-Wunknown-warning-option]`; it does not select a pointer diagnostic group.

For the eight named pointer constraint groups above (excluding `pedantic`),
bcc supports `-WGROUP`, `-Wno-GROUP`,
`-Werror=GROUP` and `-Wno-error=GROUP`. Global `-Werror` promotes warnings
from every translation phase; `-Wno-error` restores their defaults. Specific
group severity overrides win over global `-Werror` regardless of order.
Repeated options for a group use the last setting. `-Werror=GROUP` enables
the group; `-Wno-error=GROUP` does not enable a disabled group. The
`incompatible-pointer-types` parent controls qualifier and function-pointer
children at its argv position; subsequent child options override it.
Disabling and re-enabling a default-error group makes it a warning, subject to
global `-Werror`, matching Clang; it also clears an explicit group error
promotion while preserving an explicit group demotion.
`pointer-sign` is independent. `-w` suppresses warning groups even with
`-Werror`; default errors remain errors unless suppressed or demoted.
Ordering distinct function pointer types emits both applicable comparison
groups; each group's options control its diagnostic independently.

`-pedantic-errors` retains Deny for language extensions and promotes the
pointer constraint warnings, following Clang. Explicit group options can
suppress or demote those constraint diagnostics. `-pedantic` leaves their
defaults unchanged. Warnings retain ordinary pointer conversion facts and
permit lowering; errors block it. Atomic builtin value operands use the
assignment groups; other intrinsic-specific contracts remain strict.
The `float *` to `char *` argument execution regression explicitly uses
`-Wno-error=incompatible-pointer-types`; accepting it by default would
contradict the observed Clang behavior. Function/void pointer conversions
and pointer/zero ordering use the existing GNU extension policy adapter.

Every mode, the inspection views included, returns failure after an error
diagnostic, as a compiler must; code-generation modes (`--emit=ir`,
`--interpret`, `-c`, `-o`) also stop before lowering, and flag regressions
verify that gate.

These defaults satisfy the diagnostic requirement of C99 §5.1.1.3p1
(printed p. 11; PDF p. 23). The constraints remain §6.5.16.1p1
(p. 92; PDF p. 104), §6.5.9p2 (p. 86; PDF p. 98), §6.5.8p2
(p. 85; PDF p. 97), §6.5.2.2p2/p7 (pp. 71-72; PDF pp. 83-84),
§6.8.6.4p3 (p. 139; PDF p. 151), and §6.7.8p11 (p. 126; PDF p. 138).

### PR #44 review fixes (2026-10-14)

Each rule below was checked against the same bundled Clang on
`x86_64-unknown-linux-gnu`, `x86_64-w64-windows-gnu` and
`x86_64-pc-windows-msvc` in `c89`, `c99`, `gnu17` and `c23`, with
`target/review/cmp.py`; the shared classifier in
`semantic_analysis/expressions/pointers.rs` and the group table in
`configuration/diagnostics.rs` own them.

| Construct | Clang group | Default | Result or decision |
| --- | --- | --- | --- |
| `c ? ip : up`, `c ? f1 : f2`, `c ? f1 : vp`, `c ? pa : cpa` | `pointer-type-mismatch` | **warning** | Unqualified `void *`. Clang 23 does not merge the targets' qualifiers here (`_Generic(c ? cip : vup, void *: 1)` selects `void *`), so neither does bcc. |
| `c ? ip : l`, `c ? 1 : f1` | `conditional-type-mismatch` | **warning** | The pointer arm's type; the integer converts to it, and a constant integer arm stays an address constant. |
| `c ? ip : cip`, `c ? vp : cip` | none | silent | Composite or `void` target with the union of qualifiers (C99 §6.5.15p6). |
| `cpa = pa`, `f(a)` for `const double m[][3]`, `[n]` and `(*)[]` parameters, `cpa == pa` | none | silent | An array target's element qualifiers count as the array's own when pointer targets are compared for qualifier inclusion and compatibility (C23 §6.7.4.1p10, which Clang applies in every mode). The conditional operator keeps them part of the target, as Clang does. |
| `pa = cpa` | `incompatible-pointer-types-discards-qualifiers` | **warning** | Only dropping a qualifier warns. |
| `const int (**)[]` from `int (**)[3]` | `incompatible-pointer-types` | **error** | Below the first pointer level Clang requires identical array derivations, so the nested-qualifier walk keeps exact bounds. |
| `unsigned *` from `enum N *` (`N` has a negative enumerator) | `pointer-sign` | **warning** | Clang compares targets after a character type becomes `unsigned char` and a signed integer type, or an enumeration with a signed compatible type, becomes its unsigned counterpart. An enumeration with an unsigned compatible type stays itself, so `int *` from `enum E *` is an error on Linux, while MSVC (enums are `int`) makes `unsigned *` from `enum E *` a `pointer-sign` warning. |
| `__atomic_store_n(&ip, up, 0)`, `__atomic_exchange_n`, `__sync_val_compare_and_swap`, `__c11_atomic_store` | the assignment groups | as assignment | Atomic builtin value operands convert through the assignment adapter; address and buffer contracts stay builtin errors. |
| `long s = &x;` with int-conversion demoted | `int-conversion` | as configured | An address constant in an integer type exactly as wide as a pointer (also through an explicit cast) is a static initializer and lowers to a relocation; narrower types, including MSVC's 32-bit `long`, report that the initializer is not constant, as Clang does. |

`-Wpointer-type-mismatch` and `-Wconditional-type-mismatch` accept the same
option forms as the other groups, are promoted by `-Werror` and
`-pedantic-errors` and are withheld by `-w`. `DiagnosticGroup::ALL`, `COUNT`,
the names and `parse` come from one `diagnostic_groups!` table, which also
records each group's default and whether `-pedantic-errors` promotes it.

The system-header rule uses the expansion location, so a system macro
expanded in user code reports its pointer diagnostics at the user's
invocation, and every warning group, including the three that default to
errors, is withheld inside a system header at any severity
(`sema-pointer-system-header`). Clang 23 also reports a `long long` that a
system macro supplies to C89 user code at the invocation, which the
`pp-system-header-diagnostics` snapshot now shows.

Warnings that Clang does not report by default are off by default in bcc as
well, so `-Werror` rejects nothing that Clang accepts. Each has a group that
enables it, `-Werror=GROUP` included, and a system header withholds it:

| Diagnostic | Group | Clang |
| --- | --- | --- |
| Missing or escaped final newline | `newline-eof` | Off; Clang has no escaped-newline warning. |
| Identifier evaluated as 0 in `#if` | `undef` | Off. |
| Unknown `#pragma` | `unknown-pragmas` | Off (an unknown `STDC` pragma keeps its own diagnostic). |
| Unused undefined `static` function | `unused-function` | Off. |
| Constant that takes a wider type of its list | `constant-promotion` (bcc only) | Not diagnosed. |

`__VA_ARGS__` outside a variadic replacement list, `#line 0` and a `#line`
number from 2147483648 to 4294967295 are reported only under `-pedantic`, as
Clang reports them. The line number applies, as in Clang, and a later line
past 4294967295 wraps to 0; a larger `#line` number is an error and is
ignored (C99 §6.10.4p3, p. 158; PDF p. 170, leaves these values undefined).


### Diagnostic parity follow-up (issues #49 and #54)

C23 `nullptr` has its own retained scalar type, with pointer size and alignment.
It converts to pointers and Boolean values; arithmetic conversions and ordered
comparisons reject it. `typeof(nullptr)` preserves the type. Implicit Boolean
conversion warns in `-Wnull-conversion`. GNU subtraction and arithmetic of
function pointers use a byte stride, as GNU `void *` arithmetic already does.

Default warning groups now include `non-literal-null-conversion`,
`pointer-compare`, `pointer-bool-conversion`, `pointer-to-int-cast`,
`void-ptr-dereference`, and `nonnull`. Visible K&R definitions diagnose argument
pointer signedness while calls keep default argument promotions. GNU `nonnull`
annotations retain their checked parameter positions over redeclarations.
All these groups support the existing enable, suppress and error controls.
`-Wall`, `-Wextra`, and `-Wconversion` are accepted; unsupported `-W` spellings
warn through `unknown-warning-option`, which can be suppressed or promoted.
These umbrella options select supported warning groups, not a claim to implement
every Clang warning under each umbrella.

Unknown STDC pragmas warn in `unknown-pragmas`; directives written inside macro
arguments execute before prescan, as in Clang and GCC. Pedantic diagnostics
render `[-Wpedantic]`, predefined identifier warnings render their Clang group,
and assignment/initializer diagnostics identify the operator/declarator.
Semantic and parser primary locations use macro invocation locations while
preserving physical locations for tokens written as macro arguments.
`__PRETTY_FUNCTION__` resolves typedef spellings at the declaration scope, so
an intervening body declaration cannot change a function's result spelling.

The #49 static function-designator-to-`void *` initializer remains accepted as
a GNU extension and is diagnosed only under pedantic policy. Regression tests
replay its original reproducer, retain a function relocation in the static table,
and call through the stored address in interpreter and native execution.
`tests/integration/diagnostic_parity.rs` compares constraints, warnings, modes
and locations with the pinned Clang; semantic and pipeline tests cover the retained facts.

## Folded pointer constants (issues #83 / #88, 2026-10-19)

Absolute pointer arithmetic retains its byte value for static initializers,
including zero-valued BSS data without relocations. Pointer-to-integer casts
and address differences remain non-ICEs. As in Clang 23.1.1, file-scope, static
local and member array bounds fold with `-Wgnu-folding-constant`; automatic
arrays and abstract type names remain variably modified. Cases accept the
folded values, with the same warning group enabled explicitly or by pedantic.
Addition to a null pointer uses `-Wgnu-null-pointer-arithmetic`, off by default
and enabled by pedantic. Both groups support suppression and error promotion.
