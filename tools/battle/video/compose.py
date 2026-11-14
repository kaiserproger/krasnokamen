#!/usr/bin/env python3
"""Crossfade two replay shots, composite straight-alpha telemetry and encode."""
import argparse
import subprocess


def escape_filter_path(path):
    # Filtergraph syntax is parsed separately from argv by libavfilter.
    return path.replace('\\', '\\\\').replace(':', '\\:').replace("'", "'\\\\\\''").replace(',', '\\,').replace('[', '\\[').replace(']', '\\]')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--shot-a', required=True)
    p.add_argument('--shot-b', required=True)
    p.add_argument('--overlay', required=True, help='PNG sequence pattern, e.g. overlay-%%06d.png')
    p.add_argument('--music', required=True)
    p.add_argument('--title-file', required=True, help='UTF-8 text file, avoids drawtext expression injection')
    p.add_argument('--shot-a-seconds', required=True, type=float)
    p.add_argument('--duration', required=True, type=float, help='total output seconds')
    p.add_argument('--crossfade', type=float, default=1)
    p.add_argument('--resolution', choices=('1080p', '4k'), required=True)
    p.add_argument('--output', required=True)
    a = p.parse_args()
    if not 0 < a.crossfade < a.shot_a_seconds < a.duration:
        p.error('require 0 < crossfade < shot-a-seconds < duration')
    width, height = ((1920,1080) if a.resolution == '1080p' else (3840,2160))
    fade_start = a.shot_a_seconds-a.crossfade
    # Overlay is rendered at native 480x260, scaled 2x for 4K.
    panel = 1 if a.resolution == '1080p' else 2
    title = escape_filter_path(a.title_file)
    f = (f'[0:v]fps=60,scale={width}:{height}:flags=lanczos,setsar=1,settb=AVTB,setpts=PTS-STARTPTS[a];'
         f'[1:v]fps=60,scale={width}:{height}:flags=lanczos,setsar=1,settb=AVTB,setpts=PTS-STARTPTS[b];'
         f'[a][b]xfade=transition=fade:duration={a.crossfade}:offset={fade_start}[cut];'
         f'[2:v]format=rgba,scale=iw*{panel}:ih*{panel}:flags=neighbor[panel];'
         f'[cut][panel]overlay=x=W-w-32:y=32:alpha=straight:eof_action=endall,'
         f"drawtext=fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf:textfile='{title}':expansion=none:"
         f"fontcolor=white:fontsize={48*panel}:x=(w-text_w)/2:y=h*0.78:enable='between(t,0,3)',"
         'scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,'
         'setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv[v];'
         '[3:a]loudnorm=I=-14:TP=-1:LRA=11[audio]')
    cmd = ['ffmpeg','-hide_banner','-nostdin','-n','-filter_complex_threads','1',
           '-i',a.shot_a,'-i',a.shot_b,'-framerate','60','-i',a.overlay,
           '-stream_loop','-1','-i',a.music,'-filter_complex',f,'-map','[v]','-map','[audio]',
           '-t',str(a.duration),'-r','60','-c:v','libx264','-threads','2','-preset','medium','-crf','16',
           '-profile:v','high','-bf','2','-g','30','-keyint_min','30','-sc_threshold','0',
           '-flags','+cgop','-pix_fmt','yuv420p','-color_primaries','bt709','-color_trc','bt709',
           '-colorspace','bt709','-color_range','tv','-c:a','aac','-b:a','384k','-ar','48000',
           '-movflags','+faststart',a.output]
    subprocess.run(cmd, check=True)


if __name__ == '__main__':
    main()
