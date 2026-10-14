Research date: 2026-10-14.

Second conformance campaign for bcc, run against the merge of `claude/semantic-facts-m2` (semantic-facts M1-M3) into the conformance branch that carries the round-1 fixes (`#pragma pack` retention and bit-field result promotion, commit 9d019dc8). Round 1 is [`conformance-testing-2026-10-13.md`](conformance-testing-2026-10-13.md); this report uses its arms, tools and terminology. No compiler source was changed; reproducers are under [`conformance-reproducers-2026-10-14/`](conformance-reproducers-2026-10-14/).

> Status: **in progress.** Sections marked *pending* are filled as the runs finish; the findings below are final unless noted.

## Summary

- **The merge** (535fb1a3) keeps both sides' meaning: the GCC torture survey passes every (source, arm) pair that either parent passed, and every canonical check passes. The round-1 wrong-code defects W1 (`#pragma pack`) and W2 (bit-field promotion) are fixed in the merged compiler: their reproducers now print Clang's output.
- **Random programs.** In the runs finished so far, every arm agrees with Clang on every Csmith and YARPGen program the front end accepts, except two Csmith programs on which the *reference* is wrong: the bundled Clang 23.1.1 miscompiles `#pragma pack(1)` records containing a zero-width bit-field on the MinGW target (its fields overlap); MinGW GCC and bcc agree with each other and with the source.
- **New defects found by targeted generators and probes** (the random generators do not exercise these):
  1. *Wrong code (ABI layout), Windows target:* Microsoft bit-field layout ignores the rule that a zero-width bit-field closes the current allocation unit. `struct { int a : 5; short : 0; char b : 6; }` is 4 bytes in bcc and 8 in Clang and GCC.
  2. *Wrong code, both targets:* the GNU attributes `cleanup`, `constructor` and `destructor` are accepted without a diagnostic and do nothing; `weak` is dropped and `alias` emits no symbol.
  3. *Link failure, Windows target:* any program with two translation units that include MinGW `<string.h>` (without `-D__CRT__NO_INLINE`) fails to link with duplicate `strncasecmp`/`strcasecmp`: bcc reports GCC 4.2 without `__clang__`, so MinGW selects `extern __inline__`, which bcc compiles with C99 semantics.
  4. *Compiler panic:* the `known-bits` optimizer pass indexes past its fact table when a rewritten instruction's new constant reaches an equality condition (`--passes known-bits`; the default pipeline folds the pattern first). Found by the IR differential tester's 20,000-case sweep.
  5. *Refusals of valid code:* `u""`/`U""` literals cannot size an array; static initialization of a flexible array member (GNU); `transparent_union` (GNU); thread-local storage (`__thread` is unknown and `_Thread_local` is not lowered); atomics (`_Atomic`, `<stdatomic.h>`, `__atomic_*`, `__sync_*`) are not lowered.
- **What held up:** 700 random functions exchanging aggregates (packed, over-aligned, nested, unions, `long double`, variadic) between bcc and Clang objects agree in all seven caller/callee mixes on both targets; 1,500 random records without zero-width bit-fields and 1,400 System V records have Clang's layout byte for byte.

## Part 1: the merge

### Conflict resolution

Four files conflicted. Both sides changed `ExpressionInfo` and the analyzer state:

| File | conformance side (9d019dc8) | semantic-facts side (M3) | Resolution |
| --- | --- | --- | --- |
| `semantic_analysis.rs` | adds `Analyzer::pack_alignments` | realigns the `Analyzer` fields for the new fact builders | the M3 layout with `pack_alignments` added after `types` |
| `semantic_analysis/expressions.rs` | adds `ExpressionInfo::promotion_width` (one byte) | deletes the relative `field` index (now `facts::resolved::MemberFacts`) | keep `promotion_width`, drop `field` |
| `expressions/typing.rs` (member typing and `expression_result`) | sets `promotion_width` from the member's width | records `MemberFacts`, stops setting `field` | keep the `MemberFacts` insertion and `promotion_width`, drop `field` |
| `semantic_analysis/tests.rs` | `mod pragma_pack;` | `mod resolved;` | both |

