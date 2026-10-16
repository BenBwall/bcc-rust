Research date: 2026-10-11.

# Why bcc's optimizer trails LLVM's: a gap analysis and a roadmap

This note explains, benchmark by benchmark, why arm A1 (the bcc optimizer followed by LLVM's code generator alone) is slower than
arm A2 (LLVM's `-O2` optimizer on bcc's unoptimized IR), measures how much of the gap each missing transformation accounts for,
and ends with a prioritized list of passes mapped onto the bcc IR design. The arms, the corpus in `benches/c` and the harness are
those of [middle-end.md](../../middle-end.md#comparison-arms) and the [methodology note](optimizer-comparison-methodology.md).
Statements that are my inference rather than a measurement are marked "inference".

## Summary

1. **The bcc optimizer currently adds nothing that LLVM's code generator does not already do.** A new control arm N (bcc's
   *unoptimized* IR through the same `-O2` code generator with every IR pass disabled) runs as fast as A1 on every benchmark:
   geometric mean ratio to A2 of 1.15 for N against 1.16 for A1. The lowering already builds SSA for scalars, and the code
   generator's own passes (SelectionDAG combining, loop strength reduction, CodeGenPrepare, MachineCSE, MachineLICM, block
   placement and tail duplication) do what `fold`, `sccp`, `gvn`, `licm` and `simplify-cfg` do today. No bcc pass makes code
   slower, and no miscompile was found.
2. **Three missing transformations, with load CSE as an enabler, account for practically the whole gap on the four benchmarks
   that have one.** Adding only the
   named LLVM passes to the bcc output (the rest of the pipeline switched off) reaches A2 within noise:
   - `levenshtein` (A1/A2 = 2.30): inlining of a two-line `minimum` plus select formation. Inlining alone closes 59% of the gap,
     select formation alone 45%, both together 98%.
   - `collatz` (1.44): replacing `sdiv n, 2` by `ashr exact n, 1` when a dominating branch proves `n` even (known bits from a
     dominating condition), then select formation. The rewrite alone closes 56 to 65% of the gap; with select formation, all of
     it.
     Select formation alone closes nothing, because the code generator turns a select with a division operand back into a branch.
   - `hashtable` (1.23): inlining alone closes 100%.
   - `sorts` (1.16): select formation of the merge loop, which needs the redundant loads in its arms removed first, closes 100%.
3. **Redundant load elimination** (`bytecode` reloads `i->op` in every arm of an if-chain) makes `bytecode` 23% faster than A1
   when applied by hand, and is the enabler for select formation in `sorts`.
4. Items commonly blamed for such gaps measured at or below the noise floor on this corpus: loop rotation, induction-variable
   widening and strength reduction, LICM of loads, cross-iteration store-to-load forwarding, loop idiom recognition (`memcpy`),
   unrolling, vectorization, interprocedural constant propagation, and the missing metadata and attributes in our LLVM output.
   Clang's own front end (arm R) is within 3% of A2 in the geometric mean, so the facts clang adds (TBAA, `noundef`, alignment 16
   on arrays) are worth little here.
5. With inlining, select formation, known-bits division rewrites and load CSE, the projected A1 geometric mean falls from 1.19 to
   about 1.01 (from per-benchmark minima; section 6).

## 1. Setup

Machine: AMD Ryzen 9 5900X, Windows 11 Pro, no CPU isolation; other agents were running builds and benchmarks at the same time.
The statistic is whole-process wall time; each cell is the median (and minimum) of 7 or 9 repetitions, interleaved across the
variants of one benchmark with the order rotated each repetition. Differences below about 5% are noise on this machine, and
`linkedlist` (malloc-bound) varies by 40% between repetitions, so its medians are not used. Numbers are for trends and attribution
only; the methodology note's section 4.2 still applies to anything published.

