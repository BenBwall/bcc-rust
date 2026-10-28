# Native x86-64 back end

Status: design, with stages 1 and 3, most of stages 2 and 4, and stage 5
ELF/COFF object emission (the default for Windows targets, opt-in for Linux)
implemented in `src/backend/native/` (2026-10-20); see "Implementation
status" at the end.

This document designs bcc-rust's own code generator for x86-64: the path from
the post-ABI bcc IR to assembly and, later, to object files, without LLVM. It
complements [middle-end.md](../../middle-end.md), which specifies the IR, the
optimizer, ABI lowering and the two existing back ends (LLVM through the C API,
and the interpreter). Sections marked **Stage 1** describe what the prototype
implements; everything else is the plan.

## Goals and non-goals

1. **Flexibility of comparison.** The repository exists to compare bcc's
   optimizer with LLVM's. Today every native arm shares LLVM's code generator
   (A0-A3, N). A second, independent code generator lets the same question be
   asked with a deliberately simple selector and allocator: does the bcc
   optimizer's gain survive a code generator that does not clean up after it?
   It also measures, for the first time, how much of A1's speed comes from
   LLVM's machine passes (B1 against A1, below).
2. **A path to running fully native.** With the native back end and a direct
   object writer, `bcc-rust x.c -o x` needs LLVM only for the final link
   (bundled LLD), and eventually not even for that on ELF.
3. **The repository's engineering rules.** Arena allocation only
   (`.clippy.toml` and `tests/allocation_count.rs`), explicit work stacks
   instead of native recursion, dense `u32` indices, fixed-size records, and
   fast compilation. A code generator is naturally iterative: every analysis
   below is a worklist or a linear walk.
4. **Never miscompile.** A construct the back end does not implement is
   refused before any output is written, with a diagnostic naming the
   construct and the function. Refusals are counted by the surveys; wrong code
   is a bug.

Non-goals for now: debug information (DWARF line tables and CodeView), position
independent shared libraries, other architectures, instruction scheduling, and
importing LLVM-optimized IR back into bcc IR (so "LLVM optimizer + native code
generator" is not an arm).

## Scope and targets

Both x86-64 conventions the ABI lowering already implements:

| Target triple | Object format | Convention | Unwind tables | Stage |
| --- | --- | --- | --- | --- |
| `x86_64-w64-windows-gnu` | COFF (PE) | Microsoft x64 | `.pdata`/`.xdata` via `.seh_*` | 1 |
| `x86_64-unknown-linux-gnu` | ELF | System V AMD64 psABI | `.eh_frame` via `.cfi_*` | 1 |
| `x86_64-unknown-linux-musl` | ELF | System V AMD64 psABI | `.eh_frame` | 1 (same code) |
| `x86_64-pc-windows-msvc` | COFF | Microsoft x64 | `.pdata`/`.xdata` | 1 for assembly; linking needs an MSVC CRT the bundled toolchain does not ship |

Windows GNU comes first because it is the development host: every test runs
natively. System V is tested under WSL from the start, so neither convention
becomes an afterthought. The two conventions share the selector, the machine
IR, the allocator and the emitter; they differ only in a `CallConv`
description (registers, shadow space, red zone) and in the unwind directives.

The baseline instruction set is x86-64 v1 (SSE2, no POPCNT, LZCNT, BMI or
CMOV-free fallbacks needed: CMOV is baseline). Bit counts use BSR/BSF with a
zero check; population count expands to the SWAR sequence. A `-march` option
can later enable v2/v3 forms.

### References

- System V Application Binary Interface, AMD64 Architecture Processor
  Supplement, version 1.0 (<https://gitlab.com/x86-psABIs/x86-64-ABI>):
  §3.2.1 "Registers" (Figure 3.4, callee-saved `%rbx`, `%rbp`, `%r12`-`%r15`),
  §3.2.2 "The Stack Frame" (16-byte alignment: `%rsp + 8` is a multiple of 16
  at function entry; the 128-byte red zone), §3.2.3 "Parameter Passing"
  (argument registers, `%al` for variadic calls, extension of returns),
  §3.5.7 "Variable Argument Lists" (register save area, `va_list`), §3.6.2
  "DWARF Register Number Mapping" (Figure 3.36), §3.7 "Stack Unwind
  Algorithm", §4.4.1 "Relocation Types" (`R_X86_64_PC32`, `PLT32`,
  `GOTPCREL`, `GOTPCRELX`, `64`).
- Linux Standard Base Core Specification 5.0, §10.6 "Exception Frames" (the
  `.eh_frame` CIE/FDE format), and DWARF 5 §6.4 "Call Frame Information".
- Microsoft Learn: "x64 calling convention" (sections "Parameter passing",
  "Varargs", "Return values", "Caller/callee saved registers"), "x64 stack
  usage" (shadow/home space, stack allocation, `__chkstk`), "x64 prolog and
  epilog" (the required prologue and epilogue shapes), "x64 exception
  handling" (`RUNTIME_FUNCTION`, `UNWIND_INFO`, `UNWIND_CODE`,
  `UWOP_PUSH_NONVOL`, `UWOP_ALLOC_SMALL`/`LARGE`, `UWOP_SET_FPREG`), and "PE
  Format" (sections ".pdata", ".xdata", and "x64 Processors" under COFF
  relocations: `IMAGE_REL_AMD64_ADDR64`, `REL32`, `ADDR32NB`).
- Intel 64 and IA-32 Architectures Software Developer's Manual, Volume 2:
  instruction semantics the selector depends on (IDIV/DIV faults, shift counts
  masked to 5 or 6 bits, BSF/BSR leaving the destination undefined for zero,
  CMOVcc, SETcc).

## Where the back end sits

```text
 bcc IR, post-ABI profile           scalars and pointers; ABI decisions explicit
   │
   ├──► LLVM back end (src/backend/llvm)        LLVM-C → object → LLD
   ├──► interpreter (src/backend/interpreter)   reference semantics
   └──► native back end (src/backend/native)
          │ support check        refuse the whole module, or accept it
          │ per function, in a scratch arena reset between functions:
          │   instruction selection   IR → machine IR over virtual registers;
          │                           block parameters → parallel copies
          │   liveness, intervals     sparse live-in sets; ranges with holes
          │   register allocation     coalescing, linear scan with splitting
          │   rewrite                 physical registers, moves; checker
          │                           (debug), `lea` peepholes, loop rotation
          │   frame layout            callee-saved, spills, slots, out-args
          │   emission                prologue/epilogue, unwind directives
          ▼
        assembly text (AT&T) ──► bundled clang: assemble + link with LLD   (stage 1)
        object bytes (ELF/COFF) ──► LLD                                  (stage 5)
```

The native back end consumes exactly what the LLVM back end consumes: a
verified post-ABI module. ABI lowering has already decided every aggregate's
passing (`byval`, `sret`, `registers`, or plain scalars), every narrow
integer's extension (`signext`/`zeroext`), and expanded `va_arg`. The back end
never re-derives a calling-convention fact; it only maps the post-ABI
signature onto registers and stack slots (see `abi/` for the decisions and
"Calls" below for the mapping). This is the repository's root-cause rule: the
source of truth for the convention stays in `src/abi`.

