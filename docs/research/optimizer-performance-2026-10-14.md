Research date: 2026-10-14.

# Closing the optimizer gap: scalar replacement, combining, threading and join-aware GVN

This note records one round of work on bcc's own optimizer, measured against LLVM's with the same LLVM code generator. It
follows the [gap analysis](optimizer-gap-analysis.md) of 2026-10-11, whose four transformations (inlining, select formation,
known-bits division rewrites, load CSE) had brought the original ten-program corpus to parity. The arms, the harness and the
statistics are those of [middle-end.md](../../middle-end.md#comparison-arms) and the
[methodology note](optimizer-comparison-methodology.md): A1 is the bcc optimizer followed by LLVM's code generator alone,
A2 is LLVM's `default<O2>` on bcc's unoptimized IR, N is unoptimized IR through the same code generator. A ratio below 1
means the numerator arm is faster.

## Summary

| | before (`d3641419`) | after (`48011e2f`) |
| --- | --- | --- |
| A1/A2, original 10 programs | 1.003 | 0.996 |
| A1/A2, 7 new programs | 1.521 | 1.139 |
| A1/A2, all 17 | 1.191 | 1.053 [0.995, 1.098] |
| A1/A2, all 17 but `stencil` (vectorization) | 1.143 | 0.996 |
| A1/N, all 17 | 0.864 | 0.754 [0.723, 0.790] |
| paired A1(after)/A1(before), all 17, 21 reps | 1 | 0.877 (minima 0.853) |
| GCC torture A1 arm, pass / wrong result | 1354 / 24 | 1354 / 24, no per-source change |

1. The original corpus was already at parity (the brief's "0.966" was an earlier noisy run; the harness's sign convention is
   numerator over A2, so below 1 favours bcc). It could no longer show what the bcc pipeline lacks, so seven self-checking
   programs were added (`particles`, `mandel`, `fannkuch`, `strings`, `stencil`, `bits`, `switchvm`). On them A1 was 1.52
   times slower than A2.
2. **Scalar replacement of aggregates** (`sroa`) was the missing transformation with by far the largest effect: `particles`
   went from 3.03 to 1.01 times A2 and `mandel` from 2.15 to 0.98. Structures passed and returned by value, member copies
   and local structure temporaries all stayed in stack slots before.
3. **Peephole combining** (`combine`) and **branch threading** in `simplify-cfg` turn lowered `&&`/`||` chains and range
   checks into one comparison and one branch: `strings` went from 1.18 to 0.95.
4. **GVN across joins** reuses loads where no incoming path may clobber them, including loop headers whose bodies do not
   write the location.
5. What remains is dominated by **vectorization** (`stencil`, 2.56: the scalar LLVM O2 code is no faster than A1). Without
   `stencil`, A1 is at parity with A2 on all sixteen other programs within noise.
6. The optimizer costs about 0.8 ms per program on this corpus (14 ms for all seventeen, release build), against roughly
   380 ms of whole-process compile time; the change in A1 compile time is below measurement resolution.

## 1. Method

Machine: AMD Ryzen 9 5900X, Windows 11 Pro, 24 logical CPUs, no CPU isolation; other agents were building and testing in
parallel the whole time. The statistic is whole-process wall time. Two kinds of runs were made:

- **The harness**, `python scripts/run.py compare_optimizers --arms A0,N,A1,A2,A3,R --reps 11 --bcc <binary> --output
  <dir>`, for the official per-arm numbers. Runs are interleaved per benchmark with the order rotated each repetition.
- **A paired run** of two compiler binaries, needed because identical binaries varied by up to 20% between harness runs made
  at different times. A scratch script built every benchmark with the base binary's A1 and the new binary's A1, A2 and N,
  checked that all four print the same checksum and exit 0, and timed the four executables interleaved, 21 repetitions each.
  Comparing the base and new A1 in the same run cancels most of the machine's drift. When a number moved between two
  harness runs, the optimized IR of both binaries was diffed first: a benchmark whose IR did not change was treated as noise
  regardless of its timing (this happened for `sieve`, `bytecode`, `linkedlist`, `mandel` and `hashtable`, by up to 20%).

The gap was attributed by diffing `--emit=ir --opt=bcc` with LLVM's O2 output of the unoptimized `.ll`
(`clang -O2 -S -emit-llvm -x ir`), and by leaving single LLVM passes out of A2 (`-mllvm -opt-disable=<pass>`). Per-pass
compile time comes from the new `--time-passes` option, which prints the optimizer's existing per-pass report.

Base binary: `d3641419` (release). Final binary: `48011e2f` (release). Raw data: `target/optimizer-bench/{base0,base0-new,
sroa,combine,final}/` and `target/optperf/paired-*` in the worktree.

## 2. Before and after

Harness medians, ratios to A2 (11 repetitions; `before` is two runs of the base binary, `base0` for the original ten and
`base0-new` for the seven new programs).

| Benchmark | A1/A2 before | A1/A2 after | A1/N after | paired A1 after/before |
| --- | --- | --- | --- | --- |
| bits | 1.025 | 1.053 | 0.812 | 1.006 |
| bytecode | 0.944 | 0.847 | 0.781 | 0.992 |
| collatz | 1.006 | 0.997 | 0.691 | 0.995 |
| crc32 | 0.995 | 1.011 | 1.019 | 0.961 |
| fannkuch | 1.003 | 0.961 | 0.993 | 1.045 |
| hashtable | 1.176 | 1.041 | 0.833 | 1.027 |
| levenshtein | 1.031 | 1.012 | 0.447 | 0.947 |
| linkedlist | 1.104 | 1.017 | 1.025 | 0.959 |
| mandel | 2.149 | 0.978 | 0.365 | 0.452 |
| matmul | 0.954 | 1.015 | 1.010 | 0.991 |
| nqueens | 0.843 | 0.970 | 1.007 | 1.005 |
| particles | 3.027 | 1.008 | 0.235 | 0.318 |
| sieve | 0.993 | 1.061 | 1.026 | 1.004 |
| sorts | 1.024 | 1.004 | 0.864 | 0.995 |
| stencil | 2.278 | 2.558 | 1.035 | 0.985 |
| strings | 1.183 | 0.947 | 0.596 | 0.807 |
| switchvm | 1.044 | 1.032 | 1.018 | 1.021 |
| **geomean** | **1.191** | **1.053** | **0.754** | **0.877** |

Only `mandel`, `particles` and `strings` changed beyond noise; for `bits`, `bytecode`, `hashtable`, `nqueens`, `sieve` and
`linkedlist` the before/after columns differ by run-to-run noise (the IR of most of them is unchanged). In the final run arm
A3 (bcc then LLVM O2) is 1.000 and R (clang -O2 from source) 1.018 times A2.

Correctness: zero regressions in the GCC torture survey (`run_gcc_torture_execute.py`, 1677 sources, compared per source
and arm with a base run of `d3641419`); the only change is `20090527-1.c` in the optimized-interpreter arm going from an
interpreter trap (branch on an uninitialized member, undefined behaviour) to a pass, because the branch is now a select.
The differential sweep (`BCC_DIFFTEST_CASES=5000`) found no miscompile after the fixes below.

## 3. What was added

### 3.1 `sroa`: scalar replacement of aggregates

`src/optimizer/sroa.rs`, `sroa/plan.rs`, `sroa/rewrite.rs`; runs between `inline` and `promote`. A slot whose address only
reaches loads and stores at constant offsets, and `copy`/`fill` of constant ranges, is split into one slot per field, which
`promote` turns into SSA. Copies between slots join them into components (union-find); the fields inside each copied range
spread to the other side until stable, and an overlap or a field straddling a moved range keeps the component whole.

Soundness under the byte-level poison model: a field moved as a value is poison as a whole where one of its bytes is, and
uncovered bytes are not moved. That is invisible while the bytes are observed only by loads, which read whole fields. A
`copy` to other memory observes bytes too, so:

- slots that can reach a copy out get integer fields for their uncovered bytes (padding), so fills and copies carry them;
- a slot on a copy path from other memory to other memory (`p = arr[i]; ...; arr[i] = p;`) is *retained*: it keeps its
  memory and every write, so the copy out stays byte-exact, and its fields become shadows that only its loads read. LLVM's
  SROA moves such bytes as integers and accepts the imprecision; bcc keeps the IR's exact semantics, and the retained form
  still removed every load in `particles`' hot loop.

The first version required whole-slot copies and refused copy-in-and-out components; it gained `mandel` but only a third of
`particles`. Member copies and retained slots brought `particles` to parity.

### 3.2 `combine`: peephole combining

`src/optimizer/combine.rs`, after `fold`. Booleans and their zero extensions compared with 0 or 1, negated comparisons
(also as branch and select conditions), boolean selects of constants, two constant bounds of one value combined by `and`,
`or` or the lowered `&&`/`||` selects (one unsigned comparison of `x - lo`, LLVM's range-check fold), and nested additions
of constants. Each rule is poison-exact: a comparison or extension is poison exactly when its operand is, and a range
check's two comparisons share their operand, so the select's masking never hides poison the other comparison would not
have.

### 3.3 `simplify-cfg`: constant-condition threading and merged conditions

`src/optimizer/simplify_cfg/thread.rs`. An edge passing a constant to an empty block that branches on that parameter goes
straight to the chosen successor (not past a loop header, and not when blocks below use the parameter: the differential
sweep found that case, seed `0xbcc0d307`). A branch whose single-entry arm only tests a second cheap condition for one of
the branch's own destinations becomes one branch on a select (LLVM's `FoldBranchToCommonDest`); the arm's instructions keep
their flags, since a poison they produce where the first test fails reaches only the select's unchosen operand.

### 3.4 GVN across joins

`gvn.rs` and `memory.rs`. A join or loop header used to start a fresh memory generation. Now the writes of every block on a
path from its immediate dominator are logged instead (the loop body, for a header), so a fact survives when the alias check
proves none of them overlaps it; regions above 64 blocks still get a full barrier.

### 3.5 Fixes and tooling

- `known-bits` indexed its per-value table with the values its own rewrites added (seed `0xbcc0d6be`).
- `--time-passes` prints the per-pass report.
- The differential generator now emits record slots with padded fields, field accesses and copies of field runs (between
  records and to and from a global), range checks, and lowered `&&`/`||` joins with their zero-extension tests. Tests check
  that `sroa` and `combine` fire on at least 10 of the first 50 seeds.

### 3.6 Tried and reverted: loop unrolling

Full unrolling of counted loops with a constant trip count (at most 128 instructions) and partial unrolling by 4 or 2
when the factor divides the trip count, on the canonical two-block loop shape, with the exit moved to the last copy only
when no header value is used after the loop. It passed the unit and differential tests but did not pay: in a paired
21-repetition run it was 2.5% faster on `bits`, 3.8% slower on `matmul` and neutral elsewhere (geomean 0.998, minima
1.014). The code generator's own loop strength reduction and block placement already take what unrolling would give these
loops, and LLVM's O2 does not unroll most of them either. The patch is not committed.

## 4. Compile time

`--time-passes` summed over the seventeen programs (release build, mean of five runs):

| Pass | ms, all 17 programs | rewrites |
| --- | --- | --- |
| inline | 0.46 | 74 |
| sroa | 0.58 | 16 |
| promote | 0.47 | 114 |
| fold | 0.49 | 58 |
| combine | 0.33 | 45 |
| simplify-cfg | 5.76 | 444 |
| sccp | 2.02 | 1 |
| known-bits | 0.84 | 3 |
| gvn | 1.96 | 1219 |
| licm | 0.80 | 571 |
| dce | 0.30 | 60 |
| **total** | **14.0** | |

The whole optimizer takes under a millisecond per program; `simplify-cfg` is the most expensive pass, because it repeats its
steps to a fixpoint and the new threading step builds a dominator tree per round. Whole-process `--emit=ir` for the
seventeen programs takes about 6.4 s with or without `--opt=bcc`, for both binaries (minimum of seven), so the new passes do
not show at the process level. The harness's A1 build time (front end, optimizer, LLVM object emission and link) is 0.986
times A2's.

## 5. What remains, ranked

1. **Loop vectorization** (`stencil` 2.56 times A2; the only large gap left). LLVM vectorizes the three-point stencil after
   inlining proves the two buffers distinct; the IR has no vector types. Needs vector values in the IR, a dependence check
   with runtime alias guards, and a cost model; large.
2. **Loop rotation and LICM of loads**. LLVM rotates loops so the body is guaranteed to run, then hoists invariant loads
   (`left[i-1]` in `levenshtein`) and promotes memory to registers. bcc's LICM moves no loads; GVN now reuses a preheader load
   in a clobber-free loop, but cannot hoist one that has no dominating twin.
3. **Instruction combining breadth**. `strings` shows LLVM folding two ASCII range checks into one (`(c & ~32) - 'A' < 26`)
   and turning byte loops into `strlen`; `bits` shows rotate (`fshl`) and `umin`/`umax` recognition. Each is small; together
   they are LLVM's long tail.
4. **Loop unswitching** of loop-invariant branches (`flip_case` in `strings` branches on `upper` every character).
5. **Byte-exact aggregate copies from lowering**. C struct assignment copies member values (C99 §6.5.16.1p2); a `copy` flag
   that says so would let `sroa` split copy-in-and-out slots instead of retaining them. This needs a lowering change, outside
   this round's scope.
6. **Induction-variable widening** (LLVM `indvars`): the `sext` of 32-bit indices in address arithmetic (`switchvm`,
   `sorts`) costs a few percent at most; the code generator's strength reduction recovers most of it.
7. **Not recommended now:** loop unrolling (measured above), switch formation from if-chains (the gap analysis found it
   slower on `bytecode`).
