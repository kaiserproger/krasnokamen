"""Executable bug-ledger checks using the canonical harness and nextest."""
from __future__ import annotations

from collections import defaultdict
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import tomllib

from .runtime import REPO_ROOT

REQUIRED = {'id', 'title', 'area', 'repro', 'severity', 'status', 'source', 'regression'}
OPTIONAL = {'repro_check', 'failing_first'}


def _nextest_listing(text):
    decoder = json.JSONDecoder()
    for match in re.finditer(r'\{', text):
        try:
            value, _ = decoder.raw_decode(text[match.start():])
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and 'rust-suites' in value:
            return value
    raise ValueError('nextest did not produce a test listing')


def _client_command(profile, scenario):
    from . import scenarios

    entry = scenarios.SCENARIOS.get(scenario)
    if entry is None or entry.profile != profile or entry.evidence not in ('graphical', 'tcp', 'native') or profile in ('loader-live', 'dh-gate'):
        raise ValueError(f'unknown runnable scenario {profile}:{scenario}')
    argv = [sys.executable, '-m', 'tools.harness', 'run', 'scenario', scenario]
    if scenario.startswith('native-'):
        argv.extend(['--client', 'native', '--windowed'])
    else:
        argv.extend(['--client', 'java'])
    return argv

def _failed_ids(text, cargo):
    failed = set()
    for binary, name in re.findall(r'^\s*FAIL\s+\[[^\]]+\]\s+\(\s*\d+/\d+\)\s+(\S+)\s+(\S+)', text, re.MULTILINE):
        failed.update(cargo.get(binary.split('::', 1)[0], {}).get(name, ()))
    return failed



def _release_server(ctx, profiles):
    profiles._run_command(['cargo', 'build', '--release', '-p', 'mc-edge', '--bin', 'solaris-v2'], ctx, 'regressions-client-server-build.log')
    target = Path(os.environ.get('CARGO_TARGET_DIR', str(REPO_ROOT / 'target')))
    if not target.is_absolute():
        target = REPO_ROOT / target
    binary = target / 'release' / 'solaris-v2'
    digest = hashlib.sha256()
    with binary.open('rb') as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b''):
            digest.update(chunk)
    ctx.client_evidence['regressions_server'] = {'path': str(binary), 'sha256': digest.hexdigest()}
    return binary


