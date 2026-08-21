# Agent Guide

## Workflow

- This is a Rust Cargo project. Run commands from the repository root unless a task requires a narrower working directory.
- Common checks are `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `git diff --check`.
- Preserve generated reports and other unrelated untracked files unless the user explicitly asks to remove them.

## Agent skills

### Skill source

Skills vendored from `mattpocock/skills` are exposed through symlinks in both `.agents/skills/<name>` and `.claude/skills/<name>`. Each link points to `vendor/mattpocock-skills/skills/<category>/<name>`; the subtree under `vendor/mattpocock-skills/` is the single source of truth for upstream content.

Only the promoted `engineering` and `productivity` skills are linked into the agent clients. Experimental, miscellaneous, and deprecated skills remain available in the subtree without being auto-discovered.

Pull upstream updates with:

```sh
git subtree pull --prefix=vendor/mattpocock-skills https://github.com/mattpocock/skills.git main --squash
```

When upstream promotes a new skill, create matching symlinks in both client directories:

```sh
ln -s ../../vendor/mattpocock-skills/skills/<category>/<name> .agents/skills/<name>
ln -s ../../vendor/mattpocock-skills/skills/<category>/<name> .claude/skills/<name>
```
