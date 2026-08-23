# Phase 03: Parse statements and function definitions

## Implementation status

Completed on 23 August 2026. `Parser::next_item` now reaches prototype-style
and old-style function definitions, ordered compound block items, and every
structural C99 statement family through heap-backed frames. Function,
prototype, block, implicit selection/iteration, label, and switch lifetimes are
covered by recovery and translation-floor tests; valid definitions no longer
use a function-body future-child seam.

Phase 04 still owns `StatementExpression` and
`StatementConstantExpression` deferred slots and will replace their source
provenance with expression indices. Phase 05 still owns initializer, array
bound, bit-field width, and enumerator-value seams. The completion run has 99
passing all-target tests and a clean nightly formatting check. Strict Clippy
continues to report 84 existing error lines in generated bindings and
non-parser modules; no diagnostic points at `parsing.rs`.

## Objective

Extend the explicit parser stack from file-scope declarations into C99
function definitions, compound statements, and statements.

This phase builds on the declaration and declarator machinery delivered in
Phase 02. Function declarations are already ordinary declarations and stay
that way. The new work is to distinguish a definition from a declaration,
enter the function body, parse its block items, and produce statement and
function-definition syntax nodes through `Parser::next_item`.

The phase must preserve the parser's core protocol: one delimiter owner,
owned frame actions, typed child returns, heap-backed grammar nesting, exact
source provenance, and recovery that unwinds to a named owner.

Expected size: roughly 12–20 focused days for one agent, delivered as small
green commits. Expression and initializer parsing belong to Phases 04 and 05.
This phase preserves their positions as explicit typed deferred children so it
can finish independently without token skipping or duplicated grammar.

## Scope

Add or complete:

- the syntax model and arenas for function definitions, statements, and block
  items;
- external-declaration dispatch between declarations and function definitions;
- prototype-style and old-style function definitions;
- compound statements containing interleaved declarations and statements;
- labeled, selection, iteration, jump, expression, and null statements;
- function, prototype, and block scope lifetimes;
- parameter publication in function scope, `__func__`, and a function-local
  label namespace;
- recovery at semicolons, closing parentheses, closing braces, and statement
  starts;
- parser-interface tests and manual inspection through the existing CLI syntax
  dump.

Preserve the current declaration behavior, including function prototypes,
variadic parameter lists, K&R identifier lists, nested declarators, and
function-pointer declarations.

Do not add a separate function-declaration node. In C, a prototype such as
`int f(int x);` remains a `Declaration`; only a definition with a body becomes
a `FunctionDefinition`.

## Required reading and baseline

Before editing, read:

- `.agents/AGENTS.md` and `CONTEXT.md`;
- `phase-01a-conditional-expression-foundations-agent-brief.md`;
- `phase-02-legacy-parser-stack-migration-agent-brief.md` and its HTML
  companion;
- the statements, external declarations, scopes, diagnostics, delimiter
  ownership, and translation-limit items in
  `c99-parser-compliance-checklist.md`;
- the current `src/translation_phases/parsing.rs`, `src/lib.rs`, and CLI entry
  point.
- `parser-roadmap.md`, which is authoritative for the boundary between this
  phase and Phases 04–06.

At the Phase 03 planning baseline on 23 August 2026:

- `Parser::next_item` yields one `ExternalDeclaration` at a time;
- function declarations already parse as ordinary declarations;
- a sole uninitialized function-binding declarator followed by `{` is
  recognized as a future function body, diagnosed with
  `FunctionBodyNotImplemented`, skipped as balanced braces, and returned as a
  recovered declaration;
- old-style definitions with a declaration list between the declarator and
  body are not supported;
- no compound-statement or statement frame is reachable;
- `ScopeStack` handles file and prototype-name classification, but does not yet
  model function and block scope explicitly;
- the CLI already prints parser syntax storage by default, while `--tokens`
  retains the token dump for comparison;
- the all-target test suite is green at the current working baseline;
- strict workspace Clippy has pre-existing debt outside the parser, so this
  phase uses changed-file attribution rather than claiming a clean workspace.

Capture the exact test count, diagnostics, parser-frame inventory, and current
syntax-store layout before the first implementation commit.

## Phase boundary

