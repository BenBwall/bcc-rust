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
`--sha`, it uses the current clean checkout's HEAD. Uncommitted edits and
untracked files are not transferred.

```sh
bcc-windows status
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
run is not necessarily a passing run. Jobs are serialized on this one host.

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
  Actions checkout. Different source pins use the sparse source preparation
  helper and a separate build rather than overwriting those installations.
- Keep `target/llvm` and `target/llvm-libs` installations intact. The existing
  manual validation checkout and the sandbox compiler checkout are preserved.

If the runner is offline, check that Windows is awake, its user session is
running, and the listener log says `Listening for Jobs`. Agent permission
errors are checked with `gh api repos/BenBwall/bcc-rust --jq .permissions`.
