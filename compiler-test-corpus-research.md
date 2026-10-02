# External C compiler test corpora

Research date: 2026-09-30. This note identifies upstream corpora and the
interpretation needed for bcc-rust's C99 syntax parser. Actual local run
counts and failures belong in the accompanying execution report; no counts here
are inferred from an upstream directory listing.

## Recommended first corpus

Use the **GCC 15.2.0 release's `gcc/testsuite/gcc.c-torture/compile` and
`gcc/testsuite/gcc.c-torture/execute` directories**, including supporting headers
and harness files. GCC describes these as historical regression fragments run
under multiple optimization configurations: the former must compile, while the
latter must also link and execute. This makes them useful positive-source stress
inputs, although their upstream acceptance contract is broader than C99 syntax.
They do not provide a parser-only conformance score. [GCC C test suites](https://gcc.gnu.org/onlinedocs/gccint/C-Tests.html)

The deliberately fixed historical release is available as
[`gcc-15.2.0.tar.xz`](https://ftp.gnu.org/gnu/gcc/gcc-15.2.0/gcc-15.2.0.tar.xz),
with a [detached signature](https://ftp.gnu.org/gnu/gcc/gcc-15.2.0/gcc-15.2.0.tar.xz.sig).
The official directory lists the xz archive as 96M. GCC's release history dates
15.2 to August 8, 2025; it is a reproducible choice, not a claim to be the newest
release. [GNU release directory](https://ftp.gnu.org/gnu/gcc/gcc-15.2.0/),
[GCC release history](https://gcc.gnu.org/releases.html)

Download into ignored `target/` storage, record the archive SHA-512, and extract
the selected subtrees together with the release's license files and relevant
`gcc/testsuite` support. Keep each upstream source unchanged and identify it by
release-relative path. A release archive avoids fetching the much larger GCC
Git history. This is the recommended acquisition procedure for this repository.

## What the compilers themselves test

| Compiler/project | Upstream inputs | Upstream success means | Value for our parser |
| --- | --- | --- | --- |
| GCC | `gcc.c-torture/compile`, `execute`, and `execute/ieee` | Compile, or compile/link/run across optimization configurations; IEEE tests have floating-point assumptions | Large regression input pool; classify dialect and preprocessing separately |
| GCC | `gcc.dg`, including `gcc.dg/cpp` and `gcc.dg/noncompile` | Match feature behavior and expected diagnostics specified by DejaGnu directives | Targeted language/preprocessor and negative tests after reviewing each file's instructions |
| Clang | `clang/test/Parser`, `clang/test/Sema`, AST and CodeGen tests | Match expected diagnostics, AST patterns, or generated IR | Parser recovery and grammar examples; many files deliberately contain errors |
| LLVM test-suite | `SingleSource/Regression/C/gcc-c-torture/execute`; also other SingleSource and MultiSource programs | Compile and execute programs against reference outputs, with performance metrics | An independently adapted GCC execution corpus plus larger real programs |
| TinyCC | `tests/tcctest.c`, `tests/tests2`, `tests/pp`, VLA, ABI and library tests | Match execution/preprocessing output and integration behavior, including self-compilation | Smaller complementary corpus; inspect GNU extensions and build configuration |

The GCC rows follow its [test-suite documentation](https://gcc.gnu.org/onlinedocs/gccint/C-Tests.html).
Clang describes diagnostic verification with `-cc1 -verify`, AST checks and IR
checks in its [testing manual](https://clang.llvm.org/docs/InternalsManual.html#testing).
LLVM distinguishes single-file programs from whole applications and documents
reference outputs in the [test-suite guide](https://llvm.org/docs/TestSuiteGuide.html#structure).
TinyCC's own [test Makefile](https://raw.githubusercontent.com/TinyCC/tinycc/9db1105c32afd3dcf0c28b8186f08e63c761b2b5/tests/Makefile)
contains the listed test groups and compares `tcctest.c` output against a host
compiler reference built with `-std=gnu99`.

### LLVM's GCC mirror is useful evidence about exclusions

LLVM's adapted torture directory explicitly excludes unsupported GCC nested
functions, variable-length arrays in structures, some builtins, GNU89 inline
assumptions, undefined-behavior cases, and target-specific cases. It also adds
individual options such as `-fwrapv` where required. These are concrete reasons
why a copied torture corpus cannot be interpreted as a list of portable C99
programs. [Pinned LLVM torture harness](https://raw.githubusercontent.com/llvm/llvm-test-suite/cde6a9c353752cc4050561e2b004f5d26e9e109f/SingleSource/Regression/C/gcc-c-torture/execute/CMakeLists.txt)

For a later LLVM comparison, pin `llvm/llvm-test-suite` commit
`cde6a9c353752cc4050561e2b004f5d26e9e109f`, observed through `git ls-remote` on
2026-09-30, and select `SingleSource/Regression/C/gcc-c-torture/execute` with its
headers, CMake configuration, outputs and licenses. A filtered sparse checkout
is a practical alternative to downloading the complete repository. [Official repository](https://github.com/llvm/llvm-test-suite)

### Clang's diagnostic tests need their intended oracle

For example, pinned
[`clang/test/Parser/c99.c`](https://raw.githubusercontent.com/llvm/llvm-project/llvmorg-19.1.7/clang/test/Parser/c99.c)
has distinct C99, C89 and C++ invocations and deliberately expects an error for
`_Imaginary`. Its upstream test can pass while compiling its source emits an
error. Preserve `RUN`, `expected-error`, `expected-warning`, language options,
include fixtures and target requirements before adapting such a test. A source
file's suffix or its directory alone does not establish that our parser should
accept it. [Clang diagnostic verification](https://clang.llvm.org/docs/InternalsManual.html#verifying-diagnostics)

### TinyCC is a complementary source

The project founder's page points to `repo.or.cz/tinycc.git` as the Git
repository. Its `mob` head observed on 2026-09-30 is
`9db1105c32afd3dcf0c28b8186f08e63c761b2b5`; the source citations here use the
TinyCC GitHub mirror at that same commit. The GitHub repository identifies
itself as an unofficial mirror. [TinyCC project links](https://bellard.org/tcc/),
[mirror description](https://github.com/TinyCC/tinycc)

`tcctest.c` requires generated `config.h` and selects behavior by platform and
compiler, so it is less immediately self-contained than many GCC fragments.
Prefer reviewing the smaller `tests/tests2` cases first when adding a second
corpus. [TinyCC aggregate test source](https://raw.githubusercontent.com/TinyCC/tinycc/mob/tests/tcctest.c),
[numbered-test harness](https://raw.githubusercontent.com/TinyCC/tinycc/mob/tests/tests2/Makefile)

## Interpreting a local parser run

bcc-rust's existing [compatibility manifest](./tests/fixtures/parser/compatibility/manifest.md)
and [C99 parser checklist](./c99-parser-compliance-checklist.md) separate syntax
recognition from later semantic constraints. Keep that distinction in the
external run:

1. **Raw source run:** run bcc-rust on each original file. Record acceptance,
   diagnostics, panic/crash, elapsed time and timeout separately. This exercises
   the full input path, including bcc-rust's preprocessing.
2. **Compiler classification:** obtain GCC and Clang observations under explicit
   `-std=c99 -pedantic-errors -fsyntax-only` profiles, including compiler
   versions, targets, include paths and defines. Retain stderr rather than only
   exit codes.
3. **Parser isolation:** for suitable cases, generate a separate externally
   preprocessed input under the recorded profile and re-run bcc-rust on that
   input. Preserve the original source identity and exact preprocessing command;
   preprocessed host-header extensions may still require exclusion.
4. **Review discrepancies:** distinguish ordinary C99 syntax bugs from GNU or
   later-C dialect inputs, legacy implicit-int code, compiler builtins, missing
   includes/defines, host headers and semantic-only differences. Derive a small
   self-contained regression only after the distinction is established.

This procedure is an adaptation for our parser, not an implementation of GCC's
DejaGnu or LLVM's execution harness. GCC's own README asks for portable torture
tests and appropriate guards for target-specific cases; those guards must not
be silently discarded. [Pinned GCC tests README](https://raw.githubusercontent.com/gcc-mirror/gcc/releases/gcc-15.2.0/gcc/testsuite/README.gcc)

Two particularly important limits of the compiler oracle:

- Clang documents that `-fsyntax-only` still performs preprocessing, parsing
  **and semantic analysis**. A semantic rejection therefore does not prove a
  syntax-parser rejection is required. [Clang stage options](https://clang.llvm.org/docs/CommandGuide/clang.html#stage-selection-options)
- GCC permits alternate GNU keywords beginning and ending in double underscores
  in standard modes, and `__extension__` suppresses pedantic checks in its
  expression. Consequently, even acceptance with `-pedantic-errors` does not
  prove the input contains only C99 grammar. Inspect apparent positive-oracle
  discrepancies before assigning them to the parser. [GCC alternate keywords](https://gcc.gnu.org/onlinedocs/gcc/Alternate-Keywords.html),
  [GCC pedantic options](https://gcc.gnu.org/onlinedocs/gcc/Warning-Options.html#index-Wpedantic)

## Licenses and reproducibility

Retain upstream licenses and individual-file notices. The GCC release contains
[`COPYING3`](https://raw.githubusercontent.com/gcc-mirror/gcc/releases/gcc-15.2.0/COPYING3),
the GPLv3 text; this note does not establish one uniform license for every
historical test. Before vendoring or publishing an extracted fixture, inspect
that file's origin and notices. Downloading the corpus into ignored storage and
tracking the runner, manifest and results keeps the upstream data separate.

LLVM's test-suite license explicitly lists GCC torture as third-party content
with additional or alternative terms. The mirror is therefore not a way to
assume those tests have LLVM's Apache license. LLVM-authored material uses
Apache 2.0 with LLVM exceptions, while individual third-party exceptions still
apply. [LLVM test-suite license](https://raw.githubusercontent.com/llvm/llvm-test-suite/main/LICENSE.TXT),
[Clang/LLVM release license](https://raw.githubusercontent.com/llvm/llvm-project/llvmorg-19.1.7/LICENSE.TXT)

TinyCC's founder describes its distribution as GNU LGPL; retain the actual
chosen revision's `COPYING` and any per-file terms before redistributing tests.
[TinyCC license statement](https://bellard.org/tcc/)

The run manifest should record source URL and revision/release, archive digest,
selected directories, unmodified source paths, tool versions, exact flags,
target/header environment, parser version and timeout policy. Report the raw
corpus denominator separately from any reviewed C99 subset and from crashes or
timeouts. Do not count different optimization configurations as additional
parser source files.

## Further scale after the regression corpus

[Csmith](https://github.com/csmith-project/csmith) generates random C programs
for differential compiler testing and aims to avoid undefined behavior. It
can supply additional reproducible inputs once the fixed regression corpus is
established. Generated cases need the Csmith runtime include files, fixed seeds
and a pinned generator revision. Runtime-output comparison remains future work
for bcc-rust; at present these are syntax and robustness inputs.
