# Phase 04: Parse expressions, type names, and initializers

## Planning status

Created on 24 August 2026 against commit `63a2f6d`, immediately after the
Phase 03 merge. This is an implementation plan, not a claim that Phase 04 is
complete.

Design revision on 24 August 2026: the preprocessor-expression evaluator and
language-expression parser intentionally keep independent precedence-reduction
implementations. Do not extract a shared reducer module. The
`src/translation_phases/` directory contains only C translation phases, not
cross-phase utility modules.

The planning baseline has 137 passing all-target tests and a clean nightly
formatting check. `Parser::next_item` reaches declarations, prototype-style
and old-style function definitions, compound statements, and every structural
C99 statement family. Statement expression positions are explicit typed
future children with parent-owned delimiters. Expression, type-name, and
initializer syntax models exist in partial form, but no `ExpressionFrame`,
`TypeNameFrame`, or `InitializerFrame` is reachable.

## Objective

Implement the complete non-recursive C99 expression grammar together with the
type-name, initializer, and expression-dependent declaration grammar that
depends on it. Use a Double-E-style precedence reducer inside a new
`ExpressionFrame`, integrate `TypeNameFrame` and `InitializerFrame` with the
existing parser driver, and replace every Phase 02 and Phase 03 future child
with a real arena handle.

The phase must preserve the parser's established protocol: one explicit
control stack, owned `ParseAction` values, typed child returns, one delimiter
owner, exact source provenance, current-scope typedef classification, and
recovery that terminates without corrupting the frame, scope, or reducer
stacks.

Expected size: roughly 25-40 focused days for one agent, delivered as small
green commits. The next phase is parser closure, not another grammar-family
handoff.

## Scope

Add or complete:

- parser-local operator/operand stacks, state, precedence, associativity, and
  blocking-marker handling owned directly by `ExpressionFrame`;
- `ExpressionFrame` with `expression`, `assignment-expression`, and
  `constant-expression` entry modes;
- primary, postfix, unary, cast, multiplicative through logical, conditional,
  assignment, and comma expression syntax;
- `TypeNameFrame` for specifier-qualifier lists and optional abstract
  declarators;
- `InitializerFrame` for scalar initializers, brace-enclosed initializer lists,
  designations, and designators;
- typedef-sensitive cast, grouping, `sizeof`, and compound-literal decisions;
- expression, type-name, and initializer arenas, list storage, source
  provenance, recovered nodes, and syntax-store inspection;
- real expression children for all Phase 03 statement positions;
- parsed array bounds, bit-field widths, enumerator values, ordinary
  declaration initializers, and compound-literal initializers;
- expression-specific diagnostics, synchronization, delimiter traces, and
  deep-nesting tests.

Preserve all Phase 03 declaration, function, scope, label, switch, statement,
and recovery behavior. Leave the preprocessor-expression evaluator and its
distinct integer-evaluation diagnostics unchanged. Duplication between the two
expression parsers is acceptable until a future design demonstrates a
substantially deeper shared module. End the phase with no typed future child on
a supported C99 grammar path.

## Required reading and baseline

Before editing, read completely:

- `.agents/AGENTS.md`, `CONTEXT.md`, and `parser-roadmap.md`;
- this plan and the Phase 03 handoff at
  `phase-03-statements-and-function-definitions-plan.md`;
- `phase-01a-conditional-expression-foundations-agent-brief.md`;
- the expression, type-name, initializer, typedef ambiguity, recovery, and
  translation-limit sections of `c99-parser-compliance-checklist.md`;
- `double-e-integration-report.html`, treating its dated findings as research
  and revalidating them against current source;
- `src/translation_phases/parsing.rs`, the preprocessor expression evaluator
  in `src/translation_phases/preprocessing.rs`, and parser inspection in
  `src/lib.rs`.

At the Phase 04 planning baseline:

- `ParseFrame` has no expression or type-name variant, and `ParseValue` cannot
  return either child family;
- `SyntaxStore` already reserves `type_names`, `expressions`, and an
  expression-index list, but those stores are unused, initializer list storage
  is absent, and their ownership model is incomplete;
- `Expression`, `ExpressionType`, `BinaryOperator`, `UnaryOperator`,
  `ExpressionIndex`, `ConstantExpressionIndex`, and `TypeName` are present but
  marked for future construction;
