# Language parser roadmap

This is the authoritative phase sequence for completing the C99 language
parser. Phase implementation briefs define the work inside one phase; this
document defines the boundary between phases.

The roadmap covers syntax parsing only. Semantic analysis, backend lowering,
code generation, and compiler extensions remain separate projects.

## Completed foundations

### Phase 01: Conditional-expression foundations

Correct the existing conditional-expression reduction behavior and make the
syntax model suitable for a later language-expression parser.

Plan: `phase-01a-conditional-expression-foundations-agent-brief.md`.

Exit: nested conditional-expression regressions are covered and expression
syntax records grammar facts rather than premature semantic results.

### Phase 02: Declaration stack migration

Move the implemented declaration, declarator, parameter, struct/union, and enum
grammar onto the explicit parser stack. Establish typed future-child seams for
grammar that depends on expressions, initializers, or statements.

Plans:

- `phase-02-legacy-parser-stack-migration-agent-brief.md`;
- `phase-02-legacy-parser-stack-migration-plan.html`.

Exit: the migrated declaration subset is reachable through
`Parser::next_item`, uses no recursive parser calls, and recovers with
source-backed valid or recovered declaration nodes.

### Phase 03: Statements, blocks, and function definitions

Status: complete.

Add the structural parser machinery for function definitions, compound
statements, ordered block items, statement forms, and function/block scope.

Expression-bearing statement positions retain an explicit typed deferred-child
slot in this phase. The parent statement still owns its delimiters and produces
a source-backed structural node; the deferred child emits the stable unsupported
diagnostic. Initializers inside block declarations continue to use the Phase 02
initializer seam.

Plans:

- `phase-03-statements-and-function-definitions-plan.md`;
- `phase-03-statements-and-function-definitions-plan.html`.

Exit:

- prototype-style and old-style function definitions yield real external
  function-definition nodes;
- compounds retain declarations and statements in source order;
- every C99 statement production has a structural syntax form;
- valid function bodies no longer use `FunctionBodyNotImplemented` or balanced
  body skipping;
- expression and initializer gaps remain explicit typed seams rather than
  ad hoc token skipping.

### Phase 04: Expressions, type names, and initializers

Status: complete.

Implement the non-recursive language `ExpressionFrame` using the agreed
Double-E-style precedence reducer. Add the type-name parsing required by casts,
`sizeof`, and compound literals, including typedef-name ambiguity. Implement
scalar and brace-enclosed initializers, initializer lists, designations, and
designators in a non-recursive `InitializerFrame`.

Connect expression children in statements and function bodies. Replace the
Phase 03 deferred statement-expression slots with expression indices while
preserving parent delimiter ownership. Connect the same expression entry modes
to every deferred declaration position: array bounds, bit-field widths,
explicit enumerator values, declaration initializers, and compound-literal
initializers.

Plans:

- `phase-04-expressions-and-type-names-plan.md`;
- `phase-04-expressions-and-type-names-plan.html`.

Exit:

- primary, postfix, unary, cast, multiplicative through logical, conditional,
  assignment, and comma expressions produce source-backed syntax nodes;
- cast-versus-grouping decisions use current parser-visible name classes;
- all expression-bearing statement positions contain parsed expression nodes;
- file-scope and block-scope object initializers produce syntax nodes;
- C99 designated initializers and nested initializer lists parse;
- array bounds, bit-field widths, and enumerator values contain parsed
  expressions;
- malformed expressions, type names, initializers, and designators synchronize
  without corrupting the parser stack;
- no Phase 02 or Phase 03 expression/initializer future-child seam remains on a
  supported C99 grammar path.

## Completed closure phase

### Phase 05: Recovery and C99 parser closure

Status: complete.

The complete translation-unit grammar is audited against the C99 compliance
matrix. Cross-family recovery, translation limits, source provenance, phase-7
totality, diagnostics, and parser inspection output are hardened. Remaining
constraint and semantic work is explicitly assigned to later analysis.

Exit:

- whole translation units compose declarations, definitions, initializers,
  expressions, statements, labels, and nested blocks;
- every supported malformed production terminates, restores scope/frame state,
  and resumes at the nearest legal synchronization point;
- minimum C99 translation-limit cases are covered without Rust call-stack
  recursion;
- the compliance matrix identifies every C99 grammar production as supported,
  intentionally unsupported, or a semantic-analysis responsibility;
- no typed future-child seam remains for supported C99 syntax.

## Meaning of parser-complete

With Phase 05 complete, the language parser is complete for the project's declared C99
syntax target when it can construct source-backed syntax for complete
translation units and recover predictably from malformed input.

Parser-complete does not mean compiler-complete. The following remain separate:

- type resolution and compatibility;
- linkage, storage-duration, and redeclaration constraints;
- control-flow constraints and return checking;
- constant-expression evaluation required by semantic rules;
- intermediate representation, optimization, code generation, assembly, and
  linking;
- non-C99 extensions unless separately planned.

## Phase rule

Each completed phase leaves one reachable parser control flow. No deferred
child remains on a supported C99 syntax path. Typed syntax, stable structured
diagnostics, source provenance, parent-owned delimiters, and explicit semantic
handoffs remain the governing rule for later front-end work.
