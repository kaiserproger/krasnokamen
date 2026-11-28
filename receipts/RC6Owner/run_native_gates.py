#!/usr/bin/env python3
"""Focused RC6 owner gates; reuses candidate harness lifecycle and survival probe."""
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import time
import traceback

BASE = Path('/run/media/kaiserroman/games/solaris-work')
HIST = BASE / 'wt/RC6Hist'
sys.path.insert(0, str(HIST))
from tools.harness import runtime
from tools.harness.profiles import ProfileContext
from tools.harness.backends import native_client, survival

OUT = Path(sys.argv[1]).resolve()
OUT.mkdir(parents=True, exist_ok=False)
SERVER = BASE / 'pins/e1f87c207ee8-server-x86_64-unknown-linux-gnu/krasnokamen'
CLIENT = Path(os.environ['KRASNOKAMEN_CLIENT_BINARY'])
ARCHIVE = BASE / 'receipts/RC6Owner/private-linux/krasnokamen-rc6-private-e1f87c207ee8-x86_64-unknown-linux-gnu.tar.gz'
SERVER_CONTENT = BASE / 'receipts/RC6Owner/offline/bundle/content'
TEMPLATE = BASE / 'qa-armada/template-data'
EXAMPLE = BASE / 'receipts/RC6Owner/private-linux/stage/example.toml'

def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')

