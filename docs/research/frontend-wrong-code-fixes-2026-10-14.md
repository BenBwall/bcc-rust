# Frontend wrong-code fixes, 2026-10-14

Baseline: `bd188faf2f47239677139471a65774a196d51e00`, branch
`claude/frontend-wrong-code`. The campaign and its three Python helpers were
read/copied from `origin/claude/conformance-testing`; the semantic-facts design
was read from `origin/claude/semantic-facts-design`.

## Owning facts and compatibility choices

Semantic analysis owns both fixes. Preprocessing retains macro-replaced packing
operands, provenance and output-token ordinals. Parsing retains the opening-brace
ordinal, without interpreting packing. Semantic analysis interprets the ordered
stack once and retains `Tag::max_member_alignment`; record completion applies it
to layout and member-access alignment. ABI projection consumes that existing
layout. Capturing the opening state matches Clang even when a directive changes
packing inside the body; the new state governs subsequent records.

Supported forms include reset/zero, literal caps 1/2/4/8/16, unnamed and named
push/pop, optional cap overrides, and show. Show reports the default cap as 8.
Malformed directives warn and preserve state; an empty pop warns, a missing
named slot leaves the stack intact, and unterminated pushes warn at their
retained source. Named pop with a cap warns like Clang and applies both actions.
Pack operand expansion is fenced from the caller, including `_Pragma` inside a
macro argument. Unknown unrelated pragmas keep their existing policy.

System V packed value bit-fields may cross their natural storage boundary;
zero-width fields retain their natural boundary. Microsoft layout reserves
complete units. Active zero-width fields follow Clang's distinct MinGW/MSVC
rules. GNU packing caps explicit/inherited ordinary-member alignment; MSVC
ordinary members may raise the cap with explicit/inherited alignment. Record
alignment can raise the final alignment. The fixture covers these distinctions,
packed attributes, unions, includes, nested/named stack restoration and runtime
field accesses.

`ExpressionInfo::promotion_width` independently retains the promotable width;
`bit_field` remains the designation used for address/size constraints. Comma
forwards the right width; assignment, compound assignment and prefix updates
forward the destination width. Conditional arithmetic consumes promotions when
forming its common type. Postfix updates and casts clear the width, matching
Clang. Declared types still govern `sizeof` and `_Generic`. A checked compact
width retains the existing 160-byte expression record. Integer promotions,
usual arithmetic conversions, comparison folding and lowering consume the
retained semantic conversions; no consumer reconstructs syntax to recover it.
Extended long/long-long fields promote by width, including the LP64/LLP64
difference. No M2 parameter/bound/size-formula or VLA-lowering areas were edited.