- `TypeName` embeds an optional `Declarator`, while expression nodes embed
  `TypeName` values even though a type-name arena already exists;
- `DeclaratorFrame` already supports `Abstract` mode, parenthesized abstract
  declarators, array/function suffixes, and prototype parameters; its
  expression-dependent array bounds remain a typed future child;
- `ScopeStack` publishes and shadows typedef names at parser-visible token
  positions, so cast-versus-grouping decisions have the required name class;
- Phase 03 uses `ExpressionSlot::{Parsed, FutureChild, Missing}` and the
  constant-expression equivalent; every future-child branch is Phase 04 work;
- declaration, array-bound, bit-field-width, enumerator-value, and initializer
  positions still route through `FutureChildFrame` and stable unsupported
  diagnostics;
- the preprocessor evaluator has unary/binary states and separate operator and
  operand stacks, and its nested conditional regressions are green;
- `cargo test --all-targets` passes 137 tests and
  `cargo +nightly fmt --check` passes at commit `63a2f6d`;
- repository-wide strict Clippy has known debt outside the parser, so final
  attribution must distinguish new Phase 04 diagnostics from baseline debt.

Re-run the baseline commands and record the exact frame inventory, diagnostic
inventory, and syntax-store shape before the first implementation commit.

## Unified phase boundary

`parser-roadmap.md` is authoritative. Phase 04 now owns the mutually recursive
syntax cluster that was previously split across two phases:

- all expression grammar in C99 6.5 through 6.6;
- type names used by casts, `sizeof(type-name)`, and compound literals;
- scalar and brace-enclosed initializers, initializer lists, designations, and
  designators;
- expression children in every Phase 03 statement position;
- array bounds in declarators, bit-field widths, enumerator values, ordinary
  declaration initializers, and compound-literal initializers;
- the language parser's private expression-reduction machinery.

This merger removes the compound-literal handoff. `ExpressionFrame` recognizes
`(type-name) { ... }`, pushes the real `InitializerFrame`, receives an
`InitializerIndex`, constructs the compound-literal expression node, and
resumes postfix parsing after the closing brace. `InitializerFrame` in turn
pushes `ExpressionFrame` in assignment- or constant-expression mode for scalar
initializers and array designators. Both frames must therefore use the parser's
typed child protocol and one recovery design.

Type names may contain array declarators. Their bounds now push the real
assignment-expression entry mode before `TypeNameFrame` can complete. The same
expression implementation connects to struct bit fields and enumerators in
this phase rather than remaining dormant behind future children.

Phase 05 owns the final recovery and C99 parser-closure audit. It may harden
cross-family behavior and close omissions found by the compliance matrix, but
it is not the planned implementation phase for any known expression,
type-name, initializer, or expression-dependent declaration production.

Semantic type resolution, lvalue and modifiable-lvalue constraints, operand
conversions, member lookup, call validation, constant evaluation, initializer
current-object semantics, brace elision, override rules, and full-expression
execution remain later analysis. Syntax must preserve enough grammar form,
ordering, brace boundaries, and provenance for those checks.

## Grammar entry and stopping contract

Use explicit entry modes instead of one permissive expression parser:

| Entry mode | Outermost grammar | Top-level comma | Typical callers |
| --- | --- | --- | --- |
| `Expression` | `expression` | Operator | expression statements, grouped expressions, subscripts, conditional middle operand |
| `AssignmentExpression` | `assignment-expression` | Caller separator | function arguments, scalar initializers, and array bounds |
| `ConstantExpression` | `conditional-expression` | Caller separator | `case`, bit-field, enumerator, and array-designator positions |

Internal reducer states may name narrower grammar classes, but public frame
construction must make these three contracts obvious. A returned
`ConstantExpressionIndex` must be constructible only from the constant-
expression entry mode, while remaining convertible to `ExpressionIndex` for
ordinary arena inspection.

The caller provides the tokens that end an expression at the current delimiter
depth. The child stops before them and returns without consuming them.
Nested grouping, calls, subscripts, and conditional middles temporarily change
which comma, colon, or closing delimiter is active.

