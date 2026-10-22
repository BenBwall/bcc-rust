# Compilation worker and recursive lowering evidence

See [the report](../recursive-lowering.md) for implementation, commands, results
and interpretation. Rust source cohort:
`932f32a89c75682029e7f90b208b74960fc71e94`.

The final follow-up adds a `Win32_Security` dependency feature for ordinary
Windows builds; this feature was already active from dev-dependencies in the
native test cohort. No Rust source changed after the measured cohort.

## Linux

- `linux-summary.json.gz`: all 684 timing samples, 38 parity comparisons,
  eight deep probes and per-input medians/ranges/population CVs.
- `linux-build.log.gz`: complete successful build and release/debug harness log.
- `linux-validation.log.gz`: focused lowering tests and strict study Clippy.
- `linux-checks.log.gz`: complete successful canonical and study checks.
- `linux-width.json.gz`: 36 fresh-process timings/RSS samples, input and binary
  hashes, and task/recursive inspection identity for both profiles.
- `linux-width.log.gz`: wide-function timing progress; the JSON contains the
  subsequent explicit parity verification hashes.
- `linux-cli.json.gz`: actual CLI arguments, expected and actual statuses,
  binary/output hashes and stderr. Deep files contain 4,096 block pairs or
  `a+` operators, as described in the report.
- `linux-*-initial.log.gz`, `linux-validation-*.log.gz`, and `linux-check.log.gz`:
  retained initial compile/lint/test integration evidence; these precede the
  successful checks and do not describe the final code state.
- `metadata.json`: measured Rust cohort, copied binary SHA-256s and every real
  C input SHA-256, plus reproduction-script hashes.

## Native Windows

`windows-summary.json.gz` contains all 684 native timing samples, 38 parity
comparisons and eight deep probes. `windows-collection.log.gz` records the
successful serial collector. Per-run directories
retain normalized UTF-8/LF `results.json` and compressed raw `source-setup.log`
and `tests.log`, including requested/source identity, toolchain, command arguments
and exit status. Windows jobs run through the crash-dialog suppression wrapper.
`windows/runs.json` records every command, run ID, conclusion and source identity.

| Job | Run | Conclusion |
| --- | --- | --- |
| lowering-tests-debug | [39769952926](https://github.com/BenBwall/bcc-rust/actions/runs/39769952926) | success |
| lowering-measurements-debug | [39770784433](https://github.com/BenBwall/bcc-rust/actions/runs/39770784433) | success |
| lowering-tests-release | [39771370581](https://github.com/BenBwall/bcc-rust/actions/runs/39771370581) | success |
| lowering-measurements-release | [39772649911](https://github.com/BenBwall/bcc-rust/actions/runs/39772649911) | success |

## Inputs and storage

The two synthetic 4,000-function mixes and deep probes are defined in
[`runner.rs`](../../../examples/lowering_study/runner.rs). The 50,000-statement
wide function comes from
[`lowering_study_width.py`](../../../scripts/lowering_study_width.py).
The same frontend, semantic facts, IR builders and reservation policy serve
both lowering modes. Preprocessing and semantic analysis are outside the
lowering timer. Inspection verifies printed IR, function sources, call diagnostics
and refusal diagnostics outside timing. Native stack sampling is also untimed.
The existing debug-only per-function IR verifier remains inside lowering time
in both modes. All timed lowering intervals assert zero global allocator
calls/bytes.

All real C inputs come from the measured source commit; their hashes are in
`metadata.json`:

- `benches/c/bits.c`
- `benches/c/bytecode.c`
- `benches/c/collatz.c`
- `benches/c/crc32.c`
- `benches/c/fannkuch.c`
- `benches/c/hashtable.c`
- `benches/c/levenshtein.c`
- `benches/c/linkedlist.c`
- `benches/c/mandel.c`
- `benches/c/matmul.c`
- `benches/c/nqueens.c`
- `benches/c/particles.c`
- `benches/c/sieve.c`
- `benches/c/sorts.c`
- `benches/c/stencil.c`
- `benches/c/strings.c`
- `benches/c/switchvm.c`

Compressed files use deterministic gzip timestamps (`mtime=0`). Read with
`gzip -dc PATH`. Raw logs retain their original bytes; normalized JSON uses
UTF-8 and LF. RSS is available for the fresh-process Linux wide cohort only;
no Windows working-set or hardware-counter measurements are claimed.