One difference is deliberate: the LLVM back end receives the private copies
of inline-only definitions (`inline_copy_of`) and lets LLVM's always-inliner
choose between each copy and its external definition, while the native back
end receives a module in which the middle end has already made those choices
(`optimizer/always_inline.rs`, #109). Every call that remains names the
external function, so a recursive `__forceinline` function calls the external
definition, as LLVM and Clang do at O0. See `middle-end.md`, "Inputs".

### Comparison arms

Two arms join the table in `middle-end.md`:

| Arm | Middle end | Code generator | Flags |
| --- | --- | --- | --- |
| B0 | none | native | `--backend=native` |
| B1 | bcc pipeline | native | `--opt=bcc --backend=native` |

Ratios they answer:

- **B1/A1**: the same optimized bcc IR through two code generators: how much
  LLVM's instruction selection, register allocation and machine passes add.
- **B1/B0**: the bcc optimizer measured with a simple code generator. Comparing
  it with A1/N (the same optimizer measured with LLVM's code generator) shows
  whether an optimizer gain depends on the code generator cleaning up.
- **B0/N**: the code generators alone, on unoptimized IR.

All arms consume the same post-ABI module, so differences cannot come from
calling-convention choices. The torture survey runs B0 and B1 next to the
interpreter and the LLVM arms (`scripts/run_gcc_torture_execute.py --arms`),
and `scripts/compare_optimizers.py` can time them like any other arm.

## Pipeline from post-ABI IR

### Support check

`support::check_module` walks every function before anything is emitted and
returns the first refusal: `Unsupported { function, construct }`, printed as

```text
error: not yet supported by the native back end: floating-point value (in @f)
```

with exit status 1. The selector re-checks every case it handles (an
`Err(Unsupported)` from deep inside selection is a defence in depth, never a
panic), but the up-front check keeps refusals independent of selection order
and lets the survey classify them. Stage 1 refused floats, `i128`, variadic
definitions and dynamic stack allocation; **today** the check refuses only
`i128` bit counts and byte swaps, `i128` positions Win64 never
passes by value, structure results in register combinations neither
convention defines, stack arguments aligned above 16,
frames larger than 1 GiB, stack arguments (chiefly `byval` structures) of
more than 1 GiB in one signature, whose offsets and copy lengths would leave
32-bit operands, and thread-local globals in named sections. Frame layout
then refuses a frame whose combined outgoing arguments, spills, locals and
incoming stack parameters would still need offsets beyond 32-bit
displacements.

### Legalization

The post-ABI IR is already close to the machine: integers of 1-64 bits,
pointers, and floats. Legalization is therefore folded into selection for
everything but three families, each a separate IR-to-IR pass before selection
in a later stage:

| Family | Legal form | Stage |
| --- | --- | --- |
| `i8`/`i16`/`i32`/`i64`/`ptr` | registers; operations at 32 or 64 bits, sub-registers for compares, loads and stores | 1 |
| `i1` | a 32-bit register holding 0 or 1 | 1 |
| `i128` | a pair of 64-bit values; `add`/`adc`, `sub`/`sbb`, `mul` + `imul` cross terms, shifts by `shld`/`shrd` with a ≥64 select; division through `__divti3`/`__udivti3`/`__modti3`/`__umodti3` | 3 (done, in selection rather than a separate pass) |
| `f32`/`f64` | XMM registers (a second register class) | 3 (done) |
| `f80` | a pair of 64-bit registers (significand, then sign and exponent); x87 memory-to-memory sequences (no x87 register allocation) | 3 (done) |
| `f128` | a pair of 64-bit registers; libcalls (`__addtf3` …), whole XMM registers only at calls | 3 (done) |

**Narrow integer representation.** A value of type `iN` with N < 64 lives in a
64-bit register whose bits above N are unspecified, as in LLVM. Operations
whose low N bits depend only on the low N bits of the inputs (`iadd`, `isub`,
`imul`, `and`, `or`, `xor`, `shl`, `trunc`) run at 32 or 64 bits unchanged.
The others read exactly N bits: comparisons use `cmpb`/`cmpw`/`cmpl`, right
shifts and divisions first extend the operand with `movzx`/`movsx`, stores
write N bits, and `zext`/`sext` extend explicitly. An `i1` is the exception: it
is kept *canonical*, 0 or 1 in the low 32 bits, because comparisons produce it
that way (`setcc` + `movzbl`) and branches and `select` test it with `test`.
An `i1` operation that could leave other bits set (`iadd.i1`, `trunc` to `i1`)
is followed by `and $1`.

### Instruction selection

**Stage 1 is a macro-expansion selector with local peepholes**, one IR
instruction at a time in block order, not a tree or DAG pattern matcher. The
reasons: the IR is already linear SSA with explicit memory operations, the
post-ABI module has few compound patterns worth matching, and a direct
expander is easy to make total (every opcode has a case; anything else is
`Unsupported`). Three peepholes recover most of what tree matching would:

1. **Rematerialized leaves.** `iconst`, `null`, `poison`, `stack_addr`,
   `global_addr` (of a global that is not thread-local) and `func_addr`
   define no register. Each use materializes
   them on the spot: as an immediate operand where the instruction accepts one
   (32-bit sign-extended immediates), as a memory operand's base (`slot(%rsp)`
   or `sym(%rip)`), or as a fresh vreg defined right before the use (`movl`,
   `movabsq`, `leaq`). Constants therefore never occupy a register across a
   loop, and the allocator never spills one. A `poison` materializes as zero,
   consistently at every use.
2. **Address folding.** A load or store whose address is a `stack_addr`, a
   defined `global_addr`, or a chain of `ptr_add`s becomes one memory
   operand: constants go into the displacement and one register offset into
   the index (stage 4), scaled when an `imul` by 1, 2, 4 or 8 or a `shl` by
   at most 3 computed it (`disp(base, index, scale)`; iteratively: the
   selector walks the chain with a bounded loop, not a recursive call). A
   symbol base with an index, which RIP-relative addressing cannot take, is
   loaded into a register once at the function's entry. A `ptr_add` that is
   materialized is one `lea` of the same address.
3. **Compare fusion.** An `icmp` whose every use is the condition of a
   `brif` or `select` is not materialized: the selector emits the `cmp`
   again immediately before each user's `jcc` or `cmov`, in whatever block
   the user is (stage 4; stage 1 fused only the one user in the compare's
   own block). The compare's operands dominate every user, so they are
   available there. Otherwise `icmp` produces a canonical `i1` with
   `setcc`/`movzbl`.

Use counts come from one pass over the function's operands; the IR has no use
lists and the selector does not need them otherwise.

The selector's per-opcode table for stage 1:

| IR | Machine code |
| --- | --- |
| `iadd`/`isub`/`and`/`or`/`xor` | `mov d, a; op d, b` (two-address); `b` may be an immediate |
| `imul` | `mov d, a; imul d, b`, or `imul d, a, $imm`; a power of two is a shift and a 64-bit product by 3, 5 or 9 a `lea` (stage 4) |
| `sdiv`/`srem`, `udiv`/`urem` | narrow operands extended to 32 bits; `mov %rax, a; cltd/cqto` or `xor %edx,%edx`; `idiv`/`div b`; result from `%rax` or `%rdx`. By a power of two (stage 4): `shr`, `and`, or for signed operands the dividend plus its sign shifted down, then `sar` or `and` and `sub` |
| `shl`/`lshr`/`ashr` | amount in `%cl` (or an immediate masked to the width); `lshr`/`ashr` extend a narrow operand first |
| `icmp` | `cmp a, b` at the operand width, then `setcc` + `movzbl`, or fused |
| `select` | `test c`/fused `cmp`; `mov d, b; cmovcc d, a` at 32 or 64 bits |
| `zext`/`sext`/`trunc` | `movzbl`/`movzwl`/`movl`, `movsbl`/`movswl`/`movslq`/…, or a copy |
| `ptrtoint`/`inttoptr`/`bitcast`/`freeze` | a copy (an `inttoptr` from a narrower integer zero-extends) |
| `ptr_add` | `leaq` of the folded address (`off(base)`, `off(base, index, scale)`), else `mov d, base; add d, off` |
| `load`/`store` | `mov` of the access width; narrow loads zero-extend into 32 bits |
| `clz`/`ctz` (+`_defined`) | `bsr`/`bsf`; `_defined` adds `cmov` from the width on zero; `clz` computes `width-1 - bsr` with `xor` |
| `popcount` | the 12-instruction SWAR sequence (x86-64 v1 has no `popcnt`) |
| `bswap` | `bswap` (32/64), `rolw $8` (16) |
| `uadd_overflow` | `mov d, a; add d, b; setb` |
| `umul_overflow` | `mov %rax, a; mul b; seto` |
| `copy`/`fill` | up to 64 constant bytes as loads then stores; else `rep movsb`/`rep stosb`, or `memmove` for `may_overlap` |
| `call`/`call_indirect` | see "Calls" |
| `jump`/`brif`/`switch` | `jmp`; `test`/`cmp` + `jcc`; a compare chain (a jump table in stage 4) |
| `block_addr` | `leaq .LBA<function>_<label>(%rip), d` |
| `indirect_br` | incoming registers of the listed targets, then `jmpq *addr` (see "Block parameters become parallel copies") |
| `global_addr` of a thread-local | ELF: `movq %fs:0, t; leaq sym@TPOFF(t), d`, or `movq %fs:0, d; addq sym@GOTTPOFF(%rip), d` for a declaration; Windows: `movl _tls_index(%rip), i; movq %gs:88, t; mov (t + 8i), b; leaq sym@SECREL32(b), d` |
| `return` | value to `%rax` (extended per `signext`/`zeroext`), epilogue, `ret` |
| `trap`, `unreachable` | `ud2` |
| `return_address` | `movq` from the CFA minus 8 |
| `frame_address` | `leaq` of the CFA minus 16, where the prologue then pushes `%rbp` first, as a frame pointer would save it |

### Block parameters become parallel copies

SSA block parameters are destructed during selection, not by a separate pass:

1. Each block parameter becomes a vreg. Block parameters are the only vregs
   with several definitions after selection; the allocator does not require
   SSA, only liveness, so this is safe.
2. An edge passing arguments gets the copies `params := args` as one
   *parallel copy*. If the edge's source has a single successor (`jump`), the
   copies go at the end of the source block, before the `jmp`. If the source
   has several successors (`brif`, `switch`), the edge is split: a new machine
   block holds the copies and a `jmp`, and the branch targets it. Splitting
   every argument-carrying multi-successor edge, rather than only critical
   ones, keeps the rule local and avoids the *lost-copy* problem (a parameter
   redefined on one path while its old value is still live on another).
3. A parallel copy is sequentialized with the standard algorithm (Boissinot
   et al., "Revisiting Out-of-SSA Translation", Algorithm 1): emit every copy
   whose destination is not the source of a pending copy; when only cycles
   remain, break one with a fresh temporary vreg. The *swap* problem (a loop
   passing `block1(v3, v2)` to its own parameters `v2, v3`) is a cycle and
   gets the temporary.
4. Copies are `mov` between vregs with an allocation *hint* both ways, so the
   allocator usually gives an argument and its parameter the same register and
   the rewrite deletes the move.
5. An `indirect_br` edge cannot be split, since the address names its
   target. A labeled block with parameters therefore gets a *landing block*
   that copies fresh *incoming* vregs into the parameters and jumps to the
   block; the label's address is the landing block's. The `indirect_br`
   assigns every listed target's incoming vregs (the verifier makes the
   targets distinct) and then jumps. Incoming vregs are read only by their
   landing block, so the assignments for the edges not taken are dead, not
   wrong; direct edges into the block still assign the parameters. A labeled
   block that no path reaches gets a `ud2` block for its address.

```rust
/// One pending copy of a parallel copy; `dst` are distinct.
struct PendingCopy { dst: VReg, src: CopySource }
enum CopySource { Reg(VReg), Imm(i64) }   // rematerialized constants copy as immediates

/// Sequentializes `copies` into `out`, using `fresh` for cycle temporaries.
/// Iterative: a ready list and a per-destination pending count.
fn sequentialize(copies: &mut ArenaVec<PendingCopy>, out: &mut ArenaVec<MInst>, fresh: impl FnMut() -> VReg);
```

### Machine IR

The machine IR (MIR) is x86-64 instructions over virtual and physical
registers, organized like the bcc IR: dense entities, arena vectors, one
function at a time.

```rust
/// A general-purpose register, numbered by its hardware encoding.
#[repr(u8)]
pub(crate) enum PReg { Rax, Rcx, Rdx, Rbx, Rsp, Rbp, Rsi, Rdi, R8, R9, R10, R11, R12, R13, R14, R15 }

/// A set of physical registers (clobbers, call uses).
pub(crate) struct PRegSet(u16);

/// A virtual register: a dense index per function.
pub(crate) struct VReg(u32);

/// A register operand before allocation: physical (precolored) or virtual.
pub(crate) enum Reg { Phys(PReg), Virt(VReg) }

/// Operand width; selects the instruction suffix and sub-register name.
pub(crate) enum Width { B8, B16, B32, B64 }

/// A memory operand: `disp(base)` or `disp(base, index, scale)`; only a
/// register or frame base takes an index (stage 4).
pub(crate) struct Mem { base: Base, index: Option<Index>, disp: i32 }
pub(crate) struct Index { reg: Reg, scale: u8 }   // scale 1, 2, 4 or 8

pub(crate) enum Base {
    Reg(Reg),
    /// A frame object, resolved to `%rsp`-relative after frame layout.
    Frame(FrameObject),
    /// A RIP-relative symbol reference (`sym+disp(%rip)`).
    Symbol(SymbolRef),
}

pub(crate) enum FrameObject {
    /// An IR stack slot or a back-end temporary (return areas).
    Slot(FrameSlot),
    /// The incoming stack argument area, by byte offset from the first
    /// stack argument (System V) or from the home area (Win64).
    Incoming(u32),
    /// The outgoing argument area at the bottom of the frame.
    Outgoing(u32),
    /// A spill slot, assigned by the allocator.
    Spill(u32),
}

pub(crate) enum Operand { Reg(Reg), Imm(i32), Mem(Mem) }

/// One machine instruction. Two-address forms (`Alu`, `Shift`, `Unary`,
/// `Cmov`) read and write `dst`.
pub(crate) enum MInst {
    Mov { width: Width, dst: Reg, src: Operand },          // reg ← reg/imm/mem
    MovImm64 { dst: Reg, imm: i64 },                       // movabsq
    Store { width: Width, dst: Mem, src: RegOrImm },
    Lea { dst: Reg, addr: Mem },
    Extend { signed: bool, from: Width, to: Width, dst: Reg, src: Operand },
    Alu { op: AluOp, width: Width, dst: Reg, src: Operand },   // add sub and or xor
    Imul3 { width: Width, dst: Reg, src: Operand, imm: i32 },
    Cmp { width: Width, lhs: Reg, rhs: Operand },              // also `test`
    Shift { op: ShiftOp, width: Width, dst: Reg, amount: ShiftAmount }, // imm or %cl
    Unary { op: UnaryOp, width: Width, dst: Reg },             // neg not bswap
    BitScan { reverse: bool, width: Width, dst: Reg, src: Reg },
    SignExtendAx { width: Width },                             // cltd/cqto: %rdx ← sign(%rax)
    Div { signed: bool, width: Width, divisor: Reg },          // uses %rax %rdx, defines both
    MulWide { width: Width, src: Reg },                        // mul: %rdx:%rax ← %rax * src
    Setcc { cc: Cond, dst: Reg },
    Cmov { cc: Cond, width: Width, dst: Reg, src: Reg },
    Call { callee: Callee, uses: PRegSet, clobbers: PRegSet },
    Jmp { target: MBlock },
    Jcc { cc: Cond, taken: MBlock, not_taken: MBlock },
    Ret { uses: PRegSet },
    Ud2,
}
```

Every operand of every instruction is enumerable with its role through one
function, the allocator's only view of the instruction set:

```rust
/// Calls `visit` with each register the instruction reads, writes, or both,
/// including implicit physical registers (`%rax`/`%rdx` of `Div`, call
/// argument registers and clobbers).
fn visit_registers(inst: &mut MInst, visit: impl FnMut(&mut Reg, Role));
enum Role { Use, Def, UseDef }
```

**Two-address constraints** are made explicit by the selector: `d = a + b`
becomes `mov d, a; add d, b` with a fresh `d`, and a hint asks the allocator to
put `d` where `a` was if `a` dies there. No later pass has to convert
three-address code.

**Fixed-register uses** are precolored moves: the selector writes argument
values into `%rdi`/`%rcx`…, divides through `%rax`/`%rdx`, shifts by `%cl`,
and copies results out of `%rax`, each through an explicit `mov` between a
vreg and a physical register. Physical registers are never live across a
block boundary (an invariant checked in debug builds); the allocator sees them
as fixed ranges.

**Flags** are not modelled as a register. The selector emits every flag
producer immediately before its consumer (`cmp` + `jcc`, `cmp` + `setcc`,
`test` + `cmov`), and nothing the allocator inserts between them (`mov`
reloads and spills) touches the flags. The allocator never uses `xor` to zero a
register for the same reason.

### Register allocation

**Stage 4 (as built): linear scan over split intervals with lifetime holes**
(Wimmer and Mössenböck, "Optimized Interval Splitting in a Linear Scan
Register Allocator", VEE 2005, the successor of Traub et al.'s second-chance
binpacking), after copy coalescing, with fixed-register ranges, proven by a
symbolic checker. Every pass is a linear walk, a worklist or a binary heap;
nothing recurses.

1. **Critical edges.** A conditional branch's edge to a block with several
   predecessors gets an empty block of its own (placed right after the
   branch), so every edge has a place for moves. An indirect jump's edges
   cannot be split; values live across them keep one location.
2. **Numbering.** Machine instructions are numbered in block layout order;
   instruction *i* reads its uses at position `2i` and writes its defs at
   `2i+1`, so a value that dies at an instruction can share a register with
   the value it defines (x86 reads all inputs before writing outputs). Moves
   the allocator inserts go *between* instructions, so intervals are split
   only at even positions.
