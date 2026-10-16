---
name: review-pr-semantic-analysis
description: Review bcc-rust semantic analysis changes for C type rules, layout, binding and linkage, conversions, constant evaluation, initializers, retained semantic facts, builtin contracts, attributes, and semantic diagnostics with their Clang parity. Use for changes to the semantic_analysis module, src/builtins, the configuration feature gates and warning groups it consults, or shared types that alter the retained SemanticTranslationUnit.
---

# Review PR Semantic Analysis

Read [the shared review contract](../review-pr/references/review-contract.md) completely. Read `.agents/AGENTS.md`, `GLOSSARY.md`, `semantic-analysis.md` (purpose, storage, binding, the staged roadmap stage the change belongs to, and any dated section it amends), `language-standards.md` for mode and extension gates, `tests/fixtures/diagnostics/COVERAGE.md` for diagnostic changes, `docs/design/semantic-facts.md` when a retained fact changes, and `src/translation_phases/semantic_analysis.rs` with the submodules the change touches. Verify disputed language rules against WG14 N1256 §6.2-§6.9 with the `search-standard` skill; treat the pinned Clang's behavior as the parity oracle for extensions, warnings and implementation-defined choices, and say which oracle settled each claim.

Review semantic analysis as the single producer of language facts that lowering consumes without re-deriving them:

- type construction, compatibility, composite types, qualifiers, incomplete and variably modified types, and the target data model (size, storage and member-place alignment, bit-field allocation for both System V and Microsoft layouts, `#pragma pack`);
- ordinary and tag lookup, scope undo, linkage inheritance (§6.2.2), storage and thread duration, redeclaration, tentative and inline definitions, and recovered or rejected declarations that must stay visible without cascading;
- expression typing, value category, the ordered conversion records applied to each operand, usual arithmetic conversions, integer promotions, pointer arithmetic and comparison rules, `sizeof`/`alignof`/`typeof` evaluation context, and lvalue/modifiable-lvalue constraints;
- integer and floating constant evaluation, integer constant expression eligibility, address constants, and 128-bit patterns;
- initializer current-object traversal, designators, brace elision, string and flexible-array completion, and static versus automatic initializer rules;
- retained facts (control and lifetime regions, resolved calls and members, occurrence and parameter correspondence, runtime-size sites, expression effects and recipes): every lowerable construct gets exactly one record, no consumer re-derives it, and a missing record surfaces as `MissingFact` rather than a guess;
- builtin registry entries, checked instances, attribute catalog rules, and extension gating through `accepts`/`is_native` and `Context::report_extension`; and
- diagnostic kind, severity, warning group and `-W`/`-Werror` behavior, system-header suppression, primary and previous-declaration labels, and the rendered golden fixture.

For each changed rule, write the paired accepted and rejected C inputs a reviewer would try in Clang, including the same construct under C89, C99, C11 and GNU modes when gating differs and on both Linux and Windows targets when layout or a type width differs. A rule that accepts invalid C is as serious as one that rejects valid C when lowering trusts the accepted fact: follow each new or changed fact into its lowering consumer and confirm the consumer's assumptions hold for every accepted input.

Keep traversal iterative and allocation in arenas: flag native recursion over user-controlled depth, a collection outside the arena types, or a fact recorded in a working arena that lowering later reads. Leave grammar and AST shape to `review-pr-parsing`, IR construction to `review-pr-lowering`, and general diagnostic recovery mechanics to `review-pr-error-recovery`, while checking every handoff contract this change alters.

Complete only after every changed type rule, conversion, constant, initializer path, binding effect, retained fact and its consumer, builtin or attribute rule, feature gate, and diagnostic is accounted for.
