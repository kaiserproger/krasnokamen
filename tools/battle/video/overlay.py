#!/usr/bin/env python3
"""Render deterministic, straight-alpha PNGs joined to C3 frames by server tick."""
import argparse
import json
import math
import pathlib
import struct
import zlib
from collections import deque

W, H = 480, 260
# Five-by-seven uppercase glyphs, rows encoded as bits.
FONT = {
 '0':['01110','10001','10011','10101','11001','10001','01110'], '1':['00100','01100','00100','00100','00100','00100','01110'], '2':['01110','10001','00001','00010','00100','01000','11111'], '3':['11110','00001','00001','01110','00001','00001','11110'], '4':['00010','00110','01010','10010','11111','00010','00010'], '5':['11111','10000','10000','11110','00001','00001','11110'], '6':['01110','10000','10000','11110','10001','10001','01110'], '7':['11111','00001','00010','00100','01000','01000','01000'], '8':['01110','10001','10001','01110','10001','10001','01110'], '9':['01110','10001','10001','01111','00001','00001','01110'],
 'A':['01110','10001','10001','11111','10001','10001','10001'], 'B':['11110','10001','10001','11110','10001','10001','11110'], 'C':['01111','10000','10000','10000','10000','10000','01111'], 'D':['11110','10001','10001','10001','10001','10001','11110'], 'E':['11111','10000','10000','11110','10000','10000','11111'], 'F':['11111','10000','10000','11110','10000','10000','10000'], 'G':['01111','10000','10000','10111','10001','10001','01111'], 'H':['10001','10001','10001','11111','10001','10001','10001'], 'I':['111','010','010','010','010','010','111'], 'J':['00111','00010','00010','00010','10010','10010','01100'], 'K':['10001','10010','10100','11000','10100','10010','10001'], 'L':['10000','10000','10000','10000','10000','10000','11111'], 'M':['10001','11011','10101','10101','10001','10001','10001'], 'N':['10001','11001','10101','10011','10001','10001','10001'], 'O':['01110','10001','10001','10001','10001','10001','01110'], 'P':['11110','10001','10001','11110','10000','10000','10000'], 'Q':['01110','10001','10001','10001','10101','10010','01101'], 'R':['11110','10001','10001','11110','10100','10010','10001'], 'S':['01111','10000','10000','01110','00001','00001','11110'], 'T':['11111','00100','00100','00100','00100','00100','00100'], 'U':['10001','10001','10001','10001','10001','10001','01110'], 'V':['10001','10001','10001','10001','10001','01010','00100'], 'W':['10001','10001','10001','10101','10101','10101','01010'], 'X':['10001','10001','01010','00100','01010','10001','10001'], 'Y':['10001','10001','01010','00100','00100','00100','00100'], 'Z':['11111','00001','00010','00100','01000','10000','11111'],
 '.':['0','0','0','0','0','0','1'], ':':['0','1','0','0','1','0','0'], '-':['0','0','0','1','0','0','0'], '/':['00001','00001','00010','00100','01000','10000','10000'], '%':['10001','00010','00100','01000','10001','00000','00000'], '#':['01010','11111','01010','01010','11111','01010','00000'], ' ':['0']*7,
}

def rows(path):
    with open(path, encoding='utf-8') as f:
        for n, line in enumerate(f, 1):
            try:
                yield json.loads(line)
            except ValueError as e:
                raise ValueError(f'{path}:{n}: {e}') from e

def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))

def png(pixels):
    scan = b''.join(b'\0' + pixels[y*W*4:(y+1)*W*4] for y in range(H))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', W, H, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(scan, 6)) + chunk(b'IEND', b'')

def rect(p, x, y, w, h, color):
    x0, y0, x1, y1 = max(0, x), max(0, y), min(W, x+w), min(H, y+h)
    if x1 <= x0 or y1 <= y0:
        return
    row = bytes(color) * (x1-x0)
    for yy in range(y0, y1):
        p[(yy*W+x0)*4:(yy*W+x1)*4] = row

def text(p, x, y, value, color=(235,241,249,255), scale=2):
    for char in value.upper():
        glyph = FONT[char]
        for yy, line in enumerate(glyph):
            for xx, bit in enumerate(line):
                if bit == '1':
                    rect(p, x+xx*scale, y+yy*scale, scale, scale, color)
        x += (len(glyph[0])+1)*scale

def line(p, x0, y0, x1, y1, color):
    dx, dy = abs(x1-x0), -abs(y1-y0)
    sx, sy = (1 if x0 < x1 else -1), (1 if y0 < y1 else -1)
    err = dx+dy
    while True:
        rect(p, x0, y0, 1, 1, color)
        if x0 == x1 and y0 == y1:
            break
        e = 2*err
        if e >= dy: err, x0 = err+dy, x0+sx
        if e <= dx: err, y0 = err+dx, y0+sy

