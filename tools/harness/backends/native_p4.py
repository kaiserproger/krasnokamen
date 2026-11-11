"""P4 received-state probes; captures require independent visual review."""
from __future__ import annotations

import json
from pathlib import Path
import struct
from typing import Any

from .native_p1 import _aim, _capture, _inventory, _rows
from .native_v2 import _block, _click, _echo, _open, _point, _set, _use
from .probe_steps import ProbeSteps, container_slot, item_count

class DamageWireReceipt:
    """Decode received damage packets without changing the forwarded stream."""

    def __init__(self, report_path: Path) -> None:
        packets = json.loads(report_path.read_text())["play"]["clientbound"]
        registry_path = Path(__file__).resolve().parents[3] / "crates/mc-data/data/required_registry_index.json"
        damage_types = json.loads(registry_path.read_text())["damage_type"]
        self.packet_id = packets["minecraft:damage_event"]["protocol_id"]
        self.types = dict(enumerate(damage_types))
        self.events: list[dict[str, Any]] = []

    def server_packet(self, _wire: Any, phase: str, packet_id: int, payload: bytes) -> None:
        if phase != "play" or packet_id != self.packet_id:
            return
        from .light_wire import Reader

        reader = Reader(payload)
        entity, kind, cause, direct = (reader.varint() for _ in range(4))
        has_position = reader.take(1)[0]
        if has_position not in (0, 1):
            raise ValueError("invalid damage source position flag")
        position = list(struct.unpack(">ddd", reader.take(24))) if has_position else None
        if reader.offset != len(payload) or kind not in self.types:
            raise ValueError("invalid or unknown damage event")
        self.events.append({"entity_id": entity, "type": self.types[kind],
                            "cause_entity_id": cause - 1, "direct_entity_id": direct - 1,
                            "source_position": position})

    def snapshot(self, _wire: Any) -> list[dict[str, Any]]:
        return list(self.events)

RESTART_SCENARIOS = frozenset({"native-v2-p4-nameplates", "native-v2-p4-advancements",
                               "native-v2-p4-statistics"})


