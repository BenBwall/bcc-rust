# Contributing

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
