"""Server-truth at-risk exposure of dismounted sweep riders, from a take's `server.log`.

Why: the D1 bound of the FinalCampaign runbook (§6b, receipts/FinalCampaign/RUNBOOK.md)
counted "at risk" bodies from the client wire only. That proxy is not the server's exposure
(a target out of reach need not swing, and the wire cannot see the sweep plan, `active`,
`wounded` or `routing`), and its calibration was 0.36/0.18 precision at step level
(receipts/BattleSweepSeat/proxy_calibration.py). The core now logs one bounded line per
exposure episode (`crates/mc-core/src/battle/exposure.rs`): a member that `sweep_drives`
would have driven to the sweep point before the seat fix — alive, steady, active,
untargeted, a wounded-action-free member of a regiment with a sweep plan, and not seated.

usage:
    python3 -m tools.harness.battle_exposure --take yaw-1=RUN_DIR [--take ...] \
        [--positions yaw-1=POSITIONS.json] [--out report.json]

- `RUN_DIR` is a `tools/battle/run-battle.py` take directory; its `server.log` is read.
- `POSITIONS.json` is `wire_jitter.py --positions-out`
  (`{battle_start_tick, ids: {id: [[tick, qx, qy, qz], ...]}}`, quantized 1/4096 blocks).
  Optional: without it the report holds the server-truth numbers only.

Exit status: 0 = report written; 1 = malformed log (the offending line is named); 2 = at least one
take has no exposure records (fail closed: absence of records is not zero exposure, and this tool
never reports a pass). A take that started a run always writes a summary, so a silent take is a
broken or filtered log, not a clean battle.

`complete` judges the log (one summary, agreeing with its lines, nothing suppressed or capped).
`usable_for_d1` additionally needs every counted window joinable: no uncovered wire tail, no
episode without a wire step, no `unsettled` close, no exposure id the wire never carried and no
episode starting before its body's wire dismount. Both are reported per take and aggregated in
`pooled`.

The report never decides the D1 verdict: it states the server-confirmed at-risk bodies and
the D1 steps (> 2 blocks between two packets at most 6 ticks apart) that fall on their
exposure windows, which are the two numbers §6b needs.
"""
from __future__ import annotations

import argparse
import json
import math
import re
import sys
from collections import Counter
from pathlib import Path

# §6b bound: a dismounted step above 2 blocks, packets at most 6 ticks apart.
D1_STEP_BLOCKS = 2.0
D1_MAX_INTERVAL_TICKS = 6
# The core's own per-tick excess flag (`peak` of one server tick); reported, not the verdict.
SERVER_EXCESS_BLOCKS = 2.0
WIRE_QUANTUM = 4096.0

OUTCOMES = ('reseated', 'dead', 'wounded', 'routing', 'inactive', 'targeted', 'plan_end', 'stopped', 'gone',
           'unsettled')
PLANS = ('chase', 'flank')
EPISODE_KEYS = ('id', 'tick', 'ticks', 'motion', 'plan', 'from', 'to', 'travel', 'peak', 'excess', 'outcome')
# Takes recorded before the line carried its own `excess` flag: the summary's authoritative
# `excess_episodes` still counts them, and the per-episode flag is simply absent.
LEGACY_EPISODE_KEYS = tuple(key for key in EPISODE_KEYS if key != 'excess')
SUMMARY_KEYS = ('start', 'episodes', 'bodies', 'member_ticks', 'excess_episodes', 'suppressed', 'capped')
EPISODE_MARK = 'Battle exposure: '
SUMMARY_MARK = 'Battle exposure summary: '


def _vector(text):
    if not (text.startswith('[') and text.endswith(']')):
        raise ValueError(f'not a vector: {text!r}')
    parts = text[1:-1].split(',')
    if len(parts) != 3:
        raise ValueError(f'vector needs 3 components: {text!r}')
    return [float(part) for part in parts]


