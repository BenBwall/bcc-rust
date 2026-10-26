# Native Windows parser study evidence

Source commit: `32fb7b0a0aa52c7d896568f79266425d1ce5dba2`.
Tree: `34f5c75629574cbfeb583be2cc28a1699300ee87`.
See [the report](../parser-recovery-windows.md) for method, interpretation and limits.

`runs.json` records exact commands, workflow outcomes, toolchain/source identity
and download completion. `summary.json.gz` contains all parsed timing samples,
per-input medians/ranges/CVs, parser stack spans and page/overflow probes.
Per-job `results.json` and compressed `source-setup.log` / `tests.log` preserve
the original evidence. JSON is normalized UTF-8/LF; gzip logs preserve raw bytes
and use a fixed timestamp. Read with `gzip -dc PATH`.

## Jobs

| Job | Run | Conclusion |
| --- | --- | --- |
| recovery-debug | [39757463234](https://github.com/BenBwall/bcc-rust/actions/runs/39757463234) | success |
| stack-debug | [39758189652](https://github.com/BenBwall/bcc-rust/actions/runs/39758189652) | success |
| parser-debug | [39758407362](https://github.com/BenBwall/bcc-rust/actions/runs/39758407362) | success |
| recovery-release | [39758809123](https://github.com/BenBwall/bcc-rust/actions/runs/39758809123) | success |
| stack-release | [39760102752](https://github.com/BenBwall/bcc-rust/actions/runs/39760102752) | success |
| parser-release | [39760488673](https://github.com/BenBwall/bcc-rust/actions/runs/39760488673) | success |
| worker-debug | [39760964833](https://github.com/BenBwall/bcc-rust/actions/runs/39760964833) | success |
| worker-release | [39761185241](https://github.com/BenBwall/bcc-rust/actions/runs/39761185241) | success |

## Inputs

The two 4,000-function synthetic inputs and deep parser fixtures are defined in
[`parser_runner_tests.rs`](../../../examples/parser_study/parser_runner_tests.rs).
The page-frame/overflow inputs are in
[`probe_runner_tests.rs`](../../../examples/parser_study/probe_runner_tests.rs).
All source files below come from the exact source tree above. Each real C file
has four comparisons against machine AST/provenance/diagnostics per profile.
Lua was not staged and has no native timing result.

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