Every other hunk merged cleanly, including both sides' documentation in `semantic-analysis.md` (§ on `promotion_width` and the deleted relative field index). `ExpressionInfo` remains 160 bytes (`tests/expressions.rs` pins it). No other code read the deleted `field` index or its `field_index` accessor.

### Checks after the merge

| Check | Result |
| --- | --- |
| `cargo test --all-targets` | pass: 1,708 library tests (3 ignored) and every integration target |
| `cargo test --features benchmarking-internals --test allocation_count` | pass (30) |
| `cargo +nightly fmt --check`, `git diff --check` | pass |
| `cargo clippy --all-targets -- -D warnings`, the `--release` and `benchmarking-internals` variants | pass |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | pass (6; the two ignored long sweeps excluded) |
| the same with `--include-ignored` | the 20,000-case optimizer sweep fails on seed `0x1bcc111f2`: finding C1 below. Neither side changed `src/optimizer`, `src/backend` or `src/ir` since the merge base, so the failure predates the merge |
| `python scripts/test_random_program_conformance.py` | pass (19) |
| `cargo test --lib abi::tests::conformance` (System V halves run in WSL) | pass (20), including the new `pragma_packed_struct_abi_exchange_matches_clang` |

### GCC torture survey

`python scripts/run_gcc_torture_execute.py --source …\gcc-15.2.0 --output target/compiler-corpus/execute-merge --jobs 6 --timeout 10` (1,677 sources, four arms), compared per source and arm with the parents' surveys `execute-frontend-fixes` (conformance side) and `execute-m3` (semantic-facts side):

| Arm | merge | conformance parent | semantic-facts parent |
| --- | ---: | ---: | ---: |
| interpret | 1,302 | 1,302 | 1,301 |
| interpret-bcc | 1,308 | 1,308 | 1,307 |
| A0 | 1,365 | 1,365 | 1,364 |
| A1 | 1,355 | 1,355 | 1,354 |

No (source, arm) pair that either parent passed fails in the merge; no pair passes that neither parent passed. No compiler crash. The remaining A1-only wrong results (`20020508-2/3`, `20040409-{1,2,3}w`, `950704-1`, `pr22493-1`, `pr40386`, `pr43629`, `pr99079`) are the undefined-behavior or `-fwrapv` sources already explained in `gcc-torture-execute-results.md`.

## Part 2: methodology

Everything ran against one release build copied to `target/conformance-bcc/bcc-rust.exe` (the merge commit; SHA-512 prefix `a20da115214d3e680915`), with `--jobs 6`. Arms, deadlines, reference compilers (bundled Clang 23.1.1 at `-O0`, confirmed by `-O2`; MinGW GCC 13.2.0 and WSL GCC/Clang as second opinions), headers and generator builds (Csmith 2.4.0, YARPGen 2.0, the round-1 builds copied from the `bcc-rust-ir-fixb` worktree) are those of round 1. New seeds were used throughout (Csmith/YARPGen from 1,000,000 on Windows and 2,000,000 on Linux) so no program repeats round 1.

Profiles:

