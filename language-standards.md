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
- Integer constants are limited to 64-bit magnitudes, except `wb` and `uwb`
  constants, which reach `BITINT_MAXWIDTH`; larger magnitudes are diagnosed.
  Preprocessing expressions use a 64-bit intmax/uintmax model.
- C2y is the subset listed under [Decisions](#decisions), not every current or
  future draft change.
- The unified extension policy reports some reserved MSVC constructs that
  Clang exempts from pedantic diagnostics; the reserved GNU spellings follow
  the claimed compiler (below). Only `-pedantic`,
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
`__NO_MATH_ERRNO__` there, `-fsignaling-nans` defines `__SUPPORT_SNAN__`
(`CompilerConfiguration::with_signaling_nans`), and `-fno-fast-math` and
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
this identity they reach the checked functions. Its `math.h` also defines
`isgreater`, `isless`, `isunordered` and their siblings (C99 §7.12.14) as
`__builtin_isgreater` and the like, and `issignaling` and C23's `roundeven`
as `__builtin_issignaling` and `__builtin_roundeven`; bcc provides those
builtins under every identity, and GCC's `__builtin_iseqsig` under the GCC
identity only (`Feature::GccOnlyBuiltins`; bcc does not raise its "invalid"
exception for a NaN operand). The comparisons take two real operands, at
least one floating, in their common real type, as GCC and Clang check them,
fold constant operands, and raise no "invalid" for a quiet NaN, binary128
included, as GCC compares it (Clang 23 calls libgcc's signaling
`__lttf2` there). `__builtin_issignaling` folds a constant from its value
before any rounding, so `__builtin_nans("")` and its negation are
signaling. For an x87 `long double`, the encodings the format leaves
unsupported (pseudo-NaN, pseudo-infinity, unnormal) are signaling except
under the Clang identities, as GCC and Clang classify them. `msvc`
(`cl.exe`) removes the GNU and Clang macros and the inline-semantics macro,
and defines `_MT` and
`_MSVC_TRADITIONAL` `0` (bcc's preprocessor is conforming, like `cl
/Zc:preprocessor`); `_MSC_VER` still needs `-fms-extensions` on the MSVC
target, which the `cl` personality selects. `clang-cl` removes the GNU macros
and keeps Clang's. `tcc` removes the GNU and Clang macros, the `__int128`
and `__float128` size macros, `linux` and `__NO_INLINE__`, and defines
`__TINYC__` `927` (TCC 0.9.27) and glibc's `__REDIRECT` family, as TCC does. Every identity keeps `__bcc__`, the ISO
macros and the target's definitions, as clang-cl keeps `__x86_64__`.

Several options change rules the mode otherwise fixes. `-ftrigraphs` and
`-fno-trigraphs` (also GCC's `-trigraphs`) turn phase 1 trigraph replacement
on or off whatever the mode; the `cl` personality turns it off, as `cl`
replaces trigraphs only under `/Zc:trigraphs`. `-fwrapv` makes signed integer
`+`, `-`, `*`, negation and `++`/`--` wrap in two's complement, so lowering
marks none of them `nsw`; pointer arithmetic is unaffected, as in GCC.
`-ftrapv` instead traps when one of them overflows in `int` or a wider
type, or when the library's `abs`, `labs` or `llabs` (a builtin) gets the
most negative value, and the last of the two options given decides under
the GCC identity, `-ftrapv` under Clang's
(`CompilerConfiguration::with_trapv`). A
signed `<<` wraps in every mode: GCC documents that it does not treat its
overflow as undefined, and Clang emits a plain shift. Constant expressions
fold it to the same two's-complement value (`static int s = -1 << 4;` is
-16, `3 << 31` is `INT_MIN`), reported as a GNU extension only under the
pedantic policy, as GCC accepts them; Clang's default `-Wshift-negative-value`
and `-Wshift-overflow` warnings are not reproduced, and such a shift stays an
integer constant expression, where GCC treats an array bound of it as
variable. `-fpack-struct=N` (1, 2, 4, 8 or 16) sets the packing in effect
before any `#pragma pack`, which `#pragma pack()` restores, as GCC documents
and `cl /ZpN` does. `-fpack-struct` without a value instead packs every
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

All twelve public headers are discoverable in all language modes, and each behaves like
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
| `stdarg.h` | `va_list` and the four `va_*` macros in every mode, plus GNU `__gnuc_va_list` and `__va_copy`. After C17 `va_start` takes only the list and leaves any further arguments unevaluated (C23 §7.16.1.4), as GCC 15's and Clang's headers define it; under the Clang identities a third argument is an error, as Clang's builtin takes at most two. Like Clang's, it may be included repeatedly; `__need___va_list`, `__need_va_list`, `__need_va_arg`, `__need___va_copy` and `__need_va_copy` request one part, and each part keeps its conventional guard (`__GNUC_VA_LIST`, `_VA_LIST`). The intrinsic type is an array of one opaque 24-byte, 8-aligned SysV record on Linux, and `char *` on Windows. |
| `stdbool.h` | `__bool_true_false_are_defined`; `bool`, `true`, `false` macros before C23. C23 uses language keywords. |
| `stddef.h` | `size_t`, `ptrdiff_t`, `wchar_t`, `NULL`, `offsetof`; C11 adds `max_align_t`, and `__STDC_WANT_LIB_EXT1__` adds `rsize_t`. Like Clang's, it may be included repeatedly: `__need_size_t`, `__need_ptrdiff_t`, `__need_wchar_t`, `__need_NULL`, `__need_wint_t`, `__need_rsize_t`, `__need_max_align_t` and `__need_offsetof` request one part, as glibc's headers do. Each type keeps its conventional guard (`_SIZE_T`, `_PTRDIFF_T`, `_WCHAR_T`, `_WINT_T`, `_RSIZE_T`), so a definition the C library made first is kept; a requested `NULL` is always restored to `((void *)0)`. As in Clang, `_MSC_EXTENSIONS` also defines vcruntime.h's `_WCHAR_T_DEFINED` guard with `wchar_t`, and `max_align_t` is `double` on the MSVC target (`_M_X64` without `__MINGW32__`), otherwise the GCC-style record, independently of MSVC extension flags. |
| `stdint.h` | All 8/16/32/64 exact, least and fast types, pointer and maximum types, corresponding limits and constant macros; `SIG_ATOMIC`, `SIZE`, `PTRDIFF`, `WCHAR`, `WINT` limits. Hosted, a C library `<stdint.h>` replaces all of these, as with Clang. |
| `stdalign.h` | Like Clang: `alignas`, `alignof`, and the two indicator macros when `__STDC_VERSION__` exists and precedes C23; empty in C89 and C23/C2y. |
| `stdatomic.h` | C11 atomic typedefs, `memory_order`, `atomic_flag`, initialization, fences, lock-free queries and all generic operation macros. Hosted, it defers to a following system header except with `_MSC_VER` in C mode, matching the pinned Clang resource header. C17 marks `ATOMIC_VAR_INIT` deprecated through `#pragma clang deprecated`; `_CLANG_DISABLE_CRT_DEPRECATION_WARNINGS` suppresses that marker. C23 removes `ATOMIC_VAR_INIT` and adds `atomic_char8_t` and `ATOMIC_CHAR8_T_LOCK_FREE`. No `__STDC_NO_ATOMICS__` is defined or interpreted, as in Clang. |
| `stdbit.h` | C23's `<stdbit.h>` (§7.18): `__STDC_VERSION_STDBIT_H__` (`202311L`), `__STDC_ENDIAN_LITTLE__` (1234), `__STDC_ENDIAN_BIG__` (4321), `__STDC_ENDIAN_NATIVE__` (little), the 70 `stdc_*_u{c,s,i,l,ll}` functions and the 14 type-generic macros. Wherever the C library has the header (glibc 2.39 and later, whose macros use the `__builtin_stdc_*` builtins when the compiler has them) it defers to it, freestanding too, as GCC and Clang find it there. Otherwise it declares the functions with external linkage and defines each as a macro over its builtin, converting the argument as the prototype does; the same text is the `<stdbit.h>` of bcc's WASI C library, which defines the functions (`backend/wasm/runtime/stdbit.c`). Elsewhere a call that suppresses a macro needs a C library that defines them. Neither GCC nor Clang ships the header, so for the Clang and clang-cl identities the resource directory has none (`headers::provides`): without a C library's (`-nostdlibinc`), `#include <stdbit.h>` finds no header, as in Clang. |
| `stdnoreturn.h` | Like Clang: `noreturn` and `__noreturn_is_defined` in every mode, retained in C23 despite deprecation. |
| `mm_malloc.h` | Not ISO C; GCC and Clang ship it, and MinGW-w64's `<malloc.h>` includes it. Like Clang's, it includes `<stdlib.h>` and defines `static __inline__` `_mm_malloc` and `_mm_free`: through `__mingw_aligned_malloc` for MinGW-w64, `_aligned_malloc` for the MSVC runtime (from its `<malloc.h>`, unless that defines `_mm_malloc` as a macro), and `posix_memalign`, which it declares, elsewhere. An alignment of 1 uses `malloc`, and a smaller power of two is raised to a pointer's alignment. The x86 intrinsic headers (`x86intrin.h`, `emmintrin.h`, `immintrin.h`, `cpuid.h` and their family) are provided; MMX through SSE4.2 compile and run (see [x86 intrinsic headers](#x86-intrinsic-headers-2026-10-30)). |

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
| UnicodeLiteralPrefixes | - | - | - | Y | Y | Y | Y | native; GNU C99 under the GCC identity | lexer/preprocessor |
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
| Float16 | - | - | - | - | - | Y | Y | `gcc` and `clang` identities | lexer/parser/semantic analysis |
| BFloat16 | - | - | - | - | - | - | - | `gcc` and `clang` identities (GNU native) | lexer/parser/semantic analysis |
| Fp16 | - | - | - | - | - | - | - | `clang` identities (GNU native) | parser/semantic analysis |
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
| VectorBuiltins | - | - | - | - | - | - | - | extension (GNU native) | shuffle/convert/bitcast and x86 signatures; the shuffles, `__builtin_convertvector` and `__builtin_bit_cast` lower |
| BuiltinChooseExpr | - | - | - | - | - | - | - | extension (GNU native) | parser/sema |
| LabelsAsValues | - | - | - | - | - | - | - | extension (GNU native) | parser, sema, lowering |
| LocalLabels | - | - | - | - | - | - | - | extension (GNU native) | parser, sema, lowering |
| OmittedConditionalOperand | - | - | - | - | - | - | - | extension (GNU native) | parser |
| OneVoidConditional | - | - | - | - | - | - | - | extension (GNU native) | semantic analysis |
| ConstantCompoundLiteral | - | - | - | - | - | - | - | extension (GNU native) | sema, lowering |
| ArrayCompoundLiteral | - | - | - | - | - | - | - | extension (GNU native) | sema, lowering |
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
- `ArrayCompoundLiteral` has GNU origin: an array initialized by a compound
  literal of its type, as "Arrays initialized by array compound literals"
  below describes. Only the Clang identity reports it under `-pedantic`;
  GCC reports the static object's `ConstantCompoundLiteral` instead.
- `ConstantCompoundLiteral` has GNU origin. A compound literal of a static
  object's own type as its whole initializer is accepted as if its braced
  list were written in its place, when every value the literal stores is a
  constant its destination accepts: a structure or union, or a scalar of a
  compatible type, qualifiers aside (`static int x = (int){3};`). As GCC's
  folder removes them, the literal may sit inside parentheses,
  `__extension__`, `*&` and the operand `_Generic` or
  `__builtin_choose_expr` selects and, outside braces, inside an outermost
  cast to the object's own type, casts within it through types that keep
  every value of that type (`(double)((double){0x1p20})`,
  `(int)(long)(unsigned)(int){3}`), and the operand a `?:` of the object's
  type with a constant condition selects. GCC never folds away the
  implicit conversion to the object's type, so `static int x =
  (unsigned)(int){3};` and `static float f = (double)(float){1};` stay
  non-constant. A cast takes its type's main variant (GCC's
  `build_c_cast`), and a pair of conversions folds away only to an operand
  of exactly that type, so inside a cast or `?:` the literal must have
  exactly the outermost one's type: written without a typedef name,
  qualifiers or `_Atomic`, and not an enumeration where the cast is its
  integer type (`(int)(I){3}`, `(int)(const int){3}` and `(int)(enum
  E){A}` stay non-constant). Without a cast the literal's type need only
  be compatible with the object's. Lowering copies the bytes of the
  literal's own initializer. Inside braces GCC takes the value of a bare
  arithmetic literal of any type and converts it to the element's type
  (`static double a[1] = { (int){1} };`), so bcc stores that value. Under the GCC identity Warn/Deny report "'compound literal
  as a constant initializer' is a GNU extension" where GCC says
  "initializer element is not constant [-Wpedantic]"; Clang reports
  nothing, even with `-pedantic-errors`, so the Clang identity does not
  either. Source: [GCC compound
  literals](https://gcc.gnu.org/onlinedocs/gcc/Compound-Literals.html); see
  [Compound literals in constant expressions](#compound-literals-in-constant-expressions-2026-10-28).
- `AlignofExpression` has GNU origin and accepts an expression operand in
  every mode, parenthesized or, as GCC accepts it, not (`_Alignof x`,
  `_Alignof (a)[1]`): every spelling takes `__alignof__`'s operand, a
  parenthesized type name or a unary expression, like `sizeof`. ISO
  `_Alignof` and C23 `alignof` spellings report GNU origin under
  Warn/Deny, including GNU dialects; Allow is silent. Reserved GNU aliases
  retain their exemption, and parser suppression still applies. The source is
  [GCC's alignment extension](https://gcc.gnu.org/onlinedocs/gcc/Alignment.html).
  Keyword availability and spelling checks are independent and unchanged.
  Of a function type it is 1 under GCC's rules and 4 under Clang's, on every
  target; of a function designator, the function's `aligned` attribute
  replaces that (gcc.c-torture/execute/align-3.c).
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
| C23 | All `[[...]]` attribute positions with standard/vendor names and balanced arguments; `bool`/`true`/`false`, `nullptr`, `constexpr`, `typeof`/`typeof_unqual`, message-optional `static_assert`, `alignas`/`alignof`/`thread_local` aliases; labels before declarations and at block end; `{}`; `_BitInt` with signedness; fixed enum underlying specifier-qualifier lists; `auto` type inference, alone or beside another storage class except `typedef`; unnamed definition parameters; ellipsis-only prototypes; empty parameter lists as prototypes; rejected identifier-list declarators; decimal type keywords; compound-literal storage classes | Attribute applicability/meaning, width values, enum type legality, decimal literal support and literal object lifetime; inferred types are analyzed (see "Type inference") |
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

A translation unit with no external declaration, which C99 §6.9p1 does not
allow, compiles in every mode, as in GCC, Clang and TCC (a source whose text
is all conditioned out is common). `-pedantic` reports it as a warning and
`-pedantic-errors` as an error, in GCC's words ("ISO C forbids an empty
translation unit [-Wpedantic]") or, under Clang's rules, Clang's ("ISO C
requires a translation unit to contain at least one declaration
[-Wempty-translation-unit]").

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
Storage](https://gcc.gnu.org/onlinedocs/gcc/Thread-Local.html)). No
pedantic policy reports it, in any revision, as GCC and Clang report none;
Clang's separate pedantic warning for `__thread` written before `static`
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
[middle-end.md](middle-end.md#gnu-inline-assembly-2026-10-26). A declarator
assembly label on a `register` automatic object makes a GNU local register
variable under both identities, as in GCC and Clang: an ordinary object that a
general-register `asm` operand places in the named register. Builtins keep their type/expression operands and `__builtin_offsetof`
member paths. Statement expressions, local labels, label addresses, computed
goto, omitted conditional operands, GNU initializers and nested definitions
also have inspectable syntax nodes.

`__extension__` suppresses extension diagnostics for keyword, grammar and
numeric-constant tokens within its expression or declaration, including a
function body or a parameter. Suppression ends at the owning production's boundary;
later unsuppressed occurrences, including ones from macro expansions, keep their
diagnostics. Nested functions isolate typedef, label and switch state; under
the Clang identity a function definition in a block is the error "function
definition is not allowed here", as in Clang (TCC's and MSVC's identities give
"cannot use local functions" and C2601 "local function definitions are
illegal"), and under GCC's identity it is a GNU nested function that code
generation supports (`middle-end.md`, "GNU nested functions"), which `auto`
may declare ahead of its definition. GNU empty
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
prefixes from C11, and under the GCC identity from gnu99 too, as GCC does
(where it also defines `__STDC_UTF_16__` and `__STDC_UTF_32__`), and `u8`
character prefixes from C23. Earlier modes keep the identifier followed by a
literal. C23 digit separators are part of pp-numbers
and must lie between digits of the constant's radix. C23 binary constants,
`wb`/`uwb` suffixes and C2y `0o`/`0O` octal prefixes convert to typed values.
Suffix recognition follows the mode gates for pasted tokens too. GNU binary
constants before C23 report the C23 origin. GNU imaginary `i`/`j` suffixes keep
the real component's precision and signedness, in either order with ordinary
floating suffixes; an integer takes no `f` suffix. Dollar identifiers, including
continuations, report GNU policy diagnostics, except inside a written
`#include <...>` name; `-fno-dollars-in-identifiers` makes `$` an ordinary
character outside identifiers, as GCC, Clang and TCC do, and
`-fdollars-in-identifiers` restores the default. C89 long-long suffixes and hexadecimal floating constants
report C99 policy diagnostics while keeping their values.

A sign after `e` or `E` always continues a pp-number; after `p` or `P` it does
from C99, which added that form for hexadecimal floating constants. Before C99
the identity decides: GCC's rules (`--gcc-mode`, TCC and MSVC identities) take
it only in a GNU mode (GCC's "extended numbers"), so strict C90 reads
`0x1p+f` as `0x1p` `+` `f` as GCC's `gcc.dg/c90-hexfloat-2.c` expects, and
Clang's take it only in a number that starts with `0x` or `0X` and has no `_`.

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

Source text that is not UTF-8 keeps its bytes (`util/raw_bytes.rs`): phase 1
reads each byte of malformed UTF-8 as a private-use stand-in, U+10FF00 plus
the byte, and phase 5 turns a stand-in in a literal back into that byte, so
`"\0\377"` and `"\0ÿ"` written in Latin-1 are the same three bytes
(gcc.c-torture/execute/20000227-1.c). The stand-ins mean bytes only in a
translation unit that read malformed UTF-8. As each compiler does:

| Literal | GCC identity | Clang identity |
|---|---|---|
| unprefixed string or character | bytes kept, silent | bytes kept, one "illegal character encoding in string literal [-Winvalid-source-encoding]" (or "character literal") per literal, an error under `-pedantic-errors` |
| `u8` | bytes kept, silent; a `u8` character constant of more than one byte is an error | error "illegal character encoding in string literal" |
| `u`, `U`, `L` | error "converting to execution character set: Invalid or incomplete multibyte or wide character" | error "illegal character encoding in string literal" |

An unprefixed literal joined to a prefixed one takes its prefix (C11
§6.4.5p5), and so its row: Clang's warning becomes the prefixed literal's
error, and GCC rejects bytes in a `u`, `U` or `L` result.

A stray byte outside a literal is the error "stray byte <FF> in source", and
source excerpts show each byte as `<FF>`, as Clang does. `-E` copies the
bytes of a string literal that is not UTF-8, as GCC does. `-E` prints each
token as it was spelled and leaves phases 5 to 7 to the compilation that
reads its text: an escape C does not define, a number that is no constant
or a character no C token starts with (`@`, `` ` ``) passes through
unreported, as in GCC's and Clang's `-E`.

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
the dead-code model reads them (`semantic-analysis.md`). Under the GCC
identity, `#pragma scalar_storage_order big-endian` (GCC reads only `big`),
`little-endian` and `default` are retained in the same way: a structure or
union definition that ends while `big-endian` is in effect, without its own
`scalar_storage_order` attribute, is an error, as for the attribute, so
`big-endian` followed by `default` before any definition still compiles.
Another operand is ignored with GCC's `-Wpragmas` warning. Packing is implementation-defined pragma behavior
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
- Reserved GNU spellings are no extension diagnostic under any policy, since
  neither GCC nor Clang reports them under `-pedantic(-errors)`: the alternate
  keywords (`__inline__`, `__const`, `__signed__`, `__restrict__`,
  `__volatile__`, `__alignof__`, `__typeof__`, `__thread`, even in a revision
  without the keyword), `__attribute__`, `__asm__` statements and labels,
  `__real__`, `__imag__`, `__builtin_types_compatible_p`,
  `__builtin_choose_expr`, `__builtin_convertvector`, `__builtin_bit_cast`,
  and calls of `__builtin_*` functions. Only `__complex` and `__complex__`
  report the C99 origin of complex types before C99, as GCC and Clang do,
  and under every identity but Clang's `__builtin_complex` does too, as GCC
  reports it. Where the compilers differ, the identity decides: GCC reports
  `__int128` and `__label__` and Clang does not; Clang reports `__auto_type`
  and GCC does not. The GNU constructs spelled without a reserved name (case
  ranges, `?:` with an omitted operand, statement expressions, zero-length
  arrays and the like) stay reported.
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
- `__LINE__` and `__FILE__` follow the claimed compiler. One spelled in the
  source, in a macro argument too, has its own line. One spelled in a
  replacement list has the line of the innermost macro being replaced whose
  name was spelled in the source, which a `#define` line does not hold
  (`preprocessing/runtime/location.rs`): `T(__LINE__)` in the body of `L`,
  and `OBJ` expanded in `ID(OBJ)` there, are at `L`'s invocation. For GCC
  that name must also have been read from the source or from an argument
  being expanded, so a macro whose name an argument supplied and whose call
  a body completes (`#define F(m) m()` called as `G(\nE)` through
  `#define G(x) F(x)`) is at the outer `G`'s line; Clang gives `E`'s line,
  as the Clang identities do. Clang gives a multi-line invocation's body the
  line of its closing parenthesis, which bcc does not reproduce.
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
| `__BITINT_MAXWIDTH__` | The widest _BitInt follows the compiler identity; defined beside the identity macros. |
| `__CONSTANT_CFSTRINGS__` | Objective-C and CoreFoundation string intrinsics are not implemented. |
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
types GCC infers from bit-fields, fixed-underlying enums, MSVC pointer
modifiers or `ext_vector_type`. They no longer list unrelated possible causes.

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
(the C library has no such function). And under the GCC identity `conj` of
a real argument returns imaginary part -0, the conjugate of the +0 a real
number has as a complex one (§7.22p6, §7.3.9.3), as glibc and GCC give;
otherwise it returns the argument with +0, as Clang's header and the pinned
Clang do.

Binary128 literals are parsed exactly with arena-backed integer rational
conversion and one IEEE round-to-nearest-even step. Since 2026-10-27
binary128 arithmetic, comparisons and casts fold with software IEEE
binary128 arithmetic that rounds and propagates NaNs as libgcc does, so a
cast of a binary128 constant to an integer type is an integer constant
expression and static initializers hold the run-time bits; only the product
and quotient of two complex values stay unfolded. Programs compute with
binary128 on every back end: `fp128` in LLVM, libgcc's soft-float routines
(`__addtf3`, `__lttf2`, `__extenddftf2` …) in the native back end, and the
same software arithmetic in the interpreter, whose host also provides
glibc's `strtof128`, `strfromf128` and exact `_Float128` math functions.
The x87 helper is never used to approximate binary128. See
[semantic-analysis.md](semantic-analysis.md#binary128-arithmetic-and-folding-2026-10-27)
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
nonzero value). The resource `<float.h>` defines GCC's TS 18661-3 names
(`FLT128_MAX`, `FLT32X_EPSILON`, … and, after C17, `FLTN_SNAN`) when
`__STDC_WANT_IEC_60559_TYPES_EXT__` is defined, for each type whose
`__FLTN_MANT_DIG__` is predefined, as GCC's header does; a test checks
`FLT128_*` and `_Float128` arithmetic against GCC 15's output. Limits: the
resource `<tgmath.h>` takes the types with the standard type's functions and
result types (above), and refuses `_Float128`; GNU vector element types and
`__builtin_*f128` math beyond the existing primitives remain unsupported.
Imaginary `_FloatN` constants such as `1.0f32i` are described under
[Imaginary interchange constants](#imaginary-interchange-constants). `_Float16`, `__bf16`
and `__fp16` follow in
[Half-precision floating types](#half-precision-floating-types-2026-10-27).

## Half-precision floating types (2026-10-27)

`_Float16` (C23 Annex H's binary16 interchange type) is a keyword with its
`f16`/`F16` constant suffix for the identities whose compilers provide it on
x86-64 with SSE2: GCC 12 and later, and Clang (`clang` and `clang-cl`), in
every C mode (`Feature::Float16`). MSVC and TCC have no such type, so the
name stays an identifier there. GCC reports the type and the suffix before
C23 under `-pedantic`; Clang reports neither. Out-of-range constants warn
as other floating constants do. The predefined `__FLT16_*__` macros are
Clang's for the Clang identities (the frozen target tables no longer
exclude them) and GCC 15's, with `__FLT16_IS_IEC_60559__`, for the GCC
identity; `<float.h>` defines `FLT16_MANT_DIG` through `FLT16_TRUE_MIN`
under `__STDC_WANT_IEC_60559_TYPES_EXT__` where `__FLT16_MANT_DIG__` is
predefined. `FLT_EVAL_METHOD` and `__FLT_EVAL_METHOD__` stay 0, as both
compilers keep them.

Neither compiler computes in binary16: an operation converts its operands
to `float`, and the excess precision is removed where each compiler removes
it, which bcc follows per identity (verified against GCC 15 and the pinned
Clang on each back end, `tests/integration/half_precision.rs`):

| Use of a `_Float16` value | GCC | Clang |
| --- | --- | --- |
| Operand of `+ - * /`, unary `+ -` | excess kept | excess kept |
| Assignment, cast to `_Float16`, argument, return | rounded | rounded |
| Comparison, truth value | excess | rounded |
| Conversion to another type (`(double)x`, `float f = x`) | excess converted | rounded first |
| `?:`, comma operator | excess kept | rounded |
| `f16` constant | rounded to `float` | binary16 |
| Integer operand of an operation | converted to `float` | converted to `_Float16` |
| Constant expressions (`static_assert`, static initializers) | as above | each operation rounded |

Default argument promotions do not apply (a `_Float16` travels through `...`
as itself), and the System V ABI passes it in the low 16 bits of an SSE
register; structures and unions of halves are classified as usual. Native
code converts with libgcc's `__extendhfsf2` and `__trunc{sf,df,xf,tf}hf2`,
as GCC does without `-mf16c`.

`_Complex _Float16` (4 bytes, alignment 2) computes in `float _Complex`
in both compilers, multiplying and dividing with libgcc's `__mulsc3` and
`__divsc3` (not `__mulhc3`/`__divhc3`), and keeps the excess where a real
`_Float16` does. Both pass it in the low 32 bits of one SSE register, real
part first, as a structure of two halves.

`__bf16` (bfloat16: 8 exponent and 7 fraction bits) is an arithmetic type
in GCC 13 and later and in Clang on x86-64 with SSE2 (`Feature::BFloat16`),
in every mode and without a pedantic diagnostic, since the name is
reserved. It ranks below `_Float16` (`b + h` is a `_Float16`) and follows
the table above in each identity, the conversion of a `__bf16` operand to
`_Float16` included: GCC evaluates `__bf16 + _Float16` in `float`, so
`0x1p20` plus 1 is 1048577 there, while Clang converts the `__bf16` to
`_Float16` first, giving infinity. GCC carries a plain (not excess) value
of one half type converted to the other, by cast or implicitly, as its
exact `float` value: `(_Float16)b` keeps a `__bf16` beyond binary16's range
and `(__bf16)h` keeps binary16's precision. It rounds such a value only to
store, pass or return it, and as an arm of a `?:` whose type is
`_Complex _Float16` when neither arm is an excess-precision expression
(an arithmetic operation or an `f16` constant, seen through parentheses,
unary `+` and `-`, which are exact, the comma operator, `_Generic` and
`__builtin_choose_expr`). GCC's parser folds a cast whose operand is
already a constant node (a literal, an integer constant expression, a cast
of a constant node, the selected operand of a `?:` with a constant
condition, `__builtin_complex` of constant nodes, or arithmetic narrowed
to a standard floating type) and rounds that conversion where its
code does not: `(_Float16)(__bf16)0x1p20` is infinite, but
`(_Float16)(__bf16)-0x1p20` and `(_Float16)(__bf16)(0x1p19 + 0x1p19)`,
whose operands GCC folds only later, are `-0x1p20` and `0x1p20`. bcc
follows each case (`semantic_analysis/excess.rs`), with the known
deviations below;
`scripts/generate_half_matrix.py` writes a program converting every pair
of integer, real and complex types through `+`, `?:` and casts, with each
compiler's output as the expectation (`tests/integration/half_precision.rs`). The ABI passes it as `_Float16`; it is never promoted through `...`.
GCC has `bf16`/`BF16` constants, which `-pedantic` calls a non-standard
suffix, and predefines `__BFLT16_MANT_DIG__` and its relatives (with
`__BFLT16_IS_IEC_60559__` 0); the GCC identity provides both, and the Clang
identity neither, as Clang has neither. Conversions from integers differ:
Clang converts to `float` and rounds that (LLVM's lowering), so a wide
integer rounds twice, while GCC rounds once (through `double`, the x87
format or `__floattibf`); bcc follows the identity. Narrowing calls
libgcc's `__trunc{sf,df,xf,tf}bf2`, and widening shifts the encoding into a
`float`, as both compilers do inline, so a signaling NaN stays signaling
(GCC's `__extendbfsf2`, which it calls before arithmetic, quiets one; that
case alone can differ from GCC). Neither compiler has `_Complex __bf16`.

`__fp16` is Clang's binary16 storage type, which Clang accepts on x86-64
(`Feature::Fp16`, the Clang identities): every operation promotes its
value to `float`, a variadic argument to `double`, and assignment rounds
once; parameters and results of the type, and `_Complex __fp16`, are
refused with Clang's errors. GCC provides `__fp16` only on ARM, so the GCC,
MSVC and TCC identities keep the name an identifier, and GCC's "unknown
type name" follows from that.

Verified against GCC 15 and the pinned Clang on every back end
(`tests/integration/half_precision.rs`: `bfloat16.c`, `complex_half.c`,
`storage_half.c` and the ABI link tests). Limits: GCC's `__builtin_*f16`
constant builtins and `FLT16_SNAN` are absent; imaginary `f16i`
constants are described under
[Imaginary interchange constants](#imaginary-interchange-constants).

### Known GCC-identity folding deviations

bcc models GCC's parse-time folding by constant nodes (above) rather than
GCC's whole folder, so these folded constants still convert differently
under the GCC identity (`(double)` of each, GCC 15). The Clang identity
matches Clang in each.

| Expression | GCC | bcc |
| --- | --- | --- |
| `(_Float16)(__bf16)(1.0 == 1.0 ? 0x1p20 : -0x1p20)` (a comparison or `!` of floating constants as condition) | `inf` | `0x1p+20` |
| `(_Float16)(__bf16)((int)(0x1p19 + 0x1p19) ? 0x1p20 : -0x1p20)` (an integer conversion of arithmetic as condition) | `inf` | `0x1p+20` |
| `(_Float16)(__bf16)(float)(1 ? 0x1p19 + 0x1p19 : -0x1p20)` (a constant `?:` beneath a narrowing cast) | `inf` | `0x1p+20` |
| `(_Float16)(__bf16)(float)(0, (int)(0x1p19 + 0x1p19))` (a comma before an integer conversion) | `inf` | `0x1p+20` |
| `(_Float16)(__bf16)(float)(1e30 + 1e30)` (constants not exact in the narrower type, which GCC does not shorten) | `0x1.94p+100` | `inf` |
| `(__bf16)(h * 1)` (GCC simplifies `x * 1` to `x`) | unrounded | rounded |

Separately, a scalar compound literal in a static initializer, such as
`static double d = (double)((double){0x1p20});`, is refused as not
constant for every type (also on `main`); GCC and Clang accept it.

## GNU vectors and x86 SIMD resources (2026-10-09)

GNU vector semantics and x86 resource coverage follow [GCC Vector Extensions](https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html) and [Clang Language Extensions](https://clang.llvm.org/docs/LanguageExtensions.html); see semantic-analysis.md for the supported instruction families and constant-evaluation boundaries.

### GNU vector values (2026-10-29)

`vector_size` vectors compile and run on every back end (LLVM, native
x86-64, interpreter, wasm32): declarations, initializers and compound
literals (also in static initializers), assignment and loads and stores
through vector pointers (aligned or `aligned(1)`), the arithmetic, bitwise,
shift and comparison operators with vector or scalar operands, unary `+`,
`-` and `~`, subscripts (read and write, constant or variable index), casts
between vectors of one size and to and from scalars of that size,
`__builtin_shufflevector`, `__builtin_convertvector`, `__builtin_bit_cast`,
GCC's `__builtin_shuffle`, `sizeof`, `_Alignof`, vectors in structures and
arrays, `_Atomic` vectors and `va_arg` of a vector. Lowering keeps a vector
in memory and scalarizes every operator lane by lane (middle-end.md, "GNU
vector values"). `vector_size` arguments are integer constant expressions
(`vector_size(4 * sizeof(int))`).

Where GCC 15.2 and Clang 23 differ, each identity follows its compiler
(probed with both; `semantic_analysis/tests/vectors.rs` pins the cases):

| Rule | GCC identity | Clang identity |
| --- | --- | --- |
| Scalar operand of a vector operator | `scalar_to_vector`: an integer variable no wider than the lanes (sign changes allowed), an integer constant that fits (a negative one into unsigned lanes and an unsigned one into signed lanes allowed), an integer into floating lanes whose format holds its type's range, never a floating scalar into integer lanes, no `_Bool` or enumeration, nothing into enumeration lanes, which also shift and count nothing | `tryGCCVectorConvertAndSplat`: integers by rank (no sign change at equal rank) or a constant's significant bits; a floating scalar into integer lanes of its size; `_Bool` and enumerations as integers; otherwise a scalar as large as the whole vector reinterprets its bits (lax conversion), except as a compound assignment's right operand |
| Vectors of different types | binary operators need as many lanes of one kind and width, signedness aside; assignment, initialization, arguments, `return` and `?:` need compatible lane types | equal sizes suffice everywhere (lax vector conversions), the right operand reading as the left's type |
| Comparison result | an opaque mask that converts to any vector of its size | the signed mask vector |
| `++`, `--` | each lane | refused |
| `&v[i]` | allowed | refused |
| Lane count | power of two | any; the object rounds up |
| `__builtin_shuffle` | provided | absent |
| Two-operand `__builtin_shufflevector(v, mask)` | refused | run-time shuffle |
| Atomic vector operands | read first, also with a scalar operand, which a compound assignment converts as a variable (`b += 1` into `float` lanes is refused); `++`/`--` only where an `int` variable splats into the lanes; `v[i]` of an lvalue is an `_Atomic` lane lvalue (`v[1] = 2`, `&v[1]` of type `_Atomic int *`) | no scalar operand; two atomic operands only in a comparison; no `++`/`--`; `v[i]` is a value |
| Lane types besides integers and real floating types | `_Float16`, `__bf16`, enumerations; through pointer, array and function declarators to their element | `_Float16`, `__bf16`, `_BitInt(N)` of a power-of-two width of at least 8; never a pointer, array or function |

Calls pass vectors as the claimed compiler does (middle-end.md). Known
differences: GCC's `_Alignof` of a vector or structure aligned above 16
bytes reports 16 without `-mavx` (`__alignof__` reports the alignment, and
bcc reports it for both spellings); Clang returns a 64-byte vector in
XMM0-XMM3, bcc through memory as GCC does; on Win64 Clang passes a 32- or
64-byte vector argument as one pointer per 16 bytes, bcc as one pointer;
on wasm32 Clang splits a vector argument into its lanes, bcc passes it by
reference. The elementwise and non-temporal builtins
(`__builtin_elementwise_*`, `__builtin_nontemporal_*`) are checked but not
lowered (refused by name), as are the `__builtin_ia32_*` builtins.
GCC accepts a braced initializer for an `_Atomic` vector and refuses a
vector element in a vector initializer (`{a}`) or braces around a lane
(`{1, {2}}`); bcc follows Clang in all three. Both compilers only warn
about an excess element in a vector initializer, which bcc refuses, as it
does for arrays (C99 §6.7.8p2). Clang folds vector arithmetic, unary
operators and subscripts of constant vectors in static initializers, which
both bcc identities refuse, as GCC does. `__builtin_bit_cast` is accepted
under the GCC identity too, though GCC provides it only for C++. A
half-precision lane computes in `float`. Under the GCC identity it rounds
after each operation, as GCC computes vector operations on them; under the
Clang identity `+`, `-`, `*`, `/` and unary `-` keep the `float` excess to
the end of the expression, as Clang promotes a `_Float16` or `__bf16`
vector to a `float` one and as bcc already does for scalars, so
`(a + b - b)[0]` with `a[0] == 1` and `b[0] == 2048` is 1, and a comparison
reads the rounded lanes. A GCC-only builtin
(`__builtin_shuffle`, `__builtin_parityg`, `__builtin_ffsg`,
`__builtin_clrsbg`) called under another identity is an undeclared
identifier, as Clang reports it. An `_Atomic` vector of more than 16 bytes,
or whose lanes leave padding (3 `int`s in 16 bytes), updates through a
byte-wise compare-exchange loop.


### x86 intrinsic headers (2026-10-30)

`<mmintrin.h>` (MMX), `<xmmintrin.h>` (SSE), `<emmintrin.h>` (SSE2),
`<pmmintrin.h>` (SSE3), `<tmmintrin.h>` (SSSE3), `<smmintrin.h>` (SSE4.1 and
SSE4.2, with `<crc32intrin.h>`) are bcc's own: written for bcc from Intel's
definitions of each intrinsic, not derived from GCC's or Clang's headers,
and under bcc's licence. Every intrinsic GCC 15 and Clang 23 declare in
them is plain C over GNU vector operations (and, for MMX, 8-byte vectors),
so it compiles and runs on the interpreter, LLVM and the native back end
with no x86 builtin lowered: saturating, horizontal and widening arithmetic
loop over lanes; conversions round by the current rounding mode through
`nearbyint` and give the integer indefinite value out of range; `min` and
`max` keep the second operand for NaNs; the string comparisons
(`_mm_cmpistri` and family) follow the SDM's PCMPxSTRx definition. An
intrinsic with an immediate operand is a macro that also names the GCC/Clang
builtin inside `sizeof`, which is not evaluated, so semantic analysis still
checks the immediate as before, and the builtin, spelled in a system
header's macro, is no `-pedantic` extension of the caller's code: under the
Clang identity an out-of-range constant is Clang's
`-Wargument-outside-range` ("argument value 256 is outside the valid range
[0, 255]"), an error that `-w`, `-Wno-argument-outside-range` and
`-Wno-error=argument-outside-range` relax, and under the GCC identity an
error;
GCC's `_mm_shuffle_pd` takes any constant and reads its low two bits.
`__m64` is GCC's two-`int` vector (with `__m64_u` and `__v2sf`) under the
GCC identity and Clang's one-`long long` vector under the Clang identity. `tests/integration/x86_intrinsics.rs`
compares two programs covering every intrinsic with the host GCC's builds,
which run the instructions, under both identities on every native arm.

Differences: `_mm_rcp_ps` and `_mm_rsqrt_ps` return the correctly rounded
reciprocal (within the instructions' 1.5 * 2^-12 bound, but not a
processor's table), with the instructions' special cases: a subnormal
input reads as a zero of its sign (±infinity), and `_mm_rcp_ps` of a
magnitude of 2^126 or more is a zero; `_mm_cmpistr*` and `_mm_cmpestr*` call
two out-of-line static helpers rather than inlining the whole string
comparison at each call; `_mm_undefined_*` return zero; non-temporal stores are
ordinary stores; `_mm_getcsr`, `_mm_setcsr`, the fences, `_mm_pause`,
`_mm_clflush`, `_mm_monitor` and `_mm_mwait` are inline assembly, which the
interpreter cannot run. The AVX, AVX2, AES, PCLMUL, 3DNow!, ADX and CPUID
headers are still Clang 23.1.1's (Apache-2.0 WITH LLVM-exception,
`src/headers/LICENSE-LLVM`): they type-check, and their intrinsics that are
generic vector code run, but those that call `__builtin_ia32_*` builtins are
refused by lowering. The headers are x86 only, as GCC's and Clang's are.

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
GCC's `__builtin_setjmp (void *)` and `__builtin_longjmp (void *, 1)`
([Nonlocal Gotos](https://gcc.gnu.org/onlinedocs/gcc/Nonlocal-Gotos.html))
take GCC's `void *` buffer and require an integer constant that converts
to the `int` 1, as GCC and Clang do (a value the conversion changes gets
`-Wconstant-conversion`, which bcc reports only there so far); they run on every back end with LLVM's buffer layout (see
[middle-end.md](middle-end.md#gccs-builtin-setjmp-and-longjmp-2026-10-27)).

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
them. Under the GCC identity only, GCC's `scalar_storage_order` (GNU and
`gnu::` spellings) follows GCC: a missing or invalid argument is GCC's error;
`"little-endian"` is the native order of every bcc target and is accepted;
`"big-endian"` on a structure or union definition, or on a typedef of a
structure or union type, is an error, since ignoring it would store the
scalars in the other order. On an object, function, parameter, member,
enumeration definition or a typedef of another type it is ignored with a
warning, and on a tag that is only named or declared silently, as in GCC.
Two cases are conservative errors where GCC is lenient: a definition naming
both orders, whose last one GCC applies, and `[[gnu::scalar_storage_order]]`
after a body, which GCC ignores; in a type name, such as a cast's, it is
ignored without GCC's warning. `#pragma scalar_storage_order` is described
with the other pragmas. The other identities keep the unknown-attribute
warning: Clang and clang-cl ignore the attribute, and TCC compiles it in the
native order without a diagnostic. Unknown-attribute and dropped-attribute warnings follow
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
| GCC identity only | `scalar_storage_order`: little-endian accepted, big-endian refused on records, ignored with a warning elsewhere (see above); unknown under the other identities |
| Unknown (warning, as in bundled Clang) | every other name, including GCC-only `noinit`, `symver` and `persistent`, MSVC `restrict`, `uuid`, `allocate` and `align` through MinGW's `__declspec` macro, a name in the other vendor scope, and target-specific names on other targets |

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
  `__va_start(&ap, last)`, through which the MSVC `<vadefs.h>` defines
  `va_start` on x64, is Microsoft's `va_start` of the list its first
  operand points to, with Microsoft extensions (`void (char **, ...)`, as
  in Clang). A header's declaration of one of these Microsoft intrinsics
  with a compatible type keeps it the intrinsic, as Clang keeps a
  redeclared builtin; a declaration of another type is an error (Clang's
  `conflicting types for '__va_start'`), and so is a definition (Clang's
  `definition of builtin function`). Without Microsoft extensions the names
  are ordinary.

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

## Compound literals in constant expressions (2026-10-28)

`static double d = (double)((double){0x1p20});` was "not a constant
expression" under both identities. Each identity now follows a survey of
GCC 15.2 and the bundled Clang 23.1.1 in C99 through C23, with and without
`-pedantic`:

| Initializer | GCC | Clang |
| --- | --- | --- |
| `(T){v}`, `(struct S){...}`, at file or block scope | `-Wpedantic` | silent |
| `(double)((double){v})`, `(int)(long)(int){v}`, `*&(T){v}`, `1 ? (T){v} : x` | `-Wpedantic` | silent |
| `_Generic(0, int: (T){v})`, `__builtin_choose_expr(1, (T){v}, 0)`, also in braces | `-Wpedantic` | silent |
| `(_Float16)(__bf16)(_Float16){0.1}` (kept unrounded by GCC), `(long double)(__float128)(long double){v}` | `-Wpedantic` | silent |
| `{ (int)(int){v} }`, `{ 1 ? (int){v} : 0 }` (inside braces) | error | silent |
| `(int){3} + 1`, `(int)(double){2.5}`, `static double d = (int){3};`, C23 `(int){} + 1` | error | silent |
| `(int)(double)(int){3}`, `(int)(char)(int){300}`, `(__float128)(long double)(__float128){v}` | error | silent |
| `static int x = (unsigned)(int){3};`, `static float f = (double)(float){1};` | error | silent |
| `enum { E = (int){3} };`, `case (int){3}:`, `char a[(int){3}];` at file scope | error | `-Wgnu-folding-constant` |
| `static char (*p)[(int){3}];` in a block | variable bound | variable bound |
| `&(T){v}`, `(T[]){...}`, `&(T[]){...}[1]`, `&(struct S){...}.m` at file scope | silent | silent |
| `&(T){v}`, `(T[]){...}` in a block-scope `static` initializer | error | error |
| `(volatile int){3} + 1`, `(int[]){1, 2}[1]`, `*(int[]){1}` | error | error |
| `__builtin_constant_p((int){1})` | 0 | 1 |

The GCC identity accepts the literal only as the whole initializer, inside
the wrappers listed under `ConstantCompoundLiteral` above. GCC's folder
compares floating types by their precision in bits, so it removes a
conversion pair through an equally wide format that cannot hold every value
(`__bf16` for `_Float16`) and keeps the literal's value unrounded; bcc copies
the literal's bytes the same way. Under the Clang identity (and clang-cl) a
non-volatile scalar arithmetic literal whose initializer is an arithmetic
constant, or the empty initializer, also folds into any constant
expression, at file and block scope and in every mode, as Clang's evaluator
reads it: it is an arithmetic constant, never an integer constant expression,
so integer contexts accept it as the folded pointer constants above,
reported under `-Wgnu-folding-constant`. A cast around a whole literal may
then convert it to the object's type too (`(unsigned)(int){-1}` for an
`int`), and the conversions it passes through must keep every value, as
Clang's evaluator converts through them. Function code still loads the
literal, as Clang emits it, so `__builtin_constant_p((g, (int){1}))` stays 0
there. Under the GCC identity a query whose evaluated operands read a
literal is 0, as GCC answers even optimizing; operands that are never
evaluated (a `_Generic` controlling expression, `sizeof`, and the operand a
constant `?:`, with or without its middle operand, `&&`, `||` or
`__builtin_choose_expr` skips, a null pointer or an object's address
deciding it too) do not count, so `__builtin_constant_p(1 ? 2 : (int){3})`
and `__builtin_constant_p(&g ? 2 : (int){1})` are 1. A block-scope static pointer
to an array keeps a bound Clang does not fold, a compound literal's or an
address difference's, as a variable bound without a diagnostic; Clang folds
only where the type could not be variably modified: a static object's own
array bounds, and every bound of an extern object's type. A file-scope literal has static storage and a
block-scope one automatic storage (C99 §6.5.2.5p6), so only the former's
address is an address constant (§6.6p9), under both identities.

The MSVC and TCC identities take the GCC identity's rules, the stricter
ones; neither compiler was surveyed, since no `tcc` or `cl.exe` is
available here. TCC implements the GNU dialect, so GCC's whole-literal
rule is the nearer model; MSVC is assumed not to read literals in constant
expressions either.

These cases remain different from the compiler each identity claims:

- GCC accepts `static int i = (0, (int){3});` and even `(v, (int){3})` with
  two `-Wpedantic` warnings, but rejects `((int){1}, (int){3})`; bcc rejects
  every comma around a literal under the GCC identity.
- Clang also reads a member of a structure literal (`(struct S){1, 2}.b`,
  `(&(struct S){1, 2})->b`), an element of a `const` array literal
  (`(const int[]){1, 2}[1]`) and a pointer literal's value
  (`(char *)(int *){&g}`, `(intptr_t)(int *){0}`); bcc folds only scalar
  arithmetic literals.
- Clang folds a literal's value where bcc still requires an integer
  constant expression: an automatic array with an initializer
  (`char a[(int){3}] = {0};` in a block, which Clang folds with
  `-Wgnu-folding-constant`), an array designator (`[(int){1}] = 2`) and a
  GNU case range (`case 1 ... (int){3}:`).
- C23 `constexpr` compound literals (`(constexpr int){3}`), which GCC 15
  accepts in integer constant expressions and the bundled Clang does not
  parse, stay unmodeled with `constexpr` objects: lowering refuses them.
- GCC folds a `?:` before the casts around it, so `static int x = (int)(1
  ? (long)(int){3} : 0L);` is constant for GCC; bcc requires the `?:` to
  have the object's type and rejects it.
- A `?:` whose operands share a type node keeps it, so GCC accepts
  `1 ? (I){3} : (I){4}` for a typedef `I`, and the same with a typedef of
  a structure; bcc requires a literal of exactly the `?:`'s own type there.
  Conversely GCC rejects `(int)(__typeof__(I)){3}` and `(int)(int
  __attribute__((aligned(8)))){3}` as distinct variants of `int`, which
  bcc, seeing no typedef name, accepts.
- A block-scope static object whose member is a pointer to an array with a
  literal bound (`static struct { char (*p)[(int){3}]; } s;`) and a static
  or extern array of pointers to such arrays (`static char
  (*a[2])[(int){3}];`, `extern char (*b[2])[(int){3}];`): GCC
  accepts both (the first with a `-Wpedantic` warning) and Clang folds the
  first with `-Wgnu-folding-constant` and accepts the second silently; the
  GCC identity rejects both as variably modified statics, and the Clang
  identity warns about the second too.
- GCC accepts integer-zero literals as null-pointer elements (`{ (int){0} }`
  into `int *`); bcc and Clang reject them.
- Inside braces only an arithmetic literal converts to another element
  type; a pointer literal of another pointer type (`{ (int *){&g} }` for a
  `const int *`) stays rejected under both identities, and under the Clang
  identity a pointer literal under a qualifying cast (`static const int *x
  = (const int *)(int *){&g};`) too. Clang's `-Wenum-conversion` for
  `(enum B)(enum A){A0}` is not reproduced.
- GCC prints a `-Wpedantic` warning before its error for a literal inside
  braces within a cast; bcc prints the error alone. Clang's
  `-Wconstant-conversion` and overflow warnings for a literal's
  initializer (`(unsigned char){300} + 0`, `(int){2147483647} + 1`) are not
  reproduced, as they are not for other initializers.

The problems found while surveying that have nothing to do with literal
reads are fixed; see "Semantic analysis gaps" below.

`tests/fixtures/programs/static_compound_literals.c` runs every accepted
form, the half-precision ones and their excess precision included, on the
interpreter, LLVM and the native back end under each identity against the
GCC and Clang builds (`tests/integration/static_compound_literals.rs`);
`backend/wasm/tests/execution.rs` checks the binary128 `long double` and
`__fp16` bytes literals give static objects on `wasm32-wasip1`;
`semantic_analysis/tests/compound_literal_statics.rs` and the
`sema-static-compound-literal-{gcc,clang}` diagnostic goldens cover the
rejections.

## Semantic analysis gaps (2026-10-28)

Each rule below was surveyed against GCC 15.2 and the bundled Clang 23 in
C99, C17 and C23, GNU and ISO modes, with and without `-pedantic` and
`-pedantic-errors`, and follows the selected `--compiler-identity`. One
`CompilerConfiguration` helper per rule (`RuleSet`) says which identity
takes which side; neither TCC nor MSVC 19.33 was surveyed:

| Rule | GCC side | Clang side |
| --- | --- | --- |
| pointer/integer cast warnings (`pointer_cast_rules`) | GCC, TCC | Clang, clang-cl, MSVC (C4311/C4312 warn for narrower integers) |
| `_BitInt` widths (`bit_int_rules`) | GCC, TCC, MSVC | Clang, clang-cl |
| array compound literals (`array_literal_rules`) | GCC, TCC, MSVC | Clang, clang-cl |
| floating values out of an integer's range (`float_conversion_rules`) | GCC, TCC, MSVC | Clang, clang-cl |

Neither TCC nor MSVC 19.33 has `_BitInt` or arrays initialized by compound
literals. bcc's identities do not gate language features, which follow
`-std`; so `_BitInt` is a reported C23 extension before C23, and an array
compound literal keeps GCC's static-only rule with its pedantic report.

### `_BitInt` widths

The width N of `_BitInt(N)` is an integer constant expression of at least 2
(1 for `unsigned _BitInt`) and at most `BITINT_MAXWIDTH` (C23 §6.7.3.1p4).
Both compilers check it wherever the type is named: declarations,
typedefs, members, casts, compound literals, `sizeof` and `_Generic`. They
read the value differently:

| Width | GCC (`BITINT_MAXWIDTH` 65535) | Clang (`BITINT_MAXWIDTH` 2^23) |
| --- | --- | --- |
| not an integer constant expression (`int n; _BitInt(n)`, `_BitInt(2.0)`) | error | error |
| `_BitInt((int){8})`, `_BitInt(1 ? 8 : n)`, `_BitInt(0 ? n : 8)` | error, in every mode | folded in every mode; `-Wgnu-folding-constant` under `-pedantic` |
| `_BitInt(1)`, `signed _BitInt(1)` | error: at least 2 | error: at least 2 |
| `_BitInt(0)` | error: not positive | error: at least 2 |
| `unsigned _BitInt(0)` | error: not positive | error: at least 1 |
| `_BitInt(-1)` | error: not positive | error: too large (the `int`'s 32 bits read unsigned) |
| `_BitInt((signed char)-1)`, `_BitInt((short)-2)` | error: not positive | 255 and 65534 bits |
| `_BitInt(65536)` | error: too large | accepted |
| `_BitInt(8388609)`, `_BitInt(4294967298LL)` | error: too large | error: too large |

Clang zero-extends the value from its type and keeps the low 64 bits
(`BuildBitIntType`), which the identity reproduces. Clang also folds a read
of a `static const int k` in `_BitInt(k)`, in every mode. bcc's typing
reads such an object only in GNU modes, so the Clang identity folds
`_BitInt(k)` there and rejects it in ISO modes; the GCC identity rejects it
in every mode, as GCC does. Before C23, GCC reports the type under
`-pedantic` only once its width is valid ("ISO C does not support
'_BitInt(8)' before C23"), so the GCC identity reports it from semantic
analysis; Clang reports the keyword (`-Wbit-int-extension`, bcc's
`[-Wpedantic]`).

`__BITINT_MAXWIDTH__` is predefined in every mode and `<limits.h>` defines
`BITINT_MAXWIDTH` from C23, as 65535 under GCC's rules and 8388608 under
Clang's on x86-64, and as Clang's 128 on WebAssembly, which GCC does not
target (`CompilerConfiguration::bit_int_max_width`). The frozen Clang target tables keep excluding the macro, which is
defined beside the identity macros instead.

A cast between a `_BitInt` and a pointer takes the pointer-cast warnings
below (`(void *)(_BitInt(32))x`, `(_BitInt(32))p`), as both compilers
report; a constant cast to a `_BitInt` (`(_BitInt(32))3`) is an integer
constant expression there. A width that contains a `sizeof` of a `_BitInt`
is range-checked like any other (`_BitInt(sizeof(_BitInt(16)) * 65536)` is
too large for GCC, `* 0` too small for both). The check is
`Analyzer::bit_int_width`; the `sema-bitint-width-{gcc,clang}` goldens and
`semantic_analysis/tests/bit_int_widths.rs` pin it. A width that reads an
unanalyzed construct leaves the type unmodeled.

### `_BitInt` types

A checked `_BitInt(N)` or `unsigned _BitInt(N)` is an integer type of its
own (`TypeKind::BitInt`), distinct for each width and signedness, so
`_Generic` tells them apart (C23 §6.2.5p5-8). Its layout is the same on
every bcc target, as the x86-64 psABI (§3.1.2) fixes it and Clang follows it
on Win64 and WebAssembly:

| Width N | Size | Alignment |
| --- | --- | --- |
| 1-8 | 1 | 1 |
| 9-16 | 2 | 2 |
| 17-32 | 4 | 4 |
| 33-64 | 8 | 8 |
| 65 and up | 8 per 64-bit limb, ceil(N / 64) limbs | 8 |

Semantics follow C23:

- **Promotions and conversions.** A bit-precise type is not promoted, not
  even from a bit-field: the bit-field `_BitInt(40) a : 10` computes in
  `_BitInt(40)` (§6.3.1.1p2). In the usual arithmetic conversions ranks
  follow widths, and a standard type outranks a bit-precise type of its own
  width, so `(_BitInt(32))x + 1` is `int`, `(_BitInt(33))x + 1U` is
  `_BitInt(33)` and `(_BitInt(40))x + (unsigned _BitInt(40))y` is
  `unsigned _BitInt(40)` (§6.3.1.1p1, §6.3.1.8p1). Both compilers agree,
  including `__int128`, which outranks `_BitInt(128)`.
- **Constants.** `wb` and `uwb` constants take the narrowest width that holds
  the value and, for `wb`, a sign bit (`3wb` is `_BitInt(3)`, `0uwb`
  `unsigned _BitInt(1)`; §6.4.4.2p6), in every radix and up to
  `BITINT_MAXWIDTH` (`18446744073709551616wb` is `_BitInt(66)`); one wider
  is "too large", as GCC reports it. A token of more than 64 bits keeps
  its spelling, which semantic analysis reads into the exact value
  (`IntegerTokenType::WideBitInt`, `preprocessing::bit_precise_magnitude`).
  Folding computes at the type's width: an unsigned result wraps
  modulo 2^N and a signed one outside the type's range overflowed and is no
  constant. A shift checks its count and result against N, not against
  the `int` a narrow operand computes in: a count of N or more and a signed
  result past the sign bit are undefined and no constant (`(_BitInt(31))3
  << 30`, which both compilers warn "requires 33 bits", and
  `(unsigned _BitInt(8))1 << 8`, which Clang folds to 0x80 and GCC to 0,
  are refused as `1u << 32` is), while a nonnegative value shifted into
  (not past) the sign bit folds to its two's-complement result, as for
  `int` (`(_BitInt(8))1 << 7` is -128 in both compilers). Values of widths
  above 128 bits, up to `BITINT_MAXWIDTH`, fold exactly: `Integer` keeps a
  value `i128` holds in place, and any other names its two's-complement
  64-bit limbs, interned in the unit's table (`WideIntegers`, which
  `SemanticTranslationUnit::integer_limbs` reads for lowering). Every
  operator, conversion and context folds as at narrow widths, with the
  same rules at N bits: products are schoolbook and quotients Knuth's
  algorithm D (a release build checks 600 assertions with about 5,000
  operations on 65535-bit values in 0.6 s). A wide value converts to a floating type rounded
  once to nearest even (its top 128 bits from the leading one and a sticky
  bit, `bits::from_scaled`), and a floating value converts to a wide type
  truncated exactly (`double` reaches 2^1024, `long double` and
  `__float128` 2^16384); out of range it is no constant, as for narrow
  types. The overflow builtins compare the exact result with the
  destination's range at any width.
- **Bit-fields** take a `_BitInt` of at most its width; the record lays the
  field out in storage units of the declared type, as both compilers do.
- **Overflow builtins** accept bit-precise operands and results and check
  the result's width N.
- **Atomics.** Clang rejects `_Atomic _BitInt(N)` ("_Atomic cannot be
  applied to integer type"), the `__atomic` builtins on a `_BitInt`
  object except `__atomic_test_and_set` and `__atomic_clear` ("argument to
  atomic builtin of type '_BitInt' is not supported"), and the `__sync`
  builtins unless N is a power of two ("atomic memory operand must have a
  power-of-two size"); the Clang identity does the same, with Clang's
  wording. GCC accepts all of them, and its results wrap at the width. Both
  refuse a `__sync` builtin on a `_BitInt` of more than 16 bytes (Clang:
  "address argument to atomic builtin must be a pointer to 1,2,4,8 or 16
  byte type").
  Two refusals predate `_BitInt` and hold for every type under every
  identity: an `_Atomic` object in the `__atomic` builtins, which GCC
  accepts and Clang refuses, and `__atomic_clear` on an object other than
  `_Bool` or a character type, which both accept (storing one byte).
- **`_Generic` of a bit-field.** GCC types the bit-field `_BitInt(40) a : 10`
  as `_BitInt(10)` for `_Generic`; bcc, like Clang, keeps `_BitInt(40)`.

Lowering holds a `_BitInt` of up to 128 bits in the IR integer of its
storage (`i8` to `i128`), extended from N bits by its signedness after
every operation that can reach the bits above N, atomic ones included
(`Repr::narrow`, `FunctionLowerer::normalize`), so every IR operation reads
its value and the back ends need nothing new. Static initializers store the
value extended from N bits too. The padding bits are implementation-defined
(§6.2.6.2p1-2) and the psABI leaves them unspecified. Both compilers store
them extended in memory; in registers GCC extends a result to its type
(a `_BitInt(7)` only within its byte) and Clang returns a signed
`_BitInt(33)` zero-extended to 64 bits and the high word of a
`_BitInt(65)` unextended, and passes `_BitInt(N)` of up to 16 bits
`signext`/`zeroext` as `char` and `short`. Neither reads them: GCC
re-extends every incoming value. bcc stores and passes values extended and
extends again everything it reads (loads, parameters, call results,
`va_arg`), so it interoperates with both.

Calls follow each target's convention as Clang implements it:

| Width N | System V x86-64 | Microsoft x64 | WebAssembly |
| --- | --- | --- | --- |
| up to 64 | as the integer type of its size | as the integer type of its size | as the integer type of its size |
| 65-128 | two general registers, else the stack 8-aligned (unlike `__int128`'s 16); results in RAX:RDX | by reference to a caller copy; results through `sret` | as an `i128` (two `i64`) |
| 129 and up | in memory, as an array of limbs | by reference; `sret` | not available (Clang's limit is 128) |

`lowering/bit_int.rs` gives the x86-64 conventions' 65-128-bit values to ABI
lowering as the structure of their two limbs, storing and loading them at
the call (`types::limb_argument`).

A `_BitInt` of more than 128 bits has no IR type: lowering keeps it in
memory as the psABI lays it out, like a structure
(`lowering/wide_bit_int.rs`), so objects, members, arrays, calls, results
and `va_arg` take the aggregate paths, which pass it as the table above
says. Its value is the address of an object holding it, padding bits
extended; each operation writes a fresh object. Conversions to and from
integers of at most 128 bits load or build the low limbs inline; the
arithmetic, bitwise, shift and comparison operators, `++`/`--`, compound
assignments, conversions between wide widths and to and from the real
floating types, `switch` (a comparison with each case), bit-fields and the
overflow builtins call routines of a C runtime,
`lowering/wide_bit_int/runtime.c`. The middle end compiles it with bcc for
the unit's target whenever a module calls one of its routines and moves
the routines the module reaches into it as internal functions (named
`bcc.bitint.*`, which no C identifier can spell, so a program's own
`__bcc_bitint_add` keeps its name and meaning), so every
back end (the interpreter, LLVM and the native back end) runs the same
code and no program needs libgcc's `__mulbitint3`, `__divmodbitint4`,
`__floatbitint*` and `__fix*bitint`, which GCC 13's libgcc lacks. The
routines read only an operand's value bits, so a value whose padding
another compiler left unextended (GCC 15 does, in memory, for some
results) computes correctly. Division uses Knuth's algorithm D on 64-bit
digits; a conversion to a floating type rounds once, from the value's top
128 bits and a sticky bit; `_Float16` goes through `float`, which loses
nothing where binary16 does not overflow, and `__bf16` rounds once for GCC
and through `float` for Clang, as each compiler converts (and as bcc
converts `__int128`). An atomic compound
assignment loops on a compare-exchange of the object's bytes. A result's
stack slot is freed once an operation has read it, and every slot at the
next statement, so a chain of operations in one expression takes a slot or
two and a balanced tree one per level (a 1024-term `_BitInt(65535)` chain
needs three); a function whose results would still need more than 6 MiB of
stack is refused ("not yet supported by lowering: functions whose wide
_BitInt results need more than 6 MiB of stack"). The runtime's division
works in place in two arrays of the operands' limbs, 2 MiB at Clang's
widest type, 8388608 bits; its conversions to floating types take one. LLVM never
sees a wide `iN`, so its legalization of very wide integers does not
apply. A conversion between a wide `_BitInt` and a complex type is refused
("not yet supported by lowering: conversions between complex types and
_BitInt types of more than 128 bits").

Two Clang 23 conversions of wide values to x87 `long double` are wrong,
and bcc converts as GCC does under both identities: an `unsigned
_BitInt(N)` of more than 128 bits whose top bit is set becomes negative
(also in `__float128`), and at `-O0` some values close to halfway between
two `long double`s, signed ones too, round one ulp the wrong way (15 of the
780 conversions of the review probe `cl50e.c`). `tests/integration/bit_int.rs` compares every operator, conversion and
folded constant at widths 1-128 (`bit_int.c`) and at widths 129, 192, 255,
256, 257, 512, 1000 and 4096, and 65535 for GCC (`bit_int_wide.c`, from
`scripts/generate_wide_bit_int_program.py`), with the bundled Clang and GCC
15 on every back end, and with wasi-sdk's Clang on `wasm32-wasip1` for
widths up to 128; it links calls between bcc, Clang and GCC objects at
both sizes (`bit_int_abi.c`, `bit_int_wide_abi.c`).

### Imaginary interchange constants

An imaginary suffix (`i`, `I`, `j` or `J`) combines with an interchange
suffix in either order (`1.0f16i`, `1.0if16`, `0x1p0jF16`), as with the
standard suffixes (`ImaginaryConstants`). The constant has the complex type
of its real component's, and its imaginary part has the value a real
constant of that suffix has:

| Suffix | GCC | Clang |
| --- | --- | --- |
| `f16` | `_Complex _Float16`, its part in `float`'s precision (`__imag__ 0.1f16i` is `0x1.99999ap-4`) | `_Complex _Float16`, its part rounded to `_Float16` (`0x1.998p-4`) outside chains of arithmetic operators |
| `f32`, `f64`, `f32x`, `f64x`, `f128` | `_Complex _FloatN`/`_FloatNx` | invalid suffix, as for the real constants |
| `bf16` | a complex `__bf16` without a name | invalid suffix |

bcc has no complex `__bf16` type (#164 rejects `_Complex __bf16`), so it
still rejects `1.0bf16i` under the GCC identity. An invalid constant
(`1.0f32i` under Clang) reports only its error. Under `-pedantic` the
constants report the imaginary extension (GCC: "imaginary constants are a
C2Y feature or GCC extension", Clang: `-Wc2y-extensions`) and, under GCC
before C23, the suffix. Arithmetic, conversions and storage use the
existing complex `_Float16` lowering of #164, with each identity's excess
precision. `tests/fixtures/programs/imaginary_interchange_constants.c`
compares the values with each compiler on the interpreter, LLVM and the
native back end; `preprocessing/tests/language_modes.rs`,
`semantic_analysis/tests/imaginary_constants.rs` and the
`sema-imaginary-interchange-{gcc,clang}` goldens cover the forms.

### Floating values converted out of an integer's range

Converting a floating value whose integral part an integer type cannot
represent is undefined (C99 §6.3.1.4p1). bcc refused such a static
initializer (`static int x = 1e10;`); both compilers fold the conversion to
the nearest bound of the integer, or to 0 for a NaN, and store that:

| | GCC | Clang |
| --- | --- | --- |
| stored value | the nearest bound; a bit-field saturates to its own width (`100.0` in an `int : 4` stores 7) | the nearest bound of the declared type, whose low bits a bit-field keeps (`100.0` stores 4) |
| scalar static initializer | stored | refused by Clang's code generation ("cannot compile this static initializer yet"); bcc stores the value its evaluator computes, as Clang does inside braces |
| bit-field of static storage | stored | error, "initializer element is not a compile-time constant", once per initializer |
| warning | `-Woverflow`, default on, for an implicit conversion of any floating constant in a static initializer only ("overflow in conversion from 'double' to 'int' changes value ...") | `-Wliteral-conversion`, default on and under `-Wconversion`, for an implicit conversion of a floating literal (possibly signed and parenthesized) to a builtin integer type (not an enumeration, not `_Atomic`) anywhere: out of range ("... is undefined"), with a fraction dropped, -0 included (`int i = 2147483647.5;`, `int i = -0.0;`), or to `_Bool` from neither 0 nor 1 |
| `-pedantic` | "overflow in constant expression" `[-Woverflow]` for every overflowing static element, explicit casts and arithmetic on them included (`(int)-1e10 / 2`) and the operand `?:` selects, but not a comparison's result (`(int)1e10 == 2147483647`), a `?:` condition or unselected operand, or a conversion to `_Bool`; an error under `-pedantic-errors` | nothing more |
| `__builtin_constant_p((int)1e10)` | 0 | 1 |

The stored values match each compiler's object file on the interpreter,
LLVM and the native back end
(`tests/fixtures/programs/float_integer_initializers.c`), from `float`,
`double`, `long double` and NaN or infinite operands into every integer
width up to `__int128`. Such a conversion is no integer constant
expression in either identity: an enumerator, array bound, bit-field width
or `_Static_assert` rejects it, as GCC does, where Clang folds the
saturated value. A conversion in function code keeps its run-time
evaluation. GCC's pedantic warning for signed integer overflow in a static
initializer (`(int)1e10 + 1`) and Clang's `-Winteger-overflow` and
`-Wbitfield-constant-conversion` are not reproduced; bcc still rejects the
first, and its `__builtin_constant_p((int)1e10 + 1)` is 0 where Clang's is 1.

GCC's pedantic report is a pedantic child of `-Woverflow` (as GCC's
`pedwarn` with `OPT_Woverflow` is): only `-pedantic` reports it, and
`-Wno-overflow`, `-Werror=overflow` and `-Wno-error=overflow` disable it or
set its severity, so `-pedantic-errors -Wno-overflow` accepts
`static int a = 1e10;`. bcc's reporter folds a second error at one
location into the first, so under `-pedantic -Werror=overflow` only the
conversion error is printed there, though both count. Under Clang's rules
`-Wconversion`'s parent group takes `-Werror=conversion` and
`-Wno-error=conversion` too, for the groups without options of their own;
GCC's `-Wconversion` holds none of Clang's groups, so under GCC's rules it
leaves them (`-Wpointer-bool-conversion` included) alone. `Analyzer::saturated_float_cast` computes the value,
`Analyzer::static_float_conversion` stores it and warns for GCC, and
`Analyzer::literal_conversion_warning` warns for Clang; the
`sema-float-integer-{gcc,clang}` goldens and
`semantic_analysis/tests/float_integer_conversions.rs` pin the rules.

### Arrays initialized by array compound literals

C requires a braced list or a string literal for an array (C99 §6.7.8p16);
both compilers also accept a compound literal of the array's own type,
possibly parenthesized, as if its braced list were written in its place
(`ArrayCompoundLiteral`, GNU origin):

| | GCC | Clang |
| --- | --- | --- |
| storage | static only; an automatic array is an error ("array initialized from non-constant array expression", or "invalid initializer" when the literal has side effects) | any; a literal with side effects (`(int[2]){i++, 2}`) is an error |
| types | compatible once each element type's qualifiers, `_Atomic` included, are dropped | the same element type |
| bounds | equal, or the object's unknown (`char a[] = (char[3]){"ab"}` completes it) | the same |
| inside braces (`{ (char[3]){"ab"}, 1 }` for a member, a row of a 2-D array) | the element or member is initialized whole | the literal decays and converts to the first character (`-Wint-conversion`) |
| `-pedantic` | "initializer element is not constant", bcc's `ConstantCompoundLiteral` report | `-Wgnu-compound-literal-initializer`, bcc's `ArrayCompoundLiteral` report |

A size or element-type mismatch (`char a[4] = (char[3]){"ab"}`, `char a[3]
= (unsigned char[3]){"ab"}`) is an error in both. GCC's additional
pedantic "array initialized from parenthesized string constant" for a
member initialized by a string compound literal is not reproduced. Only the
object's own unknown bound completes from a literal: a flexible array
member inside braces takes none (`struct F f = { 1, (int[2]){7, 8} }`),
and both compilers reject it, as bcc does after `-Wint-conversion`. A
static object takes the literal's bytes; an automatic one copies the
literal's object.

A string literal longer than its array, null character aside, stores the
characters that fit and is a warning in both compilers that
`-pedantic-errors` makes an error (GCC prints it without an option, Clang
under `-Wexcess-initializers`); bcc rejected it before. This covers string
literals inside a compound literal (`(char[3]){"abcd"}`) too.
`Analyzer::copy_array_literal` holds the rules;
`tests/fixtures/programs/array_compound_literals.c` runs the accepted forms
on every back end against each compiler (`tests/integration/identity_programs.rs`),
and the `sema-array-compound-literal-{gcc,clang}` goldens and
`semantic_analysis/tests/array_compound_literals.rs` cover the rejections.

### Casts between pointers and integers of another width

A cast between a pointer and an integer whose width differs from the
pointer's is implementation-defined and may not be representable (C99
§6.3.2.3p5-6), and both compilers warn by default:

| Cast | GCC | Clang |
| --- | --- | --- |
| integer to pointer | `-Wint-to-pointer-cast` when a plain integer type (not `_Bool`, not an enumeration) has any other width and the operand is not constant; an address constant (`(int)(long)&x`) counts as constant | `-Wint-to-pointer-cast` when the integer type (not `_Bool`, not an enumeration) is narrower and the operand is no integer constant expression; `-Wint-to-void-pointer-cast` for a `void *` destination |
| pointer to integer | `-Wpointer-to-int-cast` for any other width of a plain integer type | `-Wpointer-to-int-cast` for a narrower integer other than `_Bool`; `-Wpointer-to-enum-cast` for an enumeration, and `-Wvoid-pointer-to-int-cast`/`-Wvoid-pointer-to-enum-cast` from a `void *` |

So GCC warns for `(char *)q` with an `__int128 q` and for `(__int128)p`,
Clang does not; Clang warns for `(enum E)p`, GCC does not. An operand that
reads an object even in an unevaluated arm (`(char *)(1 ? 2 : i)`) or a
compound literal (`(void *)(int){0}`) is not constant for either. Clang's
groups nest: `int-to-pointer-cast` holds `int-to-void-pointer-cast`, and
`pointer-to-int-cast` holds `pointer-to-enum-cast` and
`void-pointer-to-int-cast`, which both hold `void-pointer-to-enum-cast`;
a parent's `-W`, `-Wno-` or `-Werror=` applies to its children at its
position on the command line, so `-Wno-int-to-void-pointer-cast
-Werror=int-to-pointer-cast` makes both errors again. GCC has only the two
parent groups, which control its warnings whatever the child options say.
GCC also withholds the warning for a cast spelled in a macro defined in a
system header (`-isystem`, `#pragma GCC system_header`), where Clang reports
it at the user's expansion; each identity follows its compiler. The rules
are in
`Analyzer::pointer_integer_cast_warning`; the
`sema-pointer-int-cast-{gcc,clang,options,system-gcc,system-clang}` goldens
and
`semantic_analysis/tests/pointer_integer_casts.rs` pin them.

## Type-generic bit builtins and `<stdbit.h>` (2026-10-29)

`__builtin_popcountg`, `__builtin_clzg` and `__builtin_ctzg` (with their
optional result for zero), and the `__builtin_stdc_*` forms of C23's
`<stdbit.h>` functions with GCC 15's `__builtin_stdc_rotate_left` and
`__builtin_stdc_rotate_right`, exist for every identity under `GnuBuiltins`,
as GCC 15 and Clang 23 provide them. GCC 14's `__builtin_parityg`,
`__builtin_ffsg` and `__builtin_clrsbg` exist for the GCC identity only
(`Feature::GccOnlyBuiltins`, which also gates `__builtin_shuffle` and
`__builtin_iseqsig`), as Clang 23 lacks them. Each computes at its
operand's width, for every unsigned integer type up to `unsigned __int128`
and every `unsigned _BitInt(N)`. The registry shard is `src/builtins/bits.rs`;
`semantic_analysis/builtins/bits.rs` types, checks and folds the calls.

Arithmetic on a bit-field wider than `int` follows the identity too.
GCC gives a field of width N, between `int`'s width and its declared type's,
a type of that width, which converts as `_BitInt(N)` does: with
`unsigned long long u40 : 40`, `u40 + 1`, `u40 + 1u`, `u40 << 8`, `-u40` and
`~u40` are computed in 40 bits, `u40 + 1ull` in 64, and a variadic argument
passes the value at the declared type's width. Clang promotes the field to
its declared type. A field at most as wide as `int` promotes to `int` under
both. bcc does not model GCC's distinct type in `_Generic`, which matches an
unpromoted wide field against its declared type.

The identity's rules (`bit_builtin_rules`) decide what each accepts, both
compilers having been probed:

| Rule | GCC 15 | Clang 23 |
| --- | --- | --- |
| Operand | an unsigned integer type that is neither `bool` nor an enumeration; a signed one for `ffsg` and `clrsbg` | an unsigned integer type; `bool` and enumerations also for `popcountg`, `clzg`, `ctzg` and the rotations, at 1 bit or the underlying type's width |
| Bit-field operand | at the field's width (a `bit_ceil` of 5 in `unsigned : 3` is 0) | at the declared type's width |
| Result for zero | `int` after the default argument promotions, a `_BitInt` narrower than `int` (or a signed one as wide) included | `int` after the integer promotions |
| Rotation count | any integer type but an enumeration; a negative count is undefined | any integer type; a negative count turns the other way |
| `clzg`/`ctzg` of constant zero without a result | folds to the width | no constant |

A call whose operands are integer constant expressions is one at every
width. bcc reduces a negative rotation count as Clang does under both
identities, and folds a rotation of every `_BitInt` (GCC 15 does not fold
one of `unsigned _BitInt(N)` in a static assertion for some widths that no
standard type has, such as 2, 7, 63, 65 and 127). An enumeration with a fixed
underlying type, which bcc does not lower, is refused wherever GCC or Clang
refuses any enumeration. Not modeled: GCC's
`-Wshift-count-negative` warning for a negative constant count, and GCC's
bit-field type as a result type (bcc gives a value-returning form on a
bit-field the field's declared type; the value is GCC's). The misuse
diagnostics are `BitBuiltinNotInteger`, `BitBuiltinSigned`,
`BitBuiltinUnsigned`, `BitBuiltinBoolOrEnum`, `BitBuiltinFallback` and
`BitBuiltinRotateCount`, with the existing wrong-argument-count error. Clang
23 crashes ("unsupported library call operation") rotating a `_BitInt` wider
than 128 bits by a count that is not a constant; bcc computes it.

Lowering (`lowering/builtins/bits.rs`) derives every operation from the
ones, leading and trailing counts of the operand: at most 128 bits from the
IR's `popcount`, `clz_defined` and `ctz_defined` on the value's IR type,
corrected for the bits above its width; wider through new routines of the
`_BitInt` runtime (`popcount`, `leading`, `trailing`, `clrsb`, `bit_floor`,
`bit_ceil`, `rotate`, `residue`). `clzg` and `ctzg` without a result for zero
use the IR's `clz` and `ctz`, undefined at zero. The native back end now
selects `i128` bit counts from its halves' counts; bcc has no CPU-feature
model, so native code keeps its baseline `bsf`/`bsr` and SWAR sequences
rather than `popcnt`, `lzcnt` or `tzcnt`. `tests/integration/stdbit.rs` runs
`tests/fixtures/programs/stdbit{,_gcc,_clang}.c` on every back end against
Clang 23 and GCC 15 (and wasi-sdk's Clang for wasm32), and the
`sema-stdbit-{gcc,clang}` goldens pin the misuse diagnostics.

## GNU complex integer types (2026-10-30)

`_Complex` combines with the integer type specifiers in any order
(`_Complex int`, `unsigned short _Complex`, `long _Complex`, `_Complex long
long`), as GCC and Clang accept it: the parser wraps the integer specifiers
in `ExtendedType::ComplexInteger`, and semantic analysis gives
`Scalar::ComplexChar` through `Scalar::ComplexUnsignedLongLong`, laid out as
an array of two of the real type. `-pedantic` reports "complex integer
type", under the GNU complex-number feature `ImaginaryConstants`. Integer
imaginary constants (`2i`, `2li`) have the complex type of their real
constant's type, as in both compilers. `_Complex _Bool` is refused, as both
refuse it; `_Complex __int128`, which GCC accepts and Clang refuses, is
refused under both identities ("cannot combine"), and plain `_Complex`
(GCC's `_Complex double`) stays an error. A repeated `_Complex` is an error
under both identities, as in GCC; Clang 23 only warns
(`-Wduplicate-decl-specifier`), as for `_Complex _Complex double`, which bcc
already refused.

Typing follows both compilers: a complex integer operand takes part in the
usual arithmetic conversions through its real type unpromoted
(`_Complex char + _Complex char` is `_Complex char`), a real integer
operand promotes (`_Complex char + char` is `_Complex int`), and a floating
operand makes the result complex floating. Of two parts of equal rank, the
unsigned one's type wins; of equal signedness too (`_Complex signed char`
and `_Complex char`), Clang takes the left operand's type and GCC the
left's only if it is unsigned, else the right's (its `c_common_type`). `~` on any complex value is its
conjugate (also for complex floating types, a GNU extension Clang shares),
`!` and conditions test both parts, `==` and `!=` compare both, and
`__real__` and `__imag__` are lvalues. Conversions go part by part:
an integer part wraps, a floating part truncates.

Division differs between the compilers and follows the identity
(`CompilerConfiguration::complex_integer_rules`): GCC divides by the
divisor part of larger magnitude first (`expand_complex_div_wide`:
with `|c| < |d|`, `r = c / d`, `t = c r + d`, the quotient is
`((a r + b) / t) + ((b r - a) / t)i`, and symmetrically), Clang computes
`(ac + bd) / (cc + dd)` and `(bc - ad) / (cc + dd)`; both in the parts'
type, wrapping. GCC divides each part by a real divisor; Clang applies its
formula to it as `c + 0i`, so `(100000 + 7i) / 70000` is `1` under GCC and
`-2` under Clang, whose `cc` wraps. Constant
expressions fold with the same rules, the parts held exactly in the floating
carrier, so a complex integer constant initializes a static object.

Not modeled: `++` and `--` on complex values, `_Atomic` complex integers
(refused by lowering), and the `-pedantic` diagnostic for `~` as conjugate
under the Clang identity (reported as "`~` for complex conjugation" under
the `RealImag` feature for both).

## GCC library and scalar builtins (2026-10-30)

A survey of GCC 15.2's run tests under `--gcc-mode` refused 67 files with
"a type built from an extension semantic analysis does not model". None was
an extension type: each called a `__builtin_` name the registry had no
contract for, which semantic analysis binds as an opaque intrinsic of
unknown result. About 35 are the `<math.h>` comparison builtins and
`__builtin_roundeven`, which #190 registered since. The rest, by file: GCC's library aliases
(8), `_FloatN` math aliases folded in static initializers (6, below),
`__builtin_apply` and its siblings (5), `__auto_type` (3),
`__builtin_bswap128` (3), the `q` and `f16` constructors (2),
`__builtin_conjf` (2), `__builtin_assoc_barrier` (2), and one each for
`__builtin_addc`, `__builtin_alloca_with_align`, `__builtin___clear_cache`,
`__builtin_clear_padding`, `__builtin_powi`, `__builtin_call_with_static_chain`
and two `__builtin_ia32_*` names no header declares.

Now registered, with the availability `__has_builtin` reports under each
identity (GCC 15 and Clang 23 checked):

- Library aliases, lowered as calls of the C library function: `strcat`,
  `strncat`, `strstr`, `strspn`, `strcspn`, `strpbrk`, `strdup`, `strndup`,
  `stpcpy`, `stpncpy`, `mempcpy`, `bcopy`, `bzero`, `bcmp`, `index`,
  `rindex`, `strcasecmp`, `strncasecmp`, `realloc`, `labs` and `llabs` for
  both identities; `strnlen`, `aligned_alloc` and the `<ctype.h>`
  classifiers and `toupper`/`tolower` for GCC only (Clang knows those names
  as library functions but not their `__builtin_` aliases). Their ordinary
  names take the library type when called undeclared in a hosted mode, as
  the other aliases' do; the POSIX and BSD ones only in GNU modes, as GCC's
  `DEF_EXT_LIB_BUILTIN` makes them.
- `__builtin_addc`, `__builtin_subc` and their `l` and `ll` forms (GCC 14
  and Clang): the carry or borrow of either step, stored through the
  fourth operand.
- `__builtin_alloca_with_align` (both) and
  `__builtin_alloca_with_align_and_max` (GCC): `alloca` storage aligned to
  the constant power-of-two bit count, at least 16 bytes.
- `__builtin___clear_cache` (both), which needs nothing on x86.
- `__builtin_creal`, `__builtin_cimag` and `__builtin_conj` with their `f`
  and `l` forms (both), on the operand converted to the complex type.
- `__builtin_bswap128` and `__builtin_assoc_barrier` (GCC). A byte swap of
  a constant now folds, so `__builtin_bswap64 (1)` initializes a static
  object, as both compilers allow.
- `__builtin_fabsf16`, `__builtin_copysignf16`, `__builtin_nanf16`,
  `__builtin_nansf16`, `__builtin_inff16` and `__builtin_huge_valf16`
  (both), `__builtin_nansf16b` (GCC; GCC 15 has no other `f16b` builtin),
  and GCC's `q` spellings `__builtin_nanq`, `__builtin_nansq`,
  `__builtin_infq`, `__builtin_huge_valq`, `__builtin_fabsq` and
  `__builtin_copysignq`. `__builtin_fabsf128` and `__builtin_copysignf128`
  moved from the binary128 primitives, which only folded, to sign
  operations, so a call with a run-time operand lowers too.

Deviations: bcc folds `__builtin_fabsq` and `__builtin_copysignq` in a
static initializer, where GCC 15 reports "initializer element is not
constant" (it folds the `f128` spellings), and folds the `f16` sign
builtins under the Clang identity, which Clang 23 does not. GCC's
`__builtin_fabsf16` of a signaling NaN gives a quiet NaN (it computes in
`float`); Clang and bcc clear only the sign bit, under both identities.

Operand checks follow GCC 14 and Clang 23: the carry pointer of
`__builtin_addc`/`subc` converts as a prototyped argument, so a pointer to
the signed type or to a `const` one draws `-Wpointer-sign` or the
discarded-qualifier warning and one to another width an error;
`__builtin_alloca_with_align` takes a constant power of two from 8 bits to
2^31 under GCC (2^30 under Clang, which allows at most 2147483647), and
`_and_max` a constant bound. `__builtin_bswap*` of any arithmetic constant
folds (an integer constant expression only if its operand is one);
`__builtin_assoc_barrier` folds a constant and, as GCC, takes pointers and
structures; and under the GCC identity `__builtin_creal`, `cimag` and `conj`
of a constant fold, as GCC folds them and Clang does not.

Still refused, now by name ("not yet supported by lowering:
`__builtin_apply_args`"): `__builtin_apply`, `__builtin_apply_args`,
`__builtin_return`, `__builtin_call_with_static_chain`,
`__builtin_clear_padding`, `__builtin_powi{,f,l}`,
`__builtin_speculation_safe_value` and `__builtin_stack_{save,restore}`;
any other unregistered `__builtin_` name is "a builtin this compiler does
not know". `__auto_type` and C23 `auto` inference are implemented since
(see "Type inference"). GCC's `_FloatN` aliases of the math
functions (`__builtin_acoshf32`, …) stay unregistered: `floatN-builtin.c`
needs them folded in static initializers with MPFR's correctly rounded
results, which bcc has no evaluator for.

## Variably modified members (2026-10-31)

GCC accepts a structure or union member of variably modified type in block
and prototype scope (`struct s { int a; int b[n]; int c; };`) and lays the
record out at run time when its declaration is reached: `sizeof`, the
offsets of the members after the variable one, copies and arrays of the
record all read the bounds evaluated there. 19 of GCC 15.2's C torture
tests use them (8 under `execute/`, 11 under `compile/`; found with
Clang's "fields must have a constant size" error), with more in
`gcc.dg`.

bcc's lowering maps every record to an IR aggregate of constant layout, so
it has no run-time record layout yet. Under the GCC identity, semantic
analysis now accepts these members where GCC does, as it accepts other
unmodeled types, and lowering refuses any declaration, expression or `asm`
operand that needs it, pointer arithmetic on a pointer to it included,
with "not yet supported by lowering: structures with variably modified
members", instead of reporting a constraint violation GCC does not have.
The record's type is variably modified, as GCC's `C_TYPE_VARIABLE_SIZE`
makes it, so what GCC rejects for a variable-sized type is rejected for it:
an initializer or compound literal of it, a `static` or `extern` object, a
function returning it, a jump or `switch` into the scope of an object of
it or of a pointer to it, and a `_Generic` association naming it. An array
of pointers to it (or to a VLA) has a constant size and may be initialized,
as in GCC; bcc used to reject `int (*a[2])[n] = {0, 0};`. Its members are
not modeled, though: `-fsyntax-only` accepts uses GCC rejects, such as a
member it lacks (`s.zz`), or the record as an arithmetic, scalar or
assignment operand (`s + 1`, `(int)s`, `s ? 1 : 0`, `s = 5`); code
generation then refuses them by name. At file scope GCC rejects them ("variably modified 'a' at file
scope"), and so does bcc. The Clang and clang-cl identities report
"fields must have a constant size" for every such member, as Clang does in
every scope; TCC and MSVC keep the C99 §6.7.5.2p2 error.

Of those 19 torture tests under `--compiler-identity=gcc`, all pass
analysis; 15 are refused by name at lowering, and the other 3 compile
because nothing of their records is lowered (`920501-16.c` only declares
the type, `pr77754-6.c` only names a pointer to it in a prototype, and
`20050801-2.c` uses it in an inline definition that is not emitted). One
(`pr39394.c`, an `asm` memory operand of the type) used to end in an
internal error, because analysis retains no `asm` facts for an unanalyzed
operand; lowering now refuses such an operand by its type, which also fixes
the same internal error for `_Decimal32` and other unmodeled operand types.

Left to do for full support: a run-time layout (offsets and size as size
formulas with sums and alignment), member access and copies through byte
offsets, arrays of and pointers to such records, `offsetof` with run-time
offsets, and passing by value (`va_arg` of such a type in
`20020412-1.c`). A record whose only variably modified member is a pointer
to a VLA (`int (*p)[n]`) has a constant layout and would need only the
member's bound evaluated where the record is declared.

## Type inference: C23 `auto` and GNU `__auto_type` (2026-10-31)

An object declared with C23 `auto` and no type specifier, or with GNU
`__auto_type` (every mode; GCC and Clang accept it in C89 too), takes the
type of its initializer after lvalue conversion and array and function
decay, qualified by the declaration's qualifiers (C23 §6.7.10p2): `const
auto k = c;` is `const int` for an `int` or `const int` `c`, `auto s =
"ab";` is `char *`, and `auto f = g;` a pointer to `g`'s type. A lone
`auto` is no storage-class specifier, so a file-scope `auto a = 3.5;` has
external linkage; `static`, `extern`, `register` and `_Thread_local` keep
their meaning beside it. Semantic analysis types the initializer before it
binds the object, then binds and initializes it as an ordinary declaration,
so every back end (the interpreter, LLVM, native and wasm32) lowers it
without change; the macros the GCC manual shows, such as
`({ __auto_type _a = (a); __auto_type _b = (b); _a > _b ? _a : _b; })`,
evaluate each argument once. Variably modified types keep their run-time
bounds (`__auto_type p = &vla;` reads the bound evaluated at `vla`'s
declaration), and complex, vector, `_BitInt`, `_Float16` and `__float128`
operands give those types; `_Float16` arithmetic gives `_Float16`, whatever
precision computes it.

The constraints and C23's implementation-defined forms follow the claimed
compiler, as GCC 13.3, GCC 15's `c-parser.cc` and Clang 23 behave:

| Declaration | GCC | Clang |
| --- | --- | --- |
| no initializer, or a braced one | error | error |
| more than one declarator | error at the second | each deduced; error if they differ |
| `*p`, `(*f)(int)`, `(*a)[3]` declarators | error ("plain identifier") | deduced through the derivations; error if the initializer does not match, an unsized `[]` matching only an unsized array |
| `auto __auto_type x = 1;` in C23 | `auto` is a storage class | error |
| array or function declarator | error | error |
| the name in its own initializer | C23 `auto`: error; `__auto_type`: the outer declaration | error for both |
| a bit-field member access, parenthesized or not | `__auto_type`: error; C23 `auto`: the bit-field's own type (refused by lowering) | error for both |
| another operand of a bit-field's type: `(0, s.b)`, `(s.b = 1)`, `++s.b`, `s.b++`, `({ s.b; })` | the bit-field's own type (refused by lowering) | its promoted type, usually `int` |
| an `_Atomic` operand | atomic dropped | `_Atomic` kept |
| a tag declared in a C23 `auto` initializer | error | accepted |
| C23 `auto` redeclaring a name of the same scope | error | error |
| `__auto_type` redeclaring one | ordinary redeclaration rules | ordinary redeclaration rules |
| in a parameter, member, type name, typedef or function definition | error | error |
| `auto` with another storage class and a type specifier | error | error |

Unit tests in `semantic_analysis/tests/inference.rs` pin each row under
both identities; `sema-auto-type-{gcc,clang}` render the diagnostics, one
per error the compiler reports for the same source. The
`auto_type.c` (GNU17) and `inferred_auto.c` (GNU23) programs in
`tests/integration/identity_programs.rs` run under both identities on every
arm, wasm32 included, and print what GCC 13.3 and Clang 23 print.

Deviations and limits:

- `constexpr auto` infers its type but stays refused with every `constexpr`
  object ("constexpr objects").
- GCC types a C23 `auto` object inferred from a bit-field with the
  bit-field's own type (`sizeof` 1 for a 2-bit field), which lowering
  refuses ("types inferred from bit-fields"); GCC 13 and 15 agree.
- Clang's `-Wauto-decl-extensions` pedantic warning for a deduced pointer or
  function declarator is not reported: the extension policy has no feature
  bit left for it.
- A file-scope `register auto` is an error under both identities; GCC 13
  accepts it without a diagnostic.
- Clang keeps the shadowing name visible inside a `__auto_type` initializer
  for typedef lookup too (`__auto_type T = (T)1;` is a syntax error there);
  bcc's parser follows GCC and publishes the name after the initializer
  under both identities.
- GCC 13 has no `_BitInt`; GCC 14 and later infer it as Clang does.
- Clang 23 reads C23 `auto` followed by a typedef name as an inferred
  declaration of that name (`typedef int T; auto T x = 1;` is an error, and
  `auto T = 1.0;` declares `T`), and rejects `auto _Atomic x = 1;` while it
  accepts `_Atomic auto x = 1;`; bcc's parser takes the typedef name as the
  type under both identities, as GCC does, and accepts both `_Atomic`
  orders.
- An attribute after the declarator (`auto x __attribute__((unused)) = 1;`,
  `auto x [[maybe_unused]] = 1;`) used to be misread as an unknown type name
  followed by a declarator; after a storage-class specifier the identifier
  is now the declarator, as in GCC and Clang.
