"""Bounded End restart regression; deliberately not a concurrent matrix-row oracle."""
from __future__ import annotations

import re

from ..matrix_api import MissingCapability


def _dragon(client):
    snapshot = client.snapshot()
    if 'entities' not in snapshot:
        raise MissingCapability('observation.entities:' + client.name)
    dragons = [entity for entity in snapshot['entities']
               if entity['type'] == 'minecraft:ender_dragon']
    if len(dragons) > 1:
        raise AssertionError(f'{client.name} received multiple End dragons: {dragons}')
    return dragons[0] if dragons else None


def run(server, clients, report):
    if len(clients) != 8:
        raise AssertionError('peaceful End restart requires eight real joined clients')
    report['scope'] = 'causal End/Peaceful/eight-client save-restart only; no full-row or matrix credit'
    report['clients'] = [client.name for client in clients]
    server.dimension = 'the_end'
    server.command('difficulty peaceful')
    for client in clients:
        server.command(f'gamemode creative {client.name}')
        server.command(f'execute in minecraft:the_end run tp {client.name} 0.5 128 0.5')
    before = []
    for client in clients:
        before.append(client.wait_until(lambda client=client: _dragon(client),
                      description=f'{client.name} receives the persistent End dragon',
                      timeout_seconds=60))
    report['before'] = before
    identities = {entity['uuid'] for entity in before}
    if len(identities) != 1:
        raise AssertionError(f'clients disagree on End dragon identity: {identities}')
    server.save_all()
    for client in clients:
        client.disconnect()
    server.clean_restart()
    for client in clients:
        client.reconnect()
    after = []
    # Sequential joins let the live dragon fly beyond the original observers'
    # tracking range. Move observers, never the persisted dragon or its state.
    report['restart_observer_fixture'] = 'teleport each observer beside the live End dragon'
    for client in clients:
        server.command('execute in minecraft:the_end at '
                       '@e[type=minecraft:ender_dragon,limit=1] '
                       f'run tp {client.name} ~ ~ ~')
        after.append(client.wait_until(lambda client=client: _dragon(client),
                     description=f'{client.name} receives the recovered End dragon',
                     timeout_seconds=60))
    report['after'] = after
    restored = {entity['uuid'] for entity in after}
    if restored != identities:
        raise AssertionError(f'End dragon identity changed across native recovery: {identities} -> {restored}')
    log = server.log_path.read_text()
    errors = [line for line in log.splitlines() if re.search(r'\bERROR\b|panicked at', line)]
    report['shutdown_errors'] = errors
    if errors:
        raise AssertionError('save/restart emitted errors: ' + '\n'.join(errors))
    shutdown_lines = [line for line in log.splitlines()
                      if 'native checkpoint writer stopped' in line and 'shutdown requested' in line]
    if not shutdown_lines or any(' INFO ' not in line for line in shutdown_lines):
        raise AssertionError('missing explicit info-level native writer shutdown receipt')
    report['shutdown_info'] = shutdown_lines
    report['checks'] = [
        {'name': 'eight_clients_observe_same_dragon_before_restart', 'passed': True,
         'detail': {'clients': report['clients'], 'uuid': next(iter(identities))}},
        {'name': 'end_boots_with_same_retained_dragon_after_restart', 'passed': True,
         'detail': {'received_dragon_uuids': [entity['uuid'] for entity in after]}},
        {'name': 'orderly_save_restart_has_no_error_records', 'passed': True,
         'detail': {'errors': errors, 'native_shutdown': shutdown_lines}},
    ]
