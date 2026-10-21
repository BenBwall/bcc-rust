# Parser study observations

See [the report](../parser-recursive-descent.md) for methodology, commands and
interpretation. These observations belong to the isolated study branch; they
are not performance gates.

- `final-*.json` retain subprocess commands, binary hashes, return codes and
  observations. A collection process exiting zero does not mean every row
  succeeded or every inspected tree matched. In particular, the grammar files
  retain recovery mismatches, alignment inspection retains two OS kills, and
  production profiling retains two unsupported-instruction failures.
- `timing.tsv` summarizes successful timing rows. Arena counts are deterministic;
  RSS and timing remain subject to shared-host noise.
- `events.tsv` contains successful Callgrind simulations only. They are model
  events, not physical hardware measurements. Failed production controls do not
  contribute partial totals.
- `calls.tsv` counts selected out-of-line call edges. Lua `.self.txt` files retain
  source attribution for release/debug; compiler inlining limits interpretation.
- `inputs.json` and `grammar.json` record corpus hashes and generation commands.
  `environment.json` is the initial host snapshot, including its earlier revision;
  each observation's binary hash and the report's source revisions identify the
  final implementation.
- `validation.json` records completed test results and the SHA-256 of
  `validation-build.log`. `sse4a-proof.log` and `production-valgrind-failure.log`
  document the unsupported Valgrind control.

Regenerate the archive after collection with
`python3 scripts/parser_study_archive.py`. Raw Callgrind files and full rendered
syntax outputs stay in ignored scratch; the retained scripts reproduce them.
