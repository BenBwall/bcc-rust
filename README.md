# bcc-rust

`bcc-rust` is an experimental Rust implementation of a C compiler front end
supporting C89/C90, C95, C99, C11, C17/C18, C23 and a documented C2y draft
subset, plus GNU and opt-in MSVC extensions. Its non-recursive language parser
builds syntax trees; semantic analysis covers declarations, expressions,
initializers, statements and functions. Code generation is not implemented.

See [language-standards.md](language-standards.md) for language modes, extension
flags, shared configuration, implementation details and remaining semantic
boundaries. The CLI defaults to `gnu17`; library configuration defaults to strict
C99. Pass `-std=c23`, `-std=c2y`, or any documented GCC standard alias to select a
mode. `-pedantic`/`-Wpedantic` warn on extensions; `-pedantic-errors` makes them
errors. MSVC syntax is independently enabled with `-fms-extensions` or its ten
individual feature flags; later flags win. Translation targets a hosted
execution environment by default; `-ffreestanding` selects a freestanding one
and `-fhosted` restores hosted, the later flag winning.

## Current status

The CLI accepts a C source file or an input string, runs preprocessing and the
language parser, and prints diagnostics. Pass `--syntax-tree` for a stable,
source-oriented tree, `--syntax-locations` to add locations, `--raw-syntax` for
the raw Rust debug form of the tree, or `--tokens` for parser-facing
preprocessing tokens. Normal operation does not dump internal storage. The CLI does not emit an object file
or executable.

Diagnostics are rendered like `rustc`'s: a lowercase message, the
`file:line:column` location, the quoted source line with `^` under the problem
and `-` under related input, and `= note:` / `= help:` lines, which cite the
C99 clause (N1256) where a rule was broken. Messages quote source spellings, so
output never depends on interned-string handles, arena indices, or host
`long double` layout. An error at exactly the same place as the one before it
is folded into that error, and the run ends with a summary such as
`2 errors and 1 warning generated.` Color is used on terminals unless
`NO_COLOR` is set; `CLICOLOR_FORCE` forces it.

`--tokens` prints one line per token: its location, kind, source spelling, and
for constants the value and C type (for example
`` <input>:1:1: floating constant `1.5L` = 0x1.8p+0 (long double) ``).
`long double` values print as exact hexadecimal without the C library's
`printf`. `--syntax-tree` spells operators, qualifiers, storage classes, and
constants as C, and `--syntax-locations` names the file for nodes outside the
main source.

`Parser::parse_translation_unit` returns an ordered `ParsedTranslationUnit`
whose roots borrow the syntax tree from the translation-unit arena. Declarations, prototype-style and old-style
function definitions, blocks, every C99 statement family, expressions, type
names, and initializers run through one explicit frame stack in the parse arena.
Malformed input retains repaired syntax where meaningful, produces a
provenance-only external error node for pure top-level garbage, and emits
structured FIFO diagnostics with recovery context.

Parser resource limits follow representation bounds: `usize` for root and
syntax-node counts, `u32` for frame/scope depth, and each provenance arena's
`u32` index space. There is no fixed aggregate source-segment budget. Flat syntax
lists collect provenance once instead of repeatedly copying their growing prefix.

Phase 05 is complete. The parser meets the parser-relevant C99 minimum
translation floors, diagnoses invalid phase-7 input, and has deterministic
truncation/property coverage. The integrated language modes also cover C23
attributes and feature queries, resource embedding into initializers, modern
literals, GNU macros/keywords/imaginary constants, and MSVC macro pragmas and
empty variadic calls through the CLI. Declaration semantic analysis now follows parsing in the default CLI mode;
`--semantic-types` inspects resolved declaration types, linkage and duration.
See [semantic-analysis.md](semantic-analysis.md) for implemented boundaries and
validation gaps. Code generation remains unimplemented.
This is not yet a production-ready or conforming C99 compiler.

## Prerequisites

- Rust 1.99.0, pinned in [`rust-toolchain.toml`](rust-toolchain.toml) and declared
  as the minimum supported version in [`Cargo.toml`](Cargo.toml).
- Clippy and Rust's `llvm-tools` component, both selected automatically by
  `rust-toolchain.toml`.
- Git, CMake 3.20 or newer, Ninja, Python, and a C++17 compiler for building
  Clang, libclang, and LLD from Rust's pinned LLVM sources. On Windows GNU,
  installed Visual Studio C++ Build Tools are preferred for this host-tool
  build; MinGW GCC/G++ can also bootstrap it. The first Cargo build compiles
  LLVM; later builds reuse the Ninja cache under ignored `target/llvm/`.