| Context | Expression entry | Boundary owner |
| --- | --- | --- |
| Expression or return statement | `Expression` | `StatementFrame` owns `;` |
| `if`, `switch`, `while`, `do`, `for` header | `Expression` | `StatementFrame` owns `)` and header semicolons |
| `case` label | `ConstantExpression` | `StatementFrame` owns label `:` |
| Grouped primary | `Expression` | The expression grouping marker owns `)` |
| Array subscript | `Expression` | The postfix subscript marker owns `]` |
| Function argument | `AssignmentExpression` | The call marker owns separator comma and `)` |
| Conditional middle operand | `Expression` | The nearest unmatched `?` marker owns `:` |
| Cast or `sizeof(type-name)` | `TypeNameFrame` child | `ExpressionFrame` owns both parentheses |
| Scalar initializer | `AssignmentExpression` | `InitializerFrame` leaves caller comma, `}`, or declaration terminator unconsumed |
| Brace initializer / compound literal | `InitializerFrame` | `InitializerFrame` owns both braces and list commas |
| Array designator | `ConstantExpression` | `InitializerFrame` owns `[` and `]` |
| Array declarator bound | `AssignmentExpression` | `DeclaratorFrame` owns `]` |
| Bit-field width | `ConstantExpression` | Struct frame owns member separator or `;` |
| Enumerator value | `ConstantExpression` | Enum frame owns separator comma or `}` |

## Machine and reducer protocol

Extend the existing private frame and typed-return sums rather than adding a
second parser loop:

```rust
enum ParseFrame {
    // Existing Phase 03 frames...
    TypeName(TypeNameFrame),
    Expression(ExpressionFrame),
    Initializer(InitializerFrame),
}

enum ParseValue {
    // Existing Phase 03 values...
    TypeName(TypeNameIndex),
    Expression(ExpressionResult),
    ConstantExpression(ConstantExpressionResult),
    Initializer(InitializerResult),
}
```

The exact result structs may follow current conventions, but they must retain
the arena handle and whether local recovery made the child inexact. A parent
must never infer expression or initializer kind from an untyped handle or
diagnostic.

`ExpressionFrame` owns:

- its entry mode and caller stop set;
- unary/operand-expected versus binary/operator-expected state;
- an operator stack with precedence, associativity, arity, source provenance,
  and blocking markers;
- an operand stack containing expression handles plus the grammar category
  needed to enforce the assignment left-operand production;
- argument-index accumulation and the active grouping/call/subscript/
  conditional marker state;
- child-wait phases for type names and compound-literal initializers;
- recovery that drains or repairs both reducer stacks before reducing a typed
  result.

`InitializerFrame` owns:

- scalar-versus-braced initializer selection;
- both braces of an initializer list and every comma separating its elements;
- iterative nested-list state, preserving source order without Rust recursion;
- optional designation parsing, including a nonempty designator list and its
  required `=`;
- `.` plus member identifier and `[` plus constant-expression plus `]`
  designators;
- assignment-expression children for scalar initializer elements;
- recovery that retains list shape and resumes before its caller's delimiter.

The two expression parsers own independent reduction implementations. Keep the
language parser's operand/operator stacks, state, precedence comparisons,
blocking-marker rules, AST construction, progress invariants, and diagnostics
inside `parsing.rs`. Keep preprocessing tokens, integer values,
short-circuit/evaluated-comma bookkeeping, precedence handling, extension
policy, and diagnostics inside `preprocessing.rs`. Do not add a cross-phase
reducer module under `src/translation_phases/`, and do not make either parser
call the other. Repeated mechanics are preferable to a shallow interface that
mostly exposes the mechanics it claims to encapsulate.

The conditional operator needs two distinct stack concepts: an unmatched
question marker and a completed right-associative conditional operator. A
colon reduces the unrestricted middle expression to the nearest question
marker without crossing a grouping boundary, converts that marker in place,
and leaves the third operand to parse as a conditional expression.

Every `ParseAction` remains owned. A frame returns `Push`, `Reduce`,
`Reprocess`, or `Recover` before the driver mutates the control stack, cursor,
or arenas. User input must not reach `unwrap`, `expect`, `panic`, or
`unreachable` paths unless an internal invariant was already proven
independently of that input.

## Syntax model

Repair the retained future model before wiring the grammar:

- add a typed `TypeNameIndex` and store type names once in the type-name arena;
- have casts, `sizeof(type-name)`, and compound literals refer to that handle
  instead of embedding cloned `TypeName` values;
