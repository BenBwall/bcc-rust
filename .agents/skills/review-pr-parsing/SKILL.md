---
name: review-pr-parsing
description: Review bcc-rust C language parser changes for C99 grammar, AST shape, typedef ambiguity, scope, explicit frame-machine control, synchronization, diagnostics, and provenance. Use for changes to parsing.rs or shared types and phase interfaces that affect syntax parsing.
---

# Review PR Parsing

Read [the shared review contract](../review-pr/references/review-contract.md) completely. Read `.agents/AGENTS.md`, `GLOSSARY.md`, `parser-roadmap.md`, the applicable phase plan or compliance checklist, `src/translation_phases.rs`, and `src/translation_phases/parsing.rs`. Treat the roadmap as authoritative for current phase boundaries. Verify disputed language rules against WG14 N1256 §6.1-§6.9.

Review syntax parsing as an explicit, non-recursive control machine:

- complete C99 production alternatives and binding shape for each changed grammar family;
- declarator precedence and the identifier-binding path through pointers, arrays, functions, parentheses, and abstract forms;
- expression precedence, associativity, lvalue-independent syntax facts, cast/grouping ambiguity, postfix chains, conditional expressions, and comma boundaries when affected;
- declaration-versus-expression and cast-versus-grouping decisions using parser-visible typedef and ordinary-name classification;
- scope entry, publication, shadowing, restoration, and distinct C namespaces needed to choose grammar;
- ownership of lookahead and delimiters across parent/child frames, typed child results, push/reduce/reprocess actions, and EOF;
- AST arena handles, ordered children, explicit recovered variants, and complete original-source provenance; and
- diagnostics and synchronization that preserve the nearest caller-owned boundary and following valid construct.

Keep syntax and semantics separate. Flag a semantic check in the parser when it prematurely constrains valid syntax or corrupts the AST, but do not demand type checking, constraint validation, linkage analysis, or code generation that the roadmap assigns to later compiler projects.

For every changed frame or state field, enumerate construction, child-push, reduction, reprocess, recovery, and unwind paths. For ambiguous tokens such as identifiers, colons, braces, parentheses, and commas, test both grammar owners and nested contexts. A locally correct parse is insufficient if scope, frame, or provenance state is wrong for the next declaration or statement.

The `review-pr-error-recovery` pass owns the exhaustive cross-phase recovery matrix; this skill owns the parser-domain contract that defines valid syntax, caller ownership, and recovered AST shape. Report overlapping root causes normally so the Review Lead can deduplicate them.

Complete only after every changed production, ambiguity decision, frame transition, scope effect, synchronization boundary, AST result, and roadmap claim is accounted for.
