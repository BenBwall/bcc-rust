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
- [optimizer-performance-2026-10-14.md](docs/research/optimizer-performance-2026-10-14.md)
  adds seven benchmarks, measures `sroa`, `combine`, branch threading and
  join-aware GVN against LLVM, and ranks the transformations still missing.

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
   └──► native back end (src/backend/native)    x86-64 assembly → bundled clang,
                                                or objects encoded in process
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
| B0 | none | native back end | our code generator alone |
| B1 | bcc pipeline | native back end | our optimizer with our code generator |
| R | clang front end | `clang -O2 x.c` | reference ceiling |

A1/N isolates what our optimizer adds on top of LLVM's code generator;
N/A2 measures the work left to it. B1/A1 compares the two code generators on
the same optimized IR, and B1/B0 measures our optimizer with a code generator
that does not clean up after it. A ratio below 1 means the numerator arm is
faster.

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
| `BlockLabel` | function | an address-taken block: a stable name `block_addr` and relocations use while passes renumber blocks |
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
block_addr  label                → ptr     (GNU label value)
ptr_add     base, offset_i64     → ptr     flags: inbounds
load  <ty> addr                  → ty      flags: volatile, align, access tag
store <ty> value, addr                     flags: volatile, align, access tag
copy  dst, src, size, align                flags: volatile, may_overlap
fill  dst, byte, size, align               (memset)
dyn_alloca.i64 size             → ptr     (until restore or return, align 16)
stack_save                     → ptr     (opaque current stack position)
stack_restore position                   (releases later dynamic objects)
sjlj_setjmp buf                → i32     (GCC __builtin_setjmp's resume point)
sjlj_longjmp buf                         (__builtin_longjmp; then unreachable)
object_size.i64 [dynamic] ptr, mode → i64 (GCC object size, mode 0-3)
subobject   ptr, size_i64        → ptr     (ptr, naming a subobject)
```

**Object sizes.** `object_size` is GCC's `__builtin_object_size (ptr,
mode)` (`__builtin_dynamic_object_size` with `dynamic`), asked of an IR
pointer: the bytes from `ptr` to the end of its object, the most (modes 0,
1) or the fewest (2, 3), of the whole object (0, 2) or of the closest
surrounding subobject (1, 3). Like LLVM's `llvm.objectsize`, it is answered
after inlining: semantic analysis answers a query whose C operand names its
object, and lowering emits `object_size` for the others in a function,
which evaluates the operand. GCC never evaluates it, so only an operand
whose evaluation reads named variables and computes addresses is deferred;
one that has side effects, reads through a pointer (which may fault), reads
an atomic or volatile object, calls a function other than a nested query,
or has a variably modified type anywhere (a cast to `char (*)[*n]` reads
`*n`) gets the unknown answer at once.
The rest of a deferred operand is dead once the query is answered and
`dce` removes it. A deferred query is not an integer constant expression,
as in Clang.
`subobject` is its pointer operand; lowering passes each pointer argument of
a call to an `always_inline` function (an argument pack copy is one) through
it when the argument names a subobject with a constant number of bytes left
(`__builtin_object_size (arg, 1)` and `(arg, 3)` agree in the caller). The
IR's addresses do not record subobjects, so this is how a callee's mode 1
or 3 query of its parameter finds `s.name` once the call is expanded. The
hint closest to the query decides, and it holds only across pointer
arithmetic: an argument that is the calling wrapper's own parameter, offset
by pointer arithmetic and casts alone, never assigned, incremented or
decremented, and whose address `&` never takes, gets no hint (its
caller's still names the subobject), and every other argument gets a
`subobject ptr, -1`, "not known", so that no hint from further out answers
across a member access (`char *q = &p->a[1]`) that narrowed the subobject.
Likewise a mode 1 or 3 query is emitted only in such a wrapper and of such a
parameter expression; any other gets the unknown answer at once. GCC tracks
the member there and answers more often; bcc never answers more.

`optimizer::resolve_object_sizes` (`optimizer/object_size.rs`) answers every
query before ABI lowering, for every back end and at every optimization
level, after the always-inline expansion (`resolve_inline_copies`, or for
the LLVM back end `expand_object_size_wrappers`, which expands the calls of
`always_inline` functions that ask, directly or through another such
function, and keeps the copies LLVM needs). It walks the pointer back
through `ptr_add`, `subobject`, `select` and block parameters (at most 64
values) to stack slots, defined non-weak globals, `dyn_alloca` and, in a
hosted unit (not `-ffreestanding`, where a program may define them as
anything, as GCC and Clang assume), calls of the library's `malloc`,
`calloc`, `realloc` and `aligned_alloc`, and takes
the most or fewest bytes left over the objects reached; modes 1 and 3 also
need a `subobject` on the path and answer the fewer of its bytes and the
object's. Offsets must be constant: queries whose offset another query
decides wait while `fold` makes it constant (`p + ((B(p) + 1) & 1)`).
A dynamic query on one path computes a runtime answer from a runtime size or
offset; a dynamic query whose alternatives differ is unknown, since it
answers for the object reached at run time (GCC selects among them at run
time; bcc does not yet); any other query left gets the answer for an unknown object,
`(size_t) -1` for modes 0 and 1 and 0 for modes 2 and 3. Positions past the
end leave 0 bytes, before the start 0 (mode 1 before its subobject: unknown),
so no answer exceeds the bytes the pointer has left, and a checked call
never accepts a write the object cannot hold. The `subobject`s are then
removed and `fold`, `simplify-cfg` and `dce` clean up the tests of the
answers. Back ends that meet an unanswered query in hand-written IR give
the unknown answer (and `subobject` is its pointer): LLVM's `llvm.objectsize`
would answer it after LLVM's inlining, but could disagree with the
interpreter and cannot tell a member from its enclosing object for modes 1
and 3. Not answered (the unknown answer, as before): pointers loaded from
memory or passed to functions that are not expanded, offsets that stay
runtime values in non-dynamic queries, and modes 1 and 3 of a pointer that
is not a wrapper parameter's arithmetic (`char *p = s.name; strcpy (p,
...)`, or a member address formed in a wrapper). Heap buffers are the
common case of the last: a `malloc`ed pointer held in a variable names no
subobject at the call, so glibc's mode 1 checks (`strcpy`, `snprintf` at
`_FORTIFY_SOURCE=2`) get no bound for it, though mode 0 ones (`memcpy`,
`read` in GCC mode) do; Clang-mode wrappers that take `pass_object_size`
(`read`) are answered by semantic analysis at the call, which sees no
allocation.
GNU extension: [GCC Object Size Checking](https://gcc.gnu.org/onlinedocs/gcc/Object-Size-Checking.html).

Atomic memory operations carry their operation, success and failure orders,
alignment and volatile flag in one 16-byte instruction record. Operands live in
the function value pool. Scalar load/store/exchange/compare-exchange use integer
object bits; arithmetic RMW operations also support `f32`/`f64`. Separate byte
buffer operations cover arbitrary-sized objects. The verifier checks operand
shapes, eligible types and LLVM-compatible memory-order combinations. Consume
is conservatively acquire; dynamic orders are conservatively sequentially
consistent. Fences and every atomic operation are optimizer memory barriers,
including when unused or encountered on paths joining the current block.

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
(`aggregate`, `result`, `byval`, `indirect` or `sret`) normally constrains
the pointer value only. An incoming `aggregate`, `byval` or `indirect`
`complex` argument descriptor instead represents a scalar: its `noundef` also
covers each meaningful real/imaginary component byte, excluding padding. The
interpreter checks these bytes before entering the callee. Result-area pointer
attributes constrain only the address. Complex memory descriptors retain their
two leaves after ABI lowering;
Win64 uses `indirect(...)` to describe an ordinary pointer to the caller's copy
without an LLVM `byval` attribute. A synthetic `registers` return-area parameter cannot
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

Inline assembly (no result; see [GNU inline assembly](#gnu-inline-assembly-2026-10-26)):

```text
asm [sideeffect] "template" (out|in "constraint" <ty> [bytes N] value, ...) [clobber(name, ...)]
```

Terminators end every block and nothing else does:

```text
jump  block(args)
brif  cond, block(args), block(args)
switch value, default block(args), [case_value: block(args), ...]
indirect_br addr, [block(args), ...]
return [value]
unreachable
```

`indirect_br` continues at the listed edge whose target block's label has
the address `addr`; see [Labels as values and computed goto](#labels-as-values-and-computed-goto-2026-10-14).

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
attributes. Functions that may return twice (`setjmp`, `vfork` and attributed
functions) carry the IR flag `returns_twice`; the LLVM back end marks their
declarations and direct calls, the inliner keeps their callers' frames, and the
native back end keeps values live across their calls out of coalescing (see
"Returns-twice calls" below).

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

A module holds its functions, its globals, its interned signatures, its
constructor and destructor list and the target. A function has a name, a
signature, a linkage (`external`, `internal`, `weak`), an optional code
alignment, symbol attributes and, if defined, a body. A global has a name, a
linkage, size, alignment, a `constant` flag, a `thread_local` flag, symbol
attributes and an initializer: zero, or bytes plus relocations
`(offset, target, addend)` for address constants. A target is a function or
global (eight bytes), a label of a defined function (eight bytes,
`blockaddress(@f, label0)`), or a label difference truncated to `i8`-`i64`
(`labeldiff.i32(@f, label1, label0)`). A `weak` definition may be replaced by
a strong one at link time; a `weak` declaration may stay undefined with a null
address. Symbol attributes are an alias target (another definition of the
same kind, not itself an alias; the alias has no body or initializer), a
section, `used` retention and `hidden`/`protected` visibility. Constructors
and destructors are parameterless defined functions with a 16-bit priority,
kept in registration order. See [GNU entity attributes](#gnu-entity-attributes-2026-10-14).

File-scope GNU basic assembly (`__asm__("...")` outside a function) is kept as
module assembly: one text block per `asm`, in source order. It prints as
`module asm "text"` lines after the target lines, with backslashes, quotes
(`\u{22}`) and controls such as newlines (`\u{a}`) escaped so each block stays
on one line. Lowering reports qualifiers, operands and wide or Unicode
templates as invalid, in Clang's words, and refuses a template with NUL or
bytes that are not UTF-8 as unsupported, each at its syntax. The LLVM back end appends each block to the module's inline
assembly; the native listing writes the blocks verbatim after its first
`.text`, before every function and object, as Clang's output places them, then
`.att_syntax`. Direct object encoding reads only the emitter's own
vocabulary, so the native back end assembles such a module with the bundled
clang even where direct objects are its default (Windows), and
`--native-object` refuses it. The interpreter refuses to run it, since the
assembly may define code or data it cannot see.

A `thread_local` global (`global @g external thread_local size 4, align 4`)
has one copy per thread, initialized from its initializer when the thread
starts; `global_addr` yields the evaluating thread's copy. Its address is
therefore no link-time constant, and the verifier rejects a relocation that
names it (C11 §6.2.4p4, §6.6p9). Lowering gives every object with semantic
analysis's `Duration::Thread` such a global, file-scope or static local
alike. The interpreter runs one thread and has no thread-creating host
operation, so its thread-local globals are ordinary globals of that thread.
The LLVM back end marks them `thread_local` and computes each access with
`llvm.threadlocal.address`, as Clang does. The ELF access model follows the
relocation model and `-ftls-model`, as Clang's (`target/code_options.rs`).
In an executable (the default position-independent one, `-fpie`/`-fPIE`, or
`-fno-pic`) Clang marks definitions `dso_local`, which gives them the
local-exec model; LLVM-C cannot set `dso_local`, so definitions (weak ones and
aliases included) request local-exec directly, and declarations keep the
default that becomes initial-exec. Code for a shared library (`-fpic`/`-fPIC`)
requests general-dynamic, which LLVM refines to local-dynamic for internal and
hidden objects, so a library bcc builds can be linked at startup or loaded
with `dlopen`. `-ftls-model` names the least specific model allowed; a more
specific implied model wins, as in LLVM. Windows targets use native TLS
through `_tls_index` and the TEB, as Clang does for both MinGW and MSVC; the
relocation flags do not change their code. The native back end generates
executable code only: it uses the same executable models (local-exec for
declarations too with `-ftls-model=local-exec`) and refuses `-fpic`,
`-fPIC` and `-shared`.

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
- labels name distinct blocks other than the entry; `block_addr` names an
  existing label; an `indirect_br` lists distinct labeled blocks; a label
  relocation names a label of a defined function;
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
symbol. An `always_inline` one (without inline assembly) is also lowered as
an internal `name.always_inline` function, marked `alwaysinline`, that
direct calls name; addresses keep the external symbol. The copy records the
external function it stands for (`inline_copy_of @name`): the LLVM back end
emits its body as that function's `available_externally` definition and
points the copy's uses at it, as Clang emits the inline definition, so
LLVM's always-inliner makes Clang's choices (a recursive body is not inline
viable, and its calls reach the external definition). bcc's optimizer
expands the copy where it can; for the interpreter and the native back end,
`optimizer/always_inline.rs` then repeats the always-inliner's walk (module
order, with a copy in its external function's place, and inline viability
rechecked as each expansion can make a body call itself) and points every
call that still names a copy at the external function, so every back end
reaches the same definition: a recursive `__forceinline` function calls the
external definition at O0 on all of them (#109). The interpreter links
nothing else, so such a call stops it with an undefined-function trap, as
the program alone fails to link with Clang; it stops plain inline-only
calls the same way. A plain inline-only body (without inline assembly,
`noinline`, or a call of its own symbol through an assembler label or a
builtin, Clang's `isTriviallyRecursive`) is lowered too, as a copy named
`name.inline` that nothing calls: LLVM emits it as the external function's
`available_externally` body, so LLVM's inliner can expand it at O1 and above
as Clang's does and drops it at O0, while the other back ends drop it.
Lowering refuses nothing for these bodies: if the unit does not lower with
them it is lowered again without. A GNU `weak` inline definition is a weak
definition, emitted where it is referenced, as Clang gives it weak linkage
before inline-only emission. A function whose body uses GNU
`__builtin_va_arg_pack ()` or `__builtin_va_arg_pack_len ()` (glibc's GCC
fortify wrappers) is not lowered once: each direct call calls an internal
`alwaysinline` copy, `name.va_arg_pack`, specialized for the call's promoted
anonymous argument types, which it takes as extra parameters. In the copy the
pack expands to those parameters at a call whose last argument it is, the
length is an `iconst`, and `fold`, `simplify-cfg` and `dce` run on it right
after lowering, at every optimization level, so a guarded call of an `error`
function disappears as GCC folds it when it inlines. Copies are lowered on
demand by a liveness walk once every definition is lowered
(`lowering/argument_pack.rs`): it starts at externally visible, `used`,
alias-target, inline-copy, constructor and initializer-addressed functions
and follows calls and addresses through lowered (and, for copies, folded)
bodies, relaxing each copy's nesting depth to the shallowest live path at a
fixed point, so a copy that only a folded branch or an unreached internal
helper requests stays a bodiless declaration that nothing names, the
64-copy nesting limit counts only the shallowest live path, and internal
bodies left unreached are dropped, as GCC drops them at -O2. The function's
own body is lowered when it is an external definition, or internal and
`used` or with an address live code takes; in any of these a pack it
evaluates is an error, as in GCC. A copy's object-size queries of its
parameters are IR `object_size` operations, answered once the copy is
expanded into its caller ([Object sizes](#instructions)), so glibc's
`snprintf` wrapper checks the caller's buffer as GCC's does after inlining;
the same holds for every other `always_inline` wrapper (`memcpy`, `strcpy`,
`read`) in GCC and Clang mode. Known deviations from GCC, which finds the pack after constant folding:
bcc sees through parentheses, `__extension__`, casts and unary `+` that keep
the type, generic selections, and `*`, `&` or a same-type cast on the
callee, but not arithmetic (`0 + pack`) or a comma before the pack; and the
liveness walk runs at every optimization level, so dead code GCC keeps at
-O0 (and then fails to link or diagnoses) is dropped. Functions carry
their inline control in IR (`noinline` for GNU `noinline`/`noipa`,
`alwaysinline`), which the inliner and the LLVM back end honor. Plain GNU89 `inline` and `static inline` bodies are lowered normally.
The interpreter and LLVM therefore share the same decision. Internal functions
and objects are planned in `lowering/module_items/reachability.rs`: external
definitions seed an explicit worklist over retained semantic entity references,
as do constructors, destructors, aliases and their targets, and `used`
entities, which Clang emits without references.
Calls, function addresses and object initializer references reach internal
definitions transitively. Unreferenced static objects do not seed the worklist;
their initializers therefore do not force function emission. Unreachable
function signatures and bodies are not lowered, so unsupported vector or
assembly implementations in system headers do not refuse otherwise supported
programs. Unevaluated `sizeof`, `alignof` and `typeof` references are excluded
by semantic analysis; VM typeof operands retain their runtime references.
See
`language-standards.md` for mode overrides and the per-function `gnu_inline`
attribute. C99 §6.7.4p6-7 (pp. 112-113; PDF pp. 124-125), §6.9p5 (p. 140;
PDF p. 152), and [GCC Inline](https://gcc.gnu.org/onlinedocs/gcc/Inline.html).

Creation order follows definitions already emitted because of `used`, a
constructor or destructor even if a later declaration makes an inline body
external. Plain C99 inline-only bodies are not created at O0 merely because
`used` is written. A block-scope static initializer contributes only values
whose bits survive later initializer replacements, in final element order; retained references carry
their containing initializer-value identity for this query. Its enclosing
function is indexed once per scope, so nested block statics do not repeatedly
walk their ancestry. GNU range replications copy the source value identities
before subsequent replacements; a uniform contiguous range stays one interval,
and other ranges visit no more elements than static emission itself.
These rules affect creation order, independently of ordinary reachability and
of the stacked local-static elision work.

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
classification and cursor movement remain downstream. Atomic builtins now use
the neutral registry and retain object types and memory-order operands for
execution. Scalar/library support is tested against the registry on every
current target and environment.

**M7 is complete.** Lowering queries finalized entity records for effective
symbol requests, known definitions/open calls, inline-body permission,
returns-twice and `FunctionEnd`. It retains IR symbol collision checks,
reachability and ABI projection. Host signature validation shares neutral
builtin contracts and continues to reject malformed textual IR independently.
See the [M7 receipt](docs/research/semantic-facts-m7.md).

**M6 is complete.** The x86, vector and type-generic families are registry
descriptors too, and `__has_builtin` queries the registry. Their operations are
`Semantic` (folded or selected by semantic analysis) or `Unsupported`; neither
retains a checked instance, so `schedule_builtin` lets the call reach the same
refusal as before (an unsupported builtin, or an opaque binding's unanalyzed
type), never an external symbol. Lowering's
keyword scheduling still matches the keyword of a varargs, offsetof or
`choose_expr` form.

**M8 is complete.** Automatic and static initializers store by the
`InitializerPlan` semantic analysis retained for each accepted initializer
(`sema.initializers.plan`). `initializers/plan.rs` replays its operations as
source-ordered leaves at absolute offsets: a range `Repeat` runs once per
element with an explicit cursor, a changed union `Select` becomes the reset
zero leaf `CoverageIndex` expects, and a whole `Copy` forgets the selections
inside it. Automatic emission keeps the coverage-indexed zero gaps and
evaluates each ranged `InitValueId` once, remembering its converted value;
static emission maps the same leaves to bytes and relocations. The second
current-object machine (`Walker`, `leaves`, its field-name designator search,
braced-scalar and whole-copy classification) and the expression-address
counting that chose which range values to remember are deleted. Lowering
refuses a plan operation whose destination it cannot represent (a vector,
binary128, an unmodeled type, an empty scalar brace pair) with the refusal
the walker gave.

A range's later elements copy its first (issue #82) when the first
element's leaves select no union member and write one run of whole bytes:
every later element would get the same bytes and relocations from them.
Replay then ends with one `Leaf::Replicate` instead of a leaf per element,
so `{ [0 ... 999][0 ... 999] = 0 }` is one store and two replications.
Static emission copies the run's bytes and relocations into the buffer,
doubling adjacent copies, and an all-zero object still becomes `zero`. An
automatic range of at least 16 elements whose run is the whole element and
is not volatile becomes one `fill` of zero, or `copy` instructions that
double the initialized elements (at most 64); shorter or volatile automatic
ranges, ranges that select a union member, a whole copy that would forget a
later element's selection, and partial-byte runs are still replayed element
by element. Copied ranges keep exactly the bytes, relocations and effects
of element stores; `lowering/tests/ranges.rs` compares both forms and
counts the replay work of 2^20-element ranges.

**M5 is complete.** Effects-only evaluation (an overflow predicate's third operand
and the statements of a GNU block inside it) interprets retained
`EffectRecipe`s: nothing, evaluate-and-discard, the node's type-site actions and
child uses, a single branch on a condition with optional arms, or the
statement-expression body in effects mode. `builtin_effects.rs` no longer
inspects operators, volatility, ICE state or selections, and never consults a
purity predicate. A branch with one empty arm uses one effects block and the merge
block, the same shape short circuits already had; an omitted conditional, or a
conditional with one effect-free arm, now takes that shape too. A missing summary
or recipe is a provenance-bearing `MissingFact`.

Symbolic constants and final symbol/function policies remain later migration
work.
Some consumers therefore still inspect syntax in those families. See
[semantic-analysis.md](semantic-analysis.md) for producer/query contracts and the
[M1 receipt](docs/research/semantic-facts-m1.md),
[M2 receipt](docs/research/semantic-facts-m2.md),
[M3 receipt](docs/research/semantic-facts-m3.md),
[M4 receipt](docs/research/semantic-facts-m4.md),
[M5 receipt](docs/research/semantic-facts-m5.md),
[M6 receipt](docs/research/semantic-facts-m6.md) and
[M8 receipt](docs/research/semantic-facts-m8.md) for checks and corpus comparisons.

### Atomic execution

`_Atomic` objects use memory storage. Ordinary reads and writes are sequentially
consistent atomic operations; compound assignments and increments retry a
compare-exchange against the observed value, evaluating the lvalue and right
operand once. `<stdatomic.h>`, the modeled `__c11_atomic_*`, `__atomic_*` and
`__sync_*` spellings share the builtin registry. Compare-exchange updates the
expected buffer only on failure; weak operations may use a strong instruction.
Pointer fetch offsets follow each family: C11 scales by the element size, while
GNU and legacy sync offsets are bytes. Signed fetch arithmetic wraps.

LLVM emits real atomic instructions and fences. Generic buffers and large
objects call the platform `libatomic` ABI; executable linking adds `-latomic`
for buffer, 128-bit or under-aligned atomic operations. Objects using those paths
require the same runtime at final link. Windows native execution of these paths
requires a compatible libatomic installation; interpreter coverage of the
Windows data model does not verify that runtime. The interpreter executes each
operation as one indivisible step in its single-threaded model, including
x87 `f80` operands; LLVM supports the x87 object representation through padded
memory.
The native back end selects the standard C11-to-x86-64 mapping: `mov` for
loads and relaxed or release stores, `xchg` for sequentially consistent
stores and exchanges, `lock xadd`, `lock cmpxchg` and `lock` arithmetic, a
`lock cmpxchg` loop for the other read-modify-writes, and `mfence` only for
sequentially consistent thread fences. Like LLVM without `cmpxchg16b`, it
calls libatomic for `i128` and byte-buffer operations, and its driver then
links `-latomic`. It refuses floating minimum and maximum, `i128` minimum and
maximum, and accesses aligned below their size (`isel/atomic.rs`;
`docs/design/native-backend.md`, "Atomic operations").
Evidence (2026-10-26): GCC 15.2.0's `gcc.dg/atomic` run tests, through
`scripts/run_gcc_torture_execute.py` for Linux, pass 34 of 48 on both
native arms, up from 3, as many as LLVM at O0; the rest fail on every arm
(front end, compile-only tests without `main`, atomic complex values). The
native end-to-end program `atomics` matches the interpreter and LLVM, and
`tests/fixtures/programs/atomic_threads.c` keeps its counters exact with 1,
4 and 8 threads under both back ends. The counters drift once the `lock`
prefixes are removed from the listing.

### Algorithm

Lowering walks each function body with an explicit work stack, like semantic
analysis, and builds IR with a function builder:

- **Locals.** A pre-pass marks locals whose address is taken (`&x`, array decay,
  aggregates, `volatile`). Those get stack slots. A `setjmp` (or GCC's
  `__builtin_setjmp`) in the function changes nothing here: C99 §7.13.2.1p3 makes a non-volatile local changed
  between `setjmp` and `longjmp` indeterminate, so it may stay an SSA value.
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
NUL-free UTF-8; automatic register labels are unsupported. Semantic completion
retains decoded requests and their inheritance; lowering checks IR symbol
collisions and reports retained capability refusals. Semantic analysis resolves
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
sema's folded values, rounded to the destination format. Real and complex
binary128 (`__float128`, `_Float128`) lower to `f128` with the same
instructions; a folded binary128 constant becomes `fconst.f128`, and complex
products and quotients call `__multc3` and `__divtc3`.

Complex scalars use two immutable SSA component values retained in a sparse
arena table; taking an lvalue's value immediately loads both components.
GNU `__real__` and `__imag__` preserve component lvalues, packed alignment and
volatile access. Conversions, comparisons, truth, assignments, conditional
values, initializers and direct/indirect calls operate on both components.
The usual arithmetic conversions preserve real/complex domains while finding
the common real precision, so adding a real value preserves the imaginary
component's signed zero. Complex multiplication and division use the target's
`__mul[sdx]c3` and `__div[sdx]c3` runtime helpers; constant folding uses a native
C bridge in the selected component precision rather than a denominator-square
formula.

Call boundaries materialize fresh complex objects and retain a top-level scalar
ABI flag. This differs from an ordinary record of two real members: System V
complex `long double` returns two x87 values at offsets 0 and 16 in a 32-byte
return area, while a same-layout record uses a hidden result pointer.
`ResultRegisters` records that second offset explicitly. Complex meaningful
bytes are never frozen during argument packing; padding remains unspecified.
Register parameters carry scalar `noundef` contracts. Memory parameters retain
the complex descriptor and its meaningful-byte definedness contract. Complex
register results retain the `complex` marker, suppressing aggregate byte-freeze
refinement while still allowing unused indeterminate returns without an
immediate return `noundef` contract.

The default interpreter host bridges F32/F64 arithmetic helpers through real
scalar inputs and host output temporaries and implements `crealf`/`creal` and
`cimagf`/`cimag` extraction, including target ABI coercions. LLVM supports x87
complex arithmetic and transport; the interpreter extracts x87 parts with
`creall`/`cimagl` but refuses the `__mulxc3`/`__divxc3` and binary128
`__multc3`/`__divtc3` helpers, and the native
backend retains its existing explicit F80 arithmetic limitation. This does not implement the
complete complex math library or GNU complex conjugation
and increment/decrement extensions. Atomic complex object operations retain an
explicit unsupported diagnostic.

## The optimizer

A fixed pipeline in one function, with analyses computed explicitly (as QBE and
Cranelift do) rather than a pass-manager framework. Every pass is checked by the
verifier, and the interpreter checks that a test program's result is unchanged.

The command line exposes `--passes=<list>` for ablation, a global transformation
counter for bisecting a miscompile (`--opt-bisect-limit`), per-pass timing, and
`--print-after-all`.

**Prototype:** eleven passes: direct-call inlining (`inline`), scalar replacement
of aggregates (`sroa`), stack-slot promotion (`promote`), constant folding
(`fold`), peephole combining (`combine`), CFG simplification (`simplify-cfg`), sparse
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
return joins and copied definitions. Recursive callees, variadic callees,
`va_start` bodies and bodies with labels or indirect branches are refused. Functions
flagged `returns_twice` (lowered from the retained semantic fact, or projected
by the textual parser from the external-symbol catalog), their direct wrappers
and bodies taking their addresses are refused too. A call through a function
pointer carries no returns-twice mark, as in Clang.

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

`sroa` splits a stack slot accessed only by loads and stores at constant
offsets, and by `copy` and `fill` of constant ranges, into one slot per field,
which `promote` then turns into SSA values. A copy between two slots spreads
the fields inside its range to the other side; overlapping fields, a variable
offset or any other use of the address keep the slots whole. A field moves as
a value, which is poison as a whole where one of its bytes is, so a slot that
lies on a copy path from other memory to other memory is *retained*: it keeps
its memory and every write, so the copy out still moves exact bytes, and its
loads read promoted shadow fields. Bytes of a slot that reaches a copy out but
that no access covers become integer fields, so fills and copies carry them.

`combine` rewrites short integer chains: booleans and their zero extensions
compared with 0 or 1, negated comparisons, branch conditions and select
conditions, boolean selects of constants, two constant bounds of one value
joined by `and`, `or` or the lowered `&&`/`||` selects (one unsigned
comparison of `x - lo`), and nested additions of constants.

`simplify-cfg` threads an edge that passes a constant to an empty block
branching on that parameter straight to the chosen successor, unless the block
heads a loop or blocks below it use its parameters, and merges a branch with a
single-entry arm that only tests a second cheap condition for one of the
branch's destinations into one branch on a `select` (LLVM's
`FoldBranchToCommonDest`). It also forms selects for diamond and triangle joins whose arm blocks
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
barrier. One classifier, `memory_write`, decides what an instruction writes for
both numbering and the region scan below, so a new writing instruction is added
in one place. Each eliminated load consumes one `allow()` rewrite; stores stay.
Facts cross a dominator-tree edge as they are when the child has exactly one CFG
predecessor, the parent. Entering a join or loop header, GVN logs the writes of
every block on a path from its immediate dominator (the loop body, for a
header), so a fact survives when no such write may alias it; a region of more
than 64 blocks is a full barrier instead. Scope rollback restores both
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
bits unspecified, but Clang widens them and relies on it. A GNU
`transparent_union` argument is classified as its first member, as GCC and
Clang do: its `aggregate(...)` descriptor keeps the union's size and alignment
and adds `first(size N[, extension])` with the member's scalars, so it takes the
member's registers (a narrow integer member with its extension) and, once
they are taken, goes on the stack, and through `va_arg`'s overflow area, as the
whole union. Lowering also gives an array variable of at least 16 bytes, or of
variable length, 16-byte storage on the GNU targets unless its declaration
specifies an alignment (psABI §3.1.2), without changing the C type's
alignment.

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

bcc classifies aggregates that contain a `__float128` as the psABI says,
as GCC does, in every compiler identity and position (named parameters,
results, variadic and unprototyped arguments, `va_arg`). Clang deviates for
an aggregate with a `__float128` at offset 0 and a member of class SSE over
its second eightbyte: it classifies the eightbytes as SSE and SSE (or SSE
and INTEGER) but its IR type for the aggregate starts with `fp128`, which
`GetX86_64ByValArgumentPair` widens to `i64`, so it passes the low eightbyte
in a general register while counting a vector one, and its own `va_arg`
reads both eightbytes from the vector save area. So code built by Clang
does not link with GCC's or bcc's for these shapes (`Q` is `__float128`),
checked against bundled Clang 23.1.1 and GCC 15:

| Union | Clang | psABI, GCC and bcc |
| --- | --- | --- |
| `{ Q q; double d[2]; }`, and with the members swapped | `i64`, `double` | `double`, `double` |
| `{ Q q; float f[4]; }` | `i64`, `double` | `double`, `double` |
| `{ Q q; _Complex double z; }` | `i64`, `double` | `double`, `double` |
| `{ Q q; struct { double a; long b; } s; }` | `i64`, `i64` | `double`, `i64` |

A structure wrapping one of these passes like it. Clang follows the psABI
for `{ Q q; double d; }` and `{ Q q; Q r; }` (one vector register), for
unions whose low eightbyte is INTEGER anyway (`{ Q q; long l; }`, `int`,
`char`, `long l[2]`, `struct { long a; double b; }`), for a lone
`__float128` and `struct { Q q; }`, and passes `{ Q q; long double ld; }` in
memory, so those link with Clang. `tests/integration/binary128.rs` links
bcc's halves with GCC's in both directions and with Clang's for the shapes
it follows, and `tests/fixtures/programs/binary128_varargs.c` checks every
later variadic argument after such unions.

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
`printf`, `malloc`, `free`, `exit`, `abort`) so test programs can report
results. It is the oracle for differential tests: a program
must print the same output when interpreted before optimization, interpreted
after optimization, and compiled through each LLVM arm.

The default host also implements the C99 real `<math.h>` function families in
float, double and long double spellings, including pointer outputs, integer
results and NaN constructors. Native libm evaluates float and double in their
own precision; guest pointers stay in interpreter memory. Long double follows
the target representation: binary64 on MSVC, and x87 `f80` on the GNU targets.

`f128` values compute in `src/binary128.rs`, IEEE binary128 arithmetic in
software that rounds once to nearest even and propagates NaNs as libgcc's
soft-float routines do on x86-64, so interpreted programs print what
programs linked with libgcc print. The host provides glibc's `strtof128`
and `strfromf128` (with `printf`'s exact digits, since glibc's `printf`
has no binary128 conversion) and the exact `_Float128` math functions
(`sqrtf128`, `fabsf128`, `floorf128`, `fmodf128`, `frexpf128`, …), and
the complex helpers `__multc3` and `__divtc3` as libgcc computes them
(`host/complex/quad.rs`: Annex G's recovery of infinities, the scaling
of a divisor near the range's ends, and the operand order GCC 13 and 15
compile libgcc's sums and products in, which decides a NaN result's sign
and payload); the other `_Float128` functions are refused.

x87 values compute in `src/x87.rs`, a software model of the processor's x87
unit under the default control word (64-bit precision, round to nearest even,
exceptions masked): exact integer arithmetic rounded once, pseudo-denormals
as valid operands, unnormals and pseudo-NaNs as invalid operands that give the
negative "real indefinite" NaN, and the processor's NaN propagation. A test
compares it with the host's x87 unit on random encodings of every class. The
host computes the `long double` functions as glibc's x86-64 versions do,
bit for bit: the exact ones (rounding to integers, `sqrtl`, `fmodl`,
`remainderl`, glibc's `remquol` algorithm, scaling, decomposition,
neighbours, `nanl`) and the ports of glibc's C code (`sinl`, `cosl` and
`tanl` with their argument reduction, `asinl`, the hyperbolic functions,
`erfl`, `erfcl`, `cbrtl`, `hypotl`, and `fmal` correctly rounded) on the
x87 model. The exponentials, logarithms, `powl` and the arctangents follow
glibc's assembly language, whose `f2xm1`, `fyl2x`, `fyl2xp1` and `fpatan`
results come from the processor's microcode: those four instructions run
on the host's own x87 unit, so the results match glibc on the same
processor (they may differ between processor makers, as glibc's do), and a
host without an x87 unit refuses those functions. The return values of
glibc's SVID wrappers apply (`NAN` for `asinl`, `acosl` and the logarithms
of out-of-domain arguments). `lgammal` and `tgammal` are refused.
`src/backend/interpreter/host/math/extended/tests.rs` compares every
function with the test binary's glibc on random encodings (skipping, on a
glibc before 2.43, the `powl` cases its commit 1b657c53c21a changed), and
`tests/fixtures/programs/long_double_math.c` with a program built against
glibc; `long_double_modern_glibc.c` pins the `powl` cases (non-canonical
bases to small integer powers) on which glibc 2.39 and 2.43 differ. `printf` formats every
conversion of `double` and `long double` exactly, digit for digit as glibc
does, with halfway cases to even. The floating-environment APIs remain
outside the host implementation.

`errno` is one `int` in the program's memory, which glibc's `<errno.h>`
reaches through `__errno_location` (`host/errno.rs`). The host functions set
it where glibc's do: allocation failures (`ENOMEM`, `realloc` included),
`strtol` and its C23 `__isoc23_` forms (`ERANGE`, `EINVAL` for a bad base),
`strtod`, `strtof`, `strtold` and `atof` (the host's C library converts and
reports `ERANGE`), `scanf`'s integer conversions, which clamp as `strtol`
does, every stream and file failure (`EACCES` for a refused access, `EBADF`,
`EINVAL`, `EMFILE`, `EILSEQ`, and the host system's codes for reads, writes,
seeks and opens), and the math functions under glibc's `math_errhandling`
(`MATH_ERRNO | MATH_ERREXCEPT`): the host's libm reports its own `errno` for
`float` and `double`, with the positive `NAN` glibc's dynamic SVID wrappers
return for `acos`, `asin`, `log10` and `tgamma` domain errors, and
`host/math/extended_errno.rs` applies glibc's wrapper rules to the x87
functions, and `host/quad.rs` those of glibc's `_Float128` wrappers to the
binary128 ones (`strtof128` reporting `ERANGE` for an overflow or, as
glibc's `round_and_return` decides with x86's tininess after rounding, a
tiny inexact value). `strerror`, the GNU `strerror_r`, the XSI `__xpg_strerror_r` that
glibc's headers select without `_GNU_SOURCE`, and `perror` give glibc's
Linux messages, "Unknown error N" for other codes.

A normal end runs the `atexit` and `on_exit` handlers in reverse order of
registration, with the destructors where glibc's dynamic linker registered
its own handler (after every handler `main` or a constructor registered),
calls a handler registered meanwhile next (an `atexit` handler a destructor
registers runs from crtbegin's `__cxa_finalize`, between the destructors
without a priority and the prioritized ones), continues the list with the
new status when a handler or destructor calls `exit`, and then writes out
the streams (`exit.rs`). The handlers run on top of the frames that called
`exit`, whose automatic objects stay live; only a return from `main` ends
its frame first. `quick_exit` runs the
`at_quick_exit` handlers and `_Exit` and `_exit` nothing; both lose what the
streams hold, `stdout` included, as glibc does. A handler must be a function
the program defines, of its registration's type, checked when it is
registered. `abort` is unchanged: no handlers, no write-out, and the command
line still shows all of `stdout`.

The host's C library also covers streams and files: `stdin`, `stdout` and
`stderr` are objects holding pointers to stream objects the program cannot
access, and a stream used after `fclose` (or a failed `freopen`) traps, even
when `printf` or `getchar` uses it implicitly. Every stream, the captured
`stdout` and `stderr` included, keeps glibc's buffer state
(`host/stdio/stream.rs`, `host/stdio/console.rs`): a buffer of the
descriptor's `st_blksize` (4096 for a pipe), written out when a byte finds it
full, when a block does not fit (whole buffers' worth going straight
through), after each newline when line-buffered, at once when unbuffered,
and at `fflush`, seeks, reads, `fclose` and a normal end; `printf` writes
through glibc's 128-byte staging area when the put area has no room.
`setvbuf`, `setbuf`, `setbuffer` and `setlinebuf` set the mode and the
buffer size, with glibc's mode constants (or the Windows runtimes' on
Windows targets). The command line replays `stdout` and `stderr` in the
order a shared pipe or file would have received them. Host files are
opt-in: with `--interpret-allow-fs`, `fopen` (including C11's exclusive
`x`), `freopen` and `remove` act on the host's file system; without it they
fail as glibc does for a permission error (`EACCES`), the command line notes
the refusal, and `tmpnam` names files without looking at the file system.
It provides the `fprintf`, `vprintf`, `vfprintf`, `vsprintf` and
`vsnprintf` family and glibc's `__printf_chk` forms, with `%lc` and `%ls`;
`sscanf`/`fscanf`/`scanf` with glibc's failure rules, the wide conversions
and the C23 `__isoc23_` forms' `%b` and `0b` prefixes (not `long double`
input); the wide streams (`host/stdio/wide.rs`): `fwide`, `fputwc`,
`fputws`, `fgetwc`, `fgetws`, `ungetwc`, the `wprintf` family with
`swprintf`, and the `wscanf` family through the narrow engines, with a
byte function on a wide-oriented stream (or the reverse) trapping as
undefined; `setlocale` for glibc's "C" and "C.UTF-8" locales
(`host/locale.rs`; the empty name selects "C", as an environment without
locale variables does), whose `LC_CTYPE` decides how wide streams and
`%lc`/`%ls` convert: in "C" a wide stream writes `?` for a character beyond
ASCII and reads such a byte as `EILSEQ`, and in "C.UTF-8" both are UTF-8; a
stream keeps the conversion of the locale it was oriented in. A wide
stream has glibc's wide put area, as many wide characters as the byte
buffer holds bytes (a quarter as many in a `setvbuf` buffer), whose
write-outs convert and write all it holds, and a wide pushback area that
`ungetwc` fills with the characters themselves. The `wscanf` family reads
whole characters, matches scansets by character and converts what `%c`,
`%s` and `%[` store with the current locale (`EILSEQ` when it cannot); a
`printf` or `wprintf` that meets a character it cannot convert keeps what
it wrote into the put area and discards its 128-unit staging area. The
`wscanf` family's field widths count the UTF-8 bytes of a character beyond
ASCII. It also provides string search and
concatenation, `strtol` and its relatives, the "C" locale's character classes
including the tables glibc's `<ctype.h>` reads through `__ctype_b_loc`, the
wide-string basics at the target's `wchar_t` width, and `conj`, `cproj`,
`cabs` and `carg` (which set no `errno`). `qsort` and `bsearch` run in the machine: each comparison
pushes the comparison function's frame and steps until it returns, so its
instructions count against the step limit and a trap inside it names it.
`qsort` follows glibc's stable merge sort call for call, including its
indirect sort through pointers for elements over 32 bytes.

The interpreter runs one translation unit and links nothing else. A call
that reaches the external definition of an inline-only function, because
the always-inliner walk did not expand it (#109) or the function is plain
`inline`, stops with an undefined-function trap, matching the link failure
of that unit alone, rather than running the inline definition, which no
linked program would run there.

### Native back end

bcc's own x86-64 code generator is designed in
[docs/design/native-backend.md](docs/design/native-backend.md): a
macro-expansion instruction selector over the post-ABI IR, a machine IR over
virtual registers, linear-scan register allocation with fixed-register
ranges, frame layout with CFI and SEH unwind directives for System V and Win64,
and AT&T assembly, which the bundled clang assembles, or ELF and COFF
objects it encodes itself; the bundled clang links executables. It compiles
integer, pointer, `i128`, `f32` and `f64` code, `f80` on the x87 unit,
`f128` through libgcc's soft-float routines (`isel/quad.rs`), variadic
definitions, dynamic stack allocation, computed `goto`, thread-local globals,
calls that may return twice, C11 atomics and the GNU symbol attributes, and
refuses everything else before writing any output.
`--backend=native` selects it for `-o`, `-c` and `-S`, and `--emit=asm` with
it prints its assembly.
For `-o` it encodes COFF objects directly on the Windows targets and has the
bundled clang assemble its listing on Linux; `--native-object` and
`--no-native-object` choose either path on any target, and the last of the
two wins. By default a listing the encoder refuses is assembled.

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
    [--interpret-step-limit=N]      instruction limit; default 100000000, 0 unlimited
    [--interpret-allow-fs]          let the program open, create and remove host files
bcc-rust x.c -o x.exe [-c]          emit an object through LLVM-C; link with clang
    [-O0|-O1|-O2|-O3|-Os|-Oz|-Og] GCC optimization level; bare -O means -O1
    [--opt=none|bcc]                the bcc optimizer; none is the default
    [--passes=fold,dce,simplify-cfg] an explicit pass list, run once (implies bcc)
    [--opt-bisect-limit=N]          apply only the first N rewrites (implies bcc)
    [--print-after-all]             each function after every pass, to stderr
    [--time-passes]                 per-pass runs, rewrites and times, to stderr
    [--llvm-opt=none|O0|O1|O2|O3|Os|Oz|Og]   LLVM pipeline and machine level; none means O0
    [--llvm-codegen-only]           skip LLVM IR passes; retain machine level
    [--backend=llvm|native]         code generator for -o; native refuses what it lacks
    [--native-object|--no-native-object]  native objects encoded in process (the
                                    Windows default) or assembled by clang (Linux)
    [-fpic|-fPIC|-fpie|-fPIE|-fno-pic]  relocation model; Linux defaults to -fPIE
    [-ftls-model=MODEL]             least specific ELF TLS model: global-dynamic,
                                    local-dynamic, initial-exec or local-exec
    [-shared|-pie|-no-pie]          link a shared library (needs -fpic or -fPIC),
                                    a position-independent executable (the Linux
                                    default) or a fixed-address one
bcc-rust x.c --emit=asm            print LLVM's assembly (the native back
                                    end's with --backend=native)
bcc-rust -c x.c y.c                 compile each source to an object: x.o, y.o
bcc-rust -S x.c [-o x.s|-o -]       write the assembly to x.s, or stdout
bcc-rust x.c y.o -lm -o app         compile x.c to a temporary object and link
                                    it with y.o and -lm, in order
bcc-rust --print-default-passes     print the default pass list and exit
```

The driver runs one job per input in command-line order (`src/cli/driver.rs`):
`-S` and `-c` stop each C source at assembly or an object, named after the
source in the working directory unless `-o` names the only output; `-o`
alone links, after the driver has checked that the output is none of the
inputs. One C source alone links in one step as before; with several
inputs, or with `--sysroot`, each C source becomes a temporary object beside
the output and
assembly inputs are assembled by the bundled clang, which then links every
object, archive, library and `-l`, `-L`, `-Wl,`, `-Xlinker`, `-static`,
`-rdynamic` or `-pthread` option in order, adding `-latomic` when an object
needs it, against the `--sysroot` when one is given. No link runs after a
failed job. README lists the preprocessing
stages (`-E`, `-M`).

`-O0` disables optimization; the other `-O` levels enable bcc's pass
pipeline and select the corresponding LLVM pipeline (`-Og` uses `O1`).
The native back end uses the bcc pipeline. The last `-O` wins. Explicit
`--opt` and `--llvm-opt` override their respective settings from `-O`,
regardless of order. These selections also work during preprocessing and
semantic inspection: `__OPTIMIZE__` is 1 when either optimizer is enabled,
and `__OPTIMIZE_SIZE__` is 1 for an effective LLVM `Os` or `Oz` selection.
Both macros are ordinary predefined macros, so `-D` and `-U` can override
them. Explicit pass controls continue to imply the bcc optimizer.

The back-end modes exclude one another and the inspection views
(`--tokens`, `--syntax-tree`, `--raw-syntax`, `--semantic-types`); the other
options require a back-end mode, except `-O`, `--opt` and `--llvm-opt`. `--emit` and `--interpret` print to stdout and
the diagnostics, `--print-after-all` and interpreter traps to stderr.
`--emit=ir`, `--emit=ir-abi` and `--interpret` verify the module first, as the
LLVM back end does for `--emit=llvm` and `-o`, and report an internal error
instead of printing or running invalid IR. Each arm is one invocation:

| Arm | Flags |
| --- | --- |
| A0 | `--llvm-opt=O0` |
| N | `--llvm-opt=O2 --llvm-codegen-only` |
| A1 | `--opt=bcc --llvm-opt=O2 --llvm-codegen-only` |
| A2 | `--llvm-opt=O2` |
| A3 | `--opt=bcc --llvm-opt=O2` |
| B0 | `--backend=native` |
| B1 | `--opt=bcc --backend=native` |

Exit statuses: 0 on success; 1 after an error, a construct lowering or ABI
lowering does not support yet, or a clang failure; for `--interpret`, the status `main` returns
or `exit` receives, 70 when the interpreter stops the program (undefined
behaviour, an unsupported operation, a limit), and 134 on `abort`. The
interpreter's C library uses the `--target`'s type widths (`%ld` reads 32 bits
on Windows). `-o` links with the C library of `--target`, so it needs the host's
triple to produce a runnable program; `main` is called without arguments.
The relocation flags follow Clang: the last of `-fpic`, `-fPIC`, `-fpie`,
`-fPIE` and the `-fno-` spellings wins, defines `__PIC__`/`__pic__` (and
`__PIE__`/`__pie__` for an executable) with its level, and is ignored on
Windows GNU, where Clang forces position-independent code; the MSVC target
rejects the position-independent ones, as Clang does. As with Clang,
`-shared` does not imply `-fPIC`: linking executable code into a shared
library is left to the linker to reject.

## Testing

- IR golden tests: C source → IR text, and IR text → pass → IR text.
- Verifier tests with deliberately broken IR.
- Interpreter tests: small C programs with known exit codes or output.
- End-to-end tests: compile through LLVM, run, compare with the interpreter.
  `src/pipeline/tests/middle_end.rs` runs every program of
  `src/lowering/tests/programs.rs` in the interpreter with and without the bcc
  pipeline, and compiles eight of them (two printing with `printf`) in arms
  A0-A3 for the host, running them side by side and comparing status and
  output with the interpreter. `tests/integration/code_generation_cli.rs`
  covers the command line.
  Native test programs run with captured stdout/stderr and a 120-second
  deadline; `BCC_TEST_TIMEOUT_MS` overrides it with a positive millisecond
  count. On timeout the shared runner kills the process tree (Windows) or
  process group (Unix), reaps the direct child and reports its command and
  captured output. Pipe readers get a 250-millisecond grace period, then are
  detached if an escaped descendant still holds a pipe open. The same deadline
  covers pipes left open after the direct child exits. Differential failures
  also include the seed, arm,
  rerun command and both bcc and LLVM IR. Object-only output needs no MinGW
  discovery; the CLI regression removes `MINGW_ROOT` and clears `PATH`.
- `benches/c` holds seventeen self-checking performance programs; their reduced
  workloads (`-DBENCH_REDUCED`) run in the interpreter, with and without the
  bcc optimizer, in `tests/integration/bench_corpus.rs`.
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
| 8 | Native back end: integer, variadic, dynamic-stack and float code (stages 1-3 of its design) | see [docs/design/native-backend.md](docs/design/native-backend.md), "Staged roadmap", which tracks each stage's status |

Stage 7 status: promotion, SCCP, GVN, LICM and bottom-up inlining exist
(the prototype pipeline above). The comparison harness exists:
`scripts/compare_optimizers.py` runs arms A0-A3, N and R, with `--passes` ablation,
over the seventeen self-checking programs in `benches/c` and writes JSON Lines plus a
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
- The `returns_twice` rule for the in-house optimizer is tested from C99 §7.13
  (`optimizer::tests::inline`, `tests/fixtures/programs/setjmp.c`); it relies
  on every call being a memory barrier in `optimizer/memory.rs` and on
  volatile accesses never being promoted.


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
returns one only for a retained integer constant expression. An object-size
query semantic analysis cannot answer becomes `object_size`, answered after
inlining (see [Object sizes](#instructions)); outside a function, or with an
operand that has side effects, it returns `(size_t)-1` in modes 0/1 or zero in
modes 2/3 without evaluating the operand. `trap` is a side-effecting
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
C99 §6.7.5.2p5, p. 117; PDF p. 129; §6.5.3.4p2, p. 80;
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
same normalized type-site actions. Effects-only operands interpret the retained M5
recipes. Allocation, sizeof and pointer/subscript/increment strides consume the
shared semantic size-formula DAG.

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
member. Range initializers save each selected initializer expression once, including
aggregate values. The runtime and static paths share the same flattened
initializer leaves (since M8, replayed from the retained semantic plan).

Static file-scope compound literals are memoized as internal globals and filled
from an iterative queue. Constant pointer values emit bytes; addresses of compound
literals, array members, selected GNU/C11 expressions and `&*` forms emit the
existing global relocation representation. Interpreter and LLVM emission consume
the same constants. C99 §6.6p7-9 (printed pp. 95-96; PDF pp. 107-108) and §6.7.8
govern these static forms. Label addresses and differences use the label
relocations described below.

Floating queries/sign-copy lower through existing scalar operations and byte
accesses. Native x87 support is exercised without changing the interpreter's
existing f80/f128 limitation. Bounded formatting shares the existing interpreter
formatter, writes at most `n-1` characters plus a terminator, avoids the destination
for `n==0`, and returns the untruncated character count. See the dated torture
survey for corpus and focused execution results.

## GNU entity attributes (2026-10-14)

Semantic analysis retains validated GNU attributes as facts
([semantic-analysis.md](semantic-analysis.md#gnu-entity-attributes-and-cleanup-regions-2026-10-14));
lowering maps them without reading attribute tokens (`lowering/symbols.rs`,
`lowering/cleanup.rs`):

- **Textual form.** Function headers end in
  `linkage [align N] [returns_twice] [noinline|alwaysinline] [inline_copy_of @name] [hidden|protected] [section "name"] [used] [alias @target]`,
  global headers in `linkage [thread_local] [constant] size N, align N
  [hidden|protected] [section "name"] [used] [alias @target]`. Module lines `constructor @f priority N` and
  `destructor @f priority N` follow the globals. The verifier checks alias
  targets (existing, same kind, defined, not an alias, thread-local exactly
  when the alias is), section names
  (nonempty, without quote, newline or NUL), default visibility on internal
  symbols (LLVM rejects any other for local linkage) and
  constructors/destructors (defined, no parameters).
- **Lowering.** An external entity with `weak` gets `weak` linkage, and a
  Microsoft `selectany` object this unit defines `selectany` linkage (LLVM
  `weak_odr` in its own comdat), a Microsoft inline body `selectany` or
  `linkonce` linkage (`weak_odr` or `linkonce_odr` in its own COMDAT), and
  a `dllimport` declaration or `dllexport` definition its DLL storage (the
  native back end reaches an import through `__imp_name` and lists exports
  in `.drectve`); every
  retained alias is declared, even if unused, and points at its target's
  symbol; section, used, visibility and function alignment go on the symbol
  when it is declared; constructors and destructors are registered in
  definition order. A cleanup region's exit calls its function with the
  variable's address (a stack slot; cleanup variables are address-taken),
  interleaved with VLA stack restores in the transition's inner-to-outer
  order, after return values and statement-expression tails are computed.
- **Optimizer.** `inline` refuses weak and `noinline` callees, whose body a
  strong definition may replace or the program asked to keep. GVN's alias check compares alias globals as their targets.
  Aliases have no body, so they are never inlined either.
- **Interpreter.** `run_main` (used by the CLI, the difftest oracle and the
  end-to-end tests) runs constructors before main and destructors after main
  returns or `exit` (unwinding main's frames first), not after `abort`.
  Aliases share their target's address and body, and a call through an alias
whose function type is compatible with its target's but spelled differently
(`int f()` for `int g(void)`) runs as an indirect call through that type
would (`Module::function_definition` resolves every alias); an undefined weak function or
  global has the null address unless the host provides the function, and
  calling it is undefined behavior.
- **LLVM.** Weak definitions are `weak`, weak declarations `extern_weak`,
  aliases LLVM global aliases (a thread-local one `thread_local`, which
  LLVM-C cannot set on an alias, so it is parsed from textual IR in a module
  of its own, linked in with `LLVMLinkModules2` and pointed at its target),
  and `llvm.global_ctors`, `llvm.global_dtors`
  and the retained-symbol array (`llvm.compiler.used` on ELF, `llvm.used` on
  COFF) are appending arrays, as bundled Clang emits them; sections, visibility and function alignment use the
  LLVM-C setters.

Evidence: `src/ir/tests/symbols.rs`, `src/backend/interpreter/tests/startup.rs`,
`backend::llvm::tests::golden::symbol_attributes_match_clang_spellings`, and
`src/pipeline/tests/middle_end/entity_attributes.rs`, which runs the issue #56
reproducers and cleanup/priority programs through the interpreter at every
stage, all four LLVM arms and bundled Clang on the host, and links weak/alias
units with Clang-compiled units natively for the host's target (Linux GNU or
Windows GNU) and, from Windows, under WSL (System V).

## Labels as values and computed goto (2026-10-14)

GNU labels as values (`&&label`, `goto *expression`, issue #51) run end to
end, the way Clang compiles them. Contract:
[GCC Labels as Values](https://gcc.gnu.org/onlinedocs/gcc/Labels-as-Values.html);
LLVM [`blockaddress`](https://llvm.org/docs/LangRef.html#addresses-of-basic-blocks)
and [`indirectbr`](https://llvm.org/docs/LangRef.html#indirectbr-instruction).

**IR.** A body has a label table: `label0 = block_label block3` names an
address-taken block. Labels, not block numbers, are what `block_addr label0`
and global relocations refer to, so renumbering passes and ABI lowering keep
them valid by remapping one table. `indirect_br addr, [block1(v2), block3]`
lists every possible target with its own arguments, so a labeled block may
have parameters; each edge becomes one phi entry in LLVM. Labels follow
LLVM's rules: not the entry block, one label per block, `indirect_br`
targets distinct and labeled. An address that names no listed block is
undefined behaviour, as for `indirectbr`. An unreachable labeled block stays
in the body, since its address may still be taken; it is never executed.
Relocations `blockaddress(@f, labelN)` and `labeldiff.TY(@f, labelA, labelB)`
put label addresses and differences into static data.

**Interpreter.** Each function with labels owns one label object: label `n`
is its byte `n`, so a label difference is a small integer and a label
address plus a difference names another label, as the position-independent
GCC idiom `goto *(&&base + offset)` requires. The object holds no data.
`indirect_br` traps with "computed goto to an address that is not a label of
the executing function" for any other pointer, including another function's
label, and as a poison branch for poison.

**LLVM.** `block_addr` is `LLVMBlockAddress`, `indirect_br` is
`LLVMBuildIndirectBr` plus `LLVMAddDestination`. Every labeled block exists
even when unreachable (holding only `unreachable`). Bodies are built before
global initializers, which may take block addresses; a label difference is
`trunc(sub(ptrtoint blockaddress, ptrtoint blockaddress))`, Clang's constant.
LLVM leaves equality between two block addresses undefined; only comparison
with null is meaningful. The interpreter compares them as distinct
addresses, so a program comparing two label addresses may differ natively.

**Optimizer.** No pass deletes, merges or threads through a labeled block,
and no pass redirects an `indirect_br` edge. `simplify-cfg` empties an
unreachable labeled block to an `unreachable` stub, once (`Draft::retire_block`),
skips labeled forwarders and select arms and never merges a labeled block
into its predecessor. SCCP marks every `indirect_br` edge executable. DCE,
promotion and GVN treat indirect edges like any other through the shared
edge iterators, so promotion passes slot values on them. LICM leaves a loop
whose header an `indirect_br` outside it enters, since no preheader can be
inserted on that edge, as LLVM does for loops without a preheader. The
inliner refuses a callee with labels or indirect branches, as LLVM refuses
`indirectbr` and address-taken blocks; callers with labels still inline
others (a split keeps the block's start, and so its label).

**Lowering.** Semantic analysis retains each function's address-taken labels
in first-`&&` order (`ControlFacts::address_taken`, `LabelFacts::address`),
the label each `&&label` names and the labels of each constant label
difference; lowering numbers IR labels by that order, so static initializers
lowered before the body refer to the right label. As in Clang, every
`goto *e` passes its `const void *` to one indirect-branch block per
function, whose `indirect_br` lists all address-taken labels, so `n` gotos
and `m` labels make `n + m` edges rather than `n * m`. A computed goto is a
jump with an empty transition: like Clang, it releases no VLA storage it
leaves before the function returns. GNU local labels lower as ordinary
labels (only nested functions, still refused, could jump to them from
elsewhere). `void *` arithmetic steps by one byte with byte alignment.

**Evidence.** The conformance reproducer `computed_goto_table.c` interprets
and runs natively in every arm. Of the 18 GCC 15.2.0 execute sources that use
labels as values, 10 now pass in the interpreter, the optimized interpreter,
A0 and A1 (none did before); the others need nested functions (4), inline
assembly (2), a global register variable (`pr51447.c`) or hosted headers
(`comp-goto-1.c`, which with MinGW headers interprets correctly but, like
Clang's own build, crashes natively because it assumes 64-bit `long`). The
whole survey (1,677 sources, `--jobs 6 --timeout 10`) has no source and arm
that passed before and fails now; 14 sources pass newly in all four arms,
four of them through `void *` arithmetic or local labels alone. Lua 5.4.9's
`lvm.c` compiles with its `LUA_USE_JUMPTABLE` dispatch table; a Lua built
from 29 bcc-compiled files (A0 and A1; `lua.c` and `loadlib.c` use MinGW
vector types) runs a test script identically to a Clang build.

## Returns-twice calls: setjmp and longjmp (2026-10-16)

Issue #50. One external-symbol contract, `builtins/external.rs`, classifies
the library functions that return twice (`setjmp`, `_setjmp`, `sigsetjmp`,
`__sigsetjmp`, `_setjmpex`, `_setjmp3`, `__mingw_setjmp`,
`__intrinsic_setjmp(ex)`; `vfork`, `getcontext`, `savectx`), restore an
environment (`longjmp`, `_longjmp`, `siglongjmp`, `__longjmp_chk`,
`__mingw_longjmp`) or return MinGW-w64's stack pointer (`mingw_getsp`). No
other phase matches these spellings.

- **Semantic analysis.** The finalized entity retains its effective contract:
  `EntityAttributes::returns_twice` for GNU `returns_twice` or a linked
  function whose C name the catalog lists (glibc declares `_setjmp` without
  the attribute), combined with cataloged assembler requests. An internal or
  local C function of that name alone is not marked.
- **IR.** `Function::returns_twice`, printed and parsed as the header keyword
  `returns_twice` after `align`. Lowering sets it from the retained entity
  contract, including cataloged assembler requests; the textual parser also
  sets it for an unmarked non-internal
  function the catalog lists. Calls are ordinary `call`s. Every safeguard
  asks `Module::returns_twice`, which adds an alias's canonical target's
  flag to its own, and `Module::returns_twice_escapes`, which says whether a
  returns-twice address reaches a value (`func_addr`) or a global
  relocation, so that any indirect call may return twice.
- **Optimizer.** `inline` refuses functions that are or call flagged
  functions or take their address, and their direct wrappers, as before
  but from the effective contract; in a module where such an address
  escapes, a function making an indirect call is refused too. Every call is a memory barrier, and volatile accesses
  stay, which is what C99 §7.13.2.1p3 needs; a non-volatile local changed
  between `setjmp` and `longjmp` is indeterminate, so promotion stays legal.
- **LLVM.** The declaration and every direct call get `returns_twice`, as
  does every indirect call in a module where such an address escapes; LLVM
  then treats the caller as exposing a second return (no stack-slot
  coloring, for example), as for Clang's output.
- **Native back end.** Direct calls to a flagged function or alias, and
  every indirect call where such an address escapes, are marked in machine
  IR. A second return finds the callee-saved registers as they were at the
  first, but memory as the program left it, so the allocator does not
  coalesce a register live across such a call: its spill slot then holds
  only its own value (`docs/design/native-backend.md`, "Calls"). Until
  2026-10-26 the back end refused these calls and addresses.
- **Interpreter.** `setjmp`-family calls store the frame's activation, depth,
  block, position and dynamic-allocation mark in the first 40 bytes of the
  `jmp_buf` (glibc's has 200, MinGW-w64's 256) and return 0. `longjmp`
  checks that frame is the same live activation and that the position
  follows an `i32` call, pops the newer frames and the dynamic objects
  allocated since, and resumes with `val`, or 1 for 0; otherwise it traps
  with `longjmp to an environment that no active setjmp invocation saved`.
  A corrupted record traps the same way. `setjmp` accepts the three x86-64
  shapes of the family, `(env)`, `(env, int savemask)` and MinGW-w64's
  `(env, frame)`, since one spelling (`_setjmp`) has different arities
  across C libraries; others are a signature mismatch.
  `mingw_getsp` returns the frame address. `vfork`, `getcontext`, `savectx`
  and attributed functions outside the catalog trap as unsupported.

Evidence: `tests/fixtures/programs/setjmp.c` (nested buffers, longjmp three
frames down, 0 delivered as 1, repeated returns, unchanged non-volatile
locals in a loop, a Lua-style handler chain, glibc `_setjmp` and `sigsetjmp`)
through the interpreter for Linux and, with MinGW-w64's declarations, for
Windows, and natively through LLVM at O0 and with `--opt=bcc` and O2
(`tests/integration/code_generation_cli.rs`); unit tests in `ir/tests/symbols.rs`,
`lowering/tests/extensions.rs`, `optimizer/tests/inline.rs`,
`backend/interpreter/tests/nonlocal.rs`, `backend/llvm/tests/golden.rs` and
`backend/native/tests/selection.rs`. The issue's `setjmp_linux.c`
interprets and runs natively. Lua 5.4.9 with `LUA_USE_LINUX` builds from 33
bcc-compiled units, `ldo.c` (`_setjmp`/`_longjmp`, marked `returns_twice` in
its LLVM IR) included, in A0, A1 and A2; the six corpus scripts and an
error-handling script (`pcall`, `error` objects, `xpcall`, coroutine errors,
20,000 protected calls, to-be-closed variables) print what a Clang -O0
build prints. Lua as one translation unit stops in the interpreter at the
missing `realloc` host function, an unrelated host gap. Windows native
execution (MSVCRT's `_setjmp` with a frame address and SEH unwinding) was
not run for this change.

Native back end (2026-10-26): `setjmp.c` builds with `--backend=native` at
`--opt=none` and `--opt=bcc` and exits 0 (`code_generation_cli.rs`). The
native end-to-end program `setjmp_and_longjmp` (`backend/native/tests/
programs.rs`: a `longjmp` three calls down, a jump back into a loop, ten
non-volatile locals live across the call, volatile ones changed in between,
and an unchanged local copied into a variable the first pass changes)
matches the interpreter with ordinary allocation and with every value in a
spill slot; without the coalescing rule the spilled runs print the changed
copy's value after the second return.

## GNU inline assembly (2026-10-26)

GNU `asm` statements inside functions compile end to end through the LLVM
back end, as Clang compiles them; the interpreter refuses to run them.
Contract: [GCC Extended Asm](https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html),
[Constraints for asm Operands](https://gcc.gnu.org/onlinedocs/gcc/Constraints.html)
and LLVM's [inline assembler expressions](https://llvm.org/docs/LangRef.html#inline-assembler-expressions).
C99 §J.5.10 (p. 514; PDF p. 526) lists `asm` as a common extension.

**Semantic analysis** (`semantic_analysis/assembly.rs`). The operand
expressions are typed by the ordinary expression work; then each constraint
is parsed (`assembly/constraints.rs`: `=`, `+`, `&`, `%`, `,`
alternatives, the generic `r` `m` `o` `V` `<` `>` `g` `X` `i` `n` `s` `p`
`E` `F`, the x86 registers `a` `b` `c` `d` `S` `D` `q` `Q` `R` `l` `U`
`x` `v`, the accepted but unlowered `A` `f` `t` `u` `y` `k` `Y…`, the
ranged immediates `I` `J` `K` `L` `M` `N` `O` `e` `Z`, matching digits and
`[name]`, and `=@cc` flag outputs). Every operand lists the same number
of comma-separated alternatives; analysis takes the first alternative
column in which every operand has a location, as GCC chooses one
alternative for the whole statement, so `"=a,b"` and `"=b,a"` never share
a register. In that column each operand gets one location:

- an output is a modifiable lvalue; it takes a flag, a register when its
  constraint allows one and its type fits (an integer or pointer of at most
  64 bits, `float` or `double`), else its memory;
- an input takes an immediate for a constant its constraint allows (`g`
  only when the value fits a sign-extended 32-bit field; `i`, `g` and `X`
  also take an address constant, but `g` prefers a register for one), else
  a register, else the lvalue's memory;
- a matching input names an earlier output in a register; integers and
  pointers may differ in width (the register holds the input widened or
  narrowed to the output's width, as in GCC), floating values may not.

Memory operands must be addressable (no bit-field, `register` object or
vector element). Clobbers (`assembly/registers.rs`) are any width of an
x86-64 register name with or without `%`, `memory`, `cc` and its LLVM
spellings. A clobber of the register an operand's constraint names alone is
"impossible constraints". The template (`assembly/template.rs`) resolves
`%N`, `%[name]`, the x86 modifiers, `%lN`/`%l[name]` goto labels, `%=`,
`%%`, `%{` `%|` `%}`, keeps only the AT&T alternative of each `{…|…}`
dialect group (the others are skipped unread, as GCC skips them in AT&T
syntax, so they are not checked), and rewrites each
`%z` to the AT&T suffix of the operand's own mode (its C type: `b` `w` `l`
`q` for integers and pointers, `s` `l` `t` for `float`, `double` and x87
`long double`, the byte size of an aggregate memory operand, `q` for a
symbol address), so lowering and the back ends never see `z`, which LLVM
lacks; `%z` on a numeric constant, a flag output or an object of another
size is an error, as in GCC; basic assembly
(no colon) substitutes nothing. Each fault is `SemanticErrorKind::InvalidAsm`
with GCC's or Clang's message. Every `asm goto` label is a jump of the
statement, checked as a `goto` is (an undefined label, or entering a
variably modified scope, is an error), and each has its own jump fact,
which `AsmFacts::jumps` lists in label order. Template numbering follows
GCC: the input each `+` output reads is numbered after the written inputs
(`%N` there names the output), and goto labels after those, so with one
`+r` output the first label is `%l2`. `AssemblyFacts` retains, by statement, the template
pieces, each operand's role and location, the clobbers and whether the
statement is volatile (written so, basic, without outputs, or `goto`).

**IR.** `asm` names a module-level `InlineAsm` descriptor (`ir/inline_asm.rs`):
an LLVM-syntax template (`$N`, `${N:m}`, `${:uid}`, `$$`, `$(` `$|` `$)`),
operands, clobbers and `sideeffect`. It has no result. Each operand is one
value: a register or flag output is the `ptr` its result is stored to after
the assembly runs, a memory operand the `ptr` it addresses (`bytes N` for an
aggregate), and an input its value; an immediate is an `iconst`,
`global_addr` or `func_addr`, and a tied input (`"0"`) names a register
output of its type. The textual form prints the descriptor inline, as in
`asm sideeffect "addl $1, $0" (out "r" i32 v1, in "i" i32 v2, in "0" i32 v0)
clobber(memory, flags, rcx)`; the verifier checks these rules and the
template's operand numbers.

**Lowering** (`lowering/assembly.rs`). Operands are evaluated in order: an
output or memory operand as a place, an input as its converted value, a `+`
register operand as both (it becomes an output and a tied input; a `+`
memory operand becomes a memory output and a memory input of the same
address, Clang's `=*m,*m`, so the statement reads the object as well as
writing it). A register output passes a
fresh slot; its value is then assigned to the lvalue as an assignment
would, so SSA locals, bit-fields, volatile and atomic objects are written
correctly. A `float` in a general register, or an integer in an SSE
register, is reinterpreted with `bitcast`. An output and its matching
inputs share a register as wide as the widest of them, as GCC gives the
pair the wider mode: a narrower input is extended by its signedness, a
wider integer or pointer input keeps all its bits (so `%q2` reads all of
a 64-bit input tied to an `int` output), each unmodified reference to a
narrower operand gets the size modifier of its own width (`%0` stays
`%eax`), and the output takes the low bits when assigned. The shared
register keeps the output's class: an SSE output tied to a wider integer
is an `f64`, whose bits the `int` output truncates (`psrlq $32` on a
`long`, as GCC accepts). A flag output wider than 64 bits (`__int128`) is set from a
32-bit condition and zero-extended when assigned. Lowering checks each
descriptor against the IR's operand rules (`InlineAsm::check`, which the
verifier shares) and refuses one that breaks them, so no statement reaches
the verifier malformed. A local that is a memory operand lives in memory
(`address_taken.rs`). `asm goto` adds a selector output in a stack slot
(a memory operand, so a statement that clobbers every general register
still has room for it) that the template's appended code sets: 0 on
fall-through, and `N + 1` in a local stub
`.Lbcc_asm_goto${:uid}_N` that each `%lN` names; a `switch` on it takes each
label's jump with the cleanups that jump leaves. Outputs are therefore valid
on every path, and no IR terminator needs to know about assembly. Operands in
x87, MMX or register-pair registers, `__int128` or aggregates in registers,
and symbolic immediates with an offset are refused as unsupported.

**Optimizer.** Every pass treats `asm` as a call: it is never removed,
duplicated or moved, it is a full memory barrier for GVN, its pointer
operands escape, and DCE keeps it whether or not it is volatile. The inliner
copies it with its operands; each copy gets its own `${:uid}`.

**LLVM.** `LLVMGetInlineAsm` with Clang's constraint string: register and
flag outputs are the call's results (`=r`, `={ax}`, `=&r`, `={@ccz}`),
stored to their output pointers after the call; memory operands are `=*m`
and `*m` with an `elementtype` attribute. A register set becomes the LLVM
constraint that allows exactly it: one register by Clang's name (`{ax}`,
`{r8}`), `r`, `Q` (`%rax`-`%rbx`) or `R` (the legacy registers); any other
set, such as `ab` or `U`, is refused rather than widened. Every x86
statement clobbers `~{dirflag},~{fpsr},~{flags}`, and `memory` adds
`~{memory}`.

**Interpreter.** Executing an `asm` traps with "unsupported by the
interpreter: inline assembly, which only compiled code can run"; a program
that never reaches its assembly still runs. Differential tests do not
generate assembly.

**Native back end** (`backend/native/isel/asm.rs`). Selection fixes each
register operand to a physical register (the constraint's, or a free one
of its class, most constrained operand first with backtracking; never `%rsp`, `%rbp` or the spill scratch registers
`%r10`, `%r11`, `%xmm14` and `%xmm15`; a memory operand in a frame object or
at a defined symbol needs none), copies the inputs in, and emits one
`MInst::InlineAsm` that reads the input registers and writes the outputs
and clobbers, so the allocator and frame treat it as they treat a call. The
listing writes the substituted template between `#APP` and `#NO_APP`; the
direct object encoder cannot read it, so such a module is assembled by the
bundled clang and `--native-object` refuses it. Operand modifiers outside `b h w k q V z a x`
(registers), `a H` (memory) and `c P p a n` (immediates) are refused, as is
a clobber of `%rsp` or `%rbp`; see
[docs/design/native-backend.md](docs/design/native-backend.md).

Evidence: `semantic_analysis/tests/assembly.rs` (locations, template pieces,
clobbers, every fault, goto jumps), `ir/tests/inline_asm.rs` (round trip,
builder, verifier rules, template pieces), `lowering/tests/assembly.rs`, and
`tests/integration/inline_asm.rs`, which runs `rdtsc`, `cpuid`, `+r`,
named, immediate, memory, matching, widened matching, flag, SSE, `%h`,
register clobber, early clobber, global memory, `memory` barrier, `%=` and
`asm goto` operands through LLVM at O0, at O2 and after the bcc optimizer,
and through the native back end with and without the bcc optimizer;
`backend/native/tests/inline_asm.rs` checks register choice, template
substitution, saved registers, flag outputs, fully spilled functions and
the native refusals.

## GCC's builtin setjmp and longjmp (2026-10-27)

GCC's `__builtin_setjmp (buf)` and `__builtin_longjmp (buf, 1)` ([Nonlocal
Gotos](https://gcc.gnu.org/onlinedocs/gcc/Nonlocal-Gotos.html)) are the
lightweight pair: a five-word buffer, no signal mask and no saved
registers, so the compiler keeps every value live across the setjmp where
the second return finds it. The buffer layout is LLVM's, which GCC shares on
x86-64: word 0 the frame address, word 1 the resume address, word 2 the
stack pointer.

- **Semantic analysis.** Both are GNU builtins (`builtins/scalar.rs`).
  GCC's prototypes type the buffer `void *`, so any object pointer converts
  (Clang's `void **` rejects `int[5]` and GCC's `pr60003.c`). The value must
  be an integer constant that is 1 after conversion to `int`
  (`ConstantConstraint::ConvertedIntegerValue`, diagnosed as
  `InvalidLongjmpValue`), as GCC and Clang require; one the conversion
  changes, such as `4294967297LL`, gets Clang's default-on
  `-Wconstant-conversion` warning (a new group, also controlled by
  `-W[no-]conversion`), as GCC warns with `-Woverflow`. Clang rejects the
  floating `1.0` that GCC converts, and so does bcc.
  `__builtin_longjmp` does not return, like Clang's builtin. Neither GCC 15
  nor Clang diagnoses a `__builtin_longjmp` in the function that called
  `__builtin_setjmp`, which the GCC manual forbids; it works on every bcc
  arm as with Clang (GCC -O2 crashes on it).
- **Lowering** emits what Clang emits: `frame_address` stored in word 0,
  `stack_save` stored in word 2, then `sjlj_setjmp buf`, an `i32`;
  `__builtin_longjmp` becomes `sjlj_longjmp buf` and `unreachable`.
- **Optimizer.** Both opcodes have side effects (`dce`) and are full memory
  barriers (`memory_write`). A function containing `sjlj_setjmp` has an
  unsafe frame, so `inline` refuses it and its direct wrappers; GCC never
  copies a function that receives a non-local goto and rejects
  `always_inline` on one, which LLVM inlines (the bcc pipeline then keeps
  the call, and a jump back into its returned frame is undefined). As for
  `setjmp`, promotion stays legal: a non-volatile local changed between the
  setjmp and the jump is indeterminate (C99 §7.13.2.1p3).
- **LLVM** gets `llvm.eh.sjlj.setjmp` and `llvm.eh.sjlj.longjmp`, which need
  no function attribute: the frame address keeps a frame pointer, and the
  X86 back end saves every callee-saved register of a function using them.
- **Native back end** (`isel/stack.rs`). The buffer goes in `%rdi`.
  `MInst::SjljSetjmp` stores a resume label in word 1 and sets `%eax` to 0;
  at the label `%eax` becomes 1, and a frame addressed from `%rbp` first
  derives `%rbp` from the frame address it arrives with. Its clobber set is
  every register but `%rsp` (and a frame pointer), so nothing stays in a
  register across it, the prologue saves every callee-saved register, and
  `Liveness::live_across_returns_twice` keeps the values live across it out
  of coalescing, as for a returns-twice call. `MInst::SjljLongjmp` is LLVM's
  `movq (%rdi), %rbp; movq 8(%rdi), %rax; movq 16(%rdi), %rsp; jmpq *%rax`.
- **Interpreter.** `sjlj_setjmp` keeps the record `setjmp` keeps in the
  three words the frame address and stack position leave free: word 1 the
  block and position after it, word 3 the activation, word 4 the depth and
  the dynamic-allocation mark. `sjlj_longjmp` checks it like `longjmp` (the
  position must follow an `sjlj_setjmp`), so a jump to a returned frame, an
  unfilled buffer or a library `setjmp` record traps.

Evidence: `tests/fixtures/programs/builtin_setjmp.c` (a jump three frames
down, integer, pointer and double values live across, the caller's
callee-saved registers, 1,000 repeated jumps, a setjmp in a loop body, VLAs
and `alloca` before and after the setjmp, a jump through a copied buffer,
nested handlers) prints what GCC 15 and Clang print at -O0 and -O2 in the
interpreter (with and without `--opt=bcc`), through LLVM at O0 and O2 and
through the native back end with and without the bcc optimizer
(`code_generation_cli.rs`), and matches the interpreter in the native
end-to-end programs with ordinary allocation and with every value spilled;
with the callee-saved registers left out of the clobber set the native run
prints wrong values and crashes. Unit tests: `ir/tests/stack.rs`,
`lowering/tests/extensions.rs`, `semantic_analysis/tests/scalar_builtins.rs`,
the `sema-builtin-setjmp` diagnostic golden, `optimizer/tests/{dce,inline}.rs`,
`backend/interpreter/tests/nonlocal.rs`, `backend/native/tests/selection.rs`
and `operands.rs`. GCC torture (Linux, six arms): `built-in-setjmp.c` and
`pr84521.c` pass on every arm, up from none; `pr64242.c` passes on the four
compiled arms and stops the interpreter at its subtraction of pointers to
distinct `alloca` blocks; `pr60003.c` reads a non-volatile local changed
after the setjmp, which C leaves indeterminate and bcc's SSA locals do not
keep, failing as Clang -O1 and -O2 do. The `builtins/*-chk.c` tests compile
and link on every compiled arm; at O0 their checks do not run, and at
`--opt=bcc` they stop before their first `__builtin_setjmp` on fortify
folding, as GCC -O2 does outside the DejaGnu harness.
