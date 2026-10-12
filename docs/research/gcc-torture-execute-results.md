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
signature checks and `noundef` checks were retained.

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

The table below covers every remaining source with a wrong result, link
error, interpreter trap, or arm disagreement. P/T/W/L mean
pass/trap/wrong-result/link-error. There are 168 sources; the two fixed
sources are described above. Categories distinguish valid C99 lowering gaps,
interpreter capability gaps, GNU extensions and test/runtime assumptions.
The full before/after per-source classifications are also in
`target/compiler-corpus/execute-correctness-after/triage.json`.

| Source | I0/I1/A0/A1 | Classification | Cause |
| --- | --- | --- | --- |
| `20000227-1.c` | W/W/W/W | GCC/source assumption | Expects a Latin-1 source byte to become one execution byte; bcc uses UTF-8. |
| `20000412-3.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `20000717-3.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `20001101.c` | T/T/P/P | Test: undefined C operation | Old-style pointer parameter receives integer zero without prototype conversion (C99 6.5.2.2p6). |
| `20001121-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `20010904-1.c` | W/W/W/W | GNU feature | Typedef-level aligned attribute is not modeled; requires aligned type layout through arrays and containing records. |
| `20010904-2.c` | W/W/W/W | GNU feature | Typedef-level aligned attribute is not modeled; requires aligned type layout through arrays and containing records. |
| `20011121-1.c` | T/T/P/P | Test: pointer assumption | GNU null-based member-offset idiom; runtime inbounds arithmetic on null traps. Use __builtin_offsetof for supported C99 code. |
| `20020107-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `20020413-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `20020508-2.c` | T/T/P/W | Test: undefined C operation | Signed left shift of a negative value or an unrepresentable positive result (C99 6.5.7p4). |
| `20020508-3.c` | T/T/P/W | Test: undefined C operation | Signed left shift of a negative value or an unrepresentable positive result (C99 6.5.7p4). |
| `20020819-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `20021127-1.c` | W/W/W/W | GCC/library assumption | Expects builtin llabs folding to override its own aborting llabs definition; bcc calls the provided body. |
| `20030316-1.c` | T/T/P/P | Test: undefined C operation | Addition overflows 32-bit Windows long; would not overflow an LP64 long (C99 6.5p5). |
| `20030404-1.c` | T/P/P/P | Test: indeterminate input | Reads an uninitialized automatic value; optimization can eliminate the poison use (C99 6.7.8p10). |
| `20030914-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `20031003-1.c` | T/T/W/W | Test: undefined C operation | Converts 2147483648.0 to 32-bit int, outside its range (C99 6.3.1.4p1). |
| `20031211-2.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `20040208-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `20040409-1w.c` | T/T/P/W | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `20040409-2w.c` | T/T/P/W | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `20040409-3w.c` | T/T/P/W | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `20040709-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `20051012-1.c` | T/T/P/P | GCC/untyped call assumption | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `20090527-1.c` | T/T/P/P | Test: indeterminate input | Reads an uninitialized automatic value; optimization can eliminate the poison use (C99 6.7.8p10). |
| `20230630-2.c` | W/W/W/W | GNU feature | scalar_storage_order attribute needs reversed scalar/bit-field representation and access lowering. |
| `20230630-4.c` | W/W/W/W | GNU feature | scalar_storage_order attribute needs reversed scalar/bit-field representation and access lowering. |
| `920409-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `920411-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `920428-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `920501-1.c` | T/T/P/P | GCC/untyped call assumption | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `920520-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `920603-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `920612-1.c` | T/T/P/P | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `920711-1.c` | T/T/P/P | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `920721-3.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `920728-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `920730-1.c` | T/T/P/P | Test: undefined C operation | INT_MAX + 2 overflows before the cast to unsigned (C99 6.5p5). |
| `920731-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `920812-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `920908-2.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `920909-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `920922-1.c` | T/T/P/P | Test: undefined C operation | Forms a pointer beyond the object and its one-past position (C99 6.5.6p8). |
| `921013-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `921110-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `921123-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `921124-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `921202-1.c` | T/T/P/P | GCC/untyped call assumption | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `921207-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `921208-2.c` | T/T/P/P | GCC/untyped call assumption | Arguments do not match the old-style definition; some explicitly require untyped_assembly (C99 6.5.2.2p6). |
| `921218-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `921218-2.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930106-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930111-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930208-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930429-2.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930513-2.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930518-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930526-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `930527-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930529-1.c` | T/T/P/P | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `930603-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930603-3.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930621-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930622-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930622-2.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `930630-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930702-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930713-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930718-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930719-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930818-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930916-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930921-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930929-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `930930-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `931009-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `931012-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `931017-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `931018-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `931031-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `931102-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `931102-2.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `931208-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `931228-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `940115-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `940122-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `941101-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `941110-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `941202-1.c` | T/T/L/L | GNU runtime / missing symbol | Requires compiler stack-allocation lowering; MinGW has no external alloca symbol. Dynamic allocation and lifetime support are not implemented. |
| `950322-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `950426-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning strlen declaration conflicts with host i64 size_t result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `950512-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `950605-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `950621-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `950704-1.c` | T/T/P/W | Test: undefined C operation | Requires dg-options -fwrapv, which this survey records without applying; signed overflow/negation is undefined under the chosen mode. |
| `960215-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `960218-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `960402-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `960405-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `960513-1.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `960830-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `961112-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `961223-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `970217-1.c` | W/W/W/W | Lowering: large feature | Valid C99 parameter VLA bound side effects are not evaluated on function entry (6.9.1p10); needs retained runtime bounds and entry evaluation. |
| `980526-2.c` | T/T/P/P | Test: undefined C operation | Signed major << 22 produces an unrepresentable int (C99 6.5.7p4). |
| `980608-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `990106-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
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
| `cmpdi-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `cmpsf-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `cmpsi-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `cmpsi-2.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `conversion.c` | T/T/P/P | Interpreter: large feature | Needs full f80/f128 values, arithmetic and conversions; native arms pass. |
| `dbra-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `divmod-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `eeprof-1.c` | W/W/W/W | GCC/runtime assumption | Requires unapplied -finstrument-functions and the associated entry/exit instrumentation. |
| `inst-check.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `int-compare.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `loop-2.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `loop-2c.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `loop-2d.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `loop-2e.c` | T/T/P/P | Test: pointer assumption | Arithmetic/member addressing on a forged integer pointer with no backing object; outside the interpreter object model (C99 6.5.6p8). |
| `loop-3.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `loop-3b.c` | T/T/P/P | Test: signed overflow | i * 4 overflows signed int before a scalar noundef call argument; 6.5p5. I1 later reaches its implicit-int exit declaration. |
| `loop-3c.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `mod-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
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
| `pr55875.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
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
| `tstdi-1.c` | T/T/P/P | GCC/legacy call assumption | Implicit int-returning exit declaration conflicts with host void result (6.5.2.2p9); exact failing call retained in signature evidence. |
| `unroll-1.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `va-arg-7.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |
| `va-arg-8.c` | T/T/L/L | GCC/options assumption | Requires dg-options -fgnu89-inline; under GNU17 this inline definition does not provide the external body. |


