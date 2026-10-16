---
name: review-pr-backend
description: Review bcc-rust ABI lowering and back-end changes for System V and Win64 calling conventions, the LLVM C-API back end, the reference interpreter, the native x86-64 code generator, and the differential-testing harness. Use for changes to src/abi, src/backend, src/target or toolchain and linking code, or any IR form the post-ABI back ends consume.
---

# Review PR Backend

Read [the shared review contract](../review-pr/references/review-contract.md) completely. Read `.agents/AGENTS.md`, `middle-end.md` ("ABI lowering", "From the IR to machine code", "Comparison arms", "Command line", "Testing"), `docs/design/native-backend.md` for native changes (its "Implementation status" records what each stage supports), and `src/abi.rs` or `src/backend.rs` with the submodules the change touches. Settle calling-convention questions with the System V AMD64 psABI and the Microsoft x64 convention, and compare with the pinned Clang's output for the same C on the same target.

Review each back end against the same post-ABI module, since every back end must agree with the interpreter:

- ABI lowering: classification of scalars, records, unions, arrays, `long double`, `__int128` and vectors; register versus memory passing, `sret`/`byval` size and alignment, narrow-integer extension, variadic calls and `va_arg` walks, `freeze` of partially written aggregates, and that unsupported cases refuse before the module changes;
- LLVM back end: construction through the scoped `api.rs` wrappers without leaks or use-after-dispose, phi and edge construction including duplicate edges, attributes shared by calls and declarations, symbol linkage, aliases, weak, sections, thread-local models, and per-target triple and data layout;
- interpreter: exact poison and provenance tracking, traps for C-undefined operations, host functions matching C library contracts, target-dependent layouts such as `va_list` and `long double`, and step-limit behavior; it is the oracle, so a permissive interpreter hides miscompiles;
- native back end: support check refusing before any output, legalization, instruction selection, parallel copies for block parameters, register allocation with fixed registers, call clobbers and spills, frame layout and stack alignment at calls, red zone and shadow space, CFI and SEH directives, TLS and relocation forms for ELF and COFF, and emitted AT&T syntax that the bundled clang assembles; and
- harness and toolchain: differential tests comparing arms, timeouts and process-tree cleanup, crash-dialog suppression on Windows, target discovery, and anything that silently skips a test on one host.

For each changed path, name the concrete C or IR input, target and arm that exercises it, and state the expected register or memory placement, assembly, or interpreter output. Probe both System V and Win64, a struct of 1, 2, 3, 8, 9, 16 and 17 bytes where classification is involved, mixed integer and float members, more arguments than argument registers, and calls whose stack alignment depends on the argument count. A path that runs only on Windows or only on Linux CI needs evidence that the other host still covers it.

Leave lowering decisions to `review-pr-lowering` and optimizer rewrites to `review-pr-optimizer`, while checking any IR form this change newly accepts or rejects.

Complete only after every changed classification, signature rewrite, back-end construct, refusal, register or frame rule, emitted directive, interpreter semantic, and harness behavior is accounted for.
