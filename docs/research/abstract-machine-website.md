# A C abstract-machine website for bcc

Study baseline: `145898d4`, 2026-10-21. This branch contains a local-only
prototype. No hosting, server execution, or changes to the native compiler's
default semantics are intended.

## Working conclusion

Build the product around a C evaluator fed by bcc's syntax tree and retained
semantic facts, eventually executing an object-aware C IR if the parallel IR
study adopts one. Do not describe the existing LLVM-shaped IR interpreter as
the C abstract machine. Preserve it as a separate, valuable IR execution view.

The overnight prototype explores the C evaluator route with a deliberately
bounded language subset and an independent parser. It is real source-driven
Rust execution, but does not yet reuse bcc's full parser or semantic analysis.
The implementation inventory and verification results are recorded below.

## Product vision

The primary workspace joins source, the active source position, the call
stack, and C objects. Every object has an activation-stable identity, declared
type, storage duration, lifetime state, and value or indeterminate state.
Pointers connect to objects and subobjects, rather than displaying host
addresses. A sequencing panel explains pending side effects before the next
sequence point. An undefined operation stops execution with an explanation,
the relevant objects and source operations, and a verified C99 citation.

Step forward, step back, and run to cursor share a deterministic trace.
Stepping should distinguish evaluating an expression from committing its side
effects. A single concrete run must disclose choices where C permits several
evaluation orders, and cannot prove a program free of undefined behaviour.

## Which execution layer?

### Option A: instrument the existing IR interpreter

The interpreter already has the right *control shape*: one instruction per
iteration, explicit activation frames, typed runtime values, byte-addressed
objects with initialization bits, generation-checked pointers, bounded
execution, and captured host output. Start in
`src/backend/interpreter.rs`, then `machine.rs`, `memory.rs`, `value.rs`, and
`trap.rs` beneath it. A resumable `Machine::step` plus read-only inspection
would be a useful addition independent of this site. There is no need to
invent a second low-level interpreter.

However, `src/ir.rs` explicitly gives this IR LLVM poison semantics without
`undef`. `RuntimeValue::Poison` covers both unwritten loads and violated IR
flags. `freeze` chooses a deterministic zero. The interpreter's own memory
documentation calls integer-to-pointer recovery a stand-in for a full
provenance model. These are useful IR policies, not an exact statement of C.

Lowering in `src/lowering.rs` also puts non-address-taken scalar locals in
SSA. A visible C object may therefore have no backing memory object, its
value may move across block parameters, and its source declaration cannot
be reconstructed reliably from a stack slot. Optimizer passes can remove or
combine source actions. `src/pipeline/middle_end.rs` applies optimization
before ABI lowering; ABI lowering is another boundary between C calls and
the interpreter's ordinary post-ABI input. The interpreter can also accept
pre-ABI modules, which is preferable for a compiler-inspection view.

Making A faithful requires metadata and executable checks, not just source
line labels:

1. Retain complete source-vector origins per instruction and event, including
   macro spelling versus expansion sites and many-to-many transformations.
2. Retain declaration/binding IDs, C types, automatic/static/allocated
   duration, block and VLA lifetime transitions, subobject layout, and a
   mapping from SSA values to visible source objects.
3. Preserve full-expression and internal sequence points, reads/writes and
   their conflicts. An instruction order alone records a chosen order but
   cannot recover whether the corresponding C operations were unsequenced.
4. Diagnose the undefined C operation when evaluated, rather than allowing
   its representation to become poison and trapping only at a later use.
5. Disable optimization for the source machine or define and preserve all
   this metadata through every transformation. Do not promise source stepping
   across optimized code in the first version.

That is effectively a C execution layer embedded in lowering. A is the
fastest route to **an IR debugger**, but not clearly the cheapest route to a
trustworthy **C abstract-machine debugger**.

### Option B: evaluate analyzed C before semantic erasure

