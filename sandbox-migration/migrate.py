"""Lossless host-to-sandbox migration. Invoked by Move-WorkToSandboxes.ps1.

No reset, clean, branch deletion, stash operation or worktree pruning is used.
Original directories are retained until both sandbox copies and host clones pass
verification. This helper also runs in each destination sandbox via sbx exec.
"""
import argparse
import base64
import hashlib
import json
import ntpath
import os
from pathlib import Path, PurePosixPath
import posixpath
import re
import shutil
import sqlite3
import stat
import subprocess
import sys
import tarfile
import time
import uuid

REPOSITORIES = ('prevalabs-mono', 'bcc-rust')
# Only used with the explicit --skip-build-caches option. No blanket ignored-file
# exclusion: ignored configuration, .env files and local databases are preserved.
CACHES = {'target', 'node_modules', '.next', '.turbo', '.parcel-cache', '__pycache__'}
GIT_LOCKS = {'index.lock', 'HEAD.lock', 'config.lock', 'packed-refs.lock', 'shallow.lock'}
IO_REPARSE_TAG_MOUNT_POINT = 0xA0000003


def run(args, cwd=None):
    result = subprocess.run([str(a) for a in args], cwd=cwd, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE)
    if result.returncode:
        # Do not echo command arguments, which may include private remote URLs.
        raise RuntimeError(f'{args[0]} failed (exit {result.returncode}):\n'
                           + result.stderr.decode('utf-8', 'replace'))
    return result.stdout


def git(root, *args):
    return run(['git', '--no-optional-locks', '-c', 'core.fsmonitor=false',
                '-c', 'core.hooksPath=' + os.devnull, '-C', root, *args])


def write_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + '.tmp')
    temporary.write_text(json.dumps(value, indent=2, ensure_ascii=True), encoding='utf-8')
    temporary.replace(path)


def read_json(path):
    return json.loads(Path(path).read_text(encoding='utf-8'))