Toolchain: the release build of commit 3b8c9a2d (`cargo build --release`), `target/x86_64-pc-windows-gnu/release/bcc-rust.exe`, and the
bundled `target/llvm/bin/clang.exe` (clang 23.1.1). All builds use `--target=x86_64-w64-windows-gnu`.

### 1.1 Commands

The bcc IR in LLVM form, without and with the bcc optimizer:

```sh
bcc-rust benches/c/X.c --target=x86_64-w64-windows-gnu --emit=llvm > X.naive.ll
bcc-rust benches/c/X.c --target=x86_64-w64-windows-gnu --emit=llvm --opt=bcc > X.bcc.ll
clang --target=x86_64-w64-windows-gnu -O2 -S -emit-llvm -x ir X.naive.ll -o X.a2.ll      # what A2 makes of it
clang --target=x86_64-w64-windows-gnu -O2 -S -emit-llvm benches/c/X.c -o X.r.ll          # arm R, for comparison
```

Every timed executable was built from a `.ll` file with exactly the command the bcc driver uses (`driver.rs`), so the arms can be
recombined without re-running bcc:

```sh
clang --target=x86_64-w64-windows-gnu --sysroot=<MinGW root> -fuse-ld=lld <MODE> -x ir X.ll -o X.exe   # here C:\Strawberry\c
#   A0:  X.naive.ll, MODE = -O0
#   N:   X.naive.ll, MODE = -O2 -Xclang -disable-llvm-passes        (new control arm)
#   A1:  X.bcc.ll,   MODE = -O2 -Xclang -disable-llvm-passes
#   A2:  X.naive.ll, MODE = -O2
#   A3:  X.bcc.ll,   MODE = -O2
#   R:   clang ... -O2 benches/c/X.c -o X.exe
```

Two LLVM options of the pinned clang attribute the gap to passes (both are listed by `clang -mllvm --help-hidden`):

- **Leave one out of A2:** `-mllvm -opt-disable=<pass>[,<pass>...]` skips every instance of the named passes in the `-O2` pipeline.
  Example: `clang ... -O2 -mllvm -opt-disable=inline -x ir X.naive.ll -o X.exe`.
- **Add only some passes to A1:** `-mllvm -opt-bisect=<intervals>` runs only the pass instances with the listed indices, for
  example `-opt-bisect=31,32,75,162,211-99999999`. The indices come from
  `clang ... -O2 -S -emit-llvm -x ir X.bcc.ll -o NUL -mllvm -opt-bisect=0 -mllvm -opt-bisect-verbose`, which prints one
  `BISECT: NOT running pass (i) <name> on <unit>` line per instance. Skipping passes changes how many instances run (per loop, per
  call-graph SCC), so the list is recomputed from the run it configures until it stops changing, and the final verbose listing is
  checked to run exactly the chosen names. Three traps:
  1. The CGSCC wrapper `inliner-ml-advisor-release` must be enabled, or the whole inliner pipeline, `inline` included, is skipped.
  2. `loop-simplify` and `lcssa` must stay enabled: loop passes assume their forms, and without them `loop-rotate` produced an
     executable that never terminated.
  3. The bisect counter also numbers the code generator's passes, which follow the IR pipeline. A skipped `x86-isel` falls back
     to `-O0` instruction selection (`idivq` for `sdiv x, 2`, which made every "add one pass" build twice as slow as A1), so the
     list must end with an open interval `<last IR index + 1>-99999999`.

  Even so, some subsets that combine `licm` with the inliner hang clang (more than 60 s of CPU on a 50-line module); those cells
  were dropped rather than investigated, since partial pipelines are outside LLVM's contract. This is the matched-pipeline arm M
  of the methodology note, built from LLVM's own passes.

The scripts that did this bookkeeping were throwaway; the commands above are all they ran. Pass names in this note are the
pipeline names that `-opt-bisect-verbose` prints. The pipeline itself is `PassBuilder::buildPerModuleDefaultPipeline` in
`llvm/lib/Passes/PassBuilderPipelines.cpp`.

