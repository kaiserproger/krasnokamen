"""Ledger TAGS-DUPLICATES: entity tags are a set, so one removal clears a duplicate.

The row says `Tags:['d1','d1']` retains both entries and one `/tag … remove` leaves
the tag present.  The scenario summons a cow with the duplicated element list, proves
the tag matches, removes the tag once, and then proves the tag is gone through a
player-visible `say` that only runs when the selector does not match.

Every line is the player's own command and the player's own received chat; no fixture
answer is read as evidence.  `tag <uuid> list` and `data get` answers do not reach the
client chat in this gate, so the set semantics are observed through a selector.
"""
from __future__ import annotations

from typing import Any

from .probe_steps import ProbeSteps

COW = "minecraft:cow"
TAG = "dupx"
PRESENT = "DUP_PRESENT"
GONE = "DUP_GONE"


def _command(steps: ProbeSteps, text: str) -> None:
    steps.call("minecraft_send_chat", {"message": text, "command": True})


def _chat(steps: ProbeSteps, label: str, *needles: str,
          timeout_seconds: float = 30) -> dict[str, Any]:
    def hit(state: dict[str, Any]) -> bool:
        return any(all(needle in line for needle in needles) for line in state["recent_chat"])

    return steps.wait(hit, label, timeout_seconds=timeout_seconds)


def _at(steps: ProbeSteps, x: float, z: float) -> dict[str, Any] | None:
    """The fixture cow at its summon column; the world's own cows move and drift."""
    rows = steps.call("minecraft_list_entities", {"radius": 24, "limit": 128})["entities"]
    return next((row for row in rows
                 if row.get("entity_type") == COW
                 and abs(row["x"] - x) < 0.6 and abs(row["z"] - z) < 0.6), None)


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = (origin[0] + 1.5, origin[1], origin[2] + 2.5)
    # Success feedback is a gamerule; refusals are errors and always arrive.
    steps.fixture("gamerule send_command_feedback true")
    steps.fixture(f"summon {COW} {x} {y} {z} {{Tags:['{TAG}','{TAG}'],NoAI:1b}}")
    steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": COW, "radius": 12, "timeout_seconds": steps.remaining()})
    steps.check("duplicate_tag_cow_exists", _at(steps, x, z) is not None)

    # 1. The duplicated element list still selects: the tag is present.
    _command(steps, f"execute if entity @e[type={COW},tag={TAG}] run say {PRESENT}")
    _chat(steps, "duplicate_tag_matches_before_removal", PRESENT)

    # 2. One removal, which on a set leaves no tag behind.
    _command(steps, f"tag @e[type={COW},tag={TAG},limit=1] remove {TAG}")
    _chat(steps, "one_tag_removal_reported", "Updated tags on")

    # 3. The selector no longer matches: only this branch can print the marker, so a
    #    tree that kept the second entry never reaches the assertion.
    _command(steps, f"execute unless entity @e[type={COW},tag={TAG}] run say {GONE}")
    _chat(steps, "one_removal_clears_the_duplicate_tag", GONE)
    steps.check("one_removal_leaves_no_duplicate_tag", True)

    steps.fixture(f"kill @e[type={COW},tag={TAG}]")
    steps.report["limits"] = [
        "One cow, one duplicated element and one removal; the effect of the remaining entry is "
        "observed through a selector because `tag <uuid> list` and `data get` answers do not reach "
        "the client chat. No /reload, WAL or export path here; those rest on the cargo refs.",
    ]
