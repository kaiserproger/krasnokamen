"""Wave-1 native promotion probes: summon age, data query, death messages,
score food criterion, spawner eggs, golem behavior and mob lanes."""
from __future__ import annotations

import re
import uuid

from .native_p1 import _aim, _capture, _interact, _rows, _use
from .native_p4 import _entity_at
from .native_v2 import _block, _echo, _give, _point, _set
from .probe_steps import ProbeSteps, item_count


def _revive_if_dead(steps: ProbeSteps, label: str) -> None:
    """Respawn a player a mob killed: instant health cannot heal the dead, so a later wait for
    full health on the death screen would never resolve."""
    if steps.observe()["player"]["health"] > 0:
        return
    steps.call("minecraft_respawn", {"timeout_seconds": steps.remaining()})
    steps.wait(lambda s: (s["player"]["health"] or 0) > 0 and not s["screen"]["open"], label,
               wait_for_play=True, timeout_seconds=30)


def _strike(steps: ProbeSteps, ident: dict, row: dict) -> None:
    """Approach, aim and attack once, retrying when a moving mob slipped out of reach or view."""
    for _ in range(30):
        if steps.observe()["player"]["health"] <= 0:
            raise RuntimeError("player_died_before_attack")
        steps.call("minecraft_approach_entity", {"entity_id": ident["entity_id"], "timeout_seconds": steps.remaining()})
        _aim(steps, _same_row(steps, ident) or row)
        try:
            steps.call("minecraft_attack_entity_once", {"entity_id": ident["entity_id"], "entity_uuid": ident["entity_uuid"], "entity_type": ident["entity_type"]})
            return
        except RuntimeError as error:
            if not any(reason in str(error) for reason in (
                    "out of interaction reach", "entity wait timed out")):
                raise
    raise RuntimeError(f"attack_in_reach_{ident['entity_type']}")