Statements contain expression and initializer grammar positions. Phase 03 owns
their parent frames and delimiter contracts, while Phases 04 and 05 own the
general-purpose expression and initializer implementations.

Phase 03 must define typed deferred child contracts for:

- `expression`, stopping before a delimiter owned by the parent;
- `constant-expression`, used by `case` labels;
- optional expressions, whose syntactic absence is distinct from a deferred
  present expression;
- declaration initializers inside compound statements and `for` initializers,
  using the existing initializer seam.

Represent a present-but-deferred statement expression explicitly, for example
with a private `ExpressionSlot::{Parsed, FutureChild}` model used by statement
nodes. Do not overload `Option` to mean both “grammar position absent” and
“expression present but deferred.” The future child emits a focused diagnostic,
retains source provenance, and preserves the parent's delimiter. Phase 04
replaces these slots with parsed expression indices; Phase 05 removes the
initializer seams.

Do not implement an ad hoc expression parser inside `StatementFrame` or
`FunctionDefinitionFrame`.

## Machine and ownership protocol

Keep `Parser` as the `TranslationPhase<Item = ExternalDeclaration>` interface.
Add only the private frame and value variants required by this phase:

```rust
enum ParseFrame {
    // Existing declaration frames...
    ExternalDeclaration(ExternalDeclarationFrame),
    FunctionDefinition(FunctionDefinitionFrame),
    CompoundStatement(CompoundStatementFrame),
    Statement(StatementFrame),
}

enum ParseValue {
    // Existing declaration values...
    FunctionDefinition(FunctionDefinitionIndex),
    CompoundStatement(StatementIndex),
    Statement(StatementIndex),
    FutureChild(FutureChildResult),
}
```

The exact Rust names may follow existing conventions, but these ownership
rules are fixed:

- `ExternalDeclarationFrame` owns the declaration-versus-definition decision;
- `DeclarationFrame` owns semicolon-terminated declarations at file, block,
  `for`, and old-style parameter-declaration positions;
- `FunctionDefinitionFrame` owns the optional old-style declaration list and
  the function body;
- `CompoundStatementFrame` owns both braces, block scope, and the ordered block
  item sequence;
- `StatementFrame` owns statement keywords and punctuation, and pushes child
  frames for nested statements, expressions, declarations, or compounds;
- each parent names the tokens its child must leave unconsumed;
- a frame restores scope depth to its entry depth on success, recovery, and
  EOF;
- a recovered child marks the smallest enclosing syntax node that is no longer
  exact.

Do not add a shallow `TranslationUnitFrame` only to collect items.
`Parser::next_item` already provides the translation-unit sequence. Add an
aggregate translation-unit node only when a caller demonstrates a concrete
need for one.

## Syntax model

Use an ordered block-item model so a C99 compound statement can preserve mixed
declarations and statements:

```rust
enum BlockItem {
    Declaration(DeclarationIndex),
    Statement(StatementIndex),
}

struct CompoundStatement {
    items: VectorSlice<BlockItem>,
    source: SourceVector,
    recovered: bool,
}
```

The Phase 03 `Statement` model must represent at least:

- labeled statements: identifier labels, `case`, and `default`;
- compound statements;
- expression statements and the null statement;
- `if`/`else` and `switch`;
- `while`, `do`/`while`, and both declaration and expression forms of `for`;
- `goto`, `continue`, `break`, and `return` with an optional expression.

Every statement carries source provenance and recovery state. Add a distinct
`Switch` form; do not encode it as `If`. Model bare `return;` without a dummy
expression. Preserve `;` as a real null statement. Expression-bearing variants
use an explicit parsed-or-deferred slot until Phase 04; syntactically optional
positions retain a separate outer `Option`.

Add `FunctionDefinition`, its arena/index, and valid/recovered
`ExternalDeclaration` variants. The function-definition node records the
declaration specifiers, declarator, any old-style declaration list, body,
source vector, and recovery state.

## Ownership map

