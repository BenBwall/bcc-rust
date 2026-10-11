Research date: 2026-10-11.

# Driving LLVM from bcc-rust for optimizer comparisons

## Question and short answer

bcc-rust will grow its own middle-end IR and optimizer. The owner also wants
to compile the same C programs through LLVM so the performance of LLVM's
optimizer and bcc's optimizer can be compared. The question is how bcc-rust
should drive LLVM.

Short answer: lower bcc's IR to textual LLVM IR (`.ll`) and hand it to the
bundled `clang`, first. That needs no new build step, no new link-time
dependency, and no unsafe code, and it gives a debuggable artifact on disk.
The bundled `clang` can already run "LLVM's full `-O2`" and, separately,
"LLVM's code generator at `-O2` with the middle end switched off", which are
exactly the two arms of the comparison. Add `opt` and `llc` to the
distribution only if finer control is needed. Treat in-process LLVM-C as a
later, optional stage, because on this repository's Windows GNU host it needs
LLVM libraries built for the MinGW C++ ABI, and the current LLVM build
produces MSVC-ABI libraries. A hand-written bitcode writer is not worth its
cost. `llvm-sys` 231 and `inkwell` 0.10 both support LLVM 23.1 today, but both
depend on `llvm-config` and on that same ABI question, and `inkwell` allocates
from the global allocator, which the repository forbids in compiler code.

Everything below is dated 2026-10-11 and was checked against the files in this
checkout and the primary sources linked at the end. Where a figure is an
estimate and not a measurement, it says so.

## What the repository does today

These facts come from `README.md`, `build.rs`, `build_support/llvm.rs`,
`build_support/native.rs`, `build_support/llvm.sparse`, `Cargo.toml`,
`rust-toolchain.toml`, `.cargo/config.toml` and `src/target.rs`.

- `vendor/rust` is a shallow, filtered submodule at the commit for Rust 1.99.0.
  Its `src/llvm-project` submodule pins LLVM 23.1.1
  (`1b9c0d5ff9bbe7634aead059efe6b11a7eeba145`). The sparse checkout keeps
  `llvm/`, `clang/`, `lld/`, `cmake/`, part of `libc/` and a few other
  directories, and excludes tests, docs and examples. The LLVM-C headers are
  present under `llvm/include/llvm-c`, including `Transforms/PassBuilder.h`,
  `TargetMachine.h`, `IRReader.h`, `BitWriter.h`, `Analysis.h` and `Core.h`.
  The tool sources `llvm/tools/opt`, `llc`, `llvm-as`, `llvm-dis` and
  `llvm-config` are present too.
- `build_support/llvm.rs` drives CMake with the `cmake` crate and the Ninja
  generator. It sets `LLVM_ENABLE_PROJECTS=clang;lld`,
  `LLVM_TARGETS_TO_BUILD=Native`, `LLVM_DEFAULT_TARGET_TRIPLE=<host>` and
  `LLVM_DISTRIBUTION_COMPONENTS=clang;libclang;llvm-ar;lld;clang-resource-headers`,
  then builds the `install-distribution` target into `target/llvm`. Assertions,
  RTTI and exceptions are off (confirmed in `target/llvm/build/CMakeCache.txt`).
  Parallelism is capped at 8 jobs and `LLVM_PARALLEL_LINK_JOBS=1`.
- The stamp file `target/llvm/.installed-version` holds only the LLVM version
  string. If it matches, the build skips CMake entirely. Adding distribution
  components therefore does not re-run CMake on an existing cache unless the
  stamp is changed to include the component list.
- The bootstrap compiler for LLVM on a Windows GNU host is MSVC `cl.exe` when
  Visual Studio Build Tools are found, and MinGW GCC/G++ otherwise. On this
  machine the cache was configured with `cl.exe` 14.44, host triple
  `x86_64-pc-windows-msvc`, and the installed static libraries are `LLVM*.lib`
  in MSVC format. That is fine today, because the only things taken from the
  build are executables (`clang.exe`, `ld.lld.exe`, `llvm-ar.exe`) and
  `libclang.dll`, whose C++ ABI never meets the Rust program's.
- `build_support/native.rs` uses the pinned `clang` to compile a C helper with
  `-flto=full` for cross-language LTO, and runs `bindgen` against the pinned
  `libclang`. `libclang` is loaded only by the binding generator and is not
  linked into the executable. The README promises that no LLVM, libclang, Rust,
  GCC or pthread DLL needs to accompany the executable.
- `.cargo/config.toml` forces, for Windows GNU: `-C linker=target/llvm/bin/ld.lld.exe`,
  `linker-flavor=ld.lld`, `-C linker-plugin-lto`, `-C lto=fat`,
  `target-feature=+crt-static`, `link-self-contained=yes` and a list of
  `--undefined=` flags to keep `compiler_builtins` objects alive under LTO.
- `src/target.rs` models four targets: `x86_64-unknown-linux-gnu`,
  `x86_64-unknown-linux-musl`, `x86_64-w64-windows-gnu` (also accepted as
  `x86_64-pc-windows-gnu` and `x86_64-w64-mingw32`) and
  `x86_64-pc-windows-msvc`. All are 64-bit; Windows targets use LLP64 for
  `long`, 16-bit `wchar_t`, and, for MSVC, 64-bit `long double`. A backend must
  therefore emit per-target LLVM data layouts and triples, which is true for
  every option below.