def _wanderer_at(steps: ProbeSteps, kind: str, x: float, z: float,
                 exclude: tuple[str, ...] = ()) -> dict:
    """Find a just-summoned AI mob that may already have walked off its summon point.

    `_entity_at` demands the row within 0.6 blocks of the summon point and races a
    wandering villager, axolotl or dolphin; here the closest not-yet-claimed row of
    the kind within 8 blocks of it is the new mob (spawn_mobs is off in these gates).
    """
    found: dict = {}

    def received(_: dict) -> bool:
        rows = [row for row in _rows(steps, 16)
                if row["entity_type"] == kind and row["entity_uuid"] not in exclude
                and abs(row["x"] - x) < 8 and abs(row["z"] - z) < 8]
        if not rows:
            return False
        found.update(min(rows, key=lambda row: (row["x"] - x) ** 2 + (row["z"] - z) ** 2))
        return True
    steps.wait(received, f"received_{kind}_{x}_{z}")
    return found


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
    # Spawner spawns obey the creature placement rule (grass floor, as in vanilla): give the
    # whole SpawnRange (4) around the spawner a grass floor instead of the obsidian pad.
    steps.fixture(f"fill {x - 4} {y - 1} {z - 2} {x + 4} {y - 1} {z + 6} minecraft:grass_block")
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
    # GOLEM-AABB: a penned villager (2x2 cell, walls 3 high, so it cannot wander toward the
    # golem and change the geometry) with a live golem 14 blocks away on each axis
    # (spherical distance ~19.8 > 16, AABB within 16) must still detect it.
    steps.fixture(f"fill {x - 1} {y} {z - 5} {x + 2} {y + 2} {z - 5} minecraft:obsidian")
    steps.fixture(f"fill {x - 1} {y} {z - 2} {x + 2} {y + 2} {z - 2} minecraft:obsidian")
    steps.fixture(f"fill {x - 1} {y} {z - 4} {x - 1} {y + 2} {z - 3} minecraft:obsidian")
    steps.fixture(f"fill {x + 2} {y} {z - 4} {x + 2} {y + 2} {z - 3} minecraft:obsidian")
    steps.fixture(f"summon minecraft:villager {x + 1.0} {y} {z - 3.0}")
    villager = _wanderer_at(steps, "minecraft:villager", x + 1.0, z - 3.0)
    vid = _identity(villager)
    steps.fixture(f"summon minecraft:iron_golem {x + 15.0} {y} {z + 11.0} {{NoAI:1b}}")
    far = _echo(steps, "far_golem_summon_ordered")
    steps.check("far_golem_summoned", any("Summoned new Iron Golem" in line for line in far["recent_chat"]))
    far_golem: dict = {}

    def far_received(_state: dict) -> bool:
        # 20 blocks from the player: beyond _entity_at's 16-block search radius.
        row = next((r for r in _rows(steps, 32) if r["entity_type"] == "minecraft:iron_golem"
                    and abs(r["x"] - (x + 15.0)) < .6 and abs(r["z"] - (z + 11.0)) < .6), None)
        if row is not None:
            far_golem.update(row)
        return row is not None
    steps.wait(far_received, "received_far_iron_golem")
    steps.call("minecraft_wait_ticks", {"ticks": 100})
    steps.call("minecraft_wait_ticks", {"ticks": 100})

    def detected(state: dict) -> bool:
        return any("entity data: 1b" in line for line in state["recent_chat"])

    def axes_and_sphere() -> tuple[float, float]:
        row = _same_row(steps, vid) or villager
        deltas = (abs(row["x"] - far_golem["x"]), abs(row["z"] - far_golem["z"]))  # both stand on the pad
        return max(deltas), sum(d * d for d in deltas) ** .5
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
    axis, sphere = axes_and_sphere()
    steps.report["golem_detection_geometry"] = {"max_axis": axis, "sphere": sphere}
    # A wandering villager that walked inside spherical range would make a spherical
    # (buggy) detector pass too: refuse that run as evidence.
    steps.check("detection_geometry_is_axis_only", axis <= 16.0 < sphere)
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
    fighter = _wanderer_at(steps, "minecraft:iron_golem", x + 2.5, z + 2.5, exclude=(golem["entity_uuid"],))
    fid = _identity(fighter)
    # Regeneration between the golem hits would blur the per-hit damage readings.
    steps.fixture("gamerule natural_health_regeneration false")
    drops: dict[str, float] = {}
    def measure(diff: str) -> float:
        _revive_if_dead(steps, f"{diff}_player_respawned")
        steps.fixture(f"difficulty {diff}")
        steps.fixture("effect give @s minecraft:instant_health 1 5 true")
        steps.wait(lambda s: s["player"]["health"] == 20, f"{diff}_player_healthy", timeout_seconds=20)
        # The (provoked) golem may land its hit while we approach, and a Hard hit can
        # kill: a hit before our own swing is the measurement, not a failure. A golem
        # that has calmed down is provoked again.
        hit = None
        for _ in range(6):
            if steps.observe()["player"]["health"] >= 20:
                try:
                    _strike(steps, fid, fighter)
                except RuntimeError:
                    if steps.observe()["player"]["health"] >= 20:
                        raise
            try:
                hit = steps.wait(lambda s: s["player"]["health"] < 20, f"{diff}_golem_melee_landed",
                                 timeout_seconds=8)
                break
            except (RuntimeError, TimeoutError):
                continue
        if hit is None:
            raise RuntimeError(f"{diff}_golem_melee_landed")
        return 20 - hit["player"]["health"]
    # A reading that lags behind two golem hits only over-counts, so the smallest of three
    # readings is the per-hit damage. Hard hits can kill the player, so Hard is read once.
    samples = {diff: [measure(diff) for _ in range(3)] for diff in ("easy", "normal")}
    samples["hard"] = [measure("hard")]
    steps.report["golem_melee_damage_samples"] = samples
    drops = {diff: min(values) for diff, values in samples.items()}
    steps.report["golem_melee_damage_by_difficulty"] = drops
    steps.check("golem_melee_scales_easy_lt_normal_lt_hard",
                0 < drops["easy"] < drops["normal"] < drops["hard"])


