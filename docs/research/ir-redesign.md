# An object-aware IR for bcc

Study baseline: `145898d4` (`origin/main` when this worktree was created),
2026-10-21. All repository `file:line` references in this document refer to
that commit. Parts 1–3 are the design study; part 4 records the isolated
implementation added on 2026-10-23, outside the default build and pipeline.

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
| Floating arithmetic and negation (`src/ir/instructions.rs:308`) | `fadd/fsub/fmul/fdiv/frem/fneg` | **bcc/LLVM** format-specific primitives implementing C real/complex operations; no fast-math flag family or explicit rounding/exception-state operand. `frem` is a low-level remainder, not C's integer `%`. | L maps operations directly; N uses SSE/x87 or helpers; I uses host f32/f64 arithmetic and refuses extended-format arithmetic (`src/backend/interpreter/arithmetic.rs:109`). O must preserve the chosen primitive contract. |
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
| Aggregate/result descriptors, `byval/sret/registers`, narrow `signext/zeroext` (`src/ir/module.rs:574`, `src/ir/module.rs:633`) | ABI coercions and parameter/return attributes | Pre-ABI copy semantics are **bcc** implementing C by-value calls; post-ABI choices are **target ABI**, expressed partly in LLVM terms | A creates them; O sees pre-ABI descriptors; L/N/I support target packing. I also supports pre-ABI calls; the ordinary CLI currently selects post-ABI interpretation. |
| Parameter, result and variadic `noundef` (`src/ir/module.rs:501`) | LLVM `noundef` | **LLVM** contract plus a **bcc** C-lowering policy matched to Clang | A remaps indices; O uses entry-parameter definedness; L emits attributes; I raises boundary UB; N relies on refinement, with no extra machine checks. |
| `va_start/va_arg/va_copy/va_end`, aggregate `va_arg` (`src/ir/instructions.rs:373`) | `llvm.va_*`, `va_arg`, target expansion | **C99** functionality; cursor representation is **bcc/target**, intrinsic spelling is LLVM-facing | A expands `va_arg`; L retains some intrinsics; N supports target vararg setup but refuses residual `va_arg`; I has cursor and ABI machinery (`src/backend/interpreter/varargs.rs:1`). |
| `returns_twice`, external setjmp/longjmp contracts (`src/ir.rs:197`, `src/backend/interpreter/nonlocal.rs:1`) | Function/call `returns_twice`; library calls | **C99** nonlocal jumps plus **bcc** external-symbol recognition; attribute is an LLVM-oriented summary, not a full continuation model | O avoids unsafe inlining; L marks calls/declarations; I saves/resumes frames; N refuses returns-twice calls/addresses (`src/backend/native/support.rs:160`). |

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
| Constructors/destructors, module assembly, inline-control and private inline copies (`src/ir/module.rs:1019`, `src/ir.rs:206`) | `llvm.global_ctors/dtors`, module asm, `noinline/alwaysinline`, `available_externally` | **bcc/extension** behavior, with LLVM-matched inline policy | L/N emit supported module features; I startup honors initialization policy but cannot execute assembly. O's inline-copy resolver reproduces LLVM choices for N/I (`src/pipeline/middle_end.rs:143`). |
| Text parser/printer and verifier (`src/ir/parser.rs:1`, `src/ir/printer.rs:1`, `src/ir/verifier.rs:1`) | LLVM assembly and verifier play analogous roles | **bcc** grammar/invariants: not LLVM assembly | O tests, I textual runner and all backend entry checks depend on it. Verifier checks types/SSA/profiles/flags/orders, not that dynamic C preconditions or claimed `inbounds` facts hold. |

### The consumer boundaries already present

The native MIR is a distinct x86-64 machine representation, not another copy
of the shared IR (`src/backend/native/mir.rs:1`). `MFunction` owns layout-ordered
blocks, virtual-register classes, frame objects, outgoing/incoming call areas
and constants (`src/backend/native/mir.rs:36`). GPR/SSE classes and register
pairs represent scalar types; `MInst` has two-address operations and explicit
machine operands (`src/backend/native/mir.rs:801`). Instruction selection starts
at `src/backend/native/isel.rs:104`; branch arguments become parallel copies,
with edge blocks and cycle breaking (`src/backend/native/isel/edges.rs:1`).
The shared poison/flag contracts are resolved or dropped during selection;
they are not carried through register allocation as MIR value states. Preserve
this useful separation in either redesign.

The interpreter API accepts either verified IR profile
(`src/backend/interpreter.rs:135`). It already implements target-independent
logical vararg cursors before ABI lowering and target `va_list` objects after
it (`src/backend/interpreter/varargs.rs:1`). The CLI sends post-ABI IR to every
backend (`src/cli/code_generation.rs:9`, `src/cli/code_generation.rs:120`). Thus
interpreting pre-ABI IR is an available baseline experiment, not a feature
that requires a new evaluator. It removes ABI noise but does not restore C
object types, source sequencing or indeterminate-value reasons erased earlier.

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
defined neighbor bits. Shared aggregate-argument ABI packing freezes scalar
pieces and zeroes padding (`src/abi/rewrite.rs:571`), exposing the dependency
outside L. LLVM aggregate register-return packing uses byte vectors and
per-byte freeze (`src/backend/llvm/function.rs:644`), while I duplicates
byte-sensitive register-return handling
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

Primary sources below were read on 2026-10-21. Upstream `main`/`latest` links
describe designs, not pinned dependencies proposed for bcc. Historical papers
and contemporary implementations are distinguished. Evaluations of bcc cost
and suitability are design judgments, not benchmark results.

### CompCert: Clight → Csharpminor → Cminor → RTL

