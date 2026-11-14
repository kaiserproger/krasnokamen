#!/usr/bin/env python3
"""Decide whether `mem-run -x` may open its quiet measurement window now.

The schedule lives in tools/dev/measurement-windows (local time, one
`HH:MM-HH:MM` per line, `#` comments). A `-x` window pauses build-only
commands, waits for every running test and server to finish and holds new
commands at start, so it runs only inside a configured window (owner
2026-11-15). Exit 0 inside a window; exit 75 outside it with the next window
named. `--now HH:MM` pins the clock (tests); `--config` names another file.
"""

from __future__ import annotations

import argparse
import datetime as dt
import re
import sys
from pathlib import Path

DEFAULT_CONFIG = Path(__file__).with_name("measurement-windows")
LINE = re.compile(r"^(\d{1,2}):(\d{2})-(\d{1,2}):(\d{2})$")
REFUSED = 75


def parse_config(text: str, path: Path) -> list[tuple[dt.time, dt.time]]:
    windows = []
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        match = LINE.match(line)
        if match is None:
            raise SystemExit(f"{path}:{number}: expected HH:MM-HH:MM, got {line!r}")
        h1, m1, h2, m2 = (int(part) for part in match.groups())
        if h1 > 23 or h2 > 23 or m1 > 59 or m2 > 59:
            raise SystemExit(f"{path}:{number}: not a clock time: {line!r}")
        start, end = dt.time(h1, m1), dt.time(h2, m2)
        if start >= end:
            raise SystemExit(f"{path}:{number}: a window must start before it ends ({line})")
        windows.append((start, end))
    if not windows:
        raise SystemExit(f"{path}: no measurement windows configured")
    return sorted(windows)


def clock(text: str | None) -> dt.time:
    if text is None:
        return dt.datetime.now().time()
    hour, minute = text.split(":")
    return dt.time(int(hour), int(minute))


def decide(windows: list[tuple[dt.time, dt.time]], now: dt.time) -> tuple[bool, dt.time, int]:
    """(inside a window, next window start, minutes until it) for the local clock `now`."""
    minutes = lambda value: value.hour * 60 + value.minute  # noqa: E731
    for start, end in windows:
        if minutes(start) <= minutes(now) < minutes(end):
            return True, start, 0
    starts = sorted(start for start, _ in windows)
    later = [start for start in starts if minutes(start) > minutes(now)]
    nxt = later[0] if later else starts[0]
    return False, nxt, (minutes(nxt) - minutes(now)) % (24 * 60)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG, help="window file (default: beside this script)")
    parser.add_argument("--now", help="HH:MM local clock instead of the current time (tests)")
    args = parser.parse_args(argv)
    windows = parse_config(args.config.read_text(), args.config)
    now = clock(args.now)
    inside, nxt, wait = decide(windows, now)
    if inside:
        print(f"mem-run: -x: inside a measurement window ({args.config}, now {now:%H:%M})", file=sys.stderr)
        return 0
    end = next(end for start, end in windows if start == nxt)
    hours, mins = divmod(wait, 60)
    print(
        f"mem-run: -x refused at {now:%H:%M}: quiet measurement windows are scheduled in {args.config}. "
        f"Next window {nxt:%H:%M}-{end:%H:%M} in {hours}h{mins:02d}m; rerun then (a release measurement takes "
        f"the pin of the commit, see tools/dev/pin-release). Main may override one run with "
        f"KRASNOKAMEN_WINDOW_OVERRIDE=1.",
        file=sys.stderr,
    )
    return REFUSED


if __name__ == "__main__":
    sys.exit(main())