- make call-argument lists use a stable contiguous arena slice of
  `ExpressionIndex` values;
- add `InitializerIndex` plus arena-backed initializer, initializer-list,
  designation, and designator storage;
- represent a scalar initializer with its `ExpressionIndex`, and a list with a
  stable source-ordered slice of initializer elements whose optional
  designation is distinct from the nested initializer value;
- represent field and array designators distinctly, retaining the member
  identifier or `ConstantExpressionIndex` and each owned delimiter's source;
- add an explicit parenthesized/grouping expression form if grouping is not
  otherwise losslessly inspectable from the tree and source model;
- preserve direct and indirect member names as identifiers independent of
  ordinary typedef classification;
- retain all eleven assignment operators and distinct prefix/postfix increment
  and decrement forms;
- add recovered/error expression representation so malformed input can return
  one typed child without masquerading as valid syntax;
- have compound literals refer directly to a parsed `InitializerIndex`;
- remove `FutureChild` from statement expression slots when their final
  callers are migrated, while retaining `Missing` only for recovery of an
  actually absent required expression;
- replace `Initializer::FutureChild` and every expression-dependent
  declaration placeholder with parsed handles or explicit recovered/error
  nodes;
- keep the outer `Option` on grammatically optional positions such as bare
  `return` and omitted `for` clauses.

Each expression, type-name, initializer, designation, and designator node
retains the union of source vectors for its owned tokens and children. Operator
and delimiter provenance must remain available for a later diagnostic to
identify the exact construct, even when a node spans macro-expanded or
non-contiguous original input.

## Typedef-sensitive decisions

At an opening parenthesis in operand-expected state, classify the following
token using the current `ScopeStack`:

- a type-specifier or type-qualifier keyword starts a type name;
- an identifier starts a type name only when currently classified as a
  typedef name;
- otherwise the parenthesis starts a grouped expression.

After a type-name child returns:

- `)` followed by `{` is a compound-literal base;
- `sizeof (` plus a type name and `)` is `SizeofType`;
- otherwise `(type-name)` followed by a cast-expression is a cast.

The decision uses the classification at that exact token position. Test outer
typedefs, ordinary-identifier shadowing, parameter and block scopes, and
typedef spellings after `.` or `->`. Prefer deterministic classification. If
any implementation path tentatively parses a type name, checkpoint and restore
the cursor, pending return, arena lengths, reducer stacks, diagnostics, and
scope bindings before trying the expression alternative.

## Step 1: Freeze the baseline and expression matrix

Add black-box tests at the `Parser` interface for every current future-child
statement position. Record yielded external items, expression-slot shape,
diagnostics, source vectors, delimiter traces, frame inventory, and
syntax-store contents. Run the existing preprocessor suite as an unchanged
regression baseline; Phase 04 does not modify that parser.

Create an executable matrix for:

- every primary and postfix production;
- every prefix, postfix, binary, assignment, conditional, and comma operator;
- equal-precedence left-associative chains and every right-associative chain;
- all three entry modes and every caller stop token;
- casts, grouping, both `sizeof` forms, type names, and compound literals;
- scalar and brace-enclosed initializers, nested initializer lists,
  designations, and chained designators;
- array bounds, bit-field widths, enumerator values, and declaration
  initializers;
- every statement integration position;
- malformed operands, operators, delimiters, type names, initializers,
  designators, and EOF states.

Classify each row as Phase 04 syntax, Phase 05 closure audit, later semantic
constraint, extension, or out of scope. Use this matrix as the source of every
later test step.

Completion criterion: every C99 expression, type-name, initializer, and
expression-dependent declaration production and recovery edge has a named
test and owner; the 137-test baseline and all existing future-child delimiter
behavior are locked by regression tests.

## Step 2: Repair expression, type-name, initializer, and list storage

Make the syntax model and `SyntaxStore` capable of representing every in-scope
form before parsing it. Add typed handles and accessors, stable argument and
initializer-list storage, grouping/recovery forms, operator/delimiter
provenance, designations/designators, and direct compound-literal initializer
ownership. Define the smallest typed result passed among expression,
type-name, initializer, declarator, declaration, enum, and struct children.

Write focused arena tests for interleaved nested expressions and initializers,
contiguous call-argument and initializer-element slices, type-name reuse,
designator chains, valid versus recovered nodes, and exact source-vector
unions. Ensure a call or initializer list cannot accidentally select unrelated
nodes created while parsing a nested child.

