# An object-aware IR for bcc

Study baseline: `145898d4` (`origin/main` when this worktree was created),
2026-10-21. All repository `file:line` references in this document refer to
that commit. This study changes documentation only.

## 1. Inventory of the current IR

### What the owner is noticing

The strongest coupling is semantic. `src/ir.rs:13` explicitly adopts LLVM
poison semantics without `undef`; `middle-end.md:155` specifies propagation,
`freeze` and `inbounds`. The earlier design deliberately chose this approach
(`docs/research/middle-end-ir-designs.md:593`) to simplify LLVM translation and
optimizer comparison. It is implemented consistently across several layers,
so changing instruction names would accomplish little.

Several suspected leaks are absent or need qualification:

- **ABI lowering is after IR construction and optimization.** The driver calls
  lowering at `src/pipeline/middle_end.rs:62`, optimization at line 76 and ABI
  lowering at line 87. Pre/post-ABI profiles are real verifier contracts
  (`src/ir/verifier.rs:461`). Target data layout is selected earlier, as C
  compilation requires, but aggregate register classification is not.
- **Aggregates are not general first-class SSA values.** Both profiles have
  scalar/pointer values only (`src/ir/types.rs:15`,
  `src/ir/verifier.rs:471`). Pre-ABI calls represent by-value aggregate intent
  with pointers to copies and layout descriptors (`src/ir/module.rs:633`).
- **There is no typed GEP instruction.** `ptr_add` takes a byte offset; the LLVM
  emitter turns it into a single-index `getelementptr i8`
  (`src/backend/llvm/api.rs:1302`). The inherited issue is its `inbounds`
  promise and poison result, not LLVM's aggregate-index syntax.
- **There is no `undef`, arbitrary `syncscope` string, generic LLVM intrinsic
  escape hatch, general LLVM attribute bag, or general COMDAT selection API in
  the shared instruction/module definitions.** Some operations and symbol
  policies deliberately map to those facilities at emission.

Read `middle-end.md` as a design/status document, not the sole implementation
authority: its opening still calls the work a prototype. The inventories below
were checked against source at the baseline.

### Concept inventory

Semantics labels: **C** means a C99 abstract-machine rule, with C11 additions
identified; **LLVM** means a deliberately imported LLVM contract; **bcc** means
an implementation/target/extension policy owned by this compiler. Many concepts
combine these. Consumer abbreviations: **L** LLVM emitter, **N** native selector
and MIR, **I** interpreter, **O** optimizer, **A** ABI lowering. A consumer
depending on an operation can refuse it; refusal is called out below.

#### Values, control and arithmetic

