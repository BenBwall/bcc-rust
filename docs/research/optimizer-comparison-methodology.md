Research date: 2026-10-11.

# Comparing bcc-rust's optimizer with LLVM's: methodology, oracles, corpus, harness

This note answers how to compare a future bcc-rust middle-end optimizer with LLVM's optimizer fairly and usefully, and what corpus
and harness to use. It builds on [the earlier corpus note](compiler-test-corpus-research.md), which already covers the GCC c-torture
suites and pins an llvm-test-suite commit (`cde6a9c353752cc4050561e2b004f5d26e9e109f`); that material is not repeated. Statements
that are my own inference rather than something a source says are marked "inference". Compiler-internals facts were read in the LLVM
19.1.7 tag because it is a stable, linkable revision; this repository pins LLVM 23.1.1 ([README
prerequisites](../../README.md#prerequisites)), so every option named here must be re-checked with `--help` on the pinned build
before it goes into a script.

## Summary of the recommendations

1. Treat "bcc optimizer versus LLVM optimizer" as two separate questions. The product question asks how far bcc's whole pipeline is
   from LLVM `default<O2>`/`default<O3>`. The pass question asks whether each bcc pass matches the LLVM pass that does the same job.
   Each needs its own arm (section 1.3).
2. Hold the front end, the lowering to LLVM IR, the code generator, the target and the linker fixed, and vary only the IR-to-IR
   stage. `llc` is a suitable constant code generator, but it is not optimization-free, so the baseline "no optimizer" arm still
   gets some IR-level work (section 1.2).
3. Use the benchmark suites that need almost no libc at run time and verify their own output: Embench-IoT first, then a Polybench/C
   subset, then the small llvm-test-suite programs that ship `reference_output` files. Use GCC c-torture `execute` as a correctness
   gate, not as a benchmark (section 3).
4. Use random generators (YARPGen, Csmith) only for correctness. They find wrong-code bugs, they do not measure speed. Back them
   with an IR interpreter, `clang -fsanitize=undefined` for corpus hygiene, C-Reduce and `llvm-reduce` for minimization, and Alive2
   for per-pass translation validation through the LLVM-IR lowering (section 2).
5. Report geometric means of per-benchmark ratios with a bootstrap interval, take at least seven interleaved repetitions per cell,
   keep the minimum and the median, and vary build layout, because repetition cannot remove layout bias (section 4).
6. Build the harness around an explicit, ordered, named pass list on the bcc optimizer's command line, with per-pass timers, a
   global transformation counter that can stop optimization at the Nth transformation, and a deterministic IR-interpreter
   instruction count as a noise-free metric (section 5).

## 1. Experimental design

### 1.1 What must be held constant

The comparison is about the middle end, so everything before and after it must be identical or its effect must be measured
separately. The stages are the bcc front end (preprocessing, parsing, semantic analysis), bcc IR construction, the IR optimizer
under test, the lowering from bcc IR to LLVM IR, LLVM's own IR optimizer when it is the arm under test, LLVM code generation, and
the linker with its C library.

Three families of confounder follow from this decomposition.

- **Information the lowering passes on.** LLVM's optimizer is only as strong as the facts in the IR it receives: `nsw`/`nuw`,
  `inbounds`, TBAA and other alias metadata, `noalias`, alignment, lifetime markers, loop metadata and fast-math flags. Clang does
  not emit all of these at `-O0`; for example its `-disable-lifetime-markers` help text says it disables lifetime markers "even when
  optimizations are enabled" ([clang Options.td,
  19.1.7](https://github.com/llvm/llvm-project/blob/llvmorg-19.1.7/clang/include/clang/Driver/Options.td)), which shows that they
  are emitted when optimizing. The meaning of the poison-generating flags is defined by the [LLVM Language
  Reference](https://llvm.org/docs/LangRef.html). Define named lowering profiles and report results under each: a conservative
  profile that emits only what C99 semantics license (inference: signed overflow is undefined in C99, so `nsw` on signed arithmetic
  is licensed, but only where bcc's semantic analysis knows the type is signed), and a clang-equivalent profile calibrated by
  diffing bcc's lowered IR against `clang -O2 -Xclang -disable-llvm-passes -S -emit-llvm` for the same translation unit. The
  difference between the two profiles measures how much of LLVM's advantage comes from metadata rather than from passes.
- **Shape of the incoming IR.** Clang at `-O0` keeps locals in `alloca`s and leaves promotion to `mem2reg`/`sroa`, which LLVM
  documents as promoting allocas to SSA registers ([LLVM Passes](https://llvm.org/docs/Passes.html)). If bcc IR is already SSA, the
  "no optimizer" arm is cleaner than clang's `-O0` and LLVM's first passes have less to do. Record the form and compare like with
  like (inference).
- **Front-end differences that are not the optimizer.** Clang may lower some library calls to intrinsics, handles `errno` for math
  functions, and applies its own folding. Compare against `clang -O2` only as a reference point, not as a measurement of the
  optimizer (section 1.2, arm R).

### 1.2 LLVM mechanics that make "codegen only" possible

`opt` and `llc` separate the optimizer from the code generator, and the options below build the arms.

- `opt -passes='<pipeline>'` runs an explicit pass pipeline, for example `-passes="sroa,instcombine"`
  ([opt](https://llvm.org/docs/CommandGuide/opt.html)). The textual pipeline `default<O2>` is what `opt -O2` expands to: the 19.1.7
  driver documents `-O2` as "Same as -passes=\"default<O2>\"" and constructs that string
  ([optdriver.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-19.1.7/llvm/tools/opt/optdriver.cpp)), and `PassBuilder`
  accepts `default<O0>` through `default<Oz>`
  ([PassBuilder.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-19.1.7/llvm/lib/Passes/PassBuilder.cpp)). The same page
  documents `-S` (text output), `-time-passes` and `-verify-each`. The new pass manager's own page covers the nesting rules and says
  the optimization pipeline uses the new pass manager while backend code generation still uses the legacy one ([New Pass
  Manager](https://llvm.org/docs/NewPassManager.html)).
- `llc` compiles `.ll`/`.bc` to assembly or object code; `-O` selects the code-generation optimization level,
  `-filetype=obj|asm|null` selects output (`null` is for performance testing of the compiler), `-mcpu`/`-mtriple` control the
  target, and `-time-passes` and `-stats` report on the code-generation passes ([llc](https://llvm.org/docs/CommandGuide/llc.html)).
  The register allocator is `fast` for unoptimized code and `greedy` for optimized code, per the same page.
- `clang -Xclang -disable-llvm-passes` emits "pristine LLVM IR from the frontend by not running any LLVM passes at all"; `-Xclang
  -disable-O0-optnone` stops clang from adding `optnone` at `-O0`
  ([Options.td](https://github.com/llvm/llvm-project/blob/llvmorg-19.1.7/clang/include/clang/Driver/Options.td)). `optnone` makes
  "most optimization passes ... skip this function, with the exception of interprocedural optimization passes" and defaults code
  generation to the fast instruction selector ([LangRef, function
  attributes](https://llvm.org/docs/LangRef.html#function-attributes)). Any IR that reaches `opt` or `llc` carrying `optnone` is not
  a valid "optimized" input, so the lowering must never emit it.
- `-opt-bisect-limit=N` skips every skippable optimization pass whose index exceeds N; with `-1` it runs everything and prints the
  indexed list. Clang takes it as `-mllvm -opt-bisect-limit=N`. Analysis passes and passes run at `CodeGenOptLevel::None` do not
  check the limit, and passes required for register allocation always run ([OptBisect](https://llvm.org/docs/OptBisect.html)). The
  page does not discuss the new pass manager, but the 19.1.7 source contains an `OptPassGateInstrumentation` that consults the same
  gate for new-pass-manager passes
  ([StandardInstrumentations.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-19.1.7/llvm/lib/Passes/StandardInstrumentations.cpp)).

**A caveat that changes the interpretation of the baseline.** `llc` is not optimization-free above `-O0`.
`TargetPassConfig::addIRPasses` adds, only when the code-generation level is not `None`, TBAA and basic alias analyses, loop
strength reduction, `MergeICmps` and `ExpandMemCmp`, partial libcall inlining, and `CodeGenPrepare` is added alongside
([TargetPassConfig.cpp, 19.1.7](https://github.com/llvm/llvm-project/blob/llvmorg-19.1.7/llvm/lib/CodeGen/TargetPassConfig.cpp)).
Some of these have hidden disable switches such as `-disable-lsr` and `-disable-cgp` in that file; hidden options are not a stable
interface. Consequences for the design:

- A "bcc IR with no bcc optimizer, then `llc -O2`" arm is the floor of the comparison, but it contains loop strength reduction and
  `CodeGenPrepare` work. That is acceptable because the code generator is meant to be constant, as long as the report says so.
- Run the whole matrix at two code-generation settings: `llc -O2` as the main result and `llc -O0` as a sensitivity check. If the
  ranking of arms flips between them, the conclusion depends on the back end and must be reported that way.

**Arms.** Name them, fix them in a manifest, and run every program under every arm.

- **A0 (floor):** bcc front end, bcc IR, no bcc passes, lowering, `llc`.
- **A1 (bcc optimizer):** as A0 with the bcc pass pipeline enabled. Several variants (full, leave-one-out, prefix) fall under this
  arm; section 5.
- **A2/A3 (LLVM optimizer):** as A0 with `opt -passes='default<O2>'` or `default<O3>` between lowering and `llc`. Optionally
  `default<O1>`.
- **A4 (stacked):** bcc passes, then LLVM `default<O2>`. This answers whether the bcc passes still help once LLVM runs, and whether
  they remove information LLVM would have used (inference).
- **M (matched pipeline):** `opt -passes=<list>` where the list is the LLVM counterpart of the bcc pass list, one pass for one pass
  (names such as `mem2reg`, `sroa`, `instcombine`, `simplifycfg`, `gvn`, `dce`, `adce`, `licm`, `loop-unroll`, `inline` are
  documented in [LLVM Passes](https://llvm.org/docs/Passes.html)). This is the arm that answers the pass question.
- **R (reference):** `clang -O2` from the C source, and R' = `clang -O2 -Xclang -disable-llvm-passes -S -emit-llvm` then `llc`. R
  measures the industrial toolchain; R' isolates clang's front-end IR plus the same code generator, which is the right yardstick for
  A0.

### 1.3 Two questions, two comparisons

Comparing A1 with A2/A3 compares products. It will usually favor LLVM on anything that uses vectorization, interprocedural analysis
or profile-free heuristics tuned over years, and it should be reported as "distance from LLVM", not as a verdict on individual
algorithms. Comparing A1 with M compares algorithms at equal pass inventory. Both are legitimate, they answer different questions,
and the report should keep the plots separate. Where bcc has no counterpart pass for a large LLVM contributor (for example the loop
and SLP vectorizers inside `default<O2>`), flag the corresponding benchmarks rather than hiding them in the geometric mean
(inference).

### 1.4 What to measure

- **Generated-code performance:** user CPU time of the benchmark process (preferred over wall time because it excludes scheduling
  waits), plus wall time as a cross-check. llvm-test-suite records `exec_time`, `compile_time` and `size` per program ([test-suite
  guide](https://llvm.org/docs/TestSuiteGuide.html)). Embench times only the region between `start_trigger` and `stop_trigger`,
  after `warm_caches` ([Embench support/main.c](https://github.com/embench/embench-iot/blob/master/support/main.c)), and Polybench
  times only the kernel ([Polybench
  README](https://github.com/llvm/llvm-test-suite/blob/main/SingleSource/Benchmarks/Polybench/README)). Prefer such in-program
  regions so process start-up and I/O do not dilute differences.
- **Code size:** `.text` bytes per object. Embench defines a size score as the geometric mean of relative size and says benchmarks
  for size are built with size flags ([Embench user guide](https://github.com/embench/embench-iot/blob/master/doc/README.md)).
- **Compile time, split by stage.** Time the bcc front end, bcc IR build, bcc optimizer (per pass), lowering, LLVM `opt`
  (`-time-passes`) and `llc` (`-time-passes`, `-stats`) separately. Also record whole-process wall time per stage, because file I/O
  and text parsing between processes are a real cost of the LLVM path that an in-process comparison would hide. Report both numbers
  (inference). The README already insists on separating full-pipeline time from parser-only time for the same reason ([README,
  Performance changes](../../README.md#performance-changes)).
- **Memory:** peak working set and peak commit of each stage process. On Windows these are `PeakWorkingSetSize` and
  `PeakPagefileUsage`
  ([PROCESS_MEMORY_COUNTERS](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters));
  llvm-test-suite's LNT schema has a `mem_bytes` metric ([LNT importing data](https://llvm.org/docs/lnt/importing_data.html)). bcc
  reserves 100 GiB of virtual memory on the first arena allocation and commits pages as the pointer advances ([AGENTS.md code
  map](../../.agents/AGENTS.md)), so virtual size is meaningless for bcc: use peak working set and peak commit, as the repository's
  memory harness already does.
- **A deterministic proxy:** the dynamic IR instruction count from the bcc IR interpreter (section 5). It carries no timing noise
  and can be tracked per pass on every commit. It is a proxy only; it ignores instruction selection, scheduling and cache effects,
  so confirm any claim with native timings.

## 2. Correctness oracles while developing the optimizer

An optimizer that is fast and wrong is worse than none, and benchmarking a miscompiled binary is meaningless. Use layered oracles,
cheapest first.

### 2.1 Metamorphic and differential testing

Every program that is free of undefined behavior must produce identical output under A0, A1, A2, A3 and R. This is the oracle
YARPGen documents for its own programs: each prints a hash of its global variables that "is supposed to be the same for all
compilers and optimization levels" ([YARPGen README](https://github.com/intel/yarpgen)). It needs no expected-output file, so it
extends to generated programs. The GCC c-torture `execute` tests give a second form: each must compile, link and run under multiple
optimization combinations ([GCC test suites](https://gcc.gnu.org/onlinedocs/gccint/C-Tests.html)). Use the subset the earlier note
identified as portable, with LLVM's exclusion list as evidence of what to drop ([earlier
note](compiler-test-corpus-research.md#what-the-compilers-themselves-test)).

Differential testing is only sound if the programs have no undefined behavior; otherwise disagreement does not indicate a bug.
Screen every corpus program once with `clang -fsanitize=undefined` (it enables, among others, `signed-integer-overflow`, `shift`,
array `bounds` and `null` checks, and Windows is a listed platform)
([UBSan](https://clang.llvm.org/docs/UndefinedBehaviorSanitizer.html)). The page does not list uninitialized reads, so that class
needs a different tool, and the bcc IR interpreter should trap on it (inference).

### 2.2 Random program generators

**Csmith.** A random C program generator for differential compiler testing, whose output is intended to be free of undefined
behavior ([Csmith README](https://github.com/csmith-project/csmith)). The PLDI 2011 paper reports more than 325 previously unknown
compiler bugs and that every tested compiler crashed or miscompiled ([Yang et al., PLDI 2011
preprint](https://www-old.cs.utah.edu/~regehr/papers/pldi11-preprint.pdf)). Its license is the University of Utah "BSD License"
([COPYING](https://github.com/csmith-project/csmith/blob/master/COPYING)). Status: the most recent commit I observed through
GitHub's API on 2026-10-11 is dated 2026-03-02 ([commits](https://github.com/csmith-project/csmith/commits/master)), the repository
page lists no releases, and the maintainers say they work on it in spare time, so expect slow responses. Run-time needs:
`runtime/csmith.h` includes `float.h`, `math.h` and `string.h`, and `platform_generic.h` includes `stdio.h`; defining
`CSMITH_MINIMAL` selects `csmith_minimal.h`, which needs only `printf` (or `putchar` with `NO_PRINTF`) plus Csmith's own
`custom_stdint_x86.h` and `custom_limits.h` ([csmith.h](https://github.com/csmith-project/csmith/blob/master/runtime/csmith.h),
[csmith_minimal.h](https://github.com/csmith-project/csmith/blob/master/runtime/csmith_minimal.h)). That makes `-DCSMITH_MINIMAL`
the right mode while libc support is partial. Generated programs may loop forever, so every run needs a timeout (README). Whether
default Csmith output stays inside bcc's C99 mode must be tested; I did not verify which options keep it within C99 (inference:
check `csmith --help`).

**YARPGen.** A generator designed to trigger optimizer bugs; license Apache 2.0
([LICENSE.txt](https://github.com/intel/yarpgen/blob/main/LICENSE.txt)); the OOPSLA 2020 paper reports more than 220 bugs found and
that its generation policies raised optimization application rates by 20% (LLVM) and 40% (GCC) on average ([Livinskii, Babokin,
Regehr, OOPSLA 2020](https://users.cs.utah.edu/~regehr/yarpgen-oopsla20.pdf)). It is the more active project: the last commit I
observed on 2026-10-11 is dated 2026-10-08. Two caveats from the README: the scalar-optimization generator lives in the `v1` branch
while `main` is the loop-testing rewrite, and programs are "statically and dynamically correct ... no undefined behavior, but allows
for implementation defined behavior" ([README](https://github.com/intel/yarpgen)). The implementation-defined allowance means bcc's
`--target` data model must match the reference compiler's. Emitted code calls `printf` and includes `stdio.h`
([program.cpp](https://github.com/intel/yarpgen/blob/main/src/program.cpp)). Whether `main` emits C99 that bcc accepts is unverified
here; test it before depending on it.

Use both generators for correctness only. Their programs are not representative workloads, so their run times say nothing about
optimizer quality.

### 2.3 An IR interpreter as the reference semantics

Write a small bcc-IR interpreter early. It gives a reference that does not depend on LLVM, runs before the code generator exists,
produces the deterministic instruction-count metric of section 1.4, and can trap on undefined behavior (signed overflow where the IR
says it is undefined, out-of-bounds access, reads of uninitialized stack slots), which turns UB screening into a built-in check
(inference). Cross-check it against LLVM's own interpreter on the lowered IR: `lli -force-interpreter` runs bitcode in an
interpreter ([lli](https://llvm.org/docs/CommandGuide/lli.html)). A three-way agreement among the bcc interpreter, `lli` and the
native binary also validates the lowering, which otherwise becomes an unchecked assumption under every other result.

### 2.4 Translation validation

Translation validation checks each optimizer run's output after the fact instead of proving the optimizer correct. The idea behind
CompCert's untrusted passes is that the checker, not the optimizer, is verified: Tristan and Leroy present formally verified
validators for list and trace scheduling ([POPL 2008](https://xavierleroy.org/bibrefs/Tristan-Leroy-scheduling.html)), and Rideau
and Leroy check register allocation results with a checker proved sound in Coq ([CC
2010](https://xavierleroy.org/bibrefs/Rideau-Leroy-regalloc.html)). CompCert's other passes carry whole-pass proofs instead, each
pass tied to a `match_prog` relation ([CompCert Compiler module](https://compcert.org/doc/html/compcert.driver.Compiler.html)).
Formal verification of bcc's optimizer is out of scope; the transferable idea is a cheap checker per pass.

**Alive2** is the practical checker. It verifies that an LLVM IR transformation preserves meaning, licensed MIT
([repository](https://github.com/AliveToolkit/alive2)); `alive-tv src.ll tgt.ll` reports "Transformation seems to be correct!" or a
counterexample, and `-disable-undef-input` suppresses counterexamples that depend on `undef`
([README](https://github.com/AliveToolkit/alive2/blob/master/README.md)). Its limits matter here: it does not support
interprocedural transformations and may report spurious counterexamples for them, and loops are handled by unrolling with factors
that default to 0 (`-src-unroll`, `-tgt-unroll`, or `-unroll`)
([cmd_args_list.h](https://github.com/AliveToolkit/alive2/blob/master/llvm_util/cmd_args_list.h)). The commit I observed on
2026-10-04 shows it tracks current LLVM.

To use it on bcc's optimizer, lower the bcc IR before and after a single pass to LLVM IR and run `alive-tv` on the pair (inference).
This validates refinement per pass and per function and gives a counterexample on failure. Preconditions: the lowering must be
faithful, bcc IR's treatment of undefined values and overflow must map onto LLVM's poison/undef rules, and only intraprocedural
passes can be checked this way. An unfaithful lowering produces false alarms, so keep the interpreter cross-check from section 2.3
as the guard.

### 2.5 Reduction

When a differential or Alive2 failure appears, reduce it before diagnosing. C-Reduce shrinks a C program while a user-written
interestingness test still passes ([C-Reduce](https://github.com/csmith-project/creduce), license text in its COPYING, University of
Utah). `llvm-reduce --test=<script>` does the same for LLVM IR and exits with status 2 if the initial input is not interesting
([llvm-reduce](https://llvm.org/docs/CommandGuide/llvm-reduce.html)). Add an interestingness test that rejects candidates which are
no longer UB-free (for example by also running the UBSan build), otherwise reduction tends to converge on programs that were never
valid (inference). Every reduced reproducer becomes a tracked regression test.

## 3. Benchmark corpora

### 3.1 The libc requirement, restated for this project

bcc-rust today is a front end; code generation is unimplemented ([README status](../../README.md)). The planned flow ends in LLVM
IR, object code from `llc`, and a link by the pinned LLD ([README prerequisites](../../README.md#prerequisites)). A benchmark
program therefore needs the C library in two different ways. At compile time bcc must parse the C library's headers; the README
already measures this with the libc header survey for glibc, musl, MinGW-w64 and the MSVC UCRT ([libc header
survey](../../README.md#libc-header-survey)). At run time the linked binary uses the host's real C library, which bcc does not need
to implement. "No full libc support yet" therefore means "bcc cannot yet parse every header a benchmark includes", and the useful
filter for a corpus is which headers each program includes, not which functions it calls (inference). A second practical constraint:
the pinned native LLVM build installs only `clang;libclang;llvm-ar;lld;clang-resource-headers`
([build_support/llvm.rs](../../build_support/llvm.rs)), so `opt`, `llc`, `llvm-reduce` and `lli` are not in the pinned build today.
The harness needs them, either by adding them to the distribution components or by using LLVM's official prebuilt release that the
README already describes for Linux. Rust's `llvm-tools` component is installed by the toolchain, but I did not verify which of these
tools it contains.

### 3.2 llvm-test-suite `SingleSource/Benchmarks`

- **Contents and size.** Directories include `BenchmarkGame`, `Dhrystone`, `Linpack`, `McGill`, `Misc`, `Polybench`, `Shootout`,
  `Stanford`, `CoyoteBench`, `SmallPT` and several C++ ones
  ([directory](https://github.com/llvm/llvm-test-suite/tree/main/SingleSource/Benchmarks)). I did not count programs; a count would
  have to come from a pinned checkout.
- **License.** Mixed. The top-level license is the University of Illinois/NCSA license plus a list of third-party directories that
  carry their own terms ([LICENSE.TXT](https://github.com/llvm/llvm-test-suite/blob/main/LICENSE.TXT)). `Misc`, `Dhrystone` and
  `Polybench` have their own license files. Check each directory you vendor.
- **libc.** Standard I/O, `stdlib`, and `libm` (Polybench's CMake adds `-lm`;
  [CMakeLists.txt](https://github.com/llvm/llvm-test-suite/blob/main/SingleSource/Benchmarks/Polybench/CMakeLists.txt)). Polybench
  pulls in POSIX headers (`unistd.h`, `sys/time.h`, `sched.h`)
  ([polybench.c](https://github.com/llvm/llvm-test-suite/blob/main/SingleSource/Benchmarks/Polybench/utilities/polybench.c)), which
  is awkward on a Windows host.
- **Determinism and run method.** Programs carry `<name>.reference_output` files and some a `.small` variant
  ([Misc](https://github.com/llvm/llvm-test-suite/tree/main/SingleSource/Benchmarks/Misc)). Run through lit with `.test` files that
  contain `PREPARE:`, `RUN:` and `VERIFY:` lines, for example a `diff` against a reference file ([litsupport
  README](https://github.com/llvm/llvm-test-suite/blob/main/litsupport/README.md)). `compare.py` by default filters out tests faster
  than 1.0 s, a hint that many programs are too short to time
  ([compare.py](https://github.com/llvm/llvm-test-suite/blob/main/utils/compare.py)).
- **Fit.** Good for breadth and for reuse of existing reference outputs. Running the real CMake/lit harness with bcc as
  `CMAKE_C_COMPILER` requires a cc-compatible driver wrapper ([guide](https://llvm.org/docs/TestSuiteGuide.html)); defer that. Copy
  the sources and `reference_output` files into the harness instead (keeping their licenses), as the earlier note advised for the
  torture mirror.

### 3.3 Polybench/C

- **Size.** 30 numerical kernels (linear algebra, stencils, dynamic programming, data mining) in version 4.2.1 (beta), copyright
  Ohio State University 2011-2016 ([README in
  llvm-test-suite](https://github.com/llvm/llvm-test-suite/blob/main/SingleSource/Benchmarks/Polybench/README)).
- **License.** The copy in llvm-test-suite carries the "Ohio State University Software Distribution License", a BSD-style license
  that also forbids using the university's name for endorsement
  ([LICENSE](https://github.com/llvm/llvm-test-suite/blob/main/SingleSource/Benchmarks/Polybench/LICENSE)). SourceForge labels the
  project GPLv2 ([project page](https://sourceforge.net/projects/polybench/)). The two disagree; the license file that ships in the
  tree you vendor governs.
- **libc.** Kernels use `math.h` and static arrays; the driver file `polybench.c` uses the POSIX headers listed above, `malloc`, and
  `gettimeofday`-based timing; macros `POLYBENCH_USE_C99_PROTO` and `POLYBENCH_STACK_ARRAYS` change the array form (README). The
  macro-heavy source also exercises bcc's preprocessor and VLA handling.
- **Determinism and run method.** Data initialization is deterministic; `-DPOLYBENCH_DUMP_ARRAYS` writes live-out arrays to stderr
  so a reference run (for example `gcc -O0`) yields a comparison file; dataset size is selected with `-DMINI_DATASET` through
  `-DEXTRALARGE_DATASET`; the timer flushes a 33 MB cache by default (README). The bundled `time_benchmark.sh` runs five times,
  drops the two extremes and demands deviation within 5%.
- **Fit.** The best corpus here for loop optimizations (invariant code motion, strength reduction, unrolling, interchange,
  vectorization). It also guarantees large gaps against LLVM wherever bcc lacks loop passes, so report it per kernel, not as a
  single number.

### 3.4 Embench-IoT

- **Size.** 19 benchmarks, run in a few minutes ([repository](https://github.com/embench/embench-iot)), including `crc32`, `md5sum`,
  `nettle-aes`, `nettle-sha256`, `picojpeg`, `qrduino`, `slre`, `statemate`, `ud`, `wikisort` and `xgboost` (the `src` directory
  listing).
- **License.** The repository is GPL-3.0, but individual benchmarks have their own licenses (README); for example `crc32` is
  GPL-3.0-or-later and `md5sum` is MIT in their SPDX headers. Read each file before redistributing.
- **libc.** Designed for systems that "assume the presence of no OS, minimal C library support and in particular no output stream"
  (README). The support library provides its own `malloc_beebs` and `rand_beebs` and includes only `stddef.h`, `stdint.h`,
  `string.h` and `assert.h` ([beebsc.c](https://github.com/embench/embench-iot/blob/master/support/beebsc.c)). This is the closest
  match to a front end that cannot yet parse the full hosted headers.
- **Determinism and run method.** Each benchmark follows a fixed protocol: `initialise_benchmark`, `warm_caches`, `start_trigger`,
  the benchmark, `stop_trigger`, then `verify_benchmark` checks the result (support/main.c). Runs are scaled to about 4 s via a
  global scale factor. The speed score is the geometric mean of per-benchmark speed relative to a reference platform, also reported
  with a geometric standard deviation (user guide). Cycle-accurate timing assumes an embedded target or simulator; for a native x86
  host, adapt a board-support file (the repository's `examples/native`).
- **Status.** Latest stable tag `embench-1.0`; `embench-2.0rc2` is the newest release candidate (repository); the last commit I
  observed is 2024-08-29.
- **Fit.** First choice: tiny dependency surface, built-in self-check, and a published scoring method (geometric mean of ratios)
  that matches section 4.

### 3.5 CoreMark

- **Size and license.** Six C files, about 1,083 source lines, plus a small porting layer
  ([repository](https://github.com/eembc/coremark)). The source headers are Apache 2.0, 2018, EEMBC
  ([core_main.c](https://github.com/eembc/coremark/blob/main/core_main.c)). A separate acceptable-use agreement controls use of the
  CoreMark trademark and is not a restriction on running the source
  ([LICENSE.md](https://github.com/eembc/coremark/blob/main/LICENSE.md)). Publishing a "CoreMark score" requires unmodified sources
  outside the porting files, a run of at least 10 s, and passing validation (repository README); so a bcc-vs-LLVM comparison inside
  this project is fine, but do not label it an official score.
- **libc.** `coremark.h` includes `stdio.h`; I/O and timing live in `core_portme.*` (the porting layer supports targets without
  `printf`).
- **Determinism.** CRC validation of list, matrix and state results; a fixed seed set; `ITERATIONS=N` fixes the work for simulators
  (README).
- **Status.** No tagged release is listed on the repository page; the last commit I observed is 2025-05-01.
- **Fit.** Cheap single-number sanity check and a known workload, but it is one small program; do not weight it like a suite.

### 3.6 Computer Language Benchmarks Game

Revised (3-clause) BSD ([license](https://benchmarksgame-team.pages.debian.net/benchmarksgame/license.html)). The programs are small
and per-language; the C `n-body` program includes `math.h`, `stdio.h` and `stdlib.h`, takes its size from `argv[1]` and prints two
numbers whose expected values are on the page ([n-body gcc
#1](https://benchmarksgame-team.pages.debian.net/benchmarksgame/program/nbody-gcc-1.html)), which is also where the site's gcc flags
(`-O3 -fomit-frame-pointer -march=ivybridge`) appear. Some programs for other tasks use threads or libraries; I did not survey
which. The authors call these toy programs and advise checking for run-order effects and measuring your own application for real
decisions ([why measure toy
programs](https://benchmarksgame-team.pages.debian.net/benchmarksgame/why-measure-toy-benchmark-programs.html)). Use the seven
programs mirrored in llvm-test-suite's `BenchmarkGame` directory (`fannkuch`, `n-body`, `nsieve-bits`, `partialsums`, `puzzle`,
`recursive`, `spectral-norm`), which ship with reference outputs
([directory](https://github.com/llvm/llvm-test-suite/tree/main/SingleSource/Benchmarks/BenchmarkGame)).

### 3.7 MiBench and cBench

MiBench has six groups (automotive, consumer, network, office, security, telecomm) in tar archives of 889 KB, 30 MB, 470 KB, 14 MB,
2 MB and 34 MB; its page says "as a general rule, all benchmarks are considered to be covered by GNU's GPL", with per-benchmark
LICENSE files, small and large inputs via `runme_small.sh`/`runme_large.sh`, and build instructions tested on Debian x86 of the
"potato" era ([source page](https://vhosts.eecs.umich.edu/mibench/source.html)). Programs read input files and print results, so
libc dependence is heavy, and the age of the code suggests pre-C99 constructs (inference). cBench is a community collection derived
from MiBench and the MiDataSets; its wiki page was last modified in 2015 and the programs live in `ctuning-programs`, run through
Collective Knowledge (`ck compile program:cbench-automotive-susan --speed`)
([wiki](https://ctuning.org/wiki/index.php/CTools:CBench), [ctuning-programs](https://github.com/ctuning/ctuning-programs)). I found
no stated license on either page. Defer both until the front end handles hosted headers reliably; they cost porting and legal
review.

### 3.8 Suggested order

c-torture `execute` subset as a gate; Embench-IoT; a Polybench/C subset; the small llvm-test-suite programs with reference outputs;
CoreMark as a cross-check; YARPGen and Csmith programs for correctness only; MiBench and cBench later.

## 4. Statistics and noise control

### 4.1 What noise and bias exist

Two kinds need different handling. Random noise (interrupts, frequency changes, cache contention) is reduced by repetition.
Measurement bias is a systematic, repeatable effect of an experimental setup detail: Mytkowicz et al. show that such seemingly
innocuous setup details change results enough to flip conclusions, across two compilers and several architectures, and that none of
133 surveyed papers adequately addressed it ([ASPLOS 2009](https://sape.inf.usi.ch/publications/asplos09)). LLVM's own benchmarking
guide cites the same paper to warn that low noise is necessary but not sufficient ([Benchmarking
tips](https://llvm.org/docs/Benchmarking.html)). For a compiler comparison this matters directly: two builds of the same program
that differ only in code placement can differ in speed, and no number of re-runs of one binary reveals that (inference from the bias
definition). Mitigations: build several variants per arm (vary link order, function alignment or padding, and environment size) and
treat the spread across variants as part of the uncertainty. Kalibera and Jones formalize this by modelling repetitions at several
levels (builds, executions, iterations) and choosing how many at each level reach a target precision ([ISMM
2013](https://kar.kent.ac.uk/33611/)).

### 4.2 Environment control

On Linux, the LLVM guide recommends: a high-resolution timer such as `perf`; disabling other processes and services; disabling
frequency scaling and Turbo Boost; disabling address-space randomization via `/proc/sys/kernel/randomize_va_space`; the
`performance` governor; isolating cores with `cset shield`, leaving two free if `perf` is used; taking SMT siblings offline; static
linking; and a tmpfs for the program and data. With these, it says variation under 0.1% is achievable, and it suggests `cset shield
--exec -- perf stat -r 10 <cmd>` ([LLVM Benchmarking](https://llvm.org/docs/Benchmarking.html)). LNT's runner documentation adds
pinning with `taskset` and running one benchmark at a time ([LNT tests](https://llvm.org/docs/lnt/tests.html)).

On Windows there is no `cset` or ASLR switch of that kind in the sources I read, and the weaker controls available are in the Win32
API: `SetProcessAffinityMask` for a process (Microsoft warns that setting affinity should generally be avoided except for testing
processors, and the mask spans one processor group on systems with more than 64 processors) ([Multiple
Processors](https://learn.microsoft.com/en-us/windows/win32/procthread/multiple-processors)); `SetPriorityClass` with
`HIGH_PRIORITY_CLASS` ("use extreme care", it can use nearly all CPU time) and never `REALTIME_PRIORITY_CLASS` on a development
machine
([SetPriorityClass](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setpriorityclass));
and per-process timing from `GetProcessTimes`, which returns kernel and user CPU time in 100 ns units and notes that user time can
exceed elapsed time on multiple cores, with `QueryProcessCycleTime` for cycles
([GetProcessTimes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes)).
Power plan, background services and antivirus exclusions are not covered by sources I read; treat them as manual checklist items
(inference). The README already says to measure Linux performance on Linux rather than under WSL 2, where page faults pass through
the hypervisor ([README](../../README.md#performance-changes)). Therefore: final generated-code numbers come from native Linux;
Windows runs are for correctness and compile-time trends.

### 4.3 Repetitions and the summary statistic

- **Repetitions.** The repository's rule is at least seven interleaved runs, reporting the minimum alongside the central estimate
  ([README, Performance changes](../../README.md#performance-changes)). Interleave arms (A, B, A, B, ...) instead of running all of
  A then all of B, so slow drift in machine state hits both. Keep the minimum and the median.
- **Why the minimum.** If noise only adds time, the minimum is a more stable estimate than the mean; Chen, Revels and Edelman
  justify it for noisy environments and note it can underestimate when the timer is coarser than the delays ([arXiv
  1608.04295](https://arxiv.org/abs/1608.04295)). llvm-test-suite's `compare.py` merges runs with the minimum by default and offers
  mean and median ([compare.py](https://github.com/llvm/llvm-test-suite/blob/main/utils/compare.py)). The minimum does not remove
  layout bias (section 4.1).
- **Intervals and tests.** `compare.py` computes a per-program Student's t-test (`scipy.stats.ttest_ind`), p-values with a
  significance flag, optional confidence intervals for the difference, and a "Geomean difference" row (same file). Georges et al.
  argue for statistically rigorous Java performance evaluation using confidence intervals rather than single runs ([OOPSLA
  2007](https://dri.es/files/oopsla07-georges.pdf)). A t-test assumes roughly normal errors, which timing data often violates; a
  bootstrap over runs is the safer default for the harness (inference).
- **Aggregating across benchmarks.** Normalize each benchmark to the baseline arm and take the geometric mean of the ratios. Fleming
  and Wallace show the arithmetic mean of normalized numbers can mislead and the geometric mean is the correct summary for them
  ([CACM 1986](https://dl.acm.org/doi/10.1145/5666.5673)). Embench does the same and adds the geometric standard deviation and the
  one-sigma range as an indicator of variability (user guide). Also publish the per-benchmark ratios: a geometric mean hides that a
  few large wins and many small losses can average to "no change".
- **Which timer.** `hyperfine` is a convenient driver for whole-process timing: `--warmup`, `--runs`, `--prepare`, `--setup`,
  `-N`/`--shell=none` for fast commands, JSON export, outlier warnings and relative-speed output; it supports Windows, but its
  README lists peak memory and hardware counters as not supported there ([hyperfine](https://github.com/sharkdp/hyperfine)). It is
  dual-licensed MIT/Apache-2.0 (same page). Use it for ad hoc checks. For the main results prefer in-program timers on the region of
  interest, since the benchmarks already provide them.

### 4.4 Reporting and tracking

- **LNT.** LLVM's performance tracking software, with a public server at lnt.llvm.org ([LNT](https://llvm.org/docs/lnt/index.html)).
  A report is JSON: `format_version` "2", a `machine` object, a `run` object whose fields give an order (the `llvm_project_revision`
  for the standard suites), and a `tests` list whose entries carry metrics such as `execution_time` (a number or a list of numbers),
  `compile_time`, `mem_bytes`, `code_size`, `hash` and statuses; it is imported with `lnt importreport` and submitted with `lnt
  submit` ([importing data](https://llvm.org/docs/lnt/importing_data.html)). The LNT pages I could read describe the data model
  (runs, samples, machines) and the runner options (`--multisample`, `--run-under`, `--use-perf`) but not its significance rules, so
  I cannot say how its viewer decides a change is real ([concepts](https://llvm.org/docs/lnt/concepts.html),
  [tests](https://llvm.org/docs/lnt/tests.html)).
- **How llvm-test-suite reports.** lit writes `results.json` with `exec_time`, `compile_time`, `size` and others; `compare.py` shows
  one file or compares two, picks a metric, filters short tests and merges repeat runs ([test-suite
  guide](https://llvm.org/docs/TestSuiteGuide.html)). `TEST_SUITE_RUN_BENCHMARKS=OFF` collects compile-time metrics without running
  anything (same page).
- **Historical comparison.** The Rust perf site flags a change as significant when it is an outlier against historical data by an
  interquartile-range fence, which suits a continuous tracker, not a single A/B experiment
  ([rustc-perf](https://github.com/rust-lang/rustc-perf/blob/master/docs/comparison-analysis.md)). Consider it once results
  accumulate per commit.

Each results table should show, per benchmark: arm, minimum and median time, interval, ratio to baseline, code size, and a
verification status; and per arm: geometric mean ratio with interval, count of benchmarks excluded and why (too short, failed
verification, unsupported construct), compile time by stage, and peak memory. A comparison that omits the excluded count is not
reproducible.

## 5. Recommended minimal harness

### 5.1 What to build first

Order the work so that each step is useful on its own and later steps reuse it.

1. **Measurement skeleton using only clang (no bcc IR yet).** A runner that fetches and verifies corpora, builds each program with
   the pinned `clang` at `-O0`, `-O2` and `-O3`, runs them with verification, and writes result records. This validates the corpus,
   the statistics code and the noise controls with a known-good compiler, and sets the R and R' arms.
2. **The bcc IR interpreter plus deterministic instruction counts.** This is the noise-free per-pass metric, runs on any host
   (including Windows) and works before lowering exists.
3. **Lowering to textual LLVM IR and arms A0, A2, A3, R'.** Text `.ll` is enough; a bitcode writer can wait. Add `opt`, `llc` (and
   `llvm-reduce`, `lli`) to the pinned build or a prebuilt LLVM, as noted in section 3.1.
4. **The pass-ablation interface on the bcc optimizer** (below), then arms A1, A4 and M.
5. **Generator-driven correctness runs** (YARPGen, Csmith) with reduction.
6. **Per-pass Alive2 validation** over the corpus and generated programs.
7. **LNT-format export** and, if wanted, a continuous tracker.

### 5.2 Per-pass ablation: the interface that makes it possible

Copy the shape of LLVM's tooling, because the same shape lets one script drive both optimizers.

- **An explicit ordered pass list.** A flag such as `--passes=a,b,c` on the bcc optimizer entry point, mirroring `opt -passes`
  ([opt](https://llvm.org/docs/CommandGuide/opt.html)), and a default pipeline expressed as the same textual list so that "the
  default" is just a named string. Without this, every ablation needs a rebuild.
- **A per-pass verifier switch.** `--verify-each`, as in `opt -verify-each` ([opt](https://llvm.org/docs/CommandGuide/opt.html)), so
  a bad pass is identified at its own output, not at the end of the pipeline.
- **Per-pass timing and counters.** The equivalents of `-time-passes` and `-stats` (opt and llc pages above): wall time,
  instructions removed and added, blocks removed, allocations or arena bytes used, per pass invocation. Memory per pass is useful
  because bcc's arenas make high-water marks cheap to read ([README](../../README.md#performance-changes)).
- **A transformation gate.** A global counter incremented at each transformation a pass performs, with `--opt-bisect-limit=N` to
  skip all transformations beyond the Nth, mirroring LLVM's design in which indices are stable from run to run and can cover whole
  pass runs or single transformations ([OptBisect](https://llvm.org/docs/OptBisect.html)). Build this in from the first pass;
  retrofitting it is much harder (inference). It turns "which optimization broke this program" into a binary search, and turns "how
  much does optimization help" into a curve of performance versus N.
- **IR dump after each pass** to a directory, for diffing and for feeding Alive2 (before/after pairs).

Ablation modes the harness should run, each on the full corpus:

- **Leave-one-out:** the full pipeline minus pass k, for each k. It shows what each pass contributes in the presence of the others,
  including interactions.
- **Prefix (add-one-in):** pipeline prefixes of length 0..n in order. It produces the cumulative curve of improvement and shows
  diminishing returns.
- **Singleton:** pass k alone plus only the canonicalization it needs (for example promotion of locals before a pass that works on
  SSA values). It shows a pass's stand-alone power, which is not the same thing as its value in a pipeline.
- **Bisect sweep:** the full pipeline with the transformation gate at a set of N values, to separate "many small wins" from "one big
  win".
- **Matched LLVM counterpart:** the same list through `opt -passes` for arm M. Keep the pass-name map in the corpus manifest or a
  sibling file under version control.

Interactions mean no single table is a clean attribution; report leave-one-out and prefix together and say so (inference).

### 5.3 Result format

Write one JSON object per measurement (JSON Lines), appended as the run proceeds so a crash loses nothing, and derive tables from
that file. Each record should carry:

- run identity: harness git commit, bcc commit, LLVM revision and version string of `clang`/`opt`/`llc`, host triple, target triple,
  CPU model, OS version, governor/power-plan notes, and a fresh output directory name (the repository requires a fresh directory per
  run for the existing survey, [README](../../README.md#external-torture-corpus));
- cell identity: corpus name, program, input/dataset, arm name, pass list or pipeline string, lowering profile, code-generation
  level, layout variant;
- stage records: for each stage, wall time, CPU time, peak working set, peak commit, exit status, and the byte size of its output;
- run records: repetition index, region time (when the program reports it), process wall and user time, exit status, verification
  result (hash or diff outcome), timeout flag;
- static metrics: `.text` size and IR instruction count after optimization; dynamic IR instruction count from the interpreter, when
  available;
- bcc optimizer per-pass records as above.

Keep field names compatible with LNT's report schema where they overlap (`execution_time` as a list of samples, `compile_time`,
`mem_bytes`, `code_size`, `hash`, status fields; `format_version` "2"), so a later export is a transformation, not a redesign
([importing data](https://llvm.org/docs/lnt/importing_data.html)). A summary step then writes `results.json` in the llvm-test-suite
shape if `compare.py` is wanted, and a Markdown table for the repository. Write the statistics (minimum, median, bootstrap interval,
geometric mean with interval) in a standard-library Python module so no third-party dependency is needed (inference).

### 5.4 Where corpora live

Follow the existing convention that downloads go into ignored `target/` storage with a recorded SHA-512.
`scripts/run_gcc_torture.py` already downloads a pinned archive into `target/compiler-corpus`, verifies a hard-coded
`ARCHIVE_SHA512`, extracts only regular files beneath named prefixes, rejects unsafe archive members, and records a marker and
per-file `source_sha512` ([script](../../scripts/run_gcc_torture.py)). The new runner should reuse that pattern:

- cache downloads under `target/compiler-corpus/` (existing) and write results under `target/optimizer-bench/<fresh-run-name>/`;
- track in git only the runner, a corpus manifest (name, upstream URL, revision or release, archive SHA-512, selected paths, license
  identifier and license-file path, per-program arguments, timeout, expected-output source and hash, flags that must be passed), and
  a small summary of results that matters for review;
- for corpora taken from a Git revision rather than a release tarball, record the commit and a SHA-512 of a sorted manifest of the
  extracted files in addition to the archive hash, because a hosted archive's bytes are not guaranteed by the commit alone
  (inference);
- extract each upstream corpus into its own empty directory and run any script that reads it from a different directory, per the
  repository's rule for untrusted downloads;
- keep upstream files unmodified and apply bcc-specific build flags and macros from the manifest, as the earlier note did for
  torture sources, so a corpus update is a manifest change.

### 5.5 Deliberate omissions and open questions

The first version leaves out in-process LLVM linkage (textual `.ll` and separate processes keep arms identical and stage timing
simple), a cc-compatible driver for the llvm-test-suite CMake/lit/LNT path ([guide](https://llvm.org/docs/TestSuiteGuide.html)),
PGO/LTO arms, and cross-target claims (the README validates only Windows GNU so far,
[README](../../README.md#native-c-static-linking-and-cross-language-lto)). Settle before the first experiment: the target and
`-mcpu` that define "the same machine" (so A3 and R differ for no unrelated reason); whether bcc IR makes signed overflow,
out-of-bounds and uninitialized reads explicit (Alive2, the interpreter and the `nsw` profiles depend on it); which passes exist
first, since matched arm M needs the inventory; where timing regions come from for programs without a timer; and how many layout
variants per arm are affordable (section 4.1).
