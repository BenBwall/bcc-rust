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
  named after the stage and output as GCC names them; `-M` and `-MM`
  without `-E`, `-c` or `-S` take `-o` with several inputs, each unit's
  rules replacing the last, as in GCC;
- standard input (`-`) as a source, whose `-c` and `-S` outputs are `-.o`
  and `-.s`, as GCC and Clang name them; an output named like an option
  (`-o -app`), and the temporary objects named after it, reach the bundled
  clang as `./-app` and so as files;
- `-fsyntax-only` (`--syntax-only`), and a failing exit status from every
  mode, the inspection views included, after an error;
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
  `-f[no-]common`, `-f[no-]wrapv-pointer`, `-f[no-]strict-overflow` and
  `-f[no-]delete-null-pointer-checks`;
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

| | GCC | Clang | `cl.exe` | clang-cl | TCC | bcc |
| --- | --- | --- | --- | --- | --- | --- |
| Default target | host (MinGW-w64 on Windows) | host | `x86_64-pc-windows-msvc` | `x86_64-pc-windows-msvc` | host | `x86_64-unknown-linux-gnu` |
| Default mode | `gnu17` | `gnu17` | `c17` | `c17` | `gnu99` | `gnu17` |
| Microsoft extensions | off | off | all (`-fms-extensions`) | all | off | off |
| `__GNUC__` | 14.2.0 | 4.2.1 | undefined | undefined | undefined | 4.2.1 |
| `__clang__`, `__llvm__` | undefined | 23.1.1 | undefined | 23.1.1 | undefined | 23.1.1 |
| Other identity | `__VERSION__ "14.2.0"`, execution charset names, `__GCC_IEC_559` | | `_MSC_VER 1933`, `_MSC_FULL_VER`, `_MT`, `_MSVC_TRADITIONAL 0` | `_MSC_VER 1933`, `_MT` | `__TINYC__ 927` | |
| `_Float32` ... `_Float128` | keywords, distinct types | identifiers | identifiers | identifiers | identifiers | identifiers (`-ffloatn-types`) |
| `_Float16`, `__bf16` | types, GCC's `__FLT16_*__` and `__BFLT16_*__` | types, Clang's `__FLT16_*__` | identifiers | types | identifiers | types |
| `__fp16` | identifier | storage type | identifier | storage type | identifier | storage type |
| Tentative definitions | `-fno-common` (`-fcommon` makes them common) | same | same | same | same | same |
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
pinned Clang 23 rejects all five names and the suffixes in C, and neither
`cl` nor TCC has them, so the other personalities leave them identifiers
for glibc's typedefs. The half-precision types follow each compiler as
well: `_Float16` and `__bf16` are types for GCC 13 and Clang, each with its
own excess-precision rules and macros, Clang's `__fp16` storage type for
the Clang identities, and none of them for `cl` or TCC
([language-standards.md](../../language-standards.md#half-precision-floating-types-2026-10-27)).
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
  `-fomit-frame-pointer`, `-fstrict-aliasing`), code generation tuning (`-mtune=`, `-falign-*`,
  `-mtls-dialect=`), what bcc always does (`-fno-strict-aliasing`: bcc
  emits no type-based alias information;
  `-ffp-contract=off`: bcc forms no fused operations; `-fexcess-precision=`:
  x86-64 has none; `-mno-avx`: bcc uses only the baseline instruction set;
  `-march=x86-64`), the
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
  on it, but its macros such as `__AVX2__` stay undefined), and
  `-ffunction-sections`/`-fdata-sections` (the linker's `--gc-sections`
  removes less). The build goes on and says what it lacks.
