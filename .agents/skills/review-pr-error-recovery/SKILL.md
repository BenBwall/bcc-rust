---
name: review-pr-error-recovery
description: Review fallible bcc-rust compiler changes for recovery, diagnostic quality, progress, state restoration, provenance, and preservation of following valid input. Use for every non-trivial compiler PR, even when the diff is not explicitly labeled as error handling.
---

# Review PR Error Recovery

Read [the shared review contract](../review-pr/references/review-contract.md) and [the recovery matrix](references/recovery-matrix.md) completely. Apply both across every affected compiler phase.

Treat recovery as a cross-cutting protocol, not a local error branch. A change to token classification, state, lookahead, ownership, source vectors, child construction, or phase interfaces can alter recovery even when no diagnostic code changed.

Inventory each changed fallible seam and exercise the matrix against all caller contexts and sibling paths. Inspect both malformed inputs and valid inputs that share the same boundary shape. Require evidence for progress, exact state restoration, correct boundary ownership, useful primary diagnostics without avoidable cascades, source-backed or explicitly anchored recovered output, and successful continuation at the next valid construct.

This scope applies to initial character processing, preprocessing-token recognition, directives and macro expansion, parser control flow, and shared diagnostic/provenance infrastructure. Use each phase's native unit of progress and ownership rather than assuming parser tokens.

Return `Inapplicable` only after establishing that the compiler PR changes no fallible input path, recovery-sensitive state, diagnostic, provenance, boundary classification, or continuation behavior. Complete only when every changed seam and every new recovery predicate or state field has been checked against sibling callers, nested children, inverse token ownership, and valid neighboring syntax.
