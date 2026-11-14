"""Shared matrix factor/boundary execution for all harness client drivers.

No processes or CLI live here. Canonical profiles supply their real server and
client factories; domain code is shared unchanged between all three drivers.
"""
from __future__ import annotations

import hashlib
import json
import uuid
from dataclasses import asdict
from pathlib import Path

from .matrix_api import MatrixContext, MissingCapability, execute_adapter, load_adapter


def offline_uuid(name: str) -> uuid.UUID:
    return uuid.UUID(bytes=hashlib.md5(('OfflinePlayer:' + name).encode()).digest(), version=3)


def prepare_access(server_dir: Path, names: list[str], permission: str) -> None:
    """Write isolated access fixture BEFORE server start; /op only gives level 4."""
    if permission == 'console':
        level = 0
    elif permission == 'non_op_0':
        level = 0
    elif permission in {'op_1', 'op_2', 'op_3', 'op_4'}:
        level = int(permission[-1])
    else:
        raise ValueError('unknown permission context ' + permission)
    server_dir.mkdir(parents=True, exist_ok=True)
    (server_dir / 'ops.json').write_text(json.dumps([
        {'uuid': str(offline_uuid(name)), 'name': name, 'level': level,
         'bypassesPlayerLimit': False} for name in names if level
    ], indent=2) + '\n')


def copy_plugins(source: Path, target: Path, exclude_plugins=()) -> list[str]:
    """Copy immutable live packages per run, recording actual manifest IDs."""
    import shutil
    import tomllib
    excluded = set(exclude_plugins)
    if not all(isinstance(name, str) for name in excluded):
        raise ValueError('exclude_plugins must contain package IDs')
    packages = []
    known = set()
    for directory in sorted(source.iterdir()):
        if directory.is_dir() and (directory / 'plugin.toml').is_file():
            plugin_id = tomllib.loads((directory / 'plugin.toml').read_text())['id']
            packages.append((plugin_id, directory))
            known.add(plugin_id)
    if excluded - known:
        raise ValueError('unknown excluded plugins: ' + ','.join(sorted(excluded - known)))
    target.mkdir(parents=True, exist_ok=False)
    enabled = []
    for plugin_id, directory in packages:
        if plugin_id not in excluded:
            shutil.copytree(directory, target / directory.name)
            enabled.append(plugin_id)
    return enabled


def select_plugin_profile(row, profile_contract, *, adapter_exclusions=(),
                          requested_exclusions=()) -> tuple[set[str], str | None]:
    """Resolve exact matrix profiles; adapter defaults never weaken @live."""
    requested = set(requested_exclusions) | set((row or {}).get('exclude_plugins', ()))
    subject = (row or {}).get('subject')
    profile = (row or {}).get('context', {}).get('plugin_profile')
    contract = profile_contract.get(subject)
    if contract is None:
        if profile is not None or '@' in (row or {}).get('id', ''):
            raise ValueError('plugin profile on a non-profiled subject')
        return requested | set(adapter_exclusions), None
    if profile not in {'vanilla', 'live'} or not row['id'].endswith('@' + profile):
        raise ValueError('plugin-sensitive subject requires an exact @vanilla|@live profile')
    excluded = set(contract['changed_by']) if profile == 'vanilla' else set()
    if requested - excluded:
        raise ValueError('requested plugin exclusions contradict exact @' + profile)
    return excluded, contract[profile]


def plugin_set_fingerprint(root: Path) -> str:
    """Implement matrix ledger's manifest-ID/manifest/entry byte fingerprint."""
    import tomllib
    lines = []
    for directory in root.iterdir():
        manifest_path = directory / 'plugin.toml'
        if not directory.is_dir() or not manifest_path.is_file():
            continue
        manifest = manifest_path.read_bytes()
        meta = tomllib.loads(manifest.decode())
        entry = (directory / meta['entry']).read_bytes()
        lines.append(f"{meta['id']} {hashlib.sha256(manifest).hexdigest()} "
                     f"{hashlib.sha256(entry).hexdigest()}")
    return hashlib.sha256('\n'.join(sorted(lines)).encode()).hexdigest()


def apply_factors(ctx: MatrixContext) -> None:
    """Apply difficulty/mode/dimension centrally; permission is a prestart fixture."""
    factors = ctx.factors
    players = int(factors['player_count'].removeprefix('N'))
    if len(ctx.clients) != players:
        raise RuntimeError(f'context requires {players} clients, got {len(ctx.clients)}')
    if factors['game_mode'] not in {'survival', 'creative', 'adventure', 'spectator'}:
        raise ValueError('unknown game_mode')
    if factors['dimension'] not in {'overworld', 'the_nether', 'the_end'}:
        raise ValueError('unknown dimension')
    ctx.server.command('difficulty ' + factors['difficulty'])
    for client in ctx.clients:
        ctx.server.command(f'gamemode {factors["game_mode"]} {client.name}')
        if factors['dimension'] != 'overworld':
            ctx.server.command(f'execute in minecraft:{factors["dimension"]} run tp {client.name} 0.5 80 0.5')


