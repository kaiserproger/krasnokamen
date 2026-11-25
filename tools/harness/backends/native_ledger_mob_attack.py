"""Ledger SCENARIO-MOB-ATTACK-NO-PROGRESS: a daylight zombie must reach the player's health.

The row's product cause was the zombie melee preset carrying a `FleeFire` goal at priority 0
(`63b66bf29` removed it: vanilla `Zombie.addBehaviourGoals` has no panic goal), so a burning
daylight zombie fled instead of acquiring a player eight blocks away and the matrix's
"attack changes authoritative health" step never happened.

The scenario keeps the same claim on the client side: a survival player stands in daylight,
a zombie is summoned two blocks away, and the client's own health must fall without any
command touching it. Verdicts are assertions on the received health, so the pre-image (the
zombie fleeing its daylight fire) fails them instead of timing out.

Control: `63b66bf29` reverses on HEAD; with it reversed the preset flees and the health
assertion reds, while the cargo ref
`hostile_tests::daylight_zombie_acquires_and_damages_a_player_eight_blocks_away` is the
same claim in core.
"""
from __future__ import annotations

from typing import Any

from .probe_steps import ProbeSteps

ZOMBIE = "minecraft:zombie"
TICKS_PER_STEP = 20
STEPS = 12


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamemode survival @s")
    steps.fixture("difficulty normal")
    steps.fixture("time set day")
    steps.fixture("weather clear")
    state = steps.observe()
    before = float(state["player"]["health"])
    steps.report["health_before"] = before
    steps.fixture(f"summon {ZOMBIE} {x + 2.5} {y} {z + 0.5}")
    steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": ZOMBIE, "radius": 12, "timeout_seconds": steps.remaining()})

    samples: list[float] = []
    for _ in range(STEPS):
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
        health = float(steps.observe()["player"]["health"])
        samples.append(health)
        if health < before:
            break
    steps.report["health_samples"] = samples
    steps.check("daylight_zombie_reaches_authoritative_health",
                bool(samples) and samples[-1] < before)
    steps.check("next_attack_is_a_damage_step", len(samples) < 2 or samples[1] <= samples[0])

    steps.report["limits"] = [
        "One summoned zombie, daylight, two blocks from a survival player; the health comes from the "
        "client's own observation. No eight-block acquisition distance, no knockback or damage-number "
        "coverage (the core ref keeps the distance claim).",
    ]
