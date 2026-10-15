# Native x86-64 back end

Status: design, with stages 1 and 2 and most of stage 3 implemented in
`src/backend/native/` (2026-10-14); see "Implementation status" at the end.

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
          │   liveness, intervals     iterative dataflow; one range per vreg
          │   register allocation     linear scan, fixed-register ranges
          │   rewrite                 physical registers, spill code, copies
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
`f80` and `f128` values, `i128` bit counts, byte swaps, `switch`es and
conversions to or from floats, `i128` positions Win64 never passes by value,
structure results in `f80`/`f128` registers, stack objects aligned above 16,
and frames larger than 1 GiB.

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
| `f80` | the x87 stack, selected as memory-to-memory sequences (no x87 register allocation) | 3 |
| `f128` | libcalls (`__addtf3` …) over XMM | 3 |

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
   defined `global_addr`, or a `ptr_add` of a register and a constant that
   fits 32 bits becomes one memory operand (`disp(base)`), recursively over a
   chain of constant `ptr_add`s (iteratively: the selector walks the chain
   with a loop, not a recursive call).
3. **Compare fusion.** An `icmp` whose only use is the `brif` or `select` of
   the same block, with no intervening flag-clobbering instruction, is not
   materialized: the selector emits `cmp` immediately before the `jcc` or
   `cmov`. Otherwise `icmp` produces a canonical `i1` with `setcc`/`movzbl`.

Use counts come from one pass over the function's operands; the IR has no use
lists and the selector does not need them otherwise.

The selector's per-opcode table for stage 1:

| IR | Machine code |
| --- | --- |
| `iadd`/`isub`/`and`/`or`/`xor` | `mov d, a; op d, b` (two-address); `b` may be an immediate |
| `imul` | `mov d, a; imul d, b`, or `imul d, a, $imm` |
| `sdiv`/`srem`, `udiv`/`urem` | narrow operands extended to 32 bits; `mov %rax, a; cltd/cqto` or `xor %edx,%edx`; `idiv`/`div b`; result from `%rax` or `%rdx` |
| `shl`/`lshr`/`ashr` | amount in `%cl` (or an immediate masked to the width); `lshr`/`ashr` extend a narrow operand first |
| `icmp` | `cmp a, b` at the operand width, then `setcc` + `movzbl`, or fused |
| `select` | `test c`/fused `cmp`; `mov d, b; cmovcc d, a` at 32 or 64 bits |
| `zext`/`sext`/`trunc` | `movzbl`/`movzwl`/`movl`, `movsbl`/`movswl`/`movslq`/…, or a copy |
| `ptrtoint`/`inttoptr`/`bitcast`/`freeze` | a copy (an `inttoptr` from a narrower integer zero-extends) |
| `ptr_add` | `leaq off(base), d` for a constant offset, else `mov d, base; add d, off` |
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

/// A memory operand: `disp(base)`. An index register (`disp(base, index, scale)`)
/// comes with stage 4's addressing-mode selection.
pub(crate) struct Mem { base: Base, disp: i32 }

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

**Stage 1: linear scan over whole live intervals with fixed-register ranges**
(Poletto and Sarkar, "Linear Scan Register Allocation", TOPLAS 1999), with
hints and whole-interval spilling. It is linear in the number of intervals
after sorting, simple to check, and non-recursive by construction.

1. **Numbering.** Machine instructions are numbered in block layout order;
   instruction *i* reads its uses at position `2i` and writes its defs at
   `2i+1`, so a value that dies at an instruction can share a register with
   the value it defines (x86 reads all inputs before writing outputs).
2. **Liveness.** Per-block use/def bit sets, then backward iterative dataflow
   (`live_in = use ∪ (live_out − def)`) over blocks in reverse layout order
   until nothing changes. Bit sets are `u64` words in the function arena.
3. **Intervals.** One range `[start, end]` per vreg: the minimum and maximum
   position at which it is defined, used or live across a block boundary. A
   value live into a loop header and out of the latch covers the whole loop.
   The single range ignores holes, which costs registers but never
   correctness.
