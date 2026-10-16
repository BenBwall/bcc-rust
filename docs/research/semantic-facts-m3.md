# M3 resolved expressions, calls, tails and members

Base: PR #53 head `92098d53ed2a949a71d2434c0585d35d31217f6a`, branch
`claude/semantic-facts-m2`, containing M1 and M2. M3 landed on that branch as
`d30d4801..6b5327c0`. The approved design was read with
`git show origin/claude/semantic-facts-design:docs/design/semantic-facts.md`;
that branch resolves to `d6aa79b61ace166a54b0ab12518fe3b389b90943`.
This implements the M3 migration row and the M3 portion of C09–C12, C31 and C32.
M3 completion is recorded beside M1/M2 in [middle-end.md](../../middle-end.md).
Producer/query contracts are in [semantic-analysis.md](../../semantic-analysis.md).

## Result and choices

Semantic analysis retains a normalized selected leaf and binding for every
expression, including whether a child conversion intervenes. Call records retain
direct entity/binding, indirect expression ordinal or builtin binding, the
point-of-use function type, fixed prefix, prototype/ellipsis flags and argument
expression ordinals with converted types. GNU block values retain the block,
outer labeled value statement, leaf expression, prefix length and converted
result type. Member records retain owning tag and field index, qualified access
type, alignment and bit-field width. Existing semantic field records retain
offsets, storage and packing information; no second layout producer was added.

Lowering consumes these records. The public selected-designator and tail walkers,
the repeated direct-callee chain/conversion/binding checks, selected-field wrapper
peeling and owning-record recovery, address-taken designation walks, and GNU
lowering's tail/prefix reconstruction are deleted. The redundant relative field
index in `ExpressionInfo` is removed. Selected-field queries are two expression
ordinal lookups followed by tag/field indexing. The remaining selected-designator
and tail helpers are constant-time queries private to semantic analysis. Missing
required designations, calls, arguments, members and block-value facts report
`MissingFact` with provenance, including static member-address emission.

Choices made within M3:

- Dense designations use the existing expression ordinals; sparse calls and
  members use the same ordinals. Syntax references remain arena-owned. Scratch
  conversion summaries retain count, first kind and final type, so call
  classification does not scan conversion arrays or selected chains.
- A wrapper copies only its immediate completed child's designation. A statement
  completion continuation forwards only its immediate labeled child's tail.
  Neither construction nor queries recurse or perform repeated suffix walks.
- Implicit function declarations must precede identifier typing. Ordinary
  discovery forwards call context through immediate parentheses/`__extension__`
  children instead of attempting a premature lookup in completed facts. C89/GNU
  acceptance and strict C99 rejection are preserved.
- Directness requires the existing single function-decay conversion, identifier
  function binding and absence of intervening child conversions. Pointer values,
  casts, address/dereference, comma/conditional expressions and converted block
  values remain indirect. Entity identity uses M2's existing linkage-aware
  producer. Later composite prototypes do not replace occurrence-local call
  types or argument conversions.
- Builtins retain their kind and binding now. Existing name dispatch, operand
  evaluation rules and synthetic varargs adapters remain explicit M4/M6 bridges.
  An overloaded builtin may lack an ordinary function type; an ordinary lowerable
  call requires one. Builtin direct eligibility is retained separately for the
  existing library fallback. Returns-twice refusals and open-external call policy
  remain downstream capabilities/policies until M4/M7.
- A value tail retains its outer labels for execution. A final nested compound or
  declaration has no value tail and keeps the entire block as the prefix; an
  expression of void type still retains its tail. Existing body-effect summaries,
  aggregate snapshots and M1 lifetime transitions remain authoritative.
- Member facts copy access decisions beside existing member typing. Record
  layout, `#pragma pack` and integer-promotion algorithms are untouched.

Compiler changes add 721 and delete 304 Rust lines
across 23 files. Tests add 662 and delete 4
lines across 9 files. Counts use `git diff --numstat` plus new Rust
files, excluding docs and survey data. No shared LLVM tree, script or Git metadata
was changed.

## Coverage and work

New retained-interface tests cover each wrapper individually and in combination
for ordinary functions, scalar builtins and named varargs builtins; pointer and
conversion-changing callees; intervening child/extra outer conversions;
point-of-use unprototyped calls followed by later prototypes; default argument
promotions and ellipsis; implicit declarations in accepted and rejected modes;
empty, declaration, nested-block, void and labeled array tails; owning records
with equal member spellings; const/volatile accesses, alignment and bit-fields;
and missing-fact provenance. The new feature-enabled allocation case checks the
same families without global allocator calls.

Four execution programs cover all selection wrappers, conservative indirect
forms, selected `va_start`/`va_copy`/`va_end` with keyword `va_arg`, void and labeled
tails, selected address-taken locals, qualified members and bit-fields. Each runs
in eight interpreter combinations: Linux GNU and Windows GNU, optimization
off/on, before/after ABI lowering. Each also runs in four host native bcc arms,
with bcc optimization off/on and LLVM O0/O2, against bundled Clang 23.1.1 O0/O2
on the original source. Existing H4/H6 regressions remain in the full suite.
Linux native execution is not tested on this Windows host.

