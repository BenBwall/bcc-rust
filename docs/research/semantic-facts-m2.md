# M2 occurrence, parameter and bound correspondence

Base: PR #42 head `bd188faf2f47239677139471a65774a196d51e00`, branch
`claude/semantic-facts-m2`. Implementation remains uncommitted in this worktree.
The approved design was read with
`git show origin/claude/semantic-facts-design:docs/design/semantic-facts.md`;
that branch resolves to `d6aa79b61ace166a54b0ab12518fe3b389b90943`.
M2 completion is recorded beside M1 in [middle-end.md](../../middle-end.md).
Producer/query contracts are in [semantic-analysis.md](../../semantic-analysis.md).

## Result and choices

Semantic analysis retains declaration occurrences, accepted binding/entity IDs,
initializer associations, prototype/body parameter correspondence, declared,
incoming/promoted and original parameter types, identity-indexed type names,
typed runtime bounds with size_t conversions, evaluation sites and dependencies,
and shared size formulas. Lowering schedules retained site actions and projects
formulas to SSA extents and multiplications. Semantic inspection reads parameter
static minima by binding ID. Missing required occurrences, initializer
associations, parameter correspondence, bounds, entry sites and formulas report `MissingFact` with provenance.
Rejected/recovered declarations retain an occurrence without an accepted binding.

The declarator `(name, source_vectors)` index and its query adapter are deleted,
along with inspection's source-vector minimum map, the external-declaration
initializer association walk, producer-side type-name provenance lookup and
runtime-size factor discovery from types. M1 had already deleted the consumer
runtime-sizeof type-name search, type-based bound scheduling and per-function
whole-TU parameter alias scan; those reconstructions remain absent. Redundant
retained parameter/type/binding-bound arrays are removed after consumers switch.

Choices made within M2:

- Occurrence ordinals are indexed by stable arena-owned suffix-list identity,
  including parenthesized suffix aliases. Copied declarator addresses and source
  vectors never supply identity. Type-name keys are borrowed through one typed
  helper so borrowed builtin operand slots cannot become accidental identities.
- Initializer syntax association is retained as the reference consumed today.
  Subobject placement plans remain M8 work; no second initializer checker was added.
- K&R entry actions retain declaration-list written order, separately from
  identifier-list incoming parameter order. Modern parameters retain their own
  entry sites directly; both forms have ordered `ParameterFacts` and explicit holes.
- Size formulas are finalized in one canonical-type pass after nominal layouts
  complete. This supports arrays formed before their record element is completed,
  shares each child DAG, and avoids rebuilding formulas at consumer queries.
  Constant-array product nodes preserve existing VLA IR instruction order; a
  separate cached constant size preserves ordinary fixed-size emission.
- Bound dependencies include saved extents used by size queries and pointer
  strides, including embedded statement expressions. SSA values remain lowering
  state, and distinct bound derivations remain distinct source evaluations.

Compiler changes add 1,229 and delete 334 Rust lines across 25 files.
Tests add 357 and delete 15 lines across 9 files. These counts use
`git diff --numstat` plus new Rust files, excluding docs and survey data.
No shared LLVM tree, script or Git metadata was changed.

## Coverage and work

New retained-interface tests cover prototype, K&R, unnamed, register and shadowed
parameters on Linux GNU, Linux musl, Windows GNU and Windows MSVC data models;
static minimum binding correspondence; copied declarators and initializer
associations; rejected declarations and definitions; typedef extent/formula reuse;
casts, runtime and constant size queries; statement-expression and stride bound
dependencies; and missing-fact provenance, including absent facts for unused parameters. The new
feature-enabled allocation test passes with zero global allocator calls for the
retained occurrence, parameter and size records.

Nine new execution programs cover entry-bound effects, parameter adjustment,
K&R incoming conversions, unnamed parameters, typedef extent reuse,
multidimensional declaration effects, VM casts/type operands, suppressed constant
sizeof effects, multidimensional subscript/increment/difference strides, and
runtime sizeof operand effects. Each runs in eight interpreter combinations:
Linux GNU and Windows GNU, optimization off/on, before/after ABI lowering.
Each also runs in four host native bcc arms, with bcc optimization off/on and
LLVM O0/O2, against bundled Clang 23.1.1 O0/O2 on the original source.
Existing H10 parameter-bound controls, including K&R bounds, remain in the suite.
Linux native execution is not tested on this Windows host.