## 2. The arms, and what the bcc optimizer contributes today

Median wall seconds of the final run, 7 repetitions per cell:

| Benchmark | A0 | N | A1 | A2 | A3 | R | A1/A2 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| bytecode | 0.773 | 0.342 | 0.348 | 0.321 | 0.307 | 0.384 | 1.08 |
| collatz | 1.143 | 0.530 | 0.528 | 0.367 | 0.366 | 0.359 | 1.44 |
| crc32 | 0.440 | 0.385 | 0.378 | 0.384 | 0.385 | 0.384 | 0.98 |
| hashtable | 0.433 | 0.305 | 0.307 | 0.251 | 0.243 | 0.236 | 1.23 |
| levenshtein | 0.831 | 0.544 | 0.541 | 0.236 | 0.234 | 0.207 | 2.30 |
| linkedlist (min) | 0.479 | 0.477 | 0.521 | 0.473 | 0.474 | 0.462 | 1.10 |
| matmul | 0.577 | 0.328 | 0.331 | 0.332 | 0.342 | 0.340 | 1.00 |
| nqueens | 0.305 | 0.220 | 0.220 | 0.221 | 0.224 | 0.209 | 1.00 |
| sieve | 0.739 | 0.413 | 0.438 | 0.418 | 0.398 | 0.399 | 1.05 |
| sorts | 0.425 | 0.263 | 0.266 | 0.229 | 0.234 | 0.237 | 1.16 |

Geometric means of the ratios to A2 (medians; minima in parentheses): A0 1.77 (1.85), N 1.15 (1.17), A1 1.16 (1.19),
A3 0.98 (1.00), R 0.97 (1.00). These agree with the first harness run (A0 2.17, A1 1.22, A3 1.05, R 1.02) in direction and
ranking.

N against A1 is the measurement the harness lacks. On every benchmark the two are within noise. The optimized IR is 10 to 20%
smaller than the naive IR (for example 197 against 243 instructions in `sorts`), but what disappears is redundant address
arithmetic, repeated `sub`/`sext` of a loop counter and constant comparisons, which instruction selection and the machine passes
remove anyway. The code generator's pipeline at `-O2` with IR passes disabled
(`clang ... -O2 -Xclang -disable-llvm-passes -c -x ir X.ll -mllvm -debug-pass=Structure`) still contains `loop-reduce`,
`consthoist`, `codegenprepare`, `early-tailduplication`, `early-ifcvt`, `x86-cmov-conversion`, `early-machinelicm`, `machine-cse`,
`machine-sink`, `machinelicm`, `tailduplication` and `block-placement`. Not one load (there are up to 31 per benchmark) is removed by
the bcc pipeline, because `gvn` and `licm` never touch memory and `promote` only sees scalar stack slots, of which the corpus has
one (`hashtable`'s `&value`).

Recommendation for the harness (not implemented here): add N as a standard arm, so that A1 is reported against N as well as A2.
A1/N measures the bcc optimizer; N/A2 measures the work left to it.

## 3. Benchmark by benchmark

### 3.1 levenshtein (A1/A2 = 2.30)

The hot loop calls `static int minimum(int a, int b) { return a < b ? a : b; }` twice per cell. In A1, `minimum` stays a call
whose body is a branch diamond, and the code generator keeps it as a compare-and-branch that mispredicts on random strings. A2
inlines both calls and turns each diamond into `select`, which instcombine canonicalizes to `@llvm.smin.i32` and the code
generator emits as `cmov`. A2 also forwards `current[j-1]` from the previous iteration's store (`loop-load-elim`, the
`%store_forwarded` phi), hoists `left[i-1]` out of the inner loop (`licm`), widens the counters to 64 bits (`indvars`), turns the
row copy into `memcpy` (`loop-idiom`) and vectorizes the initialization loop.

