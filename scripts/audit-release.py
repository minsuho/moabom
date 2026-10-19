"""Inspect PE metadata and known source disclosures without external analysis tools."""
import argparse
import hashlib
import json
import re
import struct
from pathlib import Path


def inspect(file):
    data = file.read_bytes()
    pe = struct.unpack_from('<I', data, 0x3C)[0]
    if data[pe:pe + 4] != b'PE\0\0':
        raise ValueError('Not a PE executable')
    _, count, _, symbols, symbol_count, optional_size, characteristics = struct.unpack_from('<HHIIIHH', data, pe + 4)
    opt = pe + 24
    if struct.unpack_from('<H', data, opt)[0] != 0x20B:
        raise ValueError('Expected x64 PE32+')
    dll = struct.unpack_from('<H', data, opt + 70)[0]
    directories = [struct.unpack_from('<II', data, opt + 112 + i * 8) for i in range(16)]
    sections = []
    for i in range(count):
        offset = opt + optional_size + i * 40
        name, virtual_size, rva, raw_size, raw_offset = struct.unpack_from('<8sIIII', data, offset)
        sections.append((name.rstrip(b'\0').decode(), rva, max(virtual_size, raw_size), raw_offset))

    def offset_of(rva):
        for _, start, length, raw in sections:
            if start <= rva < start + length:
                return raw + rva - start
        raise ValueError(f'Unknown RVA {rva}')

    debug_types = []
    if directories[6][1]:
        start = offset_of(directories[6][0])
        for i in range(directories[6][1] // 28):
            debug_types.append(struct.unpack_from('<I', data, start + i * 28 + 12)[0])
    exports = 0
    if directories[0][1]:
        exports = struct.unpack_from('<I', data, offset_of(directories[0][0]) + 20)[0]
    strings = re.findall(rb'[\x20-\x7e]{6,}', data)
    own_paths = [s.decode(errors='replace')[:200] for s in strings
                 if re.search(rb'(?:src[/\\](?:main|tiles|live|platforms|store|search|notify|update)\.rs|C:[/\\](?:dev|Users)[/\\])', s)]
    source_names = ['pickHeight', 'currentVideo', 'takeReloadSlot', 'playerRoot', 'renderGroups', 'groupSummary']
    return dict(file=str(file), size=len(data), sha256=hashlib.sha256(data).hexdigest(),
                coff_symbols=symbol_count, coff_symbol_table=symbols, exports=exports,
                debug_types=debug_types, codeview=2 in debug_types,
                debug_sections=[s[0] for s in sections if s[0].startswith(('.debug', '.zdebug'))],
                aslr=bool(dll & 0x40), high_entropy_aslr=bool(dll & 0x20), dep=bool(dll & 0x100),
                own_source_paths=own_paths,
                readable_js_names=[name for name in source_names if name.encode() in data])


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', type=Path)
    parser.add_argument('--enforce', action='store_true')
    args = parser.parse_args()
    result = inspect(args.exe)
    print(json.dumps(result, indent=2, ensure_ascii=False))
    if args.enforce:
        checks = {
            'symbols/exports': result['coff_symbols'] == 0 and result['exports'] == 0,
            'debug metadata': not result['codeview'] and not result['debug_sections'],
            'source disclosure': not result['own_source_paths'] and not result['readable_js_names'],
            'memory protection': result['aslr'] and result['high_entropy_aslr'] and result['dep'],
        }
        failures = [name for name, passed in checks.items() if not passed]
        if failures:
            raise SystemExit('Release audit failed: ' + ', '.join(failures))
