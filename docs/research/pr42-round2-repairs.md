# PR #42 round-2 repair evidence

Starting revision: `32b649a055e00f51863bd3114264bcf7747e4d98` in `claude/fix42c`.
All compiler/native invocations used the repository runner or existing Cargo
and native-test helpers. Shared `target/llvm` and `target/llvm-libs` were
read only. No staging or commits were performed.

## Fact ownership and regressions

| Finding | Owner and retained fact | Regression |
| --- | --- | --- |
| 1 | Semantic completed nominal tag layout, queried by `Types::aggregate_value_layout`; storage alignment stays on the object/typedef handle. | Aligned local and fetch typedef after six integers, all interpreter stages and targets; aggregate exchange shapes use aligned locals in both directions. |
| 2 | Semantic `Member::explicit_alignment`, separate from typedef alignment. | GNU 5/1/1 versus explicit attribute/_Alignas 8/4/4; Clang MSVC preserves inherited ordinary-member alignment. Existing packed bit-field tests remain part of the full suite. |
| 3 | Semantic `FunctionRecord::parameter_aliases`; address-taking shares storage requirements before parameter allocation. | Prototype-bound address of a scalar plus body addresses in K&R/prototyped definitions and aggregates. The simple body-only examples already passed; the bound-only alias case reproduced the reported lowering error. |
| 4 | Semantic `BoundSiteFacts` on declaration/type-name identities and size-query identities. | VM typeof operands in declarations, typedefs, casts, sizeof, nesting and specifier/bound order; non-VM operands and constant sizeof remain unevaluated. Typedef reuse does not repeat the original operand. |
| 5 | Existing IR stack save/restore lifetime semantics. | Interpreter dangling-pointer trap after VLA restoration, before/after optimization and ABI lowering. No behavior change. |
| 6 | Semantic packed layout consumed by aggregate ABI projection. | Packed C shape in the two-direction bcc/Clang matrix and all interpreter stages. |
| 7 | Existing cursor-advancing aggregate-fetch IR effects. | Discarded aggregate followed by checked aggregate, copied cursor and scalar fetch in every exchange shape; optimized/unoptimized interpreter and native comparisons. |
| 8 | Existing switch target and VLA cleanup behavior. | Break and continue through switch with live enclosing VLA, all stages; 1,000 iterations and fewer than 100,000 interpreter memory cells. |

## Independent oracle observations

Raw sources, emitted code and JSON results are under `target/fix42c/`.
Bundled Clang accepted the nominal-alignment case and returned zero at O0/O2.
Clang's System V O0 IR passes the local as `byval(%struct.S) align 8`
and fetches it with align 8, while the local's allocation remains align 64.
The System V native exchange arm is skipped here because WSL returns
`Wsl/Service/CreateInstance/E_ACCESSDENIED`; the matrix still runs both
System V interpreter profiles and native Win64 GNU/MSVC exchanges.
GNU-target packed typedef assertions agree with Clang; Clang's MSVC target
instead keeps the inherited aligned typedef requirement (8/4/4).

Clang and GCC agree on ordinary VM typeof operands at O0/O2. For the explicit
specifier/bound ordering probe, GCC returns zero while Clang returns one:
Clang evaluates the declarator bound before the typeof operand. That probe is
covered by the interpreter with GCC's source-order behavior and excluded from
the Clang native test chain. A sizeof of typeof(pointer-to-VLA increment) has
constant pointer size and suppresses the increment in both compilers; the
semantic size-query record preserves this unevaluated context.

For the exact alloca/VLA report, Clang and GCC both return zero at O0/O2.
Clang O0 emits stacksave before the VLA and stackrestore at block exit, after
the alloca store. Clang O2 folds the entire probe to zero and emits neither.
GCC O0 retains dynamic allocations until function return; GCC O2 removes them.
A successful native result cannot prove validity after a stackrestore.
The interpreter's trap matches Clang's emitted lifetime boundary, so it is
pinned rather than changing stack semantics to GCC's current implementation.
The stronger probe with a subsequent VLA also returns zero in both compilers;
its Clang O0 IR still restores after the first VLA and newer alloca.

The reviewer quoted the [GCC 4.2.1 Variable Length manual](https://gcc.gnu.org/onlinedocs/gcc-4.2.1/gcc/Variable-Length.html),
which explicitly allows VLA deallocation to release newer alloca storage.
The [current manual](https://gcc.gnu.org/onlinedocs/gcc/Variable-Length.html)
says VLA storage is retained when alloca occurs in that scope. These are
version-specific policies, not equivalent claims. See also
[GCC Typeof](https://gcc.gnu.org/onlinedocs/gcc/Typeof.html) and
[GCC Common Variable Attributes](https://gcc.gnu.org/onlinedocs/gcc/Common-Variable-Attributes.html).

## Validation

All required commands passed on the final source:

| Command | Final result |
| --- | --- |
| `cargo test --all-targets` | 1,819 passed, 6 ignored across 20 suites; no failures. |
| `cargo test --features benchmarking-internals --test allocation_count` | 26 passed. |
| `cargo +nightly fmt --check` | Passed after running `cargo +nightly fmt`. |
| `cargo clippy --all-targets -- -D warnings` | Passed. |
| `cargo clippy --release --all-targets -- -D warnings` | Passed. |
| `cargo clippy --all-targets --features benchmarking-internals -- -D warnings` | Passed. |
| `git diff --check` | Passed, including after this report was finalized. |
| `BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` | 6 passed, 2 ignored. |
| `cargo build --bin bcc-rust` | Passed last, restoring the plain CLI. |

Commands ran sequentially; feature builds did not overlap CLI integration
execution. Logs and exact exit codes are in `target/fix42c/check-*.log` and
`target/fix42c/check-results.json`. Native System V execution remains unavailable
through WSL, as described above; no Linux Clippy run is claimed.

An initial full run exposed an accidentally doubled argument in the pre-existing
mixed-varargs fixture; that fixture was restored. A subsequent native numeric
launch hit Windows error 5; its focused rerun and the final full run passed.
Clippy style findings were corrected before the final complete validation.

Focused red logs: `vla-red-corrected.log` and the initial aggregate/packed
assertions. The aggregate alignment failure was a variadic type mismatch;
the packed failure was 8/4 against expected 5/1; prototype address-taking
failed with `an address operand is not in memory`; typeof returned one.
The initial typeof probe used an ISO-unavailable spelling and was corrected
to `__typeof__` before establishing the behavioral red result.
