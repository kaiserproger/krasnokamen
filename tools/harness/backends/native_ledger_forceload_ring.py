"""Ledger FORCELOAD-RING: a forced chunk's ring ticks blocks but freezes entities.

The row says the level-32 ring around a forced chunk simulates entities.  A client
that stands at the operator pad and forces a distant chunk keeps the ring out of its
own simulation range, so the only thing that can tick the ring entity is the ticket.
The scenario summons an item entity with `NoGravity` in the ring chunk (its `Pos` is
static either way) and reads `Age` twice through the client's own `data get entity`
command: the fixed tree never advances it, the pre-image does.

Verdicts are assertions; each read enables `gamerule send_command_feedback true` and
waits for a new `has the following entity data:` line, the same answer path the tags
gate uses.
"""
from __future__ import annotations

from typing import Any

from .probe_steps import ProbeSteps

DATA = "has the following entity data: "
# One chunk east of the forced chunk: the level-32 ring.
RING_OFFSET = 16
TICKS_PER_STEP = 10
WAIT_STEPS = 8


def _data_lines(state: dict[str, Any]) -> list[str]:
    return [line for line in state["recent_chat"]
            if isinstance(line, str) and DATA in line]


def _read_age(steps: ProbeSteps, uuid_text: str) -> str | None:
    """One `data get entity <uuid> Age` answer, newest chat line wins.

    The answer is success feedback, so the run enables
    `gamerule send_command_feedback true` first; `recent_chat` is newest-first.
    """
    before = len(_data_lines(steps.observe()))
    steps.fixture(f"data get entity {uuid_text} Age")

    def newer(state: dict[str, Any]) -> bool:
        return len(_data_lines(state)) > before

    try:
        state = steps.wait(newer, "age_answer_received", timeout_seconds=20)
    except TimeoutError:  # a refused or absent entity answers without the data line
        return None
    lines = _data_lines(state)
    return lines[0].split(DATA, 1)[1].strip() if lines else None


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    # A chunk far outside the client's own simulation range, forced by the ticket.
    base = {"x": x + 1536, "y": y, "z": z}
    ring = {"x": base["x"] + RING_OFFSET, "y": y, "z": z}
    steps.fixture("gamerule send_command_feedback true")
    steps.fixture(f"forceload add {base['x']} {base['z']}")
    steps.fixture(f"forceload add {ring['x']} {ring['z']}")
    # The item entity's Age advances only while its chunk simulates entities.
    # `UUID:[I;0,0,0,123456]` is 00000000-0000-0000-0000-0001e240, the p4 gate's form.
    entity_uuid = "00000000-0000-0000-0000-0001e240"
    steps.fixture(
        f"summon minecraft:item {ring['x'] + 0.5} {y + 1.0} {ring['z'] + 0.5} "
        '{Item:{id:"minecraft:stone",count:1},Age:0s,NoGravity:1b,UUID:[I;0,0,0,123456]}')
    first = _read_age(steps, entity_uuid)
    steps.check("ring_entity_exists", first is not None)
    steps.report["ring_age_first"] = first

    second = first
    for _ in range(WAIT_STEPS):
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
        second = _read_age(steps, entity_uuid)
        if second != first:
            break
    steps.report["ring_age_second"] = second
    # The fixed tree freezes the ring entity's lifetime clock; the pre-image advances it.
    steps.check("ring_entity_age_frozen", first is not None and second == first)

    steps.fixture(f"forceload remove {base['x']} {base['z']}")
    steps.fixture(f"forceload remove {ring['x']} {ring['z']}")
    steps.report["limits"] = [
        "One forced chunk and its adjacent ring chunk 96 chunks from the player; one NoGravity item "
        "entity whose Age is read twice through the client's own `data get entity` command. No block "
        "half (the row's cargo ref keeps forceload_tests.rs:305), no second entity and no chunk "
        "unload/reload.",
    ]
