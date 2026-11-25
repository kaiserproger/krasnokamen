"""Ledger POTION-CUSTOM-PAYLOAD: a lingering cloud applies its authored custom effect.

The row says a lingering potion ignores its authored `custom_effects`.  The
scenario gives the operator a lingering potion whose only payload is one custom
`minecraft:levitation` effect (no base potion at all), throws it straight down at
the player's feet, and reads the client's own position: an applied levitation
lifts the player, an ignored one leaves them standing.  The verdict is an
assertion after a bounded client-tick wait, never the wait's own expiry.

The impact colour half of the row is a level-event value the client renderer
consumes and no observation exposes; it rests on the row's cargo ref
(`lingering_custom_payload_and_impact_color_are_preserved`, which asserts the
`ProjectileLevelEvent` data `0x123456`).
"""
from __future__ import annotations

from .native_p1 import _inventory
from .probe_steps import ProbeSteps, item_count

POTION = "minecraft:lingering_potion"
LEVITATION = "minecraft:levitation"
# The cloud divides a custom effect's duration by four and drops anything under
# 20 ticks; 100 ticks therefore applies 25 ticks of levitation.
DURATION = 100
TICKS_PER_STEP = 10
WAIT_STEPS = 12


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin[0] + 0.5, origin[1], origin[2] + 0.5
    steps.fixture("gamemode survival @s")
    steps.fixture("effect clear @s")
    steps.fixture("clear @s")
    steps.fixture(f"tp @s {x} {y} {z}")
    steps.fixture(
        f"give @s {POTION}[minecraft:potion_contents={{custom_effects:"
        f'[{{id:"{LEVITATION}",amplifier:0b,duration:{DURATION}}}]}}]')
    _inventory(steps, POTION, 1)
    steps.call("minecraft_select_hotbar_item",
               {"item_id": POTION, "count": 1, "timeout_seconds": steps.remaining()})
    steps.call("minecraft_look", {"yaw_deg": 0, "pitch_deg": 90})
    # Let the spawn settle so the baseline is a resting height, not a correction.
    steps.call("minecraft_wait_ticks", {"ticks": 10})
    baseline = steps.observe()["player"]["y"]
    steps.check("fixture_potion_held", item_count(steps.observe(), POTION) == 1)

    steps.call("minecraft_press_inputs", {"keys": ["use"], "ticks": 1})
    # The throw consumes the bottle; that the bottle left the hand is the fixture
    # half, and the cloud's effect is the assertion below.
    thrown = False
    for _ in range(WAIT_STEPS):
        if item_count(steps.observe(), POTION) == 0:
            thrown = True
            break
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
    steps.check("lingering_potion_was_thrown", thrown)

    lifted = False
    peak = baseline
    for _ in range(WAIT_STEPS):
        state = steps.observe()
        peak = max(peak, state["player"]["y"])
        if peak >= baseline + 0.8:
            lifted = True
            break
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
    steps.report["levitation_y"] = {"baseline": baseline, "peak": peak}
    steps.check("cloud_applied_the_authored_custom_effect", lifted)

    steps.fixture("effect clear @s")
    steps.fixture(f"tp @s {x} {y} {z}")
    steps.report["limits"] = [
        "One lingering potion with one authored custom effect and no base potion, thrown "
        "straight down; the effect is observed through the client's own lifted position. "
        "The custom impact colour is a level-event value no observation exposes and rests on "
        "the cargo ref. No amplifier mixture, no splash form and no second cloud.",
    ]
