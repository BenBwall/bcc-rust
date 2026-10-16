# M1 semantic control and lifetime facts

Base: PR #42 head `f0be25633315f31c7c41fe776998fc08c486e258`, branch
`claude/semantic-facts-m1`. Implementation remains uncommitted in this worktree.
The approved design was read with
`git show origin/claude/semantic-facts-design:docs/design/semantic-facts.md`;
that design branch resolves to `d6aa79b61ace166a54b0ab12518fe3b389b90943`.
The proposal is absent from this branch. M1 completion is recorded in
[middle-end.md](../../middle-end.md); producer contracts are in
[semantic-analysis.md](../../semantic-analysis.md).

## Result and boundaries

M1 supplies typed control identities, lexical/VM/statement-expression regions,
resolved labels and body-only loop/switch targets, converted case/range endpoints,
and jump, dispatch and normal-exit transitions. Iterative heavy-light indexing
replaces ancestor scans. Lowering owns SSA stack tokens and IR blocks and consumes
the semantic facts. The old VLA scope index, declaration indexing walk, synthetic
region identities, label/parent reconstruction, switch case-discovery walk and
endpoint recasts are deleted.

The necessary M2/M5 slices retain source-ordered parameter typeof/bound actions
and one dual-context expression summary. Cast type-site actions have one retained
record consumed by semantic classification and both lowering schedules. Full M2
occurrence/formula graphs and M5 sparse evaluation recipes remain future work.
No struct/member/union alignment or System V classification logic was changed.

Compiler changes: 2,098 added and 997 deleted Rust lines across 28 files, including
889 lines in the new facts subsystem. Tests add 477 and delete four lines across
four files. These are `git diff --numstat` plus new Rust files, excluding generated
survey data and documentation. Nothing was staged or committed.

## Reproducers and lifetime coverage

All five supplied reproducers return zero in all eight interpreter combinations:
Linux GNU and Windows GNU, optimization off/on, before/after ABI lowering. Each
also returns zero in all four host native bcc arms, combining bcc optimization
off/on with LLVM O0/O2. Tests retain the original source text.

| Reproducer | Observed result |
| --- | --- |
| typeof dereference with internal goto, `p++`, and array result | Pointer advances once; `sizeof b` is 12; return 0. |
| typeof pointer result with internal goto and initialization | `p == &a`; return 0. |
| typeof operand with backward goto across an inner VLA | Inner allocation is released/reentered; `n == 1`; return 0. |
| overflow-p third operand containing a cast typeof action | `p++` executes once; return 0. |
| typeof in the third parameter declaration | Entry action increments `n` to 4; return 0. |

Bundled Clang 23.1.1 directly compiles and runs reproducers 1, 2, 3 and 5 at
O0/O2. It rejects the original fourth source with “unknown builtin” for
`__builtin_add_overflow_p`. Installed GCC directly compiles and runs that original
source at O0/O2. Clang additionally runs an effects control that defines the
unused predicate as `((void)(c),0)`, preserving the tested cast/typeof action.
This control is recorded separately from a direct Clang reference to the original.

Acceptance coverage includes H1 with an enclosing live VLA, forward/backward and
sibling gotos, two VM declarations in one block, VM typedef/pointer entry errors
paired with VLA execution and bounded storage, body-only header jump targets,
nested switches, converted case ranges, normal exits and for-init VLA exits,
mixed VLA/alloca, and aggregate statement-expression tail-result lifetime.
Additional actions tests cover typeof type-name bounds in declarations, casts and
parameters, parameter source order and short-circuit suppression of cast effects.
The existing alloca/VLA dangling-storage test remains unchanged and passes.

The existing 21-case header fixture executes against literal results and GCC
O0/O2. Its six-case Clang subset remains unchanged. Bundled Clang's documented
crashes/miscompilations for the other 15 cases remain excluded from that oracle;
the original evidence is [archived](semantic-control-pre-m1-history.md).
Native Linux execution remains untested on this Windows host; Linux GNU
interpretation and the platform-neutral source are tested.

