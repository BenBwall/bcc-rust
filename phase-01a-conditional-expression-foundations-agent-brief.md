# ASAP slice: Conditional-expression foundations

## Objective

Deliver one coherent expression-foundations milestone across these repositories:

- `C:\Users\benbw\Documents\GitRepo\bcc-rust`
- `C:\Users\benbw\Documents\GitRepo\double-e-parsing` (the Double-E infix demo)

The milestone adds a reusable last-entry guard, fixes the verified nested-ternary defect in both evaluators, and advances the C99 language-parser rewrite by repairing its expression syntax model.

Read `bcc-rust/CONTEXT.md` first. Consult the expression requirements in `bcc-rust/c99-parser-compliance-checklist.md` and Phase 00/01 of `bcc-rust/parser-stack-migration-plan.html`. This file is the authoritative scope for this implementation slice.

Expected size: roughly 2–4 focused days for one agent.

## Required behavior

Both evaluators must group conditional expressions correctly:

```text
1 ? 0 ? 2 : 3 : 4  => 1 ? (0 ? 2 : 3) : 4 => 3
0 ? 1 : 1 ? 2 : 3  => 0 ? 1 : (1 ? 2 : 3) => 2
```

The operand stack already uses the correct LIFO order. The defect is lost `?`/`:` pairing. Use two operator states:

- an unmatched question marker that blocks reduction;
- a completed conditional operator that reduces three operands.

When `:` arrives, reduce the middle expression until the nearest unmatched question marker in the current parenthesis group. Replace that marker in place with the completed conditional operator. A colon cannot cross an opening-parenthesis marker.

## Step 1: Capture the red baselines

Run and record the current middle-nested ternary failure in both repositories before editing. Also record the currently correct right-nested case so the fix cannot regress it. In `bcc-rust`, drive the real preprocessor through `#if` comparisons that prove the selected numeric result. In the demo, add or run focused evaluator tests.

Also run `cargo check --message-format short` in `bcc-rust` and save the existing diagnostic set. The unfinished language parser is a known compile gate; the after-change result must add no unrelated compiler errors.

This step is complete when the middle-nested case is red in both repositories, the right-nested baseline is recorded, and the bcc-rust compile baseline is captured.

## Step 2: Add the bcc-rust vector guard

Add `bcc-rust/src/util/last_entry.rs` with this crate-private interface:

```rust
pub(crate) fn last_entry<T>(vector: &mut Vec<T>) -> Option<LastEntry<'_, T>>;

pub(crate) struct LastEntry<'vector, T> {
    // Private implementation.
}

impl<'vector, T> LastEntry<'vector, T> {
    pub(crate) fn get(&self) -> &T;
    pub(crate) fn get_mut(&mut self) -> &mut T;
    pub(crate) fn insert(&mut self, value: T) -> T;
    pub(crate) fn remove(self) -> T;
    pub(crate) fn into_mut(self) -> &'vector mut T;
}
```

The entry owns the mutable borrow of a non-empty vector. `insert` follows `hash_map::OccupiedEntry::insert`: replace the final element and return the old value. `remove` removes and returns the final element. Keep construction private and use no unsafe code.

Add `pub(crate) mod last_entry;` to `src/util.rs`. Unit tests in the new module must cover empty, `get`, `get_mut`, `into_mut`, `insert`, and `remove`, and prove that every operation preserves all preceding elements.

This step is complete when the full interface has focused passing tests and no parser or reducer type appears in the utility module.

If the bcc-rust compile gate prevents these self-contained tests from running through Cargo, run the module directly with `rustc --edition=2024 --test src/util/last_entry.rs` and report both the focused result and the separate crate-wide blocker.

## Step 3: Fix ternary pairing in the bcc-rust preprocessor

In `src/translation_phases/preprocessing.rs`:

1. Represent unmatched `?` and completed `?:` as distinct operator variants. Prefer clear names such as `QuestionMark` and `Conditional`; update metadata, token mapping, diagnostic formatting, and exhaustive matches consistently.
2. Give `:` its own token-loop branch. Reduce ordinary or completed operators above the nearest unmatched question marker, stopping at an opening parenthesis or an empty stack.
3. Use `last_entry` to inspect the remaining top slot and replace the matched question marker in place with the completed conditional operator.
4. Make only the completed conditional operator pop `final`, `middle`, and `condition`, then push the selected value.
5. Preserve recoverable diagnostics for malformed input. Report a colon with no matching question marker instead of crossing a parenthesis or panicking.

