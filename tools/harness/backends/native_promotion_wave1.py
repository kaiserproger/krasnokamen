"""Wave-1 native promotion probes: summon age, data query, death messages,
score food criterion, spawner eggs, golem behavior and mob lanes."""
from __future__ import annotations

import re
import uuid

from .native_p1 import _aim, _capture, _interact, _rows, _use
from .native_p4 import _entity_at
from .native_v2 import _block, _echo, _give, _point, _set
from .probe_steps import ProbeSteps


def _identity(row: dict) -> dict:
    return {key: row[key] for key in ("entity_id", "entity_uuid", "entity_type")}


def _same_row(steps: ProbeSteps, identity: dict) -> dict | None:
    return next((row for row in _rows(steps, 48)
                 if all(row.get(key) == value for key, value in identity.items())), None)


def _chat_has(steps: ProbeSteps, label: str, *needles: str, timeout_seconds: float = 20) -> dict:
    def hit(state: dict) -> bool:
        return any(all(needle in line for needle in needles) for line in state["recent_chat"])
    return steps.wait(hit, label, timeout_seconds=timeout_seconds)


def _metadata_bool(row: dict, index: int) -> bool | None:
    value = row.get("received_metadata", {}).get(str(index))
    if not isinstance(value, dict) or value.get("kind") != "bool":
        return None
    return value.get("value")