def _axolotl(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule spawn_mobs false")
    steps.fixture(f"fill {x} {y} {z + 2} {x + 2} {y} {z + 4} minecraft:water")
    steps.fixture(f"summon minecraft:axolotl {x + 1.5} {y + 1} {z + 3.5}")
    ax = _wanderer_at(steps, "minecraft:axolotl", x + 1.5, z + 3.5)
    aid = _identity(ax)
    # Axolotl max health is 14: 8 damage leaves 6 (< 7 = half), where every
    # entity-sourced hit may trigger play-dead with probability 1/3.
    steps.fixture(f"damage {ax['entity_uuid']} 8 minecraft:generic")
    steps.wait(lambda _s: (row := _same_row(steps, aid)) is not None and row["health"] == 6,
               "axolotl_at_6_health", timeout_seconds=20)

    def playing_dead(_state: dict) -> bool:
        row = _same_row(steps, aid)
        return row is not None and _metadata_bool(row, 19) is True
    for attempt in range(25):
        if playing_dead(steps.observe()):
            break
        _strike(steps, aid, ax)
        # Play-dead lasts 200 ticks and a strike with retries can take longer than that:
        # look right after the hit, before any further delay.
        steps.call("minecraft_wait_ticks", {"ticks": 6})
        if playing_dead(steps.observe()):
            break
        steps.call("minecraft_wait_ticks", {"ticks": 19})
        row = _same_row(steps, aid)
        if row is not None and row["health"] <= 2:
            # A lethal fist hit never triggers play-dead: top up to <= 6 and keep trying.
            steps.fixture(f"effect give {aid['entity_uuid']} minecraft:instant_health 1 0 true")
    steps.wait(playing_dead, "axolotl_play_dead_metadata_19", timeout_seconds=30)
    # Play-dead and its Regeneration last 200 ticks: read the effect before the capture.
    steps.fixture(f"data get entity {aid['entity_uuid']} active_effects")
    _chat_has(steps, "axolotl_regeneration_from_play_dead", "minecraft:regeneration", timeout_seconds=30)
    _capture(steps, "axolotl-playing-dead")


def _dolphin(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule spawn_mobs false")
    # Creative feeding does not debit the fish; survival proves the real consume.
    steps.fixture("gamemode survival @s")
    # A 13-long channel: the server steers a fed dolphin 5 blocks per search toward
    # the nearest chest within 48 blocks, and only while the dolphin stays in water.
    steps.fixture(f"fill {x} {y} {z + 2} {x + 12} {y + 1} {z + 6} minecraft:water")
    chest = _point(origin, dx=17, dz=4)
    _set(steps, chest, "minecraft:chest")
    steps.fixture(f"summon minecraft:dolphin {x + 1.5} {y + 1} {z + 2.5}")
    dol = _wanderer_at(steps, "minecraft:dolphin", x + 1.5, z + 2.5)
    did = _identity(dol)
    # Real feeding: a held raw cod sets GotFish (`data merge entity` is unsupported).
    _give(steps, "minecraft:cod")
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    fed_from = dict(_same_row(steps, did) or dol)  # start of the lead, before any feeding retry
    for _ in range(12):
        steps.call("minecraft_approach_entity", {"entity_id": did["entity_id"], "timeout_seconds": steps.remaining()})
        _aim(steps, _same_row(steps, did) or dol)
        try:
            _interact(steps, did)
            steps.wait(lambda s: item_count(s, "minecraft:cod") == 0, "dolphin_ate_the_cod", timeout_seconds=4)
            break
        except (RuntimeError, TimeoutError) as error:
            if "dolphin_ate_the_cod" not in str(error) and "out of interaction reach" not in str(error):
                raise
    else:
        raise RuntimeError("dolphin_ate_the_cod")

    def dist(row: dict) -> float:
        return ((row["x"] - chest["x"]) ** 2 + (row["z"] - chest["z"]) ** 2) ** .5
    before = dist(fed_from)

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
    # A survival player is a valid ram target and stands nearer than the armor stand;
    # creative players are not (goat.rs valid_target), so only the stand can be rammed.
    steps.fixture("gamemode creative @s")
    # The goat rams the nearest valid living target; natural wildlife around the pad
    # would be picked instead of the stand (cows took the ram in the 2026-11-12 run).
    for kind in ("pig", "cow", "sheep", "chicken", "zombie", "skeleton", "spider", "creeper"):
        steps.fixture(f"kill @e[type=minecraft:{kind},distance=..40]")
    steps.fixture(f"summon minecraft:goat {x + .5} {y} {z + 2.5}")
    steps.fixture(f"summon minecraft:armor_stand {x + 4.5} {y} {z + 2.5} {{NoAI:1b}}")
    stand = _entity_at(steps, "minecraft:armor_stand", x + 4.5, z + 2.5)
    sid = _identity(stand)
    max_health = stand.get("health")
    _capture(steps, "goat-before-ram")

    def rammed(_state: dict) -> bool:
        row = _same_row(steps, sid)
        return row is None or row["health"] < max_health
    steps.wait(rammed, "goat_rammed_armor_stand", timeout_seconds=540)
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
        villagers.append(_wanderer_at(steps, "minecraft:villager", px, pz,
                                      exclude=tuple(v["entity_uuid"] for v in villagers)))
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
