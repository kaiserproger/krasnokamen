#!/usr/bin/env python3
"""Compose N replay shots in shots.txt order with the telemetry overlay.

`map` writes the per-output-frame tick map (edit.frames.jsonl) for overlay.py.
`build` does everything: tick map, overlay PNGs, crossfaded/cut encode.
"""
import argparse
import json
import math
import pathlib
import subprocess
import sys
from collections import namedtuple

FPS = 60
TICKS_PER_SECOND = 20
FONT = '/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf'
Shot = namedtuple('Shot', 'name camera start end')


def parse_shots(path):
    """Parse `NAME CAMERA START END [RADIUS]` lines; `#` and blank lines are skipped."""
    shots = []
    for number, raw in enumerate(pathlib.Path(path).read_text(encoding='utf-8').splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith('#'):
            continue
        fields = line.split()
        if len(fields) not in (4, 5):
            raise ValueError(f'{path}:{number}: expected NAME CAMERA START END [RADIUS], got {line!r}')
        start, end = float(fields[2]), float(fields[3])
        if not 0 <= start < end:
            raise ValueError(f'{path}:{number}: require 0 <= START < END')
        shots.append(Shot(fields[0], fields[1], start, end))
    if not shots:
        raise ValueError(f'{path}: no shots')
    if len({s.name for s in shots}) != len(shots):
        raise ValueError(f'{path}: duplicate shot names')
    return shots


def predicted_ticks(shot, first_tick):
    """last_applied_tick per frame of a shot rendered with --timebase tick (stand-ins, plans).

    The real source of truth is the renderer's NAME.frames.jsonl.
    """
    frames = round((shot.end - shot.start) * FPS)
    start_tick = first_tick + round(shot.start * TICKS_PER_SECOND)
    return [start_tick + i * TICKS_PER_SECOND // FPS for i in range(frames)]


def recorded_ticks(path):
    ticks = []
    with open(path, encoding='utf-8') as f:
        for n, line in enumerate(f):
            row = json.loads(line)
            if row['frame'] != n:
                raise ValueError(f'{path}: expected frame {n}, got {row["frame"]}')
            ticks.append(row['last_applied_tick'])
    return ticks


def look_yaw(key):
    v = [key['look_target'][i] - key['position'][i] for i in range(3)]
    return math.degrees(math.atan2(v[0], v[2]))


def camera_ends(path):
    """(first key, last key, attach_entity) of a camera JSON."""
    data = json.loads(pathlib.Path(path).read_text(encoding='utf-8'))
    keys = data['keyframes']
    return keys[0], keys[-1], data.get('attach_entity')


def sharp_cut(prev_camera, next_camera, sharp_yaw):
    """A cut changes the angle sharply if the camera class changes (entity-attached close-up
    vs free camera) or the horizontal look direction turns by `sharp_yaw` degrees or more."""
    if (prev_camera[2] is None) != (next_camera[2] is None):
        return True
    turn = abs((look_yaw(next_camera[0]) - look_yaw(prev_camera[1]) + 180) % 360 - 180)
    return turn >= sharp_yaw


def plan_no_backtrack(shots, ticks, cameras, sharp_yaw, min_frames):
    """Order by START and make battle time never step back at a cut, except at sharp angle changes.

    A shot whose first tick is earlier than the tick after the previous kept shot's last frame
    has its head trimmed to that tick; a shot left with fewer than `min_frames` frames is dropped.
    Returns (kept [(shot index, trim frames)], cut-table rows in play order incl. dropped shots).
    """
    kept, table = [], []
    prev = None
    for i in sorted(range(len(shots)), key=lambda i: shots[i].start):
        first, last, length = ticks[i][0], ticks[i][-1], len(ticks[i])
        row = {'shot': shots[i].name, 'start_s': shots[i].start, 'end_s': shots[i].end, 'frames': length}
        trim, exception, status = 0, False, 'kept'
        if prev is not None:
            bound = ticks[prev[0]][-1] + 1
            row['delta_ticks_original'] = first - bound
            if first < bound:
                if sharp_cut(cameras[prev[0]], cameras[i], sharp_yaw):
                    exception = True
                else:
                    trim = next((j for j, t in enumerate(ticks[i]) if t >= bound), length)
                    if length - trim < min_frames:
                        status = 'dropped'
        if status == 'dropped':
            row.update(trim_frames=trim, status=status, exception=False, in_tick=None, out_tick=None, delta_ticks=None)
        else:
            row.update(trim_frames=trim, status=status, exception=exception, in_tick=ticks[i][trim], out_tick=last,
                       delta_ticks=None if prev is None else ticks[i][trim] - (ticks[prev[0]][-1] + 1))
            kept.append((i, trim))
            prev = (i,)
        table.append(row)
    return kept, table


def cut_table_markdown(table):
    lines = ['| shot | in tick | out tick | delta vs previous (s) | original delta (s) | trimmed frames | sharp-angle exception | status |',
             '|---|---|---|---|---|---|---|---|']
    fmt = lambda ticks: '-' if ticks is None else f'{ticks / TICKS_PER_SECOND:+.2f}'
    for r in table:
        lines.append(f"| {r['shot']} | {r['in_tick'] if r['in_tick'] is not None else '-'} | {r['out_tick'] if r['out_tick'] is not None else '-'} | "
                     f"{fmt(r['delta_ticks'])} | {fmt(r.get('delta_ticks_original'))} | {r['trim_frames']} | {'yes' if r['exception'] else 'no'} | {r['status']} |")
    return '\n'.join(lines) + '\n'


def edit_frames(ticks_per_shot, crossfade):
    """Output-frame order rows {frame, last_applied_tick, shot, shot_frame}.

    A crossfade of `crossfade` frames overlaps the tail of one shot with the head
    of the next; each overlapped output frame takes the tick of exactly one shot:
    the outgoing shot for the first half, the incoming shot for the second half.
    """
    lengths = [len(t) for t in ticks_per_shot]
    if crossfade < 0 or (crossfade and any(n <= 2 * crossfade for n in lengths)):
        raise ValueError('every shot must be longer than twice the crossfade')
    starts, at = [], 0
    for n in lengths:
        starts.append(at)
        at += n - crossfade
    total = at + crossfade
    rows = []
    for out in range(total):
        # Last shot that has started; inside its first `crossfade` frames the
        # outgoing shot is still visible.
        k = max(i for i, s in enumerate(starts) if s <= out)
        if k > 0 and out - starts[k] < crossfade // 2:
            k -= 1
        rows.append({'frame': out, 'last_applied_tick': ticks_per_shot[k][out - starts[k]],
                     'shot': k, 'shot_frame': out - starts[k]})
    return rows, total


def escape_filter_path(path):
    # Filtergraph syntax is parsed separately from argv by libavfilter.
    return path.replace('\\', '\\\\').replace(':', '\\:').replace("'", "'\\\\\\''").replace(',', '\\,').replace('[', '\\[').replace(']', '\\]')


def build_filter(lengths, crossfade, resolution, title_file, music, fade_in=0.5, fade_out=1.0, trims=None):
    """Filter graph for inputs: shots 0..N-1, overlay PNG sequence N, music N+1 (optional).

    `lengths` are the visible frames per shot; `trims[i]` frames are cut off the head of input i.
    """
    n = len(lengths)
    width, height = (1920, 1080) if resolution == '1080p' else (3840, 2160)
    panel = 1 if resolution == '1080p' else 2  # overlay is rendered at native 480x260
    trims = trims or [0] * n
    parts = [f'[{i}:v]' + (f'trim=start_frame={trims[i]},setpts=PTS-STARTPTS,' if trims[i] else '')
             + f'fps={FPS},scale={width}:{height}:flags=lanczos,setsar=1,settb=AVTB,setpts=PTS-STARTPTS[s{i}]'
             for i in range(n)]
    if n == 1:
        cut = 's0'
    elif crossfade == 0:
        parts.append(''.join(f'[s{i}]' for i in range(n)) + f'concat=n={n}:v=1:a=0[cut]')
        cut = 'cut'
    else:
        cur, elapsed = 's0', lengths[0]
        for k in range(1, n):
            out = 'cut' if k == n - 1 else f'x{k}'
            parts.append(f'[{cur}][s{k}]xfade=transition=fade:duration={crossfade/FPS:.6f}:offset={(elapsed-crossfade)/FPS:.6f}[{out}]')
            cur, elapsed = out, elapsed + lengths[k] - crossfade
        cut = 'cut'
    total = sum(lengths) - (n - 1) * crossfade
    seconds = total / FPS
    title = escape_filter_path(title_file)
    video = f'[{cut}]'
    if fade_in:
        video += f'fade=t=in:st=0:d={fade_in},'
    if fade_out:
        video += f'fade=t=out:st={seconds-fade_out:.6f}:d={fade_out},'
    parts.append(f'{video.rstrip(",")}[faded]')
    parts.append(f'[{n}:v]format=rgba,scale=iw*{panel}:ih*{panel}:flags=neighbor[panel]')
    parts.append('[faded][panel]overlay=x=W-w-32:y=32:alpha=straight:eof_action=endall,'
                 f"drawtext=fontfile={FONT}:textfile='{title}':expansion=none:"
                 f"fontcolor=white:fontsize={48*panel}:x=(w-text_w)/2:y=h*0.78:enable='between(t,0,3)',"
                 'scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,'
                 'setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv[v]')
    if music:
        parts.append(f'[{n+1}:a]loudnorm=I=-14:TP=-1:LRA=11,afade=t=out:st={seconds-fade_out:.6f}:d={fade_out}[audio]')
    return ';'.join(parts)


def encode_command(shot_files, overlay_pattern, music, lengths, crossfade, resolution, title_file, output, trims=None):
    total = sum(lengths) - (len(lengths) - 1) * crossfade
    cmd = ['ffmpeg', '-hide_banner', '-nostdin', '-n', '-filter_complex_threads', '1']
    for path in shot_files:
        cmd += ['-i', str(path)]
    cmd += ['-framerate', str(FPS), '-i', overlay_pattern]
    if music:
        cmd += ['-stream_loop', '-1', '-i', music]
    cmd += ['-filter_complex', build_filter(lengths, crossfade, resolution, title_file, music, trims=trims), '-map', '[v]']
    if music:
        cmd += ['-map', '[audio]']
    # Half a frame of slack: -t drops packets at pts >= t, so exactly `total` frames survive.
    cmd += ['-t', f'{(total - 0.5) / FPS:.6f}', '-r', str(FPS), '-c:v', 'libx264', '-threads', '2', '-preset', 'medium', '-crf', '16',
            '-profile:v', 'high', '-bf', '2', '-g', '30', '-keyint_min', '30', '-sc_threshold', '0',
            '-flags', '+cgop', '-pix_fmt', 'yuv420p', '-color_primaries', 'bt709', '-color_trc', 'bt709',
            '-colorspace', 'bt709', '-color_range', 'tv']
    if music:
        cmd += ['-c:a', 'aac', '-b:a', '384k', '-ar', '48000']
    return cmd + ['-movflags', '+faststart', str(output)]


def probe_frames(path):
    out = subprocess.run(['ffprobe', '-v', 'error', '-count_frames', '-select_streams', 'v:0',
                          '-show_entries', 'stream=nb_read_frames', '-of', 'csv=p=0', str(path)],
                         check=True, capture_output=True, text=True).stdout.strip()
    return int(out)


def load_shots(args):
    """Return (shots, per-shot tick lists) from the shots file and frames dir."""
    shots = parse_shots(args.shots_file)
    ticks = []
    for shot in shots:
        frames = pathlib.Path(args.frames_dir) / f'{shot.name}.frames.jsonl' if args.frames_dir else None
        if frames is None:
            ticks.append(predicted_ticks(shot, args.first_tick))
            continue
        recorded = recorded_ticks(frames)
        expected = args.first_tick + round(shot.start * TICKS_PER_SECOND)
        if abs(recorded[0] - expected) > 1:
            raise ValueError(f'{frames}: first tick {recorded[0]} but shots.txt START {shot.start} s '
                             f'is tick {expected} (wrong take or first_tick?)')
        ticks.append(recorded)
    return shots, ticks


def write_map(rows, path):
    with open(path, 'x', encoding='utf-8') as f:
        for row in rows:
            f.write(json.dumps(row) + '\n')


def add_common(p):
    p.add_argument('--shots-file', required=True, help='shots.txt: NAME CAMERA START END [RADIUS]')
    p.add_argument('--first-tick', required=True, type=int, help='first_tick of take.solrec.result.json')
    p.add_argument('--frames-dir', help='directory with NAME.frames.jsonl from bv-render; without it ticks are predicted from shots.txt')
    p.add_argument('--crossfade-frames', type=int, default=0, help='frames of fade between shots; 0 = hard cut')
    p.add_argument('--no-backtrack', action='store_true',
                   help='order shots by START and trim heads so battle time never steps back at a cut, except at sharp angle changes; needs --frames-dir')
    p.add_argument('--sharp-yaw', type=float, default=90, help='degrees of look-direction turn that count as a sharp angle change')
    p.add_argument('--min-shot-seconds', type=float, default=2, help='a trimmed shot shorter than this is dropped')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest='command', required=True)
    m = sub.add_parser('map', help='write the output-frame tick map')
    add_common(m)
    m.add_argument('--out', required=True, help='new edit.frames.jsonl')
    b = sub.add_parser('build', help='tick map + overlay + encode')
    add_common(b)
    b.add_argument('--shots-dir', required=True, help='directory with NAME.mp4 per shot')
    b.add_argument('--telemetry', required=True)
    b.add_argument('--work', required=True, help='new directory for edit.frames.jsonl and overlay/')
    b.add_argument('--title-file', required=True, help='UTF-8 text file, avoids drawtext expression injection')
    b.add_argument('--music', help='licensed audio file; without it the video has no audio track')
    b.add_argument('--resolution', choices=('1080p', '4k'), required=True)
    b.add_argument('--output', required=True)
    a = p.parse_args()
    shots, ticks = load_shots(a)
    full_lengths = [len(t) for t in ticks]
    trims, table = [0] * len(shots), None
    if a.no_backtrack:
        if not a.frames_dir:
            p.error('--no-backtrack needs --frames-dir (recorded ticks)')
        root = pathlib.Path(a.shots_file).parent
        cameras = [camera_ends(root / s.camera) for s in shots]
        kept, table = plan_no_backtrack(shots, ticks, cameras, a.sharp_yaw, round(a.min_shot_seconds * FPS))
        shots = [shots[i] for i, _ in kept]
        trims = [t for _, t in kept]
        full_lengths = [full_lengths[i] for i, _ in kept]
        ticks = [ticks[i][t:] for i, t in kept]
    rows, total = edit_frames(ticks, a.crossfade_frames)
    if table:
        print(cut_table_markdown(table))
    if a.command == 'map':
        write_map(rows, a.out)
        print(f'{total} frames, {total / FPS:.3f} s')
        return
    shot_files = [pathlib.Path(a.shots_dir) / f'{s.name}.mp4' for s in shots]
    for path, full in zip(shot_files, full_lengths):
        got = probe_frames(path)
        if got != full:
            sys.exit(f'{path}: {got} frames but its frames file has {full}')
    work = pathlib.Path(a.work)
    work.mkdir(parents=True, exist_ok=False)
    if table:
        (work / 'cut-table.json').write_text(json.dumps(table, indent=1) + '\n')
        (work / 'cut-table.md').write_text(cut_table_markdown(table))
    write_map(rows, work / 'edit.frames.jsonl')
    import overlay
    overlay.write_overlay(a.telemetry, work / 'edit.frames.jsonl', work / 'overlay')
    subprocess.run(encode_command(shot_files, str(work / 'overlay' / 'overlay-%06d.png'), a.music,
                                  [len(t) for t in ticks], a.crossfade_frames, a.resolution, a.title_file, a.output, trims=trims),
                   check=True)


if __name__ == '__main__':
    main()
