"""Immutable native-copy probes used by the retained-state growth gate."""
import json
import shutil
import subprocess

from . import runtime


def dump(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def probe_world(binary, config, world, positions, receipt):
    command = [str(binary), '--config', str(config), 'world', 'probe', '--source', str(world), '--receipt', str(receipt)]
    for position in sorted(positions):
        command.extend(['--block', ','.join(map(str, position))])
    with receipt.with_suffix('.log').open('wb') as log:
        result = subprocess.run(command, cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT,
                                check=False, timeout=180)
    if result.returncode:
        raise RuntimeError(f'native checkpoint probe exited {result.returncode}: {receipt}')
    return json.loads(receipt.read_text())


def parent_custody(binary, config, out, ids):
    # The existing offline custody command repairs metadata only in its own
    # disposable copy. It enumerates every durable parent UUID, not merely the
    # villagers within the client's 128-block received-entity census.
    copied = out / 'input-custody-copy'
    shutil.copytree(out / 'world', copied)
    receipt = out / 'input-custody.json'
    command = [str(binary), '--config', str(config), 'world', 'custody-census',
               '--copy', str(copied), '--receipt', str(receipt)]
    with (out / 'input-custody.log').open('wb') as log:
        subprocess.run(command, cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT,
                       check=True, timeout=180)
    rows = json.loads(receipt.read_text())
    assert rows['status'] == 'PASS', rows
    return sorted(uuid for uuid, row in rows['villagers'].items() if row['settlement'] in ids)