The generated work control has 10,000 functions, 20,000 incoming parameters,
70,000 declaration occurrences and 120,000 bindings. It counts actual occurrence
construction and lowering alias visits, bounding the former by binding count and
the latter by parameter count. It also records whole-TU arena used/committed bytes;
these include source, syntax and all semantic records, not just M2 facts.
Observed work: **70,000 construction visits and 20,000 alias visits**.
Whole-TU used bytes: **219,921,648**; committed bytes: **220,581,888**.
Scratch peak residency is not measured. Full counters are retained in
`target/m2-work-counters-final.log`.

Existing IR, diagnostic and `--semantic-types` goldens are unchanged.

## Fresh matched GCC execute survey

The baseline release CLI was built from the clean bd188faf worktree before
production edits and copied to `target/m2-baseline/bcc-rust.exe`. The M2 release
CLI is built from the final implementation. Both surveys use the unchanged
harness, GCC 15.2.0 execute corpus, Windows GNU target, eight jobs and a 10-second
deadline.

```powershell
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-m2-baseline --bcc target/m2-baseline/bcc-rust.exe --jobs 8 --timeout 10
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-m2 --jobs 8 --timeout 10
```

All 1,677 source hashes and applied per-source flags match. Every previously
passing source/arm pair still passes: **5,326 preserved, zero lost passes and
zero gained passes**. Neither run has a compiler crash. Both survey commands
exit 1 because existing nonpassing programs remain.

| Arm | Baseline passes | M2 passes |
| --- | ---: | ---: |
| interpret | 1,301 | 1,301 |
| interpret-bcc | 1,307 | 1,307 |
| A0 | 1,364 | 1,364 |
| A1 | 1,354 | 1,354 |

The only two status changes are `pr36339.c` A0/A1: baseline native execution
exceeded 10 seconds; M2 execution exits with access violation `0xC0000005`.
Compilation succeeded in both surveys. The source uses `unsigned long` as a
pointer-sized integer; on this Win64 target both compilers emit `ptrtoint.i32`
for the 64-bit pointer. Their emitted IR is byte-for-byte identical, with SHA-512
`e939b1d500ebc8bda4500020eb351292eb7a83e7dd0426fdf6f3dd410e5da00b81cbf53aa1f53b4995d7bcbc37cf407843238d591838bf113b40b05f51a9a933`.
This evidence bounds the difference to execution status of an already nonpassing
source; no compiler correction or corpus edit is claimed. Both original survey
statuses and the unchanged-IR control remain in the machine-readable receipt.

Baseline: 2026-10-13T17:40:02.641126+00:00; 433.636 seconds.
M2: 2026-10-13T20:24:50.598032+00:00; 638.41 seconds.
The [machine-readable receipt](semantic-facts-m2-survey.json) retains both
manifests, compiler SHA-512 digests, modified Rust-file fingerprint, host, arms,
flags, deadlines and every source hash with baseline/M2 statuses. Full command
and output evidence remains in the two `target/compiler-corpus/execute-m2*`
directories. Survey elapsed times are exploratory, not performance measurements.

## Final checks

| Command | Result |
| --- | --- |
| `cargo test --all-targets` | Pass: 1,855 tests, six existing ignores; library 1,688 passed. |
| `cargo test --features benchmarking-internals --test allocation_count` | Pass: 28 tests. |
| `cargo +nightly fmt`, then `cargo +nightly fmt --check` | Pass. |
| `cargo clippy --all-targets -- -D warnings` | Pass. |
| `cargo clippy --release --all-targets -- -D warnings` | Pass. |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | Pass. |
| `git diff --check` | Pass. |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | Pass: six tests, two existing ignores. |
| `cargo build --bin bcc-rust` after feature checks | Pass: plain CLI restored after feature checks and after the survey. |

The final full-suite rerun includes the last unused-parameter missing-fact guard.
Earlier vector lookup regressions were corrected by borrowing the type-name node
through the typed identity helper; both existing tests pass. The failed development
run is archived as `target/m2-all-targets-identity-regression.log`.

Logs and command exit receipts are in
`target/m2-*-final.log` and `target/m2-checks.json`. Feature builds run after CLI
integration tests finish. The plain CLI is rebuilt after feature checks.
All compiled-program execution uses the protected runners. Linux-native Clippy
is not run locally; code avoids host-width casts and platform-dependent unused
expectations. The whole-file commit plan is `target/commit-plan.md`.