| Grammar decision or token | Owning module/frame |
| --- | --- |
| File item starts as declaration or definition | `ExternalDeclarationFrame` |
| Specifiers and first declarator | Existing declaration/declarator frames |
| `;` ending a declaration | `DeclarationFrame` |
| Old-style declaration list | `FunctionDefinitionFrame` using seeded `DeclarationFrame` children |
| `{` and `}` of a function body or nested block | `CompoundStatementFrame` |
| Ordered declaration/statement choice inside a block | `CompoundStatementFrame` |
| Statement keyword and its required punctuation | `StatementFrame` |
| Parentheses around statement conditions | The owning statement phase |
| Expression terminator such as `;`, `)`, or `:` | Parent statement phase, never `ExpressionFrame` |
| `else` association | The nearest unmatched `If` phase |
| Function, prototype, and block scope lifetime | The frame that enters that scope |
| Function-local labels | Function-definition scope state |

Label lookahead precedes typedef-name classification inside a compound
statement. An identifier followed by `:` is a label even if the same spelling
is currently classified as a typedef name.

## Step 1: Freeze the baseline and grammar matrix

Add black-box parser cases for every current behavior that Phase 03 will touch.
Keep tests at the `Parser` interface; frame phases remain private.

Characterize at least:

```c
int declared(int x);
int (*pointer)(int x);
int defined(int x) { return x; }
int old_style(a) int a; { return a; }
```

Record the yielded item, syntax-store entries, diagnostics, and source spans.
Add a grammar matrix covering every statement production, compound block-item
ordering, prototype and old-style definitions, malformed delimiters, and EOF
at every owned delimiter.

Classify each case as already supported, Phase 03 work, expression/initializer
dependency, semantic constraint, or out of scope. This matrix is the source of
the remaining test steps.

Completion criterion: every Phase 03 grammar production and recovery edge has
a named test case and an owner; current function-declaration behavior is locked
by regression tests.

## Step 2: Repair the syntax model and storage

Add `BlockItem`, statement source/recovery data, missing statement variants,
optional return expressions, and a null-statement representation. Add the
function-definition node, indices, arenas, syntax-store accessors, and
`ExternalDeclaration` variants.

Keep the module deep: callers receive stable indices and inspect nodes through
`SyntaxStore`; they do not coordinate frame phases or parallel vectors.

Write focused construction tests for ordering, vector-slice arena selection,
and source-vector retention before wiring new parser paths.

Completion criterion: the model can losslessly represent every in-scope
statement, mixed block item, and function-definition form; absent and deferred
expression positions are unambiguous and carry source provenance.

## Step 3: Separate declarations from function definitions

Move the file-scope decision into `ExternalDeclarationFrame`. Reuse the
existing declaration-specifier and declarator children, then choose the owner
of the remaining tokens:

- comma, initializer, or semicolon continues as a declaration;
- `{` may start a definition only after exactly one uninitialized declarator
  whose completed binding is a function;
- a declaration list after an old-style identifier-list declarator transfers
  ownership to `FunctionDefinitionFrame`;
- every other `{` after a declarator is diagnosed and recovered without being
  accepted as a definition.

A function pointer does not become a function definition merely because its
declarator contains a function suffix. Base the decision on the completed
declarator's binding shape.

Keep `DeclarationFrame` reusable in block, `for`, and old-style declaration
contexts. Its constructor should make context and allowed terminators explicit;
do not clone the declaration grammar into the function-definition frame.

Completion criterion: prototypes and function-pointer declarations still
yield `Declaration`; valid definition heads transfer to a typed
`FunctionDefinitionFrame`; illegal heads produce focused diagnostics.

## Step 4: Make scope lifetimes explicit

Replace undifferentiated nested scopes with explicit file, function,
function-prototype, and block scope kinds. Each scope-owning frame records its
entry depth and restores that depth on every exit path.

When a definition begins:

- publish the function name at file scope according to the existing declaration
  rules;
- republish parameter names from the declarator into function scope;
- add the predefined `__func__` binding at the correct point;
- create a function-local label namespace independent of ordinary identifiers;
- enter the outer compound statement's block scope without losing function
  scope.

Model implicit selection and iteration scopes where C99 requires them,
including the lifetime of a declaration in a `for` initializer.

Add tests for typedef shadowing, parameter visibility, nested blocks,
same-spelling labels and typedef names, `for` scope, recovery unwinds, and EOF.

Completion criterion: successful and malformed parses leave scope depth at its
entry value, and name classification is correct at every tested function,
prototype, block, label, and `for` scope point.

## Step 5: Parse compound statements and block items

