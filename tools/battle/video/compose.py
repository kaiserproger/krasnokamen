#!/usr/bin/env python3
"""Compose N replay shots in shots.txt order with the telemetry overlay.

`map` writes the per-output-frame tick map (edit.frames.jsonl) for overlay.py.
`build` does everything: tick map, overlay PNGs, crossfaded/cut encode.
"""
import argparse
import json
import math
import pathlib
import re
import subprocess
import sys

import live
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


def sharp_cut(prev_camera, next_camera, sharp_yaw, sharp_distance):
    """Same rule as tools/battle_shots.py: a cut changes the angle sharply when the camera positions
    or the look targets are more than `sharp_distance` blocks apart, or the horizontal view
    directions differ by more than `sharp_yaw` degrees. Entity-attached cameras have no world
    position here, so only the heading decides when either side is attached."""
    if prev_camera is None or next_camera is None:  # a live window recording has no camera file
        return True
    turn = abs((look_yaw(next_camera[0]) - look_yaw(prev_camera[1]) + 180) % 360 - 180)
    if turn > sharp_yaw:
        return True
    if prev_camera[2] is not None or next_camera[2] is not None:
        return False
    a, b = prev_camera[1], next_camera[0]
    return (math.dist(a['position'], b['position']) > sharp_distance
            or math.dist(a['look_target'], b['look_target']) > sharp_distance)


def plan_no_backtrack(shots, ticks, cameras, sharp_yaw, sharp_distance, min_frames):
    """Order by START and make battle time never step back at a cut, except at sharp angle changes.

    A shot whose first tick is earlier than the tick after the previous kept shot's last frame
    has its head trimmed to that tick; a shot left with fewer than `min_frames` frames is dropped.
    Returns (kept [(shot index, trim frames)], cut-table rows in play order incl. dropped shots).
    """
    kept, table = [], []
    prev = None
    for i in sorted(range(len(shots)), key=lambda i: shots[i].start):
        if ticks[i][0] is None:  # unanchored live clip: no ticks, no ordering constraint
            row = {'shot': shots[i].name, 'start_s': shots[i].start, 'end_s': shots[i].end, 'frames': len(ticks[i]),
                   'trim_frames': 0, 'status': 'unanchored', 'exception': False, 'in_tick': None, 'out_tick': None, 'delta_ticks': None}
            kept.append((i, 0))
            table.append(row)
            continue
        first, last, length = ticks[i][0], ticks[i][-1], len(ticks[i])
        row = {'shot': shots[i].name, 'start_s': shots[i].start, 'end_s': shots[i].end, 'frames': length}
        trim, exception, status = 0, False, 'kept'
        if prev is not None:
            bound = ticks[prev[0]][-1] + 1
            row['delta_ticks_original'] = first - bound
            if first < bound:
                if sharp_cut(cameras[prev[0]], cameras[i], sharp_yaw, sharp_distance):
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
        tick = ticks_per_shot[k][out - starts[k]]
        rows.append({'frame': out, 'last_applied_tick': tick, 'unanchored': tick is None,
                     'shot': k, 'shot_frame': out - starts[k]})
    return rows, total


def escape_filter_path(path):
    # Filtergraph syntax is parsed separately from argv by libavfilter.
    return path.replace('\\', '\\\\').replace(':', '\\:').replace("'", "'\\\\\\''").replace(',', '\\,').replace('[', '\\[').replace(']', '\\]')


