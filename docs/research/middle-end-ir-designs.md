# Middle-end IR designs for bcc-rust

Research date: 2026-10-11. This note surveys intermediate representations that
should inform bcc-rust's middle end, then recommends a design. The goals are an
in-house optimizer, a faithful lowering to LLVM IR so LLVM can optimize the same
program for a performance comparison, arena allocation with dense indices, and
an IR that is quick to build and easy to verify. It does not design the IR in
full; it fixes the choices the evidence supports and lists the ones it does not.

Method: every design below was read from its own documentation, specification,
design paper or source. LLVM statements were checked against the `LangRef.rst`
and Clang `CodeGen` sources at tag `llvmorg-21.1.0`; the repository pins a
newer LLVM (23.1.1), so re-check any rule that the recommendation leans on
before implementing it. Cranelift, MLIR, V8, rustc and QBE sources were read at
their current default branches on the research date. I did not read Click's
Rice thesis; his 1995 paper with Paleczny, which describes the same IR, stands
in for it. The GCC, libFirm, ClangIR, V8 and Fallin pages were read through a
summarizing fetch tool rather than verbatim, so figures taken from them should
be re-read at the source before they are quoted elsewhere. Where a statement is
an inference rather than a report of a source, it says so.

## What the IR has to consume

bcc-rust stops after semantic analysis. The retained `SemanticTranslationUnit`
holds canonical type nodes, record layouts with byte and bit offsets, typed
expressions with conversion records, label maps and sorted switch cases, but no
control-flow graph, no executable form of initializers, and no calling
convention. [`semantic-analysis.md`](../../semantic-analysis.md) lists the
remaining backend work: labels and branch targets, switch dispatch, runtime VLA
allocation, automatic lifetime cleanup, return lowering, calling convention,
initializer materialization and relocations. Two repository rules constrain the
design: compiler code allocates only from arenas and never needs `Drop`, and
traversals use explicit work stacks instead of Rust recursion
([`.agents/AGENTS.md`](../../.agents/AGENTS.md)). The typed syntax tree plus
semantic graph is therefore already a high-level, C-aware representation. That
fact drives the one-IR-or-two question at the end.

## Survey

### LLVM IR

