"""Native player-visible scoreboard and execute-store regression probe."""
from __future__ import annotations

import uuid

from .native_p1 import _capture, _inventory
from .native_v2 import _echo
from .probe_steps import ProbeSteps, item_count


def _require_score(steps: ProbeSteps, holder: str, objective: str, expected: int, label: str) -> None:
    marker = f"scoreproof{uuid.uuid4().hex}"
    steps.fixture(f"execute if score {holder} {objective} matches {expected} run say {marker}")
    state = _echo(steps, label + "_ordered")
    steps.check(label, any(marker in line for line in state["recent_chat"]))


def run(steps: ProbeSteps, _origin: tuple[int, int, int]) -> None:
    if not getattr(steps.client, "p4_window", None):
        raise RuntimeError("command promotion requires the harness-owned windowed Xvfb client")
    steps.check("stone_initially_absent", item_count(steps.observe(), "minecraft:stone") == 0)
    before = steps.observe()["player"]["health"]
    steps.check("damage_fixture_healthy", before == 20)
    steps.fixture("gamerule natural_health_regeneration false")
    steps.fixture("gamemode survival @s")
    _echo(steps, "damage_survival_ordered")
    steps.fixture("damage @s 3")
    steps.wait(lambda state: state["player"]["health"] == before - 3,
               "damage_command_received_health", timeout_seconds=20)
    for objective, criterion in (("deaths", "deathCount"), ("players", "playerKillCount"),
                                 ("kills", "totalKillCount")):
        steps.fixture(f"scoreboard objectives add {objective} {criterion}")
        steps.fixture(f"scoreboard players set Alice {objective} 2")
        steps.fixture(f"scoreboard players add Alice {objective} 3")
        steps.fixture(f"scoreboard players remove Alice {objective} 1")
        _require_score(steps, "Alice", objective, 4, objective + "_writable")
    steps.fixture("scoreboard objectives add scratch dummy")
    steps.fixture("scoreboard players set Bob scratch 9")
    steps.fixture("scoreboard players operation Bob scratch >< Alice deaths")
    _require_score(steps, "Bob", "scratch", 4, "swap_received_counter")
    _require_score(steps, "Alice", "deaths", 9, "counter_received_swap")
    steps.fixture("scoreboard objectives add oracle dummy")
    steps.fixture("execute store result score count oracle store success score success oracle run give @s minecraft:stone 7")
    _require_score(steps, "count", "oracle", 1, "give_result_stored")
    _require_score(steps, "success", "oracle", 1, "give_success_stored")
    steps.check("give_once_seven_stone", item_count(steps.observe(), "minecraft:stone") == 7)
    steps.fixture("execute store result score count oracle store success score success oracle run clear @s minecraft:stone 3")
    _require_score(steps, "count", "oracle", 3, "clear_result_stored")
    _require_score(steps, "success", "oracle", 1, "clear_success_stored")
    steps.check("clear_once_four_stone", item_count(steps.observe(), "minecraft:stone") == 4)
    steps.fixture("execute store result score count oracle store success score success oracle run clear @s minecraft:diamond")
    _require_score(steps, "count", "oracle", 0, "failed_clear_result_stored")
    _require_score(steps, "success", "oracle", 0, "failed_clear_success_stored")
    _capture(steps, "command-score-results")
    _protection(steps)


def _protection(steps: ProbeSteps) -> None:
    for item, label in (("minecraft:totem_of_undying", "totem"),
                        ("minecraft:stick[death_protection={death_effects:["
                         "{type:'minecraft:clear_all_effects'},"
                         "{type:'minecraft:apply_effects',effects:[{id:'minecraft:speed',duration:2000}]}]}]",
                         "custom")):
        item_id = "minecraft:totem_of_undying" if label == "totem" else "minecraft:stick"
        steps.fixture("gamemode creative @s")
        steps.fixture("effect clear @s")
        steps.fixture("effect give @s minecraft:instant_health 1 5 true")
        steps.fixture("clear @s")
        steps.fixture(f"give @s {item}")
        _inventory(steps, item_id, 1)
        steps.wait(lambda state: state["player"]["health"] >= 18, label + "_healthy")
        steps.call("minecraft_set_hotbar_slot", {"slot": 0})
        steps.call("minecraft_wait_ticks", {"ticks": 25})
        steps.fixture("gamemode survival @s")
        steps.fixture("damage @s 100 minecraft:generic")
        saved = steps.wait(lambda state: state["player"]["health"] > 0
                           and not state["screen"]["open"]
                           and item_count(state, item_id) == 0,
                           label + "_prevents_death", timeout_seconds=20)
        _inventory(steps, item_id, 0)
        steps.report[label + "_protection_health"] = saved["player"]["health"]
        if label == "custom":
            steps.fixture("data get entity @s active_effects")
            state = steps.wait(lambda s: any("has the following entity data:" in line
                                             and "minecraft:speed" in line
                                             for line in s["recent_chat"]),
                               "custom_effect_received")
            effects = next(line for line in state["recent_chat"]
                           if "has the following entity data:" in line and "minecraft:speed" in line)
            steps.check("custom_has_no_totem_effects", all(name not in effects for name in (
                "minecraft:regeneration", "minecraft:absorption", "minecraft:fire_resistance")))
