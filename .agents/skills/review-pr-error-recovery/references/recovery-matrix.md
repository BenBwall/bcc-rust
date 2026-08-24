# Compiler Recovery Matrix

[PR #3](https://github.com/BenBwall/bcc-rust/pull/3) established the repository's recovery evidence baseline: 35 of 39 review findings concerned recovery, synchronization, diagnostics, provenance, recovered output, or recovery-state handling. Those comments reduced to 16 recurring mechanisms, and eight findings showed recovery machinery corrupting valid C99. The dominant failure was a locally correct predicate that missed a sibling caller, a nested child seam, or valid syntax using the same token shape.

Use this matrix to prevent that review/fix/re-review cycle.

## 1. Inventory every fallible seam

For each changed path, name:

- the valid input and malformed prefix it recognizes;
- every caller or parent context;
- the unit owned by the child versus the caller: character, newline, preprocessing token, directive boundary, macro argument delimiter, parser token, or grammar delimiter;
- every nested child receiving recovery or continuation policy;
- state captured on entry and restored on success, recovery, early return, and EOF;
- what is consumed, preserved, synthesized, or reprocessed; and
- the diagnostic and output provenance produced after zero, one, or many units are consumed.

Do not accept labels such as “frame-specific recovery” or “skip invalid input” without the exact policy and every selection site.

## 2. Cross the state space

Review recovery as this product:

```text
malformed construct
  × caller context
  × candidate boundary
  × nested state
  × valid neighboring syntax
  × continuation and provenance
```

Relevant state varies by phase:

- **Lexical processing:** physical/logical newline, trigraph and splice lookahead, comment/string/header context, token prefix, EOF, and source-vector segments.
- **Preprocessing:** directive line, include and conditional-group stack, macro argument depth, disabled/replacing macro state, rescan position, token-paste/stringize context, and output-token conversion.
- **Parsing:** control-frame and phase, delimiter/question depth, scope and name classification, unmatched-owner state, synchronization kind, deferred child, and recovery unwind target.

For each malformed prefix, try every plausible successor or owner boundary at the same and nested depths. Include newline, comma, colon, semicolon, closing delimiters, braces, EOF, identifiers with context-dependent meaning, and a complete next construct for the phase.

## 3. Prove six invariants

### Progress

Every machine step must consume input, push, reduce or unwind, change phase, or transfer ownership before reprocessing the same unit. Exercise loops under a bounded driver and look for repeated zero-consumption output, unbounded allocation, recursion, or repeated diagnostics.

### Boundary ownership

A child may consume its malformed prefix but must leave a caller-owned boundary available to its owner. Propagate the caller's stop policy through deferred and nested children. Verify both directions: the boundary is preserved when owned by the caller and consumed when it belongs to the child.

### State restoration

Stacks, scopes, delimiters, conditional groups, macro-disable state, include state, and other contextual classifiers must return to the correct entry state on success, recovery, early exit, and EOF. Check nested instances; a boolean that works for the immediate child often loses transitive ownership one level deeper.

### Diagnostic quality

Assert the intended primary diagnostic kind, unexpected input or EOF, expected category, and original-source location. Check forbidden cascades and duplicate reports. Diagnostics that exist but point at source index zero, a macro expansion rather than its useful origin, or a swallowed successor do not satisfy the contract.

### Recovered output and provenance

Every reachable recovered node or token must be explicitly distinguishable from valid output and carry owned source vectors or an explicit zero-width anchor at the recovery cursor. Recovered provenance must not absorb a caller-owned delimiter or the following valid construct.

### Continuation and noninterference

Assert the identity and order of the next valid token, directive, declaration, statement, or external declaration. Recovery predicates also inspect valid input, so prove they do not split or swallow valid syntax.

## 4. Pair malformed cases with valid neighbors

Every heuristic distinguishing two meanings of the same input needs both sides, with the smallest possible change between them. Examples include:

- conditional colon versus label or directive colon;
- compound-literal or initializer brace versus a following body or compound;
- caller closing delimiter versus a nested child delimiter;
- visible typedef name versus member identifier or ordinary identifier;
- absent optional child versus a non-empty malformed child;
- comment opener, slash operator, and end-of-file prefix;
- directive newline versus whitespace internal to a macro argument; and
- pasted token that is valid versus the minimally different invalid paste.

For each pair, assert output structure and provenance in addition to diagnostics.

## 5. Audit each proposed fix laterally

When the PR introduces a one-off predicate, stop set, state flag, recovery kind, or special-case branch, immediately search for:

- sibling callers and synchronization policies;
- constructors, push sites, reductions, and unwind paths that do not receive or clear it;
- nested scopes where the state must remain local;
- valid forms sharing the recognized boundary;
- the inverse case where the unit must be consumed; and
- later phase consumers that assume a different recovered representation.

A test count is not evidence of coverage. Evidence is the set of matrix cells exercised and the assertions that prove all six invariants.