- For Windows GNU, MinGW headers. The build finds the installation through
  `gcc.exe` on `PATH`, or accepts `MINGW_ROOT` explicitly. GCC does not compile
  the C helper. For MSVC, the Visual Studio C headers and SDK are required.
- On Linux, the target's static C runtime development libraries. Instead of
  building LLVM, a Linux host can use LLVM's official prebuilt release of the
  same version (`LLVM-23.1.1-Linux-X64` from the `llvmorg-23.1.1` GitHub
  release). Unpack it, point `target/llvm` at it, and write `23.1.1` to its
  `.installed-version`; the build then skips compiling LLVM and still checks
  every tool's version. If the release's `ld.lld` cannot load its ICU
  libraries, as on Ubuntu 26.04, replace it with a link to the pinned
  toolchain's `lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld`, which is
  LLD 23.1.1 from the same LLVM revision as rustc. The build uses only
  `clang`, `llvm-ar`, `ld.lld`, `libclang`, and Clang's resource headers from
  the release.
- Nightly Rustfmt for the repository's unstable formatting options.
- A 64-bit host and target. Every arena and growable compiler buffer
  reserves 100 GiB of address space, which costs no memory until it is
  written, so 32-bit targets are not supported.

## Source submodules

[`vendor/rust`](vendor/rust) is a shallow, filtered submodule pinned to the
same commit as Rust 1.99.0 (`b940084d7eb6a299eb4bfeb8e34901bc051e7ac4`). Its
LLVM submodule pins `1b9c0d5ff9bbe7634aead059efe6b11a7eeba145` (LLVM 23.1.1).
Git pins both source revisions; the native build checks that Clang, libclang,
and the archiver report the same LLVM version as rustc.
Upgrading Rust requires updating the Rust submodule to the new compiler's
commit and checking out the LLVM revision that it pins.

Sparse patterns in [`build_support/rust.sparse`](build_support/rust.sparse)
retain Rust's LLVM gitlink, submodule metadata, and license files.
[`build_support/llvm.sparse`](build_support/llvm.sparse) retains the LLVM,
Clang, LLD, supporting build sources, the LLVM libc sources that LLVM's
configuration requires for its shared utilities, and LLD's required libunwind
header; tests, documentation, and examples are excluded except for small CMake
entrypoints required by configuration.
Git stores sparse-checkout settings locally, so prepare a fresh checkout with
these PowerShell commands:

```powershell
git clone --filter=blob:none --no-checkout --depth 1 --branch 1.99.0 https://github.com/rust-lang/rust.git vendor/rust
Get-Content build_support/rust.sparse | git -C vendor/rust sparse-checkout set --no-cone --stdin
git submodule update --init --checkout vendor/rust
git -C vendor/rust submodule init src/llvm-project
git init vendor/rust/src/llvm-project
git -C vendor/rust/src/llvm-project remote add origin https://github.com/rust-lang/llvm-project.git
git -C vendor/rust/src/llvm-project config remote.origin.promisor true
git -C vendor/rust/src/llvm-project config remote.origin.partialclonefilter blob:none
$llvmCommit = git -C vendor/rust rev-parse HEAD:src/llvm-project
git -C vendor/rust/src/llvm-project fetch --depth 1 --filter=blob:none origin $llvmCommit
Get-Content build_support/llvm.sparse | git -C vendor/rust/src/llvm-project sparse-checkout set --no-cone --stdin
git -C vendor/rust/src/llvm-project checkout --detach $llvmCommit
git submodule absorbgitdirs vendor/rust
```

## Build, test, and inspect

Run checks from the repository root:

```sh
cargo build
cargo test --all-targets
cargo test --features benchmarking-internals --test allocation_count
cargo +nightly fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features benchmarking-internals -- -D warnings
```

With `benchmarking-internals`, the `allocation_count` test asserts that
compiling a translation unit allocates nothing from the global allocator;
[its module documentation](tests/allocation_count.rs) lists what it covers.

Cargo explicitly targets the host so build scripts and proc macros do not
receive the executable's LTO flags. Artifacts are consequently under
`target/<host-triple>/debug/` or `target/<host-triple>/release/`.
Explicit `embed-bitcode=yes` keeps this configuration compatible with Cargo's
debug and test profiles as well as release builds.
`RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` override Cargo configuration, even
when empty; unset them for the standard build. The native build rejects
overrides that remove linker LTO.

### Native C, static linking, and cross-language LTO

