#!/usr/bin/env python3
"""Stand-in shots for a pipeline dry run: each shot's llvmpipe preview stills held for its duration.

Writes NAME.mp4 (60 fps, burned-in shot name, shot frame and predicted tick) and
NAME.frames.jsonl per shot into --out, so the stand-ins feed `compose.py build` like
bv-render output. Not battle footage.
"""
import argparse
import json
import pathlib
import subprocess

import compose


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--shots-file', required=True)
    p.add_argument('--first-tick', required=True, type=int)
    p.add_argument('--previews', required=True, help='directory with NAME-p0.png, NAME-p1.png, NAME-p2.png')
    p.add_argument('--out', required=True, help='new directory')
    a = p.parse_args()
    out = pathlib.Path(a.out)
    out.mkdir(parents=True, exist_ok=False)
    for shot in compose.parse_shots(a.shots_file):
        ticks = compose.predicted_ticks(shot, a.first_tick)
        with open(out / f'{shot.name}.frames.jsonl', 'w', encoding='utf-8') as f:
            for frame, tick in enumerate(ticks):
                f.write(json.dumps({'frame': frame, 'last_applied_tick': tick}) + '\n')
        stills = [pathlib.Path(a.previews) / f'{shot.name}-p{i}.png' for i in range(3)]
        stills = [s for s in stills if s.exists()]
        if not stills:
            raise SystemExit(f'no preview stills for {shot.name}')
        # Equal-length thirds, remainder frames go to the last still.
        counts = [len(ticks) // len(stills)] * len(stills)
        counts[-1] += len(ticks) - sum(counts)
        cmd = ['ffmpeg', '-hide_banner', '-nostdin', '-n']
        for still, count in zip(stills, counts):
            cmd += ['-framerate', str(compose.FPS), '-loop', '1', '-i', str(still)]
        n = len(stills)
        start_tick = ticks[0]
        label = (f'{shot.name}  frame %{{eif\\:n\\:d}}  tick %{{eif\\:{start_tick}+floor(n*{compose.TICKS_PER_SECOND}/{compose.FPS})\\:d}}')
        graph = (''.join(f'[{i}:v]scale=1920:1080:flags=lanczos,setsar=1,trim=end_frame={counts[i]},setpts=N/{compose.FPS}/TB[i{i}];' for i in range(n))
                 + ''.join(f'[i{i}]' for i in range(n)) + f'concat=n={n}:v=1:a=0,'
                 f"drawtext=fontfile={compose.FONT}:text='{label}':expansion=normal:fontcolor=yellow:fontsize=40:"
                 'x=40:y=h-80:box=1:boxcolor=black@0.6[v]')
        cmd += ['-filter_complex', graph, '-map', '[v]', '-frames:v', str(len(ticks)), '-r', str(compose.FPS),
                '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '18', '-pix_fmt', 'yuv420p', str(out / f'{shot.name}.mp4')]
        subprocess.run(cmd, check=True)


if __name__ == '__main__':
    main()
