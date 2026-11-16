#!/usr/bin/env python3
"""Fresh fixed-seed Spain trial using the existing native window capture lifecycle.

Run through Main's mem-run with KRASNOKAMEN_SERVER_BINARY set to Main's pin.
Stock is an explicit, bounded operator-supplied fixture, inserted ONLY into
client-observed empty slots. Cultural buildings are never pasted. Every frame
is an actual-window GPU capture; the one-minute film is 60 such frames at 1 fps.

Fast runs (owner, 2026-11-16): `--scratch /dev/shm` keeps the world and every
output on tmpfs and copies the complete directory to `--out` at the end;
`--server-cpus`/`--client-cpus` pin the server and the client with its Xvfb to
disjoint CPUs; `--frames 12` takes fewer census frames for diagnosis while the
simulation still advances in the same 60 sprints. Acceptance needs 60 frames.
"""
from __future__ import annotations

import argparse
import shutil
import tempfile
from collections import Counter
import hashlib
from itertools import islice
import json
import os
from pathlib import Path
import re
import subprocess
import time
from urllib.request import urlopen

from tools import blueprints
from tools.harness import runtime
from tools.harness.profiles import ProfileContext
from tools.harness.backends import item_merge, native_client, village_autonomy_qa as village
from tools.harness.backends.probe_steps import ProbeSteps
from tools.settlement_chronicle import extract

USERNAME = "SpainCulture"


def bill(name: str, level: int = 1) -> Counter:
    _, levels = blueprints.load(blueprints.DIR / f"{name}.json")
    result = Counter()
    for _, (block, properties) in next(islice(blueprints.diffs(levels), level - 1, None)):
        item, count = blueprints.item_of(block, properties)
        if item:
            result[item] += count
    return result