[`build_support/native.rs`](build_support/native.rs) compiles the C helper with
the `cc` crate and Clang `-flto=full` into a static archive of LLVM bitcode.
[`build_support/llvm.rs`](build_support/llvm.rs) uses the `cmake` crate and
LLVM's `install-distribution` target to build only Clang, its resource headers,
libclang, `llvm-ar`, and LLD from Rust's exact LLVM source revision. Only the
host's backend is enabled. Rust uses
`-C linker-plugin-lto -C lto=fat`, allowing the final linker to optimize across
the Rust/C boundary. The [Rust LTO documentation](https://doc.rust-lang.org/rustc/linker-plugin-lto.html)
describes this combination and why matching LLVM versions matters.

Under this combination, code generation can introduce calls to
`compiler_builtins` functions after LTO has already discarded
`rust_eh_personality`, which their unwind tables reference. Signed 128-bit
division and `f128` arithmetic are examples. The linker would then fail with
`undefined symbol: rust_eh_personality`. The Windows GNU and Linux flags in
[`.cargo/config.toml`](.cargo/config.toml) therefore force those objects in
before LTO, and [`tests/late_builtins.rs`](tests/late_builtins.rs) fails to link
if the list stops covering them.

Cargo selects the source-built linker directly from `target/llvm/bin/`.
The C compiler, archiver, libclang, and linker all come from that pinned
build; no setup script or linker wrapper is needed. The shared LLVM cache is
locked while CMake builds and installs the tools, so
independent Cargo builds cannot write its outputs simultaneously.
Rust libraries and the native C helper are statically linked into the executable.
On Windows GNU, Windows' system DLLs—including `msvcrt.dll`—remain OS
dependencies; no LLVM, libclang, Rust, GCC, or pthread DLL needs to accompany
the executable. Linux uses the
matching Clang/LLD driver and requests a static CRT; MSVC also requests a static
CRT. The complete build and executable verification have been exercised on
`x86_64-pc-windows-gnu`; Linux and MSVC configurations require validation on
their respective hosts.

libclang is loaded only by the binding generator during the build. It is not
linked into the resulting executable. Rust's `llvm-tools` component supplies
optional symbol and disassembly tools for inspecting the executable.

The source pin matters as well as the reported LLVM version: Rust may carry
LLVM changes that a same-version external Clang does not contain. Keep Clang,
libclang, the archiver, and LLD on Rust's pinned LLVM revision. When changing
this setup, verify that the C archive contains LLVM bitcode, inspect the linked
executable for cross-language optimization and dynamic dependencies, and run
float, double, and long-double hex-literal inputs with only the intended system
runtime available. A successful link alone does not demonstrate cross-language
LTO or a self-contained executable.

Float and double conversions go through the C helper as well. On MinGW it
selects the C99 conversion routines explicitly, so hex floats keep working
without relying on the linker's choice of a legacy Windows `strtod` symbol.

These are the canonical Phase 05 gates. Exercise the parser with either input
form, and opt into the desired inspection view:

```sh
cargo run -- --input 'typedef int T;
T *value;'
cargo run -- --syntax-tree --syntax-locations test-programs/test.c
cargo run -- --raw-syntax --input 'int value = 1;'
cargo run -- test-programs/test.c
cargo run -- --tokens --input '#define N 3
N + 1'
```

Header lookup follows Clang. `#include "name"` looks beside the including
file (the working directory for `--input`), then in each `-iquote` directory;
both forms then search, in order, each `-I` directory, `CPATH`, each
`-isystem` directory, `C_INCLUDE_PATH`, the embedded `<built-in>` resource
directory, the C library's directories, and each `-idirafter` directory. User
headers therefore take precedence over resource headers, and a resource header
can `#include_next` the C library's header of the same name. `--sysroot <dir>`
names the C library's directories `<dir>/usr/local/include` and
`<dir>/usr/include`; without it no library directory is searched, so a C
library elsewhere (MinGW-w64, the MSVC UCRT, a multiarch
`/usr/include/x86_64-linux-gnu`) is added with `-idirafter` or `-isystem`.
Target-driven discovery of these directories is future work. `-nostdinc`
removes the resource and library directories, `-nostdlibinc` only the library
directories, and `-nobuiltininc` only the resource directory; `-idirafter`
directories stay. The GCC single-dash spellings `-iquote`, `-isystem` and
`-idirafter` take their directory separately or joined, as `-I` does;
`--iquote`/`-q` and `--isystem`/`-s` remain. The working directory is never
searched implicitly. The environment variables use the platform path separator,
and, as in GCC, an empty element names the working directory. A missing
header's diagnostic lists every directory searched.

Build-system options `-D NAME`, `-DNAME`, `-D NAME=VALUE`, `-DNAME=VALUE`,
`-U NAME`, `-UNAME`, and `-include FILE` are supported. An omitted value means
`1`; an explicit empty value stays empty, and values may contain spaces and
additional `=` characters. Quote each argument for your shell. Function-like
definitions such as `'-DF(x)=((x)+1)'` use the normal macro parser. Definitions
and undefinitions run in their command-line order after predefined macros;
forced includes then run in their command-line order, as in GCC/Clang. A forced
include starts lookup in the working directory and then uses the quoted include
search path. Definitions and startup diagnostics have stable `<command line>`
provenance. Builtin protections still apply. `-imacros` is not implemented.

Headers found through `-isystem`, `C_INCLUDE_PATH`, the resource directory, the
C library's directories or `-idirafter`, and headers that use `#pragma GCC
system_header`, are system headers, as in GCC: their warnings and extension
diagnostics are withheld, even under `-pedantic-errors`, while errors are
still reported. See
[language-standards.md](language-standards.md#system-headers) for the rule,
including how macro expansions are attributed. Macro redefinitions are
warnings, and errors under `-pedantic-errors`, as in GCC and Clang. `__DATE__` and `__TIME__` are fixed once per translation unit. For reproducible output, `--source-date-epoch <seconds>` or the `SOURCE_DATE_EPOCH` variable pins them to that UTC time; the flag wins, a malformed variable is ignored, and a malformed flag value is an error. Files under [`test-programs/`](test-programs/) are useful manual inspection inputs, but they are not an automated conformance suite.

The resource directory holds the seven headers a freestanding implementation
provides: `<float.h>`, `<iso646.h>`, `<limits.h>`, `<stdarg.h>`, `<stdbool.h>`,
`<stddef.h>` and `<stdint.h>` (C99 §4p6, printed p. 7; PDF p. 19). `<stdalign.h>`
and `<stdnoreturn.h>` are also available, with the Clang language-mode macro
gates documented in [language-standards.md](language-standards.md).
`<mm_malloc.h>`, which GCC and Clang ship and MinGW-w64's `<malloc.h>`
includes, defines `_mm_malloc` and `_mm_free` over the C library's aligned
allocators. These are
embedded compiler resources; with `-ffreestanding`, or when no C library is
configured, no on-disk include directory is required. Hosted, each behaves like
Clang's resource header of the same name: `<limits.h>` and `<stdint.h>` chain
to the C library's header with `#include_next` (`<float.h>` too for MinGW-w64
and the MSVC runtime), and `<stddef.h>` and `<stdarg.h>` answer the partial
`__need_*` requests that glibc's headers make. Diagnostics and token dumps name
them `<built-in>/name.h`.

