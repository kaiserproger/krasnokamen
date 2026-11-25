"""Ledger DATAPACK-LIST-HOVER: `/datapack list` entries carry the pack description on hover.

The row says the listing prints joined plain names and discards `pack.mcmeta`
descriptions, so a user cannot inspect a pack by hovering it.  The scenario runs the
command as the operator and reads the client's own `recent_chat_components`: the entry
line must still name the pack, and it must carry a `show_text` hover whose rendered
payload is the pack description.  `recent_chat` alone flattens the line and drops it.

The verdicts are assertions after a bounded client-tick wait: the plain-name branch
still produces the line (so the first assertion is a fixture), while the hover
assertion is what a list without links reds.
"""
from __future__ import annotations

from typing import Any

from .probe_steps import ProbeSteps

TICKS_PER_STEP = 5
WAIT_STEPS = 12


def _line_with(state: dict[str, Any], needle: str) -> dict[str, Any] | None:
    rows = state.get("recent_chat_components") or []
    for row in rows:
        text = row.get("text")
        if isinstance(text, str) and needle in text:
            return row
    return None


def _hover_text(row: dict[str, Any] | None) -> str:
    if row is None:
        return ""
    for event in row.get("events") or []:
        if event.get("kind") == "hover" and event.get("action") == "show_text":
            value = event.get("value")
            if isinstance(value, str) and value.strip():
                return value
    return ""


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    steps.fixture("datapack list")
    line = None
    for _ in range(WAIT_STEPS):
        state = steps.observe()
        line = _line_with(state, "vanilla")
        if line is not None:
            break
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
    steps.report["datapack_list_line"] = line
    steps.check("datapack_list_names_the_vanilla_pack", line is not None)
    hover = _hover_text(line)
    steps.report["datapack_list_hover"] = hover
    # The row's symptom is the *discarded description*: a hover that only repeats
    # the pack name leaves a non-empty show_text too, so the render is checked as
    # the title line plus a second, different line, and the vanilla description is
    # named when the client's language table provides it (`Default` is the pack
    # title; its description is `The default data for Minecraft`).
    lines = [part.strip() for part in hover.splitlines() if part.strip()]
    steps.report["datapack_list_hover_lines"] = lines
    steps.check("datapack_list_entry_carries_a_description_hover",
                len(lines) >= 2 and lines[1] != lines[0])
    steps.check("datapack_list_hover_shows_the_vanilla_description",
                len(lines) >= 2 and lines[1] == "The default data for Minecraft")

    steps.report["limits"] = [
        "One `/datapack list` answer for the built-in vanilla pack; the description text is "
        "read from the client's own component projection, not from the server log. No enabled "
        "file pack, no scrolling past the first line and no click event (the row is the "
        "description hover).",
    ]