- **Refused**, with the compiler's kind of error, for everything else:
  options that change layout or the ABI away from the target's
  (`-fshort-enums`, `-fshort-wchar`, `-fpack-struct=3`, `-mms-bitfields` on
  Linux, `-mno-ms-bitfields` on Windows, `-mregparm=`, `-mlong-double-64`
  outside MSVC, `cl /Gv`), instruction sets bcc's code needs (`-mno-sse`,
  `-mgeneral-regs-only`; `cl /arch:AVX2`, which also changes macros),
  language rules bcc does not follow (`-fsemantic-interposition`,
  `-ffast-math`, `-frounding-math`, `cl /fp:strict`, `/volatile:ms`), and
  instrumentation whose output a build expects (`-fsanitize=`,
  `-fprofile-arcs`, `--coverage`, `-finstrument-functions`, `-fopenmp`).
  An option the lists do not name is refused too, so a newly added option is
  refused until someone audits it.

Where bcc can do what the option asks cheaply, the personality maps it
instead: `-fwrapv` (lowering marks signed arithmetic without `nsw`),
`-fwrapv-pointer` (pointer arithmetic without `inbounds`, so it wraps in
LLVM and the interpreter), `-fno-strict-overflow` (both, as in GCC),
`-fno-delete-null-pointer-checks` (every LLVM function is
`null_pointer_is_valid`, as Clang marks them, so a dereference does not let
LLVM drop a later null check; bcc's own optimizer never drops one),
`-fcommon` (an external tentative definition the unit leaves tentative is
a common symbol, LLVM `common` or the native back end's `.comm`, which its
ELF and COFF writers encode, with MinGW's `-aligncomm` above a byte; GNU
`common` asks for one under `-fno-common` and `nocommon` keeps an object a
definition, the last written winning. As in Clang, a common symbol is
never constant, `dllexport` exports it, and it is not made for a
thread-local object, an imported one or one in a section, nor on the MSVC
target for one whose alignment its declaration, its typedef, its
`aligned` record type or a member of that record requires, since that
linker aligns a communal symbol by its size; bcc's objects then link with
GCC's `-fcommon` objects, an initialized definition winning),
`-fpack-struct=N` and `cl /ZpN` (the packing before any `#pragma pack`),
`-fpack-struct` alone (every structure packed, the pragmas ignored with
GCC's warning), the options that give up part of IEC 60559 semantics
(`-fno-signed-zeros`, `-freciprocal-math`, `-f[no-]trapping-math`,
`-f[no-]math-errno`, `-fassociative-math`, `-fcx-limited-range`,
`-fcx-fortran-rules`, their opposites, and `-fno-fast-math` and
`-fno-unsafe-math-optimizations`, which restore them): bcc's code keeps
IEC 60559 semantics, which they permit, and the GCC identity's macros
report them as GCC's do (`__GCC_IEC_559` 0, `__NO_SIGNED_ZEROS__`, ...),
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
Stages: `-E`, `-S`, `-c`, and linking otherwise, into `a.out`. Several inputs
are compiled separately and linked. GCC rejects an unknown option
(`gcc: error: unrecognized command-line option '-ffoo'`) except an unknown
`-Wno-*`, which it ignores. Queries: `--version`, `-v`, `-dumpversion` (the
major version since GCC 7), `-dumpfullversion`, `-dumpmachine`,
`-print-prog-name=`, `-print-search-dirs`, `-print-multiarch`, `-###`.
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
reach bcc's assembly inputs. `-g*`, `-pipe`, `-save-temps` and `--param`
have no effect; `-B` and `-specs=` warn that they have none. `-Ofast` is
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
`scripts/update_driver_options.py`. Unknown warnings are ignored as GCC
ignores them; the known `-f` and `-m` options bcc does not implement are
treated as the section above describes. Many of them do change what a
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

**Identity.** bcc's default: the pinned Clang 23.1.1 and the GCC 4.2.1 Clang
claims. `--version` prints `clang version 23.1.1 (bcc 0.1.0, ...)` with the
target, thread model and installed directory, as Clang does.

## MSVC `cl.exe`

**Command line.** Options start with `/` or `-` and are case-sensitive.
Most take their value joined (`/Fefoo.exe`, `/Ifoo`, `/Fo:out.obj`); `/I`,
`/D`, `/U` and `/FI` also take the next argument, and so do `/Fe:`, `/Fo:`
and `/Fi:` alone (`/Fo: out.obj`). `/DNAME#VALUE` defines NAME
as VALUE. `cl` compiles each source to an object named after it (`a.c` to
`a.obj`) and keeps it, then links an executable named after the first source
(`a.exe`) unless `/c` is given. `/Fo` and `/Fe` name a file, or a directory
when they end in a separator. `/link` passes the rest of the command line to
the linker. `cl` prints a banner to stderr unless `/nologo` is given, and the
name of each source it compiles. An unknown option draws `cl : Command line
warning D9002 : ignoring unknown option '/foo'` and is ignored. A file
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
link, and exits with status 2, as it does after any error.

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
one. The MSVC codes bcc writes are those Microsoft documents for the
driver's own messages: warnings D9002 (an unknown option), D9024 and D9027
(inputs), error D8036 (`/Fo` for several sources), and fatal error C1083
for a source that cannot be opened. bcc's own driver messages take `cl`'s
command-line form without a code (`cl : Command line error : cannot build
...`). `/showIncludes` indents each header by one space more per level of
nesting, as `cl` does, and leaves out a re-inclusion whose include guard
is already defined. `cl` writes compiler diagnostics,
C1083 among them, to stdout; bcc writes them to stderr, as in its other
modes, and writes only the source names to stdout, as `cl` does.

**Banner.** `bcc 0.1.0, compatible with Microsoft (R) C/C++ Optimizing
Compiler Version 19.33.0 for x64; not a Microsoft product`, to stderr. Tools
read the version and `for x64` from it.

## clang-cl

clang-cl reads `cl`'s options and also Clang's (`-fsomething`, `-Wfoo`,
`/clang:` options). The personality reads `cl`'s, passes an unknown `-`
option to bcc (which reads most Clang options), and reads an argument that
starts with `/` and names no option as an input, as clang-cl does on every
host (see below); it warns, with clang-cl's `unknown argument ignored in
clang-cl`, about a `cl` option clang-cl knows and bcc does not, and about
the bare Clang options clang-cl ignores. Like
clang-cl it reads the `/clang:` options after all the others, wherever they
stand, so `/clang:-fsigned-char /J` leaves `char` signed, and ignores a
bare `-fsigned-char` or `-funsigned-char` with the same warning. The
identity keeps the Clang macros and drops the GNU ones, as clang-cl does. It
prints no banner and no source names. As Clang's driver does, it refuses a
missing source before compiling any (`clang-cl: error: no such file or
directory: 'missing.c'`; also an input for the linker whose name starts
with `/`, with `; did you mean '/Brepro'?` when an option is one edit
away), warns that a stage before linking leaves an input
for the linker unused (`clang-cl: warning: x: 'linker' input unused
[-Wunused-command-line-argument]`), stops at the first source that fails to
compile, and exits with status 1 after an error. It reads `-` as standard
input, which `/E` preprocesses as C and which compiles only when `/Tc`
names its language (`clang-cl: error: use /Tc or /Tp to set input type for
standard input` otherwise).

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
  not know it (`/Fp: /tmp/a.pch`, `/imsvc /usr/include`), so it is no
  input. `/Fo` takes its value joined only: in `/Fo /tmp/a.obj`, as in
  clang-cl, `/tmp/a.obj` is an input.
- A missing input is reported as each compiler reports one: clang-cl's
  `no such file or directory: '/tmp/missing.c'`, also for an input for the
  linker that starts with `/` (a mistyped option: `/Brepo` draws `; did you
  mean '/Brepro'?`), and `cl`'s fatal error C1083 for a source, or warnings
  D9024 and D9027 for any other input.

`cl` on Windows keeps its own reading: an unknown `/` option draws warning
D9002, and `--` is an unknown option too.

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
other value `gnu99`, as TCC sets `__STDC_VERSION__`. TCC reads its inputs
in order and stops at the first it cannot add, so the inputs after one it
cannot open, missing or unreadable, are dropped and the run fails with
`tcc: error: file 'missing.c' not found` (exit status 1), after compiling
the C sources before it. `-c` and `-o` with several inputs is `tcc: error:
cannot specify output file with -c many files`. Under the TCC identity a
`#!` line that starts a file is skipped, as TCC skips it, so a script
starting `#!/usr/bin/tcc -run` runs; elsewhere the line stays a
non-directive, which GCC and Clang reject.

**Identity.** `__TINYC__ 927` (TCC 0.9.27), no GNU or Clang macro, and
`__STDC_VERSION__ 199901L` by default. glibc's headers take their portable
paths. TCC's diagnostics are `file:line: error: message`.

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
| Pointer/integer cast warnings (`pointer_cast_rules`, #174; `cl`'s C4311 and C4312 warn only for a narrower integer) | GCC, TCC | Clang, clang-cl, MSVC (`ms_compatible_rules`) |
| `-MG` lists a missing `<...>` header under `-MM`; `-H` and `/showIncludes` name a header reached from the working directory `./a.h` (the driver) | GCC, TCC, MSVC | Clang, clang-cl |

A rule that compares with `CompilerIdentity::Gcc` alone is equivalent where
the feature it governs exists only under GCC's identity among those that
follow GCC's rules (the `_Float16` pedantic reports in the lexer and
parser). Neither TCC's nor `cl`'s behaviour was surveyed for these rules:
there is no TCC or MSVC on the machines that produced them, and Microsoft
documents too little of `cl`'s.

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
  which needs the Windows SDK and MSVC libraries (from `%LIB%`); on a Linux
  host without them only `/c`, `/E`, `/P` and `/Fa` work.
- `-v` and `-###` print the commands bcc runs, not GCC's or Clang's: each
  step is a bcc command line that bcc runs in process, named by the path
  of the running bcc, then, under `-v`, the bundled clang's assembly and
  link commands. There is no `cc1`, `as` or `collect2` line and no
  `COLLECT_GCC_OPTIONS`; GCC's `-###` quotes the words with characters
  other than letters, digits, `_`, `/`, `-` and `.`, and its `-v` none, as
  GCC does. The header search lists every configured directory, without the
  embedded resource headers, which are no directory on disk.
- clang-cl's `/o` (a deprecated `/Fe` or `/Fo`), `/imsvc` and the
  `/winsysroot` family are not implemented; they draw the unknown-option
  warning, so `/opt/a.c`, which clang-cl reads as `/o`, is ignored with it
  rather than compiled. Use `-- /opt/a.c`.
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
  runtime options do not conflict at link time.

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
- TCC stops at the first input that fails to compile, not only at a missing
  one; bcc compiles the others under `-c`, as GCC does. A missing input
  that is not a C source is reported before the C sources ahead of it are
  compiled, so under `-c` their objects are not written.
- TCC's `-run` builds the program into a temporary directory with the
  bundled clang's linker and runs it, rather than compiling it in memory.

## Gaps in the compiler these modes expose

These are compiler features, outside the driver, that the personalities
would use:

- `-fsemantic-interposition`, GCC's default with `-fPIC`: bcc inlines and
  calls a default-visibility definition of a shared library as if it could
  not be interposed, as Clang does by default and GCC under
  `-fno-semantic-interposition`, so the option is refused.
- Decimal floating types, and with them GCC's C23 `<float.h>` `DEC*`
  names; `FLT16_SNAN` and GCC's other `__builtin_*f16` constant builtins.
- Whether `cl` makes a C tentative definition a communal symbol by
  default is unverified: no MSVC was available, and Microsoft documents no
  option for it. The `cl` personality keeps bcc's `-fno-common` default;
  clang-cl, like Clang 11 and later, has the same default, and passes a
  given `-fcommon` on.

Since this list was written, bcc gained binary128 arithmetic on every back
end (#160), `_Float16` (#163), `__bf16`, complex `_Float16` and `__fp16`
(#164), the `_FloatN` `<float.h>` names, constant compound literals in
static initializers (#170), and compound assignment to atomic floating
objects (#171); the personalities use each through the identity.