def _summon_age(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule spawn_mobs false")
    _echo(steps, "summon_age_fixtures_ordered")
    age_rows: dict[str, dict] = {}
    for i, kind in enumerate(("minecraft:squid", "minecraft:glow_squid",
                              "minecraft:skeleton_horse", "minecraft:zombie_horse")):
        px, pz = x + .5 + i, z + 2.5
        steps.fixture(f"summon {kind} {px} {y} {pz} {{Age:-24000,NoAI:1b}}")
        age_rows[kind] = _entity_at(steps, kind, px, pz)
    baby_rows: dict[str, dict] = {}
    for i, kind in enumerate(("minecraft:zombie", "minecraft:piglin", "minecraft:zoglin")):
        px, pz = x + .5 + i, z + 5.5
        steps.fixture(f"summon {kind} {px} {y} {pz} {{IsBaby:1b,NoAI:1b}}")
        baby_rows[kind] = _entity_at(steps, kind, px, pz)
    _capture(steps, "summon-age-babies")
    for kind, row in age_rows.items():
        steps.fixture(f"data get entity {row['entity_uuid']} Age")
        steps.wait(lambda s: any((m := re.search(r"entity data: (-?\d+)", line)) and int(m.group(1)) < 0
                                 for line in s["recent_chat"]),
                   f"{kind}_age_is_negative", timeout_seconds=20)
    for kind in ("minecraft:skeleton_horse", "minecraft:zombie_horse"):
        steps.check(f"{kind}_age_syncs_baby_flag", age_rows[kind].get("received_baby") is True)
    for kind, row in baby_rows.items():
        steps.check(f"{kind}_received_baby", row.get("received_baby") is True)
    steps.report["summon_age"] = {kind: {"uuid": row["entity_uuid"],
                                          "received_baby": row.get("received_baby")}
                                   for kind, row in {**age_rows, **baby_rows}.items()}


def _data_query(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    _give(steps, "minecraft:stone")
    steps.fixture("data get entity @s Health")
    _chat_has(steps, "data_get_entity_health_20", "entity data", "20.0f")
    steps.fixture("data get entity @s Inventory")
    _chat_has(steps, "data_get_entity_inventory_lists_stone", "minecraft:stone")
    chest = _point(origin, dx=2, dz=2)
    _set(steps, chest, "minecraft:chest")
    steps.fixture(f"data get block {chest['x']} {chest['y']} {chest['z']} id")
    _chat_has(steps, "data_get_block_chest", "minecraft:chest")
    steps.fixture("data get entity @s NoSuchPath")
    _chat_has(steps, "data_get_absent_path_is_an_error_not_empty", "Nothing found at path", timeout_seconds=20)
    steps.report["limits"] = ["Absent-path query must surface as a chat error; it is never empty custody."]


def _death_messages(steps: ProbeSteps, _origin: tuple[int, int, int]) -> None:
    steps.fixture("gamerule show_death_messages false")
    # No team packets here: the pinned client (f4bd26d4) parses set_player_team
    # with the pre-26.3 field order and disconnects ("4 trailing packet bytes"),
    # so TEAM-DEATH-VISIBILITY is tracked as blocked on a client fix. The death
    # message text rides player_combat_kill, parsed by the pinned client at the
    # pre-26.3 id, so KILL-TYPE/DEATH-COMPONENT content is not client-observable;
    # this scenario proves the victim notification (death screen) instead.
    _echo(steps, "death_setup_ordered")
    steps.fixture("kill @s")
    state = steps.wait(lambda s: s["screen"]["open"] and "Death" in str(s["screen"].get("class")),
                       "victim_gets_death_screen_despite_show_death_messages_false", timeout_seconds=30)
    steps.report["death_screen_class"] = state["screen"].get("class")
    steps.report["death_chat"] = [line for line in state["recent_chat"] if "kill" in line.lower()]


def _score_food(steps: ProbeSteps, _origin: tuple[int, int, int]) -> None:
    steps.fixture("scoreboard objectives add hunger food")
    _chat_has(steps, "food_objective_created", "Created objective", "hunger")
    marker = f"foodfull{uuid.uuid4().hex}"
    steps.fixture(f"execute if score @s hunger matches 20 run say {marker}")
    state = _echo(steps, "food_initial_ordered")
    steps.check("food_score_tracks_20", any(marker in line for line in state["recent_chat"]))
    steps.fixture("effect give @s minecraft:hunger 255 60 false")

    def score_below_20(state: dict) -> bool:
        return any((m := re.search(r"has (\d+)", line)) and int(m.group(1)) < 20
                   for line in state["recent_chat"])
    for _ in range(30):
        steps.fixture("scoreboard players get @s hunger")
        try:
            steps.wait(score_below_20, "food_score_get_poll", timeout_seconds=5)
            break
        except (RuntimeError, TimeoutError):
            continue
    else:
        raise RuntimeError("food_score_auto_updated_below_20")
    steps.report["limits"] = ["Auto-update proven by the food criterion changing without an explicit set."]


def _spawner_egg(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("time set day")
    steps.fixture("difficulty peaceful")
    steps.fixture("gamemode survival @s")
    spawner = _point(origin, dx=0, dy=0, dz=2)
    # Place the spawner with the client, not an operator setblock: the block log
    # resyncs client uses on operator-set blocks, which would drop the egg use.
    _give(steps, "minecraft:spawner")
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    floor = _point(origin, dx=0, dy=-1, dz=2)
    steps.call("minecraft_look_at_block", {**floor, "face": "up"})
    _use(steps, floor, "up")
    _block(steps, spawner, "minecraft:spawner")
    _echo(steps, "spawner_placed_ordered")
    before = {row["entity_uuid"] for row in _rows(steps, 8) if row["entity_type"] == "minecraft:cow"}
    _give(steps, "minecraft:cow_spawn_egg")
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.call("minecraft_look_at_block", {**spawner, "face": "north"})
    _use(steps, spawner, "north")
    steps.fixture(f"data get block {spawner['x']} {spawner['y']} {spawner['z']} SpawnData.entity.id")
    _chat_has(steps, "spawner_configured_with_cow", "minecraft:cow")
    near_now = [row for row in _rows(steps, 8)
                if row["entity_type"] == "minecraft:cow" and row["entity_uuid"] not in before
                and abs(row["x"] - spawner["x"]) < 5 and abs(row["z"] - spawner["z"]) < 5]
    steps.check("egg_configured_spawner_without_adjacent_spawn", not near_now)

    def spawned(_state: dict) -> bool:
        return any(row["entity_type"] == "minecraft:cow" and row["entity_uuid"] not in before
                   and abs(row["x"] - spawner["x"]) < 7 and abs(row["z"] - spawner["z"]) < 7
                   for row in _rows(steps, 8))
    steps.wait(spawned, "spawner_spawned_configured_cow_on_peaceful", timeout_seconds=150)
    _capture(steps, "spawner-cow-spawned")
    steps.report["limits"] = ["Passive cow spawner admitted and spawned on peaceful; single spawner, one cow."]


def _golem(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule spawn_mobs false")
    steps.fixture("gamemode survival @s")
    # GOLEM-AABB: villager at origin, live golem 15 blocks away on each axis
    # (spherical distance ~21.2 > 16, AABB within 16) must still be detected.
    steps.fixture(f"summon minecraft:villager {x + .5} {y} {z + .5}")
    villager = _entity_at(steps, "minecraft:villager", x + .5, z + .5)
    steps.fixture(f"summon minecraft:iron_golem {x + 15.5} {y} {z + 15.5} {{NoAI:1b}}")
    far = _echo(steps, "far_golem_summon_ordered")
    steps.check("far_golem_summoned", any("Summoned new Iron Golem" in line for line in far["recent_chat"]))
    steps.call("minecraft_wait_ticks", {"ticks": 100})
    steps.call("minecraft_wait_ticks", {"ticks": 100})

    def detected(state: dict) -> bool:
        return any("entity data: 1b" in line for line in state["recent_chat"])
    for _ in range(10):
        steps.fixture(f'data get entity {villager["entity_uuid"]} '
                      f'Brain.memories."minecraft:golem_detected_recently".value')
        try:
            steps.wait(detected, "villager_detects_diagonal_aabb_golem", timeout_seconds=5)
            break
        except (RuntimeError, TimeoutError):
            continue
    else:
        raise RuntimeError("villager_detects_diagonal_aabb_golem")
    # GOLEM-REPAIR: hurt a near golem to 40, then heal exactly +25 with an ingot.
    steps.fixture(f"summon minecraft:iron_golem {x + .5} {y} {z + 2.5} {{NoAI:1b}}")
    golem = _entity_at(steps, "minecraft:iron_golem", x + .5, z + 2.5)
    gid = _identity(golem)
    steps.fixture(f"damage {golem['entity_uuid']} 60 minecraft:generic")
    steps.wait(lambda _s: (row := _same_row(steps, gid)) is not None and row["health"] == 40,
               "golem_at_40_health", timeout_seconds=20)
    _give(steps, "minecraft:iron_ingot")
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.call("minecraft_approach_entity", {"entity_id": gid["entity_id"], "timeout_seconds": steps.remaining()})
    _aim(steps, _same_row(steps, gid) or golem)
    _capture(steps, "golem-before-repair")
    _interact(steps, gid)
    steps.wait(lambda _s: (row := _same_row(steps, gid)) is not None and row["health"] == 65,
               "golem_repaired_exactly_25_to_65", timeout_seconds=20)
    _capture(steps, "golem-after-repair")
    # GOLEM-DIFFICULTY: a natural (AI) golem's melee on the player scales up
    # strictly from easy through normal to hard.
    steps.fixture(f"summon minecraft:iron_golem {x + 2.5} {y} {z + 2.5}")
    fighter = _entity_at(steps, "minecraft:iron_golem", x + 2.5, z + 2.5)
    fid = _identity(fighter)
    drops: dict[str, float] = {}
    for diff in ("easy", "normal", "hard"):
        steps.fixture(f"difficulty {diff}")
        steps.fixture("effect give @s minecraft:instant_health 1 5 true")
        steps.wait(lambda s: s["player"]["health"] == 20, f"{diff}_player_healthy", timeout_seconds=20)
        steps.call("minecraft_approach_entity", {"entity_id": fid["entity_id"], "timeout_seconds": steps.remaining()})
        _aim(steps, _same_row(steps, fid) or fighter)
        steps.call("minecraft_attack_entity_once", {"entity_id": fid["entity_id"], "entity_uuid": fid["entity_uuid"], "entity_type": fid["entity_type"]})
        hit = steps.wait(lambda s: s["player"]["health"] < 20, f"{diff}_golem_melee_landed",
                         timeout_seconds=40)
        drops[diff] = 20 - hit["player"]["health"]
    steps.check("golem_melee_scales_easy_lt_normal_lt_hard",
                0 < drops["easy"] < drops["normal"] < drops["hard"])
    steps.report["golem_melee_damage_by_difficulty"] = drops


def _axolotl(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule spawn_mobs false")
    steps.fixture(f"fill {x} {y} {z + 2} {x + 2} {y} {z + 4} minecraft:water")
    steps.fixture(f"summon minecraft:axolotl {x + 1.5} {y + 1} {z + 3.5}")
    ax = _entity_at(steps, "minecraft:axolotl", x + 1.5, z + 3.5)
    aid = _identity(ax)
    steps.fixture(f"damage {ax['entity_uuid']} 15 minecraft:generic")
    steps.wait(lambda _s: (row := _same_row(steps, aid)) is not None and row["health"] == 5,
               "axolotl_at_5_health", timeout_seconds=20)

    def playing_dead(_state: dict) -> bool:
        row = _same_row(steps, aid)
        return row is not None and _metadata_bool(row, 19) is True
    steps.call("minecraft_approach_entity", {"entity_id": aid["entity_id"], "timeout_seconds": steps.remaining()})
    for attempt in range(25):
        if playing_dead(steps.observe()):
            break
        _aim(steps, _same_row(steps, aid) or ax)
        steps.call("minecraft_attack_entity_once", {"entity_id": aid["entity_id"], "entity_uuid": aid["entity_uuid"], "entity_type": aid["entity_type"]})
        steps.call("minecraft_wait_ticks", {"ticks": 25})
    steps.wait(playing_dead, "axolotl_play_dead_metadata_19", timeout_seconds=30)
    _capture(steps, "axolotl-playing-dead")
    steps.fixture(f"data get entity {aid['entity_uuid']} active_effects")
    _chat_has(steps, "axolotl_regeneration_from_play_dead", "minecraft:regeneration", timeout_seconds=30)


def _dolphin(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule spawn_mobs false")
    steps.fixture(f"fill {x} {y} {z + 2} {x + 4} {y + 1} {z + 6} minecraft:water")
    chest = _point(origin, dx=12, dz=4)
    _set(steps, chest, "minecraft:chest")
    steps.fixture(f'data merge block {chest["x"]} {chest["y"]} {chest["z"]} '
                  f'{{Items:[{{Slot:0b,id:"minecraft:gold_ingot",count:1b}}]}}')
    steps.fixture(f"summon minecraft:dolphin {x + 2.5} {y + 1} {z + 4.5}")
    dol = _entity_at(steps, "minecraft:dolphin", x + 2.5, z + 4.5)
    did = _identity(dol)

    def dist(row: dict) -> float:
        return ((row["x"] - chest["x"]) ** 2 + (row["z"] - chest["z"]) ** 2) ** .5
    before = dist(_same_row(steps, did) or dol)
    steps.fixture(f"data merge entity {did['entity_uuid']} {{GotFish:1b}}")

    def approached(_state: dict) -> bool:
        row = _same_row(steps, did)
        return row is not None and dist(row) < max(6.0, before - 6.0)
    steps.wait(approached, "fed_dolphin_swims_toward_treasure_chest", timeout_seconds=90)
    after = dist(_same_row(steps, did) or dol)
    _capture(steps, "dolphin-leading-to-chest")
    steps.report["dolphin_distance_to_chest"] = {"before": before, "after": after}
    steps.check("dolphin_moved_closer_to_chest", after < before - 4)


def _goat_ram(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule spawn_mobs false")
    steps.fixture("time set day")
    steps.fixture(f"summon minecraft:goat {x + .5} {y} {z + 2.5}")
    steps.fixture(f"summon minecraft:armor_stand {x + 4.5} {y} {z + 2.5} {{NoAI:1b}}")
    stand = _entity_at(steps, "minecraft:armor_stand", x + 4.5, z + 2.5)
    sid = _identity(stand)
    max_health = stand.get("health")
    _capture(steps, "goat-before-ram")

    def rammed(_state: dict) -> bool:
        row = _same_row(steps, sid)
        return row is None or row["health"] < max_health
    steps.wait(rammed, "goat_rammed_armor_stand", timeout_seconds=420)
    _capture(steps, "goat-after-ram")
    steps.check("goat_still_present", _same_row(steps, _identity(
        next(r for r in _rows(steps, 16) if r["entity_type"] == "minecraft:goat"))) is not None)
    steps.report["armor_stand_health"] = {"max": max_health,
                                           "after": (r := _same_row(steps, sid)) and r["health"]}


def _golem_gossip(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule spawn_mobs false")
    steps.fixture("time set day")
    steps.fixture("gamerule advance_time false")
    _set(steps, _point(origin, dx=0, dy=0, dz=4), "minecraft:bell")
    villagers = []
    for i in range(5):
        px, pz = x + .5 + (i % 2) * 2, z + .5 + (i // 2) * 2
        steps.fixture(f"summon minecraft:villager {px} {y} {pz}")
        villagers.append(_entity_at(steps, "minecraft:villager", px, pz))
    _echo(steps, "gossip_villagers_ordered")
    steps.fixture("time query gametime")
    state = _chat_has(steps, "gametime_read", "time is")
    tick = int(re.search(r"time is (\d+)", next(l for l in state["recent_chat"] if "time is" in l)).group(1))
    for row in villagers:
        uuid_ = row["entity_uuid"]
        steps.fixture(f'data merge entity {uuid_} {{Brain:{{memories:{{"minecraft:last_slept":'
                      f'{{value:{tick - 100}L}}}}}},"solaris:VillagerRuntime":'
                      f'{{last_gossip:{tick - 1}L,activity:"meet"}}}}')

    def summoned(_state: dict) -> bool:
        return any(row["entity_type"] == "minecraft:iron_golem" for row in _rows(steps, 32))
    steps.wait(summoned, "five_sleeping_villagers_summoned_golem", timeout_seconds=360)
    _capture(steps, "gossip-golem-summoned")


KINDS = {
    "summon-age": _summon_age,
    "data-query": _data_query,
    "death-messages": _death_messages,
    "score-food": _score_food,
    "spawner-egg": _spawner_egg,
    "golem": _golem,
    "axolotl-play-dead": _axolotl,
    "dolphin-treasure": _dolphin,
    "goat-ram": _goat_ram,
    "golem-gossip": _golem_gossip,
}


def run(steps: ProbeSteps, origin: tuple[int, int, int], kind: str) -> None:
    if kind not in KINDS:
        raise ValueError(f"unknown wave-1 promotion probe: {kind}")
    if not getattr(steps.client, "p4_window", None):
        raise RuntimeError("wave-1 promotion probes require the harness-owned windowed Xvfb client")
    KINDS[kind](steps, origin)
