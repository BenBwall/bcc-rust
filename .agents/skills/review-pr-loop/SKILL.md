---
name: review-pr-loop
description: Review and repair an open bcc-rust pull request in successive independent rounds, automatically committing and pushing each verified fix set and handing off to one successor until the review converges. Use for repeated review-and-fix work; use review-pr when the requested result must remain read-only.
---

# Review PR Loop

Drive one pull request toward convergence through a linear chain of fresh reviewers. Each agent owns exactly one complete review, repair, validation, commit, and push round. A round may spawn at most one successor.

Read [`../review-pr/SKILL.md`](../review-pr/SKILL.md) and its linked review contract completely before starting. Sections 1-4 define this skill's snapshot, cast, independent-pass, evidence, and consolidation rules. The workflow below replaces its read-only reporting stop: invoking `review-pr-loop` explicitly authorizes fixes to verified PR findings, focused regression tests, commits, and non-force pushes to the PR head branch. It does not authorize unrelated cleanup, force-pushes, merges, reviews or comments posted on GitHub, labels, or thread resolution.

## 1. Establish the round

Resolve the PR and require the same local/remote head alignment as `review-pr`. Record the chain's round number, with the first agent as round 1; inherit it from a predecessor handoff when present. Stop at five total rounds unless the user explicitly set another limit.

Inspect the working tree before review. Preserve every pre-existing change and untracked file. Continue with non-overlapping user changes only when commit-qualified review reads and path-specific staging can isolate this round's work; stop before editing when an overlap makes ownership ambiguous.

Treat predecessor notes as leads, not conclusions. Keep them away from independent specialist agents so each pass gets a fresh read of the committed snapshot.

## 2. Complete the review

Run the selected `review-pr` cast and finish the full evidence-gated review before editing. Do not let an early candidate truncate any specialist's assigned scope. After the independent passes, explicitly probe the predecessor's suggested risk areas, all code changed by the preceding fix commit, and any previously failed or unexecuted coverage.

Verify every candidate yourself and consolidate shared root causes. A predecessor finding is historical context: report it again only if the defect remains or the fix introduced a distinct problem.

## 3. Repair verified findings

If the round confirms no actionable findings, make no commit and declare the chain converged.

Otherwise, implement every verified in-scope finding that has a safe, bounded fix. Add focused regression coverage for each concrete behavior defect. Preserve the PR's intended scope and established design; request user direction instead of guessing when a fix requires a product decision, destructive migration, unrelated redesign, or changes outside the authorized PR branch.

Review the resulting diff for collateral changes and re-run targeted diagnostics plus every repository check required for the affected area. A check may be accepted only when it passes or its failure is demonstrated to be an unchanged baseline failure. Never weaken a gate to make the round pass.

## 4. Commit and push

Read the repository's commit policy. Stage only paths owned by this round, verify the staged diff, and create focused Conventional Commit commits. Do not amend or rewrite existing commits.

Push with an explicit non-force refspec from the checked-out `HEAD` to the recorded PR head branch. Verify that the remote PR head OID now equals local `HEAD`. On a commit, hook, test, authentication, rejection, remote-advance, or verification failure, stop the chain and report the exact state; never spawn a successor for an unpublished snapshot.

## 5. Decide whether the chain continues

Spawn a successor only when another review has a concrete chance to add value. A successful fix round normally earns one fresh confirmation round because the fix changed the reviewed snapshot. Other sufficient reasons are a named unresolved risk that can be investigated without user input or required coverage that a successor can execute with a concrete new approach.

Stop when any of these holds:

- a full required review confirms no actionable findings;
- the remaining concern needs user authority or a product decision;
- the next pass would merely repeat completed coverage without a new hypothesis;
- the commit or push did not complete and verify; or
- the chain reached its round limit.

Never spawn multiple successors, continue merely because more bugs are theoretically possible, or leave a successor running against an uncommitted or unpushed tree.

When continuing, read [`references/successor-handoff.md`](references/successor-handoff.md) and follow it exactly. After dispatching the successor, make no further repository changes and end the current agent's turn so the successor owns the next round.

## Final response

If stopping, summarize the cumulative fixes and commits from the available handoff history, current PR head OID, checks, completed review coverage, remaining open questions, and the precise stopping reason. If continuing, report the pushed commit and that ownership passed to the named successor.