3. **Liveness and dead code.** Each block's live-in set is a sorted list
   of the vregs live into it, solved one vreg at a time: from each block
   that reads the vreg before writing it, a backward walk over the
   predecessors marks it live into every block it reaches and stops at
   the blocks that write it (the path exploration of Brandner et al.,
   "Computing Liveness Sets for SSA-Form Programs", INRIA RR-7503, 2011,
   which needs no SSA form when it stops at every definition). That is
   the least solution of `live_in = use ∪ (live_out − def)`, `live_out =
   ∪ live_in(successor)`, which iterative dataflow reaches, in time and
   space linear in the instructions plus the total size of the live sets,
   with no fixed-point iteration. Live-out sets are not stored: a walk
   over a block gathers its successors' live-in sets into a sparse set
   (Briggs and Torczon, LOPLAS 1993) whose clearing costs its members,
   not the function's vregs. A backward walk removes pure instructions
   whose results are dead, and arithmetic whose results and flags are (the
   `mov`/`add` of a `ptr_add` a load folded), repeating until nothing
   changes.
4. **Intervals.** Each vreg's interval is a sorted list of half-open ranges
   with lifetime holes, built backward over the blocks (Wimmer and Franz,
   CGO 2010, `BuildIntervals`), plus the positions where an instruction
   needs it in a register. A read through a *memory source operand* (the
   source of `mov`, `movzx`/`movsx`, integer ALU operations, `cmp`, scalar
   SSE operations but `xorps`) is not such a position: a spilled value is
   read from its slot there.
5. **Fixed ranges.** A backward walk per block records for each physical
   register the ranges where a precolored value lives (from its `mov` def to
   its use) and the single position `2i+1` of each clobber (calls clobber all
   caller-saved registers). An interval may hold register *r* only where it
   meets no fixed range of *r*, which keeps the call, division, shift and
   string-instruction constraints and the conventions' saved sets without
   special cases.
6. **Coalescing.** Copies between vregs whose intervals do not overlap are
   joined, deepest loop first, into one interval (union-find over the
   representatives; later candidates test against the union, from the
   later of the two starts). A join rebuilds the representative's sorted
   ranges and uses only from just before the joined interval's first one,
   so a chain of block-argument copies, which joins one statement's
   interval after another to the end of a growing one, costs each join the
   joined interval rather than the whole union. The copy
   becomes a move between a register and itself, which the rewrite deletes;
   `mov t, i; add t, 1` and a back edge's `i := t` become one `add` in place.
