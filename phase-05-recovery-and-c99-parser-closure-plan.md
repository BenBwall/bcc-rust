# Phase 05: Recovery and C99 parser closure

## Planning status

Implementation status: complete on 28 August 2026. The baseline measurements
below record the starting point; the completed exit ledger and current project
documents record the delivered state.

This is the implementation plan for the final syntax-parser phase defined by
[`parser-roadmap.md`](./parser-roadmap.md). It was prepared against repository
commit `db50415` on 27 August 2026, after Phase 04 merged, and revised on
28 August 2026 with the accepted C99/GCC/Clang compatibility policy.

The current baseline is healthy but not yet sufficient to claim parser
completion:

- `cargo test --all-targets` passes all 197 tests;
- `cargo +nightly fmt --check` passes;
- `cargo clippy --all-targets -- -D warnings` passes;
- `src/translation_phases/parsing.rs` is 16,563 lines: 10,699 production lines
  and a 5,864-line in-module test section with 171 parser test functions;
- the C99 checklist has 70 checked rows and 51 unchecked rows, but many of the
  unchecked rows are already implemented, belong upstream, or are semantic
  responsibilities. The count is therefore evidence that the inventory is
  stale, not a parser-completion percentage;
- the optional `--all-features` Clippy run is not a canonical gate and currently
  reports three benchmark-only warnings in `src/lib.rs`. Phase 05 must not hide
  that separate repository debt or treat it as parser failure.

The repository's local [`c-spec.pdf`](./c-spec.pdf) is WG14/N1256,
ISO/IEC 9899:TC3. This plan retains the checklist's clause and PDF-page citation
convention.

## Objective

Close the C99 syntax parser as one deep module with a small, usable interface.
Phase 05 must prove that complete translation units:

1. recognize every grammar production in the declared strict-C99 target;
2. retain the grammar facts and source provenance needed by later analysis;
3. diagnose malformed syntax with structured primary/related emission groups;
4. recover at the nearest legal grammar seam without stealing a caller's
   delimiters or leaking parser-visible scope state;
5. meet every parser-relevant C99 minimum translation limit without depending
   on the Rust call stack;
6. expose a stable translation-unit and syntax-tree view that tests, the CLI,
   and the future semantic analyzer can all use; and
7. leave every checklist row classified as parser-supported,
   parser-diagnosed, upstream-owned, semantic-owned, or out of scope, with no
   open row at phase exit.

Parser-complete still does not mean compiler-complete. Phase 05 must not pull
type checking, constant evaluation, object layout, linkage resolution,
initializer current-object rules, control-flow legality, lowering, or code
generation into syntax parsing.

## Compatibility policy

Phase 05 targets ordinary C99 source compatibility comparable to invoking GCC
or Clang with `-std=c99 -pedantic-errors`. The C99 standard is authoritative:
when GCC and Clang disagree about language acceptance, use the repository's
reviewed reading of the standard rather than copying either implementation's
bug or extension. GNU grammar and other vendor extensions remain future,
explicitly selected language profiles.

For this phase, front-end compatibility means:

- conforming C99 syntax is accepted and non-C99 syntax is diagnosed;
- error versus warning classification and warning suppression are deliberate;
- primary locations, related notes/ranges, and continued parsing are useful and
  recognizably compiler-like; and
- version-pinned GCC/Clang observations are recorded as compatibility evidence.

Compatibility does not require exact GCC/Clang wording, diagnostic count or
global order, internal AST shape, compiler bugs, command-line surface, ABI,
object format, assembly, or linking behavior. Full toolchain drop-in
compatibility remains a project-level direction, not a Phase 05 exit gate.

## Design decision

Keep one external parser seam and deepen it.

The caller should be able to request one parsed translation unit and receive:

- the ordered external-declaration roots;
- the arena-backed syntax tree behind typed handles;
- valid versus recovered status at meaningful grammar owners; and
- diagnostics through the existing compilation context.

`Parser`, `ParseFrame`, `ParseAction`, `ParseValue`, the token cursor, scope
state, recovery scanner, frame phases, and arena mutation remain implementation
details. Internal files may introduce private seams for locality and focused
tests, but no trait, port, or adapter should be added merely to make the large
file look modular. These dependencies are in-process and have one
implementation.

The interface is the behavioral test surface. Grammar and recovery tests should
parse complete translation units and inspect them through the same typed
read-only syntax view intended for later compiler phases. A small set of machine
invariant and trace tests may use private internal seams.

## Evidence-backed gaps

### P0 - correctness and completion blockers

