"""Physical player-input regressions on the harness-owned native window."""
from __future__ import annotations

import ctypes as c
import time
import uuid

from . import native_client
from .menu_observer import holds, observe_menu
from .native_craft_drag import Pointer, center
from .native_v2 import _echo
from .probe_steps import ProbeSteps


def _input(steps: ProbeSteps, *, key: int | None = None, button: int | None = None,
           point: tuple[int, int] | None = None, control: bool = False) -> None:
    pointer = Pointer(steps.client.p4_window)
    before = steps.observe()["client_tick"]
    pointer.x.XKeysymToKeycode.argtypes = [c.c_void_p, c.c_ulong]
    pointer.x.XKeysymToKeycode.restype = c.c_ubyte
    pointer.xt.XTestFakeKeyEvent.argtypes = [c.c_void_p, c.c_uint, c.c_int, c.c_ulong]
    def event(code: int, down: bool) -> None:
        if not code or not pointer.xt.XTestFakeKeyEvent(pointer.display, code, int(down), 0):
            raise RuntimeError(f"owned Xvfb refused keycode {code}")
    controlcode = 0
    try:
        keycode = pointer.x.XKeysymToKeycode(pointer.display, key) if key is not None else None
        controlcode = pointer.x.XKeysymToKeycode(pointer.display, 0xffe3) if control else None
        if point is not None:
            pointer.move(*point)
        if control:
            event(controlcode, True)
        started = time.monotonic_ns()
        if key is not None:
            event(keycode, True)
            event(keycode, False)
        if button is not None:
            # Queue both edges before flushing; Pointer.button synchronizes each
            # edge separately, stretching short clicks by an X-server round trip.
            for down in (True, False):
                if not pointer.xt.XTestFakeButtonEvent(pointer.display, button, int(down), 0):
                    raise RuntimeError("owned Xvfb refused mouse button")
                pointer.down = button if down else None
        elapsed = (time.monotonic_ns() - started) / 1_000_000
    finally:
        if controlcode:
            event(controlcode, False)
        pointer.x.XSync(pointer.display, 0)
        pointer.close()
    steps.report.setdefault("physical_input_ms", []).append(elapsed)
    steps.check("physical_press_release_under_tick", elapsed < 50)
    steps.wait(lambda s: s["client_tick"] >= before + 2, "physical_input_processed")


def _shot(steps: ProbeSteps, label: str) -> dict:
    frame = native_client.capture(steps.client, steps.artifact_dir.parent, label)
    steps.report.setdefault("captures", {})[label] = frame
    return frame



def _seed_main(steps: ProbeSteps, item: str, count: int, slot: int) -> None:
    # debug give addresses hotbar slots only. Fill earlier slots, use ordinary
    # give, then remove the filler without relying on the input under test.
    for hotbar in range(9):
        steps.fixture(f"debug give minecraft:stone 64 {hotbar}")
    if slot > 9:
        steps.fixture(f"give @s minecraft:stone {64 * (slot - 9)}")
    steps.fixture(f"give @s {item} {count}")
    steps.fixture("clear @s minecraft:stone")

def _open_inventory(steps: ProbeSteps, creative: bool) -> dict:
    steps.call("minecraft_open_inventory")
    state = steps.wait(lambda s: s["screen"]["open"], "input_inventory_open")
    if creative:
        tab = next(n for n in state["ui_nodes"]
                   if n["id"].startswith("creative/tab/") and n["label"] == "Survival Inventory")
        x, y, w, h = tab["bounds"]
        _input(steps, button=1, point=(round(x + w / 2), round(y + h / 2)))
        state = steps.wait(lambda s: any(n["id"] == "slot/9" for n in s["ui_nodes"]),
                           "creative_inventory_tab_open")
    return state