7. **Scan.** A binary heap orders intervals by start. *Active* intervals hold
   a register at the current position, *inactive* ones are in a hole. The
   current interval takes a register free for its whole life (its hint
   first: a register a copy connects it to, or the previous piece's), else
   the register free longest, splitting itself where that register is next
   needed, preferably at a block boundary of a shallower loop. When none is
   free, the register whose occupants are next needed latest is taken: an
   occupant used in the innermost loop around the position and live around
   its back edge counts as needed at the loop's end. The occupants leave the
   register at the shallowest boundary after their last use, wait in their
   spill slots, and return to the heap just before their next use that needs
   a register (hoisted to a shallower block boundary when there is one). If
   the current interval is needed later than every occupant, it is the one
   spilled up to its first use. A piece that must keep one location
   (across an indirect jump) or that no register can be taken for stays in
   its slot and goes through the scratch registers (`%r11`/`%r10`,
   `%xmm15`/`%xmm14`, still reserved), a fallback that never fails: a value
   an instruction only defines shares a scratch register with one it only
   reads, and an instruction reading three spilled general registers (an
   indexed store's base, index and value) first computes its address into
   `%r11` with a flag-preserving `lea`.
8. **Resolution.** Each vreg's pieces become `(from position, location)`
   lists. A move goes where a split falls inside a block while the value is
   live on both sides, and on each edge whose ends disagree: at the start of
   a successor with one predecessor, else at the end of a predecessor with
   one successor (split moves before edge moves there). The moves at one
   place form a parallel copy, sequentialized with a scratch register for
   cycles.
9. **Rewrite, check and cleanup.** Vregs become their locations at each
   instruction and the moves go in. Debug builds (so every test) then run
   the **symbolic checker** (`regalloc/checker.rs`): an abstract
   interpretation of the rewritten function against the virtual-register
   program, as in `regalloc2`, but over the final machine code. Its state is
   the set of facts "location *L* holds the current value of *v*"; a program
   instruction must be the original with registers replaced and read
   locations holding the values the original read, an inserted instruction
   must be a whole-register move or the `lea` of an indexed address (whose
   destination then holds the symbolic address `b + i × s`, stale once `b`
   or `i` is redefined, which the instruction using `d(r)` for `d(b, i, s)`
   must find in `r`), and states meet by intersection at joins
   until a fixed point. Facts about values the program no longer reads
   (by the program's own liveness) are dropped as they die, which changes
   no verdict but keeps the state as small as the live set, so checking
   is linear in the function's length for a bounded live set, and its
   liveness is the allocator's sparse one. Then identity copies go, a copy
   followed by an addition (or a `lea` followed by one) becomes one `lea`
   where the flags are dead, a copy straight back after a copy goes,
   blocks holding only a `jmp` are threaded away, and a `jmp` to a block
   of at most eight instructions that ends in a conditional branch takes
   a copy of it when the copy's branch can fall through or take a loop's
   back edge (loop rotation: the back edge tests the condition itself).

Allocation order: caller-saved registers first (no save cost) — `%rax %rcx
%rdx %rsi %rdi %r8 %r9` on System V, `%rax %rcx %rdx %r8 %r9` on Win64 — then
callee-saved `%rbx %r12 %r13 %r14 %r15 %rbp` (plus `%rsi %rdi` on Win64).
The frame pointer is omitted, so `%rbp` is allocatable. SSE registers go
`%xmm0`-`%xmm13`; on Win64 `%xmm6` and up are callee-saved and saved by the
frame when written. x87 values are never allocated (see "Legalization").

```rust
/// One piece of a vreg's lifetime (`regalloc/intervals.rs`).
struct Interval<'a> {
    vreg: VReg, class: RegClass,
    ranges: ArenaVec<'a, (u32, u32)>,   // sorted, disjoint, half-open
    uses: ArenaVec<'a, u32>,             // positions that need a register
    from: u32,                           // split position this piece starts at
    loc: Loc, hint: Option<PReg>,
    carried: bool,                       // live across `from` inside a block
    unsplittable: bool,                  // live across an indirect jump
}
enum Location { None, Reg(PReg), Spill(u32) }
struct Allocation<'a> {
    rep: ArenaVec<'a, VReg>,                 // coalescing representatives
    pieces: ArenaVec<'a, (u32, Location)>,   // per representative, by position
    edits: ArenaVec<'a, Edit>,               // moves by block, instruction, phase
    spill_slots: u32,
    fallbacks: u32,
}
```

**Stage 1** (replaced) scanned one interval per vreg from its first to its
last position (Poletto and Sarkar, TOPLAS 1999), with hints, and spilled
whole intervals, reloading every use through the scratch registers.

Still open behind the same interface:

- *Rematerialization* instead of spilling. Constants and most addresses are
  already rematerialized structurally (the selector never allocates them);
  the hoisted symbol bases of indexed operands are spilled and reloaded
  like any value.
- *Stop reserving the scratch registers.* The scan's fallback still needs
  them; with memory operands and splitting it is rare.
- *Value-based coalescing* of copies whose intervals overlap while both hold
  the same value.
- *`regalloc2`.* The `regalloc2` crate (Cranelift's allocator) was evaluated
  against the arena rules: it allocates its working state from the global
  allocator, takes its input through a trait over `Vec`-backed structures and
  recurses in places, so adopting it would need an `#[expect]`-wrapped
  boundary and an exception to `tests/allocation_count.rs`. Its
  `Function` trait was a good model for keeping our own allocator
  replaceable, and its checker the model for ours.

### Frame layout

The frame is fixed-size in stage 1 (no dynamic allocation), addressed from
`%rsp`, with no frame pointer:

```text
 higher addresses
 ┌──────────────────────────────┐
 │ incoming stack arguments     │  SysV: 8(%rsp) at entry; Win64: 40(%rsp) at entry
 │ Win64: home (shadow) area    │  32 bytes the caller reserved, 8(%rsp) at entry
 │ return address               │  0(%rsp) at entry
 ├──────────────────────────────┤  CFA = entry %rsp + 8
 │ callee-saved pushes          │  8 bytes each
 │ padding to 16                │
 │ IR stack slots               │  each at its alignment (≤ 16 in stage 1)
 │ spill slots                  │  8 bytes each
 │ outgoing stack arguments     │  max over calls
 │ Win64: home area for callees │  32 bytes if the function calls
 └──────────────────────────────┘  %rsp after the prologue, 16-byte aligned
 lower addresses
```

The size `N` subtracted after the pushes makes `entry_rsp - 8·pushes - N`
16-byte aligned: at entry `%rsp + 8` is a multiple of 16 (psABI §3.2.2;
Microsoft "x64 stack usage": "The stack will always be maintained 16-byte
aligned, except within the prolog"). Every call is made at that alignment.

- **System V red zone.** A leaf function may use the 128 bytes below `%rsp`
  without moving it (psABI §3.2.2). A function whose frame below the pushes
  is at most 128 bytes keeps it there, laid out as above but addressed
  `size` bytes lower from the unmoved `%rsp`, with no `sub`/`add` and no
  CFA change after the pushes, provided it makes no call (libcalls such as
  `memmove` included), has no dynamic allocation or frame pointer, names
  `%rsp` in no instruction (`stack_save`), has no `va_start` save area and
  no slot aligned above 16. Win64 has no red zone ("x64 ABI conventions":
  memory below `%rsp` is volatile), so its frames always move `%rsp`.
- **Win64 shadow space.** A caller reserves 32 bytes above its outgoing
  arguments for the callee to home `%rcx`, `%rdx`, `%r8`, `%r9`; every
  function that calls reserves it, even for callees with fewer parameters
  ("x64 calling convention", "Parameter passing"; "x64 stack usage").
- **Stack probes.** Windows commits stack pages through a guard page, so a
  frame of 4096 bytes or more must touch each page in order. The prologue then
  becomes `movl $N, %eax; callq ___chkstk_ms; subq %rax, %rsp` on Windows GNU
  (`__chkstk` with MSVC), as Clang emits. System V needs no probe.
- **Callee-saved registers.** Pushed in the prologue in a fixed order and
  popped in reverse. Win64's XMM6-XMM15 are callee-saved too; they matter only
  once floats are selected (stage 3, saved with `movaps` into the frame and
  described with `.seh_savexmm`).
- **Over-aligned slots** (above 16) were planned with a realigned frame and
  a frame pointer (`and $-align, %rsp` after `mov %rsp, %rbp`). As built
  (stage 2, 2026-10-18), such a slot instead gets `align - 16` spare bytes
  and its address is computed, not folded: `lea` of the slot, `add
  $align-1`, `and $-align`, into a register every use goes through. The frame,
  its unwind information and `%rbp` stay as they are; stack arguments aligned
  above 16, which need the caller's stack realigned, are still refused.

**Prologue and epilogue**:

```text
f:                                   ; ELF                        ; COFF
    .cfi_startproc                                                 .seh_proc f
    pushq %rbx
    .cfi_def_cfa_offset 16                                         .seh_pushreg %rbx
    .cfi_offset %rbx, -16
    subq $N, %rsp
    .cfi_def_cfa_offset 16+N                                       .seh_stackalloc N
                                                                   .seh_endprologue
    ...
    .cfi_remember_state
    addq $N, %rsp
    .cfi_def_cfa_offset 16
    popq %rbx
    .cfi_def_cfa_offset 8
    retq
    .cfi_restore_state
    ...
    .cfi_endproc                                                   .seh_endproc
```

Each IR `return` gets its own epilogue. On ELF the CFA rules after it are
restored with `.cfi_remember_state`/`.cfi_restore_state` so code after an
early return keeps correct unwind information. On Win64 the unwinder
recognizes epilogues by their exact shape ("x64 prolog and epilog": `add rsp,
constant` or `lea rsp, [frame + constant]`, then `pop` of non-volatile
registers, then `ret`), which the emitter produces verbatim.

**Unwind information.** Stage 1 emits assembler directives and lets the
assembler build the tables: `.cfi_*` become the `.eh_frame` CIE/FDEs (LSB
§10.6), `.seh_*` become `UNWIND_INFO` records in `.xdata` and
`RUNTIME_FUNCTION` entries in `.pdata` (PE Format; "x64 exception handling").
The direct object writer of stage 5 generates the same records itself from the
frame layout: one `UWOP_PUSH_NONVOL` per push, `UWOP_ALLOC_SMALL` (8-128
bytes) or `UWOP_ALLOC_LARGE` for the `sub`, and for ELF a CIE with
`DW_CFA_def_cfa: rsp+8`, `DW_CFA_offset: rip at cfa-8`, then per FDE
`DW_CFA_advance_loc` + `DW_CFA_def_cfa_offset`/`DW_CFA_offset` per prologue
instruction. Unwind tables are required on Win64 for any function that calls,
pushes or allocates (the OS unwinder walks frames for exceptions and
`longjmp`), and on ELF for backtraces and C++ interoperability.

### Calls

The selector maps the post-ABI signature onto the convention:

| | System V (psABI §3.2.3) | Win64 ("x64 calling convention") |
| --- | --- | --- |
| integer/pointer arguments | `%rdi %rsi %rdx %rcx %r8 %r9`, then stack | `%rcx %rdx %r8 %r9` by position, then stack |
| stack arguments | 8-byte slots from `0(%rsp)` at the call | 8-byte slots from `32(%rsp)` at the call, after the home area |
| integer result | `%rax` (second eightbyte `%rdx`) | `%rax` |
| caller-saved | `%rax %rcx %rdx %rsi %rdi %r8-%r11` | `%rax %rcx %rdx %r8-%r11` |
| callee-saved | `%rbx %rbp %r12-%r15` | `%rbx %rbp %rdi %rsi %r12-%r15` |
| variadic call | `%al` = number of vector registers used (0 for integer-only calls) | variadic floats are duplicated in the integer register; nothing extra for integers |
| narrow integers | `signext`/`zeroext` arguments widened by the caller to 32 bits; results by the callee | only `_Bool` (`zeroext`) |

A call is selected as: `byval` copies into the outgoing area, stack argument
stores, moves of register arguments into their physical registers (extended
per the signature's `signext`/`zeroext`), `movb $0, %al` for a System V
variadic callee, the `call` (whose `uses` are the argument registers and whose
`clobbers` are the caller-saved set), and a move of `%rax` into the result
vreg. Order matters only in that copies that use fixed registers (`rep movsb`
or library calls for `byval` copies) come before argument registers are
loaded; the allocator keeps argument vregs out of `%rdi`/`%rsi`/`%rcx` across
them through the fixed ranges.

Post-ABI aggregate kinds:

- `sret(size, align)`: parameter 0 is an ordinary pointer argument in the first
  integer register; the callee also returns it in `%rax` (psABI §3.2.3,
  "Returning of Values"; Microsoft "Return values").
- `byval(size, align)` (System V MEMORY arguments): the caller copies the
  object to the argument's stack position (aligned as the layout says); the
  callee's parameter is the address of that incoming stack area.
- `registers(t1[, t2])`: parameter 0 is a return area that is *not passed*. The
  callee allocates it as a frame slot (`ResultRegisters::area_bytes`, its
  alignment) and at `return` loads `%rax` (and `%rdx`) from offsets 0 and 8; the
  caller stores `%rax`/`%rdx` into the area it passed after the call. Stage 1
  supports integer parts; stage 3 adds SSE parts (`%xmm0`/`%xmm1`), a whole
  `%xmm0` for `__float128` (System V) and `__int128` (Win64), and `%st0`
  (and `%st1` for a complex `long double`) for X87 results.

Wide scalars that ABI lowering leaves as they are travel as LLVM's code
generator passes them, which is what Clang's objects expect: under System V
an `i128` in a register pair or a 16-byte-aligned stack slot, an `f80` always
in a 16-byte stack slot and returned in `%st0`, an `f128` in a whole SSE
register (or a 16-byte stack slot) and returned in `%xmm0`; under Win64 an
`f80` or `f128` argument as a pointer to a 16-byte copy, and its result
through a hidden pointer passed first and returned in `%rax`. Runtime routines
take them the same way, except that Win64 routines take an `i128` by
reference and return it in `%xmm0` (`__fixdfti`, `__divti3`).

**Variadic definitions** (stage 2). After ABI lowering only `va_start`,
`va_copy` and `va_end` remain (ABI lowering expanded `va_arg` into loads from
the `va_list` the target defines). System V: the prologue stores the six
integer registers and, guarded by `test %al, %al`, the eight vector registers
into a 176-byte register save area, and `va_start` initializes `gp_offset`,
`fp_offset`, `overflow_arg_area` and `reg_save_area` (psABI §3.5.7, Figure
3.33). Win64: the prologue homes `%rcx`-`%r9` into the caller's home area, so
all arguments are contiguous 8-byte slots, and `va_start` stores the address
of the first variadic slot ("x64 calling convention", "Varargs").

**Calls that may return twice.** A direct call to a function the module
flags `returns_twice` (`Module::returns_twice`: `setjmp`, `_setjmp`,
`sigsetjmp`, `vfork` and attributed functions, or an alias of one) is an
ordinary call whose `MInst::Call` carries `returns_twice`, as is every
indirect call in a module where such an address escapes
(`Module::returns_twice_escapes`, computed once per module). The second
return resumes after the call with `%rsp` and the callee-saved registers as
they were at the first (`longjmp` restores them; on Win64 the unwinder
does), and nothing lives in a caller-saved register across any call. Memory,
however, is as the program left it, so a value live across the call must not
share its spill slot: coalescing would join it with a copy defined later
(the copy of an unchanged local into a variable the loop after `setjmp`
changes) whose definitions then overwrite the slot the second return reads.
The allocator therefore leaves every register live across such a call out
of coalescing (`Liveness::live_across_returns_twice`), and since every
representative has its own spill slot, its slot holds only its own value.
Values defined again after the call, block parameters of a loop around
`setjmp` that C variables changed between `setjmp` and `longjmp` become,
are indeterminate after the second return (C99 §7.13.2.1p3), as with
LLVM's `returns_twice`.

GCC's `__builtin_setjmp` (`sjlj_setjmp`) saves no register at all:
`__builtin_longjmp` (`sjlj_longjmp`) restores only `%rbp` and `%rsp` from
words 0 and 2 of the buffer and jumps to the resume address in word 1
(LLVM's `EH_SjLj_SetJmp` and `EH_SjLj_LongJmp`). `MInst::SjljSetjmp`
therefore clobbers every register but `%rsp`, and `%rbp` unless it
addresses the frame: nothing lives in a register across it, the prologue
saves every callee-saved register, and the same coalescing rule keeps each
value live across it in its own spill slot. At the resume label a frame
addressed from `%rbp` derives `%rbp` from the frame address it arrives
with, a fixed distance above the frame's base.

### Atomic operations

`isel/atomic.rs` selects the IR's `atomic` instructions with the standard
mapping of the C11 memory orders onto x86-64 ("C/C++11 mappings to
processors", x86 column; GCC and LLVM emit the same). x86 already gives
every load acquire and every store release semantics. Only a later load may
pass an earlier store to another location, so sequential consistency puts
the full barrier on the store:

| IR operation | x86-64 |
| --- | --- |
| `load`, any order | `mov` (`movzx` below 32 bits) |
| `store`, relaxed or release | `mov` |
| `store`, `seq_cst` | `xchg` |
| `exchange` | `xchg` |
| `compare_exchange` (strong; weak may be strong) | `lock cmpxchg` through `%rax` |
| `add`, `sub` | `lock xadd` (`neg` first for `sub`) |
| `add`, `sub`, `and`, `or`, `xor`, old value unused | `lock add`, `sub`, `and`, `or`, `xor` |
| `and`, `or`, `xor`, `nand`, `min`, `max`, `umin`, `umax`, `fadd`, `fsub` | a `lock cmpxchg` loop |
| `thread_fence seq_cst` | `mfence` |
| other fences, `signal_fence` | nothing |

The loop loads the old value, computes the new one (`cmp` and `cmov` at 32
bits or more for minimum and maximum, `addsd`/`subsd` through `movq` for
floats), and retries `lock cmpxchg` until it succeeds. It is the one
selection that splits an IR block: the loop and the rest of the block are
new machine blocks at the end of the layout. Weaker fences emit nothing,
because the selector never moves a memory access past another instruction.

`i128` operations call libatomic's `__atomic_load_16`, `__atomic_store_16`,
`__atomic_exchange_16`, `__atomic_compare_exchange_16` (expected value
through a 16-byte temporary) and `__atomic_fetch_{add,sub,and,or,xor,nand}_16`.
The byte-buffer operations call the generic `__atomic_load`, `__atomic_store`,
`__atomic_exchange` and `__atomic_compare_exchange`. Both pass C's
`memory_order` constants, as GCC does and as LLVM does for a CPU without
`cmpxchg16b`, which baseline x86-64 lacks. The driver links `-latomic` when
the listing calls one of them. The support check refuses `fmin`/`fmax`
(IEEE `minNum` NaN handling has no single instruction), `i128` minimum and
maximum (libatomic has no routine), and accesses aligned below their size,
which a locked instruction would split across cache lines.

### Poison and undefined behaviour at the machine level

The machine is a refinement of the IR's semantics. The rules:

- A register always holds concrete bits, so **poison refines to an arbitrary
  value**, and `freeze` is a copy. The only obligation is consistency: every
  use of one frozen value must see the same bits. The back end never
  recomputes a `freeze` operand per use, and the one leaf it rematerializes
  that stands for an arbitrary value, `poison`, always materializes as 0.
- `nsw`, `nuw`, `exact` and `inbounds` are dropped: violating them produces
  poison, which any concrete result refines.
- Shift amounts at or above the width produce poison in the IR; the hardware
  masks the count, producing some value, which refines it.
- Division by zero and signed `INT_MIN / -1` are immediate UB in the IR; the
  hardware raises #DE, which is one possible behaviour of UB.
- Branching on poison is UB; the machine branches on whatever bits the
  register holds.
- The unspecified upper bits of narrow registers are never observed: every
  operation that reads them extends first, and calls extend exactly as the
  signature's `signext`/`zeroext` say, which is what other compilers rely on.
- `noundef` adds no machine obligation; it only tells optimizers what they
  may assume.
- `volatile` loads and stores are emitted once each at their exact width; the
  back end never merges, splits, removes or reorders memory operations.
- `trap` and `unreachable` become `ud2`, so falling into code the IR says is
  unreachable stops rather than running into the next function.

### Object emission

**Stage 1: assembly text** in AT&T syntax, assembled and linked by the bundled
clang (`clang --target=<triple> [--sysroot=<mingw>] -fuse-ld=lld x.s -o x`, or
`-c` for an object), exactly as the LLVM back end links today
(`Toolchain::bundled`). AT&T rather than Intel because it is LLVM MC's native
x86 syntax: Intel syntax under `.intel_syntax noprefix` is ambiguous for
symbol names that spell registers or keywords (`rax`, `offset`), which C
identifiers may. Symbols that are not plain identifiers are quoted (`"name"`).

Sections and symbols per format:

| | ELF | COFF |
| --- | --- | --- |
| code | `.text`; `.type f,@function`, `.size f,.-f` | `.text`; `.def f; .scl 2 (3 static); .type 32; .endef` |
| zero data | `.bss` | `.bss` |
| initialized data | `.data` | `.data` |
| constants | `.section .rodata`; `.data.rel.ro` if they hold addresses, which a PIE relocates at load time | `.section .rdata,"dr"` |
| address constants | `.quad sym+addend` (`R_X86_64_64`) | `.quad sym+addend` (`IMAGE_REL_AMD64_ADDR64`) |
| block label addresses | `.quad .LBA<f>_<l>+addend`; a label difference is `.byte`/`.short`/`.long`/`.quad .LBA<f>_<a>-.LBA<f>_<b>+addend`, resolved by the assembler | same |
| thread-local data | `.section .tdata,"awT",@progbits` or `.tbss,"awT",@nobits` | `.section .tls$,"dw"` |
| weak definition or declaration | `.weak sym`; a declaration is reached through the GOT | `.weak sym`; a declaration is reached through a `.refptr.sym` stub (`movq .refptr.sym(%rip)`, `callq *.refptr.sym(%rip)`) |
| alias | `.globl a` (or `.weak`), `.type a,@function` for a function, `a = target` | the same, with `.def a` for a function |
| named section | `.section name,"ax"`/`"aw"`/`"a",@progbits` | `.section name,"xr"`/`"dw"`/`"dr"` |
| visibility | `.hidden sym`, `.protected sym` | — |
| constructors, destructors | `.init_array[.prio]`, `.fini_array[.prio]` | MinGW `.ctors[.65535-prio]`, `.dtors[...]`; MSVC `.CRT$XC*`, `.CRT$XT*` |
| defined data reference | `sym(%rip)` (`R_X86_64_PC32`) | `sym(%rip)` (`IMAGE_REL_AMD64_REL32`) |
| undefined data reference | `movq sym@GOTPCREL(%rip), %r` (`GOTPCRELX`) | MinGW: `movq .refptr.sym(%rip), %r` through a `.refptr.sym` stub, as Clang emits it, but `sym(%rip)` for hidden or protected data; MSVC: `sym(%rip)` |
| direct call | `call f` / `call f@PLT` if undefined (`PLT32`) | `call f` (`REL32`; import thunks for DLL functions) |
| function address | `leaq f(%rip)` / `movq f@GOTPCREL(%rip)` if undefined | `leaq f(%rip)` |
| non-executable stack | `.section .note.GNU-stack,"",@progbits` | — |

Clang's default executables on Linux are position-independent, which is why
undefined data and function addresses go through the GOT. On Windows GNU,
`main` begins with `call __main` (MinGW's constructor runner), as Clang and
GCC emit it.

**Later: a direct object writer** (stage 5) replaces the text and the external
assembler. It has three parts behind the same MIR-after-allocation input:

1. An **encoder** from MIR to bytes: REX/ModRM/SIB encoding, a fixup list of
   `(offset, kind, symbol, addend)`, and branch relaxation (start every branch
   short, iterate enlarging the ones out of range until a fixpoint, which
   terminates because sizes only grow).
2. **Unwind generation** from the frame layout, as described above.
3. **ELF and COFF writers**: sections (`.text`, `.data`, `.rodata`/`.rdata`,
   `.bss`, `.eh_frame` with `.rela.eh_frame`, or `.pdata`/`.xdata`), the symbol
   and string tables, and relocations (`R_X86_64_PC32`/`PLT32`/`GOTPCRELX`/`64`;
   `IMAGE_REL_AMD64_REL32`/`ADDR64`/`ADDR32NB`). The output still links with
   bundled LLD.

The encoder is validated against the assembler: assemble stage 1's text with
clang and compare the complete section bytes and structural records with
independent ELF/COFF parsers on every test program and torture source.
When available, `llvm-objdump -d -r` or GNU `objdump` adds a disassembly
cross-check; its absence prints an explicit skip line. An in-process alternative available before stage 5 is to hand the
assembly text to LLVM MC as module-level inline assembly through LLVM-C
(`LLVMSetModuleInlineAsm2` on an empty module, then the existing object
emission), which removes the clang process for `-c` but not the dependency.

Debug information is a non-goal for now. When it comes, DWARF line tables
(`.loc` directives in text first) on ELF and CodeView `.cv_loc` on COFF need
only an IR-instruction-to-source map from lowering; variable locations need
the allocator's `Allocation` per instruction range.

## Source layout

Following [CONTRIBUTING.md](../../CONTRIBUTING.md#source-layout):

| File | Holds |
| --- | --- |
| `src/backend/native.rs` | the module map; `emit_assembly`; the per-function loop |
| `native/support.rs` | the up-front support check and `Unsupported` |
| `native/conv.rs` | `CallConv`: argument and result registers, saved sets, shadow space |
| `native/mir.rs` | the machine IR: registers, operands, instructions, blocks |
| `native/isel.rs` | instruction selection, with `isel/calls.rs` (calls, returns, entry, `va_start`), `isel/edges.rs` (terminators and parallel copies), `isel/memory.rs` (addresses, loads, stores, `copy`, `fill`), `isel/float.rs` (SSE), `isel/wide.rs` (`i128` pairs), `isel/x87.rs` (`f80`), `isel/quad.rs` (`f128` and the conversions that call the runtime) and `isel/stack.rs` (dynamic allocation) |
| `native/regalloc.rs` | the allocator's entry and `Allocation`, with `regalloc/edges.rs` (critical edges, jump threading, loop rotation), `regalloc/liveness.rs` (liveness, dead code, intervals, fixed ranges), `regalloc/intervals.rs`, `regalloc/coalesce.rs`, `regalloc/scan.rs` (the split-interval scan), `regalloc/resolve.rs` (pieces and moves), `regalloc/rewrite.rs` (physical rewrite and parallel moves), `regalloc/checker.rs` (the symbolic checker) and `regalloc/peephole.rs` (`lea` combining) |
| `native/frame.rs` | frame layout, prologue and epilogue |
| `native/emit.rs` | AT&T text: instructions, functions, globals, symbols |
| `native/driver.rs` | the choice of object writer, assembly or direct object output, and bundled clang linking |
| `native/object.rs`, `native/object/` | private listing parser, x86 encoder and relaxation, ELF64 writer, DWARF CFI records; the COFF writer and its unwind tables in `object/coff.rs` and `object/coff/` |
| `native/tests.rs`, `native/tests/` | selection, allocation and end-to-end tests |

Arenas: the module's `'ir` arena outlives the back end; each function is
selected, allocated and emitted in a scratch `Bump` that is reset between
functions, so peak memory is the largest function's MIR, not the module's.
The assembly text accumulates in one `ArenaString` in a module-lifetime arena.

## Testing strategy

1. **The interpreter is the oracle.** Every end-to-end test runs the same
   post-ABI module in the interpreter and as a native executable and compares
   exit status and standard output. A program the interpreter traps on is not
   a test of the back end.
2. **Selection and allocation unit tests** parse textual IR, select, allocate
   and check properties of the result: the instructions chosen for each opcode
   family, that a value live across a call gets a callee-saved register or a
   spill slot, that pressure above the register count spills without
   overlapping, that block-parameter swaps go through a temporary, and that
   the allocation checker accepts every function of the test programs (it
   runs on every function in debug builds) and rejects allocations broken
   on purpose.
3. **End-to-end programs** (`native/tests/programs.rs`): small C programs
   covering arithmetic at every width, signed and unsigned division and
   shifts, comparisons, loops with block parameters, recursion, calls with
   more arguments than registers in both directions, globals with relocated
   initializers, `printf`, structure copies, and pressure that forces spills.
   They run natively on Windows and, compiled with
   `--target=x86_64-unknown-linux-gnu`, under WSL, where WSL's clang links the
   assembly with glibc.
4. **Differential testing.** `src/backend/difftest` generates random
   verifier-clean programs and checks the optimizer and LLVM arms against the
   interpreter. A native oracle compiles each kept case that the native back
   end accepts (integer-only ones in stage 1), unoptimized (B0) and after the
   default pipeline (B1), and compares like the LLVM oracle; refusals are
   counted, never failures.
5. **The GCC torture survey.** `scripts/run_gcc_torture_execute.py` gains
   arms B0 and B1 and an `--arms` filter, and the classification
   `native-unsupported` for refusals. The acceptance rule for every stage is
   zero `wrong-result` in a native arm where A0 passes, and zero compiler
   crashes; refusals shrink stage by stage.
6. **ABI conformance.** `src/abi/tests/conformance.rs` links bcc-compiled C
   with Clang-compiled C exchanging every supported shape in both directions;
   a native variant links B0 objects instead of LLVM ones, which is the real
   test that the selector maps post-ABI signatures exactly as Clang does.
7. **Benchmarks.** `scripts/compare_optimizers.py` times B0 and B1 next to the
   LLVM arms on `benches/c` once floats are supported (most benchmarks use
   `double`).

## Staged roadmap

Sizes are estimated Rust lines including tests.

| Stage | Content | Size | Acceptance | Status |
| --- | --- | ---: | --- | --- |
| 0 | This design | — | reviewed | done |
| 1 | Integer and pointer code: arithmetic, comparisons, branches, switch chains, block parameters, direct and indirect calls (SysV and Win64, stack arguments, calls to variadic functions), stack slots, loads and stores, globals with relocated initializers, `copy`/`fill` inline or through `rep movsb`/`memmove`, bit operations, `sret`/`byval`/integer `registers`; linear scan with spilling; assembly text through clang; `--backend=native`, `--emit=asm`; arms B0/B1 in the torture survey | 5-6k | e2e programs match the interpreter on Windows and WSL; difftest native oracle clean; torture survey: zero native wrong results where A0 passes, zero crashes | done |
| 2 | Variadic definitions (`va_start` save areas), `dyn_alloca`/`stack_save`/`stack_restore` with a frame pointer, inline `copy`/`fill` for small constant sizes; over-aligned slots and jump tables for dense switches | 2k | torture: every integer-only source A0 passes also passes B0 | done except jump tables (and stack arguments aligned above 16) |
| 3 | Floats: an XMM register class in the allocator, SSE selection for `f32`/`f64`, conversions, `fcmp` with parity, SSE `registers` returns and variadic float arguments (`%al` count, Win64 duplication), `i128` register pairs and division libcalls; `f80` through x87 memory sequences and `f128` libcalls | 4-5k | torture: B0 passes ⊇ A0 passes except documented x87 gaps; `benches/c` run in B0/B1 | done |
| 4 | Quality: split intervals and second-chance binpacking or a greedy allocator, coalescing, memory operands for spills, addressing modes with index and scale, LEA arithmetic, red zone, loop rotation, compare/branch fusion across blocks, symbolic allocation checker | 4k | B1/A1 geometric mean within 1.5× on `benches/c` (native Linux), allocation checker clean on the whole torture corpus | done, with the red zone and loop alignment added after stage 5 (#85); B1/A1 1.05× on native Linux (from 1.42×), 1.03× with those two in a pinned run; checker clean on every test and on the whole torture corpus (1,890 sources, debug builds, no checker failure) |
| 5 | Direct object emission: encoder with relaxation, ELF and COFF writers, unwind table generation | 4-5k | objects byte-equivalent in disassembly and relocations to clang-assembled text on all tests; compile time measured against stage 1 | done: direct COFF objects are the Windows default and ran in native Windows CI; direct ELF objects stay opt-in (`--native-object`) |
| 6 | Debug line tables (DWARF `.debug_line`, CodeView) | 2k | gdb/lldb and WinDbg step through test programs | open |

## Implementation status

### Stage 1

The prototype in `src/backend/native/` implements stage 1 as listed above,
with the following decisions recorded for review:

- The selector is a macro expander with the three peepholes; there is no
  pattern matcher.
- `%r10` and `%r11` are reserved as spill scratch registers.
- Every argument-carrying edge out of a multi-successor block is split.
- `copy` and `fill` of up to 64 constant bytes are moves; longer ones use
  `rep movsb`/`rep stosb`, or `memmove` when they may overlap; a module that
  defines a runtime name the back end calls (`memmove`, `__main`,
  `___chkstk_ms`, `fmod`, `__divti3` …) with internal linkage is refused,
  since the call would bind to it.
- Undefined data on Windows GNU is reached through a `.refptr.sym` stub
  (a discardable `.rdata$.refptr.sym` COMDAT holding `.quad sym`), as Clang
  reaches it, and only where code uses it. Such data may live in a DLL,
  which LLD's MinGW auto-import resolves with a runtime pseudo-relocation:
  on the stub that is a 64-bit absolute address, valid wherever the DLL is
  loaded, whereas a direct `sym(%rip)` operand is a 32-bit PC-relative field
  that fails once the DLL lies more than 2 GiB from the executable (#69).
  Defined data and hidden or protected declarations, which name this
  module's own data, stay RIP-relative, functions keep direct calls
  through the import library's thunks, and the LLVM back end already gets
  the same stubs from LLVM for its non-`dso_local` declarations. On MSVC,
  whose linker does not auto-import, undefined data is addressed directly
  as Clang addresses it; DLL data there needs `__declspec(dllimport)`.
- `--emit=asm` prints the native assembly and requires `--backend=native`.
- File-scope `asm` blocks are written verbatim between `# Start of file scope
  inline assembly` and `# End of file scope inline assembly` after the
  listing's first `.text`, in source order and before every function and
  object, as Clang places module assembly; `.att_syntax` follows them.
- Inline assembly (`isel/asm.rs`) never reaches the allocator as an operand
  form. Selection fixes every register operand to a physical register
  first: the one its constraint names, or a free one of its class in the
  allocation order, assigned most constrained operand first with
  backtracking, so an unrestricted output never takes the one register a
  `Q` input could use. Every operand gets its own register, except that
  an output that is not early-clobber may share an input's fixed register
  (`"=a"(eax) : "a"(leaf)`) and a tied input shares its output's. `%rsp`,
  `%rbp` and the spill scratch registers (`%r10`, `%r11`, `%xmm14`, `%xmm15`)
  are never assigned,
  and a clobber of `%rsp` or `%rbp` is refused. Inputs and memory addresses
  are materialized in virtual registers and then copied into their
  registers; a memory operand in a frame object or at a defined symbol is
  addressed directly (`16(%rsp)`, `sym(%rip)`) and needs no register, and
  operands at one address (a `+m` output and its read) share one. `MInst::InlineAsm` then reads the input registers and writes
  the output and clobbered ones, so the allocator keeps values that are
  live across it elsewhere, as at a call, and the frame saves any
  callee-saved register it writes. Right after it come `setcc` for flag
  outputs, copies of the register outputs into virtual registers, and
  stores to the output pointers. The listing writes the template between
  `#APP` and `#NO_APP`: registers at their operand's width or the `b` `h`
  `w` `k` `q` `V` `z` `a` `x` modifiers, memory as its operand (`H` adds 8),
  immediates as `$n` (`c` `P` `p` `a` without `$`, `n` negated), symbols as
  `$sym+off`, `${:uid}` as a listing-wide counter, and the AT&T alternative
  of `{att|intel}`. Other modifiers, and templates that name a flag output,
  are refused. Like GCC and Clang, a System V leaf function may keep its
  frame in the red zone, which assembly that pushes would overwrite.

- An `i1` arriving from another compiler (a parameter or a call result) is
  masked to its low bit, since the back end keeps `i1` canonical.
- `--backend=native` rejects `--llvm-opt`, `--llvm-codegen-only` and
  `--interpret` with an explanation instead of ignoring them.

Measured on 2026-10-14 (release build, GCC 15.2.0 `gcc.c-torture/execute`,
1,677 top-level sources, `--target=x86_64-w64-windows-gnu`,
`run_gcc_torture_execute.py --arms A0,B0,B1`):

| Class | A0 | B0 | B1 |
| --- | ---: | ---: | ---: |
| pass | 1364 | 1184 | 1181 |
| frontend-error | 137 | 137 | 137 |
| lowering-unsupported | 152 | 152 | 152 |
| abi-unsupported | 6 | 6 | 6 |
| native-unsupported | 0 | 182 | 181 |
| compiler-crash | 0 | 0 | 0 |
| wrong-result | 14 | 13 | 17 |
| link-error | 4 | 3 | 3 |

Every B0 wrong result or link error is also an A0 one (sources whose
expectations the survey does not apply, such as `-fwrapv` or source
encodings). The four B1-only wrong results (`980526-2`, `pr22493-1`,
`pr43629`, `pr99079`) are programs the interpreter reports as undefined
behaviour (poison branched on or divided by) in bcc's semantics, before and
after the bcc pipeline, so no behaviour is wrong; A0 passing them is
coincidence. The native refusals are floats (`f64` in 75 sources, `f32` in
31), `va_start` (31), `i128` (18, plus 5 structure results in `i128`
registers), `f80` (10), `dyn_alloca` (7), `stack_save` (4) and `va_copy`
(1), counted per source (a source can name several). Stages 2 and 3 remove
them.

The same 2,361 B0 and B1 executables that passed on Windows were rebuilt for
`x86_64-unknown-linux-gnu` and run under WSL (glibc, WSL's clang 21): 2,359
pass; the two failures (`pr31448-2`, both arms) abort in the interpreter
too, because the test assumes a 32-bit `long`. Two arms of `20081103-1` are
refused on System V only, where a structure comes back in an SSE register.

The difftest native oracle compiled 1,000 integer-only generated programs
(500 seeds, arms B0 and B1) with no disagreement and no refusal; the default
`cargo test` run checks 40.

### Stages 2 and 3

Implemented in the priority order of the stage 1 refusals:

- **`f32`/`f64`** (`isel/float.rs`): a second register class with its own
  allocation order (Win64 saves `%xmm6`-`%xmm15` it writes with `movaps` and
  `.seh_savexmm`; `%xmm14`/`%xmm15` are the SSE spill scratch registers);
  scalar SSE arithmetic; `ucomis*` comparisons mapped to flag tests that give
  the IR's ordered and unordered results for NaN (only `oeq` and `une` need two
  `setcc`s); `cvtsi2s*`/`cvtts*2si` with branch-free unsigned 64-bit paths;
  `fneg` by `xorps` with a sign-mask constant; float `select` through general
  registers and `cmov`; constants in a per-function `.rodata`/`.rdata` pool of
  16-byte entries; `frem` through `fmod`/`fmodf`. Calls place SSE arguments
  by convention, set `%al` for System V variadic callees, duplicate Win64
  variadic floats into the integer register of their position, and return
  structures in SSE registers by part class.
- **Variadic definitions** (`isel/calls.rs`): the entry saves the six integer
  and eight SSE argument registers into a 176-byte register save area (System
  V) or homes the four integer registers into the caller's home area (Win64);
  `va_start` fills the `va_list` the expanded `va_arg` reads, `va_copy`
  copies it, `va_end` is nothing.
- **`i128`** (`isel/wide.rs`): register pairs as planned, plus branch-free
  `uadd_overflow` and `umul_overflow`, the `__divti3` family (register pairs
  on System V; pointers to 16-byte copies and an `%xmm0` result on Win64), and
  Win64's `registers(i128)` results through `movups` of `%xmm0`.
- **Dynamic stack** (`isel/stack.rs`): such functions keep `%rbp = %rsp`
  after the allocation, address the fixed frame from it, allocate right above
  the outgoing argument area, probe new pages on Windows, and end with the
  frame-pointer epilogue `lea size(%rbp), %rsp`.
- **Two peepholes**: a multiply by a power of two is a shift, and `zext` of a
  narrow load reuses the load's zero-extended register.

`f80`, `f128` and conversions between `i128` and floats were left for the
stage 3 completion below.

Measured on 2026-10-14 (release build, same corpus and target as stage 1,
`--arms A0,B0,B1`; the 22 sources whose first run timed out or failed under
machine load were rerun with two jobs and a 20-second deadline):

| Class | A0 | B0 | B1 |
| --- | ---: | ---: | ---: |
| pass | 1364 | 1348 | 1344 |
| frontend-error | 137 | 137 | 137 |
| lowering-unsupported | 152 | 152 | 152 |
| abi-unsupported | 6 | 6 | 6 |
| native-unsupported | 0 | 16 | 16 |
| compiler-crash | 0 | 0 | 0 |
| wrong-result | 14 | 14 | 18 |
| link-error | 4 | 4 | 4 |

B0 has exactly A0's result on every source it compiles; its 16 refusals are
all `f80` values. B1's four extra wrong results are the four
undefined-behaviour programs listed for stage 1. Under WSL (System V), the
2,690 B0/B1 executables that passed on Windows (stage 2 build before the
peepholes) were rebuilt for `x86_64-unknown-linux-gnu`: 2,688 pass and the
two failures are `pr31448-2` again; two arms of `20000402-1` are a front-end
error on that target. The difftest oracle now runs ordinary seeds, floats
included: 1,000 runs, no disagreement, no refusal.

`scripts/compare_optimizers.py` (Windows, wall time, 7 interleaved
repetitions, 10 programs of `benches/c`, all verified against the reduced
interpreter workload) measured:

| Ratio (geometric mean) | run 1 (before the peepholes) | run 2 |
| --- | --- | --- |
| A1/N (bcc optimizer, LLVM codegen) | 0.810 [0.787, 0.834] | — |
| B1/B0 (bcc optimizer, native codegen) | 0.830 [0.807, 0.846] | 0.795 [0.719, 0.928] |
| B1/A1 (native vs LLVM codegen, same IR) | 1.395 [1.359, 1.431] | 1.250 [1.170, 1.475] |
| B0/A2 | 1.674 [1.638, 1.728] | 1.591 [1.408, 1.809] |
| B1/A2 | 1.389 [1.357, 1.423] | 1.265 [1.175, 1.447] |

The second run was noisier (another workload shared the machine); the first
is the cleaner estimate. The bcc optimizer gains about as much with the
native code generator (B1/B0) as with LLVM's (A1/N), so its gains do not
depend on LLVM cleaning up after it. The native code generator itself costs
25-40% against LLVM's O2 machine passes on the same IR. Reading the
generated code (`levenshtein`, `bytecode`, `collatz`, the largest gaps)
shows where:

1. **Whole-interval allocation.** A loop counter and its incremented copy
   overlap for the whole loop, so `i + 1` becomes `mov; add; mov` instead of
   one `add` in place; live ranges with holes or splitting would coalesce
   them.
2. **No indexed addressing.** `a[i]` on a global is `movslq; shl;
   leaq a(%rip); addq; mov (...)` where LLVM writes `mov (%base,%rcx,4)`
   with the base hoisted.
3. **Rematerialized addresses in loops.** `leaq sym(%rip)` is repeated at each
   use instead of living in a register across the loop.
4. **No loop rotation.** Loops test at the top and jump back from the bottom,
   two branches per iteration.
5. **Compare results through `setcc`** feed arithmetic (`a != b` added to an
   integer) where `sbb`/`adc` idioms are shorter.

Stage 4 addressed the first four (see "Stage 4: register allocation and
code quality"); the fifth is still open.

Compile time is comparable: median whole-process build 1.29-1.51 s for B0/B1
against 1.09-1.25 s for N/A1, the difference being the extra clang run that
assembles the text.

### IR forms from the optimizer and front-end stack

Rebasing onto #63 brought IR forms the back end had not seen. Each is
compiled, not refused:

- **Computed `goto`** (`block_addr`, `indirect_br`, label and label-difference
  relocations): local labels `.LBA<function>_<label>`, `jmpq *reg`, and
  landing blocks for labeled blocks with parameters (see "Block parameters
  become parallel copies").
- **Thread-local globals**: ELF local-exec for definitions and initial-exec
  for declarations through `%fs:0`; Windows through `_tls_index` and the
  TEB's `ThreadLocalStoragePointer`; `.tdata`/`.tbss` and `.tls$`.
- **Symbol attributes** (`weak`, aliases, `section`, `used`, visibility,
  function alignment, constructors and destructors with priorities) as the
  directives in "Object emission"; a thread-local global in a named section
  is the one refusal.
- **`return_address` and `frame_address`** at level 0, from the CFA.

Measured on 2026-10-15 (release build at #63's head 513a78cb plus this
branch, same corpus and target as before, `--arms A0,A1,B0,B1`, six jobs):

| Class | A0 | A1 | B0 | B1 |
| --- | ---: | ---: | ---: | ---: |
| pass | 1396 | 1385 | 1380 | 1375 |
| frontend-error | 122 | 122 | 122 | 122 |
| lowering-unsupported | 139 | 139 | 139 | 139 |
| abi-unsupported | 6 | 6 | 6 | 6 |
| native-unsupported | 0 | 0 | 16 | 16 |
| compiler-crash | 0 | 0 | 0 | 0 |
| wrong-result | 13 | 24 | 13 | 17 |
| timeout | 0 | 0 | 0 | 1 |
| link-error | 1 | 1 | 1 | 1 |

B0 has A0's result on every source except the same 16 `f80` refusals; no new
refusal appeared. The 10 torture sources with computed `goto` that the front
end now lowers pass in all four arms (8 more stay `lowering-unsupported` or
`frontend-error`); no top-level torture source uses thread-local storage.
B1 differs from B0 on the four undefined-behaviour programs of stage 1
(`980526-2`, `pr43629`, `pr99079`, and `pr22493-1`, now a timeout) and on
`frame-address`, which fails in A1 as well, so the bcc pipeline, not the
native back end, changes it.

### Stage 3 completion: wide floats

Done on 2026-10-18 in `isel/x87.rs` and `isel/quad.rs`, with the calling
convention parts in `isel/calls.rs`:

- **One representation for the three wide scalars.** `i128`, `f80` and
  `f128` values all live in a pair of general registers, low eight bytes
  first; an `f80`'s high register holds its sign and exponent in the low 16
  bits, the rest unspecified like a narrow integer's. Block parameters,
  parallel copies, spills, `select`, `freeze`, loads and stores (ten bytes
  for `f80`), `fneg` (an `xor` of the sign bit) and bit casts between `i128`
  and `f128` (aliases) therefore needed nothing new, and the allocator still
  has two register classes. Constants of all three are 16-byte entries of the
  function's constants, from which `fldt` reads directly.
- **`f80` through x87 memory sequences.** Each operation stores its operands
  to a 32-byte, 16-aligned scratch slot made on first use, pushes them with
  `fldt`, computes with `fadd`/`fsub`/`fmul`/`fdiv %st(1), %st`, and pops the
  result with `fstpt` and the other operand with `fstp %st(0)`. Every sequence
  leaves the x87 stack empty, so the register stack is never allocated; only
  a System V `return` leaves its value in `%st0`, and a call's `%st0` result
  is popped right after the call (even when unused). Comparisons use
  `fucomip`, whose flags are those of `ucomisd`, so they share the SSE flag
  tests. Integer conversions use `fild`/`fistp` at 16, 32 or 64 bits, the
  stores under a control word with rounding toward zero (`fnstcw`, `or
  $0xC00`, `fldcw`, `fistp`, `fldcw`), since `fisttp` is SSE3; unsigned
  64-bit conversions add 2^64 after a negative signed load, or subtract 2^63
  before the store and pick the result with `cmov`. `frem` calls `fmodl`.
- **`f128` through runtime routines**, as LLVM lowers it: `__addtf3`,
  `__subtf3`, `__multf3`, `__divtf3`, `fmodf128`, the comparison routines
  (`__eqtf2` … `__unordtf2`; the unordered conditions negate the opposite
  ordered routine, and `ueq`/`one` add `__unordtf2`), `__extend{s,d,x}ftf2`,
  `__trunctf{s,d,x}f2`, `__float{si,unsi,di,undi,ti,unti}tf` and
  `__fix{,uns}tf{si,di,ti}`; narrower integers go through the 32-bit signed
  routines, as LLVM promotes them.
- **`i128` and floats** convert through `__float{,un}ti{sf,df,xf}` and
  `__fix{,uns}{sf,df,xf}ti`.
- **Passing**, as LLVM's code generator does and Clang's objects expect (see
  "Calls"): System V `f80` in 16-byte stack slots and `%st0`, `f128` in whole
  SSE registers (or 16-byte stack slots) and `%xmm0`, `registers(f80)`,
  complex `registers(f80, f80)` in `%st0`/`%st1` and `registers(f128)` in
  `%xmm0`; Win64 `f80` and `f128` by reference with a hidden result pointer,
  and the routines' `i128` by reference with the result in `%xmm0`. The
  System V variadic prologue now saves all 16 bytes of each SSE argument
  register, which a `__float128` argument fills. The emitted calls were
  compared with `clang -S` for both targets (`__addtf3(&result, &a, &b)`,
  `__floattidf(&x)`, `__fixdfti` returning `%xmm0`, `long double f(long
  double, long double)` reading `(%rdx)` and `(%r8)` and storing through
  `%rcx`).

Validation: the interpreter does not run `f80` or `f128`, so
`native/tests/end_to_end.rs` compares the native executables of three
`long double` C programs (both optimizer settings) and of an IR program
covering every `f80` and `f128` operation, comparison and conversion with
the LLVM back end's executables of the same modules; `i128`/float
conversions are checked against the interpreter there and in the difftest
native oracle. The ABI conformance tests link native objects with Clang's in
both directions: the structure exchange (with `struct { long double }` and
`struct { __int128 }`), complex `long double`, `long double` and
`__float128` variadic arguments and definitions, and new `long double`,
`i128`/float and `__float128` exchanges. Win64 paths are checked at the
assembly level against `clang --target=x86_64-w64-windows-gnu -S`
(`native/tests/selection.rs`), since no Windows host ran them.

The C front end still refuses `__float128` values, so `f128` arithmetic is
reachable only from IR; `f80` and `f128` `va_arg` on Win64 are refused by ABI
lowering, not here. Still refused by the back end: `i128` bit counts, byte
swaps and `switch`es, and stack arguments aligned above 16 bytes. Stack
objects aligned above 16 bytes, the stage 2 leftover, now compile with a
computed address (see "Frame layout"), checked against the interpreter by
the `over_aligned_locals` end-to-end program.

Measured on 2026-10-18 on Linux (debug build, `cargo test --all-targets
--no-fail-fast` at the conformance commit): 2,361 tests passed, none
failed, 7 ignored. The GCC torture survey was not rerun: the full
`gcc.c-torture/execute` corpus is not on the Linux host used, so the 16
`f80` refusals of the last Windows survey are expected to become passes
but are not yet measured.

### Stage 4: register allocation and code quality

Done on 2026-10-19, in the order the measurements asked for:

- **The symbolic allocation checker** came first and guards everything
  after it (`regalloc/checker.rs`, described under "Register allocation").
  Debug builds run it on every function, so every native test, the
  difftest native oracle and the ABI conformance tests run it; unit tests
  corrupt allocations on purpose (a value left in a clobbered register, two
  live values in one register or one spill slot, a value in the wrong class)
  and check it rejects them. The native tests caught one allocator bug
  during the work (a reloaded piece queued behind the scan's position),
  fixed before it was committed.
- **The allocator**: copy coalescing and linear scan over split intervals
  with lifetime holes, loop-aware split and spill positions, resolution
  moves as parallel copies, and memory operands for spilled values (see
  "Register allocation"). Fixed-register constraints (calls, divisions,
  shifts, `rep movsb`), the System V and Win64 saved sets (Win64's
  `%xmm6`-`%xmm15` saved by the frame when written) and the x87 values that
  are never allocated are unchanged.
- **Addressing modes**: memory operands take an index register and scale,
  folded from `ptr_add` chains; symbol bases of indexed operands are loaded
  once at entry; a materialized `ptr_add` is one `lea`.
- **LEA arithmetic**: 64-bit products by 3, 5 and 9, and, after allocation,
  a copy followed by a 32- or 64-bit addition (or a `lea` followed by one)
  where the flags are dead. A 64-bit `lea` leaves the upper half of a
  32-bit sum unspecified, which "Narrow integer representation" allows.
- **Loop rotation**, as tail duplication after allocation: a `jmp` to a
  block of at most eight instructions ending in a conditional branch takes
  a copy of that block when the copied branch can fall through to the next
  block, or when the jump is a loop's back edge, the branch then turned to
  take the edge to the body; a back edge thus tests the condition itself.
- **Compare/branch fusion across blocks**: an `icmp` whose every use is a
  `brif` or `select` condition, in any block, is emitted again before each
  user instead of being materialized with `setcc`.
- **Selection peepholes** found on the way: division and remainder by a
  power of two with shifts and masks (`stencil` divided by 4 with `idiv`),
  and dead-code elimination of arithmetic whose result and flags are dead.

Left open at the time: the **red zone** (it changes the prologue, the
epilogue and the unwind information, which stage 5's object writer
generates, so it waited for that writer; added later, see "Red zone and loop
alignment" below); releasing the **scratch registers** `%r10`/`%r11`
and `%xmm14`/`%xmm15`, which the allocator's fallback and parallel-copy
cycles still use; **rematerialization** of hoisted symbol bases instead of
spilling them; **value-based coalescing**; and a 32-bit `lea` form.

Measured on 2026-10-19 on native Linux (AMD Ryzen 9 5900X, a machine shared
with other builds; release builds; `scripts/compare_optimizers.py --arms
A1,B1 --baseline A1 --reps 7`, CPU time, every output checked against the
interpreter's), the stage 3 build and this branch back to back. Static
counts are over the assembly of `--opt=bcc --backend=native` for System V:
instructions, register-to-register moves, and `%rsp`/`%rbp`-relative
operands (spill code and stack slots).

| Benchmark | B1/A1 before | B1/A1 after | Instructions | Register moves | Frame operands |
| --- | ---: | ---: | ---: | ---: | ---: |
| bits | 1.341 | 1.082 | 350 → 216 | 79 → 38 | 79 → 0 |
| bytecode | 1.709 | 1.298 | 233 → 156 | 29 → 8 | 0 → 0 |
| collatz | 1.637 | 0.999 | 97 → 60 | 25 → 8 | 5 → 0 |
| crc32 | 1.116 | 1.002 | 178 → 127 | 45 → 21 | 6 → 0 |
| fannkuch | 1.289 | 1.002 | 200 → 113 | 24 → 5 | 55 → 3 |
| hashtable | 1.217 | 0.999 | 230 → 165 | 25 → 15 | 40 → 2 |
| levenshtein | 1.593 | 1.172 | 249 → 211 | 32 → 14 | 12 → 12 |
| linkedlist | 1.025 | 1.004 | 142 → 104 | 36 → 16 | 0 → 0 |
| mandel | 1.980 | 1.021 | 182 → 142 | 28 → 17 | 37 → 19 |
| matmul | 1.186 | 0.999 | 134 → 104 | 32 → 4 | 0 → 0 |
| nqueens | 1.372 | 1.129 | 108 → 120 | 18 → 17 | 10 → 16 |
| particles | 1.967 | 1.185 | 594 → 448 | 65 → 31 | 157 → 71 |
| sieve | 1.152 | 1.067 | 78 → 60 | 19 → 12 | 0 → 0 |
| sorts | 1.420 | 1.065 | 368 → 277 | 54 → 40 | 61 → 10 |
| stencil | 2.495 | 1.457 | 289 → 203 | 47 → 11 | 7 → 0 |
| strings | 1.284 | 1.057 | 399 → 296 | 70 → 12 | 48 → 0 |
| switchvm | 1.054 | 0.597 | 511 → 369 | 80 → 18 | 24 → 6 |
| **total / geometric mean** | **1.417** [1.404, 1.434] | **1.052** [1.048, 1.061] | 4342 → 3171 | 708 → 287 | 541 → 139 |

The stage's target, B1/A1 within 1.5×, held before (1.42×) and now stands
at 1.05×, with 4 of 17 programs faster than LLVM's code generator. Whole
builds take as long as before (B1 builds in 0.95-0.96× A1's time in both
runs). Instructions grew in a few programs where loop rotation copied
tests; fewer of them run.

Code placement now dominates what is left in some programs: `stencil`
moves between 1.16× and 1.46× from build to build with the same inner loop.
On this machine its executable runs in 0.246 s as built and in 0.192 s with
a 3-byte `nop` after one `lea` before the loop, while a same-length `leal`
in place of that `leaq` stays slow, so the loop's alignment, not its
instructions, costs the difference. Aligning loop headers (`.p2align 4`, as
LLVM does) belongs with stage 5's object writer, which must then pad with
multi-byte `nop`s itself; it was added with the red zone (below). Of the
rest, `bytecode` (1.30×) repeats its
dispatch test in every handler and indexes 16-byte records with a shift,
and `particles` (1.19×) copies its structure through the stack, which the
bcc optimizer leaves.

Validation: every native test runs the checker on every function (debug
builds). The difftest native oracle passed its default sweep and the
500-case sweep (`--include-ignored`) after every change, as did the native
end-to-end programs (B0 and B1 against the interpreter, wide floats against
the LLVM back end) and the ABI conformance tests that link native objects
with Clang's in both directions. The GCC torture survey was not run by the
stage itself, since its corpus was not on the Linux host used and release
builds skip the checker. The stage's review then ran it with debug builds
of the stage 3 base and the stage 4 branch (GCC 15.2.0
`gcc.c-torture/execute`, 1,677 top-level and 213 `ieee/` and `builtins/`
sources, 1,890 in all, arms A0, B0 and B1, `x86_64-unknown-linux-gnu`): no
checker rejection, no compiler crash, and no source whose result differs
between base and branch, which meets the "checker clean on the whole
torture corpus" criterion.

### Stage 5: direct ELF and COFF object emission

`--backend=native` with `-o` emits AMD64 COFF objects in process for both
Windows targets by default, and ELF64 objects for Linux targets under
`--native-object`; executables link those objects with bundled clang/LLD.
`--no-native-object` selects the text path, which bundled clang assembles,
on every target; it remains the Linux default, and the last of the two flags
wins. `--emit=asm` prints the same listing either way. The encoder reads only
the emitter's vocabulary, so a module with file-scope or inline assembly
takes the text path by default and is refused under `--native-object`.
`driver::object_writer` makes this choice from the target, the flags and the
module, in one place, including whether an encoder refusal is final, and
`driver::build` runs it. When the default direct writer's encoder refuses a
listing, `build` assembles it instead, so the Windows default still builds
everything the text default did: a user section name outside
`[A-Za-z0-9_.$-]`, a user `.pdata` or `.xdata` section, a non-ASCII export name,
or more than 65,279 sections (big-object COFF). Only `--native-object` keeps
such a refusal as an error. An executable's listing is assembled into a
temporary object, which links as a direct object does. When the assembly
itself fails, for example without the MinGW that only the assembler needs
for an object, the error gives the refusal before the assembler's failure; a
missing output directory or a link error reads as it does on the direct
path.

The encoder consumes the **private listing generated by `emit.rs`**, rather
than independently expanding allocated MIR. This shares all frame, CFI,
global and symbol decisions with the text path and keeps this work independent
of the concurrent allocator work. It is not a public assembly-language parser:
an unknown instruction, operand or directive is an error before opening the
output. Moving the shared vocabulary to typed emission events is a possible
follow-up; the current path still formats and parses arena-backed text.

`object/listing.rs` parses that vocabulary into arena-backed sections, symbols,
fixups and layout items. `object/encoder.rs` covers the emitted integer, SSE2,
x87 and TLS forms, REX/ModRM/SIB addressing and canonical alignment NOPs.
Local jumps start short; layout promotes out-of-range jumps to rel32 until no
sizes change, including alignment after branches. Since alignment padding
inside a function shrinks when a branch before it grows, the final sizes
depend on the order of those decisions, so layout repeats MC's fused pass
exactly (`MCAssembler::relaxOnce` in the pinned LLVM): each item takes its
offset from the items before it in the same pass and a branch is promoted
at once, measuring to a target behind it with this pass's offsets and to a
target ahead of it with the previous pass's offsets of both ends (MC's
`Stretch` correction). `object/elf.rs` writes ELF64 sections,
local/global/weak symbols and aliases, visibility, RELA records,
COMDAT groups (including grouped relocation sections), NOBITS, TLS,
`.init_array`/`.fini_array` and `.note.GNU-stack`. Local relocations use section
symbols plus offsets, while interposable calls retain PLT32 and GOT loads
retain GOTPCRELX relocations. `object/unwind.rs` generates a CIE and each FDE
from the shared emitter's CFI events, including multiple epilogues, CFA register
changes and remember/restore state; instruction offsets come from final layout.
No LLVM encoding or object-writing API is involved.

`native/tests/object.rs` compares direct objects against pinned clang assembly:
section contents and full section headers in order (including unwind bytes,
links, info and entry sizes), symbol properties, RELA tuples and COMDAT membership.
File offsets, string-table packing and symbol order within the local/global
partitions may differ; `sh_info` must identify the exact partition boundary.
Mutation tests cover the boundary, section order, links, info and entry sizes.
The writer places each group before its members and each RELA section after its
target. ELF and COFF byte/structure comparisons use independent Rust parsers
and do not require inspection tools. Disassembly is an optional cross-check,
using adjacent `llvm-objdump` or GNU `objdump` from PATH. Missing tools and
unsupported object formats print an explicit `SKIP optional disassembly` line;
all core comparisons still run. Native
selection helpers check their ELF listings, and native assembly/run helpers
check equivalence and link/run the direct ELF object. The latter also covers
the difftest native oracle and native ABI conformance tests. Boundary fixtures
exercise short/near jumps, alignment, register widths, immediates, displacements,
SSE/x87, TLS and symbol attributes.
After stage 4 the listing also carries `disp(base,index,scale)` operands and
the allocator's `lea` forms; a fixture encodes them for every base and index
that needs a SIB byte or a displacement, beside every `cmov` condition and the
shifts and masks of power-of-two division, and refuses a `%rsp` index, other
scales, RIP-relative indexing and 32-bit address registers. Integrated on
stage 4, the direct and text paths gave identical objects for all 266 corpus
inputs (`benches/c` and `tests/fixtures`, B0 and B1) the back end accepts, with
no direct-only refusal, and the native tests check every program with every
value spilled through the direct path as well.
Every supported instruction validates operand count, kind and width before
encoding. Unsupported suffixes, oversized fields, duplicate definitions,
conflicting bindings and unsupported size expressions refuse explicitly.
`.p2align` takes MC's optional fill and maximum (`.p2align 4,,10`): padding
that would exceed the maximum is omitted, a maximum of zero is refused as
MC refuses it, and one of at least the alignment has no effect. Section
names infer GNU/LLVM flags and types; byte/short relocations use their own
widths, and interposable references retain
relocations. The CLI object test uses fresh paths and links/runs each result.
The COFF writer lives in `object/coff.rs`, with COFF listing, unwind and writer
modules beside the ELF implementation. Section headers carry PE characteristics
and alignment; the symbol table includes section auxiliary checksums, COMDAT
selection/associative section numbers, weak external records and long-name
strings. Relocations use implicit addends, including ADDR64, ADDR32, ADDR32NB,
REL32/REL32_1..5, SECREL and SECTION. Pinned Clang uses REL32 plus an in-place negative
addend when immediate bytes follow a displacement. The writer follows that
choice. Sections with at least 65,535 relocations use the COFF overflow flag
and extended-count sentinel, with saturated header and section auxiliary counts.
`.seh_*` event offsets generate matching `.pdata`/`.xdata`, including
small/large stack allocations, saved GPR/XMM registers and frame pointers.
MinGW uses discardable `.xdata$*`/`.pdata$*` subsections; MSVC uses associative
COMDATs emitted after ordinary unwind sections.
`.refptr`, selectany, `.drectve` exports, `.ctors`/`.dtors` and `.CRT$XC*` arrays
use the same private listing as the text path. The COFF listing reads
`.p2align` with the ELF parser, so loop headers inside functions and the
max-skip form lay out and relax as on ELF. An explicit `.scl 3` keeps a
symbol static on either side of `.globl`, as in MC, and an unquoted section
name must be one MC identifier (`-` needs quotes).

`native/tests/object_coff.rs` independently parses COFF and compares section
order, bytes/characteristics, symbol types/storage classes and complete aux
records, relocation types/targets, and COMDAT selection keys. It normalizes
file placement, string packing and symbol order while preserving key identity.
Mutation tests pin metadata, unwind and COMDAT key checks;
bundled lld-link and MinGW-mode ld.lld link direct objects and select duplicate
selectany definitions. Native Windows test helpers compare listings and use
direct objects, which native Windows CI builds and runs (below). The portable
`scripts/compare_coff_objects.py` repeats the structural comparison over every
benchmark/C fixture and additional random or external corpora at B0/B1 for both
Windows targets. Neither oracle requires llvm-readobj. COFF validation
results are recorded below.

Round 1 validation on 2026-10-19: the full all-target test gate passed
**2,372 tests, 0 failed, 8 ignored** across 30 test binaries. The focused native
suite passed 52 tests (one timing test ignored), the allocation gate passed all
31 tests, and Clippy passed with and without `benchmarking-internals`. The final gate
used the normal system temporary directory; the earlier host-mounted scratch
TMPDIR could not represent a preprocessing fixture's backslash filename.

Round 2 cross-review validation on 2026-10-19: **2,382 tests passed,
0 failed, 8 ignored** across 30 binaries, including 20 passing object tests
(one additional timing test ignored). The allocation-feature gate passed all
31 tests, and Clippy passed with and without `benchmarking-internals`; formatting
and whitespace checks passed. An independent ELF corpus check attempted every
benchmark and fixture C source at B0/B1: all 17 benchmarks and 116 compilable
fixtures per arm produced **266 equivalent object pairs**, with no new
direct-emission refusal.
The other 364 fixtures per arm were refused by the text compiler, including
intentional diagnostic inputs. This covers all 114 distinct fixture sources
in the review's saved 228-comparison sweep.

The ignored `benchmark_native_object_emission` test measures the post-ABI IR to
object phase of both paths with 20 interleaved samples (one warm-up), including
text formatting in both arms, direct parsing/encoding, output file writes and
the clang assembler subprocess in the text arm. Front-end, optimization and
final-link costs are outside this measurement.

Measured on 2026-10-19 at `57bae1b2` on Linux, Rust 1.99.0
(`b940084d7`, 2026-09-28), pinned clang 23.1.1 (`1b9c0d5ff9bb`), debug test
profile, no Cargo features. Inputs are the named native C regression programs;
B0/B1 select the unoptimized/optimized post-ABI modules. Times are milliseconds;
medians and minima are from the same 20 interleaved samples after warm-up.

| Input | Arm | Text median (min) | Direct median (min) | Direct/text median |
| --- | --- | ---: | ---: | ---: |
| `widths` | B0 | 9.951 (9.088) | 1.624 (1.304) | 0.163 |
| `widths` | B1 | 9.221 (8.782) | 1.393 (1.331) | 0.151 |
| `division_and_shifts` | B0 | 10.236 (9.536) | 2.288 (2.162) | 0.224 |
| `division_and_shifts` | B1 | 9.838 (9.229) | 1.977 (1.784) | 0.201 |
| `pressure_and_calls` | B0 | 10.149 (9.638) | 2.264 (2.134) | 0.223 |
| `pressure_and_calls` | B1 | 9.069 (8.473) | 1.038 (0.963) | 0.114 |
| `float_comparisons_and_conversions` | B0 | 10.008 (9.389) | 2.071 (1.881) | 0.207 |
| `float_comparisons_and_conversions` | B1 | 9.848 (9.198) | 1.966 (1.793) | 0.200 |

These are emission-phase measurements on small inputs on a shared machine,
not full-process or release-build compile-time speedups. Reproduce with
`cargo test --lib benchmark_native_object_emission -- --ignored --nocapture`
(through the shared cargo wrapper in the overnight environment).

COFF validation for #87 on 2026-10-19: the full all-target gate passed
**2,453 tests, 0 failed, 9 ignored** across four test binaries. Focused object
checks passed **35 tests, 0 failed, 1 ignored**; the allocation-feature gate
passed all **31 tests**. Clippy passed with and without
`benchmarking-internals`; formatting and whitespace checks passed.

The independent COFF corpus runner attempted all **17 benchmarks, 480 C
fixtures and 80 random stress programs** on both Windows targets at B0/B1:
**852 equivalent object pairs, 0 mismatches, 0 new direct refusals**. The other
**1,456 attempts** were refused by both paths, including intentional diagnostic
fixtures. Four fresh C-level freestanding links (both targets at B0/B1) passed
with MinGW-mode `ld.lld` and `lld-link`; each linked image retained the first
selectany definition, value 42.

Windows execution and the default switch (#87, 2026-10-20): native Windows CI run
39405683497, at `ee3bd1bf` (which contains the COFF writer), passed
`cargo test --all-targets` with 2,302 library tests. Among them,
`native_programs_match_the_interpreter` and
`programs_with_every_value_spilled_match_the_interpreter` build every native
test program as a direct COFF object, link it and run it natively against the
interpreter, after checking each listing's equivalence with Clang's object.
With both acceptance conditions met, direct COFF objects became the Windows
default for `-o`. CLI tests pin the choice: for both Windows targets at B0 and
B1 a default `-c` object is byte-identical to the `--native-object` one, a
`--no-native-object` object is byte-identical to bundled clang's assembly of
the `--emit=asm` listing, and Linux defaults still match the latter. On a
Windows host, the CLI's native executable tests now build and run direct
objects by default and the text path under `--no-native-object`.

### Red zone and loop alignment (stage 4 leftovers)

Added on 2026-10-19 (#85), after the direct object writer, which reproduces
both:

- **Red zone** (see "Frame layout"): a System V leaf whose frame below the
  pushes fits 128 bytes keeps it below `%rsp`. Of the 56 functions of
  `benches/c` at B1, 9 do. The unwind information needs nothing new: the
  CFA stays where the pushes put it, which the shared CFI events already
  say. `unwinding_through_a_red_zone_leaf_in_a_signal_handler` interrupts
  such a leaf with `SIGPROF` while its frame is live, unwinds from the
  handler through the signal frame and the leaf to its caller and `main`
  with `_Unwind_Backtrace`, and checks that the frame survived the
  handlers. With a CFA that wrongly counts the frame (a hand-edited
  `.cfi_def_cfa_offset`), every sample taken in the leaf fails to unwind
  past it.
- **Loop alignment**: each loop header of the final layout, the target of
  an edge from a later block or from itself, starts at a 16-byte boundary
  (`.p2align 4`, multi-byte `nop`s, no padding limit), which is LLVM's
  x86-64 alignment. LLVM's block placement also aligns other hot blocks
  inside loops whose predecessor in the layout does not fall into them.
  Without block frequencies that rule aligns every case of an
  interpreter's dispatch, and it measured slower: `switchvm` went from
  0.60× to 0.79-0.83× of LLVM in three runs. A padding limit as GCC's
  `.p2align 4,,10` measured slower too, because `levenshtein`'s loop needs
  more padding and lost its gain. The direct encoder still implements the
  max-skip form (stage 5 above).

Measured on native Linux (AMD Ryzen 9 5900X, shared with other builds;
release builds; CPU time; the C programs pinned to one core and run with
ASLR off; 11 interleaved repetitions; every output checked against
A1's): B1/A1 per program for the base, loop headers aligned to 16 bytes,
as built, and to 64 bytes, for comparison. The ratios move by a few
percent between runs on this machine; `linkedlist` moves by more, since
its run time depends on where its heap lands.

| Benchmark | Base | 16-byte headers | 64-byte headers |
| --- | ---: | ---: | ---: |
| bits | 1.095 | 1.025 | 1.063 |
| bytecode | 1.278 | 1.279 | 1.299 |
| collatz | 1.008 | 0.995 | 0.983 |
| crc32 | 1.003 | 0.999 | 1.009 |
| fannkuch | 0.991 | 0.962 | 0.986 |
| hashtable | 0.987 | 1.070 | 1.045 |
| levenshtein | 1.185 | 1.009 | 0.992 |
| linkedlist | 0.998 | 0.867 | 1.087 |
| mandel | 1.029 | 1.032 | 1.021 |
| matmul | 1.014 | 1.014 | 0.989 |
| nqueens | 1.106 | 1.166 | 1.099 |
| particles | 1.184 | 1.154 | 1.149 |
| sieve | 1.055 | 1.010 | 0.996 |
| sorts | 1.070 | 1.090 | 1.049 |
| stencil | 1.468 | 1.467 | 1.299 |
| strings | 1.049 | 1.033 | 1.012 |
| switchvm | 0.609 | 0.576 | 0.599 |
| **geometric mean** | **1.052** | **1.028** | **1.028** |

`.text` over the 17 programs grows from 12,260 to 13,496 bytes (10%) with
16-byte alignment; 64-byte alignment would take 18,462 (51%).

Alignment helps `levenshtein` (1.18× to 1.01×) and `bits`, costs
`hashtable` a few percent, and does not settle `stencil`. Shifting the code
before its loop by 0 to 63 bytes, as
unrelated changes do between builds, moves it between 0.230 s and 0.265 s
with 16-byte alignment (0.227-0.263 s without): its speed follows the
loop's offset within a 64-byte line, which 16- and 32-byte alignment leave
free. Only 64-byte alignment pins it (0.216-0.237 s over the same shifts,
apart from one at 0.248 s), at the code size above, so the back end keeps LLVM's
16 bytes; 64-byte loop alignment for this processor is left open.