| Gap | Current evidence | Required Phase 05 outcome |
| --- | --- | --- |
| The checklist cannot express ownership | 51 rows remain unchecked even though several are visibly implemented, while semantic and upstream rows are mixed with parser work | Replace the binary completion reading with explicit status and evidence for every row |
| Legal repeated specifiers are coupled to parser warnings | `DeclarationSpecifiersFrame` emits `ConstSpecifiedTwice`, `VolatileSpecifiedTwice`, `RestrictSpecifiedTwice`, and `InlineSpecifiedTwice`; N1256 §6.7.3 paragraph 4 and §6.7.4 paragraph 5 say repetition behaves as one occurrence, while GCC/Clang warning defaults differ | Always accept the repetition; if retained as a quality warning, give it a named suppressible warning group that can never become a strict-C99 syntax error merely because pedantic mode is enabled |
| Absence of storage class is lost | `DeclarationSpecifiers::new` stores `StorageClass::Auto` before seeing a token | Represent absence explicitly, normally as `Option<StorageClass>`, so later constraints can distinguish `int x` from `auto int x` |
| Identifier provenance is lost | `Identifier` stores only `StringCacheId`; declarations, labels, tags, members, enumerators, and typedef names cannot identify the exact identifier token without using a larger owner's source | Store exact `SourceVectors` on identifier syntax and preserve it across macro/include expansion |
| Invalid `_Imaginary` syntax looks like a valid type | `_Imaginary` is a keyword, correctly diagnosed, but `TypeSpecifiers` still contains normalized imaginary type variants and its grammar comment lists `_Imaginary` as a type specifier | Keep keyword recognition, reject it as strict-C99 type syntax, and represent any retained recovery as invalid syntax rather than a valid type |
| The phase-7 token handoff is not exhaustively closed | `Preprocessor::map_preprocessor_token` ends in `todo!("{x:#?}")` for token categories assumed to be unreachable | Handle every preprocessing-token variant explicitly; impossible handoffs become an invariant diagnostic or are filtered before mapping, never a user-triggerable panic |
| A meaningful translation-unit result is missing | `Parser::next_item` yields handles one at a time, while the only syntax access is `syntax_debug()` returning a raw `Debug` view of a private store | Add a typed `ParsedTranslationUnit`/`SyntaxTree` interface and aggregate roots without adding a second parser implementation |
| Pure garbage is shaped as a declaration | `ExternalDeclaration::Error` is reserved and unused; even a top-level `}` becomes a recovered declaration with empty specifiers | Use a provenance-only external error node when no meaningful declaration or function syntax survives; retain recovered declarations only when a useful tree exists |
| Diagnostics omit required structure | `ParserError` contains an error enum and source vectors, while active frame and expected syntax live only in message text or are absent | Carry a symbolic code, severity, optional warning group, active `ParseFrameKind`, found token/EOF, typed expectation, primary provenance, related notes/ranges, and recovery information as data |
| Iterator diagnostics can be reversed | `Context` stores pending diagnostics in a `Vec` and `pop_pending_error` removes from the end | Preserve primary-plus-related emission grouping through FIFO delivery; do not globally sort diagnostics by source location |
| Capacity failures can panic | Many arena insertions call `to_u32()` or `try_into().expect(...)`; frame and list vectors rely on infallible growth | Centralize checked arena/list insertion and resource accounting; report a stable resource diagnostic and terminate or recover cleanly |

### P1 - recovery, test, and maintainability gaps

| Gap | Current evidence | Required Phase 05 outcome |
| --- | --- | --- |
| Cross-family recovery is hard to audit | `Parser::recover`, `SynchronizationKind`, expression boundaries, and initializer recovery combine table-like rules with grammar-specific heuristics in one large module | Make recovery policy explicit per owner, preserve the few parser-aware decisions as named hooks, and test the parent/child boundary matrix |
| EOF coverage is sampled rather than exhaustive | Existing EOF tests cover many high-risk paths, but the checklist still calls out every EOF-capable state and there is no state inventory tied to tests | Give every frame phase an EOF disposition and a named test or generated matrix row |
| Nonprogress is asserted informally | The action protocol is designed for progress, but malformed-input safety depends on hand-reviewed phase transitions | Add a test-only action budget/progress oracle and property tests over malformed token-shaped input |
| C99 minimum limits are only partly proven | Blocks, parenthesized declarations/expressions, parameters, and arguments are tested; combined derived declarators, large identifier sets, cases, members, enumerators, and nested tags are not | Add floor, floor-minus-one, floor-plus-one, and deeper stress cases as appropriate |
| Tests mostly inspect private vectors | The 5,864-line test module indexes `parser.syntax` directly | Migrate behavioral assertions to the typed syntax interface and keep only protocol tests private |
| The parser implementation has poor locality | Syntax, machine, recovery, 13 frame families, diagnostics, inspection, and tests occupy one 683 KB source file | Improve locality behind the same interface; use the suggested private-module split only where the implementation evidence supports it |
| No generative malformed-input defense exists | `proptest` is a dev dependency but the parser does not use it | Generate bounded token-shaped source and mutation cases; assert termination, no panic, diagnostic provenance, and machine cleanup |
| No versioned compiler-compatibility corpus exists | GCC and Clang behavior has been used as informal context, but no normalized dialect/warning/error-limit profile or reviewed discrepancy manifest is checked in | Add version-pinned differential fixtures as evidence; classify disagreements against C99 and keep repository-reviewed expectations authoritative |
| Inspection output is implementation noise | The CLI prints numeric roots, a full arena `Debug` dump, interned strings, and a raw source-vector table | Provide an explicit deterministic source-oriented inspection mode with readable identifiers and recovered/error markers; keep both tree and raw arena inspection opt-in |