Select the C ABI with `--target`: `x86_64-unknown-linux-gnu` (default),
`x86_64-unknown-linux-musl`, `x86_64-w64-windows-gnu`, or
`x86_64-pc-windows-msvc`. MinGW also accepts `x86_64-w64-mingw32` and
`x86_64-pc-windows-gnu`. The selected ABI controls long widths, long-double
precision, wide literals, record bit-fields, stdarg types and the
target-description and OS/ABI macros, in all language modes and independently
of the compiler host and language-extension flags. Compiler identity is a
separate contract that follows Clang: every language mode, including strict
ISO modes, claims GCC 4.2.1 (`__GNUC__` `4`), with `__GNUC_GNU_INLINE__`
before C99 and `__GNUC_STDC_INLINE__` from C99 on. `__STRICT_ANSI__` is
defined only in strict modes. `-fms-extensions` claims MSVC 19.33
(`_MSC_VER` `1933`), and every mode defines `__bcc__`; `__clang__` is never
defined. See
[language-standards.md](language-standards.md) for the full list.
`offsetof` and the varargs intrinsics have semantic types; this front end does
not generate the code that performs varargs operations.

The preprocessor permits 200 simultaneously nested included headers, excluding
the main source file. An include beyond that limit produces a diagnostic and
processing continues with the remaining input. This exceeds C99's required
minimum of 15 nested includes; the ceiling also makes an unguarded recursive
include terminate without exhausting the process.

