#!/usr/bin/env python3
"""Compare real checkpoint+tail consumers on immutable, separate world copies.

Usage: bytecheck_real.py BASE_TREE SLICE_TREE RETAINED_RUN --out RECEIPT_DIR
RETAINED_RUN contains world/, plugins/ and server.toml. Never opens that world.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import shutil
import subprocess
import tomllib
from bytecheck import RUNNER, digest, provenance, vanilla_provenance, wal_lines


def hashes(root):
    return {str(p.relative_to(root)): digest(p.read_bytes())
            for p in sorted(root.rglob('*')) if p.is_file()}


def input_hashes(run, world):
    result = {'world/' + key: value for key, value in hashes(world).items()}
    result.update({'plugins/' + key: value for key, value in hashes(run / 'plugins').items()})
    result['server.toml'] = digest((run / 'server.toml').read_bytes())
    return result


def build(tree, out):
    source = provenance(tree)
    vanilla = vanilla_provenance(tree)
    target = Path('/run/media/kaiserroman/games/solaris-work/targets') / ('CoreUnloadImpl-bytecheck-' + digest(str(tree).encode())[:16])
    env = dict(os.environ, CARGO_TARGET_DIR=str(target))
    cmd = [str(RUNNER), '-m', '6G', 'cargo', 'build', '-j', '1', '--manifest-path', str(tree / 'Cargo.toml'),
           '-p', 'mc-world-conversion', '-p', 'workspace-features', '--lib', '--message-format=json']
    with (out / 'compile.log').open('wb') as log:
        subprocess.run(cmd, cwd=tree, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    libraries = {}
    native_paths = set()
    for line in (out / 'compile.log').read_text().splitlines():
        if not line.startswith('{'):
            continue
        entry = json.loads(line)
        if entry['reason'] == 'build-script-executed':
            native_paths.update(entry['linked_paths'])
        # Cargo also reports host build-script libraries, whose serde traits
        # have a different crate identity. Link only the root's target profile.
        if entry['reason'] == 'compiler-artifact' and entry['profile']['debuginfo'] == 'line-tables-only':
            for path in entry['filenames']:
                if path.endswith('.rlib'):
                    libraries[entry['target']['name']] = path
    helper = Path(__file__).with_suffix('.rs')
    pin = out / 'probe'
    rustc = [str(RUNNER), '-m', '6G', 'rustc', '--edition=2024', '-C', 'debuginfo=line-tables-only',
             '-L', 'dependency=' + str(target / 'debug/deps'), str(helper), '-o', str(pin)]
    for name in ('mc_core', 'mc_world_conversion', 'mc_world', 'mc_nbt', 'serde_json'):
        rustc.extend(['--extern', name + '=' + libraries[name]])
    for path in sorted(native_paths):
        rustc.extend(['-L', path])
    with (out / 'link.log').open('wb') as log:
        subprocess.run(rustc, cwd=tree, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    if source != provenance(tree) or vanilla != vanilla_provenance(tree):
        raise SystemExit('source or vanilla inputs changed during real-probe build')
    (out / 'build.json').write_text(json.dumps(dict(source=source, data_vanilla=vanilla, cargo_command=cmd, rustc_command=rustc,
        helper_sha256=digest(helper.read_bytes()), binary=dict(path=str(pin), sha256=digest(pin.read_bytes()))), indent=2) + '\n')
    return pin


def refusal(log):
    errors = [line.removeprefix('Error: ') for line in log.read_text().splitlines() if line.startswith('Error: ')]
    if len(errors) != 1:
        raise SystemExit(f'expected one canonical census refusal in {log}: {errors}')
    return errors[0]


def read_artifacts(out, expected_refusal):
    names = ['fold', 'custody', 'crash', 'game-clocks']
    if expected_refusal is None:
        names.append('census')
    artifacts = {name: (out / (name + '.txt')).read_text() for name in names}
    if expected_refusal is not None:
        observed = refusal(out / 'census.log')
        if observed != expected_refusal:
            raise SystemExit(f'census refusal differs: {observed!r}, expected {expected_refusal!r}')
        artifacts['census-refusal'] = observed
    artifacts['export'] = hashes(out / 'export')
    if not artifacts['export']:
        raise SystemExit('missing real export artifacts')
    return artifacts


def consume(tree, out, snapshot, seed, pin, expected_refusal):
    shutil.copytree(snapshot / 'plugins', out / 'plugins')
    for mode in ('fold', 'export', 'custody', 'crash', 'census'):
        world = out / (mode + '-input')
        shutil.copytree(snapshot / 'world', world)
        for path in [world, *world.rglob('*')]:
            path.chmod(path.stat().st_mode | 0o200)
        cmd = [str(RUNNER), '-m', '6G', str(pin), mode, str(world), str(out), str(seed),
               str(tree / 'data/vanilla'), str(out / 'plugins')]
        with (out / (mode + '.log')).open('wb') as log:
            result = subprocess.run(cmd, cwd=tree, env=os.environ,
                                    stdout=log, stderr=subprocess.STDOUT)
        if mode == 'census' and expected_refusal is not None:
            observed = refusal(out / 'census.log')
            if result.returncode != 1 or observed != expected_refusal:
                raise SystemExit(f'expected census refusal {expected_refusal!r}, got exit={result.returncode}, {observed!r}')
            (out / 'census-refusal.json').write_text(json.dumps(dict(exit=result.returncode, error=observed)) + '\n')
        else:
            result.check_returncode()
    return read_artifacts(out, expected_refusal)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('base_tree', type=Path)
    parser.add_argument('slice_tree', type=Path)
    parser.add_argument('retained_run', type=Path)
    parser.add_argument('--world', type=Path, help='certified world directory when not RETAINED_RUN/world')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--base-run', type=Path, help='reuse its frozen input and pinned base outputs; compile/run only the slice')
    parser.add_argument('--census-refusal', help='require this exact canonical refusal on BOTH pins instead of a successful census')
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    original = args.retained_run.resolve()
    world = (args.world or original / 'world').resolve()
    before = input_hashes(original, world)
    (out / 'input-sha256.json').write_text(json.dumps(before, indent=2) + '\n')
    base_tree = args.base_tree.resolve()
    frozen_hashes = None
    snapshot = None
    try:
        if args.base_run:
            cached = args.base_run.resolve()
            base_out = cached / 'base'
            build_record = json.loads((base_out / 'build.json').read_text())
            if build_record['source'] != provenance(base_tree):
                raise SystemExit('cached real base tree/fingerprint does not match base_tree')
            if digest(Path(build_record['binary']['path']).read_bytes()) != build_record['binary']['sha256']:
                raise SystemExit('cached real base binary pin changed')
            if before != json.loads((cached / 'input-sha256.json').read_text()):
                raise SystemExit('retained real input differs from the cached base run')
            snapshot = cached / 'input'
            frozen_hashes = json.loads((cached / 'repair-after-sha256.json').read_text())
            if hashes(snapshot) != frozen_hashes:
                raise SystemExit('cached frozen real input changed')
            (out / 'reused-base.json').write_text(json.dumps(dict(run=str(cached), source=build_record['source'],
                data_vanilla_observed=vanilla_provenance(base_tree)), indent=2) + '\n')
        else:
            snapshot = out / 'input'
            snapshot.mkdir()
            shutil.copytree(world, snapshot / 'world')
            shutil.copytree(original / 'plugins', snapshot / 'plugins')
            shutil.copy2(original / 'server.toml', snapshot / 'server.toml')
            if before != hashes(snapshot):
                raise SystemExit('retained input copy changed bytes')
            base_out = out / 'base'
            base_out.mkdir()
            base_pin = build(base_tree, base_out)
            seed = tomllib.loads((snapshot / 'server.toml').read_text())['data']['seed']
            preflight = [str(RUNNER), '-m', '6G', str(base_pin), 'fold', str(snapshot / 'world'),
                         str(base_out), str(seed), str(base_tree / 'data/vanilla'), str(snapshot / 'plugins')]
            with (out / 'precondition.log').open('wb') as log:
                result = subprocess.run(preflight, cwd=base_tree, env=os.environ,
                                        stdout=log, stderr=subprocess.STDOUT)
            repaired = False
            if result.returncode:
                if 'world was not shut down cleanly; start and stop the server once to repair it' not in (out / 'precondition.log').read_text():
                    result.check_returncode()
                preflight[4] = 'repair'
                with (out / 'repair.log').open('wb') as log:
                    subprocess.run(preflight, cwd=base_tree, env=os.environ,
                                   stdout=log, stderr=subprocess.STDOUT, check=True)
                repaired = True
            (out / 'repair.json').write_text(json.dumps(dict(performed=repaired,
                description='SIGKILL image after the redb repair, journal untouched' if repaired
                            else 'SIGKILL image already readable, no repair; journal untouched'), indent=2) + '\n')
            frozen_hashes = hashes(snapshot)
            (out / 'repair-after-sha256.json').write_text(json.dumps(frozen_hashes, indent=2) + '\n')
            changed = {key for key in before.keys() | frozen_hashes.keys() if before.get(key) != frozen_hashes.get(key)}
            if not changed <= {'world/native/state.redb'}:
                raise SystemExit(f'repair changed non-database files: {changed}')
            for path in [*snapshot.rglob('*'), snapshot]:
                path.chmod(0o555 if path.is_dir() else 0o444)
            consume(base_tree, base_out, snapshot, seed, base_pin, args.census_refusal)
        seed = tomllib.loads((snapshot / 'server.toml').read_text())['data']['seed']
        classified = wal_lines({path.name: base64.b64encode(path.read_bytes()).decode()
                                for path in sorted((snapshot / 'world/journal').glob('batch-*.bin'))})
        (out / 'tail-records.json').write_text(json.dumps({kind: len(rows) for kind, rows in classified.items()}, indent=2) + '\n')
        (out / 'input.json').write_text(json.dumps(dict(source=str(world), snapshot=str(snapshot), seed=seed,
            transit_scope='See tail-records.json; synthetic S0 supplies transit WAL coverage'), indent=2) + '\n')
        base = read_artifacts(base_out, args.census_refusal)
        slice_tree = args.slice_tree.resolve()
        slice_out = out / 'slice'
        slice_out.mkdir()
        slice_pin = build(slice_tree, slice_out)
        candidate = consume(slice_tree, slice_out, snapshot, seed, slice_pin, args.census_refusal)
        verdict = {name: dict(equal=base[name] == candidate[name]) for name in base}
        (out / 'verdict.json').write_text(json.dumps(verdict, indent=2) + '\n')
        if not all(row['equal'] for row in verdict.values()):
            raise SystemExit(f'real-image comparison differs: {verdict}')
    finally:
        unchanged = before == input_hashes(original, world)
        frozen_unchanged = frozen_hashes is None or frozen_hashes == hashes(snapshot)
        (out / 'source-unchanged.json').write_text(json.dumps(dict(unchanged=unchanged, frozen_unchanged=frozen_unchanged)) + '\n')
        if not unchanged or not frozen_unchanged:
            raise SystemExit('original or frozen real input changed during proof')


if __name__ == '__main__':
    main()