### P2 - classify, retain, and hand off; do not implement as parser semantics

The following checklist areas need explicit handoff evidence, not syntax-parser
constraint solving:

- identifier binding other than the ordinary-name classification required for
  typedef ambiguity;
- tag redeclaration, member lookup, and the separate semantic namespaces;
- duplicate or undefined labels, `goto` targets, VLA-scope jumps, and
  `case`/`default`/`break`/`continue` placement;
- function type, parameter adjustment, linkage, and return constraints;
- lvalue, operand type, assignment compatibility, call, and sequence rules;
- constant-expression evaluation and its unevaluated-subexpression exception;
- initializer target type, current-object traversal, overridden elements, and
  excess initializers.

For these rows, Phase 05 must prove that the AST retains the required syntax and
provenance and then mark the checklist owner as semantic analysis. Do not emit a
parser error for constraint-invalid but grammatically valid input unless the
parser already intentionally performs that constraint check, such as duplicate
`default` within the nearest switch. Treat those existing early checks as
provisional compatibility behavior: preserve them during Phase 05, do not use
them to justify expanding semantic analysis inside the parser, and revisit
their ownership when the semantic phase exists.

## Suggested module layout

Keep `src/translation_phases/parsing.rs` as the parent module so the repository's
`mod_module_files` lint remains satisfied. Move implementation details under an
adjacent `parsing/` directory where that improves locality. The following tree
is a planning aid, not an exit contract; implementation work may combine,
rename, or omit files when doing so keeps related state and behavior more local:

```text
src/translation_phases/
├── parsing.rs                       # module docs, narrow re-exports, Parser seam
└── parsing/
    ├── syntax.rs                    # handles, nodes, SyntaxTree, checked arenas
    ├── machine.rs                   # cursor, driver, ParseAction, ParseValue
    ├── scopes.rs                    # ordinary-name and parser-local control scopes
    ├── recovery.rs                  # policies, delimiter state, recovery trace
    ├── diagnostics.rs               # kinds, expectations, codes, rendering data
    ├── inspection.rs                # deterministic tree formatter
    ├── frames.rs                    # frame sum type and dispatch
    ├── frames/
    │   ├── declarations.rs
    │   ├── declarators.rs
    │   ├── parameters.rs
    │   ├── tags.rs
    │   ├── functions.rs
    │   ├── statements.rs
    │   ├── expressions.rs
    │   ├── type_names.rs
    │   └── initializers.rs
    ├── tests.rs                     # shared complete-TU harness
    └── tests/
        ├── grammar.rs
        ├── ambiguity.rs
        ├── recovery.rs
        ├── eof.rs
        ├── limits.rs
        ├── properties.rs
        └── inspection.rs
```

Any split remains an internal locality refactor, not a collection of new
caller-facing modules. Use `pub(super)` only where sibling implementation files
genuinely need access. Frame-specific phase enums and partial results stay next
to their frame implementation. Judge the result by a narrow external interface,
reviewable files, and unchanged behavior—not conformance to this exact tree.

## Target parser interface

The exact Rust spelling may change during implementation, but the seam should
have this shape:

```rust
pub(crate) struct ParsedTranslationUnit {
    roots:  Box<[ExternalDeclaration]>,
    syntax: SyntaxTree,
}

impl Parser {
    pub(crate) fn parse_translation_unit(
        self,
        context: &mut Context,
    ) -> ParsedTranslationUnit;
}

impl ParsedTranslationUnit {
    pub(crate) fn external_declarations(&self) -> &[ExternalDeclaration];
    pub(crate) fn syntax(&self) -> &SyntaxTree;
}
```

The existing `TranslationPhase::next_item` implementation may remain as a thin
streaming adapter while callers migrate. It must drive the same machine and own
no parsing behavior.

`SyntaxTree` should:

- own all syntax arenas;
- validate typed handles and slices at construction;
- provide typed read-only lookup and iteration without exposing raw vectors;
- return slices for list handles without cloning;
- retain exact root order and valid/recovered/error status;
- make absence distinct from synthesized recovery; and
- be the sole syntax inspection surface for tests and the later semantic phase.

The deletion test applies: deleting `SyntaxTree` should force handle validation,
arena access, list slicing, and formatting logic back into every caller. If it
only forwards raw vector access, it is too shallow.

## Structured diagnostic contract

Refactor parser diagnostics toward data that can be rendered by the CLI or a
future IDE without reparsing English messages:

```rust
pub(crate) struct ParserDiagnostic {
    code:          ParserDiagnosticCode,
    severity:      ErrorSeverity,
    warning_group: Option<ParserWarningGroup>,
    frame:         ParseFrameKind,
    kind:          ParserDiagnosticKind,
    found:         Option<TokenType>,
    expected:      ExpectedSyntax,
    primary:       SourceVectors,
    ranges:        Box<[SourceVectors]>,
    related:       Box<[RelatedParserDiagnostic]>,
    recovery:      Option<RecoverySummary>,
}
```