def digest_file(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(4 * 1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def normal(path):
    return str(path).replace('\\', '/').rstrip('/').casefold()


def inside(path, parent):
    return normal(path) == normal(parent) or normal(path).startswith(normal(parent) + '/')


def is_junction(attributes):
    # Python reports a junction as a directory; only the reparse tag tells them apart.
    return getattr(attributes, 'st_reparse_tag', 0) == IO_REPARSE_TAG_MOUNT_POINT


def link_text(path):
    # Windows reports junction and some symlink targets in NT form (\\?\C:\...).
    text = os.readlink(path)
    for prefix in ('\\\\?\\', '\\??\\'):
        if text.startswith(prefix) and re.match(r'^[A-Za-z]:', text[len(prefix):]):
            return text[len(prefix):]
    return text


def sandbox_absolute(text):
    # A rooted target without a drive (\home\agent\...) can only have been
    # written by a sandbox through the mount. It is kept as that POSIX path.
    return re.match(r'^[\\/](?![\\/?])', text) is not None


def unreadable(path):
    return RuntimeError(f'Windows denies access to {path}. Restore access from an elevated PowerShell with '
                        f'takeown /F "{path}" /R /D Y; icacls "{path}" /reset /T /C, then preview again. '
                        'A directory inside a build cache can instead be omitted with -SkipBuildCaches.')


def snapshot(root):
    return {
        'head': git(root, 'rev-parse', 'HEAD').decode().strip(),
        'branch': git(root, 'rev-parse', '--abbrev-ref', 'HEAD').decode().strip(),
        'refs': git(root, 'for-each-ref', '--format=%(refname) %(objectname)').decode(),
        'status': base64.b64encode(git(root, 'status', '--porcelain=v1', '-z',
                                      '--untracked-files=all', '--ignore-submodules=none')).decode(),
        'index': hashlib.sha256(git(root, 'ls-files', '--stage', '-z')).hexdigest(),
    }


def portable_settings(root):
    values = {}
    files = {}
    for key in ('core.autocrlf', 'core.eol', 'core.symlinks', 'core.ignorecase', 'core.safecrlf'):
        result = subprocess.run(['git', '-C', str(root), 'config', '--get', key],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if result.returncode == 0:
            values[key] = result.stdout.decode().strip()
        elif result.returncode != 1:
            raise RuntimeError(f'Cannot read effective {key} for {root}')
    for key in ('core.excludesfile', 'core.attributesfile'):
        result = subprocess.run(['git', '-C', str(root), 'config', '--path', '--get', key],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if result.returncode not in (0, 1):
            raise RuntimeError(f'Cannot read effective {key} for {root}')
        value = result.stdout.decode().strip()
        if not value and key == 'core.excludesfile':
            value = str(Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home() / '.config'))) / 'git/ignore')
        if value:
            path = Path(value)
            if not path.is_absolute():
                path = Path(root) / path
            files[key] = base64.b64encode(path.read_bytes() if path.is_file() else b'').decode()
    filters = subprocess.run(['git', '-C', str(root), 'config', '--null', '--get-regexp', r'^filter\.'],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if filters.returncode not in (0, 1):
        raise RuntimeError(f'Cannot inspect Git filters for {root}')
    for record in filters.stdout.split(b'\0'):
        if record:
            key, _, value = record.partition(b'\n')
            values[key.decode()] = value.decode()
    return {'values': values, 'files': files}


def worktrees(root):
    blocks = git(root, 'worktree', 'list', '--porcelain', '-z').split(b'\0\0')
    result = []
    for block in blocks:
        fields = {}
        for field in block.split(b'\0'):
            key, _, value = field.partition(b' ')
            fields[key.decode()] = value.decode('utf-8', 'surrogateescape')
        if 'worktree' in fields:
            result.append(fields)
    return result


def mount_path(host_path):
    path = str(host_path).replace('\\', '/')
    if re.match(r'^[A-Za-z]:/', path):
        return '/' + path[0].lower() + path[2:]
    return path


def sbx(name, *arguments):
    output = run(['sbx', 'exec', name, *arguments])
    # A successful native command is still rejected if its JSON is not parseable.
    return json.loads(output.decode('utf-8-sig'))


def running_sandboxes_using(paths):
    # A sandbox mounting a folder above, at or inside a relocated path.
    listed = json.loads(run(['sbx', 'ls', '--json']).decode('utf-8-sig'))
    names = []
    for box in listed:
        mounts = [re.sub(r':ro$', '', mount) for mount in box.get('workspaces') or []]
        if box.get('status') == 'running' and any(inside(m, p) or inside(p, m) for m in mounts for p in paths):
            names.append(box['name'])
    return names


def stop_sandboxes(names):
    # Sandbox processes with files open through a mount make Windows refuse to
    # rename that folder. Stopping keeps sandbox storage for the restart.
    print('Stopping sandboxes so Windows can move their mounted folders: ' + ', '.join(names), flush=True)
    run(['sbx', 'stop', *names])
    deadline = time.time() + 120
    while not_yet_stopped(names):
        if time.time() > deadline:
            raise RuntimeError('Sandboxes did not stop: ' + ', '.join(not_yet_stopped(names)))
        time.sleep(2)


def not_yet_stopped(names):
    listed = json.loads(run(['sbx', 'ls', '--json']).decode('utf-8-sig'))
    return [box['name'] for box in listed if box['name'] in names and box.get('status') != 'stopped']


def restart_sandboxes(names):
    failed = []
    for name in names:
        try:
            run(['sbx', 'run', '--name', name, '--detach'])
        except RuntimeError as error:
            print(f'Could not restart sandbox {name}: {error}', file=sys.stderr)
            failed.append(name)
    for name in failed:
        print(f'Start it later with: sbx run --name {name} --detach', file=sys.stderr)
    return failed


def relocate(source, target):
    # Antivirus and indexers hold files briefly. Anything still holding files
    # after that, such as a terminal or editor inside the folder, blocks the move.
    for attempt in range(30):
        try:
            source.rename(target)
            return
        except PermissionError:
            if attempt == 29:
                raise RuntimeError(f'Windows still has files open in {source}, so it cannot be moved. Close every '
                                   'editor, terminal, Explorer window, Git client and dev server using it, then rerun.')
            time.sleep(1)


PROBE = '''import json,os,shutil,sys
from pathlib import Path
p=Path(sys.argv[1]); d=Path(sys.argv[2])
assert (p/'.git').is_dir(), 'Expected direct-mode repository mount is unavailable: '+str(p)
previous=d.is_dir() and not d.is_symlink() and (d/'migration/verified.json').is_file()
assert previous or not (d.exists() or d.is_symlink()), 'Destination already exists: '+str(d)
a=d
while not a.exists(): a=a.parent
assert a.is_dir(), 'Destination parent is not a directory'
assert not str(d.resolve()).startswith(str(p.resolve())+'/'), 'Destination must not be on the host mount'
assert os.stat(a).st_dev != os.stat(p).st_dev, 'Destination is on the Windows-backed filesystem'
assert shutil.which('git'), 'git is missing in the sandbox'
assert sys.version_info >= (3,10), 'Python 3.10+ required in the sandbox'
print(json.dumps({'free':shutil.disk_usage(a).free,'mount':str(p),'destination':str(d),'previous_import':previous}))
'''


def inspect_repository(root, name, destination, skip_caches):
    if not root.is_dir():
        raise RuntimeError(f'Repository directory does not exist: {root}')
    if not (root / '.git').is_dir():
        raise RuntimeError(f'Expected the standalone main checkout with its own .git directory: {root}. '
                           'A linked worktree or external Git directory cannot be used as the primary checkout.')
    if normal(git(root, 'rev-parse', '--show-toplevel').decode().strip()) != normal(root):
        raise RuntimeError(f'Unexpected repository root: {root}')
    if git(root, 'config', '--get', 'remote.origin.url').strip() == b'':
        raise RuntimeError(f'origin is not configured: {root}')
    origin = git(root, 'remote', 'get-url', 'origin').decode().strip()
    if not (re.match(r'^(https?://|ssh://|git@|[^/\\]+@[^:]+:)', origin)):
        raise RuntimeError(f'origin must be an upstream network remote for {name}, not a local path')
    roots = []
    missing = []
    records = worktrees(root)
    used = {'checkout'}
    for record in records:
        source = Path(record['worktree'])
        primary = normal(source) == normal(root)
        relative = 'checkout' if primary else 'worktrees/' + source.name
        if relative in used and not primary:
            relative += '-' + uuid.uuid4().hex[:8]
        used.add(relative)
        entry = {'source': str(source), 'relative': relative, 'record': record}
        if source.is_dir():
            common = git(source, 'rev-parse', '--path-format=absolute', '--git-common-dir').decode().strip()
            if normal(common) != normal(root / '.git'):
                raise RuntimeError(f'Worktree has an unexpected common Git directory: {source}')
            entry['git_dir'] = git(source, 'rev-parse', '--absolute-git-dir').decode().strip()
            entry['snapshot'] = snapshot(source)
            entry['portable_settings'] = portable_settings(source)
            roots.append(entry)
        else:
            missing.append(entry)
    if not any(e['relative'] == 'checkout' for e in roots):
        raise RuntimeError(f'Main checkout missing from Git worktree list: {root}')
    probe = sbx(name, 'python3', '-c', PROBE, mount_path(root), destination)
    return {'name': name, 'source': str(root), 'destination': destination,
            'mount': probe['mount'], 'free': probe['free'], 'roots': roots,
            'missing_worktrees': missing, 'skip_build_caches': skip_caches,
            'origin': origin, 'previous_import': probe.get('previous_import', False)}


STOP_WINDOWS_RELAY = r'''
function Get-MigrationPathPattern($Value) {
    $text = ([string]$Value).Replace('\', '/').TrimEnd('/')
    if (-not $text) { throw 'Empty relay identity path' }
    $escaped = [regex]::Escape($text)
    $pattern = '"' + $escaped + '"'
    if ($text -notmatch '\s') { $pattern += '|' + $escaped }
    return '(?:' + $pattern + ')'
}
# Two launch forms exist: the checkout's scripts/agentbus/relay.py with
# --directory, and the sbx helper agentbus-host.py with a JSON config whose
# directory is this repository's relay directory (checked by the caller).
function Test-MigrationRelayCommand($ProcessName, $CommandLine, $Spec) {
    if ($ProcessName -notmatch '^python(?:w|[0-9.]+)?$') { return $false }
    $command = ([string]$CommandLine).Replace('\', '/')
    $interpreter = '^\s*(?:"[^"]+"|[^\s"]+)\s+'
    $scriptPattern = $interpreter + (Get-MigrationPathPattern $Spec.script) + '(?=\s|$)'
    $directoryPattern = '(?:^|\s)--directory(?:\s+|=)' + (Get-MigrationPathPattern $Spec.directory) + '(?=\s|$)'
    if ($command -match $scriptPattern -and $command -match $directoryPattern) { return $true }
    foreach ($helper in @($Spec.helpers)) {
        if ($null -eq $helper) { continue }
        $helperPattern = $interpreter + (Get-MigrationPathPattern $helper.runner) + '\s+' +
            (Get-MigrationPathPattern $helper.config) + '\s*$'
        if ($command -match $helperPattern) { return $true }
    }
    return $false
}
$ErrorActionPreference = 'Stop'
$relayProcess = $null
try {
    try { $relayProcess = [Diagnostics.Process]::GetProcessById([int]$spec.pid) }
    catch [ArgumentException] {
        @{state='not-running'} | ConvertTo-Json -Compress
        return
    }
    # Hold the process handle while checking its command line and stopping it.
    # A recycled heartbeat PID must never cause an unrelated process to be killed.
    $null = $relayProcess.Handle
    if ($relayProcess.HasExited) {
        @{state='not-running'} | ConvertTo-Json -Compress
        return
    }
    $details = Get-CimInstance Win32_Process -Filter ('ProcessId = ' + [int]$spec.pid)
    if ($relayProcess.HasExited) {
        @{state='not-running'} | ConvertTo-Json -Compress
        return
    }
    if (-not (Test-MigrationRelayCommand $relayProcess.ProcessName $details.CommandLine $spec)) {
        @{state='unrelated'} | ConvertTo-Json -Compress
        return
    }
    # A client holds <id>.waiting for its whole exchange; <id>.json is queued
    # and <id>.running is executing. Any of them means the relay is in use.
    $requests = Join-Path ([string]$spec.directory) 'requests'
    function Get-ActiveRequests {
        if (-not (Test-Path -LiteralPath $requests)) { return @() }
        return @(Get-ChildItem -LiteralPath $requests -File |
            Where-Object { $_.Extension -in '.waiting', '.json', '.running' })
    }
    $active = Get-ActiveRequests
    if ($active.Count) {
        throw ('Agentbus has active requests (' + $active[0].FullName + '). Close its clients before applying the migration. ' +
               'If no agent is running, that file is left over from a killed client and can be deleted.')
    }
    $relayProcess.Kill()
    if (-not $relayProcess.WaitForExit(10000)) { throw 'Agentbus relay did not exit within 10 seconds.' }
    # A request claimed between the check and the kill leaves an agentbus
    # child that outlives the relay and may still write the database.
    if (@(Get-ActiveRequests | Where-Object { $_.Extension -eq '.running' }).Count) {
        throw 'Agentbus relay was stopped while it started a request; its agentbus process may still be running. Wait for it to finish, then apply again.'
    }
    @{state='stopped'} | ConvertTo-Json -Compress
} finally {
    if ($null -ne $relayProcess) { $relayProcess.Dispose() }
}
'''


def relay_helpers(directory):
    # sbx starts some relays as %APPDATA%\sbx\helpers\agentbus-host.py <config>,
    # where the config names the relay directory it serves.
    if not os.environ.get('APPDATA'):
        return []
    helpers = Path(os.environ['APPDATA']) / 'sbx' / 'helpers'
    runner = helpers / 'agentbus-host.py'
    found = []
    if runner.is_file():
        for config in sorted(helpers.glob('*.json')):
            try:
                value = json.loads(config.read_text(encoding='utf-8-sig')).get('directory')
            except (OSError, ValueError, AttributeError):
                continue
            if isinstance(value, str) and normal(value) == normal(directory):
                found.append({'runner': str(runner), 'config': str(config)})
    return found


def startup_relay_shortcuts():
    if not os.environ.get('APPDATA'):
        return []
    startup = Path(os.environ['APPDATA']) / 'Microsoft/Windows/Start Menu/Programs/Startup'
    return sorted(str(path) for path in startup.glob('Agentbus-*.lnk'))


def stop_windows_relay(pid, script, directory):
    # JSON travels as base64 data, never as executable PowerShell interpolation.
    payload = base64.b64encode(json.dumps({'pid': pid, 'script': str(script), 'directory': str(directory),
                                         'helpers': relay_helpers(directory)}).encode()).decode()
    source = ("$spec = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('"
              + payload + "')) | ConvertFrom-Json\n" + STOP_WINDOWS_RELAY)
    encoded = base64.b64encode(source.encode('utf-16le')).decode()
    output = run(['powershell.exe', '-NoProfile', '-NonInteractive', '-EncodedCommand', encoded])
    return json.loads(output.decode('utf-8-sig'))


def relay_heartbeat(plan):
    return Path(plan['source']) / '.git/agentbus/relay/heartbeat'


def read_heartbeat(heartbeat):
    # The relay replaces its heartbeat every second; Windows can briefly deny
    # a read that overlaps the rename.
    for attempt in range(20):
        try:
            return read_json(heartbeat)
        except (PermissionError, ValueError):
            if attempt == 19:
                raise
            time.sleep(0.1)


def report_relay(plan):
    heartbeat = relay_heartbeat(plan)
    if heartbeat.is_file():
        state = read_heartbeat(heartbeat)
        if time.time() - state.get('timestamp', 0) < 30:
            print('  Agentbus relay is running; preview leaves it running. '
                  '-Apply will verify its identity and stop it before copying.', flush=True)


def stop_host_relays(plans, audit):
    for plan in plans:
        root = Path(plan['source'])
        heartbeat = relay_heartbeat(plan)
        if heartbeat.is_file():
            state = read_heartbeat(heartbeat)
            pid = state.get('pid')
            recent = time.time() - state.get('timestamp', 0) < 30
            result = {'state': 'stale'}
            if isinstance(pid, int) and not isinstance(pid, bool) and pid > 0:
                result = stop_windows_relay(pid, root / 'scripts/agentbus/relay.py', heartbeat.parent)
                if result.get('state') not in ('stopped', 'not-running', 'unrelated'):
                    raise RuntimeError('Unexpected Agentbus relay process verification result')
                if result['state'] == 'unrelated' and recent:
                    raise RuntimeError(f"{plan['name']}: recent relay heartbeat does not match its process; "
                                       'no process was stopped. Close the relay manually before applying.')
            elif recent:
                raise RuntimeError(f"{plan['name']}: relay heartbeat has no valid PID; stop its relay manually first")
            if heartbeat.is_file():
                after = read_heartbeat(heartbeat)
                if (after.get('pid'), after.get('instance')) != (state.get('pid'), state.get('instance')):
                    raise RuntimeError(f"{plan['name']}: Agentbus relay restarted while stopping; originals are retained")
            plan['relay_stop'] = {'heartbeat': state, **result}
            write_json(audit / 'plan.json', plans)
            print(f"{plan['name']}: Agentbus relay {result['state']}.", flush=True)
        database = root / '.git/agentbus/bus.db'
        if database.is_file():
            # Killing an idle relay leaves SQLite able to recover its WAL. Make
            # the checkpoint explicit and refuse to copy a busy database.
            connection = sqlite3.connect(str(database), timeout=5)
            try:
                try:
                    busy, _, _ = connection.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()
                except sqlite3.OperationalError as error:
                    raise RuntimeError(f"{plan['name']}: cannot checkpoint the Agentbus database ({error}); "
                                       'close its clients first') from error
                if busy:
                    raise RuntimeError(f"{plan['name']}: Agentbus database is still in use; close its clients first")
            finally:
                connection.close()
        plan['relay_heartbeat_sha256'] = digest_file(heartbeat) if heartbeat.is_file() else None


def assert_relay_unchanged(plan):
    heartbeat = relay_heartbeat(plan)
    current = digest_file(heartbeat) if heartbeat.is_file() else None
    if current != plan.get('relay_heartbeat_sha256'):
        raise RuntimeError(f"{plan['name']}: Agentbus relay restarted during migration; originals are retained")


def excluded(plan, root, path, staging, relative):
    if inside(path, Path(plan['source']) / '.git/sandbox-migration'):
        return True
    for other in plan['roots']:
        if normal(other['source']) != normal(root) and inside(other['source'], root):
            if inside(path, other['source']):
                return True
    if plan['skip_build_caches'] and path.is_dir() and not path.is_symlink() and path.name in CACHES:
        # A cache-named directory with tracked files (bcc-rust's src/target) is
        # source, so it is copied in full. Query from inside the directory so
        # submodules and nested repositories get their own protection as well.
        # Relocated originals cannot run Git, so they reuse the recorded choice.
        kept = plan.setdefault('kept_cache_names', [])
        if not plan.get('relocated_verification') and relative not in kept \
                and git(path, 'ls-files', '-z', '--', '.'):
            kept.append(relative)
        return relative not in kept
    return False


def walk_files(plan, staging, allow_disappearing=False):
    for entry in plan['roots']:
        root = Path(entry['source'])
        stack = [(root, entry['relative'])]
        while stack:
            path, relative = stack.pop()
            if excluded(plan, root, path, staging, relative):
                continue
            try:
                attributes = path.lstat()
            except FileNotFoundError:
                if allow_disappearing:
                    continue
                raise
            except PermissionError:
                raise unreadable(path) from None
            yield path, relative
            # Junctions and symlinks are archived as links, never followed, so
            # they cannot pull an external directory into the archive.
            if stat.S_ISDIR(attributes.st_mode) and not is_junction(attributes) and not path.is_symlink():
                try:
                    children = sorted(path.iterdir(), reverse=True)
                except FileNotFoundError:
                    if allow_disappearing:
                        continue
                    raise
                except PermissionError:
                    raise unreadable(path) from None
                stack.extend((child, relative + '/' + child.name) for child in children)


def check_metadata(path):
    if path.name == 'alternates' and path.parent.name == 'info' and path.stat().st_size:
        raise RuntimeError(f'External Git object alternates must be made self-contained first: {path}')
    if path.name in GIT_LOCKS and time.time() - path.stat().st_mtime < 3600:
        raise RuntimeError(f'Recent Git lock detected; stop active Git clients before migrating: {path}')
    if path.name in ('rebase-merge', 'rebase-apply', 'sequencer') and path.is_dir():
        raise RuntimeError(f'Finish or abort the in-progress Git operation first: {path}')


def inventory(plan, staging):
    total = count = 0
    links = {'junctions': 0, 'sandbox_links': 0}
    for path, relative in walk_files(plan, staging, allow_disappearing=True):
        try:
            if '/.git/' in '/' + relative or relative.endswith('/.git'):
                check_metadata(path)
            attributes = path.lstat()
            if is_junction(attributes) or path.is_symlink():
                # Translate now so that a link leaving the repositories stops
                # the preview rather than -Apply.
                translated_link(plan, path, link_text(path), relative)
                if is_junction(attributes):
                    links['junctions'] += 1
                elif sandbox_absolute(link_text(path)):
                    links['sandbox_links'] += 1
            elif path.is_file():
                total += attributes.st_size
                count += 1
        except FileNotFoundError:
            # Preview may observe atomic relay heartbeat/log updates. Copying
            # and final source verification still use strict traversal.
            continue
    plan['bytes'] = total
    plan['files'] = count
    plan.update(links)
    # Allow filesystem allocation overhead and leave at least 1 GiB available.
    if total * 1.15 + 1024**3 > plan['free']:
        raise RuntimeError(f"{plan['name']}: {total / 1024**3:.1f} GiB to copy, "
                           f"{plan['free'] / 1024**3:.1f} GiB available. "
                           'Free sandbox space or preview again with -SkipBuildCaches.')


class HashReader:
    def __init__(self, stream):
        self.stream = stream
        self.digest = hashlib.sha256()

    def read(self, size=-1):
        block = self.stream.read(size)
        self.digest.update(block)
        return block


def translated_link(plan, source, link, target_relative):
    if sandbox_absolute(link):
        return link.replace('\\', '/')
    if not os.path.isabs(link):
        # Relative links inside a worktree preserve their original spelling.
        resolved = os.path.normpath(os.path.join(source.parent, link))
    else:
        resolved = os.path.normpath(link)
    for root in sorted(plan['roots'], key=lambda e: len(e['source']), reverse=True):
        if inside(resolved, root['source']):
            suffix = str(resolved).replace('\\', '/')[len(str(root['source']).replace('\\', '/')):].lstrip('/')
            dest = root['relative'] + ('/' + suffix if suffix else '')
            translated = posixpath.relpath(dest, posixpath.dirname(target_relative))
            # Preserve Git-tracked relative symlink text byte for byte whenever
            # the directory relationship is unchanged (including leading ./).
            if not os.path.isabs(link) and posixpath.normpath(link) == translated:
                return link
            return translated
    raise RuntimeError(f'Symlink points outside the migrated repositories: {source} -> {link}')


def export_repository(plan, staging):
    archive = staging / 'working-state.tar'
    entries = []
    with tarfile.open(archive, 'w', format=tarfile.PAX_FORMAT) as output:
        for path, relative in walk_files(plan, staging):
            attributes = path.lstat()
            if is_junction(attributes):
                # A junction becomes an equivalent relative symlink in the sandbox.
                info = tarfile.TarInfo(relative)
                info.type = tarfile.SYMTYPE
                info.mode = 0o777
                info.mtime = int(attributes.st_mtime)
            else:
                info = output.gettarinfo(str(path), arcname=relative)
            if info.islnk():
                # Materialize hardlinks independently; extraction never follows
                # archive hardlinks or depends on host object-store hardlinks.
                info.type = tarfile.REGTYPE
                info.linkname = ''
                info.size = path.stat().st_size
            info.uid = info.gid = 0
            info.uname = info.gname = ''
            record = {'source': str(path), 'relative': relative}
            if info.issym():
                record.update(kind='junction' if is_junction(attributes) else 'link',
                              source_link=os.readlink(path))
                target = link_text(path)
                info.linkname = translated_link(plan, path, target, relative)
                if record['kind'] == 'link' and sandbox_absolute(target):
                    record['sandbox_absolute'] = True
                record['link'] = info.linkname
                output.addfile(info)
            elif info.isdir():
                record['kind'] = 'directory'
                output.addfile(info)
            elif info.isfile():
                before = path.stat()
                with path.open('rb') as stream:
                    reader = HashReader(stream)
                    output.addfile(info, reader)
                after = path.stat()
                if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
                    raise RuntimeError(f'File changed during copy: {path}')
                record.update(kind='file', size=info.size, sha256=reader.digest.hexdigest())
            else:
                raise RuntimeError(f'Unsupported special file: {path}')
            entries.append(record)
    plan['entries'] = entries
    plan['archive_sha256'] = digest_file(archive)
    write_json(staging / 'plan.json', plan)
    return archive


def assert_sources_unchanged(plan, staging, check_git=True):
    expected = {e['relative']: e for e in plan['entries']}
    actual = set()
    for source, relative in walk_files(plan, staging):
        actual.add(relative)
        entry = expected.get(relative)
        if entry is None:
            raise RuntimeError(f'New path appeared during migration: {source}')
        if entry['kind'] == 'file':
            if source.is_symlink() or not source.is_file() or digest_file(source) != entry['sha256']:
                raise RuntimeError(f'File changed during migration: {source}')
        elif entry['kind'] == 'link':
            if not source.is_symlink() or os.readlink(source) != entry['source_link']:
                raise RuntimeError(f'Link changed during migration: {source}')
        elif entry['kind'] == 'junction':
            if not is_junction(source.lstat()) or os.readlink(source) != entry['source_link']:
                raise RuntimeError(f'Junction changed during migration: {source}')
        elif source.is_symlink() or is_junction(source.lstat()) or not source.is_dir():
            raise RuntimeError(f'Directory changed during migration: {source}')
    if actual != set(expected):
        raise RuntimeError('Paths disappeared during migration; originals have been retained')
    if check_git:
        for entry in plan['roots']:
            if snapshot(entry['source']) != entry['snapshot']:
                raise RuntimeError(f"Git state changed during migration: {entry['source']}")


def rewrite_path(plan, value):
    value = value.strip()
    for root in sorted(plan['roots'], key=lambda e: len(e['source']), reverse=True):
        if inside(value, root['source']):
            suffix = value.replace('\\', '/')[len(root['source'].replace('\\', '/')):].lstrip('/')
            return plan['destination'] + '/' + root['relative'] + ('/' + suffix if suffix else '')
    for root in plan['missing_worktrees']:
        if inside(value, root['source']):
            suffix = value.replace('\\', '/')[len(root['source'].replace('\\', '/')):].lstrip('/')
            return plan['destination'] + '/missing-' + root['relative'] + ('/' + suffix if suffix else '')
    return value


def rewrite_git_path(plan, source_file, value):
    # Resolve relative Git paths against their ORIGINAL administrative file.
    # Worktrees that used to be siblings now live under worktrees/, so retaining
    # relative submodule paths would point them at the wrong checkout.
    flavor = ntpath if re.match(r'^[A-Za-z]:', source_file) else posixpath
    original = flavor.normpath(flavor.join(flavor.dirname(source_file), value))
    translated = rewrite_path(plan, original)
    if translated == original:
        raise RuntimeError(f'Git metadata points outside the migration: {source_file}: {value}')
    return translated


def config_set(path, key, value=None):
    if value is not None:
        run(['git', 'config', '--file', path, key, value])
    else:
        result = subprocess.run(['git', 'config', '--file', str(path), '--unset-all', key],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if result.returncode not in (0, 5):
            raise RuntimeError(f'Cannot remove {key} from {path}')


def repair_git_paths(plan):
    destination = Path(plan['destination'])
    common = destination / 'checkout/.git'
    metadata = []
    # The full source-to-destination map also handles modules and per-worktree
    # submodule gitdirs, rather than fetching or reinitializing submodules.
    for entry in plan['entries']:
        path = destination / entry['relative']
        if entry['kind'] != 'file':
            continue
        relative = entry['relative']
        if path.name == '.git' or ('/.git/' in '/' + relative and
                path.name in ('gitdir', 'commondir', 'config', 'config.worktree', *GIT_LOCKS)):
            saved = destination / 'migration/original-metadata' / relative
            saved.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, saved)
        if path.name == '.git':
            text = path.read_text(encoding='utf-8').strip()
            if text.startswith('gitdir:'):
                value = text.partition(':')[2].strip()
                path.write_text('gitdir: ' + rewrite_git_path(plan, entry['source'], value) + '\n', encoding='utf-8')
        if '/.git/' in '/' + relative:
            if path.name in ('gitdir', 'commondir'):
                value = path.read_text(encoding='utf-8').strip()
                path.write_text(rewrite_git_path(plan, entry['source'], value) + '\n', encoding='utf-8')
            elif path.name in ('config', 'config.worktree'):
                metadata.append((path, entry['source']))
            elif path.name in GIT_LOCKS:
                # Old locks are present in the verified archive, but must not
                # keep the independent sandbox copy permanently locked.
                path.unlink()
    for config, original in metadata:
        for key in ('core.hooksPath', 'core.fsmonitor'):
            config_set(config, key)
        config_set(config, 'core.filemode', 'false')
        result = subprocess.run(['git', 'config', '--file', str(config), '--get', 'core.worktree'],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if result.returncode == 0:
            value = result.stdout.decode().strip()
            config_set(config, 'core.worktree', rewrite_git_path(plan, original, value))
        elif result.returncode != 1:
            raise RuntimeError(f'Cannot read {config}')
    relay_config = common / 'agentbus/relay/config.json'
    if relay_config.is_file():
        saved = destination / 'migration/original-metadata/checkout/.git/agentbus/relay/config.json'
        saved.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(relay_config, saved)
        config = read_json(relay_config)
        config['sandboxes'] = []
        write_json(relay_config, config)
    linked = [str(destination / e['relative']) for e in plan['roots'] if e['relative'] != 'checkout']
    git(destination / 'checkout', 'worktree', 'repair', *linked)
    # Effective host settings can differ by worktree. Enable the supported Git
    # mechanism before recording them, including repositories that did not use
    # it before the migration.
    config_set(common / 'config', 'extensions.worktreeConfig', 'true')
    main_worktree = subprocess.run(['git', 'config', '--file', str(common / 'config'), '--get', 'core.worktree'],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if main_worktree.returncode == 0:
        config_set(common / 'config.worktree', 'core.worktree', main_worktree.stdout.decode().strip())
        config_set(common / 'config', 'core.worktree')
    elif main_worktree.returncode != 1:
        raise RuntimeError('Cannot read main core.worktree setting')
    # A primary worktree's Git directory is a directory; linked .git files and
    # administrative gitdir files now point exclusively into the sandbox.
    for entry in plan['roots']:
        root = destination / entry['relative']
        resolved_common = Path(git(root, 'rev-parse', '--path-format=absolute', '--git-common-dir').decode().strip())
        if resolved_common.resolve() != common.resolve():
            raise RuntimeError(f'Git common directory escaped the sandbox: {root}')
        settings = entry.get('portable_settings', {'values': {}, 'files': {}})
        for key, value in settings['values'].items():
            git(root, 'config', '--worktree', key, value)
        for key, encoded in settings['files'].items():
            path = destination / 'migration/git-settings' / entry['relative'] / key
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(base64.b64decode(encoded))
            git(root, 'config', '--worktree', key, str(path))


def retire_unused_import(destination):
    # A verified copy from an earlier run that stopped before host replacement
    # is moved aside, never deleted, and only while it is exactly as imported.
    manifest = destination / 'migration/manifest.json'
    if destination.is_symlink() or not (destination / 'migration/verified.json').is_file() or not manifest.is_file():
        raise RuntimeError(f'Refusing to overwrite existing destination: {destination}')
    previous = read_json(manifest)
    for entry in previous['roots']:
        if snapshot(destination / entry['relative']) != entry['snapshot']:
            raise RuntimeError(f"{destination / entry['relative']} has changed since run {previous['run_id']} "
                               'imported it. Move that copy elsewhere in the sandbox, then retry.')
    retired = destination.parent / '.superseded-migrations' / (previous['run_id'] + '-' + destination.name)
    retired.parent.mkdir(exist_ok=True)
    destination.rename(retired)
    print(f'Unused copy from run {previous["run_id"]} moved to {retired}', file=sys.stderr)


def sandbox_import(plan_path):
    plan_path = Path(plan_path)
    plan = read_json(plan_path)
    destination = Path(plan['destination'])
    if destination.exists() or destination.is_symlink():
        retire_unused_import(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    if os.stat(destination.parent).st_dev == os.stat(plan['mount']).st_dev:
        raise RuntimeError('Destination is on the host filesystem, not sandbox storage')
    if shutil.disk_usage(destination.parent).free < plan['bytes'] * 1.15 + 1024**3:
        raise RuntimeError('Insufficient sandbox storage; no host data has been changed')
    archive = plan_path.parent / 'working-state.tar'
    if digest_file(archive) != plan['archive_sha256']:
        raise RuntimeError('Archive checksum mismatch')
    destination.mkdir()
    try:
        with tarfile.open(archive, 'r') as source:
            expected = {entry['relative'] for entry in plan['entries']}
            absolute_links = {entry['relative']: entry['link'] for entry in plan['entries']
                              if entry.get('sandbox_absolute')}
            members = source.getmembers()
            if {m.name.rstrip('/') for m in members} != expected or len(members) != len(expected):
                raise RuntimeError('Archive contents do not match the manifest')
            for member in members:
                parts = PurePosixPath(member.name).parts
                if member.name.startswith('/') or '..' in parts or member.islnk():
                    raise RuntimeError('Unsafe path in archive: ' + member.name)
                if not (member.isdir() or member.isfile() or member.issym()):
                    raise RuntimeError('Unsupported archive entry: ' + member.name)
                absolute = member.issym() and absolute_links.get(member.name) == member.linkname
                if member.issym() and not absolute:
                    target = posixpath.normpath(posixpath.join(posixpath.dirname(member.name), member.linkname))
                    if member.linkname.startswith('/') or target == '..' or target.startswith('../'):
                        raise RuntimeError('Unsafe symlink in archive: ' + member.name)
                # Parent components cannot be symlinks, even for self-produced archives.
                for parent in (destination / member.name).parents:
                    if parent == destination:
                        break
                    if parent.is_symlink():
                        raise RuntimeError('Archive path traverses a symlink')
                if absolute:
                    # A link the sandbox itself made to its own storage, such as
                    # /home/agent/.cache/... Tar filters refuse absolute links.
                    if not member.linkname.startswith('/'):
                        raise RuntimeError('Unexpected sandbox link in archive: ' + member.name)
                    os.symlink(member.linkname, destination / member.name)
                elif sys.version_info >= (3, 12):
                    def preserve_link_text(item, target):
                        filtered = tarfile.data_filter(item, target)
                        if filtered is not None and item.issym():
                            filtered = filtered.replace(linkname=item.linkname)
                        return filtered
                    source.extract(member, destination, filter=preserve_link_text)
                else:
                    source.extract(member, destination)
        for entry in plan['entries']:
            path = destination / entry['relative']
            if entry['kind'] == 'file':
                if path.is_symlink() or not path.is_file() or digest_file(path) != entry['sha256']:
                    raise RuntimeError(f'Copied file failed verification: {path}')
            elif entry['kind'] in ('link', 'junction'):
                if not path.is_symlink() or os.readlink(path) != entry['link']:
                    raise RuntimeError(f'Copied symlink failed verification: {path}')
            elif not path.is_dir() or path.is_symlink():
                raise RuntimeError(f'Copied directory failed verification: {path}')
        # Retain a record before administrative path conversion changes .git files.
        (destination / 'migration').mkdir()
        shutil.copy2(plan_path, destination / 'migration/manifest.json')
        shutil.copy2(__file__, destination / 'migration/migrate.py')
        repair_git_paths(plan)
        for entry in plan['roots']:
            actual = snapshot(destination / entry['relative'])
            if actual != entry['snapshot']:
                differing = [key for key in actual if actual[key] != entry['snapshot'][key]]
                raise RuntimeError(f"Git state mismatch in {entry['relative']}: {', '.join(differing)}. "
                                   'Check compatible Git filters (such as git-lfs) and global attributes; originals are retained.')
        write_json(destination / 'migration/verified.json', {'verified': True, 'time': time.time()})
    except Exception:
        # Keep failed imports for investigation; never roll back by deleting work.
        print(f'Incomplete sandbox copy retained at {destination}; host originals are intact.', file=sys.stderr)
        raise
    return {'verified': True, 'destination': str(destination), 'worktrees': len(plan['roots'])}


def verify_clean_clone(root):
    if git(root, 'branch', '--show-current').strip() != b'main':
        raise RuntimeError(f'Host checkout is not on main: {root}')
    if git(root, 'status', '--porcelain=v1', '--untracked-files=all').strip():
        raise RuntimeError(f'Host checkout is dirty: {root}')
    if git(root, 'rev-parse', 'HEAD') != git(root, 'rev-parse', 'refs/remotes/origin/main'):
        raise RuntimeError(f'Host checkout does not match origin/main: {root}')
    if git(root, 'for-each-ref', '--format=%(refname)', 'refs/heads').strip() != b'refs/heads/main':
        raise RuntimeError(f'Host checkout contains extra local branches: {root}')
    if len(worktrees(root)) != 1:
        raise RuntimeError(f'Host clone has unexpected worktrees: {root}')


def repository_source(args, name, repo_root):
    attribute, option = ('prevalabs_path', '-PrevalabsPath') if name == 'prevalabs-mono' else ('bcc_rust_path', '-BccRustPath')
    override = getattr(args, attribute, None)
    if override:
        candidate = Path(override).expanduser().resolve()
        if not candidate.is_dir():
            raise RuntimeError(f'{option} directory does not exist: {candidate}')
        return candidate
    candidates = [repo_root / 'prevalabs/prevalabs-mono', repo_root / name] if name == 'prevalabs-mono' else [repo_root / name]
    found = [path for path in candidates if path.is_dir() and (path / '.git').exists()]
    if len(found) == 1:
        return found[0]
    if len(found) > 1:
        raise RuntimeError(f'Multiple checkouts found for {name}: ' + ', '.join(map(str, found))
                           + f'. Select the main checkout with {option}.')
    raise RuntimeError(f'Cannot find {name}. Searched: ' + ', '.join(map(str, candidates))
                       + f'. Specify the main checkout directory with {option}.')


def host(args, require_windows=True):
    if require_windows and os.name != 'nt':
        raise RuntimeError('Host mode must run on Windows')
    repo_root = Path(args.repo_root).resolve()
    run_id = time.strftime('%Y%m%d-%H%M%S') + '-' + uuid.uuid4().hex[:6]
    audit = repo_root / '.sandbox-migration' / run_id
    plans = []
    print('Inspecting both repositories and sandboxes. No source files will be changed during preview.', flush=True)
    for name in REPOSITORIES:
        root = repository_source(args, name, repo_root)
        destination = args.sandbox_root.rstrip('/') + '/' + name
        plan = inspect_repository(root, name, destination, args.skip_build_caches)
        # The transfer directory is inside .git so Git status is unaffected.
        staging = root / '.git/sandbox-migration' / run_id
        plan.update(staging=str(staging), run_id=run_id)
        inventory(plan, staging)
        print(f"{name}: {len(plan['roots'])} available worktrees, "
              f"{len(plan['missing_worktrees'])} missing registrations, "
              f"{plan['files']:,} files / {plan['bytes'] / 1024**3:.2f} GiB, "
              f"{plan['free'] / 1024**3:.2f} GiB sandbox space.\n"
              f"  Destination: {name}:{destination}", flush=True)
        if plan['previous_import']:
            print(f"  An earlier run's copy is already there. If unchanged since its import, it is moved to "
                  f"{args.sandbox_root.rstrip('/')}/.superseded-migrations; otherwise the import stops.", flush=True)
        if plan['junctions'] or plan['sandbox_links']:
            print(f"  {plan['junctions']} Windows junctions become relative symlinks; "
                  f"{plan['sandbox_links']} sandbox-made absolute symlinks are kept as is.", flush=True)
        for entry in plan['roots']:
            state = entry['snapshot']
            dirty = bool(base64.b64decode(state['status']))
            print(f"  {entry['source']} [{state['branch']}, {'dirty' if dirty else 'clean'}]")
        for entry in plan['missing_worktrees']:
            print(f"  MISSING before migration: {entry['source']} (metadata and refs preserved)")
        report_relay(plan)
        plans.append(plan)
    shortcuts = startup_relay_shortcuts()
    if shortcuts:
        print('\nThese Startup shortcuts restart Agentbus relays at logon. Delete them after migrating, or a relay '
              'will route sandbox Agentbus traffic to the clean host clones again:\n  ' + '\n  '.join(shortcuts))
    if not args.apply:
        print('\nPreview complete. Close all editors, agents, builds and Git clients, then rerun with -Apply.')
        return
    for plan in plans:
        for entry in plan['roots']:
            if inside(Path.cwd(), entry['source']):
                raise RuntimeError('Run -Apply from the parent GitRepo directory, outside all checkouts and worktrees')
    host_required = sum(plan['bytes'] for plan in plans) * 1.15 + 1024**3
    if shutil.disk_usage(repo_root).free < host_required:
        raise RuntimeError('Insufficient Windows disk space for temporary transfer archives; no files were moved')
    audit.mkdir(parents=True)
    write_json(audit / 'plan.json', plans)
    print(f'Journal and recovery paths: {audit}', flush=True)
    # Prepare clean upstream clones before touching the original directories.
    for plan in plans:
        clean = audit / (plan['name'] + '-clean')
        print(f"Preparing clean origin/main clone for {plan['name']}...", flush=True)
        run(['git', '-c', 'core.hooksPath=' + os.devnull, 'clone', '--no-local',
             '--single-branch', '--branch', 'main', '--no-tags', plan['origin'], clean])
        verify_clean_clone(clean)
        plan['clean'] = str(clean)
    stop_host_relays(plans, audit)
    write_json(audit / 'plan.json', plans)
    for plan in plans:
        assert_relay_unchanged(plan)
        inventory(plan, Path(plan['staging']))
        staging = Path(plan['staging'])
        staging.mkdir(parents=True)
        print(f"Copying and hashing {plan['name']} working state...", flush=True)
        shutil.copy2(__file__, staging / 'migrate.py')
        export_repository(plan, staging)
        remote_staging = plan['mount'] + '/.git/sandbox-migration/' + run_id
        result = sbx(plan['name'], 'python3', remote_staging + '/migrate.py',
                     'sandbox-import', remote_staging + '/plan.json')
        if result.get('verified') is not True:
            raise RuntimeError(f"Sandbox did not verify {plan['name']}")
        plan['sandbox_verified'] = True
        write_json(audit / 'plan.json', plans)
    stopped = running_sandboxes_using([e['source'] for plan in plans for e in plan['roots']])
    if stopped:
        stop_sandboxes(stopped)
    try:
        # Both copies must be verified and all original bytes still unchanged.
        for plan in plans:
            print(f"Rechecking original {plan['name']} files before relocation...", flush=True)
            assert_relay_unchanged(plan)
            assert_sources_unchanged(plan, Path(plan['staging']))
    except Exception:
        restart_sandboxes(stopped)
        raise
    moved = []
    try:
        for plan in plans:
            original = Path(plan['source'])
            main_backup = audit / (plan['name'] + '-original')
            # Rename the standalone repo and all external linked worktrees; nested
            # worktrees move along with their parent, never twice.
            sources = sorted({Path(e['source']) for e in plan['roots']}, key=lambda p: len(str(p)))
            top_sources = [p for p in sources if not any(p != q and inside(p, q) for q in sources)]
            plan['relocated'] = []
            for index, source in enumerate(top_sources):
                backup = main_backup if source == original else audit / f"{plan['name']}-worktree-{index}-{source.name}"
                relocate(source, backup)
                moved.append((backup, source))
                plan['relocated'].append({'source': str(source), 'backup': str(backup)})
                write_json(audit / 'plan.json', plans)
            relocate(Path(plan['clean']), original)
            plan['host_replaced'] = True
            write_json(audit / 'plan.json', plans)
            verify_clean_clone(original)
        for plan in plans:
            verify_clean_clone(Path(plan['source']))
    except Exception:
        print('Host replacement failed; restoring original directory locations.', file=sys.stderr)
        # Move the clean replacements back out before restoring originals.
        for plan in reversed(plans):
            if plan.get('host_replaced') and Path(plan['source']).exists():
                relocate(Path(plan['source']), Path(plan['clean']))
        for backup, original in reversed(moved):
            if backup.exists() and not original.exists():
                relocate(backup, original)
        restart_sandboxes(stopped)
        raise
    # Restarted sandboxes mount the clean clones. A copy missing after the
    # restart keeps every host backup.
    restart_failed = restart_sandboxes(stopped)
    confirmed = not restart_failed
    for plan in plans:
        if plan['name'] in restart_failed:
            continue
        marker = plan['destination'] + '/migration/verified.json'
        if run(['sbx', 'exec', plan['name'], 'sh', '-c', f"test -f '{marker}' && echo ok"]).strip() != b'ok':
            print(f"{plan['name']} sandbox copy not found after restart; host backups are kept.", file=sys.stderr)
            confirmed = False
    # Rehash relocated originals before deletion to catch late writers holding
    # handles to old directories. Their raw .git pointers have not been changed.
    # Skipped caches, if requested, remain only in retained host backups.
    for plan in plans:
        relocated = json.loads(json.dumps(plan))
        relocated['relocated_verification'] = True
        def backup_path(source):
            for move in sorted(plan['relocated'], key=lambda item: len(item['source']), reverse=True):
                if inside(source, move['source']):
                    return move['backup'] + str(source)[len(move['source']):]
            raise RuntimeError('Missing recovery mapping for ' + source)
        relocated['source'] = backup_path(plan['source'])
        for entry in relocated['roots']:
            entry['source'] = backup_path(entry['source'])
        for entry in relocated['entries']:
            entry['source'] = backup_path(entry['source'])
        assert_sources_unchanged(relocated, Path(backup_path(plan['staging'])), check_git=False)
    for plan in plans:
        primary_backup = audit / (plan['name'] + '-original')
        transfer = primary_backup / '.git/sandbox-migration' / run_id
        # Move the archive and helper into the journal before deleting old trees.
        transfer.rename(audit / (plan['name'] + '-transfer'))
    if args.keep_host_backups or args.skip_build_caches or not confirmed:
        print('\nRollback copies retained in ' + str(audit), flush=True)
    else:
        # Complete the move only after both clean clones and sandbox imports pass.
        # Windows readonly flags occasionally remain on Git object files.
        def make_writable(function, path, exc):
            os.chmod(path, stat.S_IWRITE | stat.S_IREAD)
            function(path)
        for backup, original in moved:
            shutil.rmtree(backup, onerror=make_writable)
        # Tar archives duplicate all work on the host, so remove them as well.
        for plan in plans:
            shutil.rmtree(audit / (plan['name'] + '-transfer'), onerror=make_writable)
        print('\nVerified originals and transfer archives removed from the host.', flush=True)
    write_json(audit / 'plan.json', plans)
    print('\nMigration complete. Both host repositories are clean main-only clones.', flush=True)
    for plan in plans:
        checkout = plan['destination'] + '/checkout'
        print(f"{plan['name']}: {checkout}\n"
              f"  Open that folder in the sandbox, or run:\n"
              f"  sbx exec {plan['name']} bash -lc 'cd {checkout} && exec bash'", flush=True)
    print('Existing direct mounts still expose the clean host repositories. Use the sandbox paths above for all new work.')
    print('Windows-specific Git hooks/fsmonitor were removed from the copied sandbox config; reinstall compatible hooks if needed.')
    if shortcuts:
        print('Delete these Agentbus Startup shortcuts before you next log on:\n  ' + '\n  '.join(shortcuts))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    host_parser = commands.add_parser('host')
    host_parser.add_argument('--repo-root', required=True)
    host_parser.add_argument('--prevalabs-path')
    host_parser.add_argument('--bcc-rust-path')
    host_parser.add_argument('--sandbox-root', default='/home/agent/workspace')
    host_parser.add_argument('--apply', action='store_true')
    host_parser.add_argument('--keep-host-backups', action='store_true')
    host_parser.add_argument('--skip-build-caches', action='store_true')
    remote = commands.add_parser('sandbox-import')
    remote.add_argument('plan')
    args = parser.parse_args()
    if sys.version_info < (3, 10):
        raise RuntimeError('Python 3.10+ is required')
    if args.command == 'host':
        host(args)
    else:
        print(json.dumps(sandbox_import(args.plan)))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print('\nSTOPPED: ' + str(error), file=sys.stderr)
        print('No original work is intentionally discarded on failure. Check the journal and any retained copies.', file=sys.stderr)
        sys.exit(1)