Use `pipeline::parse_translation_unit` and
`pipeline::analyze_translation_unit` as the front door. The parser produces
immutable syntax, not a separate fully typed AST. The evaluator should use
that syntax together with the identity-indexed `SemanticTranslationUnit`
records in `src/translation_phases/semantic_analysis/results.rs`.

There is much more reusable material here than names and types:

- `expression_info` and conversion runs retain promotions, lvalue conversion,
  decay, assignment conversion, and target-specific scalar interpretation.
- `facts/correspondence.rs` and `facts/resolved.rs` retain declaration/entity
  relationships, designations, selected members, parameters and call facts.
- `facts/control.rs` retains lexical/VM regions, control points, jump targets
  and compact entered/left region runs; these can drive object lifetimes.
- `facts/initializer_plans.rs` retains accepted subobject placement, avoiding
  another brace-elision/designator implementation in the runtime.
- `facts/effects.rs` retains effect classes and effects-only recipes. These
  tell a consumer what must execute; they are not already a dynamic
  read/write conflict detector and do not establish all sequencing rules.

Implement execution with an explicit control/continuation stack, not native
recursion across C calls. A value stack holds intermediates. Object identity
is `(declaration/entity, activation, allocation generation)`; C objects are
separate from physical wasm addresses. Keep byte initialization and pointer
metadata alongside typed views. A pointer names its allocation, permitted
subobject/array domain and offset; one-past is representable but not
dereferenceable. A write through an alias updates the same object displayed
through every source name.

Keep an access/effect log per sequencing interval. Detect conflicting
reads/writes using object/subobject identity, and retain the two source
operations for the explanation. Commit outstanding effects at the
appropriate sequence points. A deterministic pedagogical order is acceptable
only when labeled as one permitted choice; later add alternative-order
exploration with a bounded search. Do not confuse a sequence point with an
arbitrary statement boundary or draw every write as delayed by C.

Indeterminate is a first-class state, separate from zero, a trap
representation, unspecified concrete values, unavailable inspection, and IR
poison. Inspection of an indeterminate object must not itself read that
object in the simulated C program. Indeterminate-value rules have subtleties
across type, representation and context; a prototype may reject their use
conservatively but must identify that policy.

Stop categories should distinguish a constraint/translation error, a detected
undefined operation, an unsupported language/library operation, a resource
limit, normal return, and an implementation choice. In particular, an
unsupported operation must never be reported as a C rule violation.

### Recommendation and the sibling IR study

Choose B for the product. Build a C evaluator over the existing syntax plus
semantic facts first, with a small, documented executable subset. Keep
evaluation, object memory and trace events behind a stable interface so the
syntax-to-control adapter can later emit an object-aware C IR. Do not commit
to maintaining two independently implemented sets of C conversion, binding,
initialization and control semantics.

The sibling study at
`../ir-design-study/docs/research/ir-redesign.md` is read-only input. Its
recommendation is an executable, object-aware C IR above the current
scalar CFG, preserving C undefined operations, indeterminate representations,
sequencing and lifetimes before erasure. That aligns with this boundary:
the site would execute the C layer in diagnostic mode; native and LLVM
pipelines would lower it to the current scalar/ABI layers. Merely changing
opcode names or removing poison from the low IR would not restore lost C
information. The proposed C layer is not a dependency of the overnight demo.

The latest sibling study makes this shared event vocabulary concrete:
`SourceEnter`, `ObjectBegin`, `Read/Write`, `Choice`, `SequenceBoundary`,
`Call/Return`, `ObjectEnd`, and `Undefined`, with declaration/activation/generation
identities and retained origins. Agree on those events and semantic litmus
cases first. Keep browser snapshots and undo outside CModule instructions;
replace this prototype's syntax adapter when the shared C layer is ready.
The sibling study includes no runtime prototype or performance validation yet.

The native default pipeline should remain unchanged until that research has
its own reviewed migration. Add trace sinks as optional observers, rather
than putting presentation data into default optimization semantics.

## What the main view should teach

