# Reserved native-stack follow-up

2026-10-22. Follow-up to [the recursive descent study](parser-recursive-descent.md).
The experiment remains confined to the opt-in standalone study executable.
The default compiler/parser path is unchanged.

## Implementation and measurement boundary

`--reserve-stack BYTES` runs preparation, parsing and optional inspection on a
fresh native worker, joined synchronously. Linux reserves the requested usable
range plus two 64 KiB inaccessible guards using `mmap(MAP_NORESERVE | MAP_STACK)`.
The usable range is made writable without touching its pages and advised against
transparent huge pages. `pthread_attr_setstack` registers it as the worker stack.
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

`/proc/self/smaps` snapshots identify the stack mapping's resident, writable and
swapped bytes and its `nr` flag, before/after phase 7. Per-thread minor/major
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
under `.scratch/parser-study/reserved-*`. The Python runner accepts labels such
as `reserved:recursive` and `reserved:machine`, using a default 100 GiB usable
reservation. `--reserve-stack BYTES` changes that reservation. `--stack BYTES`
continues to apply only to ordinary, non-reserved labels.

## Results

Implementation saved before the first build. Results and validation will be
added after native measurements complete.
