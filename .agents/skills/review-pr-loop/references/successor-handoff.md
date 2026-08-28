# Successor Handoff

Read this only after the current round's commit is pushed and the continuation gate is satisfied.

Spawn exactly one successor agent. Instruct it to use the absolute path to `review-pr-loop/SKILL.md`, perform one complete next round, and apply the same continuation gate when finished. Give it a compact baton containing:

- PR number, URL, title, intent, base OID, pushed head OID, and head branch;
- next round number and total round limit;
- each verified finding from this round in one line: severity, file, root cause, and implemented fix;
- commit OID(s) and the checks run with their results;
- failed, inapplicable, or unexecuted specialist coverage;
- open questions and assumptions worth challenging; and
- two or three specific search suggestions: risky callers, boundaries, state transitions, recovery paths, or adversarial inputs most likely to reveal the next defect, plus the review tactic that would test each one.

Keep the baton brief enough that it guides without anchoring the successor. Tell the successor to withhold these prior findings from its independent specialist prompts, inspect the whole PR rather than only the fixes, and treat every suggestion as an untrusted lead requiring fresh evidence.

Do not include patches, long transcripts, full specialist reports, or speculative claims presented as findings. The committed PR snapshot is the successor's source of truth.
