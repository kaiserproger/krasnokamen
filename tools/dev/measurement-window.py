#!/usr/bin/env python3
"""Decide whether `mem-run -x` may open its quiet measurement window now.

The existing tools/dev/measurement-windows allowlist accepts local daily
`HH:MM-HH:MM` and one-off `YYYY-MM-DDTHH:MM/YYYY-MM-DDTHH:MM` intervals.
Dated intervals may cross midnight and never grant the same hours on other
nights. `-x` pauses builds and waits for tests/servers, so admission is checked
before taking its locks. Exit 0 inside a window; 75 outside it.
`--now HH:MM` or `--now YYYY-MM-DDTHH:MM` pins the local clock for checks.
"""

from __future__ import annotations

import argparse
import datetime as dt
import re
import sys
from pathlib import Path

DEFAULT_CONFIG = Path(__file__).with_name("measurement-windows")
LINE = re.compile(r"^(\d{1,2}):(\d{2})-(\d{1,2}):(\d{2})$")
DATED = re.compile(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2})/(\d{4}-\d{2}-\d{2}T\d{2}:\d{2})$")
REFUSED = 75


def parse_config(text: str, path: Path, today: dt.date) -> list[tuple[dt.datetime, dt.datetime]]:
    windows = []
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        dated = DATED.fullmatch(line)
        match = LINE.fullmatch(line)
        try:
            if dated:
                start, end = (dt.datetime.fromisoformat(part) for part in dated.groups())
                if start >= end:
                    raise ValueError("a window must start before it ends")
                windows.append((start, end))
            elif match:
                h1, m1, h2, m2 = (int(part) for part in match.groups())
                start, end = dt.time(h1, m1), dt.time(h2, m2)
                if start >= end:
                    raise ValueError("a daily window must start before it ends")
                for day in (today, today + dt.timedelta(days=1)):
                    windows.append((dt.datetime.combine(day, start), dt.datetime.combine(day, end)))
            else:
                raise ValueError("expected HH:MM-HH:MM or YYYY-MM-DDTHH:MM/YYYY-MM-DDTHH:MM")
        except ValueError as error:
            raise SystemExit(f"{path}:{number}: {error}: {line!r}") from error
    if not windows:
        raise SystemExit(f"{path}: no measurement windows configured")
    return sorted(windows)


def clock(text: str | None) -> dt.datetime:
    if text is None:
        return dt.datetime.now()
    if "T" in text:
        return dt.datetime.fromisoformat(text)
    hour, minute = text.split(":")
    return dt.datetime.combine(dt.date.today(), dt.time(int(hour), int(minute)))


def decide(windows: list[tuple[dt.datetime, dt.datetime]], now: dt.datetime) -> tuple[bool, dt.datetime | None, int]:
    """Return admission, next start and whole minutes until that start."""
    for start, end in windows:
        if start <= now < end:
            return True, start, 0
    nxt = next((start for start, _ in windows if start > now), None)
    return False, nxt, int((nxt - now).total_seconds() // 60) if nxt else 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG, help="window file (default: beside this script)")
    parser.add_argument("--now", help="local HH:MM or YYYY-MM-DDTHH:MM instead of the current time (checks)")
    args = parser.parse_args(argv)
    now = clock(args.now)
    windows = parse_config(args.config.read_text(), args.config, now.date())
    inside, nxt, wait = decide(windows, now)
    if inside:
        print(f"mem-run: -x: inside a measurement window ({args.config}, now {now:%Y-%m-%d %H:%M})", file=sys.stderr)
        return 0
    if nxt is None:
        print(f"mem-run: -x refused at {now:%Y-%m-%d %H:%M}: no future window remains in {args.config}; Main must update the existing schedule.", file=sys.stderr)
        return REFUSED
    end = next(end for start, end in windows if start == nxt)
    hours, mins = divmod(wait, 60)
    print(
        f"mem-run: -x refused at {now:%Y-%m-%d %H:%M}: quiet measurement windows are scheduled in {args.config}. "
        f"Next window {nxt:%H:%M}-{end:%H:%M} in {hours}h{mins:02d}m, local dates {nxt:%Y-%m-%d} to {end:%Y-%m-%d}; "
        f"rerun then (a release measurement takes the pin of the commit, see tools/dev/pin-release). "
        f"Main may override one run with KRASNOKAMEN_WINDOW_OVERRIDE=1.",
        file=sys.stderr,
    )
    return REFUSED


if __name__ == "__main__":
    sys.exit(main())
