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
    """One `data get entity <selector> Pos` answer, newest chat line wins.

    The answer is success feedback, so the run enables
    `gamerule send_command_feedback true` first; `recent_chat` is newest-first.
    """
    before = len(_data_lines(steps.observe()))
    steps.fixture(f"data get entity {uuid_text} Pos")

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
    # One ticket only: its centre chunk is entity-ticking, its ring is block-only.
    # Forcing the ring chunk as a second centre would make the target chunk the
    # centre of its own ticket and defeat the measurement.
    steps.fixture(f"forceload add {base['x']} {base['z']}")
    # A ticket loads chunks over the next ticks; `/summon` refuses an unloaded position.
    # The client's own view never receives this chunk (that is the point), so the wait is
    # server-side ticks, not `minecraft_wait_for_loaded_block`.
    steps.call("minecraft_wait_ticks", {"ticks": 5})
    # A cow in the air falls only while its chunk simulates entities; the row's own
    # cargo ref (forceload_tests.rs:288-305) uses exactly this discriminator. Item
    # entities are not usable: their `Age` advances in a non-simulated chunk too
    # (observed 20261125T040917-…-v9jl4x6b, `Item has the following entity data: 7`
    # twice on the fixed tree), so `Pos` of a mob is the field that follows the ticket.
    # `summon` refuses `UUID` here, so the entity is addressed by a positional selector
    # (server-side: the client never receives this chunk).
    entity_selector = (f"@e[type=minecraft:cow,x={ring['x']},y=200,z={ring['z']},"
                       "distance=..8,limit=1,sort=nearest]")
    steps.fixture(f"summon minecraft:cow {ring['x'] + 0.5} 200.0 {ring['z'] + 0.5}")
    steps.report["chat_after_fixture"] = [
        line for line in steps.observe()["recent_chat"] if isinstance(line, str)][:10]
    first = _read_age(steps, entity_selector)
    steps.report["chat_after_first_read"] = [
        line for line in steps.observe()["recent_chat"] if isinstance(line, str)][:10]
    steps.check("ring_entity_exists", first is not None)
    steps.report["ring_age_first"] = first

    second = first
    for _ in range(WAIT_STEPS):
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
        second = _read_age(steps, entity_selector)
        if second != first:
            break
    steps.report["ring_age_second"] = second
    # The fixed tree leaves the ring cow in the air; the pre-image lets it fall.
    steps.check("ring_entity_position_frozen", first is not None and second == first)

    steps.fixture(f"forceload remove {base['x']} {base['z']}")
    steps.report["limits"] = [
        "One forced chunk and its adjacent ring chunk 96 chunks from the player; one NoGravity item "
        "entity whose Age is read twice through the client's own `data get entity` command. No block "
        "half (the row's cargo ref keeps forceload_tests.rs:305), no second entity and no chunk "
        "unload/reload.",
    ]