def graph_y(value, maximum):
    return 172-round(min(max(value, 0), maximum)*100/maximum)

def render(sample, history, cpu, steal, memory):
    p = bytearray(W*H*4)
    rect(p, 0, 0, W, H, (7, 16, 28, 205))
    text(p, 12, 10, f"TICK #{sample['tick']}  REPLAY TELEMETRY", scale=2)
    ms = sample['wall_us']/1000
    tps = history[-1][1]
    text(p, 12, 32, f"MSPT {ms:.1f}  TPS {tps:.1f}" if tps is not None else f"MSPT {ms:.1f}  TPS N/A")
    # Fixed 0..100 ms; 50 ms must remain at y=122 regardless of outliers.
    rect(p, 12, 72, 360, 101, (17, 30, 43, 230))
    budget_y = graph_y(50, 100)
    rect(p, 12, budget_y, 360, 1, (255,190,65,255))
    text(p, 379, budget_y-5, '50 MS', (255,190,65,255), 1)
    data = list(history)[-180:]
    for index in range(1, len(data)):
        if data[index][2] != data[index-1][2]+1: continue
        x0, x1 = 12+(index-1)*2, 12+index*2
        line(p, x0, graph_y(data[index-1][0],100), x1, graph_y(data[index][0],100), (79,220,171,255))
        if data[index-1][1] is not None and data[index][1] is not None:
            line(p, x0, graph_y(data[index-1][1],20), x1, graph_y(data[index][1],20), (99,165,255,255))
    text(p, 12, 182, 'GREEN MSPT  BLUE TPS', scale=2)
    fmt = lambda x: 'N/A' if x is None else f'{x:.1f}'
    text(p, 12, 201, f'MEM {fmt(memory)} GIB  CPU {fmt(cpu)}%', scale=2)
    text(p, 12, 220, f'STEAL {fmt(steal)}%', scale=2)
    battle = sample.get('battle') or {}
    alive = battle.get('alive') or {}
    factions = sorted(alive, key=int)
    if len(factions) > 2:
        raise ValueError('overlay supports two factions; telemetry contains more')
    sides = '  '.join(f'SIDE {key} {alive[key]}' for key in factions) or 'SIDES N/A'
    text(p, 12, 239, f"UNITS {battle.get('units', 'N/A')}  {sides}", scale=2)
    return png(p)

def load(telemetry):
    result = {}
    history = deque(maxlen=180)
    previous = previous_host = None
    memory = cpu = steal = None
    for s in rows(telemetry):
        tick, wall = s['tick'], s['wall_start_unix_ns']
        if previous and (tick <= previous['tick'] or wall <= previous['wall_start_unix_ns']):
            raise ValueError('telemetry ticks and wall_start_unix_ns must strictly increase')
        tps = None
        if previous:
            tps = (tick-previous['tick'])*1e9/(wall-previous['wall_start_unix_ns'])
        history.append((s['wall_us']/1000, tps, tick))
        host = s['host']
        if host is not None:
            mem = host['cgroup_memory_current_bytes']
            memory = None if mem is None else mem/1024**3
            cpu = steal = None
            if previous_host:
                dt = (host['sampled_unix_ns']-previous_host['sampled_unix_ns'])/1000
                a, b = host['cgroup_cpu_usage_usec'], previous_host['cgroup_cpu_usage_usec']
                if dt > 0 and a is not None and b is not None and a >= b: cpu = (a-b)/dt*100
                a, b = host['cpu_steal_ticks'], previous_host['cpu_steal_ticks']
                c, d = host['cpu_total_ticks'], previous_host['cpu_total_ticks']
                if None not in (a,b,c,d) and c > d and a >= b: steal = (a-b)/(c-d)*100
            previous_host = host
        result[tick] = (s, tuple(history), cpu, steal, memory)
        previous = s
    return result

def main():
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument('--telemetry', required=True)
    a.add_argument('--frames', required=True)
    a.add_argument('--out', required=True, help='new output directory of overlay-%%06d.png')
    args = a.parse_args()
    telemetry = load(args.telemetry)
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=False)
    for expected, frame in enumerate(rows(args.frames)):
        if frame['frame'] != expected: raise ValueError(f'expected frame {expected}, got {frame["frame"]}')
        tick = frame['last_applied_tick']
        if tick not in telemetry: raise ValueError(f'frame {expected}: tick {tick} missing from telemetry')
        (out / f'overlay-{expected:06d}.png').write_bytes(render(*telemetry[tick]))

if __name__ == '__main__':
    main()
