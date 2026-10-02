---
name: review-pr-lexical-processing
description: Review bcc-rust initial processing and preprocessing-token formation for C99 translation phases 1-3, lexical boundaries, whitespace/comment handling, and provenance. Use for changes to initial_processing.rs, preprocessor_tokenizer.rs, or shared inputs and outputs that can alter those phases.
---

# Review PR Lexical Processing

Read [the shared review contract](../review-pr/references/review-contract.md) completely. Read `.agents/AGENTS.md`, `GLOSSARY.md`, `README.md`, `src/translation_phases.rs`, `src/translation_phases/initial_processing.rs`, and `src/translation_phases/preprocessor_tokenizer.rs` as applicable. Verify disputed language rules against WG14 N1256 §5.1.1.2 and §6.4 rather than relying on intuition.

Review translation phases 1-3 as an ordered transformation with source provenance:

- source-character mapping, newline normalization, trigraph replacement, and their lookahead boundaries;
- escaped-newline deletion after phase-1 mapping, including repeated splices and EOF;
- comment replacement, whitespace-sequence behavior, and preservation of newlines needed by directives;
- preprocessing-token categories and longest-match behavior, including identifiers, pp-numbers, character and string literals, header-name contexts, punctuators, and partial invalid prefixes;
- the distinction between preprocessing tokens and parser-facing tokens;
- source vectors across replacement, deletion, normalization, multi-character tokens, and zero-width diagnostics; and
- streaming progress and bounded lookahead at chunk, line, and EOF boundaries.

Transformation ordering is part of correctness: a case that works when each operation is tested alone may fail when trigraph replacement creates a splice or when splicing changes comment/token adjacency. Build paired cases around those compositions and around inputs with the same prefix but different lexical outcomes.

Leave macro expansion, directive semantics, and include behavior to `review-pr-preprocessing`; leave C language grammar to `review-pr-parsing`. Still inspect the handoff representations and report cross-phase contract mismatches in this scope.

Complete only after every changed lexical category, transformation composition, boundary/EOF path, and provenance transition is accounted for.