`__STDC__` is `1` and `__STDC_VERSION__` is `199901L` (or the selected
revision's value). `__STDC_HOSTED__` is `1` in the default hosted execution
environment and `0` with `-ffreestanding` (C99 §4p6, §5.1.2). A hosted
translation checks the portable signatures of `main` (§5.1.2.2.1); a
freestanding one leaves its startup function implementation-defined.
`__STDC_MB_MIGHT_NEQ_WC__` is `1`, permitting multibyte and wide-character
codes to differ. These predefined values describe the front end's selected
language and execution model; runtime support and code generation remain
unfinished.

Literal values use UTF-8 for source characters in ordinary strings and 8-bit
codes for numeric escapes (`\xFF` is one byte). Wide literals use 32-bit
codes, including non-scalar values from numeric escapes; UCNs still require
valid Unicode scalar values. Ordinary multi-character constants pack the last
four execution bytes into a signed 32-bit `int`, most-significant byte first;
the same value is used in syntax nodes and `#if` expressions. Adjacent
literals retain numeric codes separately from source characters; a wide member
makes the result wide. Literal values are interned separately from UTF-8 source
spellings. Identifier UCNs follow C99 Annex D and share identity with their
UTF-8 spellings.

### Pipeline order and benchmarks

The front end runs its translation phases in order over the whole
translation unit: each source file is lexed completely when it is opened, the
whole unit is preprocessed, as a standalone preprocessor would, and only then
is it parsed. Every preprocessing diagnostic is therefore reported before any
parser diagnostic. The parser CLI renders each consecutive same-file run of
parser diagnostics and preprocessing warnings in physical source order, using
invocation positions for macro errors. Preprocessing errors and include-file
transitions retain their reporting order. `--tokens` prints each token after
the diagnostics reported while it was produced.

An `#include` operand is lexed as ordinary preprocessing tokens; the
directive takes the header name from the source text between `<` and `>` or
between the string literal's quotes. It reports `'`, `//` and `/*` in either
form, and `\` or `"` in the `<…>` form, which C99 §6.4.7p3 leaves undefined,
as well as a missing closing delimiter. A `\` in a `"…"` name is accepted as a
path character, an extension that the configured extension policy can turn
into a warning or an error. While the extension is accepted, a quoted name
ends at its first `"`: `#include "dir\"` names `dir\`, and the lexer's
unterminated-string error for that operand is withdrawn.

Compilation takes its memory from arenas built directly on the operating
system's virtual memory, each dropped when its lifetime ends. The
translation-unit arena keeps source text, interned strings, diagnostics, and
the syntax tree; the preprocessing arena keeps lexed files and macros until
preprocessing ends; an expansion arena is reset between top-level macro
expansions; and the parse arena keeps the parser's frames and scopes until
parsing ends. The token array, source provenance, string cache, parsed roots,
and file-scope typedef set each grow in place in a region of their own.
Every region reserves its address space when first used and commits memory
only as far as it is written. Compiler code allocates from nothing else:
the `disallowed-types`, `disallowed-macros`, and `disallowed-methods` lists
in [`.clippy.toml`](.clippy.toml) reject global-allocator types and calls,
and the `allocation_count` test checks it at run time. The
[glossary](GLOSSARY.md#storage-and-lifetimes) defines each lifetime.

The lexer's byte-class scans use `std::simd` when the nightly-only
`portable-simd` feature is enabled and scalar loops otherwise. Criterion
benchmarks measure the lexer, the preprocessor, and the full pipeline over
generated plain, mixed, and macro-heavy inputs. The `memory` harness runs
each input and phase range (1-3, 1-6, 1-7) in a fresh process and reports
the operating system's peak commit and peak working set (peak resident
memory on Unix, where peak commit is unavailable). It then reports each
arena's high-water mark and the peak region count, reserved address space,
and arena commit. On Linux, arena regions request transparent huge pages and
commit whole 2 MiB pages, which removes nearly all page faults at the cost of
higher resident memory for small inputs. Measure Linux performance on Linux
itself where possible: under WSL 2, each page fault also goes through the
hypervisor's nested page tables (about 2 µs per minor fault in WSL
measurements), which exaggerates fault-heavy differences.

The `Parser only` group measures phase 7 with preprocessing in untimed setup.
It also includes expression-heavy and declaration-heavy inputs to exercise
nested expressions, declarators, aggregates, and initializers. The
`Preprocessor allocations` group isolates single strings, empty macro calls,
and nested reused arguments.

```sh
cargo +nightly bench --features benchmarking-internals,portable-simd --bench bench
cargo +nightly bench --features benchmarking-internals,portable-simd --bench memory
```

### Performance changes

Record the commit, toolchain, input, and feature set for each baseline. On a noisy shared machine, compare at least seven interleaved
runs and report their minimum alongside Criterion estimates. Separate full
pipeline time from `Parser only` time; preprocessing changes can otherwise look
like parser improvements. Track peak commit and peak working set from the
`memory` harness, the arena high-water marks, source-vector growth, and driver
steps per token when changing storage, provenance, or frame scheduling.

