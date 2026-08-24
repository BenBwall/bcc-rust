---
name: review-pr-testing
description: Review a bcc-rust PR's tests and regression protection against the behavior actually changed. Use independently for test review or as the mandatory testing pass selected by review-pr; report missing tests only for concrete demonstrated risks.
---

# Review PR Testing

Read [the shared review contract](../review-pr/references/review-contract.md) completely and apply it to this scope.

Map each materially changed behavior, invariant, diagnostic, and failure mode to test evidence. Review whether tests would fail for a realistic wrong implementation rather than merely restating internal structure.

Look for:

- changed behavior with no regression that reaches the relevant public or phase boundary;
- missing malformed-input, boundary, EOF, nesting, state-restoration, and continuation cases tied to a concrete risk;
- assertions that check only that an error exists while ignoring its kind, location, cascades, recovered output, or later progress;
- valid neighboring inputs that share the same token or state shape as a recovery case and could be corrupted by the fix;
- tautological assertions, brittle snapshots, excessive mocking, or helpers that duplicate production logic;
- tests that exercise only a helper while the integration seam remains unproved; and
- completion or compliance claims whose stated matrix is still materially uncovered.

For compiler recovery changes, expect paired malformed and minimally different valid cases plus assertions for termination, primary diagnostics, forbidden cascades, source provenance, recovered representation, and the next valid construct. Coordinate ownership through deduplication: this skill owns the sufficiency of regression evidence, while phase specialists own the underlying C contract.

A missing test is a finding only when the unprotected regression scenario is concrete and the test would materially reduce that risk. Complete only after every materially changed behavior is mapped to adequate evidence or an explicit candidate.
