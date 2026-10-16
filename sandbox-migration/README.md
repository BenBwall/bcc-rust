# Move repository work into Docker sandboxes

Run `Move-WorkToSandboxes.ps1` on the **Windows host**. It handles both
`C:\Users\benbw\Documents\GitRepo\prevalabs\prevalabs-mono` and
`C:\Users\benbw\Documents\GitRepo\bcc-rust`, using the existing sandboxes named
`prevalabs-mono` and `bcc-rust`. Use `-RepoRoot` if your repositories are elsewhere.
It also recognizes `GitRepo\prevalabs-mono` when the nested checkout is absent.
If both locations contain a checkout, it requires an explicit selection.
Use `-PrevalabsPath` or `-BccRustPath` to specify each main checkout independently:

```powershell
& .\bcc-rust\sandbox-migration\Move-WorkToSandboxes.ps1 -PrevalabsPath "$env:USERPROFILE\Documents\GitRepo\prevalabs\prevalabs-mono"
```

Windows needs Git, `sbx`, and Python 3.10+. Both sandboxes need Git and Python 3.10+.

Preview from the parent directory:

```powershell
Set-Location "$env:USERPROFILE\Documents\GitRepo"
& .\bcc-rust\sandbox-migration\Move-WorkToSandboxes.ps1
```

Preview leaves running Agentbus relays alone. Before applying, close editors,
agent sessions, development servers, builds and Git clients that use either
repository or any of their worktrees. Stay in the parent `GitRepo` directory,
outside all checkouts, so Windows can rename them.

`-Apply` automatically stops each matching idle Agentbus host relay. It checks
the heartbeat PID against the Python process command line, which must be one
of the two launch forms in use:

