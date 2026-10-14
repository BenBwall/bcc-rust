# M4 builtin registry and scalar checked instances

Base: PR #53 head `16ef56a6d6edf2740d84776fa07f5a2cd31b0668`, branch
`claude/semantic-facts-m2`, containing M1, M2 and M3. The approved design was
read with `git show origin/claude/semantic-facts-design:docs/design/semantic-facts.md`;
that branch resolves to `d6aa79b61ace166a54b0ab12518fe3b389b90943`.
This implements migration row M4, the scalar/library portions of B01-B32 and C11.
M6 is deferred. Completion is recorded beside M1-M3 in
[middle-end.md](../../middle-end.md); producer/query contracts are in
[semantic-analysis.md](../../semantic-analysis.md).

## Result and choices

The neutral `src/builtins/` registry holds 97 stable descriptor IDs: 70 scalar
and special-form contracts and 27 library aliases. Target-dependent signatures
remain symbolic. Closed enums describe conversions, evaluation, immediate
constraints, checker rules, operations, effects, syntax, designator policy and
availability through the existing `Feature` policy. Library equivalence belongs
to the same contract. Descriptors contain no checker/lowerer callbacks, syntax
nodes, semantic type IDs, IR opcodes or message strings. Diagnostic identities
project into the existing `SemanticErrorKind` catalog.

Binding lookup precedes intrinsic discovery. `BindingOrigin::Intrinsic(BuiltinId)`
records registered discovery; `User`, `ImplicitOrdinary`, `LegacyIntrinsic` and
`OpaqueIntrinsic` keep shadowing, accepted implicit declarations and temporary
family recognition explicit. Addressable aliases retain ordinary calls and
runtime symbols. `__builtin_fabsf` and `__builtin_fabsl` receive descriptors with
float and long-double signatures so the parallel libm work can merge without
introducing another prototype source. This step does not add libm host functions.

The M3 sparse call map now retains arena-owned `BuiltinCallFacts` for checked
scalar calls and varargs/offsetof keyword forms. Instances retain descriptor,
operation, normalized operand references/types/constants/evaluation, result and
operation types, folds and effects. Type operands have no fabricated expression.
Varargs operands distinguish cursor storage from an adjusted SysV list pointer;
overflow predicates retain bit-field width and effect-only destination use.
Original-width prefetch immediates remain distinct from parameter-converted
object-size modes, alignment and probability constraints.

Lowering schedules and finishes checked instances. Old scalar name predicates,
prototype construction and lowering redispatch are deleted, as are
`common.rs::keyword_call`, the lowering keyword adapter and `FinishKeyword`.
Named varargs calls no longer manufacture `Builtin` syntax in the translation-unit
arena. Address-taken discovery consumes retained storage requirements. Missing
required instances or operands report `MissingFact` with source provenance.
The existing scalar IR implementations, ordinary library execution path,
constant-expression algorithms and interpreter external policy remain in place.

Choices made within M4:

- The sorted spelling index is generated from the two handwritten contract
  shards. Regeneration preserves assigned numeric IDs. The registry is the sole
  scalar/library signature and recognition source in semantic analysis.
- Existing operation-specific floating, overflow and varargs checkers match
  closed rules in their phase. Fixed scalar constraints are interpreted from
  descriptor operands. No diagnostic message table or callback mechanism is added.
- Preprocessing retains its current `__has_builtin` source until M6. Exhaustive
  agreement tests cover every registry entry and deliberately invalid suffixes.
- The only new contracts beyond baseline are the requested `fabsf`/`fabsl`
  aliases. Four semantic inspection snapshots are captured from the saved clean
  baseline executable for all 95 baseline contracts; existing goldens are untouched.
- Atomic, x86, vector and type-generic recognition/checking, their generated
  tables and special forms remain explicit `LegacyIntrinsic`/keyword M6 bridges.
  Reserved type-name seeds and preprocessing capability recognition are also
  deferred to M6. Returns-twice and open-external capability policy remain M7 work.
- Compiler storage stays arena-only and syntax/IR traversal uses existing
  explicit stacks. Normalized operand types/constants are retained for later
  consumers with narrow host-independent dead-code expectations outside tests.

Compiler changes add 3157 and delete 816 Rust lines across 34 files.
Tests add 422 and delete 2 Rust lines across 5 files. Counts use
`git diff --numstat` plus new Rust files, excluding docs, fixtures and survey data.
Shared LLVM junctions and Git metadata were not changed. No files were staged
or committed. Whole-file commit boundaries are in `target/commit-plan.md`.

## Coverage and validation

Before removing the old scalar/library predicates, a temporary exhaustive test
compared descriptor IDs, exact spellings, all suffixes, projected signatures and
support with the old producers on all four targets. All 93 pre-existing ordinary
contracts passed (`target/m4-legacy-contracts.log`); the old producers and temporary
comparison test were then deleted. An isolated regeneration and nightly rustfmt
round trip exactly reproduce the checked-in index (`target/m4-index-regeneration.json`).
The 26 scalar-focused tests also passed during
that stage (`target/m4-legacy-parity.log`).