Completion criterion: the model losslessly represents every Phase 04 grammar
form and its provenance; callers inspect stable handles through `SyntaxStore`
without coordinating parallel vectors or frame phases, and no final AST form
requires `FutureChild`.

## Step 3: Implement the language parser's private precedence reducer

Add the language-expression reducer directly to `ExpressionFrame` in
`parsing.rs`. The frame owns its operand/operator stacks, operand-expected and
operator-expected state, precedence and associativity decisions, conditional
markers, source provenance, and progress invariants. Do not introduce a
standalone reducer module and do not change the preprocessor evaluator.

Cover both nested conditional shapes, nested conditional middle operands,
parenthesis boundaries, unmatched `?`/`:`, missing operands, equal-precedence
chains, and comma contexts through the public parser interface. Replace
input-reachable reducer assertions with typed recovery returned to the owning
frame.

Completion criterion: language expressions reduce through parser-local state,
the operator/associativity matrix passes through `Parser::next_item`, and the
unchanged preprocessor test suite remains green.

## Step 4: Implement `TypeNameFrame`

Build `TypeNameFrame` as a resumable composition of
`DeclarationSpecifiersFrame(SpecifierQualifier)` and the existing optional
`DeclaratorFrame(Abstract)`. It accepts exactly a nonempty
specifier-qualifier list plus an optional abstract declarator and returns a
stored `TypeNameIndex` before the caller-owned `)`.

Exercise pointer-only abstract declarators, parenthesized abstract
declarators, omitted direct prefixes, every array/function suffix shape,
prototype parameters, `()`, and nested combinations. Preserve prototype scope
entry and restoration on success, recovery, and EOF. Keep the existing typed
array-bound child contract until Step 12 connects the completed expression
entry mode.

Completion criterion: every expression-independent C99 type name returns a
stable source-backed handle without Rust recursion; expression-dependent array
forms retain one typed within-phase child contract, identifier-bearing
declarators are rejected in type-name context, and every exit restores scope
and delimiter ownership.

## Step 5: Parse primary and postfix expressions

Land the first reachable `ExpressionFrame` vertical slice through expression
statements and `return`. Parse identifiers, integer/floating/character
constants, string literals, and parenthesized full expressions, then implement
repeatable postfix suffixes:

- subscripting with a full expression;
- calls with zero or more assignment-expression arguments;
- `.` and `->` followed by an identifier;
- postfix `++` and `--`.

Keep the frame in operator-expected state after each postfix reduction so
arbitrary chains compose. Member names bypass ordinary typedef classification.
Calls store arguments in source order, and call separator commas never become
comma-expression nodes unless nested grouping changes the active context.

Completion criterion: postfix chains such as `(*fp)(x)`, `a[i].b->c++`, and
`factory()(x)` produce exact source-backed trees; `f(a,b)` and `f((a,b))`
produce distinct argument shapes; no nested expression uses the Rust call
stack.

## Step 6: Parse unary expressions, casts, and `sizeof`

Add prefix `++`/`--`, unary `& * + - ~ !`, `sizeof unary-expression`, casts,
and `sizeof(type-name)`. Preserve the asymmetric grammar:

- prefix increment/decrement consumes a unary expression;
- a unary operator consumes a cast expression;
- a cast consumes another cast expression.

Push `TypeNameFrame` only after current-scope classification selects the
type-name alternative. The expression frame owns the surrounding parentheses
and reprocesses the first token after `)` as either `{` for a compound literal
or the next cast operand/operator.

Completion criterion: `-(T)x`, `++*p`, chained casts, grouped typedef
shadowing, and both `sizeof` forms produce the correct syntax; cast/grouping
selection is correct at every tested scope point and recovers without cursor
or arena leakage.

## Step 7: Complete infix, conditional, assignment, and comma reduction

Add the complete C99 precedence table from multiplicative through logical OR,
then conditional, all eleven assignments, and comma. Encode left and right
associativity explicitly and test every equal-precedence pair.

Track enough operand grammar category to enforce the assignment production's
unary-expression left operand without performing lvalue analysis. Configure
the conditional middle as a full expression, the third operand as a
conditional expression, and assignments as right-associative. A
constant-expression entry stops at conditional-expression grammar even though
later semantic analysis must still reject forbidden operators in unevaluated
subexpressions.

