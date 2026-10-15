Research date: 2026-10-13.

This is the first conformance campaign for bcc's middle end and back ends. Two kinds of test programs were compared against the bundled Clang: about 13,800 random programs from Csmith and YARPGen, and three real open-source programs (SQLite, Lua, zlib) with fixed workloads. The goal was a ranked list of distinct, realistic correctness defects, not a pass rate. No compiler source was changed; the tooling is under `scripts/` and the reproducers under [`conformance-reproducers-2026-10-13/`](conformance-reproducers-2026-10-13/).

## Summary

- **Two distinct wrong-code defects**, both in the front end (all five arms agree on the wrong answer, on both targets):
  1. `#pragma pack` is silently ignored, so packed structures have natural layout. Csmith seeds 95 and 100221 (and 95 on Linux) fail because of it.
  2. A bit-field narrower than `int` loses its integer promotion when it is the right operand of a comma or the result of an assignment, which flips mixed signed/unsigned comparisons. GCC and Clang agree with each other. Csmith seeds 100167 and 300681.
- **No compiler panic, abnormal compiler exit, link error, timeout or optimizer-only miscompile** in about 13,800 programs. A0 (bcc without any optimizer), A1 (bcc optimizer plus LLVM code generation) and A2 (LLVM O2 only) never disagreed with each other on a program the front end accepted. All five arms disagree with Clang only for the two defects above and for two programs where GCC and Clang disagree themselves.
- **The refusals are the larger gap.** 83% of default Csmith programs (3,309 of 3,995) are refused by the front end for pointer-type constraint violations that Clang and GCC accept with a warning. `#include <math.h>` and `#include <windows.h>` from MinGW-w64 are refused on the Windows target; `INFINITY`/`NAN`, `__PRETTY_FUNCTION__` (glibc `assert`), a function designator in a `void *` initializer, `setjmp` on Linux and labels-as-values are refused in real programs.
- **Real programs.** zlib builds completely with bcc on both targets and passes all workloads in A0, A1, A2 and the interpreters. Lua and SQLite build with bcc for all but a few translation units (below); every workload passes in A0, A1 and A2 with those units taken from Clang. On Linux the whole of `sqlite3.c` (a single 9.6 MB translation unit) and 32 of 33 Lua units are compiled by bcc and produce the reference output.
- **Interpreter gaps** are the most frequent non-pass outcome: the 100 million step limit stops 5.6% of YARPGen programs, and `fabsf` has no host function (it stops 368 of 764 Csmith float programs).

## Tools, versions and options