def _episode(line, where):
    body = line.split(EPISODE_MARK, 1)[1].strip()
    tokens = body.split()
    keys = EPISODE_KEYS if len(tokens) == len(EPISODE_KEYS) else LEGACY_EPISODE_KEYS
    if len(tokens) != len(keys):
        raise ValueError(f'{where}: {len(tokens)} fields, expected {len(EPISODE_KEYS)}: {line.strip()!r}')
    fields = {}
    for expected, token in zip(keys, tokens):
        key, sep, value = token.partition('=')
        if not sep or key != expected:
            raise ValueError(f'{where}: expected field {expected}=, got {token!r}: {line.strip()!r}')
        fields[key] = value
    try:
        motion = fields['motion'].split('-')
        if len(motion) != 2:
            raise ValueError(f'motion must be FIRST-LAST: {fields["motion"]!r}')
        episode = {
            'id': int(fields['id']),
            'tick': int(fields['tick']),
            'ticks': int(fields['ticks']),
            'motion': [int(motion[0]), int(motion[1])],
            'plan': fields['plan'],
            'from': _vector(fields['from']),
            'to': _vector(fields['to']),
            'travel': float(fields['travel']),
            'peak': float(fields['peak']),
            'excess': int(fields['excess']) if 'excess' in fields else None,
            'outcome': fields['outcome'],
        }
    except (TypeError, ValueError) as error:
        raise ValueError(f'{where}: {error}: {line.strip()!r}') from None
    if episode['plan'] not in PLANS:
        raise ValueError(f'{where}: unknown plan {episode["plan"]!r}: {line.strip()!r}')
    if episode['outcome'] not in OUTCOMES:
        raise ValueError(f'{where}: unknown outcome {episode["outcome"]!r}: {line.strip()!r}')
    if episode['excess'] not in (0, 1, None):
        raise ValueError(f'{where}: excess must be 0 or 1: {line.strip()!r}')
    if episode['ticks'] < 1:
        raise ValueError(f'{where}: an episode lasts at least one tick: {line.strip()!r}')
    # The phase contract the core writes: the guards of `tick` cover the motion
    # published under tick+1 .. tick+ticks.
    expected = [episode['tick'] + 1, episode['tick'] + episode['ticks']]
    if episode['motion'] != expected:
        raise ValueError(
            f'{where}: motion {episode["motion"]} is not tick+1..tick+ticks {expected}: {line.strip()!r}')
    return episode


def _summary(line, where):
    body = line.split(SUMMARY_MARK, 1)[1].strip()
    tokens = body.split()
    if len(tokens) != len(SUMMARY_KEYS):
        raise ValueError(f'{where}: {len(tokens)} fields, expected {len(SUMMARY_KEYS)}: {line.strip()!r}')
    fields = {}
    for expected, token in zip(SUMMARY_KEYS, tokens):
        key, sep, value = token.partition('=')
        if not sep or key != expected:
            raise ValueError(f'{where}: expected field {expected}=, got {token!r}: {line.strip()!r}')
        fields[key] = value
    if fields['capped'] not in ('true', 'false'):
        raise ValueError(f'{where}: capped must be true or false: {line.strip()!r}')
    try:
        return {key: int(fields[key]) for key in SUMMARY_KEYS[:-1]} | {'capped': fields['capped'] == 'true'}
    except ValueError as error:
        raise ValueError(f'{where}: {error}: {line.strip()!r}') from None


def parse_log(text, source='server.log'):
    """(episodes, summaries) of one server.log; any malformed exposure line raises."""
    episodes, summaries = [], []
    for number, line in enumerate(text.splitlines(), start=1):
        if SUMMARY_MARK in line:
            summaries.append(_summary(line, f'{source}:{number}'))
        elif EPISODE_MARK in line:
            episodes.append(_episode(line, f'{source}:{number}'))
    return episodes, summaries


def wire_positions(path):
    """id -> [(tick, x, y, z)] in blocks, from `wire_jitter.py --positions-out`."""
    document = json.loads(Path(path).read_text())
    table = {}
    for ident, rows in document.get('ids', {}).items():
        steps = [(int(tick), qx / WIRE_QUANTUM, qy / WIRE_QUANTUM, qz / WIRE_QUANTUM) for tick, qx, qy, qz in rows]
        steps.sort()
        table[int(ident)] = steps
    return table