def run(ctx):
    from . import profiles, scenarios

    with (REPO_ROOT / 'docs/bugs/ledger.toml').open('rb') as file:
        ledger = tomllib.load(file)
    if set(ledger) != {'bug'} or not isinstance(ledger['bug'], list) or not ledger['bug']:
        raise ValueError('ledger must contain a nonempty [[bug]] array and no other tables')
    bugs = ledger['bug']
    ids = set()
    cargo = defaultdict(lambda: defaultdict(set))
    clients = defaultdict(set)
    errors = []
    for bug in bugs:
        if not isinstance(bug, dict):
            errors.append('bug entry must be a table')
            continue
        ident = bug.get('id')
        if not isinstance(ident, str) or not ident or ident in ids:
            errors.append(f'invalid or duplicate bug id: {ident!r}')
            continue
        ids.add(ident)
        if REQUIRED - bug.keys() or bug.keys() - REQUIRED - OPTIONAL:
            errors.append(f'{ident}: missing or unknown fields')
        if any(not isinstance(bug.get(k), str) or not bug[k] for k in ('title', 'source')):
            errors.append(f'{ident}: missing title or source')
        if bug.get('area') not in ('gameplay', 'client', 'harness'):
            errors.append(f'{ident}: invalid area')
        if bug.get('severity') not in ('blocker', 'major', 'minor') or bug.get('status') not in ('open', 'fixed'):
            errors.append(f'{ident}: invalid severity or status')
        if bug.get('repro') not in ('manual', 'client', 'cargo', 'source'):
            errors.append(f'{ident}: invalid repro')
        if 'repro_check' in bug:
            repro_checks = bug['repro_check'] if isinstance(bug['repro_check'], list) else [bug['repro_check']]
            if not repro_checks or any(not isinstance(ref, str) or not ref for ref in repro_checks):
                errors.append(f'{ident}: invalid repro_check')
        if 'failing_first' in bug and bug['failing_first'] not in ('not recorded', 'reported'):
            errors.append(f'{ident}: invalid failing_first')
        refs = bug.get('regression')
        if not isinstance(refs, list) or any(not isinstance(ref, str) for ref in refs):
            errors.append(f'{ident}: regression must be a list of check refs')
            continue
        if bug.get('status') == 'fixed' and not refs:
            errors.append(f'{ident}: fixed without regression')
        for ref in refs:
            parts = ref.split(':', 2)
            if len(parts) == 3 and parts[0] == 'cargo' and re.fullmatch(r'[A-Za-z0-9_-]+', parts[1]) and re.fullmatch(r'[A-Za-z0-9_:]+', parts[2]):
                cargo[parts[1]][parts[2]].add(ident)
            elif len(parts) == 3 and parts[0] == 'harness':
                try:
                    _client_command(parts[1], parts[2])
                except ValueError as error:
                    errors.append(f'{ident}: {error}')
                else:
                    clients[ref].add(ident)
            else:
                errors.append(f'{ident}: invalid check {ref!r}')
        if bug.get('status') == 'fixed' and (bug.get('area') == 'client' or bug.get('repro') in ('manual', 'client')):
            required_evidence = ('graphical',)
            if not any(len(parts := ref.split(':', 2)) == 3 and parts[0] == 'harness' and parts[2] in scenarios.SCENARIOS and scenarios.SCENARIOS[parts[2]].profile == parts[1] and scenarios.SCENARIOS[parts[2]].evidence in required_evidence for ref in refs):
                errors.append(f'{ident}: original or client-surface repro needs a real-client check')
    fixed = sum(isinstance(b, dict) and b.get('status') == 'fixed' for b in bugs)
    print(f'[regressions] open={len(bugs)-fixed} fixed={fixed}', flush=True)
    if errors:
        raise ValueError('; '.join(errors))
    failures = []
    if cargo:
        expression = ' | '.join(
            f'(package({crate}) & test(={name}))'
            for crate, tests in sorted(cargo.items()) for name in sorted(tests)
        )
        all_ids = set().union(*(ids for tests in cargo.values() for ids in tests.values()))
        try:
            profiles._run_command(['cargo', 'nextest', 'list', '--workspace', '-E', expression, '--message-format', 'json'], ctx, 'regressions-list.log')
            listing = _nextest_listing((ctx.artifact_dir / 'regressions-list.log').read_text())
            found = {(suite['package-name'], name) for suite in listing['rust-suites'].values() for name, result in suite.get('testcases', {}).items() if result.get('filter-match', {}).get('status') == 'matches' and not result.get('ignored', False)}
            missing = {(crate, name) for crate, tests in cargo.items() for name in tests} - found
            for crate, name in sorted(missing):
                failures.extend(f'{ident}: unknown or ignored test {crate}:{name}' for ident in sorted(cargo[crate][name]))
            if not missing:
                profiles._run_command(['cargo', 'nextest', 'run', '--workspace', '-E', expression, '--ignore-default-filter', '--no-fail-fast', '--no-tests=fail'], ctx, 'regressions-run.log')
        except Exception as error:
            log = ctx.artifact_dir / 'regressions-run.log'
            failed = _failed_ids(log.read_text(), cargo) if log.exists() else set()
            failures.append(f"{', '.join(sorted(failed or all_ids))}: {error}")
    if ctx.include_regression_client and clients:
        try:
            binary = _release_server(ctx, profiles)
        except Exception as error:
            failures.append(f"{', '.join(sorted(set().union(*clients.values())))}: release server: {error}")
        else:
            env = os.environ.copy()
            # A mutation control must never replace the binary in a ledger gate.
            env.pop('SOLARIS_FIRST_START_RECOVERY_BINARY', None)
            env['SOLARIS_SERVER_BINARY'] = str(binary)
            for ref, bug_ids in sorted(clients.items()):
                _, profile, scenario = ref.split(':', 2)
                try:
                    log_name = f'regressions-client-{len(ctx.logs)}.log'
                    profiles._run_command(_client_command(profile, scenario), ctx, log_name, env=env)
                    paths = re.findall(r'artifact=(\S+)', (ctx.artifact_dir / log_name).read_text())
                    if not paths:
                        raise RuntimeError('client run produced no receipt')
                    receipt = json.loads((Path(paths[-1]) / 'result.json').read_text())
                    if receipt.get('status') != 'passed':
                        raise RuntimeError(f"client receipt is {receipt.get('status')!r}, not passed")
                    server_hash = ctx.client_evidence['regressions_server']['sha256']
                    if scenarios.SCENARIOS[scenario].evidence != 'native' and server_hash not in json.dumps(receipt.get('details', {})):
                        raise RuntimeError('client receipt does not identify the built release server')
                except Exception as error:
                    failures.append(f"{', '.join(sorted(bug_ids))}: {error}")
    if failures:
        raise RuntimeError('; '.join(failures))
    return {'open': len(bugs)-fixed, 'fixed': fixed, 'cargo_checks': sum(map(len, cargo.values())), 'client_checks': len(clients), 'client_executed': ctx.include_regression_client}
