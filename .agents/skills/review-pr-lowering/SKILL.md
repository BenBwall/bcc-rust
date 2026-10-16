---
name: review-pr-lowering
description: Review bcc-rust lowering and IR changes for faithful translation of analyzed C into bcc IR, consumption of retained semantic facts, SSA construction, places and memory facts, undefined-behavior flags, static initializers, symbol reachability, and IR verifier, parser and printer contracts. Use for changes to src/lowering, src/ir, pipeline::lower_translation_unit, or the semantic interfaces lowering reads.
---

# Review PR Lowering

Read [the shared review contract](../review-pr/references/review-contract.md) completely. Read `.agents/AGENTS.md`, `middle-end.md` ("Architecture", "The bcc IR" including "Undefined behaviour" and "Memory facts", "Verifier", "From the front end to the IR", and any dated section the change amends), `docs/design/semantic-facts.md` when the change touches a retained fact, and `src/lowering.rs` or `src/ir.rs` with the submodules the change touches. Verify C evaluation rules against WG14 N1256 §5.1.2.3, §6.5 and §6.8 with the `search-standard` skill; compare questionable IR with what the pinned Clang emits for the same source.

Review lowering as a translation that must preserve C semantics exactly, no more defined and no less:

- consumption of retained facts: conversions are applied in their recorded order, control, lifetime, effects and call resolution come from semantic analysis rather than a parallel walk, and an absent fact refuses with `MissingFact` instead of recomputing or defaulting;
- evaluation order and sequencing, including compound assignment, increment, conditional and short-circuit operators, statement expressions, comma, and side effects inside `sizeof` of variably modified types;
- places versus values: bit-field read-modify-write, volatile accesses, unions, aggregate copies and snapshots, array decay, and alignment guarantees from `lowering/alignment.rs`;
- SSA construction: address-taken analysis, `def_var`/`use_var`/`seal_block` order, loop headers and late-sealed label blocks, and `setjmp`, computed `goto` or cleanup regions that force memory;
- undefined-behavior flags: `nsw` only where C makes overflow undefined, `inbounds` only on pointer arithmetic that C bounds, `exact` only where proven, and `freeze` wherever poison could otherwise reach a branch, address or observable store that C defines;
- VLA allocation and release across jumps, cleanup scopes, switch, `goto`, and statement-expression exits;
- static initializers as bytes plus relocations, flexible-array storage, thread-local and weak or aliased symbols, and reachability planning in `module_items/reachability.rs`; and
- IR invariants: verifier coverage for any new instruction, flag, type or profile rule, printer and parser round-tripping, and a refusal (never a panic or silent miscompile) for any construct not yet lowered.

For each changed construct, trace one concrete C program through its semantic records into the emitted IR and state the IR you expect, then confirm the interpreter would produce the C-defined result before and after optimization. Probe both targets when layout, `long`, `long double`, `va_list` or calling convention differs, and probe the boundary values that change flags or poison.

Leave semantic rule correctness to `review-pr-semantic-analysis`, pass soundness to `review-pr-optimizer`, and ABI and code generation to `review-pr-backend`, while checking each handoff this change alters.

Complete only after every changed lowering path, consumed fact, flag decision, memory or alignment fact, symbol or initializer rule, refusal, and IR invariant is accounted for.