| Current concept and definition | LLVM counterpart | Whose semantics? | Consumers and dependencies |
| --- | --- | --- | --- |
| Dense function-local values/instructions, block parameters and pooled branch arguments (`src/ir.rs:3`, `src/ir/function.rs:43`) | SSA registers, basic blocks, `phi` | **bcc** representation; SSA is not LLVM-specific | O operates on these; L constructs phis; N lowers edges to parallel copies; I assigns block parameters. All need dominance and edge arity. |
| Signless `i1/i8/i16/i32/i64/i128`, `f32/f64/f80/f128`, opaque 64-bit `ptr` (`src/ir/types.rs:15`) | Integers, `float/double/x86_fp80/fp128`, opaque `ptr` | **bcc/LLVM** low-level vocabulary; C signedness/rank/qualifiers have already been erased. `_Bool` is i8 in storage and comparisons use i1. | L direct mapping; N register classes/pairs and x87 helpers; I runtime variants. I does not implement all extended floating operations. A retains some narrow signedness in signatures. |
| Constants, `null`, addresses, constant pool (`src/ir/instructions.rs:82`) | Constants, null, global/function addresses | **bcc** encoding of target/C values | L/N/I materialize them; O folds them. Wide bits live outside the 16-byte instruction record. |
| Integer arithmetic, bitwise operations, signed/unsigned division and shifts (`src/ir/instructions.rs:295`) | `add/sub/mul`, `sdiv/udiv`, shifts, bitwise ops | **C** motivates choices; ordinary unflagged arithmetic is **bcc/LLVM** fixed-width arithmetic. Division has immediate UB cases; excessive shifts produce poison. | O's evaluator, L, N, I. Native uses hardware operations/refinements rather than runtime poison. |
| `nsw`, `nuw`, `exact` (`src/ir/instructions.rs:513`, `src/ir/instructions.rs:567`) | Same flags | **LLVM** deferred-UB contracts. C's signed-overflow rule motivates `nsw`, but C has no poison value or `exact` flag. | O folding/SCCP/GVN/known-bits; L attaches flags; I tests them; N drops them as refinement. |
| Integer/floating comparisons, `select`, numeric conversions/bitcasts (`src/ir/instructions.rs:341`, `src/ir/instructions.rs:526`) | `icmp/fcmp/select`, casts | **bcc/LLVM** primitive semantics implementing C conversions. Ordered/unordered FP predicates mirror LLVM. `select` masks poison in the unchosen arm. | O/L/N/I; conditional expressions initially use control flow, and O may form selects. FP rounding environment is not an explicit state in these instructions. |
| `poison`, `freeze` (`src/ir/instructions.rs:344`, `src/ir/instructions.rs:359`) | `poison`, `freeze`; **no `undef`** | **LLVM-derived**, with **bcc** choices for uninitialized storage and a deterministic interpreter witness | O has poison-specific lattices/refinements and preserves freeze identity; L emits them; I tracks poison and freezes to zero; N uses concrete bits and copies. |
| `jump`, `brif`, `switch`, `return`, `unreachable` (`src/ir/instructions.rs:159`) | `br`, `switch`, `ret`, `unreachable` | **bcc** CFG; executing `unreachable` is low-level UB, not a C statement | O/L/N/I. Block order is not implicit fallthrough. |
| `trap`, `debug_trap` (`src/ir/instructions.rs:334`) | `llvm.trap`, `llvm.debugtrap` | **bcc** builtin/extension behavior, not the semantic definition of every C UB | L/N/I handle explicit trap/debug operations; O must respect their effects. |

#### Memory, objects and calls