Source ranges are half-open byte ranges in a named source file, mapped to
line/column for presentation. Preserve both the operation being evaluated
and the next operation; label which one the highlight shows. Macro-created
operations have an expandable origin chain. Clicking a frame shows its
suspended call site and objects without changing execution.

Group objects by storage duration and activation, with a separate lifetime
badge. A lexical name going out of scope differs from storage ending. Keep
dead-object tombstones in the trace so a pointer into expired storage can be
explained. Arrays expand into elements; structures into fields; pointer
arrows target an object or subobject, with one-past and expired targets styled
distinctly. Numeric synthetic addresses are optional advanced information.

The sequencing panel should say which effects have occurred, which are
pending under the selected evaluation schedule, and the next sequence point.
A UB stop keeps the last valid state and highlights the faulting operation,
its conflicting accesses, and the cited clause. Back leaves the failure and
re-enters the previous state; it never re-executes external effects.

Use three stepping granularities eventually: source/full expression,
evaluation event, and implementation instruction. The MVP only needs a
clearly labeled granularity. Run-to-cursor stops before the next dynamic
evaluation intersecting the selected range in the selected activation; if it
cannot be reached before exit, UB, or a limit, explain that outcome.

## Other compiler views

These are additional views of the same compilation, not separate toy parsers.
Every view links back through stable source and entity identities. They are
product proposals; the overnight page implements execution only.

| View | Interaction and existing seam | Additional work |
| --- | --- | --- |
| Macro expansion | Expand one invocation; show argument collection, substitution, stringification, token pasting and rescanning. Start in `preprocessing/macro_expansion.rs` and `preprocessing/runtime/reader.rs`. | Optional event sink before ephemeral provenance is compacted; stable copied origins and expansion IDs. Retained token output alone cannot replay all intermediate expansion steps. |
| Tokens | Switch preprocessing-token and parser-token lanes, inspect spelling/kind/literal and origins. Reuse tokenizer and preprocessing output used by `--tokens`. | Structured paginated output; virtualized lists; show phase boundaries explicitly. |
| Syntax and parser | Click source to reveal a node; animate push/consume/reduce/reprocess/recover on the explicit `Parser::drive` stack in `parsing.rs` and `parsing/machine.rs`. | Feature-gated frame/action snapshots. `parsing/inspection.rs` already walks the finished tree; `syntax_log.rs` is a test-only allocation log, not a production parser trace. |
| Semantic facts | Select an expression to see binding, type, implicit conversions, initializer placement, effects and control region. | Structured identity-based export of `SemanticTranslationUnit`, following inspection APIs; do not serialize Rust pointers. |
| bcc IR and optimization | Source-linked CFG; before/after pass slider; inspect replaced values, eliminated stores and verified invariants. | Observer around the pass list in `optimizer.rs`; serialize snapshots before dense value renumbering, plus explicit replacement maps. Existing reports count transformations; they are not complete diffs. |
| Native assembly and allocation | Synchronize C, scalar IR, machine IR and x86-64; select a live interval to see register pieces, holes, spills, reloads and edge moves. | Export `native/mir.rs` and allocation stages in `native/regalloc.rs`; keep register-allocation animation distinct from executing machine instructions. The symbolic checker can explain allocation invariants. |
| LLVM IR | Side-by-side bcc and emitted LLVM IR, with target and ABI options visible; explain poison and attributes at this boundary. | Current `backend/llvm.rs::emit_module` constructs and prints a module through LLVM-C; it is not a standalone Rust text printer. For client-only output, implement a dependency-free emitter using those mappings, or ship a separately ported LLVM wasm artifact. Running LLVM optimization in-browser is a separate, larger artifact decision. |

## Browser architecture

### Client execution and hosting

All user code compiles and executes in wasm **inside the browser**. The host
serves immutable HTML/CSS/JS/wasm and bundled headers only. There is no API
that executes submitted C on a server. Browser wasm executes the bcc engine;
it does not turn the user's C into native browser instructions for this
prototype. Interpreter library calls use a small simulated host, with
captured output and deterministic input. No arbitrary host imports, network,
processes or file access belong in that host.

