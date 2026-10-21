#!/bin/sh
# Run from the assigned worktree root. All Cargo invocations use the shared
# scheduler; package artifacts are copied before its build lock is released.
set -eu
if [ "$(pwd -P)" != /home/agent/workspace/bcc-rust/worktrees/abstract-machine-site ]; then
    echo 'Run sh web/build.sh from /home/agent/workspace/bcc-rust/worktrees/abstract-machine-site.' >&2
    exit 1
fi
exec python3 - <<'PY'
import json
from pathlib import Path
import shutil
import subprocess
import sys

wrapper = Path('/home/agent/workspace/bcc-rust/triage-2026-10-16/swarm/run.py')
if not wrapper.is_file():
    raise SystemExit('The required shared Cargo scheduler is missing: ' + str(wrapper))
destination = Path('web/pkg')
destination.mkdir(exist_ok=True)
temporary = destination / 'bcc_rust.wasm.building'
command = [sys.executable, str(wrapper), 'cargo', 'rustc',
           '--no-default-features', '--features', 'abstract-machine',
           '--lib', '--crate-type', 'cdylib', '--target', 'wasm32-unknown-unknown',
           '--release', '--message-format', 'json-render-diagnostics']
print('Building Rust Wasm through the shared Cargo scheduler.', flush=True)
copied = False
with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                      text=True, encoding='utf-8') as process:
    for line in process.stdout:
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            print(line, end='', flush=True)
            continue
        if record.get('reason') == 'compiler-message':
            print(record['message'].get('rendered', ''), end='', flush=True)
        elif record.get('reason') == 'compiler-artifact' and record.get('target', {}).get('name') == 'bcc_rust':
            for filename in record.get('filenames', []):
                artifact = Path(filename)
                if artifact.suffix == '.wasm':
                    # The wrapper's shared cache can be cleaned immediately
                    # by another worktree after exit; copy while it still runs.
                    shutil.copyfile(artifact, temporary)
                    copied = True
    result = process.wait()
if result:
    temporary.unlink(missing_ok=True)
    raise SystemExit(result)
if not copied:
    raise SystemExit('Cargo reported no bcc_rust Wasm artifact; web/pkg was not updated.')
with temporary.open('rb') as artifact:
    if artifact.read(8) != b'\0asm\x01\0\0\0':
        temporary.unlink()
        raise SystemExit('The reported artifact is not WebAssembly v1.')
temporary.replace(destination / 'bcc_rust.wasm')
print('Built web/pkg/bcc_rust.wasm. Verify with:')
print('node --experimental-default-type=module web/check.mjs')
PY