Requirements:

- `code()` and `severity()` are exhaustive symbolic data whose changes receive
  explicit review; they do not imitate GCC/Clang internal identifiers;
- optional quality warnings have a named suppressible group, and warning policy
  remains separate from whether the source is valid C99;
- `frame` is set by the driver before calling the active frame;
- `found: None` means EOF;
- `ExpectedSyntax` is a compact enum/set, not a free-form sentence;
- the primary source is the offending token or a zero-width EOF position;
- ranges, notes, and recovered/discarded provenance are attached separately
  when useful and are not substituted for the primary location;
- formatting remains human-readable and includes the production label; and
- diagnostics leave `Context` in FIFO emission groups, with each primary
  immediately followed by its related notes. Do not globally sort by source;
  macro/include provenance and explanatory notes can legitimately point to an
  earlier location.

Compatibility assertions cover classification, warning control, primary
location, related context, and useful continuation. They do not require exact
GCC/Clang prose, diagnostic multiplicity, global ordering, or identifiers.

Internal invariant failures should be distinguishable from source diagnostics.
The malformed-input test suite must demonstrate that user input cannot reach an
invariant panic. Capacity or configured-resource failures are normal diagnosed
outcomes, not invariants. Process-wide allocation failure is not a recoverability
promise; only failures the implementation can detect and report without further
allocation belong to this contract.

## Recovery contract

Every frame family must document and test:

1. which delimiters it owns;
2. which child results it accepts;
3. which tokens end the production without being consumed;
4. its EOF result;
5. the synchronization policy used after malformed input;
6. the scope, label, switch, and temporary reducer state restored on unwind;
7. the syntax or error node returned after recovery; and
8. the action that proves progress before lookahead is reprocessed.

The shared recovery scanner should remain a deep internal module. It owns
delimiter depth, conditional-question pairing, consumed provenance, and policy
evaluation. Typedef-sensitive declaration starts, identifier labels, and the
few statement-body heuristics stay as explicit named parser callbacks or
precomputed facts; they must not become a generic trait surface.

Build a recovery matrix with rows for parent owner and columns for malformed
child families. At minimum cover:

- declaration -> declarator, array bound, parameter list, tag, enum value, and
  initializer;
- function -> old-style declaration list and compound body;
- compound -> block declaration and every statement family;
- statement -> header expression, child statement, label expression, and
  `for` declaration;
- expression -> grouping, subscript, call argument, conditional operand, cast,
  type name, and compound literal;
- initializer -> scalar child, nested list, designation, and array designator;
- every row at its separator, owning closer, enclosing closer, a following
  declaration/statement starter, and EOF.

After every recovered translation unit, assert that frames, pending child
values, active recovery, nested ordinary-name scopes, label scopes, switch
scopes, and reducer scratch state are empty or at their documented terminal
state.

## C99 compliance status model

Revise `c99-parser-compliance-checklist.md` so every row records one of:

- **Parser-supported** - recognized and linked to a positive syntax test;
- **Parser-diagnosed** - intentionally invalid syntax with a stable diagnostic
  and recovery test;
- **Upstream-owned** - completed during initial processing, preprocessing-token
  recognition, macro preprocessing, or phase-7 token conversion;
- **Semantic-owned** - syntax retained for a named later constraint/semantic
  pass;
- **Out of scope** - an explicitly rejected extension or non-v0 feature; or
- **Open** - concrete Phase 05 work still lacking evidence.

Each supported or diagnosed row should name at least one test or fixture group.
Each upstream or semantic row should name the owning module/pass and the syntax
data handed off. No row may remain ambiguous at phase exit.

## Implementation sequence

### Step 1: Freeze the baseline and rebuild the compliance inventory

Actions:

- record commit, toolchain, canonical command results, parser test count, and
  existing known lint debt;
- add the explicit status vocabulary above to the checklist;
- classify all 121 current checklist rows without changing behavior;
- link existing tests to rows already implemented;
- record the strict-C99 compatibility profile, normalized GCC/Clang flags, and
  initially observed compiler versions;
- create a fixture manifest for genuinely open grammar, recovery, provenance,
  inspection, compatibility, and limit cases; and
- correct stale architecture names such as a nonexistent required
  `TranslationUnitFrame` when sequencing through the parser seam already
  satisfies the grammar.

Exit:

- there is a finite, reviewable list of Phase 05 parser work;
- semantic and upstream responsibilities are not counted as parser defects; and
- no row is marked supported solely because an enum variant exists.

### Step 2: Add the complete-translation-unit and typed syntax seam

Actions:

- add `ParsedTranslationUnit` and a read-only `SyntaxTree`;
- aggregate external roots in source order;
- provide typed handle and typed list lookup with checked construction;
- retain `next_item` only as a thin adapter over the same machine;
- migrate the CLI and a small characterization-test set to the new seam; and
- keep raw `Debug` inspection temporarily behind a private or explicit debug
  path.

Exit:

- one complete translation unit can be consumed without reaching into
  `Parser` fields;