- `Cargo.toml` already has `bindgen`, `cc` and `cmake` as build dependencies.
  `.clippy.toml` bans `Box`, `Rc`, `Arc`, `String`, `Vec`, the standard
  collections, `PathBuf`, `BufWriter` and `format!` in compiler code, and the
  feature-gated `tests/allocation_count.rs` checks that compiling a translation
  unit never touches the global allocator. `Cargo.toml` denies
  `undocumented_unsafe_blocks` and forbids `unsafe_op_in_unsafe_fn`, so any FFI
  needs documented `unsafe` blocks.
- There is no code generation today: the CLI stops after semantic analysis.

## Empirical probe of the bundled clang

All commands ran on 2026-10-11 from
`C:\Users\benbw\Documents\GitRepo\bcc-rust-ir\target\llvm-probe\` (created for
this probe; the `target\llvm` junction was not touched), using
`C:\Users\benbw\Documents\GitRepo\bcc-rust\target\llvm\bin\clang.exe`
(abbreviated `$CL` below).

`$CL --version` reports `clang version 23.1.1 (https://github.com/rust-lang/llvm-project.git 1b9c0d5ff9bbe7634aead059efe6b11a7eeba145)`
and `Target: x86_64-pc-windows-gnu`.

### Does it accept textual IR and produce a running program?

`t.ll` contains exactly:

```llvm
define i32 @main() {
  ret i32 42
}
```

```sh
$CL -O0 t.ll -o t_O0.exe ; ./t_O0.exe ; echo $?     # prints 42
$CL -O2 t.ll -o t_O2.exe ; ./t_O2.exe ; echo $?     # prints 42
$CL --target=x86_64-pc-windows-gnu --sysroot=/c/Strawberry/c -fuse-ld=lld -O2 t.ll -o r_O2.exe
./r_O2.exe ; echo $?                                  # prints 42
```

Both optimization levels and the explicit repo-style flags
(`--target`, `--sysroot` for MinGW, `-fuse-ld=lld`) produced executables that
exit with 42. With no flags at all, `clang -###` shows that the driver found
Strawberry Perl's MinGW GCC 13.2.0 on `PATH`, used its `crt2.o`/`crtbegin.o`
and its GNU `ld.exe`; adding `-fuse-ld=lld` switches to the bundled
`ld.lld.exe`. The linker is therefore a driver choice, not an IR concern.

Each of these printed `warning: overriding the module target triple with
x86_64-pc-windows-gnu [-Woverride-module]` because `t.ll` had no
`target triple` line. A backend should always emit `target triple` and
`target datalayout`; the warning disappears (verified with `loop.ll`, which
sets both).

Bitcode is accepted as well: `$CL -O2 -c -emit-llvm loop.ll -o loop.bc` wrote a
valid bitcode file (magic `BC C0 DE`), and `$CL -O2 loop.bc -o loop_bc.exe`
built and ran it (exit 45).

### Can clang run only the code generator on already-optimized IR?

`loop.ll` is a deliberately naive `sum(n)` written with `alloca` slots, loads
and stores, plus `main` calling `sum(10)`.

| Command | Result |
| --- | --- |
| `$CL -O0 -S loop.ll` | 27 instructions in the assembly, fast instruction selection |
| `$CL -O2 -S loop.ll` | 19 instructions; the loop is gone (closed form), `main` returns 45 |
| `$CL -O2 -Xclang -disable-llvm-passes -S loop.ll` | 25 instructions; IR is not optimized (three `alloca` remain), but the back end still runs at the `-O2` level |
| `$CL -O2 -S -emit-llvm loop.ll` | prints the optimized IR: the loop is replaced by a closed form, `main` is `ret i32 45` |
| `$CL -O2 -Xclang -disable-llvm-passes -S -emit-llvm loop.ll` | prints the unmodified IR (only default alignments filled in by the parser) |

`-Xclang -disable-llvm-passes` (alias `-Xclang -disable-llvm-optzns`) is
defined in `clang/include/clang/Options/Options.td` with the help text "Use
together with -emit-llvm to get pristine LLVM IR from the frontend by not
running any LLVM passes at all". The BackendUtil code only builds the
optimization pipeline `if (!CodeGenOpts.DisableLLVMPasses)`. The help text
describes the `-emit-llvm` use, but the probe shows the same switch also leaves
the middle end off when emitting assembly or objects, while the code
generator keeps the `-O2` setting: the 25 versus 27 instruction counts differ
because `-O0` selects fast instruction selection and the fast register
allocator.

A second probe (`ai.ll`) shows how complete "no passes" is. A function marked
`alwaysinline` is inlined by `-O0` (which runs only the always-inliner) but is
left as a call by `-O2 -Xclang -disable-llvm-passes`. So that combination runs
no IR pass of any kind, not even the always-inliner. It is a faithful
"code generation only" mode. Because `-Xclang` is a pass-through to the
frontend's internal `cc1` options, it is stable in practice but is not a
documented user interface; re-check after each LLVM upgrade.

Observability options that work on IR input in this distribution (all checked):

- `-mllvm -print-pipeline-passes -O2 -c loop.ll` prints the complete `-O2`
  pipeline as a `-passes=` style string. Printing it records exactly what "LLVM
  default<O2>" was for this build.
- `-mllvm -print-after-all` printed 436 "IR Dump After" sections for the small
  module at `-O2`.
- `-ftime-report` and `-mllvm -time-passes` report per-phase times, including
  "LLVM IR Parsing".
- `-Rpass=.` reports remarks such as "Loop deleted because it is invariant" and
  "'sum' inlined into 'main'".
- `-mllvm -verify-each` and `-fno-experimental-new-pass-manager` are rejected;
  `-fpass-plugin` is not usable because `CLANG_PLUGIN_SUPPORT` is `OFF` in the
  build. There is no way to give clang an arbitrary `-passes=` pipeline. That
  needs `opt`.

### Does it diagnose bad IR?

- `ret i64 42` in an `i32` function fails with a line and column caret
  diagnostic (`error: value doesn't match function result type 'i32'`).
- A missing terminator and a `phi` that names an unknown block both fail at
  parse time with a located error.
- A module that parses but violates a verifier rule (`dom.ll`, a use not
  dominated by its definition) is rejected with `error: invalid LLVM IR input:
  Instruction does not dominate all uses!`, even though the driver passes
  `-disable-llvm-verifier` to `cc1`. So textual IR gets a free verifier run,
  which makes it a good first oracle while the lowering code is young.

### Timing

Measurements on this machine (24 logical processors, Windows 11, Defender
active), minimum of several runs, wall clock including process start:

| Invocation | Time |
| --- | --- |
| `clang --version` | about 370 ms |
| `-O0 -c t.ll` (one function) | about 380 ms |
| `-O2 -c loop.ll` | about 420 ms |

So roughly 0.37 s per invocation is process start-up, loading a 75 MB
executable, and is a floor for any per-file out-of-process design. Subtract it
or batch many functions per file when timing.

A synthetic module `big.ll` has 20,000 copies of the naive loop function
(540,005 lines, 11.1 MB of text; `big.bc` is 5.5 MB):

| Invocation | Time |
| --- | --- |
| `-O0 -c big.ll` | 4.81 s |
| `-O2 -Xclang -disable-llvm-passes -c big.ll` (back end at -O2, no middle end) | 13.76 s |
| `-O2 -c big.ll` (full pipeline) | 34.35 s |
| `-O2 -emit-llvm -Xclang -disable-llvm-passes -c big.ll -o big.bc` (parse and write only) | 2.87 s |
| `-O0 -c big.bc` | 2.29 s |
| `-O2 -Xclang -disable-llvm-passes -c big.bc` | 12.22 s |

Comparing `-O0 -c big.ll` (4.81 s) with `-O0 -c big.bc` (2.29 s) puts the cost
of parsing 11 MB of textual IR at about 2.5 s, roughly 0.23 s per megabyte or
7 percent of the 34 s full `-O2` run. Textual IR is not free, but it is small
beside optimization and code generation, and it is the same for both arms of a
comparison, so it does not bias the comparison. It matters only if the goal
becomes measuring bcc's compile time end to end.

## The four options

### A. Textual LLVM IR into the bundled clang (or into opt and llc)

What it is: a lowering pass from bcc's IR to LLVM IR text, written to a file by
a small printer, then `clang -O2 -c x.ll` or the full driver command to link.
The LLVM Language Reference defines the syntax; the text is stable enough in
practice but, as the Developer Policy says, textual IR "is not backwards
compatible" across releases without specific promises, which is acceptable
because the toolchain is pinned to one LLVM version.

Effort to a first prototype: lowest. The printer needs types, constants,
globals, function definitions, basic blocks, and a few dozen instructions
(`alloca`, `load`, `store`, `add` and friends with `nsw`/`nuw` flags,
`icmp`/`fcmp`, `br`, `ret`, `call`, `getelementptr`, `phi`, casts). With
opaque pointers (`ptr`) types are simple. Allocation-free output is natural:
write into an arena buffer with `core::fmt::Write`; no `String`, `format!` or
`BufWriter` is needed, so `.clippy.toml` is respected. The probe above is the
prototype of the back end: `define i32 @main() { ret i32 42 }` already links
and runs.

Build impact: none. `clang`, `lld` and the target headers are already built.
The data layout string and triple per `Target` can be copied from
`clang -### -S -emit-llvm` output on a trivial C file for each target (the
pinned clang is the reference the repository already uses for target macros).

Compile-time overhead: the text round trip costs about 0.23 s per MB parsed
(measured above) plus 0.37 s process start-up per `clang` invocation on this
host. Neither affects the generated code.

Fair-comparison support: very good, because clang is a complete and the same
code generator for both arms:

1. "LLVM arm": bcc's front end plus a straight lowering (no bcc optimization)
   to naive IR (allocas for locals is the easy choice, as in Clang at `-O0`),
   then `clang -O2 x.ll`.
2. "bcc arm": bcc's front end, bcc IR, bcc optimizer, lowering of the optimized
   IR, then `clang -O2 -Xclang -disable-llvm-passes x.ll`. The code generator
   level is `-O2` in both arms, so selection, scheduling, register allocation
   and machine-level passes are identical, and only the middle end differs.
3. Reference points that cost nothing extra: `clang -O2 foo.c` on the original
   C source (shows how far both arms are from Clang's own IR), and
   `clang -O0 -Xclang -disable-llvm-passes` or plain `-O0` for a floor.

One subtlety: the lowering must emit the same attributes in both arms
(`nsw`, `noundef`, `nonnull`, TBAA and so on), or LLVM will have unequal
information. Metadata and attributes that bcc's optimizer does not understand
should be passed through identically.

Debuggability: very good. The `.ll` file is the exact input; `-S -emit-llvm`
shows what LLVM did; `-mllvm -print-after-all`, `-print-pipeline-passes`,
`-Rpass=.` and `-ftime-report` all work; the IR verifier gives a located
message for malformed IR.

Fit with the build: nothing to add. For finer control add tools to the
distribution:

- `opt` allows `-passes='default<O2>'` and arbitrary pipelines (for example
  `-passes='function(instcombine,gvn,simplifycfg)'`). The New Pass Manager
  documentation describes the pipeline syntax and the nesting adaptors.
- `llc -O2 -filetype=obj` runs the back end on `.ll` or `.bc` input, with
  options to control the code generator separately from the IR optimizer.
- `llvm-as` and `llvm-dis` convert between `.ll` and `.bc`.

These tools are built from libraries that the Clang build already compiles. The
ninja log of the existing build shows the LLVM libraries a code generator needs
account for roughly 50 CPU-minutes of the 131 CPU-minutes recorded, and the
final links of `clang.exe` took 4 to 9 seconds each. Adding `opt`, `llc`,
`llvm-as` and `llvm-dis` therefore should cost a few minutes of extra compile
and link time (an estimate: those four tools were not built here), and each
resulting executable would be tens of megabytes. Dist component names are the
tool names: add `opt;llc;llvm-as;llvm-dis` to `LLVM_DISTRIBUTION_COMPONENTS`.
`llc` expects `LLVMMIRParser`, which was not among the libraries built today, so
a small amount of extra compilation is certain.

Risks: per-process overhead; two textual formats to keep in step (bcc's own
dump and the LLVM dump), but a dump of bcc IR is wanted anyway; the `.ll`
format needs care with escapes in string constants and with names that need
quoting.

### B. Writing LLVM bitcode directly

The format is documented ("LLVM Bitcode File Format"): a magic number
`'B' 'C' 0xC0 0xDE`, a self-describing bitstream of abbreviations and nested
blocks, VBR integers, and an `IDENTIFICATION_BLOCK`, one `MODULE_BLOCK`
(type table, attribute groups, global variables and functions that point into a
`STRTAB`, constants, per-function blocks, metadata, and a symbol table).
The page documents the container and the block IDs but does not give a
checklist of everything a valid module needs, so a writer must follow
`BitcodeWriter.cpp` and `LLVMBitCodes.h` from the pinned source.

Effort to a first prototype: several times that of A. A minimal writer for the
subset above needs the bitstream writer (fixed, VBR, char6, abbreviations,
block lengths backpatched), a type table with consistent IDs, value
numbering with relative operand encoding and forward references for `phi`, the
string table and symbol table, attribute groups, and the identification and
version records. A mistake produces a file that LLVM rejects or, worse,
mis-reads.

Benefits: smaller (5.5 MB against 11.1 MB for the synthetic module) and faster to
read than text (about 2.5 s saved for the 20,000-function module, measured
above). LLVM's own Developer Policy
promises that current LLVM loads bitcode back to version 3.0, so bitcode ages
better than text. But the parse time is a small part of a compilation, and the
format is a worse debugging artifact: nothing in the current distribution reads
it (`llvm-dis` and `llvm-bcanalyzer` would have to be added).

Verdict: not recommended as a hand-written feature. If a bitcode file is wanted
(for example to cut the parse cost during repeated benchmarking, or to use
`clang -flto`/`lld` link-time optimization), obtain it from `clang -c
-emit-llvm x.ll -o x.bc`, which already works, or from LLVM-C
(`LLVMWriteBitcodeToFile`) if option C is adopted.

### C. Link LLVM and call the LLVM-C API through bindgen

What it is: build LLVM's static libraries, run bindgen over `llvm-c/*.h`,
construct a module in memory through `LLVMBuildAdd` and the other builder
calls (or parse text with `LLVMParseIRInContext2`), optionally verify with
`LLVMVerifyModule`, run `LLVMRunPasses`, and emit with
`LLVMTargetMachineEmitToFile` or `LLVMTargetMachineEmitToMemoryBuffer`.

The new pass manager C API exists in 23.1.1:
`llvm-c/Transforms/PassBuilder.h` declares
`LLVMRunPasses(LLVMModuleRef M, const char *Passes, LLVMTargetMachineRef TM, LLVMPassBuilderOptionsRef Options)`
whose comment says the string has "the same" format as `opt -passes` and that
"Full pipelines may also be invoked using `default<O3>` and friends". Options
include `LLVMPassBuilderOptionsSetVerifyEach`, `SetDebugLogging`,
`SetLoopVectorization`, `SetSLPVectorization`, `SetLoopUnrolling`,
`SetLoopInterleaving` and `SetAAPipeline`. `LLVMCreateTargetMachineWithOptions`
and `LLVMCodeGenOptLevel` (`None`, `Less`, `Default`, `Aggressive`) set the
back end level. So the "codegen only" arm is `LLVMRunPasses(M, "", ...)` (or no
call at all) with `LLVMCodeGenLevelDefault` or `Aggressive`; the "LLVM arm" is
`LLVMRunPasses(M, "default<O2>", ...)` with the same target machine.

What the distribution build would need:

1. The static libraries. Component names equal CMake target names. The umbrella
   component `llvm-libraries` installs all libraries (about 426 MB of
   `LLVM*.lib` exist in the existing build tree; the exact set needed is
   smaller). The set a code generator needs can be read from `opt` and `llc`'s
   `LLVM_LINK_COMPONENTS` (`Analysis`, `AsmParser`, `BitWriter`, `CodeGen`,
   `Core`, `Coroutines`, `Extensions`, `IPO`, `IRReader`, `IRPrinter`,
   `InstCombine`, `Instrumentation`, `MC`, `ObjCARCOpts`, `Passes`, `Remarks`,
   `ScalarOpts`, `Support`, `Target`, `TargetParser`, `TransformUtils`,
   `Vectorize`, plus the X86 target libraries, `AggressiveInstCombine`,
   `CFGuard`). `llvm-config --link-static --libs ...` computes the transitive
   closure, so add the `llvm-config` component as well, together with
   `llvm-headers` and `cmake-exports`.
2. The headers: `llvm-c` plus `llvm/Config/llvm-config.h` (included by
   `llvm-c/Visibility.h`).
3. The first link problem is ABI, not components, on this host (see below).

Sizes and times: all these libraries are already compiled as part of the
Clang build, so adding them to the distribution costs install time and disk
(hundreds of MB under `target/llvm/lib`) and almost no compilation. Final link
size is an estimate, not a measurement: `clang.exe` is 75 MB and contains these
libraries plus the whole Clang front end, so a bcc executable that links only
the code-generation closure and uses `--gc-sections` is plausibly 20 to 45 MB
larger than today's, with most of that being the X86 back end and the
optimizer. Link time with lld is small (the existing 75 MB `clang.exe` links in
4 to 9 seconds), but because the repository uses fat LTO for Rust and
`link-self-contained`, the final link of bcc is already the longest step of an
incremental build and the new libraries would be added to every link, test
binary and bench, unless the dependency is put behind an optional Cargo feature
or a separate crate.

The ABI problem on Windows GNU. `target/llvm/build/CMakeCache.txt` shows that
the local LLVM was compiled with MSVC (`cl.exe`, `/O2 /Ob2`, host
`x86_64-pc-windows-msvc`, `LLVM*.lib` archives). The Rust program is built for
`x86_64-pc-windows-gnu` with `ld.lld` in GNU mode, `+crt-static`, and
`link-self-contained=yes`, which uses the MinGW runtime that Rust ships.
MSVC-compiled C++ libraries need the Microsoft C++ runtime
(`msvcprt`/`vcruntime`/UCRT, by default the DLL runtime), whose headers and
import libraries are not part of that link, and whose symbols and exception and
RTTI conventions differ from MinGW. So the existing libraries cannot be reused,
and shipping them would also break the README's promise that no extra DLL
accompanies the executable. This does not affect option A, because `clang`
runs as a separate process.

There are two ways out, both with a real cost:

- Build a second, library-only LLVM with a MinGW-ABI compiler: either MinGW
  `g++` (the README says it can bootstrap LLVM) or the bundled `clang++ --target=x86_64-w64-windows-gnu`
  with MinGW's `libstdc++`. `rustc` itself links LLVM this way on `windows-gnu`:
  `compiler/rustc_llvm/build.rs` links `stdc++` (or a static libstdc++ when
  `LLVM_STATIC_STDCPP` is set), `pthread`, `shell32` and `uuid`, and uses
  `llvm-config --link-static --libs`. That is a proven recipe, but it means
  that the C++ standard library and its static archive (`libstdc++.a`) come
  from the MinGW installation, which the repository already requires for
  headers. The CPU cost is about the 50 CPU-minutes estimated above for the
  needed subset (around 6 to 10 minutes of wall time on this machine at the
  8-job cap; an estimate), once per cache, and it must be done separately on
  Linux (glibc, musl) and MSVC hosts, where the correct C++ runtime
  differs again.
- Use `LLVM-C.dll`. On MSVC builds LLVM already defaults
  `LLVM_BUILD_LLVM_C_DYLIB` to `ON` (the line in `llvm/CMakeLists.txt` is
  guarded by `if(MSVC)`, and the existing cache has it `ON`). The DLL exports
  only the C API, which has a plain C ABI, so a MinGW program can link its
  import library. This avoids a second LLVM build but requires shipping a
  large DLL next to the executable, contradicting the self-contained
  executable goal. It could be acceptable for a benchmark-only build; I did not
  test whether the `LLVM-C` target links in this configuration.

Other points for C:

- bindgen is already a build dependency, but `llvm-c/Target.h` defines
  `LLVMInitializeNativeTarget`, `LLVMInitializeAllTargets` and friends as
  `static inline` functions; bindgen only emits those with `wrap_static_fns`
  (which needs a compiled C file, and the pinned clang can compile it) or one
  calls `LLVMInitializeX86Target`, `...TargetInfo`, `...TargetMC` and
  `...AsmPrinter` directly. (`llvm-sys` solves this with a small C wrapper,
  `wrappers/target.c`.)
- `LLVM_C_ABI` expands to nothing on MinGW (`!defined(__MINGW32__)` guards the
  `dllimport` branch), so bindgen sees plain declarations.
- The LLVM Developer Policy calls C API stability "best effort", with
  higher-level operations "expected to be less stable". The pinned LLVM version
  makes this moot until the next Rust upgrade.
- Unsafe: every call is `unsafe`. The repository denies undocumented unsafe
  blocks, so a thin safe module with documented invariants is needed. LLVM
  allocates from the C++ heap, not the Rust global allocator, so the
  `allocation_count` test would still pass, but that boundary should be stated
  in the module documentation. LLVM objects need `Drop`-like disposal calls;
  the arena rule "values must not need `Drop`" means the LLVM context and
  module should live in a scope-guarded owner outside arena values.
- A debug aid: `LLVMPrintModuleToString` gives the IR as text, and
  `LLVMVerifyModule` gives verifier errors with a message, so debuggability is
  good but one step removed from the compiler's lowering, because there is no
  file unless one writes it.

Compile-time overhead: lowest of all. No text, no process spawn, and the
compilation can be timed inside the process. LLVM's own `-time-passes` is also
available through the pass builder options.

Fair-comparison support: also excellent, and cleaner: one process holds the
same `TargetMachine` and `Context` for both arms, `LLVMRunPasses` with
`"default<O2>"` or an empty or restricted pipeline gives the arms, and
`LLVMPassBuilderOptionsSetVerifyEach` can validate every pass. The ability to
run `LLVMRunPasses` with a custom pipeline string replaces `opt`.

Effort to a first prototype: high on this host. The lowering code is about the
same size as A's printer (builder calls in place of formatted writes), but
before it can start the second LLVM library build, the distribution change,
the stamp-key change, feature gating and the unsafe module must exist. I would
budget days rather than hours, and most of the risk is in the build, not in the
code.

### D. Existing crates: llvm-sys and inkwell

Versions (crates.io API, queried 2026-10-11):

- `llvm-sys` 231.0.1 was published 2026-10-09, together with new point
  releases for older LLVM lines (221.1.1, 211.1.1, 201.1.1, 191.1.1, 181.3.1,
  170.4.1, 160.2.2). Its README states that the crate version tracks LLVM:
  "llvm-sys version 191 is compatible with LLVM 19.1.x", so 231 targets LLVM
  23.1.x. The build accepts any LLVM at least as new as the target unless
  `strict-versioning` is on, which would make an exact match to 23.1.1 a good
  setting here.
- `inkwell` 0.10.0 (published 2026-08-06) advertises "One of LLVM 12-23" and
  feature flags such as `llvm23-1`, which enables `llvm-sys-231`. Its
  `Cargo.toml` makes `target-all` a default feature, so with
  `LLVM_TARGETS_TO_BUILD=Native` one must use
  `default-features = false, features = ["llvm23-1", "target-x86"]`. The README
  says the minimum rustc is 1.85; edition 2024 is used; the repository's 1.99.0
  is newer than both.

How `llvm-sys` finds and links LLVM (its README and `build.rs`): it requires a
working `llvm-config`, found through `LLVM_SYS_231_PREFIX/bin` or `PATH`. It runs
`llvm-config --libnames --link-static` (`prefer-static` is the default; features
`force-static`, `prefer-dynamic`, `force-dynamic`, `no-llvm-linking`,
`disable-alltargets-init` exist) and `--system-libs`, then emits
`cargo:rustc-link-lib=static=...`. It compiles `wrappers/target.c` with `cc`.
The C++ standard library comes from `CXXSTDLIB`/`LLVM_SYS_LIBCPP`, or defaults
to none for MSVC and `stdc++` otherwise. Dynamic linking on Windows MSVC is
rejected. The README adds that on Windows "the Rust toolchain must use the same
compiler as the LLVM build, either MSVC or MinGW", which is the ABI constraint
found above.

Fit with this repository:

- The `llvm-config` executable must be added to the distribution (component
  `llvm-config`); today's install lacks it. (The `llvm-sys` README also warns
  that binary LLVM distributions usually omit `llvm-config`.)
- `llvm-sys` needs libraries built with the right C++ ABI, which brings back
  the second-LLVM-build problem. `llvm-sys` links exactly the libraries
  `llvm-config` reports, so the distribution would need all of their
  components.
- `LLVM_SYS_231_PREFIX` could point at `target/llvm`, but the sequencing is
  awkward: the LLVM build is a side effect of this package's own build script,
  and Cargo does not order a dependency's build script after it. `llvm-sys`
  would see an empty or stale prefix on a clean checkout. A separate workspace
  member that is built after the first `cargo build` populated the prefix
  avoids this.
- `inkwell` is a safe wrapper that owns Rust values in `Vec`, `String`, `Rc`
  and similar global-allocator types, and returns `String`/`CString`. The repo
  forbids those in compiler code, but this would be a dependency, not compiler
  code; the clippy lists apply to this crate's own source. Still, the code
  that calls it would need `expect` attributes for the disallowed types, and
  its lifetime-heavy `Context<'ctx>` API would have to be adapted to arenas.
  It also pulls `thiserror` 2 (already present) and `libc`. Since inkwell
  is a thin layer over the same LLVM-C API, its main value is saving the bindgen
  and the safe-wrapper work.
- Using `llvm-sys` directly avoids the extra layer and gives the same C API
  that option C describes, but it adds a second binding generator beside
  the repo's own bindgen run. A custom bindgen pass over `llvm-c` headers is
  small, version-exact by construction, and reuses the `bindgen::clang_version()`
  check that already pins versions. The one thing `llvm-sys` provides that
  bindgen alone does not is the `static inline` target initialization wrapper
  and the `llvm-config`-driven library list.

Verdict: both crates work with LLVM 23.1 in principle and are good if option C
is chosen later, but they do not remove its main costs (the MinGW-ABI library
build and `llvm-config`), and they add dependencies. Prefer plain bindgen over
the headers from `vendor/rust/src/llvm-project/llvm/include/llvm-c` unless the
safe wrapper from `inkwell` is specifically wanted. I did not build either
crate against this LLVM.

## Comparison summary

| | A. Text to clang/opt/llc | B. Handwritten bitcode | C. LLVM-C via bindgen | D. llvm-sys/inkwell |
| --- | --- | --- | --- | --- |
| Time to first prototype | hours; works today | days | days (build first) | days (build first) |
| New build work | none (optional: add `opt`, `llc`, `llvm-as`, `llvm-dis`) | none | second MinGW-ABI LLVM library build, or LLVM-C.dll; distribution components; stamp change | same as C, plus `llvm-config` |
| New unsafe | none | none | yes, FFI | yes (llvm-sys) or none in the wrapper (inkwell) |
| Global-allocator use | none | none | none in our code; LLVM uses the C++ heap | inkwell uses `Vec`/`String`/`Rc` |
| Per-run overhead | about 0.37 s per process plus about 0.23 s per MB of text (measured) | process spawn only; no parse cost | none | none |
| Fair "same back end" comparison | yes: `-O2 -Xclang -disable-llvm-passes` vs `-O2` | yes, same | yes, `LLVMRunPasses` with an empty pipeline vs `default<O2>` | yes, same |
| Custom pass pipelines | needs `opt` | needs `opt` | `LLVMRunPasses` strings | `LLVMRunPasses` strings |
| Debuggability | best: file on disk, verifier errors with line numbers | poor without `llvm-dis` | good via `LLVMPrintModuleToString` and `LLVMVerifyModule` | same |
| Risk | text format changes at upgrade | high: silent miscompile or reject | medium: ABI, link size and time | medium: as C, plus dependency churn |

## Designing the comparison itself

The question the owner wants answered is whether bcc's optimizer produces code
as fast as LLVM's. That needs the same inputs, front end and code generator on
both sides, and only the middle end different.

- Arm L (LLVM middle end): bcc front end, unoptimized bcc lowering to LLVM IR,
  LLVM `default<O2>` (or `O3`), LLVM code generator at `-O2`.
- Arm B (bcc middle end): bcc front end, bcc IR optimizer, lowering to LLVM IR,
  no LLVM middle-end passes, LLVM code generator at the same `-O2`.
- Floor: bcc lowering with LLVM `-O0` (or `-O2 -Xclang -disable-llvm-passes`
  without bcc optimization) to show what the optimizers add.
- Ceiling reference: `clang -O2 foo.c` on the same C source, to show how much
  of Clang's own advantage comes from information the bcc lowering throws away
  (types, TBAA, `inbounds`, `nsw`, `noundef`, `restrict` as `noalias`,
  `llvm.assume`, lifetime markers).

Things that make the comparison unfair if ignored:

1. Both arms must emit identical attributes and flags for the same source
   constructs, or LLVM sees different information. In particular C signed
   overflow is undefined, so `nsw`/`nuw` on arithmetic and `inbounds` on
   `getelementptr` should be present in the lowering, and `restrict` should
   become `noalias`. An optimizer cannot use facts it was never told.
2. Do not let the code generator vary. Pin `-O2` code generation and the same
   `-mcpu` in both arms. The probe showed the back end at `-O0` selects
   different instructions, so `-O0 -Xclang ...` is not comparable.
3. Link the same way (same runtime library and linker). The repository already
   fixes `lld` and the MinGW sysroot, so use `-fuse-ld=lld --sysroot=<MinGW>`.
4. Verify behaviour first: compare program output across arms before timing.
   The IR verifier in clang (shown to be active on textual input) catches
   malformed lowering early.
5. For timing the compiler and the generated program, separate the 0.37 s
   process start from the compile work, and run the generated code many times.
6. Keep the LLVM version fixed to 23.1.1; the build already asserts that
   `clang`, `libclang` and `llvm-ar` match rustc's version.

## Recommended staged path

Stage 1 (now): textual IR into the bundled clang.

- Add an IR-to-LLVM text printer to the new IR crate or module. Emit `target
  triple` and `target datalayout` for each `Target`; write through an arena
  buffer with `core::fmt::Write`.
- Add driver flags equal to the experiment: `--emit-llvm-ir`, then a script
  (see `scripts/`) that runs the arms in the table above. Record the clang
  command lines and the `-print-pipeline-passes` output with the results.
- Use `clang -O2 -Xclang -disable-llvm-passes` for the bcc arm and `clang -O2`
  for the LLVM arm. Run `clang -S -emit-llvm` on each to confirm that the IR
  did, or did not, change.
- Reasons: it works today (probe above), costs no build time, keeps compiler
  code free of `unsafe` and of the C++ ABI problem, gives the best diagnostics
  while the lowering is young, and the output file is the bug report.

Stage 2 (when finer control is wanted): add `opt;llc;llvm-as;llvm-dis` (and
possibly `llvm-link`, `llvm-bcanalyzer`) to `LLVM_DISTRIBUTION_COMPONENTS`, and
change the `.installed-version` stamp to include a hash of the component list
so existing caches rebuild. With `opt -passes=...` you can run individual LLVM
passes after bcc's optimizer, or LLVM passes that bcc lacks, to find which LLVM
transformation accounts for a gap; with `llc` you can vary the code generator.
This is a build change of a few minutes, not a new dependency.

Stage 3 (only if profiling shows the process and text cost matter, or if bcc
needs to be a self-contained benchmark harness): LLVM-C behind an optional
Cargo feature or a separate workspace crate, so ordinary `cargo build` and
`cargo test` do not link LLVM. Required work: library-only LLVM build with a
MinGW-ABI C++ compiler on Windows GNU (or the `LLVM-C.dll` shortcut for
benchmark builds), distribution components for the libraries, headers and
`llvm-config`, a bindgen invocation over `llvm-c`, a safe wrapper module, and a
text-versus-builder equivalence test that checks both paths produce the same
object code. Keep the textual printer: it remains the oracle and the debugging
format.

Not recommended: a handwritten bitcode writer (B). Not needed for now:
`inkwell` and `llvm-sys` (D), although `llvm-sys` 231 and `inkwell` 0.10 with
`llvm23-1` would be a reasonable shortcut for Stage 3 if its `llvm-config`
requirement is acceptable.

## Open questions and things not verified

- The needed size of the LLVM library set, link time and executable-size
  increase for option C are estimates; no LLVM static libraries are installed
  and nothing was linked.
- `LLVM-C.dll` and the MinGW-ABI library build were not attempted. The
  assertion that the MSVC-ABI libraries cannot be linked into the GNU
  executable follows from the toolchain configuration and from the
  `llvm-sys` README, but was not tried.
- Whether `-Xclang -disable-llvm-passes` stays a faithful "no middle end" switch
  in future LLVM releases should be rechecked by the `alwaysinline` probe.
- The probe used Strawberry Perl's MinGW (GCC 13.2.0) as the sysroot because
  that is what `gcc.exe` on `PATH` resolves to; the repository's
  `MINGW_ROOT` logic would find the same one.
- No benchmark of generated code was run; the comparison design is a plan.
- On Linux and MSVC the same commands should work but were not exercised.

## Sources

Repository files (this checkout): `README.md` (Prerequisites, "Native C, static
linking, and cross-language LTO"), `build.rs`, `build_support/llvm.rs`,
`build_support/native.rs`, `build_support/llvm.sparse`, `Cargo.toml`,
`rust-toolchain.toml`, `.cargo/config.toml`, `.clippy.toml`, `src/target.rs`.

LLVM sources at the pinned revision, `vendor/rust/src/llvm-project`:
`llvm/include/llvm-c/Transforms/PassBuilder.h`, `TargetMachine.h`,
`IRReader.h`, `BitWriter.h`, `Analysis.h`, `Target.h`, `Visibility.h`;
`llvm/CMakeLists.txt` (`LLVM_BUILD_LLVM_C_DYLIB`, `llvm-libraries`,
`llvm-headers`); `llvm/tools/opt/CMakeLists.txt`;
`llvm/tools/llvm-shlib/CMakeLists.txt`;
`clang/include/clang/Options/Options.td` (`disable_llvm_passes`);
`clang/lib/CodeGen/BackendUtil.cpp`.

LLVM documentation:

- LLVM Language Reference Manual: https://llvm.org/docs/LangRef.html
- LLVM Bitcode File Format: https://llvm.org/docs/BitCodeFormat.html
- LLVM New Pass Manager and Optimization Pipelines: https://llvm.org/docs/NewPassManager.html
- llc command guide: https://llvm.org/docs/CommandGuide/llc.html
- opt command guide: https://llvm.org/docs/CommandGuide/opt.html
- Building a Distribution of LLVM: https://llvm.org/docs/BuildingADistribution.html
- LLVM Developer Policy (C API changes; IR backwards compatibility): https://llvm.org/docs/DeveloperPolicy.html
- LLVM-C headers on GitHub: https://github.com/llvm/llvm-project/tree/main/llvm/include/llvm-c

Crates and repositories:

- llvm-sys on crates.io: https://crates.io/crates/llvm-sys, README and `build.rs`
  at https://gitlab.com/taricorp/llvm-sys.rs, docs at https://docs.rs/llvm-sys
- inkwell: https://github.com/TheDan64/inkwell, https://crates.io/crates/inkwell,
  https://docs.rs/inkwell
- rustc's LLVM build script (windows-gnu libstdc++ handling):
  https://github.com/rust-lang/rust/blob/1.99.0/compiler/rustc_llvm/build.rs
- Rust LTO documentation: https://doc.rust-lang.org/rustc/linker-plugin-lto.html

Probe artifacts, all under `C:\Users\benbw\Documents\GitRepo\bcc-rust-ir\target\llvm-probe\`
(ignored by git): `t.ll`, `loop.ll`, `ai.ll`, `dom.ll`, `bad*.ll`, `big.ll`.
