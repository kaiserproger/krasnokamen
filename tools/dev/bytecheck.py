#!/usr/bin/env python3
"""D65 byte oracle. No source mutation; controls are isolated caller-owned trees.

A/B start from copies of one prepared native store, including its UUID. C/D
always read fresh copies of the first base checkpoint+tail. JSON object
formatting is not an artifact: native row payloads, NBT compounds, WAL frames
and exported file payloads are bytes.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess

RUNNER = Path('/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/mem-run.sh')
ARTIFACTS = ('A', 'B', 'C-live', 'C-offline', 'D-custody', 'D-crash', 'D-export')
BINARIES = {}
BUILDS = {}
BUILD_MEMORY = '6G'


def provenance(tree):
    git = lambda *args: subprocess.check_output(['git', '-C', str(tree), *args])
    untracked = git('ls-files', '--others', '--exclude-standard', '-z').split(b'\0')
    return dict(tree=str(tree), head=git('rev-parse', 'HEAD').decode().strip(),
                diff_sha256=digest(git('diff', '--binary', 'HEAD')),
                untracked={os.fsdecode(path): digest((tree / os.fsdecode(path)).read_bytes())
                           for path in untracked if path and (tree / os.fsdecode(path)).is_file()})


def vanilla_provenance(tree):
    root = tree / 'data/vanilla'
    if not root.is_dir():
        raise SystemExit(f'vanilla input directory missing: {root}')
    digest_tree = hashlib.sha256()
    count = size = 0
    for path in sorted(root.rglob('*')):
        if not path.is_file():
            continue
        with path.open('rb') as source:
            file_hash = hashlib.file_digest(source, 'sha256').hexdigest()
        digest_tree.update(f'{path.relative_to(root)}\0{file_hash}\n'.encode())
        count += 1
        size += path.stat().st_size
    return dict(sha256=digest_tree.hexdigest(), files=count, bytes=size)


def seed_snapshot(world):
    files = {str(path.relative_to(world)): dict(bytes=path.stat().st_size,
                                              sha256=digest(path.read_bytes()))
             for path in sorted(world.rglob('*')) if path.is_file()}
    if 'native/state.redb' not in files:
        raise SystemExit(f'missing prepared native store: {world}')
    return files


def test(tree, out, crate, name):
    env = dict(os.environ, CORE_UNLOAD_OUT=str(out))
    # Never use pooled output paths: their lease ends before cargo returns to
    # this caller. A private per-tree target cannot be rebuilt by another lane.
    target = Path('/run/media/kaiserroman/games/solaris-work/targets') / ('CoreUnloadImpl-bytecheck-' + digest(str(tree).encode())[:16])
    env['CARGO_TARGET_DIR'] = str(target)
    built = tree not in BINARIES
    if built:
        source = provenance(tree)
        vanilla = vanilla_provenance(tree)
        cmd = [str(RUNNER), '-m', BUILD_MEMORY, 'cargo', 'test', '-j', '1', '--manifest-path', str(tree / 'Cargo.toml'),
               '-p', 'mc-core', '-p', 'mc-world-conversion', '--lib', '--no-run', '--message-format=json']
        with (out / 'compile.log').open('wb') as log:
            subprocess.run(cmd, cwd=tree, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        if provenance(tree) != source or vanilla_provenance(tree) != vanilla:
            raise SystemExit('source or vanilla inputs changed during compile')
        binaries = {}
        for line in (out / 'compile.log').read_text().splitlines():
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            if message.get('reason') == 'compiler-artifact' and message.get('executable'):
                binaries[message['target']['name']] = message['executable']
        if not {'mc_core', 'mc_world_conversion'} <= binaries.keys():
            raise SystemExit(f'missing test executables: {binaries}')
        BINARIES[tree] = binaries
        # Shared build slots can be reused by another agent between tests.
        # Keep exactly the executables emitted by this compile.
        pins = out / 'binaries'
        pins.mkdir()
        build = dict(source=source, data_vanilla=vanilla, target_dir=str(target), binaries={})
        for target_name, binary in BINARIES[tree].items():
            pinned = pins / target_name
            shutil.copy2(binary, pinned)
            BINARIES[tree][target_name] = str(pinned)
            build['binaries'][target_name] = dict(path=str(pinned), sha256=digest(pinned.read_bytes()))
        BUILDS[tree] = build
    (out / 'build.json').write_text(json.dumps(BUILDS[tree], indent=2) + '\n')
    if built:
        # Every later producer uses immutable pins, not Cargo output paths.
        # Release this scratch target before another tree starts compiling.
        shutil.rmtree(target)
    binary = BINARIES[tree][crate.replace('-', '_')]
    cmd = [str(RUNNER), '-m', '6G', binary, name, '--nocapture']
    if name != 'persistence_byte_capture':
        cmd.append('--ignored')
    with (out / (name + '.log')).open('wb') as log:
        subprocess.run(cmd, cwd=tree, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    with (out / (name + '.log')).open('ab') as log:
        log.write(('\nbinary_sha256=' + digest(Path(binary).read_bytes()) + '\n').encode())
    if 'test result: ok. 1 passed;' not in (out / (name + '.log')).read_text():
        raise SystemExit(f'expected exactly one passing producer: {name}')


def capture(tree, out, seed):
    out.mkdir(parents=True)
    shutil.copytree(seed, out / 'world')
    inputs = seed_snapshot(seed)
    if seed_snapshot(out / 'world') != inputs:
        raise SystemExit('capture input differs from the prepared native store')
    (out / 'input.json').write_text(json.dumps(inputs, sort_keys=True, indent=2) + '\n')
    test(tree, out, 'mc-core', 'persistence_byte_capture')


def consumers(tree, out, source):
    shutil.copy2(source.parent / 'stable-transit.json', out / 'reopen-transit.json')
    for name in ('live', 'offline', 'custody', 'crash', 'export'):
        shutil.copytree(source, out / (name + '-input'))
    test(tree, out, 'mc-core', 'persistence_byte_reopen')
    test(tree, out, 'mc-world-conversion', 'persistence_byte_consumers')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def rows(path):
    result = {}
    for key, row in json.loads(path.read_text()).items():
        payload = bytes(row.pop('payload'))
        result[key] = dict(row, sha256=digest(payload), payload=base64.b64encode(payload).decode())
    return result




def artifacts(out):
    # Old unpinned runs are not admissible baselines.
    build = json.loads((out / 'build.json').read_text())
    for binary in build['binaries'].values():
        if digest(Path(binary['path']).read_bytes()) != binary['sha256']:
            raise SystemExit('binary pin changed')
    result = {name: rows(out / (name + '.json')) for name in ('A', 'C-live', 'C-offline')}
    stable = json.loads((out / 'stable-transit.json').read_text())
    result['_stable_transit'] = 'global/transit/' + stable
    result['B'] = {p.name: base64.b64encode(p.read_bytes()).decode() for p in sorted((out / 'B').iterdir())}
    if not result['B']:
        raise SystemExit('missing WAL artifacts')
    for name in ('D-custody', 'D-crash'):
        result[name] = json.loads((out / (name + '.json')).read_text())
    exported = {}
    for path in sorted((out / 'D-export').rglob('*')):
        if not path.is_file():
            continue
        key = str(path.relative_to(out / 'D-export'))
        exported[key] = base64.b64encode(path.read_bytes()).decode()
    if not exported:
        raise SystemExit('missing export artifacts')
    result['D-export'] = exported
    (out / 'artifacts.json').write_text(json.dumps(result, sort_keys=True, indent=2) + '\n')
    return result


def wal_lines(artifact):
    result = {'Settlement': [], 'Transit': [], 'Entity': [], 'other': []}
    def images(value):
        if list(value) == ['Dimension']:
            for entry in value['Dimension']['entries']:
                yield from images(entry)
        else:
            yield value
    for name, encoded in artifact.items():
        raw = base64.b64decode(encoded)
        # Classification accepts only the current store/branch/palette-bound frame
        # (format 3, 64-byte header). The verdict above still compares every
        # header, CRC and payload byte.
        if len(raw) < 64:
            raise SystemExit(f'incomplete WAL v3 frame header: {name}')
        magic, version = struct.unpack_from('<II', raw)
        if magic != 0x534F4C4A or version != 3:
            raise SystemExit(f'unsupported WAL frame header: {name}')
        payload = raw[64:]
        if len(payload) != struct.unpack_from('<I', raw, 56)[0]:
            raise SystemExit(f'WAL frame length mismatch: {name}')
        for line in payload.splitlines():
            for image in images(json.loads(line)):
                variant = next(iter(image))
                category = variant if variant in result else 'other'
                # Dict insertion order preserves every nested field's order;
                # JSON strings (the Transit payload) retain their exact bytes.
                encoded_image = json.dumps(image, separators=(',', ':'), ensure_ascii=False)
                result[category].append((name, encoded_image))
    return result


def compare(base, candidate, control):
    verdict = {name: {'equal': base[name] == candidate[name]} for name in ARTIFACTS}
    stable = base['_stable_transit']
    if candidate['_stable_transit'] != stable:
        raise SystemExit('stable transit identity differs')
    for image in (base, candidate):
        if not all(stable in image[stage] for stage in ('A', 'C-live', 'C-offline')):
            raise SystemExit('stable transit row missing')
        if not control:
            for stage in ('C-live', 'C-offline'):
                if image[stage][stable]['payload'] != base['A'][stable]['payload']:
                    raise SystemExit(f'{stage}: active transit bytes changed on reopen')
    for name in ('A', 'C-live', 'C-offline'):
        keys = set(base[name]) | set(candidate[name])
        verdict[name]['different_rows'] = sorted(k for k in keys if base[name].get(k) != candidate[name].get(k))
    if control:
        changed = {name for name in ARTIFACTS if not verdict[name]['equal']}
        left, right = wal_lines(base['B']), wal_lines(candidate['B'])
        wal_changed = {k for k in left if left[k] != right[k]}
        # Reopen always reads BASE. X2 also changes checkpoint transit JSON:
        # transit::Record.members embeds ReplayEntity.villager (Main ruling).
        expected = {'X1': {'A', 'B', 'C-live', 'C-offline'},
                    'X2': {'A', 'B', 'C-live'}, 'X3': {'A', 'C-live', 'C-offline', 'D-export'}}[control]
        if changed != expected:
            raise SystemExit(f'{control}: changed artifacts {changed}, expected {expected}')
        for name in ('A', 'C-live', 'C-offline'):
            if control == 'X1':
                expected_rows = {
                    'A': {'global/settlement/1', 'global/settlement/2', 'global/settlement/3'},
                    'C-live': {'global/settlement/1', 'global/settlement/3'},
                    'C-offline': {'global/settlement/1'},
                }[name]
            elif control == 'X2':
                expected_rows = ({k for k in base[name] if k.startswith('global/transit/')} if name == 'A'
                                 else {stable} if name == 'C-live' else set())
            else:
                expected_rows = {k for k in base[name] if k.startswith('entity/')}
            if name == 'A':
                if not expected_rows:
                    raise SystemExit(f'{control}: vacuous row expectation')
            if set(verdict[name]['different_rows']) != expected_rows:
                raise SystemExit(f'{control}/{name}: {verdict[name]}, expected rows {expected_rows}')
        expected_wal = {'X1': {'Settlement'}, 'X2': {'Entity', 'Transit'}, 'X3': set()}[control]
        if wal_changed != expected_wal:
            raise SystemExit(f'{control}: WAL variants {wal_changed}, expected {expected_wal}')
        verdict['control'] = {'id': control, 'expected_diff_only': True, 'result': 'RED',
                              'wal_variants': sorted(wal_changed)}
    return verdict


def main():
    global BUILD_MEMORY
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('base_tree', type=Path)
    parser.add_argument('slice_tree', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--base-run', type=Path, help='reuse an immutable, complete base run')
    parser.add_argument('--control', choices=('X1', 'X2', 'X3'))
    parser.add_argument('--build-memory', choices=('6G', '12G'), default='6G',
                        help='12G only for a wide compile shown to exceed 6G; tests always use 6G')
    args = parser.parse_args()
    BUILD_MEMORY = args.build_memory
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    if args.base_run:
        base_out = args.base_run.resolve()
        recorded = json.loads((base_out / 'build.json').read_text())['source']
        if recorded != provenance(args.base_tree.resolve()):
            raise SystemExit('cached base tree/fingerprint does not match base_tree')
        seed = base_out / 'seed-world'
        if seed_snapshot(seed) != json.loads((base_out / 'seed.json').read_text()):
            raise SystemExit('cached native seed changed')
        base = artifacts(base_out)
    else:
        seed_out = args.out / 'seed'
        seed_out.mkdir()
        test(args.slice_tree.resolve(), seed_out, 'mc-core', 'persistence_byte_seed')
        base_out = args.out / 'base'
        capture(args.base_tree.resolve(), base_out, seed_out / 'world')
        seed = base_out / 'seed-world'
        shutil.copytree(seed_out / 'world', seed)
        (base_out / 'seed.json').write_text(
            json.dumps(seed_snapshot(seed), sort_keys=True, indent=2) + '\n')
        consumers(args.base_tree.resolve(), base_out, base_out / 'world')
        base = artifacts(base_out)
    candidate_out = args.out / 'slice'
    capture(args.slice_tree.resolve(), candidate_out, seed)
    consumers(args.slice_tree.resolve(), candidate_out, base_out / 'world')
    candidate = artifacts(candidate_out)
    verdict = compare(base, candidate, args.control)
    equal = all(verdict[name]['equal'] for name in ARTIFACTS)
    result = dict(status='PASS' if equal else 'DIFF', artifacts=verdict,
                  base_tree=str(args.base_tree.resolve()), slice_tree=str(args.slice_tree.resolve()),
                  base_run=str(base_out), slice_run=str(candidate_out),
                  base_data_vanilla_observed=vanilla_provenance(args.base_tree.resolve()),
                  native_seed=str(seed), native_seed_files=seed_snapshot(seed),
                  slice_data_vanilla=BUILDS[args.slice_tree.resolve()]['data_vanilla'])
    (args.out / 'verdict.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
    return 0 if equal else 1


if __name__ == '__main__':
    raise SystemExit(main())
