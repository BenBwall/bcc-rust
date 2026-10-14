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

Stack-slot byte sizes are `u32`, so one slot can hold at most 4,294,967,295
bytes. Aggregate layout sizes remain `u64` in both IR profiles: wider layouts
can be verified and represented by LLVM's array types. ABI lowering currently
refuses aggregate parameters and results above the stack-slot limit on both
System V and Win64, including `sret` results and caller-owned copies, rather
than widening stack slots or allowing target-dependent storage limits. This is
an ABI lowering limit, not a verifier constraint. Alignments are independently
bounded by `Align` to powers of two no greater than 2^31 for LLVM-C.

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
dyn_alloca.i64 size             → ptr     (until restore or return, align 16)
stack_save                     → ptr     (opaque current stack position)
stack_restore position                   (releases later dynamic objects)
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
`noundef` is an independent signature attribute available in both profiles:

```text
function @f(i8 signed noundef, ptr noundef, ... noundef) -> i32 noundef external
```

Passing or returning poison in **any bit or byte of a value** at a `noundef`
position is immediate undefined behaviour, even when the callee ignores the
argument or the caller ignores the result. The interpreter reports the existing
UB trap (CLI status 70). `... noundef` applies this promise to every extra
variadic argument; LLVM emission places it on those call-site arguments, since
LLVM declarations have no attribute position for the ellipsis. Unmarked
positions retain the poison semantics above. An attribute on a memory parameter
(`aggregate`, `result`, `byval` or `sret`) constrains the pointer value only,
not the bytes it addresses. A synthetic `registers` return-area parameter cannot
be marked as an argument because it is not passed; `-> noundef` instead marks
the actual returned registers and checks every byte of every register. Its LLVM
packing does not freeze those bytes. Ordinary unmarked register returns retain
the existing bytewise freeze refinement.

Both profiles carry the attribute so optimization can see C scalar argument
facts before ABI lowering. The verifier requires sorted, unique, in-range
parameter indices, a real returned value for a return attribute (a scalar or
post-ABI registers), and a variadic signature for `... noundef`. It deliberately
does not reject a poison operand statically: reaching the position at runtime
is UB. Signature interning and indirect calls include these contracts.

C lowering follows **bundled Clang 23.1.1 at `-O0 -S -emit-llvm`** on System V
and both Win64 targets. Scalar parameters (including pointers, `_Bool` and
`__int128`) and scalar variadic arguments carry `noundef`. Aggregate register
coercions carry none, including a struct containing a single scalar. `byval`
pointers and Win64 indirect aggregate/`__int128` pointers carry it; hidden
`sret` pointers do not. Pre-ABI C aggregate arguments mark the pointer to the
caller-owned copy; register expansion removes this pointer contract rather
than applying it to the object's potentially poisonous bytes. Explicit IR
contracts on return-area pointers survive when the address becomes `sret`.
Ordinary scalar contracts retain their positions through aggregate expansion.
Win64 `__int128` arguments become pointer-only contracts on caller-owned storage;
this coercion can refine executions already undefined in the pre-ABI form, but
never imposes a defined-byte promise on the pointed-to storage. Explicit scalar
return contracts survive Win64 `__int128` register coercion. **Clang's C return values
have no `noundef` at `-O0` in the tested signatures**, so C lowering leaves
return attributes absent; hand-written IR can mark returns explicitly.
This also preserves C99 §6.9.1p12 (printed p. 142; PDF p. 154): reaching the
closing brace of a non-void function is undefined only if the caller uses the
call's value. An unused result from falling off a non-void function does not
acquire an immediate UB contract merely by crossing the return boundary.
`src/abi/tests/conformance/noundef.rs` compares this policy with live bundled
Clang output, including direct and indirect variadic call sites.

This adopts Clang's practical assumption that evaluating an indeterminate
scalar as a C argument is UB. C99 §6.2.6.1p5 (printed pp. 37-38; PDF pp. 49-50)
makes non-character reads of trap representations UB; §6.3.2.1p2 (printed p. 46;
PDF p. 58) specifies lvalue conversion. Those clauses **do not prove the broader
assumption for every indeterminate non-trapping or character representation**;
the broader rule is an explicit lowering policy, not a claim about all C99
indeterminate values. Aggregate padding remains outside that assumption.

