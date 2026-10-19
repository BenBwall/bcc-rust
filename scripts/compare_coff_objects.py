#!/usr/bin/env python3
r"""Compare native COFF objects with pinned Clang's assembly of the same listing.

No LLVM inspection tool is required. The parser compares ordered section
headers/bytes (including .pdata/.xdata), all relocation records and in-place
addends, and symbols with section and weak auxiliary records. Only physical
file placement, string packing and symbol order may differ. Example:

  python scripts/compare_coff_objects.py objects reference.obj direct.obj
  python scripts/compare_coff_objects.py corpus --bcc /path/to/bcc-rust \
      --clang target/llvm/bin/clang --output /path/to/scratch \
      --extra-corpus /path/to/random-stress

The corpus command attempts every benchmark and C fixture at B0/B1 for both
Windows targets. Extra corpus directories are recursive and may be repeated.
Text refusals are recorded separately; a direct-only refusal fails the run.
"""

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys


def view(path):
    """Return every semantic field of a standard AMD64 COFF object."""
    data = Path(path).read_bytes()
    machine, count, timestamp, symtab, nsyms, optional, characteristics = struct.unpack_from('<HHIIIHH', data)
    assert machine == 0x8664 and optional == 0
    strtab = symtab + nsyms * 18
    string_size, = struct.unpack_from('<I', data, strtab)
    assert strtab + string_size == len(data)

    def name(raw, section=False):
        if raw[:4] == bytes(4):
            offset, = struct.unpack_from('<I', raw, 4)
        elif section and raw.startswith(b'/'):
            offset = int(raw[1:].split(b'\0')[0])
        else:
            return raw.split(b'\0')[0].decode('utf-8')
        assert 4 <= offset < string_size
        return data[strtab + offset:strtab + string_size].split(b'\0')[0].decode('utf-8')

    symbols = {}
    index = 0
    while index < nsyms:
        raw, value, section, kind, storage, naux = struct.unpack_from('<8sIhHBB', data, symtab + index * 18)
        assert index + naux < nsyms
        aux = data[symtab + (index + 1) * 18:symtab + (index + 1 + naux) * 18]
        symbols[index] = [name(raw), value, section, kind, storage, aux, '']
        index += 1 + naux
    for record in symbols.values():
        if record[4] == 105:  # weak external: compare the target, not its table index
            aux = record[5]
            target, = struct.unpack_from('<I', aux)
            record[6] = tuple(symbols[target][:5])
            record[5] = bytes(4) + aux[4:]
    sections, relocations, comdats = [], [], []
    for index in range(count):
        raw, virtual_size, address, size, pointer, relptr, lines, nrel, nlines, flags = struct.unpack_from('<8sIIIIIIHHI', data, 20 + index * 40)
        assert virtual_size == address == lines == nlines == 0
        body = b'' if flags & 0x80 or not size else data[pointer:pointer + size]
        if body:
            assert pointer >= 20 + count * 40 and len(body) == size
        sections.append((name(raw, True), size, flags, body))
        if flags & 0x1000:
            section_symbols = [s for s in symbols.values() if s[2] == index + 1]
            definition = next(s for s in section_symbols if s[4] == 3 and len(s[5]) == 18)
            if definition[5][14] != 5:  # associative COMDATs use the aux section number
                key = next((s for s in section_symbols if not s[5]), None)
                comdats.append((index, tuple(key[:5]) if key else None))
        for rel in range(nrel):
            offset, target, kind = struct.unpack_from('<IIH', data, relptr + rel * 10)
            assert offset < size
            relocations.append((index, offset, kind, tuple(symbols[target][:5])))
    return {'header': (machine, count, timestamp, optional, characteristics),
            'sections': sections, 'symbols': sorted(tuple(s) for s in symbols.values()),
            'relocations': relocations, 'comdats': comdats}


def compare(reference, direct):
    expected, actual = view(reference), view(direct)
    for field in expected:
        if actual[field] != expected[field]:
            return field, repr(actual[field]), repr(expected[field])
    return None


