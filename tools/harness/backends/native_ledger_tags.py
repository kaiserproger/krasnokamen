"""Ledger tag refusals: the player's own chat answers for invalid `Tags:` NBT.

The row `TAGS-REFUSAL-MESSAGES` says invalid tag element types and an oversized
tag list must be refused with a typed message before any entity exists, instead
of a count limit or an internal storage wire-length error.  Every step here is
the player's own command and the player's own received chat line; no fixture
answer is read as evidence.
"""
from __future__ import annotations

from typing import Any

from .probe_steps import ProbeSteps

COW = "minecraft:cow"


def _chat(steps: ProbeSteps, label: str, *needles: str,
          timeout_seconds: float = 30) -> dict[str, Any]:
    def hit(state: dict[str, Any]) -> bool:
        return any(all(needle in line for needle in needles) for line in state["recent_chat"])

    return steps.wait(hit, label, timeout_seconds=timeout_seconds)


def _command(steps: ProbeSteps, text: str) -> None:
    steps.call("minecraft_send_chat", {"message": text, "command": True})


def _cows(steps: ProbeSteps, origin: tuple[int, int, int]) -> int:
    rows = steps.call("minecraft_list_entities", {"radius": 24, "limit": 128})["entities"]
    return sum(1 for row in rows if row.get("entity_type") == COW)


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    at = f"{x} {y + 1} {z + 2}"
    before = _cows(steps, origin)
    steps.report["cows_before"] = before

    # 1. A non-string list element names the element type, not a count limit.
    _command(steps, f"summon {COW} {at} {{Tags:[1b]}}")
    _chat(steps, "invalid_tag_element_type_refused_with_a_typed_message",
          "Invalid Tags:", "every list element must be a string")
    steps.check("invalid_tag_element_type_creates_no_entity", _cows(steps, origin) == before)

    # 2. A mixed list is the same typed refusal at the offending element.
    _command(steps, f"summon {COW} {at} {{Tags:['ok',2]}}")
    _chat(steps, "mixed_tag_list_refused_with_the_element_type", "every list element must be a string")
    steps.check("mixed_tag_list_creates_no_entity", _cows(steps, origin) == before)

    # 3. The same parser boundary on a non-mob spawn path.
    _command(steps, f"summon minecraft:experience_orb {at} {{Value:1,Tags:[1b]}}")
    _chat(steps, "non_mob_spawn_refuses_the_same_tag_element",
          "every list element must be a string")
    steps.check("refused_orbs_are_absent",
                not any(row.get("entity_type") == "minecraft:experience_orb"
                        for row in steps.call("minecraft_list_entities",
                                              {"radius": 24, "limit": 128})["entities"]))

    # 5. A well-formed tag list is accepted and the client reads the tags back.
    _command(steps, f"summon {COW} {at} {{Tags:['ok','second'],NoAI:1b}}")
    steps.call("minecraft_wait_ticks", {"ticks": 20})
    steps.check("valid_tagged_cow_accepted", _cows(steps, origin) == before + 1)
    rows = steps.call("minecraft_list_entities", {"radius": 24, "limit": 128})["entities"]
    nearest = min((row for row in rows if row.get("entity_type") == COW),
                  key=lambda row: row["distance"])
    _command(steps, f"tag {nearest['entity_uuid']} list")
    _chat(steps, "valid_tags_read_back_from_the_client", "tag(s):", "ok", "second")
    steps.report["cows_after"] = _cows(steps, origin)
    steps.check("exactly_one_valid_summon", steps.report["cows_after"] == before + 1)
    steps.fixture("kill @e[type=minecraft:cow]")
    steps.report["limits"] = [
        "The client chat tool caps a message at 256 characters, so the 1025-element list and the "
        "70000-byte string cannot be typed from a client; those two classes rest on the Cargo ref.",
    ]
