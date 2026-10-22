# Reserved-stack experiment observations

[The report](../parser-reserved-stack.md) explains the implementation, measurements
and limitations. The production parser remains unchanged.

- `reserved-*-timing.json`: nine rotating samples per shallow input/mode.
- `reserved-*-deep.json`: seven samples per deep workload/mode; ordinary modes
  use a 1 MiB worker, reserved recursion uses 100 GiB.
- `reserved-*-memory.json`: three fresh-process measurements with stack smaps
  snapshots enabled. Timing comparisons use the separate timing/deep batches.
- `reserved-*-grid.json`: sampled nesting through 65,536. Samples are lower
  bounds on native stack span, not exact peak occupancy.
- `reserved-*-diff.json` and `reserved-*-grammar.json`: rendered output hashes,
  including known malformed recovery differences against the machine.
  `transport-identity.json` checks each algorithm against itself on the reserved
  worker; all 800 comparisons match.
- `reserved-*-size-*.json`: boundary probes. Nine SIGSEGV exits exhaust small
  reservations. `reserved-*-stdlib-100g.json` records six ordinary-worker setup
  failures. Other reserved subprocesses succeed.
- `reserved-*-smoke.json`: the first two inputs in each build, one sample only;
  these smoke times do not support throughput claims.
- `summary.tsv`: successful timing and memory summaries; all-failed groups use
  `NA` for unavailable observations, retaining their failure counts.
- Input/environment snapshots and per-run binary hashes identify the corpus,
  build artifacts and Linux overcommit policy. The environment revision is the
  collection-start revision; native stack source is identified in the report.
- `validation.json`: original/archived log hashes, completion flags and test
  results. Archived logs normalize trailing whitespace. The first two build
  attempts failed as documented; the final build/check batches completed.

Regenerate after collection with `python3 scripts/parser_study_reserved.py archive`.
Full rendered trees remain in ignored scratch; the scripts reproduce them.