Completion criterion: the operator matrix, nested conditional shapes,
assignment chains, invalid assignment-left grammar, and all comma contexts
produce the expected AST or focused syntax diagnostic with no reducer-stack
leaks.

## Step 8: Implement scalar and brace-enclosed initializers

Implement `InitializerFrame` with two entry paths:

- a scalar initializer pushes `ExpressionFrame(AssignmentExpression)` and
  stops before the caller's comma, closing brace, or declaration terminator;
- a `{` begins an iterative initializer-list machine that owns both braces,
  accepts an optional trailing comma, and pushes nested initializer children
  without Rust recursion.

Store list elements in source order. Keep braces in the syntax even when later
semantic brace-elision rules could interpret the same values differently.
Represent an empty braced list as recovered syntax because the normative C99
initializer-list is nonempty.

Completion criterion: scalar, nested brace, and trailing-comma initializers
produce stable source-backed handles; arbitrary nesting is heap-backed; list
commas and braces have one traceable owner; malformed children retain list
shape and resume at the nearest legal boundary.

## Step 9: Parse designations and designators

Before each initializer-list element, recognize an optional designation:

- one or more `[ constant-expression ]` or `. identifier` designators;
- followed by the required `=`;
- followed by the element's initializer.

Array designators push `ExpressionFrame(ConstantExpression)` and own both
brackets. Field designators consume an identifier in the member namespace
regardless of ordinary typedef classification. Preserve the complete ordered
designator chain rather than precomputing a current subobject.

Completion criterion: field, array, chained, nested, and mixed designations
produce exact syntax; omitted `]`/`=` and malformed member names recover
without consuming the element initializer, list comma, or closing brace.

## Step 10: Build compound literals with real initializer children

After `(type-name)`, delay cast reduction through one token of lookahead. If
the next token is `{`, push `InitializerFrame`, receive its `InitializerIndex`,
and reduce a `CompoundLiteral` operand. Resume the postfix suffix loop so
`(T){1}.member` retains both the compound literal and member access.

Cover nested lists, trailing commas, designations, compound literals inside
parentheses and arguments, following postfix chains, and malformed initializer
recovery. The expression node is valid only when both its type name and
initializer child are exact.

Completion criterion: every valid compound literal has a parsed type-name and
initializer handle, following postfix syntax composes, and no compound-literal
future child or unsupported diagnostic remains.

## Step 11: Replace every Phase 03 statement-expression seam

Replace `FutureChildFrame` with the correct `ExpressionFrame` entry mode at:

- expression statements;
- `if`, `switch`, `while`, and `do`/`while` conditions;
- expression-form `for` initializers, optional `for` conditions, and optional
  iteration expressions;
- optional return expressions;
- `case` constant expressions.

The statement frame continues to own semicolons, closing parentheses, and
label colons. Preserve the outer `Option` for absent optional grammar and the
recovery-only `Missing` state for an absent required child. Remove the
`StatementExpressionNotImplemented` diagnostic from every valid statement
path and delete obsolete future-child/recovery heuristics once no caller uses
them.

Completion criterion: every present valid statement expression contains a
parsed expression handle, every `case` contains a constant-expression handle,
and statement delimiter traces are unchanged except for the typed child frame
name.

## Step 12: Connect expression-dependent declarations and initializers

Replace every remaining `FutureChildFrame` at its owning grammar site:

- array declarator bounds push `AssignmentExpression` and leave `]` to
  `DeclaratorFrame`;
- bit-field widths push `ConstantExpression` and leave the member separator or
  semicolon to `StructOrUnionSpecifierFrame`;
- explicit enumerator values push `ConstantExpression` and leave comma or `}`
  to `EnumSpecifierFrame`;
- each `=` in an init-declarator pushes `InitializerFrame` and leaves the next
  declarator comma or declaration terminator to `DeclarationFrame`.

Preserve publication timing: an ordinary or typedef binding becomes visible
immediately after its declarator, before its initializer and later declarators;
an enumerator becomes visible after its complete defining enumerator. Store
parsed handles in the existing declarator/member/enumerator/declaration syntax
instead of parallel side tables.