def menus(steps: ProbeSteps, _origin: tuple[int, int, int]) -> None:
    from PIL import Image, ImageChops
    steps.fixture("tick freeze")
    _echo(steps, "menu_world_frozen")
    for creative in (False, True):
        mode = "creative" if creative else "survival"
        steps.fixture(f"gamemode {mode}")
        steps.fixture("clear @s")
        _seed_main(steps, "minecraft:diamond", 12, 9)
        _echo(steps, mode + "_fixture")
        state = _open_inventory(steps, creative)
        seeded = observe_menu(steps, mode + "_seeded")
        steps.check(mode + "_fixture_slot", holds(seeded["slots"][9], "minecraft:diamond", 12))
        # Each number key exchanges precisely the hovered slot, not the selected hand.
        for number in range(1, 10):
            point = center(state, 9)
            _input(steps, key=ord(str(number)), point=point)
            image = observe_menu(steps, f"{mode}_number_{number}")
            steps.check(f"{mode}_number_{number}_moves_hovered", holds(image["slots"][35 + number], "minecraft:diamond", 12)
                        and image["slots"][9]["count"] == 0)
            _input(steps, key=ord(str(number)), point=point)
            image = observe_menu(steps, f"{mode}_number_{number}_return")
            steps.check(f"{mode}_number_{number}_returns", holds(image["slots"][9], "minecraft:diamond", 12))
        _input(steps, key=ord("f"), point=center(state, 9))
        image = observe_menu(steps, mode + "_offhand")
        steps.check(mode + "_hovered_offhand", holds(image["slots"][45], "minecraft:diamond", 12))
        _input(steps, key=ord("f"), point=center(state, 9))
        _input(steps, key=ord("q"), point=center(state, 9))
        image = observe_menu(steps, mode + "_drop_one")
        steps.check(mode + "_q_one", holds(image["slots"][9], "minecraft:diamond", 11))
        _input(steps, key=ord("q"), control=True, point=center(state, 9))
        image = observe_menu(steps, mode + "_drop_stack")
        steps.check(mode + "_control_q_stack", image["slots"][9]["count"] == 0)
        steps.call("minecraft_close_screen")
        steps.fixture("clear @s")
        _seed_main(steps, "minecraft:emerald", 3, 10)
        _echo(steps, mode + "_move_fixture")
        state = _open_inventory(steps, creative)
        seeded = observe_menu(steps, mode + "_move_seeded")
        steps.check(mode + "_move_fixture_slot", holds(seeded["slots"][10], "minecraft:emerald", 3))
        pointer = Pointer(steps.client.p4_window)
        try:
            pointer.move(25, 25)
            steps.call("minecraft_wait_ticks", {"ticks": 2})
            empty = _shot(steps, mode + "-cursor-empty")
        finally:
            pointer.close()
        _input(steps, button=1, point=center(state, 10))
        image = observe_menu(steps, mode + "_short_pickup")
        # Creative SET_CREATIVE_MODE_SLOT has no carried-stack field: its cursor
        # is local. Prove that cursor with pixels below and the exact destination
        # stack after reopening, not by asking the server to own it.
        carried_ok = (image["carried"]["count"] == 0 if creative else
                      holds(image["carried"], "minecraft:emerald", 3))
        steps.check(mode + "_short_click_exact_pickup",
                    carried_ok and image["slots"][10]["count"] == 0)
        pointer = Pointer(steps.client.p4_window)
        try:
            pointer.move(25, 25)
            steps.call("minecraft_wait_ticks", {"ticks": 2})
            held = _shot(steps, mode + "-cursor-held")
        finally:
            pointer.close()
        with Image.open(empty["retained_path"]) as a, Image.open(held["retained_path"]) as b:
            diff = ImageChops.difference(a.convert("RGB").crop((0, 0, 80, 80)),
                                         b.convert("RGB").crop((0, 0, 80, 80)))
            steps.check(mode + "_held_cursor_pixels", sum(max(p) > 8 for p in diff.getdata()) > 30)
        _input(steps, button=1, point=center(state, 11))
        steps.call("minecraft_close_screen")
        _open_inventory(steps, creative)
        image = observe_menu(steps, mode + "_reopened")
        steps.check(mode + "_move_persists", holds(image["slots"][11], "minecraft:emerald", 3)
                    and image["slots"][10]["count"] == 0 and image["carried"]["count"] == 0)
        _shot(steps, mode + "-reopened")
        steps.call("minecraft_close_screen")
    steps.fixture("tick unfreeze")