def episode_wire_steps(episode, steps):
    """Wire steps of the episode's body intersecting its motion window.

    One interval semantics for the whole campaign: the window is the `motion=FIRST-LAST` pair the
    core wrote — the published ticks the exposed motion belongs to — and a step counts when the
    interval between its two consecutive packets intersects that window. So a step that spans the
    window's first tick, a step that spans its last tick, and a long step that *covers a short
    exposure entirely* all count; the report flags which (`spans_start`, `spans_end`,
    `covers_window`), and nothing is filtered out. `d1_step` applies the §6b bound to the step.
    """
    first, last = episode['motion']
    inside = []
    for (t0, x0, _, z0), (t1, x1, _, z1) in zip(steps, steps[1:]):
        # The motion of a packet pair is (t0, t1]: it counts while t1 reaches the window's
        # first tick and t0 is still before its last one. A pair that merely touches the
        # right edge (t0 == last) is motion after the window.
        if t1 < first or t0 >= last:
            continue
        blocks = math.hypot(x1 - x0, z1 - z0)
        interval = t1 - t0
        inside.append({
            't0': t0,
            't1': t1,
            'blocks': round(blocks, 3),
            'interval': interval,
            'spans_start': t0 < first,
            'spans_end': t1 > last,
            'covers_window': t0 <= first and t1 >= last,
            'right_edge_touch': t0 + 1 == last,
            'd1_step': blocks > D1_STEP_BLOCKS and 0 < interval <= D1_MAX_INTERVAL_TICKS,
        })
    return inside


def fling_dismounts(path):
    """{take: {id: absolute tick of the first real dismount}}, from `fling_analysis.py --out`.

    A `left_view` (the body left this client's view) is not a dismount, so it is skipped. The
    event and the wire must agree that a body was dismounted at all: an exposure episode without
    one would be a phantom, and an episode that starts before its body's dismount would mean the
    two tick counters are not the ones the campaign joined.
    """
    document = json.loads(Path(path).read_text())
    table = {}
    for name, take in document.items():
        base = take.get('battle_start_tick')
        if base is None:
            raise ValueError(f'{path}: take {name!r} has no battle_start_tick')
        rows = {}
        for rel, ident, cause in take.get('dismounts', []):
            if cause == 'left_view':
                continue
            rows.setdefault(int(ident), int(base) + int(rel))
        table[name] = rows
    return table


