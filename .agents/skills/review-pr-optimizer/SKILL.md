---
name: review-pr-optimizer
description: Review bcc-rust optimizer changes for soundness under the IR's poison model, termination, analysis invalidation, verifier and interpreter agreement, and the validity of performance claims. Use for changes to src/optimizer, the default pass list, optimizer options, or IR analyses (CFG, dominators, known bits) that passes rely on.
---

# Review PR Optimizer

Read [the shared review contract](../review-pr/references/review-contract.md) completely. Read `.agents/AGENTS.md`, `middle-end.md` ("Undefined behaviour", "Memory facts", "The optimizer", "Comparison arms", "Testing"), `docs/research/optimizer-comparison-methodology.md` when the PR claims a speedup, and `src/optimizer.rs` with the passes the change touches. Treat the interpreter's semantics of the pre-ABI IR as the specification; LLVM's documented LangRef semantics for the matching construct settle disputes about poison and flags.

Review every rewrite as a refinement proof: the transformed function may only remove behaviors, never add them.

- poison and flags: a rewrite that moves, hoists, speculates, merges or reassociates an instruction must drop or justify `nsw`/`nuw`/`exact`/`inbounds`, insert `freeze` where a branch or address could newly observe poison, and never turn a poison-only use into undefined behavior;
- memory: aliasing assumptions, volatile, atomic or unknown calls, address escape, partial and overlapping stores, union and record-copy splitting, alignment facts, and stack-slot lifetimes across inlining and SROA;
- control flow: dominance after block merging or threading, block parameter and edge-argument consistency, duplicate edges, labeled blocks and `indirect_br` targets that must survive, loops whose headers change, and unreachable-code removal that drops side effects;
- value numbering and folding: equality only between values that are equal on every path, commutation and canonicalization that preserve types and flags, constant folding that matches the interpreter's arithmetic bit for bit including division traps, shifts and float rounding;
- inlining and interprocedural facts: weak, aliased, interposable or `noinline`-style symbols, recursion, varargs and dynamic allocation exclusions, and growth budgets;
- termination and cost: fixpoint rounds stay bounded, worklists cannot cycle, explicit stacks replace recursion, analyses are not rebuilt per instruction, and allocation stays in arenas; and
- observability: `--passes`, `--opt-bisect-limit`, `--time-passes` and `--print-after-all` still count and attribute each rewrite.

For each changed pass, construct the smallest IR or C input where the rewrite fires, a near miss where it must not, and an input where poison, a trap or a memory effect would be exposed if the precondition were wrong; state what the interpreter prints before and after. Treat performance ratios as claims needing the methodology's arms, workloads and repetitions, and report a claim that the evidence does not support.

Leave lowering choices to `review-pr-lowering` and post-ABI code generation to `review-pr-backend`, while checking any IR form this change newly produces for back ends.

Complete only after every changed rewrite, precondition, analysis, invalidation point, pass-order or option change, termination bound, and performance claim is accounted for.