Implement `CompoundStatementFrame` as the sole owner of `{` and `}`. It enters
block scope, accumulates an ordered `BlockItem` slice, and repeatedly chooses
between a declaration child and a statement child.

The choice order is:

1. `}` completes the compound;
2. identifier-plus-colon starts a labeled statement;
3. declaration-specifier lookahead starts a declaration;
4. otherwise start a statement.

The frame must support empty blocks, declaration-only blocks, statement-only
blocks, and arbitrary C99 interleaving. A malformed child recovers to a token
owned by the compound without consuming the compound's closing brace.

Prove heap-backed nesting with at least 127 nested blocks and verify exact
source spans for both an empty and a recovered compound.

Completion criterion: mixed block items are stored in source order, braces
have one traceable owner, nested blocks do not recurse on the Rust call stack,
and recovery resumes at the correct enclosing `}`.

## Step 6: Parse the statement grammar

Implement `StatementFrame` in vertical slices so each commit reaches a real
function body through `Parser::next_item`.

### 6.1 Structural and jump statements

Land null and compound statements; identifier and `default` labels; `goto`,
`continue`, and `break`; then `return` with and without an expression.
Resolve `goto` labels in the function-local label namespace, while leaving
semantic errors such as `break` outside a loop to a later validation pass.

### 6.2 Selection statements

Add `if`, optional `else`, and `switch`. The nearest unmatched `if` owns
`else`; recovery must not accidentally reattach it to an outer statement.
Add `case` labels only after the constant-expression child entry mode exists.

### 6.3 Iteration statements

Add `while`, `do`/`while`, and `for`. Model all `for` slots independently:
empty, expression, and declaration initializer; optional condition; optional
iteration expression; and the body. The `for` frame owns its two semicolons and
closing parenthesis.

### 6.4 Expression-bearing statements

Push the typed future-child frame for expression statements and all
expression-bearing positions. The child stops before, and does not consume, the
parent-owned delimiter. Retain the statement's structural node with an explicit
deferred-expression slot and focused unsupported diagnostic. Phase 04 replaces
this seam with `ExpressionFrame` without changing statement delimiter
ownership.

Completion criterion: every C99 statement production parses into a distinct,
source-backed structural node; delimiter traces identify one owner; dangling
`else`, all `for` forms, and nested selection/iteration combinations pass the
grammar matrix; each present expression position is an explicit deferred child
and no expression is ad hoc token-skipped.

## Step 7: Build complete function definitions

Have `FunctionDefinitionFrame` combine the definition head, optional old-style
declaration list, and `CompoundStatementFrame` result into a
`FunctionDefinition` node.

Support both:

```c
int add(int left, int right) { return left + right; }
int add(left, right) int left; int right; { return left + right; }
```

For old-style definitions, validate the declaration-list shape structurally:
the list belongs to the definition, does not become file-scope output, and
cannot contain a nested function definition. Defer full type compatibility and
constraint checking to semantic analysis.

Replace `FunctionBodyNotImplemented` and balanced-body skipping with the real
body path. Mark the definition recovered only when its own head, declaration
list, or body is inexact.

Completion criterion: prototype and old-style definitions yield valid or
recovered `ExternalDeclaration::FunctionDefinition` items with reachable body
nodes; `FunctionBodyNotImplemented` is absent from valid definition paths.

## Step 8: Harden diagnostics, recovery, and limits

Give each statement family focused diagnostics for missing punctuation,
missing child expressions, invalid starts, duplicate `default`, malformed
labels, and premature EOF. Separate syntactic recovery from later semantic
constraint checks.

Exercise recovery at:

- semicolons in expression, jump, `do`, and `for` statements;
- closing parentheses in selection and iteration headers;
- colons after labels and `case`/`default`;
- closing braces at every block depth;
- the next plausible statement or declaration start;
- EOF with every frame phase active.

Test translation-limit behavior for at least 127 nested compound statements,
127 parameters in a definition, and deeply nested selection/iteration frames.
The parser must diagnose configured limits without overflowing the Rust call
stack or leaking scopes.

Completion criterion: every malformed case terminates, diagnostics name the
nearest useful cause, recovered nodes are scoped precisely, and subsequent
external declarations remain parseable whenever a synchronization point
exists.

