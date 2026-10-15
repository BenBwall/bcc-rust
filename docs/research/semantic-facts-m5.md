# M5 retained expression effects and evaluation recipes

Base: `claude/semantic-facts-m2` at `5494be2284e61cab467023b2d1054ca29f6ae088`,
containing M1-M4. The approved design was read with
`git show origin/claude/semantic-facts-design:docs/design/semantic-facts.md`.
This implements migration row M5 and inventory rows C07, C08 and B11.
Completion is recorded beside M1-M4 in [middle-end.md](../../middle-end.md);
producer/query contracts are in [semantic-analysis.md](../../semantic-analysis.md).

## Result and choices

M1 had already replaced the lowering `effect_summaries` cache and the semantic
per-query `EffectSummaries::new` engine with a dense pair of purity booleans, so
at this head no fresh per-query purity cache remained to delete. What remained
was a summary too coarse to state what an evaluation does, and an effects-only
scheduler in `lowering/builtin_effects.rs` that re-matched operators, ICE state,
volatile qualifiers and selections to decide what to execute. M5 replaces both.

`semantic_analysis/facts/effects.rs` is the single producer. When
`retain_expression` runs, after the node's call and builtin-instance facts, it
summarizes two contexts: reading the value, and only designating it (address-only
or effects-only use). Each is an `EffectSet` (`WRITE`, `CALL`, `VOLATILE`,
`CONTROL`, `LIFETIME`, `UNKNOWN`) built from the immediate children's retained
records: O(1) per operand edge, no recursion, no suffix walk. After the ICE state
is final, the same postorder stores each context's effects-only recipe. The dense
record is four bytes per expression (a size pinned by a test). `None`, and
`EvaluateValue` or `StatementBlock` of the node itself, are dense recipe kinds;
`Sequence` (the node's own type-site actions, then child uses) and `Conditional`
(one condition branch, optional arm uses) live in a sparse map keyed by ordinal
and context. The old `builtins/effects.rs` (166 lines) is deleted.

Lowering's `builtin_effects.rs` is now a recipe interpreter. It no longer reads
operator kinds, ICE flags, volatile qualifiers or selections, and it consults no
purity predicate. A missing summary or recipe, or an unavailable recipe because a
child had no record, reports `MissingFact` with source provenance.

Choices made within M5:

- **Classification refinements.** These are deliberate, documented and behavior-
  preserving under the matched survey and every golden:
  - selected `_Generic`/`__builtin_choose_expr` arms forward their summary
    instead of being conservatively effectful;
  - checked builtins use the registry `BuiltinEffectClass` plus their evaluated
    operands;
  - a VLA `sizeof` uses its retained size-query operand and new bounds;
  - statement expressions summarize their body.

  Consequences: a short circuit or conditional in an effects-only operand skips
  a branch whose arm is now known to be effect-free, and
  `__builtin_*_overflow_p` folds when its third operand is now known to be
  effect-free.
- **Recipes mirror the previous schedule.** ICE nodes execute nothing; volatile
  reads evaluate; calls, stores and unmodeled forms evaluate and discard; casts
  keep their own bound actions even when their operand is pure. The one
  deliberate shape change is that a branch with one effect-free arm uses the
  two-block form short circuits already used. That now includes omitted
  conditionals. Execution is unchanged.
- **Statement-expression bodies.** A body summary opens with the block and
  closes at its expression. Each completed statement contributes its own
  expressions. Loops, jumps and returns add `CONTROL` (they may not complete
  normally); `if`/`switch` add only their controlling expression. Declarations,
  nested definitions and asm are `UNKNOWN`, as are compound-literal
  initializers and the temporary M6 builtin bridges. They are not summarized
  further.
- **Unchanged constant eligibility.** ICE operand validation (`valid_ice_operands`)
  is a constant-expression concern, not an effect cache. Constant eligibility
  stays separate. The existing rule clearing it for effectful cast type-site
  actions now reads the value summary.
- **Visibility.** `SemanticTranslationUnit::effect_summary` is retained for tests
  and later consumers (with a narrow dead-code expectation outside tests).
  `effect_recipe` is the lowering query.

Compiler changes add 1024 and delete 381 Rust lines across 17 files.
Tests add 399 and delete 11 Rust lines across 7 files, plus one 20-line C fixture.
Counts use `git diff --numstat 5494be22..HEAD`, excluding docs and survey data.
Shared LLVM junctions were not changed.

## Coverage and validation

`semantic_analysis/tests/effects.rs` queries the retained interface lowering
uses. Its classification table covers:

- calls, volatile reads versus `&v`, `&*vp` and `&p->vm`, and member bases;
- simple and compound stores, increments and decrements;
- statement expressions: write, call, labeled volatile tail, loop control, pure
  bodies and declarations;
- selected and unselected `_Generic`/`__builtin_choose_expr` arms;
- unevaluated `sizeof`/`typeof` operands and evaluated VLA `sizeof` operands and
  bounds;
- VM casts and typeof site actions;
- comma, conditional, omitted-conditional and short-circuit operands;
- registry builtins: pure, query, write, control, allocate, library, varargs, and
  overflow predicates with pure, volatile and call third operands.

Recipe tests pin:

- short-circuit and two-armed branches, and an omitted conditional reusing its
  condition;
- branch elision for effect-free arms, and volatile value versus designation
  recipes;
- sequence filtering, ICE suppression, and selected-arm forwarding;
- statement blocks, and a VM cast's action-only sequence.

The generated deep test nests short circuits, conditionals, generic selections and
statement expressions 128, 2,048 and 16,384 deep. Production steps stay below
four per expression plus one per statement:

| Depth | Expressions | Statements | Effect steps |
| ---: | ---: | ---: | ---: |
| 128 | 548 | 98 | 2,288 |
| 2,048 | 8,708 | 1,538 | 36,368 |
| 16,384 | 69,636 | 12,290 | 290,832 |

The existing deep overflow-predicate test now counts lowering recipe steps: 4,097
for a 2,048-deep `&&` chain (two per level, bounded at four). `lowering/tests/effects.rs`
removes summaries or recipes from a lowerable unit and checks for a
provenance-bearing `MissingFact`.

The new `tests/fixtures/semantic/selected-overflow-effects.c` executes:

- selected `_Generic` and choose-expression arms;
- unevaluated and VLA `sizeof`;
- omitted conditionals and short circuits in both directions;
- a loop in a statement expression and a comma.

GCC 15 in WSL returns 0 at `-O0` and `-O2`; Clang does not provide
`__builtin_add_overflow_p`. The fixture joins the scalar builtin program list, so it
runs in every interpreter/optimizer/verifier stage on two targets and in the
Windows and Linux native arms. The existing `labeled-overflow-effects.c` and
volatile-trap tests cover labeled blocks, jumps, switch bodies and
volatile tails unchanged. All existing IR, diagnostic, `--semantic-types` and
lowering goldens pass without reblessing.

## Matched GCC torture survey

A release CLI was built from clean `5494be22` before editing and saved as
`target/m5-baseline/bcc-rust.exe` (SHA-512
`c3ec439c5d182998f5fcf4018e0ac8954bf8a0944563787a617ee669c1dd35fb59b6323f54a60780f835a703e41b5dd228fec8c2c2439c24972e68248e384525`).

```text
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-m5-baseline --bcc target/m5-baseline/bcc-rust.exe --jobs 6 --timeout 10
python scripts/run_gcc_torture_execute.py --source C:\Users\benbw\Documents\GitRepo\bcc-rust\target\compiler-corpus\gcc-15.2.0 --output target/compiler-corpus/execute-m5-head --jobs 6 --timeout 10
```

The M5 run used a fresh release build of `7066971b`, the last compiler commit;
only Markdown files were modified or untracked at that point. Its SHA-512 is
`045805afe335a41cf4bddd458ab3669d56f318416d71fb92bc1fdf49da4c2fada2f3d9556bafcece8a3d66101d80acf4aee4fcc1a14e0003e3c66a061a1e536a`.
Later commits change only comments and documentation. An earlier run of
`80fbafa1`, before the lint-only `7066971b`, gave the same pass counts and also
lost no pass. Both runs use the same 1,677 GCC 15.2.0 sources, Windows GNU
target, six jobs, 10-second timeout and four arms: interpreter, interpreter with
bcc optimization, LLVM O0, and bcc plus LLVM O2 codegen-only. Source hashes,
applied and unsupported options and arm flags match exactly. This is the
existing single-file exploratory survey, not a DejaGnu run.

**All 5,326 previously passing source/arm pairs still pass**: zero lost and zero
gained passes, zero compiler crashes in both runs. Both commands exit 1 because
the exploratory corpus has nonpassing cases.

| Arm | Baseline pass | M5 pass | Lost |
| --- | ---: | ---: | ---: |
| interpret | 1301 | 1301 | 0 |
| interpret-bcc | 1307 | 1307 | 0 |
| A0 | 1364 | 1364 | 0 |
| A1 | 1354 | 1354 | 0 |

Twelve observations of eleven already-failing programs changed between
wrong-result and timeout. Ten went from wrong-result to timeout: A1 for
`20020508-3.c`, `20021127-1.c`, `20040409-2w.c`, `20040409-3w.c`, `eeprof-1.c`,
`pr22493-1.c`, `pr32244-1.c` and `pr99079.c`, and A0 for `pr32244-1.c` and
`pr34971.c`. Two A1 observations went the other way: `pr43629.c` and
`pushpop_macro.c`. Each program reaches `abort()`. In whichever run classified
it wrong-result, its fail-fast exit (`0xC0000409`) took 0.2-9.0 seconds, ten of
the twelve over 6.5 seconds, so termination fell on either side of the
10-second limit. The baseline and M5 compilers emit
byte-identical bcc IR (`--emit ir`), LLVM IR (`--emit llvm`) and bcc-optimized
LLVM IR (`--opt=bcc --emit llvm`) for all eleven. The change is therefore
termination timing on a loaded machine, not compiler output. All eleven were
already recorded as known failures in
[gcc-torture-execute-results.md](gcc-torture-execute-results.md).

The baseline took 670.2 seconds and M5 501.1 seconds. Other agents' builds shared
the machine, so these are not performance measurements. Metadata, per-source
hashes and classifications, counts and every changed classification are in
[semantic-facts-m5-survey.json](semantic-facts-m5-survey.json). Per-source results
remain in `target/compiler-corpus/execute-m5-baseline` and
`target/compiler-corpus/execute-m5-head`.

## Final checks

Final command results are below; logs are under `target/`. `cargo +nightly
fmt --check` first failed on a hand-wrapped module comment added during this
step (`m5-final-fmt.log`); the comment was rewrapped and the check rerun. That
commit changes only comments, so the other results stand.

| Command | Result | Log |
| --- | --- | --- |
| `cargo test --all-targets` | PASS | `target/m5-final-all-targets.log` |
| `cargo test --features benchmarking-internals --test allocation_count` | PASS | `target/m5-final-allocation.log` |
| `cargo +nightly fmt --check` | PASS | `target/m5-final-fmt-2.log` |
| `cargo clippy --all-targets -- -D warnings` | PASS | `target/m5-final-clippy.log` |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | PASS | `target/m5-final-clippy-feature.log` |
| `git diff --check` | PASS | `target/m5-final-diff-check.log` |

The full suite passes 1,879 tests across 20 binaries, with zero failures and six
intentionally ignored tests; its library phase passes 1,712 tests with three
ignored. The feature-enabled allocation suite passes 30 tests. Linux native
execution and Linux-host Clippy are not run on this Windows host.

## Review

One severity-gated review round read `5494be22..e15e9edb` for behavior, design,
tests and error recovery. It found no P0, P1 or realistic P2 issue. No consumer
re-derives a retained effect: lowering's only effects path is the recipe
interpreter, and the overflow-predicate fold in `builtins/common.rs` reads the
retained summary. The old purity pair, `effect_pure` query and lowering
classification have no remaining callers. Probes on a debug build (statement
expressions in `typeof`, `sizeof`, `_Generic`, file-scope, case-label, array-bound
and malformed positions) hit no assertion, and an effects probe matches GCC 15 at
`-O0` and `-O2`. Open low-severity items:

- `facts/effects.rs` `effect_use` drops a conditional arm that has no retained
  record instead of making the recipe `Unavailable`, unlike `sequence`. Only
  unretained (erroneous) children reach it, and those units are not lowered.
- `E::LabelAddress`, `Countof`, `Boolean` and `Nullptr` summarize as `UNKNOWN`,
  so such an operand blocks an overflow-predicate fold.
- The `dead_code` expectation reason on `effect_summary` contains a run of
  embedded spaces.
- `effects.rs` cites §6.7.5.2p5 as pp. 116-117 (PDF pp. 128-129); the paragraph
  is on p. 117 (PDF p. 129). Several older files share that citation.