- invalid handles cannot be manufactured by callers; and
- tests and future analysis share the same interface.

### Step 3: Improve parser locality behind the existing seam

Actions:

- use the suggested module layout as a starting hypothesis, then split or retain
  files according to demonstrated locality without changing syntax,
  diagnostics, or recovery behavior;
- keep state enums with their frame families;
- keep the driver and recovery scanner independent of concrete AST formatting;
- avoid new traits for single implementations; and
- run the full baseline after each family moves.

Exit:

- the public/crate-visible parser interface has not grown except for the typed
  translation-unit/syntax seam;
- each grammar family and its state transitions can be read and tested with
  reasonable locality, regardless of the exact file map; and
- the characterization output is unchanged.

### Step 4: Repair syntax facts and source provenance

Actions:

- change declaration storage class to explicit absence or presence;
- attach exact source vectors to `Identifier` and update ordinary-name lookup to
  key on its interned spelling only;
- audit identifiers in declarators, typedef names, tags, enumerators,
  parameters, labels, `goto`, and member access;
- remove valid-looking imaginary type combinations from strict-C99 syntax;
- distinguish absent, missing/synthesized, recovered, and invalid syntax in
  node fields; and
- audit operator/delimiter provenance for every composite syntax form.

Exit:

- `int x` and `auto int x` have distinct syntax;
- every identifier token can be diagnosed at its own provenance;
- `_Imaginary` never produces a valid C99 type node; and
- a traversal can explain which syntax was present and which was synthesized.

### Step 5: Close the phase-7 input and strict-C99 compatibility contract

Actions:

- make preprocessing-token to parser-token conversion exhaustive and remove
  the production `todo!`;
- test all C99 keywords, constants, strings, characters, punctuators, and
  digraph spellings through the complete pipeline;
- ensure `#`, `##`, header names, placemarkers, whitespace, and preprocessing
  numbers are consumed or diagnosed by the proper upstream owner;
- run the closure grammar suite under the declared C99 profile comparable to
  `-std=c99 -pedantic-errors`, with `ExtensionPolicy::Deny`;
- reject vendor grammar without accidentally rejecting conforming syntax;
- accept repeated type qualifiers and repeated `inline`; either emit no quality
  warning or route one through a named suppressible group, but never classify
  the repetition as a syntax or pedantic error;
- confirm the §6.7.5.3 paragraph 11 typedef preference with direct examples;
- audit tag-only declarations, empty specifier lists, all array suffix forms,
  prototypes, old-style definitions, initializer forms, and statement forms;
  and
- record every discovered grammar fix as a focused regression.

Exit:

- no valid C99 production is rejected or classified as invalid by the parser
  fixture suite;
- excluded extensions cannot enter the C99 AST as valid syntax;
- every invalid phase-7 token path terminates with a diagnostic rather than a
  panic; and
- the checklist's syntax rows have direct evidence.

### Step 6: Make diagnostics structured and emission grouped

Actions:

- separate diagnostic kind from found token, expected syntax, frame, and
  provenance;
- assign symbolic parser diagnostic codes and named suppressible warning groups;
- have the driver expose the active frame kind during each step;
- convert message-only expectations into `ExpectedSyntax` values;
- preserve zero-width EOF anchors;
- represent useful ranges, recovery data, and related notes separately from the
  primary location;
- make pending primary-plus-related delivery FIFO without regressing
  preprocessor lookahead ordering or globally sorting by source; and
- add CLI rendering and structured-field tests.

Exit:

- every syntax diagnostic identifies nature, expected syntax, found token or
  EOF, active production, and source;
- primary diagnostics and their related notes retain emission grouping;
- warnings can be controlled without changing C99 acceptance; and
- rendering does not need to parse debug strings.

### Step 7: Harden cross-family recovery and root error handling

Actions:

- inventory delimiter ownership and EOF behavior for every phase enum;
- table-drive the grammar-independent parts of synchronization policy;
- name and isolate typedef/label/body-specific recovery decisions;
- implement `ExternalDeclaration::Error` for unrecoverable top-level garbage;
- retain recovered declarations/functions only when their trees remain useful;
- add the parent/child recovery matrix;
- assert all parser-local state is restored after each item and at EOF; and
- add a test-only progress/action budget to catch cycles.

Exit:

- every malformed matrix case terminates, diagnoses, and preserves the nearest
  following legal construct;
- no child consumes a caller-owned separator or closer;
- pure garbage is not presented as a meaningful declaration; and
- no scope, label, switch, reducer, or recovery state leaks across roots.

### Step 8: Build grammar, compatibility, ambiguity, and handoff evidence

Actions:

- create source fixtures for every meaningful normative alternative in
  declarations, declarators, type names, initializers, statements, expressions,
  and function definitions;
- snapshot a concise stable syntax-tree rendering rather than raw arena debug;
- add typedef ambiguity cases at every scope transition and declarator timing
  point;
- add constraint-invalid but grammatical fixtures that must retain complete
  syntax for semantic analysis;
