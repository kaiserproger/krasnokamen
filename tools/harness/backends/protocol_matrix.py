"""Canonical protocol-matrix profile: shared adapters, lifecycle and receipts.

No private action language or runner. Capability probes are supplemental and
cannot be substituted for a matrix subject oracle.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import socket
from pathlib import Path

from .. import runtime, wire
from ..matrix_api import MatrixContext, MissingCapability, load_adapter
from ..matrix_execution import (copy_plugins, offline_uuid, prepare_access, run_subject,
                               select_plugin_profile, plugin_set_fingerprint)
from ..protocol_matrix_client import ProtocolMatrixClient
from .protocol_lifecycle import Blocked, ProtocolServer

CAPABILITIES = ('server_kill9_wal_replay', 'duplicate_login_same_account', 'server_hot_backup',
                'server_pause_empty', 'malformed_packet_injection', 'reconnect_mid_action',
                'slow_reader_backpressure', 'select_trade', 'container_button', 'click_advanced_modes')


def stone_count(client):
    return sum(stack['count'] for stack in client.snapshot().get('inventory', [])
               if stack['item_id'] == 'minecraft:stone')


def capability_probe(name, server, clients, report):
    if name == 'server_pause_empty':
        report['pause'] = server.pause_empty_probe()
        return
    if name == 'malformed_packet_injection':
        cases = {'overlong_length': b'\x80\x80\x80\x80\x80\x00',
                 'negative_length': b'\xff\xff\xff\xff\x0f',
                 'truncated_handshake': wire.frame(0, b'\x87')}
        for label, payload in cases.items():
            with socket.create_connection(('127.0.0.1', server.port), timeout=10) as sock:
                sock.settimeout(10)
                sock.sendall(payload)
                sock.shutdown(socket.SHUT_WR)
                try:
                    data = sock.recv(4096)
                    if data:
                        raise AssertionError(f'malformed {label}: unexpected response {data.hex()}')
                except ConnectionResetError:
                    pass
            report.setdefault('rejected_frames', []).append(label)
            wire.probe_status(('127.0.0.1', server.port))
        return
    actor = clients[0]
    server.command(f'clear {actor.name}')
    if name == 'server_kill9_wal_replay':
        server.save_all()  # Empty baseline checkpoint, mutation must replay WAL.
    server.command(f'give {actor.name} stone 16')
    actor.wait_until(lambda: stone_count(actor) == 16, description='finite received grant')
    if name == 'server_kill9_wal_replay':
        report['kill9'] = server.kill9_restart()  # No save between ACK and SIGKILL.
        actor.disconnect()
        actor.reconnect()
        actor.wait_until(lambda: stone_count(actor) == 16, description='WAL-restored inventory')
        report['restored'] = actor.snapshot()
    elif name == 'duplicate_login_same_account':
        duplicate = None
        try:
            duplicate = wire.PlayObserver(('127.0.0.1', server.port), actor.name,
                        account_uuid=actor.account_uuid, timeout=30, trace_path=server.run_dir/'duplicate.wire.jsonl')
            duplicate.ready()
            actor.observer.wait(lambda p: p.error, description='old duplicate session removed')
            report['duplicate_policy'] = 'new joins, old removed'
        except RuntimeError as error:
            report['duplicate_policy'] = str(error)
            raise AssertionError('duplicate-login vanilla policy failed: ' + str(error)) from error
        finally:
            if duplicate:
                duplicate.close()
    elif name == 'server_hot_backup':
        backup = server.hot_backup()
        report['restore'] = server.restore_proof(backup)
        actor.disconnect()
        server.stop()
        server.world_dir = Path(report['restore']['target'])
        server.start()
        actor.reconnect()
        actor.wait_until(lambda: stone_count(actor) == 16, description='backup-restored inventory')
        report['restored'] = actor.snapshot()
    elif name == 'reconnect_mid_action':
        server.command('fill -3 79 -3 3 79 3 stone')
        server.command('fill -3 80 -3 3 83 3 air')
        actor.teleport_to(0.5,80,0.5)
        server.command('setblock 1 80 0 chest')
        actor.open_container((1,80,0))
        menu = actor.snapshot()['menu']
        source = next(stack['slot'] for stack in menu['slots'] if stack['item_id']=='minecraft:stone')
        actor.click_slot(source, kind='quick_move')
        actor.wait_until(lambda: any(s['slot']<27 and s['count']==16 for s in actor.snapshot()['menu']['slots']),
                         description='received finite chest deposit')
        actor.click_slot(0)
        actor.wait_until(lambda: (actor.snapshot().get('cursor') or {}).get('count')==16,
                         description='received cursor custody')
        actor.disconnect()
        actor.reconnect()
        actor.wait_until(lambda: stone_count(actor)==16, description='cursor returned exactly once')
        actor.open_container((1,80,0))
        result = actor.snapshot()
        if sum(s['count'] for s in result['menu']['slots']) != 16 or result.get('cursor'):
            raise AssertionError('reconnect duplicated/lost cursor stock')
        report['restored'] = result
    else:
        raise MissingCapability(name + ':live_subject_oracle')


def run(ctx, *, random_mode=False):
    parser = argparse.ArgumentParser(prog='protocol-matrix')
    parser.add_argument('--matrix', type=Path)
    parser.add_argument('--matrix-id')
    parser.add_argument('--subject')
    parser.add_argument('--capability', choices=CAPABILITIES)
    parser.add_argument('--port', type=int, default=26100)
    parser.add_argument('--dashboard-port', type=int, default=26101)
    parser.add_argument('--vanilla-data', type=Path, required=True)
    parser.add_argument('--exclude-plugin', action='append', default=[])
    if random_mode:
        parser.add_argument('--seed', type=int, default=0)
        parser.add_argument('--steps', type=int, default=20)
        parser.add_argument('--bots', type=int, default=3)
        parser.add_argument('--script', type=Path)
        parser.add_argument('--fault', choices=('no_server_error', 'conservation', 'legal_action',
                                               'observer_authority', 'recovery_atomic_durable'))
        parser.add_argument('--shrink', action='store_true')
        parser.add_argument('--differential', action='store_true')
        parser.add_argument('--vanilla-jar', type=Path)
        parser.add_argument('--differential-fault', action='store_true')
    args = parser.parse_args(ctx.extra_args)
    if not random_mode and sum(bool(value) for value in (args.capability,args.matrix_id,args.subject)) != 1:
        raise ValueError('select exactly one --capability, --subject, or --matrix-id')
    if ctx.server_bin is None:
        raise ValueError('protocol-matrix requires immutable --server-bin')
    report = {'driver':'protocol_bot', 'server_sha':hashlib.sha256(ctx.server_bin.read_bytes()).hexdigest(),
              'matrix_id':args.matrix_id, 'capability':args.capability, 'status':'running'}
    manifest = ctx.server_bin.parent / 'manifest.json'
    if manifest.is_file():
        provenance = json.loads(manifest.read_text())
        if (Path(provenance.get('binary', '')) != ctx.server_bin or
                provenance.get('binary_sha256') != report['server_sha']):
            raise ValueError('server pin manifest does not match selected binary')
        report['server_pin'] = provenance
        report['server_pin_manifest'] = str(manifest.resolve())
    ctx.client_evidence['protocol_matrix'] = report
    clients = []
    server = None
    matrix_context = None
    try:
        row = None
        adapter = None
        adapter_error = None
        profile_contract = {}
        if random_mode:
            from ..quirk_parity.fuzz import player_scenario
            row = json.loads(args.script.read_text()) if args.script else player_scenario(
                args.seed, args.steps, args.bots, differential=args.differential)
            if args.differential and args.vanilla_jar is None:
                raise ValueError('--differential requires an ignored 26.3 --vanilla-jar')
            if args.differential:
                from ..quirk_parity.fuzz import DETERMINISTIC_ACTIONS
                if any(step['verb'] not in DETERMINISTIC_ACTIONS for step in row['actions']):
                    raise ValueError('differential replay contains a non-deterministic domain')
            if row['subject'] != 'random.seeded':
                raise ValueError('replay script must select the random matrix adapter')
            if args.fault:
                row['fault'] = args.fault
            (ctx.artifact_dir/'random-script.json').write_text(json.dumps(row, indent=2)+'\n')
        elif args.matrix_id:
            import yaml
            if args.matrix is None:
                raise ValueError('--matrix-id requires --matrix')
            matrix = yaml.safe_load(args.matrix.read_text())
            profile_contract = matrix.get('ledger_contract', {}).get('plugin_profiles', {})
            row = next((r for r in matrix['scenarios'] if r['id']==args.matrix_id),None)
            if row is None:
                raise ValueError('unknown matrix id: '+args.matrix_id)
            if row['driver']['kind'] != 'protocol_bot':
                raise ValueError('matrix row belongs to another driver')
        elif args.subject:
            row = {'id':'adapter-probe:'+args.subject, 'subject':args.subject, 'origin':'adapter_probe',
                   'context':{'client_kind':'protocol_bot','game_mode':'survival','dimension':'overworld',
                   'difficulty':'normal','player_count':'1','network_condition':'normal',
                   'server_lifecycle':'steady','permission':'non_op_0'}}
        if row:
            report['row'] = row
            try:
                adapter = load_adapter(row['subject'])
            except MissingCapability as error:
                adapter_error = error
        players = int(row['context']['player_count'].removeprefix('N')) if row else 1
        names = [f'Proto{i}' for i in range(players)]
        plugins = None
        enabled_plugins = []
        excluded_plugins, expected_plugin_set = select_plugin_profile(
            row, profile_contract,
            adapter_exclusions=adapter.exclude_plugins if adapter else (),
            requested_exclusions=args.exclude_plugin)
        if ctx.plugins_dir is not None:
            plugins = ctx.artifact_dir/'plugins'
            enabled_plugins = copy_plugins(ctx.plugins_dir, plugins, excluded_plugins)
            report['plugin_set'] = plugin_set_fingerprint(plugins)
            if expected_plugin_set is not None and report['plugin_set'] != expected_plugin_set:
                raise ValueError('deployed plugin_set does not match exact matrix profile: '
                                 + report['plugin_set'] + ' != ' + expected_plugin_set)
        elif excluded_plugins:
            raise ValueError('cannot exclude plugins without --plugins-dir')
        report['enabled_plugins'] = enabled_plugins
        report['excluded_plugins'] = sorted(excluded_plugins)
        if expected_plugin_set is not None and plugins is None:
            raise ValueError('exact matrix plugin profile requires --plugins-dir')
        if adapter_error is not None:
            raise adapter_error
        if row and 'same_slot_same_tick' in row.get('origin',''):
            raise MissingCapability('same_tick_admission_fence')
        prepare_access(ctx.artifact_dir,names,row['context']['permission'] if row else 'non_op_0')
        lifecycle = row['context']['server_lifecycle'] if row else 'steady'
        server = ProtocolServer(binary=ctx.server_bin,run_dir=ctx.artifact_dir,
                    port=args.port,dashboard_port=args.dashboard_port,vanilla_data=args.vanilla_data,
                    plugins_dir=plugins,seed=row.get('seed',81) if row else 81,pause_when_empty_seconds=1 if
                    args.capability=='server_pause_empty' or lifecycle=='pause_when_empty_resume' else 0,
                    command_timeout_seconds=30,port_range=(min(args.port,args.dashboard_port),max(args.port,args.dashboard_port)))
        server.enabled_plugins = enabled_plugins
        ctx.commands.append([str(ctx.server_bin),'--config',str(server.config_path)])
        ctx.cwds.append(str(runtime.REPO_ROOT)); ctx.exits.append(None); ctx.logs.append(str(server.log_path))
        server.start()
        if args.capability not in {'server_pause_empty','malformed_packet_injection'}:
            for name in names:
                clients.append(ProtocolMatrixClient(name,('127.0.0.1',args.port),ctx.artifact_dir,
                      args.vanilla_data,server.command,account_uuid=offline_uuid(name),
                      require_auth=plugins is not None and (plugins/'solaris-auth').is_dir()))
        if random_mode:
            if row['context']['dimension'] != 'overworld':
                raise ValueError('seeded player scripts require the deterministic overworld fixture')
            minimum = json.loads((args.vanilla_data/'data/minecraft/dimension_type/overworld.json').read_text())['min_y']
            for client in clients:
                client.chunk_min_y = minimum
                client.observer.chunk_min_y = minimum
        if clients:
            server.ensure_loaded_callback = clients[0].ensure_loaded
        if args.capability:
            capability_probe(args.capability,server,clients,report)
        else:
            server.dimension = row['context']['dimension']
            matrix_context = MatrixContext(row,server,clients,ctx.artifact_dir,
                                           state={'enabled_plugins': enabled_plugins})
            report['checks'] = run_subject(matrix_context)
            report['adapter_state'] = matrix_context.state
            report['observations'] = [client.snapshot() for client in clients]
            if not all(check['passed'] for check in report['checks']):
                raise AssertionError('domain oracle failed: '+','.join(c['name'] for c in report['checks'] if not c['passed']))
        report['status']='pass'
    except (Blocked,MissingCapability) as error:
        report.update(status='blocked',note=str(error)); raise
    except Exception as error:
        report.update(status='fail',note=str(error)); raise
    finally:
        if matrix_context is not None:
            report['adapter_state'] = matrix_context.state
        for client in clients:
            client.disconnect()
        if server:
            try:
                if server.running:
                    ctx.exits[-1] = server.stop()
            finally:
                server.close()
        (ctx.artifact_dir/'protocol-result.json').write_text(json.dumps(report,indent=2,allow_nan=False)+'\n')
    return {'protocol_matrix':report}