Optimizations may use a marked entry parameter as non-poison: `fold` removes
`freeze` of that parameter. They must preserve defined executions: speculation
must not introduce poison into marked arguments or returns, and a freeze of an
otherwise possibly poisonous value must remain. Inlining can refine executions
already undefined at a removed call boundary, as with other immediate UB.
The differential generator produces parameter and return contracts; the
interpreter oracle discards their UB cases before comparing LLVM arms and
rejects any optimization that introduces a UB trap into a defined execution.

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

`lowering/alignment.rs` owns each backing place's guaranteed alignment, indexed
by its IR address in function scratch. Aggregate operands carry only that address;
converting their C type cannot introduce another alignment fact. Stack/global
addresses obtain their guarantee from the allocation, incoming aggregate and
return-area pointers from the retained ABI descriptor, and aggregate `va_arg`
fetches from their descriptor's backing allocation contract. A parameter whose
declared storage alignment exceeds the incoming guarantee receives a fresh aligned
home and a copy at the weaker guarantee.

`Types::storage_alignment` supplies declared object requirements, including
variable arrays. Lowering owns its fresh slots, call-result areas, snapshots and
dynamic allocations. Constant offsets refine the base guarantee by their common
power-of-two divisor; indexing uses the element stride. Runtime VLA strides
retain the constant element-size factor, rather than an array typedef's storage
alignment. Opaque dereferences use
the retained pointee/member requirement as C's default. Resolved member offsets
and packed guarantees come from semantic analysis. Control joins conservatively
retain byte alignment.

Every load, store, copy and fill crosses `align_memory`, through `inst` or
`memory_op`. Copies use the weaker source/destination guarantee. Bit-field pieces,
zero and string initialization, floating representation operations and overflow
builtin stores use this same boundary. Fixed slots and dynamic allocations are
created through the alignment owner, and static object/literal declarations use
the same retained storage query. Initializer and member consumers no longer
encode backing guarantees by rewriting C types.