def _restart(steps: ProbeSteps) -> None:
    identity = steps.observe()["player"]["entity_uuid"]
    steps.fixture("save-all")
    _echo(steps, "save_command_ordered_receipt")
    steps.call("minecraft_disconnect")
    steps.check("restart_client_left_play", steps.call("minecraft_observe")["in_play"] is False)
    address = steps.client.p4_restart_server(steps.remaining())
    steps.report.setdefault("server_restarts", []).append(address)
    steps.call("minecraft_connect", {"server_addr": address})
    play = steps.call("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    steps.check("restart_client_entered_play", play["in_play"] is True)
    state = steps.wait(lambda state: state["player"]["on_ground"], "restart_player_grounded")
    steps.check("restart_same_player_uuid", state["player"]["entity_uuid"] == identity)
    _echo(steps, "restart_ordered_server_receipt")


def _entity_at(steps: ProbeSteps, kind: str, x: float, z: float) -> dict[str, Any]:
    found: dict[str, Any] = {}

    def received(_: dict[str, Any]) -> bool:
        row = next((row for row in _rows(steps, 16) if row["entity_type"] == kind
                    and abs(row["x"] - x) < .6 and abs(row["z"] - z) < .6), None)
        if row is None:
            return False
        found.update(row)
        return True

    steps.wait(received, f"received_{kind}_{x}_{z}")
    return found


def _target_capture(steps: ProbeSteps, row: dict[str, Any], name: str) -> None:
    _aim(steps, row)
    state = steps.wait(lambda state: state.get("target", {}).get("entity_uuid") == row["entity_uuid"],
                       name + "_actual_crosshair_target")
    steps.report.setdefault("targets", {})[name] = state["target"]
    _capture(steps, name)


def _nameplates(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    fixtures = (("armor_stand", x + 2.5, z + 2.5, '{CustomName:"QA Stand",CustomNameVisible:1b}'),
                ("armor_stand", x - 1.5, z + 2.5, '{CustomName:"Hidden Stand"}'),
                ("pig", x + .5, z + 2.5, '{CustomName:"QA Pig",NoAI:1b}'))
    entities = []
    for kind, px, pz, tags in fixtures:
        steps.fixture(f"summon {kind} {px} {y} {pz} {tags}")
        _echo(steps, f"{kind}_{px}_summon_ordered_receipt")
        entities.append(_entity_at(steps, "minecraft:" + kind, px, pz))
    steps.report["fixtures"] = entities
    for row, name in zip(entities, ("p4-stand-visible", "p4-stand-hidden", "p4-pig-targeted")):
        _target_capture(steps, row, name)
    steps.call("minecraft_look", {"yaw_deg": 18, "pitch_deg": 0})
    state = steps.observe()
    steps.check("pig_untargeted_crosshair", state.get("target", {}).get("entity_uuid") != entities[2]["entity_uuid"])
    _capture(steps, "p4-pig-not-targeted")
    _restart(steps)
    restored = _entity_at(steps, "minecraft:armor_stand", entities[0]["x"], entities[0]["z"])
    steps.check("visible_stand_uuid_survives_server_restart", restored["entity_uuid"] == entities[0]["entity_uuid"])
    _target_capture(steps, restored, "p4-stand-visible-after-restart")
    hidden = _entity_at(steps, "minecraft:armor_stand", entities[1]["x"], entities[1]["z"])
    steps.check("hidden_stand_uuid_survives_server_restart", hidden["entity_uuid"] == entities[1]["entity_uuid"])
    _target_capture(steps, hidden, "p4-stand-hidden-after-restart")
    steps.report["visual_review_required"] = ["QA Stand visible before/after restart", "Hidden Stand absent even when targeted", "QA Pig visible only when targeted"]


def _button(steps: ProbeSteps, label: str) -> None:
    screen = steps.observe()["screen"]
    steps.call("minecraft_click_screen_button", {
        "expected_screen_class": screen["class"], "expected_title": screen["title"], "button_label": label,
    })


def _advancement_labels(state: dict[str, Any]) -> dict[str, str]:
    return {node["id"]: node["label"] for node in state.get("ui_nodes", [])
            if node.get("id", "").startswith("advancement/")}


def _scroll(steps: ProbeSteps, count: int) -> None:
    """Real wheel events on this runner's focused, owned Xvfb window."""
    import ctypes as c
    import ctypes.util

    owned = steps.client.p4_window
    x = c.CDLL(ctypes.util.find_library("X11"))
    xt = c.CDLL(ctypes.util.find_library("Xtst"))
    x.XOpenDisplay.restype = c.c_void_p
    x.XOpenDisplay.argtypes = [c.c_char_p]
    x.XWarpPointer.argtypes = [c.c_void_p, c.c_ulong, c.c_ulong, c.c_int, c.c_int,
                              c.c_uint, c.c_uint, c.c_int, c.c_int]
    x.XSync.argtypes = [c.c_void_p, c.c_int]
    x.XCloseDisplay.argtypes = [c.c_void_p]
    xt.XTestFakeButtonEvent.argtypes = [c.c_void_p, c.c_uint, c.c_int, c.c_ulong]
    display = x.XOpenDisplay(owned["display"].encode())
    if not display:
        raise RuntimeError("cannot open this P4 runner's Xvfb display for ordinary scrolling")
    try:
        x.XWarpPointer(display, 0, owned["window_id"], 0, 0, 0, 0, 80, 100)
        for _ in range(abs(count)):
            button = 5 if count > 0 else 4
            if not xt.XTestFakeButtonEvent(display, button, 1, 0) or not xt.XTestFakeButtonEvent(display, button, 0, 0):
                raise RuntimeError("owned Xvfb display refused ordinary wheel input")
        x.XSync(display, 0)
    finally:
        x.XCloseDisplay(display)


def _advancement_screen(steps: ProbeSteps, name: str) -> dict[str, Any]:
    steps.call("minecraft_open_pause_menu")
    _button(steps, "Advancements")
    state = steps.wait(lambda state: state["screen"]["class"].endswith("AdvancementsScreen"), name + "_screen")
    _button(steps, "Minecraft")
    state = steps.observe()
    labels = _advancement_labels(state)
    required = {"advancement/minecraft:story/root", "advancement/minecraft:story/mine_stone",
                "advancement/minecraft:story/upgrade_tools"}
    _capture(steps, name)
    for page in range(24):
        if required <= labels.keys():
            break
        previous = _advancement_labels(state)
        _scroll(steps, 1)
        state = steps.wait(lambda current: _advancement_labels(current) != previous, name + f"_scroll_{page}")
        current = _advancement_labels(state)
        labels.update(current)
        if required.intersection(current):
            _capture(steps, name + f"-story-{page}")
    steps.check(name + "_story_nodes_accessible", required <= labels.keys())
    steps.report.setdefault("advancement_screens", {})[name] = labels
    _button(steps, "Done")
    steps.call("minecraft_close_screen")
    return state


def _place_ingredients(steps: ProbeSteps, item: str, targets: tuple[int, ...]) -> None:
    state = steps.observe()
    sources = [row for row in state["container"]["slots"]
               if row["slot"] >= 10 and row["item_id"] == item and row["count"] == len(targets)]
    steps.check(item + "_unique_crafting_source", len(sources) == 1)
    _click(steps, sources[0]["slot"])
    for target in targets:
        steps.call("minecraft_click_container_slot", {
            "slot": target, "button": "secondary", "timeout_seconds": steps.remaining(),
        })
    steps.check(item + "_crafting_cursor_empty", steps.observe()["container"]["carried"]["count"] == 0)


def _advancements(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    steps.report["recipes_before"] = steps.call("minecraft_read_recipe_book", {"limit": 8192})
    _advancement_screen(steps, "p4-advancements-fresh")
    steps.fixture("give @s minecraft:oak_log 1")
    _inventory(steps, "minecraft:oak_log", 1)
    steps.fixture("give @s minecraft:crafting_table 1")
    _inventory(steps, "minecraft:crafting_table", 1)
    _echo(steps, "wood_and_table_received")
    _advancement_screen(steps, "p4-advancements-root")
    steps.fixture("give @s minecraft:cobblestone 3")
    _inventory(steps, "minecraft:cobblestone", 3)
    steps.fixture("give @s minecraft:stick 2")
    _inventory(steps, "minecraft:stick", 2)
    table = _point(origin)
    steps.call("minecraft_select_hotbar_item", {"item_id": "minecraft:crafting_table", "count": 1,
                                               "timeout_seconds": steps.remaining()})
    _use(steps, _point(origin, dy=-1))
    _block(steps, table, "minecraft:crafting_table")
    _open(steps, table, "CraftingMenu", 46)
    _place_ingredients(steps, "minecraft:cobblestone", (1, 2, 3))
    _place_ingredients(steps, "minecraft:stick", (5, 8))
    result = steps.wait(lambda state: container_slot(state, 0)["item_id"] == "minecraft:stone_pickaxe",
                        "earned_stone_pickaxe_result")
    steps.report["crafting_result"] = container_slot(result, 0)
    _capture(steps, "p4-stone-pickaxe-crafting-result")
    _click(steps, 0, quick=True)
    steps.call("minecraft_close_screen")
    _inventory(steps, "minecraft:stone_pickaxe", 1)
    earned = _echo(steps, "stone_pickaxe_crafted_ordered_receipt")
    steps.report["completion_chat"] = earned["recent_chat"]
    steps.report["recipes_after"] = steps.call("minecraft_read_recipe_book", {"limit": 8192})
    _advancement_screen(steps, "p4-advancements-earned")
    _restart(steps)
    _inventory(steps, "minecraft:stone_pickaxe", 1)
    _advancement_screen(steps, "p4-advancements-after-restart")
    steps.report["recipes_after_restart"] = steps.call("minecraft_read_recipe_book", {"limit": 8192})
    # Reopen without awarding anything: progress/UI must not clear or announce again.
    _advancement_screen(steps, "p4-advancements-reopened")
    steps.report["visual_review_required"] = ["Minecraft story root", "Stone Age and Getting an Upgrade completion", "same completions after server restart", "completion chat; no join toast flood"]


def _wait_damage_identity(steps: ProbeSteps, predicate: Any, label: str, start: int,
                          **kwargs: Any) -> dict[str, Any]:
    """Wait for a received damage identity; a timeout reports the wire events it saw."""
    try:
        return steps.wait(predicate, label, **kwargs)
    except TimeoutError:
        steps.report.setdefault("damage_wire_on_timeout", {})[label] = steps.client.p4_damage_wire()[start:]
        raise


def _note_region(steps: ProbeSteps, label: str, origin: tuple[int, int, int], reach: int = 1) -> None:
    """Record the non-air cells (blocks and fluids) around the player's cell as the client sees them."""
    x, y, z = origin
    scan = steps.call("minecraft_scan_blocks", {
        "min_x": x - 1, "max_x": x + 1, "min_y": y - 1, "max_y": y + 3,
        "min_z": z - 1, "max_z": z + reach, "max_blocks": 128})
    steps.report.setdefault("region_cells", {})[label] = [
        row for row in scan.get("blocks", [])
        if row.get("block_id") != "minecraft:air" or not (row.get("fluid") or {}).get("empty", True)]


def _douse_and_drain(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    """Douse a burning player, then wait until the client sees no water around the fixture cell.

    Removing the sources leaves flowing water that decays for dozens of ticks; a player standing in
    it is cushioned from falls, cannot ignite and loses a berry bush to the flow."""
    x, y, z = origin
    steps.fixture(f"fill {x - 1} {y} {z - 1} {x + 1} {y + 3} {z + 1} minecraft:water")
    steps.call("minecraft_wait_ticks", {"ticks": 10})
    steps.fixture(f"fill {x - 1} {y} {z - 1} {x + 1} {y + 3} {z + 1} minecraft:air")

    def dry(_: dict[str, Any]) -> bool:
        scan = steps.call("minecraft_scan_blocks", {
            "min_x": x - 3, "max_x": x + 3, "min_y": y, "max_y": y + 3,
            "min_z": z - 3, "max_z": z + 3, "max_blocks": 256})
        return all((row.get("fluid") or {}).get("empty", True) for row in scan.get("blocks", []))

    steps.wait(dry, "douse_water_drained", timeout_seconds=60)


def _disable_gamerule(steps: ProbeSteps, rule: str) -> None:
    """Set a boolean rule false and wait for the server's acknowledgement."""
    steps.fixture(f"gamerule {rule} false")
    steps.wait(
        lambda state: f"Game rule {rule} is now set to false" in state.get("recent_chat", []),
        f"{rule}_disabled",
    )


# A falling stalactite deals 6 damage per fallen block (cap 40); from y+5 that is
# 24 and kills the 20 HP survival player before the scenario can clean up, so it
# starts lower. The anvil row keeps y+5 (2 per block, 8 damage).
_STALACTITE_DROP = 3


_DAMAGE_ZOMBIE_UUID = "00000000-0000-0000-0000-00000001e242"  # UUID:[I;0,0,0,123458]


def _damage(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    # The night below would otherwise spawn zombies that kill the survival player
    # during the long environment rows; authored mobs are summoned by command.
    _disable_gamerule(steps, "spawn_mobs")
    # Rain douses a burning player in the open; the on_fire row waits for the burn to outlast
    # the fire block, so the weather must stay clear however long the earlier rows ran.
    _disable_gamerule(steps, "advance_weather")
    steps.fixture("weather clear")
    steps.fixture("time set 18000")
    _echo(steps, "damage_night_fixture_ordered_receipt")
    steps.call("minecraft_wait_ticks", {"ticks": 20})
    steps.call("minecraft_look", {"yaw_deg": 0, "pitch_deg": 0})
    baseline = steps.observe()["player"]["health"]
    steps.report["baseline_health"] = baseline
    _capture(steps, "p4-damage-before")
    steps.fixture(f"summon zombie {x + .5} {y} {z + 2.5} {{UUID:[I;0,0,0,123458]}}")
    zombie: dict[str, Any] = {}
    def zombie_received(_: dict[str, Any]) -> bool:
        row = next((row for row in _rows(steps, 16)
                    if row.get("entity_uuid") == _DAMAGE_ZOMBIE_UUID), None)
        if row is None:
            return False
        zombie.update(row)
        return True
    steps.wait(zombie_received, "damage_zombie_authored_uuid_received")
    hit = steps.call("minecraft_wait_for_health_below", {"health": baseline, "timeout_seconds": steps.remaining()})
    steps.report["hit"] = hit
    # The hurt tilt lasts about ten client ticks, so it is observed before anything slow.
    hurt = steps.wait(lambda current: current["player"].get("hurt_tick") is not None
                      and abs(current["player"].get("hurt_tilt_deg", 0)) > .05,
                      "received_damage_event_nonzero_camera_tilt")
    # One received hit is the whole claim. Screenshots and observations below take
    # many client ticks; a living zombie keeps hitting and kills the survival
    # player before the cleanup fixture below reaches the server.
    steps.fixture(f"kill {_DAMAGE_ZOMBIE_UUID}")
    steps.report["hurt_observation"] = {key: hurt["player"].get(key) for key in
                                       ("hurt_tick", "hurt_tilt_deg", "hurt_direction", "health")}
    steps.report["hurt_observation"]["client_tick"] = hurt["client_tick"]
    player_id = hurt["player"]["entity_id"]
    damage_wire = steps.client.p4_damage_wire()
    mob_hits = [event for event in damage_wire
                if event["entity_id"] == player_id and event["type"] == "minecraft:mob_attack"]
    steps.check("zombie_hit_has_mob_attack_wire_identity", bool(mob_hits))
    steps.check("zombie_hit_has_source_identity",
                any(event["cause_entity_id"] == zombie["entity_id"] and
                    event["direct_entity_id"] == zombie["entity_id"] for event in mob_hits))
    steps.report["damage_wire_mob_hits"] = mob_hits
    _capture(steps, "p4-damage-hit")
    # One bounded adjacent check: live owner still accepts input and receives state.
    steps.call("minecraft_press_inputs", {"keys": ["back"], "ticks": 4})
    state = steps.observe()
    steps.check("post_hit_still_in_play", state["in_play"] is True and state["player"]["health"] > 0)
    _capture(steps, "p4-damage-after")
    steps.report["visual_review_required"] = ["hit frame hurt direction/tilt versus before", "no client errors or runtime disconnect"]

    _death_protection(steps, origin)

    _warden_damage(steps, origin)
    _damage_environment(steps, origin, steps.observe()["player"]["entity_id"])


def _death_protection(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    player_id = steps.observe()["player"]["entity_id"]
    steps.fixture("gamerule natural_health_regeneration false")
    # Real client receives and consumes a non-totem death-protection component.
    steps.fixture("gamemode creative @s")
    steps.fixture("effect give @s minecraft:instant_health 1 5 true")
    steps.fixture("effect give @s minecraft:slowness 60 0 true")
    steps.fixture("data get entity @s active_effects")
    steps.wait(lambda state: any("has the following entity data:" in line and
                               "minecraft:slowness" in line
                               for line in state.get("recent_chat", [])),
               "custom_death_protection_initial_effect_received")
    steps.fixture("clear @s")
    steps.fixture(
        "give @s minecraft:stick[death_protection={death_effects:["
        "{type:'minecraft:clear_all_effects'},"
        "{type:'minecraft:apply_effects',effects:[{id:'minecraft:speed',duration:2000}]}]}]"
    )
    _inventory(steps, "minecraft:stick", 1)
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.call("minecraft_wait_ticks", {"ticks": 25})
    generic_start = len(steps.client.p4_damage_wire())
    steps.fixture("gamemode survival @s")
    steps.fixture("damage @s 100 minecraft:generic")
    saved = steps.wait(lambda state: state["player"]["health"] == 1,
                       "custom_death_protection_prevents_death")
    steps.check("generic_command_damage_identity_received", any(
        event["entity_id"] == player_id and event["type"] == "minecraft:generic"
        for event in steps.client.p4_damage_wire()[generic_start:]))
    _inventory(steps, "minecraft:stick", 0)
    steps.fixture("data get entity @s active_effects")
    snapshot = steps.wait(
        lambda state: any("has the following entity data:" in line and "minecraft:speed" in line
                          for line in state.get("recent_chat", [])),
        "custom_death_protection_effects_received",
    )
    effects = next(line for line in snapshot["recent_chat"]
                   if "has the following entity data:" in line and "minecraft:speed" in line)
    steps.check("no_generic_totem_effects",
                all(name not in effects for name in (
                    "minecraft:regeneration", "minecraft:absorption", "minecraft:fire_resistance")))
    steps.check("custom_death_protection_clears_initial_effect",
                "minecraft:slowness" not in effects)
    steps.report["custom_death_protection"] = {"health": saved["player"]["health"], "effects": effects}
    _capture(steps, "custom-death-protection")


def _warden_damage(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    player_id = steps.observe()["player"]["entity_id"]
    # Provoke a real warden with ordinary lever vibrations, then observe its
    # sonic attack on the native connection (not a /damage identity alias).
    steps.fixture("gamemode creative @s")
    _disable_gamerule(steps, "natural_health_regeneration")
    steps.fixture("effect clear @s")
    steps.fixture("effect give @s minecraft:instant_health 1 5 true")
    steps.fixture(f"tp @s {x + .5} {y} {z + .5}")
    steps.wait(lambda state: state["player"]["health"] >= 18 and
               abs(state["player"]["z"] - z - .5) < .3, "warden_fixture_player_ready")
    lever = _point(origin, dz=-2)
    _set(steps, lever, "minecraft:lever[face=floor,facing=north]")
    steps.fixture(f"fill {x - 5} {y} {z - 4} {x + 5} {y + 4} {z - 4} minecraft:stone")
    steps.fixture(f"summon minecraft:warden {x + .5} {y} {z - 7.5} {{UUID:[I;0,0,0,123457]}}")
    warden: dict[str, Any] = {}
    def warden_received(_: dict[str, Any]) -> bool:
        row = next((row for row in _rows(steps, 16)
                    if row.get("entity_uuid") == "00000000-0000-0000-0000-00000001e241"), None)
        if row is None:
            return False
        warden.update(row)
        return True
    steps.wait(warden_received, "warden_authored_uuid_received")
    steps.fixture("gamemode survival @s")
    _echo(steps, "warden_fixture_ready")
    for _ in range(3):
        _use(steps, lever)
        steps.call("minecraft_wait_ticks", {"ticks": 50})
    steps.fixture("damage @s 13 minecraft:generic")
    # Observations lag the server by many ticks on a graphical client: the warden's
    # first sonic boom can land before the 7 HP state is ever observed, so "below
    # lethal" includes already dead. The sonic wire identity is asserted next.
    steps.wait(lambda state: state["player"]["health"] < 10, "warden_lethal_health_prepared")

    def sonic_received(_: dict[str, Any]) -> bool:
        return any(event["entity_id"] == player_id and event["type"] == "minecraft:sonic_boom"
                   and event["cause_entity_id"] == warden["entity_id"]
                   for event in steps.client.p4_damage_wire())
    steps.wait(sonic_received, "warden_sonic_damage_identity_received", timeout_seconds=60)
    steps.report["damage_wire_warden_sonic"] = [
        event for event in steps.client.p4_damage_wire()
        if event["entity_id"] == player_id and event["type"] == "minecraft:sonic_boom"
    ]
    death = steps.wait(lambda state: state["player"]["health"] == 0 and state["screen"]["open"],
                       "warden_sonic_death_screen_received")
    steps.report["warden_sonic_death_screen"] = death["screen"]
    steps.call("minecraft_respawn", {"timeout_seconds": steps.remaining()})
    steps.wait(lambda state: state["player"]["health"] > 0 and not state["screen"]["open"],
               "warden_sonic_respawn_received", wait_for_play=True)
    steps.fixture("gamemode creative @s")
    steps.fixture("kill @e[type=minecraft:warden]")
    steps.call("minecraft_wait_ticks", {"ticks": 70})


def _damage_identities(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    """Identity admission and emission, not absent physical implementations."""
    player_id = steps.observe()["player"]["entity_id"]
    steps.fixture("gamerule natural_health_regeneration false")
    events = []
    for kind in ("freeze", "fly_into_wall", "cramming", "sonic_boom",
                 "stalagmite", "dragon_breath", "indirect_magic"):
        steps.fixture("gamemode creative @s")
        steps.fixture("effect give @s minecraft:instant_health 1 5 true")
        steps.wait(lambda state: state["player"]["health"] == 20, f"{kind}_healed")
        steps.call("minecraft_wait_ticks", {"ticks": 25})
        steps.fixture("effect clear @s")
        start = len(steps.client.p4_damage_wire())
        steps.fixture("gamemode survival @s")
        steps.fixture(f"damage @s 2 minecraft:{kind}")
        steps.wait(lambda state: state["player"]["health"] < 20 and any(
            event["entity_id"] == player_id and event["type"] == f"minecraft:{kind}"
            for event in steps.client.p4_damage_wire()[start:]),
            f"{kind}_command_identity_received")
        events.extend(steps.client.p4_damage_wire()[start:])
    steps.report["command_damage_identities"] = events
    _warden_damage(steps, origin)
    steps.report["limits"] = [
        "Command identity cases do not claim freeze, cramming or wall collision mechanics.",
        "Physical source: angered warden sonic attack, including death and respawn.",
    ]


def _damage_environment(steps: ProbeSteps, origin: tuple[int, int, int], player_id: int) -> None:
    """Physical sources only; packet names alone are not evidence of mechanics."""
    x, y, z = origin
    fixtures = (
        ("in_wall", f"setblock {x} {y + 1} {z} minecraft:stone"),
        ("hot_floor", f"setblock {x} {y - 1} {z} minecraft:magma_block"),
        ("campfire", f"setblock {x} {y} {z} minecraft:campfire[lit=true]"),
        ("cactus", f"setblock {x + 1} {y} {z} minecraft:cactus"),
        ("sweet_berry_bush", f"setblock {x} {y} {z} minecraft:sweet_berry_bush[age=3]"),
        ("fall", f"tp @s {x + .5} {y + 10} {z + .5}"),
        ("stalagmite", f"tp @s {x + .5} {y + 10} {z + .5}"),
        ("falling_anvil", f"setblock {x} {y + 5} {z} minecraft:anvil"),
        ("falling_stalactite", f"setblock {x} {y + _STALACTITE_DROP} {z} minecraft:pointed_dripstone[vertical_direction=down,thickness=tip]"),
        ("arrow", f"summon minecraft:arrow {x + .5} {y + 1} {z + 4.5} {{Motion:[0d,0d,-1d]}}"),
        ("trident", f"summon minecraft:trident {x + .5} {y + 1} {z + 4.5} {{Motion:[0d,0d,-1d]}}"),
        ("thrown", f"summon minecraft:wind_charge {x + .5} {y + 1} {z + 4.5} {{Motion:[0d,0d,-1d]}}"),
        ("fireball", f"summon minecraft:small_fireball {x + .5} {y + 1} {z + 4.5} {{Motion:[0d,0d,-1d]}}"),
        ("explosion", f"summon minecraft:tnt {x + 5.5} {y} {z + .5}"),
        ("lightning_bolt", f"summon minecraft:lightning_bolt {x + .5} {y} {z + .5}"),
        ("wither", "effect give @s minecraft:wither 30 0"),
        ("poison", "effect give @s minecraft:poison 30 0"),
        ("magic", "effect give @s minecraft:instant_damage 1 0"),
        ("outside_border", "worldborder damage buffer 0"),
        ("starve", "effect give @s minecraft:hunger 120 255"),
        ("drown", f"fill {x - 1} {y} {z - 1} {x + 1} {y + 3} {z + 1} minecraft:water"),
        ("lava", f"setblock {x} {y} {z} minecraft:lava"),
        ("in_fire", f"setblock {x} {y} {z} minecraft:fire"),
    )
    results = []
    for kind, fixture in fixtures:
        steps.fixture("gamemode creative @s")
        _douse_and_drain(steps, origin)
        if kind in ("fall", "stalagmite", "falling_anvil", "falling_stalactite"):
            steps.fixture(f"fill {x} {y + 4} {z} {x} {y + 11} {z} minecraft:air")
        if kind == "falling_stalactite":
            steps.fixture(f"setblock {x} {y + _STALACTITE_DROP + 1} {z} minecraft:stone")
        if kind == "cactus":
            steps.fixture(f"setblock {x + 1} {y - 1} {z} minecraft:sand")
        steps.fixture(f"setblock {x} {y - 1} {z} minecraft:obsidian")
        steps.fixture(f"tp @s {x + .5} {y} {z + .5}")
        if kind == "outside_border":
            steps.fixture(f"worldborder center {x + 10} {z + .5}")
            steps.fixture("worldborder set 4")
        steps.fixture("effect clear @s")
        steps.fixture("effect give @s minecraft:instant_health 1 5 true")
        steps.wait(lambda state: state["player"]["health"] >= 18 and
                   abs(state["player"]["z"] - z - .5) < .3, f"{kind}_fixture_ready")
        steps.call("minecraft_wait_ticks", {"ticks": 25})
        baseline = steps.observe()["player"]["health"]
        _note_region(steps, f"{kind}_before_fixture", origin)
        start = len(steps.client.p4_damage_wire())
        steps.fixture("gamemode survival @s")
        if kind == "stalagmite":
            steps.fixture(f"setblock {x} {y} {z} minecraft:pointed_dripstone[vertical_direction=up,thickness=tip]")
        steps.fixture(fixture)
        if kind == "falling_stalactite":
            steps.fixture(f"setblock {x} {y + _STALACTITE_DROP + 1} {z} minecraft:air")
        if kind in ("cactus", "sweet_berry_bush"):
            steps.call("minecraft_look", {"yaw_deg": -90, "pitch_deg": 0})
            steps.call("minecraft_press_inputs", {"keys": ["forward"], "ticks": 10})
        identity = "magic" if kind == "poison" else kind
        def hit_received(state: dict[str, Any]) -> bool:
            return (state["player"]["health"] < baseline and any(
                event["entity_id"] == player_id and event["type"] == f"minecraft:{identity}"
                for event in steps.client.p4_damage_wire()[start:]))
        hit_state = _wait_damage_identity(steps, hit_received, f"{kind}_physical_identity_received", start,
                                          timeout_seconds=60 if kind == "starve" else 30)
        results.append({"type": kind, "health": hit_state["player"]["health"],
                        "events": steps.client.p4_damage_wire()[start:]})
        if kind == "outside_border":
            steps.fixture("worldborder set 59999968")
            steps.fixture("worldborder center 0 0")
        if kind == "starve":
            steps.fixture("effect clear @s")
            steps.fixture("effect give @s minecraft:saturation 1 20 true")
        if kind == "in_fire":
            before_burn = len(steps.client.p4_damage_wire())
            steps.fixture(f"setblock {x} {y} {z} minecraft:air")
            burned = _wait_damage_identity(
                steps, lambda state: state["player"]["health"] < hit_state["player"]["health"]
                and any(event["entity_id"] == player_id and event["type"] == "minecraft:on_fire"
                        for event in steps.client.p4_damage_wire()[before_burn:]),
                "on_fire_physical_identity_received", start, timeout_seconds=30)
            results.append({"type": "on_fire", "health": burned["player"]["health"],
                            "events": steps.client.p4_damage_wire()[before_burn:]})
    steps.fixture("gamemode creative @s")
    _douse_and_drain(steps, origin)
    steps.fixture(f"tp @s {x + .5} {y} {z + .5}")
    steps.fixture("effect give @s minecraft:instant_health 1 5 true")
    steps.fixture("clear @s")
    steps.fixture("give @s minecraft:splash_potion[potion_contents={potion:'minecraft:harming'}]")
    _inventory(steps, "minecraft:splash_potion", 1)
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.wait(lambda state: state["player"]["health"] >= 18, "indirect_magic_fixture_healed")
    steps.call("minecraft_wait_ticks", {"ticks": 25})
    baseline = steps.observe()["player"]["health"]
    start = len(steps.client.p4_damage_wire())
    steps.fixture("gamemode survival @s")
    steps.call("minecraft_look", {"yaw_deg": 0, "pitch_deg": 90})
    steps.call("minecraft_press_inputs", {"keys": ["use"], "ticks": 1})
    splash = _wait_damage_identity(steps, lambda state: state["player"]["health"] < baseline and any(
        event["entity_id"] == player_id and event["type"] == "minecraft:indirect_magic"
        for event in steps.client.p4_damage_wire()[start:]), "splash_potion_indirect_magic_received", start,
        timeout_seconds=30)
    results.append({"type": "indirect_magic", "health": splash["player"]["health"],
                    "events": steps.client.p4_damage_wire()[start:]})
    steps.fixture("gamemode creative @s")
    steps.fixture("effect give @s minecraft:instant_health 1 5 true")
    # The earlier rows run into dawn on a slow host; the sun would set the thorns zombie on fire
    # and its burn invulnerability swallows the player's hit.
    steps.fixture("time set 18000")
    steps.fixture(
        f'summon minecraft:zombie {x + .5} {y} {z + 2.5} '
        '{UUID:[I;0,0,0,123456],NoAI:1b,equipment:{chest:{id:"minecraft:diamond_chestplate",count:1,'
        'components:{"minecraft:enchantments":{"minecraft:thorns":10}}}}}'
    )
    target: dict[str, Any] = {}
    def thorns_target_received(_: dict[str, Any]) -> bool:
        row = next((row for row in _rows(steps, 8)
                    if row.get("entity_uuid") == "00000000-0000-0000-0000-00000001e240"), None)
        if row is None:
            return False
        target.update(row)
        return True
    steps.wait(thorns_target_received, "thorns_authored_uuid_received")
    steps.fixture(f"tp @s {x + .5} {y} {z + .5}")
    steps.wait(lambda state: state["player"]["health"] >= 18 and
               abs(state["player"]["x"] - x - .5) < .3 and abs(state["player"]["z"] - z - .5) < .3,
               "thorns_fixture_ready")
    steps.call("minecraft_wait_ticks", {"ticks": 25})
    baseline = steps.observe()["player"]["health"]
    start = len(steps.client.p4_damage_wire())
    steps.fixture("gamemode survival @s")
    # A creative player takes no reflected damage; the command and the attack packet are not
    # ordered, so let the server apply the mode before the hit.
    steps.call("minecraft_wait_ticks", {"ticks": 10})
    steps.call("minecraft_attack_entity_once", {
        **{key: target[key] for key in ("entity_id", "entity_uuid", "entity_type")},
        "timeout_seconds": steps.remaining(),
    })
    try:
        thorn_state = _wait_damage_identity(steps, lambda state: state["player"]["health"] < baseline and any(
            event["entity_id"] == player_id and event["type"] == "minecraft:thorns"
            for event in steps.client.p4_damage_wire()[start:]), "thorns_physical_identity_received", start,
            timeout_seconds=30)
    except TimeoutError:
        _note_region(steps, "thorns_timeout", origin, reach=3)
        raise
    steps.check("player_attack_physical_identity_received", any(
        event["entity_id"] == target["entity_id"] and event["type"] == "minecraft:player_attack"
        and event["cause_entity_id"] == player_id
        for event in steps.client.p4_damage_wire()[start:]))
    results.append({"type": "thorns", "health": thorn_state["player"]["health"],
                    "events": steps.client.p4_damage_wire()[start:]})
    steps.fixture("kill @e[type=minecraft:zombie,distance=..16]")
    steps.fixture("gamemode creative @s")
    steps.fixture("effect clear @s")
    steps.fixture("effect give @s minecraft:instant_health 1 5 true")
    steps.wait(lambda state: state["player"]["health"] >= 18, "void_fixture_healed")
    steps.call("minecraft_wait_ticks", {"ticks": 25})
    start = len(steps.client.p4_damage_wire())
    steps.fixture("gamemode survival @s")
    steps.fixture(f"tp @s {x + .5} -200 {z + .5}")
    # Below the world's block range the client shows no playable world (in_play false, no player
    # row), so health is not observable there: the row is the received out_of_world wire event
    # for the player and then the death screen.
    steps.wait(lambda state: any(
        event["entity_id"] == player_id and event["type"] == "minecraft:out_of_world"
        for event in steps.client.p4_damage_wire()[start:]), "void_physical_identity_received",
        outside_play=True, timeout_seconds=30)
    results.append({"type": "out_of_world", "events": steps.client.p4_damage_wire()[start:]})
    steps.wait(lambda state: state["screen"]["open"], "void_death_screen_received",
               outside_play=True)
    steps.call("minecraft_respawn", {"timeout_seconds": steps.remaining()})
    steps.wait(lambda state: state["player"]["health"] > 0 and not state["screen"]["open"],
               "void_respawn_received", wait_for_play=True)
    player_id = steps.observe()["player"]["entity_id"]
    start = len(steps.client.p4_damage_wire())
    steps.fixture("kill @s")
    killed = steps.wait(lambda state: state["player"]["health"] == 0 and state["screen"]["open"]
                        and any(event["entity_id"] == player_id and event["type"] == "minecraft:generic_kill"
                                for event in steps.client.p4_damage_wire()[start:]),
                        "kill_physical_identity_received")
    results.append({"type": "generic_kill", "screen": killed["screen"],
                    "events": steps.client.p4_damage_wire()[start:]})
    steps.call("minecraft_respawn", {"timeout_seconds": steps.remaining()})
    steps.wait(lambda state: state["player"]["health"] > 0 and not state["screen"]["open"],
               "kill_respawn_received", wait_for_play=True)
    steps.report["damage_wire_unexercised"] = [
        "freeze, cramming and fly_into_wall: no physical DamageKind producer found",
        "dragon_breath: dragon cloud deals indirect_magic; registry name alone is not physical evidence",
    ]
    steps.report["damage_wire_environment"] = results


def _hopper(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    hopper = _point(origin, dz=1)
    _set(steps, hopper, "minecraft:hopper[facing=down]")
    steps.fixture("give @s minecraft:diamond 1")
    _inventory(steps, "minecraft:diamond", 1)
    steps.call("minecraft_select_hotbar_item", {"item_id": "minecraft:diamond", "count": 1,
                                               "timeout_seconds": steps.remaining()})
    steps.call("minecraft_look", {"yaw_deg": 0, "pitch_deg": 0})
    steps.call("minecraft_wait_ticks", {"ticks": 4})
    _echo(steps, "hopper_throw_rotation_ordered_receipt")
    _capture(steps, "p4-hopper-before-q")
    before_q = steps.observe()
    steps.report["throw_tick_before"] = {"client_tick": before_q["client_tick"], "time": before_q["time"]}
    steps.call("minecraft_press_inputs", {"keys": ["key.keyboard.q"], "ticks": 1})
    _inventory(steps, "minecraft:diamond", 0)
    after_q = steps.observe()
    steps.report["throw_tick_after"] = {"client_tick": after_q["client_tick"], "time": after_q["time"]}
    steps.report["drop_trace"] = []
    elapsed = 0
    for ticks in (1, 2, 4, 8, 16, 32):
        steps.call("minecraft_wait_ticks", {"ticks": ticks})
        elapsed += ticks
        state = steps.observe()
        steps.report["drop_trace"].append({
            "wait_ticks_sum": elapsed,
            "client_tick": state["client_tick"],
            "time": state["time"],
            "diamond_inventory": item_count(state, "minecraft:diamond"),
            "items": [row for row in _rows(steps, 8) if row["entity_type"] == "minecraft:item"],
        })
        if elapsed in (3, 15):
            _capture(steps, f"p4-hopper-dropped-item-{elapsed}")
    opened = _open(steps, hopper, "HopperMenu", 41)
    received = steps.wait(lambda state: sum(row["count"] for row in state["container"]["slots"]
                                           if row["slot"] < 5 and row["item_id"] == "minecraft:diamond") == 1,
                          "hopper_received_exact_dropped_diamond")
    steps.check("hopper_no_player_repickup", item_count(received, "minecraft:diamond") == 0)
    steps.report["hopper_slots"] = [row for row in received["container"]["slots"] if row["slot"] < 5]
    _capture(steps, "p4-hopper-item-in-menu")
    slot = next(row["slot"] for row in received["container"]["slots"] if row["slot"] < 5 and row["item_id"] == "minecraft:diamond")
    _click(steps, slot, quick=True)
    steps.call("minecraft_close_screen")
    _inventory(steps, "minecraft:diamond", 1)
    steps.call("minecraft_wait_ticks", {"ticks": 20})
    reopened = _open(steps, hopper, "HopperMenu", 41)
    steps.check("adversarial_hopper_withdraw_no_duplicate", all(row["count"] == 0 for row in reopened["container"]["slots"] if row["slot"] < 5))
    _capture(steps, "p4-hopper-empty-after-withdraw")
    steps.call("minecraft_close_screen")
    steps.check("adversarial_exact_diamond_custody", item_count(steps.observe(), "minecraft:diamond") == 1)


def _statistics_request(steps: ProbeSteps, name: str) -> dict[tuple[str, str], int]:
    steps.call("minecraft_open_pause_menu")
    _button(steps, "Statistics")
    state = steps.wait(lambda current: current.get("statistics", {}).get("received") is True,
                       name + "_authoritative_stats_received")
    steps.check(name + "_still_play", state["in_play"] is True)
    steps.report.setdefault("statistics_receipts", {})[name] = state["statistics"]
    _capture(steps, name + "-general")
    for label in ("Items", "Mobs"):
        _button(steps, label)
        _capture(steps, name + "-" + label.lower())
    _button(steps, "Done")
    steps.call("minecraft_close_screen")
    return {(row["stat_type"], row["stat"]): row["value"] for row in state["statistics"]["counters"]}


def _stats(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    steps.call("minecraft_look", {"yaw_deg": 0, "pitch_deg": 0})
    steps.call("minecraft_press_inputs", {"keys": ["forward"], "ticks": 8})
    steps.call("minecraft_press_inputs", {"keys": ["back"], "ticks": 8})
    steps.call("minecraft_press_inputs", {"keys": ["jump"], "ticks": 1})
    steps.wait(lambda state: state["player"]["on_ground"], "statistics_jump_landed")
    steps.fixture("give @s minecraft:iron_pickaxe 1")
    _inventory(steps, "minecraft:iron_pickaxe", 1)
    steps.call("minecraft_select_hotbar_item", {"item_id": "minecraft:iron_pickaxe", "count": 1,
                                               "timeout_seconds": steps.remaining()})
    for dx, block, drop in ((1, "stone", "cobblestone"), (-1, "dirt", "dirt")):
        point = _point(origin, dx=dx)
        _set(steps, point, "minecraft:" + block)
        result = steps.call("minecraft_break_block", {
            **point, "face": "up", "expected_drop_item_id": "minecraft:" + drop,
            "expected_drop_count": 1, "timeout_seconds": steps.remaining(),
        })
        steps.check("statistics_" + block + "_mined_and_picked_up",
                    result.get("became_air") is True and result.get("pickup_confirmed") is True)
        steps.report.setdefault("statistics_mining", {})[block] = result
    before = _statistics_request(steps, "p4-statistics-earned")
    walk = before.get(("minecraft:custom", "minecraft:walk_one_cm"), 0)
    jump = before.get(("minecraft:custom", "minecraft:jump"), 0)
    steps.check("statistics_walk_plausible_nonzero", 0 < walk < 2000)
    steps.check("statistics_jump_plausible_nonzero", 1 <= jump <= 3)
    exercised = (("minecraft:mined", "minecraft:stone"), ("minecraft:mined", "minecraft:dirt"),
                 ("minecraft:picked_up", "minecraft:cobblestone"), ("minecraft:picked_up", "minecraft:dirt"))
    for stat in exercised:
        steps.check("statistics_exact_" + "/".join(stat), before.get(stat) == 1)
    _restart(steps)
    after = _statistics_request(steps, "p4-statistics-after-restart")
    for stat in exercised + (("minecraft:custom", "minecraft:jump"),):
        steps.check("statistics_persisted_" + "/".join(stat), after.get(stat) == before[stat])
    steps.check("statistics_walk_persisted", after.get(("minecraft:custom", "minecraft:walk_one_cm"), 0) >= walk)
    # A second ordinary request must replace absolute counters, not accumulate them.
    repeated = _statistics_request(steps, "p4-statistics-repeated-request")
    steps.check("adversarial_statistics_repeated_request_no_double_count",
                all(repeated.get(stat) == after[stat] for stat in exercised))


def run(steps: ProbeSteps, origin: tuple[int, int, int], surface: str) -> None:
    steps.fixture("gamemode survival @s")
    _echo(steps, "p4_survival_fixture_receipt")
    steps.report["surface"] = surface
    try:
        {"nameplates": _nameplates, "advancements": _advancements, "damage": _damage,
         "identities": _damage_identities, "death-protection": _death_protection,
         "hopper": _hopper, "statistics": _stats}[surface](steps, origin)
    except Exception:
        try:
            steps.report["failure_observation"] = steps.call("minecraft_observe")
            _capture(steps, "p4-" + surface + "-failure")
        except Exception as error:
            steps.report["failure_capture_error"] = str(error)
        raise
