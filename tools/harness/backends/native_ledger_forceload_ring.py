"""Ledger FORCELOAD-RING: a forced chunk's ring ticks blocks but freezes entities.

The row says the level-32 ring around a forced chunk simulates entities. A client that
stands at the operator pad and forces a distant chunk keeps the ring out of its own
simulation range, so the only thing that can tick the ring entity is the ticket.

One `/forceload add` 96 chunks from the pad; a cow is summoned at y=200 in the ring chunk
(one chunk east) and in the centre chunk; `data get entity <selector> Pos` is read twice
per cow through the client's own command. On the fixed tree the ring cow keeps its
position and the centre cow falls — the centre fall is the positive control, so a build
that stops ticking forced chunks altogether cannot pass. The pre-image simulates the ring
too, and the ring cow falls out of its selector.

`summon` refuses `UUID` on this tree, so each cow is addressed by a positional selector;
the answer is success feedback, so the run enables `gamerule send_command_feedback true`
first and waits for a *new* `has the following entity data:` line (`recent_chat` is
newest-first), the same answer path the tags gate uses.
"""
from __future__ import annotations

from typing import Any

from .probe_steps import ProbeSteps

DATA = "has the following entity data: "
# One chunk east of the forced chunk: the level-32 ring.
RING_OFFSET = 16
TICKS_PER_STEP = 10
WAIT_STEPS = 8
HEIGHT = 200.0


def _data_lines(state: dict[str, Any]) -> list[str]:
    return [line for line in state["recent_chat"]
            if isinstance(line, str) and DATA in line]


def _read_position(steps: ProbeSteps, selector: str, label: str) -> str | None:
    """One `data get entity <selector> Pos` answer, newest chat line wins."""
    before = len(_data_lines(steps.observe()))
    steps.fixture(f"data get entity {selector} Pos")

    def newer(state: dict[str, Any]) -> bool:
        return len(_data_lines(state)) > before

    try:
        state = steps.wait(newer, label, timeout_seconds=20)
    except TimeoutError:  # a refused or absent entity answers without the data line
        return None
    lines = _data_lines(state)
    return lines[0].split(DATA, 1)[1].strip() if lines else None


def _selector(pos: dict[str, int]) -> str:
    return (f"@e[type=minecraft:cow,x={pos['x']},y={int(HEIGHT)},z={pos['z']},"
            "distance=..8,limit=1,sort=nearest]")


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
    # A cow in the air falls only while its chunk simulates entities; the row's own cargo
    # ref (forceload_tests.rs:288-305) uses exactly this discriminator.
    steps.fixture(f"summon minecraft:cow {ring['x'] + 0.5} {HEIGHT} {ring['z'] + 0.5}")
    steps.fixture(f"summon minecraft:cow {base['x'] + 0.5} {HEIGHT} {base['z'] + 0.5}")
    steps.report["chat_after_fixture"] = [
        line for line in steps.observe()["recent_chat"] if isinstance(line, str)][:10]
    ring_selector, centre_selector = _selector(ring), _selector(base)
    ring_first = _read_position(steps, ring_selector, "ring_position_received")
    centre_first = _read_position(steps, centre_selector, "centre_position_received")
    steps.report["ring_position_first"] = ring_first
    steps.report["centre_position_first"] = centre_first
    steps.check("ring_entity_exists", ring_first is not None)
    steps.check("centre_entity_exists", centre_first is not None)

    ring_second, centre_second = ring_first, centre_first
    for _ in range(WAIT_STEPS):
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
        ring_second = _read_position(steps, ring_selector, "ring_position_rechecked")
        centre_second = _read_position(steps, centre_selector, "centre_position_rechecked")
        if ring_second != ring_first or centre_second != centre_first:
            break
    steps.report["ring_position_second"] = ring_second
    steps.report["centre_position_second"] = centre_second
    # The fixed tree leaves the ring cow in the air; the pre-image lets it fall.
    steps.check("ring_entity_position_frozen", ring_first is not None and ring_second == ring_first)
    # Positive control: the centre chunk must tick, or the gate would pass on a build
    # that stopped ticking forced chunks at all.
    fell = centre_first is not None and centre_second != centre_first
    steps.check("centre_entity_still_falls", fell)

    steps.fixture(f"forceload remove {base['x']} {base['z']}")
    steps.report["limits"] = [
        "One forced chunk and its adjacent ring chunk 96 chunks from the player, one cow in each, "
        "`Pos` read twice per cow through the client's own `data get entity` command; no block half "
        "(the row's cargo ref keeps forceload_tests.rs:305), no third entity and no chunk "
        "unload/reload.",
    ]