Completion criterion: array bounds, bit-field widths, enumerator values, and
file/block/`for` declaration initializers contain parsed handles; their parent
delimiter traces and scope publication timing remain correct; every
`*ExpressionNotImplemented` and `InitializerNotImplemented` diagnostic is gone
from valid paths.

## Step 13: Harden diagnostics, recovery, and limits

Give expression, type-name, and initializer states focused diagnostics for
missing operands, operators, delimiters, member names, argument/list
separators, conditional `:`/third operands, cast operands, type-name
components, designators, designation `=`, initializer elements, and premature
EOF. Recovery must construct a recovered/error node or return a typed missing
child at the smallest legal owner.

Test synchronization before caller semicolons, `for` semicolons, closing
parentheses/brackets/braces, case-label colons, call commas, and EOF. Verify
that inner commas and colons cannot terminate an outer expression and that a
malformed expression does not consume a following statement, declaration,
label, initializer element, designator, or external declaration.

Meet the C99 floor of 63 nested parenthesized expressions and 127 call
arguments, 12 combined pointer/array/function declarator derivations, and deep
nested initializer lists, then stress substantially deeper heap-backed
nesting. Add limit - 1, limit, and limit + 1 tests for any configured resource
ceiling. Every failure must be a stable diagnostic, never a process-stack
overflow, panic, or uncontrolled scan.

Completion criterion: every malformed matrix row terminates, reducer and frame
stacks are balanced, scopes restore to entry depth, caller delimiters remain
unconsumed, and later file items remain parseable whenever a synchronization
point exists.

## Step 14: Verify, document, and hand off

Run the canonical checks from `.agents/AGENTS.md`:

```text
cargo test --all-targets
cargo +nightly fmt --check
cargo clippy --all-targets -- -D warnings
git diff --check
```

If strict Clippy still fails on baseline debt, retain the complete output and
prove that no diagnostic points at a Phase 04 changed file. Do not broaden the
phase into unrelated lint cleanup.

Use the CLI syntax dump to inspect every expression family, type-name form,
initializer/designator form, declaration integration site, typedef-shadowing
case, statement integration site, compound literal, and representative
malformed recovery. Confirm `--tokens` still exposes the tokenizer comparison
path.

Update `CONTEXT.md`, the roadmap/status guidance, the C99 compliance matrix,
and the Phase 05 closure handoff with exact remaining parser, semantic, and
extension gaps. Documentation must state that Phase 04 removed every supported
future-child path.

Completion criterion: formatting and tests pass, changed parser files are
Clippy-clean, manual syntax inspection reaches every Phase 04 node, and
the handoff contains no ambiguous expression/initializer ownership or typed
future-child placeholder on a supported C99 path.

## Test matrix

The executable matrix must include at least:

- identifiers, every constant payload, string literals, and grouping;
- subscripts, empty/nonempty calls, nested call arguments, member access, and
  prefix/postfix increments and decrements;
- every unary, binary, conditional, assignment, and comma operator;
- left-associative chains at every precedence level and right-associative cast,
  conditional, and assignment chains;
- `a ? b ? c : d : e`, `a ? b : c ? d : e`, conditional middles containing
  comma expressions, and grouping boundaries around question markers;
- `f(a,b)`, `f((a,b))`, `f(g(a,b), (c,d))`, and 127 arguments;
- full-expression, assignment-expression, and constant-expression entry modes;
- pointer, parenthesized, array, function, and nested abstract type names;
- cast/grouping and `sizeof` ambiguity under file, function, parameter, block,
  and implicit-scope typedef shadowing;
- `.`/`->` member names whose spelling is a visible typedef;
- scalar initializers, nested initializer braces, trailing commas, and
  source-ordered list elements;
- field, array, chained, nested, and mixed designators, including typedef-named
  members and conditional/comma expressions inside array designators;
- compound literals with parsed initializers and following postfix suffixes;
- array bounds, bit-field widths, enumerator values, and declaration
  initializers in file, block, `for`, struct, enum, parameter, and type-name
  contexts;
- every Phase 03 statement expression position, including absent optional
  `for`/`return` positions and missing required expressions;
- caller-owned `;`, `)`, `]`, `:`, `}`, and context-sensitive comma boundaries;
- malformed and EOF cases for every operand/operator/marker/type-name/
  initializer/designator phase;
- continued parsing after a recovered expression and after a recovered type
  name, initializer, or expression-dependent declaration child;