Use a dedicated module Web Worker to own the wasm instance and compilation
session. The UI communicates via versioned messages with request IDs; discard
responses from an old source revision. Step/run use a per-request fuel budget
and yield between chunks. A hard stop terminates and recreates the Worker if
compilation or an engine bug does not yield. Bound source bytes, token/AST
nodes, evaluation depth, objects, memory, trace size, output and event count.
Browser sandboxing alone does not keep a tab responsive or bound resource
use. No SharedArrayBuffer or cross-origin isolation is needed initially.

Static deployment can use GitHub Pages, Cloudflare Pages, Netlify, or any
ordinary static host. Serve `.wasm` as `application/wasm`; offer a
`fetch(...).arrayBuffer()` fallback if streaming instantiation fails. Use
relative asset URLs so a repository subpath works. Pin release assets by
content hash and invalidate worker/wasm pairs together. A strict CSP can keep
all assets local with the required wasm compilation directive. Deploying or
publishing is outside this branch's scope.

### Native dependencies and wasm gates

The repository is not currently a portable browser frontend with a switch
that merely disables code generation. At baseline it has these barriers:

| Surface | Required split for a full frontend port |
| --- | --- |
| `Cargo.toml` | LLVM, CLI/time and platform dependencies must not be selected by the browser feature set. Optional dependencies plus a default native feature preserve ordinary invocations; browser builds explicitly use `--no-default-features`. Keep benchmarking/coz native-only. |
| `build.rs` and build dependencies | Skip *compilation* of `build_support/native.rs`/`llvm.rs`, not just their invocation. Gate bindgen/cc/cmake dependencies too. `native::compile` asserts `TARGET == HOST` and configures Rust/C fat LTO, so an early unconditional wasm call cannot work. |
| `vendor/` and LLVM | The build script normally builds pinned LLVM from `vendor/rust/src/llvm-project`. Wasm must neither bootstrap nor link it. Vendored skills need no browser packaging. A wasm engine must not depend on checkout-relative LLVM tooling. |
| FFI and floating values | `float_parsing/ffi.rs` includes generated bindings; conversions use native C routines and the target-options bridge uses LLVM/C++. Gate these modules or implement portable, target-correct conversion. `binary128/arithmetic.rs` is Rust arena arithmetic but still imports native-associated float carrier/error types; do not assume all floating support is isolated by disabling LLVM. |
| Arenas and pointer layout | `util/memory/vm.rs` reserves 100 GiB per region on a 64-bit OS and uses platform VM APIs. That constant and reserve/commit strategy cannot be carried into wasm32. Add a wasm arena with stable chunk allocations or a bounded pre-reserved linear-memory layout, then audit 64-bit layout assertions, packed pointers, `usize` assumptions and serialized offsets. Changing only `REGION_BYTES` is insufficient. |
| File system and source loading | Replace `Path`-driven loading and native include search with a virtual source map. Uploads are explicit source entries. Compile-time `include_str!` resource headers in `headers.rs` can be reused; host include directories, environment and filesystem probes cannot. Normalize paths and constrain includes to the virtual map. |
| CLI and external tools | Gate `main.rs`, `cli`, `std::process` drivers, linker/assembler invocation, native host I/O and OS resource reporting. Pure bcc IR and native assembly emission can later be separate browser features. LLVM emission currently needs LLVM-C even for text; a pure printer would be new work. |
| Time and entropy | Pass deterministic clock/input choices from the simulated host; compiler trace output must not depend on wall-clock timing. Audit chrono's clock features and allocator/hash initialization. |

The prototype may isolate a smaller new engine instead of completing this
port. That is a useful delivery boundary, but does **not** demonstrate that
bcc's existing C frontend, LLVM backend, or IR interpreter runs on wasm.

### Engine protocol

The production interface should be independent of binding technology:

```text
compile({sourceFiles, entryFile, language, targetModel, limits})
  -> {compilationId, diagnostics, capabilities, sourceMap}
createSession({compilationId, entry: "main", inputs, evaluationPolicy})
  -> {sessionId, state}
step({sessionId, count: 1, granularity}) -> {events, state}
back({sessionId, count: 1}) -> {state}
runTo({sessionId, fileId, range, activationId, fuel}) -> {events, state}
state({sessionId}) -> MachineState
trace({sessionId, start, count}) -> {events, next}
dispose({sessionId | compilationId})
```

Each response carries `protocolVersion`, compilation revision, target data
model, evaluation policy and capability limits. `MachineState` includes a
trace cursor and status, current/next source ranges, frames with activation
IDs and suspended sites, objects with duration/lifetime/type/value and
subobjects, pointer relationships, pending effects, output, and an optional
diagnostic. Diagnostic fields include category, standard revision, clause,
paragraph, printed/PDF pages, primary range and related ranges. Export IDs,
not borrowed arena references. Use decimal strings for integers wider than
JavaScript's exact range and tagged pointer/value variants.

A raw wasm ABI can exchange UTF-8 requests/responses through explicit
allocation and length exports; alternatively use wasm-bindgen once tool
availability and generated glue are part of reproducible setup. Refresh JS
memory views after allocations because `memory.grow` detaches old views.
Keep wasm ABI handles private to the Worker and validate input sizes before
copying. The overnight ABI can be narrower; its actual exports must be
documented rather than presented as the complete protocol above.

### Time travel

Use recorded immutable snapshots for the small demo. Back simply moves a
cursor and replay reuses the recorded result. A snapshot contains state,
control/operand stacks and host state if future execution can fork from it;
a JSON presentation snapshot alone is insufficient to resume arbitrary
execution. Never snapshot just the visible variables and rerun effects.

For a larger engine, combine periodic internal checkpoints with an event
log of object allocation/lifetime, reads, writes, calls/returns, sequencing,
input/output and nondeterministic choices. Record old and new values so
reverse stepping is local, or restore a checkpoint and deterministically
replay choices. Copy-on-write object pages reduce repeated array storage.
Cap retained history and show the oldest retained step. External inputs and
outputs are simulated and recorded; reversing never writes an output twice.

### Frontend stack

The MVP uses plain HTML, CSS and ES modules, with no CDN or package manager
requirement. A source pane, frame list and object inspector are enough to
validate the execution protocol before adopting a large editor framework.
For the product, use TypeScript and a component framework the maintainers
prefer (Svelte or React are reasonable), CodeMirror 6 for ranges and source
navigation, and SVG for pointer connections and small CFGs. Virtualize large
token/object/trace lists. Keep an accessible textual pointer target alongside
every arrow, keyboard equivalents for controls, and a reduced-motion mode.
Source text and diagnostics must be rendered as text, never unsanitized HTML.

## Milestones and acceptance criteria

1. **Protocol and vertical slice (this branch).** A bounded executable subset
   parses changed input, runs locals/loop/call/pointer/array in Rust wasm,
   exports state, and supports reverse replay. The page explains limitations.
   Native default behavior and build configuration remain covered by checks.
2. **Portable real frontend.** Split native dependencies, introduce wasm-safe
   arenas and a virtual source map, retain actual bcc parser/sema identities.
   Accept the same positive/negative C99 fixtures as native frontend paths;
   headers and diagnostics run offline in a Worker. Remove the subset parser
   instead of expanding it into a second C compiler.
3. **C memory and execution rules.** Add automatic/static/allocated duration,
   block/VLA lifetimes, subobjects, typed conversions and initialization,
   pointer domains and one-past rules, sequence points and access-conflict
   detection. Each rule has standard-backed tests; distinguish unsupported
   behavior and implementation policy from UB. Validate well-defined
   programs against existing backends and purpose-built semantic oracles.
4. **Reliable time travel and limits.** Checkpoint/event replay, recorded
   choices and host I/O, cancellation, configurable budgets and trace export.
   Replaying to an index must reproduce object IDs, values, diagnostics and
   output exactly. Back/forward properties cover loops, recursion and failure.