C99 6.3.2.3p7, pp. 47-48 (PDF pp. 59-60), 6.7.2.1p12, p. 103 (PDF p. 115),
6.5.16.1p2-3, p. 92 (PDF p. 104); C11 6.7.5p6, p. 127 (PDF p. 145);
GNU [aligned/packed](https://gcc.gnu.org/onlinedocs/gcc/Common-Type-Attributes.html)
and [Cast to Union](https://gcc.gnu.org/onlinedocs/gcc/Cast-to-Union.html);
System V AMD64 ABI 3.2.3/3.5.7 and Microsoft x64 parameter passing.

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

Inline body emission is decided by semantic analysis and consumed by lowering:
C99 inline-only bodies and GNU89 `extern inline` bodies remain external
declarations in IR, so uninlined calls and addresses refer to the external
symbol. Plain GNU89 `inline` and `static inline` bodies are lowered normally.
The interpreter and LLVM therefore share the same decision. See
`language-standards.md` for mode overrides and the per-function `gnu_inline`
attribute. C99 §6.7.4p6-7 (pp. 112-113; PDF pp. 124-125), §6.9p5 (p. 140;
PDF p. 152), and [GCC Inline](https://gcc.gnu.org/onlinedocs/gcc/Inline.html).

### Retained semantic interfaces

Lowering consumes typed expressions and conversions, bindings and definitions,
resolved members/types, function records and the M1 control/lifetime graph.
Each function has an explicit semantic identity/root and parameter aliases.
Each lowerable statement, label and jump must have its retained fact; a missing
record produces a `MissingFact` internal error carrying source provenance.

**M1 is complete.** Semantic analysis produces execution regions and transitions;
lowering maps semantic labels and loop/switch owner IDs to IR blocks and VLA region
IDs to SSA stack tokens. It no longer indexes VLA scopes, reconstructs label/parent
maps, invents pointer/binding region IDs, walks switch bodies to rediscover cases,
or recasts case endpoints. Lexical name scopes and execution regions remain distinct.

**M2 is complete.** Lowering consumes declaration occurrences, parameter facts,
identity-indexed type names, runtime-bound records and the shared size-formula DAG.
The declarator `(name, source_vectors)` index and initializer association walk are
deleted. Semantic inspection no longer joins parameter minima by provenance.
Runtime allocation, sizeof and strides emit the retained formulas, and casts and
size queries schedule their retained site actions. M1 had already removed the
consumer type-name search and bound-discovery type walk; those remain absent.

**M3 is complete.** Lowering consumes bottom-up designation summaries,
`CallFacts`, `StatementExpressionFacts` and `MemberFacts`. Calls use their retained
direct/indirect/builtin classification, point-of-use type/shape and argument
conversion references. Function pointers and conversion-changing wrappers retain
their indirect behavior. GNU blocks schedule retained outer labeled tails and
prefixes; member queries index owning tags/fields and retained access facts.
The linear address-use scan queries designations and calls. Consumer selection
walks, repeated direct-callee checks, tail reruns and owning-record recovery are
deleted; selection/tail helpers are private to semantic analysis. Missing facts
produce provenance-bearing `MissingFact` errors. Open-external ABI policy remains
later migration work.

**M4 is complete.** The phase-neutral `src/builtins.rs` registry owns scalar and
library contracts, exact spelling/ID lookup, signatures, feature/naming policy,
operand evaluation/conversion/constant rules, effects and library equivalence.
`BindingOrigin` preserves user shadowing before intrinsic lookup. Named scalar,
library and varargs calls, plus keyword varargs and offsetof, retain
`BuiltinCallFacts` inside `Callee::Builtin`: descriptor, checked operation,
normalized operands and result/operation types. Lowering schedules those operand
modes and dispatches the checked operation. Scalar name redispatch, the synthetic
varargs `Builtin` AST adapters and varargs storage rediscovery are deleted. ABI
classification and cursor movement remain downstream. Atomic, x86, vector and
type-generic recognition/keyword handlers are explicit temporary M6 bridges;
preprocessing retains its capability adapter until that migration. Scalar/library
support is tested against the registry on every current target and environment.

Initializer placement plans, sparse effect recipes, symbolic constants, remaining
builtin families and final symbol/function policies remain later migration work.
Some consumers therefore still inspect syntax in those families. See
[semantic-analysis.md](semantic-analysis.md) for producer/query contracts and the
[M1 receipt](docs/research/semantic-facts-m1.md),
[M2 receipt](docs/research/semantic-facts-m2.md),
[M3 receipt](docs/research/semantic-facts-m3.md), and
[M4 receipt](docs/research/semantic-facts-m4.md) for checks and corpus comparisons.

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

It plans every interned signature for the target without changing the module,
checks aggregate storage sizes and expanded parameter counts, then interns the
lowered signatures and gives each function its
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

ABI refusal ownership includes indirect-call signatures and aggregate-fetch
descriptors. C lowering retains the emitted function symbol and its semantic
binding's source at the existing function projection, including assembler
aliases. The pipeline carries that association into a refusal diagnostic;
it does not rediscover functions from syntax or C spellings. Raw textual IR
keeps its function owner without inventing a C source location.

The storage preflight covers fixed aggregate parameters of variadic signatures
as well as ordinary parameters, return areas and `byval` copies, whether a call
passes an existing stack slot or needs a temporary. Scalar variadic arguments
and `va_arg` use at most sixteen bytes; aggregate variadic arguments and
aggregate `va_arg` descriptors use the same storage preflight as fixed aggregates. Signatures and their expanded parameter
counts must also fit `u32`, as required by IR indices and LLVM-C.

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

The LLVM back end emits register extension and `noundef` independently and
exactly as the signature records them, on declarations, definitions and both
kinds of call. A C `signed char` parameter on System V becomes
`i8 noundef signext`; its result remains `signext i8`.

**Layout notes:** packed structures reach ABI lowering with their retained
sizes, member offsets and unaligned-member facts. A named bit-field in a small aggregate
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


## GCC scalar intrinsics and GNU block expressions (2026-10-12)

The scalar intrinsic layer shares a name classifier between semantic analysis
and lowering. Fixed signatures retain target-dependent `long` and `size_t`
widths; bswap64 uses the target's uint64_t type (unsigned long on LP64,
unsigned long long on LLP64). Overloaded overflow operands retain their
individual integer types.
The call checker validates counts, pointer destinations, bool/enum exclusion,
read-only destinations, and constant control operands. All ordinary-call GNU
intrinsics use `Feature::GnuBuiltins` and `Context::report_extension`.

`clz`/`ctz` are unary integer instructions with a same-width count result and
poison on zero. `clz_defined`/`ctz_defined` return the operand width on zero;
`popcount` and `bswap` are total unary integer operations (`bswap` requires
16/32/64/128 bits). These map to LLVM `ctlz`, `cttz`, `ctpop`, and `bswap`;
parity, ffs, and redundant-sign counts use ordinary integer operations.
`uadd_overflow` and `umul_overflow` take two same-width integers and return
`i1`, using LLVM's unsigned overflow intrinsics. Generic and typed GCC
add/sub/mul overflow lower through unsigned 128-bit magnitudes and explicit
signs, preserving infinite-precision comparisons even for mixed signedness
and 128-bit extremes. Stores retain the wrapped result on overflow.
Predicate forms evaluate the third operand's effects and use its bit-field
precision without promoting it. The effect-only schedule discards unused
nonvolatile reads even beside calls/increments, retains volatile reads, and
branches only when needed to select effects. Constant predicates are integer constant
expressions when their evaluated operands are integer constant expressions.

`expect` and alignment assumptions return their first argument; probability
and alignment arguments are checked without introducing optimizer promises.
Prefetch evaluates its arguments and emits no memory access. `constant_p`
returns one only for a retained integer constant expression. Both object-size
queries conservatively return `(size_t)-1` in modes 0/1 or zero in modes 2/3;
no bound is inferred and no argument is evaluated. `trap` is a side-effecting
nullary instruction mapping to LLVM `trap`, followed by `unreachable`;
`unreachable` itself ends control flow and is undefined when executed.

`dyn_alloca.i64 size` allocates dynamic stack storage.
It creates a fresh uninitialized byte object of the unsigned size on every
execution, including each loop iteration, with 16-byte alignment (the current
x86-64 targets' ordinary stack alignment). Its result is a pointer and its
storage remains live until a matching stack restore or this invocation returns. The interpreter adds the
object to the current frame's stack allocations and releases it on return;
LLVM emits `alloca i8, i64 size, align 16` at the point of execution. Poison
size is undefined; allocation/resource exhaustion is an interpreter failure.
Zero-size allocations produce an object whose bytes cannot be accessed.
Dynamic allocations are effectful, are neither moved nor merged, and callees
containing them are not inlined, preserving function lifetime. Fixed stack
slots keep their existing representation and size bound. Explicitly aligned
alloca variants remain deferred.

GNU omitted-middle conditionals test a saved first value before converting the
selected result, and evaluate that value once. Case ranges compare unsigned
`control - lower <= upper - lower`, so very wide ranges need constant space;
ordinary switch labels remain switch entries. Statement expressions execute
an explicit-stack block schedule and return the final expression statement's converted
value, including array/function decay; other endings yield void. Constant choose-expr
selections lower only their chosen operand. Scalar header varargs use existing
IR `va_start`, `va_arg`, `va_copy`, and `va_end`; aggregate fetches use
`va_arg_aggregate`.

Contracts: [GCC bit operations](https://gcc.gnu.org/onlinedocs/gcc/Bit-Operation-Builtins.html),
[GCC overflow builtins](https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html),
[GCC other builtins](https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html),
[GCC object sizes](https://gcc.gnu.org/onlinedocs/gcc/Object-Size-Checking.html),
[GCC stack allocation](https://gcc.gnu.org/onlinedocs/gcc/Stack-Allocation.html),
[GCC statement expressions](https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html),
[GCC case ranges](https://gcc.gnu.org/onlinedocs/gcc/Case-Ranges.html), and
[GCC omitted operands](https://gcc.gnu.org/onlinedocs/gcc/Conditionals.html).
C99 §6.5.2.2p2,7, pp. 71-72; PDF pp. 83-84 governs fixed call conversions;
§6.5.15p4, p. 90; PDF p. 102 governs conditional sequencing; §7.15.1,
pp. 249-251; PDF pp. 261-263 governs the header varargs contracts.

### PR #41 round-1 lowering corrections (2026-10-13)

Builtin dispatch and library-symbol aliasing consume semantic binding origin;
user calls retain their declared types and assembler labels. All user `alloca`
declarations use ordinary calls, including compatible unlabeled definitions.
Clang substitutes compatible unlabeled definitions even in strict mode by
default, so that particular native reference uses `-fno-builtin-alloca` to test
the requested ordinary-call contract. Incompatible definitions and assembler
labels compare against Clang's default behavior. Paired intrinsic/pointer
shadow programs cover all scalar, overflow, hint, expression-only varargs and
supported library name families through semantics, interpretation and native
O0/O2 comparisons.

### Scalar overflow execution coverage

Textual IR now executes both `uadd_overflow` and `umul_overflow` at i8, i16,
i32, i64 and i128. Each width has overflowing, adjacent non-overflowing and zero
boundary cases, exercised as both parameterized instructions and constant
instructions before/after the fold pass. LLVM emission must retain each width's
intrinsic in the parameterized function; native O0/O2 executions compare with
Clang-compiled C references. These checks passed without an overflow
implementation change. The IR has no signed overflow predicate opcodes;
separate C probes cover signed add/subtract/multiply destinations at 8, 16 and
32 bits, with literal overflow and wrapped-result expectations, through the
interpreter stages and native Clang comparisons. Contract:
[GCC Integer Overflow Builtins](https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html).

## Aggregate varargs and runtime arrays (2026-10-13)

A call-site signature can retain typed arguments after its ellipsis, for example
`(i32, ..., ptr aggregate(size 16, align 8, [0: i64, 8: f64]))`.
`Signature::variadic_fixed` identifies the fixed prefix; only that prefix enters
LLVM's function type. Function declarations and definitions cannot carry typed
variadic tails. The tail describes argument ABI attributes and classification,
including `byval` on MEMORY arguments. Scalar-only calls keep the usual compact
signature. ABI projection materializes a described tail's inherited `noundef`
promises on the scalar and memory-pointer positions, leaving aggregate register
parts unmarked. Scalars appended beyond that description retain their inherited
promise through an extended projected call-site signature. The printer and parser preserve the ellipsis and every tail attribute.

`va_arg_aggregate list : (ptr aggregate(...))` returns the address of a fresh
copy of the next aggregate. Its signature is a layout descriptor with exactly
one aggregate parameter and no result or ellipsis. It is legal only before ABI
lowering. On System V, INTEGER and SSE eightbytes are reconstructed from the
register save area only when both register classes have room; otherwise the
whole object is fetched from the aligned overflow area. MEMORY aggregates use
the overflow area directly. Empty aggregates do not advance the cursor. Win64
reads aggregates of 1, 2, 4 or 8 bytes from a slot, and otherwise follows the slot's
pointer to a caller copy. The pre-ABI interpreter retains argument layouts; its
post-ABI model independently materializes target save and overflow areas.
Conformance exchanges cover mixed classes, exhausted registers, unions, arrays,
small and large objects, empty objects, and handwritten packed-layout IR.

Semantic array bounds retain an identity and the typed bound expression. Lowering
evaluates bounds on the explicit task stack, saves each extent in an SSA variable,
and uses it for allocation, runtime `sizeof`, and multidimensional strides.
Typedefs retain the declaration's extent. Parameter facts retain declared,
promoted and original types, so adjustment does not discard entry-time actions.
Prototype/body parameter binding pairs are retained by semantic analysis;
function entry visits only those pairs, without scanning unrelated bindings or
matching source positions.
C99 §6.7.5.2p5, pp. 116-117; PDF pp. 128-129; §6.5.3.4p2, p. 80;
PDF p. 92; §6.9.1p10, p. 142; PDF p. 154.

Each automatic VLA declaration saves the stack into an SSA variable keyed by
its semantic region. Lowering consumes retained normal-exit and jump transitions
and follows only cleanup links to emit restores, taking work proportional to the
number of released allocations. A backward goto to before a declaration therefore
reuses storage; a goto within a statement expression preserves enclosing VLA regions.
VM typedef/pointer regions protect entry without emitting stack operations.
Return expressions and aggregate statement-expression tail copies execute before
cleanup. For-init VLA normal exits and breaks meet after their separate restores.
Switch dispatch uses retained converted cases/default/no-match transitions and
resolved label identities. No semantic scope or case fact is reconstructed in IR.
C99 §6.2.4p6, p. 32; PDF p. 44; §6.8p3, p. 131; PDF p. 143;
§6.8.6.1p1, p. 137; PDF p. 149.

Function entry executes retained parameter-site typeof and new-bound actions in
source order after parameter initialization and alias installation. Saved bounds
remain SSA extents. A bound owned by an evaluated typeof operand is scheduled by
that operand's declaration, preventing premature reservation of its SSA variable.
Both ordinary expression lowering and effects-only overflow operands consume the
same normalized type-site actions and semantic dual-context purity summaries.
Allocation, sizeof and pointer/subscript/increment strides consume the shared
semantic size-formula DAG. M5 sparse effect recipes remain a later step.

`stack_save` returns an opaque `ptr` token for the current dynamic stack position;
`stack_restore` takes such a token from the current invocation and releases later
dynamic objects. It leaves fixed slots and varargs backing storage live. These
operations are effectful and cannot be merged or moved across allocations.
A VLA restore also releases newer `__builtin_alloca` storage. This follows
Clang's emitted `llvm.stackrestore` placement and the
[GCC 4.2.1 Variable Length rule](https://gcc.gnu.org/onlinedocs/gcc-4.2.1/gcc/Variable-Length.html).
Reading that storage afterward is undefined in this model, and the interpreter
reports a dangling pointer. A native run returning the stored value does not
establish lifetime: optimization or unreused physical stack bytes can hide the
invalid access. Current GCC retains VLA storage when `alloca` occurs in its
scope ([current manual](https://gcc.gnu.org/onlinedocs/gcc/Variable-Length.html));
that is a different implementation policy. The regression pins bcc's restore
semantics before and after optimization and ABI lowering.

The verifier checks their result and operand types. LLVM emission uses
`llvm.stacksave.p0` and `llvm.stackrestore.p0`; the interpreter validates activation
and position, frees later objects, and reuses their memory. Text round-trip tests
cover both operations and aggregate fetch descriptors; VLA execution tests cover
both optimization settings, native Clang comparisons, and bounded interpreter
memory across repeated allocations and jumps.

Nested loop-header `continue` and `break` also target the enclosing loop, as
specified by the [GCC statement-expression manual](https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html).
Their 1,000-iteration stage matrix checks both results and bounded storage.
Bundled Clang 23.1.1 at O0 on Win64 times out on the nested-header
`continue` source, so native Clang comparisons use an explicit outward `goto`
control. Every bcc arm and the available GCC reference retain the original
source; Clang's control is not evidence that Clang implements the header jump.

`dyn_alloca.i64 size, align N` retains the completed declaration's required
alignment through text, optimization, interpretation and LLVM allocation.
Legacy text without `align` and `__builtin_alloca` retain alignment 16.
VLA declarations retain their element and declaration alignment (C11
§6.7.5p6, p. 127; PDF p. 145). The aligned alloca builtin remains unsupported.
## 2026-10-13: Representation attributes and static constants

Packed member reads, writes and aggregate initializers use their guaranteed
alignment. GNU union casts materialize a temporary union and write the selected
member. Range initializers use explicit Cartesian cursors and save each selected
initializer expression once, including aggregate values. The runtime and static
walkers share the same flattened initializer leaves.

Static file-scope compound literals are memoized as internal globals and filled
from an iterative queue. Constant pointer values emit bytes; addresses of compound
literals, array members, selected GNU/C11 expressions and `&*` forms emit the
existing global relocation representation. Interpreter and LLVM emission consume
the same constants. C99 §6.6p7-9 (printed pp. 95-96; PDF pp. 107-108) and §6.7.8
govern these static forms. Label addresses and differences remain deferred because
they need a code-label relocation model and computed-goto execution.

Floating queries/sign-copy lower through existing scalar operations and byte
accesses. Native x87 support is exercised without changing the interpreter's
existing f80/f128 limitation. Bounded formatting shares the existing interpreter
formatter, writes at most `n-1` characters plus a terminator, avoids the destination
for `n==0`, and returns the untruncated character count. See the dated torture
survey for corpus and focused execution results.