def build_filter(lengths, crossfade, resolution, title_file, music, fade_in=0.5, fade_out=1.0, trims=None, ends=None, caption_lines=None):
    """Filter graph for inputs: shots 0..N-1, overlay PNG sequence N, music N+1 (optional).

    `lengths` are the visible frames per shot; `trims[i]` frames are cut off the head of input i and,
    if `ends[i]` is set, the clip ends at its frame `ends[i]`. The clip is resampled to 60 fps first,
    so frame numbers are on the 60 fps grid (what the tick map describes).
    """
    n = len(lengths)
    width, height = (1920, 1080) if resolution == '1080p' else (3840, 2160)
    panel = 1 if resolution == '1080p' else 2  # overlay is rendered at native 480x260 (280 high with --units-label)
    trims = trims or [0] * n
    ends = ends or [None] * n

    def source(i):
        if not trims[i] and ends[i] is None:
            return f'fps={FPS},'
        end = '' if ends[i] is None else f':end_frame={ends[i]}'
        return f'fps={FPS},trim=start_frame={trims[i]}{end},setpts=PTS-STARTPTS,'
    parts = [f'[{i}:v]{source(i)}scale={width}:{height}:flags=lanczos,setsar=1,settb=AVTB,setpts=PTS-STARTPTS[s{i}]'
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
    caption = ''
    # Persistent honest label (what the footage is, how many soldiers), one drawtext per line: a newline inside
    # a drawtext text file renders as a missing-glyph box on some ffmpeg builds. Lines are stacked bottom-left.
    step = round(26 * panel * 1.5)
    for i, line_file in enumerate(caption_lines or []):
        caption += (f"drawtext=fontfile={FONT}:textfile='{escape_filter_path(line_file)}':expansion=none:"
                    f"fontcolor=white:fontsize={26*panel}:box=1:boxcolor=black@0.55:boxborderw={5*panel}:"
                    f"x={48*panel}:y=h-{(len(caption_lines) - i) * step + 48 * panel},")
    parts.append(f'[faded][panel]overlay=x=W-w-{48*panel}:y={48*panel}:alpha=straight:eof_action=endall,' + caption +
                 f"drawtext=fontfile={FONT}:textfile='{title}':expansion=none:"
                 f"fontcolor=white:fontsize={48*panel}:x=(w-text_w)/2:y=h*0.78:enable='between(t,0,3)',"
                 'scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,'
                 'setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv[v]')
    if music:
        parts.append(f'[{n+1}:a]loudnorm=I=-14:TP=-1:LRA=11,afade=t=out:st={seconds-fade_out:.6f}:d={fade_out}[audio]')
    return ';'.join(parts)


def encode_command(shot_files, overlay_pattern, music, lengths, crossfade, resolution, title_file, output, trims=None, ends=None, caption_lines=None):
    total = sum(lengths) - (len(lengths) - 1) * crossfade
    cmd = ['ffmpeg', '-hide_banner', '-nostdin', '-n', '-filter_complex_threads', '1']
    for path in shot_files:
        cmd += ['-i', str(path)]
    cmd += ['-framerate', str(FPS), '-i', overlay_pattern]
    if music:
        cmd += ['-stream_loop', '-1', '-i', music]
    cmd += ['-filter_complex', build_filter(lengths, crossfade, resolution, title_file, music, trims=trims, ends=ends, caption_lines=caption_lines), '-map', '[v]']
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


_PROBE60 = {}
_ANCHORS = {}


def probe_frames60(path):
    """Frames of a clip after `fps=60`, as the montage filter graph sees it (cached per file: aliases share a recording)."""
    key = str(pathlib.Path(path).resolve())
    if key not in _PROBE60:
        err = subprocess.run(['ffmpeg', '-hide_banner', '-nostdin', '-i', str(path), '-vf', f'fps={FPS}', '-an', '-f', 'null', '-'],
                             check=True, capture_output=True, text=True).stderr
        counts = re.findall(r'frame=\s*(\d+)', err)
        if not counts:
            raise ValueError(f'{path}: could not count frames')
        _PROBE60[key] = int(counts[-1])
    return _PROBE60[key]


def load_live(shot, args, telemetry_clock):
    """(ticks of the excerpt, head, end, full frames, report) of a live clip on the 60 fps grid.

    Anchored: START/END are take seconds, the excerpt is the frames whose tick is in
    [first_tick + START*20, first_tick + END*20) and every frame of it must have a tick. Unanchored: the whole
    clip, no ticks (START/END only order it).
    """
    if not args.frames_dir or not args.shots_dir:
        raise ValueError(f'{shot.name}: live shots need --frames-dir (NAME.anchors.json, NAME.ffmpeg.log) and --shots-dir (NAME.mp4)')
    full = probe_frames60(pathlib.Path(args.shots_dir) / f'{shot.name}.mp4')
    anchors_path = (pathlib.Path(args.frames_dir) / f'{shot.name}.anchors.json').resolve()
    log_path = (pathlib.Path(args.frames_dir) / f'{shot.name}.ffmpeg.log').resolve()
    if (anchors_path, log_path, full) not in _ANCHORS:  # aliases of one recording share the (slow) tick grid
        _ANCHORS[(anchors_path, log_path, full)] = live.anchor(anchors_path, log_path, full, telemetry_clock)
    ticks, report = _ANCHORS[(anchors_path, log_path, full)]
    if ticks is None:
        return [None] * full, 0, full, full, report
    lo = args.first_tick + round(shot.start * TICKS_PER_SECOND)
    hi = args.first_tick + round(shot.end * TICKS_PER_SECOND)
    head = next((i for i, t in enumerate(ticks) if t is not None and t >= lo), full)
    end = next((i for i, t in enumerate(ticks) if t is not None and t >= hi), full)
    excerpt = ticks[head:end]
    if end - head < 1 or None in excerpt or ticks[head] > lo + 1 or excerpt[-1] < hi - 2:
        raise ValueError(f'{shot.name}: excerpt of ticks {excerpt[0] if excerpt else None}..{excerpt[-1] if excerpt else None} '
                         f'does not cover shots.txt {lo}..{hi} with telemetry ticks')
    return excerpt, head, end, full, report


def load_shots(args):
    """Return (shots, per-shot tick lists, per-shot source dicts {head, end, full}, live reports)."""
    shots = parse_shots(args.shots_file)
    ticks, sources, reports = [], [], {}
    telemetry_clock = None
    for shot in shots:
        if shot.camera == 'live':
            if telemetry_clock is None:
                if not args.telemetry:
                    raise ValueError('live shots need --telemetry')
                telemetry_clock = live.read_telemetry_clock(args.telemetry)
            t, head, end, full, reports[shot.name] = load_live(shot, args, telemetry_clock)
            ticks.append(t)
            sources.append({'head': head, 'end': end, 'full': full})
            continue
        frames = pathlib.Path(args.frames_dir) / f'{shot.name}.frames.jsonl' if args.frames_dir else None
        if frames is None:
            ticks.append(predicted_ticks(shot, args.first_tick))
            sources.append({'head': 0, 'end': None, 'full': len(ticks[-1])})
            continue
        recorded = recorded_ticks(frames)
        expected = args.first_tick + round(shot.start * TICKS_PER_SECOND)
        if abs(recorded[0] - expected) > 1:
            raise ValueError(f'{frames}: first tick {recorded[0]} but shots.txt START {shot.start} s '
                             f'is tick {expected} (wrong take or first_tick?)')
        ticks.append(recorded)
        sources.append({'head': 0, 'end': None, 'full': len(recorded)})
    return shots, ticks, sources, reports


def write_map(rows, path):
    with open(path, 'x', encoding='utf-8') as f:
        for row in rows:
            f.write(json.dumps(row) + '\n')


def add_common(p):
    p.add_argument('--telemetry', help='telemetry.jsonl; required for live shots (and by build)')
    p.add_argument('--shots-dir', help='directory with NAME.mp4 per shot; required for live shots (and by build)')
    p.add_argument('--shots-file', required=True, help='shots.txt: NAME CAMERA START END [RADIUS]')
    p.add_argument('--first-tick', required=True, type=int, help='first_tick of take.solrec.result.json')
    p.add_argument('--frames-dir', help='directory with NAME.frames.jsonl from bv-render; without it ticks are predicted from shots.txt')
    p.add_argument('--crossfade-frames', type=int, default=0, help='frames of fade between shots; 0 = hard cut')
    p.add_argument('--no-backtrack', action='store_true',
                   help='order shots by START and trim heads so battle time never steps back at a cut, except at sharp angle changes; needs --frames-dir')
    p.add_argument('--sharp-yaw', type=float, default=60, help='view-direction difference (degrees) above which a cut is a sharp angle change')
    p.add_argument('--sharp-distance', type=float, default=40, help='camera or look-target distance (blocks) above which a cut is a sharp angle change')
    p.add_argument('--min-shot-seconds', type=float, default=2, help='a trimmed shot shorter than this is dropped')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest='command', required=True)
    m = sub.add_parser('map', help='write the output-frame tick map')
    add_common(m)
    m.add_argument('--out', required=True, help='new edit.frames.jsonl')
    b = sub.add_parser('build', help='tick map + overlay + encode')
    add_common(b)
    b.add_argument('--work', required=True, help='new directory for edit.frames.jsonl and overlay/')
    b.add_argument('--title-file', required=True, help='UTF-8 text file, avoids drawtext expression injection')
    b.add_argument('--music', help='licensed audio file; without it the video has no audio track')
    b.add_argument('--resolution', choices=('1080p', '4k'), required=True)
    b.add_argument('--output', required=True)
    b.add_argument('--caption-file', help='UTF-8 text file drawn bottom-left for the whole video (what the footage is)')
    b.add_argument('--units-label', help='panel UNITS line names what it counts, e.g. "SOLDIERS + HORSES" (two-line units block, taller panel)')
    b.add_argument('--alive-total', action='store_true', help='with --units-label: the line reads ALIVE N, N = sum of the SIDE numbers (telemetry units also counts fallen bodies)')
    b.add_argument('--panel-label', default='REPLAY TELEMETRY', help='panel header after the tick, e.g. "LIVE SERVER TELEMETRY" (max ~22 characters)')
    a = p.parse_args()
    shots, ticks, sources, reports = load_shots(a)
    for name, report in reports.items():
        print(f"live {name}: " + (f"anchored (marker shift {report['marker_shift_ticks']} ticks {report['marker_shifts']}, rtt ms {report['rtt_ms']})"
                                  if report['anchored'] else f"UNANCHORED, no telemetry: {report['reason']}"))
    table = None
    if a.no_backtrack:
        if not a.frames_dir:
            p.error('--no-backtrack needs --frames-dir (recorded ticks)')
        root = pathlib.Path(a.shots_file).parent
        cameras = [None if s.camera == 'live' else camera_ends(root / s.camera) for s in shots]
        kept, table = plan_no_backtrack(shots, ticks, cameras, a.sharp_yaw, a.sharp_distance, round(a.min_shot_seconds * FPS))
        shots = [shots[i] for i, _ in kept]
        sources = [dict(sources[i], head=sources[i]['head'] + t) for i, t in kept]
        ticks = [ticks[i][t:] for i, t in kept]
    rows, total = edit_frames(ticks, a.crossfade_frames)
    if table:
        print(cut_table_markdown(table))
    if a.command == 'map':
        write_map(rows, a.out)
        print(f'{total} frames, {total / FPS:.3f} s')
        return
    if not a.telemetry or not a.shots_dir:
        p.error('build needs --telemetry and --shots-dir')
    shot_files = [pathlib.Path(a.shots_dir) / f'{s.name}.mp4' for s in shots]
    for shot, path, source in zip(shots, shot_files, sources):
        got = probe_frames60(path) if shot.camera == 'live' else probe_frames(path)
        if got != source['full']:
            sys.exit(f'{path}: {got} frames but its frames file has {source["full"]}')
    work = pathlib.Path(a.work)
    work.mkdir(parents=True, exist_ok=False)
    if table:
        (work / 'cut-table.json').write_text(json.dumps(table, indent=1) + '\n')
        (work / 'cut-table.md').write_text(cut_table_markdown(table))
    write_map(rows, work / 'edit.frames.jsonl')
    (work / 'live-anchors.json').write_text(json.dumps(reports, indent=1) + '\n')
    import overlay
    overlay.write_overlay(a.telemetry, work / 'edit.frames.jsonl', work / 'overlay', label=a.panel_label, units_label=a.units_label, alive_total=a.alive_total)
    caption_lines = []
    if a.caption_file:  # one text file per line (see build_filter)
        lines = [x.strip() for x in pathlib.Path(a.caption_file).read_text(encoding='utf-8').splitlines() if x.strip()]
        for i, text in enumerate(lines):
            line_file = work / f'caption-{i}.txt'
            line_file.write_text(text, encoding='utf-8')
            caption_lines.append(str(line_file))
    subprocess.run(encode_command(shot_files, str(work / 'overlay' / 'overlay-%06d.png'), a.music,
                                  [len(t) for t in ticks], a.crossfade_frames, a.resolution, a.title_file, a.output,
                                  trims=[s['head'] for s in sources],
                                  ends=[s['end'] for s in sources], caption_lines=caption_lines),
                   check=True)


if __name__ == '__main__':
    main()
