"""Same seeded MatrixContext and wire clients against the existing vanilla oracle."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re

from .vanilla import VanillaOracle
from ..matrix_api import MatrixContext
from ..matrix_execution import offline_uuid, run_subject
from ..protocol_matrix_client import ProtocolMatrixClient
from ..backends.protocol_lifecycle import ProtocolServer


class VanillaMatrixServer(VanillaOracle):
    ensure_loaded = ProtocolServer.ensure_loaded

    @property
    def running(self):
        return self.process is not None and self.process.poll() is None

    def query(self, text):
        return '\n'.join(self.run(text))

    command = query

    def dashboard_stats(self):
        feedback = self.query('time query gametime')
        match = re.search(r'The (?:game )?time is (\d+)', feedback)
        if match is None:
            raise RuntimeError('vanilla did not acknowledge the game-time fence: ' + feedback)
        return {'simulation_tick': int(match.group(1))}


def run_player_script(ctx, row, jar, vanilla_data):
    directory = ctx.artifact_dir / 'vanilla'
    clients = []
    server = VanillaMatrixServer(directory, seed=row['seed'], jar=jar, use_mem_run=False,
                                 properties={'max-players': row['context']['player_count'],
                                             'difficulty': 'peaceful', 'white-list': 'false',
                                             'generator-settings': json.dumps({
                                                 'biome': 'minecraft:plains',
                                                 'layers': [{'height': 1, 'block': 'minecraft:stone'}],
                                                 'features': False, 'lakes': False,
                                                 'structure_overrides': []})})
    evidence = {'jar': str(jar), 'jar_sha256': hashlib.sha256(jar.read_bytes()).hexdigest(),
                'status': 'running'}
    ctx.client_evidence['vanilla'] = evidence
    try:
        server.start()
        ctx.logs.append(str(server.log_path))
        minimum = json.loads((vanilla_data/'data/minecraft/dimension_type/overworld.json').read_text())['min_y']
        for index in range(int(row['context']['player_count'])):
            name = f'Proto{index}'
            client = ProtocolMatrixClient(name, ('127.0.0.1', server.port), directory,
                                          vanilla_data, server.command, account_uuid=offline_uuid(name))
            # Vanilla omits cursor packets when the client's prediction matches;
            # force a correction so the observer never relies on predicted state.
            client.observer.force_cursor_sync = True
            client.chunk_min_y = minimum
            client.observer.chunk_min_y = minimum
            clients.append(client)
        matrix = MatrixContext(row, server, clients, directory, state={'enabled_plugins': []})
        checks = run_subject(matrix)
        evidence.update(status='pass' if all(check['passed'] for check in checks) else 'fail',
                        checks=checks, final=matrix.state.get('final'), state=matrix.state)
        if evidence['status'] != 'pass':
            raise AssertionError('vanilla script invariant failed')
        return matrix.state['final']
    except Exception as error:
        evidence.update(status='fail', note=str(error))
        raise
    finally:
        for client in clients:
            client.disconnect()
        server.stop()
        directory.mkdir(parents=True, exist_ok=True)
        (directory/'result.json').write_text(json.dumps(evidence, indent=2)+'\n')


def compare_player_states(krasnokamen, vanilla, *, fault=False):
    if fault:
        import copy
        vanilla = copy.deepcopy(vanilla)
        vanilla['container']['minecraft:dirt'] = vanilla['container'].get('minecraft:dirt', 0) + 1
    return {'passed': krasnokamen == vanilla, 'krasnokamen': krasnokamen, 'vanilla': vanilla,
            'exclusions': ['RNG', 'AI', 'random loot', 'mounts', 'rejoin', 'kill9'],
            'normalization': 'item registry names and counts per holder; touched block registry names'}
