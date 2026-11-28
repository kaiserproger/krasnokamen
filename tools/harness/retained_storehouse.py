"""Bounded D54 retained-stall gate; never claims natural-growth acceptance."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
from urllib.request import urlopen

from . import runtime
from .backends import item_merge, native_client, village_autonomy_qa as village
from .backends.probe_steps import ProbeSteps
from .gathering_audit import audit_growth
from .growth_accounting import audit_world, custody, live_carries, reconcile
from .growth_io import dump, parent_custody, probe_world
from .journal_witness import JournalWitness
from .profiles import ProfileContext
from tools.showcase import spain_trial as trial


DAY_TICKS = 24_000
# Adult schedule: Activity::Work at day phase 2_000..8_999 (villager/brain.rs);
# the builder's working() also requires that phase (villager/building.rs).
WORK_SHIFT_START, WORK_SHIFT_END = 2_000, 9_000
# R5-bootstrap-1909d54a81e1-36000 placed 61 storehouse units in 1_488 running
# ticks during the 272630..274118 burst (0.0410 per tick). The first admitted
# shift placed only three units while the builder collected materials, so the
# window doubles the recorded burst requirement for a repeat of that warm-up.
RECORDED_PLACEMENTS, RECORDED_TICKS = 61, 1_488
PLACEMENT_MARGIN = 2
REMAINING_PLACEMENTS = 78  # certified cursor 64 of 142
OFF_SHIFT_BATCH = 2_000
WORK_BATCH, WORK_BATCH_AFTER_PROGRESS = 20, 200
MAX_RUNNING_TICKS = 24_000


def day_phase(day_time):
    return day_time % DAY_TICKS


def in_work_shift(day_time):
    """Builders and the donor are Rest outside the adult work schedule."""
    return WORK_SHIFT_START <= day_phase(day_time) < WORK_SHIFT_END


def next_work_shift(day_time):
    """First day time at or after `day_time` inside the work schedule."""
    phase = day_phase(day_time)
    if phase < WORK_SHIFT_START:
        return day_time - phase + WORK_SHIFT_START
    return day_time - phase + DAY_TICKS + WORK_SHIFT_START


def advance_work_window(day_time, work_ticks):
    """Day time after `work_ticks` ticks that can actually be spent working.

    Rest gaps between shifts are crossed without consuming the work budget:
    only ticks inside the adult work schedule count toward placements.
    """
    while work_ticks:
        if in_work_shift(day_time):
            step = min(work_ticks, WORK_SHIFT_END - day_phase(day_time))
            day_time += step
            work_ticks -= step
        else:
            day_time = next_work_shift(day_time)
    return day_time


def placement_work_ticks(placements, margin=PLACEMENT_MARGIN):
    """Work-shift ticks needed for `placements` at the recorded R5 pace."""
    return math.ceil(placements * RECORDED_TICKS / RECORDED_PLACEMENTS) * margin


def run_window(checkpoint_day_time, remaining_placements=REMAINING_PLACEMENTS, cap_ticks=MAX_RUNNING_TICKS):
    """Running-tick budget: to the next work shift plus a placement work window.

    The checkpoint's day phase alone decides the budget, so an off-shift
    continuation can never exhaust the cap before the builders wake.
    """
    if cap_ticks <= 0:
        raise ValueError('the running-tick cap must be positive')
    work_ticks = placement_work_ticks(remaining_placements)
    shift_start = next_work_shift(checkpoint_day_time)
    end = advance_work_window(shift_start, work_ticks)
    total = end - checkpoint_day_time
    if total > cap_ticks:
        raise ValueError(f'derived window {total} running ticks exceeds the explicit cap {cap_ticks}')
    return {'checkpoint_day_time': checkpoint_day_time,
            'checkpoint_day_phase': day_phase(checkpoint_day_time),
            'next_shift_start': shift_start, 'gap_ticks': shift_start - checkpoint_day_time,
            'remaining_placements': remaining_placements,
            'placement_pace_per_tick': RECORDED_PLACEMENTS / RECORDED_TICKS,
            'pace_source': 'R5-bootstrap-1909d54a81e1-36000 ticks 272630..274118',
            'placement_margin': PLACEMENT_MARGIN, 'work_ticks': work_ticks,
            'window_ticks': total, 'cap_ticks': cap_ticks}


def next_batch(day_time, remaining, progressed):
    """(running ticks, full_census) for the next sprint at this day time.

    Off shift one large batch runs before only a cheap observation, clamped to
    the shift boundary so the boundary itself gets the full frozen census.
    Inside the shift the fine cadence returns and every batch ends in a full
    census; the progress switch only coarsens batches that already saw it.
    """
    if remaining <= 0:
        return 0, False
    if in_work_shift(day_time):
        size = WORK_BATCH_AFTER_PROGRESS if progressed else WORK_BATCH
        return min(size, remaining), True
    boundary = next_work_shift(day_time) - day_time
    size = min(OFF_SHIFT_BATCH, remaining, boundary)
    return size, size == boundary


def observe_cursor(steps, console, log_path, dashboard, anchor, target):
    """Cheap off-shift observation: exact server time, storehouse cursor, WAL frontier.

    Item custody itself needs no live counter here: the receipt's audits read it
    from the published journal, and the frontier recorded with each observation
    proves which published frames back the next audit.
    """
    before = log_path.read_text(errors='replace').count('The time is ')
    console('time query gametime')
    reply = item_merge._wait_log(log_path, 'The time is ', before + 1, steps.remaining())
    day_time = int(re.search(r'The time is (\d+)', reply)[1])
    console(f'tp {trial.USERNAME} {anchor[0] + 0.5} {anchor[1] + 2} {anchor[2] + 0.5}')
    steps.wait(lambda observed: abs(observed['player']['x'] - anchor[0] - 0.5) < 1
               and abs(observed['player']['z'] - anchor[2] - 0.5) < 1, 'cursor_position')
    projects = village._pending_dialog(village._project_status(steps), clipped_ok=True)['projects']
    cursor = next((p for p in projects if p['blueprint'] == 'storage' and p['origin'] == target), None)
    with urlopen(dashboard + '/stats', timeout=10) as response:
        tick = json.load(response)['simulation_tick']
    return {'tick': tick, 'game_time': day_time, 'day_phase': day_phase(day_time),
            'cursor': cursor['done'] if cursor else None,
            'cursor_total': cursor['total'] if cursor else None}


def construction_acceptance(balance, construction):
    """Scope the retained-stall proof without changing the ALL-item report."""
    kinds = construction['touched_item_kinds']
    residual = {kind: balance['per_item'].get(kind, {}).get('unexplained', 0)
                for kind in kinds if balance['per_item'].get(kind, {}).get('unexplained', 0)}
    ground = construction['construction_kinds_on_ground']
    return {'status': 'FAIL' if residual or ground else 'PASS', 'scope': 'retained_stall_construction_only',
            'item_kinds': kinds, 'unexplained_delta': residual, 'construction_kinds_on_ground': ground,
            'all_item_status': balance['status'], 'all_item_unexplained_delta': balance['unexplained_delta']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('server', 'auditor', 'source', 'fixture-root', 'out'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--cap-ticks', type=int, default=MAX_RUNNING_TICKS)
    parser.add_argument('--wall-seconds', type=int, default=3600)
    args = parser.parse_args()
    if not 1 <= args.cap_ticks <= MAX_RUNNING_TICKS or args.wall_seconds <= 0:
        parser.error(f'retained-state authorization permits at most {MAX_RUNNING_TICKS} running ticks')
    binary, out, source = args.server.resolve(), args.out.resolve(), args.source.resolve()
    out.mkdir(parents=True, exist_ok=False)
    shutil.copytree(source, out / 'world')
    shutil.copytree(args.fixture_root / 'plugins', out / 'plugins')
    baseline = json.loads((args.fixture_root / 'after-audit.json').read_text())
    positions = {tuple(bank['position']) for bank in baseline['banks']}
    original = json.loads((args.fixture_root / 'run.json').read_text())
    marker = source / 'checkpoint.marker'
    marker_hash = hashlib.sha256(marker.read_bytes()).hexdigest()
    port, dashboard = runtime.reserve_port(), runtime.reserve_port()
    config = out / 'server.toml'
    text = village.server_config_text((runtime.REPO_ROOT / 'playable.toml').read_text(), port=port,
        world_dir=str(out / 'world'), plugins_dir=str(out / 'plugins'), seed=100, civilization=False)
    text = text.replace('view_distance = 4', 'view_distance = 12').replace('simulation_distance = 4', 'simulation_distance = 12')
    text += f'\n[dashboard]\nenabled = true\nbind_address = "127.0.0.1"\nport = {dashboard}\n'
    text += '\n[test_acceleration]\ngathering_yield = 4.0\nconstruction_speed = 4.0\nbirth_chance = 8.0\nmigration_rate = 1.0\n'
    config.write_text(text)
    registry = json.loads((runtime.REPO_ROOT / 'data/vanilla/reports/registries.json').read_text())
    names = {entry['protocol_id']: name for name, entry in registry['minecraft:item']['entries'].items()}
    run = {'status': 'running', 'scope': 'retained_stall_only_not_full_D54', 'owner': original['owner'],
           'source': str(source), 'snapshots': [], 'fixture_commands': [], 'seen_projects': {},
           'dashboard': f'http://127.0.0.1:{dashboard}', 'checks': {}, 'requested_running_ticks': 0,
           'disclosure': 'Agent-run graphical native client; immutable retained save COPY; no gifts/orders/time-add/disabled simulation.',
           'provenance': {str(path): hashlib.sha256(path.read_bytes()).hexdigest()
                          for path in (binary, args.auditor, Path(__file__),
                              Path(__file__).with_name('gathering_audit.py'),
                              Path(__file__).with_name('gathering_natural.py'),
                              Path(__file__).with_name('growth_accounting.py'),
                              Path(__file__).with_name('journal_witness.py'))}}
    before = audit_world(args.auditor, out / 'world', out / 'plugins', runtime.REPO_ROOT / 'data/vanilla', positions, out, 'before')
    if before['checkpoint_tick'] != 383732 or before['checkpoint_tick'] != baseline['checkpoint_tick']:
        raise ValueError('not the approved retained checkpoint 383732')
    # The off-shift failure of R6b came from a fixed budget that ended before the
    # next work shift; the budget now starts from the audited checkpoint phase.
    if 'climate' not in before['globals']['checkpoint']:
        raise ValueError('the checkpoint audit lacks the climate day time that budgets the window')
    run['window'] = run_window(before['globals']['checkpoint']['climate']['day_time'],
                               REMAINING_PLACEMENTS, args.cap_ticks)
    run['before_custody'] = custody(before, names)
    parents = {town['id'] for town in original['snapshots'][-1]['settlements']}
    run['durable_parent_uuids'] = parent_custody(binary, config, out, parents)
    required = {'minecraft:dirt'}
    for record in before['structure_postimages'].values():
        if 'snapshot' in record:
            required.update(material['resource'] for stage in record['snapshot']['stages'] for material in stage['materials'])
            required.update(names[int(item)] for item in record.get('crafting_delta', {}))
    ctx = ProfileContext(out, client='native', windowed=True, timeout_seconds=args.wall_seconds + 600)
    server = client = xvfb = mcp = client_log = witness = None
    log_path = out / 'server.log'
    log = trial.PhaseLog(log_path)
    command = [str(binary), '--config', str(config), '--start-frozen']
    env = {**os.environ, 'KRASNOKAMEN_HARNESS_LOG_STDOUT': '1', 'MC_CORE_PHASE_LOG': '1',
           'RUST_LOG': 'info,settlement_audit=info,mc_core::settlements::construction=debug'}
    started = time.monotonic()
    dump(out / 'run.json', run)
    try:
        def start_server():
            result = runtime.start_server(command, cwd=runtime.REPO_ROOT, env=env, stdin=subprocess.PIPE,
                                          stdout=log, stderr=log.stderr, start_new_session=True)
            runtime.wait_port(port, 180, result, log_path=log_path)
            return result
        server = start_server()
        witness = JournalWitness(out / 'world/journal', out / 'journal-witness')
        settings = out / 'client-settings.json'
        dump(settings, {'vsync': False, 'max_fps': 30})
        os.environ['KRASNOKAMEN_CLIENT_SETTINGS'] = str(settings)
        xvfb, display, _ = runtime.start_xvfb(out)
        client, client_log, mcp = native_client.start(ctx, native_client.resolve_binary(ctx),
            f'127.0.0.1:{port}', trial.USERNAME, 180, display=display)
        with ProbeSteps(mcp, out / 'probe', args.wall_seconds, 'd54-retained-storehouse') as steps:
            steps.call('minecraft_wait_for_play', {'timeout_seconds': 180})
            def console(command):
                run['fixture_commands'].append(command)
                item_merge._console(server, command)
            def snapshot():
                value = trial.record_census(steps, console, run, log_path)
                run['snapshots'].append(value)
                for town in value['settlements']:
                    for building in town['buildings']:
                        if building['kind'] == 'storage':
                            positions.update((x, y, z)
                                for x in range(building['min'][0], building['max'][0] + 1)
                                for y in range(building['min'][1], building['max'][1] + 1)
                                for z in range(building['min'][2], building['max'][2] + 1))
                dump(out / 'run.json', run)
                return value
            def carry(value):
                uuids = set(run['durable_parent_uuids'])
                for town in value['settlements']:
                    uuids.update(resident['entity_uuid'] for resident in town['resident_census'])
                replies = {}
                for uuid in sorted(uuids):
                    needle = 'Villager has the following entity data: '
                    count = log_path.read_text(errors='replace').count(needle)
                    console(f'data get entity {uuid} Inventory')
                    replies[uuid] = item_merge._wait_log(log_path, needle, count + 1, steps.remaining())
                return live_carries(replies)
            first = snapshot()
            initial_carry = carry(first)
            steps.check('initial_live_carry_matches_durable', initial_carry == run['before_custody']['carries'])
            donor = 'd58ae3b6-0d44-6b93-bbc6-b8c74b654a92'
            steps.check('retained_donor_has_four_fences', initial_carry.get(donor, {}).get('minecraft:acacia_fence') == 4)
            full = {(117, 70, 5), (126, 70, -3), (126, 70, 1), (130, 70, -3), (130, 70, 1)}
            steps.check('five_original_banks_full', all(any(tuple(bank['position']) == point
                and len(bank['slots']) == 27 and all(slot and slot['count'] for slot in bank['slots'])
                for bank in before['banks']) for point in full))
            initial_projects = [p for town in first['settlements'] for p in town['project']['projects']]
            target = next(p for p in initial_projects if p['blueprint'] == 'storage' and p['origin'] == [109, 66, 106])
            steps.check('retained_storehouse_cursor_64_of_142', target['done'] == 64 and target['total'] == 142)
            run['initial_capture'] = native_client.capture(mcp, out, 'retained-full-banks')
            initial_stores = {tuple(b['min']) for town in first['settlements'] for b in town['buildings'] if b['kind'] == 'storage'}
            elapsed, restarted, receiving, progressed = 0, False, False, False
            anchor = next(town['anchor'] for town in first['settlements'])
            state = {'tick': first['tick'], 'game_time': first['game_time']}
            while elapsed < run['window']['window_ticks']:
                batch, full = next_batch(state['game_time'], run['window']['window_ticks'] - elapsed, progressed)
                item_merge._sprint(server, log_path, batch, steps.remaining(limit=None))
                elapsed += batch
                run['requested_running_ticks'] = elapsed
                if not full:
                    cheap = observe_cursor(steps, console, log_path, run['dashboard'], anchor, target['origin'])
                    cheap['journal_frontier'] = witness.frontier
                    run['off_shift_observations'] = (run.get('off_shift_observations', []) + [cheap])[-64:]
                    state = cheap
                    if cheap['cursor'] is not None and cheap['cursor'] <= target['done']:
                        continue
                    # A cheap cursor move (or a vanished project) spends a full
                    # census at once, so every verdict input exists at that tick.
                    run['early_progress_observation'] = cheap
                current = snapshot()
                state = {'tick': current['tick'], 'game_time': current['game_time']}
                projects = [p for town in current['settlements'] for p in town['project']['projects']]
                current_target = next((p for p in projects if p['blueprint'] == 'storage' and p['origin'] == target['origin']), None)
                new_stores = [b for town in current['settlements'] for b in town['buildings']
                              if b['kind'] == 'storage' and tuple(b['min']) not in initial_stores]
                if new_stores or current_target and current_target['done'] > target['done']:
                    progressed = True
                if not restarted and progressed:
                    live_before = carry(current)
                    run['restart_capture'] = native_client.capture(mcp, out, 'retained-progress-before-crash')
                    witness.wait_published(current['tick'], steps.remaining())
                    steps.call('minecraft_disconnect')
                    server.kill()
                    server.wait(timeout=120)
                    mid = audit_world(args.auditor, out / 'world', out / 'plugins', runtime.REPO_ROOT / 'data/vanilla', positions, out, 'mid-crash')
                    steps.check('precrash_carry_matches_published_wal', live_before == custody(mid, names)['carries'])
                    server = start_server()
                    steps.call('minecraft_connect', {'server_addr': f'127.0.0.1:{port}'})
                    steps.call('minecraft_wait_for_play', {'timeout_seconds': 180})
                    reopened = snapshot()
                    reopened_projects = [p for town in reopened['settlements'] for p in town['project']['projects']]
                    steps.check('restart_same_game_time', reopened['game_time'] == current['game_time'])
                    steps.check('restart_same_project_cursor', reopened_projects == projects)
                    steps.check('restart_same_live_carry', carry(reopened) == live_before)
                    steps.call('minecraft_disconnect')
                    runtime.stop_process(server, interrupt=True, timeout_seconds=120)
                    steps.check('restart_clean_checkpoint', server.returncode == 0)
                    restored = audit_world(args.auditor, out / 'world', out / 'plugins', runtime.REPO_ROOT / 'data/vanilla', positions, out, 'mid-restored')
                    run['restart_balance'] = {'before': custody(mid, names), 'after': custody(restored, names)}
                    dump(out / 'restart-balance.json', run['restart_balance'])
                    steps.check('restart_all_physical_custody_identical', run['restart_balance']['before'] == run['restart_balance']['after'])
                    server = start_server()
                    steps.call('minecraft_connect', {'server_addr': f'127.0.0.1:{port}'})
                    steps.call('minecraft_wait_for_play', {'timeout_seconds': 180})
                    restarted = True
                if restarted and new_stores:
                    saved = log_path.read_text(errors='replace').count('Saved the game')
                    console('save-all flush')
                    item_merge._wait_log(log_path, 'Saved the game', saved + 1, steps.remaining())
                    witness.wait_published(current['tick'], steps.remaining())
                    observed = audit_world(args.auditor, out / 'world', out / 'plugins', runtime.REPO_ROOT / 'data/vanilla', positions, out,
                                           f'receiving-{current["tick"]}', out / 'journal-witness', before['checkpoint_tick'])
                    balance = audit_growth(before, observed, names, out, binary, config, source, probe_world, reconcile)
                    construction = reconcile(before, observed, names, required)
                    proof = construction_acceptance(balance, construction)
                    dump(out / f'balance-{current["tick"]}.json', balance)
                    run.setdefault('receiving_probes', []).append({'tick': current['tick'], 'construction': proof,
                                                                  'warehouse_proofs': balance['warehouse_proofs']})
                    if proof['status'] == 'PASS' and balance['warehouse_proofs'] and all(p['status'] == 'PASS' for p in balance['warehouse_proofs']):
                        receiving = True
                        break
            steps.check('retained_progress_crash_restart_observed', restarted)
            steps.check('extra_storehouse_certified_receiving_exact_construction_ledger', receiving)
            run['final_capture'] = native_client.capture(mcp, out, 'retained-extra-storehouse')
            run['checks'] = dict(steps.report['assertions'])
            run['status'] = 'functional_proven'
    except BaseException as error:
        run['status'], run['error'] = 'failed', f'{type(error).__name__}: {error}'
        raise
    finally:
        if mcp is not None:
            mcp.close()
        runtime.stop_process(client)
        if client_log is not None:
            client_log.close()
        runtime.stop_process(server, interrupt=True, timeout_seconds=120)
        runtime.stop_process(xvfb)
        log.close()
        run['elapsed_seconds'] = time.monotonic() - started
        run['original_marker_unchanged'] = hashlib.sha256(marker.read_bytes()).hexdigest() == marker_hash
        run['shutdown'] = {'server': server.returncode if server else None, 'client': client.returncode if client else None}
        run['client'] = ctx.client_evidence
        dump(out / 'run.json', run)
        if witness is not None:
            witness.close()
        if server is not None and server.returncode == 0:
            after = audit_world(args.auditor, out / 'world', out / 'plugins', runtime.REPO_ROOT / 'data/vanilla',
                                positions, out, 'after', out / 'journal-witness', before['checkpoint_tick'])
            balance = audit_growth(before, after, names, out, binary, config, source, probe_world, reconcile)
            proof = construction_acceptance(balance, reconcile(before, after, names, required))
            dump(out / 'full-balance.json', balance)
            dump(out / 'construction-balance.json', proof)
            run['all_item_status'], run['construction_status'] = balance['status'], proof['status']
            if run['status'] == 'functional_proven':
                assert run['original_marker_unchanged']
                assert proof['status'] == 'PASS', proof
                assert balance['warehouse_proofs'] and all(p['status'] == 'PASS' for p in balance['warehouse_proofs'])
                run['status'] = 'PASS_RETAINED_STALL_ONLY'
            dump(out / 'run.json', run)


if __name__ == '__main__':
    main()
