# Language parser roadmap

This records the completed C99 language-parser phases and the maintenance
contracts retained from their implementation plans. The
[compliance checklist](c99-parser-compliance-checklist.md) is the detailed
grammar, ownership, and evidence inventory; [GLOSSARY.md](GLOSSARY.md) defines
the canonical domain terms.

The roadmap covers syntax parsing only. Semantic analysis has its own record in
[semantic-analysis.md](semantic-analysis.md), and language modes and extensions
in [language-standards.md](language-standards.md); backend lowering and code
generation are not implemented.

## Completed foundations

These milestones describe the boundaries during implementation. The deferred
children mentioned in early phases were removed by Phase 04.

### Phase 01: Conditional-expression foundations

Correct the existing conditional-expression reduction behavior and make the
syntax model suitable for a later language-expression parser.

Exit: nested conditional-expression regressions are covered and expression
syntax records grammar facts rather than premature semantic results.

### Phase 02: Declaration stack migration

Move the implemented declaration, declarator, parameter, struct/union, and enum
grammar onto the explicit parser stack. Establish typed future-child seams for
grammar that depends on expressions, initializers, or statements.

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

## Maintenance contracts

`Parser::parse_translation_unit` returns the shared caller and behavior-test
interface, `ParsedTranslationUnit<'tu>`, whose source-ordered roots refer to
immutable syntax nodes in the translation-unit arena. `next_item` drives the
same machine one external declaration at a time over the already
preprocessed token array; the batch pipeline finishes preprocessing before
either begins. Keep private frame phases and parse-arena working state behind
that interface. Function prototypes remain declarations; only definitions
with bodies become function-definition nodes. Preserve source order in block
items, arguments, initializer elements, and designators through immutable
arena lists.

### Grammar ownership

| Decision or boundary | Owner |
| --- | --- |
| Declaration versus function definition | `ExternalDeclarationFrame` |
| Declaration terminator `;` | `DeclarationFrame` |
| Old-style parameter declarations and function body | `FunctionDefinitionFrame`, using declaration and compound children |
| Block braces, block scope, and ordered block items | `CompoundStatementFrame` |
| Statement keywords, header parentheses, `for` semicolons, label colon, and nearest unmatched `else` | `StatementFrame` |
| Array-declarator closing bracket | `DeclaratorFrame` |
| Initializer braces, element commas, designations, and designator brackets | `InitializerFrame` |
| Grouping, call/subscript markers, and conditional `?`/`:` pairing | `ExpressionFrame` |

An expression child leaves its caller's delimiter unconsumed. Nested grouping,
calls, subscripts, and conditional operands change which delimiter is active.
Keep the entry modes distinct:

| Entry mode | Outermost grammar | Uses |
| --- | --- | --- |
| Expression | Includes comma operators | Statements, conditions, grouped expressions, subscripts, conditional middle operand |
| Assignment expression | Comma belongs to the caller | Call arguments, scalar initializers, array bounds |
| Constant expression | Conditional-expression grammar | Case labels, enumerator values, bit fields, array designators |

Constant-expression syntax does not establish that a value is constant; later
analysis owns that constraint. The unmatched question marker blocks reduction
until its colon is matched, while the completed conditional operator associates
to the right. Test nested middle and right operands separately, and distinguish
`f(a,b)` from `f((a,b))`.

Typedef classification uses the name visible at the exact token position.
Label lookahead takes precedence inside blocks: `T:` is a label even when `T`
is a typedef name. Member identifiers after `.` or `->` use their own namespace.
The scope-owning frame restores its entry depth on success, recovery, and EOF.
Function label and switch state have separate lifetimes from ordinary names.

### Recovery and validation

For each frame phase, specify its delimiters, accepted child values, unconsumed
stop tokens, EOF result, synchronization policy, state restoration, returned
syntax, and progress before reprocessing lookahead. A recovered child marks the
smallest enclosing inexact node. Preserve absent optional syntax separately
from missing required syntax, recovered syntax, and an error node.

Test malformed children at separators, owned and enclosing closers, following
declaration/statement starts, and EOF. Include declaration/declarator, function/
compound, statement/expression, expression/type-name, and initializer/designator
boundaries. Verify that following valid input survives and that frames, pending
children, recovery state, scopes, and reducer scratch return to their documented
terminal state. Bounded property tests and truncations must prove termination
without input-triggered panics.

Retain exact identifier, operator, delimiter, and macro/include provenance.
Use structured diagnostics for codes, expectations, found tokens, primary and
related locations, and recovery context; user-facing text must not expose arena
handles or private enum names. Internal FIFO delivery and CLI presentation are
distinct: see the [README](README.md#pipeline-order-and-benchmarks) for
the presentation order.

Defaults must meet the C99 translation floors in the compliance checklist.
Configured or representational resource exhaustion should produce a stable
diagnostic. Exhausting operating-system memory or an arena's address-space
reservation is outside that recovery guarantee, except that a source file
that cannot be read into its arena is reported like any other unreadable file.
Measure storage growth for deeply nested and macro-fragmented syntax, and keep
inspection iterative and bounded by reachable syntax.

GCC/Clang comparisons use pinned versions and normalized language, warning,
target, and error-limit settings. Classify disagreements against C99 in the
[compatibility manifest](tests/fixtures/parser/compatibility/manifest.md);
compiler output alone is not the language specification. Keep quality-warning
policy separate from syntax acceptance and later semantic constraints.

## Meaning of parser-complete

With Phase 05 complete, the language parser is complete for the project's declared C99
syntax target when it can construct source-backed syntax for complete
translation units and recover predictably from malformed input.

Parser-complete does not mean compiler-complete. The following remain outside
the parser:

- type resolution and compatibility;
- linkage, storage-duration, and redeclaration constraints;
- control-flow constraints and return checking;
- constant-expression evaluation required by semantic rules;
- intermediate representation, optimization, code generation, assembly, and
  linking;
- non-C99 extensions unless separately planned.

Semantic analysis ([semantic-analysis.md](semantic-analysis.md)) owns the first
four; the backend items are not implemented; extensions are planned in
[language-standards.md](language-standards.md).

## Phase rule

Each completed phase leaves one reachable parser control flow. No deferred
child remains on a supported C99 syntax path. Typed syntax, stable structured
diagnostics, source provenance, parent-owned delimiters, and explicit semantic
handoffs remain the governing rule for later front-end work.
