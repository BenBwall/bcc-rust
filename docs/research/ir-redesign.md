# An object-aware IR for bcc

Study baseline: `145898d4` (`origin/main` when this worktree was created),
2026-10-21. All repository `file:line` references in this document refer to
that commit. This study changes documentation only.

## 1. Inventory of the current IR

### Initial findings

The strongest LLVM coupling is semantic, not syntactic. `src/ir.rs:13`
explicitly adopts LLVM poison semantics without `undef`, and
`middle-end.md:161` specifies poison propagation, `freeze`, and `inbounds`.
The block-parameter CFG, dense entity IDs, explicit memory and verifier are
useful independent choices and do not require those semantics.

Two candidate leaks need qualification:

- ABI lowering is **after** IR construction and optimization, not before the
  IR. The current pipeline has pre-ABI and post-ABI profiles
  (`middle-end.md:45`). Every execution back end, including the interpreter,
  ultimately receives ABI-lowered IR.
- Aggregates are **not** ordinary first-class SSA values. `src/ir/types.rs:5`
  says they live in storage; pre-ABI calls retain aggregate layout and
  by-value intent through pointer parameters and signature descriptors.

The remaining inventory will distinguish C rules, LLVM-derived contracts and
bcc implementation choices, including cases where a feature is absent.

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
