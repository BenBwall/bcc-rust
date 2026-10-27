# Driver modes: GCC, Clang, MSVC, clang-cl and TCC personalities

bcc can stand in for another C compiler on the command line. A **driver
personality** reads the command line with that compiler's conventions,
translates it into bcc's own options, and turns on every compatibility feature
bcc has for that compiler: its predefined identity macros, language mode,
target, system headers, Microsoft extensions, diagnostic format, output
names and version banners. Build systems that probe a compiler (Autoconf,
CMake, Meson, plain Makefiles) should find the compiler they expect, while
every banner and `__bcc__` still say that bcc is the compiler.

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
  named after the stage and output as GCC names them;
- `-fsyntax-only` (`--syntax-only`), and a failing exit status from every
  mode, the inspection views included, after an error;
- GNU line markers (`# 12 "file.c" 1 3`) in input, so `.i` files from
  `-E` compile again with their original locations;
- `--sysroot` for the libraries the bundled clang links against, not only the
  headers;
- `-fwrapv`, `-fpack-struct=N` and `-f[no-]trigraphs`, which the
  personalities map `-fwrapv`, `cl /ZpN` and `cl /Zc:trigraphs` onto;
- `--compiler-identity=clang|gcc|msvc|clang-cl|tcc` and
  `--diagnostic-format=bcc|clang|gcc|msvc|msvc-classic|msvc-caret|tcc`
  (also Clang's `-fdiagnostics-format=`).

## Compatibility features by personality

| | GCC | Clang | `cl.exe` | clang-cl | TCC | bcc |
| --- | --- | --- | --- | --- | --- | --- |
| Default target | host (MinGW-w64 on Windows) | host | `x86_64-pc-windows-msvc` | `x86_64-pc-windows-msvc` | host | `x86_64-unknown-linux-gnu` |
| Default mode | `gnu17` | `gnu17` | `c17` | `c17` | `gnu99` | `gnu17` |
| Microsoft extensions | off | off | all (`-fms-extensions`) | all | off | off |
| `__GNUC__` | 14.2.0 | 4.2.1 | undefined | undefined | undefined | 4.2.1 |
| `__clang__`, `__llvm__` | undefined | 23.1.1 | undefined | 23.1.1 | undefined | 23.1.1 |
| Other identity | `__VERSION__ "14.2.0"`, execution charset names, `__GCC_IEC_559` | | `_MSC_VER 1933`, `_MSC_FULL_VER`, `_MT`, `_MSVC_TRADITIONAL 0` | `_MSC_VER 1933`, `_MT` | `__TINYC__ 927` | |
| `_Float32` ... `_Float128` | keywords, distinct types | identifiers | identifiers | identifiers | identifiers | identifiers (`-ffloatn-types`) |
| Unsigned plain `char` | `-funsigned-char` | `-funsigned-char` | `/J` | `/J`, `-funsigned-char` | `-funsigned-char` | `-funsigned-char` |
| System headers | host's `/usr/local/include`, multiarch, `/usr/include` | same | `%INCLUDE%` (unless `/X`) | same | host's | the host's when compiling for the host; none for a cross target unless asked |
| Diagnostics | `file:line:col: error:` + snippet | same + `N errors generated.` | `file(line,col): error:` | same | `file:line: error:` | annotated snippet |
| Default output | `a.out` (`a.exe` on Windows) | same | `first.exe`, objects kept | same | `a.out` | none (analysis only) |
| `__bcc__` | defined | defined | defined | defined | defined | defined |

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
pinned Clang 23 rejects all five names and the suffixes in C (it has
`_Float16` and `__fp16`, which bcc lacks), and neither `cl` nor TCC has them,
so the other personalities leave them identifiers for glibc's typedefs.
bcc's own `-ffloatn-types` and `-fno-floatn-types`, which the GCC, Clang,
TCC and clang-cl personalities pass on, override either default.

## Options a compiler knows and bcc does not implement

Each personality sorts these into three kinds (`personality/unimplemented.rs`
for GCC, Clang and TCC; `msvc.rs` for `cl` and clang-cl):

- **Ignored**, when bcc's output already means what the option asks for:
  warnings and diagnostics formatting, debug information (`-g*`,
  `-fdebug-prefix-map=`), reports and dumps, build mechanics (`-pipe`,
  `-save-temps`, `-flto`, `-fuse-ld=`), optimizations
  the compiler is permitted but not required to make (`-funroll-loops`,
  `-fomit-frame-pointer`, `-fstrict-aliasing`, `-fno-math-errno`,
  `-fassociative-math`), code generation tuning (`-mtune=`, `-falign-*`,
  `-mtls-dialect=`), what bcc always does (`-fno-common`,
  `-fno-strict-aliasing`: bcc emits no type-based alias information;
  `-ffp-contract=off`: bcc forms no fused operations; `-fexcess-precision=`:
  x86-64 has none; `-mno-avx`: bcc uses only the baseline instruction set;
  `-fmath-errno`: the math functions set `errno`; `-march=x86-64`), the
  target's own ABI choices (`-mms-bitfields` for a Windows target,
  `-mlong-double-80` for Linux; `-m` options are judged for the target,
  after the whole command line is read), options that only change
  which programs are accepted (`-fpermissive`), and options for other
  languages (C++'s `-fno-rtti`).
- **Warned and ignored**, for hardening and instrumentation that a correct
  program does not depend on: `-fstack-protector*`,
  `-fstack-clash-protection`, `-fcf-protection`, `-ftrivial-auto-var-init=`,
  `-fexceptions` (bcc emits no landing pads, so cleanups do not run while an
  exception unwinds), Spectre mitigations, `-mfentry`, `-specs=`, `-B`, and
  `cl`'s `/GS`, `/sdl`, `/RTC*`, `/guard:cf` and `/Qspectre`; and requests
  bcc's output satisfies less fully but correctly: a newer processor
  (`-march=native`, `-march=x86-64-v3`, `-mavx2`; baseline x86-64 code runs
  on it, but its macros such as `__AVX2__` stay undefined), `-fcommon` (two
  units' identical tentative definitions then fail to link, as with
  `-fno-common`), and `-ffunction-sections`/`-fdata-sections` (the linker's
  `--gc-sections` removes less). The build goes on and says what it lacks.
- **Refused**, with the compiler's kind of error, for everything else:
  options that change layout or the ABI away from the target's
  (`-fshort-enums`, `-fshort-wchar`, `-fpack-struct=3`, `-mms-bitfields` on
  Linux, `-mno-ms-bitfields` on Windows, `-mregparm=`, `-mlong-double-64`
  outside MSVC, `cl /Gv`), instruction sets bcc's code needs (`-mno-sse`,
  `-mgeneral-regs-only`; `cl /arch:AVX2`, which also changes macros),
  language rules bcc does not follow (`-fno-delete-null-pointer-checks`,
  `-fno-strict-overflow`, `-ffast-math`, `-frounding-math`, `cl /fp:strict`,
  `/volatile:ms`), and
  instrumentation whose output a build expects (`-fsanitize=`,
  `-fprofile-arcs`, `--coverage`, `-finstrument-functions`, `-fopenmp`).
  An option the lists do not name is refused too, so a newly added option is
  refused until someone audits it.

