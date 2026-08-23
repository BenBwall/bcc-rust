# Codex prompt: implement parser Phase 03

Copy the prompt below into a new Codex task opened on this repository.

```text
Implement Phase 03 of the bcc-rust language parser in the current workspace.
Carry the implementation through verification and documentation; do not stop
after auditing the code or producing another plan.

Start by running `git status --short` and preserve every existing user change.
Then read these files completely before editing:

1. `.agents/AGENTS.md`
2. `CONTEXT.md`
3. `parser-roadmap.md`
4. `phase-03-statements-and-function-definitions-plan.md`
5. `phase-02-legacy-parser-stack-migration-agent-brief.md`
6. the relevant statement, scope, recovery, and translation-limit sections of
   `c99-parser-compliance-checklist.md`
7. `src/translation_phases/parsing.rs`, plus the parser inspection path in
   `src/lib.rs`

Treat `parser-roadmap.md` as authoritative for phase boundaries and the Phase
03 Markdown plan as authoritative for implementation order, ownership, test
coverage, and completion criteria. Revalidate every status claim against the
current source before relying on it.

Phase 03 goal:

- replace balanced function-body skipping with real prototype-style and
  old-style function-definition nodes;
- parse compound statements with ordered, interleaved declaration and
  statement block items;
- parse the full structural C99 statement grammar: labeled, compound,
  expression, null, selection, iteration, and jump statements;
- add explicit function, prototype, block, implicit selection/iteration, and
  function-local label scope lifetimes;
- preserve source provenance, recovery state, typed child results, one
  delimiter owner, and heap-backed grammar nesting;
- keep all existing Phase 02 declaration behavior green, including function
  prototypes, K&R declarators, variadics, typedef classification, and function
  pointers.

Phase boundary:

- Phase 04 owns general expression and type-name parsing.
- Phase 05 owns initializer parsing and expression-dependent declaration
  branches.
- In Phase 03, every present expression position must be an explicit typed
  deferred child with source provenance and the stable unsupported diagnostic.
  Represent syntactic absence separately from a present-but-deferred
  expression; do not overload one `Option` state for both meanings.
- The parent statement owns `;`, `)`, `:`, or any other expression terminator.
  The deferred child stops before that token.
- Block and `for` declarations continue to use the existing typed initializer
  seam.
- Remove `FunctionBodyNotImplemented` and balanced-body skipping from valid
  function-definition paths. Expression and initializer future-child seams are
  expected Phase 03 output and must be documented for Phases 04 and 05.

Implement the plan as vertical parser paths reachable through
`Parser::next_item`. Keep frame phases private and reuse the existing
declaration/declarator modules instead of cloning their grammar. Base the
declaration-versus-definition choice on the completed declarator binding shape:
a pointer to a function remains a declaration and cannot acquire a body.

Write black-box parser tests through the public parser interface before or with
each vertical slice. Cover every case required by the Phase 03 grammar matrix,
including dangling `else`, all `for` forms, mixed block items, label/typedef
name collisions, parameter visibility, old-style definitions, malformed and
EOF recovery for every owned delimiter, continued parsing after a recovered
body, 127 nested blocks, and 127 parameters. Assert syntax shape, provenance,
recovery flags, diagnostics, scope restoration, and delimiter traces where the
plan requires them.

Keep implementation changes focused on Phase 03. Update `CONTEXT.md`, README or
status guidance only where completed behavior or a durable term changes. Keep
`parser-roadmap.md` phase boundaries intact. Do not create commits unless I ask
for them.

Before handing back, run the repository's canonical checks from
`.agents/AGENTS.md`:

- `cargo test --all-targets`
- `cargo +nightly fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `git diff --check`

Also exercise the CLI syntax dump manually with representative prototypes,
prototype-style definitions, old-style definitions, every statement family,
mixed block items, and malformed recovery; verify `--tokens` still retains the
tokenizer view. If repository-wide Clippy reports pre-existing diagnostics,
record the complete evidence and prove that no diagnostic points to a Phase 03
changed file.

Finish with a concise report containing:

- the implemented grammar and syntax-store changes;
- the scope and recovery behavior added;
- the expression and initializer seams deliberately left for Phases 04 and 05;
- every validation command and result;
- any genuine blocker or remaining Phase 03 completion criterion.

Continue until every Phase 03 done criterion is satisfied or a concrete blocker
requires new user authority. An intermediate green slice is progress, not the
requested final outcome.
```