5. **Compiler exploration.** Add tokens and semantic facts first, then
   parser/macro event sinks, optimizer diffs, allocation and backend views.
   Keep inspection features opt-in and measure overhead in ordinary builds.

The largest risk is semantic credibility: a pleasing memory animation can
teach incorrect C if sequencing, lifetime, indeterminate values and pointer
identity are simplified without labels. Other risks are maintaining a
duplicate parser/evaluator, unsafe allocator portability, trace memory growth,
source mapping after macro expansion and optimization, library coverage,
and coordinating the new C layer with ongoing IR design. Resource limits
and explicit refusal are features of an honest early release.

Decisions for the owner: endorse the C evaluator/C IR boundary; choose the
first supported target data model; decide which unspecified evaluation
choices to display initially; select the supported library and header set;
and decide whether the first public release prioritizes execution fidelity
or the compiler inspection views. The static hosting provider and eventual
UI framework can wait until the protocol and semantics are stable.

## Standard anchors checked for this study

These citations were verified against the searchable N1256 edition through
`.agents/skills/search-standard/standard.sh`; the PDF remains authoritative.

- C99 §5.1.2.3p2, printed p. 13; PDF p. 25: side effects and completion at
  sequence points.
- C99 §6.2.4p2, printed p. 32; PDF p. 44: object lifetime, access outside
  lifetime, and indeterminacy of a pointer when the pointed-to lifetime ends.
- C99 §6.5p2, printed p. 67; PDF p. 79: restrictions on reads and multiple
  modifications between sequence points. Use C99's own wording when teaching
  this rule rather than silently substituting a later revision's terminology.
- C99 §6.7.8p10, printed p. 126; PDF p. 138: automatic indeterminate values
  and static default initialization.
- C99 §6.2.4p5, printed p. 32; PDF p. 44: fixed automatic object lifetime
  starts at block entry; initialization occurs on reaching the declaration.
- C99 §6.5.3.2p3, printed pp. 78-79; PDF pp. 90-91: address-of cancels
  indirection in `&*E`, and `&E1[E2]` forms a pointer without the implied
  indirection. This matters when displaying one-past and null pointers.

## Prototype and verification record

### Implementation boundaries

- `src/abstract_machine.rs` and its child modules own the independent subset
  parser, source-located operations, C object runtime, snapshots and JSON.
- `src/abstract_machine_wasm.rs` exposes the versioned single-session raw
  wasm ABI. It passes through engine JSON and retains the compiled program
  for reset. No binding generator or host imports are required.
- Cargo's default `native` feature preserves the original compiler modules,
  dependency graph and build-script behavior. `abstract-machine` can also be
  enabled alongside native; browser builds disable default features.
- Native dependencies, build dependencies and build-support module loading
  are gated. The wasm build does not compile the native frontend, VM arenas,
  LLVM, FFI, CLI, filesystem or embedded headers. None is needed by this
  headerless demo. These are isolated rather than secretly ported.
- `web/` contains the static page, renderer, controls and wasm adapter. Every
  value is supplied by Rust. Missing wasm disables execution and displays
  instructions; there is no scripted execution or JavaScript C evaluator.

The implemented subset uses signed 32-bit int, int pointers with
element-based offsets, fixed-size int arrays, automatic locals, named
int-returning functions and int/int-pointer parameters, lexical blocks,
conditionals, loops, return, assignment, arithmetic and comparison. Source
ranges refer to the next operation; the event text describes the previous
completed operation. Objects are revealed indeterminate before initialization
stores. Calls use explicit activation frames; ended objects remain observable
as tombstones. Back/forward replays complete recorded engine states.

### Simplifications to keep visible

This is not the full B implementation proposed above. The separate subset
parser is disposable prototype code: do not keep extending it instead of
porting the real frontend. There is no preprocessing, globals/static/allocated
storage, full C declarator or conversion system, floating point, structure or
union representation, effective-type model, function pointers or C library.
No native/LLVM output or other compiler inspection views are implemented.