Leaving one pass out of A2 (median seconds; A2 0.301, N 0.635): no `inline` 0.396, no `simplifycfg` 0.375; every other pass of the
list (`instcombine licm gvn loop-rotate indvars loop-idiom loop-load-elim loop-vectorize loop-unroll loop-unroll-full
jump-threading correlated-propagation early-cse sccp ipsccp globalopt function-attrs simple-loop-unswitch dse memcpyopt
aggressive-instcombine reassociate div-rem-pairs constraint-elimination slp-vectorizer tailcallelim loop-instsimplify
speculative-execution`) between 0.238 and 0.310, that is, within noise of A2. Leaving out `loop-load-elim` was even 20% faster in
that run.

Adding passes to the bcc output (median seconds, 7 repetitions):

| Variant | Seconds | Share of the A1-A2 gap closed |
| --- | --- | --- |
| A1 | 0.560 | 0% |
| + `inline` | 0.373 | 59% |
| + `simplifycfg` | 0.416 | 45% |
| + `inline`, `simplifycfg` | 0.250 | 98% |
| + `inline`, `simplifycfg`, `instcombine` | 0.251 | 98% |
| + those and `gvn` | 0.250 | 98% |
| + those and `loop-rotate licm indvars gvn loop-load-elim loop-idiom` | 0.246 | 99% |
| A2 | 0.244 | 100% |

`simplifycfg` alone helps because it turns `minimum`'s own diamond into a `select`, which removes the misprediction even though
the call stays. With inlining as well, the two `select`s sit in the loop body (`%spec.select.i.i`, `%spec.select.i2.i`) and the
loop runs as fast as A2. Everything else LLVM does to this loop is worth 1 to 2%.

Arm R is 12% faster than A2 here. Clang's IR keeps `previous[j-1]` in a register across iterations (one load of `previous` per
cell instead of two) and aligns the arrays to 16 bytes; this is the only benchmark where R and A2 differ by more than noise.

### 3.2 collatz (1.44)

`chain_length` loops on `if (n & 1) n = 3 * n + 1; else n = n / 2;` with `long long n`. In A1 the `else` arm is a signed division
by 2, which the code generator expands to `shr $63; add; sar` because `n` might be negative, and the `if` stays a branch on the
parity of `n`, which is unpredictable. A2's instcombine rewrites the division to `ashr exact i64 %v6, 1`: the branch condition
`(n & 1) == 0` dominates it, so the low bit of `n` is known zero, `n` is a multiple of 2 and the division is exact
(`-mllvm -print-changed=quiet -mllvm -filter-print-funcs=chain_length` shows the rewrite in the first `InstCombinePass` and the
`select` in the next `SimplifyCFGPass`). simplifycfg then speculates both arms and selects; the loop body becomes
`lea; inc; sar; test; cmovne`.

Leaving one pass out of A2 (A2 0.369, N 0.522): no `instcombine` 0.510, no `simplifycfg` 0.457, all others within 5% of A2.
Adding passes to A1, and applying the two rewrites by hand to `collatz.bcc.ll` (codegen only), agree:

| Variant | Seconds | Gap closed |
| --- | --- | --- |
| A1 | 0.540 | 0% |
| + `inline` | 0.511 | 18% |
| + `simplifycfg` | 0.537 | 2% |
| + `instcombine` | 0.436 | 65% |
| + `simplifycfg`, `instcombine` | 0.363 | 111% (A2 0.381 in this run) |
| by hand: `sdiv` to `ashr exact` | 0.425 (A1 0.508) | 56% |
| by hand: branch to `select`, `sdiv` kept | 0.509 | 0% |
| by hand: `select` and `ashr` | 0.359 | 100% (A2 0.359) |