- add strict rejection fixtures for the explicitly excluded extensions;
- run version-pinned GCC and Clang over normalized differential fixtures,
  recording dialect, pedantic, warning, target, and error-limit flags; and
- classify every disagreement against C99 in a checked-in manifest. Compiler
  output is evidence; the reviewed repository classification is authoritative.

Exit:

- the grammar checklist is backed by complete-tree assertions for each
  meaningful alternative and interaction;
- the compatibility corpus is reproducible and every observed difference is
  classified as agreement, standard-required divergence, extension, compiler
  quirk, or intentional diagnostic improvement;
- syntax-valid/constraint-invalid input is not mislabeled as unknown syntax;
  and
- formatting or arena layout changes do not rewrite unrelated goldens.

### Step 9: Exhaust EOF and malformed-input states with generated tests

Actions:

- map each frame phase to its possible EOF disposition;
- generate truncations of representative valid fixtures at token boundaries;
- use `proptest` to generate bounded token-shaped inputs and mutations such as
  deletion, duplication, delimiter replacement, and separator insertion;
- assert no panic, termination within the test action budget, at least one
  source-backed diagnostic for malformed syntax, and clean terminal state;
- persist every discovered minimal failure as a normal regression; and
- add minimized recovery cases to the compatibility corpus when GCC/Clang
  behavior materially informs continuation, without requiring identical prose,
  diagnostic count, or global order.

Exit:

- every EOF-capable state is covered;
- the generative suite is deterministic under a recorded seed on failure; and
- malformed input cannot reach parser invariant panics in the tested domain.

### Step 10: Meet translation floors and make resource failure explicit

Actions:

- centralize syntax arena and typed-list insertion behind checked methods;
- replace input-scale `to_u32()`/`expect` paths with checked errors;
- use checked representational limits, `try_reserve`, or configured ceilings
  where failure can be detected and reported without further allocation;
- add test-injectable limits above all normative C99 floors;
- test 12 mixed pointer/array/function derivations;
- test 511 block identifiers and 4,095 external identifiers;
- test 1,023 `case` labels in one switch;
- test 1,023 members and 1,023 enumerators;
- test 63 nested struct/union definitions;
- retain the existing block, parenthesis, parameter, argument, expression, and
  initializer nesting floors;
- for each configured ceiling, prove ceiling-minus-one and ceiling succeed and
  ceiling-plus-one produces one stable resource diagnostic; and
- measure runtime, peak arena sizes, and source-vector growth for the large
  fixtures to catch accidental superlinear behavior.

Exit:

- every parser-relevant §5.2.4.1 floor passes without Rust stack recursion;
- configured or representational over-limit input produces a stable diagnostic
  rather than an input-scale conversion panic;
- the phase makes no promise to recover from process-wide allocator failure; and
- large-scope lookup and provenance growth remain acceptably close to linear.

### Step 11: Add an opt-in deterministic tree inspection view

Actions:

- render identifiers by spelling rather than cache IDs;
- display declaration/function roots in source order;
- recursively render through typed handles with indentation, concise node
  labels, operators, and child roles;
- mark recovered, missing, invalid, and error syntax visibly;
- make source locations optional but readable;
- expose the readable tree through an explicit inspection option and keep raw
  arenas/source vectors behind a separate debug option; and
- add stable inspection goldens for valid, macro-originated, and recovered
  translation units.

Exit:

- normal CLI behavior remains compiler-like and does not print an AST unless
  inspection is requested;
- the opt-in tree explains parsed syntax rather than storage implementation;
- inspection uses `SyntaxTree`, not private arena fields; and
- a user can locate recovered syntax and its diagnostic without correlating
  numeric dumps manually.

### Step 12: Close documentation and verification

Actions:

- update `c99-parser-compliance-checklist.md` with final status and evidence;
- mark Phase 05 complete in `parser-roadmap.md` only after every exit gate is
  green;
- update `CONTEXT.md` for durable interface or recovery vocabulary changes;
- update `README.md` with the final parser/CLI status and remove stale
  “incomplete parser” wording only where justified;
- document the C99 compatibility profile, tie-break rule, differential corpus,
  and deliberately excluded notions of exact compiler compatibility;
- document remaining semantic-analysis work separately; and
- run and record the canonical checks plus `git diff --check`.

Exit:

- no checklist row is unclassified;
- documentation matches current code and commands;
- all Phase 05 done criteria below pass; and
- the parser can be called complete for the repository's declared syntax
  target without implying compiler completion.

## Test matrix

