#!/usr/bin/env python3
"""Frame -> server tick map for live x11grab clips recorded on a machine whose clock is not the server's.

Inputs per clip NAME (all produced on the recording machine, see the G2 TAKE-RUNBOOK section):
- NAME.ffmpeg.log: stderr of the recorder started with
  `-use_wallclock_as_timestamps 1 -copyts -vf showinfo,setpts=PTS-STARTPTS -fps_mode passthrough`;
  every captured frame has a showinfo line whose pts_time is the recorder's wall clock (unix seconds).
- NAME.anchors.json: {"clock": [{"laptop_unix_ns", "host_minus_laptop_ns", "rtt_ns"}, ...],
                      "markers": [{"event", "server_tick", "captured_frame"}, ...]}
  `clock` entries are ssh round-trip measurements (before and after the take); `host_minus_laptop_ns`
  is host clock minus the recorder's clock. `markers` are visible events with a known server tick
  and the 0-based captured frame (frame number in the passthrough MP4) where each first shows.

A clip is anchored only if a marker agrees with the clock map; otherwise `anchor` returns the reason and
the caller shows no telemetry. Nothing is estimated beyond the measured marker shift.
"""
import bisect
import json
import pathlib
import re
import statistics
from decimal import Decimal

FPS = 60
MAX_MARKER_SHIFT_TICKS = 6     # clock map vs marker: more than 300 ms apart means the clock map is wrong
MAX_MARKER_SPREAD_TICKS = 2    # two markers disagreeing by more than 100 ms mean drift
SHOWINFO = re.compile(r'Parsed_showinfo\S* @ \S+\s+n:\s*(\d+)\s+pts:\s*\d+\s+pts_time:\s*([0-9.]+)')


def parse_showinfo(path):
    """Recorder wall time in ns of each captured frame, in capture order."""
    walls = []
    for line in pathlib.Path(path).read_text(encoding='utf-8', errors='replace').splitlines():
        m = SHOWINFO.search(line)
        if not m:
            continue
        if int(m.group(1)) != len(walls):
            raise ValueError(f'{path}: showinfo frame {m.group(1)} after {len(walls)} frames')
        walls.append(int(Decimal(m.group(2)) * 10**9))
    if not walls:
        raise ValueError(f'{path}: no showinfo lines (recorder not started with the showinfo filter?)')
    return walls


def read_telemetry_clock(path):
    """(wall_start_unix_ns list, tick list) of telemetry.jsonl, strictly increasing."""
    walls, ticks = [], []
    with open(path, encoding='utf-8') as f:
        for line in f:
            row = json.loads(line)
            if ticks and (row['tick'] <= ticks[-1] or row['wall_start_unix_ns'] <= walls[-1]):
                raise ValueError('telemetry ticks and wall_start_unix_ns must strictly increase')
            ticks.append(row['tick'])
            walls.append(row['wall_start_unix_ns'])
    return walls, ticks


def host_minus_laptop(clock, laptop_ns):
    """Offset at `laptop_ns`: linear between the first and last measurement, constant with one."""
    if not clock:
        raise ValueError('anchors.json has no clock measurement')
    clock = sorted(clock, key=lambda c: c['laptop_unix_ns'])
    first, last = clock[0], clock[-1]
    if first is last or last['laptop_unix_ns'] == first['laptop_unix_ns']:
        return first['host_minus_laptop_ns']
    # Clamped: outside the measured interval the nearest measurement holds (no extrapolated drift).
    share = min(max((laptop_ns - first['laptop_unix_ns']) / (last['laptop_unix_ns'] - first['laptop_unix_ns']), 0), 1)
    return round(first['host_minus_laptop_ns'] + share * (last['host_minus_laptop_ns'] - first['host_minus_laptop_ns']))


def tick_at(telemetry_clock, host_ns):
    """Last telemetry tick whose wall_start is <= host_ns; None before the first tick or after the last."""
    walls, ticks = telemetry_clock
    i = bisect.bisect_right(walls, host_ns) - 1
    if i < 0 or i == len(walls) - 1 and host_ns - walls[i] > 100_000_000:
        return None
    return ticks[i]


def clock_ticks(walls, anchors, telemetry_clock):
    """Clock-only tick per captured frame (None where telemetry does not cover it)."""
    return [tick_at(telemetry_clock, w + host_minus_laptop(anchors['clock'], w)) for w in walls]


def marker_shift(walls, anchors, telemetry_clock):
    """Median ticks to add to the clock map so each marker frame shows its marker tick; raises ValueError."""
    markers = anchors.get('markers') or []
    if not markers:
        raise ValueError('no marker: nothing checks the clock map')
    shifts = []
    for m in markers:
        frame = m['captured_frame']
        if not 0 <= frame < len(walls):
            raise ValueError(f"marker {m['event']!r}: captured_frame {frame} outside the clip ({len(walls)} frames)")
        tick = tick_at(telemetry_clock, walls[frame] + host_minus_laptop(anchors['clock'], walls[frame]))
        if tick is None:
            raise ValueError(f"marker {m['event']!r}: clock map puts frame {frame} outside the telemetry")
        shifts.append(m['server_tick'] - tick)
    if max(abs(s) for s in shifts) > MAX_MARKER_SHIFT_TICKS:
        raise ValueError(f'marker shifts {shifts} ticks exceed {MAX_MARKER_SHIFT_TICKS}: clock offset wrong')
    if max(shifts) - min(shifts) > MAX_MARKER_SPREAD_TICKS:
        raise ValueError(f'marker shifts {shifts} ticks disagree by more than {MAX_MARKER_SPREAD_TICKS}: drift')
    return round(statistics.median(shifts)), shifts


