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
- [optimizer-gap-analysis.md](docs/research/optimizer-gap-analysis.md)
  measures the optimizer gap, introduces control arm N, and prioritizes missing
  transformations.

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

The driver (`pipeline::lower_translation_unit`) follows this order: it lowers,
optimizes when asked, and lowers the ABI for every back end, the interpreter
included. `--emit=ir` prints the pre-ABI module the optimizer worked on and
`--emit=ir-abi` the post-ABI one the back ends receive.

### Comparison arms

The LLVM back end exists as much for measurement as for code generation. The
arms below differ only in who optimizes; LLVM's code generator is shared
(details in the methodology note):

| Arm | Middle end | LLVM invocation | Question it answers |
| --- | --- | --- | --- |
| A0 | none | `default<O0>`, O0 machine | baseline |
| N | none | no IR passes, O2 machine | codegen-only control on unoptimized bcc IR |
| A1 | bcc pipeline | no IR passes, O2 machine | our optimizer alone, LLVM codegen |
| A2 | none | `default<O2>`, O2 machine | LLVM's optimizer alone on our IR |
| A3 | bcc pipeline | `default<O2>`, O2 machine | does our optimizer help or hurt LLVM |
| R | clang front end | `clang -O2 x.c` | reference ceiling |

A1/N isolates what our optimizer adds on top of LLVM's code generator;
N/A2 measures the work left to it. A ratio below 1 means the numerator arm
is faster.

`--llvm-codegen-only` skips `LLVMRunPasses` entirely while the target machine
keeps the selected optimization level. Otherwise LLVM-C runs the new pass
manager's `default<O0|O1|O2|O3>`. All arms share the same module construction,
target machine CPU/features, flags and ABI attributes. Access tags remain
unmapped until the IR has a TBAA descriptor table.

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
or one result in both profiles. A call to a variadic signature carries its extra
arguments after the fixed ones. `va_start`, `va_arg`, `va_copy` and `va_end` are
operations until ABI lowering expands `va_arg` for the target; `va_start`,
`va_copy` and `va_end` stay operations after it.

**Aggregates at calls.** The IR has no aggregate values, so a structure or union
passed or returned by value is a `ptr` parameter that the signature describes
with an aggregate kind and the layout facts conventions classify by: size,
alignment, whether a member is misaligned (`unaligned`) or the aggregate ends in
a flexible array (`flexible`), and, for aggregates of at most 16 bytes, the
scalars it flattens to by offset (lowering records these from sema's type
graph, since the IR has none). Before ABI lowering there are two kinds:

```text
function @swap(ptr result(size 16, align 8, [0: i64, 8: f64]),
               ptr aggregate(size 16, align 8, [0: i64, 8: f64])) external
```

- `aggregate(...)`: the argument is a copy the caller made for this call; the
  callee owns it until it returns, may write it, and nothing else addresses it.
  The callee uses it as the parameter object itself.
- `result(...)`, parameter 0 only, in a signature without a result: the address
  of a fresh object the caller passes, which the callee stores the result into
  before returning.

Aggregates in the variable part of a call are not described and are refused by
lowering. ABI lowering replaces each description by the target's decision:
plain scalars for an aggregate in registers, a plain `ptr` for one passed by
reference (Win64), nothing for an empty one (System V), or one of three
post-ABI kinds:

- `byval(size, align)`: copied to the stack argument area (LLVM `byval`); the
  alignment is the stack slot's.
- `sret(size, align)`, parameter 0: returned in memory (LLVM `sret`).
- `registers(t1[, t2], align a)`, parameter 0: returned in one or two registers.
  The IR keeps storing the result through the parameter, a *return area*, but
  the parameter is not passed: a back end reserves the area in the callee and
  loads the registers from it at `return`, and in the caller stores the
  returned registers to the area the call passes, which ABI lowering makes at
  least as large as the registers. This settles post-ABI register pairs without
  multi-result values.

**Narrow integers at calls.** Conventions differ in whether a `char`, `short`
or `_Bool` argument or result arrives widened to 32 bits, so the signature
records what C knew: before ABI lowering an `i8` or `i16` parameter or result
of such a type carries `signed`, `unsigned` or `bool`, and ABI lowering
replaces it by the target's decision, `signext`, `zeroext` or nothing:

```text
function @f(i8 signed, i16 unsigned, i8 bool) -> i8 signed external      ; pre-ABI
function @f(i8 signext, i16 zeroext, i8 zeroext) -> i8 signext external  ; System V
function @f(i8, i16, i8 zeroext) -> i8 external                          ; Win64
```

The values stay `i8` and `i16` in both profiles; only the LLVM attributes, and a
native back end's extension instructions, depend on the decision.

Extensions describe register bits only: they do not require an argument or
result to be defined. Poison may reach an unused parameter or be returned to
a caller that ignores it, with the same semantics in the interpreter and LLVM.
The IR does not yet model `noundef`, and the LLVM back end emits no `noundef`
parameter, argument or return attributes. Clang's scalar C signatures carry
that additional promise, so optimizer comparisons do not yet have identical
defined-value attributes. Explicit support is deferred until C lowering,
aggregate ABI conversion, interpreter checks and optimizer rewrites can all
preserve the promise; inferring it from `signext` or `zeroext` is unsound.

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

The optimizer's current alias check uses object identity and byte ranges, not
access tags: distinct stack slots and distinct globals are disjoint, as are
non-overlapping constant `ptr_add` ranges from the same base. A slot whose
address never escapes cannot alias an opaque pointer. Escape analysis follows
pointer offsets and conservatively treats stored, call, returned,
integer-converted and unsupported uses (including block arguments) as escapes.
Variable or overflowing offsets retain object identity but give no range proof.
All remaining pairs may alias.

Bit-fields are lowered in lowering, not in the IR: loads and stores of the
bytes that hold the field with shifts and masks, from sema's byte offset, bit
offset and width. A read loads exactly the field's bytes, a write loads (and
`freeze`s) only the bytes it covers in part, and a `volatile` field is accessed
with volatile loads and stores. The accesses stay inside the field's own bytes
because a load of bytes never written is poison as a whole.

### Module

A module holds its functions, its globals, its interned signatures and the
target. A function has a name, a signature, a linkage (`external`, `internal`),
and, if defined, a body. A global has a name, a linkage, size, alignment, a
`constant` flag, and an initializer: zero, or bytes plus relocations
`(offset, symbol, addend)` for address constants.

### Textual form

The printer and parser round-trip a textual form used by `--emit=ir`, golden
tests (`BLESS=1` rewrites them, as for the lexing snapshots) and pass unit tests.

Symbols use `@name` when the nonempty name contains only ASCII letters,
digits, `_`, `.`, `$`, or non-ASCII characters other than controls. UTF-8
identifiers (including decoded C universal character names) remain UTF-8.
Other names use `@"name"`: quotes and backslashes escape as `\"` and `\\`,
and controls as `\u{hex}` Unicode scalars. The parser accepts these same
escapes and rejects invalid scalars and NUL (`\u{0}` or a literal NUL).
Every function and global symbol name must be NUL-free; the verifier enforces
this invariant for programmatically constructed modules too. C identifiers
cannot introduce NUL (C99 §6.4.3p2); lowering also rejects it in asm labels.
Values, blocks and stack slots have
numeric IDs (`v0`, `block0`, `slot0`), not source identifier names.

All alignments are powers of two from 1 through 2^31 bytes. `Align` construction
enforces this common limit for globals, stack slots (LLVM allocas), loads,
stores, memory intrinsics and ABI layouts; the text parser rejects larger
values. LLVM's C++ IR supports 2^32, but `LLVMSetAlignment` takes a 32-bit
unsigned byte count, so 2^31 is the largest representable power of two.

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
- the profile's constraints: no `va_arg` post-ABI; each aggregate kind only in
  its profile, on a `ptr` parameter, results on parameter 0 of a signature
  without a result, and layout scalars within the aggregate; each extension
  only in its profile and on an `i8` or `i16` parameter or result. A post-ABI module
  prints an `abi lowered` line, which the interpreter reads to lay out
  `va_list` for the target.

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
slots with member access and subscripts, direct and indirect calls, structures
and unions passed and returned by value, string literals,
`if`/`while`/`do`/`for`/`break`/`continue`/`return`, and file-scope functions.
It rejects anything else with a "not yet supported by lowering" diagnostic
rather than producing wrong code.