The `select` with `sdiv` kept does not help because CodeGenPrepare converts a select back into a branch when an operand is an
expensive instruction it can sink into the arm (the assembly shows a block named `select.false.sink`;
`CodeGenPrepare::optimizeSelectInst` in `llvm/lib/CodeGen/CodeGenPrepare.cpp`). Select formation therefore pays only once the arms
are cheap, and a bcc implementation should use the same cost rule rather than expect the code generator to sort it out. The x86
machine-level if-converter that could also do it (`early-ifcvt`) is in the pipeline but switched off by default for x86
(`-mllvm -x86-early-ifcvt` turns `chain_length`'s branch into two `cmov`s).

### 3.3 hashtable (1.23)

`insert`, `lookup` and `slot_of` are small static functions called 13 million times; `lookup` returns its result through a pointer
to a local (`&value`), which is the corpus's only address-taken scalar. Leaving one pass out of A2 (A2 0.231, N 0.285): no
`inline` 0.275; no other pass more than 10% (`loop-rotate` 0.254, `correlated-propagation` 0.251). Adding passes to A1 (A1 0.289, A2 0.241): `inline` alone 0.240, which is A2. Adding
`sroa` after inlining (which promotes `value` to a register) changes nothing measurable, because the store and reload of `value`
hit the store buffer. Inlining is the whole gap; the probe loops themselves are memory-bound and identical in every arm.

### 3.4 sorts (1.16)

The merge loop `tmp[k++] = v[j] < v[i] ? v[j++] : v[i++]` is branchy on random data. bcc lowers the conditional to a diamond whose
arms reload `v[j]` and `v[i]` (the loads the comparison already made) and increment one index. A2's simplifycfg makes it
branch-free: both arms are speculated (the loads are safe to speculate because the same addresses were loaded earlier in the
block), the value becomes a `select` and the increments become `add (zext cmp)`. Instcombine also rewrites the two `sdiv x, 2` to
`lshr`, because `x = hi - lo` with `lo < hi` (or `hi - lo >= 2`) is known non-negative; those run once per call and do not matter.

Leaving one pass out of A2 (A2 0.231, N 0.256): no `simplifycfg` 0.264, everything else within noise; `ipsccp`, which specializes
`quicksort` and `mergesort` to the one global array each is called with, and `loop-idiom`, which turns the copy loops into
`memcpy`, are worth nothing measurable. Adding passes to A1 (A1 0.260, A2 0.226): `simplifycfg` alone 0.260, `simplifycfg` and
`instcombine` 0.225, `ipsccp`/`globalopt`/`deadargelim` 0.265, `correlated-propagation` 0.265, `loop-idiom` 0.256. Simplifycfg
needs instcombine's help because the loads in the arms block speculation until the pass pipeline has cleaned them up; in bcc terms,
the arms' loads are redundant with the comparison's (no store intervenes), so load CSE followed by select formation is the
equivalent.

### 3.5 bytecode (1.08)

The dispatch loop is an if-chain `if (i->op == LOADI) ... else if (i->op == ADD) ...`, and bcc reloads `i->op` in every condition
block because `gvn` keeps no loads. Deleting the five redundant loads by hand in `bytecode.bcc.ll` (replacing them with the first
load; no store lies between them) gives 0.257 s against A1 0.335 and A2 0.311: faster than A2, because A2 goes on to merge the
chain into a `switch` that becomes a jump table, and the single indirect jump predicts worse than the compare chain for this
program. Running simplifycfg on top of the hand-edited file (which forms that switch) gives 0.271. No single LLVM pass left out
of A2 changes it by more than 7%.

### 3.6 sieve, crc32, matmul, nqueens, linkedlist

`sieve` (A1/A2 1.05) gains a little from vectorizing the counting loop and from if-converting its `if (!composite[i])`; no single
pass left out of A2 moves it by more than 8%, which is within the noise of this program. `crc32`, `matmul` and `nqueens` have no
gap: their hot loops are already what the code generator wants, and A2 neither vectorizes `matmul` (a reduction over a strided
column) nor unrolls usefully. `linkedlist` is dominated by `malloc` and cache misses, and its minima agree within 10%.

## 4. What does not matter on this corpus, and why

- **Loop rotation and layout.** MachineBlockPlacement already lays out bcc's top-tested loops bottom-tested (A1's assembly enters
  each loop with one compare and loops on one conditional branch). What IR-level rotation adds is a guard that LICM can rely on;
  leaving `loop-rotate` out of A2 cost nothing measurable on any benchmark.