Add source-driven preprocessor regression tests for:

- `1 ? 0 ? 2 : 3 : 4` producing `3`;
- `0 ? 1 : 1 ? 2 : 3` producing `2`;
- parentheses isolating a conditional;
- an arithmetic operator in the middle expression reducing before `:`;
- `:` without a matching `?` producing a diagnostic rather than a panic.

Exercise these through `#if` branch selection so the tests reach the real preprocessor expression evaluator. If the legacy parser compile gate prevents the committed tests from running normally, use a temporary validation checkout or harness that excludes only the unfinished language-parser module; keep that validation-only change out of the implementation diff.

This step is complete when the original bcc-rust repro is green, both nesting directions are locked down, and parentheses remain a hard pairing boundary.

## Step 4: Apply the same algorithm to the Double-E demo

In `C:\Users\benbw\Documents\GitRepo\double-e-parsing`, update `src/parser.rs` so `?` is an unmatched marker and `:` converts the nearest valid marker into a completed conditional operator. Keep function calls and comma handling working.

Add evaluator tests for:

- both nested-ternary expressions from Required behavior;
- a conditional whose selected branch contains `exp(2, 3)`;
- a parenthesized nested conditional;
- the existing arithmetic, unary, call, and simple-conditional cases.

Run the demo's complete test suite.

This step is complete when `cargo test` passes in the demo and the fix uses the same pairing model as bcc-rust.

## Step 5: Advance the C99 language-parser expression model

In `bcc-rust/src/translation_phases/parsing.rs`, make the existing expression arena a syntax model that the future stack-based `ExpressionFrame` can construct:

1. Remove the required parse-time `Expression.result_type: TypeIndex`. Types belong to later semantic analysis.
2. Store `SourceVectors` on each `Expression` node.
3. Change `ExpressionType::Call.arguments` from `VectorSlice<Expression>` to `VectorSlice<ExpressionIndex>`.
4. Add a dedicated `Vec<ExpressionIndex>` list arena to `Parser` and initialize it in `Parser::new`.
5. Add explicit direct-member and indirect-member expression forms for C99 `postfix-expression . identifier` and `postfix-expression -> identifier`.

Keep this step to the expression syntax model. The later `ExpressionFrame`, type analysis, member lookup, and complete postfix parser remain subsequent slices.

Update the relevant conditional-expression item in `c99-parser-compliance-checklist.md`: describe the question-marker/completed-operator pairing used by Double-E. Remove the implication that C requires a separate non-Double-E “three-part syntax” mechanism while retaining the standard's asymmetric middle and final operand grammar.

This step is complete when every model gap above is represented, the checklist matches the agreed algorithm, and the after-change bcc-rust compiler diagnostics contain no new error attributable to this slice.

## Step 6: Verify and hand off

Run:

- focused `last_entry` tests;
- the bcc-rust preprocessor ternary regression loop in the validation setup described above;
- `cargo test` in `double-e-parsing`;
- `cargo fmt --check` in both repositories;
- `cargo check --message-format short` in bcc-rust and compare it with the captured baseline.

The handoff must report:

- changed files in each repository;
- the red-before and green-after ternary results;
- every verification command and result;
- the bcc-rust compile diagnostic comparison;
- any remaining known limitation within this slice.

## Scope boundary

This slice owns the vector guard, ternary pairing in both existing evaluators, and the five listed language-parser expression-model changes. Keep the remaining recursive-parser compile failures, reducer extraction, full `ParserMachine`, `ExpressionFrame`, semantic typing, and other C99 productions in later slices.

## Done

- The bcc-rust preprocessor selects branches proving results of `3` and `2` for the two nested forms and has regression coverage.
- The Double-E demo returns the same results and its complete suite passes.
- `last_entry` has its complete tested interface and is used for bcc-rust's in-place marker conversion.
- The old language-parser expression model is syntax-only, source-mapped, index-list-safe, and includes both C99 member-access forms.
- The C99 checklist describes the agreed Double-E ternary mechanism accurately.
- Verification evidence and baseline comparison are included in the handoff.