GNU assembler labels select the IR symbol name of functions and static-duration
objects, and propagate across linked declarations. Labels must be nonempty,
NUL-free UTF-8; automatic register labels are unsupported. Lowering checks
calling-convention modifiers on every declaration and type name, including
typedefs and abstract function-pointer types. It accepts the target's default
(`ms_abi` on Windows, `sysv_abi` on Linux) and the conventions bundled Clang
ignores on x86-64 (`stdcall`, `cdecl`, `fastcall`, `thiscall`, `regparm`). Other
conventions, such as the opposite target ABI or `vectorcall`, are refused.

Floating point and bit-fields are lowered as well. `float`, `double` and
`long double` become `f32`, `f64` and, on the x86 GNU targets, `f80` (`long
double` is `f64` on MSVC); arithmetic is `fadd`/`fsub`/`fmul`/`fdiv`/`fneg`,
comparisons are ordered `fcmp`s except `!=`, which is `une`, and conversions are
`sitofp`/`uitofp`/`fptosi`/`fptoui`/`fpext`/`fptrunc` (a value converts to
`_Bool` by comparing with zero). Floating constants and static initializers use
sema's folded values, rounded to the destination format. `__float128` and
`_Complex` types are still rejected.

## The optimizer

A fixed pipeline in one function, with analyses computed explicitly (as QBE and
Cranelift do) rather than a pass-manager framework. Every pass is checked by the
verifier, and the interpreter checks that a test program's result is unchanged.

The command line exposes `--passes=<list>` for ablation, a global transformation
counter for bisecting a miscompile (`--opt-bisect-limit`), per-pass timing, and
`--print-after-all`.

**Prototype:** nine passes: direct-call inlining (`inline`), stack-slot promotion
(`promote`), constant folding (`fold`), CFG simplification (`simplify-cfg`), sparse
conditional constant propagation (`sccp`), dominating-condition integer facts
(`known-bits`), dominator-scoped value numbering (`gvn`), loop-invariant code motion
(`licm`) and dead code elimination (`dce`). When inline is enabled, an explicit-stack
Tarjan walk of the module's original direct-call graph orders callees before
callers and marks recursive SCCs. Each function runs inline once, then the other
passes repeat to a fixpoint (at most eight rounds). Callees therefore finish
cleanup before callers copy them, and newly copied slots are promoted in the
caller. This avoids spending the caller's growth budget again each cleanup round.
An explicit `--passes` list runs in its given order once per function, including
any repeated inline entries; without inline, functions keep declaration order.

Inlining eagerly expands small leaves (at most 30 instructions) and accepts other
defined internal or external callees of at most 100 instructions, including
terminators, after their cleanup. The caller can grow by its original instruction
count or 256 instructions, whichever is larger, with an absolute cap of 2,000.
Every site consumes exactly one `allow()` rewrite, including all block splits,
return joins and copied definitions. Recursive callees, variadic callees and
`va_start` bodies are refused. The IR has no returns-twice attribute yet; textual
IR's recognized `setjmp`, `_setjmp`, `sigsetjmp`, `__sigsetjmp` and `vfork` symbols,
their direct wrappers and bodies taking their addresses are refused too. Lowering
already rejects those builtins. General indirect returns-twice metadata remains
outside the IR's supported contract.

Call expansion splits the caller at the call, maps entry parameters directly to
arguments, copies every other parameter, instruction, constant and stack slot,
and redirects each return to a continuation parameter (no parameter for void).
Address-taken slots remain independent per call site. Global and function symbols,
access tags and signatures retain their module ids; indirect calls retain owned,
remapped operand lists. Inline runs only in the pre-ABI pipeline. Aggregate-by-value
signatures are currently rejected by lowering; the pre-ABI design represents them
as pointers to caller-owned copies, and expansion preserves those pointers without
inventing ABI copies. Definitions are retained even if their direct calls vanish,
so address references and exported symbols remain intact.

The editable draft supports inserting general non-terminators at a position,
rewriting their records without changing result identity, moving them across
blocks, and owning new call operand lists. Operand reads use the current
records and the replacement map; lowering densely renumbers the live results,
blocks and instructions. Added stack slots and foreign wide constants also live
in the draft arena and are densely copied on rebuild.