def camera(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    from PIL import Image, ImageChops
    x, y, z = origin
    steps.fixture("tick freeze")
    _echo(steps, "freeze_ordered")
    before = steps.observe()
    first = _shot(steps, "frozen-before-teleport")
    steps.fixture(f"tp @s {x + 3.5} {y} {z + 0.5} 90 0")
    after = steps.wait(lambda s: abs(s["player"]["x"] - x - 3.5) < .05,
                       "frozen_teleport_received", wait_for_play=True)
    steps.call("minecraft_wait_ticks", {"ticks": 3})
    current = steps.observe()
    steps.check("frozen_client_clock_advances", current["client_tick"] > before["client_tick"])
    steps.check("frozen_camera_tracks_teleport", abs(current["player"]["camera_position"][0] - x - 3.5) < .05)
    second = _shot(steps, "frozen-after-teleport")
    steps.check("frozen_new_frame", first["frame_id"] != second["frame_id"])
    with Image.open(first["retained_path"]) as a, Image.open(second["retained_path"]) as b:
        steps.check("frozen_camera_changes_pixels", ImageChops.difference(a.convert("RGB"), b.convert("RGB")).getbbox() is not None)
    # Same fixed world/camera: only HUD state differs between these frames.
    steps.check("frozen_captured_camera_tracks_teleport",
                abs(second["camera_position"][0] - x - 3.5) < .05)
    shown = _shot(steps, "hud-shown")
    steps.call("minecraft_press_inputs", {"keys": ["key.keyboard.f1"], "ticks": 1})
    hidden = _shot(steps, "hud-hidden")
    steps.check("f1_hides_hud_setting", steps.observe()["client_settings"]["hide_hud"] is True)
    steps.call("minecraft_press_inputs", {"keys": ["key.keyboard.f1"], "ticks": 1})
    restored = _shot(steps, "hud-restored")
    steps.check("f1_restores_hud_setting", steps.observe()["client_settings"]["hide_hud"] is False)
    with Image.open(shown["retained_path"]) as a, Image.open(hidden["retained_path"]) as b, Image.open(restored["retained_path"]) as d:
        box = (a.width // 2 - 180, a.height - 45, a.width // 2 + 180, a.height)
        a, b, d = [im.convert("RGB").crop(box) for im in (a, b, d)]
        steps.check("f1_hides_hotbar_pixels", sum(max(p) > 20 for p in ImageChops.difference(a, b).getdata()) > 200)
        steps.check("f1_restores_hotbar_pixels", sum(max(p) > 20 for p in ImageChops.difference(d, b).getdata()) > 200)
    steps.fixture("tick unfreeze")
    _echo(steps, "unfreeze_ordered")


def pick(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    from .native_p1 import _aim
    from .native_v2 import _block
    x, y, z = origin
    steps.fixture(f"tp @s {x + .5} {y} {z + .5} 0 0")
    steps.wait(lambda s: abs(s["player"]["x"] - x - .5) < .05,
               "pick_player_position", wait_for_play=True)
    steps.fixture(f"setblock {x} {y + 1} {z + 3} minecraft:diamond_block")
    _block(steps, {"x": x, "y": y + 1, "z": z + 3}, "minecraft:diamond_block")
    for creative in (False, True):
        mode = "creative" if creative else "survival"
        steps.fixture(f"gamemode {mode}")
        steps.fixture("clear @s")
        if not creative:
            _seed_main(steps, "minecraft:diamond_block", 7, 12)
        steps.fixture("debug give minecraft:emerald 3 0")
        _echo(steps, mode + "_pick_fixture")
        steps.call("minecraft_set_hotbar_slot", {"slot": 0})
        steps.call("minecraft_look", {"yaw_deg": 0, "pitch_deg": 0})
        _input(steps, button=2)
        image = observe_menu(steps, mode + "_pick_block")
        diamonds = [(i, s) for i, s in enumerate(image["slots"]) if s and s.get("item") == "minecraft:diamond_block"]
        steps.check(mode + "_picked_block_hotbar", len(diamonds) == 1 and 36 <= diamonds[0][0] <= 44
                    and diamonds[0][1]["count"] == (1 if creative else 7))
        _input(steps, key=ord("f"))
        image = observe_menu(steps, mode + "_gameplay_offhand")
        steps.check(mode + "_gameplay_f_swaps", holds(image["slots"][45], "minecraft:diamond_block", 1 if creative else 7))
        _input(steps, key=ord("f"))
        steps.fixture("clear @s")
        _echo(steps, mode + "_empty_pick")
        _input(steps, button=2)
        image = observe_menu(steps, mode + "_empty_pick_result")
        steps.check(mode + "_no_free_survival_block",
                    sum(s["count"] for s in image["slots"] if s and s.get("item") == "minecraft:diamond_block") == int(creative))
    steps.fixture(f"summon minecraft:cow {x + 2.5} {y} {z + 2.5} {{NoAI:1b}}")
    cow = steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": "minecraft:cow", "radius": 8, "timeout_seconds": steps.remaining()})
    _aim(steps, cow)
    for creative in (False, True):
        mode = "creative" if creative else "survival"
        steps.fixture(f"gamemode {mode}")
        steps.fixture("clear @s")
        _echo(steps, mode + "_entity_fixture")
        _input(steps, button=2)
        image = observe_menu(steps, mode + "_pick_entity")
        steps.check(mode + "_entity_pick_custody",
                    sum(s["count"] for s in image["slots"] if s and s.get("item") == "minecraft:cow_spawn_egg") == int(creative))
    _shot(steps, "pick-block-and-entity")


# (double) 0.98f: 26.3 FallingBlockEntity.tick scales its motion by Entity.getAirDrag.
_FALL_DRAG = c.c_float(.98).value


def _vanilla_fall(row: dict, ticks: int) -> float:
    """Open-air 26.3 FallingBlockEntity.tick: 0.04 gravity, move, then air drag."""
    y, velocity = row["predicted_position"][1], row["velocity"]["y"]
    for _ in range(ticks):
        velocity -= .04
        y += velocity
        velocity *= _FALL_DRAG
    return y


def sand(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    from PIL import Image, ImageChops
    from .native_p1 import _aim
    from .native_v2 import _block
    x, y, z = origin
    steps.fixture("gamemode creative")
    steps.fixture("kill @e[type=!minecraft:player]")
    steps.fixture(f"fill {x - 1} {y} {z + 2} {x + 1} {y + 32} {z + 4} minecraft:air")
    steps.fixture(f"tp @s {x + .5} {y} {z - 5.5} 0 -55")
    steps.call("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    steps.fixture("tick freeze")
    steps.fixture(f"setblock {x} {y + 30} {z + 3} minecraft:sand")
    _echo(steps, "sand_frozen_setup")
    _shot(steps, "sand-source")
    steps.fixture("tick step 3")
    falling = steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": "minecraft:falling_block", "radius": 64, "timeout_seconds": steps.remaining()})
    steps.report["falling_spawn"] = falling
    # A frozen world holds the block between steps (vanilla TickRateManager.isEntityFrozen).
    # Read the server's Pos until two successive reads agree. Each read is sent after its chat
    # marker came back and the previous reply arrived, so the one reply newer than the marker
    # (recent_chat lists newest first) is its own, answered by a later server tick. The spawn
    # may arrive before `tick step 3` is spent; a held block then stops changing, a dropping
    # one never does and soon lands ("No entity was found").
    data, missing, reads = "has the following entity data: ", "No entity was found", []
    def reply(state: dict, marker: str) -> str | None:
        chat = [line for line in state["recent_chat"] if isinstance(line, str)]
        newer = chat[:next((i for i, line in enumerate(chat) if marker in line), 0)]
        return next((line for line in reversed(newer) if data in line or missing in line), None)
    for index in range(8):
        marker = f"sand hold {index} {uuid.uuid4().hex}"
        steps.call("minecraft_send_chat", {"message": marker, "command": False})
        steps.wait(lambda s, marker=marker: any(isinstance(line, str) and marker in line
                                                for line in s["recent_chat"]),
                   f"sand_hold_marker_{index}")
        steps.fixture("data get entity @e[type=minecraft:falling_block,limit=1] Pos")
        line = reply(steps.wait(lambda s, marker=marker: reply(s, marker) is not None,
                                f"sand_hold_read_{index}"), marker)
        reads.append(line.split(data, 1)[1] if data in line else None)
        if reads[-1] is not None and reads[-2:] == [reads[-1]] * 2:
            break
    steps.report["falling_server_pos"] = reads
    steps.check("sand_server_holds_frozen_block",
                len(reads) > 1 and reads[-1] is not None and reads[-1] == reads[-2])
    _aim(steps, falling)
    def sample() -> dict:
        rows = steps.call("minecraft_list_entities", {
            "radius": 64, "limit": 16, "entity_type": "minecraft:falling_block"})["entities"]
        return next(r for r in rows if r["entity_uuid"] == falling["entity_uuid"])

    def step_two(row: dict, label: str) -> dict:
        # One command runs both stepped ticks on consecutive server ticks.
        steps.fixture("tick step 2")
        current = row

        def stepped(_: dict) -> bool:
            nonlocal current
            current = sample()
            return current["presentation_tick"] >= row["presentation_tick"] + 2

        steps.wait(stepped, label)
        return current

    first = _shot(steps, "sand-entity")
    # 26.3 ServerChunkCache.tick runs ChunkMap.tick -> ServerEntity.sendChanges on
    # every server tick, frozen or not, so a moved falling block is resent every 20
    # server ticks of wall clock, not every 20 stepped ticks. At one tick per second
    # a window that starts on a received update and steps twice in one command ends
    # long before the next update; one an update still interrupts restarts from it.
    steps.fixture("tick rate 1")
    start = sample()
    received = step_two(start, "sand_client_step_prime")

    def updated(_: dict) -> bool:
        nonlocal received
        received = sample()
        return received["y"] != start["y"]

    steps.wait(updated, "sand_position_update_while_frozen", timeout_seconds=60)
    before, windows = received, []
    for index in range(4):
        after = step_two(before, f"sand_client_step_{index}")
        windows.append({"before": before, "after": after})
        if after["y"] == before["y"]:
            break
        before = after
    steps.fixture("tick rate 20")
    steps.report.update(falling_windows=windows, falling_before_step=before,
                        falling_after_step=after)
    steps.check("sand_two_steps_without_position_packet",
                after["y"] == before["y"]
                and after["presentation_tick"] == before["presentation_tick"] + 2)
    steps.check("sand_predicts_between_received_updates",
                abs(after["predicted_position"][1] - _vanilla_fall(before, 2)) < 1e-9)
    # Vanilla draws a frozen entity at partial tick 1 (its predicted position); any
    # drawn pose must lie on the predicted path, below what any packet supplied.
    presented = after["presented_position"][1]
    steps.check("sand_presents_predicted_descent",
                after["predicted_position"][1] <= presented
                < min(before["presented_position"][1], after["y"]))
    second = _shot(steps, "sand-descended")
    for frame in (first, second):
        steps.check("sand_entity_geometry_submitted",
                    frame["entity_extraction"]["entity_indices"] > 0
                    and frame["entity_geometry"]["gpu_completed_indices"] > 0)
    with Image.open(first["retained_path"]) as a, Image.open(second["retained_path"]) as b:
        steps.check("sand_fall_changes_pixels",
                    ImageChops.difference(a.convert("RGB"), b.convert("RGB")).getbbox() is not None)
    steps.fixture("tick unfreeze")
    _block(steps, {"x": x, "y": y, "z": z + 3}, "minecraft:sand")
    _block(steps, {"x": x, "y": y + 30, "z": z + 3}, "minecraft:air")
    steps.call("minecraft_look", {"yaw_deg": 0, "pitch_deg": 5})
    _shot(steps, "sand-landed")
    steps.check("setblock_sand_lands_and_source_clears", True)