**Core representation.** A function is a list of basic blocks in SSA form. The
`phi` instruction must be first in its block and carries one value per
predecessor basic block; each incoming value's use is deemed to occur on the
incoming edge. Every use must be dominated by its definition, which the verifier
checks. [LangRef: phi](https://llvm.org/docs/LangRef.html#i-phi),
[well-formedness](https://llvm.org/docs/LangRef.html#well-formedness)

**Memory.** Memory is not in SSA form. Variables that need an address use
`alloca`; loads and stores are ordinary instructions ordered by position.
[LangRef: alloca](https://llvm.org/docs/LangRef.html#i-alloca) Frontends are told
to put allocas in the entry block because SROA and Mem2Reg only promote those.
[Frontend performance tips](https://llvm.org/docs/Frontend/PerformanceTips.html#use-of-allocas)
Clang emits every local this way and relies on promotion. Braun et al. measured
that 25 percent of the instructions the LLVM front end produced were local
variable loads and stores that SSA construction immediately removed.
[Braun et al. 2013, section 1](https://pp.ipd.kit.edu/uploads/publikationen/braun13cc.pdf)
LLVM also offers an analysis-only memory SSA: `MemorySSA` is a virtual IR built
on demand, with one `MemoryDef` chain per function, not part of the instruction
set. [MemorySSA](https://llvm.org/docs/MemorySSA.html)

**Types.** Integers are `iN` and signless; signedness belongs to the operation
(`sdiv` versus `udiv`, `icmp slt` versus `ult`).
[Integer type](https://llvm.org/docs/LangRef.html#t-integer) Pointers are a
single opaque `ptr` type. Frontends are advised not to load or store
first-class aggregates and to access fields individually, except for returning
several register values.
[Performance tips: aggregates](https://llvm.org/docs/Frontend/PerformanceTips.html#avoid-creating-values-of-aggregate-type)

**C semantics.**

- Signed overflow: `add`, `sub`, `mul` take `nsw`/`nuw`, and violating the flag
  yields a poison value rather than immediate UB.
  [LangRef: add](https://llvm.org/docs/LangRef.html#i-add) Clang selects
  `nsw` per operation from `-fwrapv` state
  ([`CGExprScalar.cpp`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.0/clang/lib/CodeGen/CGExprScalar.cpp)).
  `sdiv` by zero and `INT_MIN / -1` are immediate UB; an over-wide `shl` yields
  poison. [LangRef: sdiv](https://llvm.org/docs/LangRef.html#i-sdiv),
  [shl](https://llvm.org/docs/LangRef.html#i-shl)
- Poison and `freeze`: poison stops propagating at `select` or `freeze`, and
  branching on it is UB. The LangRef asks frontends to prefer poison to `undef`.
  [LangRef: poison](https://llvm.org/docs/LangRef.html#poisonvalues),
  [undef](https://llvm.org/docs/LangRef.html#undefvalues)
  The PLDI 2017 paper explains why both are needed and how `freeze` repairs
  loop unswitching and GVN. [Lee et al., Taming Undefined Behavior in LLVM](https://www.cs.utah.edu/~regehr/papers/undef-pldi17.pdf)
- Strict aliasing: `!tbaa` access tags `(base type, access type, offset)` over a
  tree of type descriptors, plus `noalias` on parameters and scoped
  `!alias.scope`/`!noalias` metadata.
  [LangRef: TBAA](https://llvm.org/docs/LangRef.html#tbaa-node-semantics),
  [noalias](https://llvm.org/docs/LangRef.html#parameter-attributes)
  Clang omits TBAA at `-O0` and under relaxed aliasing
  ([`CodeGenTBAA.cpp`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.0/clang/lib/CodeGen/CodeGenTBAA.cpp)).
- `volatile` is a flag on `load`, `store` and `llvm.memcpy`; the optimizer may
  not add, drop or reorder volatile operations relative to each other.
  [LangRef: volatile](https://llvm.org/docs/LangRef.html#volatile)
- Bit-fields do not exist. Clang gives the record a byte-array or `iN` storage
  unit and emits load, shift, mask sequences.
  [`CGRecordLayout.h`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.0/clang/lib/CodeGen/CGRecordLayout.h)
- Struct copy is `llvm.memcpy`; Clang passes whether the two objects may
  overlap, because C permits exact self-assignment
  ([`CGExprAgg.cpp`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.0/clang/lib/CodeGen/CGExprAgg.cpp),
  [`llvm.memcpy`](https://llvm.org/docs/LangRef.html#int-memcpy)).
- Varargs: `llvm.va_start`/`va_copy`/`va_end` over a target-specific `va_list`
  ([LangRef](https://llvm.org/docs/LangRef.html#int-va-start)); Clang expands
  `va_arg` in per-ABI code (`ABIInfo::EmitVAArg`, reached from `EmitVAArg` in
  [`CGCall.cpp`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.0/clang/lib/CodeGen/CGCall.cpp)).
- `setjmp`: the `returns_twice` function attribute, which disables optimizations
  such as tail calls in callers. Clang applies it by callee name.
  [LangRef: function attributes](https://llvm.org/docs/LangRef.html#function-attributes),
  [`CGCall.cpp`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.0/clang/lib/CodeGen/CGCall.cpp)
- `goto` and `switch` are `br` and `switch` terminators over arbitrary CFGs;
  computed goto is `indirectbr`; VLAs use `alloca` with
  `llvm.stacksave`/`stackrestore`.
  [switch](https://llvm.org/docs/LangRef.html#i-switch),
  [indirectbr](https://llvm.org/docs/LangRef.html#i-indirectbr),
  [stacksave](https://llvm.org/docs/LangRef.html#int-stacksave)
- Struct arguments: `byval(<ty>)` and `sret(<ty>)` parameter attributes; the
  frontend must already have done target ABI classification.
  [Parameter attributes](https://llvm.org/docs/LangRef.html#parameter-attributes)

**Data layout.** In memory, LLVM is pointer-based. Instructions sit on an
intrusive list per block ([`BasicBlock.h`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.0/llvm/include/llvm/IR/BasicBlock.h)),
and each value keeps a use list threaded through `Use` objects, described as a
notionally two-dimensional linked list
([`Use.h`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.0/llvm/include/llvm/IR/Use.h)).
That buys O(1) replace-all-uses at the cost of pointer chasing and per-edge
bookkeeping.

**Lowering to LLVM** is the identity; the lesson is how much work a C frontend
does before reaching LLVM: ABI classification, bit-field layout, va_arg
expansion, TBAA tag construction, overflow flags and attribute inference.

### Cranelift CLIF and the aegraph mid-end

**Core representation.** A CFG of blocks in SSA form with no phi instruction;
blocks declare typed parameters and every branch passes matching arguments. The
entry block's parameters are the function parameters.
[CLIF reference](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/docs/ir.md)
Values are defined once and defs must dominate uses. The reference describes no
poison, `nsw` flag or `undef`; undefined behavior appears only for accesses to
unaddressable memory, and loads and stores take `notrap`, `aligned` and
`readonly` flags. Variables that need an address live in explicit stack slots.

**Types.** Fixed-width integers (i8 to i128), f32/f64, vectors; signedness is
chosen per instruction; no aggregate types. Calling conventions and `sret`/`sarg`
parameter extensions are part of the signature. (CLIF reference, as above.)

**Building SSA from a front end.** `cranelift-frontend` implements Braun et al.
with `declare_var`, `def_var`, `use_var` and `seal_block`, so a front end never
runs a separate SSA pass. The implementation of the recursive algorithm is an
explicit state machine over `calls` and `results` vectors rather than
recursion. [`ssa.rs`](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/frontend/src/ssa.rs)
A `Switch` helper lowers sparse C-style `switch` onto compares, jump tables or a
mix, because the IR's `br_table` is dense only.
[`switch.rs`](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/frontend/src/switch.rs)

**Data layout.** This is the closest precedent for bcc-rust's constraints. The
`cranelift-entity` crate keys arrays by small integer entity references
(typically `u32` newtypes):
`PrimaryMap` owns the entities, `SecondaryMap` and `EntitySet` attach data
without storing keys, and `EntityList` stores short lists in a shared pool with
a smaller footprint than `Vec`.
[`cranelift-entity`](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/entity/src/lib.rs)
Instructions are a `PrimaryMap<Inst, InstructionData>` and a test pins
`InstructionData` at 16 bytes
([`instructions.rs`](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/codegen/src/ir/instructions.rs),
[`dfg.rs`](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/codegen/src/ir/dfg.rs)).
Block order and instruction order within blocks are doubly linked lists stored in
`SecondaryMap`s, so insertion and removal never move data
([`layout.rs`](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/codegen/src/ir/layout.rs)).
I found no use lists in `dfg.rs`; replacing a value is done by turning it into
an alias of another (`change_to_alias`).

**Mid-end: aegraphs.** Cranelift replaced separate GVN, LICM and simple
pre-optimization passes with one "acyclic e-graph" pass. Pure operators float as
nodes in a sea of nodes; the side-effecting skeleton (loads, stores, calls,
block parameters) stays in the CFG in original order. Rewrite rules are
written in ISLE and fire once, when a node is created. Elaboration walks the
dominator tree with a scoped hash map, which performs GVN, places loop
invariants in preheaders (LICM) and rematerializes cheap values.
[Aegraph RFC](https://github.com/bytecodealliance/rfcs/blob/main/accepted/cranelift-egraph.md)
Fallin's 2026 retrospective gives the current numbers: against the earlier
classical pipeline the aegraph yields about 2 percent faster code for roughly
7 to 8 percent more compile time; the multi-representation part (union nodes)
adds only about 0.1 percent over the sea-of-nodes-with-CFG core, with an average
e-class of 1.13 nodes. The author concludes that the sea-of-nodes aspect works
well and that the e-class part may not yet pay for itself.
[Fallin, The acyclic e-graph](https://cfallin.org/blog/2026/04/09/aegraph/)

**Lowering to LLVM.** There is none, and the design explains why it would lose
information: CLIF has no UB flags, TBAA or struct types. Cranelift is a precedent
for the IR shape, not for LLVM fidelity.

### MLIR and ClangIR

**MLIR.** Uses block arguments instead of phis. The rationale lists five reasons:
phis must stay at block starts and be skipped by transforms, function arguments
need no separate node, phi groups execute atomically (the lost-copy problem),
unordered phi entry lists scale poorly for blocks with thousands of
predecessors, and a value cannot be defined on only one outgoing edge.
Integers are signless by default, as in LLVM.
[MLIR rationale](https://mlir.llvm.org/docs/Rationale/Rationale/#block-arguments-vs-phi-nodes)
The LLVM dialect documents the mapping both ways: blocks arguments become phis,
and terminators that reach the same block with different arguments are legal.
[LLVM dialect](https://mlir.llvm.org/docs/Dialects/LLVM/#phi-nodes-and-block-arguments)
The translator creates one phi per block argument when it enters a block and
adds incoming values after all blocks exist
([`ModuleTranslation.cpp`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.0/mlir/lib/Target/LLVMIR/ModuleTranslation.cpp)).
Regions, dialects and progressive lowering are MLIR's real contribution, and they
come with a dynamic op and attribute system.

**ClangIR (CIR).** A C/C++-level dialect between the Clang AST and LLVM IR; the
official page warns it is still being upstreamed, is off in default builds, and
may be out of date. [ClangIR docs](https://clang.llvm.org/docs/CIR/) Its
stated motivation: LLVM IR has lost source semantics, idiom recognition and
analyses want structure, and codegen lowers too early.
[Upstreaming RFC](https://discourse.llvm.org/t/rfc-upstreaming-clangir/76587)
The operation list shows what a C-level IR keeps that LLVM does not: structured
`if`/`for`/`while`/`do`/`switch` and `scope` regions, `goto` and `label`, a
bit-field get/set pair, `copy`, `freeze`, stack save and restore, `nsw`/`nuw`
flags on integer operations, signed and unsigned integer types, and `volatile`
on memory operations
([`CIROps.td`](https://github.com/llvm/llvm-project/blob/main/clang/include/clang/CIR/Dialect/IR/CIROps.td)).
Lowering goes through a "flatten CFG" step that turns structured regions and
cleanups into blocks
([cleanup and EH design](https://clang.llvm.org/docs/CIR/CleanupAndEHDesign.html)),
a "lowering prepare" pass, and a late ABI-lowering pass that rewrites signatures
and calls to `sret`, `byval` and coerced forms, deliberately separate from CIR
generation
([ABI lowering](https://clang.llvm.org/docs/CIR/ABILowering.html)). The RFC's
cost data is a warning for a compile-time-sensitive project: adding MLIR raised
Clang's build time by about 45 percent and the stripped binary from 95 to
139 MiB, and no reliable compile-time numbers existed.

### GCC GIMPLE and SSA

**Core representation.** GIMPLE is three-address code lowered from GENERIC; the
C front end produces it directly. High GIMPLE keeps `GIMPLE_BIND` and
`GIMPLE_TRY` containers; the lower-control-flow pass flattens `if` into two
gotos and removes lexical scopes, giving Low GIMPLE.
[GIMPLE](https://gcc.gnu.org/onlinedocs/gccint/GIMPLE.html),
[Tree SSA passes](https://gcc.gnu.org/onlinedocs/gccint/Tree-SSA-passes.html)
GCC is therefore a two-level design within one data structure, split only to
expose control flow and exception jumps.

**SSA and memory.** `SSA_NAME` nodes record a version and defining statement;
PHI nodes merge. Scalars whose address is never taken get real SSA names, while
memory is renamed through virtual operands (`VDEF` and `VUSE`), which GCC now
tracks with a single `.MEM` version rather than one per variable. The older
operands page still describes per-variable virtual operands, which LLVM's
MemorySSA document says GCC eventually replaced with one.
[SSA](https://gcc.gnu.org/onlinedocs/gccint/SSA.html),
[SSA operands](https://gcc.gnu.org/onlinedocs/gccint/SSA-Operands.html),
[alias analysis](https://gcc.gnu.org/onlinedocs/gccint/Alias-analysis.html)
Passes that disturb SSA must register name mappings and request `update_ssa`;
that burden is the price of an SSA IR that passes mutate.

**C semantics.** Signed overflow is controlled by `-fwrapv`; strict aliasing by
`-fstrict-aliasing` and alias sets, where nodes in different alias sets may not
alias and each front end supplies the sets (the C front end uses
`c_get_alias_set`).
[`-fwrapv`](https://gcc.gnu.org/onlinedocs/gcc/Code-Gen-Options.html#index-fwrapv),
[`-fstrict-aliasing`](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html#index-fstrict-aliasing)

**Lowering to LLVM** does not exist inside GCC; GIMPLE's value for bcc-rust is
the staged lowering and the statement that C-specific alias information is
supplied by the front end.

### QBE, with cproc as its C front end

**Core representation.** SSA with an optional `phi`; the IL accepts non-SSA input
and QBE builds SSA itself. Types are `w`, `l`, `s`, `d` plus `b`/`h` inside
aggregates; signedness lives in operations; `alloc4/8/16` reserves stack;
`blit` copies bytes; calls carry full argument types, aggregates are passed
by pointer and variadics are `...`/`vastart`/`vaarg`.
[QBE IL](https://c9x.me/compile/doc/il.html)

**Passes.** The whole pipeline is a fixed list in one function: `promote`
(stack slots to temporaries), `ssa`, `ssacheck`, `loadopt`, `coalesce`, `gvn`,
`simplcfg`, `gcm`, optional `ifconvert`, ABI lowering, instruction selection and
register allocation, with the use-information pass `filluse` re-run after almost
every step
([`main.c`](https://c9x.me/git/qbe.git/tree/main.c)). The `ssacheck` verifier
runs between passes. The front page claims 70 percent of the performance of
industrial compilers in about 10 percent of the code
([QBE](https://c9x.me/compile/)); treat that as the author's claim, not a
measurement. Data structures are plain arrays and structs: an instruction is a
packed opcode and class, a destination and two operand references
([`all.h`](https://c9x.me/git/qbe.git/tree/all.h)).

**cproc.** A C11 compiler that emits QBE IL. It builds basic blocks directly
from its AST, uses `alloc` slots for locals, and implements bit-fields by shift
and mask with signedness-selected `sar`/`shr`
([`qbe.c`](https://github.com/michaelforney/cproc/blob/master/qbe.c)). It
reports `volatile store is not yet supported` as an error, which marks QBE's gap
for C: no volatile, no overflow or aliasing flags, no `returns_twice`. cproc shows
that a small IR can host a nearly complete C front end, and also what a lowering
to LLVM from such an IR would be missing.

### libFirm, with cparser as its C front end

**Core representation.** A graph of nodes with a program-wide number and
per-graph index; blocks are nodes, control flow edges are stored as per-block
predecessor arrays whose order matters for Phi inputs; multi-result nodes return
tuples extracted with `Proj`.
[libFirm introduction](https://pp.ipd.kit.edu/firm/Introduction.html)
Memory is a state value: `Load` takes `mem` and `ptr`, `Store` takes `mem`, `ptr`
and a value, `Call` and `Div` take and produce memory, and `Div` also yields
exception control flow. Loads and stores have `volatility` and `unaligned`
attributes, and volatile accesses are visible side effects that may not be
optimized away. `Switch` maps selector values to `Proj` numbers, and `Member` and
`Sel` compute addresses from entities.
[libFirm nodes](https://pp.ipd.kit.edu/firm/Nodes.html)
The library constructs SSA directly from an attributed syntax tree
([README](https://github.com/libfirm/libfirm)); the Braun et al. algorithm
originates in this group, and its libFirm implementation interleaves local
optimizations with construction and cuts the final graph to 88.2 percent of the
nodes. [Braun et al., section 6.2](https://pp.ipd.kit.edu/uploads/publikationen/braun13cc.pdf)
cparser is a recursive-descent C99 front end that targets libFirm and acts as a
drop-in gcc replacement ([cparser](https://github.com/libfirm/cparser)), which
makes libFirm the only surveyed IR with a long-lived, whole-C99 front end on a
graph IR. I did not find documentation of its overflow or aliasing flags in the
pages above, so I make no claim about them.

### Sea of nodes and V8's move to Turboshaft

**Click and Paleczny.** One graph holds data and control: vertices are opcodes,
ordered inputs, unordered outputs; `Region` and `If` nodes stand for basic
blocks and branches, `Phi` is tied to a `Region`; memory is a single `STORE`
value threaded through `LOAD`/`STORE` nodes, which the authors call very coarse
and suggest splitting per variable; volatile I/O gets a separate state value.
Nodes live in an arena with fast bump allocation.
[Click and Paleczny 1995](https://dl.acm.org/doi/10.1145/202530.202534)

**Why V8 left.** V8's post gives these reasons for Turbofan's JavaScript
pipeline: effect and control chains had to be maintained by hand during
lowering and were repeatedly wrong, and most JavaScript operations are effectful
anyway; the scheduler was complex and re-hoisted duplicated nodes; peephole
passes visited nodes from the returns upward, so a node was often visited
several times and changed rarely; load elimination bailed out on large graphs
(the CFG version is up to 190 times faster with much less memory); new nodes
were created far from their originals, giving about three times as many L1
data-cache misses (up to seven in some phases) and an estimated, by the post's
own account handwavy, up to 5 percent of compile time; and control-flow
dependent typing, finding loop contents and adding control flow during lowering
were all awkward. Compile time roughly halved with a CFG-based design, and the
post describes the new IR as having fixed instruction positions in blocks.
[V8: Land ahoy, leaving the sea of nodes](https://v8.dev/blog/leaving-the-sea-of-nodes)
Turboshaft stores operations in a zone-allocated append-only buffer of 8-byte
slots addressed by `OpIndex` offsets
([`graph.h`](https://github.com/v8/v8/blob/main/src/compiler/turboshaft/graph.h)).
Caveat: nearly every JavaScript operation is effectful, which limits what
floating pure nodes can gain there. C has more pure arithmetic, so V8's result
does not transfer fully; Cranelift's skeleton-plus-floating-pure-nodes design
(for WebAssembly and Rust input) reports a gain. The sources agree on the main
point: effects and memory belong on a fixed CFG skeleton.

### Rust MIR

MIR is a CFG of basic blocks with statements and a terminator, no nested
expressions, fully explicit types, and mutable locals addressed as places. It is
deliberately not SSA: RFC 1211 notes that temporaries are single-assignment but
can be borrowed, "more analogous to allocas than SSA values", and says SSA
analyses can run on the subset of unborrowed locals or on a later lower-level IR.
[RFC 1211](https://rust-lang.github.io/rfcs/1211-mir.html),
[MIR overview](https://rustc-dev-guide.rust-lang.org/mir/index.html)
MIR is built from the typed THIR
([construction](https://rustc-dev-guide.rust-lang.org/mir/construction.html)).
Borrow checking and MIR-level optimizations run on it; the optimization guide
gives the compile-time rationale that optimizing MIR leaves LLVM less to do
([MIR optimizations](https://rustc-dev-guide.rust-lang.org/mir/optimizations.html)).
SSA appears only at code generation: `rustc_codegen_ssa` runs an analysis that
decides which locals need an alloca and which can be SSA operands
([`analyze.rs`](https://github.com/rust-lang/rust/blob/master/compiler/rustc_codegen_ssa/src/mir/analyze.rs)),
using `IndexVec`, `DenseBitSet` and dominators. The RFC records that moving the
language's semantics out of the LLVM-lowering step into a mid-level IR was a goal
because migrating away from LLVM was otherwise nearly impossible. This is the
strongest precedent for a mid-level IR whose purpose is to be lowered to LLVM
and to other backends, and it is a precedent for keeping that level non-SSA.

### Braun et al. 2013: SSA construction straight from an AST

The algorithm needs no dominators, dominance frontiers or prior analysis and
keeps the IR in SSA form during construction. Each block tracks the current
definition of every variable; a read in a block without a definition recursively
reads predecessors; a block that is not yet sealed (not all predecessors known)
gets an operandless phi that is completed at sealing; trivial phis (all operands
equal one value or itself) are removed as soon as detected, with their users
rechecked. The result is pruned SSA for any CFG and minimal SSA for reducible
CFGs; an extra post pass handles irreducible flow. Across SPEC CINT2000 in
LLVM 3.1 the runtime matched Cytron et al., and the executed instruction count
was 99.72 percent of LLVM's; combining construction with on-the-fly
optimization cut the node count to 88.2 percent and shortened total compilation
by 1.49 seconds on a 147 second total, at 0.84 seconds extra construction time.
The paper also shows CPS output, with parameters in place of phis.
[Braun et al.](https://pp.ipd.kit.edu/uploads/publikationen/braun13cc.pdf),
[DOI 10.1007/978-3-642-37051-9_6](https://doi.org/10.1007/978-3-642-37051-9_6)
Cranelift's `ssa.rs` is a production Rust implementation in block-parameter form
with an explicit stack in place of recursion (links above).

## Comparison

| IR | CFG or graph | Merge | Memory | C-specific semantics | Layout |
| --- | --- | --- | --- | --- | --- |
| LLVM | CFG | phi, first in block | alloca plus loads and stores; MemorySSA analysis | nsw/nuw, poison, TBAA, volatile, `returns_twice`, byval, memcpy | pointers, intrusive lists, use lists |
| CLIF | CFG | block parameters | stack slots, loads and stores, flags | none; no UB flags | dense u32 entities, 16-byte instructions |
| Aegraph | CFG skeleton plus floating pure nodes | block parameters | skeleton, ordered | none | same, plus union nodes |
| CIR | structured regions then CFG | block arguments | alloca/load/store | structured control flow, bit-field ops, nsw, volatile | MLIR objects |
| GIMPLE | CFG | PHI | SSA for scalars, one virtual memory SSA name | front-end alias sets, `-fwrapv` | tuples, SSA names |
| QBE | CFG | phi | alloc slots promoted by `promote` | little; no volatile or UB flags | arrays and packed structs |
| libFirm | graph, blocks as nodes | Phi | memory state value (Mem) | volatility flag on Load and Store | graph nodes |
| Sea of nodes | graph | Phi | single store value | volatile via I/O state | arena nodes |
| MIR | CFG | none (mutable places) | places, locals | Rust semantics | `IndexVec` |

The recurring patterns are that every production CFG IR except LLVM's has
converged on block parameters, that memory ordering is either implied by
position or carried by a state value, that dense indices dominate recent
designs, and that every IR which is meant for C-like source carries explicit
flags for the semantics an optimizer may exploit.

## C semantics mapped to an IR

The right-hand columns are recommendations; the left citations are to the
standard. Citations follow the repository's form and were checked with the
`search-standard` tool.

| C99 rule | IR representation | LLVM lowering |
| --- | --- | --- |
| Signed overflow is UB (C99: §6.5 paragraph 5, p. 67; PDF p. 79) | `nsw` flag on add, sub, mul, neg, shl; absent under `-fwrapv` | `nsw` |
| Shift by negative or over-wide count is UB (C99: §6.5.7 paragraph 3, p. 84; PDF p. 96) | shifts yield poison, as LLVM | `shl`/`lshr`/`ashr` |
| Pointer arithmetic outside an array is UB (C99: §6.5.6 paragraph 8, p. 83; PDF p. 95) | `inbounds` flag on pointer add | `getelementptr inbounds` |
| Effective-type access rule (C99: §6.5 paragraph 7, p. 68; PDF p. 80) | per-access tag (record, scalar type, byte offset) | `!tbaa` struct-path tag |
| `restrict` (C99: §6.7.3.1 paragraph 4, p. 110; PDF p. 122) | `noalias` on parameters first; block scope later | `noalias`; scoped metadata later |
| `volatile` accesses follow the abstract machine (C99: §6.7.3 paragraph 6, p. 109; PDF p. 121) | `volatile` flag on loads, stores and copies; slot never promoted | `volatile` |
| Bit-field storage unit is implementation-defined (C99: §6.7.2.1 paragraph 10, p. 102; PDF p. 114) | lower to storage-unit load plus shift and mask using sema's offsets | same |
| Struct assignment with exact overlap only (C99: §6.5.16.1 paragraph 3, p. 92; PDF p. 104) | `copy` op with a may-overlap flag | `llvm.memcpy` or `memmove` |
| Indeterminate automatic values (C99: §6.7.8 paragraph 10, p. 126; PDF p. 138) | `poison` for an unwritten SSA variable; stack slot content uninitialized | `poison`, no store |
| Non-volatile locals changed after `setjmp` are indeterminate after `longjmp` (C99: §7.13.2.1 paragraph 3, p. 244; PDF p. 256) | call attribute `returns_twice` on callee | `returns_twice` |
| `va_arg` type mismatch is UB (C99: §7.15.1.1 paragraph 2, pp. 249-250; PDF pp. 261-262) | `va_start`, `va_copy`, `va_end` ops; `va_arg` expanded per target | intrinsics plus per-ABI expansion |
| `goto` targets any label in the function (C99: §6.8.6.1 paragraph 1, p. 137; PDF p. 149) | ordinary edges; Braun construction handles any CFG | `br` |
| `switch` cases are distinct integer constants (C99: §6.8.4.2 paragraph 3, p. 134; PDF p. 146) | multiway `switch` terminator with sorted cases | `switch` |

C99 text: [N1256](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1256.pdf),
mirrored at `standards/c99-n1256.html`.

## Recommendation

The recommendations are ordered by how much downstream work each decision
removes. Where evidence is thin, the text says so.

### 1. One IR, not two; reuse the typed AST as the high level

Use a single SSA IR built directly from the semantic graph, and treat the
sema-annotated syntax tree as the C-aware high level that every surveyed
two-level design has to invent for itself. The evidence:

- Two-level designs exist where the high level has to carry semantics the
  frontend AST cannot cheaply provide: Rust's borrowck needs places and drops
  (RFC 1211), CIR wants structured control flow and cleanups for lifetime and
  idiom analyses (upstreaming RFC), and GCC separates High and Low GIMPLE to
  flatten scopes and expose exception jumps.
- bcc-rust already retains scope trees, type graphs, record layouts, conversion
  records, label maps and sorted switch cases, and has no goal for C-level
  analyses beyond optimization. A CIR-style level would add a second tree to
  build, verify and keep in sync (CIR's published build and size costs come from
  its MLIR dependency, so they do not measure this duplication; the argument
  here is the duplication itself).
- Single-IR C pipelines work: cproc into QBE and cparser into libFirm go from
  AST to the one SSA-form IR.

Instead of a second IR, give the one IR two verifier profiles, modelled on
QBE's `abi0`/`abi1` hooks and CIR's staged passes:

- *Pre-ABI*: calls and parameters carry C types (including aggregates by value),
  with `copy` for aggregate assignment and `va_arg` as a high-level op.
- *Post-ABI*: aggregates are lowered to `sret`/`byval`/coerced scalars, `va_arg`
  is expanded, and everything is a scalar or pointer for the target.

The LLVM path and the in-house backend branch after the post-ABI step so both
see the same lowering, which keeps the comparison fair. Build the ABI lowering
as its own module with target classification tables; CIR's design document
argues for exactly that separation and cites the maintenance cost of the
alternative. If a later experiment needs a C-level transformation, such as
struct promotion before ABI lowering, it can run in the pre-ABI profile
without a new IR.

### 2. CFG with block parameters, not phis, not a sea of nodes

- Use a CFG of basic blocks. V8's reasons, Cranelift's aegraph structure (a CFG
  skeleton with floating pure nodes only after the fact) and the whole CFG-IR
  survey point the same way, and a fixed position per instruction makes the
  verifier, textual dump and diffing simple.
- Use **block parameters** with arguments on branch edges. They are what
  Cranelift, MLIR and Swift's SIL (credited by the MLIR rationale) converged on; MLIR's list of five problems with phis
  applies unchanged to an IR that is built, edited and verified by a small team.
  Braun construction maps onto them directly (the algorithm's incomplete phi is a
  block parameter appended while the block is unsealed; completion adds an
  argument to each predecessor's terminator, as in Cranelift's `ssa.rs`).
  Function parameters are the entry block's parameters, removing a node kind.
- The lowering to LLVM is mechanical and has a known shape from MLIR's
  translator: create one phi per block parameter at block entry, emit all blocks,
  then add each predecessor's argument as an incoming value.
- One subtlety to design out: LLVM's phi has one entry per predecessor block,
  while block arguments allow two edges from one terminator to the same block
  with different values (MLIR's LLVM-dialect page documents this as legal on its
  side). Choose a rule now: either the construction and `switch` lowering never
  create such edges (split with a forwarding block), or the LLVM printer
  splits them. Splitting at construction costs one extra block in rare cases and
  keeps the IR verifier simple; I recommend it.
- Do not use a sea of nodes as the stored form. The V8 post documents what it
  costs in lowering, scheduling and cache behaviour, and Fallin's data shows
  the equivalence-class machinery is not where Cranelift's gain comes from. If
  a later GVN and LICM design is wanted, adopt the aegraph *idea* of scoped
  elaboration over the dominator tree on top of the CFG IR, without e-classes.

### 3. Build SSA during lowering with Braun's algorithm

The lowering from the typed AST should call `def_var`/`use_var`/`seal_block`
style operations on scalar locals that are never address-taken, and use stack
slots for everything else.

- Why: it needs no dominator tree or frontier computation, yields pruned SSA
  immediately, and avoids the 25 percent of instructions Braun measured as
  alloca traffic in a mem2reg-based flow. That matters for the stated goal of
  fast compile time and also for a small IR that never contains a full-function
  non-SSA form.
- It needs a cheap pre-pass per function that marks locals whose address is
  taken (`&x`, array decay, struct locals, `volatile`, and anything read by a
  nested function or statement expression). Sema already walks each body once;
  the mark could be a bit set computed alongside it, or a separate iterative
  walk.
- Sealing: for loops seal the header after the back edge is emitted; for
  `goto` labels, seal at the end of the function (or when sema's label map shows
  all gotos to that label have been lowered). The algorithm is correct for
  irreducible control flow, which `goto` can create; only minimality is lost
  (Braun et al., section 3.2), and the optimizer's trivial-phi cleanup recovers
  most of it.
- Unreachable or uninitialized reads: Cranelift materializes a zero in that
  case; emit `poison` instead, matching C's indeterminate value, and let the
  verifier accept `poison` as a constant.
- Recursion: the paper's `readVariableRecursive` is recursive. Copy Cranelift's
  explicit `calls`/`results` stack formulation, which already satisfies this
  repository's no-native-recursion rule.
- Keep a general stack-slot promotion pass in the optimizer anyway (QBE's
  `promote`, rustc's `non_ssa_locals` analysis are the precedents). It handles
  locals whose address uses disappear after inlining or simplification, and it is
  the path for `restrict`-heavy or struct-typed code.

### 4. Memory: explicit slots, loads and stores in program order, no effect tokens

- Locals that need memory are `stack_slot` entities with size, alignment and an
  optional lifetime marker, addressed by an op that yields a pointer, as in
  CLIF and QBE; a `dyn_alloca` plus `stack_save`/`stack_restore` pair covers
  VLAs.
- Loads, stores, calls and copies are ordered by their position in the block.
  Do not add Mem tokens (libFirm, Click) or a memory SSA to the IR. The
  evidence is that LLVM and GCC both converged on memory SSA as an analysis
  layered over ordinary instructions (LLVM's MemorySSA is explicitly a virtual
  IR; GCC's `.MEM` is derived from alias analysis), V8's effect-chain bugs were
  the first item in its list, and Click's own paper calls the single store
  coarse. Build a MemorySSA-style analysis on demand if load elimination needs
  it.
- Every memory operation carries: access type, alignment, `volatile`, and an
  optional `AccessTag`. An `AccessTag` is a `u32` into a per-module table of
  type descriptors shaped like LLVM's struct-path TBAA, constructed once from
  sema's canonical types: scalar types in a tree rooted at char types, struct
  types with member offsets. The in-house optimizer can use the same tags in its
  alias oracle, and the LLVM printer emits them as `!tbaa`. Read Clang's
  `CodeGenTBAA.cpp` for the cases (may-alias char, unions, `may_alias`) and
  follow its decisions rather than re-deriving them.
- Aggregates are never SSA values. A struct is a stack slot or pointer, copied
  by a `copy` op with size, alignment and a may-overlap flag. Fields are reached
  by pointer arithmetic with sema's byte offsets. This avoids first-class
  aggregate loads and stores, which the LLVM performance guide steers away from,
  and keeps the IR free of a type graph. The one place that needs register-pair
  results is post-ABI calls returning two registers; model those as a call with
  multiple results (Cranelift) or `proj` of a pair (Click, libFirm) and decide in
  the first design review.
- Bit-fields lower in the front end to loads and stores of the containing
  storage unit with shifts and masks, using the offsets and widths sema already
  retains, as cproc and Clang do. CIR's `get_bitfield`/`set_bitfield` ops exist
  to preserve volatile-bit-field rules and for high-level analysis; neither is a
  goal here. Revisit if profiles show LLVM missing combined bit-field updates.
- `setjmp`: record `returns_twice` on callee declarations (by attribute and by the
  names Clang lists). The in-house optimizer should treat such a call as
  clobbering every value live across it, and must not forward stores across it;
  the standard makes non-volatile locals indeterminate after `longjmp`, which
  allows promotion, but anything read after the second return that was not
  modified must still be correct. This is the one rule I could not source from
  an IR document, so write a test from the standard's example before relying
  on it.

### 5. Types, values and UB semantics: LLVM's, minus `undef`

- Types: `i1`, `i8`, `i16`, `i32`, `i64`, (`i128` for GNU `__int128`),
  `f32`, `f64`, target-width `f80`/`f128` where the target has them, `ptr`
  (opaque, no pointee type), and no signedness. Signed or unsigned meaning is
  in the op (`sdiv`/`udiv`, `slt`/`ult`, `sext`/`zext`), exactly as LLVM,
  CLIF, MLIR and QBE. All surveyed IRs agree, and sema's conversion records
  already say which extension applies.
- Flags per instruction: `nsw`, `nuw`, `exact`, `inbounds`, plus fast-math
  disabled by default. Make the flags a small bitfield in the instruction
  record, not metadata, so the optimizer reads them without a lookup.
- Poison and `freeze`: adopt LLVM's semantics (arithmetic with a violated flag
  produces poison; poison propagates; branching on it or using it as a divisor
  or address is UB; `freeze` stops it) and drop `undef`. This follows the
  LangRef's own advice and the PLDI 2017 argument, and it gives the optimizer
  one deferred-UB concept to implement correctly. It also gives a verifier and
  interpreter a precise specification.
- Constant-folding must respect those semantics. An IR interpreter that
  implements poison exactly is the cheapest oracle for testing the optimizer,
  and doubles as the reference for the LLVM-path equivalence tests.

### 6. Data layout: dense entity indices in arena regions

Follow Cranelift and Turboshaft, using this repository's own containers.

- Entities `Block`, `Inst`, `Value`, `StackSlot`, `FuncRef`, `GlobalRef`,
  `AccessTag`, `TypeDesc` are 32-bit newtypes. A `Value` is the index of its
  defining instruction or block parameter, so a single-result instruction needs
  no separate value table.
- Instruction records are fixed-size (target 16 bytes, as Cranelift pins):
  opcode, result type, flags, and either two inline operand indices or one
  index into a pooled operand list for calls, switches and branches with
  arguments. Pin the size with a test in the style of `node_sizes.rs`.
- Storage uses growable-in-place regions (`RegionVec` per function for
  instructions, blocks and operand pools), so indices stay stable and nothing
  needs `Drop`. Side tables are dense arrays keyed by entity (`SecondaryMap`
  equivalent), sized once per pass.
- Order inside a block: Cranelift's doubly linked list in a side array allows
  O(1) insert and remove without moving records. A simpler alternative that suits
  a pass pipeline of rebuilds is to keep each block's instructions in a
  contiguous range and have each pass write its output into a fresh region
  (the V8 and QBE style of whole-function re-emission). Pick one in the first
  prototype and measure; I did not find a source comparing the two.
- No use lists. Cranelift has none and uses value aliases for replace-all-uses;
  QBE recomputes use information with `filluse` after nearly every pass; LLVM's
  per-value use lists are the thing the V8 and Cranelift designs avoid
  maintaining. Recompute use counts as an analysis when a pass needs them.
- Per-function arenas: lower and optimize one function at a time, print or emit
  it, then reset its region. QBE's `freeall` per function follows this; it keeps
  the working set small and leaves room for parallelism later.

### 7. Passes: a fixed pipeline, explicit analyses, verifier everywhere

- No pass-manager framework. Cranelift and QBE both use a fixed list in one
  function, with analyses named and recomputed explicitly (`filldom`, `fillcfg`,
  `filluse`). Start with: stack-slot promotion, CFG simplification, SCCP,
  scoped-hash GVN over the dominator tree, DCE, LICM, then inlining. QBE's
  list shows how small a useful set is.
- All traversals use explicit work stacks and the existing `Work` pattern from
  sema; reverse post-order, dominators and loop discovery are iterative.
- Write the verifier before the second pass. Checks: terminator last in every
  block; block argument counts and types match on every edge; definitions
  dominate uses; flags legal for the opcode; every `inbounds` or `nsw` use on an
  allowed opcode; pre-ABI and post-ABI profile constraints; no critical-edge
  duplication (section 2). Run it between passes in debug and test builds, as
  QBE does with `ssacheck`.
- Add a textual dump and parser with golden snapshots (`BLESS=1` convention),
  an IR interpreter, and differential tests that compile the GCC torture
  execute corpus through both back ends ([corpus note](compiler-test-corpus-research.md)).

### 8. LLVM path

- First implement a textual `.ll` printer. LLVM defines a human-readable form
  ([LangRef](https://llvm.org/docs/LangRef.html)) and [`llc`](https://llvm.org/docs/CommandGuide/llc.html)
  compiles it, so no LLVM library has to be linked into arena-only compiler
  code. (Clang accepting `.ll` input, and whether the pinned build provides
  `llc`, are unchecked.)
- The printer is near-mechanical given sections 2 to 6: block parameters become
  phis, `ptr_add inbounds` becomes `getelementptr inbounds i8`, access tags
  become `!tbaa` nodes, flags map one-to-one, `stack_slot`s become entry-block
  `alloca`s (required for SROA), `returns_twice` and `noalias` become
  attributes.
- Measurement caveat: text printing and LLVM's parse add time that a library
  binding would not. Time each phase separately (our front end, our IR build,
  our optimizer, print, LLVM parse, LLVM optimize, LLVM codegen) so comparisons
  are not biased by the interface. Consider the LLVM C API later, with a narrow
  documented exception to the allocator rule, if the textual overhead becomes
  the story.
- Compare three configurations, not two: LLVM given our unoptimized IR, LLVM
  given our optimized IR, and our full pipeline. Without the first, you cannot
  tell whether our optimizer helps LLVM or merely hurts it.
- Typed versus byte GEPs is the one lowering question I could not settle from
  documentation: the LangRef allows `getelementptr inbounds i8` with byte
  offsets (it appears in its own examples), but how well LLVM's own passes
  treat byte-offset addressing versus element-typed indexing is a measurement,
  not a documented fact. Support both in the printer and compare on one
  benchmark early.

### 9. Build order and things not to do

Build in this order: types, entities, instruction record, dump and verifier
(with the 16-byte size test); scalar AST-to-IR lowering with Braun construction,
interpreter and `.ll` printer; memory (slots, access tags, aggregates,
bit-fields, VLAs); ABI lowering for one target, then varargs; optimizer passes
one at a time, each checked by verifier and interpreter; own backend last.
Avoid, until a measurement asks for it: a sea of nodes, memory tokens, MLIR as a
dependency, a second high-level IR, a signed/unsigned integer type split, and
use lists.

## Open questions

- Multiple results: multi-result values (Cranelift) or a `proj` of a pair
  (Click, libFirm) for post-ABI calls that return two registers.
- Per-block instruction ranges versus linked lists (section 6), and typed versus
  byte-offset address arithmetic (section 8): both are measurements.
- `restrict`: parameter `noalias` only is sound and cheap; block scope needs
  scoped alias metadata.
- GNU features (computed goto as `indirectbr`, statement expressions, nested
  functions, `__int128`) need IR support the standard does not; see
  [`language-standards.md`](../../language-standards.md). Add them after the core
  path works.
- The `returns_twice` rule for the in-house backend needs a test written from
  C99 §7.13 before it is trusted.
- Which LLVM details differ between LLVM 21 (read here) and the pinned 23.1.1;
  the frontend performance guide's new byte type is one to read before
  finalizing the memory model.
