# Windows jobs from the bcc-rust sandbox

Runner `bcc-win-x64` belongs to `BenBwall/bcc-rust`. Its connection method is
**GitHub Actions**, with labels `self-hosted`, `Windows`, `X64`, `bcc-windows`,
and `bcc-windows-gnu`. No inbound Windows SSH connection or token is needed.
The host listener starts in the background at Windows login. The host must be
awake and logged in for this user-session runner to accept jobs.

This dedicated `ci/windows-runner` branch is an operational dispatcher. Its
`ci.yml` has only `workflow_dispatch`; it does not change `main` or its Linux
CI, and does not automatically execute pull requests. GitHub already knows
`ci.yml` on the default branch, so dispatch that existing workflow **with
`--ref ci/windows-runner`**. Do not merge this branch over the main CI file.

## Commands inside the sandbox

`bcc-windows` submits a job and prints its unique run ID and URL. Add `--wait`
to wait for the result and receive a nonzero exit status on failure. It uses
the sandbox's existing authenticated `gh`, without copying credentials to
Windows. Select a trusted commit already pushed to the repository. Without
`--sha`, it uses the current clean checkout's HEAD. From outside a Git
repository (including `~`), it uses
`/home/agent/workspace/bcc-rust/checkout`. It prints the selected checkout and
commit before submission. Use `--checkout PATH` to choose another clean
worktree from any directory, or `--sha FULL_SHA` to select a pushed commit
without a local checkout. Uncommitted edits and untracked files are not
transferred; a dirty checkout or an unpushed commit stops submission.

```sh
bcc-windows status
bcc-windows ci --wait  # also works from ~ using the sandbox checkout
bcc-windows ci --checkout /path/to/agent-worktree --wait
bcc-windows test --sha FULL_SHA --wait -- --all-targets --no-fail-fast
bcc-windows test --sha FULL_SHA --wait -- --test code_generation_cli mingw_crt_inline_helpers_link_from_two_translation_units -- --nocapture
bcc-windows bench --sha FULL_SHA --wait -- --bench bench -- 'Lexer/batch' --sample-size 10 --warm-up-time 1 --measurement-time 1 --noplot
bcc-windows ci --sha FULL_SHA --wait
bcc-windows list
bcc-windows wait RUN_ID
bcc-windows logs RUN_ID
bcc-windows download RUN_ID --dir /c/Users/benbw/Documents/GitRepo/bcc-rust/.git/windows-results/RUN_ID
```

Cargo arguments after `--` are passed as a JSON array, preserving argument
boundaries. Bench jobs add `--features benchmarking-internals`. CI runs
all-target tests, allocation tests, strict Clippy with and without benchmark
features, nightly formatting, and `git diff --check`, recording every check
even when another fails. Cargo programs and their descendants receive the
Windows crash-dialog suppression wrapper.

The same interface works directly through GitHub CLI:

```sh
gh workflow run ci.yml --repo BenBwall/bcc-rust --ref ci/windows-runner \
  -f commit_sha=FULL_SHA -f suite=test \
  -f 'cargo_args_json=["--all-targets","--no-fail-fast"]'
gh run list --repo BenBwall/bcc-rust --workflow ci.yml --branch ci/windows-runner
gh run watch RUN_ID --repo BenBwall/bcc-rust --exit-status
gh run download RUN_ID --repo BenBwall/bcc-rust --dir RESULT_DIRECTORY
```

Logs and `results.json` (requested SHA, tree, actual Windows/Rust identity,
arguments, and per-command exit codes) are uploaded for 14 days. Criterion
reports are uploaded when produced. Artifacts include failures; a completed
run is not necessarily a passing run. The requested compiler commit receives
the `windows/test`, `windows/bench`, or `windows/ci` GitHub commit status and a
link to its run. Jobs are serialized on this one host.

## Windows host maintenance

- Runner installation: `C:\bcc\actions-runner`.
- Start/restart after deliberately stopping it:
  `pwsh -NoProfile -File C:\bcc\runner\Start-WindowsRunner.ps1`.
- Listener logs: `C:\bcc\runner\listener.stdout.log` and `.stderr.log`.
- Per-user startup shortcut: `bcc Windows Actions runner.lnk`.
- Native toolchain: Rust `1.99.0-x86_64-pc-windows-gnu`, MinGW
  `C:\Strawberry\c`, Python 3.14; at most eight Cargo build jobs.
- Matching Rust/LLVM source pins reuse junctions to the verified Windows
  installations under `C:\bcc\manual-checkout`; Cargo outputs stay in the
  Actions checkout. A different source pin requires a fresh source/cache
  checkout; the source helper stops on a mismatched existing pin, preserving
  the prepared installations.
- Keep `target/llvm` and `target/llvm-libs` installations intact. The existing
  manual validation checkout and the sandbox compiler checkout are preserved.

If the runner is offline, check that Windows is awake, its user session is
running, and the listener log says `Listening for Jobs`. Agent permission
errors are checked with `gh api repos/BenBwall/bcc-rust --jq .permissions`.

## Verified on 17 October 2026 (Helsinki)

All three workloads were submitted by the installed `bcc-windows` command
inside the sandbox, testing compiler commit
`c114b485499bc5bb1ad81837084126a6e5cf7497` on native Windows x64.

| Workload | GitHub Actions run | Result |
|---|---|---|
| Focused Windows CRT test | [38975948896](https://github.com/BenBwall/bcc-rust/actions/runs/38975948896) | One executed passing test, no skip; client exit 0 |
| Timed lexer benchmark | [38976028725](https://github.com/BenBwall/bcc-rust/actions/runs/38976028725) | Actual timing measurements and Criterion artifacts; client exit 0 |
| Canonical CI | [38975693228](https://github.com/BenBwall/bcc-rust/actions/runs/38975693228) | 2,257 passed, 23 failed, 7 ignored; client exit 1 |

CI also passed all 31 allocation tests, both strict Clippy configurations,
nightly formatting, and whitespace checks. Its 23 all-target failures match
the prior manual validation: 22 Windows-to-Linux links cannot find `-lm`,
and `reachable_functions::tests::emitted_symbols_match_clang_o0` observes
`llvm.compiler.used` versus Clang's `llvm.used`. These remain failed compiler
checks. Rust harness counts include tests that can return early on a host;
those results do not establish runtime coverage.

The sandbox downloaded all three runs' artifacts under
`/c/Users/benbw/Documents/GitRepo/bcc-rust/.git/windows-results/`. Their
`windows/test`, `windows/bench`, and `windows/ci` commit statuses were verified
as success, success, and failure respectively. The earlier benchmark run
[38975422542](https://github.com/BenBwall/bcc-rust/actions/runs/38975422542)
also produced three measured lexer cases.

The eleven submission-client regressions passed on both Linux and Windows,
including submission from home and selection of a linked worktree;
PowerShell parsing and actionlint passed. Existing compiler checkouts and
the default branch were preserved. The source compiler was not changed to
make these checks pass.
