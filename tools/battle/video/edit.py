#!/usr/bin/env python3
"""N-shot edit: plan, tick-exact telemetry overlay per output frame, ffmpeg encode.

Reads an edit spec (see README.md), validates every shot against its C3 frames.jsonl and video, joins the overlay
to telemetry by `last_applied_tick` (a tick missing from the shot's telemetry fails the edit), writes edit.frames.jsonl,
plan.json, overlay PNGs and the exact ffmpeg command, then encodes. No wall-clock is read.
"""
import argparse
import json
import pathlib
import shlex
import subprocess
import sys

import edit_compose
import overlay

SPEC_KEYS = {'resolution', 'music', 'texts', 'shots'}
SHOT_KEYS = {'id', 'video', 'frames', 'in', 'out', 'telemetry', 'transition'}
TEXT_KEYS = {'file', 'start', 'end'}


class EditError(ValueError):
    pass


def reject_unknown(what, value, allowed):
    unknown = sorted(set(value) - allowed)
    if unknown:
        raise EditError(f'{what}: unknown keys {unknown}')


def probe_video(path):
    """Return (decoded-frame packet count, r_frame_rate, width, height) of the first video stream."""
    r = subprocess.run(['ffprobe', '-v', 'error', '-count_packets', '-select_streams', 'v:0', '-show_entries',
                        'stream=nb_read_packets,r_frame_rate,width,height', '-of', 'json', str(path)],
                       capture_output=True, text=True, check=True)
    streams = json.loads(r.stdout)['streams']
    if len(streams) != 1:
        raise EditError(f'{path}: expected one video stream, found {len(streams)}')
    s = streams[0]
    return int(s['nb_read_packets']), s['r_frame_rate'], s['width'], s['height']


def read_shot_ticks(path):
    """Per-frame last_applied_tick of a C3 frames.jsonl; frames are numbered from zero and ticks never go back."""
    ticks = []
    for expected, row in enumerate(overlay.rows(path)):
        if row.get('frame') != expected:
            raise EditError(f'{path}: expected frame {expected}, got {row.get("frame")!r}')
        tick = row.get('last_applied_tick')
        if not isinstance(tick, int) or isinstance(tick, bool):
            raise EditError(f'{path}: frame {expected} has no integer last_applied_tick')
        if ticks and tick < ticks[-1]:
            raise EditError(f'{path}: frame {expected} tick {tick} goes back from {ticks[-1]}')
        ticks.append(tick)
    if not ticks:
        raise EditError(f'{path}: no frames')
    return ticks


def resolve(base, value, what):
    path = (base / value) if not pathlib.Path(value).is_absolute() else pathlib.Path(value)
    if not path.is_file():
        raise EditError(f'{what}: {path} is not a file')
    return str(path)


def int_field(what, value, minimum):
    if not isinstance(value, int) or isinstance(value, bool) or value < minimum:
        raise EditError(f'{what}: expected integer >= {minimum}, got {value!r}')
    return value


