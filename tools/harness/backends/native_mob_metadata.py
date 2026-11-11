"""Frog, strider, camel and sniffer entity-data accessors received by one native client."""
from __future__ import annotations

import re

from .native_p1 import _aim, _capture
from .native_v2 import _echo
from .probe_steps import ProbeSteps

# (entity, x offset): spaced for the widest bodies; all four stand 8.5 blocks north of
# the player, with the camera turned left so the group clears the chat overlay.
MOBS = (("frog", -4.0), ("strider", -1.6), ("camel", 0.9), ("sniffer", 3.0))
# A client that cannot decode an accessor reports it; startup progress lines are exempt.
DECODE_FAILURE = re.compile(r"decode|metadata|panick|disconnect|unsupported", re.IGNORECASE)


def _client_log_failures(steps: ProbeSteps) -> list[str]:
    log = (steps.artifact_dir.parent / "native-client.log").read_text(errors="replace")
    return [line for line in log.splitlines()
            if DECODE_FAILURE.search(line) and not line.startswith("session startup")]


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    # The only diggable ground on the obsidian pad, so the sniffer has a real target.
    steps.fixture(f"setblock {x + 3} {y - 1} {z - 9} minecraft:dirt")
    for name, dx in MOBS:
        steps.fixture(f"summon minecraft:{name} {x + 0.5 + dx} {y} {z - 8.5}")
    _echo(steps, "mobs_summoned")
    rows = {name: steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": f"minecraft:{name}", "radius": 12,
        "timeout_seconds": steps.remaining()}) for name, _ in MOBS}
    steps.report["mob_rows"] = {name: row["entity_uuid"] for name, row in rows.items()}
    centre = {"x": x - 3.8, "z": z - 8.5,
              "bounding_box": {"min_y": y, "max_y": y + 1.5}}
    _aim(steps, centre)
    _capture(steps, "four-mobs-in-frame")
    # Frog.DATA_TONGUE_TARGET_ID follows the variant: a tongue at the wrong
    # accessor would be undecodable or overwrite the variant.
    steps.fixture(f"summon minecraft:slime {x + 0.5 - 4.9} {y} {z - 8.5} {{Size:0,NoAI:1b}}")
    slime = steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": "minecraft:slime", "radius": 12, "timeout_seconds": steps.remaining()})
    steps.call("minecraft_wait_for_entity_removed", {
        key: slime[key] for key in ("entity_id", "entity_uuid", "entity_type")}
        | {"timeout_seconds": steps.remaining()})
    steps.check("frog_ate_the_small_slime", True)
    _capture(steps, "frog-after-tongue")
    _echo(steps, "connection_alive_after_all_accessors")
    for name, _ in MOBS:
        steps.check(f"{name}_still_present", any(
            row["entity_uuid"] == rows[name]["entity_uuid"]
            for row in steps.call("minecraft_list_entities", {"radius": 12, "limit": 128})["entities"]))
    failures = _client_log_failures(steps)
    steps.report["client_log_failures"] = failures
    steps.check("client_log_has_no_decode_failure", not failures)