4. **Fixed ranges.** A backward walk per block records for each physical
   register the ranges where a precolored value lives (from its `mov` def to
   its use) and the single position `2i+1` of each clobber (calls clobber all
   caller-saved registers). An interval may take register *r* only if it
   overlaps no fixed range of *r*; a value live across a call therefore
   overlaps every caller-saved clobber and gets a callee-saved register or the
   stack, without special cases.
5. **Scan.** Intervals sorted by start; an active list expires intervals that
   end before the current start. The current interval takes its hint if free
   and fixed-compatible, else the first free compatible register in the
   allocation order. When none is free it spills the interval, among itself
   and the active intervals whose register is compatible, that ends last
   (Poletto's heuristic).
6. **Rewrite.** Virtual registers become their physical registers; identity
   moves are deleted. A spilled vreg lives in its own 8-byte spill slot for
   its whole life; each instruction that uses it loads it into a *reserved
   scratch register* first and each that defines it stores it afterwards.
   Stage 1 reserves `%r10` and `%r11` (caller-saved in both conventions and
   never argument registers; no MIR instruction names more than two distinct
   vregs, so two suffice).

Allocation order: caller-saved registers first (no save cost) — `%rax %rcx
%rdx %rsi %rdi %r8 %r9` on System V, `%rax %rcx %rdx %r8 %r9` on Win64 — then
callee-saved `%rbx %r12 %r13 %r14 %r15 %rbp` (plus `%rsi %rdi` on Win64).
The frame pointer is omitted, so `%rbp` is allocatable.

```rust
struct Interval { vreg: VReg, start: u32, end: u32, hint: Option<PReg> }
/// Sorted, disjoint fixed ranges of one physical register.
struct FixedRanges<'a> { ranges: [ArenaVec<'a, (u32, u32)>; 16] }
enum Location { Reg(PReg), Spill(u32) }
struct Allocation<'a> { locations: &'a [Location], used_callee_saved: PRegSet, spill_slots: u32 }
```

Debug builds check the result: no two overlapping intervals share a register,
and no interval overlaps a fixed range of its register.

**Later stages** (4) keep the interface (`MFunction` in, `Allocation` out) and
improve quality behind it:

- *Splitting.* Intervals with holes (a list of ranges), split at block
  boundaries and around calls, with spill code placed at split points rather
  than at every use; second-chance binpacking (Traub et al.) or a greedy
  priority-queue allocator in the style of LLVM's `RegAllocGreedy`.
- *Coalescing.* Join copy-related intervals that do not interfere before
  allocation (the hints cover the common case in stage 1).
- *Rematerialization* of constants and addresses is already structural
  (stage 1 never allocates them).
- *Memory operands.* Use a spilled value directly as an instruction's memory
  operand instead of reloading it through a scratch register, then stop
  reserving `%r10`/`%r11`.
- *`regalloc2`.* The `regalloc2` crate (Cranelift's allocator) was evaluated
  against the arena rules: it allocates its working state from the global
  allocator, takes its input through a trait over `Vec`-backed structures and
  recurses in places, so adopting it would need an `#[expect]`-wrapped
  boundary and an exception to `tests/allocation_count.rs`. Its
  `Function` trait is a good model for keeping our own allocator replaceable;
  its symbolic checker (abstract interpretation of the allocated code against
  the vreg program) is the right verification design for stage 4.

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
  without moving it (psABI §3.2.2). Stage 1 always moves `%rsp`; using the red
  zone for small leaf frames is a stage 4 peephole that saves the `sub`/`add`
  and the CFA updates.
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
- **Over-aligned slots** (above 16) need a realigned frame with a frame
  pointer (`and $-align, %rsp` after `mov %rsp, %rbp`); stage 2, together with
  `dyn_alloca`, which also needs `%rbp` because `%rsp` then moves at run time.

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
  supports integer parts; SSE parts (`%xmm0`/`%xmm1`) and `f80` (`%st0`) come
  with floats.

**Variadic definitions** (stage 2). After ABI lowering only `va_start`,
`va_copy` and `va_end` remain (ABI lowering expanded `va_arg` into loads from
the `va_list` the target defines). System V: the prologue stores the six
integer registers and, guarded by `test %al, %al`, the eight vector registers
into a 176-byte register save area, and `va_start` initializes `gp_offset`,
`fp_offset`, `overflow_arg_area` and `reg_save_area` (psABI §3.5.7, Figure
3.33). Win64: the prologue homes `%rcx`-`%r9` into the caller's home area, so
all arguments are contiguous 8-byte slots, and `va_start` stores the address
of the first variadic slot ("x64 calling convention", "Varargs").

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
| defined data reference | `sym(%rip)` (`R_X86_64_PC32`) | `sym(%rip)` (`IMAGE_REL_AMD64_REL32`) |
| undefined data reference | `movq sym@GOTPCREL(%rip), %r` (`GOTPCRELX`) | `sym(%rip)`; LLD's MinGW auto-import handles DLL data |
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
clang, disassemble both objects with `llvm-objdump -d -r`, and require
identical instructions and relocations on every test program and torture
source. An in-process alternative available before stage 5 is to hand the
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
| `native/isel.rs` | instruction selection, with `isel/calls.rs` (calls, returns, entry, `va_start`), `isel/edges.rs` (terminators and parallel copies), `isel/memory.rs` (addresses, loads, stores, `copy`, `fill`), `isel/float.rs` (SSE), `isel/wide.rs` (`i128` pairs) and `isel/stack.rs` (dynamic allocation) |
| `native/regalloc.rs` | linear scan, with `regalloc/liveness.rs` (liveness, intervals, fixed ranges) and `regalloc/rewrite.rs` (spill code and physical rewrite) |
| `native/frame.rs` | frame layout, prologue and epilogue |
| `native/emit.rs` | AT&T text: instructions, functions, globals, symbols |
| `native/driver.rs` | the assembly file and the bundled clang invocation |
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
   the allocation checker accepts every function of the test programs.
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
| 2 | Variadic definitions (`va_start` save areas), `dyn_alloca`/`stack_save`/`stack_restore` with a frame pointer, inline `copy`/`fill` for small constant sizes; over-aligned slots and jump tables for dense switches | 2k | torture: every integer-only source A0 passes also passes B0 | done except over-aligned slots and jump tables |
| 3 | Floats: an XMM register class in the allocator, SSE selection for `f32`/`f64`, conversions, `fcmp` with parity, SSE `registers` returns and variadic float arguments (`%al` count, Win64 duplication), `i128` register pairs and division libcalls; `f80` through x87 memory sequences and `f128` libcalls | 4-5k | torture: B0 passes ⊇ A0 passes except documented x87 gaps; `benches/c` run in B0/B1 | done except `f80`, `f128` and `i128`/float conversions |
| 4 | Quality: split intervals and second-chance binpacking or a greedy allocator, coalescing, memory operands for spills, addressing modes with index and scale, LEA arithmetic, red zone, loop rotation, compare/branch fusion across blocks, symbolic allocation checker | 4k | B1/A1 geometric mean within 1.5× on `benches/c` (native Linux), allocation checker clean on the whole torture corpus | B1/A1 is 1.25-1.40× on Windows already; the allocator work is open |
| 5 | Direct object emission: encoder with relaxation, ELF and COFF writers, unwind table generation | 4-5k | objects byte-equivalent in disassembly and relocations to clang-assembled text on all tests; compile time measured against stage 1 | open |
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
- Undefined data on Windows is referenced directly, relying on LLD's MinGW
  auto-import.
- `--emit=asm` prints the native assembly and requires `--backend=native`.

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

`f80` was not attempted: it needs memory-resident x87 values, `fnstcw`
truncation sequences, and ST0 returns, which is not cheap; 16 sources need
it.

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

Compile time is comparable: median whole-process build 1.29-1.51 s for B0/B1
against 1.09-1.25 s for N/A1, the difference being the extra clang run that
assembles the text.
