# Recovery parity and Windows reserved-stack follow-up

Status: investigation in progress. This follow-up addresses the ten malformed
cases recorded in `parser-reserved-stack.md`, and Windows validation requested
by the owner. The production parser remains unchanged by study-only fixes.

Initial reproduction uses the preserved
`.scratch/parser-study/reserved-release-grammar-inspections` snapshots. The ten
cases cover missing or stray header operands, grouped expression synchronization,
unowned closing delimiters, missing call separators, missing `if` bodies,
missing `while` after `do`, and extra `for` header clauses. Some differences lose
following statements or detach an `else`; they are not only message differences.

The available host is Linux. Neither Windows Rust targets nor Wine are initially
installed. `build_support/native.rs` explicitly requires `TARGET == HOST`, so a
full compiler cross-build is unsupported. An isolated harness can compile the
actual example-only Windows reserved-stack module without altering the compiler
build. Wine results, if obtained, will be identified as compatibility-layer
checks and will not be used as native Windows performance evidence.

Study-only fixes now seed expression recovery with the already parsed operand,
retain statement else ownership across recursive/hybrid calls, represent absent
header operands as missing slots, preserve the next statement after a missing
`while`, and reuse bounded `for`-header lookahead. Validation is pending.

Windows GNU strict Clippy passes for the isolated probe. Initial linking
requires the MinGW `dlltool` utility in addition to import libraries. The
first expanded regression run found a duplicate diagnostic on a hybrid handoff;
the prototype now tracks whether the frame already finished recovery.
