#!/bin/sh
set -eu
# Prerequisite: saved recovery-final-{release,debug} and reserved-baseline binaries.
git rev-parse HEAD > .scratch/parser-study/recovery-final-collect-revision
for profile in release debug; do
 binary=".scratch/parser-study/recovery-final-$profile"
 prefix=".scratch/parser-study/recovery-final-$profile"
 python3 scripts/parser_study.py diff --binary "$binary" --inputs .scratch/parser-study/grammar --manifest .scratch/parser-study/grammar/manifest.json --modes machine recursive growing hybrid:1 hybrid:3 hybrid:16 reserved:recursive --output "$prefix-grammar.json"
 python3 scripts/parser_study.py diff --binary "$binary" --modes machine recursive growing hybrid:1 hybrid:3 hybrid:16 reserved:recursive --output "$prefix-corpus.json"
 python3 scripts/parser_study.py bench --binary "$binary" --baseline-binary ".scratch/parser-study/reserved-baseline-$profile" --modes baseline machine recursive reserved:recursive --files lua-supported.i --repeats 9 --output "$prefix-timing.json"
 python3 scripts/parser_study.py stack --binary "$binary" --modes reserved:recursive --stack-memory --files parentheses-4096.c parentheses-16384.c blocks-4096.c blocks-16384.c --output "$prefix-stack.json"
done
python3 - <<'PY'
from pathlib import Path
import json
for p in Path('.scratch/parser-study').glob('recovery-final-*.json'):
 d=json.loads(p.read_text()); bad=[r for r in d['rows'] if r['returncode']!=0 or d['metadata']['action']=='diff' and r.get('identical') is not True]
 print(p.name, len(d['rows']), 'runs;', len(bad), 'failures')
 assert not bad, (p,bad)
PY