Automatic objects are allocated when their declarations execute. For fixed
automatic objects C99 §6.2.4p5 instead starts lifetime on block entry and
separately performs initialization when a declaration is reached (printed
p. 32; PDF p. 44). The demo gets allocation identity, block exits and call
activations, but does not yet reproduce that earlier lifetime boundary.
Pointer tombstones retain historical target information for inspection;
using a pointer after its target dies stops under the indeterminate policy.

The evaluator uses a documented deterministic operand order and rejects
nested side effects and calls in compound expressions rather than claiming
complete unsequenced-access detection. The pending-effects panel previews
upcoming stores, not a full implementation of deferred effects or all C99
sequence points. Runtime checks cover a bounded subset; successful execution
does not establish C conformance or absence of UB. Pointer/lifetime and
indeterminate-value policies must be read with the engine limitations, not
inferred from the appearance of the diagram. In particular C99 does not
uniquely specify a complete modern pointer-provenance model.

### Environment and browser evidence

Rust 1.99.0 was already installed. Initially only the native target was
installed, and no wasm-bindgen or wasm-pack executable was present.
`rustup target add wasm32-unknown-unknown` succeeded; the raw ABI avoids
needing either binding tool. Exact build/serve commands are in `web/README.md`.

T3 `preview_status` reported no attached browser. Both `preview_open` and
`html_preview` then failed with the host prerequisite error:

```text
This host is missing libraries T3's browser needs
(libglib-2.0.so.0, libgobject-2.0.so.0, libgio-2.0.so.0,
libatk-1.0.so.0, libatk-bridge-2.0.so.0, libdbus-1.so.3,
libX11.so.6, libXcomposite.so.1, libXdamage.so.1, libXext.so.6,
libXfixes.so.3, libXrandr.so.2, libgbm.so.1, libxcb.so.1,
libxkbcommon.so.0, libasound.so.2, libatspi.so.0).
Run `sudo env "PATH=$PATH" t3 browser setup` on the host to install them,
then try again.
```

No alternative browser executable is installed in the execution environment.
This blocks actual browser rendering and screenshots, not wasm build or
Node-based WebAssembly execution checks. The documented local serve/open
commands provide a reproducible manual check. No screenshot is claimed.

### Verification results

Commands below run from this worktree root. Every Cargo invocation used
`python3 /home/agent/workspace/bcc-rust/triage-2026-10-16/swarm/run.py`.

- `cargo test --lib pipeline::tests`: **166 passed**, no failures. These
  exercise the original default pipeline, including interpreted C and native
  and LLVM comparison arms. Log: `target/abstract-machine/native-pipeline-tests.log`.
- Native `cargo check --all-targets` and
  `cargo clippy --all-targets -- -D warnings`: passed during build isolation.
  The final feature-enabled lint pass is queued separately.
- Static page checks: HTML IDs are unique, all literal renderer/control
  element references exist, and local assets/module imports resolve.
- `node --experimental-default-type=module web/worker-check.mjs`: passed
  against real wasm. The actual Worker/proxy modules compile and execute the
  tour to `15`, report rejected source, reload a program returning `42`, and
  reject pending/future requests after cancellation. `--missing` also passes
  the explicit missing-artifact setup-error path. This uses a Node
  messaging/fetch shim, not a browser rendering test.
- Local HTTP checks: page, JS modules, CSS and wasm returned 200; the wasm
  content type was `application/wasm` and its binary header was valid.
- `cargo +nightly fmt --check`: passed.

- `cargo test --lib --no-default-features --features abstract-machine`:
  **12 passed**, no failures, including complete reverse replay, calls/recursion,
  pointer cancellation/one-past rules, postfix precedence, declaration visibility,
  conservative indeterminate stops, UB citations and resource limits.
- `cargo clippy --lib --no-default-features --features abstract-machine
  --target wasm32-unknown-unknown --target-dir target/abstract-machine/build
  -- -D warnings`: passed after correcting the bridge's initial style lints.

The final combined lint, wasm rebuild and integrated web execution checks are
still in progress.