def run(command, timeout):
    command = [str(s) for s in command]
    try:
        return subprocess.run(command, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired as error:
        return subprocess.CompletedProcess(command, 124, error.stdout or b'',
                                           (error.stderr or b'') + b'\nvalidation timeout\n')


def corpus(args):
    root = Path(__file__).resolve().parents[1]
    sources = [] if args.only_extra else sorted((root / 'benches/c').glob('*.c')) + sorted((root / 'tests/fixtures').rglob('*.c'))
    for directory in args.extra_corpus:
        sources.extend(sorted(Path(directory).rglob('*.c')))
    sources = list(dict.fromkeys(sources))
    if not sources:
        raise ValueError('No C sources found for validation')
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    results = {}
    for target in ('x86_64-w64-windows-gnu', 'x86_64-pc-windows-msvc'):
        for optimize in (False, True):
            for source in sources:
                key = f'{target} B{int(optimize)} {source}'
                tag = hashlib.sha256(key.encode()).hexdigest()[:16]
                assembly, reference, direct = [output / (tag + suffix) for suffix in ('.s', '.reference.obj', '.direct.obj')]
                base = [args.bcc, f'--target={target}', '--backend=native', '-I', source.parent]
                if optimize:
                    base += ['--opt=bcc']
                text = run(base + ['--emit=asm', source], args.timeout)
                if text.returncode == 124:
                    results[key] = 'baseline-timeout'
                    (output / (tag + '.stderr')).write_bytes(text.stderr)
                    print('baseline-timeout', key, flush=True)
                    continue
                if text.returncode == 0:
                    assembly.write_bytes(text.stdout)
                native = run(base + ['--native-object', '-c', source, '-o', direct], args.timeout)
                if native.returncode == 124:
                    status = 'direct-timeout'
                elif text.returncode:
                    status = 'both-refuse' if native.returncode else 'text-only-refuse'
                else:
                    reference_result = run([args.clang, f'--target={target}', '-c', assembly, '-o', reference], args.timeout)
                    if reference_result.returncode == 124:
                        status = 'reference-timeout'
                        text = reference_result
                    elif reference_result.returncode:
                        status = 'both-refuse' if native.returncode else 'text-only-refuse'
                        text = reference_result
                    elif native.returncode:
                        status = 'direct-refuse'
                    else:
                        difference = compare(reference, direct)
                        status = 'mismatch' if difference else 'equivalent'
                        if difference:
                            (output / (tag + '.difference')).write_text('\n'.join(difference))
                results[key] = status
                if status not in ('equivalent', 'both-refuse'):
                    (output / (tag + '.stderr')).write_bytes(text.stderr + b'\nDIRECT:\n' + native.stderr)
                print(status, key, flush=True)
    (output / 'results.json').write_text(json.dumps(results, indent=2))
    counts = Counter(results.values())
    print(json.dumps(counts, sort_keys=True), flush=True)
    return int(any(k not in ('equivalent', 'both-refuse') for k in counts))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    objects = sub.add_parser('objects')
    objects.add_argument('reference')
    objects.add_argument('direct')
    sweep = sub.add_parser('corpus')
    sweep.add_argument('--bcc', required=True)
    sweep.add_argument('--clang', required=True)
    sweep.add_argument('--output', required=True)
    sweep.add_argument('--timeout', type=float, default=120, help='Seconds per compiler/assembler invocation; timeouts fail validation.')
    sweep.add_argument('--extra-corpus', action='append', default=[])
    sweep.add_argument('--only-extra', action='store_true', help='Run only additional corpus directories.')
    args = parser.parse_args()
    if args.command == 'corpus':
        return corpus(args)
    difference = compare(args.reference, args.direct)
    if difference:
        print('\n'.join(difference))
        return 1
    print('equivalent')
    return 0


if __name__ == '__main__':
    sys.exit(main())