| Current concept and definition | LLVM counterpart | Whose semantics? | Consumers and dependencies |
| --- | --- | --- | --- |
| Fixed stack slots (`size: u32`, alignment), dynamic allocation, save/restore (`src/ir/function.rs:356`, `src/ir/instructions.rs:50`, `src/ir/instructions.rs:323`) | `alloca`, `llvm.stacksave/stackrestore` | **bcc** storage scheme implementing some **C** automatic/VLA behavior. Not a complete C lifetime model. | O promotes/splits slots; A creates temporary areas; L uses allocas; N creates frame objects/dynamic stack operations; I allocates objects. |
| `ptr_add` with `inbounds` (`src/ir/instructions.rs:365`, `src/ir/instructions.rs:576`) | Byte GEP, optionally `inbounds` | Byte offset is **bcc**; violated promise producing poison is **LLVM-derived**; C pointer arithmetic is restricted by an array/subobject domain | L emits byte GEP; I checks a live allocation and one-past bounds; N forms addresses without poison checks; O reasons about offsets/flags. |
| `ptrtoint`, `inttoptr` (`src/ir/instructions.rs:354`) | Same casts | Cast availability is **C**, details implementation-defined; provenance behavior here is partly **bcc**, not a specified full LLVM or C model | L casts; N integer/register operations; I reconstructs provenance by address lookup; O loses/reasons conservatively about address origins. |
| Loads/stores with width, alignment and optional access tag (`src/ir/instructions.rs:128`) | `load/store`, `align`, TBAA metadata | **C** memory access lowered to **bcc/LLVM** byte/scalar contracts; load of any poison byte poisons the scalar | O forwarding/promotion; L attaches alignment but no TBAA descriptor mapping; N chooses machine widths; I checks access bounds, liveness and alignment. |
| Volatile memory accesses (`src/ir/instructions.rs:508`) | `volatile` accesses | **C** observable accesses, with target/implementation policy for what constitutes an access | All consumers preserve observable accesses; O avoids normal elimination/merging. Volatile is neither atomicity nor an LLVM-only concept. |
| `Align` powers of two capped at 2^31 (`src/ir/instructions.rs:618`) | LLVM instruction/global alignment | Object alignment is **C/target**; this global cap is explicitly **LLVM-C API** driven | Lowering/A calculate guarantees; verifier uses validated Align; L passes unsigned alignment; N lays out frames; I checks guarantees. |
| `AccessTag` (`src/ir/entities.rs:92`) | TBAA descriptor reference | **bcc** reserved carrier for C alias facts; not an implemented effective-type model | Parser/printer preserve numeric tags. Lowering emits none; O's memory analysis explicitly does not use C type aliasing (`src/optimizer/memory.rs:12`); no semantic enforcement by I. |
| Aggregate copies, fill, `may_overlap` (`src/ir/instructions.rs:143`) | `llvm.memcpy/memmove/memset` | **C** object-representation copying and assignment; **bcc** byte operations distinguish overlap. Aggregate values remain in storage. | O SROA/memory passes; L intrinsics; N memory expansion; I copies byte state and pointer fragments. |
| Direct/indirect calls and interned signatures (`src/ir/instructions.rs:150`, `src/ir/module.rs:486`) | `call`, function types, call attributes | **C** calls lowered to **bcc** signatures; no arbitrary per-call LLVM calling-convention ID | O inlining; A call rewriting; L/N/I consume signatures and argument lists. Signature matching includes semantic attributes, not only machine types. |
| Aggregate/result descriptors, `byval/sret/registers`, narrow `signext/zeroext` (`src/ir/module.rs:574`, `src/ir/module.rs:633`) | ABI coercions and parameter/return attributes | Pre-ABI copy semantics are **bcc** implementing C by-value calls; post-ABI choices are **target ABI**, expressed partly in LLVM terms | A creates them; O sees pre-ABI descriptors; L/N/I all unpack/pack target calls. Internal I execution pays ABI complexity unnecessarily for C explanation. |
| Parameter, result and variadic `noundef` (`src/ir/module.rs:501`) | LLVM `noundef` | **LLVM** contract plus a **bcc** C-lowering policy matched to Clang | A remaps indices; O uses entry-parameter definedness; L emits attributes; I raises boundary UB; N relies on refinement, with no extra machine checks. |
| `va_start/va_arg/va_copy/va_end`, aggregate `va_arg` (`src/ir/instructions.rs:373`) | `llvm.va_*`, `va_arg`, target expansion | **C99** functionality; cursor representation is **bcc/target**, intrinsic spelling is LLVM-facing | A expands `va_arg`; L retains some intrinsics; N supports target vararg setup but refuses residual `va_arg`; I has cursor and ABI machinery (`src/backend/interpreter/varargs.rs:1`). |
| `returns_twice`, external setjmp/longjmp contracts (`src/ir.rs:196`, `src/backend/interpreter/nonlocal.rs:1`) | Function/call `returns_twice`; library calls | **C99** nonlocal jumps plus **bcc** external-symbol recognition; attribute is an LLVM-oriented summary, not a full continuation model | O avoids unsafe inlining; L marks calls/declarations; I saves/resumes frames; N refuses returns-twice calls/addresses (`src/backend/native/support.rs:160`). |

#### Extensions, atomics and module/linker policy