Evidence is retained under `target/`: both survey manifests, summaries,
source hashes and per-arm logs; `compiler-corpus/execute-correctness-reference/results.json`
contains exact reference compiler commands and exit statuses;
`torture-signature-evidence.json` names every failing signature instruction
and declaration, with emitted IR beside its baseline log. The scratch
reference/triage drivers never write the corpus. These files are local
evidence artifacts, not new source inputs or waived tests.

All eight requested gates passed, with source changes complete before the
final checks:

| Check | Result |
| --- | --- |
| `cargo test --all-targets` | Passed: 1,720 tests across 20 targets; six existing ignored tests. |
| `cargo test --features benchmarking-internals --test allocation_count` | Passed: 25 tests. |
| `cargo +nightly fmt --check` | Passed, after running `cargo +nightly fmt`. |
| `cargo clippy --all-targets -- -D warnings` | Passed. |
| `cargo clippy --release --all-targets -- -D warnings` | Passed. |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | Passed. |
| `git diff --check` | Passed, including the final report. |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | Passed: six checks, two existing ignored long sweeps. |

The full test run finished before the feature allocation build, and all
subsequent Cargo checks ran sequentially. Logs and exact command exit statuses
are `target/correctness-checks-all.json`, `target/correctness-checks-rest.json`
and `target/correctness-check-*.log`. The reduced failures and intended golden
instruction-order mismatches were observed during development, then resolved;
no behavioral assertions, survey classifications or ignored-test settings were
weakened. The whole corpus remains an exploratory run with recorded unsupported
features, not a passing DejaGnu profile or completed Stage 6 acceptance criterion.

Finally, `cargo build --bin bcc-rust` passed and restored the ordinary debug
CLI after feature checks. Its `--print-default-passes` command exits normally.

The whole-file commit plan is `target/commit-plan.md`. No files were staged
or committed, and no Git metadata writes were performed.