def dump(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n")


def resident_census(probe: ProbeSteps) -> dict:
    """Page only received villagers while the world and observer are stationary."""
    rows, cursors = [], set()
    cursor = ""
    while True:
        page = probe.call("minecraft_list_entities", {
            "radius": 128, "limit": 8, "entity_type": "minecraft:villager", "cursor": cursor,
        })
        rows.extend(page["entities"])
        following = page.get("next_cursor")
        if following is None:
            if page.get("truncated"):
                raise RuntimeError("resident census truncated without a continuation cursor")
            return {"resident_census": rows, "resident_visible_count": page["visible_count"]}
        if not isinstance(following, str) or following == cursor or following in cursors:
            raise RuntimeError("resident census cursor did not advance")
        cursors.add(following)
        cursor = following


def hud_visible(observed: dict) -> bool:
    # F1 is a session toggle, not a managed client_settings key.
    return any(node.get("id", "").startswith(("hud/", "chat/"))
               for node in observed["ui_nodes"])


def record_census(probe: ProbeSteps, console, run: dict, server_log: Path) -> dict:
    console(f'gamemode creative {USERNAME}')
    listing = village._recent_chat(village._send(probe, "settlement list"), 20)
    line = next((line for line in listing if line.startswith("Settlements:")), None)
    names = line.split(":", 1)[1].strip().rstrip(".").split(", ") if line else []
    towns = []
    audit = server_log.read_text(errors="replace")
    for name in names:
        info = village._recent_chat(village._send(probe, f"settlement info {name}"), 20)
        match = re.search(rf"{re.escape(name)} \| owner=(\S+) anchor=\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\] radius=(\d+) members=(\d+)", "\n".join(info))
        if match is None:
            raise RuntimeError(f"native settlement census unavailable for {name}: {info}")
        owner, x, y, z, radius, population = match.groups()
        if owner == 'autonomous' or ('owner' in run and owner != run['owner']):
            continue
        run['owner'] = owner
        anchor = [int(x), int(y), int(z)]
        console(f"tp {USERNAME} {anchor[0] + 0.5} {anchor[1] + 2} {anchor[2] + 0.5}")
        probe.wait(lambda observed: abs(observed['player']['x'] - anchor[0] - 0.5) < 1 and abs(observed['player']['z'] - anchor[2] - 0.5) < 1, f"census_{name}_position")
        lines = village._recent_chat(village._send(probe, "settlement building list"), 25)
        buildings = village._listed_buildings(lines)
        ids = {building['id'] >> 32 for building in buildings}
        if len(ids) != 1:
            raise RuntimeError(f"cannot establish native settlement identity from certificates: {name}: {lines}")
        sid = ids.pop()
        for building in buildings:
            # Proven native construction only, never a generated certificate.
            completed = re.findall(rf"settlement_building_completed[^\n]*settlement={sid}\b[^\n]*building={building['id']}\b[^\n]*blueprint=(\S+)", audit)
            building['blueprint'] = completed[-1] if completed else None
        residents = resident_census(probe)
        project_ui = village._project_status(probe)
        towns.append({'id': sid, 'name': name, 'owner': None if owner == 'autonomous' else owner,
                      'anchor': anchor, 'radius': int(radius), 'population': int(population), 'buildings': buildings,
                      'info': info, 'building_lines': lines, 'project': village._pending_dialog(project_ui),
                      **residents})
    # Console's time query gives an exact server tick; client clocks interpolate.
    before = server_log.read_text(errors='replace').count('The time is ')
    console('time query gametime')
    reply = item_merge._wait_log(server_log, 'The time is ', before + 1, probe.remaining())
    age = int(re.search(r'The time is (\d+)', reply)[1])
    # /stats tick and simulation age are intentionally kept distinct while frozen.
    with urlopen(run['dashboard'] + '/stats', timeout=10) as response:
        stats = json.load(response)
    tick = stats['simulation_tick']
    if not isinstance(tick, int):
        raise RuntimeError(f'dashboard lacks native tick: {stats.keys()}')
    return {'tick': tick, 'game_time': age, 'settlements': towns, 'stats': stats}


def supply(probe: ProbeSteps, console, positions: list[tuple[int, int, int]], stock: Counter, run: dict) -> None:
    commands = []
    for position in positions:
        console(f'tp {USERNAME} {position[0] + 0.5} {position[1] + 1} {position[2] + 1.5}')
        probe.wait(lambda observed: abs(observed['player']['x'] - position[0] - 0.5) < 1
                   and abs(observed['player']['z'] - position[2] - 1.5) < 1, 'storage_observer_arrived')
        slots = village._open_chest(probe, position)
        free = [slot for slot in range(27) if slot not in slots or slots[slot].get('item_id') in (None, 'minecraft:air')]
        for slot in free:
            if not stock:
                break
            item = sorted(stock)[0]
            count = min(1 if item.endswith('_bed') else 64, stock[item])
            command = f'item replace block {position[0]} {position[1]} {position[2]} container.{slot} with minecraft:{item} {count}'
            commands.append(command)
            stock[item] -= count
            if stock[item] == 0:
                del stock[item]
        probe.call('minecraft_close_screen', {})
    if stock:
        raise RuntimeError(f'bounded fixture lacks client-observed empty slots: {dict(stock)}')
    # All slots were observed while the world is frozen: workers cannot race us.
    for command in commands:
        console(command)
    run.setdefault('material_fixture_commands', []).extend(commands)
    probe.call('minecraft_wait_ticks', {'ticks': 40})


def cpu_count(spec: str) -> int:
    """Number of CPUs in a taskset list such as `6,7,14-15`."""
    total = 0
    for part in spec.split(','):
        low, _, high = part.partition('-')
        total += int(high or low) - int(low) + 1
    return total


def pin(process: subprocess.Popen, cpus: str) -> None:
    """Restrict every thread of a started process; new threads inherit it."""
    subprocess.run(['taskset', '-a', '-p', '-c', cpus, str(process.pid)], check=True, stdout=subprocess.DEVNULL)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--seed', type=int, choices=[100], default=100)
    parser.add_argument('--days', type=int, default=4)
    parser.add_argument('--timeout', type=float, default=3600)
    parser.add_argument('--frames', type=int, default=60, help='census frames; must divide the 60 sprints')
    parser.add_argument('--scratch', type=Path, help='run on this (tmpfs) directory and copy the result to --out')
    parser.add_argument('--server-cpus', help='taskset CPU list for the server')
    parser.add_argument('--client-cpus', help='taskset CPU list for the client and its Xvfb')
    parser.add_argument('--client-max-fps', type=int, help='isolated client settings with vsync off and this FPS cap')
    args = parser.parse_args()
    if args.days < 1 or args.days * 24000 % 60:
        parser.error('days must yield 60 integral simulation intervals')
    if args.frames < 1 or 60 % args.frames:
        parser.error('frames must divide 60')
    every = 60 // args.frames
    final = args.out.resolve()
    if final.exists():
        parser.error(f'{final} already exists')
    if args.scratch is None:
        out = final
        out.mkdir(parents=True)
    else:
        final.parent.mkdir(parents=True, exist_ok=True)
        out = Path(tempfile.mkdtemp(prefix=f'{final.name}-', dir=args.scratch)).resolve()
    binary = Path(os.environ['KRASNOKAMEN_SERVER_BINARY']).resolve()
    if not binary.is_file():
        raise RuntimeError(f'missing Main-pinned server: {binary}')
    with binary.open('rb') as executable:
        sha = hashlib.file_digest(executable, 'sha256').hexdigest()
    run = {'seed': args.seed, 'days_requested': args.days, 'server': {'path': str(binary), 'sha256': sha},
           'snapshots': [], 'frames': [], 'fixture_commands': [], 'status': 'running',
           'frames_requested': args.frames, 'scratch': None if args.scratch is None else str(out),
           'cpus': {'server': args.server_cpus, 'client': args.client_cpus},
           'fixture_disclosure': 'Bounded gifted supplies in real storage; no structure paste, no population grant, no time add.'}
    driver = Path(__file__).resolve()
    run['driver'] = {'path': str(driver), 'sha256': hashlib.sha256(driver.read_bytes()).hexdigest()}
    run['material_fixture_plan'] = {
        'initial': 'Four parent houses through L3, three outpost houses L1, church, fortress, outpost plaza; bread256, dirt128, seeds64, logs128, cobble128.',
        'at_population_16_once': 'Four further parent houses through L3 and bread512, from observed empty slots only.',
        'at_daughter_once': 'One house L2+L3 upgrade bill; bread64, dirt64, seeds32, logs64, cobble64.',
    }
    port, dashboard = runtime.reserve_port(), runtime.reserve_port()
    run['dashboard'] = f'http://127.0.0.1:{dashboard}'
    config = out / 'server.toml'
    config.write_text(village.server_config_text((runtime.REPO_ROOT / 'playable.toml').read_text(), port=port,
        world_dir=str(out / 'world'), plugins_dir=str(out / 'plugins'), seed=args.seed, civilization=False)
        .replace('view_distance = 4', 'view_distance = 12').replace('simulation_distance = 4', 'simulation_distance = 12')
        + f'\n[dashboard]\nenabled = true\nbind_address = "127.0.0.1"\nport = {dashboard}\n')
    (out / 'plugins').mkdir()
    ctx = ProfileContext(out, client='native', windowed=True, timeout_seconds=args.timeout)
    server = client = xvfb = mcp = None
    client_log = None
    log_path = out / 'server.log'
    log_handle = log_path.open('wb')
    telemetry_path = out / 'telemetry.jsonl'
    started = time.monotonic()
    try:
        env = {**os.environ, 'KRASNOKAMEN_HARNESS_LOG_STDOUT': '1', 'RUST_LOG': 'info,settlement_audit=info'}
        command = [str(binary), '--config', str(config)]
        if args.server_cpus:
            command = ['taskset', '-c', args.server_cpus, *command]
        run['server']['command'] = command
        server = runtime.start_server(command, cwd=runtime.REPO_ROOT, env=env, stdin=subprocess.PIPE,
                                      stdout=log_handle, stderr=subprocess.STDOUT, start_new_session=True)
        runtime.wait_port(port, 180, server, log_path=log_path)
        xvfb, display, _ = runtime.start_xvfb(out)
        if args.client_cpus:
            pin(xvfb, args.client_cpus)
            # llvmpipe/lavapipe size their render pools from this, not from affinity.
            os.environ['LP_NUM_THREADS'] = str(cpu_count(args.client_cpus))
        if args.client_max_fps:
            # The client caps frames only with vsync off (solaris-client frame_interval).
            settings = {'vsync': False, 'max_fps': args.client_max_fps}
            dump(out / 'client-settings.json', settings)
            os.environ['KRASNOKAMEN_CLIENT_SETTINGS'] = str(out / 'client-settings.json')
        client, client_log, mcp = native_client.start(ctx, native_client.resolve_binary(ctx), f'127.0.0.1:{port}', USERNAME, 180, display=display)
        if args.client_cpus:
            pin(client, args.client_cpus)
        with ProbeSteps(mcp, out / 'probe', args.timeout, 'spain-trial') as probe:
            probe.call('minecraft_wait_for_play', {'timeout_seconds': 180})
            if args.client_max_fps:
                run['client_settings'] = {key: probe.observe()['client_settings'].get(key) for key in ('vsync', 'max_fps')}
                probe.check('client_fps_cap_applied', run['client_settings'] == settings)
            def console(command: str) -> None:
                run['fixture_commands'].append(command)
                item_merge._console(server, command)
            console(f'gamemode creative {USERNAME}')
            console(f'tp {USERNAME} 120 75 7')
            probe.wait(lambda observed: abs(observed['player']['x'] - 120) < 1, 'village_observer_arrived')
            probe.call('minecraft_wait_ticks', {'ticks': 200})
            names = village._recent_chat(village._send(probe, 'settlement list'), 20)
            probe.check('generated_native_town', any(line.startswith('Settlements:') for line in names))
            claim = village._recent_chat(village._send(probe, 'settlement claim'), 20)
            probe.check('owner_claimed', any(line.startswith('Claimed ') for line in claim))
            console('tick freeze')
            probe.call('minecraft_wait_ticks', {'ticks': 40})
            chest = village._founding_storage(probe, (120, 70, 7))
            run['bootstrap_storage'] = list(chest)
            supply(probe, console, [chest], bill('spanish_plaza'), run)
            order = village._recent_chat(village._send(probe, 'settlement build spanish_plaza'), 20)
            probe.check('plaza_order_admitted', any('spanish_plaza' in line and ('Started' in line or 'Queued' in line) for line in order))
            run['snapshots'].append(record_census(probe, console, run, log_path))
            telemetry_tick = -1
            funded = False
            expansion_funded = False
            supplied_children = set()
            queue = ['spanish_house', 'spanish_church', 'spanish_fortress']
            ordered = 'spanish_plaza'
            for step in range(60):
                run.setdefault('sprints', []).append(item_merge._sprint(server, log_path, args.days * 24000 // 60, probe.remaining(limit=None)))
                with urlopen(run['dashboard'] + f'/telemetry.jsonl?after_tick={max(0, telemetry_tick)}', timeout=20) as response:
                    raw = response.read().decode()
                rows = [json.loads(line) for line in raw.splitlines() if line]
                with telemetry_path.open('a') as telemetry:
                    for row in rows:
                        if row['tick'] > telemetry_tick:
                            telemetry.write(json.dumps(row) + '\n')
                            telemetry_tick = row['tick']
                if (step + 1) % every:
                    continue
                frame = (step + 1) // every - 1
                snapshot = record_census(probe, console, run, log_path)
                run['snapshots'].append(snapshot)
                parent = next(town for town in snapshot['settlements'] if town['owner'] is not None and not town['name'].startswith('colony_'))
                completed = {building.get('blueprint') for building in parent['buildings']}
                if 'spanish_plaza' in completed and not funded:
                    plaza = next(building for building in parent['buildings'] if building.get('blueprint') == 'spanish_plaza')
                    positions = [(plaza['min'][0] + x, plaza['min'][1] + 1, plaza['min'][2] + z) for x, z in [(1, 1), (5, 1), (1, 5), (5, 5)]]
                    stock = Counter()
                    for name in ['spanish_house'] * 7 + ['spanish_church', 'spanish_fortress', 'spanish_plaza']:
                        stock.update(bill(name))
                    for level in (2, 3):
                        stock.update({item: 4 * count for item, count in bill('spanish_house', level).items()})
                    stock.update({'bread': 256, 'dirt': 128, 'wheat_seeds': 64, 'oak_log': 128, 'cobblestone': 128})
                    console(f'gamemode creative {USERNAME}')
                    supply(probe, console, positions + [chest], stock, run)
                    funded = True
                if funded and not expansion_funded and parent['population'] >= 16:
                    plaza = next(building for building in parent['buildings'] if building.get('blueprint') == 'spanish_plaza')
                    positions = [(plaza['min'][0] + x, plaza['min'][1] + 1, plaza['min'][2] + z)
                                 for x, z in [(1, 1), (5, 1), (1, 5), (5, 5)]]
                    stock = Counter()
                    for level in (1, 2, 3):
                        stock.update({item: 4 * count for item, count in bill('spanish_house', level).items()})
                    stock['bread'] += 512
                    console(f'gamemode creative {USERNAME}')
                    supply(probe, console, positions + [chest], stock, run)
                    expansion_funded = True
                children = [town for town in snapshot['settlements'] if town['name'].startswith('colony_')]
                for child in children:
                    if child['id'] in supplied_children:
                        continue
                    storage = next(building for building in child['buildings'] if building['kind'] == 'storage')
                    positions = [(storage['min'][0] + x, storage['min'][1] + 1, storage['min'][2] + z)
                                 for x, z in [(1, 1), (5, 1), (1, 5), (5, 5)]]
                    stock = bill('spanish_house', 2) + bill('spanish_house', 3)
                    stock.update({'bread': 64, 'dirt': 64, 'wheat_seeds': 32, 'oak_log': 64, 'cobblestone': 64})
                    console(f'gamemode creative {USERNAME}')
                    supply(probe, console, positions, stock, run)
                    supplied_children.add(child['id'])
                if ordered in completed and queue:
                    ordered = queue.pop(0)
                    console(f"tp {USERNAME} {parent['anchor'][0] + 0.5} {parent['anchor'][1] + 2} {parent['anchor'][2] + 0.5}")
                    probe.wait(lambda observed: abs(observed['player']['x'] - parent['anchor'][0] - 0.5) < 1
                               and abs(observed['player']['z'] - parent['anchor'][2] - 0.5) < 1, 'order_at_parent')
                    village._send(probe, f'settlement build {ordered}')
                console(f'gamemode spectator {USERNAME}')
                focus = children[0] if children and frame % 2 else parent
                anchor = focus['anchor']
                console(f'tp {USERNAME} {anchor[0] + 30} {anchor[1] + 35} {anchor[2] + 40}')
                probe.wait(lambda observed: abs(observed['player']['y'] - anchor[1] - 35) < 1, 'camera_position')
                probe.call('minecraft_close_screen', {})
                probe.call('minecraft_look', {'yaw_deg': 145, 'pitch_deg': 40})
                hud_was_visible = hud_visible(probe.observe())
                if hud_was_visible:
                    probe.call('minecraft_press_inputs', {'keys': ['key.keyboard.f1'], 'ticks': 1})
                probe.wait(lambda observed: not hud_visible(observed), 'film_hud_hidden')
                shot = native_client.capture(mcp, out, f'frame-{frame:03d}')
                if hud_was_visible:
                    probe.call('minecraft_press_inputs', {'keys': ['key.keyboard.f1'], 'ticks': 1})
                    probe.wait(hud_visible, 'film_hud_restored')
                run['frames'].append({'game_time': snapshot['game_time'], 'settlement': focus['id'], 'capture': shot})
                dump(out / 'run.json', run)
            final = run['snapshots'][-1]
            native_events = log_path.read_text(errors='replace')
            run['culture_completed'] = sorted({building.get('blueprint') for town in final['settlements'] for building in town['buildings'] if building.get('blueprint')})
            probe.check('four_cultural_buildings_completed', all(name in run['culture_completed'] for name in ['spanish_house', 'spanish_church', 'spanish_plaza', 'spanish_fortress']))
            probe.check('daughter_founded', 'settlement_daughter_founded' in native_events)
        run['status'] = 'passed'
    except BaseException as error:
        run['status'], run['error'] = 'failed', f'{type(error).__name__}: {error}'
        raise
    finally:
        try:
            if mcp is not None:
                mcp.close()
            runtime.stop_process(client)
            if client_log is not None:
                client_log.close()
            runtime.stop_process(server, interrupt=True, timeout_seconds=120)
            runtime.stop_process(xvfb)
            log_handle.close()
            run['elapsed_seconds'] = time.monotonic() - started
            run['shutdown'] = {'server': server.returncode if server is not None else None, 'client': client.returncode if client is not None else None}
            run['client'] = ctx.client_evidence
            dump(out / 'run.json', run)
            if server is not None and server.returncode == 0:
                command = [str(binary), '--config', str(config), 'world', 'census', '--source', str(out / 'world'), '--receipt', str(out / 'census.json')]
                with (out / 'census.log').open('wb') as log:
                    result = subprocess.run(command, cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT, check=False, timeout=180)
                run['census'] = {'command': command, 'exit': result.returncode, 'receipt': str(out / 'census.json')}
            if run['snapshots'] and telemetry_path.exists():
                chronicle = extract(run, [json.loads(line) for line in telemetry_path.read_text().splitlines() if line], log_path.read_text(errors='replace'))
                dump(out / 'chronicle.json', chronicle)
                (out / 'captions.txt').write_text('\n'.join(event['caption'] for event in chronicle['events']) + '\n')
            if len(run['frames']) == 60:
                command = ['ffmpeg', '-nostdin', '-y', '-framerate', '1', '-i', str(out / 'screenshots/frame-%03d.png'), '-c:v', 'libx264', '-pix_fmt', 'yuv420p', str(out / 'timelapse.mp4')]
                with (out / 'encode.log').open('wb') as log:
                    subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=180)
            dump(out / 'run.json', run)
        finally:
            if args.scratch is not None:
                # The receipt is the complete copy; the tmpfs run directory is
                # removed only after copytree returned.
                shutil.copytree(out, final, symlinks=True)
                shutil.rmtree(out)


if __name__ == '__main__':
    main()