Clight separates effect-free expressions from assignments, calls and control.
The initial simplification chooses an evaluation order. Csharpminor makes
type-dependent operations explicit; Cminor makes stack storage explicit;
CminorSel introduces machine operations; RTL builds a CFG with pseudo-registers
and three-address instructions. RTL is not simply LLVM SSA with a different
name, and this is a chain with additional intermediate passes, not four
interchangeable formats. The official
[language/pass map](https://compcert.org/doc/) identifies each boundary.

The shared [memory model](https://compcert.org/doc/html/compcert.common.Memory.html)
has fresh blocks, offset-indexed contents, permissions, allocation and free.
[Values](https://compcert.org/doc/html/compcert.common.Values.html) distinguish
integers, floats, block-plus-offset pointers and `Vundef`; it is not LLVM
poison. Memory injections relate allocations as lowering merges or lays out
storage. This is an especially useful discipline: explain how source objects
relate to target bytes instead of starting with target bytes as the definition
of an object.

**For bcc:** borrow the staged semantic contracts and explicit object identity.
A Clight-like executable layer can feed I directly and lower to the existing
pre-ABI CFG for L/N, preserving the native MIR. Dense arena IDs can represent
blocks/objects without importing a proof assistant or CompCert's containers.
Do not adopt `Vundef` and assume indeterminate C behavior is solved, or assume
CompCert's abstract pointer model covers every systems-C integer-pointer idiom
bcc accepts. Its supported C subset/extensions and proof-specific choices are
not bcc's feature set. A faithful port/proof effort is far more work than an
object-aware Rust IR. For the website, Clight after evaluation-order selection
would still need source choice/sequencing annotations; it does not automatically
explore all C orders.

### Cerberus Core

Cerberus elaborates a substantial C11 fragment to executable Core and separates
the thread-local semantics from the memory-object model. It can explore one or
many behaviors. Its [project description](https://www.cl.cam.ac.uk/~pes20/cerberus/)
explicitly treats unclear memory rules as a design space informed by systems
practice, not a claim that every detail follows unambiguously from ISO prose.
Core exposes sequencing: `unseq` permits action interleavings, and binding forms
distinguish sequencing obligations; overlapping unsequenced accesses are
checked. The [2023 semantics thesis, chapters 4–6](https://www.cl.cam.ac.uk/techreports/UCAM-CL-TR-981.pdf)
gives the elaboration, memory interface and operational rules.

**For bcc:** this is the best semantic reference for the sibling site, including
why a single deterministic run does not settle UB freedom, and why a function
argument's chosen order is not a source sequencing edge. Reuse the separation
between an evaluation layer and a replaceable object model. Full Core is a
semantic exploration language rather than a ready-made scalar optimizer; bcc
would still need scheduling, CFG construction, scalarization and ABI lowering
for L/N. A wholesale integration would duplicate the existing frontend and
introduce a substantial toolchain/runtime dependency. Implement a bounded
Core-inspired layer from bcc's existing sema facts and use Cerberus as a
reference on carefully matched subsets, with target/memory-policy differences
recorded. Do not promise to reproduce Core's full nondeterministic evaluator in
the first experiment.

### Cranelift CLIF and the acyclic-egraph mid-end

CLIF is a typed SSA CFG with block parameters, explicit memory and low-level
calls. It is function-oriented; a frontend supplies module and C-ABI policies.
The [IR reference](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/docs/ir.md)
itself directs readers to current instruction documentation. Arithmetic is
specified directly: wrapping integer operations, explicit overflow results and
trapping operations/conversions are available in the
[instruction API](https://docs.rs/cranelift-codegen/latest/cranelift_codegen/ir/trait.InstBuilder.html).
This avoids importing LLVM poison as the universal scalar value algebra; it
does not supply C object/effective-type semantics.

The egraph design keeps an ordered effect/control skeleton and optimizes pure
expressions using acyclic equivalence classes, eager rewrites and scoped
elaboration back into scheduled code. It is not unrestricted equality
saturation over arbitrary effectful C operations. Read both the
[original RFC](https://github.com/bytecodealliance/rfcs/blob/main/accepted/cranelift-egraph.md)
and the implementer's [2026 account](https://cfallin.org/blog/2026/04/09/aegraph/),
which describes the evolved implementation.

**For bcc:** most useful as inspiration for a poison-free scalar layer and,
later, cooperative pure-expression optimization. The existing dense IDs,
block arguments and rebuild-based optimizer already fit that direction.
Replacing the IR with library CLIF would require custom adapters for globals,
extended floats, aggregates and C varargs, and still require an LLVM translator
and independent I semantics. Using Cranelift codegen would also displace the
native back end the owner wants to retain. Preserve bcc's MIR and ABI work;
experiment with egraphs only after operation semantics stabilize. Egraphs
cannot decide whether a C load is well typed or an indeterminate read is UB.

### QBE

QBE's [IL specification](https://c9x.me/compile/doc/il.html) defines a small SSA
language with four temporary base types (`w/l/s/d`), phi nodes, integer-typed
addresses, loads/stores, calls and varargs. Aggregates are described for C
interfacing and supplied through memory addresses; the back end handles ABI
classification. Its compact type system and direct arithmetic avoid much of
LLVM's contract surface.

**For bcc:** QBE demonstrates that by-value aggregate intent can survive until
a backend ABI pass without general aggregate SSA values—something bcc already
does. Borrow small operation families and self-contained pass boundaries.
Adopting it wholesale would give up or separately legalize bcc's i128,
x87/binary128 and richer extension/atomic surface, require another native
backend, and leave L/I translation to maintain. Integer addresses make C
provenance and object explanation harder unless a higher layer preserves them.
The benefit is engineering restraint, not a ready C abstract machine. Compare
the vocabulary, not just QBE's textual terseness; fewer documented contracts can
also mean more work defining an independent interpreter oracle.

### Vladimir Makarov's MIR

This is the separate [MIR project](https://github.com/vnmakarov/mir), not Rust MIR
and not `src/backend/native/mir.rs`. It combines a compact machine-independent
representation, interpreter/JIT and a C frontend. The
[MIR specification](https://github.com/vnmakarov/mir/blob/master/MIR.md)
describes imperative local registers, labels/branches, typed memory operands,
prototype-based calls and block arguments/results for ABI use. Pointers have
target integer representations; `long double` is target-dependent. The public
representation does not require source SSA construction; optimizations can use
their own internal forms.

**For bcc:** attractive if the goal were a small executable/JIT substrate with
straightforward mutable locals. A direct interpreter could be simpler than
today's post-ABI SSA interpreter, and the instruction selector might be easy
to feed. But changing bcc to imperative registers would either replace much
of O or rebuild SSA for it, while L still requires SSA construction. MIR does
not provide the missing C object lifetimes, sequencing graph or effective
types, and its C/allocator/runtime integration is another dependency to own.
Borrow the idea of a simple executable contract and explicit call prototypes,
not its low-level address model as the meaning of C. It has little advantage
over retaining bcc's existing scalar CFG plus a new C layer.

### MLIR-style levels: a C dialect over a scalar dialect

MLIR separates extensible operations/types, regions and dialect legality from
lowering machinery. It can represent high and low levels together; neither
regions nor dialects require LLVM semantics. Its
[rationale](https://mlir.llvm.org/docs/Rationale/Rationale/) describes the
multi-level approach; [dialect conversion](https://mlir.llvm.org/docs/DialectConversion/)
makes legal/illegal operations and type conversion explicit. A C dialect can
retain `object.begin`, typed reads, member places and C calls, lowering to
scalar operations only when the relevant facts are no longer needed.

**For bcc:** this is the best architectural fit **as a pattern**. Implement two
small Rust representations using the existing arenas and verifier discipline;
do not make adopting the MLIR C++ framework a prerequisite. L/N consume the
lower level; I and the site can consume the C level. A full MLIR dependency
adds binding/build/version costs and still requires bcc to specify the C
dialect. Merely using an existing LLVM-oriented C dialect can reproduce
`nsw/freeze` in a higher-looking syntax. The risks are duplicate operations,
poorly defined mixed levels and accidental fallbacks that silently erase
unsupported C constructs. Distinct typed stages and an explicit legality
check are more useful here than an open-ended dialect registry.

### RVSDG

The Regionalized Value State Dependence Graph represents computation through
dependencies and nested regions. Value edges carry data; state edges order
effects. Structural nodes represent alternatives (gamma), loops (theta),
functions (lambda), globals (delta), recursion (phi) and a translation unit
(omega). Its [specification and evaluation](https://arxiv.org/html/1912.05036v2)
also discuss constructing it from control flow and recovering control flow for
code generation. These phi nodes describe recursive environments, not LLVM
block-entry phis. The [jlm research compiler](https://github.com/phate/jlm)
provides an implementation to investigate.

**For bcc:** promising for research into unified interprocedural/loop
transformations and making dependence explicit. Objects and UB would still
need separate operation semantics. C's irreducible gotos, computed goto,
setjmp/longjmp and arbitrary exits impose restructuring and escape-handling
costs before either L or N can consume the graph. The existing CFG optimizer
would mostly be rewritten, while I would need a new region evaluator and the
site would still need source sequencing distinctions. Keep it as an optional
future optimization form for suitable regions, not the source semantic
backbone. A global memory state edge is easy but serializes effects; splitting
state demands the alias/provenance analysis this study is trying to define.

### Sea of nodes, and V8's retreat to Turboshaft

A sea-of-nodes IR lets pure computations float among value, effect and control
dependencies, delaying scheduling. It does not by itself choose C semantics.
V8's [March 2025 explanation](https://v8.dev/blog/leaving-the-sea-of-nodes)
reports that many operations were pinned by guards/effects anyway; manually
maintaining parallel control/effect chains was difficult; scheduling could
undo earlier commoning; graph traversal and in-place pointer-heavy layouts
cost compilation time and locality. V8 moved its JavaScript backend and Wasm
pipeline to the CFG-based Turboshaft, while some earlier/builtin paths still
used the old form at the time of that report. This is engineering evidence
from V8, not a theorem that sea of nodes is always inferior.

**For bcc:** C's static types reduce some JavaScript guard traffic, but calls,
aliasing, volatile, atomics and partial operations still constrain effects.
Changing the whole optimizer and scheduler would leave the LLVM poison issue
untouched and introduce a large graph maintenance burden, contrary to bcc's
dense arena/rebuild design. Keep the CFG; if pure expressions need flexible
commoning, study CLIF's bounded graph-within-a-skeleton approach. For the site,
a scheduling freedom graph is less useful than a source sequencing graph: an
optimization dependency and a C sequence point are different facts.

### Decision comparison for these three back ends

| Design influence | What it resolves for bcc | Main cost or remaining gap | Disposition |
| --- | --- | --- | --- |
| CompCert staged languages | Explicit semantic erasure boundaries and object-to-storage relation | Full proof chain/subset is not bcc's compiler | Borrow stage discipline and object identities |
| Cerberus Core | Sequencing, UB explanation, parameterized memory policy | Exploration and lowering complexity | Primary semantic reference; bounded implementation |
| CLIF/aegraph | Direct scalar semantics; optimizer cooperation | Does not preserve C objects on its own | Borrow scalar contract; defer egraph experiment |
| QBE | Small IL and backend-owned ABI | Narrower primitive surface; no C object oracle | Borrow simplicity; retain bcc backends |
| Makarov MIR | Simple imperative executable substrate | SSA rebuild/optimizer rewrite; still low-level | Reference, not replacement |
| Multi-level dialect pattern | C execution plus low-level optimization/three consumers | Two verifiers and one carefully tested erasure | Recommended architecture without MLIR dependency |
| RVSDG | Region/dependence optimization research | CFG restructuring, escape semantics, broad rewrite | Optional later regional experiment |
| Sea of nodes | Flexible pure scheduling | Effect-chain/scheduler cost; semantics unchanged | Reject as the main redesign objective |

## 3. Recommendation and migration

**Recommend candidate A: a small executable C/object IR above a scalar CFG.**
Borrow CompCert's staged erasure and Cerberus's separation of evaluation and
memory policy, while retaining bcc's CFG, arena IDs, ABI implementation and
native MIR. Introduce it additively; retire the LLVM-derived shared value
algebra in a later, separately measurable phase. An additive C layer by itself
helps the website but does **not** finish semantic decoupling: that requires
the scalar contract/emitter work below.

Candidate B is a smaller fallback if the owner prioritizes backend independence
and cannot fund a C execution layer. It replaces shared poison semantics in the
current scalar CFG but supplies much less value to the website. Neither design
requires adopting a large external compiler framework.

### Candidate A: C operations, logical objects, then scalarization

The following names and syntax are proposals, not existing APIs:

```text
typed syntax + retained semantic facts
    -> CModule                         C objects/evaluations/logical calls
         -> C evaluator + event sink    website / diagnostic execution
         -> select order, lower places and representations
              -> ScalarModule          typed CFG; explicit bcc contracts
                   -> target ABI plan   logical values -> registers/memory
                        -> LLVM emitter
                        -> native selector -> existing x86 MIR
                        -> scalar interpreter (optimizer/ABI oracle)
```

During migration the ScalarModule position is occupied by today's IR, through
an explicitly named legacy adapter. It remains available as an oracle while
the replacement is validated. The final C evaluator does not execute target
ABI coercions for ordinary internal calls.

#### Types and storage

Use immutable, module-owned dense IDs and arena tables, not borrowed pointers
to sema internals whose arena may be released. Intern a reachable snapshot of
semantic types, layouts and source origins; avoid copying the entire syntax
tree. Keep arena allocation, nonrecursive work stacks and compact instruction
records, but do not force every new semantic record into 16 bytes before
measurement.

```text
CType = Void | Bool | Integer{rank, signedness, target_width}
      | Real{format} | Complex{component}
      | Pointer{pointee} | Array{element, extent}
      | Record{kind: struct|union, members, layout}
      | Function{params, result, variadic, language_call_kind}
QualifiedType = {CTypeId, const, volatile, restrict, atomic}
ObjectDecl = {type, duration, lexical_region, layout, origin}
Place = Local(ObjectDeclId) | Global(SymbolId) | Deref(value)
      | Member(place, member_id) | Index(place_or_pointer, index)
      | BitField(place, member_id)
Value = typed scalar | aggregate snapshot | logical function/label value
```

C integer rank and nominal record identity matter even when widths match.
Pointer pointee types describe C accesses; pointer runtime provenance is a
separate property. Arrays are places, with decay explicit; an array is not
copied as a general C value. Aggregate snapshots have value semantics at calls,
assignment and return, but can initially be implemented as immutable hidden
storage plus a descriptor. Avoid eager scalar tuples that invent meaningful
padding values or accidentally alias the source after a copy.

Reuse target layout facts from sema for `sizeof`, bit-field layout, padding,
endianness and float formats. This is a C IR parameterized by an implementation,
not an unrealistic target-free C machine. `_Atomic`, volatile and bit-fields
remain visible until their access semantics are lowered. Extern calling
convention extensions and symbol policies are typed module descriptors,
separate from logical function argument/result types. Keep link-time policies
in a shared symbol layer; LLVM emits attributes, native emits ELF/COFF policy.

#### Objects, lifetime and provenance

The C evaluator's memory contains objects with distinct identities and
generations, extent/alignment, storage duration, declared/effective-type state,
subobject layouts, representation bits and pointer fragments. An object's
storage address is an implementation detail; object identity is not its current
stack offset. Fixed locals can later be promoted or share physical storage while
their C identities remain distinct in the unoptimized execution model.

An operational pointer contains `{object, generation, byte_offset,
arithmetic_domain}` plus a null or unresolved-address case. The arithmetic
domain identifies the C array/subobject range, including its one-past position;
it is not just the whole allocation. Member selection, decay, `&`, and casts
construct or transform places/domains according to C rules. Typed access checks
lifetime, access bounds, alignment and permitted effective type. Pointer
subtraction remains `ptr_diff(p, q, element_type)` until it can preserve the
same-array and representable-`ptrdiff_t` preconditions. Comparisons use their
own C equality/relational rules; never reduce pointer equality to token equality
alone. Layout, adjacent objects and one-past equality need separate cases.

Represent lifetime transitions explicitly, and get their timing right:

- A fixed automatic object's lifetime begins on entry to its associated block,
  including paths jumping past its declaration; its initializer executes when
  the declaration is reached. A declaration reached again can make its value
  indeterminate without necessarily beginning a new lifetime.
- VLA lifetime begins at its declaration and ends when execution leaves its
  scope. Leaving a block through return/goto/break/continue applies its region
  transitions. Recursion and repeated block entry create new instances.
- Heap allocation, free and realloc have separate identity/representation
  effects. Static and thread objects have their own initialization rules.

These distinctions come from C99 §6.2.4p5-6, not from where an LLVM alloca
happens to be placed. Consume sema's region/jump facts rather than rediscovering
scope exit from final CFG shape. Mark objects dead and retain enough generation
history to explain stale references; do not eagerly scan every pointer in
memory at `object.end`. A later use can classify the reference using the dead
generation, while reporting that pointer indeterminacy began earlier.

For effective types, retain both declared types and typed access operations.
For allocated objects without a declared type, distinguish a non-character
typed store, representation copying from another object and other accesses as
C99 §6.5p6 specifies. Character accesses are an explicit exception; qualifiers,
signed/unsigned compatible access and aggregate/union cases need their own
rules. Do not import a C++ rule that every read of an inactive union member is
automatically invalid. Begin conservatively: disable type-based optimization
until these rules and the relevant union/copy tests are agreed.

**Pointer/integer policy is a named implementation decision, not a solved
consequence of C99.** Propose explicit `expose_address(ptr)` and
`recover_pointer(integer, access_type)` operations. The first can record an
exposure effect; the latter consults the implementation's recovery policy.
Prototype an exposure-based object model, allowing round trips and ordinary
address arithmetic for exposed objects, with choices recorded where recovery
is ambiguous. Keep today's permissive address lookup as a separately named
compatibility mode. Foreign/MMIO addresses require explicit environment policy.
Do not exploit a strict-provenance assumption in the release optimizer while
the interpreter is still using permissive recovery. Test pointer-copy-through-
bytes, integer round trips, tagged addresses, one-past boundaries, reuse after
free and external pointers before enabling provenance optimizations. Cerberus's
[provenance research](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n2311.pdf)
is a reference for selecting and documenting a policy, not permission to label
one experimental policy as the uniquely mandated C99 model.

#### Undefined behavior and indeterminate representations

Do not add `Poison` to the C Value enum. A C operation returns a value/effects,
a choice over permitted values, a structured undefined-execution result, or an
unsupported-feature result. The latter two must remain distinct.

```text
Undefined = {rule_id, source_operations, object_instances, explanation_data}
Choice = {choice_id, domain, reason, source_origin}
Representation = {known_bits, indeterminate_bits, pointer_fragments,
                  padding_map, origins}
```

Signed addition is a partial C operation with an overflow precondition;
unsigned addition wraps. Invalid shifts, bad pointer arithmetic, division by
zero, undefined floating-to-integer conversions and lifetime/type failures
retain their own reasons. Out-of-range signed integer conversions instead
follow the implementation-defined result or signal policy in C99 §6.3.1.3p3.
This detects the undefined C operation when evaluated, even if its
result would later be discarded. It does not turn UB into a C-defined runtime
exception. The diagnostic evaluator stops and explains; compiled release code
has no obligations on that undefined execution.

Indeterminate storage is an object-representation state, not either zero or
automatic UB on every byte copy. C99 defines an indeterminate value as an
unspecified value or trap representation (§3.17.2p1). Track meaningful value
bits separately from padding; copying representations preserves what needs to
be preserved without performing a typed value read of every member. A bit-field
store changes its bits without reading neighboring bits as an integer value.
Aggregate return packing must not let unspecified padding invalidate a defined
field. A permitted unspecified-value observation creates a ChoiceId; uses of
that resulting value agree, but this need not force all later reads of the
same indeterminate object to make the same choice.

The exact treatment of non-trapping indeterminate reads needs an explicit
language/policy table. Include the C99/C11 difference noted above and the
current Clang-compatible argument assumption as a compatibility policy. Do not
claim a fully settled standard model or silently diagnose every such case as
mandatory ISO UB. Where the first implementation lacks a settled rule, report
unsupported/policy-dependent execution. Only permit optimization assumptions
that the selected release policy justifies. In particular, derive argument
definedness from the operation/policy; do not define C validity by whether
Clang happened to emit `noundef` at O0.

#### Control, sequencing and choices

Keep the function CFG with block parameters for statement-level control and
loops. It already handles gotos, switch and irreducible flow. Attach small
evaluation regions to source full expressions: ordered evaluation, unsequenced
operand evaluation, short-circuit/conditional alternatives and explicit source
sequence boundaries. Each operation has an OriginId and a language evaluation
identity. This is a bounded semantic structure over a CFG, not a wholesale
RVSDG or sea-of-nodes scheduler.

Initially choose one documented legal schedule for executable code and
deterministic stepping. Keep the required sequencing relation separately from
that schedule. The checker compares dynamic object/subobject access footprints
and source sequencing identities; linear execution order alone cannot legalize
`i++ + i++`. Do not implement the checker as “two writes in a full expression
always fail”: short circuit, nested sequence points, calls and C11's
indeterminately sequenced function executions matter. C99's sequence-point
rules and C11's relations need separate rule handling. Alternative schedules
and bounded exploration can be added after a correct single-schedule evaluator;
a single schedule must disclose that it does not explore all behavior.

Sequence points constrain completion of side effects; they do not require all
stores to be buffered until the next boundary. Trace events distinguish value
computation, side effects and required completion so the site can display
pending work without teaching that false rule. Compiler-only instruction
ordering must not appear as a C sequencing guarantee.

Preserve logical calls, return, varargs and exceptional control:

- Internal calls pass C scalar values and aggregate snapshots; an internal
  vararg pack retains promoted C types. `va_arg` checks logical compatibility
  in the evaluator. Only external/native ABI adapters see register-save areas.
- Represent setjmp-like resumptions with an explicit call-effect summary and
  saved activation/continuation. On longjmp, apply lifetime exits and mark the
  affected nonvolatile locals indeterminate according to the chosen rules.
  Unsupported nonlocal extension cases remain explicit refusals during the
  experiment; a `returns_twice` bit alone is not a complete CFG model.
- GNU labels are function-scoped label values with stable identity; computed
  branches retain their possible targets and region-exit restrictions. Convert
  to code addresses only when lowering that extension. Do not invent general
  ISO object provenance for them.
- Carry C11 atomic operation/order and signal/thread fence distinctions in a
  typed extension family. Start with the current single-thread evaluator and
  label it as such; a concurrency explorer/happens-before engine is separate
  work. Native continues to refuse atomics until its backend implements them.

#### The scalar contract and LLVM boundary

The endpoint ScalarModule can retain signless machine types, block parameters,
explicit byte operations and target layouts. It owns its numeric, memory and
call contracts in bcc terminology. There are no shared LLVM poison values,
freeze operation, `noundef` bit or raw LLVM layout string. Keep target layout in
a typed descriptor, generated into an LLVM string inside L. The ABI planner
returns target calling locations/copy/extension decisions; each backend maps
them to its representation. It remains shared target knowledge, not LLVM
semantics. Alignment limits belong to target/emitter validation, not a maximum
chosen solely for one foreign API.

For potentially undefined scalar arithmetic, use either explicitly partial
operations or total operations with separate preconditions. Prefer the latter
for the first experiment:

```text
entry(a: bits32, b: bits32):
    (sum, overflow) = signed_add_checked a, b  // total; sum is wrapped bits
    require !overflow, SignedOverflow, origin
    return sum
```

`require` means failing this source precondition makes this execution undefined;
it is not a generic language-level trap. In checked execution it diagnoses and
is ordered with other source checks. Release optimizations may remove it or
refine an undefined path; valid executions and their observable behavior must
remain valid. Separate the pure `checked` computation from the non-speculatable
check: pure bits can be moved, but a check must not be hoisted to an execution
where the source operation was not evaluated. Division/conversion primitives
must similarly define their total result/status without executing a host UB or
hardware exception before the check. This is a specification cost, not solved
by renaming `sdiv`.

Mapping example: L can emit ordinary LLVM `add` without flags and preserve all
defined C results. It may emit `add nsw` when the C/Scalar contract establishes
that its violating cases were already undefined. The native selector emits a
normal add; the scalar interpreter checks preconditions. Range analysis can
remove redundant checks. These backends need not implement LLVM poison to
share the C contract. Check deletion is valid release refinement, but not a
trace-preserving diagnostic transformation; execute the unoptimized CModule
for source stepping.

At L's boundary specifically:

- Map object/byte offsets to GEP; add `inbounds` only with a compatible object,
  range and integer-width argument. Use unflagged GEP for opaque/recovered
  pointers when the stronger LLVM promise is not justified.
- Map C/target definedness guarantees to parameter attributes only when they
  apply to the actual lowered scalar/register bits. Never make padding bytes
  `noundef` just because a logical aggregate value is valid.
- Materialize unspecified representation bits in a way preserving defined
  neighbors. Byte/lane freeze can remain a private LLVM implementation detail,
  with a short local explanation of why it refines the source choice domain.
- Map bit-count/overflow/stack operations to intrinsics; the C/shared contracts
  describe results and effects, including zero and overflow cases. LLVM's
  zero-poison parameter is not the source builtin specification.
- Lower C11 orders, signal scope, linkage, COMDAT and DLL requests here. N uses
  the same logical requests to emit its own machine/object-file features.
- Restrict speculative annotations and provenance assumptions on recovered
  addresses until their mapping has been validated against pinned LLVM.

N continues to use its existing x86 MIR register classes, parallel-copy lowering,
frame layout, instruction selection and register allocator. It consumes scalar
operations/ABI plans rather than C objects. A checked native mode could lower
requires to checks/traps later; release N need not simulate the educational
object store. The scalar interpreter remains useful to test O and the ABI
adapter, while the C evaluator becomes the source-semantic oracle. Share small
bit/float utilities where correct, but keep the semantic transitions independent
enough that both evaluators do not merely repeat the same lowering bug.

#### A concrete C-level example

For `int f(int n) { int s = 0; while (n > 0) { s += n; --n; } return s; }`,
the first CModule can keep logical local reads/writes; promotion is optional:

```text
function f(n: C.int) -> C.int {
entry:
    region.enter function_body       // starts fixed automatic instances
    local.initialize s, 0
    jump loop
loop:
    eval condition: read n; compare_gt 0; sequence.end
    branch condition, body, done
body:
    eval sum: read s; read n; signed_add; write s; sequence.end
    eval decrement: read n; signed_sub 1; write n; sequence.end
    jump loop
done:
    result = eval read s
    sequence.end
    region.exit function_body
    return result
}
```

Parameter-object setup and origins are omitted for readability. `signed_add`
reports its own C overflow, not a poison result waiting to be used. Region
boundaries preserve source instances even if later scalarization promotes s/n
to loop block parameters. `sequence.end` marks a C boundary, not a CPU fence.
The record of operand relations must still accompany general expressions;
this simple loop alone would not validate the full semantics.

### Candidate B: replace the shared scalar semantics in place

Keep today's Type set and CFG, memory-backed aggregates and ABI pass. Replace
poison/flags/freeze with total arithmetic plus explicit preconditions and
choice operations, as above. Introduce stack lifetime instructions where sema
already provides transitions, and move LLVM layout/attribute names into L.
Separate logical ABI descriptors from LLVM parameter attributes. Retain the
current allocation-bound memory model with a named pointer-recovery policy;
do not claim effective-type or subobject enforcement has appeared just because
the poison enum disappeared.

Indeterminate bytes remain distinct from a failing arithmetic precondition.
`choose_bits(mask, origin)` makes allowed unspecified representations concrete
once per value, preserving known bits; it cannot repair a trap representation
or an invalid C read. L may privately use freeze; N picks permissible bits;
I keeps masks/reasons. `noundef` becomes an emitter-derived fact rather than
an IR-level definition of argument validity. This change reaches O's folding,
SCCP, GVN, promotion, selects and LICM, not just `ir/instructions.rs`.

**Trade-off:** fewer new data structures and lower frontend disruption, but
we still erase effective types and source sequencing too early. Reconstructing
them with a large side table plus a separate AST evaluator would effectively
build candidate A indirectly. Choose B only if an independent low-level IR is
the immediate product and the site intentionally owns another evaluator. Both
candidates ultimately need the same careful arithmetic/refinement tests.

### Migration in reviewable stages

1. **Freeze the contracts, not the implementation.** Write a small semantics
   table for each new operation: inputs, outputs, preconditions, observable
   effects, allowed choices, C revision and implementation policy. Add an
   explicit compatibility note for today's poison and noundef behavior. Keep
   the current textual format/tests working during the experiment. Exercise
   the existing pre-ABI interpreter API first to separate ABI complexity from
   missing C semantics; it is a cheap baseline for the new evaluator.
2. **Build an isolated CModule slice.** Add a module behind a study/test-only
   entry point, using existing sema conversions, bindings, type layouts,
   effects and control facts. Support integer functions, fixed locals, loops,
   calls and origins; refuse everything outside the subset. Add text dump,
   structural verifier and a bounded evaluator. No default pipeline changes.
3. **Add CModule → legacy pre-ABI IR.** Reuse the existing builder, O, A, L
   and N. For this stage LLVM-like contracts remain in the adapter and legacy
   IR. Compare defined executions across direct C evaluation, legacy I and
   emitted native/LLVM output. Add one object feature at a time: arrays/member
   places, explicit lifetimes, aggregate snapshots, then representation copies.
4. **Introduce the scalar contract alongside legacy IR.** Port one family at
   a time: arithmetic/checks, choices/representation masks, pointer operations,
   memory, calls. Adapt O through explicit compatibility shims until its
   poison-specific transformations are replaced. Do not reinterpret old text
   under new semantics without a version/stage marker. Compare before/after
   optimization with the new scalar interpreter and emitted code.
5. **Move ABI and LLVM promises to their owners.** Keep ABI classification
   algorithms but return typed target plans; put `noundef`, LLVM flags,
   inbounds/GEP, data-layout serialization, freeze and intrinsic spellings in
   L. N consumes target plans/scalar ops; direct C I skips ABI. Preserve a
   post-ABI interpreter route for conformance/differential tests.
6. **Broaden only after gates pass.** Add varargs, union/bit-field edge cases,
   setjmp/labels, atomic and platform extensions with explicit capability
   checks. Retire the old path only after the supported corpus and performance
   gates pass. Do not make a partly implemented CModule the only route for
   language features the compiler already supports.

Use distinct Rust stage types (`CModule`, `ScalarModule`, `AbiModule` or checked
wrappers) so consumers cannot accidentally accept an unfinished mixed level.
Each stage has a verifier; semantic preconditions additionally need runtime or
proof evidence, which a structural verifier alone cannot supply. Preserve
OriginId through transformations using a side table of origin sets; optimized
instructions can correspond to several source operations. The site's primary
trace comes from CModule before these transformations.

### Cost, risks and the validating experiment

These are planning ranges for one maintainer familiar with the code, not
measured effort or a delivery promise:

| Scope | Approximate effort | What it would establish |
| --- | --- | --- |
| Contracts and isolated integer/object slice | 1–2 weeks | Feasible sema integration and useful source trace |
| Slice through legacy scalar IR to all currently supported consumers | Another 2–4 weeks | Viable staged architecture without replacing codegen |
| Pointer/subobject, lifetime, representation and aggregate coverage | Another 4–8 weeks, with semantic decisions as the main uncertainty | Whether object semantics pay for their complexity |
| Full scalar/optimizer/ABI boundary migration and extension parity | Several months, likely overlapping the prior work | Actual semantic independence from LLVM |
| Candidate B alone | Several weeks for a scalar spike; longer for complete O/ABI compatibility | Narrower low-level independence; no complete source machine |

Principal risks are two semantic definitions drifting, a new object model
rejecting supported systems idioms, hidden storage/snapshot overhead,
inaccurate source sequencing, changed optimizer freedom, and regressions in
ABI padding/varargs/longjmp cases. Avoid simultaneously changing the graph
structure, optimization framework and memory semantics. Keep a capability
matrix and refuse unsupported execution instead of falling back to a misleading
model. The object evaluator is a semantic tool, not a guarantee of real-world
concurrency, signal or foreign-library behavior.

The first experiment must exercise the reasons for redesign, not just show
that a new enum can run factorial:

| Case family | Evidence required |
| --- | --- |
| Defined integer loops/calls/locals | C evaluator = legacy I = L/N observable output/status, before/after O; logical locals survive in the trace even after scalar promotion |
| Unsigned wrap, signed overflow, invalid shift/division | Valid boundary cases agree; C evaluator names the invalid source operation even when its result is unused. No demand that all backends agree on UB outputs. |
| Short circuit, comma, `i++ + i++`, aliased unsequenced reads/writes, calls in operands | Correct required sequencing and conflict detection; chosen schedule disclosed; call sequencing handled by the selected C revision |
| Fixed block lifetime, declaration re-entry, recursion, VLA exit | Correct instance/generation timing, initialization timing and stale references; no “all locals live for the frame” shortcut |
| Array/subobject one-past and pointer difference | Distinguish valid one-past construction from dereference; different-array subtraction diagnosed at the C operation; no implicit same-allocation substitute |
| Partial bit-field initialization, struct padding, union views, memcpy | Defined neighboring bits/fields preserved; representation copying separate from typed reads; permitted unspecified choices recorded |
| Pointer/integer round trip, bytewise pointer copies, tagged address | Explicit results under the selected provenance/implementation policy; disagreements with legacy I classified rather than hidden |
| Aggregates and varargs over System V/Win64 | Logical calls agree with ABI adapters on meaningful values; independent external ABI oracle for emitted code |

Start with the portable integer sources already embedded in
`src/lowering/tests/programs.rs` and loop/call examples in
`src/lowering/tests/golden.rs`, plus focused object/sequence litmus programs.
Inspect `test-programs/` rather than assuming it is an execution suite: most
files are preprocessing fragments, `func.c` is a token-paste example, and
`donut.c` is an unbounded extended floating/terminal demo. Any later prototype
comparison using this directory must disclose that subset and bound execution;
it cannot honestly count these as several representative integer programs.

Use the prebuilt compiler only as an explicitly identified comparison arm:
`bcc-main` predates the latest merge, so record its `.rev` and do not present
it as a build of `145898d4`. Use pinned Clang 23.1.1 for an independent ABI/code
oracle, not as the authority on unsettled C semantics. Match data model,
language mode, optimizer settings and runtime environment. No equality claim
for arbitrary unspecified values: compare permitted domains or recorded choice
witnesses, and compare observable behavior of defined executions.

Suggested acceptance gates: zero defined-program mismatches on the supported
subset; every supported semantic litmus either yields the specified event/result
or an explicitly classified policy outcome; text round-trip and structural
verification; bounded memory/steps; the legacy path still supports its prior
features. Measure lowering time, arena bytes and source/low-level instruction
counts separately from backend time. A provisional budget of at most 15%
extra frontend-plus-lowering time and 25% extra retained IR bytes on the slice
would force useful design discipline, but the owner should set final budgets
after measurements. Investigate/stop the expansion if the object layer adds
large compile overhead without reducing duplicated I/L/N semantic machinery.

Long builds must use the shared cargo wrapper, run detached with logs, and
follow a saved/committed checkpoint first. Run narrow checks on each slice,
then the repository's canonical checks before changing the default pipeline.
Nothing in this study authorizes weakening a failing baseline or silently
skipping an unsupported construct.

### Relationship to the abstract-machine website

The sibling's provisional `docs/research/abstract-machine-website.md` on
`codex/abstract-machine-site` was read without modification. It proposes source,
object/lifetime and sequencing views, deterministic reversible stepping and
an eventual object-aware IR. That moving document is separate from this
study's pinned source baseline.

Candidate A gives both projects the same executable C operations, declaration
and activation identities, type/layout tables, lifetime transitions, ChoiceIds
and structured failure reasons. It avoids making the site infer C behavior
from optimizer temporaries or reconstruct a struct out of ABI registers. A
small versioned event sink can carry:

```text
SourceEnter(origin), ObjectBegin(instance, type, duration), Read/Write(place),
Choice(id, reason, witness), SequenceBoundary(evaluation),
Call/Return(activation), ObjectEnd(instance), Undefined(rule, related_origins)
```

Keep snapshot/undo/UI serialization outside compiler instructions; the engine
emits deltas and the site can replay from checkpoints. IDs must be stable within
a compiled session, scoped by activation/generation where appropriate, and
independent of optimizer value numbers. Origins should connect to retained
macro expansion/source provenance, not only a physical line number. Inspection
of compilation phases and optimized/native IR remains a separate useful view,
with mappings and gaps disclosed.

The costs to the site are a larger semantic core, explicit unsupported/policy
states, and an eventual migration from its bounded AST/sema evaluator to CModule.
Do not block its prototype on completing the compiler redesign: agree first on
object/event vocabulary and semantic litmus cases, then replace its execution
adapter when available. Do not put browser concerns into CModule, or force it
through mandatory LLVM/native FFI merely to obtain a trace; a later portable
evaluator package should sit above those dependencies. Candidate B makes the
site's IR view cleaner but still requires its own C evaluator, increasing the
risk of two competing C models.

### Decisions for the owner

1. Is source-semantic execution a shared compiler product, or should the site
   intentionally own a separate bounded model? This determines A versus B.
2. Which indeterminate-value and integer-pointer policies should bcc promise
   for C99/C11 and systems extensions? Preserve today's behavior as named
   compatibility while choosing, rather than treating Clang output as the spec.
3. What compilation-time/memory budget and supported subset gate the first
   experiment? The proposed slice deliberately leaves existing backends and
   their default routing intact.
4. Is the first milestone a useful C trace plus legacy lowering, or complete
   elimination of shared poison? They are different deliverables; the former
   is a smaller reversible test of whether the architecture is worthwhile.

## 4. Prototype and validation status

The first isolated slice is in `src/c_ir_study.rs` and `src/c_ir_study/`, behind
`cfg(test)`. It snapshots existing typed syntax/sema facts into logical integer
objects, explicit lexical lifetimes, a statement CFG and an expression graph
retaining sequence points. A bounded evaluator runs after the frontend arena
has been dropped. It has no dependency on today's scalar IR, optimizer, ABI or
LLVM evaluator. The default build and pipeline do not use it.

The slice includes integer functions, automatic locals, conversions, loops,
break/continue, direct calls and recursion. Execution reports source operations
that overflow or violate simple unsequenced-access rules. Indeterminate reads
have a separate policy-dependent outcome. Conflicts spanning nested sequencing,
pointers, aggregates, globals, extensions and other unsupported constructs are
refused. This is not the complete object/provenance or sequencing model proposed
above, and does not replace the current scalar contract.

Three new portable fixtures live in `test-programs/ir-design-study/`. Tests also
select seven existing lowering programs and compare results against today's
interpreter before/after ABI lowering and with/without optimization, using the
Linux and Windows target data models. Separate litmus tests check UB, object
events, explicit capability refusals and execution limits. The new IR has a
deterministic text dump and reference verifier; a text parser and the adapter
back to legacy IR remain future work.

Implementation checkpoint: standard paragraphs and formatting checked; shared
wrapper compilation and differential validation are pending. The original
study's source/citation/link checks remain design evidence, not performance
measurements.