- `python <checkout>\scripts\agentbus\relay.py --directory <checkout>\.git\agentbus\relay ...`
  (bcc-rust's `start-relay.ps1`)
- `python %APPDATA%\sbx\helpers\agentbus-host.py <config>.json`, where that
  config's `directory` is the checkout's relay directory (the sbx helper used
  for prevalabs-mono)

It leaves unrelated processes alone. A relay is idle only when its `requests`
directory has no `.waiting`, `.json` or `.running` file, so it is never stopped
while a client is waiting or a request is executing. If a request starts in the
moment before the stop, the migration stops too, because its `agentbus` process
may still be writing. It checkpoints the Agentbus SQLite database and stops the
migration if the database is still busy or the relay restarts. Recent Git locks
also stop the migration. Relays stopped by `-Apply` stay stopped even if a later
step fails; the journal records their state. Agentbus hooks are never enabled.

The Startup shortcuts `Agentbus-bcc-rust.lnk` and `Agentbus-prevalabs-mono.lnk`
restart relays at logon. The preview lists them. Delete them after migrating;
otherwise the prevalabs-mono relay starts against the clean host clone and
routes the sandbox's Agentbus traffic back to Windows.

```powershell
& .\bcc-rust\sandbox-migration\Move-WorkToSandboxes.ps1 -Apply
```

The script copies its runner outside the repositories before applying, so its
own files can be removed from the host checkout during migration. It prints that
saved location and the migration journal location.

The result is:

| Location | Contents |
| --- | --- |
| Host `GitRepo\prevalabs\prevalabs-mono` and `GitRepo\bcc-rust` | Independent, clean `main` clones matching `origin/main`, with only one local branch and one worktree |
| Each sandbox: `/home/agent/workspace/<repo>/checkout` | The original primary checkout, current branch, staged and unstaged changes, untracked and ignored files, and the full `.git` history |
| Each sandbox: `/home/agent/workspace/<repo>/worktrees/<original-name>` | Original linked worktrees and their dirty state |
| Each sandbox: `/home/agent/workspace/<repo>/migration` | File manifest, verification record, helper, and original administrative files that needed Windows path conversion |
| Host `GitRepo\.sandbox-migration` | Saved tools and journal; optional rollback copies |

Every local branch, tag, stash, reflog and initialized submodule is copied with
the original Git directories. It does not stash, commit, reset, clean, delete
branches, prune missing worktrees or initialize/fetch submodules. Worktree
registrations already missing on the host are reported and their metadata is
preserved. Worktrees nested inside the main checkout are copied separately.

All copied regular files are checked with SHA-256 before paths are repaired.
The script compares each worktree's HEAD, branch, refs, staged index and full
Git status. It rechecks the original files before replacing the host folders,
then rehashes the relocated originals before cleanup. Both sandbox copies must
pass before either host repository is replaced. Host replacement failures
trigger restoration of the original directory locations.

Windows cannot rename a folder while a process inside a sandbox holds a file in
it through the mount; it reports `[WinError 5] Access is denied`. After both
imports pass, `-Apply` therefore runs `sbx stop` on every running sandbox that
mounts one of the folders, before the final recheck. Stopping keeps sandbox
storage. Once the folders are replaced (or restored after a failure), it
restarts those sandboxes with `sbx run --name <sandbox> --detach`; they then
mount the clean clones. Host backups are deleted only if both sandboxes restart
and their copies are found again. Agent sessions in the sandboxes are
disconnected while they are stopped. If a host program still holds files in a
folder, the move is retried for 30 seconds and then stops with the folder's path.

The default includes build caches and deletes the verified host originals and
temporary transfer archives after successful migration. Add `-KeepHostBackups`
to retain originals and transfer archives in the printed journal directory.

If the preview reports insufficient sandbox disk space, you can preview again
with `-SkipBuildCaches`, then apply with the same switch:

```powershell
& .\bcc-rust\sandbox-migration\Move-WorkToSandboxes.ps1 -SkipBuildCaches
& .\bcc-rust\sandbox-migration\Move-WorkToSandboxes.ps1 -Apply -SkipBuildCaches
```

That option omits directories named `target`, `node_modules`, `.next`, `.turbo`,
`.parcel-cache` and `__pycache__`. A directory with one of those names that
contains tracked files is source, not a cache, and is copied in full (bcc-rust's
`src/target`, for example). It still copies ignored configuration and secrets
such as `.env` files.
**Using this option always retains the host originals and transfer archives**,
so skipped files can be recovered. The canonical host checkouts are still clean
`main` clones. Delete the printed rollback copies later when satisfied that
the omitted caches can be regenerated.

Open the **new sandbox folders** for subsequent work. For a terminal:

```powershell
sbx exec bcc-rust bash -lc 'cd /home/agent/workspace/bcc-rust/checkout && exec bash'
sbx exec prevalabs-mono bash -lc 'cd /home/agent/workspace/prevalabs-mono/checkout && exec bash'
```

The existing sandboxes retain their original direct-mode host mounts and their
default launch directories. Those paths expose the clean host clones; editing
there will change the host again. Update T3/Claude/Codex project folders to the
new `/home/agent/workspace/.../checkout` paths. Do not remove/recreate the
sandboxes while they contain your only copies of unpushed work.

Windows-specific configured Git hooks and filesystem monitors are disabled in
the sandbox copies, with their original administrative files retained under
`migration/original-metadata`. Line-ending/symlink settings, Git filters, and
effective global ignore/attribute files carry over. Required external Git
filters, such as `git-lfs`, must also be installed in the destination sandbox.
The copied Agentbus configuration uses native sandbox storage rather than the
old Windows relay; hooks are not enabled. Delete the Agentbus Startup shortcuts
listed by the script (see above).

If the script stops, read the printed error and journal. Before replacement,
original directories are untouched. Failed or verified sandbox imports are
retained for inspection, and existing destination folders are never overwritten.
When you rerun after a failure, a verified copy left by the earlier run is moved
to `/home/agent/workspace/.superseded-migrations/<run>-<repo>`, but only if its
worktrees are still exactly as imported. If anything changed in that copy, the
import stops and leaves it where it is. Delete superseded copies once the new
migration is complete.
If the failure happens after replacement during final checking or cleanup,
the journal records the original and backup directory locations. Do not delete
retained copies until the failure is understood. Symlinks and junctions that
point outside the migrated repositories, external Git object alternates, recent
Git locks and unfinished rebases are deliberately rejected.

Links are copied as links, never followed:

- Windows junctions and symlinks that point inside the migrated repositories
  become relative symlinks. That covers bcc-rust's `target\llvm` and
  `target\llvm-libs` junctions in each worktree, and bun's `node_modules` and
  `.next` links. Their `\\?\C:\...` and backslash spellings are converted.
- Symlinks that a sandbox wrote through the mount, such as prevalabs-mono's
  `.codegraph-sandbox` -> `/home/agent/.cache/codegraph/prevalabs-mono`, keep
  their absolute sandbox path.

A directory Windows cannot read stops the preview with its path. Restore access
from an elevated PowerShell, then preview again:

```powershell
takeown /F "<path>" /R /D Y
icacls "<path>" /reset /T /C
```

On 2026-10-16, `prevalabs-mono\node_modules\.cache\ihd-swc` was such a directory.
It is empty in the sandbox's view. `-SkipBuildCaches` omits it with the rest of
`node_modules`.

The first real `-Apply` (run `20261016-035020-7f7f32`) verified both sandbox
imports, then stopped on `Access is denied` when renaming prevalabs-mono. Nothing
was moved. A T3 server inside the prevalabs-mono sandbox was running from the
mounted folder at the time. The stop/restart step above fixes this.

Validated using disposable repositories: dirty primary and linked checkouts,
staged and unstaged edits, ignored files, submodules in both kinds of worktree,
symlinks, stashes, stale worktree registrations, Windows administrative paths,
CRLF/global excludes, archive corruption, concurrent edits, and an injected
host rename failure. PowerShell syntax was checked with PowerShell 7.6.7 on
Linux.

On the Windows host (2026-10-16), a disposable repository passed a further
50 checks:

- Real relay processes in both launch forms were stopped.
- Unrelated processes, a relay from another checkout and a helper serving
  another directory were refused and left running.
- `.waiting` and `.json` requests blocked the stop, and a relay restart and a
  busy database were detected. The WAL was checkpointed.
- A junction, an NT-form symlink, a backslash relative symlink and a
  sandbox-made absolute symlink were archived.
- A real `sbx exec` import into the bcc-rust sandbox verified status, the
  stash, branches, links, ignored files, Agentbus data and `git fsck`.
- A rerun moved an unchanged earlier copy aside and refused a changed one.
- In a throwaway sandbox, a file held open through the mount blocked the move
  (after retrying). The move then succeeded after `sbx stop`. The restarted
  sandbox kept its storage and mounted the new folder.

Windows PowerShell 5.1 and PowerShell 7 parse the wrapper. The real preview
with `-SkipBuildCaches` passes for both repositories:

| Repository | Worktrees | Copy without caches | Sandbox free |
| --- | --- | --- | --- |
| prevalabs-mono | 5 | 0.05 GiB | 16.5 GiB |
| bcc-rust | 15 | 1.61 GiB | 17.8 GiB |

Without that switch, prevalabs-mono's roughly 21 GB of `node_modules` and
`.next` does not fit its sandbox, so use `-SkipBuildCaches`. The host
replacement step (rename, sandbox restart, cleanup) has not yet completed
against the real repositories.
