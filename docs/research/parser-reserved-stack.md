# Reserved native-stack follow-up

2026-10-22. Follow-up to [the recursive descent study](parser-recursive-descent.md).
The experiment remains confined to the opt-in standalone study executable.
The default compiler/parser path is unchanged.

## Implementation and measurement boundary

`--reserve-stack BYTES` runs preparation, parsing and optional inspection on a
fresh native worker, joined synchronously. Linux reserves the requested usable
range plus two 64 KiB inaccessible guards using `mmap(MAP_NORESERVE | MAP_STACK)`.
The usable range is made writable without touching its pages and advised against
transparent huge pages where the kernel supports that advice. The host rejects
that optional advice with `EINVAL`; snapshots record actual huge-page use. `pthread_attr_setstack` registers it as the worker stack.
The kernel backs ordinary pages on demand; recursion performs no growth checks.
The mapping stays fixed and is unmapped after successful join. Callback panics
are caught inside the native entry point and resumed after joining.

Windows has a counterpart using `CreateThread` with
`STACK_SIZE_PARAM_IS_A_RESERVATION`. Its OS-managed guard commits pages as the
stack descends. This source path has not yet been compiled or executed here.
Linux measurements are not Windows measurements.

Unlike the arena's explicit commit frontier, Linux makes the full usable stack
writable up front with no commit reservation. `MAP_NORESERVE` depends on the
host overcommit policy; physical-memory exhaustion and crossing the bottom
stack guard are fatal, not parser resource diagnostics. This experiment does
not install a signal handler or attempt recovery from actual stack overflow.

Opt-in `--stack-memory` `/proc/self/smaps` snapshots identify the stack mapping's resident, writable and
swapped bytes and its `nr` flag, before/after phase 7. Timing runs leave this option disabled. Per-thread minor/major
fault counts bracket the same interval. All OS snapshots, setup and join are
outside the parser timer/global allocation counter. Reported RSS remains
whole-process high water, including preparation. Windows commit/protection
snapshots are implemented but physical stack residency is unavailable there.

## Reproduction

Run from `/home/agent/workspace/bcc-rust/worktrees/parser-recursive-descent-study`.
All Cargo commands stay inside the shared wrapper:

```sh
python3 /home/agent/workspace/bcc-rust/triage-2026-10-16/swarm/run.py sh scripts/parser_study_build.sh reserved
```

The build batch saves separate study and feature-disabled release/debug copies
under `.scratch/parser-study/reserved-*`.

```sh
python3 scripts/parser_study_reserved.py prepare
python3 scripts/parser_study_reserved.py collect --profile release
python3 scripts/parser_study_reserved.py archive
python3 scripts/parser_study_reserved.py collect --profile debug
python3 scripts/parser_study_reserved.py archive
```

The collection batches contain exact inputs/options, rotate mode order over nine
shallow or seven deep samples, record stack-memory samples separately, compare
AST/provenance/diagnostic output and sweep reservation sizes. The generated extra
fixtures reach depth 65,536. Deep full-tree rendering is deliberately avoided. The Python runner accepts labels such
as `reserved:recursive` and `reserved:machine`, using a default 100 GiB usable
reservation. `--reserve-stack BYTES` changes that reservation. `--stack BYTES`
continues to apply only to ordinary, non-reserved labels.

## Initial smoke observations

Both builds successfully parsed Lua and 65,536 nested blocks with a 100 GiB
usable reservation. These are single smoke samples, not the final throughput
comparison. Lua's stack mapping was 44 KiB resident after parsing in release
and 168 KiB in debug. At 65,536 blocks, stack residency was about 63 MiB release
and 511 MiB debug. The mapping reports `nr`, zero swap and zero anonymous huge
pages. Every smoke parse made zero global allocator calls during phase 7.
The reservation is therefore demand-backed; active recursive frames still cost
physical memory. Repeated measurements are running.

A standard Rust worker requested with `--stack 107374182400` failed to start in
all three release attempts (`EAGAIN`, resource temporarily unavailable). The
explicit no-reserve mapping succeeds at the same usable size. This demonstrates
why requesting a large ordinary worker is not equivalent on this host.

## Validation so far

The study feature passes strict Clippy. All three native-stack tests pass:
borrowed callback/result transfer with a lazily backed 1 GiB stack; worker panic
propagation after joining; and rejection of unrepresentable reservations.
The first test confirms a writable mapping with Linux's `nr` flag and residency
below 4 MiB. Release/debug measurements and canonical checks are running.

The first build caught five Clippy issues, all corrected. The first native test
run failed because the host rejected optional `MADV_NOHUGEPAGE` advice. Making
that advice best-effort corrected the failure; actual backing is measured rather
than assumed. Failed validation logs are retained alongside the successful run.
