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
bounded language subset. The final implementation inventory and verification
results will be recorded below as work completes.

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

## Investigation in progress

- Compare instrumenting lowering plus the IR interpreter against a C evaluator.
- Inventory wasm barriers: mandatory LLVM linkage and build script, native
  float parsing and binary128 FFI, OS virtual-memory arenas, CLI, file system,
  and header search.
- Define a versioned compile/session/state/trace interface with execution and
  memory limits, and reversible observation.
- Read the sibling `ir-design-study/docs/research/ir-redesign.md` without
  editing it. Its initial hypothesis is an executable object-aware C IR above
  the current scalar CFG, with C semantics preserved before erasure.
- Explore macro expansion, tokens, parser frames, semantic facts, optimizer
  diffs, native register allocation and LLVM IR as views of one compilation.

## Prototype and verification record

Implementation and browser validation are in progress. Use `web/README.md`
for the eventual exact local build and serve commands. Generated wasm should
be reproducible from source; a missing engine must never trigger an unlabeled
JavaScript simulation.