def analyse_take(name, run_dir, positions_path=None, dismounts=None, fling_take=None):
    run_dir = Path(run_dir)
    log = run_dir / 'server.log'
    if not log.is_file():
        raise SystemExit(f'{name}: {log} is missing (a take directory has server.log)')
    episodes, summaries = parse_log(log.read_text(errors='replace'), str(log))
    wire = wire_positions(positions_path) if positions_path else None
    bodies = sorted({episode['id'] for episode in episodes})
    take = {
        'name': name,
        'run_dir': str(run_dir),
        'server_log': str(log),
        'positions': str(positions_path) if positions_path else None,
        'records': bool(episodes or summaries),
        'episodes': len(episodes),
        'bodies_at_risk': len(bodies),
        'exposed_member_ticks': sum(episode['ticks'] for episode in episodes),
        'outcomes': dict(Counter(episode['outcome'] for episode in episodes)),
        'plans': dict(Counter(episode['plan'] for episode in episodes)),
        # The core's own excess: per line when the line carries the flag (exact peak on the
        # server), else its authoritative summary counter. Never re-derived from the
        # three-decimal `peak` printed beside it.
        'server_excess_episodes': (
            sum(episode['excess'] for episode in episodes)
            if episodes and all(episode['excess'] is not None for episode in episodes)
            else sum(summary['excess_episodes'] for summary in summaries)
        ),
        'excess_source': (
            'lines' if episodes and all(episode['excess'] is not None for episode in episodes)
            else 'summary (legacy lines without the flag)'
        ),
        'max_peak_blocks_per_tick': round(max((episode['peak'] for episode in episodes), default=0.0), 3),
        'max_travel_blocks': round(max((episode['travel'] for episode in episodes), default=0.0), 3),
        'summaries': summaries,
        'runs': sorted(summary['start'] for summary in summaries),
        'suppressed': sum(summary['suppressed'] for summary in summaries),
        'capped': any(summary['capped'] for summary in summaries),
        'reported_episodes': sum(summary['episodes'] for summary in summaries),
        'reported_bodies': sum(summary['bodies'] for summary in summaries),
        'reported_member_ticks': sum(summary['member_ticks'] for summary in summaries),
        'episode_rows': episodes,
        # The log's own burst: how many episode lines closed on the same published tick. The
        # logger is synchronous inside the tick, so this is the measurement the cost claim needs.
        'max_lines_per_close_tick': max(
            (count for count in Counter(episode['motion'][1] + 1 for episode in episodes).values()),
            default=0,
        ),
    }
    # Fail-closed completeness is about the LOG: one summary, the summary agreeing with the lines
    # in it, nothing suppressed or capped, and no episode from a run the summary does not cover.
    # Wire coverage (a body that left the view, a recorder that stopped before the last battle
    # ticks) is reported per episode instead: it cannot turn a complete log into a count, but it
    # also cannot be allowed to read as "no exposure".
    reasons = []
    if not summaries:
        reasons.append('no summary line')
    if len(summaries) > 1:
        reasons.append(f'{len(summaries)} summaries (run resumed or restarted)')
    if episodes and take['runs'] and take['runs'][-1] > min(e['tick'] for e in episodes):
        reasons.append('an episode precedes the summary run start')
    if take['reported_episodes'] != len(episodes):
        reasons.append(f"summary episodes {take['reported_episodes']} != {len(episodes)} lines")
    if take['suppressed']:
        reasons.append(f"{take['suppressed']} suppressed episodes")
    if take['capped']:
        reasons.append('capped')
    if wire is not None:
        last_wire = max((tick for rows in wire.values() for tick, *_ in rows), default=None)
        last_window = max((episode['motion'][1] for episode in episodes), default=None)
        take['coverage'] = {
            'last_recorded_tick': last_wire,
            'last_motion_window_end': last_window,
            'tail_uncovered_ticks': max(0, (last_window or 0) - (last_wire or 0)),
        }
        if not wire:
            reasons.append('the take carries no wire positions at all')
    take['incomplete'] = reasons
    take['complete'] = not reasons
    if wire is not None:
        rows, d1_bodies, d1_steps, uncorrelated, tail = [], set(), 0, [], 0
        for episode in episodes:
            steps = episode_wire_steps(episode, wire.get(episode['id'], ()))
            if not steps:
                uncorrelated.append(episode['id'])
            last_wire = take['coverage']['last_recorded_tick']
            if last_wire is not None and episode['motion'][1] > last_wire:
                tail += 1
            fired = [step for step in steps if step['d1_step']]
            d1_steps += len(fired)
            if fired:
                d1_bodies.add(episode['id'])
            rows.append({'episode': episode, 'wire_steps': steps, 'd1_steps': len(fired)})
        take['wire'] = {
            'bodies_with_d1_step': len(d1_bodies),
            'd1_steps': d1_steps,
            'episodes_without_wire_step': len(uncorrelated),
            'episodes_past_the_last_packet': tail,
            'episode_rows': rows,
        }
    if dismounts is not None:
        exposed = {episode['id'] for episode in episodes}
        without = sorted(exposed - set(dismounts))
        known = set(wire) if wire is not None else set()
        absent = [ident for ident in without if wire is not None and ident not in known]
        early = [
            episode for episode in episodes
            if episode['id'] in dismounts and episode['motion'][0] < dismounts[episode['id']]
        ]
        take['wire_dismounts'] = {
            'real_dismounts': len(dismounts),
            'exposed_bodies': len(exposed),
            'exposed_of_dismounted': len(exposed & set(dismounts)),
            'exposure_without_wire_dismount': without,
            'exposure_absent_from_wire': absent,
            'episodes_starting_before_dismount': len(early),
        }
        # An id the wire never carried is a real disagreement; an id whose dismount happened
        # before the recording began is a coverage note. Both block usability (decided below);
        # only the first also breaks the log's own completeness.
        if absent:
            take['incomplete'].append('exposure absent from the wire')
    coverage_gaps = []
    if not take['complete']:
        coverage_gaps.extend(take['incomplete'])
    if wire is not None:
        tail_ticks = take['coverage']['tail_uncovered_ticks']
        if tail_ticks > 0:
            coverage_gaps.append(
                f"uncovered wire tail: {tail_ticks} ticks past the last packet "
                f"({take['wire']['episodes_past_the_last_packet']} episodes)")
        if take['wire']['episodes_without_wire_step']:
            coverage_gaps.append(f"{take['wire']['episodes_without_wire_step']} episodes without a wire step")
    unsettled = take['outcomes'].get('unsettled', 0)
    if unsettled:
        coverage_gaps.append(f"{unsettled} episodes closed without a final sample (unsettled)")
    if dismounts is not None:
        if take['wire_dismounts']['exposure_absent_from_wire']:
            coverage_gaps.append('exposure absent from the wire')
        if take['wire_dismounts']['episodes_starting_before_dismount']:
            coverage_gaps.append(
                f"{take['wire_dismounts']['episodes_starting_before_dismount']} episodes start before"
                ' their wire dismount')
    take['coverage_gaps'] = coverage_gaps
    take['usable_for_d1'] = not coverage_gaps
    if fling_take is not None:
        # D1 is a wire bound over ALL dismounted steps of the take; the exposure filter never
        # hides one. `fling_analysis.py` counts them, so carry its number as it stands.
        take['d1'] = {
            'dismounted_steps': fling_take.get('steps', {}).get('dismounted'),
            'dismounted_over_2': len(fling_take.get('dismounted_over_2', [])),
            'dismounted_over_2_spanning_dismount': fling_take.get('dismounted_over_2_spanning_dismount'),
        }
    return take


