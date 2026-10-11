# Contributing

## Source layout

Code reads top-down: a reader meets a module's main algorithm first and the
details it relies on afterwards. Rust resolves items regardless of their order,
so nothing needs to be declared before it is used.

- **A module's entry file is short.** The entry file (`parsing.rs` for
  `parsing/`) holds only these, in order:
  1. A `//!` map of the module: a paragraph on how it works, a short worked
     example when the algorithm has steps worth tracing, a reading order that
     names the items to read first, a list of its files grouped by role, and
     then the standard citations described below.
  2. The `mod` declarations, grouped by role, with a `//` heading comment over
     each group. rustfmt sorts modules only within a group, so the grouping
     stays put.
  3. The re-exports that form the module's interface.
  4. The module's main loop or entry point, followed by the state type that
     loop runs on. Constructors, helpers, error types, token types, and debug
     views live in submodules. A module with no loop, such as a collection of
     utilities, gives this place to its entry point or its central type.
- **Every file puts its most important item first.** Order items as a reader
  should meet them: the entry point or core algorithm, then the types and
  helpers it uses in the order it uses them, then constructors and accessors,
  then trait impls such as `Debug`, `Display`, and `Default`. `#[cfg(test)]`
  modules come last.
- **A family of parallel files gets its own directory.** When a module has one
  file per grammar frame, extension, builtin table, directive, or scanner
  concern, those files move into a subdirectory with its own entry file, as in
  `parsing/frames/`, `parsing/extensions/`, `semantic_analysis/builtins/`,
  `preprocessing/directives/`, and `preprocessor_tokenizer/scanning/`. A
  subsystem with enough parts to need its own map gets one too: the
  preprocessor's working state in `preprocessing/runtime/`, and the arena and
  collection utilities in `util/memory/` and `util/collections/`. Other role
  groups stay flat in the module's directory, under a `//` heading over their
  `mod` lines in the entry file. The entry file re-exports what other
  modules use, so moving a file does not change crate-visible paths.

## Citing the C standard

Compiler code says which part of the C standard it implements. The reference is
the repository's [`standards/c99-n1256.pdf`](standards/c99-n1256.pdf),
WG14/N1256 (ISO/IEC 9899:TC3: C99 with Technical Corrigenda 1-3).

- Every module under `src/translation_phases/` has a `//!` comment that names
  the translation phases and clauses it implements, and where its
  responsibility stops (for example, a constraint left to semantic analysis).
  In an entry file these citations follow the module map.
- A type, frame, function, or diagnostic that implements a specific grammar
  production, constraint, semantic rule, translation limit, or
  implementation-defined choice carries a `C99:` line in its doc comment. Mark
  extensions and implementation-defined choices as such.
- Doc comments give the clause, the printed page, and the 1-based PDF page of
  the cited text (PDF page = printed page + 12):

  ```rust
  /// C99: §6.8.4.1, p. 133; PDF p. 145.
  /// C99: §6.7.2 paragraph 2, pp. 99-100; PDF pp. 111-112.
  ```

- Inline comments and diagnostic notes may use the compact form
  `C99 §6.7.6p1:`.

## Commit messages

Enable the repository's commit-message hook after cloning.

On Windows, run:

```powershell
pwsh -NoProfile -File .githooks/setup.ps1
```

On Linux and macOS, run:

```sh
git config --local core.hooksPath .githooks/unix
```

Both paths use the tracked Rust validator in `.githooks/commit-msg.rs`. The
Windows setup script compiles it into Git's private hooks directory; the
installed validator refreshes itself when its tracked source changes. The Unix
wrapper compiles and caches the same source with the default stable `rustc`.

Commits use a lightweight Conventional Commits format:

```text
type(scope): Concise imperative summary

Optional body explaining why the change was needed, important design
decisions, or non-obvious consequences.

Optional trailers
```

The scope is optional. Use a lowercase domain or subsystem name such as
`preprocess`, `parse`, `ast`, `diagnostics`, `cli`, `build`, or `tooling`. Omit
the scope for repository-wide changes.

Allowed types:

- `feat`: New user-facing functionality
- `fix`: Bug fix
- `perf`: Performance improvement
- `refactor`: Behavior-preserving code restructuring
- `docs`: Documentation only
- `test`: Test-only changes
- `build`: Dependencies, Cargo configuration, or build process
- `ci`: CI configuration
- `chore`: Repository maintenance not covered above
- `revert`: Revert an earlier commit

Subject rules:

- Write an imperative, present-tense summary.
- Capitalize the summary.
- Do not end the summary with a period.
- Keep the complete subject at or below 72 characters.
- Keep each commit focused on one cohesive change.
- Run relevant checks and avoid introducing failures beyond the documented
  baseline.

Add a body when the subject does not adequately explain the motivation,
important decisions, or consequences. Reference issues using trailers such as
`Fixes #123`.

Mark breaking changes with `!` in the subject and a `BREAKING CHANGE:` trailer:

```text
feat(parse)!: Replace the syntax tree representation

BREAKING CHANGE: Parser consumers must migrate to the new node types.
```

Examples:

```text
feat(preprocess): Expand variadic macros
fix(parse): Preserve declarator source ranges
perf(ast): Reuse expression arena storage
docs: Explain the compiler pipeline
chore(tooling): Enforce standard commit messages
```
