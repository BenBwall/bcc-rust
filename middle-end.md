# Middle end and back end plan

This document plans what comes after semantic analysis: an intermediate
representation (the bcc IR), the lowering from the analyzed syntax tree into it,
an optimizer over it, and back ends that turn it into machine code. It is a plan
and a specification for the first prototype, not a record of finished work;
sections marked **Prototype** describe what the first implementation covers.

The research behind the decisions lives in `docs/research/`:

- [middle-end-ir-designs.md](docs/research/middle-end-ir-designs.md) surveys
  LLVM IR, Cranelift, MLIR and ClangIR, GIMPLE, QBE, libFirm, sea of nodes and
  Rust MIR, and recommends the IR shape below.
- [front-end-lowering-readiness.md](docs/research/front-end-lowering-readiness.md)
  audits what semantic analysis retains for lowering and lists the gaps.
- [llvm-backend-integration.md](docs/research/llvm-backend-integration.md)
  compares ways to drive LLVM and checks them against the pinned toolchain.
- [optimizer-comparison-methodology.md](docs/research/optimizer-comparison-methodology.md)
  designs the experiments that compare our optimizer with LLVM's.

## Goals

1. Compile C to native code through our own optimizer.
2. Compile the same programs through LLVM, so that LLVM's optimizer and ours can
   be compared on the same input with the same code generator.
3. Keep the repository's engineering rules: arena allocation only, explicit work
   stacks instead of native recursion, dense indices, and fast compilation.
4. Keep every stage testable on its own: a textual IR, a verifier, and an IR
   interpreter that serves as the reference semantics.

## Architecture

```text
 C source
   │  translation phases 1-7 (existing front end)
   ▼
 SemanticTranslationUnit<'tu>      typed syntax, bindings, conversions, layouts
   │  lowering (src/lowering)      Braun SSA construction, C rules made explicit
   ▼
 bcc IR, pre-ABI profile           aggregates by value at calls, va_arg as an op
   │  optimizer (src/optimizer)    fixed pipeline, verifier after every pass
   ▼
 bcc IR, pre-ABI profile (optimized)
   │  ABI lowering (src/abi)       per-target classification, sret/byval, va_arg
   ▼
 bcc IR, post-ABI profile          scalars and pointers only
   │
   ├──► LLVM back end (src/backend/llvm)        textual .ll → bundled clang
   ├──► interpreter (src/backend/interpreter)   reference semantics, test oracle
   └──► native back end (later)                 instruction selection, regalloc
```

One IR serves every stage. The analyzed syntax tree is already the C-aware high
level, so no second IR is introduced. The IR has two *verifier profiles* instead:
before ABI lowering, calls may pass and return aggregates by value; after it,
every value is a scalar or a pointer. Both back ends consume the post-ABI form,
so they see identical calling-convention decisions. The optimizer runs before ABI
lowering, where aggregate copies and calls are still visible as such; passes that
need post-ABI form can run after it later.

### Comparison arms

The LLVM back end exists as much for measurement as for code generation. The
arms below differ only in who optimizes; LLVM's code generator is shared
(details in the methodology note):

| Arm | Middle end | LLVM invocation | Question it answers |
| --- | --- | --- | --- |
| A0 | none | `clang -O0 x.ll` | baseline |
| A1 | bcc pipeline | `clang -O2 -Xclang -disable-llvm-passes x.ll` | our optimizer alone, LLVM codegen |
| A2 | none | `clang -O2 x.ll` | LLVM's optimizer alone on our IR |
| A3 | bcc pipeline | `clang -O2 x.ll` | does our optimizer help or hurt LLVM |
| R | clang front end | `clang -O2 x.c` | reference ceiling |

`-Xclang -disable-llvm-passes` keeps the `-O2` code generator but runs no IR
pass; the integration note verified it on the pinned clang. It is a clang
internal option, so the harness checks it on each LLVM upgrade. The LLVM printer
must emit the same facts (`nsw`, `inbounds`, alignment, `noalias`, TBAA) in every
arm, or the arms measure different inputs.

Lowering builds SSA for non-address-taken scalars directly (see below). Clang's
own front end instead emits every local as an `alloca` and lets LLVM's SROA
promote it. To show that the comparison is not skewed by that difference, the
lowering also offers a `memory` mode that keeps every local in a stack slot; A2
can then run on either form.