| Item | Value |
| --- | --- |
| bcc | `cargo build --release` at head `47e07d8b` (PR #42), SHA-512 `bbe24405e507a2c26e9b...` for every run |
| Reference compiler | bundled Clang 23.1.1 (`target\llvm\bin\clang.exe`), `-O0`; a second `-O2` build must print the same output or the program is discarded |
| Second opinion | MinGW-w64 GCC 13.2.0 on Windows, WSL GCC 15.2.0 on Linux, only for programs on which an arm disagreed |
| Csmith | 2.4.0, commit `0cdc710`, built with MinGW g++ 13.2.0 |
| YARPGen | 2.0, commit `1adb290`, built with MinGW g++ 13.2.0 |
| Windows target | `--target=x86_64-w64-windows-gnu`, MinGW-w64 headers (`-idirafter`), `-D__CRT__NO_INLINE` |
| Linux target | `--target=x86_64-unknown-linux-gnu`; headers copied from the WSL Ubuntu distribution (`target/sysroots/wsl-ubuntu`); objects linked and programs run by WSL Clang 21.1.8 |
| Host | Windows 11 (10.0.26300), Python 3.14.7, 24 logical processors |

The arms are those of the torture survey plus A2: `interpret` (`--interpret`), `interpret-bcc` (`--interpret --opt=bcc`), A0 (`--llvm-opt=O0`), A1 (`--opt=bcc --llvm-opt=O2 --llvm-codegen-only`) and A2 (`--llvm-opt=O2`). Deadlines: 120 s to compile or interpret, 30 s to run a native program, 30 s for the reference run. Every program an arm builds is started by the script (which disables Windows error dialogs) or, on Linux, by `wsl.exe ... timeout -s KILL`.

Generator options, with the reason for each:

| Generator | Options | Reason |
| --- | --- | --- |
| YARPGen | `--std=c --max-array-dims=2 --emit-pragmas=none` | C dialect; the default tests declare globals over 2 GiB that no arm can build (the interpreter caps one global, COFF relocations are 32-bit, Clang itself fails to link them); OpenMP/loop pragmas are not under test |
| YARPGen mutate profile | adds `--mutate=all --allow-dead-data=true --expl-loop-param=true --max-array-dims=3` | more varied expression shapes |
| Csmith, default | none | measures the refusal rate |
| Csmith `nopointers` | `--no-pointers` | the default profile is refused for pointer-type constraint violations (defect R2), which hides everything after the front end |
| Csmith `nounions` | `--no-pointers --no-unions` | as above, without unions |
| Csmith `nopacked` | `--no-pointers --no-packed-struct` | avoids `#pragma pack` (defect W1), which made the first wrong outputs |
| Csmith `float` | `--no-pointers --float`, plus a header beside the program that casts the one argument of `transparent_crc_bytes` | with floats Csmith passes `&float_object` to a `char *` parameter, another pointer constraint violation |
| Csmith `float-nopacked` | the same with `--no-packed-struct` | |

`-D__CRT__NO_INLINE` is passed to Clang and bcc alike on the Windows target: without it MinGW's `<math.h>` is refused (defect R1). The Linux comparison of the real programs adds a header forced into every unit (for Clang and bcc) that defines `__PRETTY_FUNCTION__`, `__builtin_inff`, `__builtin_nanf` and `__builtin_huge_val` as macros, because glibc's `assert()` and `INFINITY` hit defects R5 and R4 and would hide the rest of SQLite and Lua; those refusals are recorded separately.

Csmith needs the `m4` macro processor, which neither Windows nor this WSL distribution has, so `scripts/mini_m4.py` implements the subset its three runtime headers use (`define`, `ifelse`, `dnl`, `#` comments, `$N`). Both generators link statically because the MSYS and Strawberry MinGW runtime DLLs on `PATH` conflict at start-up (the programs segfault before `main`). The sources under `target/compiler-corpus` were not touched; a force-included header papers over Csmith calling the one-argument MinGW `mkdir` with two.

## Campaign

The campaign ran from 21:11 to 23:19 local time (2 h 08 min of wall time, parts of it in parallel with up to 46 workers on 24 processors) in resumable runs; `results.jsonl`, `manifest.json` and `summary.md` of each run are under `target/conformance/` (not committed). Programs "compared" are those the front end accepted; the others are refusals, which do not run anything.

| Profile | Generated | Compared | Refused by the front end | Discarded (Clang cannot run it, or -O0 and -O2 differ) |
| --- | ---: | ---: | ---: | ---: |
| Csmith default, Windows | 4,000 | 686 | 3,309 | 5 |
| Csmith `nopointers`, Windows | 1,152 | 917 | 0 | 235 |
| Csmith `nounions`, Windows | 441 | 323 | 0 | 118 |
| Csmith `nopacked`, Windows | 1,206 | 961 | 0 | 245 |
| Csmith `float`, Windows | 602 | 476 | 0 | 126 |
| Csmith `float-nopacked`, Windows | 384 | 288 | 0 | 96 |
| **Csmith, Windows** | **7,785** | **3,651** | 3,309 | 825 |
| YARPGen default, Windows | 3,024 | 3,024 | 0 | 0 |
| YARPGen mutate, Windows | 1,512 | 1,512 | 0 | 0 |
| **YARPGen, Windows** | **4,536** | **4,536** | 0 | 0 |
| Csmith `nopointers`, Linux | 409 | 326 | 0 | 83 |
| YARPGen default, Linux | 1,031 | 1,031 | 0 | 0 |

The discards are Csmith programs: 880 whose Clang `-O0` run did not finish in 30 s (loops with huge trip counts), 27 on which Clang `-O0` and `-O2` print different checksums, and one whose `-O2` build crashed. The Csmith Linux run is below the 500-program target (326 compared) because it was started late and was cut to give processors to the reductions.

Outcome per arm, in programs. "Wrong output" for Csmith is the checksum line.

| Csmith, Windows (3,651 compared) | interpret | interpret-bcc | A0 | A1 | A2 |
| --- | ---: | ---: | ---: | ---: | ---: |
| pass | 3,266 | 3,269 | 3,645 | 3,645 | 3,646 |
| interpreter trap | 382 | 379 | 0 | 0 | 0 |
| wrong output | 3 | 3 | 6 | 6 | 5 |
| front-end refusal (additional programs) | 3,309 | 3,309 | 3,309 | 3,309 | 3,309 |

| YARPGen, Windows (4,536 compared) | interpret | interpret-bcc | A0 | A1 | A2 |
| --- | ---: | ---: | ---: | ---: | ---: |
| pass | 4,284 | 4,427 | 4,536 | 4,536 | 4,536 |
| interpreter trap | 252 | 109 | 0 | 0 | 0 |

| Csmith, Linux (326 compared) | interpret | interpret-bcc | A0 | A1 | A2 |
| --- | ---: | ---: | ---: | ---: | ---: |
| pass | 324 | 325 | 325 | 325 | 325 |
| interpreter trap | 1 | 0 | 0 | 0 | 0 |
| wrong output | 1 | 1 | 1 | 1 | 1 |

| YARPGen, Linux (1,031 compared) | interpret | interpret-bcc | A0 | A1 | A2 |
| --- | ---: | ---: | ---: | ---: | ---: |
| pass | 979 | 1,009 | 1,031 | 1,031 | 1,031 |
| interpreter trap | 52 | 22 | 0 | 0 | 0 |

313 of the interpreter traps are "the program exceeded its step limit" (100 million steps; 252 and 52 of them YARPGen programs on Windows and Linux); the bcc optimizer cuts the YARPGen ones by more than half. The other 374 are Csmith float programs: 368 hit the missing `fabsf` host function and 6 are interpreter "poison" traps (below). There were no compiler crashes, link errors, timeouts or abnormal exits in any arm.

Each Csmith program with a wrong output was checked with GCC, because Csmith and YARPGen output is only a valid oracle when the compilers agree. Of the six programs: 95 and 100221 (W1), 100167 and 300681 (W2) have GCC equal to Clang and bcc different; 1040 has GCC equal to bcc and Clang different; 100526 has all three different (D1).

## Distinct defects, ranked

### Wrong code

**W1. `#pragma pack` is ignored (silent wrong layout).** Realistic and severe: network and file-format structures and Windows headers rely on it.

- Reproducer: [`pragma_pack_ignored.c`](conformance-reproducers-2026-10-13/pragma_pack_ignored.c). `#pragma pack(1)`, `#pragma pack(push, 1)`, `#pragma pack(push)` followed by `#pragma pack(2)`, nested pushes, `#pragma pack()` and `pop` are all accepted without a diagnostic and have no effect. Clang prints `A5 B5 C6 D5 E12 F5 G8`; bcc prints `A8 B8 C8 D5 E16 F8 G8`. `__attribute__((packed))` (struct D) works.
- Arms: all five, Windows and Linux. Found by Csmith seeds 95 (both targets) and 100221; with the pragmas rewritten to `__attribute__((packed))` both programs print Clang's checksum.
- Component: preprocessing (`preprocessing/directives/pragma.rs` treats it as an unknown pragma) and record layout in semantic analysis (needs a maximum field alignment).
- Fix direction: keep a pack stack (`pack(n)`, `push`, `pop`, `pack()`, identifier forms) in the preprocessor or parser state, attach the current value to each record declaration, and cap member alignment at it when laying the record out. Warn on an unsupported form instead of ignoring it silently.

**W2. Bit-field promotion is lost through comma and assignment results.** A bit-field narrower than `int` promotes to `int`. GCC and Clang keep that for `(e, bf)` and `(bf = v)`; bcc gives those expressions the declared type `unsigned int`, so `((0, b.f) & 1L) >= -7L` becomes an unsigned comparison.

- Reproducer: [`bitfield_promotion_comma.c`](conformance-reproducers-2026-10-13/bitfield_promotion_comma.c). GCC and Clang print `1 1 1 1` and `1`; bcc prints `0 1 0 1` and `0` on the Windows target (32-bit `long`); on Linux only the last line differs. A conditional operator and a plain parenthesized member are fine.
- Arms: all five, both targets. Found by Csmith seeds 300681 (wrong checksum in all arms; localized by printing the diverging global and then the statement `l_569 &= (g_375.f1 = (((..., g_397.f3) & 1L) >= (-7L)))`, where `g_397.f3` is a volatile 17-bit unsigned field) and 100167 (float programs; the first diverging value comes from `(..., g_512[4][1].f6) > l_1675[1]`, a 29-bit unsigned field against a `short`, which has the same shape; the program itself was not re-run with a fixed compiler).
- Component: semantic analysis typing (`semantic_analysis/expressions/typing.rs`, `promote` reads `ExpressionInfo::bit_field`, which the comma and assignment result infos drop).
- Fix direction: carry `bit_field` into the result of the comma operator (from the right operand) and of simple and compound assignment (from the left operand). Strictly, C99 6.3.1.1p2 speaks of "a bit-field", and a comma or assignment result is not one; GCC and Clang nevertheless behave as above and Csmith relies on it, so this is a compatibility defect with an ambiguous standard reading.

### Programs on which the reference compilers disagree (not attributed to bcc)

**D1. Csmith seeds 1040 (`nopointers`) and 100526 (`float`).** Seed 1040: GCC and bcc print `1C49D650`, Clang prints `83F81F84`, in every arm. Seed 100526: Clang `33C20859`, GCC `77A44B39`, bcc A0/A1 `118442BB`, and A2 passes. The programs lean on behavior the standard leaves open, or on a bug in one of the reference compilers, so they are not evidence against bcc. Seed 100526 is a float program, and Csmith's float mode has unguarded float-to-integer conversions (I3), so undefined behavior is the likely cause there. Both are kept in `target/conformance/*/failures/` for later diagnosis; the A2-passes-but-A0/A1-fail pattern of 100526 is worth a look if the cause turns out to be bcc's, because LLVM's optimizer would be hiding it.

### Compiler panics, abnormal exits, link errors, timeouts

None in any arm of any program.

### Interpreter

- **I1. Step limit.** `the program exceeded its step limit` (100,000,000 steps, fixed in `interpreter/outcome.rs`) stops 252 of 4,536 YARPGen programs on Windows, 52 of 1,031 on Linux and 9 Csmith programs; `--opt=bcc` more than halves the YARPGen count (109 and 22). Fix direction: a command-line limit.
- **I2. No host function for `fabsf`** (and other float libm functions): `call to @fabsf, which has no definition` stops 368 Csmith float programs; native arms pass. Reproducer: [`interpreter_fabsf.c`](conformance-reproducers-2026-10-13/interpreter_fabsf.c). Fix direction: add the C99 `<math.h>` set to the host table, or lower `fabsf` to an IR intrinsic.
- **I3. Poison traps on float programs (explained, not a bug).** Six Csmith float programs (100265, 100337, 100413, 100549, 100590, 400136) stop in the interpreter with `undefined behaviour: branch on poison` or `poison reached a noundef parameter or return`. Csmith's float mode converts floats to integers without a range check (its `safe_convert_func_float_to_*` helpers are not used), and an out-of-range `fptosi` is poison: `volatile float f = 3e10f; int i = (int)f;` stops the same way, which is correct for C (undefined behavior), while Clang and the native arms just produce `INT_MIN`. Seed 100265 stops in `func_82` at a branch on a value derived from `g_81`; I did not trace the conversion itself. Treat Csmith float programs as not UB-free.
- **I4. `setjmp` on the Windows target** works natively but the interpreter has no host function for `mingw_getsp` ([`setjmp_linux.c`](conformance-reproducers-2026-10-13/setjmp_linux.c)).

### Refusals of common constructs

Ranked by how much of real code they block. "Programs" is the number of programs in this campaign that hit it first.

| # | Construct | What bcc says | Where it matters | Reproducer |
| --- | --- | --- | --- | --- |
| R1 | MinGW-w64 `<math.h>` (inline `fabsf`/`fabsl`: `__builtin_fabsf`, x87 inline assembly) | `type was not analyzed`; `inline assembly` | every Windows program that includes `<math.h>`; avoided here with `-D__CRT__NO_INLINE` | [`mingw_math_h.c`](conformance-reproducers-2026-10-13/mingw_math_h.c) |
| R2 | `#include <windows.h>`: `winnt.h` includes `<x86intrin.h>`/`<emmintrin.h>`; bcc lowers every static inline function of its built-in `mmintrin.h` although none is called | `lowering: vector types` | every Win32 program; SQLite (`sqlite3.c`, `shell.c`) and Lua (`lua.c`, `loadlib.c`) on Windows | [`windows_h_vector_types.c`](conformance-reproducers-2026-10-13/windows_h_vector_types.c) |
| R3 | Pointer constraint violations Clang and GCC accept with warnings: comparing or assigning `int *` with `unsigned *`, `T **` with `const T **`, passing `&float` to `char *` | `invalid operand types for comparison` (2,920 programs), `incompatible assignment conversion` (246), `function argument cannot be converted` (136), `return expression cannot be converted` (7) | 83% of default Csmith programs; all 80 sampled refusals fail under Clang `-Werror=compare-distinct-pointer-types -Werror=incompatible-pointer-types -Werror=pointer-sign` | [`pointer_sign_comparison.c`](conformance-reproducers-2026-10-13/pointer_sign_comparison.c) |
| R4 | `INFINITY`, `NAN`, `HUGE_VAL`: glibc and MinGW define them with `__builtin_inff()`, `__builtin_nanf()`; the typed math builtins are patchy (`floor`, `fabs`, `inf`, `isinf`, `signbit`, `copysign` work; `ceil`, `sqrt`, `sin`, `pow`, `exp`, `log`, `round`, `trunc`, `fma`, `fmin`, `fabsf`, `inff`, `isnan`, `rotateleft32` do not) | `type was not analyzed` | SQLite `sqlite3.c` and `shell.c` on Linux, any numeric code | [`infinity_macro.c`](conformance-reproducers-2026-10-13/infinity_macro.c), [`math_builtins.c`](conformance-reproducers-2026-10-13/math_builtins.c) |
| R5 | `__PRETTY_FUNCTION__`, which glibc's `assert()` expands to (`__func__` works) | `identifier has no visible declaration` | every `assert()` with glibc headers; SQLite `shell.c` on Linux | [`pretty_function_assert.c`](conformance-reproducers-2026-10-13/pretty_function_assert.c) |
| R6 | A function designator initializing a `void *` member of a static object (a GNU extension) | `initializer cannot initialize this object` | SQLite's math function table (`SQLITE_ENABLE_MATH_FUNCTIONS`) | [`function_to_void_pointer_initializer.c`](conformance-reproducers-2026-10-13/function_to_void_pointer_initializer.c) |
| R7 | `setjmp` on the Linux target (glibc `_setjmp`) | `lowering: setjmp` | Lua `ldo.c` (error handling) on Linux; works on the Windows target | [`setjmp_linux.c`](conformance-reproducers-2026-10-13/setjmp_linux.c) |
| R8 | Labels as values in a static table (`&&label`, `goto *p`) | `lowering: this static initializer` | Lua `lvm.c` (the default dispatch loop) | [`computed_goto_table.c`](conformance-reproducers-2026-10-13/computed_goto_table.c) |

Likely owners and fix directions: R1 and R2 are the same issue seen from two headers (lower only the static inline functions that are reachable, as Clang does; for R1 also type `__builtin_fabsf` and accept `__asm__` in an unreferenced body); R3 is a policy decision (a permissive mode that downgrades the three constraint violations to warnings, as GCC and Clang do by default) rather than a bug; R4 needs the missing builtins typed and lowered to intrinsics or libm calls (an inventory beats case-by-case); R5 is one more predefined identifier next to `__func__` (`lowering/module_items.rs`); R6 is constant evaluation of an address constant through a function-pointer-to-`void *` conversion; R7 and R8 are the existing setjmp and computed-goto roadmap items.

### Refusals by frequency

Inline assembly, complex numbers, vector types in user code and variable length arrays did not occur in the generated programs; inline assembly and vector types reached bcc only through system headers (R1, R2). YARPGen uses statement expressions in its `min`/`max` macros and bcc accepts them. All refusals seen, by number of programs or translation units refused:

| Rank | Refusal | Programs or units |
| ---: | --- | ---: |
| 1 | pointer constraint violations (R3) | 3,309 programs |
| 2 | `fabsf` missing in the interpreter (I2) | 368 programs (interpreter only) |
| 3 | vector types via `windows.h` (R2) | 4 translation units (Lua 2, SQLite 2) |
| 4 | `setjmp` on Linux (R7) | 1 unit |
| 5 | label addresses in a static table (R8) | 1 unit |
| 6 | `INFINITY`/`__builtin_*` math (R4) | 2 units (SQLite on Linux) |
| 7 | `__PRETTY_FUNCTION__` (R5) | 1 unit (Linux) |
| 8 | function-to-`void *` initializer (R6) | 1 unit |
| 9 | MinGW `<math.h>` inline bodies (R1) | every program that includes it |

## Real programs

`scripts/run_program_corpus.py` compiles each translation unit to an object file with Clang `-O0` (reference) and with bcc (A0, A1, A2), links with the bundled Clang (WSL's on Linux), and runs the fixed workloads. A translation unit bcc refuses is replaced by its Clang object and the build is marked *mixed*; the first diagnostic is kept. zlib is also run through both interpreters as one translation unit (`gz*.c` excluded, they share the macro name `COPY` with `inflate.c`; a small variant of the round-trip fits the interpreter's step limit). Workloads: zlib `example.c` plus a 200-line round-trip driver (levels 0-9, every strategy, window bits, streaming one byte at a time); the Lua interpreter on six scripts (closures, tables, strings, coroutines, numeric edge cases, a mixed workload; 317 lines); the SQLite shell and a C driver over one 113-line SQL script (tables, indexes, joins, window functions, CTEs, triggers, floating point, strings, transactions, savepoints, VACUUM, ANALYZE) in memory and on a file. Sources: SQLite amalgamation 3.54.0, Lua 5.4.9, zlib 1.3.2.

| Program (target) | Units built by bcc | Refused units | Workloads, A0 / A1 / A2 |
| --- | --- | --- | --- |
| zlib (Windows) | 18 of 18 | none | 3 of 3 pass in every arm; `roundtrip_small` also passes in both interpreters |
| zlib (Linux) | 18 of 18 | none | same |
| Lua (Windows) | 31 of 33 | `lua.c`, `loadlib.c` (R2); `lvm.c` only with the jump table (R8) | 6 of 6 pass in every arm (mixed with Clang objects for the two units) |
| Lua (Linux) | 32 of 33 | `ldo.c` (R7); `lvm.c` only with the jump table (R8) | 6 of 6 pass in every arm (mixed for `ldo.c`) |
| SQLite (Windows) | 1 of 3 (the driver) | `sqlite3.c`, `shell.c` (R2); `sqlite3.c` with math functions (R6) | all pass, but only with Clang's `sqlite3.c`, so they test nothing of bcc |
| SQLite (Linux) | 2 of 3 | `shell.c` (R4); `sqlite3.c` with math functions (R6) | 5 of 5 pass in A0, A1 and A2, with `sqlite3.c` compiled by bcc |

The Linux SQLite result is the strongest positive finding: bcc compiles the entire 9.6 MB amalgamation and the workload output is identical for A0, A1 and A2, with and without the bcc optimizer. Lua is built with `LUA_USE_JUMPTABLE=0` so the rest of `lvm.c` is tested; the refusal is recorded by a separate `lua-jumptable` entry, likewise `sqlite-math` for the math function table.

## What was not run, and why

- **Csmith in the default profile** is refused for 83% of programs (R3), so its coverage of back-end code is the 686 programs that pass the front end. Four other profiles remove pointers (and packed structures, unions, floats) to reach deeper; they exercise arrays, structs, bit-fields, unions, volatile, floats and integer arithmetic but not pointer-heavy code.
- **Struct passing by value and varargs** are not generated by either tool, so the System V and Windows ABIs are not tested beyond scalars.
- **Linux** covers YARPGen (1,031 programs) and one Csmith profile (326); WSL has no `m4`, so Csmith was built once on Windows and its programs were compiled for the Linux target. The Linux interpreter arms use glibc headers copied from WSL; native Linux arms use bcc objects linked and run in WSL. bcc itself runs only on Windows.
- **A1 versus A2 differences** were never observed, so no optimizer pass is implicated; a larger or differently shaped corpus (loops over pointers, aliasing, bit-fields in optimized code) is needed to stress the bcc optimizer.
- **Reductions** (`scripts/reduce_conformance.py`, a line and parenthesis delta reducer standing in for C-Reduce) were slow on 100 KB Csmith files under load (a thousand-line program needs hundreds of 5-second candidates). W1 and W2 were therefore isolated by hand (printing the diverging global, instrumenting the assigning statements, testing the suspect construct in isolation) and then written as small programs; the reductions of the W2 programs were abandoned once the cause was known. The reduction of the poison trap (I3) was stopped once an unguarded float-to-integer conversion was found to explain it.

## Reproducing

```powershell
cargo build --release
python scripts/random_program_conformance.py build-generators
python scripts/random_program_conformance.py run --output target/conformance/first --generators csmith,yarpgen --count 500 --time-budget 3600 --jobs 12 --csmith-option=--no-pointers
python scripts/random_program_conformance.py run --output target/conformance/linux --platform linux-wsl --generators yarpgen --count 500
python scripts/random_program_conformance.py rerun target/conformance/first csmith 95
python scripts/run_program_corpus.py --output target/conformance/programs
python scripts/run_program_corpus.py --output target/conformance/programs-linux --platform linux-wsl
python scripts/reduce_conformance.py test.c --arm A0 --kind wrong-output --no-preprocess -I <csmith runtime> -I target/generators/build/csmith/runtime --output reduced.c
python scripts/test_mini_m4.py; python scripts/test_conformance_common.py; python scripts/test_random_program_conformance.py; python scripts/test_run_program_corpus.py; python scripts/test_reduce_conformance.py
```

Each run directory records the bcc hash, generator versions, options and deadlines in `manifest.json`; a second `run` with the same command resumes and skips finished seeds, and refuses to continue if the options or the bcc build changed. A harness failure (for example Windows refusing to put a process in its job object, or an executable still locked) becomes a `harness-error` row and is retried on resume instead of ending the run.

## 2026-10-14 follow-up: pointer diagnostic defaults (issue #46)

The maintainer decision is to follow the bundled Clang's defaults for each
pointer diagnostic group. The original R3 description above predates this
measurement: **Clang 23 makes incompatible object pointer conversions errors**,
including `float *` passed to `char *`. This change therefore accepts signedness,
qualifier and comparison warnings while retaining incompatible object/function
pointer and integer/pointer conversion errors. It does not introduce a general
permissive mode. [Issue #46](https://github.com/BenBwall/bcc-rust/issues/46) was read
through the GitHub connector after the local `gh` credential configuration was
inaccessible.

### Oracle and implementation

Bundled Clang 23.1.1, Rust LLVM revision
`1b9c0d5ff9bbe7634aead059efe6b11a7eeba145`, was invoked with `-fsyntax-only`
for 25 probes across Linux GNU, Windows GNU and Windows MSVC and every supported
ISO/GNU dialect (13 spellings, including C95 and C2y). The 975 default probes
had identical severities, groups and wording across targets and dialects.
Another 90 probes checked global and group warning options. The final release
bcc binary matched Clang's diagnostic severities and groups on all **1,065 probes**.
Source, commands and complete stderr are retained in
`target/pointer-diagnostics/{matrix,flags}.json`; the comparison is in
`target/pointer-diagnostics/bcc-comparison.json`. The exact Clang wording and
option precedence are recorded in
[language-standards.md](../../language-standards.md#pointer-diagnostic-defaults-2026-10-14-issue-46).

| Constraint | Clang group | Default |
| --- | --- | --- |
| Distinct object/function pointer comparison | `compare-distinct-pointer-types` | warning |
| Incompatible object pointer assignment, initialization, argument, return | `incompatible-pointer-types` | error |
| Direct pointer signedness change | `pointer-sign` | warning |
| Discarded qualifiers, including nested `T **` / `const T **` | `incompatible-pointer-types-discards-qualifiers` | warning |
| Incompatible function signatures | `incompatible-function-pointer-types` | error |
| Integer/pointer conversion | `int-conversion` | error |
| Pointer/integer equality | `pointer-integer-compare` | warning |
| Function pointer ordering | `ordered-compare-function-pointers` | warning |
| Pointer/integer ordering | unnamed | warning |

Ordering distinct function pointer types emits both comparison warnings, in
Clang's order. A regression suppresses each group independently while promoting
the other to an error; an initial focused test reproduced the missing diagnostic
before the emitter was corrected.

Function/void pointer conversions and pointer/zero ordering are silent by
default and retain the existing pedantic extension policy. Clang does not
recognize GCC's `-Wdiscarded-qualifiers` spelling. Use its qualifier group above.

One semantic conversion classifier selects the group; shared configuration
policy selects severity. Accepted or suppressed diagnostics retain ordinary
conversion facts, so IR and lowering are unchanged. `-Werror`, `-Wno-error`,
the eight named groups' enable/disable and error/no-error options, parent/child
overrides and `-w` are tested. `-pedantic-errors` retains its existing Deny
policy. C99 requires a diagnostic rather than rejection: §5.1.1.3p1,
§6.5.16.1p1, §6.5.9p2, §6.5.8p2, §6.5.2.2p2/p7, §6.8.6.4p3 and §6.7.8p11.
Semantic tests and five golden fixture families cover every constraint group,
default errors and promotion/demotion. Execution regressions check actual
values in both interpreters and A0/A1/A2, including the float-to-char argument
with explicit `-Wno-error=incompatible-pointer-types`.

### Fresh default Csmith batch

Csmith 2.4.0 (`0cdc710`), default options, seeds **600001–600360**, Windows GNU,
eight jobs, no time budget. The unchanged compiler at branch base
`6118c37be1fa6dc6b52424f2e74f26a8b1f3702f` was built before implementation and
preserved as `target/bcc-pointer-baseline.exe`. The same seed batch was run with
that binary and the final release compiler. Manifests record these SHA-512s:

```text
baseline a3f9d9ae9e02af84835ec871ffcee3c5938b120c42ca8068184c0643a11b715690fcdcce2d7395bc5d1ce14e4829b49bc407a71a4966b81500ffba7c4b60ce67
final    09f9c7959b5121a36578df594f01a01b288f58abaf21e36ec0053bfb9372740fb3f1b465641045371d724ef1b7676b9f28c2f4a7dd42db714b92d11e6e05068c
```

**Front-end acceptance rose from 68/360 (18.9%) to 360/360 (100%).** The baseline
refused 292 programs: comparison 257, assignment 22, argument 12, return 1.
Its 67 reference-tested programs passed every arm; another accepted seed had
a reference failure. The harness checks the front end before running Clang,
so its baseline summary's `359 compared` includes 292 refusals and must not
be interpreted as 359 successful reference comparisons.

With the final compiler, **310 programs reached reference comparison**. Fifty
were excluded because execution of the Clang O0 reference programs timed out
(the configured reference-run deadline is 30 seconds, with five seconds of wrapper grace)
(`clang -O0: exit None timeout True`); they passed bcc's front-end probe but
provide no execution-conformance evidence. Final outcomes:

| Outcome | interpret | interpret-bcc | A0 | A1 | A2 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Matches Clang | 299 | 299 | 309 | 309 | 309 |
| Misaligned-access trap | 10 | 10 | 0 | 0 | 0 |
| Wrong checksum against Clang | 1 | 1 | 1 | 1 | 1 |
| Front-end/lowering/ABI/link refusal | 0 | 0 | 0 | 0 | 0 |
| bcc timeout/crash/abnormal exit | 0 | 0 | 0 | 0 | 0 |

There is **no claim of zero disagreements**. Seed **600300**, a 219,974-byte
program, produces `checksum = 9ADEB394` with Clang O0/O2 and
`checksum = 2AAC662E` in all five bcc arms. MinGW GCC 13.2.0 O0 exits 0 with
`checksum = 2AAC662E`, agreeing with bcc. This is an unresolved reference
disagreement, not a verified attribution to either compiler. The unreduced
source and all arm evidence are under
`target/conformance/pointer-warnings-verified/failures/csmith/600300`; source
SHA-512 is
`a41ab13a2acf96f60fcb3121ae672e75c5e31faf380c42e72c505917d4683fe011b4ad9f36cc53740238b2c201761e5944c5f5f6db7fcb285a631e67631c0221`.

Both interpreters report `undefined behaviour: misaligned access` for these
ten seeds; all three native bcc arms match Clang's checksum. This report does
not classify the sources as undefined solely from the interpreter diagnosis.
Each seed's original source, commands and stderr are retained under the same
failure directory, and every row is retained in `results.jsonl`.

| Seed | Clang and native bcc checksum |
| ---: | --- |
| 600025 | `CEAC978F` |
| 600081 | `44369851` |
| 600146 | `3A323E76` |
| 600150 | `CB9BE1DA` |
| 600166 | `20072564` |
| 600229 | `2015DDB1` |
| 600305 | `84A1699D` |
| 600312 | `E585252A` |
| 600337 | `4881A5DD` |
| 600343 | `4FE9CF40` |

The final run took 1,177.8 seconds. Preceding candidate runs gave the same counts; the final run uses the frozen
compiler after removing a duplicate retained value conversion and correcting
the dual function-pointer comparison diagnostics. Raw baseline and final evidence is in
`target/conformance/pointer-baseline` and
`target/conformance/pointer-warnings-verified`. The random-program harness exits
0 after writing its summary even with defect candidates; the per-arm results,
not that exit status, establish conformance.

### GCC torture comparison

The requested GCC 15.2.0 execute survey was run on all **1,677 sources**, eight
jobs, ten-second timeout, with both binaries. Source SHA-512s were compared
before comparing every arm's status. **No previously passing source/arm pair
regressed after rechecking four compilation timeouts; 22 pairs gained a pass
across six sources.** Final pass counts below include those focused rechecks.

| Arm | Baseline passes | Final passes | Wrong results, baseline → final |
| --- | ---: | ---: | --- |
| interpret | 1,302 | 1,307 | 12 → 12 |
| interpret-bcc | 1,308 | 1,313 | 13 → 13 |
| A0 | 1,365 | 1,371 | 13 → 13 |
| A1 | 1,355 | 1,361 | 23 → 23 |

`20001017-2.c`, `20041212-1.c`, `920429-1.c`, `980506-3.c` and `980617-1.c`
changed from front-end error to pass in all four arms. `20121108-1.c` changed
from front-end error to native pass in A0/A1 and an interpreter poison trap
in both interpreter arms. Its signed `int` address construction repeatedly
shifts left by eight to form `0xc0a8bea0`, exceeding signed representability;
C99 §6.5.7p4 (p. 85; PDF p. 97) makes that signed shift undefined. This is
newly reached execution, not a passing-test regression. `pr17377.c` changed
from front-end error to existing lowering rejection (`type was not analyzed`)
in every arm.

Front-end refusals fell from 137 to 130 per arm. Lowering refusals rose from
152 to 153; interpreter traps rose from 68/61 to 69/62. ABI refusals stayed
at six per arm; native link errors stayed at four per native arm. There were
zero compiler crashes in either survey. The baseline had zero timeouts; the
final eight-worker run recorded four A1 compilation timeouts and 1,357 A1
passes. Focused one-worker reruns used the same ten-second deadline and
identical source/compiler hashes; every affected source passed all four arms.
The original timeout rows remain unchanged in the full-run evidence:

| Source | Full-run A1 compilation, seconds (timeout) | Recheck compilation | Recheck execution |
| --- | ---: | ---: | ---: |
| `20120808-1.c` | 10.3452 | 2.3615 | 0.2339 |
| `pr82387.c` | 10.1051 | 1.0421 | 0.2163 |
| `pr82388.c` | 10.0591 | 1.3617 | 0.1777 |
| `pr82524.c` | 10.0861 | 1.3407 | 0.2328 |

These full-run timeouts occurred alongside Cargo builds; the successful
rechecks support a transient timing failure rather than a reproducible source
regression. Recheck evidence is in
`target/compiler-corpus/execute-pointer-timeout-recheck{,-late}`.
The raw disagreement count rose from 70 to 75; after the rechecks it is 71,
with the remaining increase due to `20121108-1.c`. Both full survey commands
exit 1 for the retained failures; both focused reruns exit 0. Per-source evidence is in
`target/compiler-corpus/execute-pointer-{baseline,warnings-verified}/results.json`;
the hash-checked comparison is in
`target/compiler-corpus/pointer-comparison-verified.json` (28 changed statuses
after rechecks, plus the four original timeout rows and their recheck evidence).

### Reproducing the follow-up

```powershell
python scripts/probe_pointer_diagnostics.py
python scripts/probe_pointer_diagnostics.py --flags-only
python scripts/random_program_conformance.py run --output target/conformance/pointer-baseline --bcc target/bcc-pointer-baseline.exe --generators csmith --count 360 --seed-start 600001 --jobs 8 --platform windows
python scripts/random_program_conformance.py run --output target/conformance/pointer-warnings-verified --generators csmith --count 360 --seed-start 600001 --jobs 8 --platform windows
python scripts/random_program_conformance.py rerun target/conformance/pointer-warnings-verified csmith 600300
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --bcc target/bcc-pointer-baseline.exe --output target/compiler-corpus/execute-pointer-baseline --jobs 8 --timeout 10
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-pointer-warnings-verified --jobs 8 --timeout 10
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-pointer-timeout-recheck --jobs 1 --timeout 10 --match 20120808-1.c
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-pointer-timeout-recheck-late --jobs 1 --timeout 10 --match pr82524.c --match pr82388.c --match pr82387.c
```

The original requested output name, `execute-pointer-warnings`, contains a
preceding candidate survey; `execute-pointer-warnings-verified` verifies the
final release binary. The harness requires a fresh output directory for each
run. Its confirmed outcomes match the preceding survey after the focused
timeout rechecks. LLVM junctions
were preserved. Concurrent Windows harness changes were preserved separately
and are excluded from the pointer-warning commit plan.

### Final validation

All requested checks passed on Windows. The formatter ran before the checks.
The full suite passed 1,695 library tests (three existing ignored tests), and
the five pointer CLI tests were rerun successfully after a lint-only assertion
correction. The allocation suite passed all 28 feature-enabled tests. No
benchmarking-internals build overlapped CLI integration tests. The plain CLI
was rebuilt after all feature-enabled checks. Linux Clippy was not run on this
Windows host; the new compiler policy code has no host-specific branches.

| Command | Result |
| --- | --- |
| `cargo test --all-targets` | pass |
| `cargo test --features benchmarking-internals --test allocation_count` | pass |
| `cargo +nightly fmt --check` | pass |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `cargo clippy --release --all-targets -- -D warnings` | pass |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | pass |
| `git diff --check` | pass |
| `python scripts/test_random_program_conformance.py` | pass, 19 tests |
| `cargo build --bin bcc-rust` | pass, plain CLI restored |

Check logs are under `target/pointer-{all-targets,allocation}-complete.log`,
`target/pointer-cli-final.log`, `target/pointer-clippy-{debug,release,feature}-final.log`,
`target/pointer-fmt-final.log`, `target/pointer-harness-complete.log` and
`target/pointer-plain-cli-final.log`. The whole-file commit plan is
`target/commit-plan.md`; no files were staged or committed.
