Research date: 2026-10-11.

GCC 15.2.0 top-level `gcc.c-torture/execute/*.c` was surveyed through release bcc code generation on Windows. All 1,677 files were read from the existing extracted corpus. No sources were downloaded, rewritten, or written back. Every generated executable, temporary LLVM file, program working directory, and evidence file stays under this worktree's `target/`. The `target/llvm` junction was only read for tool execution.

This is an exploratory single-file survey, using the existing parser runner's evidence and source-hash conventions. It does not run DejaGnu. The default bcc dialect is GNU17; target skips, `dg-options`, additional sources, and requested runtime assumptions are recorded without applying them. The nested `ieee/` and `builtins/` directories are excluded, matching the requested top-level glob. The runner can include them with `--recursive`. Raw differences remain failures in the machine-readable output, even when triage explains them.

The reproducible PowerShell commands were:

```powershell
cargo build --release
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-before --jobs 8 --timeout 10
cargo build --release
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-done --jobs 8 --timeout 10
```

Both survey commands return 1 because unresolved raw failures and disagreements remain. Each directory is fresh. The first run used the unchanged branch compiler; the second used the final compiler changes. Intermediate full runs are retained in `execute-round2/`, `execute-after/`, and `execute-final/` under the same corpus output directory. The interrupted `execute-verified/` and `execute-complete/` runs exposed the terminating-process Windows job-assignment race; neither is used in the final totals. The manifests retain the executable SHA-512, Git head/status, host, source root, flags, worker count, and deadline; each source row retains its SHA-512.

The arms all pass `--target=x86_64-w64-windows-gnu`:

| Arm | Compiler options | Program execution |
| --- | --- | --- |
| interpret | `--interpret` | Unoptimized interpreter |
| interpret-bcc | `--interpret --opt=bcc` | bcc-optimized interpreter |
| A0 | `--llvm-opt=O0 -o <exe>` | Native executable |
| A1 | `--opt=bcc --llvm-opt=O2 --llvm-codegen-only -o <exe>` | Native executable |

