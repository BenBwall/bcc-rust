Research date: 2026-10-26.

What does bcc still lack for an x86-64 Linux or Windows user to treat it as
an ordinary hosted C99/C11/C17 compiler? This audit answers from fresh
surveys on Linux against `main` at `32cd6cc1`, with debug builds of the
command line from a private Cargo target directory, and ranks the gaps by
impact and effort. The fixes made with it are listed at the end.

## Surveys

All runs pass `--target=x86_64-unknown-linux-gnu --sysroot=/ -idirafter
/usr/include/x86_64-linux-gnu`: without the last two flags `<stdio.h>` is
not found (gap 6). GCC sources come from the `releases/gcc-15.2.0` tag of
the GitHub mirror, not from the checksummed release tarball that
`scripts/run_gcc_torture.py` downloads; `gcc.gnu.org` was unreachable from
the sandbox. The `gcc.dg` sources that four execute tests include are
checked out beside them.

| Corpus | Method | Result on `main` |
| --- | --- | --- |
| GCC `gcc.c-torture/execute`, 1,677 files | `scripts/run_gcc_torture_execute.py` with `TARGET` set to Linux, arms interpret, A0, A1, B0 | A0 1,491 pass, B0 1,489, interpret 1,403; 41 front-end errors, 134 lowering refusals, 11 shared wrong results |
| GCC `gcc.c-torture/compile` and `gcc.dg`, 9,084 files | `--emit=llvm`, looking for panics, internal errors and timeouts | 41 internal errors, 1 invalid-IR panic, 1 allocation abort, 2 timeouts |
| c-testsuite `single-exec`, 220 files | LLVM, native and interpreter, output compared | 210 pass on every arm |
| chibicc `test/`, 41 files | `-c`, linked with chibicc's `common` built by GCC | most failures are chibicc-only prototypes; 4 constructs GCC and Clang accept |
| 14 hand-written probes | C11 keywords, designated initializers, bit-fields, flexible arrays, varargs (structures, `long double`, `va_copy`), structure ABI, VLAs, complex, atomics, `asm`, `setjmp`, wide and Unicode strings, function pointers, `<tgmath.h>` | all pass on LLVM at -O0 and -O2 and on the native back end, except the gaps below |

The 11 shared execute wrong results are the cases that
[gcc-torture-execute-results.md](gcc-torture-execute-results.md) already
triages: undefined behaviour, `-fwrapv`/`-fgnu89-inline` options the survey
does not apply, typedef alignment, `scalar_storage_order` and GCC's extended
bit-field arithmetic. The 88 interpreter-only traps are undefined behaviour
the interpreter detects, x87 `long double` (22) and host functions it lacks.

## Ranked gaps

| Rank | Gap | Evidence | Effort | Status |
| ---: | --- | --- | --- | --- |
| 1 | GNU extended `asm` statements in function bodies are not lowered on any arm (file-scope `asm` is) | 72 execute sources; any `rdtsc`, `cpuid`, syscall or barrier idiom; chibicc `asm.c`; probe | large: operand typing and constraint checks in semantic analysis, an IR instruction, LLVM inline-asm emission, native register-constraint allocation, an interpreter refusal | open |
| 2 | Internal errors on valid code ("lowering is missing a semantic fact"), a panic and an abort | 41 + 1 + 1 in the compile surveys | small to medium each | 30 internal errors, the panic and the abort fixed; 11 open (below) |
| 3 | Compile time quadratic in function size | 20,000 `if (i) {}` statements took 5.8 s of the verifier (debug); 100,000 case labels 68 s | small | fixed |
| 4 | The native back end refuses C11 atomics | `stdatomic.h` probe | medium | open |
| 5 | The native back end refuses `setjmp`/`longjmp` | probe | medium | open |
| 6 | The Linux driver searches no C library headers by default | every hosted program needs `--sysroot=/ -idirafter /usr/include/<triple>` | small, driver | open, belongs to the command-line work |
| 7 | GNU and Clang front-end forms rejected | c-testsuite 00206, 00213, 00216; GCC `pushpop_macro.c`; chibicc `arith.c`, `string.c` | small each | `push_macro`/`pop_macro`, `\e`, one-void conditionals and constant compound-literal statics fixed; the rest open (below) |
| 8 | The interpreter lacks x87 `long double` values and several host functions (`qsort`, `strcat`, `wcslen`, `conj`, `fopen`, `stdout`, `vprintf`, `signal`) | 22 + 14 execute traps; probes | medium | open |
| 9 | Accepts-invalid: the value of an incomplete structure, union or enumeration object | gcc.dg `enum-incomplete-4.c`, reduced probes | small | fixed |
| 10 | A function with more than about 10,000 parameters needs memory quadratic in their number in LLVM-C attribute lists (`limits-fndefn.c` with 100,000) | far beyond C99's 127-parameter floor | small, needs a C++ shim for bulk attributes | open |

GNU-only extensions that Clang also rejects (nested functions, 29 execute
sources; variable length arrays in structures, 8; integer `_Complex`, 7)
rank below these: real code that must also build with Clang avoids them.
GCC vector types (22 execute sources) are accepted by semantic analysis but
not lowered.

### Remaining internal errors

After the fixes, these 11 compile-survey sources still report a missing fact:

- `VLA bound was not evaluated`: `gcc.dg/vla-16.c` (a variably modified
  `typeof` operand of a compound literal) and `pr108423-2.c`, `-3.c`,
  `-4.c`, `-6.c` (a parameter whose type is a function returning a pointer
  to a variable length array).
- `an object type has no layout`: `gcc.c-torture/compile/pr16566-1.c`,
  `-2.c`, `-3.c`, a flexible array member of a structure rvalue such as
  `((void)1, x->c).tab[0]`.
- `an operand has no value`: `gcc.dg/Wstringop-overflow-69.c` and
  `pr118684.c`, vector values.
- `gcc.dg/atomic-pr81231.c`: `__atomic_load_n` on a pointer to an incomplete
  enumeration, which GCC rejects; semantic analysis accepts it.

`gcc.c-torture/compile/limits-fndefn.c` still times out (gap 10).

### Remaining front-end gaps that GCC and Clang accept

- A scalar compound literal as a constant, `int x = (int){4};`, and member
  access or arithmetic on constant compound literals (Clang only).
- A cast of a structure to its own type, `(struct s)value` (GCC).
- `_Alignof expr` without parentheses (GNU).
- Unknown escape sequences such as `\j`: GCC and Clang warn and use the
  character; bcc reports an error, which C99 §6.4.4.4 (footnote 65) also
  permits.

## Fixed with this audit

- Valid code that reported missing semantic facts: incomplete external
  objects whose address is taken, `*` on a `void *` as a void expression,
  GNU function pointer arithmetic, a `main` whose result is not `int`,
  `T[]` initialized by `{}` (GNU zero-length arrays), and objects of 4 GiB
  or more, which are now refused as unsupported. Values of incomplete
  structure types are diagnosed by semantic analysis.
- Dominance queries in constant time, and constant-time terminator edge
  lookup in lowering and SCCP.
- `#pragma push_macro`/`pop_macro`, the GNU `\e` escape, conditionals with
  one void alternative (including `x ?: f()`), and constant compound literals
  of structure or union type in static initializers.

After these fixes the execute survey's front-end errors drop from 41 to 39
and A0 passes rise from 1,491 to 1,493 (`pushpop_macro.c` now passes on
every arm), no pass regresses, and the
compile surveys report 11 internal errors and one timeout instead of 41
internal errors, a panic, an abort and two timeouts; the case-label limit
test lowers in 3 s instead of 68 s.