| Suite | Positive evidence | Negative/recovery evidence | Required assertions |
| --- | --- | --- | --- |
| Compatibility | version-pinned GCC/Clang runs with normalized C99, pedantic, warning, target, and error-limit flags | compiler disagreements, extensions, and intentional bcc diagnostic improvements | C99-based reviewed classification; compiler output is evidence rather than an unreviewed oracle |
| Phase-7 handoff | all keywords, literal kinds, punctuators, digraphs, macro/include provenance | impossible/unconvertible token categories | no `todo!`, correct owner diagnostic, exact source |
| Declarations | all specifier orders, tag-only, multi-declarator, typedef, repeated inline, repeated qualifiers | empty specifiers, missing type/declarator/semicolon, invalid `_Imaginary` | AST shape, explicit storage absence, name publication timing, optional warning control independent of acceptance |
| Declarators and type names | pointer/array/function combinations, prototypes, K&R, abstract and parenthesized forms | missing delimiters, malformed parameters/bounds, EOF per phase | typed children, caller delimiter ownership, typedef preference |
| Tags | named/reference/definition struct/union/enum, bit-fields, trailing enum comma | empty bodies, malformed members/enumerators, nested recovery | kind, list order, identifier provenance, nearest boundary |
| Expressions | all operators and precedence, calls, members, casts, `sizeof`, compound literals | missing operands/operators/closers, malformed child type/initializer | exact tree, operator provenance, recovered propagation |
| Initializers | scalar, nested lists, trailing comma, mixed/chained designators | missing `]`, `=`, element, comma, `}`, EOF | element order, punctuation provenance, caller boundary |
| Statements/functions | every family, dangling `else`, mixed blocks, both `for` forms, old-style definitions | malformed headers/bodies/labels/jumps, missing function body | source order, scope restoration, following-item preservation |
| Translation unit | mixed declarations and definitions, macros/includes | empty unit, garbage root, malformed item followed by valid item | ordered roots, valid/recovered/error classification |
| Semantic handoff | undeclared identifier, invalid lvalue/type use, jump/switch constraints, initializer constraints | n/a at parser layer | complete syntax retained; owner marked semantic |
| EOF | truncation at every frame phase | every truncation is the test | diagnostic, zero-width EOF source, termination, clean state |
| Property | generated bounded token-shaped and mutated valid input | arbitrary malformed combinations | no panic, action budget, provenance, deterministic seed |
| Limits | every C99 floor and deeper architecture stress | configured or representational ceiling plus one | no call-stack overflow, stable catchable-resource diagnostic, growth metrics; no process-wide OOM promise |
| Inspection | opt-in readable valid/macro/recovered trees | recovered/error markers | deterministic output independent of arena debug layout; ordinary CLI output remains compiler-like |

## Performance and resource expectations

Phase 05 is not a parser optimization project, but closure tests must prevent
obvious scaling defects:

- normal driver and recovery work should remain amortized linear in tokens;
- ordinary-name lookup should remain approximately constant time per lookup;
- frame, operator, operand, and list storage should scale with active grammar
  depth or retained syntax, not Rust call-stack depth;
- inspection must be linear in reachable syntax and must detect impossible
  handle cycles defensively;
- source-provenance merging must be measured on deeply nested and
  macro-fragmented syntax. If flat vector copying becomes superlinear, replace
  it with a shared/interned composition representation behind the existing
  provenance interface rather than weakening provenance; and
- diagnostic and recovery limits must prevent one malformed region from
  producing unbounded repeated messages; and
- catchable resource failures should diagnose without panic, while
  process-wide allocator failure remains outside the parser's recovery
  guarantee.

## File impact

| File or area | Planned responsibility |
| --- | --- |
| `src/translation_phases/parsing.rs` | Narrow parser module seam; retain or re-export implementation chosen by the locality refactor |
| `src/translation_phases/parsing/*.rs` *(if split)* | Candidate homes for syntax, machine, scopes, recovery, diagnostics, inspection, frame dispatch, and test harness |
| `src/translation_phases/parsing/frames/*.rs` *(if split)* | Candidate locality for frame phase enums and implementations by grammar family |
| `src/translation_phases/parsing/tests/*.rs` *(if split)* | Candidate locality for grammar, compatibility, ambiguity, recovery, EOF, limits, properties, and inspection suites |
| `src/translation_phases/preprocessing.rs` | Exhaustive phase-7 token conversion and handoff diagnostics |
| `src/translation_phases.rs` | FIFO primary/related diagnostic grouping and any shared provenance/resource support |
| `src/lib.rs` | Complete-translation-unit CLI use and opt-in readable inspection flags/output |
| `tests/fixtures/parser/` | Stable C99 source fixtures, versioned compatibility cases/manifest, and concise expected tree output where file-backed fixtures improve readability |
| `c99-parser-compliance-checklist.md` | Explicit ownership/status/evidence inventory |
| `CONTEXT.md` | Durable terms for `SyntaxTree`, parsed translation unit, or revised recovery concepts |
| `parser-roadmap.md` | Phase status after all closure gates pass |
| `README.md` | Current parser and inspection behavior after implementation |

Do not edit the historical root HTML reports to make their dated findings look
current. They remain research notes.

## Suggested commit sequence

1. `docs(parser): classify phase 5 compliance inventory`
2. `feat(parser): expose parsed translation unit syntax`
3. `refactor(parser): improve parser implementation locality`
4. `fix(parser): preserve storage absence and identifier provenance`
5. `fix(parser): close strict c99 token and specifier handling`
6. `feat(parser): structure parser diagnostics`
7. `fix(parser): harden cross-family recovery and root errors`
8. `test(parser): add c99 grammar and compatibility evidence`
9. `test(parser): exhaust eof and malformed input states`
10. `feat(parser): diagnose parser resource limits`
11. `feat(cli): add opt-in parser syntax inspection`
12. `docs(parser): close phase 5 parser roadmap`