- **Induction variables and strength reduction.** bcc's 32-bit counters are sign-extended and multiplied by the element size at
  each use, but `loop-reduce` in the code generator rewrites them into pointer increments and scaled addressing; leaving `indvars`
  out of A2 never cost more than 5%.
- **LICM and GVN of loads, cross-iteration forwarding.** `licm`, `gvn` and `loop-load-elim` left out of A2 cost at most 7% on any
  benchmark (`hashtable` without `gvn`). The exception is block-local and dominator-scoped load CSE (sections 3.4 and 3.5), which is a smaller and
  different transformation.
- **SROA of local aggregates.** The corpus has no local arrays or structures on a hot path (zero `alloca`s except `hashtable`'s
  `value`). This is a corpus limitation, not evidence that SROA does not matter; see section 7.
- **Loop idiom, unrolling, vectorization, IPO.** Each was within noise when left out, except a few percent on `sieve`.
- **Byte-offset GEPs.** middle-end.md leaves open whether `getelementptr i8` hampers LLVM. LLVM's own output keeps byte GEPs
  (`getelementptr inbounds nuw i8, ptr @previous, i64 %14` in A2), recent instcombine canonicalizes constant-offset GEPs to that
  form, and the code generator folds `base + sext(i) * 4` into a scaled address. Nothing measured here argues for typed GEPs.

## 5. Our LLVM output

Differences between bcc's `.ll` and clang's, and their effect on A1 (where no IR pass infers anything):

- No function attributes: definitions lack `nounwind`, `noundef` on parameters and results, `local_unnamed_addr`/`unnamed_addr`,
  and the `"target-cpu"="x86-64"` `"tune-cpu"="generic"` strings. The code generator's defaults for this triple are the same CPU
  and tuning, and Win64 emits unwind tables either way, so A1 is unaffected (inference from identical instruction selection in
  the inspected loops). In A2 and A3, `function-attrs` and `inferattrs` recover them.
- No TBAA. The corpus indexes distinct global arrays, which BasicAA separates without TBAA; R, which carries TBAA, is within 3% of
  A2. TBAA will matter for pointer-parameter kernels, which the corpus lacks.
- Arrays are aligned to their element alignment (`align 4`); clang aligns arrays of 16 bytes or more to 16, as the x86-64 ABIs
  ask. That only affects vector code.
- Lowering choices that cost something only because no pass cleans them up: the conditional operator and `if`/`else` assignments
  become diamonds with a block parameter rather than `select`s (sections 3.1, 3.2, 3.4); a comparison feeding a `return`, such as
  `return checksum == EXPECT ? 0 : 1`, becomes a diamond with an empty forwarding block. All of these are the optimizer's job, not
  lowering's, and are covered by select formation.

None of these explains a measurable part of the A1 gap.

## 6. Quantified impact

From the per-benchmark minima of the final run, A1/A2 is 1.08 (`bytecode`), 1.45 (`collatz`), 0.99 (`crc32`), 1.24
(`hashtable`), 2.33 (`levenshtein`), 1.10 (`linkedlist`), 1.00 (`matmul`), 1.00 (`nqueens`), 0.99 (`sieve`) and 1.18 (`sorts`),
geometric mean 1.19. Setting the four benchmarks that the add-pass experiments brought to A2 (`levenshtein` 1.02, `collatz`,
`hashtable` and `sorts` 1.00) gives a projected geometric mean of 1.017; adding load CSE for `bytecode` gives 1.01. Attribution of
that projected improvement by transformation (inference where two are needed together, from the tables above):

