# Review Contract

This is the single finding and reporting contract for the `review-pr-*` suite.

## Specialist contract

Review the committed PR snapshot independently within the skill's assigned scope.

- Treat the head commit as source of truth. Use commit-qualified reads when a dirty working tree could affect inspection.
- Inspect the diff, then enough surrounding code, callers, tests, configuration, history, repository instructions, domain docs, and authoritative standards to establish behavior.
- Report only issues introduced or materially exposed by the PR.
- Prefer no candidate over speculation. Put plausible unresolved risks under `Open questions`.
- Exclude pure preferences, unrelated pre-existing debt, and issues completely enforced by existing automation.
- Do not read existing PR review comments during the independent pass.
- Stay read-only. Do not edit, commit, push, post comments, submit reviews, label, or resolve threads.
- Avoid broad test suites. Suggest a targeted diagnostic when it would resolve uncertainty.
- Continue through the whole assigned scope after finding an issue.

Return `Inapplicable: <concrete reason>` only after verifying the scope has no meaningful bearing on the change. Otherwise return every candidate using:

```text
Proposed severity: P0 | P1 | P2 | P3
Title: imperative, at most 80 characters
Location: path and tight changed-line range
Scenario: concrete input or sequence that triggers the problem
Impact: observable consequence or material change cost
Evidence: code-level reasoning and the violated contract
Fix direction: concise and bounded, not a patch
Confidence: high | medium | low
```

If nothing qualifies, return `No candidate findings.` followed by a concise account of the surfaces and edge cases inspected.

## Evidence gate for the Review Lead

A final finding must:

1. Be introduced or materially exposed by the PR.
2. Identify a concrete reachable scenario.
3. Explain an observable impact or material maintenance risk.
4. Point to the smallest useful changed-line range causing the issue.
5. Be independently verified against the committed head snapshot.
6. Give a practical fix direction without demanding unrelated redesign.

Reject candidates that fail any gate. Move a plausible but unresolved concern to `Open questions`. Generated output, automated scanners, prior reviews, and specialist claims are leads rather than proof.

## Severity

- **P0 — catastrophic:** exploitable compromise, irreversible data loss, or system-wide outage; release-blocking and broadly reproducible.
- **P1 — serious:** likely correctness or compatibility failure with substantial impact, nontermination, compiler crash on ordinary input, or widespread output corruption; normally blocks merge.
- **P2 — concrete:** bounded bug, demonstrated diagnostic/recovery defect, missing regression protection for a concrete risk, or material design problem worth fixing before or soon after merge.
- **P3 — limited:** valid low-impact improvement or localized design debt; optional and never a pure preference.

Severity measures impact and urgency, not confidence. Low-confidence candidates do not become findings solely because their hypothetical impact is high.

## Consolidation

Merge candidates only when they share the same root cause and fix. Preserve all detecting specialists. Keep findings separate when either can occur independently or requires a different fix. Prefer a false split over a merge that hides distinct impact.

After verification, inspect unresolved PR threads. Mark a matching final finding `Already reported` with a URL when useful. Do not copy another reviewer's wording or let a thread substitute for verification.

## Final finding format

```text
P1 #1  src/path.rs:42  Preserve the caller-owned closing delimiter
Specialists: Error recovery, Parsing
Scenario: A declaration in a for header omits its semicolon before the closing parenthesis.
Impact: Recovery consumes the loop boundary and reparses the body outside the loop.
Evidence: The child scanner accepts `)` even though the parent frame owns it.
Fix direction: Propagate the parent boundary through the deferred child and add the paired regression.
```

Use a tight location and sort by P0, P1, P2, then P3; within a severity, sort by impact and then path. Include every verified finding rather than imposing an arbitrary count limit.

## Report format

```text
PR #<n> — <title>
Review cast: <selected skills>
CI: <concise state>

Findings
<numbered findings, or "No actionable findings were confirmed within the completed coverage.">

Already reported
<matching unresolved threads; omit when empty>

Open questions
<unresolved risks or missing context; omit when empty>

Coverage
Completed: <skills>
Inapplicable: <skill — reason>
Not executed: <skill — reason>
Failed: <skill — reason>
Limitations: <missing evidence or diagnostics>
```

Never describe a review as clean when required coverage failed, was not executed, or lacked a necessary evidence surface.