def sha(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()

identity = {'candidate': 'e1f87c207ee826706e0a4b1f684565d93b6150c4',
            'build': 'candidate binary from retained pin (build identity recorded at release time)',
            'server': {'path': str(SERVER), 'sha256': sha(SERVER)},
            'client': {'path': str(CLIENT), 'sha256': sha(CLIENT)},
            'archive': {'path': str(ARCHIVE), 'sha256': sha(ARCHIVE)}}
save(OUT / 'identity.json', identity)
assert identity['server']['sha256'] == 'b485f3feb31dc8b08bc091f621be606cd8ce26f1e1bb6b0d61befca18252fb23'
assert identity['client']['sha256'] == 'b9acdc363f2176e8ec64005249786814d06f908ddb16d0184b4577b6584b60a2'
assert identity['archive']['sha256'] == 'e2f458a74638816e12a8e92910582cee4629763daed67171b6d80023e919a559'
results = {name: {'verdict': 'not run'} for name in ('player_path', 'restart_custody', 'owner_package')}

# Content-only links; never borrow the owner's settings, world, or microphone.
xdg = OUT / 'xdg-data'
content = xdg / 'krasnokamen-client'
content.mkdir(parents=True)
for name in ('facts', 'data', 'resources', 'assets', 'mojang'):
    source = TEMPLATE / name
    if source.exists():
        (content / name).symlink_to(source, target_is_directory=source.is_dir())
client_root = OUT / 'client-root'
client_root.mkdir()
os.environ.update(XDG_DATA_HOME=str(xdg), KRASNOKAMEN_CLIENT_ROOT=str(client_root))
for name in ('KRASNOKAMEN_CLIENT_DATA_DIR', 'KRASNOKAMEN_CLIENT_RESOURCE_ROOT',
             'KRASNOKAMEN_CLIENT_ASSET_ROOT', 'KRASNOKAMEN_CLIENT_ASSET_INDEX'):
    assert Path(os.environ[name]).exists(), f'missing client-stand path: {name}'
os.environ.pop('KRASNOKAMEN_CLIENT_SETTINGS', None)
save(OUT / 'environment.json', {k: v for k, v in os.environ.items() if k.startswith(('KRASNOKAMEN_', 'XDG_', 'PULSE_', 'PIPEWIRE_', 'ALSOFT_', 'ALSA_'))})

class Session:
    def __init__(self, directory, binary, example):
        self.directory, self.binary = directory, binary
        directory.mkdir(parents=True, exist_ok=True)
        self.ctx = ProfileContext(directory, client='native', windowed=True)
        self.server = self.client_process = self.xvfb = self.client = None
        self.handles = []
        self.events = []
        self.port = runtime.reserve_port()
        self.address = f'127.0.0.1:{self.port}'
        self.config = directory / 'gate.toml'
        text = example.read_text().replace('bind_address = "0.0.0.0"', 'bind_address = "127.0.0.1"', 1)
        text = text.replace('port = 25565', f'port = {self.port}', 1)
        text = text.replace('# vanilla_data_dir = "data/vanilla"', f'vanilla_data_dir = "{SERVER_CONTENT}"')
        self.config.write_text(text)
        save(directory / 'configuration-scope.json', {'source': str(example), 'changes': ['network.bind_address=127.0.0.1', f'network.port={self.port}', f'data.vanilla_data_dir={SERVER_CONTENT}'], 'gameplay': 'all packaged defaults, including seed=0, view_distance=8, simulation_distance=8', 'client': 'fresh settings; offline-test automation transport; default window dimensions and renderer'})
    def start_server(self, name):
        self.server_log = self.directory / (name + '.log')
        log = self.server_log.open('wb')
        self.handles.append(log)
        cmd = [str(self.binary), '--config', str(self.config)]
        self.events.append({'command': cmd, 'cwd': str(self.directory), 'log': str(self.server_log)})
        self.server = runtime.start_server(cmd, cwd=self.directory, stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        runtime.wait_port(self.port, 180, self.server, log_path=self.server_log)
    def stop_server(self):
        self.server.stdin.write(b'stop\n')
        self.server.stdin.flush()
        code = self.server.wait(timeout=runtime.SERVER_STOP_TIMEOUT_SECONDS)
        self.events.append({'console': 'stop', 'exit': code, 'log': str(self.server_log)})
        assert code == 0, f'server stop exit {code}'
        text = self.server_log.read_text(errors='replace')
        assert 'checkpoint committed' in text, 'missing graceful checkpoint'
        assert 'ERROR' not in text and 'panicked at' not in text, 'server error; inspect log'
    def start_client(self):
        self.xvfb, display, log = runtime.start_xvfb(self.directory)
        self.handles.append(log)
        self.client_process, log, self.client = native_client.start(self.ctx, CLIENT, self.address, 'RC6Owner', 180, display=display)
        self.handles.append(log)
        assert self.call('minecraft_wait_for_play', {'timeout_seconds': 180})['in_play']
    def call(self, tool, args=None):
        event = {'tool': tool, 'arguments': args or {}}
        self.events.append(event)
        try:
            event['result'] = self.client.call_tool(tool, args or {})
            return event['result']
        except Exception as e:
            event['error'] = str(e)
            raise
    def observe(self):
        result = self.call('minecraft_observe')
        assert result['in_play']
        return result
    def block(self, position, block_id):
        r = self.call('minecraft_wait_for_block_state', dict(zip(('x', 'y', 'z'), position)) | {'block_id': block_id, 'timeout_seconds': 60})
        assert r['status'] == 'matched' and r['block']['block_id'] == block_id
        return r
    def close(self):
        runtime.stop_process(self.client_process)
        runtime.stop_process(self.server, interrupt=True)
        runtime.stop_process(self.xvfb)
        for handle in self.handles:
            handle.close()
        save(self.directory / 'commands-and-observations.json', self.events)
        save(self.directory / 'client-launch.json', {'commands': self.ctx.commands, 'cwds': self.ctx.cwds, 'evidence': self.ctx.client_evidence})

def extra_custody(s, placement):
    state = s.observe()
    x, y, z = (math.floor(state['player'][axis]) for axis in ('x', 'y', 'z'))
    for bx, bz in ((x-3, z-3), (x+3, z+3)):
        s.call('minecraft_wait_for_loaded_block', {'x': bx, 'y': y-2, 'z': bz, 'timeout_seconds': 60})
    blocks = s.call('minecraft_scan_blocks', {'min_x': x-3, 'max_x': x+3, 'min_y': y-2, 'max_y': y+2, 'min_z': z-3, 'max_z': z+3, 'max_blocks': 245})['blocks']
    candidates = [b for b in survival._candidates(blocks, state['player']) if [b[a] for a in ('x', 'y', 'z')] != placement]
    assert candidates, 'no second safe ordinary mining target'
    target = candidates[0]
    x, y, z = (target[a] for a in ('x', 'y', 'z'))
    broken = s.call('minecraft_break_block', {'x': x, 'y': y, 'z': z, 'face': 'up', 'expected_drop_item_id': survival.DIRT, 'expected_drop_count': 0, 'timeout_seconds': 90})
    assert broken['started'] and broken['became_air']
    s.block((x, y, z), 'minecraft:air')
    deadline = time.monotonic() + 120
    while survival._count(state, survival.DIRT) == 0:
        assert time.monotonic() < deadline, 'second ordinary pickup timed out'
        drops = [e for e in s.call('minecraft_list_entities', {'radius': 4.0, 'limit': 128})['entities'] if e['entity_type'] == 'minecraft:item' and e.get('item', {}).get('item_id') == survival.DIRT]
        if drops:
            drop = min(drops, key=lambda e: (e['x']-x-.5)**2 + (e['z']-z-.5)**2)
            dx, dz = drop['x'] - state['player']['x'], drop['z'] - state['player']['z']
            s.call('minecraft_look', {'yaw_deg': round(math.degrees(math.atan2(-dx, dz))), 'pitch_deg': 0})
            s.call('minecraft_press_inputs', {'keys': ['forward'], 'ticks': 1}) if dx*dx+dz*dz > .15**2 else s.call('minecraft_wait_ticks', {'ticks': 1})
        else:
            s.call('minecraft_wait_for_state_change', {'observed_version': state['state_version'], 'timeout_seconds': min(60, deadline-time.monotonic())})
        state = s.observe()
    assert survival._count(state, survival.DIRT) == 1
    return [x, y, z], state

s = Session(OUT / 'player-world', SERVER, EXAMPLE)
phase = 'player_path'
try:
    s.start_server('server-first')
    s.start_client()
    first = s.observe()
    s.call('minecraft_press_inputs', {'keys': ['forward'], 'ticks': 2})
    moved = s.observe()
    assert any(abs(first['player'][a]-moved['player'][a]) > .001 for a in ('x', 'z')), 'no ordinary movement'
    probe = survival.run(s.client, s.directory / 'survival', 240)
    assert probe['passed'], probe
    native_client.capture(s.client, s.directory, 'after-break-place')
    extra, before = extra_custody(s, probe['placement_position'])
    s.block(probe['placement_position'], survival.DIRT)
    save(s.directory / 'custody-before.json', before)
    phase = 'restart_custody'
    s.stop_server()
    s.start_server('server-restart')
    s.call('minecraft_connect', {'server_addr': s.address})
    assert s.call('minecraft_wait_for_play', {'timeout_seconds': 180})['in_play']
    s.call('minecraft_wait_for_inventory', {'item_id': survival.DIRT, 'count': 1, 'timeout_seconds': 60})
    after = s.observe()
    assert before['player']['entity_uuid'] == after['player']['entity_uuid'], 'player identity changed'
    assert before['inventory'] == after['inventory'], 'full inventory custody changed'
    mined = [probe['mining_target'][a] for a in ('x', 'y', 'z')]
    for pos in (mined, extra):
        s.block(pos, 'minecraft:air')
    s.block(probe['placement_position'], survival.DIRT)
    save(s.directory / 'custody-after.json', after)
    native_client.capture(s.client, s.directory, 'persisted-after-restart')
    s.stop_server()
    restart_log = (s.directory / 'server-restart.log').read_text(errors='replace')
    recovery = [line for line in restart_log.splitlines() if any(word in line.lower() for word in ('replay', 'recover', 'journal', 'checkpoint'))]
    save(s.directory / 'recovery-log-lines.json', recovery)
    results['player_path'] = {'verdict': 'pass', 'receipt': str(s.directory), 'persisted_blocks': [mined, extra, probe['placement_position']]}
    results['restart_custody'] = {'verdict': 'pass', 'receipt': str(s.directory), 'inventory': after['inventory'], 'wal_scope': 'graceful restart; recovery log retained; no forced-crash replay claimed'}
except Exception as e:
    results[phase] = {'verdict': 'fail', 'error': f'{type(e).__name__}: {e}', 'receipt': str(s.directory)}
    if phase == 'restart_custody':
        results['player_path'] = {'verdict': 'blocked', 'reason': 'ordinary path ran; persistence not proven', 'receipt': str(s.directory)}
    traceback.print_exc()
finally:
    s.close()
    save(OUT / 'result.json', results)

if os.environ.get('RC6_GATES') == 'AB':
    print(json.dumps(results, indent=2), flush=True)
    sys.exit(0 if all(results[name]['verdict'] == 'pass' for name in ('player_path', 'restart_custody')) else 1)

package = OUT / 'package-clean'
package.mkdir()
s = None
try:
    assert not list(package.iterdir())
    with tarfile.open(ARCHIVE) as archive:
        members = archive.getmembers()
        save(OUT / 'archive-members.json', [m.name for m in members])
        archive.extractall(package, filter='data')
    assert sha(package / 'krasnokamen') == identity['server']['sha256']
    (package / 'eula.txt').write_text('eula=true\n')
    s = Session(package, package / 'krasnokamen', package / 'example.toml')
    s.start_server('server-package')
    s.start_client()
    state = s.observe()
    save(package / 'joined-player.json', state)
    native_client.capture(s.client, package, 'package-real-client')
    s.stop_server()
    assert (package / 'world').is_dir()
    results['owner_package'] = {'verdict': 'pass', 'receipt': str(package), 'archive': identity['archive'], 'eula': 'eula=true written before launch; this is not a claim that server enforces an EULA prompt', 'content': 'pre-existing licensed content cache explicitly supplied; no bundled content claimed'}
except Exception as e:
    results['owner_package'] = {'verdict': 'fail', 'error': f'{type(e).__name__}: {e}', 'receipt': str(package)}
    traceback.print_exc()
finally:
    if s:
        s.close()
    save(OUT / 'result.json', results)
print(json.dumps(results, indent=2), flush=True)
sys.exit(0 if all(r['verdict'] == 'pass' for r in results.values()) else 1)