| Transformation | Benchmarks | Measured effect |
| --- | --- | --- |
| Inlining | hashtable, levenshtein, (bytecode, collatz) | hashtable 1.23 to 1.00; levenshtein 2.30 to 1.53 alone |
| Select formation | levenshtein, collatz, sorts, (sieve) | levenshtein 1.53 to 1.02 after inlining; needed by collatz and sorts |
| Known-bits division rewrite | collatz, (sorts) | collatz 1.44 to 1.14 alone, 0.95 to 1.00 with select formation |
| Load CSE | bytecode, sorts | bytecode 23% faster than A1 by hand; enables select formation in sorts |
| Everything else in `-O2` | all | under 5% per benchmark on this corpus |

## 7. Roadmap

The bcc IR's design shapes the cost of each item: a function is lifted into a `Draft` that can replace values, fold instructions
to constants, remove and move instructions, add blocks, block parameters and constants, but cannot yet add a general new
instruction; there are no use lists, so a pass computes what it needs (use counts, predecessors, dominators, loops); and phis are
block parameters with arguments on each edge.

A prerequisite for most items is **new instructions in the draft**: `Draft::add_inst(block, position, InstData) -> Value` and
`Draft::rewrite_inst(inst, InstData)`, numbered after the body's instructions as constants already are, with operand-pool storage
for instructions of more than two operands, and `lower` taught to emit them. Small (a few hundred lines), and it removes the
restriction that every rewrite be a fold to a constant or a replacement by an existing value.

In priority order:

1. **Select formation in `simplify-cfg`.** Expected impact: high (`levenshtein` with inlining, `collatz` with item 3, `sorts` with
   item 4). Cost: low, about 200 lines on top of the draft prerequisite. A `brif` whose two edges reach a join block either
   directly or through an arm block with no other predecessor, where each arm holds at most a few speculatable instructions
   (no load unless the same address was loaded in a dominating block with no intervening write, no division by a value not known
   nonzero, no call or store), becomes: the arm instructions moved into the branching block, one `select cond, a, b` per join
   parameter whose two incoming arguments differ, the parameter replaced by the select, and a `jump`. With block parameters the
   rewrite is local to the `brif`, its arm blocks and the join's parameter list, and needs no use lists. Follow LLVM's limits
   (`FoldTwoEntryPHINode` and `SpeculativelyExecuteBB` in `llvm/lib/Transforms/Utils/SimplifyCFG.cpp`; the pinned clang's
   `-two-entry-phi-node-folding-threshold` speculates at most 4 cost units per two-entry join) and CodeGenPrepare's rule from section 3.2: do not form a select whose arm
   is an expensive operation such as a division. Keep `nsw` on speculated arithmetic: poison in the unselected arm of a `select`
   is not undefined behaviour. Fold `select c, 1, 0` to `zext c` as part of the pass.
2. **Inlining.** Expected impact: high (`hashtable` all of its gap, `levenshtein` most, with item 1 all). Cost: medium to high,
   about 800 to 1200 lines with tests. The optimizer runs functions in module order; inlining needs bottom-up order over the
   call graph, computed with Tarjan's SCC algorithm on an explicit stack, so a callee is optimized before its callers. Each call
   site of a defined callee below a size threshold (count instructions after optimization; LLVM's `-inline-threshold` defaults to 225
   cost units, see `llvm/lib/Analysis/InlineCost.cpp`, with a large bonus for the last call site of an internal function) is expanded in the caller's draft: the call's block is split after the call, the callee's blocks are copied
   with values remapped (the callee's entry parameters replaced by the call's arguments through the replacement map), each
   `return v` becomes `jump cont(v)` to a continuation block with one parameter for the result, and the callee's stack slots
   become caller slots. After a round of inlining the caller re-runs the pipeline, so `promote` picks up slots whose address only
   escaped into the callee. Internal functions left without references are deleted. Skip recursive SCCs and `returns_twice` and
   variadic callees at first. The draft prerequisite above must also accept foreign instructions and new blocks with parameters.