- 63 parenthesized expressions, 127 arguments, deeply nested initializer lists,
  and substantially deeper architecture stress;
- unchanged preprocessor evaluation, extension-policy, comma-liveness, and
  conditional diagnostics.

Assert syntax shape, expression/initializer/designator arena slices, exact
source vectors, recovered/error state, diagnostics, typedef classification,
publication timing, scope cleanup, reducer-stack cleanup, and frame/cursor
delimiter traces where relevant.

## Suggested commit sequence

1. `test(parser): characterize phase 4 future-child seams`
2. `refactor(parser): complete expression type-name and initializer storage`
3. `feat(parser): add private expression reduction machinery`
4. `feat(parser): parse type names on the control stack`
5. `feat(parser): parse primary and postfix expressions`
6. `feat(parser): parse unary cast and sizeof expressions`
7. `feat(parser): parse binary and conditional expressions`
8. `feat(parser): parse assignment and comma expressions`
9. `feat(parser): parse scalar and brace-enclosed initializers`
10. `feat(parser): parse initializer designations and designators`
11. `feat(parser): build compound literals with parsed initializers`
12. `feat(parser): connect expressions to every statement position`
13. `feat(parser): replace declaration expression and initializer seams`
14. `test(parser): harden phase 4 recovery and translation limits`
15. `docs(parser): record phase 4 grammar completion`

Keep every commit buildable and testable. The private reducer commit must leave
the independently implemented preprocessor evaluator untouched and green.
Land `InitializerFrame` before compound literals or declaration
initializers depend on it, and keep each integration commit reachable through
`Parser::next_item`.

## Out of scope

- semantic name resolution beyond parser-visible typedef classification;
- type checking, conversions, lvalue rules, member lookup, call checking,
  constant evaluation, initializer current-object traversal, brace elision,
  override checking, sequence analysis, or full-expression execution;
- determining compound-literal storage duration or object identity;
- GNU statement expressions, `typeof`, omitted conditional operands, generic
  selections from later C revisions, or other non-C99 extensions;
- any preprocessor-expression parser or language change;
- a separate translation-unit frame or unrelated parser architecture rewrite;
- backend lowering, code generation, or unrelated workspace Clippy cleanup.

## Done

Phase 04 is complete only when all of the following are true:

- one reachable `ExpressionFrame` parses primary through comma expressions
  through the existing `Parser` driver without recursive grammar calls;
- one reachable `InitializerFrame` parses scalar, brace-enclosed, designated,
  and nested initializers without recursive grammar calls;
- the preprocessor and language expression parsers independently own their
  tested precedence-reduction state, values, operators, and diagnostics;
- every C99 operator has source-backed syntax and precedence/associativity
  coverage;
- nested conditional middles and right branches associate correctly;
- call separators and comma operators are distinguished at every nesting
  depth;
- `TypeNameFrame` parses all C99 type names, including expression-dependent
  array forms, and restores prototype scopes on every exit;
- casts, grouping, `sizeof`, and compound literals use current parser-visible
  typedef classification;
- compound literals contain parsed initializer handles and allow following
  postfix syntax;
- every valid present statement expression contains an `ExpressionIndex`, and
  every valid `case` expression contains a `ConstantExpressionIndex`;
- `StatementExpressionNotImplemented` is absent from valid statement paths;
- array bounds, bit-field widths, enumerator values, ordinary declaration
  initializers, and compound-literal initializers contain parsed handles;
- no `FutureChildKind` or unsupported future-child diagnostic remains on a
  supported C99 grammar path;
- malformed expressions, type names, initializers, and designators terminate,
  retain the nearest useful syntax/error node, preserve caller delimiters, and
  allow later file items to parse;
- 63 nested parenthesized expressions and 127 call arguments pass without Rust
  call-stack recursion;
- source vectors, arena slices, recovered flags, scope cleanup, reducer-stack
  cleanup, and delimiter ownership are covered by tests;
- `cargo test --all-targets` and `cargo +nightly fmt --check` pass;
- strict Clippy is clean for all changed Phase 04 files;
- the CLI syntax dump makes every representative expression, type-name,
  initializer, designation, and designator node manually inspectable;
- status and handoff documents assign Phase 05 only closure/audit work and
  enumerate semantic gaps without a typed future-child path.