| Current concept and definition | LLVM counterpart | Whose semantics? | Consumers and dependencies |
| --- | --- | --- | --- |
| Leading/trailing zero count, defined-on-zero variants, population count, swap; unsigned overflow predicates (`src/ir/instructions.rs:314`) | `llvm.ctlz/cttz/ctpop/bswap`, `llvm.uadd/umul.with.overflow` | **bcc** builtin semantics; zero-undefined bit-count result uses LLVM poison. Many operations are useful independent primitives, not intrinsically leaks. | L emits intrinsics (`src/backend/llvm/function.rs:350`, line 410); N expands supported widths; I and O evaluate. N refuses some i1/i128 bit-count cases. |
| Frame/return addresses, timestamp, stack operations (`src/ir/instructions.rs:323`) | `llvm.frameaddress/returnaddress/readcyclecounter`, stack intrinsics | **bcc/target** extensions, some defined in terms of LLVM facilities | L maps to intrinsics; N/I support or explicitly refuse by capability. Timestamp is refused by N at baseline. Keep extensions distinct from C core operations. |
| Atomic load/store/RMW/CAS, byte atomics, fences (`src/ir/instructions.rs:114`, `src/ir/instructions.rs:719`) | Atomic `load/store/atomicrmw/cmpxchg/fence`, runtime libcalls | **C11** and GNU extensions with **bcc** strengthening; CAS returns old value, weak may become strong. Some RMW vocabulary mirrors LLVM. | O treats every atomic as a barrier, even relaxed; L emits real atomics/libatomic calls; I executes sequential indivisible operations; N refuses atomics (`src/backend/native/support.rs:152`). |
| `Relaxed/Acquire/Release/AcqRel/SeqCst`, failure ordering (`src/ir/instructions.rs:783`) | `monotonic/acquire/release/acq_rel/seq_cst` | **C11/bcc** vocabulary compatible with LLVM; consume is strengthened to acquire | Verifier checks order contracts; L maps orders (`src/backend/llvm/api.rs:1075`); I has no interleaving/happens-before model. No arbitrary shared-IR syncscope: signal fence maps to LLVM single-thread scope only in emitter (`src/backend/llvm/api.rs:1089`). |
| Stable label identities, block addresses, indirect branches and label relocations (`src/ir/instructions.rs:109`, `src/ir/module.rs:1117`) | `blockaddress`, `indirectbr`, constant expressions | **GNU C extension/bcc** with an explicitly LLVM-like invalid-target UB contract | O must preserve address-taken labels; L blockaddress/indirectbr; N code labels/jump tables; I synthetic label objects. Labels are not ordinary ISO C object pointers. |
| Target triple and LLVM data-layout string (`src/ir/module.rs:290`) | `target triple`, `target datalayout` | Target selection is **bcc**; raw LLVM string in shared Module is a **representation leak** | Lowering supplies it (`src/lowering/module_items.rs:1807`); text format preserves it; L consumes it. N/I need target layout facts, not LLVM string grammar. |
| Globals: bytes, zero initialization, relocations, constant/TLS flags (`src/ir/module.rs:1056`) | Global variables/constants, `thread_local`, initializers | **C/target/bcc** flattened object representation | L/N emit global storage and relocations; I builds static/thread storage. C nominal type and subobject identities are absent. |
| External/internal/weak/selectany/linkonce, aliases, sections, retention, visibility and DLL storage (`src/ir/module.rs:872`, line 925) | Linkage, aliases, `llvm.used`, visibility, DLL attributes; own-name `any` COMDAT | **C** internal/external linkage plus **platform/GNU/MSVC** requirements. Selectany/linkonce policies are described with LLVM `weak_odr/linkonce_odr` terminology. | L maps flags and owns COMDAT (`src/backend/llvm/api.rs:569`); N independently emits ELF/COFF COMDAT sections (`src/backend/native/emit.rs:269`); I resolves aliases/weak symbols but has no real linker/interposition. Not all of this belongs to expression semantics. |
| Constructors/destructors, module assembly, inline-control and private inline copies (`src/ir/module.rs:1019`, `src/ir.rs:203`) | `llvm.global_ctors/dtors`, module asm, `noinline/alwaysinline`, `available_externally` | **bcc/extension** behavior, with LLVM-matched inline policy | L/N emit supported module features; I startup honors initialization policy but cannot execute assembly. O's inline-copy resolver reproduces LLVM choices for N/I (`src/pipeline/middle_end.rs:143`). |
| Text parser/printer and verifier (`src/ir/parser.rs:1`, `src/ir/printer.rs:1`, `src/ir/verifier.rs:1`) | LLVM assembly and verifier play analogous roles | **bcc** grammar/invariants: not LLVM assembly | O tests, I textual runner and all backend entry checks depend on it. Verifier checks types/SSA/profiles/flags/orders, not that dynamic C preconditions or claimed `inbounds` facts hold. |

### Concrete semantic coupling and its cost

**1. Deferred UB is the common value algebra.** Signed C addition becomes
`iadd nsw` (`src/lowering/expressions.rs:1142`). Violating the promise produces
poison, rather than recording that a C undefined operation occurred. I must
reproduce division's immediate UB versus shifts'/overflow's deferred poison
(`src/backend/interpreter/arithmetic.rs:4`, line 39), propagation, `select`
masking and freeze (`src/backend/interpreter/execute.rs:212`). O has its own
integer poison evaluator, a refinement lattice in SCCP
(`src/optimizer/sccp.rs:15`) and rules about unchosen arms and flags in GVN
(`src/optimizer/gvn.rs:47`). Its contract explicitly says it optimizes the IR,
not C (`src/optimizer.rs:79`). This is the central maintenance dependency.

