---
name: review-pr-design
description: Review a bcc-rust PR for architecture, maintainability, and repository conventions as one design pass. Use independently for structural review or as the generic design pass selected by review-pr; concrete runtime bugs and general test adequacy belong to sibling skills.
---

# Review PR Design

Read [the shared review contract](../review-pr/references/review-contract.md) completely and apply it to this scope. Read `.agents/AGENTS.md`, applicable `GLOSSARY.md` material, plans, and ADRs before evaluating intended boundaries.

Review the smallest structure that makes the change durable:

- module boundaries, dependency direction, ownership, cohesion, and coupling;
- whether public and internal interfaces expose stable concepts rather than incidental machinery;
- state and invariant ownership, especially when policy is copied through constructors or child seams;
- representation choices, type safety, impossible states, and provenance ownership;
- complexity, duplication, naming, comments, and cleanup completeness where they materially affect comprehension or change safety;
- consistency with the explicit non-recursive compiler architecture and current roadmap boundaries; and
- compliance with repository instructions, including keeping first-party customization out of vendored snapshots.

Do not flag subjective style, formatter output, speculative future extensibility, or a large diff merely for being large. A finding needs a concrete maintenance cost, inconsistent invariant, likely divergence point, or failure risk. Prefer the smallest viable structural correction over broad redesign.

Complete only after every new or changed abstraction, state owner, module seam, and documented invariant is accounted for.
