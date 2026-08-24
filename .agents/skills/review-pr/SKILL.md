---
name: review-pr
description: Review an open bcc-rust pull request with independent behavior, design, testing, error-recovery, and applicable compiler-phase specialists, then return only verified Markdown findings. Use for final, deep, or exhaustive PR review; this workflow is read-only and does not fix findings or mutate GitHub.
---

# Review PR

Coordinate the repo's independent review skills and synthesize one evidence-backed report. Read [references/review-contract.md](references/review-contract.md) completely before starting.

## 1. Resolve the committed PR snapshot

Accept an explicit PR number or URL; otherwise find the open PR associated with the checkout. Record the PR number, URL, title, body, base ref and OID, head ref and OID, changed paths, and CI state.

Require local `HEAD` to equal the remote PR head OID. An attached checkout must also be on the PR head branch. A detached checkout is acceptable only when its OID matches exactly. On mismatch, explain whether local is ahead, behind, or diverged and stop without switching, pulling, rebasing, or resetting.

Inspect `git status --short`. Warn about dirty paths but review the committed head snapshot; use commit-qualified reads wherever working-tree changes could affect evidence. Read `.agents/AGENTS.md` and the repository documents it routes to for the changed area.

## 2. Select the cast

Run these generic skills for every PR, allowing the specialist itself to return `Inapplicable` with a concrete reason for documentation-only, generated-only, or mechanical changes:

- `review-pr-behavior`
- `review-pr-design`
- `review-pr-testing`

For every non-trivial compiler PR, also run `review-pr-error-recovery`. Never omit it based only on a superficial diff scan; it may return `Inapplicable` after inspecting the changed fallible paths.

Add every compiler-phase skill whose implementation, inputs, outputs, shared types, diagnostics, provenance, or tests could be affected:

- `review-pr-lexical-processing` for initial processing and preprocessing-token formation, including translation phases 1-3.
- `review-pr-preprocessing` for directives, includes, conditional preprocessing, macro expansion, preprocessing expressions, and conversion to parser-facing tokens.
- `review-pr-parsing` for C language grammar, AST construction, parser-visible scope, parser control flow, and parser recovery.

There is no specialist-count cap. Announce the selected cast and a one-line reason for each phase specialist before dispatching it.

## 3. Run independent passes

Resolve the absolute path to each selected skill's `SKILL.md`. Dispatch one read-only subagent per skill, filling available slots and queuing later batches. Give every subagent only:

- its full skill path and instruction to use that skill;
- PR metadata and intent;
- base and head OIDs;
- changed paths; and
- the requirement to review the committed snapshot.

Each pass must remain independent and must not read existing PR review comments. Specialists return candidates under the shared contract and never edit, run broad suites, commit, push, post comments, submit reviews, add labels, or resolve threads.

Keep synthesis and user interaction in the Review Lead. If a selected pass cannot run, record it as `Not executed` or `Failed`; do not silently absorb its scope into another pass.

## 4. Verify and consolidate

Treat every candidate as an untrusted lead. Reproduce it against the head snapshot, inspect relevant callers and contracts, compare with the base when attribution is uncertain, and run only targeted diagnostics that materially resolve uncertainty. Deduplicate candidates sharing one root cause and fix while retaining all detecting specialists.

After independent verification, inspect unresolved PR threads and mark matching findings `Already reported`. Existing comments are duplication data, never evidence.

## 5. Report and stop

Return raw Markdown using the report format in the shared contract. Include every verified finding, numbered and sorted P0-P3, with a tight file and line location. Report completed, inapplicable, failed, and unexecuted coverage explicitly.

This is a review-only workflow. Stop after the report. Do not edit code, offer an implicit fix phase, post to GitHub, add labels, commit, or push unless the user makes a separate explicit request under an appropriate workflow.