Existing fixed-size IR goldens, the existing VLA IR golden and diagnostic goldens
pass without edits. New lifetime-sensitive emission follows corrected transitions:
typeof-operand labels share their enclosing region; backward jumps leave the VM
declarations after their destination; for-init natural and break exits each release
their VLA once; aggregate tail values are copied before their exit releases storage.
The stale semantic classification of all statement expressions as unanalyzed array
bounds was removed so executed sizeof bounds can enter this same graph.

## Work and retained memory

`m1_deep_region_construction_and_jump_queries_are_bounded` generates distinct
outward goto sources and a forward VM-entry error followed by repeated deepest
sources at depths 128, 2,048 and 16,384. Construction counters measure actual
region-index loop iterations; query counters measure light-edge and binary-search
steps. Tests bound construction by 5R and queries by their count times O(log R).
The deepest outward and inward patterns have:

| Pattern | Regions | Statements | Jumps | Construction steps | Query steps | TU used bytes | TU committed bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Outward | 32,769 | 32,773 | 16,384 | 163,845 | 49,157 | 39,558,640 | 39,559,168 |
| Inward | 32,769 | 32,774 | 16,385 | 163,845 | 49,176 | 39,543,296 | 39,546,880 |

TU bytes include source, syntax and all semantic records, not just the new facts.
Scratch peak residency was not measured. The retained fact records are checked
for no destruction requirements; the allocation test covers valid region/actions
production and VM-entry diagnostics with zero global allocations.
Full counter output is retained in `target/m1-work-counters.log`.

## Fresh matched GCC execute survey

The original release CLI was built from f0be2563 before production edits and copied
to `target/m1-baseline/bcc-rust.exe`. Its manifest records only the added red test
as a working-tree change. The M1 release CLI was built after the implementation
and before feature-enabled checks. Both surveys use the unchanged harness,
GCC 15.2.0 execute corpus, Windows GNU target, eight jobs and a 10-second deadline.

```powershell
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-m1-baseline --bcc target/m1-baseline/bcc-rust.exe --jobs 8 --timeout 10
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-m1 --jobs 8 --timeout 10
```

All 1,677 source hashes and applied per-source flags match. Every source's status
matches in every arm: **5,326 previously passing source/arm pairs preserved,
zero lost passes, zero gained passes, zero other status changes**. Neither run
has a compiler crash. Existing failure statuses remain recorded. Both survey
commands exit 1 because the harness reports existing wrong results/timeouts.

| Arm | Baseline passes | M1 passes |
| --- | ---: | ---: |
| interpret | 1,301 | 1,301 |
| interpret-bcc | 1,307 | 1,307 |
| A0 | 1,364 | 1,364 |
| A1 | 1,354 | 1,354 |

Baseline: 2026-10-13 11:34:45 UTC; 366.375 seconds. M1: 2026-10-13 12:44:45 UTC;
385.786 seconds. These exploratory elapsed times are not performance measurements.
The [machine-readable receipt](semantic-facts-m1-survey.json) retains both manifests,
full compiler SHA-512 digests, the modified Rust-file fingerprint, host, arms, flags,
deadlines and every source hash with baseline/M1 statuses. Full command/output
evidence remains under the two `target/compiler-corpus/execute-m1*` directories.

## Final checks

| Command | Result |
| --- | --- |
| `cargo test --all-targets` | Pass: 1,827 tests passed, six existing ignores; library 1,661 passed. |
| `cargo test --features benchmarking-internals --test allocation_count` | Pass: 27 tests. |
| `cargo +nightly fmt`, then `cargo +nightly fmt --check` | Pass. |
| `cargo clippy --all-targets -- -D warnings` | Pass. |
| `cargo clippy --release --all-targets -- -D warnings` | Pass. |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | Pass. |
| `git diff --check` | Pass. |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | Pass: six tests, two existing ignores. |
| `cargo build --bin bcc-rust` after feature checks | Plain CLI restored. |

Check logs are `target/m1-all-targets-final.log`, `target/m1-allocation.log`,
`target/m1-clippy-{debug,release,features}.log`, `target/m1-difftest.log` and
`target/m1-plain-cli.log`. Feature builds ran after CLI integration tests finished.
All compiled-program execution used the protected runners. Shared LLVM junctions
were preserved. Linux-native Clippy was not run locally; new code avoids
Windows-only lint assumptions and the required host Clippy configurations pass.
The whole-file commit plan is `target/commit-plan.md`.