Where bcc can do what the option asks cheaply, the personality maps it
instead: `-fwrapv` (lowering marks signed arithmetic without `nsw`),
`-fpack-struct=N` and `cl /ZpN` (the packing before any `#pragma pack`),
`-fpack-struct` alone (every structure packed, the pragmas ignored with
GCC's warning), `-fno-builtin` and `-fno-builtin-NAME` (the LLVM functions
carry `no-builtins` or `no-builtin-NAME`, so LLVM neither replaces library
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
Stages: `-E`, `-S`, `-c`, and linking otherwise, into `a.out`. Several inputs
are compiled separately and linked. GCC rejects an unknown option
(`gcc: error: unrecognized command-line option '-ffoo'`) except an unknown
`-Wno-*`, which it ignores. Queries: `--version`, `-v`, `-dumpversion` (the
major version since GCC 7), `-dumpfullversion`, `-dumpmachine`,
`-print-prog-name=`, `-print-search-dirs`, `-print-multiarch`, `-###`.

**Mapping.** bcc's own command line already takes GCC's spelling of the
options it implements, so the personality passes them through: `-c -S -E -o
-x -I -isystem -iquote -idirafter -D -U -include -std= -ansi -O* -w -W*
-pedantic* -l -L -Wl, -Xlinker -shared -static -pie -M* -fpic -fms-extensions
-fgnu89-inline -ffreestanding -ftls-model=`. `-Wp,` is split and read as
preprocessor options (Fedora's `-Wp,-D_FORTIFY_SOURCE=2`). `-s`, `-T`, `-u`,
`-z` and `-e` become linker options. `-Wa,` and `-Xassembler` reach bcc's
assembly inputs. `-g*`, `-pipe`, `-save-temps` and `--param` have no effect;
`-B` and `-specs=` warn that they have none. `-Ofast` is refused: it implies
`-ffast-math`, which defines `__FAST_MATH__` and links `crtfastmath`, which
bcc does not do.

The `-W`, `-f` and `-m` names GCC knows are those GCC 15.2 lists in
`--help=warnings,common,optimizers,c,target,undocumented`, frozen in
`src/cli/personality/options/gcc-*.list` by
`scripts/update_driver_options.py`. Unknown warnings are ignored as GCC
ignores them; the known `-f` and `-m` options bcc does not implement are
treated as the section above describes. Many of them do change what a
program means (`-fwrapv`, `-fshort-enums`, `-fpack-struct`, `-mno-sse`),
so they are not all ignored. `-m32`, `-mx32`, `-m16`, `-traditional` and
`-imacros` are refused as well.

**Identity.** `__GNUC__` 14, `__GNUC_MINOR__` 2, `__GNUC_PATCHLEVEL__` 0,
`__VERSION__ "14.2.0"`, `__GNUC_EXECUTION_CHARSET_NAME`, GCC's
`__GCC_IEC_559` 2, `__GCC_IEC_559_COMPLEX` 2 and `__FLT_IS_IEC_60559__`
family (1, as GCC 13 and later define them), and no Clang macro. Headers then
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

**Command line.** GCC's, plus `--target=`/`-target`, `-isysroot`,
`-Xclang`, `--driver-mode=`, and `-fdiagnostics-format=`. Clang errors on an
unknown option (`clang: error: unknown argument: '-ffoo'`) but only warns
about an unknown warning option (`warning: unknown warning option '-Wfoo'
[-Wunknown-warning-option]`) and goes on.

**Mapping.** As for GCC, with the option names the pinned Clang lists for
`--autocomplete=-W`, `-f` and `-m` (`options/clang-*.list`). An unknown
warning option is passed to bcc, which warns as Clang does. `-Xclang OPTION`
is refused: bcc has no Clang frontend options, and Clang's can change the
program.

**Identity.** bcc's default: the pinned Clang 23.1.1 and the GCC 4.2.1 Clang
claims. `--version` prints `clang version 23.1.1 (bcc 0.1.0, ...)` with the
target, thread model and installed directory, as Clang does.

## MSVC `cl.exe`

**Command line.** Options start with `/` or `-` and are case-sensitive.
Most take their value joined (`/Fefoo.exe`, `/Ifoo`, `/Fo:out.obj`); `/I`,
`/D`, `/U` and `/FI` also take the next argument. `/DNAME#VALUE` defines NAME
as VALUE. `cl` compiles each source to an object named after it (`a.c` to
`a.obj`) and keeps it, then links an executable named after the first source
(`a.exe`) unless `/c` is given. `/Fo` and `/Fe` name a file, or a directory
when they end in a separator. `/link` passes the rest of the command line to
the linker. `cl` prints a banner to stderr unless `/nologo` is given, and the
name of each source it compiles. An unknown option draws `cl : Command line
warning D9002 : ignoring unknown option '/foo'` and is ignored.

**Mapping.**

| `cl` | bcc |
| --- | --- |
| `/c`, `/Fo`, `/Fe`, `/link ...` | `-c -o NAME.obj`, then a link of the objects with `-Wl,...`; `/Fo`, `/Fa` or `/Fi` naming a file for several sources is error D8036, as in `cl`; an output of any step that is an input of the command line (by device and inode) is refused before any step runs |
| `/E`, `/EP`, `/P`, `/Fi` | `-E`, `-E -P`, `-E -o NAME.i` |
| `/FA`, `/Fa` | `-S -o NAME.asm` beside the object |
| `/I`, `/D`, `/U`, `/FI`, `/X` | `-I`, `-D`, `-U`, `-include`; `/X` skips `%INCLUDE%` |
| `/O1`, `/Os` / `/O2`, `/Ox`, `/Ot` / `/Od` | `-Os` / `-O2` / `-O0` |
| `/W0`, `/w` / `/W1`-`/W3` / `/W4`, `/Wall` / `/WX` | `-w` / default / `-Wall -Wextra` / `-Werror` |
| `/std:c11`, `/std:c17`, `/std:clatest` | `-std=c11`, `-std=c17`, `-std=c23` |
| `/TC`, `/Tc FILE` | the inputs (or FILE) are C |
| `/MD`, `/MDd`, `/MT`, `/MTd`, `/LD`, `/LDd` | the last runtime option wins: `_DLL` for `/MD*`, `_DEBUG` for `/M*d` and `/LDd`, and the link takes `msvcrt`, `msvcrtd` or `libcmtd` instead of `libcmt`; `/LD` links a DLL with `/MT`'s runtime unless `/MD` names another |
| `/Zs` | `--syntax-only` for each source, with no object and no link |
| `/Zp`, `/ZpN` | `-fpack-struct=1`, `-fpack-struct=N`; another N is error D8021 |
| `/Za`, `/Ze` | the last wins: `/Za` turns off Microsoft's anonymous tagged members and `_MSC_EXTENSIONS` |
| `/Zc:trigraphs`, `/Zc:trigraphs-` | `-ftrigraphs`, `-fno-trigraphs` (the default, as in `cl`) |
| `/J` | `-funsigned-char`: plain `char` is unsigned and `_CHAR_UNSIGNED` defined |
| `/external:I DIR`, `/external:env:VAR` | `-isystem` |
| `/showIncludes` | `--show-includes`: `Note: including file: PATH` to stderr |
| `/diagnostics:classic`, `column`, `caret` | `file(line):`, `file(line,col):`, and with the snippet |
| `/nologo`, `/Zi`, `/Z7`, `/Zc:inline` and the other `/Zc:` switches for C++, `/utf-8`, `/EH*`, `/wd####`, `/MP`, `/FS`, `/Fd`, `/Gy`, `/GL`, `/Oi`, `/Oy`, `/arch:SSE2`, `/fp:precise`, `/fp:fast`, `/volatile:iso` ... | accepted, no effect |
| `/GS`, `/sdl`, `/RTC*`, `/guard:cf`, `/guard:ehcont`, `/Qspectre*`, `/d1*`, `/d2*` | accepted with a warning |
| `/TP`, `/Tp`, `/std:c++*`, `.cpp` inputs, `/Gv`, `/arch:` other than SSE2, `/fp:strict`, `/fp:except`, `/volatile:ms`, `/Zc:preprocessor-`, non-UTF-8 charsets | refused with `cl : Command line error : ...` |

**Defaults.** The `x86_64-pc-windows-msvc` target with every Microsoft
extension bcc implements (`-fms-extensions`), which on that target also
predefines `_MSC_VER` 1933, `_MSC_FULL_VER` 193300000, `_MSC_BUILD` and
`_MSC_EXTENSIONS`. The identity removes the GNU and Clang macros and adds
`_MT` (the default static runtime) and `_MSVC_TRADITIONAL 0`: bcc's
preprocessor is conforming, like `cl /Zc:preprocessor`. Directories in
`%INCLUDE%` are system header directories.

**Language mode.** `cl` without `/std:` compiles a C dialect close to C17 and
leaves `__STDC_VERSION__` undefined; `/std:c11` and `/std:c17` define it.
bcc compiles C17 by default, as clang-cl does, and defines
`__STDC_VERSION__` as `201710L`. `cl` defines `__STDC__` only under `/Za` or
`/Zc:__STDC__`; bcc always defines it, as ISO C requires.

**Diagnostics.** `file(line,col): error: message`, the form clang-cl and
`cl /diagnostics:column` write and MSBuild parses. `cl`'s own messages carry
a code such as `C2065`; bcc's messages are its own, and no mapping from them
to MSVC's codes is documented, so bcc writes no code rather than an invented
one. The one MSVC code bcc writes is D9002, Microsoft's documented warning
for an unknown command-line option. `cl` writes compiler diagnostics to
stdout; bcc writes them to stderr, as in its other modes.

**Banner.** `bcc 0.1.0, compatible with Microsoft (R) C/C++ Optimizing
Compiler Version 19.33.0 for x64; not a Microsoft product`, to stderr. Tools
read the version and `for x64` from it.

## clang-cl

clang-cl reads `cl`'s options and also Clang's (`-fsomething`, `-Wfoo`,
`/clang:` options). The personality reads `cl`'s, passes an unknown `-`
option to bcc (which reads most Clang options), and warns about an unknown
`/` option as clang-cl does (`unknown argument ignored in clang-cl`). The
identity keeps the Clang macros and drops the GNU ones, as clang-cl does. It
prints no banner and no source names.

## TCC

**Command line.** GCC's short options: `-c`, `-o`, `-E`, `-I`, `-D`, `-U`,
`-l`, `-L`, `-Wl,`, `-shared`, `-static`, `-rdynamic`, `-nostdlib`, `-g`,
`-w`, `-M`, `-MD`, `-MF`, `-dM`, `-P`, `-x`, `-std=c99|c11`, plus `-run
FILE ARGS...` (compile FILE and run it with ARGS), `-b` (bounds checking),
`-bt` (backtraces), `-B` (library directory), `-v`, `-vv`, `-dumpversion`,
`-print-search-dirs`. TCC ignores the `-W`, `-f` and `-m` options it does
not know, and rejects any other unknown option (`invalid option -- '-q'`).

**Mapping.** `-run` builds the program in a temporary directory, runs it with
the remaining arguments and exits with its status (the signal number plus 128
when it is killed). `-b` and `-bt` are accepted with a warning; bcc has no
bounds checker. `-B` is ignored. Of the `-f` and `-m` options TCC ignores,
those GCC knows are treated as the GCC personality treats them, so one that
would change the program (`-fshort-enums`) is refused rather than silently
dropped. `-std=c11` and `gnu11` select `gnu11`, any
other value `gnu99`, as TCC sets `__STDC_VERSION__`.

**Identity.** `__TINYC__ 927` (TCC 0.9.27), no GNU or Clang macro, and
`__STDC_VERSION__ 199901L` by default. glibc's headers take their portable
paths. TCC's diagnostics are `file:line: error: message`.

## Decisions

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
  fail as they would there.
- **Claim the compiler mimicked.** The GCC and TCC personalities claim GCC and
  TCC in the predefined macros rather than Clang, because build systems and
  headers choose their paths from them (CMake reads `__GNUC__` and
  `__clang__`; glibc and many projects test `__GNUC_PREREQ`). The identity
  therefore brings the GCC features those paths use: the `_FloatN` types
  and `__builtin_va_arg_pack`. Use `--clang-mode`, or `-Xbcc
  --compiler-identity=clang`, to keep Clang's paths.
- **Name bcc.** Every banner names bcc and says it is not the compiler it
  mimics; `__bcc__` stays defined in every identity.
- **No invented MSVC codes.** See Diagnostics under MSVC.
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
- Of a GNU line marker's flags, only 3 has an effect: it makes the lines
  up to the next marker a system header, as in GCC, so a `.i` file's system
  header sections keep their warnings withheld. Flags 1 and 2 (entering and
  leaving an include) do not rebuild the include stack.
- The `-dM` listing holds the macros bcc predefines, not the ones GCC or the
  real Clang would; the target tables are Clang's. The GCC identity's
  floating-point macros for the types bcc has match GCC's, but the standard
  types' constants keep Clang's spelling, and GCC's `_Float16` and `__bf16`
  macros (`__FLT16_*__`, `__BFLT16_*__`) are missing with those types.
- `cl /J` defines `__CHAR_UNSIGNED__` as well as `_CHAR_UNSIGNED`, as
  clang-cl does; `cl` defines only the latter. The MSVC identity keeps
  Clang's target macros, as for `__x86_64__`.
- The MSVC personality links through Clang's driver and LLD (`lld-link`),
  which needs the Windows SDK and MSVC libraries (from `%LIB%`); on a Linux
  host without them only `/c`, `/E`, `/P` and `/Fa` work.
- `-v` prints the version lines but not the commands GCC runs; `-###`
  prints the bcc command line instead of GCC's subprocesses.
- `/showIncludes` prints `Note: including file: PATH` for every header but
  does not indent nested ones; `/sourceDependencies`, `-H` and `-MG` are not
  implemented.
- TCC's `#!` script line (`#!/usr/bin/tcc -run`) is not skipped.
- The GCC lists come from GCC 15.2, which knows a few options GCC 14 did not.
- `/Za` turns off only what bcc implements of Microsoft's extensions to C:
  the anonymous tagged members and `_MSC_EXTENSIONS`. The `__`-prefixed
  keywords stay, and `cl`'s other `/Za` changes concern extensions bcc never
  accepted.
- The MSVC runtime libraries are chosen at link time with
  `-nodefaultlib:libcmt` and `-defaultlib:`; bcc's objects carry no
  `/DEFAULTLIB` directives as `cl`'s do, so objects compiled with different
  runtime options do not conflict at link time.

## Gaps in the compiler these modes expose

These are compiler features, outside the driver, that the personalities
would use:

- `-fsemantic-interposition`, `-fwrapv-pointer`,
  `-fno-delete-null-pointer-checks` and common symbols (`-fcommon`).
- `_Float16` (GCC 12 and Clang on x86-64), `__bf16` (GCC 13) and Clang's
  `__fp16`, with their macros.
- `<float.h>` names for the `_FloatN` types (`FLT32_MAX` and the rest):
  bcc's resource header is Clang's, which knows none of them.
- Binary128 arithmetic in lowering: `_Float128` and `__float128` values
  type-check, and glibc's declarations of them compile, but computing with
  them is refused.
