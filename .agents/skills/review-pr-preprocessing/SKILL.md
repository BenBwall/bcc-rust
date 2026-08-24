---
name: review-pr-preprocessing
description: Review bcc-rust preprocessing changes for C99 directive, include, conditional-group, macro-expansion, preprocessing-expression, and token-conversion correctness. Use for changes to preprocessing.rs or shared types and phase interfaces that affect preprocessor behavior.
---

# Review PR Preprocessing

Read [the shared review contract](../review-pr/references/review-contract.md) completely. Read `.agents/AGENTS.md`, `CONTEXT.md`, `README.md`, `src/translation_phases.rs`, and `src/translation_phases/preprocessing.rs` as applicable. Verify disputed language rules against WG14 N1256 §5.1.1.2, §6.4, and §6.10.

Review the full preprocessing lifecycle:

- directive recognition at logical line boundaries and behavior in skipped conditional groups;
- object-like and function-like macro argument collection, prescan, substitution, disabling, rescan, recursion, variadics, stringizing, token pasting, and empty arguments;
- `#if` expression token treatment, `defined`, integer evaluation, operator precedence, diagnostics, and strict-C99 extension policy;
- nested conditional-group state, branch selection, EOF, and restoration after malformed directives;
- quote and system include search order, include provenance, recursion, missing files, and interaction with caller configuration;
- `#line`, predefined macros, pragma handling, and source location effects when changed;
- conversion of surviving preprocessing tokens into parser-facing identifiers, keywords, punctuators, and typed literals; and
- source vectors through macro arguments, replacement lists, paste/stringize results, includes, and diagnostics.

Macro correctness is expansion-order correctness. Trace concrete token sequences through collection, replacement, disabling, rescan, and output rather than reviewing one helper in isolation. Exercise nested and mutually referential macros, arguments containing commas or directives-sensitive newlines, valid and invalid paste pairs, and tokens whose classification changes only after expansion.

Distinguish preprocessing-token rules from parser-token and language-grammar rules. Leave translation phases 1-3 to `review-pr-lexical-processing` and C syntax parsing to `review-pr-parsing`, while checking both handoff contracts.

Complete only after every changed directive family, macro state transition, conditional/include stack path, token conversion, error/EOF branch, and provenance flow is accounted for.
