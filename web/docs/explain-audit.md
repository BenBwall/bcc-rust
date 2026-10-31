# Site audit: step explanations and compiler views (2026-10-30)

A newcomer's-eye audit of the site at main e915d07e. The tools are in
`~/.cache/site-redesign-tools/explain/` (`narrate.mjs`, `views.mjs`,
`states.mjs`, `shots.mjs`, `cards.mjs`, `terms.mjs`, `hover-ir.mjs`).

## Narration of the abstract machine

I stepped through all 519 steps of the 7 machine lessons and read each one
(`narrate.mjs`).

| Finding | Where | Fixed in |
| --- | --- | --- |
| Every declaration repeated a 3-line caveat; reads, stores, constants and sequence points restated the same idea at every step (92.8k characters in total, 37 steps over 300 characters) | all lessons | agent/site-explain: notes once per run per topic (74.5k characters, 8 steps over 300) |
| "Initialize `narrow` = -56" said 200 "becomes the first value stored", then that it does not fit | promotion, step 25 | agent/site-explain |
| Array initializer elements cited 6.5.2.1p2 (subscripting) instead of 6.7.8p17 | every array initializer | agent/site-explain |
| The uninitialized-read fault said "prototype policy"; it gave no reason why the read is a bug | indeterminate | agent/site-explain (6.2.6.1p5, C11 6.3.2.1p2) |
| The out-of-bounds message named neither the array nor the index | bounds | agent/site-explain |
| `narrow++` on a char never mentioned the promotion to int | promotion | agent/site-explain |
| "a pointer to values[0] + 0 moves 0 ints" reads as `values[0] + 0` | `&values[0]` | agent/site-explain |
| Main's return said "hands the value back to the caller" | all lessons | agent/site-explain ("as if exit(n)") |
| Terms (lvalue, sequence point, indeterminate…) were never defined where used; clause ids were not links | everywhere | agent/site-explain (glossary, tooltips, N1256 links) |
| The values a step used were only in the tree and the memory diagram, usually below the fold at 1440x900 | every step | agent/site-explain (value-flow chips, memory access marks) |

Still open: at 1440x900 the memory diagram stays below the fold under the
narration and the tree.

## IR, LLVM IR and assembly views

I checked each of the 8 lessons, an edited program, a semantic error and a
syntax error at -O0, and the edited program at -O2 (`views.mjs`,
`states.mjs`).

- All five back-end views render with their Monaco grammar (bcc-ir, llvm,
  asm, objdump; 8 to 12 token classes each). Errors show a "Compilation
  stopped" banner and a ✕ on the stages that did not run.
- The bcc IR, ABI-lowered IR, LLVM IR and assembly linked to the source
  per function only. agent/site-views links each IR instruction to its
  line. It records lines in lowering for the explorer, and a `return`
  terminator takes the return statement's line. Assembly and LLVM IR stay
  per function: the native back end keeps no source lines, and adding them
  would touch register allocation.
- LLVM IR cannot be produced for an edited program: the in-browser compiler
  has no LLVM. agent/site-views explains this, offers the live bcc IR, and
  shows the `bcc-rust --emit=llvm` command.
- The -O0/-O2 note showed even when the stage had no output (an error, or
  LLVM IR for an edited program); agent/site-views hides it then.
- Minor, not fixed: the stage header's command keeps the lesson's file name
  (`objects.c`) for an edited program.

## Audits of agent/site-explain

- Contrast (compositing, every text node, dark and light at 1440/960/390):
  0 failures in 89,278 checks.
- Keyboard: every stop has a visible focus ring.
- Reveal: 20 of 22 clear, the same as main.
