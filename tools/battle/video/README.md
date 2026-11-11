# Battle telemetry overlay and N-shot edit

Python 3 standard library, ffmpeg and ffprobe with libx264, drawtext, xfade and AAC, and DejaVu Sans Bold at `/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf` are required. No wall-clock is read. Inputs are the E1 `/telemetry.jsonl` export and C3 `shot.frames.jsonl`/`shot.mp4` per shot. Use only licensed music.

```sh
python3 tools/battle/video/edit.py --spec /work/edit.json --out /work/edit-out        # --out must not exist
python3 tools/battle/video/edit.py --spec /work/edit.json --out /work/edit-dry --dry-run   # plan + command only
```

`edit.py` writes to `--out`: `plan.json` (resolved spec, output length), `edit.frames.jsonl` (one row per output frame), `ffmpeg-command.txt` (the exact command), `overlay/overlay-%06d.png` (one per output frame), `final.mp4`. A failed validation (including a tick missing from telemetry) exits before `--out` is created; a failed or interrupted encode leaves partial output, so use a new `--out` for each attempt. Same spec and inputs give byte-identical PNGs, `edit.frames.jsonl` and `final.mp4` (bitexact flags, fixed encoder threads; tested).

## Spec

```json
{
 "resolution": "1080p",
 "music": "music.wav",
 "texts": [{"file": "title.txt", "start": 0, "end": 3}],
 "shots": [
  {"id": "wide-a", "video": "wide.mp4", "frames": "wide.frames.jsonl", "telemetry": "telemetry.jsonl",
   "in": 0, "out": 240, "transition": {"type": "fade", "frames": 30}},
  {"id": "close", "video": "close.mp4", "frames": "close.frames.jsonl", "telemetry": "telemetry.jsonl", "in": 120, "out": 420},
  {"id": "settlement", "video": "village.mp4", "frames": "village.frames.jsonl", "in": 0, "out": 300}
 ]
}
```

Relative paths resolve against the spec's directory; unknown keys are errors. `resolution` is `1080p` or `4k`. `music` is looped, cut to the edit length and loudness-normalized. `texts` are literal UTF-8 files shown between `start` and `end` seconds (title, credits).

Shot `i` shows source frames `[in, out)`. Every shot video must be 60/1 fps and have exactly as many frames as its `frames.jsonl` has rows (numbered from 0, `last_applied_tick` integer and never decreasing); there is no implicit resampling. `transition` is the join into the NEXT shot: `{"type":"cut"}` (default) or `{"type":"fade"|"fadeblack","frames":N}`. The `N` frames lie inside both shots' ranges (last `N` of shot `i`, first `N` of shot `i+1`), so output frames = sum(`out`-`in`) - sum(`N`). A shot must fit its incoming plus outgoing transition; the last shot cannot have one.

`telemetry` switches the overlay on for that shot, from that shot's own E1 export (a settlement take has its own); omit it and the shot gets fully transparent overlay frames.

## Tick join (fail closed)

Output frame `f` takes its tick from `last_applied_tick` of the shot frame it shows (the last shutter sample when motion blur is enabled) and looks it up in that shot's telemetry; a tick missing there fails the edit and names the frame and shot. Nothing is interpolated or estimated. During a crossfade two source frames are visible: the outgoing shot owns output frame `j` of the transition while `2*j < N`, the incoming shot afterwards, and the overlay, shot name and tick are those of the owner. The other side is kept in the row's `blend` (`shot`, `source_frame`, `last_applied_tick`, `progress` as `j/N`). A shot without telemetry that owns a frame shows no overlay there. Retain `edit.frames.jsonl` with the video: it is the mapping from every output frame to source shot, source frame and tick.

## Overlay panel

180 tick history; green = full active `wall_us` MSPT on a fixed 0–100 ms scale, clipped at 100; amber 50 ms budget line; blue = instantaneous TPS on a 0–20 TPS scale, clipped at 20 (numeric TPS remains unclipped). Missing ticks create line gaps. `SRV RSS` is the resident memory of the server process (`host.process_rss_bytes`), held from the last host sample; the cgroup `memory.current` is never used, it also counts the recorder and anything else in the scope. The cyan strip below the text is that RSS over the last 1200 ticks (60 s) on a `0-N GIB` axis, N = the telemetry file's peak RSS rounded up to a whole GiB, fixed for the whole file. `CPU` is the cgroup of the server process as percent of one CPU (may exceed 100), derived from consecutive host samples, so it includes the recorder unless the server runs in its own scope; steal is from proc-stat deltas. Unavailable counters, including a null RSS, are `N/A`, not zero and not a fallback. `battle.units` is the number of faction rows, not the number alive; `battle.alive` supplies per-faction living counts. Faction IDs are read from the map, ordered numerically; absent battle data displays `N/A`, and more than two factions is an error. The 480x304 panel is placed top right, scaled 2x at 4K. Identical (telemetry, tick) pairs are rendered once.

`overlay.py --telemetry T --frames F --out DIR` still renders one PNG per C3 frame for a single shot without an edit.

Encode: H.264 High, 2 B frames, closed 30-frame GOP, CRF 16, AAC 384k/48 kHz, BT.709 limited-range tags, faststart, exactly the planned number of frames. Cuts are `concat`, fades are `xfade`, each stage re-timestamped to exact 60 fps (`setpts=N/(60*TB)`); `edit.py` ffprobes the result and fails if its frame count differs from the plan.

Tests: `python3 tools/battle/video/test_overlay.py` and `python3 tools/battle/video/test_edit.py` (the latter encodes a 3-shot synthetic edit twice and compares bytes). Synthetic input: `synthetic.py`. A synthetic or replay-receipt edit is not proof of live server performance.