## The bcc IR

### Entities

All entities are `u32` newtypes indexing dense per-function or per-module
tables:

| Entity | Scope | Meaning |
| --- | --- | --- |
| `Value` | function | an SSA value: an instruction result or a block parameter |
| `Inst` | function | an instruction |
| `Block` | function | a basic block; the entry block's parameters are the function's parameters |
| `StackSlot` | function | a fixed-size, fixed-alignment region of the frame |
| `FuncId` | module | a function, defined or declared |
| `GlobalId` | module | a global object, defined or declared |
| `SigId` | module | an interned call signature |

Instruction records are fixed size; operand lists longer than two (call
arguments, branch arguments, switch cases) live in a per-function value pool and
are referenced by `(start, len)`. A size test pins the record size, in the style
of `parsing/tests/node_sizes.rs`. There are no use lists; passes that need uses
compute them as an analysis.

Each block keeps its instructions in program order in its own arena vector. A
pass that rewrites a function rebuilds block contents rather than splicing a
linked list. The research note leaves this choice (contiguous vs linked list) to
measurement; contiguous order is simpler and suits a fixed pipeline of rebuilds.

### Types

`i1`, `i8`, `i16`, `i32`, `i64`, `i128`, `f32`, `f64`, `f80`, `f128`, `ptr`.
Integers are signless: signedness is part of the operation (`sdiv`/`udiv`,
`slt`/`ult`, `sext`/`zext`). Pointers are opaque. Aggregates are never values;
they live in stack slots or globals and are copied with `copy`. `i1` is the
result of comparisons and the type of branch conditions; C's `_Bool` is an `i8`
in memory.

### Undefined behaviour

The IR follows LLVM's model without `undef`. A `nsw`/`nuw` add, sub, mul or
shl that overflows, an `exact` division that is inexact, and an `inbounds`
pointer offset that leaves its object all produce *poison*. Poison propagates
through arithmetic; branching on poison, dividing by it, or using it as an
address is undefined behaviour. `freeze` turns poison into an arbitrary but
fixed value. Reading an uninitialized local yields `poison`. The interpreter
implements this exactly and is the specification the optimizer is tested
against.

C lowering sets `nsw` on signed `+ - *` and on left shifts of signed operands,
because C99 §6.5p5 makes signed overflow undefined; unsigned arithmetic wraps and
gets no flags. Pointer arithmetic gets `inbounds` (C99 §6.5.6p8).

### Instructions

Arithmetic and logic (operands and result share one type):

```text
iadd isub imul sdiv udiv srem urem          flags: nsw nuw exact
and or xor shl lshr ashr
fadd fsub fmul fdiv frem fneg
icmp <eq|ne|slt|sle|sgt|sge|ult|ule|ugt|uge>  → i1
fcmp <oeq|one|olt|ole|ogt|oge|ord|ueq|une|ult|ule|ugt|uge|uno>  → i1
select cond, a, b
freeze v
```

Conversions: `zext sext trunc fpext fptrunc fptosi fptoui sitofp uitofp ptrtoint
inttoptr bitcast`.

Constants: `iconst <ty> <value>`, `fconst <ty> <bits>`, `poison <ty>`,
`null` (a `ptr` zero).

Memory:

```text
stack_addr  slot                 → ptr
global_addr global               → ptr
func_addr   func                 → ptr
ptr_add     base, offset_i64     → ptr     flags: inbounds
load  <ty> addr                  → ty      flags: volatile, align, access tag
store <ty> value, addr                     flags: volatile, align, access tag
copy  dst, src, size, align                flags: volatile, may_overlap
fill  dst, byte, size, align               (memset)
dyn_alloca size, align           → ptr     (VLAs; later)
```

Calls: `call func(args)` and `call_indirect sig, callee(args)`, each with zero
or one result before ABI lowering (multi-result calls are reserved for post-ABI
register pairs). A call to a variadic signature carries its extra arguments after
the fixed ones. `va_start`, `va_arg`, `va_copy` and `va_end` are operations until
ABI lowering expands `va_arg` for the target.

Terminators end every block and nothing else does:

