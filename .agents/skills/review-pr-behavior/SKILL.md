---
name: review-pr-behavior
description: Review a bcc-rust PR for observable correctness, edge cases, state transitions, integration behavior, and compliance with the change's stated intent. Use independently for behavior review or as the generic correctness pass selected by review-pr; architecture and general test quality belong to sibling skills.
---

# Review PR Behavior

Read [the shared review contract](../review-pr/references/review-contract.md) completely and apply it to this scope.

Review what the changed code does, including failure paths and interactions across modules. Establish the PR's intent from its description, commits, applicable plans, current public behavior, and repository contracts; distinguish planned future behavior from the current implementation.

Trace important inputs and state through changed boundaries. Look for:

- behavior contradicting the PR intent or an existing contract;
- wrong branching, ordering, ownership, or state transitions;
- boundary values, empty input, EOF, malformed input, and partial progress;
- stale state, incorrect restoration, nontermination, panics, and resource-lifetime mistakes;
- source-provenance or diagnostic plumbing that changes observable locations or messages;
- phase integration failures where individually plausible components disagree about representation or ownership; and
- compatibility regressions in the CLI or public Rust interfaces.

Inspect tests as behavioral evidence, but leave broad test-suite adequacy to `review-pr-testing`. Leave domain-specific C grammar and translation-rule completeness to the applicable compiler-phase skill. Report an architectural concern here only when it directly produces a concrete behavior failure.

Complete only after every materially changed behavior and failure path is accounted for with either a candidate, an open question, or a concise explanation of why it remains correct.