## Step 9: Verify, document, and hand off

Run:

```text
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

If strict Clippy still fails, record the complete output and prove that no
diagnostic points at Phase 03 changed files. Do not broaden this phase into an
unrelated lint cleanup.

Use the CLI syntax dump to inspect representative declarations, prototype and
old-style definitions, every statement family, mixed block items, deferred
expression/initializer slots, and malformed recovery. Confirm `--tokens` still
provides the tokenizer comparison path.

Update `CONTEXT.md`, parser status documentation, frame/grammar inventories,
and the next-phase handoff with exact remaining expression, initializer,
semantic, or extension gaps. Documentation must describe implemented behavior,
not merely merged types.

Completion criterion: formatting and tests pass, changed parser files are
Clippy-clean, the manual syntax dump shows reachable structural nodes and typed
deferred children, and the handoff contains no ambiguous Phase 03 ownership or
untyped placeholder paths.

## Test matrix

The executable matrix must include at least:

- prototypes, variadics, function pointers, prototype-style definitions, and
  old-style definitions;
- empty, declaration-only, statement-only, and mixed compound statements;
- null and expression statements;
- identifier, `case`, and `default` labels;
- `if`, `if`/`else`, dangling `else`, and `switch`;
- `while`, `do`/`while`, every `for` initializer form, and nested loops;
- every jump statement, including bare and valued `return`;
- typedef shadowing, parameter visibility, label/typedef spelling collisions,
  and nested/implicit scopes;
- complex declarators that are functions versus pointers to functions;
- malformed and EOF cases for every owned delimiter;
- continued parsing after a recovered function body;
- 127 nested blocks and 127 parameters.

Assert syntax shape, source vectors, recovery flags, diagnostics, scope cleanup,
and frame/cursor delimiter traces where relevant.

## Suggested commit sequence

1. `test(parser): characterize phase 3 grammar and recovery`
2. `refactor(parser): add statement and function definition syntax storage`
3. `refactor(parser): split declarations from function definitions`
4. `feat(parser): add function and block scope lifetimes`
5. `feat(parser): parse compound statements and block items`
6. `feat(parser): parse structural and jump statements`
7. `feat(parser): parse selection and iteration statements`
8. `feat(parser): preserve deferred expression statement children`
9. `feat(parser): parse prototype and old-style function definitions`
10. `test(parser): harden statement recovery and translation limits`
11. `docs(parser): record phase 3 completion and remaining gaps`

Keep every commit buildable and testable. Keep the deferred-expression seam
isolated so Phase 04 can replace it without redesigning statement frames or
delimiter ownership.

## Out of scope

- implementing the Phase 04 expression parser or Phase 05 initializer parser;
- semantic type checking, control-flow validation, and return-type checking;
- diagnosing all C99 constraints such as illegal `break`, duplicate labels, or
  incompatible old-style parameter declarations unless the existing parser
  already owns that analysis;
- preprocessor expression changes or shared preprocessor/parser machinery;
- compiler extensions such as GNU statement expressions, computed `goto`, or
  declaration-after-label extensions;
- backend lowering or code generation for the new nodes;
- a separate function-declaration syntax node;
- an aggregate translation-unit frame without a demonstrated caller need;
- unrelated workspace Clippy cleanup.

## Done

Phase 03 is complete only when all of the following are true:

- function declarations remain ordinary declarations with no regression;
- prototype-style and old-style function definitions produce real
  function-definition nodes;
- compound statements preserve ordered, interleaved block items;
- every in-scope C99 statement production produces a distinct syntax node;
- expression and initializer positions use explicit typed deferred-child
  results with source provenance and stable diagnostics, not ad hoc token
  skipping;
- function, prototype, block, implicit, and label scope lifetimes are tested;
- `FunctionBodyNotImplemented` is gone from valid function-definition paths;
- recovery terminates, restores scopes, and allows later file items to parse;
- grammar nesting is heap-backed and translation-limit cases are covered;
- `cargo fmt --check` and `cargo test --all-targets` pass;
- strict Clippy is clean for all changed Phase 03 files;
- the CLI syntax dump makes every representative node manually inspectable;
- status and handoff documents identify every deferred expression and
  initializer path assigned to Phases 04 and 05.
