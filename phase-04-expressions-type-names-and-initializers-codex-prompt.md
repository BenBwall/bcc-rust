# Codex prompt: implement parser Phase 04

Copy the prompt below into a new Codex task opened on this repository.

```text
Implement the merged Phase 04 of the bcc-rust language parser in the current
workspace. Carry the implementation through verification and documentation;
do not stop after auditing the code, implementing only expression reduction,
landing one grammar slice, or producing another plan.

Start by running `git status --short` and preserve every existing user change
and untracked artifact. Then read these files completely before editing:

1. `.agents/AGENTS.md`
2. `GLOSSARY.md`
3. `parser-roadmap.md`
4. `phase-04-expressions-and-type-names-plan.md`
5. `phase-03-statements-and-function-definitions-plan.md`
6. `phase-01a-conditional-expression-foundations-agent-brief.md`
7. the expression, type-name, initializer, typedef-ambiguity, recovery, and
   translation-limit sections of `c99-parser-compliance-checklist.md`
8. `double-e-integration-report.html`
9. `src/translation_phases/parsing.rs`
10. the preprocessor expression evaluator in
    `src/translation_phases/preprocessing.rs`
11. the parser inspection path in `src/lib.rs`

Treat `parser-roadmap.md` as authoritative for phase boundaries and the Phase
04 Markdown plan as authoritative for implementation order, token ownership,
test coverage, and completion criteria. Revalidate every dated status claim,
test count, syntax model, frame inventory, and diagnostic against the current
source before relying on it.

Phase 04 goal:

- implement Double-E-style precedence reduction privately inside
  `ExpressionFrame`, with parser-local operand/operator stacks, state,
  precedence, associativity, markers, provenance, and recovery; leave the
  independently implemented preprocessor-expression evaluator unchanged;
- implement non-recursive `ExpressionFrame`, `TypeNameFrame`, and
  `InitializerFrame` families through the existing parser driver;
- parse the complete C99 expression grammar: primary, postfix, unary, cast,
  multiplicative through logical, conditional, assignment, and comma forms;
- provide distinct `expression`, `assignment-expression`, and
  `constant-expression` entry modes with context-correct comma behavior;
- parse type names and abstract declarators for casts, `sizeof(type-name)`,
  compound literals, and nested parameter/array/function forms using current
  typedef classification;
- parse scalar, brace-enclosed, nested, trailing-comma, and designated
  initializers, including field, array, chained, and mixed designators;
- connect real expression handles to every Phase 03 statement position;
- connect parsed array bounds, bit-field widths, enumerator values, ordinary
  declaration initializers, and compound-literal initializers;
- remove every supported `FutureChildKind` path and its valid-input
  unsupported diagnostic by the end of the phase;
- preserve exact source provenance, recovery state, scope/publication timing,
  typed child returns, one delimiter owner, and heap-backed grammar nesting.

Architecture and ownership:

- Keep one `Parser` driver and one explicit `Vec<ParseFrame>` control stack.
  Nested grammar work must use `ParseAction::Push` and typed `ParseValue`
  returns, never recursive Rust parser calls.
- Add typed expression, constant-expression, type-name, initializer,
  designation, and designator handles and stable arena/list storage. A caller
  must not coordinate parallel vectors or infer a child kind from a diagnostic.
- Keep language-expression reduction inside `parsing.rs`, owned directly by
  `ExpressionFrame`. Do not add a shared reducer module under
  `src/translation_phases/`; that directory contains only C translation
  phases. Duplication with the preprocessor evaluator is intentional until a
  future design demonstrates a substantially deeper shared module.
- Represent `?` as an unmatched blocking marker. `:` reduces the unrestricted
  middle expression to the nearest marker without crossing grouping, then
  forms the right-associative conditional operator. Cover both nested
  conditional shapes before integrating all expression callers.
- Distinguish comma operators from call arguments, initializer elements,
  declarator/enumerator separators, and every other caller-owned comma.
- Track enough operand grammar category to enforce the assignment production's
  unary-expression left operand. Leave lvalue validity to semantic analysis.
- Use current `ScopeStack` classification at the exact opening parenthesis to
  choose type name versus grouped expression. Member names after `.`/`->` and
  field designators consume identifiers independently of ordinary typedef
  classification.
- `ExpressionFrame` owns grouping/call/subscript/conditional markers but stops
  before delimiters owned by a statement, declarator, struct, enum,
  declaration, or initializer caller.
- `InitializerFrame` owns brace pairs and list commas, designation `=`, field
  designator `.`, and array-designator brackets. Its scalar children use
  assignment-expression mode and array designators use constant-expression
  mode.
- `ExpressionFrame` must push the real initializer child for a compound
  literal and resume postfix parsing after it; `(T){1}.member` must compose.
- Preserve declaration publication timing: each ordinary/typedef binding is
  visible after its declarator and before its initializer or later declarator;
  an enumerator is visible after its complete defining enumerator.
- Malformed user input must reduce to focused diagnostics plus recovered/error
  syntax and synchronization. It must not reach input-dependent panics,
  nonprogress loops, process-stack overflow, or corrupted reducer/frame/scope
  state.

Phase boundary:

- Phase 04 owns the full mutually recursive expression/type-name/initializer
  cluster and every currently deferred statement/declaration integration site.
- Phase 05 is the final recovery and C99 parser-closure audit. Do not defer a
  known expression, type-name, initializer, designator, array-bound,
  bit-field-width, or enumerator-value production to it.
- Semantic type resolution, conversions, lvalue checks, member/call checking,
  constant evaluation, initializer current-object and brace-elision rules,
  sequence analysis, control-flow validation, extensions, and code generation
  remain out of scope. Preserve syntax needed by those later passes.

Execution order:

Follow all 14 steps in `phase-04-expressions-and-type-names-plan.md` in order.
Keep each vertical slice reachable through `Parser::next_item` and green before
building the next dependent slice. In particular:

1. lock the current future-child behavior with black-box parser tests and keep
   the existing preprocessor suite as an unchanged regression baseline;
2. repair syntax and arenas before wiring parser control flow;
3. implement the language parser's private reducer directly in
   `ExpressionFrame` without modifying preprocessing;
4. land expression-independent type names before casts, then connect their
   array-bound expressions after the expression entry modes exist;
5. finish expression precedence and recovery before `InitializerFrame`
   depends on assignment/constant-expression children;
6. land scalar/list initializers and designators before compound literals or
   declaration initializers depend on them;
7. replace all statement and declaration future-child seams;
8. harden the combined grammar and complete the documentation handoff.

Write black-box tests through the public parser interface before or with every
vertical slice. Use the complete matrix in the Phase 04 plan. At minimum, prove:

- every C99 operator, equal-precedence chain, right-associative cast/
  conditional/assignment chain, and invalid assignment-left grammar;
- `a ? b ? c : d : e`, `a ? b : c ? d : e`, conditional middle comma
  expressions, grouping boundaries, and malformed `?`/`:` recovery;
- `f(a,b)`, `f((a,b))`, nested mixtures, postfix chains, 127 arguments, and
  caller-owned commas/closing delimiters;
- cast/grouping and both `sizeof` forms under file, parameter, function, block,
  and implicit-scope typedef shadowing;
- pointer, parenthesized, array, function, and nested abstract type names;
- scalar/nested/trailing-comma initializers and field/array/chained/mixed
  designators with exact order and source provenance;
- compound literals with parsed initializers and following postfix suffixes;
- every statement expression site and every array-bound, bit-field,
  enumerator-value, and declaration-initializer site;
- malformed and EOF recovery for every frame phase, continued parsing of later
  elements/statements/declarations/file items, and unchanged parent delimiter
  traces;
- 63 nested parenthesized expressions, 127 call arguments, required combined
  declarator depth, deeply nested initializer lists, and substantially deeper
  heap-backed stress without Rust call-stack recursion;
- unchanged preprocessor evaluation, nested conditional behavior,
  comma-liveness, extension-policy behavior, and diagnostic wording.

Assert syntax shape, stable arena slices, exact source vectors,
recovered/error state, diagnostics, typedef classification and publication
timing, scope restoration, reducer-stack cleanup, frame-stack cleanup, and
cursor/delimiter traces wherever the plan requires them.

Keep implementation changes focused on merged Phase 04. Update `GLOSSARY.md`,
README/status guidance, `parser-roadmap.md`, and the compliance checklist only
where completed behavior or durable vocabulary changes. Do not create commits
unless I ask for them.

Before handing back, run the canonical checks from `.agents/AGENTS.md`:

- `cargo test --all-targets`
- `cargo +nightly fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `git diff --check`

If repository-wide strict Clippy reports baseline debt, retain the complete
evidence and prove that no diagnostic points at a Phase 04 changed file.

Exercise the CLI syntax dump manually with representative examples from every
expression family, type-name form, initializer/designator form, statement and
declaration integration site, typedef-shadowing case, compound literal, and
malformed recovery family. Verify `--tokens` still retains the tokenizer view.

Finish with a concise report containing:

- the implemented reducer, frame, syntax-store, and grammar changes;
- statement and declaration integration completed;
- scope, publication, delimiter, provenance, and recovery behavior;
- evidence that no supported future-child path or valid-input unsupported
  diagnostic remains;
- every validation command and result;
- every semantic/extension item deliberately left out of syntax parsing;
- any genuine blocker or unsatisfied Phase 04 completion criterion.

Continue until every Phase 04 done criterion is satisfied or a concrete
blocker requires new user authority. An intermediate green grammar slice is
progress, not the requested final outcome.
```