There are eight workers, with the four arms run sequentially inside each test. Compilation/interpretation and native execution each have a ten-second deadline. Each arm gets an isolated working and temporary directory. Compiler classification uses stderr even when the compiler returns zero with diagnostics. Native aborts/faults are wrong results, not compiler crashes. Interpreter traps retain their kind. An invalid emitted LLVM module is a compiler crash rather than a link error. Windows process trees are owned by kill-on-close jobs, following [Microsoft's job-object contract](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects); a regression verifies that a timed-out parent cannot leave a child holding the output pipe.

The complete before/after counts are:

| Class | interpret before | after | interpret-bcc before | after | A0 before | after | A1 before | after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pass | 716 | 1141 | 719 | 1145 | 889 | 1195 | 880 | 1185 |
| frontend-error | 138 | 139 | 138 | 139 | 138 | 139 | 138 | 139 |
| lowering-unsupported | 625 | 311 | 625 | 311 | 625 | 311 | 625 | 311 |
| abi-unsupported | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| compiler-crash | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| interpreter-trap | 187 | 69 | 183 | 64 | 0 | 0 | 0 | 0 |
| wrong-result | 11 | 17 | 12 | 18 | 13 | 17 | 22 | 27 |
| timeout | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| link-error | 0 | 0 | 0 | 0 | 12 | 15 | 12 | 15 |

All-four-arm passes increased from 715 to 1140; both-native passes increased from 880 to 1185. Raw source disagreements decreased from 188 to 72. Elapsed survey time was 259.411s before and 319.399s after. There were no compiler crashes or timeouts in either full run. No previously passing arm regressed. Wrong-result and link-error counts rise because previously refused sources now reach execution/linking; this is not a regression in an existing pass.

The fixes and reduced regressions are:

1. A declaration such as `void exit();` had been lowered as a zero-parameter function. A call with its default-promoted argument then acquired a different signature and trapped in the interpreter. Open external declarations now use a variadic declaration with no fixed parameters, preserving default argument promotions. Known old-style definitions retain their real parameter signature, indexed by linked name so block-scope and implicit declarations resolve to the same definition. The reduction `float f(x) float x; { return x; } int main(void) { float f(); return f(4.) != 4.; }` protects that distinction. The same open external contract now works through a function pointer, while defined-function and result-type mismatch checks remain intact. The pointer-call regression was observed red and then green. C99 §6.2.2p2 and §6.5.2.2p6 govern linkage and these calls.

2. The CLI always supplied zero arguments to interpreted `main`, rejecting valid `main(int, char **)` programs. Hosted entry now supplies `argc == 1`, writable empty program-name storage, and writable `argv` terminated by a null pointer. The general explicit-argument interpreter API retains its contract. A reduced CLI regression checks both targets and both optimization settings, including modifying the strings and argument array. C99 §5.1.2.2.1p2 permits an empty unavailable program name and requires modifiable argument storage.

3. Common `__builtin_` library calls had opaque semantic types or were refused as intrinsics. Twenty-four aliases now receive complete library prototypes and lower to host C symbols, using the target size type and const-qualified inputs. This follows [GCC's library-builtin contract](https://gcc.gnu.org/onlinedocs/gcc/Library-Builtins.html). The table covers termination, allocation, memory/string operations, basic output, integer absolute value, and double fabs/floor. A builtin alias is conservatively refused if a same-spelling internal function would capture it. Invalid argument counts/types have semantic regressions. The dead `__builtin_abort` reduction and small memset/memcpy/ordinary-alias programs run through pre/post ABI, optimized/unoptimized interpretation, and native arms.

4. The interpreter host now provides `memmove`, `memcmp`, `strcmp`, `strncmp`, `strcpy`, `strncpy`, `memchr`, `strchr`, `strrchr`, `sprintf`, `abs`, `fabs`, `floor`, and initial-C-locale `isprint`. Regressions cover unsigned-byte comparison, bounded unterminated input, zero-length search at one-past, search for the null terminator, first/last occurrence, bounded-copy padding, overlapping memmove, and sprintf termination/count without stdout. The corpus checks the same calls against native execution. Extended floating point and unsupported printf conversions still report explicit interpreter traps.

The main regressions are in `src/pipeline/tests/middle_end/runtime_calls.rs`, `src/backend/interpreter/tests/host.rs`, `src/backend/interpreter/tests/calls.rs`, and `tests/code_generation_cli.rs`. Lowering goldens now show open external declarations as `(...)` and direct calls with promoted arguments. Only the expected `calls.ir` and `float_calls.ir` snapshots changed; behavioral assertions were not weakened.

The prioritized lowering roadmap below counts distinct sources per construct, not four repeated arm refusals. A source can contribute to several constructs, so the rows do not sum to the number of refused sources. The opaque-type bucket needs a semantic inventory first: its remaining sources mention overflow builtins, alloca, prefetch, setjmp/longjmp, bit operations, and target-specific intrinsics. Mentions of already supported abort/memcpy are not evidence those calls remain unsupported; another opaque expression can block the same source.

| Priority | Unsupported construct | Before sources | After sources | Next work |
| ---: | --- | ---: | ---: | --- |
| 1 | type not analyzed by semantic analysis | 436 | 111 | Inventory and type the remaining intrinsics; separate cheap arithmetic builtins from stack/nonlocal-control features. |
| 2 | inline assembly | 51 | 61 | Separate empty barriers from target-specific assembly; requires an explicit IR/backend contract. |
| 3 | this builtin | 53 | 54 | Implement already typed intrinsic operations; 54 is a conservative bucket, including library-name collision. |
| 4 | nested functions | 29 | 29 | Plan captures and trampoline/static-chain ABI support. |
| 5 | vector types | 20 | 20 | Define vector IR and ABI lowering. |
| 6 | structures and unions passed as variable arguments | 16 | 17 | Complete aggregate varargs across call and ABI boundaries. |
| 7 | complex arithmetic | 12 | 12 | Define complex values, operations, and ABI representation. |
| 8 | this static initializer | 10 | 10 | Extend constant evaluation and relocation emission. |
| 9 | variable length arrays | 5 | 6 | Implement bound evaluation, dynamic stack storage, and lifetime. |
| 10 | statement expressions | 4 | 4 | Lower the GNU block-result value and control flow. |
| 11 | wide string literals | 2 | 2 | Emit target-width encoded globals. |
| 12 | this initializer | 2 | 2 | Inventory the remaining initializer forms. |
| 13 | computed goto | 1 | 1 | Model the GNU control-flow extension explicitly. |
| 14 | label addresses | 1 | 1 | Model the GNU control-flow extension explicitly. |
| 15 | local labels | 1 | 1 | Model the GNU control-flow extension explicitly. |
| 16 | case ranges | 1 | 1 | Model the GNU control-flow extension explicitly. |
| 17 | conditionals with an omitted operand | 1 | 1 | Model the GNU control-flow extension explicitly. |

Large missing features were recorded rather than implemented. Interpreter f80/f128 values and full printf formatting are separate backend/host roadmap items. GNU symbol-alias, typedef alignment, scalar-storage-order, extended bit-field arithmetic, parameter-VLA evaluation, and legacy input encoding also remain compatibility gaps.

Every remaining arm disagreement is listed below with its triage. I0/I1 are the two interpreter arms, P means pass, T interpreter trap, W wrong result, and L link error. The raw statuses are preserved in JSON Lines and the runner summary. Compiler crash list: none.

| Test | I0/I1/A0/A1 | Triage |
| --- | --- | --- |
| `20001101.c` | T/T/P/P | Old-style pointer parameter receives integer zero without prototype conversion (C99 6.5.2.2p6). |
| `20001121-1.c` | T/T/L/L | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `20011121-1.c` | T/T/P/P | GNU null-based member-offset idiom; runtime inbounds arithmetic on null traps. Use __builtin_offsetof for supported C99 code. |
| `20020107-1.c` | T/T/L/L | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `20020413-1.c` | T/T/P/P | Extended floating-point interpretation is explicitly unsupported; both native arms pass. |
| `20020508-2.c` | T/T/P/W | Signed left shift of a negative value or an unrepresentable positive result (C99 6.5.7p4). |
| `20020508-3.c` | T/T/P/W | Signed left shift of a negative value or an unrepresentable positive result (C99 6.5.7p4). |
| `20030316-1.c` | T/T/P/P | Addition overflows 32-bit Windows long; would not overflow an LP64 long (C99 6.5p5). |
| `20030404-1.c` | T/P/P/P | Reads an uninitialized automatic value; optimization can eliminate the poison use (C99 6.7.8p10). |
| `20030914-1.c` | T/T/P/P | Extended floating-point interpretation is explicitly unsupported; both native arms pass. |
| `20031003-1.c` | T/T/W/W | Converts 2147483648.0 to 32-bit int, outside its range (C99 6.3.1.4p1). |
| `20040208-1.c` | T/T/P/P | Extended floating-point interpretation is explicitly unsupported; both native arms pass. |
| `20040409-1w.c` | T/T/P/W | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `20040409-2w.c` | T/T/P/W | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `20040409-3w.c` | T/T/P/W | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `20040709-1.c` | T/T/P/P | Extended floating-point interpretation is explicitly unsupported; both native arms pass. |
| `20051012-1.c` | T/T/P/P | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `20090527-1.c` | T/T/P/P | Reads an uninitialized automatic value; optimization can eliminate the poison use (C99 6.7.8p10). |
| `920501-1.c` | T/T/P/P | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `920612-1.c` | T/T/P/P | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `920711-1.c` | T/T/P/P | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `920730-1.c` | T/T/P/P | INT_MAX + 2 overflows before the cast to unsigned (C99 6.5p5). |
| `920922-1.c` | T/T/P/P | Forms a pointer beyond the object and its one-past position (C99 6.5.6p8). |
| `921202-1.c` | T/T/P/P | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `921208-2.c` | T/T/P/P | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `930526-1.c` | T/T/L/L | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `930529-1.c` | T/T/P/P | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `930622-2.c` | T/T/P/P | Extended floating-point interpretation is explicitly unsupported; both native arms pass. |
| `941202-1.c` | T/T/L/L | Nonstandard alloca call needs stack-allocation lowering; no runtime symbol on this host. |
| `950704-1.c` | T/T/P/W | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `960215-1.c` | T/T/P/P | Extended floating-point interpretation is explicitly unsupported; both native arms pass. |
| `960405-1.c` | T/T/P/P | Extended floating-point interpretation is explicitly unsupported; both native arms pass. |
| `960513-1.c` | T/T/P/P | Extended floating-point interpretation is explicitly unsupported; both native arms pass. |
| `961223-1.c` | T/T/L/L | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `980526-2.c` | T/T/P/P | Signed major << 22 produces an unrepresentable int (C99 6.5.7p4). |
| `980608-1.c` | T/T/L/L | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `991014-1.c` | T/T/P/P | GNU null-based member-offset idiom; runtime inbounds arithmetic on null traps. Use __builtin_offsetof for supported C99 code. |
| `alias-2.c` | W/W/L/L | GNU symbol-alias attribute is not emitted; interpreter sees separate storage and native linking cannot find the alias. |
| `alias-3.c` | W/W/L/L | GNU symbol-alias attribute is not emitted; interpreter sees separate storage and native linking cannot find the alias. |
| `alias-4.c` | T/T/L/L | GNU symbol alias is not emitted; main(int) also has an unsupported entry signature. |
| `align-2.c` | T/T/P/P | Extended floating-point interpretation is explicitly unsupported; both native arms pass. |
| `arith-rand-ll.c` | T/P/P/P | Random values exercise signed ABS at the minimum representable value; optimized algebra can remove this poison use. |
| `arith-rand.c` | T/P/P/P | Random values exercise signed ABS at the minimum representable value; optimized algebra can remove this poison use. |
| `bcp-1.c` | T/T/L/L | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `bswap-2.c` | T/T/P/P | Integer promotions make the sign-bit left shift signed int; its result is unrepresentable in C99 (6.5.7p4). |
| `conversion.c` | T/T/P/P | Extended floating-point interpretation is explicitly unsupported; both native arms pass. |
| `loop-2c.c` | T/T/L/L | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `loop-2e.c` | T/T/P/P | Arithmetic/member addressing on a forged integer pointer with no backing object; outside the interpreter object model (C99 6.5.6p8). |
| `pr103255.c` | T/T/P/P | Arithmetic/member addressing on a forged integer pointer with no backing object; outside the interpreter object model (C99 6.5.6p8). |
| `pr22493-1.c` | T/T/P/W | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `pr23047.c` | T/P/P/P | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `pr34099-2.c` | T/T/P/P | Reads an uninitialized automatic value; optimization can eliminate the poison use (C99 6.7.8p10). |
| `pr34099.c` | T/T/P/P | Reads an uninitialized automatic value; optimization can eliminate the poison use (C99 6.7.8p10). |
| `pr34176.c` | T/T/P/P | Increments and subtracts a null pointer, outside an array object (C99 6.5.6p8-9). |
| `pr36339.c` | P/P/W/W | Casts a 64-bit stack pointer through 32-bit unsigned long on LLP64. GCC also faults on this host. |
| `pr40386.c` | T/T/P/W | Signed left shift of a negative value or an unrepresentable positive result (C99 6.5.7p4). |
| `pr43629.c` | T/W/P/W | Reads an uninitialized automatic value; optimization can eliminate the poison use (C99 6.7.8p10). |
| `pr56051.c` | T/T/P/P | Integer promotions make the sign-bit left shift signed int; its result is unrepresentable in C99 (6.5.7p4). |
| `pr57124.c` | T/T/P/P | Requires -fno-strict-overflow; promoted unsigned-short multiplication overflows signed int. |
| `pr67037.c` | T/T/P/P | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `pr7284-1.c` | T/T/P/P | Requires -std=c89 for implementation-defined signed shift; the default GNU17 survey uses C99 shift rules. |
| `pr79327.c` | T/T/P/P | Interpreter formatter does not implement the alternate-form # flag / octal conversion; capability gap. |
| `pr99079.c` | T/T/P/W | Integer promotions make the sign-bit left shift signed int; its result is unrepresentable in C99 (6.5.7p4). |
| `ptr-arith-1.c` | T/T/P/P | Forms a pointer beyond the object and its one-past position (C99 6.5.6p8). |
| `regstack-1.c` | T/T/P/P | Extended floating-point interpretation is explicitly unsupported; both native arms pass. |
| `restrict-1.c` | T/T/L/L | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `return-addr.c` | T/T/P/P | Forms before-object pointers and compares unrelated objects; source comments acknowledge undefined behavior. |
| `shiftdi-2.c` | T/T/P/P | Signed left shift of a negative value or an unrepresentable positive result (C99 6.5.7p4). |
| `string-opt-18.c` | T/T/P/P | Overlapping memcpy/strcpy on an uninitialized buffer (C99 7.21.2.1p2). |
| `unroll-1.c` | T/T/L/L | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `va-arg-7.c` | T/T/L/L | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `va-arg-8.c` | T/T/L/L | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |

Eleven sources still have a pass-versus-wrong-result signal: signed-rotation tests `20020508-2.c`, `20020508-3.c`, and `pr40386.c`; the required-wrap tests `20040409-1w.c`, `20040409-2w.c`, `20040409-3w.c`, `950704-1.c`, and `pr22493-1.c`; `pr36339.c` (LLP64 pointer truncation); `pr43629.c` (uninitialized value); and `pr99079.c` (signed sign-bit shift). Source inspection establishes why these signals do not demonstrate a defined C99 optimizer miscompile under the recorded options. C99 §6.5p5, §6.5.7p4, §6.7.8p10, and §6.3.2.3 govern those boundaries. No optimizer poison or pointer-provenance rules were weakened to force a green result.

A supplementary reference survey used the installed x86_64-w64-mingw32 GCC 13.2.0 with `-std=gnu17 -O0`, `-O2`, and `-O2 -fwrapv` for these eleven sources. `pr36339.c` faults in all three GCC arms on this host. The three `20040409-*w.c` files and `pr22493-1.c` abort under GCC `-O2` and pass with `-fwrapv`. Other undefined-shift/indeterminate tests happen to pass GCC, which does not make their C99 operations defined. The commands, GCC version, diagnostics, and native statuses are retained in `target/compiler-corpus/execute-reference/results.json`; the scratch reference driver is `target/torture-reference.py`.

Tests with wrong results and no passing bcc arm were also inspected. `20000227-1.c` depends on legacy source encoding; `20010904-1.c` and `20010904-2.c` on typedef alignment; `20230630-2.c` and `20230630-4.c` on scalar_storage_order; `970217-1.c` on VLA parameter-bound side effects; `bf-sign-2.c` and `bitfld-3.c` on GNU extended bit-field promotion/precision; `pr32244-1.c` and `pr34971.c` on GNU 40-bit bit-field arithmetic; `pr77767.c` on VLA parameter-bound side effects; `eeprof-1.c` on -finstrument-functions; `pushpop_macro.c` on push_macro/pop_macro preprocessing pragmas. `pr58943.c` is a shared failure involving a compound assignment whose RHS call modifies the destination; it remains a separate lowering investigation. `20021127-1.c` expects library builtin folding despite a conflicting user llabs definition. Alias-attribute and out-of-range float-conversion failures are included in the disagreement table. These shared results and compatibility gaps are not counted as completed compiler support.

The supported all-four-arm subset is now much larger, but this exploratory run does not establish that the whole GCC execute suite or a DejaGnu conformance profile passes. The Stage 6 exit criterion in `middle-end.md` has not been relabeled as complete. The next acceptance run should define its supported subset and apply the relevant language, target, and overflow directives.

Final verification commands are:

```powershell
cargo +nightly fmt
cargo test --all-targets
cargo test --features benchmarking-internals --test allocation_count
cargo +nightly fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features benchmarking-internals -- -D warnings
git diff --check
$env:BCC_DIFFTEST_CASES = '3000'
cargo test --lib difftest
python -m unittest discover -s scripts -p "test_run_gcc_torture*.py"
```

The required checks are run sequentially when a feature build could replace the debug CLI used by integration tests. An earlier overlapping run was stopped after that collision caused hanging diagnostic subprocesses; a later linking attempt consequently failed on the still-running test executable. Those interrupted runs remain in `target/torture-check-*.log` as evidence. Final outcomes: all eight requested checks passed. `cargo test --all-targets` passed 1,558 tests across 17 suites, with three existing ignored tests; the allocation test passed 25 checks; the requested differential invocation passed five checks with two existing long sweeps ignored (the ordinary optimizer sweep uses `BCC_DIFFTEST_CASES=3000`); the Python runner/classifier suite passed 20 tests. Both Clippy profiles, formatting, and whitespace checks passed. Logs are `target/torture-check-*-last.log`; formatting/Python/whitespace were also verified directly. Source changes were complete before these final Rust checks.

The whole-file Conventional Commit plan is `target/commit-plan.md`. No staging, commits, or Git metadata writes were performed.

## Correctness follow-up: 2026-10-12

This follow-up starts at clean head `0a756543` in `claude/torture-correctness`.
It uses the same 1,677-source top-level corpus, target, four arms, default
GNU17 dialect, eight workers and ten-second deadlines described above. Corpus
SHA-512 values match between every before/after source row. Neither shared
LLVM junction was changed or rebuilt; the corpus was only read. The earlier
survey's totals are historical, not the baseline for this head.

```powershell
cargo build --release
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-correctness-before --jobs 8 --timeout 10
cargo build --release
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-correctness-after --jobs 8 --timeout 10
```

Both complete surveys return 1 because raw failures remain. Before and after
counts are below; I0 and I1 denote `interpret` and `interpret-bcc`.

| Class | I0 before | after | I1 before | after | A0 before | after | A1 before | after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pass | 1062 | 1064 | 1066 | 1068 | 1199 | 1200 | 1189 | 1190 |
| frontend-error | 139 | 139 | 139 | 139 | 139 | 139 | 139 | 139 |
| lowering-unsupported | 307 | 307 | 307 | 307 | 307 | 307 | 307 | 307 |
| abi-unsupported | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| compiler-crash | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| interpreter-trap | 154 | 153 | 149 | 148 | 0 | 0 | 0 | 0 |
| wrong-result | 15 | 14 | 16 | 15 | 17 | 16 | 27 | 26 |
| timeout | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| link-error | 0 | 0 | 0 | 0 | 15 | 15 | 15 | 15 |


No passing source/arm regressed. The only changed sources are `pr58943.c`
(W/W/W/W to P/P/P/P) and `pr79327.c` (T/T/P/P to P/P/P/P).
Disagreements decreased from 155 to 154: the first fix was a shared failure,
so only the second removes a disagreement. There were zero compiler crashes
and zero timeouts in either run. Before elapsed time was 609.937 seconds;
after elapsed time was 488.608 seconds; these uncontrolled times are not a
performance comparison.

The fresh interpreter totals differ from the old report because the current
head checks host signatures and unresolved external objects more strictly.
Captured failing-instruction evidence identifies 81 implicit `int`-returning
`exit` declarations, one implicit `int`-returning `strlen` declaration, and
six mismatched old-style indirect calls. `exit` returns `void`, and Win64
`strlen` returns 64-bit `size_t`; the retained call-site result types differ.
These are legacy/GCC call assumptions, not defined C99 calls
(C99 §6.5.2.2p6,p9, pp. 71-72; PDF pp. 83-84). One source, `loop-3b.c`, also
overflows signed multiplication before passing a scalar argument; its I0
trap occurs there before I1 reaches its incompatible `exit` call. Host
signature checks and `noundef` checks were retained. The implicit-prototype
follow-up below resolves the implicit library types while preserving those checks.

The fixes are:

1. **Compound assignment read across a called function.** Lowering read the
   destination before the RHS call, then overwrote that call's modification
   using the stale value. The smallest array reduction from `pr58943.c` is:

   ```c
   unsigned x[1] = {2};
   unsigned f(void) { x[0] |= 128; return 1; }
   int main(void) { x[0] |= f(); return x[0] != 131; }
   ```

   Both the interpreter and native A0 returned 1 before the change. The
   lowering work stack now evaluates the RHS before evaluating and updating
   the lvalue, preserving exactly one lvalue evaluation. All four arms now
   return 0. Regressions also cover a scalar, a bit-field, the expression's
   returned value, and a side-effecting function returning the lvalue's
   address. C99 §6.5.16.2p3 (p. 93; PDF p. 105) permits this chosen operand
   order and requires one evaluation of the lvalue. C11 §6.5.16.2p3
   (pp. 103-104; PDF pp. 121-122) additionally specifies that the compound
   assignment is one evaluation relative to an indeterminately sequenced
   function call; that rule applies to this survey's GNU17 dialect. The
   fix preserves C99's available RHS-first evaluation choice as well.

2. **Interpreter alternate integer formatting.** The default host trapped
   on `%#hho` and `%#hhx` in `pr79327.c`. A minimal reduction is:

   ```c
   int sprintf(char *, const char *, ...);
   int main(void) {
       char b[8];
       return sprintf(b, "%#hho", 127) != 4
           || b[0] != '0' || b[1] != '1' || b[2] != '7'
           || b[3] != '7' || b[4] != 0;
   }
   ```

   It previously trapped in the interpreter while native execution passed.
   The host now supports octal conversion and `#` for octal/hexadecimal,
   emits no hexadecimal prefix for zero, truncates promoted `hh` arguments
   before deciding the prefix, and puts zero field padding after the base
   prefix. Regressions check exact bytes, counts, null termination, zero,
   uppercase hexadecimal, left alignment, zero padding and narrow argument
   truncation; `sprintf` must leave stdout empty. C99 §7.19.6.1p6
   (pp. 275-276; PDF pp. 287-288) defines the alternate forms and padding.
   Precision and other previously unsupported conversions remain explicit
   capability limits.

Each reduction runs through optimized/unoptimized interpretation before and
after ABI lowering on LinuxGNU and WindowsGNU. Native regressions run A0,
A1, A2 and A3 on WindowsGNU. The compound regression was observed red in
both interpreter and native tests before the fix. The reduced formatting
program was observed to trap before the host change and pass natively;
the final regression checks the same operation. Bundled Clang and installed
MinGW GCC also pass both original fixed sources at `-O0` and `-O2`.

The five changed lowering goldens (`assignment`, `bit_field_updates`,
`bit_field_volatile`, `float_increments`, `globals`) reflect only the chosen
RHS-first instruction order and consequent SSA value renumbering. Their C
sources and semantic assertions were retained.

The ten A0-pass/A1-wrong sources were prioritized and inspected individually:

| Sources | Root cause under the recorded survey options |
| --- | --- |
| `20020508-2.c`, `20020508-3.c`, `pr40386.c` | Signed rotate expressions left-shift negative values or produce unrepresentable signed results; C99 §6.5.7p4. |
| `20040409-1w.c`, `20040409-2w.c`, `20040409-3w.c`, `950704-1.c`, `pr22493-1.c` | Require their recorded `-fwrapv`; overflowing signed addition/subtraction/negation has no defined C99 result under the actual options; §6.5p5. |
| `pr43629.c` | Applies `&=` to uninitialized automatic `x`; the interpreter's poison is observable before optimization; §6.7.8p10 and the documented indeterminate-value policy. |
| `pr99079.c` | `1 << 31` is signed-int arithmetic before assignment to unsigned long long; §6.5.7p4. |

No defined-program bcc optimizer miscompile was established in these ten
signals. The raw failures were retained. Reference controls compiled all ten
unchanged sources through GCC `-O0`, `-O2`, `-O2 -fwrapv`, and bundled Clang
`-O0`, `-O2`, all with GNU17 on this host. The three `20040409-*w.c` files and
`pr22493-1.c` abort under both GCC/Clang `-O2` and pass under GCC `-fwrapv`.
`950704-1.c` aborts under Clang `-O2` and passes with GCC's required wrapping
mode. The undefined shift/indeterminate sources happen to pass the reference
compilers, which does not establish defined C99 semantics. The additional
`pr36339.c` control faults under every GCC/Clang arm: its 32-bit Windows
`unsigned long` truncates a 64-bit stack address.

All 15 native link-error sources were inspected. Eleven require GNU89-inline
options that the survey deliberately records without applying; three require
GNU symbol aliases; `941202-1.c` requires compiler `alloca` lowering, for which
MinGW has no external runtime symbol. No missing ordinary supported C library
symbol was found. Implementing GNU aliases or dynamic stack allocation needs
an IR/backend contract, rather than a dummy runtime function.

Two remaining shared wrong results expose a valid-C99 missing feature, not
undefined tests: parameter VLA bounds are not executed on function entry.
The complete reduced programs are:

```c
int f(int n, int a[n++]) { return n; }
int main(void) { int a[1]; return f(1, a) != 2; }
```

```c
int f(int a, int b[a++], int c, int d[c++]) { return a != 2 || c != 2; }
int main(void) { int a[1]; return f(1, a, 1, a); }
```

Both return 1 through bcc interpretation/native A0 and 0 through bundled
Clang at O0/O2. Local reproductions are `target/vla-parameter-bound.c` and
`target/vla-parameter-bounds.c`. C99 §6.9.1p10
(p. 142; PDF p. 154) requires evaluating those size expressions. The current
retained `ArrayBound::Variable` carries no runtime expression/extent, and
prototype bindings differ from function-body bindings. A complete repair
needs retained runtime-bound semantics, binding replay, saved extents,
positivity and `sizeof`/pointer-stride behavior; it is left as a large feature.
These failures are explicitly recorded as lowering gaps.

Other required capabilities are full interpreter f80/f128 semantics, GNU
typedef alignment, scalar-storage order, extended bit-field precision,
symbol aliases, macro-save pragmas and function instrumentation. They were
classified rather than hidden by refusing more inputs or altering survey
options. GCC-specific expected behavior, unavailable options and actual
undefined operations likewise remain visible in the raw counts.

### Implicit library prototypes: 2026-10-12

This update starts at `bdc88ada` on `claude/torture-correctness` and compares
against `execute-correctness-after`, the preceding correctness baseline above.
It reruns the same 1,677 unchanged sources with eight workers, ten-second
deadlines, GNU17 and the same four Windows GNU arms. Every source SHA-512
matches. Both shared LLVM junctions were preserved.

```powershell
cargo build --release --bin bcc-rust
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-implicit-after --jobs 8 --timeout 10
```

The survey returns 1 because adverse sources remain. Its 518.243
seconds and the baseline's 488.608 seconds are uncontrolled elapsed times,
not a performance comparison.

| Class | I0 before | after | I1 before | after | A0 before | after | A1 before | after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pass | 1064 | 1145 | 1068 | 1150 | 1200 | 1200 | 1190 | 1190 |
| frontend-error | 139 | 139 | 139 | 139 | 139 | 139 | 139 | 139 |
| lowering-unsupported | 307 | 307 | 307 | 307 | 307 | 307 | 307 | 307 |
| abi-unsupported | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| compiler-crash | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| interpreter-trap | 153 | 72 | 148 | 66 | 0 | 0 | 0 | 0 |
| wrong-result | 14 | 14 | 15 | 15 | 16 | 16 | 26 | 26 |
| timeout | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| link-error | 0 | 0 | 0 | 0 | 15 | 15 | 15 | 15 |

There are **168 -> 87 adverse sources** and **154 -> 73 disagreements**.
All 163 changed arms, across 82 sources, change from interpreter trap to pass;
no passing source/arm regresses. Native counts, frontend errors, unsupported
lowering, wrong results and link errors are unchanged. There are zero compiler
crashes and zero timeouts. All 81 sources previously classified as
`GCC/legacy call assumption` now pass all four arms. `loop-3b.c` additionally
passes I1; its I0 signed-overflow trap remains, so its existing undefined-test
classification is retained.

Semantic analysis now gives permitted implicit declarations the library
prototype, with target `size_t`, const input pointees and variadic parameters,
and emits a structured warning naming the function and complete type. Unknown
names keep `int ()`; explicit declarations keep the user's type. Strict C99
and later retain their undeclared-identifier error. The existing extension
policy still diagnoses the removed implicit-declaration feature. No host
signature or defined-value checks were relaxed, and builtin folding, attributes
and unsupported-intrinsic handling were not expanded.

Captured failing instructions name `exit` (81) and `strlen` (one). Bundled
Clang's diagnostics across the 88 signature-evidence sources additionally
identify implicit `abort` in 79 sources, `exit` in
85 and `strlen` in 1. All are covered by the existing
24 complete library-alias prototypes, so no additional prototype entries are
needed. The ordinary-name lookup shares that table; a mechanical comparison
confirms all 24 alias symbols, signature codes and variadic flags are unchanged.

Bundled Clang `-std=gnu17 -Wno-error=implicit-function-declaration -S -emit-llvm`
confirms real types for `exit`, `abort`, `memcpy`, `strcmp`, `printf` and `strlen`.
The warning option is needed because this Clang promotes undeclared calls to
errors by default in GNU17. An explicit `int exit()` control retains an
`i32 @exit(...)` declaration and call despite Clang's incompatible-library
warning. The live regression compares scalar LLVM types, fixed/variadic
arguments and `noundef` at declarations and calls on Linux GNU, Windows GNU
and Windows MSVC. Clang rewrites direct `memcpy` to an intrinsic; its retained
C declaration and indirect call provide the corresponding C ABI comparison.
Return `noundef` remains absent, and scalar arguments retain their contracts.

The semantic regression checks retained types, repeated-call warning count,
full diagnostic type spelling, C89/GNU17, extension policies and LP64/LLP64.
The diagnostic golden renders six library warnings without truncating types.
Headerless `int main(void) { exit(0); }` and memory/string reductions return
0 through optimized/unoptimized interpretation before/after ABI lowering on
Linux GNU and Windows GNU, and through native A0/A1/A2/A3 on this host.
The existing lowering regression now expects `putchar(int)` while explicitly
checking that the unknown `exit2` remains open. User-written incompatible and
open declarations retain their existing behavior and mismatch checks.

This is a Clang-compatible extension to C89's implicit declaration rule
(ANSI C89 §3.3.2.2, p. 41; PDF p. 55), not permission for undeclared
C99 calls. C99 §6.5.2.2p6,p9 (pp. 71-72; PDF pp. 83-84) governs call
compatibility; §7.1.4p2 (p. 167; PDF p. 179) permits a suitable library
declaration without its associated header.

Every changed source is reclassified below. The following adverse-source
table reflects the new 87-source result; the full transition records, corpus
hashes, manifests and arm logs remain under `target/`.

| Source | Before | After | Reclassification |
| --- | --- | --- | --- |
| `20000412-3.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `20000717-3.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `20020819-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `20031211-2.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `920409-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `920411-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `920428-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `920520-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `920603-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `920721-3.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `920728-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `920731-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `920812-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `920908-2.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `920909-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `921013-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `921110-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `921123-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `921124-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `921207-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `921218-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `921218-2.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930106-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930111-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930208-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930429-2.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930513-2.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930518-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930527-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930603-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930603-3.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930621-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930622-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930630-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930702-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930713-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930718-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930719-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930818-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930916-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930921-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930929-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `930930-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `931009-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `931012-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `931017-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `931018-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `931031-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `931102-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `931102-2.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `931208-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `931228-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `940115-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `940122-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `941101-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `941110-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `950322-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `950426-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `950512-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `950605-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `950621-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `960218-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `960402-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `960830-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `961112-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `990106-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `cmpdi-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `cmpsf-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `cmpsi-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `cmpsi-2.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `dbra-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `divmod-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `inst-check.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `int-compare.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `loop-2.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `loop-2d.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `loop-3.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `loop-3b.c` | T/T/P/P | T/P/P/P | Signed overflow remains in I0; I1 now passes |
| `loop-3c.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `mod-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `pr55875.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |
| `tstdi-1.c` | T/T/P/P | P/P/P/P | Resolved: implicit library prototype |

The table below covers every remaining source with a wrong result, link
error, interpreter trap, or arm disagreement. P/T/W/L mean
pass/trap/wrong-result/link-error. There are now 87 sources after the
implicit-prototype follow-up; its 81 resolved sources are listed above. Categories distinguish valid C99 lowering gaps,
interpreter capability gaps, GNU extensions and test/runtime assumptions.
The full before/after per-source classifications are also in
`target/compiler-corpus/execute-implicit-after/triage.json`.

| Source | I0/I1/A0/A1 | Classification | Cause |
| --- | --- | --- | --- |
| `20000227-1.c` | W/W/W/W | GCC/source assumption | Expects a Latin-1 source byte to become one execution byte; bcc uses UTF-8. |
| `20001101.c` | T/T/P/P | Test: undefined C operation | Old-style pointer parameter receives integer zero without prototype conversion (C99 6.5.2.2p6). |
| `20001121-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `20010904-1.c` | W/W/W/W | GNU feature | Typedef-level aligned attribute is not modeled; requires aligned type layout through arrays and containing records. |
| `20010904-2.c` | W/W/W/W | GNU feature | Typedef-level aligned attribute is not modeled; requires aligned type layout through arrays and containing records. |
| `20011121-1.c` | T/T/P/P | Test: pointer assumption | GNU null-based member-offset idiom; runtime inbounds arithmetic on null traps. Use __builtin_offsetof for supported C99 code. |
| `20020107-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `20020413-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `20020508-2.c` | T/T/P/W | Test: undefined C operation | Signed left shift of a negative value or an unrepresentable positive result (C99 6.5.7p4). |
| `20020508-3.c` | T/T/P/W | Test: undefined C operation | Signed left shift of a negative value or an unrepresentable positive result (C99 6.5.7p4). |
| `20021127-1.c` | W/W/W/W | GCC/library assumption | Expects builtin llabs folding to override its own aborting llabs definition; bcc calls the provided body. |
| `20030316-1.c` | T/T/P/P | Test: undefined C operation | Addition overflows 32-bit Windows long; would not overflow an LP64 long (C99 6.5p5). |
| `20030404-1.c` | T/P/P/P | Test: indeterminate input | Reads an uninitialized automatic value; optimization can eliminate the poison use (C99 6.7.8p10). |
| `20030914-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `20031003-1.c` | T/T/W/W | Test: undefined C operation | Converts 2147483648.0 to 32-bit int, outside its range (C99 6.3.1.4p1). |
| `20040208-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `20040409-1w.c` | T/T/P/W | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `20040409-2w.c` | T/T/P/W | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `20040409-3w.c` | T/T/P/W | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `20040709-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `20051012-1.c` | T/T/P/P | GCC/untyped call assumption | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `20090527-1.c` | T/T/P/P | Test: indeterminate input | Reads an uninitialized automatic value; optimization can eliminate the poison use (C99 6.7.8p10). |
| `20230630-2.c` | W/W/W/W | GNU feature | scalar_storage_order attribute needs reversed scalar/bit-field representation and access lowering. |
| `20230630-4.c` | W/W/W/W | GNU feature | scalar_storage_order attribute needs reversed scalar/bit-field representation and access lowering. |
| `920501-1.c` | T/T/P/P | GCC/untyped call assumption | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `920612-1.c` | T/T/P/P | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `920711-1.c` | T/T/P/P | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `920730-1.c` | T/T/P/P | Test: undefined C operation | INT_MAX + 2 overflows before the cast to unsigned (C99 6.5p5). |
| `920922-1.c` | T/T/P/P | Test: undefined C operation | Forms a pointer beyond the object and its one-past position (C99 6.5.6p8). |
| `921202-1.c` | T/T/P/P | GCC/untyped call assumption | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `921208-2.c` | T/T/P/P | GCC/untyped call assumption | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `930526-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `930529-1.c` | T/T/P/P | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `930622-2.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `941202-1.c` | T/T/L/L | GNU runtime / missing symbol | Requires compiler stack-allocation lowering; MinGW has no external alloca symbol. Dynamic allocation and lifetime support are not implemented. |
| `950704-1.c` | T/T/P/W | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `960215-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `960405-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `960513-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `961223-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `970217-1.c` | W/W/W/W | Lowering: large feature | Valid C99 parameter VLA bound side effects are not evaluated on function entry (6.9.1p10); needs retained runtime bounds and entry evaluation. |
| `980526-2.c` | T/T/P/P | Test: undefined C operation | Signed major << 22 produces an unrepresentable int (C99 6.5.7p4). |
| `980608-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `991014-1.c` | T/T/P/P | Test: pointer assumption | GNU null-based member-offset idiom; runtime inbounds arithmetic on null traps. Use __builtin_offsetof for supported C99 code. |
| `alias-2.c` | T/T/L/L | GNU feature / missing symbol | GNU alias attributes need an IR/backend alias model; native links lack the alias. Interpreter correctly rejects the unresolved external object. |
| `alias-3.c` | T/T/L/L | GNU feature / missing symbol | GNU alias attributes need an IR/backend alias model; native links lack the alias. Interpreter correctly rejects the unresolved external object. |
| `alias-4.c` | T/T/L/L | GNU feature / missing symbol | GNU alias attributes need an IR/backend alias model; native links lack the alias. Also uses nonstandard main(int). |
| `align-2.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `arith-rand-ll.c` | T/P/P/P | Test: undefined C operation | Random values exercise signed ABS at the minimum representable value; optimized algebra can remove this poison use. |
| `arith-rand.c` | T/P/P/P | Test: undefined C operation | Random values exercise signed ABS at the minimum representable value; optimized algebra can remove this poison use. |
| `bcp-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `bf-sign-2.c` | W/W/W/W | GNU feature | Requires promotions of implementation-defined long/long-long bit-fields by declared bit width. |
| `bitfld-3.c` | W/W/W/W | GNU feature | Requires GNU arithmetic at extended bit-field precision, including 33/40/41-bit operations. |
| `bswap-2.c` | T/T/P/P | Test: undefined C operation | Integer promotions make the sign-bit left shift signed int; its result is unrepresentable in C99 (6.5.7p4). |
| `conversion.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `eeprof-1.c` | W/W/W/W | GCC/runtime assumption | Requires unapplied -finstrument-functions and the associated entry/exit instrumentation. |
| `loop-2c.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `loop-2e.c` | T/T/P/P | Test: pointer assumption | Arithmetic/member addressing on a forged integer pointer with no backing object; outside the interpreter object model (C99 6.5.6p8). |
| `loop-3b.c` | T/P/P/P | Test: signed overflow | i * 4 overflows signed int before a scalar noundef call argument; C99 6.5p5. I1 now passes with the corrected exit prototype. |
| `pr101188.c` | T/T/P/P | Test: indeterminate input | Passes uninitialized s.a to an indirect scalar noundef parameter; native Clang also emits noundef for it. Existing documented Clang-compatible indeterminate-argument policy applies. |
| `pr103255.c` | T/T/P/P | Test: pointer assumption | Arithmetic/member addressing on a forged integer pointer with no backing object; outside the interpreter object model (C99 6.5.6p8). |
| `pr22493-1.c` | T/T/P/W | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `pr23047.c` | T/P/P/P | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `pr32244-1.c` | W/W/W/W | GNU feature | Requires shift arithmetic at GNU 40-bit bit-field precision. |
| `pr34099-2.c` | T/T/P/P | Test: indeterminate input | Reads an uninitialized automatic value; optimization can eliminate the poison use (C99 6.7.8p10). |
| `pr34099.c` | T/T/P/P | Test: indeterminate input | Reads an uninitialized automatic value; optimization can eliminate the poison use (C99 6.7.8p10). |
| `pr34176.c` | T/T/P/P | Test: undefined C operation | Increments and subtracts a null pointer, outside an array object (C99 6.5.6p8-9). |
| `pr34971.c` | W/W/W/W | GNU feature | Requires rotate arithmetic at GNU 40-bit bit-field precision. |
| `pr36339.c` | P/P/W/W | Target assumption | Truncates a 64-bit stack pointer through 32-bit unsigned long on LLP64; GCC and Clang fault at both O0/O2 on this host. |
| `pr40386.c` | T/T/P/W | Test: undefined C operation | Signed left shift of a negative value or an unrepresentable positive result (C99 6.5.7p4). |
| `pr43629.c` | T/W/P/W | Test: indeterminate input | Reads an uninitialized automatic value; optimization can eliminate the poison use (C99 6.7.8p10). |
| `pr56051.c` | T/T/P/P | Test: undefined C operation | Integer promotions make the sign-bit left shift signed int; its result is unrepresentable in C99 (6.5.7p4). |
| `pr57124.c` | T/T/P/P | Test: undefined C operation | Requires -fno-strict-overflow; promoted unsigned-short multiplication overflows signed int. |
| `pr67037.c` | T/T/P/P | GCC/untyped call assumption | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `pr7284-1.c` | T/T/P/P | Test: undefined C operation | Requires -std=c89 for implementation-defined signed shift; the default GNU17 survey uses C99 shift rules. |
| `pr77767.c` | W/W/W/W | Lowering: large feature | Valid C99 parameter VLA bound side effects are not evaluated on function entry (6.9.1p10); needs retained runtime bounds and entry evaluation. |
| `pr99079.c` | T/T/P/W | Test: undefined C operation | Integer promotions make the sign-bit left shift signed int; its result is unrepresentable in C99 (6.5.7p4). |
| `ptr-arith-1.c` | T/T/P/P | Test: undefined C operation | Forms a pointer beyond the object and its one-past position (C99 6.5.6p8). |
| `pushpop_macro.c` | W/W/W/W | GNU feature | Requires push_macro/pop_macro pragma stacks, which are not implemented. |
| `regstack-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `restrict-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `return-addr.c` | T/T/P/P | Test: undefined C operation | Forms before-object pointers and compares unrelated objects; source comments acknowledge undefined behavior. |
| `shiftdi-2.c` | T/T/P/P | Test: undefined C operation | Signed left shift of a negative value or an unrepresentable positive result (C99 6.5.7p4). |
| `string-opt-18.c` | T/T/P/P | Test: indeterminate input | Overlapping memcpy/strcpy on an uninitialized buffer (C99 7.21.2.1p2). |
| `unroll-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `va-arg-7.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `va-arg-8.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |


Evidence is retained under `target/`: the correctness and implicit-prototype
survey manifests, summaries,
source hashes and per-arm logs; `compiler-corpus/execute-correctness-reference/results.json`
contains exact reference compiler commands and exit statuses;
`torture-signature-evidence.json` names every failing signature instruction
and declaration, with emitted IR beside its baseline log. The scratch
reference/triage drivers never write the corpus. These files are local
evidence artifacts, not new source inputs or waived tests.

All eight requested gates passed for the implicit-prototype update, with
compiler changes complete before the final checks:

| Check | Result |
| --- | --- |
| `cargo test --all-targets` | Passed: 1,724 tests across 20 targets; six existing ignored tests. |
| `cargo test --features benchmarking-internals --test allocation_count` | Passed: 25 tests. |
| `cargo +nightly fmt --check` | Passed, after running `cargo +nightly fmt`. |
| `cargo clippy --all-targets -- -D warnings` | Passed. |
| `cargo clippy --release --all-targets -- -D warnings` | Passed. |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | Passed. |
| `git diff --check` | Passed, including the final report. |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | Passed: six checks, two existing ignored long sweeps. |

The full test run finished before the feature allocation build, and all
subsequent Cargo checks ran sequentially. Logs and exact command exit statuses
for this update are `target/implicit-checks.json` and
`target/implicit-check-*.log`; the original correctness logs remain in
`target/correctness-checks-all.json`, `target/correctness-checks-rest.json` and
`target/correctness-check-*.log`. During development the intended prototype
change required updating the existing implicit `putchar` assertion, while
adding an explicit check for the unknown-name open signature. The warning
golden now renders the complete library type. No behavioral assertions,
survey classifications or ignored-test settings were weakened. The whole corpus remains an exploratory run with recorded unsupported
features, not a passing DejaGnu profile or completed Stage 6 acceptance criterion.

Finally, `cargo build --bin bcc-rust` passed and restored the ordinary debug
CLI after feature checks. Its `--print-default-passes` command exits normally.

The whole-file commit plan is `target/commit-plan.md`. No files were staged
or committed, and no Git metadata writes were performed.

### GNU89 inline and supported dg-options: 2026-10-12

This update starts at `b78cdbae` on `claude/torture-correctness` and compares
against `execute-implicit-after`, the preceding snapshot. The final survey
uses the same 1,677 sources, eight workers, ten-second deadlines and four
Windows GNU arms, now with supported source options applied. Every source
SHA-512 matches the baseline. Both shared LLVM junctions were preserved.

```powershell
cargo build --release --bin bcc-rust
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-gnu89-after --jobs 8 --timeout 10
```

| Class | I0 before | after | I1 before | after | A0 before | after | A1 before | after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pass | 1145 | 1151 | 1150 | 1156 | 1200 | 1206 | 1190 | 1196 |
| frontend-error | 139 | 139 | 139 | 139 | 139 | 139 | 139 | 139 |
| lowering-unsupported | 307 | 311 | 307 | 311 | 307 | 311 | 307 | 311 |
| abi-unsupported | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| compiler-crash | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| interpreter-trap | 72 | 61 | 66 | 55 | 0 | 0 | 0 | 0 |
| wrong-result | 14 | 15 | 15 | 16 | 16 | 17 | 26 | 27 |
| timeout | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| link-error | 0 | 0 | 0 | 0 | 15 | 4 | 15 | 4 |

There are **87 -> 77 adverse sources** and **73 -> 62 disagreements**. All
eleven named inline sources lose their native link failures. Six now pass all
four arms; four expose lowering capability gaps; one exposes a builtin wrong
result. All 44 changed arms are confined to those eleven sources. No previously
passing arm regresses. Frontend errors are unchanged; compiler crashes and
timeouts are zero.

The survey returns 1 because adverse sources and unsupported features remain.
Its elapsed time is 400.239 seconds, an uncontrolled observation rather
than a performance comparison. The initial run, preserved as
`execute-gnu89-contended`, had seven native compilation timeouts while the
full test build linked multiple LTO binaries. All seven arms pass in the
final repeat, which ran after all Cargo checks and builds had finished.

The compiler now selects GNU89 inline semantics before C99, including the
C95 amendment, and C99 semantics from C99 onward. `-fgnu89-inline` and
`-fno-gnu89-inline` override that default independently of `-std`; the last
inline flag wins. An explicit C99-inline override is also accepted in a
pre-C99 mode, a bcc extension beyond GCC's accepted flag combinations.
The existing `gnu_inline`/`__gnu_inline__` attribute selects GNU semantics
per function, and `__has_attribute(gnu_inline)` now reports 1. The predefined
`__GNUC_GNU_INLINE__` or `__GNUC_STDC_INLINE__` follows the global mode:
exactly one is defined, with value 1. Frozen target macro tables are unchanged.

GNU plain inline definitions emit an external body; GNU extern-inline
bodies remain inline-only even after an ordinary declaration. Those facts
are retained per definition in semantic analysis and consumed by lowering,
so interpreter and LLVM emission share one decision. C99 inline-only
classification still considers all file-scope declarations; a declaration
with extern or without inline makes the body external. Static inline remains
internal. The existing Microsoft ABI behavior in C99 mode is retained.
The emission rules cite C99 §6.7.4p6-7 (pp. 112-113; PDF pp. 124-125),
§6.9p5 (p. 140; PDF p. 152), [GCC Inline](https://gcc.gnu.org/onlinedocs/gcc/Inline.html)
and [GCC's inline option](https://gcc.gnu.org/onlinedocs/gcc/C-Dialect-Options.html#index-fgnu89-inline).

IR inspection covers plain, extern and static inline, declarations before
and after a body, block-scope declarations, attribute inheritance and
replacement bodies. Semantic tests check mode-dependent body restrictions
and duplicate definitions. CLI regressions run two-function programs through
unoptimized/optimized interpretation and native O0, including function
addresses, and check that inline-only calls remain external. Macro tests
cover both overrides; bundled Clang O0/O2 and macro probes independently
confirm the defaults and supported combinations.

The runner parses unconditional `dg-options` and `dg-additional-options`,
preserving order: the former replaces the base options, and the latter
appends additional options. It applies the CLI's supported language,
preprocessing and header-search flags, including `-f[no-]gnu89-inline`,
supported `-std` aliases, `-w`, pedantic policy, hosted/freestanding and MSVC
flags, definitions, undefinitions and forced includes. `-w` now suppresses
warning rendering and counts without suppressing errors. Unknown flags
remain recorded; target/effective-target selectors are deferred rather than
evaluated, and backend/output controls cannot replace the fixed survey arms.
Additional-source and target-skip behavior is unchanged. Applied flags,
unsupported tokens and option directive decisions are recorded per source
in both results and the manifest, beside each exact compiler command.

102 sources receive supported options: 75 request GNU89, 13 request GNU17,
12 request GNU89 inline, one requests C99 and one requests C89. These
language selections cause no other status transitions. `p18298.c` is the
twelfth inline-option source; it remains F/F/F/F because `stdlib.h` is not
available under the survey's header-search configuration.

Arm order is I0/I1/A0/A1. P = pass, T = interpreter trap, L = link error,
U = lowering unsupported, W = wrong result, F = frontend error.

| Source | Before | After | Reclassification |
| --- | --- | --- | --- |
| `20001121-1.c` | T/T/L/L | P/P/P/P | Resolved: GNU89 external inline body |
| `20020107-1.c` | T/T/L/L | U/U/U/U | Lowering: inline assembly in the newly emitted body |
| `930526-1.c` | T/T/L/L | P/P/P/P | Resolved: GNU89 external inline body |
| `961223-1.c` | T/T/L/L | P/P/P/P | Resolved: GNU89 external inline body |
| `980608-1.c` | T/T/L/L | U/U/U/U | Lowering: va_start in the newly emitted body |
| `bcp-1.c` | T/T/L/L | W/W/W/W | Builtin semantics: constant_p rejects a string literal |
| `loop-2c.c` | T/T/L/L | P/P/P/P | Resolved: GNU89 external inline body |
| `restrict-1.c` | T/T/L/L | P/P/P/P | Resolved: GNU89 external inline body |
| `unroll-1.c` | T/T/L/L | P/P/P/P | Resolved: GNU89 external inline body |
| `va-arg-7.c` | T/T/L/L | U/U/U/U | Lowering: va_start in the newly emitted body |
| `va-arg-8.c` | T/T/L/L | U/U/U/U | Lowering: va_start in the newly emitted body |
| `p18298.c` | F/F/F/F | F/F/F/F | Unchanged: missing stdlib.h |
| `pr7284-1.c` | T/T/P/P | T/T/P/P | Updated triage: missing C89 signed-shift policy |

`20020107-1.c` now exposes its empty output/input-constraint asm at line 20;
lowering reports unsupported inline assembly. The three varargs sources
now expose unsupported `__builtin_va_start` in their emitted `debug` bodies:
`980608-1.c:17`, `va-arg-7.c:16`, and `va-arg-8.c:20`. These are capability
refusals, with valid source bodies retained and checked, rather than residual
inline linkage failures.

`bcp-1.c` now aborts in all four arms. Its `good1` requires
`__builtin_constant_p("hi") == 1`; bcc currently folds only arithmetic
constants and emits 0 for this string literal. A two-function reduction
returns 1 in bcc's interpreter, while GCC 13.2.0 on PATH returns 0 at O0/O2.
The complete unmodified source also passes GCC at O0/O2 with
`-std=gnu17 -fgnu89-inline`. This is a demonstrated builtin semantic gap;
no builtin tables were changed.

`pr7284-1.c` remains T/T/P/P, but now receives its requested `-std=c89`.
Its previous “unapplied option / undefined C99 shift” explanation no longer
applies. The interpreter still observes poison for the signed shift in
`f(128)`, so it is reclassified as a missing C89 signed-shift policy in
lowering. C89 §3.3.7 (p. 49; PDF p. 63) specifies the bit shift without
C99's signed representability condition. GCC 13.2.0 O0/O2 and both native
bcc arms pass. The remaining native link errors are `941202-1.c`, which
requires compiler stack-allocation lowering for `alloca`, and `alias-2.c`,
`alias-3.c`, and `alias-4.c`, which require GNU symbol aliases. Those
limitations are unchanged.

Other adverse sources receiving standard options retain their existing
diagnoses: `20051012-1.c`, `920501-1.c`, `921202-1.c`, `921208-2.c`, and
`pr67037.c` have untyped-call/signature assumptions; `920730-1.c` overflows
signed addition before its unsigned cast; `loop-3b.c` overflows signed
multiplication; `20021127-1.c` expects builtin `llabs` folding despite its
own aborting definition. None is repaired by selecting its requested
standard, and signed addition/multiplication overflow remains undefined
before C99 too. Their statuses are unchanged.

| Check | Result |
| --- | --- |
| `cargo test --all-targets` | Passed: 1,732 tests across 20 targets; six existing ignored tests. |
| `cargo test --features benchmarking-internals --test allocation_count` | Passed: 25 tests. |
| `cargo +nightly fmt --check` | Passed, after cargo +nightly fmt. |
| `cargo clippy --all-targets -- -D warnings` | Passed. |
| `cargo clippy --release --all-targets -- -D warnings` | Passed. |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | Passed. |
| `git diff --check` | Passed. |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | Passed with BCC_DIFFTEST_CASES=1000: six checks; two existing ignored long sweeps. |
| `python scripts/run.py --help` | Passed. |
| `python -m unittest discover -s scripts -p test_run_gcc_torture*.py` | Passed: 26 tests. |
| `cargo build --release --bin bcc-rust` | Passed. |
| `cargo build --bin bcc-rust` | Passed. |

All Cargo checks ran sequentially in the final validation pass. No
benchmarking-internals build overlapped CLI integration tests. After the
feature checks, `cargo build --bin bcc-rust` restored the plain CLI; the
final handoff repeats that build and verifies `--print-default-passes` exits
normally. The first development Clippy pass identified an independent-switch
struct lint and a single-character test pattern; both were addressed before
the complete final check pass. No test assertions or ignored-test settings
were weakened.

Evidence is retained in `target/compiler-corpus/execute-gnu89-after/`
(manifest, summaries, source hashes, arm logs and `transitions.json`),
`target/gnu89-checks.json`, `target/gnu89-final-*.log`,
`target/gnu89-reference/clang.json` and `target/gnu89-triage/`. The source
hash comparison and option/command checks were rerun against the final
manifest. This remains an exploratory survey, not a passing DejaGnu profile.
The whole-file commit plan is `target/commit-plan.md`. Nothing was staged
or committed, and no Git metadata was modified.

## 2026-10-12: common GCC builtins and inexpensive GNU lowering

This section records a fresh before/after survey at base commit
`0a75654397d5c65825a5bdc36ec12a77727ee350` on `claude/torture-builtins`.
Both runs read the existing GCC 15.2.0 corpus without editing it, select the same
1,677 top-level execute sources, and use eight jobs, a ten-second deadline, GNU17,
and `x86_64-w64-windows-gnu`. DejaGnu directives remain recorded rather than applied.
The shared `target/llvm` and `target/llvm-libs` junctions were preserved.

```powershell
cargo build --release
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-builtins-before --jobs 8 --timeout 10
# After the implementation and another release build:
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-builtins-after --jobs 8 --timeout 10
```

Manifests, compiler hashes, per-source results, command lines, diagnostics, and
execution logs are in those two output directories. The final compiler SHA-512 is
`e49974c66bf23a180236562cc50c1439234e885363453d1e554b4f5ed64d79783eea3b18b6b105e79e7babeb3ee2ccaa11332268404f727eb75b358d0943aba1`.
Intermediate surveys are retained separately at `execute-builtins-after-initial`
and `execute-builtins-after-pre-final`; the tables below use the final run only.

### Counts per arm

Each cell is **before → after**. I0 is the interpreter without bcc optimization;
I1 adds bcc optimization; A0 uses LLVM O0; A1 uses bcc optimization and LLVM O2.

| Outcome | I0 | I1 | A0 | A1 |
| --- | ---: | ---: | ---: | ---: |
| pass | 1062 → 1137 | 1066 → 1141 | 1199 → 1279 | 1189 → 1269 |
| frontend-error | 139 → 138 | 139 → 138 | 139 → 138 | 139 → 138 |
| lowering-unsupported | 307 → 224 | 307 → 224 | 307 → 224 | 307 → 224 |
| abi-unsupported | 0 → 4 | 0 → 4 | 0 → 4 | 0 → 4 |
| interpreter-trap | 154 → 158 | 149 → 153 | 0 → 0 | 0 → 0 |
| wrong-result | 15 → 16 | 16 → 17 | 17 → 17 | 27 → 27 |
| link-error | 0 → 0 | 0 → 0 | 15 → 15 | 15 → 15 |
| timeout | 0 → 0 | 0 → 0 | 0 → 0 | 0 → 0 |
| compiler-crash | 0 → 0 | 0 → 0 | 0 → 0 | 0 → 0 |

All-four-arm passes rise from 1061 to 1136. There are
80 newly executable sources, of which 75 pass all four arms. The combined
frontend/lowering refusal count falls from 446 to 362 per arm; four sources now
reach the existing extended-floating-point ABI refusal. Arm disagreements rise
from 155 to 160 as additional sources reach execution. No compiler crash occurs.
The final run completes in 331.0 seconds with no deadlines or compiler crashes.
The intermediate pre-final survey had two A0 compile deadlines (`pr56899.c` and
`pr57124.c`); both pass in this full final run and in the earlier isolated recheck
at `target/compiler-corpus/execute-builtins-timeout-recheck`.


### Features and sources unblocked

Counts below are distinct newly executable sources per family. Families overlap:
`pr19449.c` needs both the hint/query path and `__builtin_choose_expr`. A builtin
name that appears zero times in the first-refusal inventory still has focused
tests and support; first refusals do not enumerate latent dependencies.

| Added support | Newly executable sources |
| --- | ---: |
| Existing IR varargs operations connected to `__builtin_va_start/arg/copy` | 28 |
| Generic and typed add/sub/mul overflow, and `*_overflow_p` | 14 |
| Expect/probability, assume-aligned, unreachable, trap, prefetch, constant/object-size queries | 14 |
| clz/ctz/popcount/parity/ffs/clrsb and l/ll variants; bswap16/32/64 | 9 |
| Statement expressions, including lvalue results | 7 |
| Dynamic `__builtin_alloca` and ordinary GNU `alloca` | 5 |
| `__builtin_choose_expr` lvalue-preserving lowering | 1 |
| Existing constant `__builtin_offsetof` semantic result accepted by lowering | 1 |
| Case ranges lowered to bounded comparisons, without enumerating values | 1 |
| Omitted-middle conditional with one evaluation of its first operand | 1 |

The alloca implementation adds `dyn_alloca` to IR with function-lifetime storage,
16-byte alignment, verifier/parser/printer support, interpreter frame cleanup,
and LLVM dynamic array allocation. Functions containing it are excluded from
inlining because extending the allocation lifetime would change the contract.
Bit operations use `llvm.ctlz`, `llvm.cttz`, `llvm.ctpop`, and `llvm.bswap`;
zero clz/ctz is poison. Overflow uses exact sign/magnitude arithmetic through
128 bits plus carry, and LLVM unsigned add/mul overflow intrinsics. Typed result
pointers retain the destination type; predicates retain bit-field precision and
evaluate third-operand effects while ignoring its value. Prefetch evaluates its
arguments but emits no prefetch operation. Constant-p returns one only for an
integer constant expression. Both object-size queries conservatively return the
documented unknown answer (all ones for modes 0/1, zero for modes 2/3), without
evaluating the queried pointer. GNU builtins use the shared extension-policy gate.

Contracts follow GCC's [Other Builtins](https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html),
[Bit Operation Builtins](https://gcc.gnu.org/onlinedocs/gcc/Bit-Operation-Builtins.html),
[Byte-Swapping Builtins](https://gcc.gnu.org/onlinedocs/gcc/Byte-Swapping-Builtins.html),
[Integer Overflow Builtins](https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html),
[Object Size Checking](https://gcc.gnu.org/onlinedocs/gcc/Object-Size-Checking.html),
and [Stack Allocation](https://gcc.gnu.org/onlinedocs/gcc/Stack-Allocation.html).
`middle-end.md`, `semantic-analysis.md`, and `language-standards.md` record the
implementation boundaries. No optimizer or ABI correctness repairs outside this
feature support are included.

The statement-expression sources are `20000703-1.c`, `20000917-1.c`,
`20001203-2.c`, `20020206-1.c`, `20020320-1.c`, `950906-1.c`, and `compndlit-1.c`.
The ordinary-alloca frontend recovery is `20020314-1.c`; the constant offsetof
recovery is `pr30778.c`; case ranges unblock `pr34154.c`; omitted-middle
conditionals unblock `pr68249.c`. Remaining family source lists are recorded in
the per-builtin table below.

### Exact first-refusal builtin inventory

The baseline contains 111 sources with an unanalyzed semantic type and 54 with
“this builtin”; these are diagnostic construct counts, not 165 independent
builtin names. The following inventory extracts only builtin tokens under the
primary diagnostic underline, avoiding incidental names elsewhere in a function
or macro expansion. Counts are distinct sources and can overlap. “Unblocked”
means that source subsequently reaches interpretation rather than a refusal.
Already recognized aliases such as memcpy-family/library names can appear here
because a surrounding type is opaque; their appearance is not evidence that the
library call itself was newly implemented. Non-builtin opaque-type excerpts are
listed separately. The raw JSON inventories are retained alongside results.

| Name at baseline refusal | Before | Unblocked | After first refusal | Baseline sources |
| --- | ---: | ---: | ---: | --- |
| `__builtin_va_start` | 50 | 28 | 0 | `20000519-1.c`, `20041113-1.c`, `20041214-1.c`, `20071213-1.c`, `920625-1.c`, `920908-1.c`, `931004-10.c`, `931004-12.c`, `931004-14.c`, `931004-2.c`, `931004-4.c`, `931004-6.c`, `931004-8.c`, `980205.c`, `980716-1.c`, `991216-2.c`, `multi-ix.c`, `pr44575.c`, `pr44942.c`, `pr56205.c`, `pr64979.c`, `pr92904.c`, `stdarg-1.c`, `stdarg-2.c`, `stdarg-3.c`, `stdarg-4.c`, `strct-stdarg-1.c`, `strct-varg-1.c`, `va-arg-1.c`, `va-arg-10.c`, `va-arg-11.c`, `va-arg-12.c`, `va-arg-13.c`, `va-arg-14.c`, `va-arg-15.c`, `va-arg-16.c`, `va-arg-17.c`, `va-arg-18.c`, `va-arg-19.c`, `va-arg-2.c`, `va-arg-20.c`, `va-arg-22.c`, `va-arg-23.c`, `va-arg-26.c`, `va-arg-4.c`, `va-arg-5.c`, `va-arg-6.c`, `va-arg-9.c`, `va-arg-pack-1.c`, `va-arg-trap-1.c` |
| `__builtin_alloca` | 9 | 4 | 0 | `20021113-1.c`, `20070824-1.c`, `alloca-1.c`, `built-in-setjmp.c`, `frame-address.c`, `pr36321.c`, `pr64242.c`, `pr84521.c`, `pr86528.c` |
| `__builtin_mul_overflow` | 9 | 9 | 0 | `pr108789.c`, `pr64006.c`, `pr68381.c`, `pr71554.c`, `pr84169.c`, `pr89434.c`, `pr91450-1.c`, `pr91450-2.c`, `pr93213.c` |
| `__builtin_va_arg` | 8 | 6 | 0 | `20000519-1.c`, `20071213-1.c`, `pr64979.c`, `stdarg-1.c`, `stdarg-2.c`, `va-arg-13.c`, `va-arg-20.c`, `va-arg-9.c` |
| `__builtin_prefetch` | 6 | 5 | 0 | `builtin-prefetch-1.c`, `builtin-prefetch-2.c`, `builtin-prefetch-3.c`, `builtin-prefetch-4.c`, `builtin-prefetch-5.c`, `builtin-prefetch-6.c` |
| `__builtin_add_overflow` | 5 | 5 | 0 | `pr108789.c`, `pr85095.c`, `pr90311.c`, `pr91635.c`, `pr93494.c` |
| `__builtin_expect` | 5 | 3 | 0 | `20020418-1.c`, `pr115277.c`, `pr43385.c`, `pr85156.c`, `pr91597.c` |
| `__builtin_ffs` | 4 | 4 | 0 | `ffs-1.c`, `ffs-2.c`, `pr61725.c`, `pr97325.c` |
| `__builtin_longjmp` | 4 | 0 | 4 | `built-in-setjmp.c`, `pr60003.c`, `pr64242.c`, `pr84521.c` |
| `__builtin_unreachable` | 4 | 4 | 0 | `20230509-1.c`, `20241029-1.c`, `pr81281.c`, `pr97888-1.c` |
| `__builtin_return_address` | 3 | 0 | 3 | `20010122-1.c`, `20030323-1.c`, `20030811-1.c` |
| `__builtin_bswap32` | 2 | 2 | 0 | `bswap-3.c`, `pr90025.c` |
| `__builtin_offsetof` | 2 | 1 | 0 | `pr30778.c`, `pr49390.c` |
| `__builtin_setjmp` | 2 | 0 | 4 | `pr60003.c`, `pr84521.c` |
| `__builtin_signbit` | 2 | 0 | 2 | `20080502-1.c`, `pr35456.c` |
| `__builtin_apply` | 1 | 0 | 1 | `pr47237.c` |
| `__builtin_apply_args` | 1 | 0 | 1 | `pr47237.c` |
| `__builtin_bswap16` | 1 | 1 | 0 | `pr97386-1.c` |
| `__builtin_bswap64` | 1 | 1 | 0 | `bswap-1.c` |
| `__builtin_choose_expr` | 1 | 1 | 0 | `pr19449.c` |
| `__builtin_clz` | 1 | 1 | 0 | `pr37780.c` |
| `__builtin_constant_p` | 1 | 1 | 0 | `pr19449.c` |
| `__builtin_copysign` | 1 | 0 | 1 | `pr44683.c` |
| `__builtin_ctz` | 1 | 1 | 0 | `pr37780.c` |
| `__builtin_frame_address` | 1 | 0 | 1 | `frame-address.c` |
| `__builtin_isinf` | 1 | 0 | 1 | `pr39228.c` |
| `__builtin_isinff` | 1 | 0 | 1 | `pr39228.c` |
| `__builtin_isinfl` | 1 | 0 | 1 | `pr39228.c` |
| `__builtin_memcmp` | 1 | 0 | 1 | `pr77718.c` |
| `__builtin_memset` | 1 | 0 | 1 | `pr77718.c` |
| `__builtin_mul_overflow_p` | 1 | 1 | 0 | `pr105984.c` |
| `__builtin_shuffle` | 1 | 0 | 1 | `pr94591.c` |
| `__builtin_snprintf` | 1 | 0 | 1 | `pr78622.c` |
| `__builtin_strcmp` | 1 | 0 | 1 | `921007-1.c` |
| `__builtin_trap` | 1 | 1 | 0 | `pr115033.c` |
| `__builtin_va_arg_pack` | 1 | 0 | 1 | `va-arg-pack-1.c` |
| `__builtin_va_copy` | 1 | 1 | 0 | `va-arg-10.c` |

<details>
<summary>Exact non-builtin opaque-type baseline excerpts</summary>

- non-builtin opaque type (61): `20000917-1.c`, `20001203-2.c`, `20010518-2.c`, `20020108-1.c`, `20020206-1.c`, `20020227-1.c`, `20020320-1.c`, `20030224-2.c`, `20040625-1.c`, `20040709-2.c`, `20040709-3.c`, `20050107-1.c`, `20050119-1.c`, `20050119-2.c`, `20051113-1.c`, `20100430-1.c`, `20111212-1.c`, `20120105-1.c`, `20170401-1.c`, `20190820-1.c`, `960117-1.c`, `960416-1.c`, `990208-1.c`, `bf-pack-1.c`, `cbrt.c`, `memclr.c`, `memcpy-a1.c`, `memcpy-a2.c`, `memcpy-a4.c`, `memcpy-a8.c`, `misalign.c`, `nest-align-1.c`, `packed-1.c`, `packed-2.c`, `packed-aligned.c`, `pr108064.c`, `pr114552.c`, `pr22141-2.c`, `pr23135.c`, `pr23467.c`, `pr29006.c`, `pr45070.c`, `pr52979-1.c`, `pr52979-2.c`, `pr57344-1.c`, `pr57344-2.c`, `pr57344-3.c`, `pr57344-4.c`, `pr65369.c`, `pr68532.c`, `pr70602.c`, `pr71083.c`, `pr77718.c`, `simd-1.c`, `strct-pack-1.c`, `strct-pack-2.c`, `strct-pack-3.c`, `strct-pack-4.c`, `struct-cpy-1.c`, `struct-ret-2.c`, `zerolen-2.c`

</details>

### Remaining unsupported constructs

These counts describe the final interpreter diagnostic and can overlap within a
source. Newly revealed VLA/local-label refusals explain increases in those rows.
The 138 frontend-error sources are predominantly unavailable hosted headers;
they are retained as frontend errors, not recategorized as builtin coverage.

| Construct | Before | After | Remaining sources |
| --- | ---: | ---: | --- |
| type not analyzed by semantic analysis | 111 | 66 | `20010122-1.c`, `20010518-2.c`, `20020108-1.c`, `20020227-1.c`, `20030224-2.c`, `20030323-1.c`, `20030811-1.c`, `20040625-1.c`, `20040709-2.c`, `20040709-3.c`, `20050107-1.c`, `20050119-1.c`, `20050119-2.c`, `20051113-1.c`, `20080502-1.c`, `20100430-1.c`, `20111212-1.c`, `20120105-1.c`, `20170401-1.c`, `20190820-1.c`, `960117-1.c`, `960416-1.c`, `990208-1.c`, `alloca-1.c`, `bf-pack-1.c`, `built-in-setjmp.c`, `cbrt.c`, `frame-address.c`, `memclr.c`, `memcpy-a1.c`, `memcpy-a2.c`, `memcpy-a4.c`, `memcpy-a8.c`, `misalign.c`, `packed-1.c`, `packed-2.c`, `packed-aligned.c`, `pr108064.c`, `pr22141-2.c`, `pr23135.c`, `pr23467.c`, `pr29006.c`, `pr35456.c`, `pr39228.c`, `pr44683.c`, `pr45070.c`, `pr47237.c`, `pr52979-1.c`, `pr52979-2.c`, `pr60003.c`, `pr64242.c`, `pr65369.c`, `pr68532.c`, `pr70602.c`, `pr71083.c`, `pr77718.c`, `pr78622.c`, `pr84521.c`, `simd-1.c`, `strct-pack-1.c`, `strct-pack-2.c`, `strct-pack-3.c`, `strct-pack-4.c`, `struct-cpy-1.c`, `struct-ret-2.c`, `zerolen-2.c` |
| inline assembly | 61 | 61 | `20001009-2.c`, `20030222-1.c`, `20061031-1.c`, `20071211-1.c`, `20071220-1.c`, `20071220-2.c`, `20080122-1.c`, `960312-1.c`, `990130-1.c`, `990413-2.c`, `bitfld-5.c`, `pr103376.c`, `pr108498-1.c`, `pr110252-2.c`, `pr114552.c`, `pr38533.c`, `pr40022.c`, `pr40657.c`, `pr41239.c`, `pr43385.c`, `pr43560.c`, `pr44852.c`, `pr45695.c`, `pr47925.c`, `pr49218.c`, `pr49279.c`, `pr49390.c`, `pr51581-1.c`, `pr51581-2.c`, `pr51877.c`, `pr51933.c`, `pr52286.c`, `pr56205.c`, `pr56866.c`, `pr57344-1.c`, `pr57344-2.c`, `pr57344-3.c`, `pr57344-4.c`, `pr58277-1.c`, `pr58419.c`, `pr63641.c`, `pr65053-1.c`, `pr65053-2.c`, `pr65648.c`, `pr65956.c`, `pr68328.c`, `pr69691.c`, `pr78438.c`, `pr78726.c`, `pr79354.c`, `pr79737-2.c`, `pr80421.c`, `pr81588.c`, `pr82954.c`, `pr84478.c`, `pr84524.c`, `pr85156.c`, `pr85756.c`, `pr88904.c`, `pr93945.c`, `pr94130.c` |
| nested functions | 29 | 29 | `20000822-1.c`, `20010209-1.c`, `20010605-1.c`, `20030501-1.c`, `20040520-1.c`, `20061220-1.c`, `20090219-1.c`, `920415-1.c`, `920428-2.c`, `920501-7.c`, `920612-2.c`, `920721-4.c`, `921017-1.c`, `921215-1.c`, `931002-1.c`, `comp-goto-2.c`, `nest-align-1.c`, `nest-stdar-1.c`, `nestfunc-1.c`, `nestfunc-2.c`, `nestfunc-3.c`, `nestfunc-5.c`, `nestfunc-6.c`, `nestfunc-7.c`, `pr103405.c`, `pr22061-3.c`, `pr22061-4.c`, `pr24135.c`, `pr71494.c` |
| vector types | 20 | 20 | `20050316-1.c`, `20050316-2.c`, `20050316-3.c`, `20050604-1.c`, `20050607-1.c`, `20060420-1.c`, `pr105613.c`, `pr109040.c`, `pr110817-1.c`, `pr110817-2.c`, `pr60960.c`, `pr70903.c`, `pr85169.c`, `pr94524-1.c`, `pr94524-2.c`, `pr94591.c`, `simd-2.c`, `simd-4.c`, `simd-5.c`, `simd-6.c` |
| structures and unions passed as variable arguments | 17 | 17 | `20180921-1.c`, `920625-1.c`, `920908-1.c`, `931004-10.c`, `931004-12.c`, `931004-14.c`, `931004-2.c`, `931004-4.c`, `931004-6.c`, `931004-8.c`, `pr44575.c`, `pr92904.c`, `stdarg-3.c`, `strct-stdarg-1.c`, `strct-varg-1.c`, `va-arg-22.c`, `va-arg-pack-1.c` |
| complex arithmetic | 12 | 12 | `20010605-2.c`, `20020411-1.c`, `20030910-1.c`, `960512-1.c`, `complex-1.c`, `complex-2.c`, `complex-4.c`, `complex-5.c`, `complex-7.c`, `pr38969.c`, `pr42248.c`, `pr49644.c` |
| this static initializer | 10 | 10 | `20040302-1.c`, `20041214-1.c`, `20050929-1.c`, `20071210-1.c`, `20071220-1.c`, `20071220-2.c`, `920501-4.c`, `pr23324.c`, `pr70460.c`, `pr84478.c` |
| variable length arrays | 6 | 7 | `20040411-1.c`, `20040811-1.c`, `920721-2.c`, `920929-1.c`, `pr22061-1.c`, `pr43220.c`, `vla-dealloc-1.c` |
| local labels | 1 | 2 | `930406-1.c`, `980526-1.c` |
| computed goto | 1 | 1 | `920501-3.c` |
| label addresses | 1 | 1 | `920501-5.c` |
| this builtin | 54 | 1 | `921007-1.c` |
| case ranges | 1 | 0 | None |
| conditionals with an omitted operand | 1 | 0 | None |
| statement expressions | 4 | 0 | None |

<details>
<summary>Exact baseline construct sources</summary>

- case ranges (1): `pr34154.c`.
- complex arithmetic (12): `20010605-2.c`, `20020411-1.c`, `20030910-1.c`, `960512-1.c`, `complex-1.c`, `complex-2.c`, `complex-4.c`, `complex-5.c`, `complex-7.c`, `pr38969.c`, `pr42248.c`, `pr49644.c`.
- computed goto (1): `920501-3.c`.
- conditionals with an omitted operand (1): `pr68249.c`.
- inline assembly (61): `20001009-2.c`, `20030222-1.c`, `20061031-1.c`, `20071211-1.c`, `20071220-1.c`, `20071220-2.c`, `20080122-1.c`, `960312-1.c`, `990130-1.c`, `990413-2.c`, `bitfld-5.c`, `pr103376.c`, `pr108498-1.c`, `pr110252-2.c`, `pr114552.c`, `pr38533.c`, `pr40022.c`, `pr40657.c`, `pr41239.c`, `pr43385.c`, `pr43560.c`, `pr44852.c`, `pr45695.c`, `pr47925.c`, `pr49218.c`, `pr49279.c`, `pr49390.c`, `pr51581-1.c`, `pr51581-2.c`, `pr51877.c`, `pr51933.c`, `pr52286.c`, `pr56205.c`, `pr56866.c`, `pr57344-1.c`, `pr57344-2.c`, `pr57344-3.c`, `pr57344-4.c`, `pr58277-1.c`, `pr58419.c`, `pr63641.c`, `pr65053-1.c`, `pr65053-2.c`, `pr65648.c`, `pr65956.c`, `pr68328.c`, `pr69691.c`, `pr78438.c`, `pr78726.c`, `pr79354.c`, `pr79737-2.c`, `pr80421.c`, `pr81588.c`, `pr82954.c`, `pr84478.c`, `pr84524.c`, `pr85156.c`, `pr85756.c`, `pr88904.c`, `pr93945.c`, `pr94130.c`.
- label addresses (1): `920501-5.c`.
- local labels (1): `980526-1.c`.
- nested functions (29): `20000822-1.c`, `20010209-1.c`, `20010605-1.c`, `20030501-1.c`, `20040520-1.c`, `20061220-1.c`, `20090219-1.c`, `920415-1.c`, `920428-2.c`, `920501-7.c`, `920612-2.c`, `920721-4.c`, `921017-1.c`, `921215-1.c`, `931002-1.c`, `comp-goto-2.c`, `nest-align-1.c`, `nest-stdar-1.c`, `nestfunc-1.c`, `nestfunc-2.c`, `nestfunc-3.c`, `nestfunc-5.c`, `nestfunc-6.c`, `nestfunc-7.c`, `pr103405.c`, `pr22061-3.c`, `pr22061-4.c`, `pr24135.c`, `pr71494.c`.
- statement expressions (4): `20000703-1.c`, `930406-1.c`, `950906-1.c`, `compndlit-1.c`.
- structures and unions passed as variable arguments (17): `20180921-1.c`, `920625-1.c`, `920908-1.c`, `931004-10.c`, `931004-12.c`, `931004-14.c`, `931004-2.c`, `931004-4.c`, `931004-6.c`, `931004-8.c`, `pr44575.c`, `pr92904.c`, `stdarg-3.c`, `strct-stdarg-1.c`, `strct-varg-1.c`, `va-arg-22.c`, `va-arg-pack-1.c`.
- this builtin (54): `20000519-1.c`, `20041113-1.c`, `20041214-1.c`, `20071213-1.c`, `920625-1.c`, `920908-1.c`, `921007-1.c`, `931004-10.c`, `931004-12.c`, `931004-14.c`, `931004-2.c`, `931004-4.c`, `931004-6.c`, `931004-8.c`, `980205.c`, `980716-1.c`, `991216-2.c`, `multi-ix.c`, `pr19449.c`, `pr30778.c`, `pr44575.c`, `pr44942.c`, `pr49390.c`, `pr56205.c`, `pr64979.c`, `pr92904.c`, `stdarg-1.c`, `stdarg-2.c`, `stdarg-3.c`, `stdarg-4.c`, `strct-stdarg-1.c`, `strct-varg-1.c`, `va-arg-1.c`, `va-arg-10.c`, `va-arg-11.c`, `va-arg-12.c`, `va-arg-13.c`, `va-arg-14.c`, `va-arg-15.c`, `va-arg-16.c`, `va-arg-17.c`, `va-arg-18.c`, `va-arg-19.c`, `va-arg-2.c`, `va-arg-20.c`, `va-arg-22.c`, `va-arg-23.c`, `va-arg-26.c`, `va-arg-4.c`, `va-arg-5.c`, `va-arg-6.c`, `va-arg-9.c`, `va-arg-pack-1.c`, `va-arg-trap-1.c`.
- this static initializer (10): `20040302-1.c`, `20041214-1.c`, `20050929-1.c`, `20071210-1.c`, `20071220-1.c`, `20071220-2.c`, `920501-4.c`, `pr23324.c`, `pr70460.c`, `pr84478.c`.
- type not analyzed by semantic analysis (111): `20000917-1.c`, `20001203-2.c`, `20010122-1.c`, `20010518-2.c`, `20020108-1.c`, `20020206-1.c`, `20020227-1.c`, `20020320-1.c`, `20020418-1.c`, `20021113-1.c`, `20030224-2.c`, `20030323-1.c`, `20030811-1.c`, `20040625-1.c`, `20040709-2.c`, `20040709-3.c`, `20050107-1.c`, `20050119-1.c`, `20050119-2.c`, `20051113-1.c`, `20070824-1.c`, `20080502-1.c`, `20100430-1.c`, `20111212-1.c`, `20120105-1.c`, `20170401-1.c`, `20190820-1.c`, `20230509-1.c`, `20241029-1.c`, `960117-1.c`, `960416-1.c`, `990208-1.c`, `alloca-1.c`, `bf-pack-1.c`, `bswap-1.c`, `bswap-3.c`, `built-in-setjmp.c`, `builtin-prefetch-1.c`, `builtin-prefetch-2.c`, `builtin-prefetch-3.c`, `builtin-prefetch-4.c`, `builtin-prefetch-5.c`, `builtin-prefetch-6.c`, `cbrt.c`, `ffs-1.c`, `ffs-2.c`, `frame-address.c`, `memclr.c`, `memcpy-a1.c`, `memcpy-a2.c`, `memcpy-a4.c`, `memcpy-a8.c`, `misalign.c`, `packed-1.c`, `packed-2.c`, `packed-aligned.c`, `pr105984.c`, `pr108064.c`, `pr108789.c`, `pr115033.c`, `pr115277.c`, `pr22141-2.c`, `pr23135.c`, `pr23467.c`, `pr29006.c`, `pr35456.c`, `pr36321.c`, `pr37780.c`, `pr39228.c`, `pr44683.c`, `pr45070.c`, `pr47237.c`, `pr52979-1.c`, `pr52979-2.c`, `pr60003.c`, `pr61725.c`, `pr64006.c`, `pr64242.c`, `pr65369.c`, `pr68381.c`, `pr68532.c`, `pr70602.c`, `pr71083.c`, `pr71554.c`, `pr77718.c`, `pr78622.c`, `pr81281.c`, `pr84169.c`, `pr84521.c`, `pr85095.c`, `pr86528.c`, `pr89434.c`, `pr90025.c`, `pr90311.c`, `pr91450-1.c`, `pr91450-2.c`, `pr91597.c`, `pr91635.c`, `pr93213.c`, `pr93494.c`, `pr97325.c`, `pr97386-1.c`, `pr97888-1.c`, `simd-1.c`, `strct-pack-1.c`, `strct-pack-2.c`, `strct-pack-3.c`, `strct-pack-4.c`, `struct-cpy-1.c`, `struct-ret-2.c`, `zerolen-2.c`.
- variable length arrays (6): `20040411-1.c`, `20040811-1.c`, `920721-2.c`, `920929-1.c`, `pr43220.c`, `vla-dealloc-1.c`.
- vector types (20): `20050316-1.c`, `20050316-2.c`, `20050316-3.c`, `20050604-1.c`, `20050607-1.c`, `20060420-1.c`, `pr105613.c`, `pr109040.c`, `pr110817-1.c`, `pr110817-2.c`, `pr60960.c`, `pr70903.c`, `pr85169.c`, `pr94524-1.c`, `pr94524-2.c`, `pr94591.c`, `simd-2.c`, `simd-4.c`, `simd-5.c`, `simd-6.c`.

</details>

The remaining builtin first-refusal sources are:

- `__builtin_longjmp` (4): `built-in-setjmp.c`, `pr60003.c`, `pr64242.c`, `pr84521.c`.
- `__builtin_setjmp` (4): `built-in-setjmp.c`, `pr60003.c`, `pr64242.c`, `pr84521.c`.
- `__builtin_return_address` (3): `20010122-1.c`, `20030323-1.c`, `20030811-1.c`.
- `__builtin_signbit` (2): `20080502-1.c`, `pr35456.c`.
- `__builtin_apply` (1): `pr47237.c`.
- `__builtin_apply_args` (1): `pr47237.c`.
- `__builtin_copysign` (1): `pr44683.c`.
- `__builtin_frame_address` (1): `frame-address.c`.
- `__builtin_isinf` (1): `pr39228.c`.
- `__builtin_isinff` (1): `pr39228.c`.
- `__builtin_isinfl` (1): `pr39228.c`.
- `__builtin_memcmp` (1): `pr77718.c`.
- `__builtin_memset` (1): `pr77718.c`.
- `__builtin_shuffle` (1): `pr94591.c`.
- `__builtin_snprintf` (1): `pr78622.c`.
- `__builtin_strcmp` (1): `921007-1.c`.
- `__builtin_va_arg_pack` (1): `va-arg-pack-1.c`.

Deferred families include setjmp/longjmp, apply/apply_args, stack/frame address
introspection, vector shuffle, additional floating classification/library opaque
types, and va_arg_pack. Aggregate varargs and the Windows extended-float ABI remain
existing refusals. Inline assembly, nested functions, vector and complex values,
VLAs, computed goto/local labels, and nonconstant static-initializer cases remain
separate work. No feature in the requested scalar builtin list was deferred.

### Triage of newly executable failures

| Source | Final arms I0/I1/A0/A1 | Evidence and boundary |
| --- | --- | --- |
| `20020314-1.c` | trap/trap/pass/pass | Alloca now lowers; the interpreter reaches an old unprototyped `exit` declaration and rejects its signature against the runtime host contract. |
| `pr36321.c` | wrong/wrong/pass/pass | Subtracts addresses of separate alloca objects, including a zero-sized allocation, and assumes a contiguous descending native stack. The interpreter intentionally uses separate objects; C99 §6.5.6p9 permits pointer subtraction only within one array (printed pp. 83–84, PDF pp. 95–96). |
| `pr44942.c` | trap/trap/pass/pass | Newly reached f80/f128 varargs interpretation is an existing unsupported interpreter representation. |
| `stdarg-4.c` | trap/trap/link/link | GNU17 external inline definitions with `always_inline` do not emit the external bodies expected by the test; ignored attribute/inline linkage behavior predates this work. |
| `va-arg-14.c` | trap/trap/pass/pass | Passes an uninitialized Windows va_list cursor by value before overwriting the callee copy. Poison is observed at the call; C99 §6.7.8p10 leaves an uninitialized automatic object indeterminate (printed p. 126, PDF p. 138). |

`stdarg-1.c`, `stdarg-2.c`, `va-arg-5.c`, and `va-arg-6.c` additionally move from
lowering refusal to the existing Windows f80 ABI refusal. `941202-1.c` was already
executable in the interpreter before, so is not counted among the 80; ordinary
alloca lowering changes both native arms from link errors to passes. Its remaining
interpreter failure is the same old-style exit signature boundary. No newly
executable failure was traced to the added scalar builtin implementations, so
unrelated optimizer/ABI behavior was preserved.

### Validation

Focused tests cover target-dependent semantic signatures, wrong operand counts
and types, immediate constraints and extension policy, integer constant queries,
IR verifier failures, parse/print round trips before and after ABI legalization,
poison and termination, LLVM intrinsic emission, and optimized/unoptimized
execution. Eight C programs cover bit variants, query argument effects, overflow
boundaries including mixed signedness/128-bit/bit-field/volatile destinations,
allocation lifetime and ordinary-name shadowing, GNU expressions, and varargs.
Both targets execute in the interpreter and emit LLVM; the five native arms run
on Windows. The Linux native test reports a skip when WSL is unavailable on this
host. Installed GCC 13.2.0 O0/O2 comparisons pass eight reference executions.
Their evidence is `target/builtins-gcc-oracles.json`; the added effect-only
overflow regression also passes GCC O0/O2 (`target/overflow-effects-gcc.json`).
Final release CLI probes are
`target/final-builtin-probes.json`. The bswap64 signature is also checked with
`__builtin_types_compatible_p` against `__UINT64_TYPE__` on both targets
(`target/bswap-signature-probes.json`), following GCC 15.2
[builtins.def](https://github.com/gcc-mirror/gcc/blob/releases/gcc-15.2.0/gcc/builtins.def)
and [c-common.cc](https://github.com/gcc-mirror/gcc/blob/releases/gcc-15.2.0/gcc/c-family/c-common.cc).

The required checks pass on the final source revision. `cargo +nightly fmt`
was run before validation. Final checks were sequential; feature builds began
only after CLI integration tests exited.

| Check | Result |
| --- | --- |
| `cargo test --all-targets` | Pass: 1,726 passed, six existing ignored tests across 20 suites |
| `cargo test --features benchmarking-internals --test allocation_count` | Pass: 26 checks |
| `cargo +nightly fmt --check` | Pass |
| `cargo clippy --all-targets -- -D warnings` | Pass |
| `cargo clippy --release --all-targets -- -D warnings` | Pass |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | Pass |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | Pass: six tests, two existing long sweeps ignored |
| `git diff --check` | Pass |
| `cargo build --bin bcc-rust` | Pass; ordinary CLI restored after feature builds and verified with `--help` |

Logs are `target/builtins-check-*.log`. The final release build and all 32 focused
release probes pass. The source SHA-512 values match across the before/after runs
for all 1,677 files. The Linux native runner is unavailable on this Windows host;
its conditional test reports that limitation rather than claiming Linux native
execution. Interpreter, verifier, round-trip, and LLVM checks cover both targets.

The whole-file plan is `target/commit-plan.md`: five focused Conventional Commits,
with no file split across commits. Shared dispatch/task/opcode files require
family changes to be grouped by compiler seam. No files are staged and no commits
have been created. The shared prebuilt LLVM junctions are preserved.

## 2026-10-13: torture type representations and static initializers

Branch `claude/torture2-sema`, based on PR #41 head `a5670e79`. Both release
surveys read the GCC 15.2.0 corpus from the sibling checkout without modifying
it. They use all 1,677 execute sources, eight jobs and a ten-second arm timeout.
The compiler and generated programs run through `scripts/cargo_runner.py`.
Shared `target/llvm` and `target/llvm-libs` junctions remain unchanged.

Evidence: `target/compiler-corpus/execute-torture2-sema-before/results.json`,
`target/compiler-corpus/execute-torture2-sema-after/results.json`,
`target/torture2-final-inventory.json`, and `target/torture2-check-*.log`.
Source SHA-512 values match for every source. No previously passing source
regresses in any surveyed arm; there are no compiler crashes.

### Before and after

| Arm | Pass | Frontend refusal | Lowering refusal | ABI refusal | Trap | Wrong | Timeout | Link |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| interpret | 1231 → 1277 | 138 → 137 | 225 → 176 | 4 → 4 | 63 → 68 | 16 → 15 | 0 → 0 | 0 → 0 |
| interpret-bcc | 1236 → 1283 | 138 → 137 | 225 → 176 | 4 → 4 | 57 → 61 | 17 → 16 | 0 → 0 | 0 → 0 |
| A0 | 1289 → 1340 | 138 → 137 | 225 → 176 | 4 → 4 | 0 → 0 | 16 → 15 | 1 → 1 | 4 → 4 |
| A1 | 1279 → 1330 | 138 → 137 | 225 → 176 | 4 → 4 | 0 → 0 | 26 → 25 | 1 → 1 | 4 → 4 |

The two named refusal groups shrink from **66 to 10 unanalyzed types** and
**10 to 7 static initializers**. Representation analysis clears 56 original
type refusals; static emission clears three original initializer refusals.
Nine type sources reveal other unsupported constructs (six assembly, two
vector, one complex), and one static source reveals assembly. Thus 49 of the
original 76 sources become native passes. Two additional packed-layout sources,
`20010904-1.c` and `20010904-2.c`, change from wrong results to native passes.

### Exact inventory of the original 66 type sources

Counts below are distinct sources within each row, after removing comments
and following local quoted includes. Rows overlap: six aligned sources also
use packed, and `misalign.c` uses packed, mode and ranges. Representation
attributes account for 50 distinct sources. In nonoverlapping priority order:
29 packed, seven additional aligned, 13 additional scalar mode, one `_Alignas`.
The remaining 16 consist of ten deferred control/stack forms, one union cast,
four floating builtins and one snprintf builtin. Implementation followed this
source-count order. Vector and complex rows identify latent blockers.

| Construct | Original type sources | Native passes among them | Exact sources |
| --- | ---: | ---: | --- |
| packed | 29 | 27 | `20010518-2.c`, `20020227-1.c`, `20030224-2.c`, `20040625-1.c`, `20040709-2.c`, `20040709-3.c`, `20051113-1.c`, `20100430-1.c`, `20111212-1.c`, `20120105-1.c`, `20170401-1.c`, `20190820-1.c`, `960117-1.c`, `bf-pack-1.c`, `misalign.c`, `packed-1.c`, `packed-2.c`, `packed-aligned.c`, `pr29006.c`, `pr45070.c`, `pr52979-1.c`, `pr52979-2.c`, `pr70602.c`, `pr71083.c`, `strct-pack-1.c`, `strct-pack-2.c`, `strct-pack-3.c`, `strct-pack-4.c`, `struct-ret-2.c` |
| aligned | 13 | 13 | `20190820-1.c`, `960117-1.c`, `alloca-1.c`, `packed-aligned.c`, `pr108064.c`, `pr22141-2.c`, `pr23467.c`, `pr65369.c`, `pr77718.c`, `strct-pack-1.c`, `strct-pack-2.c`, `strct-pack-3.c`, `struct-cpy-1.c` |
| scalar mode | 14 | 6 | `20020108-1.c`, `20050107-1.c`, `20050119-1.c`, `20050119-2.c`, `cbrt.c`, `memclr.c`, `memcpy-a1.c`, `memcpy-a2.c`, `memcpy-a4.c`, `memcpy-a8.c`, `misalign.c`, `pr23135.c`, `simd-1.c`, `zerolen-2.c` |
| _Alignas | 1 | 1 | `pr68532.c` |
| return/frame address | 4 | 0 | `20010122-1.c`, `20030323-1.c`, `20030811-1.c`, `frame-address.c` |
| setjmp/longjmp | 4 | 0 | `built-in-setjmp.c`, `pr60003.c`, `pr64242.c`, `pr84521.c` |
| signbit | 2 | 2 | `20080502-1.c`, `pr35456.c` |
| scalar-to-union cast | 1 | 1 | `960416-1.c` |
| label addresses | 1 | 0 | `990208-1.c` |
| isinf | 1 | 1 | `pr39228.c` |
| copysign | 1 | 1 | `pr44683.c` |
| apply/argument replay | 1 | 0 | `pr47237.c` |
| snprintf | 1 | 1 | `pr78622.c` |
| range designators | 6 | 0 | `memclr.c`, `memcpy-a1.c`, `memcpy-a2.c`, `memcpy-a4.c`, `memcpy-a8.c`, `misalign.c` |
| vector forms | 2 | 0 | `pr23135.c`, `simd-1.c` |
| _Complex | 1 | 0 | `20020227-1.c` |

There are **zero independent typeof, types-compatible or offsetof blockers**
in these 66 sources. Their existing implementations now consume retained
representation types; focused tests include qualified/nested member types,
type compatibility, expression alignment and array-member offsetof.

### The original ten static initializer sources

| Form | Sources | Result |
| --- | --- | --- |
| Nested file-scope compound literals and their addresses | `20050929-1.c` | Pass in all four surveyed arms |
| Integer constant cast to pointer | `pr23324.c`, `pr84478.c` | Both emit constants; pr23324 passes, pr84478 reaches assembly refusal |
| Label addresses | `20040302-1.c`, `20041214-1.c`, `20071210-1.c`, `20071220-1.c`, `20071220-2.c`, `920501-4.c` | Deferred code-label relocation/control-flow subsystem |
| Label differences in integer static arrays | `pr70460.c` | Deferred label-difference relocation |

### Implemented contracts and decisions

GNU object/member/type attributes retain packed layouts, scalar machine modes,
explicit alignment and packed enum representations. Completed declarators keep
array/pointer storage alignment separate from element/pointee alignment. Typedef
alignment may weaken the natural alignment; ordinary variables may not. Packed
member accesses and initializer stores use guaranteed alignment while ABI
classification retains natural member types. C99 §6.7.2.1p10 (printed p. 102;
PDF p. 114) permits implementation-defined allocation details; standard alignment
uses C11 §6.7.5p2-6 (printed p. 127; PDF p. 145). GCC contracts are in
[Common Attributes](https://gcc.gnu.org/onlinedocs/gcc/Common-Attributes.html).

[Union casts](https://gcc.gnu.org/onlinedocs/gcc/Cast-to-Union.html) select an
exactly compatible member and create a union rvalue. Both scalar and aggregate
members lower through ordinary temporaries and stores.
[GNU ranges](https://gcc.gnu.org/onlinedocs/gcc/Designated-Inits.html) use inclusive
integer constant endpoints, infer incomplete bounds, check bounds and preserve
evaluation once across every selected subobject. Nested ranges use iterative
Cartesian cursors and arena-held value snapshots, without recursive syntax walks.
All six original range sources still need inline assembly, so their standalone
corpus pass gain is zero.

Floating classification/sign-copy builtins analyze generic and f/l operands,
diagnose arity/types, fold allowed constants and preserve signed zero/NaN signs.
They lower to existing scalar/memory IR; no new interpreter floating subsystem
is introduced. See [Other Builtins](https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html),
C99 §7.12.3.3 (printed p. 217; PDF p. 229), §7.12.3.6 (printed p. 218;
PDF p. 230), §7.12.11.1 (printed p. 236; PDF p. 248), and §6.6p10
(printed p. 96; PDF p. 108) for additional constant expressions.
The binding-aware snprintf signature and interpreter formatter implement
truncation, n==0, terminators and the untruncated return count, per C99
§7.19.6.5p2-3 (printed p. 290; PDF p. 302).

Static compound literals become memoized internal globals, populated by an
iterative queue. Integer pointer constants emit bytes; compound/array-member
addresses, selected GNU/C11 expressions and `&*` forms emit existing relocations.
Both back ends use the same byte/relocation representation. C99 §6.6p7-9
(printed pp. 95-96; PDF pp. 107-108) and §6.7.8p4 (printed p. 125;
PDF p. 137) govern these constant initializers. Extension diagnostics continue
to use the existing policy; GNU mode does not suppress `Deny`.

### Newly reached failures and deferred work

The remaining ten opaque sources require stack introspection (four return/frame
address sources), nonlocal execution (four setjmp/longjmp sources), ABI argument
replay (pr47237), or code-label identity (990208-1). These cannot be implemented
as scalar signatures alone: they need interpreter frame/control support and
native stack/ABI or code-label representations. Label-address/difference static
initializers share the latter subsystem. Inline assembly, nested functions,
vectors and complex arithmetic remain deferred. Aggregate varargs and VLA
lowering are intentionally left to the separate worktree.

Native passes that still encounter interpreter limitations are listed in the
machine-readable inventory. `20040709-2.c`, `20040709-3.c`, `20080502-1.c` and
`pr39228.c` reach the existing f80/f128 interpreter refusal. `20100430-1.c`
reaches a poison/noundef failure before bcc optimization and passes afterward.
Function `aligned` code placement is outside the original type/static refusal
inventory and remains unrepresented in IR: `align-3.c` now reaches its failing
function-alignment query instead of an incompatible redeclaration diagnostic.
This is a newly exposed failure, not a regression of a previously passing source.

### Remaining lowering inventory

Distinct source counts per construct may overlap:

| Construct | Sources | Exact remaining inventory |
| --- | ---: | --- |
| nested functions | 29 | `20000822-1.c`, `20010209-1.c`, `20010605-1.c`, `20030501-1.c`, `20040520-1.c`, `20061220-1.c`, `20090219-1.c`, `920415-1.c`, `920428-2.c`, `920501-7.c`, `920612-2.c`, `920721-4.c`, `921017-1.c`, `921215-1.c`, `931002-1.c`, `comp-goto-2.c`, `nest-align-1.c`, `nest-stdar-1.c`, `nestfunc-1.c`, `nestfunc-2.c`, `nestfunc-3.c`, `nestfunc-5.c`, `nestfunc-6.c`, `nestfunc-7.c`, `pr103405.c`, `pr22061-3.c`, `pr22061-4.c`, `pr24135.c`, `pr71494.c` |
| inline assembly | 68 | `20001009-2.c`, `20020107-1.c`, `20030222-1.c`, `20061031-1.c`, `20071211-1.c`, `20071220-1.c`, `20071220-2.c`, `20080122-1.c`, `960312-1.c`, `990130-1.c`, `990413-2.c`, `bitfld-5.c`, `memclr.c`, `memcpy-a1.c`, `memcpy-a2.c`, `memcpy-a4.c`, `memcpy-a8.c`, `misalign.c`, `pr103376.c`, `pr108498-1.c`, `pr110252-2.c`, `pr114552.c`, `pr38533.c`, `pr40022.c`, `pr40657.c`, `pr41239.c`, `pr43385.c`, `pr43560.c`, `pr44852.c`, `pr45695.c`, `pr47925.c`, `pr49218.c`, `pr49279.c`, `pr49390.c`, `pr51581-1.c`, `pr51581-2.c`, `pr51877.c`, `pr51933.c`, `pr52286.c`, `pr56205.c`, `pr56866.c`, `pr57344-1.c`, `pr57344-2.c`, `pr57344-3.c`, `pr57344-4.c`, `pr58277-1.c`, `pr58419.c`, `pr63641.c`, `pr65053-1.c`, `pr65053-2.c`, `pr65648.c`, `pr65956.c`, `pr68328.c`, `pr69691.c`, `pr78438.c`, `pr78726.c`, `pr79354.c`, `pr79737-2.c`, `pr80421.c`, `pr81588.c`, `pr82954.c`, `pr84478.c`, `pr84524.c`, `pr85156.c`, `pr85756.c`, `pr88904.c`, `pr93945.c`, `pr94130.c` |
| type not analyzed by semantic analysis | 10 | `20010122-1.c`, `20030323-1.c`, `20030811-1.c`, `990208-1.c`, `built-in-setjmp.c`, `frame-address.c`, `pr47237.c`, `pr60003.c`, `pr64242.c`, `pr84521.c` |
| complex arithmetic | 13 | `20010605-2.c`, `20020227-1.c`, `20020411-1.c`, `20030910-1.c`, `960512-1.c`, `complex-1.c`, `complex-2.c`, `complex-4.c`, `complex-5.c`, `complex-7.c`, `pr38969.c`, `pr42248.c`, `pr49644.c` |
| this static initializer | 7 | `20040302-1.c`, `20041214-1.c`, `20071210-1.c`, `20071220-1.c`, `20071220-2.c`, `920501-4.c`, `pr70460.c` |
| variable length arrays | 7 | `20040411-1.c`, `20040811-1.c`, `920721-2.c`, `920929-1.c`, `pr22061-1.c`, `pr43220.c`, `vla-dealloc-1.c` |
| vector types | 22 | `20050316-1.c`, `20050316-2.c`, `20050316-3.c`, `20050604-1.c`, `20050607-1.c`, `20060420-1.c`, `pr105613.c`, `pr109040.c`, `pr110817-1.c`, `pr110817-2.c`, `pr23135.c`, `pr60960.c`, `pr70903.c`, `pr85169.c`, `pr94524-1.c`, `pr94524-2.c`, `pr94591.c`, `simd-1.c`, `simd-2.c`, `simd-4.c`, `simd-5.c`, `simd-6.c` |
| structures and unions passed as variable arguments | 17 | `20180921-1.c`, `920625-1.c`, `920908-1.c`, `931004-10.c`, `931004-12.c`, `931004-14.c`, `931004-2.c`, `931004-4.c`, `931004-6.c`, `931004-8.c`, `pr44575.c`, `pr92904.c`, `stdarg-3.c`, `strct-stdarg-1.c`, `strct-varg-1.c`, `va-arg-22.c`, `va-arg-pack-1.c` |
| computed goto | 1 | `920501-3.c` |
| label addresses | 1 | `920501-5.c` |
| this builtin | 1 | `921007-1.c` |
| local labels | 2 | `930406-1.c`, `980526-1.c` |

### Validation

Focused semantic tests cover retained representations, completion order,
diagnostics/recovery, constants and extension policy. Six execution programs
cover layouts (including packed 64-bit initializer stores), scalar/aggregate union
casts, nested static addresses and absolute/null pointers, nested ranges with
side effects, floating queries/sign-copy and snprintf truncation/zero size.
They execute in every interpreter stage on both targets and in five native arms
against bundled Clang. Native floating probes include long double; interpreter
probes use double equivalents because f80 transport remains unsupported.
Clang needs an explicit C99 once-evaluated oracle for nonconstant GNU ranges and
generic isinf in place of GCC f/l aliases. Independent GCC O0/O2 executions use
all six original programs, including actual ranges and aliases: 12 passes in
`target/torture2-gcc-oracles.json`. LLVM emission checks cover both target triples;
native execution in these focused tests covers the Windows host only.

All required checks pass on the final source revision. Feature builds were
sequential and began after all-target CLI integration tests exited. Nightly
formatting ran before its check, and the ordinary CLI was rebuilt last.

| Check | Result |
| --- | --- |
| `cargo test --all-targets` | Pass: 1,787 passed, six existing ignores across 20 suites |
| `cargo test --features benchmarking-internals --test allocation_count` | Pass: 26 tests |
| `cargo +nightly fmt --check` | Pass, after `cargo +nightly fmt` |
| `cargo clippy --all-targets -- -D warnings` | Pass |
| `cargo clippy --release --all-targets -- -D warnings` | Pass |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | Pass |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | Pass: six tests, two existing ignores |
| `git diff --check` | Pass |
| `cargo build --bin bcc-rust` | Pass: plain CLI restored |

The final survey release executable's SHA-512 matches the recorded digest.
Both surveys retain all 1,677 source hashes; the final after survey completes
in 338.149 seconds. The whole-file plan is `target/commit-plan.md`: seven
commits, with every changed file listed exactly once. Shared compiler dispatch
and type files require semantic families to remain together. Nothing is staged
or committed, and neither shared LLVM junction was modified.
