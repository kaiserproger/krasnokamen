"""Causal native-client snow-golem shearing probe."""
from __future__ import annotations

from .native_p1 import _aim, _capture, _interact, _rows
from .native_p4 import _entity_at
from .native_v2 import _echo, _give
from .probe_steps import ProbeSteps, item_count




def _identity(row: dict) -> dict:
    return {key: row[key] for key in ("entity_id", "entity_uuid", "entity_type")}


def _metadata(row: dict, index: int, kind: str):
    value = row.get("received_metadata", {}).get(str(index))
    if not isinstance(value, dict) or value.get("kind") != kind:
        return None
    return value.get("value")


def _snow_flag(row: dict, present: bool) -> bool:
    value = _metadata(row, 16, "byte")
    return type(value) is int and bool(value & 16) == present


def _snow_initial_pumpkin(row: dict) -> bool:
    # Vanilla 26.3 SnowGolem.defineSynchedData defines this byte as 16.
    # Unchanged defaults need not be sent at spawn; an explicit wrong value
    # must never be replaced by that default.
    return _snow_flag(row, True) or (row.get("entity_type") == "minecraft:snow_golem"
                                    and "16" not in row.get("received_metadata", {}))



def _same_row(steps: ProbeSteps, identity: dict) -> dict | None:
    return next((row for row in _rows(steps, 32)
                 if all(row.get(key) == value for key, value in identity.items())), None)


def _snow(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.check("pumpkin_inventory_initially_empty",
                item_count(steps.observe(), "minecraft:carved_pumpkin") == 0)
    steps.check("no_preexisting_carved_pumpkin_drop", not any(
        row.get("item", {}).get("item_id") == "minecraft:carved_pumpkin"
        for row in _rows(steps, 32)))
    steps.fixture(f"summon minecraft:snow_golem {x + .5} {y} {z + 2.5} {{NoAI:1b,Rotation:[180f,0f]}}")
    _echo(steps, "snow_summon_ordered")
    row = _entity_at(steps, "minecraft:snow_golem", x + .5, z + 2.5)
    identity = _identity(row)
    steps.check("snow_pumpkin_initial_received_or_vanilla_default", _snow_initial_pumpkin(row))
    steps.report["snow_initial_pumpkin_origin"] = (
        "received_byte" if "16" in row.get("received_metadata", {}) else "vanilla_default_16_not_sent_on_spawn")
    _give(steps, "minecraft:shears")
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    _aim(steps, row)
    _capture(steps, "snow-golem-before-shearing")
    _interact(steps, identity)
    after = {}

    def sheared(_state: dict) -> bool:
        current = _same_row(steps, identity)
        if current is None or not _snow_flag(current, False):
            return False
        after.update(current)
        return True

    steps.wait(sheared, "snow_same_uuid_received_pumpkin_removed", timeout_seconds=10)
    steps.report["snow_before"] = row
    steps.report["snow_after"] = after
    _aim(steps, after)
    _capture(steps, "snow-golem-face-after-shearing")
    # Survival reach is farther than item pickup range; approach_entity may
    # correctly return without moving at the two-block interaction position.
    steps.call("minecraft_press_inputs", {"keys": ["forward"], "ticks": 8})
    steps.wait(lambda state: item_count(state, "minecraft:carved_pumpkin") == 1,
               "one_carved_pumpkin_in_player_custody")
    steps.report["visual_review_required"] = [
        "Before: pumpkin covers the snow golem head; after: same golem face is visible without pumpkin."]
    steps.report["limits"] = ["One received carved pumpkin in initially empty player inventory; not a proof of global absence of extra drops or checkpoint persistence."]


def _peaceful(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule spawn_mobs false")
    steps.fixture("time set 18000")
    steps.fixture("gamerule advance_time false")
    steps.fixture("difficulty normal")
    steps.fixture(f"summon minecraft:zombie {x + .5} {y} {z + 2.5} {{NoAI:1b}}")
    steps.fixture(f"summon minecraft:cow {x + 2.5} {y} {z + 2.5} {{NoAI:1b}}")
    _echo(steps, "peaceful_fixtures_ordered")
    zombie = _identity(_entity_at(steps, "minecraft:zombie", x + .5, z + 2.5))
    cow = _identity(_entity_at(steps, "minecraft:cow", x + 2.5, z + 2.5))
    _capture(steps, "peaceful-before")
    steps.fixture("difficulty peaceful")
    _echo(steps, "peaceful_command_ordered")
    steps.wait(lambda _state: _same_row(steps, zombie) is None,
               "same_zombie_removed_after_peaceful", timeout_seconds=20)
    steps.check("same_cow_remains", _same_row(steps, cow) is not None)
    _capture(steps, "peaceful-after")
    steps.report["identities"] = {"zombie": zombie, "cow": cow}


def run(steps: ProbeSteps, origin: tuple[int, int, int], kind: str) -> None:
    if kind not in ("snow-golem-shear", "peaceful-removal"):
        raise ValueError(f"unknown mob promotion probe: {kind}")
    if not getattr(steps.client, "p4_window", None):
        raise RuntimeError("mob promotion probe requires the harness-owned windowed Xvfb client")
    if kind == "snow-golem-shear":
        _snow(steps, origin)
    else:
        _peaceful(steps, origin)
