"""Native matrix primitives invoked only by the centralized harness profile.

The external YAML selects fixed Python probes, never executable commands/DSL.
Receipts fail closed when an exact matrix oracle cannot be observed by MCP.
"""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import math
import os
import time
import uuid
import shlex
import sys
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace
from typing import Any

from .. import runtime
from . import native_client
from .probe_steps import ProbeSteps, container_slot, item_count


def select(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [row for row in rows if row['context']['client_kind'] in {'native_linux', 'native_windows'}]


def pending(rows: list[dict[str, Any]], statuses: dict[str, str]) -> list[dict[str, Any]]:
    return [row for row in rows if statuses.get(row['id']) != 'pass']


def transport_blocker(scenario: dict[str, Any]) -> str | None:
    context = scenario['context']
    if context['client_kind'] == 'native_windows':
        return 'windows_mcp_lane: Windows VM is Main-owned; Linux evidence cannot substitute'
    condition = context['network_condition']
    if condition == 'slow_reader_backpressure':
        return 'slow_reader_backpressure: native MCP exposes no socket-reader rate control'
    if condition == 'malformed_or_stale_packet':
        return 'malformed_packet_injection: ordinary native MCP does not expose raw/stale packet injection'
    return None


def record_receipt(receipt: dict[str, Any], receipt_path: Path) -> None:
    """Called by cmd_run only after its authoritative result.json exists."""
    matrix = receipt.get('details', {}).get('matrix')
    if not matrix:
        return
    ledger = Path(matrix['ledger'])
    ledger.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(ledger, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        for row in matrix['rows']:
            line = {'id': row['id'], 'status': row['status'], 'driver': 'native_mcp',
                    'server_sha': matrix['server_sha'], 'run_dir': row['run_dir'],
                    'ts': datetime.now(timezone.utc).isoformat().replace('+00:00', 'Z'),
                    'note': f"{row['note']}; harness_receipt={receipt_path}"}
            if 'plugin_set' in row:
                line['plugin_set']=row['plugin_set']
            payload = (json.dumps(line, ensure_ascii=False) + '\n').encode()
            if os.write(fd, payload) != len(payload):
                raise RuntimeError('short append to matrix ledger')
        os.fsync(fd)
    finally:
        fcntl.flock(fd, fcntl.LOCK_UN)
        os.close(fd)
    if 'run_root' in matrix:
        _write_summary(matrix)


def _write_summary(matrix: dict[str,Any]) -> None:
    """Aggregate current frozen cells from last ledger rows on this binary."""
    import yaml

    wanted=set(matrix['all_native_ids'])
    latest={}
    with Path(matrix['ledger']).open() as handle:
        fcntl.flock(handle,fcntl.LOCK_SH)
        try:
            for text in handle:
                row=json.loads(text)
                if row['driver']=='native_mcp' and row['server_sha']==matrix['server_sha'] and row['id'] in wanted:
                    latest[row['id']]=row
        finally:
            fcntl.flock(handle,fcntl.LOCK_UN)
    counts={status:sum(row['status']==status for row in latest.values())
            for status in ('pass','fail','blocked','skipped')}
    failures=[row for row in latest.values() if row['status']=='fail']
    root=Path(matrix['run_root'])
    summary={'executed':counts['pass']+counts['fail'],**counts,'total':len(wanted),
             'not_recorded':len(wanted)-len(latest),'server_sha':matrix['server_sha'],
             'wave1_id_set_sha256':matrix['wave1_id_set_sha256'],
             'matrix_sha256':matrix['matrix_sha256'],
             'new_capabilities_added':['native MCP shared matrix verbs and received chunk fences',
                 'isolated graphical clients with live-plugin copies and exact permission fixtures',
                 'SIGKILL/WAL and clean-restart storage/cursor conservation probes',
                 'duplicate-account admission probe and account-isolated ender storage',
                 'canonical receipt-derived locked append-only ledger'],
             'top_bugs':[{'severity':'P1','id':row['id'],'note':row['note'],
                          'run_dir':row['run_dir']} for row in failures[:5]]}
    (root/'summary.yaml').write_text(yaml.safe_dump(summary,sort_keys=False))
    bugs=['# Native matrix findings','',
          'Only latest frozen-cell outcomes on the recorded server binary are summarized.','']
    for row in failures:
        command=shlex.join([*matrix['repro_prefix'],'--only-id',row['id']])
        bugs.extend(['## P1 '+row['id'],'',row['note'],'',
                     f"Evidence: `{row['run_dir']}`",'',f'```sh\n{command}\n```',''])
    if not failures:
        bugs.append('No current fail row. Blocked cells remain explicitly unproven in the ledger.')
    (root/'bugs.md').write_text('\n'.join(bugs)+'\n')


class Lane:
    """Native clients around the shared ProtocolServer lifecycle primitive."""
    def __init__(self, ctx: Any, scenario: dict[str, Any], directory: Path, binary: Path,
                 native: Path, port: int, timeout: float, exclude_plugins: tuple[str,...]=()) -> None:
        from .protocol_lifecycle import ProtocolServer

        self.ctx, self.scenario, self.directory = ctx, scenario, directory
        self.binary, self.native, self.port, self.timeout = binary, native, port, timeout
        self.address = f'127.0.0.1:{port}'
        self.clients: list[Any] = []
        self.processes: list[Any] = []
        self.handles: list[Any] = []
        self.contexts: list[Any] = []
        self.names: list[str] = []
        self.server_processes: list[Any] = []
        self.xvfb = None
        self.steps: ProbeSteps
        self.round = 0
        self.fixture_commands: list[str] = []
        self.exclude_plugins=exclude_plugins
        self.chunk_fences: list[dict[str,Any]]=[]
        self.wal_checkpoint_baseline: str | None=None
        self.host = ProtocolServer(binary=binary, run_dir=directory, port=port,
            vanilla_data=Path(os.environ.get('SOLARIS_VANILLA_DATA', runtime.REPO_ROOT/'data/vanilla')), seed=81,
            plugins_dir=directory/'plugins', server_name='RunNative', view_distance=3,
            startup_timeout_seconds=timeout, command_timeout_seconds=timeout,
            port_range=(26300,26399),
            pause_when_empty_seconds=1 if scenario['context']['server_lifecycle']=='pause_when_empty_resume' else 0)

    def _record_server(self) -> None:
        index=len(self.ctx.commands)
        self.ctx.commands.append([str(self.binary),'--config',str(self.host.config_path)])
        self.ctx.cwds.append(str(runtime.REPO_ROOT)); self.ctx.exits.append(None)
        self.ctx.logs.append(str(self.host.log_path))
        self.server_processes.append((self.host._process,index))
        self.round+=1

    def launch(self, name: str) -> Any:
        from ..audio import private_audio_preflight

        audio_dir=self.directory/f'audio-before-{len(self.processes)}'
        private_audio_preflight(audio_dir,require_alsa=True)
        self.ctx.client_evidence.setdefault('private_audio_receipts',[]).append(
            str(audio_dir/'private-audio-preflight.json'))
        directory = self.directory / f'client-{len(self.processes)}'
        directory.mkdir()
        ctx = SimpleNamespace(artifact_dir=directory, client_evidence={}, windowed=True,
                              commands=self.ctx.commands, cwds=self.ctx.cwds,
                              exits=self.ctx.exits, logs=self.ctx.logs)
        process, log, client = native_client.start(ctx, self.native, self.address, name,
                                                  self.timeout, display=self.display)
        self.processes.append(process); self.handles.append(log); self.contexts.append(ctx)
        client.request_timeout_seconds = self.timeout + 5
        ctx.client_evidence['available_tools']=client.list_tools()
        return client

    def console(self, command: str) -> str:
        self.fixture_commands.append(command)
        return self.host.command(command)

    def authenticate(self, client: Any, name: str) -> None:
        play = client.call_tool('minecraft_wait_for_play', {'timeout_seconds': self.timeout})
        if play.get('in_play') is not True:
            raise RuntimeError(f'native client cannot enter Play: {play}')
        if 'solaris-auth' not in self.host.enabled_plugins:
            return
        client.call_tool('minecraft_send_chat', {'message': 'register native-matrix-test native-matrix-test', 'command': True})
        client.call_tool('minecraft_send_chat', {'message': 'login native-matrix-test', 'command': True})
        with ProbeSteps(client, self.directory / f'auth-{name}-{self.round}', self.timeout, 'plugin-auth') as probe:
            probe.wait(lambda s: any('Successfully logged in' in line or 'Уже авторизован' in line
                                    or 'already logged' in line.lower() for line in s['recent_chat']), 'test_account_authenticated')

    def __enter__(self) -> Lane:
        from ..matrix_execution import prepare_access, copy_plugins

        self.directory.mkdir(parents=True, exist_ok=True)
        self.host.enabled_plugins=copy_plugins(self.ctx.plugins_dir,self.directory/'plugins',self.exclude_plugins)
        context=self.scenario['context']
        count=8 if context['player_count']=='N8' else int(context['player_count'])
        prepare_access(self.directory,[f'NativeMx{index}' for index in range(count)],context['permission'])
        try:
            self.host.start()
            self._record_server()
            self.xvfb,self.display,handle=runtime.start_xvfb(self.directory)
            self.handles.append(handle)
            for index in range(count):
                name=f'NativeMx{index}'
                client=self.launch(name); self.clients.append(client); self.names.append(name)
                self.authenticate(client,name)
            self.host.ensure_loaded_callback=self.ensure_loaded
            self.host.dimension=context['dimension']
            self.console('difficulty '+context['difficulty'])
            self.console('fill 185 99 -455 215 99 -435 minecraft:obsidian')
            self.console('fill 185 100 -455 215 105 -435 minecraft:air')
            for index,name in enumerate(self.names):
                self.console(f'gamemode {context["game_mode"]} {name}')
                self.console(f'execute in minecraft:{context["dimension"]} run tp {name} {192+index*2}.5 100 -447.5')
                self.settle(client=self.clients[index],name=name,
                            expected=(192+index*2+.5,100.,-447.5))
            self.console('kill @e[type=minecraft:item]')
            return self
        except BaseException:
            self.close()
            raise

    def reconnect(self) -> None:
        self.clients[0].call_tool('minecraft_disconnect')
        self.clients[0].call_tool('minecraft_connect',{'server_addr':self.address})
        self.authenticate(self.clients[0],self.names[0])
        self.settle(self.clients[0],self.names[0])

    def restart(self, crash: bool=False) -> None:
        proof=self.host.kill9_restart() if crash else self.host.clean_restart()
        self._record_server()
        self.steps.report.setdefault('restarts',[]).append(proof)
        if crash and self.wal_checkpoint_baseline is not None:
            proof['checkpoint_sha256_before_action']=self.wal_checkpoint_baseline
            if proof.get('checkpoint_sha256_before_restart')!=self.wal_checkpoint_baseline:
                raise NotImplementedError('server_kill9_before_next_checkpoint: checkpoint changed during action; WAL-only recovery unproven')
            self.steps.check('ordinary_transfer_not_checkpointed_before_SIGKILL',True)
        for client,name in zip(self.clients,self.names):
            state=client.call_tool('minecraft_observe')
            while state.get('in_play'):
                client.call_tool('minecraft_wait_for_state_change',{'observed_version':state['state_version'],'timeout_seconds':self.timeout})
                state=client.call_tool('minecraft_observe')
            client.call_tool('minecraft_connect',{'server_addr':self.address})
            self.authenticate(client,name)
            self.settle(client,name)
        self.steps.deadline=time.monotonic()+self.timeout

    def duplicate(self) -> None:
        client=self.launch(self.names[0])
        try:
            state=client.call_tool('minecraft_wait_for_play',{'timeout_seconds':self.timeout})
            self.steps.report['duplicate_login']=state
            self.steps.check('duplicate_refused',state.get('in_play') is False)
            self.steps.check('duplicate_uuid_reason','already joined' in state.get('disconnect_reason','').lower())
            self.steps.check('original_session_untouched',self.steps.observe()['in_play'])
        finally:
            client.close()

    def settle(self, client: Any, name: str,
               expected: tuple[float,float,float] | None=None) -> None:
        with ProbeSteps(client,self.directory/f'settle-{name}-{self.round}',self.timeout,'fixture-settle') as probe:
            def ready(state: dict[str,Any]) -> bool:
                grounded=state['player']['on_ground'] or self.scenario['context']['game_mode']=='spectator'
                if not state.get('in_play') or not grounded:
                    return False
                if expected is None:
                    return state['player'].get('health') is not None
                correction=state['player'].get('server_position')
                return correction is not None and all(abs(a-b)<.2 for a,b in zip(correction,expected))
            probe.wait(ready,'received_grounded_fixture_pose')

    def custody_boundary(self) -> None:
        context=self.scenario['context']
        if context['network_condition']=='duplicate_login':
            self.duplicate()
        elif context['network_condition'] in {'reconnect_mid_action','disconnect_during_menu'}:
            self.reconnect()
        lifecycle=context['server_lifecycle']
        if lifecycle=='crash_kill9_wal_replay':
            self.restart(crash=True)
        elif lifecycle=='clean_restart_mid_action':
            self.restart()
        elif lifecycle=='save_all_checkpoint':
            self.steps.report['save_all']=self.host.save_all()
        elif lifecycle=='pause_when_empty_resume':
            self.host.mark_boundary()
            for client in self.clients:
                client.call_tool('minecraft_disconnect')
            proof=self.host.wait_empty_pause()
            self.steps.report['pause']=proof
            for client,name in zip(self.clients,self.names):
                client.call_tool('minecraft_connect',{'server_addr':self.address})
                self.authenticate(client,name)
            self.steps.wait(lambda state:self.host.dashboard_stats()['simulation_tick']>proof['tick'],
                            'world_tick_resumed_after_native_rejoin')
        elif lifecycle!='steady':
            raise NotImplementedError('server_'+lifecycle+': complete restored-state oracle unavailable')

    def ensure_loaded(self, area: Any) -> None:
        low,high=area
        for client,name in zip(self.clients,self.names):
            for chunk_x in range(low[0]//16,high[0]//16+1):
                for chunk_z in range(low[2]//16,high[2]//16+1):
                    result=client.call_tool('minecraft_wait_for_loaded_block',{
                        'x':chunk_x*16,'y':80,'z':chunk_z*16,'timeout_seconds':self.timeout})
                    self.chunk_fences.append({'client':name,'chunk':[chunk_x,chunk_z],'result':result})

    def close(self) -> None:
        audio_error=None
        if self.processes:
            from ..audio import private_audio_preflight
            audio_dir=self.directory/'audio-final'
            try:
                private_audio_preflight(audio_dir,require_alsa=True)
                self.ctx.client_evidence.setdefault('private_audio_receipts',[]).append(
                    str(audio_dir/'private-audio-preflight.json'))
            except Exception as error:
                audio_error=error
        for client in self.clients:
            try:
                client.close()
            except Exception:
                pass
        for process in [*self.processes,self.xvfb]:
            runtime.stop_process(process)
        self.host.close()
        for process,index in self.server_processes:
            self.ctx.exits[index]=process.returncode
        for process,ctx in zip(self.processes,self.contexts):
            self.ctx.exits[ctx.client_evidence['command_index']]=process.returncode
        for handle in self.handles:
            handle.close()
        (self.directory/'lifecycle.json').write_text(json.dumps({
            'fixtures':self.fixture_commands,
            'clients':[ctx.client_evidence for ctx in self.contexts],
            'server_rounds':self.round,
            'enabled_plugins':self.host.enabled_plugins,'excluded_plugins':self.exclude_plugins,
            'chunk_fences':self.chunk_fences,
        },indent=2))
        if audio_error is not None:
            raise audio_error

    def __exit__(self,*args: Any) -> None:
        self.close()


STORAGE = {
    'containers.chest': ('chest', 27), 'containers.trapped_chest': ('trapped_chest', 27),
    'containers.barrel': ('barrel', 27), 'containers.copper_chest': ('copper_chest', 27),
    'containers.ender_chest': ('ender_chest', 27), 'containers.shulker_box': ('shulker_box', 27),
    'containers.double_chest': ('chest', 54),
}


def storage(lane: Lane, scenario: dict[str, Any]) -> None:
    from .native_v2 import _block, _click, _open, _point, _use
    steps = lane.steps
    block, slots = STORAGE[scenario['subject']]
    before = steps.wait(lambda s: s['player']['on_ground'], 'grounded')
    origin = tuple(math.floor(before['player'][key]) for key in ('x', 'y', 'z'))
    point = _point(origin, dz=2)
    x,y,z = point.values()
    lane.console(f'setblock {x} {y} {z} minecraft:{block}[facing=north]' if block != 'shulker_box'
                 else f'setblock {x} {y} {z} minecraft:shulker_box')
    if slots == 54:
        lane.console(f'setblock {x + 1} {y} {z} minecraft:chest[facing=north,type=right]')
        lane.console(f'setblock {x} {y} {z} minecraft:chest[facing=north,type=left]')
    _block(steps, point, 'minecraft:' + block)
    signal={'x':x+1,'y':y,'z':z}
    if block=='trapped_chest':
        lane.console(f'setblock {x+1} {y} {z} minecraft:redstone_wire')
    lane.console(f'clear {lane.names[0]}')
    lane.console(f'give {lane.names[0]} minecraft:diamond 1')
    steps.wait(lambda s: item_count(s, 'minecraft:diamond') == 1, 'fixture_one_diamond')
    if block=='shulker_box':
        lane.console(f'give {lane.names[0]} minecraft:shulker_box 1')
        steps.wait(lambda s:item_count(s,'minecraft:shulker_box')==1,'fixture_nested_shulker')
    state = _open(steps, point, 'ChestMenu' if block != 'shulker_box' else 'ShulkerBoxMenu', slots + 36)
    if block=='barrel':
        _block(steps,point,'minecraft:barrel',{'open':'true'})
    if block=='trapped_chest':
        _block(steps,signal,'minecraft:redstone_wire',{'power':'1'})
    if block=='shulker_box':
        nested=next(row['slot'] for row in state['container']['slots'] if row['item_id']=='minecraft:shulker_box')
        _click(steps,nested)
        try:
            _click(steps,0)
        except RuntimeError as error:
            steps.report['nested_shulker_refusal']=str(error)
        refused=steps.observe()
        steps.check('nested_shulker_refused',container_slot(refused,0)['count']==0
                    and refused['container']['carried']['item_id']=='minecraft:shulker_box'
                    and refused['container']['carried']['count']==1)
        _click(steps,nested)
    source = next(row['slot'] for row in state['container']['slots'] if row['item_id'] == 'minecraft:diamond')
    if scenario['origin']=='triple:dupe:kill9_between_click_and_checkpoint':
        checkpoint=lane.host.world_dir/'native/state.redb'
        if not checkpoint.is_file():
            raise NotImplementedError('server_kill9_before_next_checkpoint: native checkpoint baseline unavailable')
        with checkpoint.open('rb') as handle:
            lane.wal_checkpoint_baseline=hashlib.file_digest(handle,'sha256').hexdigest()
    _click(steps, source); _click(steps, 0)
    steps.wait(lambda s: container_slot(s, 0)['count'] == 1 and item_count(s, 'minecraft:diamond') == 0
               and s['container']['carried']['count'] == 0, 'deposit_exact_once')
    # Boundary while the stack is in container custody and menu is still open.
    lane.custody_boundary()
    state = steps.observe(wait_for_play=True)
    if state['container']['container_id'] == 0:
        _open(steps, point, 'ChestMenu' if block != 'shulker_box' else 'ShulkerBoxMenu', slots + 36)
    state = steps.observe()
    holder=container_slot(state,0)
    holder_count=holder['count'] if holder['item_id']=='minecraft:diamond' else 0
    player_count=item_count(state,'minecraft:diamond')
    steps.check('persisted_holder_xor_inventory',(holder_count,player_count) in {(1,0),(0,1)}
                and state['container']['carried']['count']==0)
    if player_count==1:
        # The matrix permits either committed side after crash recovery.
        # Re-deposit the recovered player copy for the account-isolation and
        # withdrawal checks, without changing the already-recorded XOR oracle.
        source=next(row['slot'] for row in state['container']['slots']
                    if row['slot']>=slots and row['item_id']=='minecraft:diamond')
        _click(steps,source);_click(steps,0)
        steps.wait(lambda s:container_slot(s,0)['item_id']=='minecraft:diamond'
                   and container_slot(s,0)['count']==1 and item_count(s,'minecraft:diamond')==0,
                   'recovered_player_copy_redeposited')
    if block=='trapped_chest':
        _block(steps,signal,'minecraft:redstone_wire',{'power':'1'})
    entities=steps.call('minecraft_list_entities',{'radius':16,'limit':512})
    steps.check('no_replayed_duplicate_item_entity',not entities['truncated'] and not any(
        row.get('item',{}).get('item_id')=='minecraft:diamond' for row in entities['entities']))
    if slots==54:
        steps.call('minecraft_close_screen')
        _open(steps,{'x':x+1,'y':y,'z':z},'ChestMenu',90)
        other=steps.observe()
        steps.check('double_chest_halves_consistent',container_slot(other,0)['item_id']=='minecraft:diamond'
                    and container_slot(other,0)['count']==1)
    if block=='ender_chest':
        # Keep one connected player, but prove storage belongs to the account:
        # a sequential second identity must not see the actor's diamond.
        lane.clients[0].call_tool('minecraft_disconnect')
        other=lane.launch('NativeOther')
        try:
            lane.authenticate(other,'NativeOther')
            lane.console(f'tp NativeOther {origin[0]+.5} {origin[1]} {origin[2]+.5}')
            with ProbeSteps(other,lane.directory/'ender-other',lane.timeout,'ender-account-isolation') as probe:
                probe.wait(lambda s:s['player']['on_ground'],'other_grounded')
                _open(probe,point,'ChestMenu',63)
                seen=probe.observe()
                probe.check('ender_other_account_empty',all(row.get('count')==0 for row in seen['container']['slots'] if row['slot']<27))
            other.call_tool('minecraft_disconnect')
        finally:
            other.close()
        lane.clients[0].call_tool('minecraft_connect',{'server_addr':lane.address})
        lane.authenticate(lane.clients[0],lane.names[0])
        _open(steps,point,'ChestMenu',63)
        seen=steps.observe()
        steps.check('ender_original_account_retained_one',container_slot(seen,0)['item_id']=='minecraft:diamond'
                    and container_slot(seen,0)['count']==1)
    _click(steps,0,quick=True)
    state = steps.wait(lambda s:item_count(s,'minecraft:diamond')==1, 'withdraw_exact_once')
    steps.check('holder_empty_after_quickmove', container_slot(state,0)['count']==0
                and state['container']['carried']['count']==0)
    steps.call('minecraft_close_screen')
    if block=='barrel':
        _block(steps,point,'minecraft:barrel',{'open':'false'})
    if block=='trapped_chest':
        _block(steps,signal,'minecraft:redstone_wire',{'power':'0'})
    _open(steps,point,'ChestMenu' if block!='shulker_box' else 'ShulkerBoxMenu',slots+36)
    state=steps.observe()
    steps.check('reopen_no_copy',container_slot(state,0)['count']==0 and item_count(state,'minecraft:diamond')==1)
    entities=steps.call('minecraft_list_entities',{'radius':16,'limit':512})
    steps.check('final_diamond_conserved_across_holders',not entities['truncated'] and not any(
        row.get('item',{}).get('item_id')=='minecraft:diamond' for row in entities['entities']))
    if block=='shulker_box':
        steps.check('nested_shulker_remains_in_player',item_count(state,'minecraft:shulker_box')==1)


def run(ctx: Any) -> dict[str, Any]:
    parser = argparse.ArgumentParser(prog='native-matrix')
    parser.add_argument('--matrix',type=Path,default=os.environ.get('SOLARIS_MATRIX_PATH'),
                        required=not os.environ.get('SOLARIS_MATRIX_PATH'))
    parser.add_argument('--ledger',type=Path,default=os.environ.get('SOLARIS_MATRIX_LEDGER'),
                        required=not os.environ.get('SOLARIS_MATRIX_LEDGER'))
    parser.add_argument('--run-root',type=Path,default=os.environ.get('SOLARIS_MATRIX_RUN_ROOT'),
                        required=not os.environ.get('SOLARIS_MATRIX_RUN_ROOT'))
    parser.add_argument('--only-id',action='append',default=[])
    parser.add_argument('--limit',type=int)
    parser.add_argument('--pending',action='store_true')
    parser.add_argument('--exclude-plugin',action='append',default=[])
    args=parser.parse_args(ctx.extra_args)
    if ctx.client!='native' or not ctx.windowed or ctx.server_bin is None or ctx.plugins_dir is None:
        raise ValueError('native-matrix requires --client native --windowed --server-bin --plugins-dir')
    binary=ctx.server_bin.resolve()
    with binary.open('rb') as handle:
        digest=hashlib.file_digest(handle,'sha256').hexdigest()
    import yaml
    matrix_bytes=args.matrix.read_bytes()
    matrix=yaml.safe_load(matrix_bytes)
    all_native=select(matrix['scenarios'])
    rows=all_native
    if args.only_id:
        rows=[row for row in rows if row['id'] in args.only_id]
        if len(rows)!=len(set(args.only_id)):
            raise ValueError('unknown/non-native matrix id')
    if args.pending and args.ledger.exists():
        statuses={line['id']:line['status'] for line in map(json.loads,args.ledger.read_text().splitlines())
                  if line.get('server_sha')==digest and line.get('driver')=='native_mcp'}
        rows=pending(rows,statuses)
    if args.limit is not None:
        rows=rows[:args.limit]
    if not rows:
        raise ValueError('native matrix selector matched no scenarios')
    native=native_client.resolve_binary(ctx)
    timeout=ctx.timeout_seconds or 60
    args.run_root.mkdir(parents=True,exist_ok=True)
    outcomes=[]
    id_set_hash=hashlib.sha256('\n'.join(sorted(row['id'] for row in matrix['scenarios'])).encode()).hexdigest()
    result={'ledger':str(args.ledger.resolve()),'server_sha':digest,'rows':outcomes,
            'matrix_sha256':hashlib.sha256(matrix_bytes).hexdigest(),
            'wave1_id_set_sha256':id_set_hash,'run_root':str(args.run_root.resolve()),
            'all_native_ids':[row['id'] for row in all_native]}
    ctx.client_evidence['matrix']=result
    resource_env={name:os.environ[name] for name in (
        'SOLARIS_CLIENT_ROOT','SOLARIS_CLIENT_BINARY','SOLARIS_CLIENT_DATA_DIR',
        'SOLARIS_CLIENT_RESOURCE_ROOT','SOLARIS_CLIENT_ASSET_ROOT','SOLARIS_CLIENT_ASSET_INDEX',
        'TMPDIR','SA_SCRATCH') if name in os.environ}
    resource_env['SOLARIS_AUDIO_RECEIPT_DIR']=str(args.run_root.resolve()/'repro-audio')
    result['repro_prefix']=['env','-u','WAYLAND_DISPLAY',*(f'{key}={value}' for key,value in resource_env.items()),
        '/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/mem-run.sh','-m','6G','sh',
        str(runtime.REPO_ROOT/'tools/harness/screen_audio_graph.sh'),sys.executable,'-m','tools.harness',
        'run','native-matrix','--client','native','--windowed','--server-bin',str(binary),
        '--plugins-dir',str(ctx.plugins_dir.resolve()),'--matrix',str(args.matrix.resolve()),
        '--ledger',str(args.ledger.resolve()),'--run-root',str(args.run_root.resolve()),
        '--timeout-seconds',str(timeout)]
    (args.run_root/'environment.json').write_text(json.dumps({
        'working_directory':str(runtime.REPO_ROOT),'environment':resource_env,
        'server_binary':str(binary),'server_sha':digest,'client_binary':str(native)},indent=2))
    from . import native_matrix_custody, native_matrix_interactions
    handlers=[native_matrix_custody,native_matrix_interactions]
    from ..matrix_api import MatrixContext, MissingCapability, load_adapter
    from ..matrix_execution import run_subject, select_plugin_profile, plugin_set_fingerprint, copy_plugins
    from .native_matrix_client import NativeMatrixClient
    plugin_profiles=matrix.get('ledger_contract',{}).get('plugin_profiles',{})
    plugin_cache={}
    for index,scenario in enumerate(rows):
        directory=args.run_root / ctx.artifact_dir.name / (hashlib.sha256(scenario['id'].encode()).hexdigest()[:16])
        directory.mkdir(parents=True,exist_ok=True)
        (directory/'matrix-scenario.json').write_text(json.dumps(scenario,indent=2))
        row={'id':scenario['id'],'status':'blocked','run_dir':str(directory.resolve()),'note':''}
        outcomes.append(row)
        blocker=transport_blocker(scenario)
        context=scenario['context']
        if not blocker and context['player_count'] == 'N8':
            blocker = 'needs exclusive perf window'
        if not blocker and context['server_lifecycle']=='hot_backup_during_action':
            blocker='server_hot_backup_restore_state_oracle: checksum restore alone does not inspect restored custody'
        primitive_context=(context['game_mode']=='survival' and context['dimension']=='overworld'
                           and context['player_count']=='1')
        handler=next((handler for handler in handlers if scenario['subject'] in handler.SUBJECTS),None) if primitive_context else None
        storage_primitive=primitive_context and scenario['subject'] in STORAGE
        adapter=None
        if not blocker and not storage_primitive and handler is None:
            try:
                adapter=load_adapter(scenario['subject'])
            except MissingCapability as error:
                blocker=str(error)
        exclusions,expected_plugin_set=select_plugin_profile(scenario,plugin_profiles,
            adapter_exclusions=adapter.exclude_plugins if adapter is not None else (),
            requested_exclusions=args.exclude_plugin)
        cache_key=tuple(sorted(exclusions))
        if cache_key not in plugin_cache:
            package_dir=args.run_root/ctx.artifact_dir.name/'plugin-sets'/hashlib.sha256(
                '\n'.join(cache_key).encode()).hexdigest()[:12]
            package_dir.parent.mkdir(parents=True,exist_ok=True)
            enabled=copy_plugins(ctx.plugins_dir,package_dir,cache_key)
            plugin_cache[cache_key]=(plugin_set_fingerprint(package_dir),enabled,str(package_dir))
        actual_plugin_set,enabled,package_dir=plugin_cache[cache_key]
        if expected_plugin_set is not None and actual_plugin_set!=expected_plugin_set:
            raise ValueError('exact plugin profile fingerprint differs from frozen matrix')
        row.update(plugin_set=actual_plugin_set,enabled_plugins=enabled,excluded_plugins=list(cache_key),
                   prepared_plugins_dir=package_dir)
        if not blocker and scenario['origin']=='triple:multiplayer_x_same_block':
            blocker='same_tick_native_MCP_batch: no server-tick-aligned multi-client submission primitive'
        if blocker:
            row['note']=blocker
        else:
            try:
                with Lane(ctx,scenario,directory,binary,native,26301+(index%90),timeout,tuple(sorted(exclusions))) as lane:
                    from ..matrix_api import MissingCapability
                    deployed=plugin_set_fingerprint(directory/'plugins')
                    if deployed!=actual_plugin_set:
                        raise RuntimeError('deployed plugin bytes differ from prepared fingerprint')
                    row['plugin_set']=deployed
                    row['enabled_plugins']=lane.host.enabled_plugins
                    row['excluded_plugins']=list(sorted(exclusions))
                    with ProbeSteps(lane.clients[0],directory/'probe',timeout,scenario['id']) as steps:
                        lane.steps=steps
                        if storage_primitive:
                            storage(lane,scenario)
                        elif handler is not None:
                            handler.run(lane,scenario)
                        else:
                            transcript=[]
                            wrappers=[NativeMatrixClient(name,client,lane.address,transcript,
                                      on_reconnect=lane.authenticate,fixture_command=lane.console)
                                      for name,client in zip(lane.names,lane.clients)]
                            shared=MatrixContext(scenario,lane.host,wrappers,directory,
                                state={'enabled_plugins':lane.host.enabled_plugins},
                                boundary_hook=lambda context:lane.custody_boundary())
                            try:
                                checks=run_subject(shared)
                                steps.report['domain_checks']=checks
                                for check in checks:
                                    steps.check(check['name'],check['passed'])
                            finally:
                                (directory/'adapter-transcript.json').write_text(json.dumps(transcript,indent=2))
                        native_client.capture(lane.clients[0],directory,'final')
                        row['status']='pass';row['note']='complete native probe; '+str(directory/'probe/scenario.json')
            except (NotImplementedError, MissingCapability) as error:
                row['note']=str(error)
            except Exception as error:
                row['status']='fail';row['note']=f'{type(error).__name__}: {error}; evidence={directory}'
        (directory/'outcome.json').write_text(json.dumps(row,indent=2))
        print('[native-matrix]',row['status'],scenario['id'],row['note'][:180],flush=True)
    counts={status:sum(row['status']==status for row in outcomes) for status in ('pass','fail','blocked','skipped')}
    result['counts']=counts
    (args.run_root/'windows-scenarios.json').write_text(json.dumps([s for s in all_native if s['context']['client_kind']=='native_windows'],indent=2))
    if counts['fail'] or counts['blocked']:
        raise RuntimeError('native matrix incomplete: '+json.dumps(counts))
    return {'matrix':result}