def pooled(takes):
    return {
        'takes': [take['name'] for take in takes],
        'records': any(take['records'] for take in takes),
        'bodies_at_risk': sum(take['bodies_at_risk'] for take in takes),
        'exposed_member_ticks': sum(take['exposed_member_ticks'] for take in takes),
        'server_excess_episodes': sum(take['server_excess_episodes'] for take in takes),
        'd1_steps': sum(take.get('wire', {}).get('d1_steps', 0) for take in takes),
        'bodies_with_d1_step': sum(take.get('wire', {}).get('bodies_with_d1_step', 0) for take in takes),
        'capped': any(take['capped'] for take in takes),
        'complete': all(take['complete'] for take in takes),
        'usable_for_d1': all(take['usable_for_d1'] for take in takes),
        'unusable': [take['name'] for take in takes if not take['usable_for_d1']],
        'exposure_without_wire_dismount': sum(
            len(take.get('wire_dismounts', {}).get('exposure_without_wire_dismount', ())) for take in takes),
        'episodes_starting_before_dismount': sum(
            take.get('wire_dismounts', {}).get('episodes_starting_before_dismount', 0) for take in takes),
    }


def paired(values, what):
    """`NAME=VALUE` arguments into an ordered {name: value} mapping, refusing duplicates."""
    result = {}
    for value in values or ():
        name, sep, rest = value.partition('=')
        if not sep or not name or not rest:
            raise SystemExit(f'--{what} {value!r}: expected NAME=PATH')
        if name in result:
            raise SystemExit(f'--{what}: duplicate take {name!r}')
        result[name] = rest
    return result