References: C99 6.3.1.1p2 (printed pp. 42-43, PDF pp. 54-55), 6.5.16p3
(printed p. 91, PDF p. 103), 6.5.17p2 (printed p. 94, PDF p. 106),
6.7.2.1p10/p12 (printed pp. 102-103, PDF pp. 114-115), and 6.10.6p1
(printed p. 159, PDF p. 171);
[GCC structure layout pragmas](https://gcc.gnu.org/onlinedocs/gcc/Structure-Layout-Pragmas.html),
[GCC type attributes](https://gcc.gnu.org/onlinedocs/gcc/Common-Type-Attributes.html),
[Microsoft pack](https://learn.microsoft.com/en-us/cpp/preprocessor/pack),
[Clang pragma parsing](https://clang.llvm.org/doxygen/ParsePragma_8cpp_source.html),
and [Clang pragma semantics](https://clang.llvm.org/doxygen/SemaAttr_8cpp_source.html).

## Regression evidence

Before the fix, the new retained-expression regression failed on Linux GNU:
comma's result promoted to unsigned int instead of int. The new pack-layout
regression failed with natural layout (8 instead of 5). The saved baseline CLI
also rejected the Clang-checked packing fixture and extended-field promotion
assertions. The fixture additionally exposed the unfenced pragma-operand reader;
the bounded replay fixes that overread.

Both original campaign reproducers now produce their expected output under
Linux GNU, Windows GNU and Windows MSVC interpretation: `A5 B5 C6 D5 E12 F5 G8`
and `1 1 1 1` followed by `1`. For these standalone probes only, the stdio include
was replaced by printf's compatible declaration so cross-target checks require
no foreign libc headers. The campaign checks use their original headers.

Bundled Clang 23.1.1 checks the regression assertions for Linux GNU, Windows GNU
and Windows MSVC (and Linux musl). Interpreter execution checks all those data
models; native LLVM O0/O2 checks the host. Two-way pragma-packed struct exchange
checks Windows GNU and Windows MSVC with/without bcc optimization and LLVM
O0/O2. Linux native exchange is skipped by the existing runner because WSL is
unavailable here; cross-target layout/type assertions and interpretation run.

## Campaign reruns

The generator is the original Csmith 2.4.0, Git version `0cdc710`. Regenerated
sources match the campaign sources except for the output-path comment.

| Seed | Original options | Clang checksum | Result |
| --- | --- | --- | --- |
| 100167 | `--no-pointers --float` | `CE3DBD2A` | A0/A1/A2 agree; both interpreter arms retain the existing undefined `fabsf` trap |
| 300681 | `--no-pointers --no-packed-struct` | `5E3D4D12` | All five arms agree |
| 95 | `--no-pointers` | `68EBB3CC` | All five arms agree |
| 100221 | `--no-pointers --float` | `BAEBBF7B` | A0/A1/A2 agree; both interpreter arms retain the existing undefined `fabsf` trap |

Reruns use the branch's `random_program_conformance.py`, with one generated
program per seed, the original options, and bundled Clang O0/O2 references.
Artifacts, manifests and compile/run logs are under `target/frontend-fixes/`.

## GCC torture comparison and final checks

The saved plain release CLI at `bd188faf` surveyed the same 1,677 sources with
the same source hashes, four arms, eight workers and ten-second deadlines as
the final release CLI. Per-source/per-arm comparison preserves all 5,326
baseline passes. `bf-sign-2.c` changes from wrong-result to pass in all four
arms; there are no other status changes and no compiler crashes or timeouts.

| Arm | Baseline pass | Final pass |
| --- | ---: | ---: |
| interpret | 1301 | 1302 |
| interpret-bcc | 1307 | 1308 |
| A0 | 1364 | 1365 |
| A1 | 1354 | 1355 |

The survey's nonzero exit remains expected for its existing disagreements and
unsupported sources; it does not mean a baseline pass regressed. Evidence:
`target/compiler-corpus/execute-frontend-baseline/`,
`target/compiler-corpus/execute-frontend-fixes/`, and
`target/frontend-fixes/survey-comparison.json`. The requested survey command
was run exactly against the final binary. The earlier pre-fence survey was
preserved separately. No shared LLVM junction contents were changed.

Canonical checks ran on Windows GNU:

| Command | Result |
| --- | --- |
| `cargo test --all-targets` | Pass; library: 1689 passed, 3 ignored; integration suites pass |
| `cargo test --features benchmarking-internals --test allocation_count` | Pass; 28 tests, including both facts and the warning path |
| `cargo +nightly fmt` | Pass |
| `cargo +nightly fmt --check` | Pass |
| `cargo clippy --all-targets -- -D warnings` | Pass |
| `cargo clippy --release --all-targets -- -D warnings` | Pass |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | Pass |
| `git diff --check` | Pass |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | Pass; 6 tests, 2 ignored by the requested command |

No new platform-specific lint expectations were added. Native Linux execution
and Linux-hosted Rust checks require CI/another host; they were not run here.
The allocation-feature build ran after CLI integration tests. Check logs are
`target/frontend-fixes/check-*.log`. Every changed repository file and the
whole-file commit plan end with exactly one newline; nothing was staged or
committed. The plan uses one commit because both fixes share whole test and
documentation files.

`cargo build --bin bcc-rust` passed at the end, restoring the plain CLI after
benchmarking-feature checks. Final newline/whole-file-plan validation and
`git diff --check` passed again after this report was completed.