def action_boundary(ctx: MatrixContext) -> None:
    """One semantic boundary; unimplemented network injection fails explicitly."""
    network = ctx.factors['network_condition']
    lifecycle = ctx.factors['server_lifecycle']
    if network in {'duplicate_login', 'malformed_or_stale_packet', 'slow_reader_backpressure'}:
        method_name = {'duplicate_login': 'duplicate_login',
                       'malformed_or_stale_packet': 'inject_malformed_or_stale',
                       'slow_reader_backpressure': 'slow_reader_boundary'}[network]
        method = getattr(ctx.clients[0], method_name, None)
        if not callable(method):
            raise MissingCapability('client:' + method_name)
        method()
    mark = getattr(ctx.server, 'mark_boundary', None)
    if callable(mark):
        mark()
    if lifecycle == 'save_all_checkpoint':
        ctx.server.save_all()
    elif lifecycle == 'hot_backup_during_action':
        backup = ctx.server.hot_backup()
        restore = getattr(ctx.server, 'restore_proof', None)
        if not callable(restore):
            raise MissingCapability('server:restore_proof')
        ctx.state['backup_restore'] = restore(backup)
    requires_disconnect = (network in {'disconnect_during_menu', 'reconnect_mid_action'}
                           or lifecycle in {'clean_restart_mid_action', 'crash_kill9_wal_replay',
                                            'pause_when_empty_resume'})
    if lifecycle == 'crash_kill9_wal_replay':
        # Direct SIGKILL while sockets/menu are still alive, not orderly leave.
        ctx.state['kill9'] = ctx.server.kill9_restart()
    if requires_disconnect:
        for client in ctx.clients:
            client.disconnect()
        if lifecycle == 'clean_restart_mid_action':
            restart = getattr(ctx.server, 'clean_restart', None)
            if not callable(restart):
                raise MissingCapability('server:clean_restart')
            restart()
        elif lifecycle == 'pause_when_empty_resume':
            pause = getattr(ctx.server, 'wait_empty_pause', None)
            if not callable(pause):
                raise MissingCapability('server:wait_empty_pause')
            ctx.state['pause'] = pause()
        for client in ctx.clients:
            client.reconnect()
    allowed_lifecycle = {'steady', 'save_all_checkpoint', 'hot_backup_during_action',
                         'clean_restart_mid_action', 'crash_kill9_wal_replay', 'pause_when_empty_resume'}
    allowed_network = {'normal', 'duplicate_login', 'malformed_or_stale_packet',
                       'slow_reader_backpressure', 'disconnect_during_menu', 'reconnect_mid_action'}
    if lifecycle not in allowed_lifecycle or network not in allowed_network:
        raise ValueError('unknown context boundary ' + network + ':' + lifecycle)


def run_subject(ctx: MatrixContext) -> list[dict]:
    """Called by every driver profile after its own process/client creation."""
    adapter = load_adapter(ctx.row['subject'])
    if ctx.boundary_hook is None:
        ctx.boundary_hook = action_boundary
    apply_factors(ctx)
    checks = execute_adapter(adapter, ctx)
    if all(check.passed for check in checks):
        players = int(ctx.factors['player_count'].removeprefix('N'))
        multiplayer = players > 1
        origin = ctx.row.get('origin', '')
        concurrent_origin = origin in {
            'triple:dupe:two_players_same_slot_same_tick',
            'triple:dupe:break_container_while_open',
            'triple:dupe:two_players_take_same_tick',
            'triple:dupe:destroy_during_interaction',
            'triple:multiplayer_x_same_block',
        }
        if players == 8 or (multiplayer and concurrent_origin):
            if ctx._actor_targets != {'own', 'shared'}:
                raise MissingCapability('concurrent_subject_actors')
        if (origin.startswith('triple:dupe:') or origin == 'triple:multiplayer_x_same_block'):
            strict = origin in {
                'triple:dupe:two_players_same_slot_same_tick',
                'triple:dupe:two_players_take_same_tick',
                'triple:multiplayer_x_same_block',
            }
            if strict and multiplayer and ctx._same_tick_actor_targets != {'own', 'shared'}:
                raise MissingCapability('same_tick_admission_fence')
            if ctx.state.get('composite_origin_verified') != origin:
                raise MissingCapability('composite_origin_oracle:' + origin)
    return [asdict(check) for check in checks]