`simplify-cfg` forms selects for diamond and triangle joins whose arm blocks
have a single incoming edge and at most four cheap computation units per arm.
Constants and addresses cost zero. It refuses loads, stores, calls, division,
remainder and other potentially trapping or expensive instructions. Moved
instructions lose `nsw`, `nuw`, `exact` and `inbounds`, because the original
path's facts no longer guard them. Each differing join argument consumes one
rewrite; a partial bisect run retains the branch and the unconverted arguments.
A select of integer one and zero becomes a zero extension.

`known-bits` follows `sccp` and walks the dominator tree with explicit enter/leave
frames. Scoped known-zero masks and signed ordering facts come from unambiguous
branch edges, with rollback on leaving a subtree; multi-entry block parameters
remain unknown. Masks propagate through constants, bitwise operations, zero
extension, truncation, logical shifts and selects. An ordered `nsw` subtraction
is nonnegative. Positive power-of-two signed division becomes `ashr exact` when
the low bits are zero, or `lshr` when the dividend is nonnegative; nonnegative
signed remainder becomes a mask. Unsigned power-of-two division and remainder
become a logical shift and a mask for any dividend. Each rewrite consumes one
`allow()` call. Later pipeline rounds let these cheap shifts enable select
formation, including the Collatz parity diamond.

`gvn` also eliminates redundant non-volatile loads and forwards non-volatile
stores to loads with the same resolved address and exact type. `memory.rs`
keeps a scoped table of available values and an EarlyCSE-style memory
generation, the length of a write log on the current dominator path. A fact
from an earlier generation survives only when the alias check proves every
intervening store disjoint. Calls (direct and indirect), volatile accesses,
`copy`, `fill` and every `va_*` operation advance the generation with a full
barrier. Each eliminated load consumes one `allow()` rewrite; stores stay.
Facts cross a dominator-tree edge only when the child has exactly one CFG
predecessor, the parent. A join or loop header starts a fresh generation,
even when its paths happen to preserve memory. Scope rollback restores both
overwritten facts and generations before visiting siblings. No loop-carried
load elimination, load motion or type-based aliasing is attempted. Later
pipeline rounds let removed arm loads enable select formation in sorts-style
diamonds.

## ABI lowering

`abi::lower_abi(module, target, scratch)` turns a verified pre-ABI module into a
post-ABI one. The driver calls it after the optimizer and before any back end, for
`--emit=llvm`, `--interpret`, `-o` and `--emit=ir-abi`; only `--emit=ir` stops
before it:

```text
lowering → optimize → lower_abi → emit_module (LLVM) | run (interpreter)
```

It plans every interned signature for the target, gives each function its
lowered signature, and rebuilds each body: the entry block rebuilds an
aggregate that arrived in registers in a fresh slot standing for the old
pointer parameter; each call loads the register parts of its aggregate
arguments from their copies (one load for a lone scalar, else scalar by scalar
with `freeze`, so an unwritten member does not poison its neighbours, and byte
by byte for overlapping union members), makes `byval` copies and return areas
as aligned and large as the target needs, and drops ignored aggregates; each
`va_arg` becomes the target's walk over `va_list` and a load. Unsupported cases
are reported before anything changes. The interpreter runs both profiles; in a
post-ABI module its `va_start` lays the variadic arguments out as the target
passes them.