N does **not** track poison dynamically. It materializes explicit poison as zero
(`src/backend/native/isel.rs:381`) and freeze as a copy (line 634). Its documented
contract drops `nsw/nuw/exact/inbounds` and chooses concrete results for
poison-producing machine operations (`docs/design/native-backend.md:727`).
This is an intended refinement, not evidence of wrong code. N nevertheless has
to justify freeze consistency, width semantics and speculation against an
LLVM-like model that a C execution engine would not need.

**2. Clang's `noundef` policy is part of C lowering.**
`src/lowering/module_items.rs:754` explicitly calls its choice Clang's
scalar-argument model; lines 782-784 copy the tested absence of return
`noundef` at O0 and mark variadic arguments. `middle-end.md:312` pins this to
bundled Clang 23.1.1 and lines 335-341 explicitly acknowledge that the broad
indeterminate-value assumption does not follow from the cited C99 rules for
all non-trapping/character representations. This is unusually clear evidence
of LLVM/Clang policy becoming shared semantics, not an inferred resemblance.
I checks these boundaries (`src/backend/interpreter/machine.rs:266`, line 399);
A rewrites them alongside ABI indices. Changing a Clang policy can therefore
affect interpreted C behavior as well as LLVM output.

**3. Representation bytes force freeze into frontend and ABI code.**
`src/lowering/bit_fields.rs:12` explains why partial bit-field writes load and
freeze unwritten bytes. Any unwritten byte makes a scalar load poison
(`src/backend/interpreter/memory.rs:181`), so careless widening would destroy
defined neighbor bits. LLVM aggregate register-return packing consequently uses
byte vectors and per-byte freeze (`src/backend/llvm/function.rs:644`), while I
duplicates byte-sensitive register-return handling
(`src/backend/interpreter/machine.rs:402`). C needs object representations,
padding and bit-fields; it does not require this particular whole-scalar
poison/freeze mechanism. The current byte granularity also cannot describe
every independent indeterminate bit of a storage unit.

**4. Pointer rules lose C domains while inheriting a low-level promise.**
Normal pointer arithmetic sets `INBOUNDS` (`src/lowering/places.rs:528`);
I checks the containing allocation, not a retained C array/subobject domain
(`src/backend/interpreter/memory.rs:351`). C99 §6.5.6p8-9 constrains array
arithmetic and subtraction more specifically. Subtraction becomes two
`ptrtoint`s, subtraction and `sdiv exact`
(`src/lowering/expressions.rs:1086`), so the IR cannot diagnose different-array
subtraction from that sequence alone. I explicitly documents address-based
`inttoptr` as a deterministic stand-in, accepting integers not derived from the
object (`src/backend/interpreter/memory.rs:23`). LLVM provenance is itself more
subtle than this; do not call this model a complete implementation of LLVM
provenance, or claim C99 uniquely specifies a modern provenance model.

**5. Backend boundaries contain some platform policy but leak others.**
The alignment cap is justified by LLVM-C argument width in the shared type
(`src/ir/instructions.rs:618`). Module stores an LLVM data-layout string.
Inline-copy handling makes non-LLVM consumers reproduce LLVM choices
(`src/pipeline/middle_end.rs:154`). Conversely, native COMDAT emission shows why
COMDAT and DLL storage are real platform requirements; removing their names
does not remove the requirement. Keep logical linkage and object-file policy
separate rather than treating everything also present in LLVM as a leak.

**6. Atomics are shared C11 functionality, not evidence that C99 needs LLVM
syncscope.** The IR deliberately uses C-style `relaxed`, maps consume to acquire,
and hides single-thread LLVM fence scope in L. The extra concerns are a broad
LLVM-shaped RMW menu, conservative optimizer barriers and capability differences:
I ignores fence/order scheduling in its single-threaded execution
(`src/backend/interpreter/execute.rs:72`); N refuses atomics. An I pass is not
evidence for C11 concurrency correctness.

### C facts erased before the common IR

These are losses of explanatory/analysis power, not automatically miscompiles.
A compiler may choose an allowed evaluation order and need not diagnose every
UB. An educational evaluator has a different obligation to explain the source.

- **Declared/effective types and subobjects.** Lowering collapses types into
  scalar widths and addresses. Memory accesses supply `tag: None`
  (`src/lowering/places.rs:349`); AccessTag is an unused carrier, and O does
  not use type-based aliasing. C99 §6.5p6-7's rules for typed stores to allocated
  objects, character access and copying effective types are not an executable
  shared-IR contract. Struct fields, unions and arrays become offsets/copies;
  I's allocation bounds do not recover those identities.