```text
jump  block(args)
brif  cond, block(args), block(args)
switch value, default block(args), [case_value: block(args), ...]
return [value]
unreachable
```

Two edges from one terminator to the same block must pass the same arguments;
lowering and passes split such an edge with a forwarding block. That keeps the
IR mappable onto LLVM's one-phi-entry-per-predecessor rule, and the verifier
checks it.

### Memory facts

Every memory operation records its access type, alignment and `volatile`.
Strict aliasing becomes an *access tag*: an index into a per-module table of
type descriptors shaped like LLVM's struct-path TBAA, built once from sema's
canonical types (character types alias everything; unions and `may_alias` are
handled as in Clang's `CodeGenTBAA.cpp`). Tags are optional; the prototype emits
none. Function parameters declared `restrict` become `noalias` parameter
attributes. Calls to functions that may return twice (`setjmp` and attributed
functions) are marked `returns_twice`; the optimizer must not keep values in
registers across them.

Bit-fields are lowered in lowering, not in the IR: loads and stores of the
containing storage unit with shifts and masks, from sema's bit offsets and
widths.

### Module

A module holds its functions, its globals, its interned signatures and the
target. A function has a name, a signature, a linkage (`external`, `internal`),
and, if defined, a body. A global has a name, a linkage, size, alignment, a
`constant` flag, and an initializer: zero, or bytes plus relocations
`(offset, symbol, addend)` for address constants.

### Textual form

The printer and parser round-trip a textual form used by `--emit-ir`, golden
tests (`BLESS=1` rewrites them, as for the lexing snapshots) and pass unit tests:

```text
function @add(i32, i32) -> i32 external {
block0(v0: i32, v1: i32):
    v2 = iadd.i32 nsw v0, v1
    return v2
}

function @count(i32) -> i32 external {
    slot0 = stack_slot 4, align 4
block0(v0: i32):
    v1 = iconst.i32 0
    jump block1(v1, v1)
block1(v2: i32, v3: i32):
    v4 = icmp.i32 slt v2, v0
    brif v4, block2, block3
block2:
    v5 = iadd.i32 nsw v3, v2
    v6 = iconst.i32 1
    v7 = iadd.i32 nsw v2, v6
    jump block1(v7, v5)
block3:
    return v3
}
```

### Verifier

The verifier runs after lowering, after every pass in debug and test builds, and
on every parsed test input. It checks:

- every block ends in exactly one terminator, and no terminator appears earlier;
- operand types match each instruction's rules, and flags are legal for the
  opcode;
- each edge passes as many arguments as the target has parameters, with matching
  types, and duplicate edges to one block pass identical arguments;
- every use is dominated by its definition (dominators from the iterative
  Cooper-Harvey-Kennedy algorithm);
- the profile's constraints (no aggregate-typed call operands post-ABI).

## From the front end to the IR (lowering)

### Inputs

Lowering reads `SemanticTranslationUnit<'tu>` and the syntax tree it annotates.
The readiness audit found that most of what lowering needs is retained: a typed
record for every expression (`ExpressionInfo`: type before conversion, value
category, resolved binding, bit-field information, operation type, folded
constant), the conversion records applied to each operand in order, every
binding with storage duration and linkage, and the type graph with target
layouts and member offsets.

### Front-end prerequisites (stage 0)

The audit lists gaps that block lowering. In order:

1. **Expose the semantic types to the crate.** `TypeKind`, `ConversionKind`,
   `ExpressionInfo` and friends sit in private modules; lowering needs
   `pub(crate)` re-exports from `semantic_analysis`.
2. **Gate lowering on errors.** Keep the semantic error count and expose
   `Context::error_count()` (errors only, not warnings). Lowering refuses a unit
   with errors, and treats any `Unknown` type it meets as an internal error,
   because some unmodeled extensions produce `Unknown` without a diagnostic.
3. **Retain the expression index.** Keep the map from an expression node to its
   `ExpressionInfo` and to its run of conversion records, instead of discarding
   it after analysis.
4. **Retain per-function records.** For each function definition: its binding,
   its body, and the bindings of its parameters, so lowering does not match on
   source positions.
5. **Fix the `!` conversion record.** It records a conversion of the operand to
   `int`; it should record no conversion (or a truth-value conversion), since
   `!0.5` is 0.
6. **Record the selected field.** Member access should store the field index
   instead of making lowering search the record by name.

Later gaps (initializer placement plans, static address constants, VLA extents,
builtin result types, bit-field storage units) are scheduled with the features
that need them.

Status: items 1-6 are done; [semantic-analysis.md](semantic-analysis.md)
describes the retained interfaces.

### Algorithm

Lowering walks each function body with an explicit work stack, like semantic
analysis, and builds IR with a function builder:

- **Locals.** A pre-pass marks locals whose address is taken (`&x`, array decay,
  aggregates, `volatile`, `setjmp` in the function). Those get stack slots.
  Every other scalar local is an SSA *variable* handled with Braun et al.'s
  algorithm: `def_var` on assignment, `use_var` on read, and `seal_block` once
  all predecessors of a block are known (loop headers after their back edge;
  label blocks at the end of the function). The recursive `readVariable` of the
  paper becomes Cranelift's explicit-stack formulation.
- **Expressions** produce either a value or a *place* (an address, plus bit-field
  information when the place is a bit-field). The conversion records drive the
  rest: an `Lvalue` conversion is a load (or a variable read), `ArrayDecay` takes
  the place's address, `Arithmetic`, `Assignment` and `DefaultArgument`
  conversions become `sext`/`zext`/`trunc`/float conversions chosen from the
  source and target scalars.
- **Short-circuit and conditional operators** become control flow with a block
  parameter carrying the result.
- **Statements** become blocks: `if`, loops, `break`/`continue` targets on a
  stack, `switch` from sema's sorted case list, `goto` through a label-to-block
  map, `return` with its recorded conversion.
- **Objects with static duration** become globals. Their initializers are
  evaluated by sema's constant evaluator into bytes and relocations.
- **Calls** pass arguments after their default-argument or assignment
  conversions; unprototyped calls use the promoted argument types.

### Prototype

The first lowering covers: integer and pointer arithmetic, comparisons, casts
between integer types, `&& || ?: !`, assignment and compound assignment, `++`
and `--`, local scalars (SSA and address-taken), arrays and structs in stack
slots with member access and subscripts, direct calls, string literals,
`if`/`while`/`do`/`for`/`break`/`continue`/`return`, and file-scope functions.
It rejects anything else with a "not yet supported by lowering" diagnostic
rather than producing wrong code.

## The optimizer

A fixed pipeline in one function, with analyses computed explicitly (as QBE and
Cranelift do) rather than a pass-manager framework. Every pass is checked by the
verifier, and the interpreter checks that a test program's result is unchanged.

The command line exposes `--passes=<list>` for ablation, a global transformation
counter for bisecting a miscompile (`--opt-bisect-limit`), per-pass timing, and
`--print-after-all`.

Planned passes, in pipeline order: stack-slot promotion (for slots that become
promotable after other passes), CFG simplification, sparse conditional constant
propagation, scoped-hash value numbering over the dominator tree, dead code
elimination, loop-invariant code motion, and inlining. **Prototype:** constant
folding, dead code elimination and CFG simplification.

## From the IR to machine code

### Back-end interface

A back end receives a post-ABI module and the target, and produces an artifact:

```rust
pub(crate) trait Backend {
    fn emit(&mut self, module: &Module<'_>, out: &mut dyn Write) -> io::Result<()>;
}
```

### LLVM back end

The integration note recommends staged access to LLVM, and the prototype takes
the first stage:

1. **Textual `.ll` handed to the bundled clang** (prototype). The pinned
   `target/llvm/bin/clang` accepts `.ll` input, honours `-O0`/`-O2` and
   `-disable-llvm-passes`, verifies the IR, and links with the same sysroot and
   `lld` the repository already uses. No LLVM library is linked into the
   compiler, so the arena rules hold. Process start-up costs about 0.4 s and text
   parsing about 7% of a full `-O2` run; the harness times them separately.
2. **`opt`/`llc` in the LLVM distribution build**, when the experiments need
   custom pass lists (`opt -passes=...`) or a strictly IR-pass-free `llc -O0`.
3. **In-process LLVM-C**, only if text overhead dominates measurements. On the
   Windows GNU host this needs a MinGW-ABI build of the LLVM libraries, because
   the cached ones are built with MSVC.

The mapping is mechanical: each module becomes one `.ll` file with the target's
`target triple` and `target datalayout`; block parameters become phis at block
entry with incoming values added once all blocks exist; stack slots become
entry-block `alloca`s; `ptr_add inbounds` becomes `getelementptr inbounds i8`;
flags map one-to-one; access tags become `!tbaa` metadata; `copy` and `fill`
become `llvm.memcpy`/`llvm.memmove`/`llvm.memset`.

### Interpreter

The interpreter executes a module directly: one frame per call with a value
array, a byte-addressed memory with provenance (object, offset) for pointers, and
exact poison tracking. It calls a small set of host functions (`putchar`,
`printf` with integer formats, `malloc`, `free`, `exit`, `abort`) so test
programs can report results. It is the oracle for differential tests: a program
must print the same output when interpreted before optimization, interpreted
after optimization, and compiled through each LLVM arm.

### Native back end (later)

Instruction selection from the post-ABI IR, register allocation (evaluate the
`regalloc2` crate against the arena rules), frame layout and object emission.
It comes after the optimizer is measurably useful through the LLVM path.

## Source layout

New code follows the repository's [source layout](CONTRIBUTING.md#source-layout):

| Module | Entry file holds |
| --- | --- |
| `src/ir.rs`, `src/ir/` | the module and function types; entities, instructions, builder, printer, parser, verifier, dominators in submodules |
| `src/lowering.rs`, `src/lowering/` | the per-function lowering loop over the work stack |
| `src/optimizer.rs`, `src/optimizer/` | the pass pipeline; one file per pass |
| `src/abi.rs`, `src/abi/` | the ABI lowering pass; one file per target family |
| `src/backend.rs`, `src/backend/` | the back-end interface; `llvm.rs`, `interpreter.rs` |

Arenas: the module lives in an `'ir` arena created after semantic analysis; the
syntax tree (`'tu`) outlives it. Per-function scratch (lowering state, pass
analyses) lives in a scratch arena reset between functions.

## Command line

```text
bcc-rust x.c --emit=ir              print the bcc IR after the optimizer
bcc-rust x.c --emit=llvm            print the LLVM IR
bcc-rust x.c --interpret            run main in the interpreter; exit with its status
bcc-rust x.c -o x.exe [--opt=0|bcc] [--llvm-opt=none|O0|O2|O3]
                                    build an executable through clang
bcc-rust x.c --passes=fold,dce,simplify-cfg
```

## Testing

- IR golden tests: C source → IR text, and IR text → pass → IR text.
- Verifier tests with deliberately broken IR.
- Interpreter tests: small C programs with known exit codes or output.
- End-to-end tests: compile through LLVM, run, compare with the interpreter.
- Corpora later: GCC c-torture `execute`, then Csmith/YARPGen differential runs,
  then Embench and Polybench for performance (see the methodology note).

## Roadmap

| Stage | Content | Exit criterion |
| --- | --- | --- |
| 0 | Front-end prerequisites 1-6 | lowering can find every fact it needs in O(1) |
| 1 | IR core: entities, builder, printer, parser, verifier, dominators | golden IR round-trips; verifier rejects broken IR |
| 2 | Interpreter | hand-written IR programs run |
| 3 | Lowering for the prototype subset | C test programs interpret correctly |
| 4 | LLVM text back end and `-o` driver | the same programs compile, run and match the interpreter |
| 5 | Optimizer prototype: fold, DCE, CFG simplification; `--passes` | arms A0-A3 run on a small benchmark set |
| 6 | Floats, switch, globals with initializers, varargs, ABI lowering for x86-64 SysV and Win64 | GCC torture `execute` subset passes |
| 7 | SCCP, GVN, LICM, inlining, slot promotion; comparison harness | first optimizer-vs-LLVM report |
| 8 | Native back end exploration | — |

## Open questions

- Byte-offset `getelementptr i8` versus typed GEPs: measure LLVM's response on
  one benchmark before settling the printer.
- Multi-result values versus a pair projection for post-ABI calls returning two
  registers.
- Whether contiguous per-block instruction storage stays cheap once inlining and
  LICM move code; measure before switching to linked lists.
- The `returns_twice` rule for the in-house optimizer needs a test written from
  C99 §7.13 before it is trusted.