3. **Known bits from dominating conditions, and the division rewrites.** Expected impact: medium (`collatz` half its gap alone,
   all of it with item 1). Cost: medium, about 400 lines. A dominator-tree walk on an explicit stack, like `gvn`'s, keeps a scoped
   table of facts implied by the `brif` conditions on the path from the entry: `and x, m == 0` gives known-zero bits of `x`;
   `icmp slt/sge x, C` and comparisons of two values give sign facts. With these: `sdiv x, 2^k` becomes `ashr exact x, k` when
   the low `k` bits are known zero, and `sdiv`/`srem` by a positive constant become `udiv`/`urem` (then `lshr`/`and` for powers
   of two) when `x` is known non-negative (LLVM: `computeKnownBits` with the `DomConditionCache` in
   `llvm/lib/Analysis/ValueTracking.cpp`, `InstCombinerImpl::visitSDiv` in
   `llvm/lib/Transforms/InstCombine/InstCombineMulDivRem.cpp`, and `processSDiv` in
   `llvm/lib/Transforms/Scalar/CorrelatedValuePropagation.cpp`). The facts are attached to blocks, not uses, which suits the
   absence of use lists. `nsw` subtraction of a smaller from a larger value (`hi - lo` under `lo < hi`) is the second case
   `sorts` shows.
4. **Load CSE in `gvn`.** Expected impact: medium (`bytecode` 23% by hand; the enabler of item 1 in `sorts`). Cost: medium, about
   300 to 500 lines with a small alias oracle. Give non-volatile loads a key (address value, type) in the existing scoped table,
   valid only while no instruction that may write the location has run since. EarlyCSE's memory generation is the simplest
   sound rule: a counter incremented by every store, call, `copy`, `fill` and `va_*`, recorded with each available load; a load is
   reused only if its twin's generation equals the current one, and entering a block with more than one predecessor starts a new
   generation (`llvm/lib/Transforms/Scalar/EarlyCSE.cpp`). A store also makes its value available for the same address
   (store-to-load forwarding). An alias oracle that separates distinct globals and stack slots, and constant offsets from one
   base, lets stores to other objects keep the generation; it is the same oracle that LICM of loads and dead-store elimination
   need later.
5. **Keep the existing passes, but stop expecting speed from them through LLVM.** `fold`, `sccp`, `gvn`, `licm`, `dce` and
   `simplify-cfg` duplicate the code generator's work in arm A1 (section 2), so improving them will not move A1. They are the
   cleanup that items 1 to 4 need between rounds, and they will matter for a native back end, which has no SelectionDAG behind
   it. No change proposed.
6. **Later, once the corpus can show it:** LICM of loads and scalar promotion of memory in loops (needs item 4's alias oracle),
   loop rotation (to give LICM guarded preheaders), SROA of local structures and arrays (extend `promote` to slots accessed at
   constant offsets with non-overlapping types), loop idiom (`copy`/`fill` from loops), and TBAA tags from sema's types. None of
   these is measurable on `benches/c`; adding pointer-parameter kernels, local structures and Polybench-style loops to the corpus
   should come before implementing them.
7. **Not recommended:** switch formation from if-chains (slower than the compare chain on `bytecode`), and vectorization and
   unrolling (a few percent on `sieve` only, at a large cost).

Expected outcome: items 1 to 4 bring the A1 geometric mean from about 1.19 to about 1.01 on this corpus, with item 1 and item 2
together delivering most of it. Item 1 is the cheapest and enables three of the four benchmarks, so it should come first even
though it shows its full effect only together with items 2 to 4.