Keep syntax trees, diagnostics, recovery, and ordered source provenance
equivalent. Exercise macro/include locations, malformed-input continuation,
deep nesting, and resource boundaries; never weaken limits or drop provenance
to make a benchmark pass. Keep temporary counters out of production changes.
Run the checks listed under [Build, test, and inspect](#build-test-and-inspect),
including both `benchmarking-internals` commands.

The binary built with `benchmarking-internals` runs a preprocessing workload
instead of the normal CLI. Its Unix `coz` progress point measures preprocessing,
not parser time. Use the parser benchmark groups for parser measurements.

### External torture corpus

`cargo test --test torture_regressions` runs the reduced regressions found in
the corpus, including literal values, macro expansion, native long-double
conversion, and bounded recovery. The parser unit tests also check that long
syntax lists retain source provenance with linear storage growth.

After `cargo build --bin bcc-rust`, run
`python scripts/run_gcc_torture.py --output target/compiler-corpus/new-run`
to download and checksum-verify the pinned GCC 15.2.0 torture corpus, then survey
its compile and execute sources through this front end. Use a fresh output
directory for each run. `--limit 30` runs a smoke sample; `--match` selects
corpus-relative globs. Sources and detailed results stay in ignored `target/`.
The runner reports crashes/timeouts with a nonzero exit status and does not
execute C programs. It is a corpus survey rather than a conformance gate.
See the [compiler corpus research](compiler-test-corpus-research.md) and
[recorded parser results](gcc-torture-parser-results.md) for comparison profiles,
known discrepancies, and interpretation limits.

### libc header survey

The header survey measures how many of a C library's public headers bcc
compiles, taking Clang for the same target as the reference. It covers four
configurations: `glibc-x86_64-linux`, `musl-x86_64-linux`, `mingw-w64`, and
`msvc-ucrt`.

```sh
python scripts/fetch_libc_sysroots.py
cargo build --bin bcc-rust
python scripts/libc_header_survey.py --output target/libc-survey/new-run
```

`fetch_libc_sysroots.py` downloads about 10 MB of pinned distribution packages:
Debian 13 `libc6-dev` and `linux-libc-dev` from `snapshot.debian.org`, and
Alpine 3.20 `musl-dev` and `linux-headers`. It checks each against its recorded
SHA-256 before reading it, then extracts only `usr/include/` into
`target/sysroots/<configuration>/`, where a `sysroot.json` manifest records
the packages. Python reads the archives directly, and nothing from a package
is executed. Links inside the header tree become copies of their targets;
links that leave the sysroot abort the extraction. A few Linux headers differ
only in case (`xt_MARK.h` and `xt_mark.h`); on a case-insensitive file system
the second is skipped and listed in the manifest. `--list` prints the pinned
URLs, versions, sizes, and hashes.

The MinGW-w64 configuration takes its include directories from
`gcc -xc -E -v` on `PATH`, leaving out GCC's private `lib/gcc/...` directories.
The MSVC configuration takes them from `INCLUDE` when it is set, and otherwise
from `vswhere.exe` and the Windows Kits registry key (the Visual C++ `include`
directory, then the SDK's `ucrt`, `shared`, `um`, and `winrt` directories).
A configuration whose headers are missing is skipped unless `--config` names
it.

Each selected header becomes a translation unit holding only its `#include`
and one declaration. The selection is the C17 standard headers, the C23
headers the library ships, the common POSIX headers for glibc and musl, and
`windows.h` with other common Win32 and CRT headers for MinGW-w64 and MSVC
(Win32 headers such as `shellapi.h`, which need `windows.h`'s types, include
it first).
`--all-headers` adds every `.h` under the library's include roots. One more
unit includes all the standard headers together. Clang runs first with the
configuration's target and include flags and `-fsyntax-only`, in `-std=gnu17`
and, for the standard headers, also `-std=c17`. Clang's verdict decides which
headers are valid: bcc does not run on headers that Clang rejects, and the
results record why Clang rejected them. bcc exits 0 even after reporting
errors, so the survey counts its rendered diagnostics instead. A crash or
timeout makes the survey exit nonzero. `--config`, `--match` (a header glob),
`--jobs`, and `--timeout` narrow or tune a run.

bcc's arguments come from a template. The default,
`--target={triple} {flags} --std={std} "-idirafter {dir}"`, selects each
configuration's target and passes the library's include directories after
bcc's built-in resource headers, the order Clang uses, so the resource
headers can wrap the library's (`#include_next`). The separate word
`{flags}` expands to the configuration's equivalent bcc flags: `--sysroot`
for glibc and musl (as their Clang command passes), `-fms-extensions` for
MSVC, and no extra flags for MinGW-w64.
The other placeholders are `{triple}`, `{sysroot}`, `{std}`, `{config}`, `{dir}`,
and `{input}`. A word containing `{dir}` repeats once for each include directory, and a word
whose placeholder is empty is dropped (the MSVC configuration has no sysroot;
MinGW-w64's is the GCC installation root). The translation unit is appended
unless `{input}` appears. Override the template for every configuration or
for one, for example to rely on `--sysroot` lookup alone:

```sh
python scripts/libc_header_survey.py --output target/libc-survey/hosted \
  --bcc-args "--target={triple} {flags} --sysroot={sysroot} --std={std}" \
  --bcc-args-for msvc-ucrt "--target={triple} {flags} --std={std} '--isystem {dir}'" \
  --compare target/libc-survey/baseline/results.json
```

Each output directory holds `results.json` (run metadata, a per-configuration
summary, and one row per header and language mode), `results.jsonl` (rows as
they finish), the generated units under `tu/`, and the Clang and bcc output
under `logs/`. The metadata records the bcc binary's SHA-256, the Git head,
the Clang version, each configuration's flags and include directories, and
the package versions and hashes. The printed table gives, per configuration
and mode, the headers surveyed, those Clang accepts, those bcc also accepts,
and the combined unit's verdicts, followed by bcc's most common first errors.
The survey refuses an output directory that already has results.
`--compare OLD/results.json` lists the headers that bcc newly accepts or newly
rejects relative to an earlier run, and adding `--against NEW/results.json`
compares two finished runs without surveying. Run the scripts' tests with
`python -m unittest discover -s scripts -p "test_*.py"`.

### Branch cleanup

[`scripts/branch-cleanup.rs`](scripts/branch-cleanup.rs) is a nightly Cargo
script that deletes finished local branches. It needs an authenticated GitHub
CLI (`gh`):

```sh
cargo +nightly -Zscript scripts/branch-cleanup.rs                   # Update main and delete the latest merged PR's local branch.
cargo +nightly -Zscript scripts/branch-cleanup.rs --all             # Also delete other clearly finished local branches.
cargo +nightly -Zscript scripts/branch-cleanup.rs --all --dry-run   # Preview without switching, pulling, or deleting.
```

Cleanup reads every PR to find the most recent merge into `main`, fetches and
prunes `origin`, switches to `main`, and runs `git pull --ff-only origin main`
before deleting anything. The `--all` mode also removes merged PR branches,
closed PR branches whose local tip still matches the PR head, and branches
contained in `origin/main` whose upstream is missing or unset. Branches with open
PRs, extra local commits, protected names (`main`, `master`, `develop`, `dev`,
`production`, `staging`), or another worktree checkout are kept, and fork PRs
never authorize deleting a same-named local branch. Comparing the local tip with
the merged PR's head commit recognizes squash merges. Commit or stash changes
before a real cleanup; only local branches are deleted. Run its tests with
`cargo +nightly -Zscript test --manifest-path scripts/branch-cleanup.rs`.

### CodeGraph index

[CodeGraph](https://github.com/colbymchenry/codegraph) gives coding agents a
local symbol and call graph of the repository sources. The project configuration in
`.mcp.json` (Claude Code) and `.codex/config.toml` (Codex) starts its MCP
server only when CodeGraph resolves the checkout or worktree's own root as an
initialized project, so an empty index directory (without CodeGraph's database)
or an enclosing checkout's index is not used. Agents can then query the graph
instead of searching file by file.
It is optional: install the `codegraph` CLI, then run `codegraph init` in each
checkout or worktree:

```sh
npm install -g @colbymchenry/codegraph
codegraph init --yes
```

CodeGraph chooses the index directory, including `.codegraph-wsl/` on Windows
drives under WSL and `CODEGRAPH_DIR` overrides. The index follows file changes
automatically; `codegraph sync` catches up after edits made while no server ran.
[`codegraph.json`](codegraph.json) leaves the C inputs under `tests/fixtures/`
and `test-programs/` out of the graph, since they are compiler test data rather
than code. Without an index at the checkout's own root, or without the CLI,
agents report the server as failed to start and search as before. The guard in
[`scripts/codegraph_mcp.py`](scripts/codegraph_mcp.py) makes that check; its
tests run with `python -m unittest discover -s scripts -p "test_codegraph_mcp.py"`.

## Pipeline and code map

| Area | Role |
| --- | --- |
| [`src/translation_phases/initial_processing.rs`](src/translation_phases/initial_processing.rs) | Defines the diagnostics of translation phases 1 and 2: a missing or escaped final newline. |
| [`src/translation_phases/preprocessor_tokenizer.rs`](src/translation_phases/preprocessor_tokenizer.rs) and [`preprocessor_tokenizer/`](src/translation_phases/preprocessor_tokenizer/) | Lexes each source buffer completely when it is opened (phases 1-3: trigraphs, line splices, comments, and preprocessing tokens) and replays its tokens to preprocessing, retaining source provenance. |
| [`src/translation_phases/preprocessing.rs`](src/translation_phases/preprocessing.rs) and [`preprocessing/`](src/translation_phases/preprocessing/) | Handles macros, directives, includes, conditional preprocessing, literals, and conversion to parser-facing tokens. It owns the preprocessor-expression evaluator and its values and diagnostics; each concern has its own submodule. |
| [`src/translation_phases/parsing.rs`](src/translation_phases/parsing.rs) and [`parsing/`](src/translation_phases/parsing/) | Contains the explicit parser driver; declaration, function-definition, statement, expression, type-name, initializer, declarator, and tag frames (one submodule per frame); syntax nodes, which live in the translation-unit arena; scopes; and parser diagnostics. |
| [`src/translation_phases.rs`](src/translation_phases.rs) | Defines the shared translation-phase interface and diagnostic plumbing; [`context.rs`](src/translation_phases/context.rs) and [`provenance.rs`](src/translation_phases/provenance.rs) hold the compilation context and source provenance. |
| [`src/util/`](src/util/) | Provides the virtual-memory regions, the arena allocator and its vectors, strings, queues, and lists, the per-compilation region vector and bit set, interned strings, and the lexer's byte scans. |
| [`src/diagnostics.rs`](src/diagnostics.rs) | Builds diagnostics (message, labelled source ranges, notes, help) and renders them as annotated source snippets. Each phase's error type explains itself through a `ToDiagnostic` implementation. |
| [`src/lib.rs`](src/lib.rs), [`src/cli.rs`](src/cli.rs), and [`src/pipeline.rs`](src/pipeline.rs) | Wire the inspection CLI to the parser by default and to the token dump with `--tokens`, create each phase's arenas in order, and report diagnostics. |

## Parser direction

The language parser uses one explicit control stack of specialized, resumable
frames. `Parser` borrows the translation context and owns the token cursor,
frame stack, typed child result, syntax-node count, scope and label state,
recovery state, and representation-based resource ceilings. Production parsing
allows as many external declarations and syntax nodes as their `usize` counters
can represent. Frame depth follows the `u32` width of stored scope depths.
There is no aggregate source-segment ceiling: each provenance arena checks its
own `u32` index space. Source-vector range starts use all 32 index bits; their
lengths use 30 bits because the top two bits identify the arena. The parser's
working state lives in the parse arena; the syntax nodes it builds are immutable
and live in the translation-unit arena.

`ExpressionFrame` owns Double-E-style operator/operand reduction alongside
`TypeNameFrame` and `InitializerFrame`, and all supported statement and
declaration expression sites contain parsed expressions. The preprocessor evaluator
retains its independent reducer. Phase 05 closes the parser through structured
diagnostics, cross-family recovery, syntax provenance, compatibility fixtures,
translation floors, and opt-in inspection. See the [project glossary](GLOSSARY.md)
for canonical terms.

## Parser roadmap

All five syntax-parser phases are complete. Phase 05 audited the full grammar
and closed cross-family recovery, provenance, inspection, phase-7 totality,
diagnostics, and translation limits.

The [language parser roadmap](parser-roadmap.md) records the completed phase
boundaries, ongoing maintenance contracts, and remaining compiler work. The
[C99 parser compliance checklist](c99-parser-compliance-checklist.md) records
grammar ownership, supported behavior, and evidence.

## Further reading

- [`CONTRIBUTING.md`](CONTRIBUTING.md) — commit-message format and hook setup.
- [`GLOSSARY.md`](GLOSSARY.md) — canonical compiler-domain and parser vocabulary.
- [`parser-roadmap.md`](parser-roadmap.md) — authoritative Phase 01–05 language-parser sequence and exit gates.
- [`c99-parser-compliance-checklist.md`](c99-parser-compliance-checklist.md) — C99 grammar, ownership, and regression evidence.
- [`tests/fixtures/diagnostics/COVERAGE.md`](tests/fixtures/diagnostics/COVERAGE.md) — diagnostic golden corpus, review criteria, and remaining output issues.
- [`.agents/AGENTS.md`](.agents/AGENTS.md) — compact operational guidance for coding agents; `.claude/CLAUDE.md` imports the same file.
- [`scripts/agentbus/README.md`](scripts/agentbus/README.md) — agentbus, the Rust crate that lets coding agents working in this repository coordinate.
- [The Double-E Method](https://erikeidt.github.io/The-Double-E-Method.html) — background for the expression reducer. Use the algorithm description as a conceptual reference; verify source licensing before copying any reference implementation.

## License

See [`LICENSE`](LICENSE).
