"""Random mode orchestration; every trial reuses the canonical matrix profile."""
from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys

from .. import runtime
from ..quirk_parity.fuzz import failure_identity, shrink_player_scenario
from .protocol_matrix import run as run_protocol_matrix


def replay(ctx, row, directory):
    directory.mkdir(parents=True, exist_ok=True)
    script = directory / 'script.json'
    script.write_text(json.dumps(row, indent=2) + '\n')
    # Keep fixture and product selectors, remove generation/reduction selectors.
    args = []
    iterator = iter(ctx.extra_args)
    for value in iterator:
        if value in ('--seed', '--steps', '--bots', '--script', '--fault'):
            next(iterator)
        elif value not in ('--shrink',):
            args.append(value)
    argv = [sys.executable, '-m', 'tools.harness', 'run', 'matrix-random',
            '--server-bin', str(ctx.server_bin), '--script', str(script), *args]
    if ctx.plugins_dir is not None:
        argv += ['--plugins-dir', str(ctx.plugins_dir)]
    env = dict(os.environ, SOLARIS_VALIDATION_ROOT=str(directory / 'receipts'))
    log = directory / 'replay.log'
    ctx.commands.append(argv)
    ctx.cwds.append(str(runtime.REPO_ROOT))
    ctx.logs.append(str(log))
    ctx.exits.append(None)
    with log.open('w') as output:
        result = subprocess.run(argv, cwd=runtime.REPO_ROOT, env=env, stdout=output,
                                stderr=subprocess.STDOUT, timeout=ctx.timeout_seconds or 600)
    ctx.exits[-1] = result.returncode
    receipts = list((directory / 'receipts').glob('*/result.json'))
    if len(receipts) != 1:
        raise RuntimeError('reduction trial did not produce exactly one canonical receipt')
    receipt = json.loads(receipts[0].read_text())
    report = receipt.get('details', {}).get('protocol_matrix', {})
    return report, str(receipts[0]), argv


def shrink(ctx, report):
    fingerprint = failure_identity(report)
    if fingerprint is None:
        return {'status': 'not_reducible', 'reason': 'no stable oracle failure identity'}
    root = ctx.artifact_dir / 'shrink'
    trials = []
    def reproduces(row):
        trial, receipt, argv = replay(ctx, row, root / f'trial-{len(trials):04d}')
        identity = failure_identity(trial)
        trials.append({'receipt': receipt, 'identity': identity,
                       'preserved_failure': identity == fingerprint, 'command': argv})
        return identity == fingerprint
    original = copy.deepcopy(report['row'])
    if not reproduces(original):
        result = {'status': 'not_reproduced', 'failure_identity': fingerprint, 'trials': trials}
    else:
        minimal = shrink_player_scenario(original, reproduces)
        path = root / 'minimal.json'
        path.write_text(json.dumps(minimal, indent=2) + '\n')
        replays = []
        for index in range(3):
            trial, receipt, argv = replay(ctx, minimal, root / f'confirm-{index}')
            replays.append({'receipt': receipt, 'identity': failure_identity(trial), 'command': argv})
        stable = all(item['identity'] == fingerprint for item in replays)
        command = ['python3', '-m', 'tools.harness', 'run', 'matrix-random',
                   '--server-bin', str(ctx.server_bin), '--script', str(path)]
        iterator = iter(ctx.extra_args)
        for value in iterator:
            if value == '--vanilla-data':
                command += [value, next(iterator)]
        result = {'status': 'reproduced' if stable else 'unstable', 'failure_identity': fingerprint,
                  'original_actions': len(original['actions']), 'minimal_actions': len(minimal['actions']),
                  'minimal_script': str(path), 'seed': minimal['seed'],
                  'repro_command': shlex.join(command), 'trials': trials, 'confirmations': replays}
        (root / 'ledger-draft.toml').write_text(
            '# Draft only: product classification and ledger promotion belong to Main.\n'
            'state = "open"\n' + 'seed = ' + str(minimal['seed']) + '\n' +
            'failure_identity = ' + json.dumps(fingerprint) + '\n' +
            'repro = ' + json.dumps(shlex.join(command)) + '\n')
    (root / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def run(ctx):
    try:
        result = run_protocol_matrix(ctx, random_mode=True)
        if '--differential' in ctx.extra_args:
            from ..quirk_parity.player import run_player_script, compare_player_states
            def option(name):
                return Path(ctx.extra_args[ctx.extra_args.index(name) + 1])
            report = result['protocol_matrix']
            vanilla = run_player_script(ctx, report['row'], option('--vanilla-jar'), option('--vanilla-data'))
            comparison = compare_player_states(report['adapter_state']['final'], vanilla,
                                                fault='--differential-fault' in ctx.extra_args)
            ctx.client_evidence['differential'] = comparison
            (ctx.artifact_dir/'differential.json').write_text(json.dumps(comparison, indent=2)+'\n')
            if not comparison['passed']:
                report.update(status='fail', note='random:differential:step=0: normalized final states differ')
                raise AssertionError(report['note'])
        return result
    except Exception:
        report = ctx.client_evidence.get('protocol_matrix', {})
        if '--shrink' in ctx.extra_args and report.get('status') == 'fail':
            ctx.client_evidence['shrink'] = shrink(ctx, report)
        raise