**System V x86-64** (Linux): eightbyte classification per the psABI §3.2.3, with
Clang's choices where the document leaves room — aggregates over 16 bytes,
with a misaligned member or a flexible array member are MEMORY; INTEGER
eightbytes travel as the narrowest integer covering their scalars (or the lone
scalar's own type), SSE ones as `f32` or `f64`, SSE+SSEUP as `f128`, and a
narrow low part before a narrow high part widens to eight bytes. Arguments get
registers only if all they need of the six general and eight vector registers
are free; an `sret` pointer takes the first. `long double` aggregates go in
memory as arguments and come back in ST0 (`registers(f80)`). `va_arg` follows
psABI §3.5.7 for integers, pointers, `f32`, `f64`, `i128`, `f80` and `f128`.
Every narrow integer argument and result is widened to 32 bits, signed ones with
`signext` and unsigned ones and `_Bool` with `zeroext`: the psABI leaves those
bits unspecified, but Clang widens them and relies on it.

**Win64** (both Windows targets): aggregates of 1, 2, 4 or 8 bytes without a
flexible array member travel as integers of that size and come back in RAX;
any other is passed by reference to the caller's copy and returned through
`sret`. Scalar `__int128` arguments instead use pointers to caller-owned,
16-byte, 16-aligned storage, and scalar `__int128` results come back as
`<2 x i64>` in XMM0, matching bundled Clang on GNU and MSVC targets. The
post-ABI IR models that vector result with `registers(i128, align 16)` and
a memory return area; the interpreter transports the same 128 bits.
`va_arg` reads 8-byte slots for scalars of at most eight bytes; `i128`,
`f80` and `f128` are refused for now. Only `_Bool` arguments and results are
widened (`zeroext`), as Clang does; other narrow integers leave their upper bits
unspecified.

The LLVM back end prints the decision as Clang does: `i8 noundef signext` on a
parameter or argument and `signext i8` before a result.

**Known gaps:** aggregates in the variable part of a call and `va_arg` of
aggregates are unsupported; packed structures do not reach ABI lowering yet,
since the front end does not model them. A named bit-field in a small aggregate
contributes one `i8` scalar per byte its bits reach, so its bytes classify as
INTEGER, as Clang classifies them.

The conformance tests in `src/abi/tests/conformance.rs` link bcc-compiled C with
Clang-compiled C, exchanging every supported shape and the narrow integers in
both directions, and run the result at `-O0` and `-O2`: natively for Windows
GNU and under WSL, as a freestanding static executable, for System V.

## From the IR to machine code

### Back-end interface

A back end receives a post-ABI module and the target, and produces an artifact:

```rust
pub(crate) trait Backend {
    fn emit(&mut self, module: &Module<'_>, out: &mut dyn Write) -> io::Result<()>;
}
```

### LLVM back end

The backend constructs LLVM 23.1.1 modules directly through `llvm-sys` 231.0.1
and the scoped wrappers in `src/backend/llvm/api.rs`. LLVM verifies the module
before optimization; the new pass manager runs the requested default pipeline,
then the target machine emits an object (or assembly) into an LLVM-owned memory
buffer. `--emit=llvm` prints the constructed module using
`LLVMPrintModuleToString`. The handwritten printer and `.ll` compile process
are gone. Final executable linking still uses bundled clang/LLD.

Each target retains its triple and data layout. Block parameters become phis;
all blocks and phis exist before instructions, and incoming edges are attached
last, including duplicate terminator edges. Stack slots become entry allocas,
`ptr_add` becomes byte GEP with its inbounds flag, arithmetic flags survive
construction, and memory/varargs operations use overloaded LLVM intrinsics.
Calls and declarations share byval, sret, alignment and narrow integer
extension attributes. Register aggregate returns retain their return areas.
Each return part is loaded as a byte vector and frozen lane by lane before
bitcasting to its ABI type, preserving defined bytes beside poison bytes.
Paired returns assemble these parts independently at the shared byte offsets.
Globals with relocations use packed structures with exact byte offsets.

Windows GNU needs a separate GNU-ABI static library build in `target/llvm-libs`:
the shared MSVC-built `target/llvm` remains untouched. Bundled clang/clang++
use the located MinGW sysroot and its static C++/pthread runtime. The build
script queries the installed `llvm-config` for the link closure. Binding-only
llvm-sys features avoid Cargo's dependency/bootstrap ordering problem. Linux
source tool distributions include libraries and llvm-config, and compatible
prebuilt installations can supply both directly.

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
bcc-rust x.c --emit=ir              print the bcc IR (after the optimizer, if it runs)
bcc-rust x.c --emit=ir-abi          print the bcc IR after ABI lowering
bcc-rust x.c --emit=llvm            print the LLVM IR
bcc-rust x.c --interpret            run main in the interpreter; exit with its status
bcc-rust x.c -o x.exe [-c]          emit an object through LLVM-C; link with clang
    [--opt=none|bcc]                the bcc optimizer; none is the default
    [--passes=fold,dce,simplify-cfg] an explicit pass list, run once (implies bcc)
    [--opt-bisect-limit=N]          apply only the first N rewrites (implies bcc)
    [--print-after-all]             each function after every pass, to stderr
    [--llvm-opt=none|O0|O1|O2|O3]   LLVM pipeline and machine level; none means O0
    [--llvm-codegen-only]           skip LLVM IR passes; retain machine level
bcc-rust --print-default-passes     print the default pass list and exit
```

The back-end modes exclude one another and the inspection views
(`--tokens`, `--syntax-tree`, `--raw-syntax`, `--semantic-types`); the other
options require a back-end mode. `--emit` and `--interpret` print to stdout and
the diagnostics, `--print-after-all` and interpreter traps to stderr. Each arm
is one invocation:

| Arm | Flags |
| --- | --- |
| A0 | `--llvm-opt=O0` |
| N | `--llvm-opt=O2 --llvm-codegen-only` |
| A1 | `--opt=bcc --llvm-opt=O2 --llvm-codegen-only` |
| A2 | `--llvm-opt=O2` |
| A3 | `--opt=bcc --llvm-opt=O2` |

Exit statuses: 0 on success; 1 after an error, a construct lowering or ABI
lowering does not support yet, or a clang failure; for `--interpret`, the status `main` returns
or `exit` receives, 70 when the interpreter stops the program (undefined
behaviour, an unsupported operation, a limit), and 134 on `abort`. The
interpreter's C library uses the `--target`'s type widths (`%ld` reads 32 bits
on Windows). `-o` links with the C library of `--target`, so it needs the host's
triple to produce a runnable program; `main` is called without arguments.

## Testing

- IR golden tests: C source → IR text, and IR text → pass → IR text.
- Verifier tests with deliberately broken IR.
- Interpreter tests: small C programs with known exit codes or output.
- End-to-end tests: compile through LLVM, run, compare with the interpreter.
  `src/pipeline/tests/middle_end.rs` runs every program of
  `src/lowering/tests/programs.rs` in the interpreter with and without the bcc
  pipeline, and compiles eight of them (two printing with `printf`) in arms
  A0-A3 for the host, running them side by side and comparing status and
  output with the interpreter. `tests/code_generation_cli.rs` covers the
  command line.
  Native test programs run with captured stdout/stderr and a 20-second
  deadline; `BCC_TEST_TIMEOUT_MS` overrides it with a positive millisecond
  count. On timeout the shared runner kills the process tree (Windows) or
  process group (Unix), reaps the direct child and reports its command and
  captured output. Pipe readers get a 250-millisecond grace period, then are
  detached if an escaped descendant still holds a pipe open. The same deadline
  covers pipes left open after the direct child exits. Differential failures
  also include the seed, arm,
  rerun command and both bcc and LLVM IR. Object-only output needs no MinGW
  discovery; the CLI regression removes `MINGW_ROOT` and clears `PATH`.
- `benches/c` holds ten self-checking performance programs; their reduced
  workloads (`-DBENCH_REDUCED`) run in the interpreter, with and without the
  bcc optimizer, in `tests/bench_corpus.rs`.
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

Stage 7 status: promotion, SCCP, GVN, LICM and bottom-up inlining exist
(the prototype pipeline above). The comparison harness exists:
`scripts/compare_optimizers.py` runs arms A0-A3, N and R, with `--passes` ablation,
over the ten self-checking programs in `benches/c` and writes JSON Lines plus a
summary of per-benchmark medians, ratios to A2, a separate A1/N comparison,
and their geometric means with a bootstrap interval (README, "Optimizer
comparison harness"). What remains for the exit criterion is a run on native
Linux with CPU isolation, a larger corpus (Embench, Polybench) and the report
itself. Windows inliner measurements and validation are recorded in
`docs/research/inline-measurements.md`.

## Open questions

- Byte-offset `getelementptr i8` versus typed GEPs: measure LLVM's response on
  one benchmark before introducing typed GEP construction.
- Multi-result values versus a pair projection for post-ABI calls returning two
  registers: ABI lowering avoids both for now with return areas
  (`registers(...)`); revisit when a native back end wants the registers as
  values.
- Whether contiguous per-block instruction storage stays cheap once inlining and
  LICM move code; measure before switching to linked lists.
- The `returns_twice` rule for the in-house optimizer needs a test written from
  C99 §7.13 before it is trusted.