def build_plan(spec, base, probe=probe_video):
    """Validate the spec and return (plan, per-output-frame rows).

    Shot i shows source frames [in, out). A transition into shot i+1 lasts d frames that lie INSIDE both shots' ranges
    (the last d frames of shot i, the first d of shot i+1), so output length = sum(out - in) - sum(d).
    During a transition output frame j (0-based) takes its tick from the outgoing shot while 2*j < d and from the
    incoming shot afterwards; the other side is kept in the row's `blend`.
    """
    if not isinstance(spec, dict):
        raise EditError('spec must be an object')
    reject_unknown('spec', spec, SPEC_KEYS)
    if spec.get('resolution') not in edit_compose.RESOLUTIONS:
        raise EditError(f'resolution must be one of {sorted(edit_compose.RESOLUTIONS)}')
    if not spec.get('shots'):
        raise EditError('spec needs at least one shot')
    shots, ids = [], set()
    for i, raw in enumerate(spec['shots']):
        where = f'shot {i}'
        reject_unknown(where, raw, SHOT_KEYS)
        sid = raw.get('id')
        if not isinstance(sid, str) or not sid or sid in ids:
            raise EditError(f'{where}: id must be a unique non-empty string')
        ids.add(sid)
        where = f'shot {sid}'
        video, frames_path = resolve(base, raw.get('video', ''), f'{where} video'), resolve(base, raw.get('frames', ''), f'{where} frames')
        ticks = read_shot_ticks(frames_path)
        packets, rate, _, _ = probe(video)
        if rate != '60/1':
            raise EditError(f'{where}: video is {rate} fps, edits need 60/1 (no implicit resampling)')
        if packets != len(ticks):
            raise EditError(f'{where}: video has {packets} frames, {frames_path} has {len(ticks)}')
        first = int_field(f'{where} in', raw.get('in'), 0)
        last = int_field(f'{where} out', raw.get('out'), first + 1)
        if last > len(ticks):
            raise EditError(f'{where}: out {last} is beyond its {len(ticks)} frames')
        telemetry = resolve(base, raw['telemetry'], f'{where} telemetry') if raw.get('telemetry') is not None else None
        transition = raw.get('transition', {'type': 'cut'})
        if not isinstance(transition, dict):
            raise EditError(f'{where}: transition must be an object')
        kind = transition.get('type')
        if kind == 'cut':
            reject_unknown(f'{where} transition', transition, {'type'})
            transition = {'type': 'cut', 'frames': 0}
        elif kind in edit_compose.TRANSITIONS:
            reject_unknown(f'{where} transition', transition, {'type', 'frames'})
            transition = {'type': kind, 'frames': int_field(f'{where} transition frames', transition.get('frames'), 1)}
        else:
            raise EditError(f'{where}: transition type must be cut or one of {list(edit_compose.TRANSITIONS)}')
        shots.append({'id': sid, 'video': video, 'frames': frames_path, 'in': first, 'out': last, 'telemetry': telemetry,
                      'transition': transition, 'ticks': ticks})
    if shots[-1]['transition']['type'] != 'cut':
        raise EditError(f'shot {shots[-1]["id"]}: the last shot has no next shot to transition into')
    for i, shot in enumerate(shots):
        length = shot['out'] - shot['in']
        incoming = shots[i - 1]['transition']['frames'] if i else 0
        if incoming + shot['transition']['frames'] > length:
            raise EditError(f'shot {shot["id"]}: transitions ({incoming} + {shot["transition"]["frames"]} frames) do not fit its {length} frames')
    texts = []
    total = 0
    starts = []
    for i, shot in enumerate(shots):
        starts.append(total)
        total += shot['out'] - shot['in'] - shot['transition']['frames']
    music = resolve(base, spec.get('music', ''), 'music')
    for i, raw in enumerate(spec.get('texts', [])):
        reject_unknown(f'text {i}', raw, TEXT_KEYS)
        start, end = raw.get('start'), raw.get('end')
        if not all(isinstance(v, (int, float)) and not isinstance(v, bool) for v in (start, end)) or not 0 <= start < end <= total / edit_compose.FPS:
            raise EditError(f'text {i}: need 0 <= start < end <= {total / edit_compose.FPS}')
        texts.append({'file': resolve(base, raw.get('file', ''), f'text {i}'), 'start': start, 'end': end})
    telemetry_cache = {}
    rows = []
    for f in range(total):
        i = max(k for k in range(len(shots)) if starts[k] <= f)
        j = f - starts[i]
        incoming = shots[i - 1]['transition']['frames'] if i else 0
        in_transition = j < incoming
        if in_transition and 2 * j < incoming:
            owner, partner = i - 1, i
        else:
            owner, partner = i, (i - 1 if in_transition else None)
        src = {k: shots[k]['in'] + f - starts[k] for k in (owner, partner) if k is not None}
        shot = shots[owner]
        row = {'frame': f, 'shot': shot['id'], 'source_frame': src[owner], 'last_applied_tick': shot['ticks'][src[owner]],
               'overlay': shot['telemetry'] is not None, 'telemetry': shot['telemetry']}
        if partner is not None:
            row['blend'] = {'shot': shots[partner]['id'], 'source_frame': src[partner], 'last_applied_tick': shots[partner]['ticks'][src[partner]],
                            'progress': f'{j}/{incoming}'}
        if row['overlay']:
            if shot['telemetry'] not in telemetry_cache:
                telemetry_cache[shot['telemetry']] = overlay.load(shot['telemetry'])
            if row['last_applied_tick'] not in telemetry_cache[shot['telemetry']]:
                raise EditError(f'frame {f} (shot {shot["id"]} source frame {src[owner]}): tick {row["last_applied_tick"]} missing from telemetry')
        rows.append(row)
    plan = {'resolution': spec['resolution'], 'music': music, 'texts': texts, 'frames': total,
            'shots': [{k: s[k] for k in ('id', 'video', 'frames', 'in', 'out', 'telemetry', 'transition')} for s in shots]}
    return plan, rows, telemetry_cache


def render_overlays(rows, telemetry_cache, out):
    """One PNG per output frame; identical (telemetry, tick) pairs are rendered once. Shots without telemetry get transparent frames."""
    out.mkdir(parents=True, exist_ok=False)
    blank = overlay.png(bytearray(overlay.W * overlay.H * 4))
    pngs = {}
    for row in rows:
        if row['overlay']:
            key = (row['telemetry'], row['last_applied_tick'])
            if key not in pngs:
                pngs[key] = overlay.render(*telemetry_cache[row['telemetry']][row['last_applied_tick']])
            data = pngs[key]
        else:
            data = blank
        (out / f'overlay-{row["frame"]:06d}.png').write_bytes(data)


def main():
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument('--spec', required=True, help='edit spec JSON; relative paths resolve against its directory')
    a.add_argument('--out', required=True, help='new output directory')
    a.add_argument('--dry-run', action='store_true', help='validate, write plan.json, edit.frames.jsonl and ffmpeg-command.txt; no PNGs, no encode')
    args = a.parse_args()
    spec_path = pathlib.Path(args.spec).resolve()
    out = pathlib.Path(args.out).resolve()
    try:
        plan, rows, telemetry_cache = build_plan(json.loads(spec_path.read_text(encoding='utf-8')), spec_path.parent)
    except (EditError, ValueError, OSError, subprocess.CalledProcessError) as e:
        sys.exit(f'edit: {e}')
    out.mkdir(parents=True, exist_ok=False)
    (out / 'edit.frames.jsonl').write_text(''.join(json.dumps(r, sort_keys=True) + '\n' for r in rows), encoding='utf-8')
    (out / 'plan.json').write_text(json.dumps(plan, indent=2, sort_keys=True) + '\n', encoding='utf-8')
    command = edit_compose.build_command(plan, str(out / 'overlay' / 'overlay-%06d.png'), str(out / 'final.mp4'))
    (out / 'ffmpeg-command.txt').write_text(shlex.join(command) + '\n', encoding='utf-8')
    if args.dry_run:
        return
    render_overlays(rows, telemetry_cache, out / 'overlay')
    subprocess.run(command, check=True)
    packets = probe_video(out / 'final.mp4')[0]
    if packets != plan['frames']:
        sys.exit(f'edit: encoded {packets} frames, plan has {plan["frames"]}')
    print(json.dumps({'output': str(out / 'final.mp4'), 'frames': packets}))


if __name__ == '__main__':
    main()
