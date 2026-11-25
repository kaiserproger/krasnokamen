"""Ledger FORCELOAD-RING: a forced chunk's ring ticks blocks but freezes entities.

The row says the level-32 ring around a forced chunk simulates entities.  A client
that stands at the operator pad and forces a distant chunk keeps the ring out of its
own simulation range, so the only thing that can tick the ring entity is the ticket.
The scenario summons an item entity with `NoGravity` in the ring chunk (its `Pos` is
static either way) and reads `Age` twice through the client's own `data get entity`
command: the fixed tree never advances it, the pre-image does.

Verdicts are assertions; the two reads use the summon-nbt gate's answer idiom (a chat
marker after the command, the `has the following entity data:` line before it).
"""
from __future__ import annotations

import uuid
from typing import Any

from .probe_steps import ProbeSteps

DATA = "has the following entity data: "
# One chunk east of the forced chunk: the level-32 ring.
RING_OFFSET = 16
TICKS_PER_STEP = 10
WAIT_STEPS = 8


def _read_age(steps: ProbeSteps, marker: str, uuid_text: str) -> str | None:
    """One `data get entity <uuid> Age` answer, fenced by a chat marker."""
    steps.call("minecraft_send_chat", {"message": marker, "command": False})
    steps.wait(lambda s: any(isinstance(line, str) and marker in line
                            for line in s["recent_chat"]), f"marker_{marker.split()[-1]}")
    steps.fixture(f"data get entity {uuid_text} Age")

    def reply(state: dict[str, Any]) -> str | None:
        chat = [line for line in state["recent_chat"] if isinstance(line, str)]
        newer = chat[:next((i for i, line in enumerate(chat) if marker in line), 0)]
        return next((line for line in reversed(newer) if DATA in line), None)

    seen = steps.wait(lambda s: reply(s) is not None, f"age_read_{marker.split()[-1]}")
    line = reply(seen)
    return None if line is None else line.split(DATA, 1)[1].strip()


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    # A chunk far outside the client's own simulation range, forced by the ticket.
    base = {"x": x + 1536, "y": y, "z": z}
    ring = {"x": base["x"] + RING_OFFSET, "y": y, "z": z}
    steps.fixture(f"forceload add {base['x']} {base['z']}")
    steps.fixture(f"forceload add {ring['x']} {ring['z']}")
    # The item entity's Age advances only while its chunk simulates entities.
    # `UUID:[I;0,0,0,123456]` is 00000000-0000-0000-0000-0001e240, the p4 gate's form.
    entity_uuid = "00000000-0000-0000-0000-0001e240"
    steps.fixture(
        f"summon minecraft:item {ring['x'] + 0.5} {y + 1.0} {ring['z'] + 0.5} "
        '{Item:{id:"minecraft:stone",count:1},Age:0s,NoGravity:1b,UUID:[I;0,0,0,123456]}')
    first = _read_age(steps, f"ring age first {uuid.uuid4().hex}", entity_uuid)
    steps.check("ring_entity_exists", first is not None)
    steps.report["ring_age_first"] = first

    second = first
    for _ in range(WAIT_STEPS):
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
        second = _read_age(steps, f"ring age next {uuid.uuid4().hex}", entity_uuid)
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
