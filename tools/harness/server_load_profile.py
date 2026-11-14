#!/usr/bin/env python3
"""Always-profile server workloads through their existing lifecycle; run in mem-run -x.

Perf period (CPU-clock nanoseconds) / completed source ticks gives sampled CPU
ms/tick. Self and inclusive tables are separate; inclusive rows overlap.
"""
import argparse
import collections
import ctypes
import functools
import json
import os
from pathlib import Path
import re
import select
import signal
import struct
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent

def dump(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')

def snapshot(out, label):
    with (out / 'neighbours.txt').open('a') as f:
        f.write(f'\n{label} unix_ns={time.time_ns()} monotonic_ns={time.monotonic_ns()}\n')
        f.flush()
        subprocess.run(['ps', '-eo', 'pid,ppid,stat,etimes,time,pcpu,comm,args', '--sort=-pcpu'], stdout=f, check=True)

def capture(out, command):
    out.mkdir(parents=True, exist_ok=True)
    # Export/snapshot invocations do not start a live server or acquire samples.
    if '--config' not in command:
        os.execvp(command[0], command)
    name = f'perf-{os.getpid()}'
    data = out / (name + '.data')
    offset = time.time_ns() - time.monotonic_ns()
    server = subprocess.Popen(command)
    dump(out / (name + '.json'), {'pid': server.pid, 'command': command, 'unix_minus_monotonic_ns': offset})
    def terminate(signum, frame):
        if server.returncode is None:
            server.send_signal(signum)
    signal.signal(signal.SIGTERM, terminate)
    signal.signal(signal.SIGINT, terminate)
    with (out / (name + '.log')).open('w') as log:
        perf = subprocess.Popen(['perf', 'record', '-e', 'cpu-clock', '-F', '199', '--call-graph', 'fp',
                                 '-k', 'CLOCK_MONOTONIC', '-o', str(data), '-p', str(server.pid)], stdout=log, stderr=log, start_new_session=True)
        code = server.wait()
        if perf.poll() is None:
            perf.send_signal(signal.SIGINT)
        perf_code = perf.wait()
    dump(out / (name + '.exit.json'), {'server': code, 'perf': perf_code})
    return code if code else int(perf_code not in (0, -signal.SIGINT))

class Changes:
    def __init__(self, root):
        self.lib = ctypes.CDLL(None, use_errno=True)
        self.fd = self.lib.inotify_init1(os.O_CLOEXEC | os.O_NONBLOCK)
        if self.fd < 0:
            raise OSError(ctypes.get_errno(), 'inotify_init1')
        self.paths = {}
        self.add(root)
    def add(self, root):
        for path in [root, *(p for p in root.rglob('*') if p.is_dir())]:
            wd = self.lib.inotify_add_watch(self.fd, os.fsencode(path), 0x100 | 0x2 | 0x80)
            if wd >= 0:
                self.paths[wd] = path
    def events(self):
        data = os.read(self.fd, 1 << 20)
        at = 0
        changed = []
        while at < len(data):
            wd, mask, cookie, size = struct.unpack_from('iIII', data, at)
            name = data[at+16:at+16+size].split(b'\0', 1)[0]
            at += 16 + size
            if wd not in self.paths:
                continue
            path = self.paths[wd] / os.fsdecode(name)
            if mask & 0x40000000 and path.is_dir():
                self.add(path)
            elif path.name in ('samples.jsonl', 'telemetry.jsonl'):
                changed.append(path)
        return changed
    def close(self):
        os.close(self.fd)

def midpoint_reached(count, first_ns, last_ns):
    # Slow real windows can finish without ever completing 600 source ticks.
    return count >= 600 or (first_ns is not None and last_ns - first_ns >= 30_000_000_000)

def run(out, command):
    out.mkdir(parents=True, exist_ok=False)
    snapshot(out, 'first')
    dump(out / 'command.json', {'command': command, 'cwd': os.getcwd()})
    watch = Changes(out)
    with (out / 'command.log').open('w') as log:
        child = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
        pidfd = os.pidfd_open(child.pid)
        poll = select.poll()
        poll.register(pidfd, select.POLLIN)
        poll.register(watch.fd, select.POLLIN)
        counts = collections.Counter()
        offsets = {}
        first_clocks = {}
        last_clocks = {}
        middle = False
        try:
            done = False
            while not done:
                for fd, mask in poll.poll():
                    if fd == pidfd:
                        done = True
                        break
                    for path in watch.events():
                        with path.open() as f:
                            f.seek(offsets.get(path, 0))
                            data = f.read()
                            # Ignore a partial last line until the next pushed write.
                            complete = data.rfind('\n') + 1
                            offsets[path] = f.tell() - len(data[complete:])
                        for line in data[:complete].splitlines():
                            row = json.loads(line)
                            if path.name == 'telemetry.jsonl' and 'battle' in row and row['battle'].get('fighting', 0) == 0:
                                continue
                            source = row.get('metrics', {}).get('samples', []) if 'metrics' in row else [row]
                            counts[path] += len(source)
                            if source:
                                first_clocks.setdefault(path, source[0]['wall_start_unix_ns'])
                                last_clocks[path] = source[-1]['wall_start_unix_ns']
                        if not middle and midpoint_reached(counts[path], first_clocks.get(path), last_clocks.get(path, 0)):
                            snapshot(out, 'mid')
                            middle = True
            code = child.wait()
        finally:
            os.close(pidfd)
            watch.close()
    snapshot(out, 'last')
    dump(out / 'command-exit.json', {'exit': code, 'mid_snapshot': middle})
    return code

HEADER = re.compile(r'^\s*(?P<comm>.*?)\s+(?P<time>\d+\.\d+):\s+(?P<period>\d+)\s+cpu-clock:')
FRAME = re.compile(r'^\s*[0-9a-f]+\s+(.*?)\s+\((.*)\)\s*$')
FLAGS = {
    'string_hash_compare': re.compile(r'(?i)core::hash::sip::|siphasher::|ahash::|(?:^|::)(?:memcmp|bcmp|strcmp|strncmp|hash_one|hash_bytes)$|(?:str|String)(?: as [^>]+)?>?::(?:hash|eq|cmp)$|core::str::(?:cmp|eq)$'),
    'allocation': re.compile(r'(?:^|::)(?:__rust_alloc(?:_zeroed)?|__rust_dealloc|__rust_realloc|malloc|calloc|realloc|free)$|alloc::alloc::|(?:RawVec(?:Inner)?|Vec|String)(?: as [^>]+)?>?::(?:reserve|reserve_exact|grow_amortized|grow_exact|with_capacity|try_reserve|try_allocate_in|shrink_to_fit)$'),
    'map_lookup': re.compile(r'(?:hashbrown::.*|HashMap|BTreeMap|RawTable)(?: as [^>]+)?>?::(?:get|get_mut|contains_key|entry|find|find_inner|find_or_find_insert_slot|lookup|search_tree)$'),
}

@functools.lru_cache(maxsize=65536)
def classification_name(function):
    """Remove generic arguments, not UFCS owners, before matching operations."""
    result = []
    depth = 0
    for i, char in enumerate(function):
        if depth:
            depth += (char == '<') - (char == '>')
        elif char == '<' and i and (function[i-1].isalnum() or function[i-1] in '_:'):
            depth = 1
        else:
            result.append(char)
    return ''.join(result)

def samples(lines):
    header = None
    frames = []
    import itertools
    for line in itertools.chain(lines, ['']):
        match = HEADER.match(line)
        if match:
            if header:
                yield header, frames
            header, frames = match.groupdict(), []
        elif not line.strip():
            if header:
                yield header, frames
            header, frames = None, []
        elif header:
            match = FRAME.match(line)
            if match:
                frames.append(match.group(1))

def summarize(out, rows, budget_ms, bounds=None):
    if not rows or any(b['tick'] != a['tick'] + 1 for a, b in zip(rows, rows[1:])):
        raise ValueError('missing or discontinuous exact source ticks')
    if any(type(r.get('wall_us')) is not int or r['wall_us'] < 0 or
           type(r.get('wall_start_unix_ns')) is not int or r['wall_start_unix_ns'] <= 0 for r in rows):
        raise ValueError('invalid exact source wall clock/duration')
    if any(b['wall_start_unix_ns'] <= a['wall_start_unix_ns'] for a,b in zip(rows, rows[1:])):
        raise ValueError('source wall clock regressed')
    first = rows[0]['wall_start_unix_ns']
    end = rows[-1]['wall_start_unix_ns'] + rows[-1]['wall_us'] * 1000
    if bounds:
        first, end = bounds
    totals = collections.Counter()
    inclusive = collections.Counter()
    categories = collections.Counter()
    count = 0
    stack_count = 0
    for data in sorted(out.glob('perf-*.data')):
        exits = json.loads(data.with_suffix('.exit.json').read_text())
        # capture() stops perf with SIGINT after the server's clean exit.
        if exits.get('server') != 0 or exits.get('perf') not in (0, -signal.SIGINT):
            raise ValueError(f'failed capture: {data}: {exits}')
        meta = json.loads(data.with_suffix('.json').read_text())
        offset = meta['unix_minus_monotonic_ns']
        start_s, end_s = (first-offset)/1e9, (end-offset)/1e9
        cmd = ['perf', 'script', '-i', str(data), '--ns', '--time', f'{start_s:.9f},{end_s:.9f}', '-F', 'comm,time,period,event,ip,sym,dso', '--no-inline']
        text = subprocess.check_output(cmd, text=True)
        (out / (data.stem + '.script')).write_text(text)
        for head, frames in samples(text.splitlines()):
            if not frames:
                raise ValueError('perf sample lacks a symbol frame')
            period = int(head['period'])
            count += 1
            stack_count += len(frames) > 1
            totals[frames[0]] += period
            for frame in set(frames):
                inclusive[frame] += period
            operations = [classification_name(frame) for frame in frames]
            for name, pattern in FLAGS.items():
                if any(pattern.search(operation) for operation in operations):
                    categories[name] += period
    if not count or not stack_count:
        raise ValueError('window has no perf samples with call stacks')
    n = len(rows)
    wall = sorted(r['wall_us']/1000 for r in rows)
    report = {'ticks': [rows[0]['tick'], rows[-1]['tick']], 'samples': n,
              'wall_ms_mean': sum(wall)/n, 'wall_ms_p95': wall[max(0, int(n*.95+.999999)-1)],
              'budget_ms': budget_ms, 'cpu_ms_per_tick': sum(totals.values())/1e6/n,
              'perf_samples': count, 'stack_samples': stack_count,
              'self': [{'function': k, 'ms_per_tick': v/1e6/n} for k,v in totals.most_common(40)],
              'inclusive': [{'function': k, 'ms_per_tick': v/1e6/n} for k,v in inclusive.most_common(40)],
              'flagged_inclusive_ms_per_tick': {k:categories[k]/1e6/n for k in FLAGS}}
    report['status'] = 'pass' if report['wall_ms_p95'] <= budget_ms else 'budget_exceeded'
    dump(out / 'profile.json', report)
    return report

def source_windows(out, kind, *, diagnostic=False):
    work = out / "workload"
    if kind == "battle":
        result = json.loads((work / "result.json").read_text())
        take = json.loads((work / "take.json").read_text())
        if result.get("status") != "complete" or not take.get("clean"):
            raise ValueError("battle workload failed or is not clean")
        rows = [json.loads(line) for line in (work / "telemetry.jsonl").read_text().splitlines()]
        lo, hi = take["window"]["first_tick"], take["window"]["last_tick"]
        return {"battle": ([r for r in rows if lo <= r["tick"] <= hi], None)}
    if kind == "coreperf":
        final = json.loads((work / "result.json").read_text())
        if final.get("status") != "measured" and not diagnostic:
            raise ValueError("coreperf workload is not measured (prepared/smoke is not acceptance)")
        result = json.loads((work / "run/result.json").read_text())
        path = work / "run/samples.jsonl"
        bounds = {"measurement": (result["start_unix_ns"], result["end_unix_ns"])}
    else:
        paths = list(work.glob("*/rung-*/result.json"))
        if len(paths) != 1:
            raise ValueError("profile exactly one catalog scenario/rung")
        result = json.loads(paths[0].read_text())
        if (result.get("outcome") != "PASS" or result.get("smoke_only")) and not diagnostic:
            raise ValueError("catalog workload is not a non-smoke PASS")
        path = paths[0].parent / "samples.jsonl"
        bounds = {name: (v["start_unix_ns"], v["end_unix_ns"]) for name,v in result["windows"].items()}
    rows = []
    for line in path.read_text().splitlines():
        rows.extend(json.loads(line)["metrics"]["samples"])
    return {name: ([r for r in rows if lo <= r["wall_start_unix_ns"] < hi], (lo,hi))
            for name,(lo,hi) in bounds.items()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--analyze", action="store_true", help="reanalyze a completed capture, without rerunning its workload")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--kind", choices=("battle", "coreperf", "catalog"))
    parser.add_argument("--budget-ms", type=float, default=50.0)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if (not command and not args.analyze) or not args.out.is_absolute():
        parser.error("an explicit workload command (or --analyze) and absolute output path are required")
    if args.capture:
        return capture(args.out, command)
    if not args.kind or not 0 < args.budget_ms < float("inf"):
        parser.error("--kind and a finite positive --budget-ms are required")
    result = {"status": "failed", "kind": args.kind, "command": command}
    try:
        if not args.analyze:
            run(args.out, command)
        command_exit = json.loads((args.out / "command-exit.json").read_text())
        failed = command_exit["exit"] != 0
        if failed:
            result.update(diagnostic_only=True, error=f'workload exited {command_exit["exit"]}')
        if not command_exit["mid_snapshot"]:
            raise ValueError("missing mid-window neighbour snapshot")
        reports = {}
        for name,(rows,bounds) in source_windows(args.out, args.kind, diagnostic=failed).items():
            reports[name] = summarize(args.out, rows, args.budget_ms, bounds)
        result["windows"] = reports
        if not failed:
            result["status"] = "pass" if reports and all(r["status"] == "pass" for r in reports.values()) else "budget_exceeded"
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        result["error"] = f"{type(error).__name__}: {error}"
    if args.out.is_dir():
        dump(args.out / ("profile-analysis.json" if args.analyze else "profile-result.json"), result)
    print(json.dumps(result))
    return int(result["status"] != "pass")

if __name__ == "__main__":
    raise SystemExit(main())