Permanent coverage checks all 97 IDs and preprocessing support across four
targets, strict/GNU and hosted/freestanding modes; user function-pointer shadowing
for every ordinary spelling; scalar Feature Allow/Warn/Deny behavior; binding
origins; normalized conversion-boundary constants, probability and object-size
folds; effect-only bit-fields; named and keyword varargs storage; new alias types;
missing-instance/operand provenance; baseline semantic inspection and zero global
allocations in the feature-enabled compilation case. Existing scalar execution,
128-bit prefetch/immediate, conversion-boundary, diagnostic, IR, semantic-types
and H2/H5/H6 golden tests are included in the full suite without reblessing.
The rebuilt release CLI also matches every baseline inspection byte on all four
targets (`target/m4-semantic-types-parity.json`).

Development validation first exposed two errors in the new test harness: stdout
was captured instead of the inspector's stderr, and extension diagnostics were
incorrectly assumed to disappear in GNU mode or contribute to `unit.errors`.
The snapshots were recaptured from the saved baseline; assertions now check the
existing shared policy severity and count in both modes. Compiler behavior was
not changed to satisfy those assertions. Development failures remain in
`target/m4-all-targets-final.log`, `target/m4-all-targets-final-2.log` and
`target/m4-policy-diagnosis.log`.

Final command results are recorded below. The new M4 arena-only
compilation case reports zero global allocations. Linux native execution and
Linux-host Clippy are not run on this Windows host. No OS-specific lint suppression
was introduced by M4. GNU extensions still report Warn/Deny policy diagnostics
in GNU mode; enabling GNU syntax does not disable pedantic diagnostics.

| Command | Result | Log |
| --- | --- | --- |
| `cargo test --all-targets` | PASS | `target/m4-all-targets-final-3.log` |
| `cargo test --features benchmarking-internals --test allocation_count` | PASS | `target/m4-allocation-final.log` |
| `cargo +nightly fmt --check` | PASS | `target/m4-fmt-check-final-2.log` |
| `cargo clippy --all-targets -- -D warnings` | PASS | `target/m4-clippy-final-3.log` |
| `cargo clippy --release --all-targets -- -D warnings` | PASS | `target/m4-clippy-release-final-2.log` |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | PASS | `target/m4-clippy-feature-final.log` |
| `git diff --check` | PASS | `target/m4-diff-check-final.log` |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | PASS | `target/m4-difftest-final-current.log` |
| `cargo build --bin bcc-rust` | PASS | `target/m4-plain-cli-final.log` |

`cargo +nightly fmt` ran before the final formatting check. The full suite
passes 1,875 tests across 20 binaries, with zero failures and six intentionally
ignored tests; its library phase passes 1,708 tests with three ignored. The
feature-enabled allocation suite passes 30 tests. The 1,000-case difftest command
passes six tests with its two longer tests intentionally ignored.
The plain CLI was rebuilt after all feature builds and final tests.
No feature-enabled build ran concurrently with CLI integration tests.

## Matched GCC torture survey

A fresh release CLI was built from clean `16ef56a6` before editing and saved as
`target/m4-baseline/bcc-rust.exe`. Its SHA-512 is
`0ac416ef4eb635f9204ad011e9282faccd0c78a3557beb6b583aa73e4e4a30bd70ff75dcc0f67ecbcacffbd726174180a16357ba66f4a3440bfa9feaf8d03a42`.
The old M3 survey records head `92098d53`, so a fresh M4 baseline was run even
though the saved binary matches the final M3 binary digest.

Baseline command:

```text
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-m4-baseline --bcc target/m4-baseline/bcc-rust.exe --jobs 8 --timeout 10
```

M4 command:

```text
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-m4 --jobs 8 --timeout 10
```

Both runs use the same 1,677 GCC 15.2.0 sources, Windows GNU target, eight jobs,
10-second timeout and four arms: interpreter, interpreter with bcc optimization,
LLVM O0, and bcc plus LLVM O2 codegen-only. Source hashes, options, status counts,
compiler identities and every changed source/arm classification are retained in
[semantic-facts-m4-survey.json](semantic-facts-m4-survey.json). This is the existing
single-file exploratory survey, not a complete DejaGnu run; unsupported options,
target selectors and additional-source directives remain recorded.

All 5,326 previously passing source/arm pairs still pass. There are zero lost
passes, gained passes or classification changes across the 6,708 source/arm
observations. Both surveys have zero compiler crashes and zero timeouts. Both
commands exit 1 because the existing exploratory corpus has nonpassing cases;
that exit status does not indicate a regression in this matched comparison.

| Arm | Baseline pass | M4 pass | Lost |
| --- | ---: | ---: | ---: |
| interpret | 1301 | 1301 | 0 |
| interpret-bcc | 1307 | 1307 | 0 |
| A0 | 1364 | 1364 | 0 |
| A1 | 1354 | 1354 | 0 |

The baseline completed in 539.764 seconds; M4 in 618.85 seconds.
These survey timings include separate background build activity and are not a
controlled performance measurement. Feature-enabled checks overlapped the M4
survey's release executable; no feature build overlapped CLI integration tests.
Baseline and M4 source hashes, applied/unsupported options, target, arm flags,
job count and timeout match exactly. The M4 release compiler SHA-512 is
`b8e93e6eb7178edeae1054ae35f147e61d7b210090db58575afd30ca8831146b4d391f4a3c9c243dd66391633dc32658429bb30f4364ff670e276565c045b527`.
Per-source results and command logs remain in
`target/compiler-corpus/execute-m4-baseline`,
`target/compiler-corpus/execute-m4` and `target/m4-survey-final.log`.