def fling_takes(path):
    """{take: fling_analysis.py take object}, for the wire-side D1 count."""
    return json.loads(Path(path).read_text())


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--take', action='append', required=True, metavar='NAME=RUN_DIR',
                        help='a take directory (its server.log is read); repeatable')
    parser.add_argument('--positions', action='append', metavar='NAME=JSON',
                        help='wire positions of that take (wire_jitter.py --positions-out); repeatable')
    parser.add_argument('--fling-json', action='append', metavar='NAME=JSON',
                        help='fling_analysis.py --out for that take: its real dismounts and its D1 count '
                             'are cross-checked against the exposure episodes; repeatable')
    parser.add_argument('--out', type=Path, help='write the JSON report here')
    arguments = parser.parse_args(argv)
    takes_spec, positions = paired(arguments.take, 'take'), paired(arguments.positions, 'positions')
    fling = paired(arguments.fling_json, 'fling-json')
    unknown = sorted((set(positions) | set(fling)) - set(takes_spec))
    if unknown:
        raise SystemExit(f'--positions/--fling-json name unknown take(s): {", ".join(unknown)}')
    dismounts, fling_rows = {}, {}
    for name, path in fling.items():
        table = fling_dismounts(path)
        rows = fling_takes(path)
        if name not in table:
            raise SystemExit(f'--fling-json {path}: no take named {name!r} ({", ".join(table)})')
        dismounts[name] = table[name]
        fling_rows[name] = rows[name]
    takes = [
        analyse_take(name, run_dir, positions.get(name), dismounts.get(name), fling_rows.get(name))
        for name, run_dir in takes_spec.items()
    ]
    report = {'takes': takes, 'pooled': pooled(takes)}
    text = json.dumps(report, indent=1)
    if arguments.out:
        arguments.out.write_text(text)
    for take in takes:
        wire = take.get('wire')
        extra = ''
        if wire is not None:
            extra = (f"; wire D1 steps {wire['d1_steps']} on {wire['bodies_with_d1_step']} bodies"
                     f" ({wire['episodes_without_wire_step']} episodes without a wire step)")
        d1 = take.get('d1')
        if d1 is not None:
            extra += (f"; all dismounted steps {d1['dismounted_steps']}, over 2 blocks "
                      f"{d1['dismounted_over_2']} (independent of the exposure filter)")
        dismounts = take.get('wire_dismounts')
        if dismounts is not None:
            extra += (f"; of {dismounts['real_dismounts']} wire dismounts {dismounts['exposed_of_dismounted']}"
                      f" are exposed, {len(dismounts['exposure_without_wire_dismount'])} exposed bodies have"
                      f" no wire dismount, {dismounts['episodes_starting_before_dismount']} episodes start"
                      f" before their dismount")
            if dismounts['exposure_without_wire_dismount']:
                extra += f" {dismounts['exposure_without_wire_dismount']}"
        print(f"{take['name']}: {take['episodes']} episodes, {take['bodies_at_risk']} at-risk bodies, "
              f"{take['exposed_member_ticks']} member-ticks, server excess episodes "
              f"{take['server_excess_episodes']} (peak {take['max_peak_blocks_per_tick']} blocks/tick, "
              f"max {take['max_lines_per_close_tick']} lines on one close tick)"
              f"{extra}; outcomes {take['outcomes']}"
              f"{'; CAPPED' if take['capped'] else ''}"
              f"{' ' if take['complete'] else '; INCOMPLETE: ' + '; '.join(take['incomplete'])}")
        if not take['usable_for_d1']:
            print(f"{take['name']}: UNUSABLE for a bound: " + '; '.join(take['coverage_gaps']),
                  file=sys.stderr)
    pool = report['pooled']
    print(f"pooled: {pool['bodies_at_risk']} at-risk bodies, {pool['exposed_member_ticks']} member-ticks, "
          f"server excess episodes {pool['server_excess_episodes']}, D1 steps {pool['d1_steps']}")
    silent = [take['name'] for take in takes if not take['records']]
    if silent:
        print(f"no exposure records in {', '.join(silent)}: absence is not zero exposure (fail closed)",
              file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main())
