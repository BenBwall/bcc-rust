# Driver modes: GCC, Clang, MSVC, clang-cl, TCC, zig cc and icx personalities

bcc can stand in for another C compiler on the command line. A **driver
personality** reads the command line with that compiler's conventions,
translates it into bcc's own options, and turns on every compatibility feature
bcc has for that compiler: its predefined identity macros, language mode,
target, system headers, Microsoft extensions, diagnostic format, output
names and version banners. Build systems that probe a compiler (Autoconf,
CMake, Meson, plain Makefiles) find the compiler they expect ([Build
systems](#build-systems) records what each concludes), while every banner
and `__bcc__` still say that bcc is the compiler.

This document records the conventions of each mimicked compiler, how bcc
maps them, the decisions made, and every known deviation. The implementation
is `src/cli/personality.rs` and its submodules. The driver they translate
onto (`src/cli/driver.rs`) is bcc's own and is documented in the README.

## Selecting a personality

| Flag | Executable names | Compiler mimicked |
| --- | --- | --- |
| `--gcc-mode` | `gcc`, `cc`, `c89`, `c99` | GCC 14.2 |
| `--clang-mode` | `clang` | Clang 23.1 (the pinned Clang) |
| `--msvc-mode` (alias `--cl-mode`) | `cl` | MSVC 19.33 `cl.exe` |
| `--clang-cl-mode` | `clang-cl` | clang-cl 23.1 |
| `--tcc-mode` | `tcc` | TCC 0.9.27 |
| `--zig-cc-mode` | `zig-cc`, `zigcc`, `zig` given `cc` first | zig 0.17.1's `zig cc` (Clang 22.1.8) |
| `--icx-mode` | `icx` (off Windows), `icx-cc` | Intel oneAPI DPC++/C++ Compiler 2026.2.0's `icx` (Clang 23.1.1) |
| `--icx-cl-mode` | `icx-cl`, and `icx` on Windows | the same compiler's `icx-cl` |

`--driver-mode=NAME` (Clang's spelling) is accepted too. The first mode flag
anywhere on the command line wins and is removed before the rest is read.
Without a flag, the executable's name selects the personality, as busybox
and Clang do: a symbolic link or copy named `gcc` behaves as GCC. The name may
carry a prefix and a version: `bcc-gcc`, `gcc-14`, `clang-18.1`, `cl.exe`.
A prefix that names a target selects it, as Clang's driver does:
`x86_64-w64-mingw32-gcc` compiles for MinGW-w64, `x86_64-linux-musl-gcc` for
musl. `c89` and `c99` select `-std=c89` and `-std=c99`, as the POSIX
utilities do. Any other name, including `bcc-rust`, runs bcc's own command
line.

## Constants that overflow

Under `--gcc-mode` and `--clang-mode`, the `--compiler-extensions` they pass
makes signed overflow, a bad shift count and the like in a constant expression
fold to each compiler's value with its warning, where plain bcc keeps refusing
them as C99 §6.6p4 lets it. `2147483647+1` wraps in both, `1<<40` is 0 for GCC
and -2147483648 for Clang, `1<<-1` is no constant for GCC and 0 for Clang,
and `1/0` is no constant for either. A negative or too large shift count and a
zero divisor also warn in code whose left operand is not constant. The tables
of values, contexts and warning options are in
[language-standards.md](../../language-standards.md#exceptional-integer-constants-2026-11-07).
An address's truth folds the same way (`int i = !&g;`, `&g ? 1 : 2`), with each
compiler's rules for weak symbols and for what a static `_Bool` takes
([language-standards.md](../../language-standards.md#an-addresss-truth-in-a-constant-2026-11-07)).

## `-pedantic` and what each compiler refuses

bcc's own `-pedantic` reports every extension. Under `--gcc-mode` and
`--clang-mode` it says what the compiler says instead: `typeof` is silent in GCC's
`gnu` modes, `__float128` in both, and an active `#foo` is an error in both, where
plain bcc warns. GCC refuses `for (int i...)` before C99 and `int a[] = {};`;
Clang refuses `_Decimal64` and `#assert`. The table is in
[language-standards.md](../../language-standards.md#-pedantic-under-a-personality-2026-11-07).

## Warning options

Each personality reads the warning options as its compiler does, with the
same words and exit status, checked on 2026-11-19 against GCC 14 (and GCC
13's driver), the pinned Clang 23.1.5, clang-cl and TCC 0.9.27, with and
without another diagnostic in the unit
(`tests/integration/warning_options_cli.rs`).

**GCC.** The driver refuses an empty `-Wno-` (`unrecognized command-line
option '-Wno-'`), a joined option without its value (`missing argument to
'-Werror='`, also `-Wno-error=`, `-Wformat=` and every other `-W…=` GCC
knows), a bad level for `-Wformat=` or `-Wunused-const-variable=`
(`argument to '-Wformat=' is not between 0 and 2`, `... should be a
non-negative integer`; `02` is 2), a `-Wnormalized=` form it does not have
(with its note listing `id nfc nfkc none`), and the `no-` form of a joined
option (`-Wno-format=2`). A missing argument is reported where it stands,
the unrecognized options once the command line has been read. A `-W` name
GCC does not know is unrecognized even when bcc implements it for Clang
(`-W#warnings`, `-Wmain-return-type`), and `-Weverything` is one. An
unknown `-Wno-` option is ignored, as is one that is Clang's only
(`-Wno-#warnings` leaves `#warning` reported), and after the diagnostics of
a unit that has any, GCC's compiler notes each, last first, after `At top
level:` when the last diagnostic was in a function:

```text
cc1: note: unrecognized command-line option '-Wno-foo' may have been intended to silence earlier diagnostics
```

Its compiler refuses a `-Werror=` or `-Wno-error=` naming no warning
(`cc1: error: '-Werror=foo': no option '-Wfoo'`; `-Werror=no-shadow` looks
for `-Wno-shadow`), and an empty `-Wp,` operand, which it takes for a second
input (`cc1: error: too many filenames given; type 'cc1 --help' for usage`),
once for each C source, before compiling it, so nothing is compiled. A unit
whose warnings `-Werror` made errors ends with `cc1: all warnings being
treated as errors`, or `some` when only a `-Werror=` did (also after
`-Werror -Wno-error`); `-Wfatal-errors` ends one with `compilation
terminated due to -Wfatal-errors.` before that line. An empty `-Wa,` passes
nothing to the assembler and draws no warning.

**Clang.** The front end reads the warning options its driver passes, once
for each source it compiles, as `ProcessWarningOptions` does: it warns about
each it does not know (`warning: unknown warning option '-Wfoo'
[-Wunknown-warning-option]`), before the unit's other diagnostics and
counted with them (`1 warning generated.`), and suggests the group at the
least edit distance from a misspelled one (`did you mean '-Wshadow'?`), as
`DiagnosticIDs::getNearestOption` does over the groups that hold warnings,
frozen from the pinned Clang's `DiagnosticGroups.inc` in
`options/clang-warning-groups.list`: none when two are as near, or none is
nearer than the name is long. It names a `-Wno-error=` group `-Werror=`;
`-Werror-x`, `-Wno-error-x` and `-Wfatal-errors=` are `unknown -Werror
warning specifier: '-Werror-x'` (`-Werror-implicit-function-declaration`
stands); `-Wno-no-shadow`, `-Wformat=1`, `-Wformat=` and `-Wpass` (a group
of remarks) are unknown. The warning follows the warning options:
`-Wno-unknown-warning-option` and `-w` silence it, and `-Werror` or
`-Werror=unknown-warning-option` make it an error
(`[-Werror,-Wunknown-warning-option]`), which leaves the unit without an
object and the status 1. `-Wsystem-headers`, `-Wfatal-errors=GROUP` and an
empty `-Wno-` are taken. `-Weverything` turns on every warning no option
has set, the pedantic ones included, and `-Wno-everything` turns every one
off, so that a later `-Weverything` leaves them off but their own options
turn them on again. `-Wfatal-errors` makes the first error a `fatal error`,
without the `-Werror,` in the option it names, and nothing after it is
reported or counted; an error a diagnostic pragma made stays an error. The personality passes the options Clang does not
take apart from the others, so they have no effect even where bcc has the
group (`-Wnormalized`, bcc's own `-Wconstant-promotion`); bcc's own command
line keeps its groups and warns, in Clang's words, about the options it does
not implement.

**Pedantic options.** Each personality classifies an extension diagnostic no
diagnostic pragma governs as its compiler reads `-pedantic`, `-Wpedantic`,
`-pedantic-errors`, `-w`, `-Werror` and their group forms
(`language-standards.md`, "Implementation notes"), checked on 2026-11-20 in
every order of pairs and triples against Clang 23.1.5 and GCC 14 for a
zero-length array, `_Static_assert`, a binary constant, `__VA_OPT__`, the four
C23 extensions Clang warns of by default, `#elifdef`, a `void` expression
returned and arithmetic on `void *` (`tests/integration/pedantic_options_cli.rs`).
Clang's `-w` silences what `-pedantic-errors` made an error, its
`-Wno-c23-extensions` silences the C23 extensions it warns of by default, its
`-Wno-error=pedantic` and `-Wno-pedantic` leave those errors under
`-pedantic-errors`, and its `-Werror=c11-extensions`, `-Werror=c23-extensions`
and `-Werror=pedantic` make their diagnostics errors; `-pedantic` itself is read
apart from `-Wpedantic`, so `-Wno-pedantic -pedantic` reports nothing. GCC's
`-w` silences its pedantic warnings before `-pedantic-errors` can make them
errors, its `-Werror=pedantic` enables them as errors (`[-Werror=pedantic]`,
`cc1: some warnings being treated as errors`), and its `-Wno-error=pedantic`
keeps them warnings where they come after the other two, though not GCC's
`-Wpointer-arith` and `-Wvariadic-macros` or the preprocessor warnings it files
under no option. `long long`, mixed declarations and code and variable length
arrays before C99 are under `-Wlong-long`, `-Wdeclaration-after-statement` and
`-Wvla`, which `-pedantic` enables and neither classification reaches. Each
of these five options takes its `-W`, `-Wno-`, `-Werror=` and `-Wno-error=`
forms in argv order, and one of the first three ahead of `-Werror=pedantic`
keeps it from classifying `-Wpointer-arith` and `-Wvariadic-macros`. Clang
warns of a typedef redefinition before C11 by default under
`-Wtypedef-redefinition`, which no form of `-Wpedantic` reaches, holds an
omitted variadic argument in `-Wc23-extensions` and `-Wgnu` too, and files
variadic macros and `__VA_OPT__` outside a variadic macro under
`-Wvariadic-macros` (`tests/integration/pedantic_options_cli.rs`, second
table).

**clang-cl.** As Clang; its driver has no `-Wl,`, `-Wa,` or `-Wp,`, so each
is an unknown warning option. `/Wall` is `-Weverything`.

**TCC.** Any `-W` option, known or not, takes back an earlier `-w`, so `-w
-Wfoo` reports warnings again. `-Wp,` is read as one argument where it
stood, commas and all: `-Wp,-DX=1,-DY=2` defines `X` as `1,-DY=2`, and
`-Wp,` alone is an input file with an empty name (`tcc: error: file '' not
found`).

**Not matched.** GCC's spelling suggestions (`did you mean '-Wshadow'?`,
also in `cc1`'s `no option` error): they search GCC's whole option table
across its front ends, which bcc does not freeze, so bcc leaves them out.
GCC's `-Wno-error-implicit-function-declaration` (an option without a
negative form) is ignored rather than refused. GCC files many of its
preprocessor's pedantic warnings under options of their own or none, which bcc
models only for those `language-standards.md` lists, so its `-Werror=pedantic` and
`-Wno-error=pedantic` leave the others as the extension policy says; and bcc
names `-Wpedantic` where GCC or Clang name a group bcc does not model, such as
Clang's `-Wgnu-statement-expression`. GCC's `-Wlong-long`,
`-Wdeclaration-after-statement` and `-Wvla` also warn of their C99 syntax in
C99 and later, which bcc does not report, and of Clang's `-Wgnu`,
`-Wgnu-zero-variadic-macro-arguments`, `-Wpre-c23-compat` and
`-Wc++20-extensions` bcc reads only what they make of an omitted variadic
argument; bcc's own command line warns that each is only partly
implemented. `-Wfatal-errors` stops at
the first error bcc reports, and bcc reports a preprocessing diagnostic
before the errors of the lines above it where GCC, Clang and TCC report
them in source order.

## How a personality runs

A personality is a translator, not a second driver. It reads its compiler's
arguments and produces one or more *steps*: bcc command lines, which bcc's
ordinary argument parser and driver then run in process, plus printing a
banner or running a built program. Every option is therefore implemented
once, in bcc's own command line, and a personality only spells it: `cl
/DX#5 /c a.c /Foout/` becomes `bcc --target=x86_64-pc-windows-msvc
-fms-extensions -std=c17 -DX=5 -c -o out/a.obj -x c a.c`. `-Xbcc OPTION`
passes an option to bcc unchanged in the GCC, Clang and clang-cl
personalities.

What the steps rely on was added to bcc's own driver, so it is available
without a personality as well:

- several inputs per command, in order, with `-x LANGUAGE` and classification
  by extension (`.c` and `.h` are C, `.i` preprocessed C; `.s`, `.S`, `.sx`
  assembly, with `-x` and the preprocessor options passed to clang; anything
  else goes to the linker), and the final output refused when it is one of
  the inputs;
- `-c` without `-o` (the object is named after the source in the working
  directory), `-S` (LLVM's or the native back end's assembly), `-E` (the
  preprocessed source, with GCC's line markers, or without them under `-P`),
  `-E -dM` (the macros in effect at the end, sorted by name), and `-o -`;
- linking several objects, archives and libraries with `-l`, `-L`, `-Wl,`,
  `-Xlinker`, `-static`, `-rdynamic`, `-pthread`, `-nostdlib` and their
  relatives, in command-line order;
- `-M`, `-MM`, `-MD`, `-MMD`, `-MF`, `-MT`, `-MQ` and `-MP` dependency rules,
  named after the stage and output as GCC names them; `-M` and `-MM`
  without `-E`, `-c` or `-S` take `-o` with several inputs, each unit's
  rules replacing the last, as in GCC;
- standard input (`-`) as a source, whose `-c` and `-S` outputs are `-.o`
  and `-.s`, as GCC and Clang name them; an output named like an option
  (`-o -app`), and the temporary objects named after it, reach the bundled
  clang as `./-app` and so as files;
- `-fsyntax-only` (`--syntax-only`), and a failing exit status from every
  mode, the inspection views included, after an error;
- `--link-only`, a link of the linker inputs and options alone, with no
  input file, as GCC and Clang link `gcc -Wl,--version` (what the linker
  prints on stdout reaches bcc's stdout), and `--no-default-libm`, which
  leaves `libm` to `-lm` as the GCC, Clang and TCC drivers do, where bcc's
  own driver links it into every hosted Linux program;
- an input that cannot be read failing its own job alone, reported in the
  words of the compiler whose diagnostic format is selected: GCC's `cc1:
  fatal error: missing.c: No such file or directory` and `compilation
  terminated.`, Clang's `error: error reading 'missing.c': ...`, `cl`'s `c1:
  fatal error C1083: Cannot open source file: 'missing.c': ...` (Clang's
  words under clang-cl's identity), TCC's `tcc: error: file 'missing.c' not
  found`, or bcc's `error: cannot read ...`; and, as in GCC, an error for a
  linker input that does not exist when the stage leaves it unused;
- GNU line markers (`# 12 "file.c" 1 3`) in input, so `.i` files from
  `-E` compile again with their original locations;
- `--sysroot` for the libraries the bundled clang links against, not only the
  headers;
- `-fwrapv`, `-fpack-struct=N` and `-f[no-]trigraphs`, which the
  personalities map `-fwrapv`, `cl /ZpN` and `cl /Zc:trigraphs` onto, and
  `-f[no-]trapv`, `-f[no-]signaling-nans`, `-f[no-]common`, `-f[no-]wrapv-pointer`, `-f[no-]strict-overflow` and
  `-f[no-]delete-null-pointer-checks`;
- `-mms-bitfields` and `-mno-ms-bitfields`, which lay out every record that
  names neither `ms_struct` nor `gcc_struct` in Microsoft's or System V's
  bit-field layout, as GCC 14 and Clang 23 do on every target; under Clang's
  rules `-mno-ms-bitfields` on an MSVC target makes each record Clang's
  "Itanium-compatible layout" error, in the Clang and clang-cl personalities
  alike;
- `-v` (the header search and each command the driver runs, quoted as
  Clang quotes them), `-H` (the include tree), `-MG` (with `-M` or `-MM`,
  missing headers taken to be generated), and `--show-includes` (`cl
  /showIncludes`), indented by nesting;
- `--driver-name=NAME`, which the personalities pass so that the driver's
  own warnings and errors begin `NAME: warning:`, or take `cl`'s
  `cl : Command line warning :` form under the `cl` identity;
- `--compiler-identity=clang|gcc|msvc|clang-cl|tcc` and
  `--diagnostic-format=bcc|clang|gcc|msvc|msvc-classic|msvc-caret|tcc`
  (also Clang's `-fdiagnostics-format=`).

## Compatibility features by personality

| | GCC | Clang | `cl.exe` | clang-cl | TCC | zig cc | icx | bcc |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Default target | host (MinGW-w64 on Windows) | host | `x86_64-pc-windows-msvc` | `x86_64-pc-windows-msvc` | host | host, its own processor | host | `x86_64-unknown-linux-gnu` |
| Default mode | `gnu17` | `gnu17` | `c17` | `c17` | `gnu99` | `gnu17` | `gnu17` | `gnu17` |
| Microsoft extensions | off | off | all (`-fms-extensions`) | all | off | off | off | off |
| `__GNUC__` | 14.2.0 | 4.2.1 | undefined | undefined | undefined | 4.2.1 | 4.2.1 | 4.2.1 |
| `__clang__`, `__llvm__` | undefined | 23.1.5 | undefined | 23.1.5 | undefined | 22.1.8 | 23.1.1 | 23.1.5 |
| Other identity | `__VERSION__ "14.2.0"`, execution charset names, `__GCC_IEC_559` | | `_MSC_VER 1933`, `_MSC_FULL_VER`, `_MT`, `_MSVC_TRADITIONAL 0` | `_MSC_VER 1933`, `_MT` | `__TINYC__ 927` | `NDEBUG` in the release modes, `__GLIBC_MINOR__` | `__INTEL_LLVM_COMPILER 20260200`, Intel's `__VERSION__`, `-O2` and the `fast` model | |
| `_Float32` ... `_Float128` | keywords, distinct types | identifiers | identifiers | identifiers | identifiers | identifiers | identifiers | identifiers (`-ffloatn-types`) |
| `_Float16`, `__bf16` | types, GCC's `__FLT16_*__` and `__BFLT16_*__` | types, Clang's `__FLT16_*__` | identifiers | types | identifiers | types, Clang's `__FLT16_*__` | types, Clang's `__FLT16_*__` | types |
| `__fp16` | identifier | storage type | identifier | storage type | identifier | storage type | storage type | storage type |
| Tentative definitions | `-fno-common` (`-fcommon` makes them common) | same | `-fcommon` (`cl` makes them common) | `-fno-common` | `-fno-common` | `-fno-common` | `-fno-common` | `-fno-common` |
| Unsigned plain `char` | `-funsigned-char` | `-funsigned-char` | `/J` | `/J`, `-funsigned-char` | `-funsigned-char` | `-funsigned-char` | `-funsigned-char` | `-funsigned-char` |
| System headers | host's `/usr/local/include`, multiarch, `/usr/include` | same | `%INCLUDE%` (unless `/X`) | `/imsvc`, then `%INCLUDE%` (unless `/X`), before the resource headers (clang-cl: after them) | host's | the host's when compiling for the host, as Clang (zig: its own) | as Clang | the host's when compiling for the host; none for a cross target unless asked |
| Diagnostics | `file:line:col: error:` + snippet | same + `N errors generated.` | `file(line): error:` | `file(line,col): error:` | `file:line: error:` | Clang's | Clang's | annotated snippet |
| Default output | `a.out` (`a.exe` on Windows) | same | `first.exe`, objects kept | same | `a.out` | `a.out` (`a.exe` for Windows) | `a.out` | none (analysis only) |
| `__bcc__` | defined | defined | defined | defined | defined | defined | defined | defined |

Plain `char` is signed by default in every personality, as GCC, Clang, MSVC
and TCC default to on x86-64. `-funsigned-char` and `-fno-signed-char` make it
unsigned and `-fsigned-char` and `-fno-unsigned-char` signed, the last one
winning, in the GCC, Clang, TCC and clang-cl personalities; `cl /J` makes it
unsigned in `cl` and clang-cl. bcc then defines `__CHAR_UNSIGNED__`, and
`_CHAR_UNSIGNED` on the MSVC target, so `<limits.h>` gives `CHAR_MIN` 0.

The `_FloatN` types follow the compiler too. GCC 7 and later make
`_Float32`, `_Float64`, `_Float32x`, `_Float64x` and `_Float128` keywords in
every C mode, so the GCC personality's identity turns on bcc's
`-ffloatn-types`: distinct types, the `f32`-style suffixes, the
`__FLT32_*__`-family macros and the `__builtin_inff32`-family builtins. The
pinned Clang 23 rejects all five names and the suffixes in C, and neither
`cl` nor TCC has them, so the other personalities leave them identifiers
for glibc's typedefs. The half-precision types follow each compiler as
well: `_Float16` and `__bf16` are types for GCC 13 and Clang, each with its
own excess-precision rules and macros, Clang's `__fp16` storage type for
the Clang identities, and none of them for `cl` or TCC
([language-standards.md](../../language-standards.md#half-precision-floating-types-2026-10-27)).
bcc's own `-ffloatn-types` and `-fno-floatn-types`, which the GCC, Clang,
TCC and clang-cl personalities pass on, override either default.

GCC and Clang share a snippet: the source line under a gutter five wide, the
caret and tildes under it, and nothing around them. bcc's labels, notes and help
are left out; `-Xbcc --diagnostic-format=bcc` shows them, including the C99
citations, the directories searched for a missing header and the advice naming
`--max-nesting-depth`. What a diagnostic relates to, such as an earlier
declaration, is a `note:` of its own with its own snippet.

Most messages keep bcc's wording. The wording is each compiler's for an
undeclared identifier, a missing expression, and the conversions in
`semantic_analysis/compiler_wording.rs`. The differences left:

- GCC's `In function 'f':` and `At top level:` lines, both compilers'
  `In file included from` lines, and their notes about the macro whose list
  wrote a token are written, but not GCC's `each undeclared identifier` note,
  its `in definition of macro` note for a token from an argument, its labels
  under an operand, or either compiler's notes for the middle levels of a
  nested expansion.
- A range is a caret and tildes. Clang's caret is a point beside the operands
  it underlines for a rejected conversion, an invalid operator, a sign
  comparison, a shift count or a divisor, and alone for an unused parameter,
  a variable set but not used, an unused local typedef and a shadowing
  declaration; other diagnostics underline the token where Clang marks it with
  the caret alone. An operand a function-like macro wrote is underlined at the
  macro's name, where Clang covers the whole invocation. GCC's range from a
  parameter's type to its name (`~~~~^~~~`) starts at the caret, and a range
  over several lines starts each line with a caret.
- A snippet's line number is the physical line, where Clang shows the one `#line`
  gave. Tabs print as four spaces and each Unicode scalar counts as one cell, and
  a column counts scalars, where GCC's counts display cells and Clang's bytes.
- `-fdiagnostics-show-caret`, `-fcaret-diagnostics`, `-fshow-column`,
  `-fdiagnostics-show-line-numbers`, `-fdiagnostics-show-option`,
  `-fdiagnostics-color=`, `-fcolor-diagnostics`, their negations and
  `-fdiagnostics-plain-output` work, each under the compiler that knows it. Color
  marks the same parts, but with bcc's codes rather than each compiler's, and
  leaves quoted source uncolored.
- The whole unit is parsed before semantic analysis, so syntax errors come before
  semantic ones, and recovery can find different errors after the first.

A computed `goto` in a function that takes no label's address is GCC's to
accept and Clang's to refuse; bcc follows the compiler it answers as. The jump
has nowhere defined to go, so `-O2` treats it as unreachable.

On Linux with the LLVM back end, `-fsanitize=address` and `-fsanitize=thread`
instrument the code and link the bundled clang's run-time library, and
`no_sanitize`, `no_sanitize_address` and `no_sanitize_thread` keep a function out.
The two can't be combined, as in both compilers; `--backend=native` refuses them,
since it writes no such instrumentation.

### UndefinedBehaviorSanitizer

On Linux, `-fsanitize=undefined` and its checks' names work on every back end:
lowering writes each check into the code (`middle-end.md`, "UndefinedBehaviorSanitizer").
A failed check calls the run-time library the compiler bcc answers as links:
GCC's `libubsan`, or the bundled clang's `libclang_rt.ubsan_standalone`, whose
reports are the compilers' own. With `-fsanitize=address` or `-fsanitize=thread`
as well, the bundled clang's library for that sanitizer reports in both modes,
since a second sanitizer run time in the process would not work. The report names
the place, the types and the values as that compiler's build of the program
would: on the programs in `tests/integration/undefined_sanitizer.rs` and the
corpus behind them, every report is GCC 13's or Clang 23's, file, line, column,
types and values, through the LLVM back end and `--backend=native`.

- The groups differ, and each mode expands its compiler's (measured): both hold
  `shift` (`shift-base`, `shift-exponent`), `integer-divide-by-zero`,
  `unreachable`, `vla-bound`, `null`, `return`, `signed-integer-overflow`,
  `bool`, `enum`, the array bounds check (GCC's `bounds`, Clang's
  `array-bounds`), `alignment`, `object-size`, `nonnull-attribute`,
  `returns-nonnull-attribute`, `vptr`, `pointer-overflow` and `builtin`; Clang's
  also holds `float-cast-overflow` and `function`. `return`, `enum` and `vptr`
  check nothing in C, in either compiler, and neither does bcc.
- `-fno-sanitize=`, `-fsanitize-recover[=]`, `-fno-sanitize-recover[=]`,
  `-fsanitize-trap[=]`, `-fno-sanitize-trap[=]` and
  `-fsanitize-undefined-trap-on-error` apply in order. A check recovers by
  default, except `unreachable`, as in both. A trapping check runs `ud2`, as
  GCC's `__builtin_trap` does (Clang's `llvm.ubsantrap` is `ud1` with a code);
  either ends the program with `SIGILL`, and needs no library.
- A name the compiler does not know there gets its error
  (`unrecognized argument to '-fsanitize=' option`, `unsupported argument ... to
  option '-fsanitize='`, and `-fsanitize-recover=unreachable`'s). A check bcc
  does not write is refused by name: Clang's `function`, `local-bounds`,
  `implicit-conversion` and the `integer` group, GCC's `bounds-strict`, and
  `float-divide-by-zero`; so are `-fsanitize-minimal-runtime` and
  `-fsanitize-undefined-strip-path-components=`. Clang's `-fsanitize=undefined`
  leaves out `function`, which checks calls through a function pointer of the
  wrong type.
- `no_sanitize("undefined")`, `no_sanitize("check")` and GCC's
  `no_sanitize_undefined` keep a function out. `__has_feature
  (undefined_behavior_sanitizer)` answers 1, in both modes as in GCC 14 and Clang.
- Where a report points follows each compiler. Clang's is the expression's
  location (an operator, a member's name, where a subscript or a converted
  operand starts). GCC checks most operations after gimplification: an operation
  computed straight into a register (a local scalar whose address is never taken)
  reports at the `=` or the declared name, a call's argument at the call, and any
  other at its operator; its shift, division, bounds and conversion checks report
  at the operator, and a conversion of a name where the statement starts. GCC
  orders a commutative operation's operands with a constant second.
- GCC checks the room left in an object only for a local or global object at
  `-O0` (all objects when optimizing); Clang only when optimizing, in the same
  report as `null` and `alignment`, and not for an element whose index it checked.
- `-fwrapv` drops the overflow checks and `shift-base` in both; GCC still checks
  `INT_MIN / -1`.

What differs:

- A type is spelled from its canonical type: where both compilers name a typedef
  (`'size_t [2]'`, `'myint' (aka 'int')`), bcc names what it stands for.
- GCC's folding removes some checks before they run (`m + 1 > 0` becomes
  `m > -1`), and both compilers' optimizers drop checks after an earlier report
  told them an address was aligned or a pointer non-null; bcc keeps those
  checks, so after a report the run may report more than theirs.
- A `_BitInt` of a width other than a standard type's is not checked, where
  Clang checks it; nor is an operation on a GNU complex integer, a vector, or a
  16-bit floating type, or a conversion to a 128-bit integer.
- `-fsanitize=bounds` in the Clang mode checks arrays only, without Clang's
  `local-bounds`.
- The interpreter refuses checks that report through the library, which it does
  not have; it runs trapping ones. The musl and Windows targets refuse
  `-fsanitize=undefined`, as they do the other sanitizers: the bundled libraries
  are glibc's.

The feature-test operators (`__has_include`, `__has_c_attribute`,
`__has_attribute`, `__has_builtin`, `__has_feature`, `__has_extension`, and
Clang's `__has_embed`) exist in every `-std=` under `--gcc-mode` and
`--clang-mode`, and neither compiler calls them extensions under `-pedantic`.
A `void` expression in a `return`, binary constants, case ranges and `__VA_OPT__`
also stay in every `-std=` under both, and `//` comments in C89 and `#embed` under
Clang: `-std=c99` selects a language, and each is still an extension under
`-pedantic`. Plain bcc keeps all of these behind the language mode.

`-msse3`, `-mssse3`, `-msse4.1`, `-msse4.2`, `-msse4`, `-mpopcnt`, `-mcrc32`,
`-mavx`, `-mavx2`, `-mfma`, `-mf16c`, `-mbmi`, `-mbmi2`, `-mlzcnt`, `-mmovbe`,
`-maes`, `-mpclmul`, `-mxsave`, `-msahf`, their `-mno-` forms, and `-march=x86-64`,
`x86-64-v2`, `x86-64-v3` and `native` define each compiler's macros and let LLVM use
the instructions. Each turns on what it needs and `-mno-` turns off what needs it,
and LLVM hears `-avx` as well as `+avx2`, since the processor `-march=` names may have
what the command line took away. POPCNT and CRC32 come with SSE4.2 by default and
aren't needed by it, so `-mavx -mno-popcnt` keeps AVX. AVX needs XSAVE in GCC, while
Clang's `-mno-xsave` leaves AVX on. An option with a later opposite doesn't count at all
in either compiler, so `-mavx -mno-avx` turns nothing on while `-mavx2 -mno-avx` leaves
SSE4.2. The last `-march=` names defaults, which replace the
previous `-march=`'s, and an explicit option outranks them wherever it stands:
`-mno-avx -march=x86-64-v3` has no AVX, and `-mavx2 -march=x86-64` keeps AVX2.
Random combinations of these options defined the same feature macros as GCC 13, GCC
14's cc1 and Clang 23.

Intrinsics compile without them, because bcc's headers compute on any x86-64, except
those whose header function carries a `target` attribute (AVX, AVX2, AES, POPCNT),
which need the feature in the calling function, as in both compilers. A function's
`target` attribute gives its own code its features (`language-standards.md`, "Attribute
status").

These options apply only to x86-64. On a WebAssembly target `--clang-mode` refuses
them as Clang does, with `unsupported option '-mavx2' for target
'wasm32-unknown-unknown'`, and leaves `-msse4.2` and `-msse4` unused as it does.
`--gcc-mode` says `unrecognized command-line option '-mavx2'` and `bad value
'x86-64-v3' for '-march=' switch`, since GCC has no such target and that is what a GCC
for another processor says. They define no macros there.

With AVX on, vectors of 32 bytes, and the records each compiler lets wrap one, travel
in YMM registers as GCC and Clang pass them, so bcc's objects link with theirs both
ways; a 64-byte result comes back in YMM0 and YMM1 under `--clang-mode` and through
memory under `--gcc-mode`. What bcc can't follow is refused, in functions other objects
can reach and calls of them:

- A vector of 32 bytes or more by value when an option bcc only warns about would
  change how it travels (`-mavx512f`, `-march=haswell`, `-march=x86-64-v4`), or, for
  64 bytes, under `-march=native` on an AVX-512 processor.
- The same in a function with a `target` attribute that changes AVX, or a call from
  one: GCC passes YMM there and Clang refuses mismatched calls, while bcc keeps the
  command line's convention for every function. A `static` function whose attribute
  turns AVX off or AVX-512 on under the command line's AVX is refused too, since LLVM
  would pass its vectors in other registers than bcc's calls do.
- Under `--gcc-mode`, a variadic function's named 32-byte vector, which GCC passes in
  YMM0 and LLVM only on the stack, whether AVX comes from the command line or the
  function's `target` attribute. `--clang-mode` keeps Clang's stack.
- `--backend=native` refuses any YMM signature, since it writes no AVX instructions.

A `static` function whose address nothing takes has only bcc's callers, so it keeps
bcc's convention under each of these. An address taken reaches other objects, so the
function is refused like an external one.

An option bcc only warns about counts until AVX goes in that compiler or its own opposite
follows: `-mavx512bw -mno-avx512f` keeps AVX in both, and `-mavx512f -mno-xsave` keeps it in
Clang alone. clang-cl's options count the same way. On the stack, a vector is aligned to
its size, as both compilers place it, whatever `aligned` its typedef says.

`__builtin_cpu_supports`, `__builtin_cpu_is` and `__builtin_cpu_init` read `__cpu_model`
and `__cpu_features2`, as GCC's and Clang's code does (`target/cpu_model.rs`). libgcc
defines them, and the bundled clang links `-lgcc` with or without compiler-rt's
builtins, so a program links wherever any program links; `-nostdlib` leaves them
undefined, as in GCC. Each mode accepts the names its compiler accepts and reads the
bits it reads. `__builtin_cpu_supports("avx2")` gives the bit where it stands in GCC
(`1024`), and 1 in Clang. An unknown name is an error in GCC, `parameter to builtin not
valid: bogus`, and in Clang's `__builtin_cpu_is`, `invalid cpu name for builtin`, while
Clang's `__builtin_cpu_supports` warns, `invalid cpu feature string for builtin`, and
answers 0. GCC reads the name as far as a NUL, so `"avx\0junk"` is `avx` there, and Clang
reads all of it. Both accept `u8""`. WebAssembly refuses all three, `builtin is not
supported on this target`, and `__has_builtin` says it has none there. The Microsoft target needs
`clang_rt.builtins` on the link line, as Clang does; `--interpret` stops at the unresolved
`__cpu_model`.

`-masm=intel` writes assembly in Intel syntax and reads `asm` statements in it,
taking the second alternative of a `{att|intel}` group, on the LLVM back end. The
native back end writes AT&T syntax whatever it is given.

`weakref` works on the LLVM back end: a use is weak unless this unit declares the
target, as in both compilers.

`ifunc` works on glibc's targets with the LLVM back end: the resolver runs when
the program loads and chooses the function. Elsewhere it is an unknown attribute,
as in Clang. The native back end and the interpreter have no such thing, so a call
there is an undefined symbol.

`-pg` on Linux makes each function call `mcount` and links the profiling startup
file, so a run writes `gmon.out` for `gprof`, and `no_instrument_function` keeps a
function out. `-p` still warns.

`-fno-omit-frame-pointer` on Linux keeps a frame pointer in every function on
both back ends, and `-fomit-frame-pointer` drops it. As in GCC, which turns
`-fomit-frame-pointer` on from `-O1`, an unoptimized compilation (one without
`__OPTIMIZE__`) keeps them by default; see
[middle-end.md](../../middle-end.md#outer-frame-addresses-2026-11-08).

## Options a compiler knows and bcc does not implement

Each personality sorts these into three kinds (`personality/unimplemented.rs`
for GCC, Clang and TCC; `msvc.rs` for `cl` and clang-cl). TCC ignores
every option it does not know, so its personality applies the kinds only
to the options TCC itself implements; see [TCC](#tcc).

- **Ignored**, when bcc's output already means what the option asks for:
  warnings and diagnostics formatting, the shape of debug information
  (`-gsplit-dwarf`, `-gz`, `-gstrict-dwarf`, `-fdebug-prefix-map=`; `-g`
  itself is [implemented](#debug-information)), reports and dumps, build mechanics (`-pipe`,
  `-save-temps`, `-flto`, `-fuse-ld=`), optimizations
  the compiler is permitted but not required to make (`-funroll-loops`,
  `-fstrict-aliasing`), code generation tuning (`-mtune=`, `-falign-*`,
  `-mtls-dialect=`), what bcc always does (`-fno-strict-aliasing`: bcc
  emits no type-based alias information;
  `-ffp-contract=off`: bcc forms no fused operations; `-fexcess-precision=`:
  x86-64 has none; `-mno-avx`: bcc uses only the baseline instruction set;
  `-march=x86-64`), the
  target's own ABI choices (`-mabi=ms` for a Windows target,
  `-mlong-double-80` for Linux; `-m` options are judged for the target,
  after the whole command line is read), options that only change
  which programs are accepted (`-fallow-parameterless-variadic-functions`),
  and options for other
  languages (C++'s `-fno-threadsafe-statics`; GCC's `-frtti` and
  `-fno-rtti` draw its C compiler's warning, which libtool looks for).
- **Warned and ignored**, for hardening and instrumentation that a correct
  program does not depend on: `-fstack-protector*`,
  `-ftrivial-auto-var-init=`,
  `-fexceptions` (bcc emits no landing pads, so cleanups do not run while an
  exception unwinds), Spectre mitigations, `-mfentry`, `-specs=`, `-B`, and
  `cl`'s `/GS`, `/sdl`, `/RTC*`, `/guard:cf` and `/Qspectre`; and requests
  bcc's output satisfies less fully but correctly: a processor bcc doesn't name
  (`-march=haswell`, `-march=x86-64-v4`) or a feature outside the list below
  (`-mavx512f`); baseline x86-64 code runs on it, but its macros stay undefined.
  The build goes on and says what it lacks, unless a vector by value would travel
  differently from how those options pass it (see above).
- **Refused**, with the compiler's kind of error, for everything else:
  options that change layout or the ABI away from the target's
  (`-fshort-enums`, `-fshort-wchar`, `-fpack-struct=3`, `-mabi=ms` on
  Linux, `-mregparm=`, `-mlong-double-64` outside MSVC, `cl /Gv`), instruction sets bcc's code needs (`-mno-sse`,
  `-mgeneral-regs-only` except as below; `cl /arch:AVX2`, which also changes
  macros),
  language rules bcc does not follow (`-fsemantic-interposition`,
  `-frounding-math`, `cl /fp:strict`, `/volatile:ms`),
  run-time support bcc's code lacks (`-fsplit-stack`: bcc's functions run on
  one contiguous stack, which a program that relies on split stacks, such
  as GCC's `split-2.c` test recursing 10 MB deep on a thread, would
  overflow), and
  instrumentation whose output a build expects (`-fsanitize=`,
  `-fprofile-arcs`, `--coverage`, `-finstrument-functions`, `-fopenmp`).
  An option the lists do not name is refused too, so a newly added option is
  refused until someone audits it.

Where bcc can do what the option asks cheaply, the personality maps it
instead: GCC's `-mgeneral-regs-only` on Linux targets (GCC's macro changes, and
every function's code keeps to general registers; floating code is refused with
GCC's error where it has one, and by name where it would compile with general
registers or libgcc routines x86-64 lacks: floating parameters, copies and
comparisons), `-muintr`, on either personality on Linux targets (`__UINTR__`, and
handlers return with `uiret`), `-fwrapv` (lowering marks signed arithmetic without `nsw`),
`-ftrapv` (signed `+`, `-`, `*`, negation and `++`/`--` of `int` and
wider check for overflow, and so does `abs`, `labs` or `llabs` of the most
negative value while the library builtins are on (`__builtin_abs` and
its relatives always), and run the `trap`
instruction, on every back end, where GCC's libgcc routines call `abort`,
so the program ends with `SIGILL` rather than `SIGABRT`; GNU complex
integers' sums, products, negation and conjugate are checked in their
signed `int` or wider parts, as in GCC, and a narrow `_BitInt` is not
checked; with `-fwrapv` also given, the last of the two decides
in the GCC personality, and `-fno-wrapv` only turns `-fwrapv` off, as in
GCC, while `-ftrapv` always wins in the Clang personality, which refuses
`-fno-trapv` as Clang does; Clang's `-ftrapv-handler=` is refused),
`-fwrapv-pointer` (pointer arithmetic without `inbounds`, so it wraps in
LLVM and the interpreter), `-fno-strict-overflow` (both, as in GCC),
`-fno-delete-null-pointer-checks` (every LLVM function is
`null_pointer_is_valid`, as Clang marks them, so a dereference does not let
LLVM drop a later null check; bcc's own optimizer never drops one),
`-fcommon` (an external tentative definition the unit leaves tentative is
a common symbol, LLVM `common` or the native back end's `.comm`, which its
ELF and COFF writers encode, with MinGW's `-aligncomm` above a byte; GNU
`common` asks for one under `-fno-common` and `nocommon` keeps an object a
definition, `common` on any declaration winning silently, as in Clang,
where GCC warns and keeps the attribute it met first. As in Clang, a
common symbol is never constant, `dllexport` exports it, and it is not
made for a thread-local object, an imported one or one in a section, nor
on the MSVC target for one whose alignment its declaration, its typedef,
its `aligned` record type or a member of that record requires, or whose
type needs more than 32-byte alignment, since that linker aligns a
communal symbol by its size, which is rounded up to the alignment there;
bcc's objects then link with GCC's `-fcommon` objects, an initialized
definition winning),
`-fstrict-volatile-bitfields` (GCC only and off by default, as on x86-64 in
GCC; the last of it and `-fno-strict-volatile-bitfields` decides, Clang
refuses both as unknown and TCC ignores both: every back end gives a
`volatile` bit-field GCC's number and width of accesses, one aligned access
of its declared type's width where that applies and GCC's ordinary access
otherwise, and refuses at compile time what it cannot match, such as a whole
128-bit field, an access beyond a known object, or an access that depends on
the alignment GCC gives a definition of eight bytes or more or an automatic
object; see `lowering/bit_fields/volatile_access.rs`),
`-fpack-struct=N` and `cl /ZpN` (the packing before any `#pragma pack`),
`-fpack-struct` alone (every structure packed, the pragmas ignored with
GCC's warning), the options that give up part of IEC 60559 semantics
(`-fno-signed-zeros`, `-freciprocal-math`, `-f[no-]trapping-math`,
`-f[no-]math-errno`, `-fassociative-math`, `-fcx-limited-range`,
`-fcx-fortran-rules`, their opposites, and `-ffast-math`,
`-funsafe-math-optimizations`, `-ffinite-math-only` and their `-fno-`
forms, which give up or restore the options they imply, with `-Ofast` read
as `-O3 -ffast-math`): the LLVM back end gives LLVM Clang's fast-math flags
and an executable links `crtfastmath.o` as the drivers do, and the
identities' macros report them as GCC's and Clang's do (`__GCC_IEC_559` 0,
`__NO_SIGNED_ZEROS__`, `__FAST_MATH__`, ...; see `language-standards.md`),
`-fsignaling-nans` (an arithmetic operation or conversion on a signaling
NaN, a constant one included, is kept and raises `FE_INVALID` at run
time, as in GCC: lowering does not use the value semantic analysis folded
for it, the LLVM back end emits floating operations as constrained
intrinsics with strict exception behavior in `strictfp` functions, so
LLVM neither folds them nor applies identities such as `x * 1.0 == x`,
and the native back end and bcc's optimizer never fold floating
arithmetic; copies, negation, `fabs`, `copysign` and
`__builtin_issignaling` keep the encoding; the GCC identity defines
`__SUPPORT_SNAN__` and keeps trapping math, as GCC does. GCC folds a
widening and narrowing round trip such as `(double)(long double)x` to
`x`, keeping a signaling NaN; bcc converts twice, quieting it and raising
`FE_INVALID`, as IEC 60559 conversions do. A unit with a `__bf16` value
keeps LLVM's ordinary floating semantics: LLVM 23 lowers `bf16`
conversions in `strictfp` code to `__truncsfbf2` and `__extendbfsf2`
calls with the wrong register convention, a bug Clang's
`-ffp-exception-behavior=strict` shares. Without the option, LLVM may
fold such operations as GCC does), `-mno-mmx` (bcc's code uses no MMX register, and its MMX
intrinsics work in SSE registers, as GCC's and Clang's do on x86-64, so the
option changes only the macros: the GCC and Clang personalities pass
`-U__MMX__` ahead of the command line's `-D` and `-U`, unless a later
`-mmmx` restores it),
`-fno-builtin` and `-fno-builtin-NAME` (the LLVM functions carry `no-builtins` or `no-builtin-NAME`, so LLVM neither replaces library
calls nor forms them from loops; `-ffreestanding` implies it, and GCC's
`-fno-tree-loop-distribute-patterns` maps onto `memset`, `memcpy` and
`memmove`), `-fvisibility=` (definitions with linkage and no visibility
attribute get it; declarations keep the default, as in GCC), `-mwindows`
on MinGW-w64 (`-Wl,--subsystem,windows`), `cl /Za` (no Microsoft anonymous
members and no `_MSC_EXTENSIONS`), and `cl /Zc:trigraphs`.

## GCC

**Command line.** Single-dash options; short options take their operand
joined or as the next argument (`-o out`, `-Idir`, `-D NAME=VALUE`); long
single-dash options such as `-include`, `-isystem`, `-std=`, `-pedantic`.
`@FILE` is replaced by the arguments FILE holds, split at white space outside
quotes, with a backslash taking the next byte literally; FILE may name further
files, against the working directory, and after 2000 `@` arguments GCC stops
with `too many @-files encountered`. A file that does not exist stays as an
input, which is then reported missing, and a directory is an error. Clang does
the same with these differences: an empty `""` is no argument, which GCC keeps,
a NUL ends only its argument, where GCC stops reading the file, a final
backslash stays,
`@` alone is the working directory, and a file naming itself is `recursive
expansion of: 'FILE'`. Both take `--print-` for `-print-`. An option's value
that is not UTF-8, such as `-DMSG="\xff"`, is refused on the command line and
from a file, where both compilers take it; a file name need not be UTF-8.
Stages: `-E`, `-S`, `-c`, and linking otherwise, into `a.out`. Several inputs
are compiled separately and linked. GCC rejects an unknown option
(`gcc: error: unrecognized command-line option '-ffoo'`) except an unknown
`-Wno-*`, which it ignores and notes after other diagnostics ([Warning
options](#warning-options)), and reports the unrecognized options only
after reading the whole command line. Queries: `--version`, `-v`,
`-dumpversion` (the major version since GCC 7), `-dumpfullversion`,
`-dumpmachine`, `-print-file-name=`, `-print-libgcc-file-name`,
`-print-prog-name=`, `-print-search-dirs`, `-print-multi-lib`,
`-print-multi-directory`, `-print-multi-os-directory`, `-print-multiarch`,
`-print-sysroot`, `-###`. The `-dump` options print at once wherever they
stand; the others are answered once the command line has been read, so a
later `--sysroot` applies, and only one: `-print-search-dirs`, then the
last of `-print-file-name=` and `-print-libgcc-file-name`, the last
`-print-prog-name=`, `-print-multi-lib`, `-print-multi-directory`,
`-print-multiarch`, `-print-sysroot`, `-print-multi-os-directory`, `--help`,
and `--version`. `-print-multi-os-directory` ignores `-m32`. A query ignores the inputs, and its status is 0 even after an
error. An unknown `-std=` is unrecognized wherever it stands.
`-v` prints the version lines, then each command before running it and
the header search its compiler uses; `-###` prints the commands without
running them. `-H` prints each header an `#include` enters, after a dot
per level of nesting, and `-MG` with `-M` or `-MM` takes a missing header
to be generated. Messages from GCC's own driver begin with its name, as
the executable was run (`gcc: warning: a.o: linker input file unused
because linking not done`, `x86_64-w64-mingw32-gcc: error: ...`).

**Mapping.** bcc's own command line already takes GCC's spelling of the
options it implements, so the personality passes them through: `-c -S -E -o
-x -I -isystem -iquote -idirafter -D -U -include -std= -ansi -O* -w -W*
-pedantic* -l -L -Wl, -Xlinker -shared -static -pie -M* -fpic -fms-extensions
-fgnu89-inline -ffreestanding -ftls-model= -H -MG`. `-Wp,` is split and
read as preprocessor options (Fedora's `-Wp,-D_FORTIFY_SOURCE=2`). `-s`,
`-T`, `-u`, `-z` and `-e` become linker options. `-Wa,` and `-Xassembler`
reach bcc's assembly inputs, which the system's `as` assembles as with GCC
(see [The assembler](#the-assembler)). `-save-temps` and `-save-temps=cwd|obj` keep
each C source's `.i` and `.s`, named after the output (`o/x.i` for `-c g.c -o
o/x.o`, `o/p.v1-g.i` and `o/p.v1-g.o` for a program `o/p.v1`), and
`-save-temps=cwd` leaves the directory out. bcc writes them after the program
compiles, from separate runs that read the saved `.i`, and never over the
command's own output. So a failed compile leaves none, where GCC keeps the
`.i`; a source read from standard input saves nothing; and `-fsyntax-only`
writes nothing, where GCC and Clang write the `.i`.
`-pipe` and `--param` have no effect; `-specs=` warns that it has none, and `-B`
that it finds only the assembler. `-Ofast` is
refused: it implies `-ffast-math`, which defines `__FAST_MATH__` and links
`crtfastmath`, which bcc does not do.

**Inputs and stages.** GCC's driver leaves a missing input to `cc1`, which
reports `cc1: fatal error: missing.c: No such file or directory` and
`compilation terminated.`; the other inputs are still compiled, the link is
skipped, and the exit status is 1. A directory reads as a missing file.
`-fsyntax-only` checks the C sources and writes nothing for them, so it
overrides `-c`, `-S` and `-o` there, while `-E`, `-M` and `-MM` override it;
it leaves assembly to the assembler, which still assembles each assembly
input unless `-S` is given (also with `-c`), keeping the object, at `-o`
if given, only for `-c`.
`-o` with `-c`, `-S` or `-E` and several compiled inputs is `gcc: fatal
error: cannot specify '-o' with '-c', '-S' or '-E' with multiple files`,
also under `-fsyntax-only`; with `-M` or `-MM` alone each unit's rules
replace the last in the file. Errors that stop the driver, such as `no
input files`, are `gcc: fatal error: ...` followed by `compilation
terminated.`

The `-W`, `-f` and `-m` names GCC knows are those GCC 15.2 lists in
`--help=warnings,common,optimizers,c,target,undocumented`, frozen in
`src/cli/personality/options/gcc-*.list` by
`scripts/update_driver_options.py`. Unknown warnings are treated as GCC
treats them ([Warning options](#warning-options)); the known `-f` and `-m`
options bcc does not implement are treated as the section above describes. Many of them do change what a
program means (`-fwrapv`, `-fshort-enums`, `-fpack-struct`, `-mno-sse`),
so they are not all ignored. `-m32`, `-mx32`, `-m16`, `-traditional` and
`-imacros` are refused as well.

**Identity.** `__GNUC__` 14, `__GNUC_MINOR__` 2, `__GNUC_PATCHLEVEL__` 0,
`__VERSION__ "14.2.0"`, `__GNUC_EXECUTION_CHARSET_NAME`, GCC's
`__FLT_IS_IEC_60559__` family (1, as GCC 13 and later define them),
`__GCC_IEC_559` and `__GCC_IEC_559_COMPLEX` (2, or 0 under the options
that give up IEC 60559 semantics, with `__NO_SIGNED_ZEROS__`,
`__RECIPROCAL_MATH__`, `__NO_TRAPPING_MATH__`, `__NO_MATH_ERRNO__` and
`__ASSOCIATIVE_MATH__` as GCC defines them), and no Clang macro. Headers then
take their GCC paths. Two of glibc's GCC paths rely on GCC features, which
bcc provides under this identity:

- GCC 7 and later name the TS 18661-3 types `_Float32`, `_Float64`,
  `_Float32x`, `_Float64x` and `_Float128` as keywords, and glibc's
  `bits/floatn.h` stops declaring them. The GCC identity turns on
  `-ffloatn-types`, so they are distinct keyword types with their suffixes,
  builtins and `__FLT32_*__`, `__FLT64_*__`, `__FLT32X_*__`, `__FLT64X_*__`,
  `__FLT128_*__` and `__FLT_EVAL_METHOD_TS_18661_3__` macros, which match
  `gcc -dM -E` for x86-64 Linux. Like GCC there, bcc defines no
  `__STDC_IEC_60559_TYPES__`. With `-fno-floatn-types` bcc instead
  predefines the five names as macros for `float`, `double`, `double`,
  `long double` and `__float128`, the types with the same representations,
  which keeps glibc's headers working but cannot tell `_Float64` from
  `double`.
- With `_FORTIFY_SOURCE` and optimization, glibc's GCC fortify wrappers for
  the `printf` family, `open`/`openat`, `syslog` and a few others forward
  their variadic arguments with `__builtin_va_arg_pack` or
  `__builtin_va_arg_pack_len`. Lowering expands each direct call of such a
  wrapper into a copy specialized for the call's arguments
  (`lowering/argument_pack.rs`), so the calls reach the checked functions
  (`__sprintf_chk`, `__printf_chk`, `__open_2`, ...), as with GCC.

**Banner.** `gcc (bcc 0.1.0) 14.2.0`, then a line that says bcc is not GCC
and is not affiliated with the GNU Project or the Free Software Foundation.
Scripts read the version at the end of the first line; Meson recognizes GCC
by the Foundation's name, which the disclaimer contains.

## Clang

`-save-temps` names what it keeps after the input: `g.i` and `g.s` in the
working directory, or beside the output under `-save-temps=obj`. Clang's `.bc`
is not written, because bcc makes no bitcode file.

**Command line.** GCC's, plus `--target=`/`-target`, `-isysroot`,
`-Xclang`, `--driver-mode=`, and `-fdiagnostics-format=`. Clang errors on an
unknown option (`clang: error: unknown argument: '-ffoo'`) but only warns
about an unknown warning option (`warning: unknown warning option '-Wfoo'
[-Wunknown-warning-option]`, [Warning options](#warning-options)) and goes
on. Its queries are answered once
the command line has been read, and only one: `-dumpmachine`,
`-dumpversion`, `--help`, `--version`, `-print-resource-dir`,
`-print-search-dirs`, `-print-runtime-dir`, the last `-print-file-name=`, the
last `-print-prog-name=`, `-print-libgcc-file-name`, `-print-multi-lib`,
`-print-multi-directory`, then `-print-target-triple` or
`-print-effective-triple`. The bundled clang answers
`-print-resource-dir` and `-print-runtime-dir`. A driver error still prints
and makes the status 1; `-print-multiarch` and `-print-multi-os-directory` are
such errors, and `-print-sysroot` and `-dumpfullversion` are not Clang
queries. Clang's compiler, not its driver,
validates `-std=`, and only the last one: an unknown value is `error:
invalid value 'bogus' in '-std=bogus'` with its list of standards, once for
each C source, unless a driver error, `-###` or a query stops first.

**Mapping.** As for GCC, with the option names the pinned Clang lists for
`--autocomplete=-W`, `-f` and `-m` (`options/clang-*.list`). An unknown
warning option is passed to bcc, which warns as Clang does. `-Xclang OPTION`
is refused: bcc has no Clang frontend options, and Clang's can change the
program.

Clang also takes options `--autocomplete` does not list, frozen from the
pinned Clang's `Options.td` in `options/clang-ignored.list` and classified
by asking that Clang. The GCC optimization flags of its
`clang_ignored_gcc_optimization_f_Group` (`-fno-ipa-cp`,
`-fexpensive-optimizations`, `-fno-vect-cost-model`, `-finline-limit=N`)
draw `clang: warning: optimization flag '-fgcse' is not supported
[-Wignored-optimization-argument]`, and its retired flags (`-mmpx`,
`-fslp-vectorize-aggressive`) `clang: warning: the flag '-mmpx' has been
deprecated and will be ignored [-Wunused-command-line-argument]`; both are
ignored. Clang gives these warnings as it builds the job for each source its
compiler takes, so once per C source or `.S` file, and none for a link or a
`.s` file; they come after the errors about unknown options and missing
inputs. They follow the warning options as Clang's driver warnings do: `-w`
or `-Wno-GROUP` silences them, and `-Werror` makes them errors (exit status
1) unless `-Wno-error=GROUP` exempts them. The options of its ignored `-f`
and `-m` groups (`-finline`, `-fno-var-tracking`, `-frandom-seed=`,
`-mieee-fp`, `-Wlarger-than=`) are ignored without a word, and an alias
`Options.td` defines without a name of its own (`-ftree-vectorize` for
`-fvectorize`, `-fno-tree-slp-vectorize`) is read as the option it stands
for. The zig and Intel personalities inherit this, and clang-cl's driver
ignores all of them as unknown arguments (`unknown argument ignored in
clang-cl`).

**Inputs and stages.** Clang's driver checks every input file before it
runs anything: each missing one is `clang: error: no such file or
directory: 'missing.c'`, followed by `clang: error: no input files` when
none is left, and nothing is compiled (exit status 1). A file that exists
but cannot be read reaches the front end: `error: error reading 'x.c':
Permission denied`. A stage that writes a file per input refuses `-o` for
several inputs, `-M` and `-MM` too: `clang: error: cannot specify -o when
generating multiple output files`; only the inputs it kept count.
`-fsyntax-only` overrides `-c`, `-S` and `-o`, with `clang: warning:
argument unused during compilation: '-c' [-Wunused-command-line-argument]`
for each stage option, and leaves `.s` inputs unused (`clang: warning: f.s:
'assembler' input unused`); `-E`, `-M` and `-MM` override it with the same
warning for `-fsyntax-only`. A command line that does not link leaves the
linker's input files unused, with `clang: warning: a.o: 'linker' input
unused`, and so are the linker options among them, in order with the
files (`-lm: 'linker' input unused`, `-Wl,...`, `-Xlinker`, `-z`, `-e`);
after those, in command-line order, come the arguments only the link reads
(`argument unused during compilation: '-L/usr/lib'`, also `-T`, `-u`,
`-s`, `-shared`, `-pie`, `-rdynamic`, `-nostartfiles`), with the stage
options and `-fsyntax-only` the stage leaves unused. These warnings follow
the warning options, as Clang's
`-Wunused-command-line-argument` does: `-w`, `-Qunused-arguments` or
`-Wno-unused-command-line-argument` silences them, and `-Werror` or
`-Werror=unused-command-line-argument` makes them errors (`[-Werror,...]`,
exit status 1) unless `-Wno-error=unused-command-line-argument` exempts
them, after which, as in Clang, the unused linker inputs are still
reported but no unused argument is; `-Weverything` turns on only the warnings no option has set, so it
does not undo `-Wno-unused-command-line-argument` or `-Wno-everything`. After a source that fails to compile, Clang compiles the others, as
GCC does.

`-v`, `-###`, `-H` and `-MG` work as for GCC, with Clang's forms: `-v`
and `-###` quote every word; `-H`, like clang-cl's `/showIncludes`, begins
every path it reaches from a main file in the working directory through
its includers' directories with `./` (`./a.h`, `./sub/../b.h`); `-MM -MG`
lists a missing `<...>` header too; and `-MG` without `-M` or `-MM` is
`clang: error: option '-MG' requires '-M' or '-MM'`.

**Identity.** bcc's default: the pinned Clang 23.1.5 and the GCC 4.2.1 Clang
claims. `--version` prints `clang version 23.1.5 (bcc 0.1.0, ...)` with the
target, thread model and installed directory, as Clang does.

## MSVC `cl.exe`

**Command line.** Options start with `/` or `-` and are case-sensitive.
Most take their value joined (`/Fefoo.exe`, `/Ifoo`, `/Fo:out.obj`); `/I`,
`/D`, `/U` and `/FI` also take the next argument, and so do `/Fe:`, `/Fo:`
and `/Fi:` alone (`/Fo: out.obj`). `/DNAME#VALUE` defines NAME
as VALUE. `cl` compiles each source to an object named after it (`a.c` to
`a.obj`) and keeps it, then links an executable named after the first source
(`a.exe`) unless `/c` is given. `/Fo` and `/Fe` name a file, or a directory
when they end in a separator (a backslash on every host). `/link` passes the
rest of the command line to the linker. `cl` prints a banner to stderr unless
`/nologo` is given, and the file name of each source it compiles, without its
directories, to stdout (to stderr under `/E`, `/EP` and `/P`). An unknown
option draws `cl : Command line warning D9002 : ignoring unknown option
'/foo'` and is ignored. An option that overrides an earlier one of its kind
with another value draws `cl : Command line warning D9025 : overriding '/W3'
with '/W4'`: the warning level (`/w` is `/W0`, and `/Wall` is no level), the
optimization level (`/Ox` is `/O2`) and, apart from it, `/Os` and `/Ot`, the
runtime, `/std:`, the debug information, `/WX`, `/Zp`, `/Fo`, `/Fe`, `/Fa`,
`/Fi`, the switches with a `-` form (`/GL-`), and a `/U` of a macro whose last
option was a `/D`; a later `/D` of the same macro replaces it silently. A file
whose extension `cl` does not know (Microsoft documents `.c`, `.cxx`,
`.cpp`, `.obj`, `.lib`, `.def` and `.exp`; `.res` goes to the linker too) is
assumed to be an object, with `cl : Command line warning D9024 :
unrecognized source file type 'x', object file assumed`, and under `/c`
every input for the linker draws `cl : Command line warning D9027 : source
file 'x' ignored`. `cl` runs only on Windows, where a path does not start
with `/`; on other hosts the personality reads paths as clang-cl does (see
"Paths that start with `/`" below), so `cl /c /tmp/a.c /Fo/tmp/a.obj`
compiles `/tmp/a.c`, and `/bogus` there is an input of unknown type (D9024)
rather than an unknown option. `cl` compiles every source even after one fails, reports
a missing one with `c1: fatal error C1083: Cannot open source file:
'missing.c': No such file or directory` after printing its name, skips the
link, and exits with status 2, as it does after any error. After compiling
several sources it prints `Generating Code...` to stdout, also when one was
missing. `cl` alone prints its banner and, to stdout, `usage: cl [
option... ] filename... [ /link linkoption... ]`, and succeeds; with options
but no file it stops with `cl : Command line error D8003 : missing source
filename` and status 2. `/?`, `/help` and `/HELP` print bcc's help.

`cl` reads more options from the `CL` environment variable, before the
command line, and from `_CL_`, after it, and replaces each `@file` (a
command file) with its contents: UTF-8, or UTF-16 after a byte order mark,
as MSBuild writes them. These are split into arguments as the Microsoft C
runtime splits a command line. The rest of a command file's line, or of a
variable, after `/link` goes to the linker, and the next line holds compiler
options again. A command file that cannot be opened is `cl : Command line
error D8022 : cannot open 'x.rsp'` (status 2), and a command file cannot
name another: `@inner.rsp` inside one is an input like any other. clang-cl
reads `CL`, `_CL_` and command files too, as one stream of arguments in
which a command file may name another, as LLVM's `ExpandResponseFiles`
does, and leaves one it cannot read on the command line as an input it then
reports missing. (Microsoft Learn: "CL environment variables", "CL Command
Files", "@ (Specify a Compiler Response File)".)

**Mapping.**

| `cl` | bcc |
| --- | --- |
| `/c`, `/Fo`, `/Fe`, `/link ...` | `-c -o NAME.obj`, then a link of the objects with `-Wl,...`; `/Fo`, `/Fa` or `/Fi` naming a file for several sources is error D8036, as in `cl`; an output of any step that is an input of the command line (by device and inode) is refused before any step runs |
| `/E`, `/EP`, `/P`, `/Fi` | `-E`, `-E -P`, `-E -o NAME.i`, in `cl`'s layout (Preprocessed text) |
| `/FA`, `/Fa` | `-S -o NAME.asm` beside the object |
| `/I`, `/D`, `/U`, `/FI`, `/X` | `-I`, `-D`, `-U`, `-include`; `/X` skips `%INCLUDE%` |
| `/O1`, `/Os` / `/O2`, `/Ox`, `/Ot` / `/Od` | `-Os` / `-O2` / `-O0` |
| `/W0`, `/w` / `/W1`-`/W3` / `/W4`, `/Wall` / `/WX` | `-w` / default / `-Wall -Wextra` / `-Werror` (clang-cl: `-w` / `-Wall` / `-Wall -Wextra`, `-Weverything` for `/Wall` / `-Werror`); each also sets the level of the warnings bcc reports with `cl`'s code (see Diagnostics) |
| `/wdNNNN`, `/weNNNN`, `/w1NNNN`-`/w4NNNN` | for a warning bcc reports with `cl`'s code, `-Wno-GROUP`, `-Werror=GROUP` or its level; other numbers have no effect, and `/wo` none. `#pragma warning` reads the same numbers (language-standards.md, "Diagnostic pragmas") |
| `/std:c11`, `/std:c17`, `/std:clatest` | `-std=c11`, `-std=c17`, `-std=c23` |
| `/TC`, `/Tc FILE` | the inputs (or FILE) are C |
| `/MD`, `/MDd`, `/MT`, `/MTd`, `/LD`, `/LDd` | the last runtime option wins: `_DLL` for `/MD*`, `_DEBUG` for `/M*d` and `/LDd`, and the link takes `msvcrt`, `msvcrtd` or `libcmtd` instead of `libcmt`; `/LD` links a DLL with `/MT`'s runtime unless `/MD` names another |
| `/Zs` | `--syntax-only` for each source, with no object and no link |
| `/Zp`, `/ZpN` | `-fpack-struct=1`, `-fpack-struct=N`; another N is error D8021 |
| `/Za`, `/Ze` | the last wins: `/Za` turns off Microsoft's anonymous tagged members and `_MSC_EXTENSIONS`, and defines `__STDC__` |
| `/Zc:__STDC__` | defines `__STDC__` |
| `/Zc:trigraphs`, `/Zc:trigraphs-` | `-ftrigraphs`, `-fno-trigraphs` (the default, as in `cl`) |
| `/J` | `-funsigned-char`: plain `char` is unsigned and `_CHAR_UNSIGNED` defined |
| `/external:I DIR`, `/external:env:VAR` | `-isystem` |
| `/imsvc DIR` (clang-cl and `icx-cl`) | a system directory searched just before `%INCLUDE%` (see "clang-cl") |
| `/showIncludes` | `--show-includes`: `Note: including file: PATH` to stdout (to stderr under `/E`, `/EP` and `/P`), the path absolute (see "Preprocessed text" for where `cl` keeps it relative) |
| `/diagnostics:classic`, `column`, `caret` | `file(line):`, `file(line,col):`, and with the snippet |
| `/Zi`, `/Z7`, `/ZI` | CodeView in the object, as clang-cl treats all three; a link keeps it through `lld-link -debug` |
| `/nologo`, `/Zc:inline` and the other `/Zc:` switches for C++, `/utf-8`, `/EH*`, `/Wv`, `/MP`, `/FS`, `/Fd`, `/Gy`, `/GL`, `/Oi`, `/Oy`, `/arch:SSE2`, `/fp:precise`, `/fp:fast`, `/volatile:iso` ... | accepted, no effect (clang-cl's `/fp:` options set the math functions' `errno`, see "clang-cl") |
| `/GS`, `/sdl`, `/RTC*`, `/guard:cf`, `/guard:ehcont`, `/Qspectre*`, `/d1*`, `/d2*` | accepted with a warning |
| `/TP`, `/Tp`, `/std:c++*`, `.cpp` inputs, `/Gv`, `/arch:` other than SSE2, `/fp:strict`, `/fp:except`, `/volatile:ms`, `/Zc:preprocessor-`, non-UTF-8 charsets | refused with `cl : Command line error : ...` |

**Defaults.** The `x86_64-pc-windows-msvc` target with every Microsoft
extension bcc implements (`-fms-extensions`), which on that target also
predefines `_MSC_VER` 1933, `_MSC_FULL_VER` 193300000, `_MSC_BUILD` (0, as
`cl` 19.44 defines it; Clang and clang-cl define 1) and `_MSC_EXTENSIONS`.
The identity removes the GNU and Clang macros and adds `_MT` (the default
static runtime) and `_MSVC_TRADITIONAL 0`: bcc's preprocessor is
conforming, like `cl /Zc:preprocessor`. Directories in `%INCLUDE%` are
system header directories. A tentative definition is a communal symbol
(`-fcommon`), as `cl` makes it, so two units that both declare `int x;`
link. `__STDC__` is defined only under `/Za` or `/Zc:__STDC__`, as by `cl`
and by Clang for its Windows MSVC targets: the Universal C Runtime's headers
declare POSIX names such as `O_RDONLY` and `open` only without it. Under
`/std:` the personality defines `__STDC_NO_ATOMICS__`, `__STDC_NO_COMPLEX__`
and `__STDC_NO_VLA__`, as `cl` does, though bcc has all three, so that a
program takes the paths it takes with `cl`, and in no mode
`__STDC_NO_THREADS__`: Visual C++ has `<threads.h>`.

**Language mode.** `cl` without `/std:` compiles a C dialect close to C17 and
leaves `__STDC_VERSION__` undefined; `/std:c11` and `/std:c17` define it.
bcc compiles C17 by default, as clang-cl does, and defines
`__STDC_VERSION__` as `201710L`.

**Preprocessed text.** `/E` writes a line for each source line, blank lines
and directives included, as `cl` does: a `#line 1 "a.c"` marker first, a
header's marker (`#line 1 "C:\\src\\h.h"`) on the `#include` line and the
includer's on return, and a marker for the next line in place of each
`#endif` and of an `#elif` that ends a skipped group (an `#else` is an empty
line). `/EP` writes an empty line for each marker, and both stop at a fatal
error (a missing header, `#error`). A `#pragma` directive keeps its line,
`#pragma once` included, and a `_Pragma` or `__pragma` operator's pragma
stays among the tokens of its line as `__pragma(...)`, spaced as the
operator was, with `_Pragma`'s literal as it destringizes
(`__pragma(message("hi"))`), so that the text compiles again. Within a line,
bcc spaces tokens as GCC does, which `cl`'s traditional preprocessor (the
default) also does in the cases surveyed. What still differs:

- `cl`'s conforming preprocessor (`/Zc:preprocessor`, and every `/std:`)
  also puts a space after each macro's expansion (`int a = 1 ;`), which bcc
  does not.
- `cl`'s traditional preprocessor joins a macro invocation whose arguments
  span lines onto the line where it starts and writes a `#line` marker for
  the line after it; bcc, like the conforming one, leaves each token of the
  invocation's arguments on its own line. It also keeps a `\`-new-line
  inside a string literal, which bcc joins.
- In a pragma, bcc writes each run of white space as one space; `cl` keeps
  the runs after its first token as written (`#pragma pack(   2 )`), and its
  traditional preprocessor drops a comment there.
- `cl` names a header found beside its includer by the includer's directory,
  made absolute, and the name the directive wrote
  (`C:\src\sub\../h.h`), and so does bcc, with the host's separators; but
  `cl` names one found through a relative `/I` directory relatively, with
  the directory as written (`#line 1 "inc\\deep.h"`, `Note: including file:
  inc\deep.h`), where bcc makes that path absolute too. Ninja reads both
  forms of a `/showIncludes` note.

**Diagnostics.** `file(line): error: message`, the form `cl` writes
without `/diagnostics:` (its `/diagnostics:classic`); `/diagnostics:column`
gives `file(line,col):`, the form clang-cl writes by default and MSBuild
asks `cl` for. The diagnostics `cl`'s users meet most take `cl`'s code and
words, and drop bcc's notes:

| Code | `cl`'s words | Level | bcc's group |
| --- | --- | --- | --- |
| fatal error C1189 | `#error:  text`, the directive's text after `#error` | | |
| error C2065 | `'y': undeclared identifier` | | |
| error C2143 | `syntax error: missing ';' before 'return'` (a type keyword is `'type'`, a literal `'constant'` or `'string'`) | | |
| error C2146 | `syntax error: missing ';' before identifier 'y'` | | |
| fatal error C1075 | `'{': no matching token found`, at the `{` of the innermost block the end of the file leaves open | | |
| error C2039 | `'y': is not a member of 'S'`, at the member, and `note: see declaration of 'S'` at the tag | | |
| error C2063 | `'x': not a function`, at the name called | | |
| error C2084 | `function 'int f(void)' already has a body`, and `note: see previous definition of 'f'` | | |
| error C2100 | `you cannot dereference an operand of type 'int'` | | |
| error C2106 | `'=': left operand must be l-value`, at the left operand's operator, or the start of a constant or a call | | |
| error C2124 | `divide or mod by zero`, of an integer constant by a constant zero | | |
| error C2197 | `'int g(int)': too many arguments for call`, at the first extra argument | | |
| error C2198 | `'int g(int,int)': too few arguments for call` | | |
| error C2703 | `illegal __leave statement`, outside every `__try` block | | |
| error C2705 | `'in': illegal jump into '__try' scope` (or `'__except'`, `'__finally'`), at the label, once; a `case` label is `'null'` | | |
| warning C4013 | `'g' undefined; assuming extern returning int` | 3 | `implicit-function-declaration` |
| warning C4018 | `'<': signed/unsigned mismatch`, of a relational comparison | 3 | `sign-compare` |
| warning C4033 | `'h' must return a value` | 1 | `return-mismatch` |
| warning C4047 | `'=': 'int *' differs in levels of indirection from 'int'` (also between pointers: `'int *'` from `'char **'`) | 1 | `int-conversion` |
| warning C4098 | `'g': 'void' function returning a value` | 1 | `return-mismatch` |
| warning C4100 | `'p': unreferenced parameter` | 4 | `unused-parameter` |
| warning C4101 | `'a': unreferenced local variable` | 3 | `unused-variable` |
| warning C4127 | `conditional expression is constant` | 4 | bcc's own, unnamed |
| warning C4133 | `'=': incompatible types - from 'float *' to 'int *'` | 1 | `incompatible-pointer-types` |
| warning C4189 | `'b': local variable is initialized but not referenced` | 4 | `unused-but-set-variable` |
| warning C4242 | `'=': conversion from 'int' to 'char', possible loss of data`, off unless `/Wall` or a setting names it | 4 | bcc's own, unnamed |
| warning C4244 | `'initializing': conversion from 'double' to 'int', possible loss of data` | 2, 3 or 4 | bcc's own, unnamed |
| warning C4245 | `'=': conversion from 'int' to 'unsigned int', signed/unsigned mismatch` | 4 | bcc's own, unnamed |
| warning C4293 | `'<<': shift count negative or too big, undefined behavior` | 1 | `shift-count-overflow`, `shift-count-negative` |
| warning C4305 | `'initializing': truncation from 'double' to 'float'` | 2 | bcc's own, unnamed |
| warning C4311 | `'type cast': pointer truncation from 'int *' to 'int'` | 1 | `pointer-to-int-cast` |
| warning C4312 | `'type cast': conversion from 'int' to 'int *' of greater size` | 1 | `int-to-pointer-cast` |
| warning C4389 | `'==': signed/unsigned mismatch`, of an equality comparison | 4 | bcc's own, unnamed: only its number reaches it |
| warning C4532 | `'out': jump out of __finally block has undefined behavior during termination handling`, at a `goto`'s label (once) or at `break` or `continue` (`'identifier'` when its loop is a do-while) | 1 | `jump-seh-finally` |
| warning C4706 | `assignment used as a condition`, and `cl`'s note | 4 | `parentheses` |
| warning C4715 | `'g': not all control paths return a value` | 1 | |
| warning C4716 | `'h': must return a value` | 1 | |

C4018 and C4389 compare operands whose promoted types have one width and
differ in sign, neither of them a constant, whatever its value (`-1 < u`
draws neither). C4706 is an `if`, `while`, `do` or `for` condition that is an
`=` assignment outside parentheses (`cl` also warns of one that ends a comma
expression, `if (b, a = b)`, and bcc does not). C4293 is a constant shift
count of a binary `<<` or `>>`, not of `<<=` or `>>=`, that is negative or not
below the width of the promoted left operand (`cl` also warns of one under
`sizeof` and in an arm of `?:` that a constant condition skips, and bcc does
not). Names that only `__try`, `__except` or `__finally` use count as used,
and the warnings reach what those blocks hold. A `goto` or `case` label may
not enter one of those blocks (C2705, naming the innermost): `cl` counts how
deep each point is in them, a handler one level inside its `__try` block, and
rejects a jump only to a deeper label or to one as deep in another innermost
block, so a jump to a shallower label is accepted wherever it lands. `__leave`
belongs in a `__try` block (C2703), and a `goto`, `break` or `continue` that
leaves a `__finally` block draws C4532, a `return` nothing. C4100, C4101
and C4189 name a parameter, or a variable with automatic storage, that
nothing refers to (an assignment refers to it; C4189's initializer does not),
under the settings at its declaration; `cl` reports them at the end of the
function, the parameters (the last one listed first) and the outermost
block's variables first and then each inner block's in the order the blocks
begin, and none once the unit has had an error (a warning `/WX` promotes
aside), and so does bcc; for an old-style definition C4100 is at the
parameter's name in the identifier list. A reference in an operand that `cl`
folds away does not count (`b` in `1 ? a : b`, `0 && b` and `1 || b`, by the
constant conditions described for C4244 below), but one under `sizeof` does,
where `cl` folds nothing.
C4715 and C4716 are `cl`'s code generator's: a path reaches the closing brace
or a `return` without a value, and the function has a `return` with a value,
reached or not (C4715: `if (0) return 1;` is one), or has none (C4716), `main`
aside. A call to a function declared `noreturn` or `__assume(0)` ends a path.
They are reported only for a function `cl` generates: an external definition
that is not a discardable inline one (`inline`, `__inline` or
`__forceinline` without `extern`), and a function that a generated function
or a static object's initializer names, even in code no path reaches, but not
under `sizeof`; an unreferenced `static` or `inline` function draws neither.
The settings at the function's name decide them, not those at the closing
brace where they are reported. They come after every diagnostic of the front end,
and only if it reported no error, led by the file's full path and the line
alone (`C:\src\a.c(5) : warning C4715: ...`), with `error C2220` before each
under `/WX`, and so they do in bcc; no group of bcc's controls them, only
their numbers. C2124, like `cl`'s, is reported only while the unit has had
no error, and C1075 is a fatal error, after which nothing more is reported.

C4244, C4305 and C4245 are a conversion as if by assignment (`initializing`,
`=`, `function` for a prototyped call's argument, `return`, or the
compound operator, `+=`). C4244 is a floating value made an integer (level
2), a value made a narrower floating type (level 3, or 4 from `long double`
to `double`, which `cl` reports though both are 64 bits), an integer made
`float` from a 32-bit type or any floating type from a 64-bit one (level 2;
not `!x`), and an integer made a narrower integer type when its value is
wider than the type (level 4 from `int` and `unsigned int`, level 3 from
the others). `cl` measures how wide the value is rather than its type:
an object, a call or a bit-field (whatever its width) is as wide as its
type, a constant as its magnitude, `&` and `%` the narrower of their
operands (a constant divisor counting one less), `>>` by a constant that
many bits less, `<<`, unary operators and the operand of a cast, which
keeps no more than its type, as wide as before, a comparison 1 bit, and
`+`, `-`, `*`, `/`, `|`, `^` and `?:` the wider operand, so `ch = ch + 1`,
`ch = i & 0xff` and `ch = i >> 24` draw nothing. A `?:` whose condition is a
constant `cl` folds to the operand it selects, which alone it measures and
reports at (`ch = sizeof(long) == 8 ? ll : sc` draws nothing): an integer
constant, `0 && x`, an integer made a pointer (a null pointer is false) or a
floating constant (`-0.0` is false), but not an address, a string literal or
anything with a comma operator. Such a fold that leaves a constant converts
it as one. A compound assignment
converts the result of the operation (`'int' to 'char'` for `char += short`)
and narrows only for `+=`, `-=`, `*=` and `/=` of a wider right operand;
`_Bool` is a byte there, and an integer made floating draws nothing. A
constant draws C4305 instead when its new type does not hold it (an
integer that fits neither the signed nor the unsigned type of that width,
a floating constant made `float` inexactly, but not to an infinity or
zero; `cl` reads a 64-bit unsigned constant as signed, so
`0xFFFFFFFFFFFFFFFFull` is -1 and fits), or C4245 when it is negative and
made unsigned; a floating constant made an integer is C4244, and one made
`_Bool` only when it is 1 and not a `?:`. No
enumeration, on either side, and no value made `_Bool` draws one. `cl`
names both types as written, typedef names and qualifiers included
(`'volatile Byte'`, `'U32'`, and `'const u64'` for the member of a `const`
object), and an operation by an operand that already has its type
(`'U32'` for `u32 + 1`, `'int'` for `ush + 1`); bcc's types keep no typedef
names, so each binding and member records how its declaration wrote its
type, and a conversion whose types bcc cannot name so draws nothing. `cl`
checks conversions only where the warning level is 2 or more, whatever
`/w1`, `/we` or a pragma says of the warnings, reports them at the value
(the operator that computes it, `?` for a conditional expression it does
not fold, or where it begins; a floating value in parentheses at the first
of them), and an automatic
object's initializer conversions at the end of its function, before
C4100 and the others it reports there. C4244's level depends on the
conversion, so `/wd`, `/we`, `/w1`-`/w4` and the pragmas set all three, and
`default` restores each. Under `/Wall`, which shows the warnings that are
off by default, C4242 replaces C4244 for the narrowing of a 64-bit value
and of one `cl` has not computed (an object, a member, a call, a cast to
its own type, or such an operand that a constant `?:` of its type selects),
apart from C4244's settings. C4127 is an `if`, `while`,
`do` or `for` condition that `cl` folds to a constant (arithmetic, a null
pointer or another integer made a pointer, a string literal, or `0 && x`,
but not an object's or a function's address, nor anything with a comma
operator left after folding) unless it is an integer literal of 0 or 1
under parentheses, unary operators, a cast to its own type or the operand a
constant `?:` selects, as in `while (1)` and `do ... while (0)`; it is
reported at the folded operator, or where the value begins under its
parentheses and casts and in the operand a constant `?:` selects.

What `cl` writes of these that bcc does not: C4267 for a `size_t` value
(`size_t` and the arithmetic it takes part in, as `sizeof(x) + i`), which
`cl` reports apart from C4244 and bcc leaves out; C4244 when an operation's
operands have one type spelled two ways (`u + u32`, where `cl` keeps the left
spelling for 32-bit operands and the right one for 64-bit ones), or reads an
unsigned bit-field narrower than `int`, which `cl` keeps unsigned where C
promotes it; C4305 of an unsuffixed decimal constant above `INT_MAX`, which
`cl` types `unsigned long` as C90 did, and of a constant whose arithmetic
overflows (`cl`'s C4307 with its value); a compound assignment of an
integer constant; and a conversion whose type comes from a declaration bcc
cannot read for its spelling (a GNU or standard attribute in its
specifiers, `typeof`, a call through a typedef name's function type that
is shadowed). Because the identity defines `__x86_64__`, a program that
tests it, as zlib's `crc32.c` does, compiles code `cl` skips, and draws its
warnings.

The operation that leads C4047 and C4133 is `initializing`, `=`,
`function` (an argument) or `return`, as in `cl`. A type `cl` spells its
own way (`__int64`, a function pointer with `__cdecl`) keeps bcc's words,
without a code, and so does every other diagnostic: no mapping from bcc's
other messages to MSVC's codes is documented, so bcc writes no code rather
than an invented one. As in `cl`, the command line shows level 1 warnings
by default (`/W1`), so C4013 appears from `/W3`; `/wd`, `/we` and
`/w1`-`/w4` with these numbers control them, `/we` making one an error at
any level. `cl` only warns about a conversion between a pointer and an
integer or between incompatible pointers, and (C4113) between function
pointers whose parameters differ, so the personality makes those groups,
which Clang's rules make errors, warnings unless `/WX` is given. C4113 and
C4090 (lost qualifiers) keep bcc's words, but `/wd`, `/we` and the levels
name them apart from C4133, as `cl` does. The rightmost of `/W0`-`/W4`,
`/Wall` and `/w` wins. Under
`/WX`, as in `cl`, `error C2220: the following warning is treated as an
error` comes before a unit's first warning, at its line, and every warning
keeps its words (`warning C4047: ...`), where `/we4047` writes `error
C4047`. One group holds C4033 and C4098, so `/wd` or `/we` of either sets
both. bcc reports in source order, while `cl` reports an initializer's C4047 after
the warnings of the statements that follow it in the same function, as the
Windows comparison with `cl` shows. The level comes from the rightmost of
`/W0`-`/W4` and `/Wall`, and `#pragma warning(push, n)` sets it until its
`pop`; `default` and a level specifier in a pragma show a warning from that
level again, as `cl` does, undoing a `/wd` or `/we`, and an error a pragma
makes is `cl`'s `error C4018`, not a warning `/WX` promotes. `cl` announces
with C2220 only a warning before which the unit had no error.
`#pragma warning(suppress: N)` withholds the warning on the pragma's line
and the next, whatever they hold (a blank line counts); a `__pragma` or
`_Pragma` covers its own line too, the invocation's for one a macro
produces, and a `suppress` on a line another reaches holds both to its own
next line. `once` (and `/wo`) shows a warning the first time it is shown so
in the unit, and not again under `once`, wherever the setting comes from.
Neither applies to C4715 and C4716 beyond the settings at the function's
name: `once` shows them every time.

The MSVC codes bcc writes for the driver's own messages are those
Microsoft documents: warnings D9002 (an unknown option), D9024 and D9027
(inputs), errors D8003 (no source file), D8022 (a command file that cannot
be opened) and D8036 (`/Fo` for several sources), and fatal error C1083
for a source that cannot be opened, which gives the C runtime's reason
(`No such file or directory`) on Windows too, as `cl` does, rather than
the system's. bcc's own driver messages take `cl`'s command-line form
without a code (`cl : Command line error : cannot build ...`).
`/showIncludes` names each header by its absolute path, the directory it was
found in joined with the name the directive wrote, indents it by one space
more per level of nesting, as `cl` does, and leaves out a re-inclusion
whose include guard is already defined. As `cl` does, the personality
writes a compile's diagnostics, C1083 among them, and its `/showIncludes`
notes to stdout, after the source's name, where MSBuild and Ninja's `deps
= msvc` read them, and to stderr under `/E`, `/EP` and `/P`, which keep
stdout (or the file) for the text; the command line's own messages go to
stderr. (Microsoft Learn: "Fatal Error C1189", "Compiler Error
C2065", "Compiler Error C2143", "Compiler Error C2146", "Compiler Warning
(level 3) C4013", "Compiler Warning (level 1) C4033", "Compiler Warning
(level 1) C4047", "Compiler Warning (level 1) C4098", "Compiler Warning
(level 1 and level 3) C4133", "Compiler Warning (level 1) C4311",
"Compiler Warning (level 1) C4312", "Compiler Error C2220", "Command-Line
Warning D9025", "/w, /W0, /W1, /W2, /W3, /W4, /w1, /w2, /w3, /w4, /Wall,
/wd, /we, /wo, /Wv, /WX (Warning level)", "warning pragma", "Fatal Error
C1075", "Compiler Error C2039", "Compiler Error C2063", "Compiler Error
C2084", "Compiler Error C2100", "Compiler Error C2106", "Compiler Error
C2124", "Compiler Error C2197", "Compiler Error C2198", "Compiler Error
C2703", "Compiler Error C2705", "Compiler Warning (level 3) C4018", "Compiler Warning (level 4) C4100", "Compiler Warning
(level 3) C4101", "Compiler Warning (level 4) C4189", "Compiler Warning
(level 1) C4293", "Compiler Warning (level 4) C4389", "Compiler Warning
(level 4) C4706", "Compiler Warning (level 1) C4532", "Compiler Warning
(level 1) C4715", "Compiler Warning (level 1) C4716", "Compiler Warning
(level 4) C4127", "Compiler Warning (level 4, off) C4242", "Compiler Warning
(level 2) C4244", "Compiler Warning (levels 3 and 4) C4244", "Compiler
Warning (level 4) C4245", "Compiler Warning (level 1) C4305", "__assume".)

**Banner.** `bcc 0.1.0, compatible with Microsoft (R) C/C++ Optimizing
Compiler Version 19.33.0 for x64; not a Microsoft product`, to stderr. Tools
read the version and `for x64` from it.

### Checked against `cl.exe` on Windows

`tests/integration/msvc/` compares the personality with the real `cl.exe`
on a Windows host with Visual Studio or its Build Tools; elsewhere, and
without them, its tests skip. They take the newest installation `vswhere`
finds with the x64 C compiler and run both compilers in the environment its
`vcvars64.bat` sets. The ignored `msvc::toolchain::probe` reports what a
host has (`cargo test --test integration msvc::toolchain::probe --
--ignored --nocapture`). On the host they were run on (29 October 2026):
Build Tools 2022 17.14 (and 2019 and 2017), `cl` 19.44.35228, `link`
14.44.35228, the Windows SDK 10.0.19041.0 (the registry's `KitsRoot10`,
with `ucrt.lib` and `kernel32.lib`), and no `lld-link` or `clang-cl` on
`PATH`; bcc uses the bundled LLVM's.

- **The C runtime.** Programs using `printf` and `vsnprintf`, `malloc`,
  `realloc` and `calloc`, files (`fopen`, `fgets`, `fread`, `fseek`,
  `ftell`, `remove`), `qsort` with a callback, `atexit`, `strtod`,
  `setjmp` and `longjmp` through several frames, wide characters and
  `exit` from a nested call, built under `/MD`, `/MT` and `/MDd` by `cl`,
  by `bcc --msvc-mode` and by bcc's native back end (clang-cl's
  `--backend=native`), print and return the same. `long double` is
  `double` (`sizeof` 8, `LDBL_MANT_DIG` 53, `%Lg`). bcc links through the
  bundled clang and LLD, which find Visual Studio's libraries and the SDK
  themselves, also without `vcvars64.bat`'s `%LIB%`; headers come only
  from `%INCLUDE%`, so bcc, like `cl`, needs that environment.
- **DLLs.** A DLL exporting functions and data with
  `__declspec(dllexport)`, built with `/LD`, and its import library link
  with a client importing them with `__declspec(dllimport)`, for every
  pair of the three compilers.
- **ABI.** bcc's objects link with `cl`'s in both directions, through
  `link.exe`: structures of 1, 3, 8, 16 and 20 bytes, of a `double` and of
  two `float`s, passed and returned by value; eleven mixed arguments; `int`,
  `double`, `long long`, pointer and structure variadic arguments and a
  `va_list` passed on; `__int64` arithmetic; callbacks taking a structure;
  `_Bool`, `signed char`, `unsigned short`, `float` and `long double`
  results. bcc refuses `__vectorcall` (and `/Gv`), so it is not compared.
- **Tentative definitions.** `int shared;` and `double shared[100];` in two
  units are communal symbols in `cl` and, under the personality's
  `-fcommon`, in bcc: every pairing links and runs alike. An object of a
  `struct __declspec(align(64))` type is a strong definition in both, so
  every pairing fails alike (`right-cl.obj : error LNK2005: shared already
  defined in left-cl.obj`), as Clang's MSVC rule for required alignment
  says. A record with an `_Alignas(64)` member (which `cl` reads only under
  `/std:c11`), a `__declspec(dllexport)` object and a `const` one were not
  compared on Windows: the test that surveys them first ran with the
  `_Alignas` case compiled without `/std:c11`.
- **Command lines.** `/c`, `/Fo` naming a file or a directory, `/Fe`,
  `/link /OUT:`, `/std:c11`, `/std:c17`, `/O2`, `/Zi`, no source file
  (D8003), `/Fo` for several sources (D8036), an unknown option (D9002)
  and a header among the inputs (D9024, D9027) give the same status, the
  same files and the same lines; `/E`, `/EP`, `/P`, `/D`, `/I`, `/W4`,
  `/WX`, missing sources and headers and compile errors the same status and
  files.
- **Predefined macros.** `_WIN32`, `_WIN64`, `_M_X64` and `_M_AMD64` (100),
  `_MT`, `_DLL`, `_DEBUG`, `_CHAR_UNSIGNED`, `_MSC_EXTENSIONS`,
  `_MSC_BUILD`, `_INTEGRAL_MAX_BITS` (64), `_ISO_VOLATILE`, `_M_FP_PRECISE`
  and `__STDC_HOSTED__` agree with no option and under `/MD`, `/MDd`,
  `/MTd`, `/LD`, `/std:c11`, `/std:c17`, `/std:clatest`, `/Za`, `/J`,
  `/O2`, `/fp:fast` and `/Zc:preprocessor`, and neither defines
  `__GNUC__`, `__clang__`, `__AVX__`, `_M_IX86` or `_M_ARM64`.

The comparison found what this section and Deviations now record: bcc's
`printf` failed to link (`lld-link: error: undefined symbol: __va_start`)
until the Microsoft intrinsic `__va_start`, through which `<vadefs.h>`
defines `va_start`, stayed an intrinsic where the header declares it; the
native back end's `longjmp` died with `STATUS_BAD_STACK` (0xC0000028)
until `_setjmp` received the caller's establisher frame (a one-argument
`_setjmp` or `_setjmpex` gets the frame address, as `cl` and Clang pass
it, and the native back end's Win64 frame address is the establisher
frame, as LLVM's is); and the driver differed in D8003, the classic
diagnostic form, `Generating Code...`, C1083's reason, `_MSC_BUILD` and
communal tentative definitions.

### Checked against `cl.exe` under Wine

On 19 November 2026 the personality, run as `bcc-rust --msvc-mode` and as an
executable named `cl`, was compared byte for byte with the real `cl.exe`
19.44.35230 (Build Tools 2022 17.14, `link` 14.44.35207, Windows SDK
10.0.26100) on x86-64 Linux, where Wine 9.0 (Ubuntu 24.04's `wine64`) ran
it. [msvc-wine](https://github.com/mstorsjo/msvc-wine)'s `vsdownload.py`
fetched only the x64-hosted x64 toolset and the SDK (`--major 17
--architecture x64 --host-arch x64 --only-host yes`, without ATL, MFC, ASan,
DIA or MSBuild) and `install.sh` set it up; `cl.exe` ran under `wine64` with
the `INCLUDE` and `LIB` of msvc-wine's `msvcenv.sh`, but without its
wrappers, which rewrite the paths in `cl`'s output. bcc ran with the same
directories in host form and the pinned LLVM's `lld-link`.
`scripts/compare_cl_under_wine.py` repeats the comparison: each of 232
command lines runs in a fresh copy of one directory of sources (those of
`tests/integration/msvc/cli.rs` and more) with each compiler, and the exit
status, the files written, stdout, stderr and each `.i` file's text are
compared, after the directory's path in either form becomes one token and
CRLF becomes LF. The command lines cover `cl /?` and the banner, `/c`,
`/Fo`, `/Fe` and `/link`, `/W0`-`/W4`, `/Wall`, `/w`, `/WX`, `/wd`, `/we`
and `/w1`, `/std:`, `/E`, `/EP` and `/P`, `/D`, `/U` and `/I`, `/O1`, `/O2`,
`/Od` and `/Zi`, `/showIncludes`, command files and `CL` and `_CL_`, the
predefined macros (`/EP` of a file naming them, under the options that
change them), options that override others, and the common errors and
warnings. As `cl` and as `--msvc-mode` alike, all five agreed for 70 of them
before the fixes this comparison led to and for 173 after, and the status
and files for 226.

The comparison found, and bcc now does as `cl` does: a compile's
diagnostics, C1083 and `/showIncludes` notes go to stdout, not stderr, and
the notes name a header found beside its includer by its absolute path;
`/E` lays out its text with a line for each source line and `#line` markers,
keeps a pragma operator among its line's tokens as `__pragma(...)`, and
stops at a fatal error;
`/WX` announces its first warning with C2220 and keeps the warnings'
words; D9025 for an overriding option; `#error` as C1189, and C4033 and
C4098; the file name, not the path, of each source compiled; a trailing
backslash in `/Fo` and `/Fe` off Windows; a repeated `/D` without a
warning; the `__STDC_NO_*` macros under `/std:`. The real-code check found
the rest:
[zlib](https://zlib.net) 1.3.1, configured by CMake 3.31 with Ninja for an
x64 Windows target (`CMAKE_MSVC_DEBUG_INFORMATION_FORMAT=Embedded`, as
`cl.exe` under Wine cannot reach its program database server for `/Zi`,
error C1902), built its DLL, static library, `example.exe` and
`minigzip.exe` with `cl` and `link.exe` (msvc-wine's wrappers), and with
bcc failed: the Visual C++ `<limits.h>`, which bcc's own `<limits.h>`
includes next, spells `ULLONG_MAX` with Microsoft's `ui64` suffix, and the
Universal C Runtime's `<fcntl.h>` hid `O_RDONLY` because bcc defined
`__STDC__`. With both fixed, zlib builds with bcc as `cl` linked by
`lld-link` and by `link.exe`, and with `cl` linked by `lld-link`; in all
four pairings `example.exe` passes under Wine and `minigzip.exe` compresses
200,000 bytes of `deflate.c` to the same 19,748 bytes and back. bcc's
objects name no runtime library (Deviations), so its CMake toolchain file
names `msvcrt.lib` and `oldnames.lib` itself.

What still differs, as Deviations records: the help text and the banner;
the predefined macros' values (`_MSC_VER`, `_MSVC_TRADITIONAL`,
`__STDC_VERSION__`, Clang's target macros) in each macro dump; `cl`'s
warnings bcc does not report (C4244 and C4305, whose conversion rules
depend on the operand's form, C4477, C4473, C4474, C4476 and C4313, whose
words name types by their typedef names, which bcc does not keep, C4142, a
typedef redefined with another type of the same representation, which bcc
refuses, and C4456, C4127, C4996 and others at `/W4`) and its words and
codes for the errors outside the Diagnostics table; and, among the warnings
in the table, the order of the unreferenced names of one block, which `cl`
takes from its symbol table and bcc from the declarations, an unsigned
bit-field compared with an `int`, which `cl` keeps unsigned (C4018) where C
and bcc promote it to `int`, a call to `abort` or another library function
bcc knows does not return, which ends a path for bcc and not for `cl`
unless a declaration says so (C4715), the order and number of C2220 lines
among the code generator's warnings, which vary from run to run of `cl`,
C1075 for an unclosed member or initializer list, `cl`'s C4033 after an
error in a `return`'s expression, and the errors of the rest of a unit
after a syntax error (C2106 for a binary left operand is one), which bcc's
semantic analysis does not read; names that only `_Generic`'s controlling
expression or its unselected associations use, which `cl` reports as
unreferenced; C4013 for `__assume`, an intrinsic bcc does not declare; C2197
once per call where `cl` writes it for each extra argument, and bcc's own
words for C2197, C2198 and C2084 when the function is called through a
pointer or declared with a typedef name, which `cl` keeps in its words;
C4702 (unreachable code); C4715 and C4716 for a function in an
`/external:W0` header, which `cl`'s code generator still reports; a
`#pragma warning` in the middle of a line, as `cl`'s settings are those of
lines and bcc's those of the tokens after the pragma (`suppress` aside); and
a change another pragma makes on a `suppress` pragma's next line, which
`cl` undoes after that line; the snippet of `/diagnostics:caret`; the
system headers' paths under `/showIncludes`, which name the `INCLUDE`
directories in each host's form, and a header found through a relative `/I`
directory, which `cl` names relatively; the `/E` layout differences listed
under "Preprocessed text"; the type of a sized integer constant whose value
does not fit the signed type of its suffix's width (`0xffffffffi32 > 0` is
1 in bcc and 0 in `cl`, which wraps the value to -1, as Clang does); the
warning bcc adds for `/GS`;
`/fp:strict`, which bcc refuses; and the import library LLD writes for a
DLL without exports.

## clang-cl

clang-cl reads `cl`'s options and also Clang's (`-fsomething`, `-Wfoo`,
`/clang:` options). The personality reads `cl`'s, passes an unknown `-`
option to bcc (which reads most Clang options), and reads an argument that
starts with `/` and names no option as an input, as clang-cl does on every
host (see below); it warns, with clang-cl's `unknown argument ignored in
clang-cl`, about a `cl` option clang-cl knows and bcc does not. Like
clang-cl it reads the `/clang:` options after all the others, wherever they
stand, so `/clang:-fsigned-char /J` leaves `char` signed. Its
`/fp:fast` gives up the math functions' `errno`, as clang-cl's
`-ffast-math` does, so a call of `sqrt` has no side effect for `-Wassume`,
and `/fp:precise` restores the target's default, as `-ffp-model=precise`
does; bcc takes none of the rest of `-ffast-math`, nor its macros. The
identity keeps the Clang macros and drops the GNU ones, as clang-cl does. It
prints no banner and no source names. As Clang's driver does, it refuses a
command line without inputs (`clang-cl: error: no input files`, status 1),
except for the queries it answers without one: `--version` (to stdout),
`-v` and `-###` (the same lines to stderr), `-print-target-triple`,
`-print-search-dirs` (the bundled LLVM's programs and resource libraries)
and `/help`, `/HELP` or `-help`; and it refuses
a missing source before compiling any (`clang-cl: error: no such file or
directory: 'missing.c'`; also an input for the linker whose name starts
with `/`, with `; did you mean '/Brepro'?` when an option is one edit
away), warns that a stage before linking leaves an input
for the linker unused (`clang-cl: warning: x: 'linker' input unused
[-Wunused-command-line-argument]`), stops at the first source that fails to
compile, and exits with status 1 after an error. It reads `-` as standard
input, which `/E` preprocesses as C and which compiles only when `/Tc`
names its language (`clang-cl: error: use /Tc or /Tp to set input type for
standard input` otherwise).

`/imsvc DIR` (also `-imsvc DIR` and joined, `/imsvcDIR`), which CMake
gives clang-cl for a `SYSTEM` include directory, names a system directory.
bcc searches the `/I` directories first, then the `/imsvc` directories,
then those of `%INCLUDE%` unless `/X` is given (`/X` keeps the `/imsvc`
ones), then the `/external:I` and `/external:env:VAR` directories in
command-line order, and only then its built-in headers. A header found in
any but `/I`'s directories is a system header, so its warnings stay
hidden, and `/showIncludes` lists it. `/imsvc` without a directory is
clang-cl's `argument to '/imsvc' is missing (expected 1 value)`.
`icx-cl` reads these options as clang-cl does, and `cl` draws D9002 for
`/imsvc`, which it does not know.

This order differs from clang-cl's driver
(`MSVCToolChain::AddClangSystemIncludeArgs`), which searches its resource
headers right after the `/I` and `/external:I` directories, and then the
`/imsvc`, `/external:env:` and `%INCLUDE%` directories. bcc keeps
`%INCLUDE%` ahead of its built-in headers because its intrinsic headers do
not yet work with the MSVC headers: the UCRT's `wchar.h` and `tchar.h` and
the SDK's `winnt.h` include `<intrin.h>`, and bcc's `intrin.h` and
`emmintrin.h` define `_mm_pause` and other intrinsics `static`, which
MSVC's `intrin0.inl.h` declares `extern` (identifier has both internal and
external linkage), and bcc's `_mm_prefetch` macro breaks `winnt.h`. Two
further differences are bcc's in every mode: clang-cl drops a directory
given twice, keeping the system one (so `/I d -imsvc d` makes `d` a system
directory there), and it honours `-Wsystem-headers`.

### Options clang-cl ignores

clang-cl's driver takes `cl`'s options and only some of Clang's: any other
argument that starts with `-` is unknown to it, and it ignores it with a
warning (`Driver::ParseArgStrings`). That covers GCC's and Clang's
`-fsigned-char`, `-ffreestanding`, `-ffast-math`, the other fast-math flags
(`-funsafe-math-optimizations`, `-ffinite-math-only`, `-fno-honor-nans`,
`-fassociative-math`, `-freciprocal-math`, `-fno-signed-zeros`,
`-ffp-model=` ...), `-std=`, `-pedantic`, `-ansi`, `-g0` and the like, `-l`,
`-L`, `-isystem`, `-include`, `-M` and its relatives, `-shared` and
`-static`. The personality ignores exactly the options clang-cl ignores:
`personality/options/clang-cl-unknown.list` lists them, frozen by
`scripts/update_driver_options.py --clang-cl-unknown`, which asks the pinned
Clang's clang-cl about every option the GCC and Clang lists, GCC's
`--help` and Clang's `--autocomplete=-` name, each `-f` and `-m` flag in
both forms (`-fno-strict-volatile-bitfields`). A flag is ignored as
spelled, an option with a value with any value (`-std=c11`, `-lm`,
`-isystem/usr/include`), and an ignored option takes no argument, so in
`-isystem dir` the directory is an input, as in clang-cl. bcc's own
options that neither compiler has (`--emit=`, `-Xbcc`) still reach bcc.
`/clang:` gives clang-cl's front end a Clang option, which bcc reads as
Clang does: `/clang:-ffreestanding` makes the environment freestanding,
and `/fp:fast /clang:-fno-fast-math` gives up `/fp:fast`, where a bare
`-fno-fast-math` keeps it.

The warning is clang-cl's, byte for byte: `clang-cl: warning: unknown
argument ignored in clang-cl: '-fsigned-char' [-Wunknown-argument]`, or
with the option clang-cl's option table finds one edit away, `unknown
argument ignored in clang-cl '-fbuiltin-'; did you mean '-fbuiltin'?`,
which keeps a value it is given (`-mabs=x` suggests `-mabi=x`). It comes
in command-line order, and the warning options anywhere on the command
line decide it, as Clang's `ProcessWarningOptions` maps the group
`-Wunknown-argument` before the driver parses the rest:

- `/w`, `/W0`, `-w` and `--no-warnings` silence it, whatever else says;
- `/WX` and `-Werror` make it an error, `clang-cl: error: ...
  [-Werror,-Wunknown-argument]`, with status 1 and nothing compiled, unless
  `/WX-`, `-Wno-error` or `-Wno-error=unknown-argument` follows or
  `-Wno-error=unknown-argument` stands anywhere; `-Werror=unknown-argument`
  makes it one by itself, and a later `-Wunknown-argument` keeps it one;
- `-Wno-unknown-argument` and `-Wno-everything` turn it off until a later
  `-Wunknown-argument`, `-Werror=unknown-argument` or
  `-Wfatal-errors=unknown-argument` turns it on;
- `-Wfatal-errors` makes the error fatal (`clang-cl: fatal error: ...
  [-Wunknown-argument]`), which ends the command line at the first one, and
  so does `-Wfatal-errors=unknown-argument` without `/WX`;
- the `/W1`-`/W4`, `/Wall` and `-Weverything` levels leave it as it is,
  and a `/clang:` warning option, which clang-cl hands its front end,
  does not reach it.

`/clang:-Wno-unknown-argument` and the other groups of the driver's own
warnings are taken by the front end with no effect, so bcc's command line
does not call them unknown.

icx-cl ignores a slightly different set, which
`personality/options/icx-cl-unknown.list` lists, frozen from icx-cl
2026.2.0 by `scripts/update_driver_options.py --icx-cl`: of the options
clang-cl ignores it takes `-ffast-math`, `-fast` and the complex-range
options, reports most of the other 90 or so in its own words (`command line
warning #10430: Unsupported command line options encountered`, which bcc
does not model, passing them on as before), and its suggestions name
Intel's options (`'-ffreestanding'; did you mean '-Qfreestanding'?`).

### Paths that start with `/`

A `cl` option and a Unix path both start with `/`. clang-cl's option parser
(LLVM's `OptTable`) matches an argument against its options first and reads
one that starts with `/` and matches none as an input file, on every host;
it does not check that the file exists until it diagnoses missing inputs.
The personalities follow it, deciding with the options the pinned Clang's
clang-cl knows (`personality/options/clang-cl.list`, frozen from its
`Options.td` by `scripts/update_driver_options.py --options-td`):

- A flag matches only its whole name and an option with a value by the
  name the argument starts with, so `/tmp/a.c`, `/home/me/a.c`,
  `/nologox` and `/analyzeme.c` are inputs, while `/nologo` stays the
  option even if a file of that name exists. This is decided before bcc
  reads the option, so none of its own spellings reads a path; the `cl`
  personality alone also keeps the `cl` options clang-cl does not know
  (`/Zf`, `/Qpar-`, `/Qspectre-`, `/analyze` and `/analyze:...`).
- A path that starts with an option taking a joined value is that option,
  as in clang-cl: `/Users/me/a.c` is `/U` with `sers/me/a.c`, `/Data/a.c`
  `/D`, and `/workspace/a.c` `/w`. `--` ends the options: every argument
  after it is an input (`clang-cl /c -- /Users/me/a.c`). A name after it
  that starts with `-` reaches bcc's own command line as `./-a.c`; clang-cl
  passes it on unchanged, and its front end reads it as an option.
- Options with paths read them joined or, where clang-cl does, from the
  next argument: `/Fo/tmp/out/`, `/Fo: /tmp/a.obj`, `/I/usr/include`,
  `/I /usr/include`, `/Fe/tmp/a.exe`, `/Fa/tmp/a.asm`, `/Fi/tmp/a.i`. The
  bare name of an option clang-cl reads with its value in the next
  argument takes that argument, also where bcc ignores the option or does
  not know it (`/Fp: /tmp/a.pch`, `/vctoolsdir /opt/vc`), so it is no
  input. `/Fo` takes its value joined only: in `/Fo /tmp/a.obj`, as in
  clang-cl, `/tmp/a.obj` is an input.
- A missing input is reported as each compiler reports one: clang-cl's
  `no such file or directory: '/tmp/missing.c'`, also for an input for the
  linker that starts with `/` (a mistyped option: `/Brepo` draws `; did you
  mean '/Brepro'?`), and `cl`'s fatal error C1083 for a source, or warnings
  D9024 and D9027 for any other input.

`cl` on Windows keeps its own reading: an unknown `/` option draws warning
D9002, and `--` is an unknown option too. Elsewhere the `cl` personality
also reads two kinds of argument as `cl.exe` would, since build systems
drive `cl` as on Windows: CMake gives clang-cl a `--` before its sources
but not `cl`, and expects `cl`'s warning D9002 for an unknown option.

- A file that exists inside a directory is an input, whatever option its
  name starts with: CMake's ABI probe `/opt/cmake/.../CMakeCCompilerABI.c`,
  which clang-cl reads as `/o`, compiles.
- A name without a directory that names no file is an option, so `/bogus`
  draws D9002, where clang-cl reports a missing input.

## TCC

The personality was compared with three real TCCs on x86-64 Linux:
TinyCC built from the `mob` branch (`tcc version 0.9.28rc 2026-10-16
mob@e23dabed`), Ubuntu's `tcc` 0.9.27+git20200814.62c30a4a, which prints
`tcc version 0.9.27`, and the `release_0_9_27` tag built from source
(without its bounds checker, which no longer builds against glibc). bcc
claims 0.9.27 (`__TINYC__ 927`, `-dumpversion`), and its reference is
Ubuntu's build, the `mob` snapshot `0.9.27+git20200814.62c30a4a`: Debian
and Ubuntu ship that snapshot as 0.9.27, the CI compares against it, and
0.9.28 has never been released. Its warnings are the snapshot's, which
are not the release's: the `release_0_9_27` tag has no `function might
return no value` at all, and under `-Werror` writes its warnings as
`warning:` though it exits with 1. Of the 52 files bcc's TCC warnings were
probed with, compiled with no options, the tag differs from the snapshot
on 8: the 6 that warn of a missing return, and 2 whose conditional
expressions mix pointer types, which it does not warn of. The snapshot's
predefined macros are the tag's with 12 GCC-style ones (`__CHAR_BIT__`,
`__INT_MAX__`, `__SIZEOF_INT__`...) and a `__builtin_offsetof` macro
added; `mob`'s `tccdefs.h` adds 13 more (`__amd64__`, `__has_attribute`,
`_Nonnull`...). bcc reads the options `mob` added where they only extend
0.9.27's command line: `-dumpmachine`, `-M`, `-MM`, `-MMD`, `-MP`,
`-std=gnu11`, `-run --` and `-rstdin`. The opt-in test
`tcc_parity_with_a_real_tcc` in `tests/integration/personality_cli.rs`
repeats the comparison when `BCC_TCC` names a `tcc` binary, which the Linux
CI job does with Ubuntu's.

**Command line.** TCC reads its arguments with `tcc_parse_args`
(`libtcc.c`), and so does the personality: an option is the first entry
of TCC's table, in TCC's order, that the argument starts with and whose
operand rule it satisfies, so `-ofile`, `-xc`, `-Ifoo`, `-runFILE` and
`-gdwarf` are options with joined values and `-MT` is `invalid option --
'-MT'`. `@FILE` is replaced by the arguments FILE holds, split at blanks
with double quotes grouping and a backslash escaping a quote (`tcc: error:
listfile 'x' not found`). TCC reads listfiles without a limit, so one
that names itself makes TCC hang; bcc stops after 4096 (`tcc: error:
listfile 'self.txt' not read: more than 4096 listfiles (does one name
itself?)`). `-Wp,A,B` reads `A,B` as the next argument, as TCC's
`tcc_parse_args` does. An option's value is
the next argument even when that starts with `-`, so `tcc -I -c -c a.c`
searches a directory named `-c`.
`-c`, `-E`, `-r`, `-shared` and `-run` choose what is built: the last one
wins, and each one after the first draws `tcc: warning: -c: overriding
compiler action already specified` (also `-c -c`). A second `-o` draws
`tcc: warning: multiple -o option`. TCC rejects any option it does not
know (`tcc: error: invalid option -- '-q'`, exit status 1), but ignores the
`-W`, `-f` and `-m` options it does not know and its no-op options
`-arch`, `-C`, `--param`, `-pedantic`, `-pie`, `-no-pie`, `-pipe`, `-s` and
`-traditional`, and the unknown `-d` letters, warning only under
`-Wunsupported` (`tcc: warning: unsupported option '-fbogus'`). With no
arguments, or with `-h`, it prints its help and exits with status 0; `-hh`
prints more; `-v` alone prints the version, `-vv` alone the version and
`-print-search-dirs`; `-v` with inputs prints the version first and then
`-> FILE` for each input before it is compiled and `<- OUTPUT` after the
output is written, all to stdout. `-h`, `-hh`, `-print-search-dirs` and
`-ar` must come first (`tcc: error: cannot parse -h here`);
`-dumpversion` and `-dumpmachine` print at once wherever they stand.

**Outputs.** As TCC names them (`default_outputfile` in `tcc.c`): `-c`
writes an object per input, named after its base name with `.o` for its
extension, or `a.out` for standard input and a name without an
extension; a link, `-shared` and `-r` write `a.out`. `-c` stops at the
first input that fails, as TCC does, and refuses `-o` with several inputs
(`tcc: error: cannot specify output file with -c many files`) and any
`-l` (`tcc: error: cannot specify libraries with -c`). TCC reads its inputs
in order and stops at the first it cannot add, so the inputs after one it
cannot open, missing or unreadable, are dropped and the run fails with
`tcc: error: file 'missing.c' not found` (exit status 1), after compiling
the C sources before it. `-run FILE ARGS...` compiles FILE, with the options
between `-run` and FILE, and runs it with ARGS; `tcc FILES -run -- ARGS`
runs FILES; `-run` with no input is `tcc: error: no input files`. The
program's `argv[0]` is the argument TCC stopped at, FILE or `--`, and its
exit status is the command's. `-rstdin FILE` gives the program FILE as its
standard input; a FILE that cannot be opened is `tcc: error: failed to
reopen stdin from 'FILE'` before anything is compiled, as in `mob`. Under the TCC identity a `#!` line that starts
a file is skipped, as TCC skips it, so a script starting `#!/usr/bin/tcc
-run` runs; elsewhere the line stays a non-directive, which GCC and Clang
reject.

**Mapping.**

| TCC | bcc |
| --- | --- |
| `-c`, `-E`, `-o`, `-I`, `-D`, `-U`, `-include`, `-isystem`, `-nostdinc`, `-pthread` | the same |
| `-w` | `-w`, unless a `-W` option follows it |
| `-l`, `-L`, `-Wl,`, `-static`, `-rdynamic`, `-nostdlib`, `-shared` | the same, in order with the inputs; `-soname NAME` is `-Wl,-soname,NAME` |
| `-r` | `-no-pie -nostdlib -Wl,-r`: one relocatable object from every input |
| `-run` | a build in a temporary directory, then the program |
| `-std=c11`, `-std=gnu11` | `-std=gnu11`; any other `-std=` leaves `gnu99`, as TCC sets `__STDC_VERSION__` only for those two (0.9.27 only for `c11`) |
| `-ON` | `-ON`, at most `-O3`: 0.9.27 reads the level with `atoi`, so `-O` and `-Os` are level 0 (`mob`: 1), and a positive level defines only `__OPTIMIZE__` |
| `-xc`, `-xa`, `-xn` (first letter) | `-x c`, `-x assembler-with-cpp`, `-x none`; another is `tcc: warning: unsupported language 'X'`; `-xb` is refused |
| `-P`, `-P1`, `-dM` (with `-E`) | `-P`; `-P1` keeps GNU line markers; `-dM` |
| `-dD` | `-dM`, with `tcc: warning: '-dD' is treated as '-dM'` |
| `-M`, `-MM`, `-MD`, `-MMD`, `-MD,FILE`, `-MF`, `-MP` | the same, with `-MT` and `-MF` naming the rule and file after TCC's output (`a.out`, `a.d` for a link of one source) |
| `-funsigned-char`, `-fsigned-char`, `-f[no-]common`, `-f[no-]gnu89-inline`, `-f[no-]dollars-in-identifiers` | the same |
| `-f[no-]ms-extensions` | `-f[no-]ms-anonymous-structs`: TCC's flag allows only anonymous `struct` and `union` members, and is on by default |
| `-mms-bitfields`, `-mno-ms-bitfields`, and the same in `#pragma comment(option, "...")` from there on | the same, in TCC's own Microsoft layout ([TCC's test suite rules](../../language-standards.md#tccs-test-suite-rules-2026-11-07)); the pragma's other options are refused, except a char signedness already in effect |
| `-fleading-underscore`, `-ftest-coverage`, `-mno-sse`, `-m32` | refused, as the GCC personality refuses them |
| `-freverse-funcargs` | ignored with a warning: C leaves the order unspecified |
| `-g`, `-gN`, `-gdwarf[-N]` | [debug information](#debug-information), as for GCC |
| `-fasynchronous-unwind-tables`, `-msse`, `-m64`, `-gstabs` and other `-g*`, `-B` | ignored |
| `-b`, `-bt[N]`, `-bench` | ignored with a warning: bcc has no bounds checker, no backtraces and no statistics |
| `-Werror`, `-Wdiscarded-qualifiers`, `-Wunsupported` | `-Werror`, `-Wincompatible-pointer-types-discards-qualifiers`; `-Wunsupported` makes the personality warn about ignored options |
| `-Wimplicit-function-declaration` (and `no-`, `error=`) | passed on |
| `-Wall`, `-Wwrite-strings` | ignored: bcc's `-Wall` reports more than TCC's, and it has no `-Wwrite-strings` |
| any other `-W`, `-f` or `-m` | ignored, as TCC ignores it; the `-f` options that only bring bcc's code closer to TCC's (`-fwrapv`, `-fno-strict-overflow`, `-fwrapv-pointer`, `-fno-strict-aliasing`, `-fno-delete-null-pointer-checks`, `-fno-builtin*`) and the `-fpic` family are passed on |
| `-ar ARGS...` | the bundled `llvm-ar ARGS...` |
| `-dumpmachine` | `x86_64-pc-linux-gnu` (`-musl`, or `x86_64-pc-mingw32`), as `mob` prints it |
| `-print-search-dirs` | TCC's layout: bcc's directory, the C library's header directories, the bundled clang's library and start-file directories, and the dynamic linker; there is no `libtcc1` |

TCC itself ignores an option it does not know, so a `-f` or `-m` option
GCC knows changes nothing in the program TCC builds, and the personality
builds the same one by ignoring it too: `-fshort-enums`, `-fpack-struct`,
`-fstack-protector` and `-fsanitize=address` are ignored silently, as in
TCC (they were refused or warned about before TCC was surveyed). An option
TCC implements and bcc does not is refused.

TCC keeps three kinds of diagnostic that GCC 14 and Clang make errors as
warnings: an assignment between a pointer and an integer, between
incompatible object pointers, or between incompatible function pointers
(`warn.c:1: warning: assignment makes pointer from integer without a
cast`). The personality passes `-Wno-error=int-conversion`,
`-Wno-error=incompatible-pointer-types` and
`-Wno-error=incompatible-function-pointer-types`, unless `-Werror` is
given, which makes them errors again in both.

**Identity.** On Linux, `-dM` lists TCC 0.9.27's predefined macros
(`tcc_new` and `tcc_set_output_type` in `libtcc.c`) in TCC's spellings
(`__SIZE_TYPE__ unsigned long`, `__INT_MAX__ 0x7fffffff`): `__TINYC__ 927`,
`__STDC_VERSION__ 199901L`, `__x86_64__`, `unix` and the `__unix`,
`__linux` forms (not `linux`), the type and size macros, glibc's
`__REDIRECT` and `__REDIRECT_NTH`, `__builtin_extract_return_addr` and
`__BASE_FILE__`; `-funsigned-char` adds `__CHAR_UNSIGNED__`, `-std=c11`
`__STDC_VERSION__ 201112L` with `__STDC_NO_ATOMICS__`,
`__STDC_NO_COMPLEX__`, `__STDC_NO_THREADS__`, `__STDC_UTF_16__` and
`__STDC_UTF_32__`, `-O1` and higher `__OPTIMIZE__`, and `-pthread`
`_REENTRANT`. There is no GNU or Clang macro, no `__ATOMIC_*`, `__SSE2__`,
`__ELF__`, `__PIC__`, `__SIZEOF_INT128__` or `__STDC_MB_MIGHT_NEQ_WC__`.
bcc's own `<stdatomic.h>` defines the `__ATOMIC_*` memory orders itself
when they are not predefined, as `mob`'s `<stdatomic.h>` does, and bcc's
freestanding `<stdint.h>` spells out the LP64 types and limits that the
other identities' `__INT8_TYPE__` family names, as TCC's `<stddef.h>`
spells out its exact-width types, with the target C library's
`int_fast16_t` and `int_fast32_t` (glibc's 64-bit `long`, musl's 32-bit
`int`). bcc keeps these besides, each for a reason:

| Macros | Why bcc keeps them |
| --- | --- |
| `__bcc__`, `__bcc_major__`, `__bcc_minor__`, `__bcc_patchlevel__`, `__bcc_version__` | bcc's vendor macros, defined under every identity as the GCC and Clang modes define them; bcc's `<tgmath.h>` reads `__bcc__` |
| `__BITINT_MAXWIDTH__`, in C23 alone | bcc's `<limits.h>` reads it for `BITINT_MAXWIDTH`; only bcc's own driver (`--compiler-identity=tcc -std=c23`) selects C23 for this identity |
| `__REDIRECT_NTHNL` | `mob`'s definition (`tccdefs.h`): glibc 2.34 and later use it without checking, so `<glob.h>` with `_FILE_OFFSET_BITS=64` fails under 0.9.27 (`identifier expected`) |
| `__FLT_*`, `__DBL_*` and `__LDBL_*` characteristics (`MANT_DIG`, `DIG`, `MIN_EXP`, `MIN_10_EXP`, `MAX_EXP`, `MAX_10_EXP`, `MAX`, `MIN`, `EPSILON`, `DENORM_MIN`, `HAS_DENORM`, `DECIMAL_DIG`, `NORM_MAX`) and `__FLT_RADIX__` | bcc's `<float.h>` reads them; TCC's spells the values out |
| `__SCHAR_MAX__`, `__SHRT_MAX__` | bcc's `<limits.h>` reads them; TCC has no `<limits.h>` and takes glibc's |
| `__INT_FAST16_TYPE__`, `__INT_FAST16_MAX__`, `__UINT_FAST16_TYPE__`, `__UINT_FAST16_MAX__` and their `FAST32` counterparts | bcc's freestanding `<stdint.h>` reads them for `int_fast16_t`, `int_fast32_t` and their limits, which glibc gives 64 bits and musl 32; TCC takes the C library's `<stdint.h>`, so they come from GCC's listing for the target (Clang's listing names `short` for `int_fast16_t` with either library) |
| `__GCC_ATOMIC_*_LOCK_FREE` | bcc's `<stdatomic.h>` reads them for `ATOMIC_*_LOCK_FREE`; 0.9.27 has no `<stdatomic.h>` and `mob`'s defines no lock-free macros |
| `__STDC_EMBED_EMPTY__`, `__STDC_EMBED_FOUND__`, `__STDC_EMBED_NOT_FOUND__` | they come with `#embed`, which the personality accepts as GCC accepts it in `gnu99` (see the table below) |

TCC's `__builtin_offsetof` macro is not defined: bcc's builtin of that name
gives the same value as a constant expression. The `__has_include`,
`__has_attribute`, `__has_builtin`, `__has_feature` and `__has_c_attribute`
queries stay defined to `#ifdef`, as `-dM` lists none of them, where 0.9.27
has none (bcc's own headers use `__has_include_next`). On the other
targets (Windows and WebAssembly) the identity keeps the target's macros
without the GNU and Clang ones. TCC's
diagnostics are `file:line: error: message`, one line each with no notes,
and TCC stops at the first error, so bcc prints the warnings before the
first error and that error alone in this format.

**Language.** 107 probes of TCC's documented extensions and quirks, run
with `-run` under both TCCs and the personality, agree in what is accepted
and what runs, except where noted:

| Feature | TCC (both) | bcc `--tcc-mode` |
| --- | --- | --- |
| Statement expressions, `typeof`, `&&label` and computed `goto`, case ranges, `?:` without a middle operand, zero-length arrays, `__attribute__((packed, aligned, section, weak, alias, constructor, cleanup))`, `asm` with operands, `__builtin_expect`, `__builtin_constant_p`, `__builtin_types_compatible_p`, `__builtin_offsetof`, `__builtin_alloca`, VLAs, `_Generic`, `_Static_assert`, `_Alignas`, `_Noreturn`, anonymous members, `$` in identifiers, K&R definitions, implicit `int`, designated initializers with ranges, compound literals, `void *` arithmetic, empty structures (size 0) | accepted | same |
| Nested functions | `error: cannot use local functions` | refused (`not yet supported by lowering`) |
| `asm goto` | `mob` only | accepted |
| `__attribute__((vector_size))` | `'}' expected` | refused (`not yet supported by lowering: vector types`) |
| `__builtin_clz` | declared by `tccdefs.h`; link error (`unresolved reference`) without `libtcc1` | builtin |
| `__builtin_bswap32`, `__builtin_add_overflow`, any `__builtin_` name TCC lacks | `warning: implicit declaration of function '__builtin_bswap32'`, an ordinary function, so the link fails unless the program defines it | same |
| `_Complex` | `error: _Complex is not yet supported` (`mob`), parse error (0.9.27) | supported |
| `_Atomic`, `<stdatomic.h>` | `mob` only | supported |
| `__int128`, `__float128` | undeclared | types, with no size macros |
| `_Thread_local`, `__thread` | `mob`: refused with `-run`; 0.9.27: parse error | supported |
| `__VA_OPT__`, C23 `[[attributes]]`, `#embed` | refused (TCC warns that `#embed` is an unknown directive and ignores it) | accepted, as GCC accepts them in `gnu99` |
| `bool`, `nullptr`, `constexpr`, `typeof_unqual`, `static_assert(1)`, `u8""`, digit separators | refused | refused |
| `0b1011`, `auto x = 3.5` (implicit `int`) | accepted | same |
| `long double` | 16 bytes, 64-bit significand | same |
| Bit-field layout and packing (`packed`, `#pragma pack`, `:0`, 40-bit `long long` fields) | GCC's | same |
| `alignas(32)` on a local | not always honoured (`mob` and 0.9.27 print 0 for the address test about one run in three) | honoured |
| Plain C99 `inline` definition with no external one | a local function (`nm`: `t f`), so the program links | GCC's rule: no external definition, `undefined symbol: f` at link time |
| Assignment to a `const` object, `return 1;` in a `void` function | warnings (`assignment of read-only location`, `void function returns a value`) | same |
| Implicit function declaration | `warning: implicit declaration of function 'foo'` | same |
| Missing `return` in a non-`void` function | `warning: function might return no value: 'f'`, except in `int main` and in an `inline` function nothing calls, which waits for the end of the unit | same, from bcc's control-flow analysis; see [warnings](#tccs-warnings) |

Errors keep bcc's wording. The common ones read:

| Error | TCC (`mob`; 0.9.27 quotes the token with `"`) | bcc |
| --- | --- | --- |
| undeclared identifier | `'y' undeclared` | `identifier has no visible declaration: `y`` |
| missing `;` | `';' expected (got 'return')` | `expected an operator, found keyword `return`` |
| member | `field not found: b` | `member access requires a declared record member` |
| arguments | `too few arguments to function` | `function call has the wrong number of arguments` |
| redeclaration | `incompatible types for redefinition of 'a'` | `redeclaration has an incompatible type: `a`` |
| lvalue | `lvalue expected` | `operator requires a modifiable lvalue` |
| header | `include file 'nope.h' not found` | `cannot find header `nope.h`` |
| `break` | `cannot break` | `break is outside an iteration or switch statement` |
| `goto` | `label 'nowhere' used but not defined` | `goto target label is not defined: `nowhere`` |
| link | `tcc: error: unresolved reference to 'f'` (0.9.27: `undefined symbol 'f'`) | `tcc: error: cannot build `a.out`: clang failed` and `ld.lld: error: undefined symbol: f` |

### TCC's warnings

The warnings `tccgen.c` gives by default (its `tcc_warning` calls) read in
TCC's words, which 0.9.27 and `mob` share, wherever bcc reports the same
condition:

| TCC's warning | Where |
| --- | --- |
| `cast between pointer and integer of different size` | a cast, or an assignment's, initializer's, argument's, `return`'s or `?:` arm's conversion, between a pointer and an integer of another width (below) |
| `assignment makes pointer from integer without a cast`, `... integer from pointer ...`, `assignment from incompatible pointer type`, `assignment discards qualifiers from pointer target type` | any assignment-like conversion |
| `pointer/integer mismatch in comparison`, `... in conditional expression` | a pointer against an integer |
| `pointer type mismatch in comparison('int *' and 'char *')`, `... in conditional expression ('int *' and 'char *')` | incompatible pointers, with bcc's spelling of the types |
| `function might return no value: 'f'` | control reaches the end of a function that returns a value, other than a `main` returning TCC's `VT_INT`: `int`, `unsigned int`, an enumeration TCC represents as one of those, or a 32-bit `long` (below) |
| `unnamed struct/union that defines no instances` | a structure or union with no tag and no declarator, a `typedef` among them, at the line of its `;` |
| `static storage ignored for redefinition of 'f'` | a `static` declaration of a function after one with external linkage, neither it nor TCC's `inline` flag of the function so far `inline` (below); the function stays external |
| `storage mismatch for redefinition of 'x'` | a declaration of an object whose `static` differs from its first declaration's, `extern` after `static` among them; the object keeps the first declaration's linkage |
| `__mode__(X) not supported`, then an empty line | a `__mode__` other than `__QI__`, `__HI__`, `__SI__`, `__DI__` and `__word__`, which leaves the type alone |
| `integer constant overflow` | an integer constant whose digits overflow 64 bits, or a decimal one without `u` of 2<sup>63</sup> or more, which keeps its wrapped value (typed `unsigned long` unless it says `ll`) |
| `deprecated use of label at end of compound statement` | a label, `case` or `default` before a `}`, at the `}` |
| `multi-character character constant` | a character constant of more than one byte, wide ones and those in `#if` included |
| `A redefined`, `Ignoring unknown preprocessing directive #foo`, `unknown escape sequence: '\q'` (`'\x1'` for a byte that is not printable ASCII) | `tccpp.c`'s words for what bcc reports |
| `assignment of read-only location`, `void function returns a value`, `'return' with no value`, `implicit declaration of function 'f'`, `empty case range`, `initializer-string for array is too long` | as worded |

TCC refuses a declaration of no declarator whose type is not a structure,
union or enumeration (`int;`, `typedef int;`, `const;`) with `identifier
expected`, and accepts the others silently. It ignores a `section`
attribute on anything but a function or a static object, a `typedef`
among them, and the `__mode__` attribute after a declarator or spelled
`mode`; it applies `__word__` as `int`. It applies a mode where it reads
it among the declaration specifiers, so a type specifier after it other
than `int`, `signed` or `unsigned` replaces it (`__attribute__((
__mode__(__word__))) long` is `long`, `__attribute__((__mode__(__QI__)))
enum E` an enumeration), unless another attribute after that applies the
mode again. A wide character constant of
several characters takes the last, and a narrow one packs its bytes as
signed `char`s, as `'\xff\x01'` is `0xffffff01`.

A `static` declaration of a function after one with external linkage
keeps TCC's `inline` flag where it or the flag so far says `inline`
(`patch_type`), and TCC gives a function whose flag is set when the unit
ends a local symbol (`update_storage`). So `int f(void); static inline
int f(void) {...}`, or `static inline int rand(void) {...}` after
`<stdlib.h>`, gives the function internal linkage at every declaration,
and the calls before the definition bind to the local function, as under
TCC. A
later declaration that clears the flag (`int f(void);` again) would make
the function external once more, and one after a definition the earlier
declarations left external (`inline int f(void) {...} static int
f(void);`) would make it internal after its body; bcc refuses both with
`identifier has both internal and external linkage`, as it refuses these
redeclarations in the other identities.

What `function might return no value` sees is TCC's: the C library's
declarations lose their attributes, as glibc's `<sys/cdefs.h>` defines
`__attribute__` away without `__GNUC__`, so a call of `exit`, `abort` or
`longjmp` ends no path unless a declaration says `_Noreturn` or
`noreturn` itself, and a statement expression ends none, a `return` inside
it included. TCC keeps an `inline` definition that neither it nor an
earlier declaration says `extern`, and that every earlier declaration also
says `inline` or one of them `static`, as tokens, and compiles it after
the rest of the unit only if code it compiles uses it (or a later
declaration drops the `inline`), in passes over them in order
(`gen_inline_functions`). Its warning comes then, after all the others,
and 0.9.27 names the line before the closing brace (`inline.h:0` for a
function on a header's first line), as it replays the tokens.
When several warnings fall on one line and semantic analysis reports all
of them, they come in the order it reports them, which is TCC's order of
compilation (`return c ? p : q;` reports the conditional's mismatch before
the conversion the `return` makes); otherwise in the order of their
places.

TCC's cast warning (`gen_cast`) differs from GCC's: it counts an
enumeration as its integer type (`(enum E)p`) and a `_Bool` source as a
byte (`(void *)b`; a `_Bool` destination tests the pointer), passes over an
operand that folds to a constant that is no address (`(void *)(1 ? 2 : i)`,
`(int)(char *)5`, but not `(int)&g`), never warns in a static object's
initializer, which it only folds, and warns of the conversions an
assignment, an initializer, a prototyped argument, a `return` or an integer
arm of `?:` makes as well as of casts: `int x = "s";` draws both
`assignment makes integer from pointer without a cast` and the cast
warning.

These still differ from TCC 0.9.27:

- TCC defers every diagnostic of a deferred `inline` function's body, and
  says nothing of one nothing uses; bcc defers only `function might return
  no value` and reports the rest where they stand.
- The `static` redeclarations that bcc refuses above are accepted, TCC
  compiling a local or a global function, as the flag ends.
- A label before the `}` that ends an `if` or loop body that is no block
  (`if (x) l: }`) is an error, where TCC warns and closes the enclosing
  block.
- A constant in a macro's replacement list draws `unknown escape sequence`
  and, in `#if`, `multi-character character constant` at the `#define`'s
  line, where TCC names the line of each use.
- Under `-Werror`, a warning of preprocessing is the error bcc stops at
  wherever it falls, as preprocessing reports before the other phases: in
  `int f(void) { }` followed by `char *s = "\q";` it is the escape, where
  TCC stops at `function might return no value`. A parser's or semantic
  analysis's warning is the first by place, as under TCC.
- A line with a warning from preprocessing or parsing as well as those of
  semantic analysis orders them all by place, so in `char *s = "\q"; int
  f(int c, int *p, char *q) { return c ? p : q; }` the conversion comes
  before the conditional's mismatch.

Of the repository's `tests/fixtures/programs`, those both TCCs compile
(the `long double`, `<math.h>`, `scanf`, `stdio`, wide-stream, POSIX
console and `setjmp` programs) print the same as under bcc with `mob`;
0.9.27 differs in `gamma_functions.c` and `long_double_math.c`, where it
prints `ffff:c000000000000000` (a negative NaN) for `lgammal` and `fmal`
results that `mob`, GCC and bcc give as positive NaNs or zero. The others
use features TCC lacks.

**ABI.** Objects from `--tcc-mode -c` link with TCC's (with TCC as the
linker, or with bcc given TCC's `libtcc1.a`, which TCC's objects call for
`__va_arg` and friends) and agree on scalars, `long double`, varargs of
scalars and `long double`, structures in memory (larger than 16 bytes, or
holding a `long double`), structures of one class (`{int, int}`, `{float,
float}`, `{short, char[5]}`), `_Bool` and narrow parameters, and the
layout and passing of bit-field structures. They do not agree on the
structures of at most 16 bytes that mix integer and floating members or
hold an array of floats (`{double, long}`, `{int, double}`, `{char,
double}`, `{float[3]}`), passed or returned directly or through `...`: TCC
classifies such a structure as a whole (`classify_x86_64_inner` in
`x86_64-gen.c` merges the members' classes) rather than by eightbyte as
the x86-64 psABI requires, so `{double, long}` travels in two integer
registers. bcc follows the psABI and agrees with GCC there, as both TCCs
disagree with GCC; it does not copy TCC's classification.

## zig cc

`zig cc` is Clang 22.1.8's driver linked into the `zig` executable, behind a
command line zig reads first (`src/main.zig`, the `cc` loop) and a bundled C
library for every target it supports. The personality was compared with zig
0.17.1 (the `x86_64-linux` release tarball) on x86-64 Linux with glibc 2.39 on
2026-11-19, over 94 command lines (`--version` for each target,
the target-query, CPU and option errors, `-dM -E` under each mode and target,
missing inputs, the diagnostics probes of the TCC comparison, and programs
linked and run); 59 are identical byte for byte, after each
compiler's install directory and bcc's banner note are replaced, and the rest
differ only as listed below. The opt-in test `zig_cc_parity_with_a_real_zig`
in `tests/integration/zig_cc_cli.rs` repeats part of the comparison when
`BCC_ZIG` names a `zig` binary.

zig's driver is Clang's personality plus what zig reads itself, so the
personality is a translator in front of the Clang personality
(`personality/zig.rs`): it reads zig's own options, reports zig's errors, and
hands every other option to the Clang personality with the options zig gives
Clang before them. Everything the [Clang](#clang) section says holds, except
what follows.

**Selecting it.** `--zig-cc-mode` (`--driver-mode=zig-cc`), or an executable
named `zig-cc` or `zigcc`, or one named `zig` whose first argument is `cc`, as
build systems run it when given `CC="zig cc"`. A `zig` given another command
`zig help` lists (`build`, `version`, `env`, ...) says `error: 'zig build' is
not supported: bcc implements only 'zig cc'`, or
zig's `error: unknown command: foo` or `error: expected command argument`,
without zig's usage text. The driver's messages say `zig:`, as Clang's driver
inside `zig` names itself.

**What zig reads itself.**

| zig cc | bcc |
| --- | --- |
| `-target QUERY`, `--target=QUERY` | the target ([Targets](#zig-cc-targets)); `--target QUERY` is `error: unknown clang option: "--target"`, and `-target` alone `error: expected parameter after "-target"` |
| `-mcpu=CPU`, and `-march=` and `-mtune=` read the same way | `CPU` is one of zig's processors, `NAME+FEATURE-FEATURE` with zig's spelling of LLVM's names (`x86_64_v3`, `sse4_2`): `-march=` with LLVM's (`x86-64-v3`), and `-mFEATURE` or `-mno-FEATURE` for each feature Clang has an option for; `baseline` and `x86_64` add nothing. GCC's `-march=x86-64-v3` reads as processor `x86` with features, which zig refuses |
| no `-O`, `-O0`, `-Og` | Debug: `-O0`, UBSan, `-fstack-protector-strong` |
| `-O1` to `-O4`, `-Ofast` | ReleaseFast: `-O2 -DNDEBUG`; `-Ofast` brings no fast math. With UBSan's checks on (`-fsanitize=undefined` or `-fsanitize-trap=undefined`), ReleaseSafe instead, as zig makes it after reading the command line: `-O2 -D_FORTIFY_SOURCE=2`, the stack protector as in Debug, and no `NDEBUG` |
| `-Os`, `-Oz` | ReleaseSmall: `-Os -DNDEBUG`, stripped as by `-s` (no DWARF, and a link without symbols) unless a `-g` option keeps the debug information; the checks leave it ReleaseSmall |
| other `-O` options (`-O`, `-O5`) | Clang's, after zig's level |
| `-f[no-]sanitize=` naming `undefined`, `thread` or a fuzzer | taken whole, the other names in the list dropped, as zig does: `undefined` turns zig's checks on or off in any mode (and makes ReleaseFast ReleaseSafe), `thread` is passed back alone, and a fuzzer is refused |
| `-f[no-]sanitize-trap=undefined` | trapping checks, or reporting ones; `-fno-sanitize-trap=undefined` alone turns the checks off |
| `-f[no-]PIC`, `-f[no-]pic`, `-f[no-]PIE`, `-f[no-]pie` | on Linux, `-fPIE` or `-fno-PIE` and then `-fPIC` or `-fno-PIC`, as zig passes them, so Clang's reading makes `-fPIE` PIC alone; PIC by default where zig links glibc dynamically, `-shared` or `-fPIE`; executables link with `-no-pie` unless `-fPIE` |
| `-f[no-]omit-frame-pointer` | frame pointers kept unless `-fomit-frame-pointer` |
| `-fstack-protector`, `-fstack-protector-strong`, `-fstack-protector-all`, `-fno-stack-protector` | `-fstack-protector-strong` or none |
| `-g`, `-g1`, `-gline-tables-only`, `-gdwarf32`, `-gdwarf64`, `-s` | DWARF 4 (`-gdwarf-4`) unless `-s` strips, which also strips the symbols, or ReleaseSmall does; other `-g` options are Clang's after it |
| `-x LANGUAGE` | zig's languages (`c`, `c-header`, `assembler`, `assembler-with-cpp` and the C++ and Objective-C ones); another is `error: language not recognized: "bogus"` |
| `-flto=MODE` | `auto`, `full`, `thin` or `jobserver`; another is `error: invalid -flto mode "bogus"; must be "auto", "full", "thin", or "jobserver"` |
| `-static`, `-dynamic`, `-shared`, `-nostdlib`, `-nodefaultlibs`, `-nolibc` | the link, and whether zig links the C library at all |
| `-Wl,--version`, `-Wl,-v` | `zig ld 0.17.1 (bcc 0.1.0; not zig)` on stdout at once, which Meson reads as zig's linker; with no input, nothing more |

The last `-O` option chooses the mode; zig gives Clang its own level and
macros first and the command line's options after them, so `-O2 -UNDEBUG`
leaves `NDEBUG` undefined.

<a id="zig-cc-targets"></a>**Targets.** A query is
`ARCH-OS[.MIN[...MAX]]-ABI[.VERSION]` (`std.Target.Query.parse`), where
`native` stands for the machine's architecture or system. bcc parses it as
zig does and refuses what zig refuses in zig's words:
`error: unable to parse target query 'x86_64-unknown-linux-gnu':
UnknownOperatingSystem`, and likewise `MissingOperatingSystem`,
`UnknownApplicationBinaryInterface`, `InvalidOperatingSystemVersion`,
`InvalidAbiVersion` and `UnexpectedExtraField`; an unknown architecture lists
zig's architectures before `error: unknown architecture: 'bogus'`. Of the
targets zig accepts, bcc has these:

| Query | bcc target | zig's LLVM triple (`--version`) |
| --- | --- | --- |
| none, `native`, `x86_64-native` | `x86_64-unknown-linux-gnu` (on Linux) | `x86_64-unknown-linux6.8.0-gnu2.39.0`: the running kernel and glibc |
| `x86_64-linux-gnu[.VERSION]` | `x86_64-unknown-linux-gnu` | `x86_64-unknown-linux5.10.0-gnu2.31.0`, or the version given |
| `x86_64-linux`, `x86_64-linux-musl` | `x86_64-unknown-linux-musl` | `x86_64-unknown-linux5.10.0-musl` |
| `x86_64-windows[-gnu]` | `x86_64-w64-windows-gnu` | `x86_64-unknown-windows-gnu` |
| `x86_64-windows-msvc` | `x86_64-pc-windows-msvc` | `x86_64-unknown-windows-msvc` |
| `wasm32-wasi[-musl]` | `wasm32-unknown-wasi` | `wasm32-unknown-wasip1-unknown` |
| `wasm32-freestanding` | `wasm32-unknown-unknown`, freestanding | `wasm32-unknown-unknown-unknown` |

Any other target zig has (`aarch64-linux-gnu`, `x86_64-macos`,
`x86_64-linux-gnux32`) is `error: unable to compile for target
'aarch64-linux-gnu': bcc compiles for ...`. A Linux version range keeps its
minimum. Without a `-target`, or with `native` as the architecture, zig
compiles for the machine's processor, `-march=native`; with any other, for
the baseline processor. An unknown `-mcpu` processor or feature lists zig's
processors or features for the architecture (`info: available CPUs for
architecture 'x86_64':`) before `error: unknown CPU: 'x86'` or `error:
unknown CPU feature: 'bogus'`.

**What zig gives Clang**, in every compilation, before the command line's
options (`addCCArgs` in `src/Compilation.zig`):

- Clang 22.1.8's identity over the pinned Clang's: `__clang_major__` 22,
  `__clang_patchlevel__` 8, `__clang_version__ "22.1.8 "` and `__VERSION__
  "Clang 22.1.8"`, without the four macros Clang 23 added
  (`__SIG_ATOMIC_MIN__`, `__SIG_ATOMIC_TYPE__`, `__WCHAR_MIN__`,
  `__WINT_MIN__`); bcc's `<stdint.h>` spells `WCHAR_MIN` and `WINT_MIN`
  without them. `__bcc__` stays.
- `-DNDEBUG` in ReleaseFast and ReleaseSmall, `-D_FORTIFY_SOURCE=2` in
  ReleaseSafe.
- Where zig links the C library (not under `-nostdlib`, `-nodefaultlibs` or
  `-nolibc`, nor for `wasm32-freestanding`): `-D__GLIBC_MINOR__=N` for glibc,
  the query's minor version, the machine's when zig detects it, or 31; and
  `-D__MSVCRT_VERSION__=0xE00 -D_WIN32_WINNT=0x0a00 -D__CRT__NO_INLINE` for
  MinGW, `_WIN32_WINNT` from the query's Windows version (`win7` is
  `0x0601`).
- In Debug, on glibc's targets, UBSan's checks (`-fsanitize=undefined`) that
  stop at the first report, as zig's own library does. zig always has that
  library; where the bundled clang was built without compiler-rt's
  `libclang_rt.ubsan_standalone.a` (as CI's pinned LLVM is), the checks trap
  instead (`-fsanitize-trap=undefined`), so the program still stops at the
  first one, by `SIGILL` and without a report; the stack protector,
  in Debug and ReleaseSafe where zig links the C library, on Linux; frame
  pointers; PIC as above; DWARF 4 outside ReleaseSmall; and `-ffreestanding`
  for `wasm32-freestanding`.

So `-dM -E` under each mode and target lists what zig cc's does, apart from
the [Clang personality's own differences from Clang](#deviations) and those
below.

**Errors.** zig's own errors are `error: MESSAGE` with no driver name, and
stop it at once, so only the first is reported: `error: unknown clang option:
"-fbogus"` for an option Clang does not know (where Clang's driver says
`unknown argument`); for a missing source, after the whole command line is
read, `missing.c:1:1: error: checking cache failed: file_open FileNotFound
/abs/missing.c` for each; for a missing object `error: failed to open
object missing.o: FileNotFound`; for a response file it cannot read, at any
depth, `error: unable to read response file "missing.rsp": FileNotFound`;
`-fno-PIC` where zig needs PIC, `error:
unable to create module "e": the selected target requires position
independent code` (glibc, Windows) or `dynamic linking requires enabling
Position Independent Code` (`-shared`), or with `-fPIE` `making a Position
Independent Executable requires enabling Position Independent Code` (musl),
the module named after the first source; `-shared -fPIE` when linking,
`error: dynamic libraries cannot be position independent executables`; and `-static` with a glibc target that is not the machine's, `error:
libc of the specified target requires dynamic linking`. Nothing runs after any
of them. zig hands each source to Clang's compiler itself, so Clang's
driver warnings about unused arguments and inputs are not printed. With no
input at all it is Clang's driver that answers, `zig: error: no input files`.

**Queries.** `--version` prints Clang's banner with zig's LLVM triple, naming
bcc as every banner does (`clang version 22.1.8 (bcc 0.1.0, a zig
cc-compatible C compiler; not zig)`). `-dumpversion` is `22.1.8`. zig hands
`-dumpmachine` and `-print-target-triple` to Clang's driver with the query as
written, so they print Clang's normalized reading of it
(`x86_64-unknown-windows-msvc` for `x86_64-windows`, `wasm32-freestanding`
unchanged) or, with no `-target`, the driver's own `x86_64-unknown-linux-musl`;
a glibc version is Clang's `zig: error: version '.2.17' in target triple
'x86_64-unknown-linux-gnu.2.17' is invalid` before the answer, exit status 1.
`-v` and `-###` without inputs print the same banner for that triple.

**What differs from zig cc:**

- **The C library.** zig builds and links its own: musl, MinGW-w64, wasi-libc,
  and glibc stubs of the version the query names, so a `.2.17` program links
  against glibc 2.17's symbol versions. bcc uses the machine's glibc for the
  glibc targets and the selected root (`--sysroot`) for the others, as the
  Clang personality does: the glibc version is only `__GLIBC_MINOR__` (which
  the machine's `<features.h>` redefines once included), and `-target
  x86_64-linux-musl` fails at `<stdio.h>` without a musl root, where zig
  links a static musl program.
- **UBSan's reports.** zig's library panics at the first report, prints
  zig's stack trace and aborts (`thread 1 panic: signed integer overflow:
  2147483647 + 1 cannot be represented in type 'int'`, exit status 134);
  bcc's reports come from the bundled clang's library, stopping at the first
  (`ub.c:5:5: runtime error: signed integer overflow: ...`, status 1). The
  messages are Clang's in both. bcc writes the checks only for glibc's
  targets, so Debug on musl, Windows and WebAssembly has none, and no stack
  protector off Linux, where zig has both (`__SSP_STRONG__` is undefined
  there). A shared library links the bundled clang's static UBSan library
  with its symbols hidden, as zig links its own, so `-shared
  -Wl,--no-undefined` still links.
- **No `-Wdate-time`.** zig's release modes pass `-Werror=date-time`, which
  makes `__DATE__`, `__TIME__` and `__TIMESTAMP__` errors; bcc has no such
  warning, so they expand. zig's `-fno-spell-checking` changes nothing, since
  bcc suggests no corrections.
- **The processor.** bcc's `-march=native` defines the features it knows of
  the machine (here AVX2 and below, not the AVX-512 family and the 38 other
  macros zig's `znver5` defines). zig's WebAssembly baseline is `lime1`
  (`__wasm_extended_const__`, no bulk memory or reference types), where bcc
  keeps Clang's `generic`; `-mcpu` names for WebAssembly are checked and
  otherwise ignored. A feature without a Clang option (`+fast_lzcnt`) is
  checked and dropped.
- **The linker.** zig reads every `-Wl,` option itself and refuses those it
  does not know (`error: unsupported linker arg: --bogus-opt`), and resolves
  `-l` libraries even under `-c`; bcc passes them to LLD, which reports its
  own errors when it links. zig's link line is its own, so CMake finds no
  linker ID for it and implicit link libraries of zig's (`libubsan_rt.a`,
  `libcompiler_rt.a`); bcc's are LLD and the bundled clang's.
- **Clang's own differences.** Diagnostics, `-E`'s markers and the `-dM`
  listing differ from zig cc exactly as the Clang personality's differ from
  Clang ([Deviations](#deviations)): on the 10 diagnostics probes bcc's
  output under `--zig-cc-mode` is the same as under `--clang-mode`. Clang 22
  defines `__GXX_TYPEINFO_EQUALITY_INLINE` for MinGW where 23 does not, and
  the MSVC target lacks the `_MSC_VER` family as under `--clang-mode`.
- **zig's quirks.** `-c` with several sources and `-o` links them into one
  object, and `-fsyntax-only` still compiles and fails to rename the object
  zig never wrote; bcc refuses the first as Clang does and checks the syntax
  for the second. `-v` and `-###` print bcc's steps, not zig's `-cc1` lines.

## icx

Intel's oneAPI DPC++/C++ Compiler `icx` is Clang 23.1.1's driver behind a
wrapper of Intel's own (`bin/icx`), which reads Intel's options first, and
an Intel build of Clang that adds Intel's defaults and identity. On Linux
`icx` (and `icx-cc`) reads Clang's command line; `icx-cl` reads clang-cl's,
and on Windows `icx` does too. The personalities were compared with icx
2026.2.0 (build 20261001, the `intel-oneapi-dpcpp-cpp-2026.2` packages of
Intel's apt repository, unpacked without root) on x86-64 Linux with glibc
2.39 on 2026-11-19, over 99 `icx` and 34 `icx-cl` command lines (the
queries, the classic-option warning, Intel's options and their errors,
`-dM -E` under each model, level and processor, preprocessing, programs
linked and run, and the diagnostics probes of the TCC comparison). For
`icx`, 80 are identical byte for byte after each compiler's install
directory, bcc's banner note and the configuration file icx's banner names
are replaced, with each `-dM` listing compared as a set of definitions
without the macros in which the Clang personality differs from the pinned
Clang already; the rest differ only as listed below. The opt-in test
`icx_parity_with_a_real_icx` in `tests/integration/icx_cli.rs` repeats part
of the comparison when `BCC_ICX` names an `icx` binary.

Like zig cc's, the personality is a translator in front of the Clang
personality (`personality/icx.rs`): it reads Intel's options, reports the
wrapper's warning and errors, and hands every other option to the Clang
personality with what Intel's driver gives Clang before them. Everything
the [Clang](#clang) section says holds, except what follows. `icx-cl` is
the [clang-cl](#clang-cl) personality with Intel's identity and defaults
([icx-cl](#icx-cl)).

**Selecting it.** `--icx-mode` (`--driver-mode=icx`), or an executable named
`icx` or `icx-cc`, also with a prefix ending in `-` or a version
(`bcc-icx`, `icx-2026`), as the other personalities' names are matched
(`gicx-cc` is GCC's `cc` and `myicx-cl` is `cl`). Intel's driver calls itself `icx` whatever it was run as
(`icx-cc` says `icx: error:`), and so does bcc. `icc`, the classic compiler
icx replaced, does not select it: Intel withdrew `icc` in oneAPI 2024.0 and
installs no `icc` that runs icx, and a build system that finds `icc` expects
`__INTEL_COMPILER`, which icx does not define.

**Identity.** Clang 23.1.1's macros, as the pinned Clang's with
`__clang_patchlevel__` 1, and Intel's:

| Macro | Value |
| --- | --- |
| `__INTEL_LLVM_COMPILER`, `__INTEL_CLANG_COMPILER` | `20260200` (`VVVVMMUU`) |
| `__clang_version__` | `"23.1.1 (icx 2026.2.0.20261001)"` |
| `__VERSION__` | `"Intel(R) oneAPI DPC++/C++ Compiler 2026.2.0 (2026.2.0.20261001)"` |
| `__DPCPP__`, `__dpcpp_major__`, `__dpcpp_minor__`, `__dpcpp_patchlevel__` | `1`, `2026`, `2`, `0` |
| `__AMX_SUPPORTED__`, `__GCC_HAVE_SYNC_COMPARE_AND_SWAP_16` | `1` |
| `__I__` | `1j` |
| `__LONG_DOUBLE_SIZE__` | `80` (`64` under `icx-cl`) |
| `__FINITE_MATH_ONLY__` | `0`, even under `-ffast-math`, as Intel's Clang reports it |

`__INTEL_COMPILER`, the classic compiler's, stays undefined, as in icx.
CMake reads `__INTEL_LLVM_COMPILER` (`IntelLLVM`), and Meson the banner.
`--version` prints `Intel(R) oneAPI DPC++/C++ Compiler 2026.2.0
(2026.2.0.20261001)`, naming bcc after it, then Clang's target, thread model
and installation lines; the banner has no `clang` in it, which Meson would
read as Clang. `-dumpversion` is `23.1.1`.

**Defaults that differ from Clang**, each overridden by the command line's
own options:

- **`-O2`** without an `-O` option, but **`-O0`** when a debug option (`-g`,
  `-gdwarf-4`, `-g0`, `-debug`) is given without one, with Intel's remark
  `icx: remark: note that use of '-g' without any optimization-level option
  will turn off most compiler optimizations similar to use of '-O0'; use
  '-Rno-debug-disables-optimization' to disable this remark
  [-Rdebug-disables-optimization]`, naming the last debug option, after
  the driver's errors about unknown options and before a query's answer;
  `-w` leaves it, `-Rno-debug-disables-optimization` takes it away.
- **The `fast` floating-point model**: Clang's `-ffp-model=fast`
  (`-ffast-math` without finite math), so `__NO_MATH_ERRNO__` is defined
  and `__FAST_MATH__` is not. A later `-ffp-model=` or `-fp-model=`
  replaces it without Clang's warning about an overridden model, since the
  command line gave none; `-Ofast` reports the complex range the default
  implied (`'-Ofast' sets complex range to "basic" overriding the setting of
  "promoted" that was implied by '-ffp-model=fast'`).
- **Code at a fixed address**: no `__pic__` or `__pie__`, and executables
  linked with `-no-pie` unless `-pie`, `-shared` or `-r` asks otherwise, as
  icx links them (`-fPIE` alone leaves the executable at a fixed address).
- **`libm`** is linked whether or not `-lm` asks for it, as icx links it with
  Intel's math libraries, so Autoconf's `AC_SEARCH_LIBS([sqrt], [m])` finds
  `none required`.
- **Subnormals flushed from an optimized `main`.** icx decides the flush
  when it compiles `main`, not when it links: above `-O0`, under a model
  that flushes, its back end begins `main` with `stmxcsr`, `orl $0x8040`
  and `ldmxcsr`, setting MXCSR's flush-to-zero and denormals-are-zero bits
  for the rest of the process, and bcc writes the same three instructions
  there. The last of these options decides whether the model flushes:
  `-ftz`, `-ffast-math`, `-funsafe-math-optimizations`, `-Ofast` (wherever
  it stands) and the `fast` models flush; `-no-ftz`, `-fno-fast-math`,
  `-fno-unsafe-math-optimizations`, `-fdenormal-fp-math=ieee` and the
  `precise` and `consistent` models keep subnormals. So `-O0`, `-g` alone,
  `-no-ftz` and `-fp-model=precise` keep them, and so does a program whose
  `main` was compiled so, however it is linked (Meson's `c_args`, CMake's
  compile options and its Debug configuration never reach the link). Like
  icx, bcc links `crtfastmath.o`, whose constructor sets the same bits, only
  where Clang's driver would for the options of the link command itself
  (the last of `-ffast-math`, `-funsafe-math-optimizations`,
  `-ffp-model=fast` and `-fp-model=fast` and their opposites), after any
  `-Ofast`, and never after `-mno-daz-ftz`; the default model links none.

**Intel's options.** Of Intel's own (those `icx --autocomplete` lists beside
the pinned Clang's, and the wrapper's):

| icx | bcc |
| --- | --- |
| `-xCODE` (any case) | the processor's x86-64 level and the AVX, F16C and PCLMUL options bcc has: `-xHost` is `-march=native`, `-xSSE3` to `-xSSE4.1` the SSE options, `SSE4.2` and the Atom codes `-march=x86-64-v2`, `AVX`, `CORE-AVX-I` and the Sandy and Ivy Bridge names v2 with AVX, and `CORE-AVX2` and every newer processor name (`SKYLAKE-AVX512`, `SAPPHIRERAPIDS`, `AVX10.1`) `-march=x86-64-v3`; another `-x` is read as a language, as icx reads the codes it dropped (`KNL`, `MIC-AVX512`) and `SSE2` |
| `-axCODE,...`, `-mauto-arch=CODE` | checked (`unknown target CPU 'bogus' for option 'ax'`, after the driver's other errors) and dropped: they add code paths beside the baseline |
| `-fp-model=MODEL`, `-fp-model MODEL` | `fast` and `fast=1` are `-ffp-model=fast`, `fast=2` `-ffp-model=aggressive`, `precise` `-ffp-model=precise`, `consistent` precise without contraction; `strict` is refused, as bcc refuses `-ffp-model=strict`; another model, also the classic compiler's `source`, `double`, `extended` and `except`, is `unsupported argument 'source' to option '-ffp-model='`, as in icx 2026.2.0; the spelling with a space takes the next argument whatever it is |
| `-ftz`, `-no-ftz` | whether an optimized `main` flushes subnormals (above) |
| `-qopenmp`, `-fiopenmp`, `-qopenmp-stubs` | refused, as `-fopenmp` is; `-fopenmp` draws Intel's `use of '-qopenmp' recommended over '-fopenmp' [-Wrecommended-option]` first |
| `-qopenmp-simd`, `-fiopenmp-simd` | `-fopenmp-simd` with `__INTEL_HOST_OPENMP` |
| `-ipo`, `-no-ipo` | `-flto`, `-fno-lto` |
| `-ansi-alias`, `-no-ansi-alias`, `-fma`, `-no-fma`, `-strict-ansi` | `-fstrict-aliasing`, `-fno-strict-aliasing`, `-ffp-contract=fast`, `-ffp-contract=off`, `-ansi` |
| `-fstack-security-check`, `-use-msasm` | `-fstack-protector-strong` (warned about as Clang's), `-fasm-blocks` (refused as Clang's) |
| `-EP`, `-dryrun`, `-#` | `-E -P`, `-###` |
| `-fast` | `-Ofast -march=native -flto -static` |
| `-debug[=KIND]`, `-debug KIND` | `-g`, or `-g0` for `none`, turning optimization off as `-g` does; an unknown kind is `unsupported argument 'bogus' to option '-debug='` |
| `-static-intel`, `-shared-intel`, `-no-intel-lib` | how to link Intel's libraries, which bcc never links: link options Clang reports as unused when not linking |
| `-qopt-*`, `-qno-opt-*`, `-unroll[=N]`, `-inline-level=N`, `-fimf-*`, `-fp-speculation=`, `-vec`, `-fvec-*`, `-floop-*`, `-fno-alias`, `-traceback`, `-sox` and the like | tuning, reports and debug-information variants: dropped |
| `-diag-disable=IDS`, `-diag-enable`, `-diag-warning`, `-diag-error` (the IDs joined or after a space) | dropped; `-diag-disable=10430` silences the classic-option warning |
| the SYCL and offloading sub-options (`-fsycl-*`, `-fopenmp-target*`, `-Xsycl-target-*`) | dropped, since `-fsycl` and `-qopenmp` are refused |
| `-fsycl`, `-fhls` | refused: bcc compiles C, not SYCL or device code |
| `-fintelfpga` | icx's own error (`option '-fintelfpga' is not supported and has been removed from the compiler. ...`) |
| `-qmkl[=]`, `-qtbb`, `-qipp`, `-qdaal` | refused: bcc has none of Intel's libraries |
| `-regcall`, `-pc32`, `-pc64`, `-fintel-long-double-size=` other than 80, `-falign-stack=` other than `maintain-16-byte`, `-isystem-env=` | refused: they change the calling convention, the precision or the header search |
| Intel's warning groups (`-Wrecommended-option`, `-Wintel-compat`, `-Wno-debug-disables-optimization`) | dropped, also under `-Werror=` |

**The classic options.** icx drops the classic `icc` options it no longer
has, warning about them all at once before anything else, even a query's
answer, in the order given (icx ends `option list:` with a space):

```text
icx: command line warning #10430: Unsupported command line options encountered
These options as listed are not supported.
For more information, use '-qnextgen-diag'.
option list:
	-ip
	-wd10441
```

The table (`personality/icx/classic.list`) is icx 2026.2.0's
`-qnextgen-diag` list of options "to be supported" and "being removed",
less the nine entries it was found not to warn about. A `=<val>` entry
takes a value starting with a digit, joined with or without `=` (`-w2`,
`-par-num-threads=4`), a `=<string>` entry any nonempty value (`-wd10441`,
`-A=foo`), and `-tcheck`, `-pch-dir` and `-create-pch` the next argument.
The table wins over Clang's names: `-Winline`, `-Wchar-subscripts`,
`-traditional`, `-print-sysroot` and `-ffloat-store` draw the warning.

<a id="icx-cl"></a>**icx-cl.** `--icx-cl-mode` (`--driver-mode=icx-cl`), an
executable named `icx-cl`, or one named `icx` on Windows, reads clang-cl's
command line as the clang-cl personality does, with:

- Intel's logo on stderr before anything else, `/nologo` or not:
  `Intel(R) oneAPI DPC++/C++ Compiler for applications running on Intel(R)
  64, Version 2026.2.0 Build 20261001`, then a line naming bcc where Intel
  prints its copyright, and a blank line. Meson recognizes `icx-cl` by its
  first line. `--version` prints the logo and then the banner above with
  the `x86_64-pc-windows-msvc` target.
- The identity above with `__LONG_DOUBLE_SIZE__` 64, and `_ISO_VOLATILE`
  and `_UCRT_NOISY_NAN`, and `/O2` and `/fp:fast` by default: `_M_FP_FAST`
  and `_M_FP_CONTRACT`, or `_M_FP_PRECISE` under `/fp:precise`, and
  `__NO_MATH_ERRNO__`, which bcc's clang-cl identity does not define. `/Zi`,
  `/Z7` or `/ZI` without an `/O` option turns optimization off with the
  remark above naming it and `/Od`. An optimized `main` flushes subnormals
  as under `icx`, unless the last of `/fp:` and `/Qftz[-]` is
  `/fp:precise`, `/fp:strict` or `/Qftz-`. `__SSP_STRONG__` stays
  undefined, as in Intel's, which applies no `/GS` by default.
- Intel's `/Q` options, which clang-cl would read as paths: `/QxCODE` as
  `-x`, `/Qax`, `/Qipo`, `/Qopt-*`, `/Qdiag-*`, `/Qimf-*` and the like
  dropped, `/Qopenmp` refused, `/Qopenmp-simd` with `__INTEL_HOST_OPENMP`,
  and `/Qmkl` and its kin refused. Any other `/Q` name is a path, as in
  `icx-cl` on Linux (`no such file or directory: '/Qbogus'`).
- The options CMake's IntelLLVM rules give `icx-cl`:
  `-Rno-debug-disables-optimization`, dependency files with `-QMD -QMT
  TARGET -QMF FILE`, linker options as `/Qoption,link,OPTION`, and static
  libraries made through the compiler, `-fuse-ld=llvm-lib -o NAME.lib
  OBJECTS`, which bcc archives with the bundled `llvm-ar` in COFF format,
  as Intel's driver runs `llvm-lib`. `-o` names the object under `/c`
  (the last of it and `/Fo` wins), and otherwise the executable or library.

Over the 34 `icx-cl` command lines, each `-dM` listing is the same set as
Intel's apart from the processor macros below and `__SIZEOF_FLOAT128__`,
which icx-cl defines and bcc does not, having no `__float128` on the MSVC
target; the rest differ as the clang-cl personality differs from clang-cl
(`/arch:AVX2`
refused, `/E`'s `#line` markers) or as bcc refuses OpenMP and the strict
model, and Intel's diagnostics name the source by its absolute path
(`/home/me/a.c(1,22): error: ...`) where bcc keeps the name given.

**What differs from icx:**

- **Intel's code.** icx optimizes with Intel's loop optimizer and
  vectorizer, calls its SVML vector math library and links its `libimf`,
  `libsvml`, `libirng` and `libirc` (`-Bstatic` in executables, shared
  in shared libraries, which then need Intel's runtime on the library
  path); bcc generates code through its own back ends and links only the C
  library, `libm` and the bundled clang's runtime, so CMake's implicit link
  libraries and libtool's search path differ, and so may the last bits of
  results from math functions.
- **Subnormal options.** `-fdenormal-fp-math=` other than `ieee` and
  `-mdaz-ftz` are refused, as under the Clang personality, where icx
  accepts them. Where a link takes `crtfastmath.o`, the bundled clang's
  brings DWARF that `file` reports (`with debug_info`).
- **Processor macros.** `-x` codes and `-xHost` give bcc's x86-64 levels,
  so the processor's name macros (`__corei7__`, `__tune_corei7__`), the
  features above the level bcc has no option for (`__FSGSBASE__`,
  `__RDRND__`, `__XSAVEOPT__`, `__INVPCID__`, `__LAHF_SAHF__`) and the
  AVX-512 family are missing, and `__k8` stays, as under the Clang
  personality's `-march=x86-64-v3`.
- **Refused options.** OpenMP (`-qopenmp`, `-fopenmp`), SYCL, Intel's
  libraries and the strict model are errors in bcc where icx compiles.
- **No `-qnextgen-diag` listing**; the option is accepted and prints
  nothing.
- **Clang's own differences.** Diagnostics, `-E`'s markers, `-v`'s GCC
  installation lines and the `-dM` listing differ from icx exactly as the
  Clang personality's differ from Clang ([Deviations](#deviations)): on
  the 10 diagnostics probes bcc's output under `--icx-mode` is the same as
  under `--clang-mode`, and under `--icx-cl-mode` the same as under
  `--clang-cl-mode` after Intel's logo. Intel's Clang 23.1.1 reports some
  floating-point overrides differently from the pinned Clang 23.1.5
  (`-Ofast` after an explicit model); bcc reports the pinned Clang's, plus
  the default model's complex range above.

## Debug information

`-g` writes debug information through the LLVM back end; `middle-end.md` has the
design. Every personality reads the options in order, as measured against GCC
14.2 and Clang 23:

| Option | GCC personality | Clang personality |
| --- | --- | --- |
| `-g`, `-ggdb`, `-gdwarf` | level 2, or the level already asked for | the same |
| `-g0` | off, whatever came before | the same |
| `-g1`, `-g2`, `-g3`, `-ggdb1`-`-ggdb3` | that level | the same |
| `-g4` and above | `debug output level '4' is too high` | `unknown argument: '-g4'` |
| `-gdwarf-N` | DWARF N, 2 through 5, and on as `-g` is; others `dwarf version N is not supported` | the same with 2 through 6; others `unknown argument` |
| `-gline-tables-only`, `-gmlt` | ignored; GCC refuses them, and bcc keeps ignoring an unknown `-g` option | level 1 |
| `-gtoggle` | flips the result, last | ignored; Clang refuses it |
| `-gcolumn-info`, `-gno-column-info` | columns on or off | the same |
| `-gsplit-dwarf`, `-gz`, `-gstrict-dwarf`, `-gdwarf32`, `-gcodeview` and others | ignored: alone they do not ask for debug information, in either compiler | the same; `-gdwarf32` and `-gdwarf64` turn Clang's on, and bcc does not |

`cl` and clang-cl's `/Zi`, `/Z7` and `/ZI` mean `-g` with CodeView in the object.
`tcc -g` means `-g`; its `-gstabs` is ignored. The native and WebAssembly back
ends warn that they write nothing, and `--interpret` says nothing.

Levels 2 and 3 write the same, without macros. Level 1 is line tables, without
variables or types, as Clang's `-gline-tables-only`. The tables' other differences from GCC and
Clang are in `middle-end.md`, "Debug information".

Every personality keeps named locals in memory when no optimizer runs, whether or
not `-g` is given, because GCC and Clang do and `-g` must not change the code.

## Diagnostic pragmas

Each personality reads the diagnostic pragmas its compiler does:
`#pragma GCC diagnostic` for GCC, both spellings and `fatal` for Clang and
clang-cl, `#pragma warning` for `cl` and clang-cl, none for TCC.
`language-standards.md`, "Diagnostic pragmas", has the rules and the differences
left from each compiler. `-Wfatal-errors` stops the GCC personality at its first
error, with GCC's `compilation terminated.`; Clang's accepts it and goes on.

## Build systems

What CMake 3.31 (Ninja, with response files forced, and Unix Makefiles),
Autoconf 2.71 with Automake 1.16 and libtool 2.4.7, and Meson 1.12 conclude
about each personality was compared with GCC 13.3, the pinned Clang 23.1.5
and TCC 0.9.27 (Ubuntu's, and the release tag) on x86-64 Linux on
2026-11-18. `scripts/build_system_probes.py` repeats the comparison for any
compilers, and `tests/integration/build_system_probes.rs` pins the probes
that decided it. A personality chosen by flag (`CC="bcc --gcc-mode"`) and
by executable name (`gcc`, `cc`, `clang`, `tcc`, `cl`, `clang-cl`) lead to
the same conclusions, except for clang-cl below. zlib 1.3.1 with CMake and
xz 5.6.3 with Autoconf and libtool build and pass their tests under
`--gcc-mode` and `--clang-mode`.

- **GCC.** CMake finds `GNU 14.2.0`, GCC's default C17 with extensions,
  the same compile features (to `c_std_23`), implicit link libraries
  (`gcc;gcc_s;c;gcc;gcc_s`), library architecture, pointer size, PIC and
  dependency-file flags, and the same results for its checks
  (`check_c_compiler_flag(-Wno-bogus)` passes, `-fbogus` fails). Autoconf
  finds a GNU compiler that needs no option for C11, `gcc3` dependency
  tracking, `sqrt` in `-lm`, and libtool GCC's PIC flags, `-c -o`, static
  linking, shared libraries and `-fno-rtti` warning. Meson finds `gcc
  14.2.0`.
- **Clang.** The same as with the pinned Clang: `Clang 23.1.5`, the same
  features, libraries, checks (`-Wno-bogus` fails, as Clang warns) and
  libtool search path; Meson finds `clang 23.1.5`.
- **TCC.** CMake finds `TinyCC` with no compile features, so
  `target_compile_features` fails as with TCC; Meson knows no TCC, as with
  TCC; Autoconf finds a compiler that is not GNU, `tcc` dependency tracking
  and `sqrt` in `-lm`.
- **`cl`.** Off Windows, CMake with a Windows toolchain file and static
  libraries only (no `link.exe`) finds `MSVC 19.33.0.0`, MSVC-like, C17
  without extensions, pointer size 8, `/showIncludes` dependencies, and
  `check_c_compiler_flag(/bogus)` failing on D9002, as `cl.exe` makes it
  fail. Meson recognizes MSVC 19.33.0 for x64 from `cl /?` and then looks
  for `link.exe`, which Linux does not have.
- **clang-cl.** CMake finds `Clang 23.1.5 with MSVC-like command-line`
  simulating MSVC 19.33, C17 with extensions, as with the pinned Clang's
  clang-cl.
- **zig cc.** Compared with zig 0.17.1 on 2026-11-19 (CMake 3.31.3,
  Autoconf 2.71 with libtool 2.4.7, Meson 1.12.1), with `CC="bcc
  --zig-cc-mode"`, `CC="zig cc"` and `CC=zig-cc` for an executable of each
  name, which all conclude the same: CMake finds `Clang 22.1.8` with a GNU
  front end, C17 with extensions, the same compile features (to `c_std_23`),
  pointer size, PIC and dependency-file flags and check results; Autoconf
  and libtool reach every value zig cc does; Meson finds `clang 22.1.8` and,
  from `-Wl,--version`, zig's linker `ld.zigcc`, and builds and tests the
  probe project. CMake's linker ID (`LLD`, none for zig), its implicit link
  libraries and directories (the bundled clang's, where zig's are its own
  `libubsan_rt.a` and `libcompiler_rt.a`) and linker dependency files
  differ, since zig links in process. zlib 1.3.1 with CMake, in its default
  build type (zig's Debug, with UBSan) and in Release, and xz 5.6.3 with
  Autoconf and libtool build and pass their tests with `CC="zig cc"` naming
  bcc.
- **icx.** Compared with icx 2026.2.0 on 2026-11-19 (CMake 3.31.3,
  Autoconf 2.71 with libtool 2.4.7, Meson 1.12.1), with `CC` naming an
  executable `icx` that is bcc: CMake finds `IntelLLVM 2026.2.0` with a GNU
  front end simulating GNU 4.2.1, C17 with extensions, the same compile
  features (to `c_std_23`), pointer size, PIC and dependency-file flags and
  check results; Autoconf and libtool reach every value icx does, `sqrt`
  without `-lm` included (`none required`, as icx links `libm` itself);
  Meson finds `intel-llvm 2026.2.0` and builds and tests the probe project.
  CMake's linker ID (`LLD`, `GNU` for icx), its implicit link libraries and
  directories (icx adds `svml`, `irng`, `imf`, `irc` and `irc_s` and
  Intel's library directories) and libtool's `LD`,
  `sys_lib_search_path_spec` and `archive_cmds_need_lc` differ, as for the
  Clang personality. Off Windows, CMake with a Windows toolchain file finds
  bcc's `icx-cl` to be `IntelLLVM 2026.2.0 with MSVC-like command-line`
  simulating MSVC 19.33, as it finds Intel's, and configures and builds
  with CMake's icx-cl rules (`-QMD -QMT -QMF` dependencies, static
  libraries through `-fuse-ld=llvm-lib`); its `<stdatomic.h>` and
  `-fstack-protector-strong` checks pass where Intel's `icx-cl` fails them
  without Microsoft's headers and as clang-cl warns. zlib 1.3.1 with CMake
  (its default build type and Release) and xz 5.6.3 with Autoconf and
  libtool build and pass their tests with bcc as `icx`, as with Intel's;
  xz's `config.h` is the same byte for byte.

Before these were measured, Meson stopped on `-Wl,--version` (`no input
files`) under GCC and Clang; CMake found no implicit link libraries,
directories, library architecture or linker; Autoconf found `sqrt` without
`-lm`; libtool kept `-fno-rtti` for GCC and searched `/usr/local/lib`;
`check_c_compiler_flag(-Wno-bogus)` passed under Clang; CMake read `cl`'s
probe `/opt/cmake/.../CMakeCCompilerABI.c` as `/o` and found no pointer
size, and passed `/bogus`; clang-cl's `__STRICT_ANSI__` turned CMake's
extensions default off; and Meson found no `cl`.

What still differs, and why:

- **Version.** The GCC personality claims GCC 14.2 (the comparison GCC was
  13.3), so the version, `__GNUC__` and CMake's `gcc-ar-14` search (which
  settles on `gcc-ar`) follow 14.
- **The linker is LLD.** bcc links through the bundled clang with LLD, so
  CMake's `CMAKE_C_COMPILER_LINKER_ID` is `LLD` (GNU with GCC and with the
  pinned Clang, which runs the system's `ld`), Meson's linker `ld.lld`
  (`ld.bfd`), and libtool's `LD` the bundled `ld.lld`, which
  `-print-prog-name=ld` names (GCC prints `ld`). Each passes GNU-style
  linker options, which LLD reads, and CMake still uses linker dependency
  files. The implicit link directories and libtool's
  `sys_lib_search_path_spec` are the bundled clang's.
- **Embedded headers.** The implicit include directories lack the
  compilers' own include directory, since bcc's resource headers are no
  directory on disk; CMake only leaves such directories out of `-I`.
- **TCC's newer features.** `-std=gnu11` selects C11, as TCC's `mob` does
  (0.9.27 keeps `__STDC_VERSION__` at 199901), so Autoconf adds
  `-std=gnu11` to `CC` where 0.9.27 stays in C99; `<stdatomic.h>` exists,
  so CMake's check for it passes; and LLD links `-static` and writes
  `RUNPATH`, so libtool uses `-static` and takes `RUNPATH` to override
  `LD_LIBRARY_PATH`, where TCC's own linker fails a static link and writes
  `RPATH`.
- **`--clang-cl-mode` by flag.** CMake tells clang-cl from Clang by running
  `CC -?` with the compiler's first word alone, so `CC="bcc
  --clang-cl-mode"` reads as a GNU-like Clang and fails CMake's first
  compile. Name an executable `clang-cl`, as CMake also needs for the real
  Clang (`clang --driver-mode=cl` fails the same way).
- **Other C++ options.** GCC's C compiler warns about every option of
  another language; bcc warns only about `-frtti` and `-fno-rtti`, which
  libtool tests, and ignores the others silently.

## Identity policy

`--compiler-identity=gcc|clang|msvc|clang-cl|tcc`, which each personality
sets, decides two kinds of thing by different rules.

**What an identity claims follows its own compiler.** The predefined
macros, the version, and which types and keywords exist are those of the
compiler named: `_Float16` and `__bf16` are types for GCC, Clang and
clang-cl but not for `cl` or TCC, `__fp16` for the Clang identities alone,
the `_FloatN` keywords for GCC alone, and TCC's `#!` script line for TCC
alone. Code matches on the identity for these. Language features follow
`-std`, not the identity: under MSVC, which lacks `_BitInt`, the type is
still a C23 feature that is reported as an extension before C23.

**Behaviour bcc models only for GCC and Clang follows one of them.** Where
the two differ in a rule bcc implements both ways (a constant-folding
rule, a warning's conditions, the evaluation format of `_Float16`, whether
an extension is reported under `-pedantic`), each identity takes one
family of rules:

| Identity | Rules | Why |
| --- | --- | --- |
| `gcc` | GCC's | |
| `tcc` | GCC's | TCC implements a subset of GCC's GNU dialect |
| `clang` | Clang's | |
| `clang-cl` | Clang's | it is Clang |
| `msvc` | GCC's, or Clang's where `cl` is known to behave as Clang's MS-compatible mode models it | `cl` lacks most of the GNU and C23 extensions Clang accepts silently, so GCC's rules, which reject or report more, are the closer model; where `cl`'s behaviour is documented or surveyed and matches clang-cl's, Clang's are exact |

`CompilerIdentity::rules` (`src/configuration/identity.rs`) gives the
`RuleSet`, and `CompilerIdentity::ms_compatible_rules` the one for a rule
where `cl` is known to match Clang; each rule's `CompilerConfiguration`
helper calls one of them rather than matching on identities:

| Rule (helper) | GCC's rules | Clang's rules |
| --- | --- | --- |
| Compound literals in static initializers fold and go unreported (`folds_compound_literals`, #170) | GCC, TCC, MSVC | Clang, clang-cl |
| `_Float16` evaluates in `float` (`half_in_float`, #163) | GCC (TCC and MSVC have no `_Float16`) | Clang, clang-cl |
| `_BitInt` widths, array compound literals, out-of-range floating conversions (`bit_int_rules`, `array_literal_rules`, `float_conversion_rules`, #174) | GCC, TCC, MSVC | Clang, clang-cl |
| Pointer/integer cast warnings (`pointer_cast_rules`, #174; `cl`'s C4311 and C4312 warn only for a narrower integer; TCC has its own rule, [TCC's warnings](#tccs-warnings)) | GCC, TCC | Clang, clang-cl, MSVC (`ms_compatible_rules`) |
| `-MG` lists a missing `<...>` header under `-MM`; `-H` and `/showIncludes` name a header reached from the working directory `./a.h` (the driver) | GCC, TCC, MSVC | Clang, clang-cl |

A rule that compares with `CompilerIdentity::Gcc` alone is equivalent where
the feature it governs exists only under GCC's identity among those that
follow GCC's rules (the `_Float16` pedantic reports in the lexer and
parser). TCC was surveyed for these rules later (the TCCs of the
[TCC](#tcc) section): its pointer/integer cast warnings fall on the casts
GCC 15 warns about (`cast between pointer and integer of different size`
for `(void *)int`, `(void *)short`, `(int)p` and `(char)p`, none for
`(void *)5` or `(unsigned long long)p`) but follow TCC's own rule in the
rest, which the personality applies ([TCC's warnings](#tccs-warnings)),
and it folds compound literals in
static initializers as GCC does; it has no `_Float16` or `_BitInt` and
rejects array compound literals and out-of-range floating conversions in
static initializers (`initializer element is not constant`), so GCC's
rules stay the closer model. `cl`'s behaviour in these rules was not
surveyed: Microsoft documents too little of it, and the Windows
comparison ([Checked against `cl.exe` on
Windows](#checked-against-clexe-on-windows)) covers the driver, the
runtime and the ABI, not these rules.

## The assembler

GCC's driver hands every `.s` and `.S` input to the system assembler, `as`
(GCC manual, "Overall Options"), with the `-Wa,` and `-Xassembler` options
(GCC manual, "Assembler Options"); it finds `as` through the `-B` prefixes,
then `COMPILER_PATH`, its own installation directories and `PATH` (GCC
manual, "Directory Options"; `gcc -print-prog-name=as`). Clang assembles
with its integrated assembler unless `-fno-integrated-as` is given; that
assembler refuses the GNU options it lacks (`clang: error: unsupported
argument '-v' to option '-Wa,'`), answers `-Wa,--version` with Clang's own
banner, and refuses GNU-only syntax such as Solaris section flags. Build
systems probe for the difference. OpenSSL's perlasm (`x86_64-xlate.pl`) runs
`$CC -Wa,-v -c -o /dev/null -x assembler /dev/null` and looks for `GNU
assembler version`, then for `clang` in `$CC --version`; when neither
answers it writes `.section .note.gnu.property, #alloc`, which Clang's
assembler rejects (`expected string`), and the AVX code paths of its
`*-x86_64.pl` scripts, which run the same probe, stay off.

bcc's driver assembles an assembly input with the bundled clang, and
`--assembler=integrated|system|auto` chooses the assembler that clang uses:

| Mode | Assembler | Without an `as` |
| --- | --- | --- |
| `--gcc-mode` (`gcc`, `cc`, `c89`, `c99`) | `auto`: the system's `as`, run by clang under `-fno-integrated-as` with the assembler options | the integrated assembler, after a note (`gcc: note: no system assembler ...`), once per command |
| `--clang-mode`, `--zig-cc-mode`, bcc's own command line | `integrated`, or `system` under `-fno-integrated-as` (the last of `-f[no-]integrated-as` wins) | `system`: an error (`cannot assemble ...: no system assembler ... in the -B prefixes, COMPILER_PATH or PATH`) |
| `--tcc-mode` | `integrated`; TCC ignores `-Wa,` options, warning only under `-Wunsupported`, and bcc does the same | — |
| `--msvc-mode`, `--clang-cl-mode` | `integrated` | — |

bcc looks for `as` as each `-B` prefix followed by `as` (a prefix that is a
directory gets its separator), then in each `COMPILER_PATH` directory, then
on `PATH`, and hands clang the prefix that found it (`-B/usr/bin/`), so `-v`
prints the clang command with `-fno-integrated-as` and that prefix. It looks
only on an x86-64 Linux host compiling for Linux (glibc or musl), whose `as`
writes those objects; elsewhere, Windows among them, `auto` is the integrated
assembler with the note. The WebAssembly site runs no driver.

The probe therefore answers as each compiler does on x86-64 Linux:

| Command | GCC 13.3 | Clang 23.1.1 | TCC 0.9.28rc | bcc |
| --- | --- | --- | --- | --- |
| `-Wa,-v -c -o /dev/null -x assembler /dev/null` | `GNU assembler version 2.42 ...`, status 0 | `unsupported argument '-v' to option '-Wa,'`, status 1 | nothing, status 0 | each mode as its compiler (`--gcc-mode` through the host's `as`) |
| `-Wa,--version ...` | `as --version` | Clang's banner | nothing | each mode as its compiler |
| `--version` | `gcc ...` | `clang version ...` | `tcc version ...` | each mode names its compiler, so perlasm finds `clang` only under `--clang-mode` |

So OpenSSL's perlasm writes GNU syntax for `--gcc-mode`, which GNU `as`
assembles, and for `--clang-mode`, whose integrated assembler takes it. Under
`--tcc-mode` it writes the Solaris flags, which the integrated assembler
refuses, as TCC's own assembler does (`string constant expected`).

**Decision.** The system assembler, not a translation of its options for the
integrated one. Answering `-Wa,-v` with a GNU banner from the integrated
assembler would claim an assembler that did not run, and accepting every
GNU-only option and directive would mean reimplementing GNU `as` in front of
LLVM's; running `as` gives GCC's behaviour exactly, including the options bcc
does not know about (`-adhln=FILE`, `--generate-missing-build-notes`,
`-mrelax-relocations=no`) and the errors GNU `as` gives for those it does not
know. Assembling through the bundled clang keeps the `.S` preprocessing and
`-g` mapping in one place.

bcc's own objects keep coming from its back ends, which write them without an
assembler. Assembling them with `as` costs little: bcc's `-S` output for
OpenSSL's `x509_vfy.c` (8,229 lines) and `s3_lib.c` assembles in about 5 ms
with GNU as 2.42 and 10 ms with the integrated assembler, against 0.9 s for
`--gcc-mode -O2 -c`. But that text is LLVM 23's dialect: it aligns every
function with `.prefalign`, which GNU as 2.42 rejects (`unknown pseudo-op`),
so the round trip would break every compile. A `-Wa,` option given with a C
source therefore still draws `assembler option '...' is not applied to C
sources, which bcc assembles itself` (except `--noexecstack`, which bcc's
objects already satisfy).

## Decisions

- **Run the assembler GCC runs.** Under `--gcc-mode` assembly inputs go to the
  system's `as`, so the assembler options, probes and directives behave as
  with GCC, and bcc never claims an assembler that did not run (see [The
  assembler](#the-assembler)).
- **Translate, don't reimplement.** A personality produces bcc command lines,
  so the semantics of each option live in one place, and the personalities
  stay small tables of spellings. The few driver features they need (several
  inputs, `-c` naming, `-E`, `-M`) became bcc features.
- **Ignore only what cannot change the program.** Real build flags contain
  dozens of warning and tuning options, and refusing them would make every
  real build fail, so those are ignored. But an option that changes layout,
  the ABI, the instruction set or a language rule is refused: ignoring it
  would build a different program than the one asked for, without saying
  so. Hardening warns. The lists are frozen from the compilers themselves,
  so "known" means what the compiler accepts, and unknown spellings still
  fail as they would there. TCC is the exception that proves the rule: it
  ignores every `-W`, `-f` and `-m` option it does not know, so ignoring
  one builds the program TCC builds, and only the options TCC implements
  are refused when bcc lacks them.
- **Claim the compiler mimicked.** The GCC and TCC personalities claim GCC and
  TCC in the predefined macros rather than Clang, because build systems and
  headers choose their paths from them (CMake reads `__GNUC__` and
  `__clang__`; glibc and many projects test `__GNUC_PREREQ`). The identity
  therefore brings the GCC features those paths use: the `_FloatN` types
  and `__builtin_va_arg_pack`. Use `--clang-mode`, or `-Xbcc
  --compiler-identity=clang`, to keep Clang's paths.
- **Name bcc.** Every banner names bcc and says it is not the compiler it
  mimics; `__bcc__` stays defined in every identity.
- **`cl`'s codes only where its words are modeled.** Under the `cl`
  identity a diagnostic carries an MSVC code only where bcc writes `cl`'s
  own message for it, for the diagnostics `cl`'s users meet most; the
  others keep bcc's words and no code rather than an invented one. See
  Diagnostics under MSVC.
- **The host's C library for the host only.** Compiling for the host on an
  x86-64 Linux host, bcc's own driver searches the host's
  `/usr/local/include`, multiarch directory and `/usr/include` after its
  resource headers, as Clang does, so every mode finds `<stdio.h>` with no
  option and the personalities add no directories of their own. A cross
  target (Windows, or musl on a glibc host) stays hermetic until
  `--sysroot`, `-isystem` or `-idirafter` names its library; a personality
  passes `--sysroot` on, also with `-nostdinc`, so the link uses it.

## Deviations

Besides those above:

- `-E` writes `#pragma pack` and `#pragma weak` back; other pragmas, which
  have no effect in bcc, are dropped. Comments are never kept (`-C` warns).
  Adjacent string literals are joined, as phase 6 joins them. A macro's
  expansion keeps the spacing of its definition, so the text differs from
  GCC's in whitespace.
- `-E` writes a line marker for every file entered and returned to, with
  flags 1 and 2, and with no token needed between them, as GCC and Clang do.
  A system header adds 3 and, under the GCC identity, 4. The markers still
  differ in these ways: GCC's and Clang's first lines for `<built-in>` and the
  command line are not written, the pseudo-file is `<command line>` under
  every identity where GCC says `<command-line>`, a header is named as
  bcc's search found it (`a.h` where Clang says `./a.h`), a resource header
  is `<built-in>/limits.h`, and an `#include` late in a file writes no blank
  lines before its marker. `cl`'s text has no flags 1 and 2.
- Of a GNU line marker's flags, only 3 has an effect: it makes the lines
  up to the next marker a system header, as in GCC, so a `.i` file's system
  header sections keep their warnings withheld. Flags 1 and 2 (entering and
  leaving an include) do not rebuild the include stack.
- The `-dM` listing holds the macros bcc predefines, not the ones GCC or the
  real Clang would; the target tables are Clang's. The GCC identity's
  floating-point macros match GCC 15's for every type bcc has, `_Float16`'s
  `__FLT16_*__` and `__bf16`'s `__BFLT16_*__` included, but the standard
  types' constants keep Clang's spelling (`__FLT_NORM_MAX__
  3.40282347e+38F` for GCC's 36 digits). GCC's `__STDC_IEC_559__` family
  comes from glibc's `stdc-predef.h`, which GCC includes before every file
  and bcc, like the pinned Clang, does not; so `-M`, `-H` and `-v` do not
  list it either.
- `<float.h>` defines every name GCC 15's and the pinned Clang's define in
  each mode, C23's `FLT_NORM_MAX`, `FLT_SNAN`, `INFINITY` and `NAN` and
  GCC's `FLT_IS_IEC_60559` and, under the IEC 60559 extension macros,
  `CR_DECIMAL_DIG` among them, except GCC's decimal floating
  `DEC32_*`, `DEC64_*` and `DEC128_*` names in C23 (bcc has no decimal
  types) and `FLT16_SNAN`, whose `__builtin_nansf16` bcc lacks.
- `<tgmath.h>` gives a `_FloatN` argument the function of the standard type
  with its format, so `sqrt` of a `_Float32` is `sqrtf`'s value with type
  `float`, where GCC's `__builtin_tgmath` calls `sqrtf32` and keeps the type
  `_Float32`. `_Float128` arguments are refused, as `__float128` ones are,
  and so is a `_FloatN` first argument of `nexttoward`, as in GCC.
- GCC warns that `-fassociative-math` is disabled without
  `-fno-signed-zeros` and `-fno-trapping-math`; bcc drops it the same way
  but silently.
- `cl /J` defines `__CHAR_UNSIGNED__` as well as `_CHAR_UNSIGNED`, as
  clang-cl does; `cl` defines only the latter. The MSVC identity keeps
  Clang's target macros, as for `__x86_64__`.
- The MSVC personality links through Clang's driver and LLD (`lld-link`),
  which needs the Windows SDK and MSVC libraries; on Windows Clang's driver
  finds them itself, and on a Linux host without them only `/c`, `/E`,
  `/P` and `/Fa` work. LLD writes an import library for every DLL, where
  `link.exe` writes one (and an `.exp` file) only for a DLL that exports
  something, so `cl /LD b.c` of a DLL without exports leaves `b.dll` and
  `b.obj` and bcc also `b.lib`. `/Zi` writes no `vc140.pdb`: the CodeView
  stays in the object, as with `/Z7`, as clang-cl does.
- Against `cl` 19.44 the predefined macros differ in these, as the MSVC
  identity decides above: `_MSC_VER` and `_MSC_FULL_VER` (1944 and
  194435230; bcc claims 19.33); `_MSVC_TRADITIONAL`, which `cl` defines as
  1 unless `/Zc:preprocessor` or a `/std:` option selects its conforming
  preprocessor; `__STDC_VERSION__`, which `cl` leaves undefined without
  `/std:` and defines as `202312L` for `/std:clatest`, where bcc's C23 is
  `202311L`; Clang's target macros, `__x86_64__`, `__SSE2__` and
  `__SIZEOF_POINTER__` among them, and `__bcc__`; and `_M_FP_FAST`, which
  `cl /fp:fast` defines and bcc, keeping IEC 60559 semantics, does not.
- Only the diagnostics in the table under Diagnostics take `cl`'s code and
  words; the others are bcc's own, with their C99 notes indented below
  them, where `cl` writes its own (`error C2039: 'b': is not a member of
  's'`, `error C2197`, `C2198`, `C2106`, `C2100`, `C2084`, `C2124`,
  `C2063`, `fatal error C1075`) and `note:` lines of its own. bcc has no
  warning levels of its own, so `/W2` and `/W3` change only the coded
  warnings, and `/W4` and `/Wall` turn on bcc's `-Wall -Wextra`. bcc
  reports none of `cl`'s unreferenced local or parameter warnings (C4101,
  C4100, C4189), conversion warnings (C4244, C4018), C4706, format checks
  (C4477, C4313), C4293, C4142, or its code generator's C4715 and C4716, so
  `cl /W4 /WX` fails (error C2220) on units bcc compiles. bcc warns, where
  `cl` does not, about a constant that changes value in a conversion
  (`-Wconstant-conversion`), and does not diagnose a constant division by
  zero (`cl`'s error C2124). `/wo` (report a warning once) has no effect.
- bcc implements no `deprecated` attribute, `__declspec(deprecated)`
  included, so it never writes `cl`'s C4996, which the CRT headers raise
  through it for `strcpy` and its relatives at `/W3`.
- A missing `;` at the end of a file, `cl`'s fatal error C1004, keeps
  bcc's words, and so does one before a keyword other than a statement or
  basic type keyword.
- `-v` and `-###` print the commands bcc runs, not GCC's or Clang's: each
  step is a bcc command line that bcc runs in process, named by the path
  of the running bcc, then, under `-v`, the bundled clang's assembly and
  link commands. There is no `cc1`, `as` or `collect2` line and no
  `COLLECT_GCC_OPTIONS` (an assembly input's `as` runs under the clang
  command, which shows `-fno-integrated-as`), and GCC's `-v` also passes
  `-v` to `as`, which prints its version, where bcc's does not. Under `-v`
  a link also prints the linker's command, as the bundled clang's `-###`
  lists it (`ld.lld` with its `-L` directories and libraries), where GCC
  prints `collect2`'s; CMake reads its implicit link libraries and linker
  from it. GCC's `-###` quotes the words with characters
  other than letters, digits, `_`, `/`, `-` and `.`, and its `-v` none, as
  GCC does. The header search lists every configured directory, without the
  embedded resource headers, which are no directory on disk.
- clang-cl's `/o` (a deprecated `/Fe` or `/Fo`) and the
  `/winsysroot` family are not implemented; they draw the unknown-option
  warning, so `/opt/a.c`, which clang-cl reads as `/o`, is ignored with it
  rather than compiled. Use `-- /opt/a.c`.
- clang-cl searches `%INCLUDE%`, its `/imsvc` and its `/external:env:`
  directories after its resource headers; bcc searches them before its
  built-in headers, whose intrinsic headers do not yet work with the MSVC
  headers that include `<intrin.h>` (see "clang-cl").
- clang-cl's suggestion for a missing input considers its `cl` options,
  not the Clang options it also takes, which start with `-` only.
- `-H` and `/showIncludes` print their lists after the unit is compiled,
  not interleaved with its diagnostics, and `-H` omits GCC's closing
  `Multiple include guards may be useful for:` list. A re-inclusion is left
  out when the header is one `#ifndef` or `#if !defined` group whose macro
  is defined, as GCC's and Clang's multiple-include optimization detects
  it; the embedded resource headers are named `<built-in>/stddef.h`.
  `/sourceDependencies` is not implemented.
- The GCC lists come from GCC 15.2, which knows a few options GCC 14 did not.
- `/Za` turns off only what bcc implements of Microsoft's extensions to C:
  the anonymous tagged members and `_MSC_EXTENSIONS`. The `__`-prefixed
  keywords stay, and `cl`'s other `/Za` changes concern extensions bcc never
  accepted.
- The MSVC runtime libraries are chosen at link time with
  `-nodefaultlib:libcmt` and `-defaultlib:`; bcc's objects carry no
  `/DEFAULTLIB` directives as `cl`'s do, so objects compiled with different
  runtime options do not conflict at link time. A build system that links
  bcc's objects with `link.exe` or `lld-link` itself, as CMake does for
  `cl`, must name the runtime (`msvcrt.lib oldnames.lib` for `/MD`), or the
  link finds no `mainCRTStartup`. `#pragma comment(lib, ...)` is not
  implemented either.
- bcc writes the help of `/?` in its own words, not `cl`'s option list,
  and a banner of one line (`cl`'s also has a copyright line and an empty
  one). It warns about the hardening options it ignores (`/GS`), and
  refuses `/fp:strict`, which `cl` accepts.

## Gaps in the driver

These driver behaviours still differ from the compilers mimicked:

- bcc's `-E` skips assembly inputs, so Clang's `-fsyntax-only`, which
  preprocesses a `.S` input to `-o` or stdout, writes nothing for one, and
  neither does GCC's `-fsyntax-only -S`, which prints a `.S` input
  preprocessed.
- The `cl` personality reads `-` as an unknown option (warning D9002);
  Microsoft does not document whether `cl` reads standard input.
- A missing assembly input is reported by the bundled clang (`error: cannot
  assemble ...: clang: error: no such file or directory`) rather than as
  GCC's assembler (`Error: can't open missing.s for reading`) or `cc1` (for
  `.S`) words it; a missing object or library at link time likewise in
  clang's words rather than `ld`'s and `collect2`'s, or `LINK`'s fatal error
  LNK1181 for `cl`.
- A missing TCC input that is not a C source is reported before the C
  sources ahead of it are compiled, so under `-c` their objects are not
  written.
- TCC's `-run` builds the program into a temporary directory with the
  bundled clang's linker and runs it, rather than compiling it in memory.
  A program killed by a signal makes TCC itself die by that signal (`tcc
  -run` of a `raise(SIGSEGV)` is killed by signal 11); bcc exits with the
  signal number plus 128 (139), which is what a shell reports for both.
- TCC's `-E` writes `# 1 "<command line>" 1` and flagged markers for every
  header, and names standard input `-`; bcc writes its GNU markers and
  `<stdin>`. TCC's `-P1` writes `#line` directives, which bcc writes as GNU
  markers. TCC's `-dD` writes the macro definitions with the text; bcc
  writes only the definitions, as `-dM`. With an input that cannot be
  found, TCC's `-E` also writes an empty line to stdout.
- `tcc -vv FILE` lists every file TCC opens, start files and headers
  included, nested by indentation; bcc prints `-> FILE` and `<- OUTPUT`
  as for `-v`. `-print-search-dirs` lists bcc's directories, not TCC's,
  and has no `libtcc1:` entry.
- TCC links with its own linker: `-Wl,` options it does not know
  (`tcc: error: unsupported linker option '-zfoo'`), a missing library
  (`tcc: error: library 'nope' not found`) and an undefined symbol
  (`tcc: error: unresolved reference to 'f'`; 0.9.27: `undefined symbol
  'f'`) are its errors, where bcc passes the options to LLD and reports
  its errors in its own words (`tcc: error: cannot build `a.out`: clang
  failed` with LLD's lines). `-static` links where both
  TCCs fail against this host's static glibc (`unresolved reference to
  '_Unwind_GetCFA'`; 0.9.27: `Unknown relocation type for got: 23`).
- TCC's `-M` lists the headers of its own include directory, where bcc's
  are embedded. (A link's one rule, `-` for standard input and TCC's
  escaping follow TCC since #251.)
- TCC's `-c` given an object rewrites that object (`tcc -c a.c b.o`
  writes `a.o` and relinks `b.o` into itself); bcc warns that the
  object is unused, as GCC does.
- `-ar` reads its arguments and reports its errors as TCC's archiver does,
  then runs the bundled `llvm-ar`, whose archive keeps member names longer
  than 15 characters in a GNU long-name table; TCC truncates them, and its
  `-ar t` lists such members of bcc's archive as `/` and `/0`.
- `-B` is ignored: bcc's headers are embedded, and it has no `libtcc1.a`.

## Gaps in the compiler these modes expose

These are compiler features, outside the driver, that the personalities
would use:

- `-fsemantic-interposition`, GCC's default with `-fPIC`: bcc inlines and
  calls a default-visibility definition of a shared library as if it could
  not be interposed, as Clang does by default and GCC under
  `-fno-semantic-interposition`, so the option is refused.
- Decimal floating types, and with them GCC's C23 `<float.h>` `DEC*`
  names; `FLT16_SNAN` and GCC's other `__builtin_*f16` constant builtins.
- TCC lists `__BASE_FILE__` in `-dM`; bcc defines it in every identity
  but, as GCC and Clang, does not list it. In a `-include` header Clang
  expands it to `"<built-in>"`; bcc names the main file there, as GCC and
  TCC do.
- TCC's warning for a function that may return no value (`function might
  return no value: 'f'`) has no bcc counterpart.
- A C99 `inline` definition with no external one: TCC emits it as a local
  function, so a program calling it links; bcc follows GCC and Clang and
  leaves the call to an external definition, so the link fails (`ld.lld:
  error: undefined symbol: f`). C99 §6.7.4p6 allows either.
- `__vectorcall` and `cl /Gv` are refused, so bcc cannot call or define a
  function with that convention.

Since this list was written, bcc gained binary128 arithmetic on every back
end (#160), `_Float16` (#163), `__bf16`, complex `_Float16` and `__fp16`
(#164), the `_FloatN` `<float.h>` names, constant compound literals in
static initializers (#170), and compound assignment to atomic floating
objects (#171); the personalities use each through the identity.