- **Ordinary automatic lifetimes and identity.** `StackSlotData` has size and
  alignment only. I allocates all fixed slots at function entry
  (`src/backend/interpreter/machine.rs:328`); there are no general fixed-object
  begin/end operations on block exit/re-entry. VLA save/restore and GNU cleanup
  transitions *are* retained and implemented (`src/lowering/vla.rs:225`,
  `src/lowering/cleanup.rs:1`), so it would be inaccurate to say all lifetimes
  are lost. Sema already retains an execution-region tree
  (`semantic-analysis.md:824`) and storage duration
  (`src/translation_phases/semantic_analysis/results.rs:177`); use these before
  lowering erases them. C99 §6.2.4p2 also makes pointers indeterminate at the
  pointed-to object's lifetime end, a stronger story than an allocation-bound
  check at a later dereference.
- **Indeterminate values versus invalid operations.** An undefined SSA local
  becomes poison (`src/lowering/ssa.rs:18`); uninitialized memory and a stored
  poison use the same cell tag (`src/backend/interpreter/memory.rs:150`). The
  reasons, source declarations, padding distinctions and possible trap
  representations are lost. A successful freeze/unused-poison run cannot
  establish that the original C execution had no UB. C99 §6.2.6.1p5 treats
  non-character trap reads specially; C11 §6.3.2.1p2 additionally has explicit
  uninitialized automatic-object wording absent from C99's paragraph. A new
  model must state which revision and implementation choices it follows.
- **Sequencing and unspecified order.** Short circuit, comma and conditional
  execution are preserved as CFG behavior, not explicit source sequence-point
  events. Other operands are scheduled in a selected order
  (`src/lowering/expressions.rs:159`); compound assignment explicitly chooses
  RHS-first (line 192); arguments are scheduled at line 242. Once linearized,
  the distinction between required sequencing and a chosen order is gone.
  C99 §6.5p2 and C11's sequenced/unsequenced relations cannot be reconstructed
  from instruction order. Retained effects-only recipes
  (`src/translation_phases/semantic_analysis/facts/effects.rs:748`) help find
  what must execute; they are not by themselves a complete sequencing model.
- **Source operations and reasons for failure.** `Body` retains opaque IDs for
  call diagnostics, with messages/source outside it (`src/ir/function.rs:55`),
  not general source origins or a C event stream. Promotion, bit-field
  expansion, ABI copies and optimization further separate instructions from
  source actions. Sema's typed syntax remains the best place to capture this.
- **Nonlocal-jump indeterminacy.** I recognizes setjmp/longjmp and resumes
  frames, keeping changed nonvolatile values as a permitted implementation
  choice (`src/backend/interpreter/nonlocal.rs:11`). It does not emit an
  explanation that those C locals became indeterminate. This is useful IR
  execution, but insufficient for the sibling site's C object display.

The [LLVM UB manual](https://llvm.org/docs/UndefinedBehavior.html) explains why
deferred UB and freeze serve optimization. It does not establish that they are
the right user-facing semantics for a C abstract-machine engine.

## 2. Survey of alternatives

Research in progress: CompCert's staged languages and memory model; Cerberus
Core; CLIF and acyclic egraphs; QBE; Makarov MIR; multi-level dialects; RVSDG;
and sea-of-nodes/Turboshaft. The comparison will evaluate semantic ownership,
three-back-end reuse, implementation cost and abstract-machine tracing rather
than visual similarity to LLVM.

## 3. Recommendation and migration

Working hypothesis: add an executable, object-aware C IR above the existing
scalar CFG, keeping the current low-level optimizer and ABI layer initially.
Make C undefined operations, indeterminate representations, source sequencing
and object lifetimes explicit before erasing them. Translate the resulting
contracts to LLVM poison/attributes only at the LLVM boundary.

This would give the abstract-machine website a source-level execution surface
that the current byte/scalar IR cannot reconstruct. A recommendation requires
an explicit policy for pointer/integer casts, subobjects, unspecified evaluation
order and the distinction between a diagnostic execution mode and release
optimization.

## 4. Prototype and validation status

No prototype yet. The inventory, primary-source survey and implementable
design take priority. No cargo commands have been run.
