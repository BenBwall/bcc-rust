# Control and effect history before M1

Archived from PR #42 head `f0be25633315f31c7c41fe776998fc08c486e258`.
These paragraphs describe earlier implementations and their historical
validation. Current contracts are in semantic-analysis.md and middle-end.md.

## Semantic label ancestry

Labels, gotos and switch contexts retain statement-expression ancestry.
Semantic analysis rejects entering an expression from outside or from a
sibling expression, with a primary jump/case diagnostic and a secondary label
at the expression. Internal and outward jumps remain legal, including nested
outward jumps. Function entry resets this state for nested functions.
The constraint comes from [GCC Statement Exprs](https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html),
extending C99 6.8.6.1p1 (printed p. 137, PDF p. 149) and 6.8.4.2p5
(printed p. 134, PDF p. 146).

## Semantic ancestry measurements

Statement-expression ancestry uses preorder subtree intervals in the semantic
scratch arena. Entering an expression assigns its index; its restore
continuation records the first index outside its subtree. Open intervals have
an unbounded end, so case/default checks during traversal use the same query
as resolved gotos. Testing whether the destination encloses the source now
costs O(1), without recursion or a cache whose cost depends on repeated pairs.
Function-local jump filtering and expression-source diagnostic labels retain
the round-1 contract. The generated regression checks both repeated deepest
sources and distinct sources at each nesting level, up to 32,768 expressions.
A test-only work counter replaces noisy timing assertions; the original walk
failed at depth 128 with 8,384 checks against a bound of 256.
The interval query performs exactly one check per goto: 32,768 checks for
each depth-32,768 pattern. Existing rejected-entry and legal-jump tests pass.

## Semantic tails and header contexts

### PR #41 round-3 corrections (2026-10-13)

A statement expression returns its tail expression statement's converted value:
arrays and function designators decay, lvalues lose their qualifiers and value
category, and bit-field identity does not escape. Struct/union results are values;
empty blocks, void expressions and other endings remain void. The ordinary
expression-statement visit already records these conversions, so result typing
uses the shared conversion classifier without adding duplicate records. Lowering
now evaluates the tail with those records. Enclosing `sizeof` does not suppress
the conversions inside the block, and block effects still prohibit folding the
construct to its tail constant. C99 6.3.2.1p2-4 (printed p. 46, PDF p. 58) and
[GCC Statement Exprs](https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html).

Loop jump context is installed by a body continuation and removed before a
`do` condition; `while` and `for` headers likewise retain only outer contexts.
Switch traversal already installs its context after its controlling expression,
so it needs no change. Invalid header jumps use the existing break/continue
constraint diagnostics and preserve following declarations. Tests cover every
header, expression and declaration initializers, and valid jumps to outer loops
or switches, including continue through a nested switch. C99 6.8.6.2p1 and
6.8.6.3p1 (printed p. 138, PDF p. 150), extended by GCC Statement Exprs.

The shared value and header-target fixtures run before/after bcc optimization
and ABI lowering on Linux GNU and Windows GNU, and compare host native execution
with bundled Clang at O0/O2. Semantic tests inspect categories and exact types;
the header-jump diagnostic fixture pins complete rendered output. The previous
positive test assigning to a statement-expression result was corrected to read
its value, consistent with the independent Clang reference.

Native-reference limitation: bundled Clang 23.1.1 crashes for outer-switch
breaks from for initializers and a nested switch controlling expression. It
also selects the inner loop for jumps from while/do conditions and for
conditions/increments, even when an outer body exists; continue can loop
forever. `CLANG_HEADER_REFERENCE` excludes those 15 cases from Clang alone,
retaining the six for-initializer and switch-header jumps to an outer loop.
All 21 cases execute in bcc against literal results and, when GCC is available,
an independent GCC O0/O2 reference. The installed GCC passes all 21 at both
levels. No compiler or LLVM-tree change was made for these upstream failures.

Validation passed: `cargo test --all-targets`, the 26 feature-enabled allocation
tests, nightly formatting, Clippy with warnings denied in debug, release and
benchmarking-internals configurations, `git diff --check`, and
`BCC_DIFFTEST_CASES=1000 cargo test --lib difftest`. The plain CLI was rebuilt
last. The invalid-header `--emit=ir` probe reports twelve ordinary constraints
and exits with status 1, without an internal lowering error. Shared LLVM trees
were preserved; no staging or commits were performed.

## Lowering effect-cache history

Overflow predicate type operands now have cached effect summaries keyed by
expression identity and whether its value is read. An explicit postorder
stack computes child summaries once, and a function-local arena-backed cache
reuses them during short-circuit scheduling. Volatile reads and address-only
operands retain their previous distinctions. A 2,048-deep chain regression
counts expression visits through the public pipeline: the pre-fix code visited
6,298,625 expressions; the fixed code visits 20,479, within the regression's
32,768-visit bound. Existing
short-circuit, volatile, call and increment tests preserve effect behavior.
The contract remains [GCC Integer Overflow Builtins](https://gcc.gnu.org/onlinedocs/gcc/Integer-Overflow-Builtins.html)
and C99 6.7.3p6 (printed p. 109, PDF p. 121).

## Lowering tails and headers

GNU block tails now consume the expression statement's recorded conversions,
matching their retained semantic value types. Loop header jumps use outer body
targets, with semantic rejection when none exists; switch controlling-expression
context already matched lowering. See the round-3 semantic record and
[GCC Statement Exprs](https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html).
C99 6.3.2.1p2-4 (printed p. 46, PDF p. 58) and 6.8.6.2p1/6.8.6.3p1
(printed p. 138, PDF p. 150) govern the underlying conversions and constraints.

## Header oracle limitations and historical validation

Native-reference limitation: bundled Clang 23.1.1 crashes for outer-switch
breaks from for initializers and a nested switch controlling expression. It
also selects the inner loop for jumps from while/do conditions and for
conditions/increments, even when an outer body exists; continue can loop
forever. `CLANG_HEADER_REFERENCE` excludes those 15 cases from Clang alone,
retaining the six for-initializer and switch-header jumps to an outer loop.
All 21 cases execute in bcc against literal results and, when GCC is available,
an independent GCC O0/O2 reference. The installed GCC passes all 21 at both
levels. No compiler or LLVM-tree change was made for these upstream failures.

Validation passed: `cargo test --all-targets`, the 26 feature-enabled allocation
tests, nightly formatting, Clippy with warnings denied in debug, release and
benchmarking-internals configurations, `git diff --check`, and
`BCC_DIFFTEST_CASES=1000 cargo test --lib difftest`. The plain CLI was rebuilt
last. The invalid-header `--emit=ir` probe reports twelve ordinary constraints
and exits with status 1, without an internal lowering error. Shared LLVM trees
were preserved; no staging or commits were performed.