def grid_ticks(walls, frames60, anchors, telemetry_clock, shift):
    """Tick of every frame of the clip after `fps=60`: slot i is i/60 s after the first captured frame."""
    out = []
    for i in range(frames60):
        wall = walls[0] + i * 10**9 // FPS
        tick = tick_at(telemetry_clock, wall + host_minus_laptop(anchors['clock'], wall))
        shifted = None if tick is None else tick + shift
        out.append(shifted)
    ticks = set(telemetry_clock[1])
    for i, t in enumerate(out):
        if t is None or t not in ticks:
            raise ValueError(f'clip frame {i}: tick {t} is not in the telemetry')
    return out


def anchor(anchors_path, log_path, frames60, telemetry_clock):
    """(ticks per 60 fps frame | None, report). None means unanchored, report['reason'] says why."""
    report = {'anchored': False}
    try:
        anchors = json.loads(pathlib.Path(anchors_path).read_text(encoding='utf-8'))
        walls = parse_showinfo(log_path)
        shift, shifts = marker_shift(walls, anchors, telemetry_clock)
        ticks = grid_ticks(walls, frames60, anchors, telemetry_clock, shift)
    except (OSError, ValueError, KeyError) as error:
        report['reason'] = str(error)
        return None, report
    report.update(anchored=True, marker_shift_ticks=shift, marker_shifts=shifts,
                  rtt_ms=[c['rtt_ns'] / 1e6 for c in anchors['clock']], captured_frames=len(walls))
    return ticks, report


def measure_clock(ssh_command, samples=9):
    """Laptop side: clock offset to the host over ssh, minimum-RTT sample. host_minus_laptop is exact to +-rtt/2."""
    import shlex
    import subprocess
    import time
    argv = shlex.split(ssh_command) + ['date +%s%N']
    subprocess.run(argv, check=True, capture_output=True)  # opens the connection, not measured
    best = None
    for _ in range(samples):
        t0 = time.time_ns()
        host = int(subprocess.run(argv, check=True, capture_output=True, text=True).stdout.strip())
        t1 = time.time_ns()
        if best is None or t1 - t0 < best['rtt_ns']:
            best = {'laptop_unix_ns': (t0 + t1) // 2, 'host_minus_laptop_ns': host - (t0 + t1) // 2, 'rtt_ns': t1 - t0}
    return best


def main():
    import argparse
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest='command', required=True)
    c = sub.add_parser('clock', help='laptop: measure host-minus-laptop clock offset over ssh')
    c.add_argument('--ssh', required=True, help='e.g. "ssh -B wlp4s0 -o ControlMaster=auto -o ControlPath=/tmp/g2-ssh -o ControlPersist=300 kaiserroman@194.26.229.17"')
    c.add_argument('--out', required=True)
    a = sub.add_parser('anchors', help='write NAME.anchors.json from clock measurements and markers')
    a.add_argument('--clock', nargs='+', required=True, help='clock-before.json clock-after.json')
    a.add_argument('--marker', action='append', default=[], metavar='EVENT:SERVER_TICK:CAPTURED_FRAME')
    a.add_argument('--out', required=True)
    f = sub.add_parser('predict', help='captured frame where a server tick should show (to find a marker by eye)')
    f.add_argument('--log', required=True)
    f.add_argument('--clock', nargs='+', required=True)
    f.add_argument('--telemetry', required=True)
    f.add_argument('--tick', required=True, type=int)
    args = p.parse_args()
    if args.command == 'clock':
        pathlib.Path(args.out).write_text(json.dumps(measure_clock(args.ssh)) + '\n')
        return
    clock = [json.loads(pathlib.Path(x).read_text()) for x in args.clock]
    if args.command == 'anchors':
        markers = []
        for spec in args.marker:
            event, tick, frame = spec.rsplit(':', 2)
            markers.append({'event': event, 'server_tick': int(tick), 'captured_frame': int(frame)})
        with open(args.out, 'x', encoding='utf-8') as f_out:
            json.dump({'clock': clock, 'markers': markers}, f_out, indent=1)
        return
    walls = parse_showinfo(args.log)
    telemetry_clock = read_telemetry_clock(args.telemetry)
    index = telemetry_clock[1].index(args.tick)
    host_wall = telemetry_clock[0][index]
    best = min(range(len(walls)), key=lambda i: abs(walls[i] + host_minus_laptop(clock, walls[i]) - host_wall))
    print(f'tick {args.tick} should show near captured frame {best} (look at frames {max(best - 20, 0)}..{best + 40}: the display lags the server tick)')


if __name__ == '__main__':
    main()
