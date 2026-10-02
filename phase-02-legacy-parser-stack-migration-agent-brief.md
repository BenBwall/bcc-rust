# Phase 02: Migrate the existing parser onto the stack machine

## Objective

Move the parser's substantial existing declaration code onto the explicit
stack-machine architecture agreed in `GLOSSARY.md` and
`parser-stack-migration-plan.html`.

This phase is a replacement, not a second parser. Port one reachable grammar
path at a time, prove it through the parser interface, then delete the recursive
`parse_*` path it replaces.

Start from merged PR
[#1](https://github.com/BenBwall/bcc-rust/pull/1), merge commit `e022fa5`.
That PR corrected the syntax model but intentionally left parser control flow
unchanged.

This revises the older roadmap split: Phase 02 is no longer a kernel-only gate
followed by a later declaration migration. The kernel is proven by migrating
the existing declaration code immediately, in the vertical increments below.

Expected size: roughly 10–15 focused days for one agent, delivered as small
green commits.

## Scope

Port the implemented-looking parts of `src/translation_phases/parsing.rs`:

- token lookahead and reprocessing;
- external-declaration and ordinary-declaration entry;
- declaration specifiers, including storage classes, qualifiers, function
  specifiers, primitive type combinations, and typedef-name lookup;
- pointer, direct, parenthesized, array, function, abstract, and K&R
  declarators;
- parameter declarations and lists;
- struct, union, and enum specifiers that do not require expression parsing;
- the minimum file-scope name classification needed for a `typedef` declaration
  to affect the next declaration;
- identifier and delimiter handling used by those grammar families;
- their AST arenas, diagnostics, and source provenance.

Expression-dependent alternatives remain later work: array bounds, bit-field
widths, explicit enumerator values, initializers, and function bodies. Preserve
their grammar positions so a later `ExpressionFrame`, `InitializerFrame`, or
`StatementFrame` can be pushed there without redesigning the migrated parent.

The preprocessor expression evaluator remains unchanged. Shared
preprocessor/language-parser machinery is deferred until both parsers are
substantially complete.

## Required reading and baseline

Before editing, read:

- `.agents/AGENTS.md` and `GLOSSARY.md`;
- Phase 02, module layout, testing, risk, and cutover sections of
  `parser-stack-migration-plan.html`;
- the frame protocol and current-parser gap sections of
  `double-e-integration-report.html`;
- declaration, declarator, tag-specifier, diagnostics, delimiter ownership,
  and translation-limit items in `c99-parser-compliance-checklist.md`;
- the declaration audit and completion path in `parser-status-report.html`;
- the current `src/translation_phases/parsing.rs`.

At `e022fa5` on 22 August 2026:

- `cargo check --message-format short` reports 14 parser errors;
- 27 of 30 parser methods have bodies, concentrated in declarations and
  declarators, but none can run through Cargo;
- `pending_token` provides one ad hoc pushback slot;
- `parse_declaration_or_function_definition` ends after the no-declarator case;
- declaration/function-definition arenas and several diagnostic/model pieces
  are missing;
- union nodes are hard-coded as `Struct`;
- enum slices start from the identifier arena instead of the enumerator arena;
- array bounds, bit-field widths, and explicit enumerator values call the
  missing expression parser;
- expressions, statements, initializers, and function definitions are not
  implemented grammar paths.

Capture the exact compiler diagnostics and final `parse_*` method inventory
before editing. The phase is complete only when the migrated parser builds and
the replaced method inventory is gone.

## Machine protocol

Keep `Parser` as the `TranslationPhase<Item = ExternalDeclaration>` interface.
Its implementation becomes the machine driver and owns:

```rust
struct Parser {
    cursor: TokenCursor,
    frames: Vec<ParseFrame>,
    returned: Option<ParseValue>,
    syntax: SyntaxStore,
    scopes: ScopeStack,
    recovery: RecoveryState,
}

enum ParseAction {
    Consume,
    Push(ParseFrame),
    Reduce(ParseValue),
    Reprocess,
    Recover(SynchronizationSet),
}
```

Add only the `ParseFrame` and `ParseValue` variants used by the migrated
grammar. Frame phases stay private to their frame family. A frame returns an
owned action; it never holds a mutable machine borrow while the driver mutates
the cursor, stack, stores, scopes, or diagnostics.

Protocol invariants:

- exactly one frame owns each expected delimiter;
- `Push` leaves the child's first token unconsumed;
- `Reduce` pops one child and returns one typed result;
- `Reprocess` follows a frame-stack or phase-state change;
- a parent accepts only the `ParseValue` variants its phase expects;
- every recovery action names both a synchronization set and a legal unwind
  target;
- EOF completes or diagnoses every active frame;
- grammar nesting grows heap-backed frame storage, not the Rust call stack.

Tests and callers use the `Parser` interface. Frame constructors and internal
stacks remain private.

## Migration map

| Existing path | Stack-machine replacement |
| --- | --- |
| `pending_token`, `next_token`, scattered token restoration | `TokenCursor` |
| `parse_external_declaration` | `ExternalDeclarationFrame` |
| incomplete `parse_declaration_or_function_definition` and `parse_declaration` | `DeclarationFrame` |
| `parse_declaration_specifiers` and four `handle_*` helpers | `DeclarationSpecifiersFrame` with local phases |
| `parse_declarator`, `parse_direct_declarator`, `parse_first_direct_declarator`, `parse_nested_direct_declarator` | `DeclaratorFrame` with pointer/base/suffix phases |
| `parse_pointer_declarator`, `parse_type_qualifiers` | private phases inside `DeclaratorFrame` |
| array, function, and K&R declarator methods | child/suffix phases of `DeclaratorFrame` and `ParameterListFrame` |
| `parse_struct_or_union_declaration` | `StructOrUnionSpecifierFrame` |
| `parse_enum_declaration` | `EnumSpecifierFrame` |
| `parse_identifier`, `parse_maybe_identifier` | token actions inside the owning frame |

Do not create one-method frames that merely rename helpers. The frame seam is
for resumable grammar state, child results, delimiter ownership, or recovery.

## Step 1: Characterize the old implementation

Create token-driven characterization cases from every currently implemented
branch before moving it. Because the crate does not build, use a temporary
validation checkout or harness that repairs only the 14 mechanical compile
blockers long enough to observe behavior. Keep validation-only repairs out of
the implementation diff.

Record:

- AST shape and diagnostics for each supported specifier combination;
- pointer qualifiers at each level;
- identifier, parenthesized, array, function, abstract, and K&R declarators;
- named/anonymous struct, union, and enum forms;
- parameter lists and variadics;
- every EOF and malformed-delimiter path reachable in those methods;
- the union-kind and enum-slice defects as red cases;
- which branches immediately depend on an expression, initializer, statement,
  or scope feature that does not exist.

Completion criterion: every legacy branch is classified as preserve, fix, or
defer, with an executable case or an explicit missing-child dependency.

## Step 2: Land the cursor and driver

Replace `pending_token` with a buffered `TokenCursor`. Add the owned-action
driver, root frame, typed child return, recovery state, and a private frame trace
under `cfg(test)`.

Prove the driver with the first real vertical path:

```c
int;
```

The path is preprocessing → cursor → external-declaration frame → declaration
frame → declaration-specifiers frame → declaration AST → yielded external
declaration.

The declaration-specifiers frame ports storage classes, qualifiers, `inline`,
primitive type combinations, duplicate/conflict diagnostics, and stopping
before the first non-specifier token. Structured/tag specifiers may initially
return a typed "child not migrated" result; they must not fall back to the old
method.

Delete `parse_declaration_specifiers` and its superseded `handle_*` helpers when
the characterization matrix is green.

Completion criterion: specifier-only declarations yield through
`TranslationPhase`, cursor/frame traces identify every token owner, and no
legacy specifier parser remains.

## Step 3: Port ordinary declarators and complete declarations

Add declarator phases for pointer stars, per-level qualifiers, identifiers, and
parenthesized declarators. Parenthesized nesting pushes a child
`DeclaratorFrame`; it never calls the parser recursively.

Complete the minimum ordinary-declaration shell needed to make migrated
declarators reachable:

- one or more comma-separated init-declarators without initializers;
- the terminating semicolon;
- declaration and init-declarator arenas;
- `ExternalDeclaration::Declaration` output.

Publish file-scope typedef and ordinary-name classifications when each
declarator completes. This is the first concrete `ScopeStack` use and must make
`typedef int T; T value;` parse as two declarations. Nested scopes and complete
redeclaration constraints remain later work.

Prove at least:

```c
int x;
int x, y;
const unsigned long *p;
int *const *volatile p;
int (*p);
typedef int T;
T value;
```

Delete the replaced identifier, pointer, qualifier, direct-declarator, and
ordinary-declaration methods after their cases are green.

Completion criterion: named and nested declarators parse end to end, every
comma/parenthesis/semicolon has one owner, and the equivalent recursive methods
are absent.

## Step 4: Port array and function declarators

Add resumable suffix phases for:

- `[]` and `[*]` array forms that need no bound expression;
- qualifier/static array syntax up to the point where an assignment-expression
  child is required;
- empty/K&R identifier lists;
- prototype parameter lists, abstract parameters, and variadics;
- nested suffix chains.

Represent expression-dependent array bounds as an explicit future child phase.
Until `ExpressionFrame` exists, emit one stable "expression parser not yet
implemented" diagnostic and recover at `]`; never call the old expression
`todo!()`.

Prove simple arrays, multidimensional empty arrays, prototypes, abstract
parameters, variadics, K&R lists, and mixed pointer/function/array nesting.
Delete the old array, function, K&R, and parameter-list methods.

Completion criterion: every expression-independent declarator branch in the
legacy inventory runs through `DeclaratorFrame`, and expression-dependent
positions have a typed handoff plus deterministic recovery.

## Step 5: Port struct, union, and enum specifiers

Add `StructOrUnionSpecifierFrame` and `EnumSpecifierFrame` with child
declaration/declarator frames and owned `}`, `,`, `:`, and `;` handling.

During the port:

- derive `StructOrUnion` from the consumed keyword instead of hard-coding
  `Struct`;
- build enum slices from the enumerator arena;
- preserve named, anonymous-body, trailing-enum-comma, and member-list forms;
- route bit-field widths and explicit enumerator values to future
  constant-expression child phases;
- retain source vectors on nodes and diagnostics.

Prove forward/named forms, anonymous bodies, nested tags, unions, enum trailing
commas, member declarations, and malformed/EOF cases. Delete the old struct,
union, enum, and identifier helper methods.

Completion criterion: all expression-independent tag-specifier branches are
frame-driven, both known AST defects are green, and no tag parser calls back
into a recursive grammar method.

## Step 6: Remove the legacy control flow and close the compile gate

Delete the empty `State` enum, expression/statement `todo!()` entry methods with
no migrated callers, incomplete declaration/function-definition control flow,
and any obsolete pushback helpers.

Add only the model/store definitions needed to compile the retained syntax
types. Keep function definitions, expressions, statements, initializers, and
type-name parsing visibly unimplemented through typed future frame phases or
unsupported diagnostics, rather than placeholder control flow.

Search for every pre-migration `parse_*` method named in the migration map and
confirm it is deleted. No migrated frame may call one as a fallback.

Completion criterion: `cargo check` passes, the parser has one control-flow
implementation, and remaining unimplemented grammar is explicit in the frame
inventory rather than hidden behind `todo!()`.

## Step 7: Verify and document

Run:

- the complete characterization matrix through `Parser::next_item`;
- malformed-input and EOF cases for every migrated frame phase;
- `cargo test --all-targets`;
- `cargo +nightly fmt --check`;
- `cargo clippy --all-targets -- -D warnings`;
- `git diff --check`;
- a 63-level parenthesized-declarator test and a substantially deeper frame
  stress test with no Rust call-stack growth;
- the existing preprocessor regression tests to confirm no behavior changed.

Update:

- `GLOSSARY.md` so `ParserMachine`, migrated frames, and actions are current
  partial implementation rather than wholly proposed;
- `README.md` with the closed compile gate and exact declaration subset;
- `parser-stack-migration-plan.html` with Phase 02 evidence;
- `c99-parser-compliance-checklist.md` only for grammar forms proven through the
  parser interface.

Completion criterion: every command and result, migrated/deferred branch, old
method deletion, stress-test result, and remaining limitation appears in the
handoff.

The repository-wide strict-Clippy command is an attribution gate for this
phase, not authorization to change generated bindings, the preprocessor, or
unrelated utilities. Phase 02 must remove parser-local regressions and document
any remaining pre-existing failures; clearing unrelated lint debt belongs in a
separate change so the preprocessor-preservation boundary remains enforceable.

## Commit sequence

1. `test(parse): Characterize legacy declarations`
2. `feat(parse): Add the frame-machine driver`
3. `refactor(parse): Port declaration specifiers`
4. `refactor(parse): Port declarators`
5. `refactor(parse): Port tag specifiers`
6. `refactor(parse): Remove legacy parser control flow`
7. `docs(parse): Record the migrated parser kernel`

Each refactor commit must be green for its migrated matrix and delete the path
it replaces.

## Out of scope

- preprocessor-expression refactoring or shared parser machinery;
- expression, initializer, statement, or function-body implementation;
- array bounds, bit-field widths, or explicit enumerator values beyond their
  typed future-child handoff and recovery;
- initializer-bearing declarations and function definitions;
- nested scope handling, complete redeclaration constraints, or semantic
  analysis beyond the file-scope name classification required above;
- public parser or CLI behavior guarantees beyond the internal
  `TranslationPhase` interface.

## Done

- Existing declaration/specifier/declarator/tag logic runs on the explicit
  frame stack.
- `Parser::next_item` yields the supported declarations end to end.
- Replaced recursive `parse_*` methods and `pending_token` are deleted.
- Nested declarators grow heap-backed frame storage, not the Rust call stack.
- Union and enum arena defects have regressions and are fixed.
- Expression-dependent grammar positions have typed future child phases and
  deterministic diagnostics.
- `cargo check`, tests, and formatting pass; strict-Clippy results are run and
  attributed, with no broad parser lint suppression or unrelated subsystem
  changes used to manufacture a green result.
- Preprocessor behavior is unchanged and no shared parser seam is introduced.
- Documentation states exactly what migrated and what remains.
