# Recovery and Windows follow-up evidence

See [the report](../parser-recovery-windows.md) for methodology and conclusions.
`summary.json` derives Linux counts, timing, and stack metrics from the complete
compressed JSON. Use `gzip -dc FILE` to read an archive. The underlying final
parser implementation is `c9fbaccf`; binary hashes are in each JSON's metadata.

The Windows files record cross-compilation and Wine compatibility execution.
They contain no native Windows measurements. `windows-wine-results.json` has
one rejected negative-probe outcome; `windows-wine-collect.log.gz` ends EXIT 1.
The saved manifest/lock, pinned setup manifest, and build logs identify the
actual probe build and dependencies. Paths refer to this designated worktree.