Keep behavior-preserving moves separate from grammar or recovery changes. Any
module split commit should be reviewable as a move; a recovery commit should
show the regression that required it. Do not create files merely to satisfy the
suggested layout.

## Risks and controls

| Risk | Level | Control | Proof point |
| --- | --- | --- | --- |
| Semantic work leaks into parser closure | High | Explicit checklist owner and handoff fields | constraint-invalid grammar reaches a complete AST without parser rejection |
| Locality refactor hides behavior changes or creates shallow files | High | Characterization trees before moves; evidence-driven family-at-a-time commits | identical behavior with a narrow seam and demonstrably improved locality |
| Recovery steals enclosing punctuation | High | Parent/child matrix and ownership trace assertions | following declaration/statement survives each malformed child |
| A “clean” interface becomes a raw arena pass-through | High | Typed handles, checked slices, no exposed vectors | behavioral tests and CLI need no parser-private access |
| Provenance fixes cause memory blow-up | High | fragmented-source stress metrics and shared representation fallback | bounded source-store growth for nested macro syntax |
| Resource limits reject conforming minimums | High | defaults above every C99 floor and direct floor tests | all §5.2.4.1 parser floors succeed |
| Property tests become slow or flaky | Medium | bounded strategies, deterministic seeds, persisted regressions | repeatable failures and predictable CI duration |
| GCC/Clang versions or defaults drift | Medium | pin versions and normalize flags in fixture metadata; review manifest updates | repository classifications remain authoritative and host upgrades cannot silently change the gate |
| Compiler quirks are mistaken for C99 | High | standard citations and explicit disagreement classifications | no acceptance decision is inherited from one compiler without review |
| Diagnostic migration causes message churn | Medium | structured-field assertions plus a small rendering golden set | reviewed symbolic codes/data with intentionally reviewed prose changes |

## Out of scope

- semantic type construction, compatibility, conversions, and lvalue checks;
- constant folding/evaluation and integer-constant-expression constraints;
- linkage, redeclaration, storage-duration, and object-layout analysis;
- initializer current-object and override semantics;
- duplicate/undefined label, jump legality, and general control-flow analysis;
- non-C99 vendor extensions or a general dialect plugin interface;
- exact GCC/Clang diagnostic prose, multiplicity, global order, internal IDs, or
  AST layout;
- full drop-in command-line compatibility, ABI, object-file compatibility,
  assembler invocation, and linker-driver behavior;
- preprocessor redesign beyond closing the parser-token handoff;
- intermediate representation, optimization, code generation, assembly, and
  linking; and
- deleting or rewriting the historical research reports.

## Done

Phase 05 is complete only when all of the following are true:

- [x] Every compliance row has an explicit status, owner, and evidence link.
- [x] Every meaningful strict-C99 grammar alternative and interaction in scope
      has positive complete-tree evidence.
- [x] Version-pinned, normalized GCC/Clang fixtures are classified against C99
      in a reviewed compatibility manifest.
- [x] Excluded extensions and invalid phase-7 input are diagnosed without panic.
- [x] Repeated type qualifiers and `inline` are accepted; any quality warning is
      named, suppressible, and independent of strict-C99 acceptance.
- [x] Absent storage class is distinct from explicit `auto`.
- [x] Every identifier syntax node retains exact source provenance.
- [x] `_Imaginary` is a keyword but never a valid strict-C99 type node.
- [x] `ParsedTranslationUnit` and `SyntaxTree` form the shared caller/test seam.
- [x] No behavioral test needs raw mutable arena access.
- [x] Every parser diagnostic has a symbolic kind/code, severity, optional
      warning group, frame, expectation, found token or EOF, primary source,
      useful ranges/notes, and recovery data where applicable.
- [x] Primary diagnostics and related notes are yielded in FIFO emission groups
      without a global source-order sort.
- [x] Every frame phase has an owned-delimiter, EOF, recovery, and progress test.
- [x] Pure top-level garbage yields an external error node, not a fake declaration.
- [x] Malformed cross-family cases preserve the nearest following legal construct.
- [x] Property and truncation tests terminate without parser panic or state leak.
- [x] Every parser-relevant C99 minimum translation limit passes without Rust
      call-stack recursion.
- [x] Configured or representational resource exhaustion produces a stable
      diagnostic rather than an input-scale conversion panic; process-wide OOM
      is explicitly outside the recovery guarantee.
- [x] Readable syntax-tree and raw-storage inspection are opt-in; normal CLI
      behavior remains compiler-like.
- [x] `cargo test --all-targets` passes.
- [x] `cargo +nightly fmt --check` passes.
- [x] `cargo clippy --all-targets -- -D warnings` passes.
- [x] `git diff --check` passes.
- [x] `parser-roadmap.md`, `CONTEXT.md`, `README.md`, and the compliance checklist
      describe the implemented state accurately.