The generated work control mixes parentheses, `__extension__`, `_Generic` and
`__builtin_choose_expr` wrappers with labeled tails at depths 128, 2,048 and
16,384. It counts actual designation reductions plus immediate-child queries,
bounded by twice the expression count, and actual statement-tail reductions,
equal to the statement count. Whole-TU arena used/committed bytes include source,
syntax and all semantic facts, not only M3 records. Scratch peak residency is not
measured. Observations are retained in `target/m3-work-counters-final.log`:

| Depth | Expressions | Statements | Designation steps | Tail steps | TU used bytes | TU committed bytes |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 128 | 357 | 133 | 581 | 133 | 339,296 | 524,288 |
| 2,048 | 5,637 | 2,053 | 9,221 | 2,053 | 4,711,808 | 5,242,880 |
| 16,384 | 45,061 | 16,389 | 73,733 | 16,389 | 37,429,456 | 37,924,864 |

Existing IR, diagnostic and `--semantic-types` goldens are unchanged.

## Fresh matched GCC execute survey

The baseline release CLI was built from the clean 92098d53 worktree before
production edits and copied to `target/m3-baseline/bcc-rust.exe`. Its clean Git
identity and digest are retained in `target/m3-baseline/identity.json`. The M3
release CLI is built from the final production implementation. Both surveys use
the unchanged harness, GCC 15.2.0 execute corpus, Windows GNU target, eight jobs
and a 10-second deadline.

```powershell
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-m3-baseline --bcc target/m3-baseline/bcc-rust.exe --jobs 8 --timeout 10
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-m3 --jobs 8 --timeout 10
```

All 1,677 source hashes and applied per-source flags match. Every previously
passing source/arm pair still passes: **5,326 preserved, zero lost passes and
zero gained passes**. Every final status matches the baseline, including existing
nonpassing sources. Neither run has a compiler crash. Both baseline/final survey
commands exit 1 because existing nonpassing programs remain.

| Arm | Baseline passes | M3 passes |
| --- | ---: | ---: |
| interpret | 1,301 | 1,301 |
| interpret-bcc | 1,307 | 1,307 |
| A0 | 1,364 | 1,364 |
| A1 | 1,354 | 1,354 |

The first M3 survey overlapped the final native test suite and recorded one
compilation timeout: `20000412-4.c` A0 exceeded the 10-second compiler-subprocess
deadline (10.0418 seconds). That run is preserved in
`target/compiler-corpus/execute-m3-development` and
`target/m3-survey-development.log`. After the tests and checks finished, the
entire survey was repeated with the **same compiler digest, sources, flags, eight
jobs and 10-second deadline**. The A0 control compiles in 1.1383 seconds and exits
0; all 5,326 baseline passes are preserved in the complete rerun. The receipt
retains baseline/development/final control outcomes. No code or deadline change
was made to clear the timeout.

Baseline: 2026-10-13T21:24:41.432988+00:00; 782.297 seconds.
M3: 2026-10-13T22:36:51.429707+00:00; 445.809 seconds.
The [machine-readable receipt](semantic-facts-m3-survey.json) retains both
manifests, compiler SHA-512 digests, modified Rust-file fingerprint, host, arms,
flags, deadlines and every source hash with baseline/M3 statuses. Full command
and output evidence remains in the two `target/compiler-corpus/execute-m3*`
directories. Survey elapsed times are exploratory, not performance measurements.

## Final checks

| Command | Result |
| --- | --- |
| `cargo test --all-targets` | Pass: 1,867 tests, six existing ignores; library 1,700 passed. |
| `cargo test --features benchmarking-internals --test allocation_count` | Pass: 29 tests; M3 case has zero global allocator calls. |
| `cargo +nightly fmt` | Pass. |
| `cargo +nightly fmt --check` | Pass. |
| `cargo clippy --all-targets -- -D warnings` | Pass. |
| `cargo clippy --release --all-targets -- -D warnings` | Pass. |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | Pass. |
| `git diff --check` | Pass. |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | Pass: six tests, two existing ignores. |
| `cargo build --bin bcc-rust` | Pass: plain CLI restored after feature checks and after the final survey. |

The initial development suite exposed a test-mode error: the new implicit-call
acceptance assertion used strict C99. The test now explicitly checks C89/GNU
acceptance and strict C99 rejection for each transparent wrapper. That failed
development run remains in `target/m3-all-targets.log`; final evidence is in
`target/m3-all-targets-final.log`.

Logs and command exit receipts are in `target/m3-*-final.log` and
`target/m3-checks.json`. Feature builds run after CLI integration tests finish.
The plain CLI is rebuilt after feature checks. All compiled-program execution
uses the protected runners. Linux-native Clippy is not run locally; code uses no
platform-specific lint expectations or unchecked host-width casts. The whole-file
commit plan is `target/commit-plan.md`.
