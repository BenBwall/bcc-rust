Research date: 2026-11-01.

# GCC run tests that abort under `--gcc-mode`

The GCC 15.2 testsuite survey (`build/scratch.gcc-flags/tools/survey.py`, host
GCC 13 as the reference) builds every `dg-do run` test of
`gcc.c-torture/execute`, `gcc.dg`, `gcc.dg/torture` and
`c-c++-common/torture` with `bcc --gcc-mode` three ways: LLVM at `-O0` (A0)
and `-O2` (A2), and the native back end at `-O2` (N2); a test's own
`dg-options` follow and may override the level. On main plus the fast-math
branch, 77 tests aborted (signal 6) in at least one configuration. Each was
triaged against the pinned Clang 23 at `-O0` and `-O2`: an abort only bcc
shows is wrong code to fix; one Clang shares, or one that rests on what GCC's
optimizers prove, is a deviation recorded here.

Handled elsewhere and not repeated below: `builtin-arith-overflow-6`, `-17`
and `-p-17` (an LLVM `-O0` `i128` miscompile, branch `agent/overflow-abd`);
`bitfld-3`, `bitfld-5`, `pr32244-1`, `pr34971` and `20110718-1` (GCC's
bit-field types, PR #223); `sso-6`, `sso-8`, `sso-17`, `20230630-2` and
`20230630-4` (`scalar_storage_order`, refused since PR #222); `sinatan-1`
and `pr95115` (GCC's folding and trapping); and the remaining
`builtin(-dynamic)-object-size` tests, which measure the precision of GCC's
object-size analysis.

## Fixed

Each fix has a regression in `tests/integration/survey_wrong_code.rs` that
runs on the interpreter (unoptimized and optimized), LLVM at `-O0` and `-O2`
and the native back end, except where noted.

| Test | Cause | Fix |
| --- | --- | --- |
| gcc.dg/c90-scope-1 | bcc gave selection and iteration statements their C99 blocks in every mode, so a tag declared in an `if` condition left scope after it under `-std=c90` | Under C90 and C95 (`c89`, `c90`, `iso9899:1990`, `iso9899:199409`, `gnu89`, `gnu90`), sema and the parser's typedef scopes put what a controlling expression, a non-compound substatement or a `for` declaration (which Clang accepts there and GCC rejects) declares in the enclosing block, as GCC and Clang do; a compound substatement keeps its block |
| gcc.c-torture/execute/20031003-1 | `(int)2147483648.0f` was converted at run time (poison in LLVM, a crash at `-O2`); GCC folds it to `INT_MAX` | Under GCC's rules (`float_conversion_rules`) a floating constant converted out of an integer's range folds to the bound GCC gives it (0 for a NaN): explicit casts in sema, and every implicit conversion (assignment, initializers, arguments, `return`, arithmetic operands) in lowering, from the constant's bits (`lowering/saturation.rs`). A bit-field store whose own operand is a floating constant saturates it to the field's width, as GCC's `convert_and_check` does (`b.s = 100.0` stores 15 in an `int s : 5`); an integer, even one converted from a constant earlier or by a cast, keeps its low bits. The conversion is undefined; Clang's identity keeps converting at run time |
| gcc.dg/pr101836_5, builtin-object-size-9, -12, -14, builtin-dynamic-object-size-9, -12, -14 | `__builtin_object_size(p->c, 1)` of a trailing array member through a pointer answered the declared array size; GCC and Clang treat any trailing array as possibly flexible (`-fstrict-flex-arrays=0`) and bound it by the whole object, so bcc could make a fortified copy into a struct-hack member trap | Mode 1 of a trailing array reached through a pointer (`p->c`, `(*p).c`, `p[i].c`, a union's members) is answered as mode 0: unknown through a parameter, the remaining size once the object is known (a local, an allocation), so mode 1 never exceeds mode 0. Under Clang's identity also the trailing array of a trailing structure member and of the last element of each dimension of a trailing array of structures, as Clang does; GCC 13's `-O2` pass bounds those by the declared size, and so does bcc under GCC's identity (`flexible_array_rules`; GCC at `-O0` answers every such query unknown). Mode 3 keeps the declared size, as Clang does; GCC 13 answers 0 |
| gcc.dg/bitint-28, -29, -31 | The `_BitInt` (>128 bits) floating conversions in bcc's runtime negated a rounded magnitude, so `FE_DOWNWARD`/`FE_UPWARD` rounded negative values the wrong way; they raised no "invalid" or "inexact" and did not saturate out of range | `runtime.c` rounds the signed value, raises "invalid" and gives the nearest bound (the maximum for a NaN) out of range, and raises "inexact" for a discarded fraction, as libgcc does (C23 F.4). Compiled back ends only: the interpreter has no floating-point environment |
| gcc.dg/atomic-store-6 | `__atomic_store_16`, a GCC builtin with libatomic's prototype, was an implicit `int` declaration, so `-1` was passed as a 32-bit `int` | Under GCC's identity the size-specific `__atomic_*_N` routines (N = 1, 2, 4, 8, 16) have libatomic's prototypes, with GCC's `volatile void *` operand (`const volatile void *` for a load); the call goes to libatomic, where GCC inlines sizes up to 8. bcc called libatomic for those before too, as implicit declarations, so `-latomic` was already needed. Compiled back ends only |
| gcc.dg/atomic-noinline | `-fno-inline-atomics` was ignored with the `-f[no-]inline*` tuning options, though the test (and any program that interposes the routines) observes it | Refused: bcc expands the atomic builtins inline |

## Documented deviations

| Test | Configs | Clang 23 | Cause |
| --- | --- | --- | --- |
| gcc.c-torture/execute/20021127-1 | all | passes | The program defines its own `llabs` that aborts; GCC and Clang expand the library builtin regardless. Defining a reserved library name with external linkage is undefined (C99 §7.1.3p2); bcc calls the definition |
| gcc.c-torture/execute/pr43629 | A2, N2 | passes | `x &= 0xff` on an uninitialized `x`: bcc reads it as poison, which its interpreter reports as undefined (C11 §6.3.2.1p2, an automatic object that could be `register`); Clang's `undef` keeps the masked range |
| gcc.c-torture/execute/20030125-1 | N2 | passes | Expects `(float)floor((double)f)` narrowed to `floorf`, with a `floor` that aborts; the native back end calls the program's `floor` |
| gcc.c-torture/execute/bcp-1, builtin-constant, gcc.dg/va-arg-pack-len-1 | A2, N2 (va-arg-pack-len-1: all) | passes / no `__builtin_va_arg_pack` | `__builtin_constant_p` folded to 1 after inlining, which only GCC's optimizer proves |
| gcc.c-torture/execute/990208-1 | A2, N2 | fails at `-O2` | Expects a function with an address-taken label to be inlined (quality of implementation) |
| gcc.c-torture/execute/frame-address | A2 | fails at `-O2` | Frame order across `noipa` functions; LLVM has no `noipa` |
| gcc.c-torture/execute/pr64242 | A2 | fails at `-O2` | Stack pointer after `__builtin_longjmp` through a copied buffer at LLVM `-O2` |
| gcc.c-torture/execute/fprintf-chk-1, printf-chk-1, vprintf-chk-1 | A2, N2 | fail at `-O2` | GCC's folding of the `__*printf_chk` builtins into `fputs`/`putchar` calls, which the test counts |
| gcc.c-torture/execute/pr60003, gcc.dg/setjmp-3 | all | rejects the test | A bcc limitation of GCC's `__builtin_setjmp` semantics: a non-volatile local modified between `__builtin_setjmp` and `__builtin_longjmp` keeps its latest value in GCC, through the abnormal edges GCC models; bcc's SSA keeps the value at the `setjmp`. (For C's `setjmp` the value is indeterminate, C99 §7.13.2.1p3.) |
| gcc.dg/alias-14, gnu23-tag-alias-2, -3, -7 | all | alias-14 and tag-alias-7 fail, tag-alias-2 passes, tag-alias-3 needs LTO | Type-based alias analysis: GCC assumes incompatible types (and C23 tag types) do not alias; bcc does no type-based alias analysis |
| gcc.dg/signbit-6 | all | fails | Undefined `-INT_MIN` compared between an `optimize("O0")` function and an optimized one |
| gcc.dg/pr61441 | all | fails | Signaling NaN through `-fsignaling-nans` folding at `-O1` |
| gcc.dg/pr88598-3 | all | fails | `x * 0.0` folded to `+0.0` under `-fno-signed-zeros`, compared with `memcmp` |
| gcc.dg/torture/pr52451, pr82692 | all; A2 | fail | Exception flags of quiet and signaling comparisons without `FENV_ACCESS` (C99 §7.6.1p2) |
| gcc.dg/pr118224 | A0, A2 | fails | LLVM removes an unused `malloc`/`calloc` and folds its result non-null, even for a size that cannot be allocated |
| gcc.dg/torture/bfloat16-builtin-issignaling-1 | A0 | no `__builtin_nansf16b` | LLVM 23 at `-O0` passes a `bfloat` argument through `float` and back (`__truncsfbf2`), which quiets a signaling NaN before `__builtin_issignaling` reads it |
| gcc.dg/sibcall-1, -2, -6 | N2 | sibcall-2 fails | Expect sibling calls; the native back end makes none |

## Survey after the fixes

The 77 tests re-surveyed with the branch's debug build
(`build/scratch.torture-aborts/after1.jsonl`): c90-scope-1, 20031003-1,
atomic-store-6, bitint-28, -29, -31, pr101836_5 and the four object-size tests
above pass in all three configurations; atomic-noinline is refused. A survey of
310 related tests (object size, atomics, `_BitInt`, flexible arrays, C90 and
C94 modes; `regress1.jsonl`) gained 30 passing configurations and lost none
but `pr54919` and `var-expand2`, which use `-ffast-math`: the baseline was
surveyed with the fast-math branch, which main does not have yet.
