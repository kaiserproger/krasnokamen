#!/usr/bin/env python3
"""ffmpeg command for an N-shot edit plan from edit.py: hard cuts, xfade transitions, straight-alpha overlay, titles, music.

Library only; run edit.py. Every shot is trimmed by source frame index and given exact 60 fps timestamps
(setpts=N/(60*TB)), never resampled, so output frame f is the frame edit.frames.jsonl row f describes.
"""

FONT = '/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf'
RESOLUTIONS = {'1080p': (1920, 1080), '4k': (3840, 2160)}
# Overlay is rendered natively at 480 px wide, scaled by an integer for 4K.
PANEL_SCALE = {'1080p': 1, '4k': 2}
TRANSITIONS = ('fade', 'fadeblack')
FPS = 60


def escape_filter_path(path):
    # Filtergraph syntax is parsed separately from argv by libavfilter.
    return str(path).replace('\\', '\\\\').replace(':', '\\:').replace("'", "'\\\\\\''").replace(',', '\\,').replace('[', '\\[').replace(']', '\\]')


def seconds(frames):
    return f'{frames / FPS:.9f}'


def filter_graph(plan, music_input, overlay_input):
    width, height = RESOLUTIONS[plan['resolution']]
    panel = PANEL_SCALE[plan['resolution']]
    steps = []
    for i, shot in enumerate(plan['shots']):
        steps.append(f"[{i}:v]trim=start_frame={shot['in']}:end_frame={shot['out']},settb=AVTB,setpts=N/({FPS}*TB),"
                     f'scale={width}:{height}:flags=lanczos,setsar=1,format=yuv420p[s{i}]')
    current, length = 's0', plan['shots'][0]['out'] - plan['shots'][0]['in']
    for i, shot in enumerate(plan['shots'][1:], 1):
        transition = plan['shots'][i - 1]['transition']
        label = f'm{i}'
        if transition['type'] == 'cut':
            steps.append(f'[{current}][s{i}]concat=n=2:v=1:a=0,settb=AVTB,setpts=N/({FPS}*TB)[{label}]')
        else:
            d = transition['frames']
            steps.append(f"[{current}][s{i}]xfade=transition={transition['type']}:duration={seconds(d)}:offset={seconds(length - d)},"
                         f'settb=AVTB,setpts=N/({FPS}*TB)[{label}]')
            length -= d
        length += shot['out'] - shot['in']
        current = label
    if length != plan['frames']:
        raise ValueError(f'plan length {plan["frames"]} != filtergraph length {length}')
    chain = [f'[{overlay_input}:v]format=rgba,scale=iw*{panel}:ih*{panel}:flags=neighbor[panel]',
             f'[{current}][panel]overlay=x=W-w-32:y=32:alpha=straight:eof_action=endall']
    drawtext = [f"drawtext=fontfile={FONT}:textfile='{escape_filter_path(t['file'])}':expansion=none:"
                f"fontcolor=white:fontsize={48 * panel}:x=(w-text_w)/2:y=h*0.78:enable='between(t,{t['start']},{t['end']})'"
                for t in plan['texts']]
    steps.append(chain[0])
    steps.append(','.join([chain[1]] + drawtext + ['scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,'
                 'setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv[v]']))
    steps.append(f'[{music_input}:a]atrim=duration={plan["frames"] / FPS:.6f},loudnorm=I=-14:TP=-1:LRA=11[audio]')
    return ';'.join(steps)


def build_command(plan, overlay_pattern, output):
    inputs = []
    for shot in plan['shots']:
        inputs += ['-i', shot['video']]
    overlay_input = len(plan['shots'])
    inputs += ['-framerate', str(FPS), '-i', overlay_pattern, '-stream_loop', '-1', '-i', plan['music']]
    return (['ffmpeg', '-hide_banner', '-nostdin', '-n', '-filter_complex_threads', '1'] + inputs +
            ['-filter_complex', filter_graph(plan, overlay_input + 1, overlay_input), '-map', '[v]', '-map', '[audio]',
             '-frames:v', str(plan['frames']), '-r', str(FPS), '-c:v', 'libx264', '-threads', '2', '-preset', 'medium',
             '-crf', '16', '-profile:v', 'high', '-bf', '2', '-g', '30', '-keyint_min', '30', '-sc_threshold', '0',
             '-flags:v', '+cgop+bitexact', '-fflags', '+bitexact', '-map_metadata', '-1', '-pix_fmt', 'yuv420p',
             '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv',
             '-c:a', 'aac', '-b:a', '384k', '-ar', '48000', '-movflags', '+faststart', output])