| Run | Generator and options | Why |
| --- | --- | --- |
| `r2-win-default` | Csmith default, YARPGen default (`--std=c --max-array-dims=2 --emit-pragmas=none`), 1,000 each | refusal rate and the YARPGen baseline |
| `r2-win-nopointers` | Csmith `--no-pointers`, 1,000 | avoids the pointer-constraint refusals (#46) that hide 80% of default Csmith programs; packed structures and bit-fields stay on, since W1/W2 are fixed |
| `r2-win-yarpgen-mutate` | YARPGen with `--mutate=all --allow-dead-data=true --expl-loop-param=true --max-array-dims=3`, 1,000 | more varied expression shapes |
| `r2-linux-default` | Csmith default and YARPGen default for `x86_64-unknown-linux-gnu`, 1,000 each | System V target |
| `r2-linux-nopointers` | Csmith `--no-pointers`, Linux, 600 | as on Windows |
| `r2-programs`, `r2-programs-linux` | SQLite, Lua, zlib through every arm | real programs |

Round 1 noted that the generators exercise neither aggregate passing nor unusual record layouts, and the merge changed exactly those front-end facts (packing, members, designations). Three small generators written for this campaign (kept in `target/r2scratch/`, not committed) cover them:

- **Layout fuzzer:** random structures and unions (scalars, arrays, bit-fields of every integer type including zero-width and unnamed ones, `#pragma pack(1|2|4|8)` or `__attribute__((packed))`); the program prints `sizeof`, `_Alignof` and, for each named member, the byte image after storing all ones into it. bcc (A0) is compared with Clang and GCC.
- **ABI exchange fuzzer:** random aggregates (nested records, arrays, unions, `_Bool`, `float`, `double`, `long double`, packed and over-aligned records) passed by value and returned, mixed with scalars and, in 30% of the functions, variadic aggregate arguments read with `va_arg`. Caller and callee are separate translation units, built by Clang, by bcc (A0) and by bcc (A1); seven mixes are linked and must print what Clang/Clang prints.
- **Feature probes:** about 60 short programs for constructs real code uses (GNU attributes, TLS, atomics, `__int128`, overflow and bit builtins, `alloca`, `_Generic`, K&R definitions, local labels, asm labels, `__has_include`, unicode literals, flexible array initializers, large switch values, conversions), run through all five arms on both targets.

Disagreements were triaged by printing Csmith's per-variable checksums (`./program 1`) for Clang, GCC and bcc, then isolating the first diverging variable's type or statement by hand; the reproducers are hand-minimized programs that show the defect directly. Optimizer involvement was checked by comparing A0 (no optimizer) with A1 (bcc optimizer); `--passes`/`--opt-bisect-limit` were needed only for C1.

## Counts per generator, target and arm

Programs "compared" are those Clang can run reproducibly; front-end refusals are counted in the arm tables.

*Windows, Csmith default (1,000 programs, 996 compared; 4 discarded because Clang could not run them):* all five arms pass 191 and refuse 805 (pointer constraint violations, #46: comparison 695, assignment 64, argument 43, return 3). No other outcome.

*Windows, YARPGen default (1,000 compared):* A0, A1 and A2 pass all 1,000; the interpreter passes 953 and `interpret-bcc` 981, the rest stopping at the step limit (#52).

*Windows, Csmith `--no-pointers` (1,000 programs, 795 compared; 205 discarded, almost all because Clang's run exceeds 30 s):*

| Outcome | interpret | interpret-bcc | A0 | A1 | A2 |
| --- | ---: | ---: | ---: | ---: | ---: |
| pass | 792 | 793 | 793 | 793 | 793 |
| interpreter trap (step limit, #52) | 1 | 0 | 0 | 0 | 0 |
| wrong output (reference defect D1) | 2 | 2 | 2 | 2 | 2 |

*Pending:* YARPGen mutate (Windows), Linux runs, real programs.

## Findings, ranked

Severity order: wrong code, then crash/ICE, then refusal of valid code, then diagnostics. "Known" issues (#45-#52) are counted but not re-reported.

### W3. Microsoft bit-field layout: a zero-width bit-field does not close the allocation unit (wrong code, Windows)

- Reproducer: [`ms_zero_width_bitfield_layout.c`](conformance-reproducers-2026-10-14/ms_zero_width_bitfield_layout.c). Clang and MinGW GCC put the member after a zero-width bit-field at the end of the preceding unit; bcc puts it at the zero-width type's alignment boundary inside the unit:

  | Record | Clang | GCC | bcc |
  | --- | --- | --- | --- |
  | `{ int a : 5; short : 0; char b : 6; }` | size 8, `b` at byte 4 | 8, 4 | **4, 2** |
  | `{ long long a : 15; unsigned char : 0; int b; }` | 16, 8 | 16, 8 | **8, 4** |
  | `#pragma pack(1) { char a; unsigned b : 18; unsigned : 0; unsigned c : 4; }` | 8, 5 | 9, 5 | **8, 4** |
  | `#pragma pack(1) { long long a : 27; char : 0; }` | 8 | 8 | **4** |
  | `#pragma pack(4) { long long a : 9; _Bool : 0; short b; }` | 12, 8 | 12, 8 | **4, 2** |
  | the same with `__attribute__((packed))` | 8, 5 | 12, 5 | **8, 4** |

  `sizeof` and every later member offset differ, so a program that prints `sizeof` is wrong on its own, and records exchanged with Clang- or GCC-compiled code (headers, files, network buffers) are misread. Values stay self-consistent inside a bcc-only program.
- Scope: Windows targets only (Microsoft layout). The layout fuzzer found 31 differing records among 1,900 Windows records, every one with a zero-width bit-field, with and without packing; 1,500 Windows records without zero-width fields and 1,400 Linux records (zero-width included) match Clang exactly.
- Component: semantic analysis, record layout (`Analyzer::complete_record` in `semantic_analysis/declarations.rs`, the `ms_unit` handling of `width == Some(0)`).
- Not a merge regression: it reproduces without packing, which predates 9d019dc8.

### W4. Semantics-bearing GNU attributes are silently ignored (wrong code, both targets)

- Reproducers: [`gnu_semantic_attributes_ignored.c`](conformance-reproducers-2026-10-14/gnu_semantic_attributes_ignored.c) (Clang/GCC print `21 12` and `bye 7`; bcc prints `0 0` and nothing more in every arm, with no warning even without `-w`) and [`gnu_alias_weak_dropped.c`](conformance-reproducers-2026-10-14/gnu_alias_weak_dropped.c) (bcc: `undefined symbol: alias_fn` at link time; `weak_fn` is emitted as an ordinary strong definition).
- `cleanup(f)` does not call `f` at scope exit; `constructor`/`constructor(priority)` and `destructor` functions never run; `alias("target")` defines no symbol; `weak` is dropped. `section`, `used` and `aligned` work.
- Impact: `cleanup` is the basis of `g_autoptr`/`g_autofree` (GLib), systemd's `_cleanup_` and many lock guards; `constructor` registers plug-ins and initializes libraries; `weak` provides overridable defaults. Ignoring them silently turns correct programs into leaking or uninitialized ones.
- Component: semantic analysis (the attribute pass accepts unknown attribute names without diagnostics; see `language-standards.md`, "attribute pass preserves their syntax without applicability warnings") and lowering (no cleanup obligations, `llvm.global_ctors`/`dtors`, aliases or weak linkage).
- Direction: implement them, or at minimum diagnose every attribute whose meaning bcc drops (an error for `cleanup`, `constructor`, `destructor`, `alias`; a warning for `weak`).

### L1. MinGW `__CRT_INLINE` functions are defined in every translation unit (link failure, Windows)

- Reproducer: [`mingw_crt_inline_multiple_definition_one.c`](conformance-reproducers-2026-10-14/mingw_crt_inline_multiple_definition_one.c) and [`…_main.c`](conformance-reproducers-2026-10-14/mingw_crt_inline_multiple_definition_main.c), each compiled with `-c` for `x86_64-w64-windows-gnu` with the MinGW headers: the link fails with `multiple definition of 'strncasecmp'` and `'strcasecmp'`. Clang and GCC link and print `0 0`.
- Cause: MinGW's `_mingw.h` defines `__CRT_INLINE` as `extern inline __attribute__((__gnu_inline__))` only for `__GNUC__ >= 4.3` or `__clang__`, otherwise as `extern __inline__`. bcc predefines `__GNUC__ 4`, `__GNUC_MINOR__ 2` (copied from Clang) but not `__clang__`, and gives `extern __inline__` C99 semantics (`__GNUC_STDC_INLINE__ 1`), so each translation unit emits an external definition. GCC before 4.3 used GNU89 inline semantics, which is what the header assumes. bcc's own `gnu_inline` handling is correct (it emits no definition).
- Impact: every multi-file Windows program that includes `<string.h>`, `<stdlib.h>`, `<wchar.h>` or other MinGW headers with `__CRT_INLINE` bodies, unless built with `-D__CRT__NO_INLINE` (which round 1 and this campaign pass to every compiler, hiding it).
- Component: preprocessing, predefined macros (`target/predefined.rs`).
- Direction: report a GNU version whose inline semantics match bcc's (4.3 or later, as GCC and Clang's GNU mode imply), or give `extern __inline__` GNU89 semantics when the dialect claims pre-4.3 GCC.

### C1. `known-bits` indexes past its fact table (optimizer panic)

- Reproducer: [`known_bits_new_constant_index.c`](conformance-reproducers-2026-10-14/known_bits_new_constant_index.c): `bcc-rust known_bits_new_constant_index.c --passes known-bits --emit ir` panics with `index out of bounds: the len is 7 but the index is 7` at `src/optimizer/known_bits.rs:143`. The default pipeline does not panic on it (fold and SCCP remove the remainder first).
- Found by: the IR differential tester's ignored 20,000-case sweep (`cargo test --lib difftest -- --include-ignored`), seed `0x1bcc111f2`, and again with `BCC_DIFFTEST_SEED=0x3bcc00000 BCC_DIFFTEST_CASES=20000` (seed `0x3bcc03de9`), both under `Single(KnownBits)`.
- Cause: `run` sizes `zeros` by `draft.value_count()` once. `rewrite` turns `urem x, 2^k` into `and x, 2^k - 1` and creates the mask with `draft.add_constant`, a value past the table. When a later equality condition tests that `and`, `condition` looks through it and calls `remember(draft.resolve(n), …)` for the operand pair `(new constant, constant dividend)`, indexing `zeros` with the new value.
- Component: optimizer pass `known-bits`. Severity: compiler panic, reachable only with pass lists that run `known-bits` before folding (ablation and bisection workflows).
- Direction: grow `zeros` when a rewrite adds values, or skip constants in `remember`.

### R9-R13. Refusals of valid code

| # | Construct | bcc says | Reproducer | Component |
| --- | --- | --- | --- | --- |
| R9 | array of unknown size initialized by `u""`/`U""` (C11 §6.7.9p22; `L""` works) | `object requires a complete type` | [`utf16_string_array_size.c`](conformance-reproducers-2026-10-14/utf16_string_array_size.c) | semantic analysis, initializers |
| R10 | static initialization of a flexible array member (GNU) | `excess initializer for the current object` | [`flexible_array_static_initializer.c`](conformance-reproducers-2026-10-14/flexible_array_static_initializer.c) | semantic analysis, initializers; lowering of static data |
| R11 | `__attribute__((transparent_union))` parameters (GNU) | `function argument cannot be converted to its parameter type` | [`gnu_transparent_union.c`](conformance-reproducers-2026-10-14/gnu_transparent_union.c) | semantic analysis |
| R12 | thread-local storage: `__thread` keyword, and `_Thread_local` objects | `unknown type name '__thread'`; `construct not supported by code generation: its type was not analyzed` | [`thread_local_storage.c`](conformance-reproducers-2026-10-14/thread_local_storage.c) | parser keyword table; lowering |
| R13 | atomics: `_Atomic` objects, `<stdatomic.h>` operations, `__atomic_*` and `__sync_*` builtins (typed by semantic analysis, see `front-end-lowering-readiness.md`) | `not yet supported by lowering: atomic types`; `its type was not analyzed` | [`atomic_builtins_not_lowered.c`](conformance-reproducers-2026-10-14/atomic_builtins_not_lowered.c) | lowering |

R12 and R13 also show a diagnostics defect: "its type was not analyzed" names neither the construct nor the missing lowering.

Also seen and not re-reported: `__FUNCTION__` is refused like `__PRETTY_FUNCTION__` (#48); `isnan(NAN)`/`isinf(INFINITY)` fail through the untyped `__builtin_nanf`/`__builtin_inff` (#47); the interpreter lacks `qsort`, `strtol`, `wcslen`, `__isnan`, `%f` and `%Lf` conversions and x87 values (#52 family); local labels, inline assembly, complex arithmetic, `setjmp` on Linux (#50) remain refused as documented.

### D1 (reference defect, not attributed to bcc). Clang miscompiles packed records with a zero-width bit-field

- Csmith seeds 1000061 and 1000312 (`--no-pointers`, Windows) print one checksum in all five bcc arms and MinGW GCC (`-O0`, `-O2`) and another in Clang `-O0`/`-O2`. On Linux, Clang, GCC and bcc all agree on seed 1000061.
- Both programs declare a `#pragma pack(1)` structure with `unsigned : 0` between bit-fields. [`clang_reference_packed_zero_width.c`](conformance-reproducers-2026-10-14/clang_reference_packed_zero_width.c) shows Clang reading back `1` and `-48234475` for fields initialized to `3` and `5396`, and a store to one field changing another: the bundled Clang lays the fields out overlapping. GCC and bcc print the initialized values.
- These programs are counted as wrong output in the tables (the harness compares with Clang) but are not bcc defects. bcc's layout of these records still differs from both references (W3).

## Proposed issues

Titles and bodies for filing; not filed by this task.

1. **Windows bit-field layout: zero-width bit-field must end the Microsoft allocation unit.** Body: W3 above (table, reproducer, `complete_record`), noting the layout fuzzer result (31/1,900 Windows records, all with zero-width fields; 0/1,500 without; 0/1,400 on Linux) and that Clang itself mislays packed variants (D1), so tests should compare offsets with GCC and Clang where they agree.
2. **GNU `cleanup`, `constructor`, `destructor`, `alias` and `weak` are accepted and ignored.** Body: W4, reproducers, impact (GLib/systemd cleanup, library constructors), and the request to diagnose dropped attributes until they are implemented.
3. **MinGW: two translation units including `<string.h>` fail to link (duplicate `strncasecmp`).** Body: L1 with the `_mingw.h` condition, the `__GNUC__`/`__GNUC_MINOR__`/`__GNUC_STDC_INLINE__` combination, and the note that the conformance harnesses mask it with `-D__CRT__NO_INLINE`.
4. **`known-bits` panics on values created by its own rewrites.** Body: C1, both difftest seeds, the C reproducer with `--passes known-bits`.
5. **`u""`/`U""` string literals do not complete an array of unknown size.** Body: R9.
6. **Static initializers for flexible array members (GNU extension) are refused.** Body: R10.
7. **`transparent_union` is not supported.** Body: R11.
8. **Thread-local storage: accept `__thread` and lower `_Thread_local`.** Body: R12, including the misleading diagnostic.
9. **Lower atomics (`_Atomic`, `<stdatomic.h>`, `__atomic_*`, `__sync_*`).** Body: R13, readiness note that semantic analysis already types them.

## Reproducing

```powershell
cargo build --release
python scripts/random_program_conformance.py run --output target/conformance/r2-win-default --bcc target/conformance-bcc/bcc-rust.exe --generators csmith,yarpgen --count 1000 --seed-start 1000000 --jobs 6
python scripts/random_program_conformance.py run --output target/conformance/r2-win-nopointers --bcc target/conformance-bcc/bcc-rust.exe --generators csmith --count 1000 --seed-start 1000000 --csmith-option=--no-pointers --jobs 6
python scripts/random_program_conformance.py run --output target/conformance/r2-win-yarpgen-mutate --bcc target/conformance-bcc/bcc-rust.exe --generators yarpgen --count 1000 --seed-start 1001000 --yarpgen-option=--mutate=all --yarpgen-option=--allow-dead-data=true --yarpgen-option=--expl-loop-param=true --yarpgen-option=--max-array-dims=3 --jobs 6
python scripts/random_program_conformance.py run --output target/conformance/r2-linux-default --platform linux-wsl --bcc target/conformance-bcc/bcc-rust.exe --generators csmith,yarpgen --count 1000 --seed-start 2000000 --jobs 6
python scripts/random_program_conformance.py run --output target/conformance/r2-linux-nopointers --platform linux-wsl --bcc target/conformance-bcc/bcc-rust.exe --generators csmith --count 600 --seed-start 2000000 --csmith-option=--no-pointers --jobs 6
python scripts/run_program_corpus.py --output target/conformance/r2-programs --bcc target/conformance-bcc/bcc-rust.exe --jobs 6
python scripts/run_program_corpus.py --output target/conformance/r2-programs-linux --platform linux-wsl --bcc target/conformance-bcc/bcc-rust.exe --jobs 6
BCC_DIFFTEST_SEED=0x1bcc111f2 BCC_DIFFTEST_CASES=1 cargo test --lib difftest::tests::optimizer_preserves_generated_programs
```
