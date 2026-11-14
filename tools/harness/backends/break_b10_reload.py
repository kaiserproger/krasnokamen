"""Breaker attack run for DATAPACK-HOT-RELOAD-RECIPES-LOOT (QA glue, not a gate).

One release server, one windowed native client. Packs are written into the
running server's world like an operator would; every check is recorded and
the run keeps going after a failed one.
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import traceback
from pathlib import Path
from typing import Any, Callable

from .. import runtime
from ..profiles import ProfileContext
from . import native_client
from .chisel import rewrite_config
from .datapack_reload import BONE, GRID_SLOT, RESULT_SLOT, validate_args
from .gameplay import prepare_operator_fixture
from .probe_steps import ProbeSteps, container_slot, item_count

USERNAME = "ReloadGate"
SEED = 82
MCMETA = {"pack": {"description": "Breaker pack", "min_format": 121, "max_format": 121}}


def recipe(result: str, count: int) -> dict[str, Any]:
    return {"type": "minecraft:crafting_shapeless", "ingredients": [BONE], "result": {"id": result, "count": count}}


def write_pack(world: Path, name: str, files: dict[str, Any]) -> None:
    root = world / "datapacks" / name
    for relative, content in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content if isinstance(content, str) else json.dumps(content))


def run(ctx: ProfileContext) -> dict[str, Any]:
    timeout = validate_args(ctx)
    binary = Path(os.environ["SOLARIS_SERVER_BINARY"]).expanduser().resolve()
    artifact = ctx.artifact_dir
    port = runtime.reserve_port()
    address = f"127.0.0.1:{port}"
    plugins = artifact / "plugins"
    plugins.mkdir()
    config = artifact / "datapack-reload.toml"
    vanilla = os.environ.get("SOLARIS_SERVER_VANILLA_DATA_DIR", "data/vanilla")
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    source = source.replace('vanilla_data_dir = "data/vanilla"', f"vanilla_data_dir = {json.dumps(vanilla)}")
    world = artifact / "world"
    config.write_text(rewrite_config(source, port=port,
        world=world.relative_to(runtime.REPO_ROOT).as_posix() if world.is_relative_to(runtime.REPO_ROOT) else str(world),
        plugins=str(plugins), seed=SEED))
    prepare_operator_fixture(ctx, [USERNAME])
    with binary.open("rb") as handle:
        digest = hashlib.file_digest(handle, "sha256").hexdigest()
    ctx.client_evidence.update(server_address=address, server_binary_path=str(binary),
        server_binary_sha256=digest, world_dir=str(world), seed=SEED)
    procs: dict[str, Any] = {"server": None}
    logs: list[Any] = []
    client = xvfb = mcp = None

    def start_server(n: int) -> None:
        log_path = artifact / f"server-{n}.log"
        log = log_path.open("wb")
        logs.append(log)
        ctx.logs.append(str(log_path))
        procs["server"] = runtime.start_server([str(binary), "--config", str(config), "--no-console"],
            cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        runtime.wait_port(port, timeout, procs["server"], log_path=log_path)

    try:
        start_server(0)
        xvfb, display, _ = runtime.start_xvfb(artifact)
        client, _, mcp = native_client.start(ctx, native_client.resolve_binary(ctx), address,
                                              USERNAME, timeout, display=display)
        mcp.p4_window = ctx.client_evidence.get("window_focus")
        mcp.request_timeout_seconds = min(timeout + 10, 300.0)
        with ProbeSteps(mcp, artifact / "break-reload", timeout, "break-b10-reload") as probe:
            attacks(probe, world, address,
                    restart=lambda n: (runtime.stop_process(procs["server"], interrupt=True), start_server(n)))
        return {"scenario": probe.report}
    finally:
        if mcp is not None:
            try:
                mcp.close()
            except Exception:
                pass
        runtime.stop_process(client)
        runtime.stop_process(procs["server"], interrupt=True)
        runtime.stop_process(xvfb)
        for log in logs:
            log.close()


def attacks(probe: ProbeSteps, world: Path, address: str, restart: Callable[[int], Any]) -> None:
    report = probe.report
    report["attacks"] = {}
    report["errors"] = {}
    report["crafts"] = []
    report["feedback"] = {}
    fences = {"dirt": 0}

    def soft(name: str, ok: bool, detail: Any = None) -> bool:
        report["attacks"][name] = {"ok": bool(ok), "detail": detail}
        return ok

    only = os.environ.get("BREAK_B10_ONLY", "").split(",") if os.environ.get("BREAK_B10_ONLY") else None

    def step(name: str, body: Callable[[], None]) -> None:
        if only is not None and name not in only:
            return
        try:
            body()
        except Exception as error:
            report["errors"][name] = "".join(traceback.format_exception_only(type(error), error)).strip()
            report["errors"][name + "_trace"] = traceback.format_exc()[-1500:]
            try:
                probe.call("minecraft_close_screen")
            except Exception:
                pass

    def chat(state: dict[str, Any]) -> list[str]:
        return [str(line) for line in state.get("recent_chat", [])]

    def place_marker(label: str) -> str:
        fences["mark"] = fences.get("mark", 0) + 1
        marker = f"MARK{fences['mark']}_{label}"
        probe.fixture(f'tellraw @s {{"text":"{marker}"}}')
        probe.wait(lambda s: marker in chat(s), f"{label}_marker", timeout_seconds=30)
        return marker

    def after(marker: str, state: dict[str, Any]) -> list[str]:
        """recent_chat is newest first: the lines above the marker arrived after it."""
        now = chat(state)
        return now[:now.index(marker)] if marker in now else now

    def command_feedback(label: str, command: str, done: Callable[[list[str]], bool] | None = None,
                         wait: float = 30) -> list[str]:
        """Run a command after a fresh marker line; wait until the lines after it satisfy `done` (default: any)."""
        marker = place_marker(label)
        probe.fixture(command)
        accept = done or (lambda lines: bool(lines))
        try:
            state = probe.wait(lambda s: accept(after(marker, s)), label, timeout_seconds=wait)
        except Exception as error:
            state = probe.observe()
            report["feedback"][label + "_wait_error"] = str(error)
        lines = after(marker, state)
        report["feedback"][label] = lines
        return lines

    def fence(label: str) -> dict[str, Any]:
        count = item_count(probe.observe(), "minecraft:dirt") + 1
        probe.fixture("give @s minecraft:dirt 1")
        probe.call("minecraft_wait_for_inventory", {"item_id": "minecraft:dirt", "count": count,
                                                    "timeout_seconds": probe.remaining(limit=40)})
        return probe.observe()

    def put_bone(label: str) -> int:
        probe.call("minecraft_open_inventory", {})
        opened = probe.wait(lambda s: s["container"]["container_id"] == 0
                            and any(row.get("item_id") == BONE for row in s["container"]["slots"]),
                            f"{label}_inventory_open", timeout_seconds=30)
        source = next(int(row["slot"]) for row in opened["container"]["slots"]
                      if row.get("item_id") == BONE and int(row["slot"]) != GRID_SLOT)
        probe.call("minecraft_click_container_slot", {"slot": source, "button": "primary"})
        probe.call("minecraft_click_container_slot", {"slot": GRID_SLOT, "button": "primary"})
        return source

    def take_bone(label: str, source: int) -> None:
        probe.call("minecraft_click_container_slot", {"slot": GRID_SLOT, "button": "primary"})
        probe.call("minecraft_click_container_slot", {"slot": source, "button": "primary"})
        probe.wait(lambda s: container_slot(s, GRID_SLOT)["item_id"] == "minecraft:air", f"{label}_grid_cleared",
                   timeout_seconds=30)

    def result_now(label: str, settle: float = 6) -> tuple[str, int]:
        try:
            shown = probe.wait(lambda s: container_slot(s, RESULT_SLOT)["item_id"] != "minecraft:air",
                               f"{label}_result_shown", timeout_seconds=settle)
            row = container_slot(shown, RESULT_SLOT)
            return row["item_id"], int(row["count"])
        except Exception:
            return "minecraft:air", 0

    def craft(label: str) -> tuple[str, int]:
        source = put_bone(label)
        result = result_now(label)
        report["crafts"].append({"label": label, "result": result[0], "count": result[1]})
        take_bone(label, source)
        probe.call("minecraft_close_screen")
        return result

    def book(label: str) -> Any:
        read = probe.call("minecraft_read_recipe_book", {"limit": 8192})
        text = json.dumps(read)
        summary = {"bone_meal_mentions": text.count("bone_meal"), "size": len(text)}
        report.setdefault("recipe_book", {})[label] = summary
        report.setdefault("recipe_book_raw", {})[label] = text[:2000]
        return summary

    def in_dim(name: str) -> Callable[[dict[str, Any]], bool]:
        return lambda s: name in json.dumps(s.get("dimension", s.get("player", {})))

    probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
    home = probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "player_present")["player"]
    HOME = f"{home['x']:.2f} {home['y'] + 0.5:.2f} {home['z']:.2f}"
    report["home"] = HOME
    probe.fixture(f"give @s {BONE} 1")
    probe.call("minecraft_wait_for_inventory", {"item_id": BONE, "count": 1, "timeout_seconds": probe.remaining()})
    # Dimension hops must not kill the player (a dead player has no @s for the marker lines).
    probe.fixture("effect give @s minecraft:resistance infinite 4 true")
    probe.fixture("effect give @s minecraft:fire_resistance infinite 0 true")

    VANILLA = ("minecraft:bone_meal", 3)
    DIAMOND = ("minecraft:diamond", 2)
    EMERALD = ("minecraft:emerald", 5)

    def r0() -> None:
        book("start")
        soft("r0_vanilla_baseline", craft("r0_vanilla") == VANILLA)
    step("r0_baseline", r0)

    def r1_invalid_json() -> None:
        write_pack(world, "broken", {"pack.mcmeta": MCMETA,
                                     "data/minecraft/recipe/bone_meal.json": '{"type": "minecraft:crafting_shapeless", '})
        lines = command_feedback("r1_enable_broken", "datapack enable file/broken")
        soft("r1_broken_pack_fails_clearly", any(word in l for l in lines for word in ("retained", "rror", "ail", "nvalid"))
             and not any("Enabled data pack" in l for l in lines), lines)
        soft("r1_old_recipe_kept", craft("r1_after_broken") == VANILLA)
        listed = command_feedback("r1_list_enabled", "datapack list enabled")
        soft("r1_broken_not_enabled", not any("broken" in l for l in listed), listed)
        shutil.rmtree(world / "datapacks" / "broken")
    step("r1_invalid_json", r1_invalid_json)

    def r2_enable() -> None:
        write_pack(world, "gems", {"pack.mcmeta": MCMETA, "data/minecraft/recipe/bone_meal.json": recipe(*DIAMOND),
                                   "data/gems/function/hello.mcfunction": 'tellraw @s {"text":"FuncHello"}\n'})
        command_feedback("r2_enable_gems", "datapack enable file/gems", lambda ls: any("file/gems" in l for l in ls))
        soft("r2_pack_recipe_after_enable", craft("r2_enabled") == DIAMOND)
        book("r2_enabled")
    step("r2_enable", r2_enable)

    def r3_edit_and_reload() -> None:
        write_pack(world, "gems", {"pack.mcmeta": MCMETA, "data/minecraft/recipe/bone_meal.json": recipe(*EMERALD)})
        lines = command_feedback("r3_reload", "reload")
        got = craft("r3_after_reload")
        if got != EMERALD:  # /reload may finish after its first line; give it one more fence
            fence("r3_reload_late")
            got = craft("r3_after_reload_retry")
        soft("r3_reload_picks_up_edited_file", got == EMERALD, {"feedback": lines, "craft": got})
    step("r3_edit_and_reload", r3_edit_and_reload)

    def r4_grid_open_during_disable() -> None:
        source = put_bone("r4")
        before = result_now("r4_before")
        report["r4_result_before"] = before
        lines = command_feedback("r4_disable_while_open", "datapack disable file/gems",
                                 lambda ls: any("file/gems" in l for l in ls))
        stale = probe.observe()
        report["r4_result_right_after_disable"] = container_slot(stale, RESULT_SLOT)
        # Change the grid: the result must now come from the new recipes.
        take_bone("r4_retake", source)
        probe.call("minecraft_click_container_slot", {"slot": source, "button": "primary"})
        probe.call("minecraft_click_container_slot", {"slot": GRID_SLOT, "button": "primary"})
        after = result_now("r4_after_regrid")
        report["r4_result_after_regrid"] = after
        soft("r4_regrid_after_disable_uses_new_recipes", after == VANILLA,
             {"before": before, "after": after, "feedback": lines})
        take_bone("r4_cleanup", source)
        probe.call("minecraft_close_screen")
        # Server truth: no stray pack result anywhere.
        state = fence("r4_fence")
        soft("r4_no_emerald_in_inventory", item_count(state, "minecraft:emerald") == 0)
    step("r4_grid_open_during_disable", r4_grid_open_during_disable)

    def r4b_take_stale_result() -> None:
        """Take the result a disabled pack left in the open grid: the client and the server must agree on what was made."""
        write_pack(world, "gems", {"pack.mcmeta": MCMETA, "data/minecraft/recipe/bone_meal.json": recipe(*EMERALD)})
        command_feedback("r4b_enable", "datapack enable file/gems", lambda ls: any("file/gems" in l for l in ls))
        put_bone("r4b")
        report["r4b_before"] = result_now("r4b_before")
        command_feedback("r4b_disable", "datapack disable file/gems", lambda ls: any("file/gems" in l for l in ls))
        report["r4b_shown_after_disable"] = container_slot(probe.observe(), RESULT_SLOT)
        try:
            probe.call("minecraft_quick_move_container_slot", {"slot": RESULT_SLOT})
        except Exception as error:
            report["r4b_take_error"] = str(error)
        try:
            client = probe.wait(lambda s: container_slot(s, GRID_SLOT)["item_id"] == "minecraft:air"
                                or item_count(s, "minecraft:emerald") > 0 or item_count(s, "minecraft:bone_meal") > 0,
                                "r4b_take_settled", timeout_seconds=15)
        except Exception as error:
            report["r4b_take_wait"] = str(error)
            client = probe.observe()
        report["r4b_client_after_take"] = {"emerald": item_count(client, "minecraft:emerald"),
                                           "bone_meal": item_count(client, "minecraft:bone_meal"),
                                           "bone": item_count(client, BONE),
                                           "grid": container_slot(client, GRID_SLOT)}
        probe.call("minecraft_close_screen")
        # Server truth: a reconnect resends the whole inventory.
        probe.call("minecraft_disconnect", {})
        probe.call("minecraft_connect", {"server_addr": address})
        probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
        server = probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "r4b_rejoined")
        report["r4b_server_after_rejoin"] = {"emerald": item_count(server, "minecraft:emerald"),
                                             "bone_meal": item_count(server, "minecraft:bone_meal"),
                                             "bone": item_count(server, BONE)}
        soft("r4b_client_and_server_agree", {k: v for k, v in report["r4b_client_after_take"].items() if k != "grid"}
             == report["r4b_server_after_rejoin"], report["r4b_server_after_rejoin"])
    step("r4b_take_stale_result", r4b_take_stale_result)

    def r5_two_quick_reloads() -> None:
        marker = place_marker("r5_quick")
        probe.fixture("datapack enable file/gems")
        probe.fixture("datapack disable file/gems")
        probe.fixture("reload")
        try:
            probe.wait(lambda s: len(after(marker, s)) >= 3, "r5_three_answers", timeout_seconds=30)
        except Exception as error:
            report["feedback"]["r5_quick_wait_error"] = str(error)
        fence("r5_fence")
        lines = [l for l in after(marker, probe.observe()) if "Gave 1 Dirt" not in l]
        report["feedback"]["r5_quick"] = lines
        listed = command_feedback("r5_list_enabled", "datapack list enabled")
        enabled = any("gems" in l for l in listed)
        got = craft("r5_after_quick")
        soft("r5_quick_reloads_end_consistent", got == (EMERALD if enabled else VANILLA),
             {"feedback": lines, "listed": listed, "craft": got})
        soft("r5_disable_right_after_enable_applies_like_vanilla", not enabled,
             {"feedback": lines, "note": "vanilla /datapack enable updates the selection before its reload finishes"})
        if not enabled:
            command_feedback("r5_reenable", "datapack enable file/gems", lambda ls: any("file/gems" in l for l in ls))
    step("r5_two_quick_reloads", r5_two_quick_reloads)

    def r6_nether() -> None:
        probe.fixture("execute in minecraft:the_nether run tp @s 0 80 0")
        probe.wait(in_dim("the_nether"), "r6_in_nether", wait_for_play=True, timeout_seconds=60)
        soft("r6_nether_sees_enabled_pack", craft("r6_nether_enabled") == EMERALD)
        lines = command_feedback("r6_function_in_nether", "function gems:hello")
        soft("r6_pack_function_runs_in_nether", any("FuncHello" in l for l in lines), lines)
        lines = command_feedback("r6_reload_in_nether", "reload")
        soft("r6_reload_from_nether_succeeds", any("Reloading" in l for l in lines)
             and not any("No datapack directory" in l for l in lines), lines)
        lines = command_feedback("r6_disable_in_nether", "datapack disable file/gems")
        nether_ok = any("file/gems" in l for l in lines)
        soft("r6_disable_from_nether_reported", nether_ok, lines)
        expected = VANILLA if nether_ok else EMERALD
        got = craft("r6_nether_after_disable")
        soft("r6_nether_craft_consistent_with_feedback", got == expected, {"craft": got, "nether_ok": nether_ok})
        report["feedback"]["r6_list_in_nether"] = command_feedback("r6_list_in_nether", "datapack list enabled")
        probe.fixture(f"execute in minecraft:overworld run tp @s {HOME}")
        probe.wait(in_dim("overworld"), "r6_back_overworld", wait_for_play=True, timeout_seconds=60)
        got = craft("r6_overworld_after_nether_disable")
        soft("r6_overworld_craft_consistent", got == expected, {"craft": got, "nether_ok": nether_ok})
        listed = command_feedback("r6_list_overworld", "datapack list enabled")
        soft("r6_overworld_selection_consistent", any("gems" in l for l in listed) == (not nether_ok), listed)
        lines = command_feedback("r6_function_overworld", "function gems:hello")
        report["feedback"]["r6_function_overworld"] = lines
        if nether_ok:
            lines = command_feedback("r6_reenable_overworld", "datapack enable file/gems",
                                     lambda ls: any("file/gems" in l for l in ls))
        soft("r6_enabled_again_in_overworld", craft("r6_reenabled") == EMERALD, lines)
    step("r6_nether", r6_nether)

    def r7_loot() -> None:
        write_pack(world, "lootgate", {"pack.mcmeta": MCMETA,
            "data/minecraft/loot_table/blocks/dirt.json": {"type": "minecraft:block", "pools": [
                {"rolls": 1, "entries": [{"type": "minecraft:item", "name": "minecraft:gold_ingot"}]}]}})
        def gold() -> int:
            return item_count(fence("r7_gold"), "minecraft:gold_ingot")
        before = command_feedback("r7_loot_before", "loot give @s loot minecraft:blocks/dirt")
        start = gold()
        soft("r7_vanilla_loot_no_gold", start == 0, before)
        command_feedback("r7_enable_loot", "datapack enable file/lootgate", lambda ls: any("file/lootgate" in l for l in ls))
        lines = command_feedback("r7_loot_enabled", "loot give @s loot minecraft:blocks/dirt")
        enabled_gold = gold()
        soft("r7_pack_loot_after_enable", enabled_gold == start + 1, {"feedback": lines, "gold": [start, enabled_gold]})
        # The same table from the Nether (registry adopted by the other world).
        probe.fixture("execute in minecraft:the_nether run tp @s 0 80 0")
        probe.wait(in_dim("the_nether"), "r7_in_nether", wait_for_play=True, timeout_seconds=60)
        lines = command_feedback("r7_loot_nether", "loot give @s loot minecraft:blocks/dirt")
        nether_gold = gold()
        soft("r7_pack_loot_in_nether", nether_gold == enabled_gold + 1, {"feedback": lines, "gold": nether_gold})
        probe.fixture(f"execute in minecraft:overworld run tp @s {HOME}")
        probe.wait(in_dim("overworld"), "r7_back", wait_for_play=True, timeout_seconds=60)
        command_feedback("r7_disable_loot", "datapack disable file/lootgate", lambda ls: any("file/lootgate" in l for l in ls))
        lines = command_feedback("r7_loot_disabled", "loot give @s loot minecraft:blocks/dirt")
        soft("r7_vanilla_loot_after_disable", gold() == nether_gold, lines)
    step("r7_loot", r7_loot)

    def r7b_block_and_chest_loot() -> None:
        """Loot the player actually gets: a mined stone block, and a chest table through /loot."""
        write_pack(world, "lootstone", {"pack.mcmeta": MCMETA,
            "data/minecraft/loot_table/blocks/stone.json": {"type": "minecraft:block", "pools": [
                {"rolls": 1, "entries": [{"type": "minecraft:item", "name": "minecraft:diamond"}]}]}})
        x, y, z = (int(float(v) // 1) for v in HOME.split())
        target = {"x": x + 1, "y": y, "z": z}
        probe.fixture("gamemode survival @s")
        probe.fixture("give @s minecraft:diamond_pickaxe 1")
        probe.call("minecraft_select_hotbar_item", {"item_id": "minecraft:diamond_pickaxe", "count": 1,
                                                     "timeout_seconds": probe.remaining()})

        def mine(label: str, expect: str) -> dict[str, int]:
            probe.fixture(f"setblock {x + 1} {y} {z} minecraft:stone")
            probe.call("minecraft_wait_for_block_state", {**target, "block_id": "minecraft:stone",
                                                          "timeout_seconds": 30})
            before = {item: item_count(probe.observe(), item) for item in ("minecraft:diamond", "minecraft:cobblestone")}
            probe.call("minecraft_look_at_block", {**target, "face": "west"})
            try:
                probe.call("minecraft_break_block", {**target, "face": "west", "expected_drop_item_id": expect,
                                                     "expected_drop_count": 1, "timeout_seconds": 30})
            except Exception as error:  # a different drop than expected: still read what arrived
                report.setdefault("mine_errors", {})[label] = str(error)
                fence(f"{label}_after_mine")
            after = {item: item_count(probe.observe(), item) for item in before}
            delta = {item: after[item] - before[item] for item in before}
            report.setdefault("mined", {})[label] = delta
            return delta

        soft("r7b_vanilla_stone_drops_cobblestone", mine("vanilla", "minecraft:cobblestone")["minecraft:cobblestone"] == 1)
        lines = command_feedback("r7b_enable", "datapack enable file/lootstone", lambda ls: any("lootstone" in l for l in ls))
        soft("r7b_block_loot_pack_enabled", any("Enabling data pack" in l for l in lines), lines)
        delta = mine("pack", "minecraft:diamond")
        soft("r7b_pack_stone_drops_diamond_after_hot_enable", delta == {"minecraft:diamond": 1, "minecraft:cobblestone": 0},
             delta)
        command_feedback("r7b_disable", "datapack disable file/lootstone", lambda ls: any("lootstone" in l for l in ls))
        delta = mine("disabled", "minecraft:cobblestone")
        soft("r7b_vanilla_drop_after_hot_disable", delta == {"minecraft:diamond": 0, "minecraft:cobblestone": 1}, delta)
        probe.fixture("gamemode creative @s")
    step("r7b_block_and_chest_loot", r7b_block_and_chest_loot)

    def r8_filter_removes_recipe() -> None:
        write_pack(world, "nobone", {"pack.mcmeta": {**MCMETA, "filter": {"block": [
            {"namespace": "minecraft", "path": "recipe/bone_meal.*"}]}},
            "data/nobone/function/noop.mcfunction": "say NoBone\n"})
        lines = command_feedback("r8_enable_filter", "datapack enable file/nobone last",
                                 lambda ls: any("nobone" in l or "rror" in l or "nsupported" in l for l in ls))
        got = craft("r8_filtered")
        soft("r8_pack_filter_removes_recipe", got == ("minecraft:air", 0), {"feedback": lines, "craft": got})
        book("r8_filtered")
        command_feedback("r8_disable_filter", "datapack disable file/nobone",
                         lambda ls: any("nobone" in l or "rror" in l for l in ls))
    step("r8_filter_removes_recipe", r8_filter_removes_recipe)

    def r8t_item_tag_pack() -> None:
        """A common pack shape: an item tag. Live enable must agree with what a restart accepts."""
        write_pack(world, "tagged", {"pack.mcmeta": MCMETA,
            "data/tagged/tags/item/gems.json": {"values": ["minecraft:diamond", "minecraft:emerald"]}})
        lines = command_feedback("r8t_enable_tagged", "datapack enable file/tagged",
                                 lambda ls: any("tagged" in l or "rror" in l or "nsupported" in l for l in ls))
        listed = command_feedback("r8t_list", "datapack list enabled")
        report["r8t_live_enabled"] = any("tagged" in l for l in listed)
        soft("r8t_live_answer_recorded", True, {"enable": lines, "listed": listed})
        if not report["r8t_live_enabled"]:
            shutil.rmtree(world / "datapacks" / "tagged")  # refused live; keep r10a about the enabled pack only
    step("r8t_item_tag_pack", r8t_item_tag_pack)

    def r9_reconnect() -> None:
        probe.call("minecraft_disconnect", {})
        probe.call("minecraft_connect", {"server_addr": address})
        probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
        probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "r9_player_present")
        report["r9_screen_after_reconnect"] = probe.observe().get("screen")
        try:
            probe.call("minecraft_close_screen")
        except Exception:
            pass
        book("r9_reconnected")
        soft("r9_reconnect_sees_pack_recipe", craft("r9_reconnected") == EMERALD)
    step("r9_reconnect", r9_reconnect)

    def r10_restart() -> None:
        try:
            restart(1)
        except Exception as error:
            soft("r10a_server_boots_after_restart", False, str(error)[-600:])
            return
        soft("r10a_server_boots_after_restart", True)
        probe.call("minecraft_connect", {"server_addr": address})
        probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
        probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "r10_player_present")
        listed = command_feedback("r10_list", "datapack list enabled")
        soft("r10a_restart_keeps_enabled_pack", craft("r10_restarted") == EMERALD, listed)
    step("r10_restart", r10_restart)

    def r10b_restart_after_refused_pack() -> None:
        """The brief's invalid-JSON case across a restart: live enable refuses it and keeps the old packs."""
        write_pack(world, "broken", {"pack.mcmeta": MCMETA,
                                     "data/minecraft/recipe/bone_meal.json": '{"type": "minecraft:crafting_shapeless", '})
        lines = command_feedback("r10b_enable_broken", "datapack enable file/broken")
        soft("r10b_live_refused", any("retained" in l for l in lines), lines)
        try:
            restart(2)
        except Exception as error:
            soft("r10b_server_boots_after_refused_pack", False, str(error)[-700:])
            return
        soft("r10b_server_boots_after_refused_pack", True)
    step("r10b_restart_after_refused_pack", r10b_restart_after_refused_pack)

    boots = {"n": 10}

    def reboot(label: str) -> tuple[bool, str]:
        """Restart the server; on success reconnect the client. Returns (started, server log)."""
        boots["n"] += 1
        n = boots["n"]
        try:
            restart(n)
            started = True
        except Exception as error:
            report.setdefault("boot_errors", {})[label] = str(error)[-800:]
            started = False
        log = (world.parent / f"server-{n}.log").read_text(errors="replace")
        report.setdefault("boot_logs", {})[label] = [l for l in log.splitlines()
                                                     if "atapack" in l or "ERROR" in l or "kipping" in l][:20]
        if started:
            probe.call("minecraft_connect", {"server_addr": address})
            probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
            probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), f"{label}_rejoined")
        return started, log

    def book_count(label: str) -> int:
        fence(f"{label}_book_fence")
        read = probe.call("minecraft_read_recipe_book", {"limit": 8192})
        report.setdefault("recipe_book", {})[label] = read
        return int(read.get("entry_count", -1))

    def rb_recipe_book() -> None:
        """Client recipe book follows a pack's new recipe in and out (entry count from the client's own book)."""
        write_pack(world, "shiny", {"pack.mcmeta": MCMETA, "data/breaker/recipe/shiny.json": {
            "type": "minecraft:crafting_shapeless", "ingredients": ["minecraft:dirt"],
            "result": {"id": "minecraft:emerald", "count": 7}}})
        start = book_count("rb_start")
        command_feedback("rb_enable", "datapack enable file/shiny", lambda ls: any("shiny" in l for l in ls))
        lines = command_feedback("rb_give", "recipe give @s breaker:shiny")
        given = book_count("rb_given")
        soft("rb_book_gains_pack_recipe", given == start + 1, {"start": start, "given": given, "feedback": lines})
        command_feedback("rb_disable", "datapack disable file/shiny", lambda ls: any("shiny" in l for l in ls))
        gone = book_count("rb_disabled")
        soft("rb_book_drops_removed_recipe", gone == start, {"start": start, "after_disable": gone})
        command_feedback("rb_reenable", "datapack enable file/shiny", lambda ls: any("shiny" in l for l in ls))
        back = book_count("rb_reenabled")
        report["rb_after_reenable"] = back
        command_feedback("rb_off", "datapack disable file/shiny", lambda ls: any("shiny" in l for l in ls))
    step("rb_recipe_book", rb_recipe_book)

    def rb2_recipe_book_follows_filter() -> None:
        """/recipe is not implemented, so use the recipes the player already knows: a filter that hides every
        minecraft recipe must empty the client's own book, disabling it must bring the entries back."""
        write_pack(world, "allrecipes", {"pack.mcmeta": {**MCMETA, "filter": {"block": [
            {"namespace": "^minecraft$", "path": "^recipe/"}]}}})
        start = book_count("rb2_start")
        command_feedback("rb2_on", "datapack enable file/allrecipes last", lambda ls: any("allrecipes" in l for l in ls))
        hidden = book_count("rb2_filtered")
        soft("rb2_client_book_drops_hidden_recipes", start > 0 and hidden == 0, {"start": start, "filtered": hidden})
        command_feedback("rb2_off", "datapack disable file/allrecipes", lambda ls: any("allrecipes" in l for l in ls))
        back = book_count("rb2_restored")
        soft("rb2_client_book_restored", back == start, {"start": start, "restored": back})
    step("rb2_recipe_book_follows_filter", rb2_recipe_book_follows_filter)

    def f1_filter_layers() -> None:
        """gems (emerald) < cut (filter bone_meal) < gems2 (diamond): the top pack wins, the filter hides only below."""
        write_pack(world, "cut", {"pack.mcmeta": {**MCMETA, "filter": {"block": [
            {"namespace": "minecraft", "path": "recipe/bone_meal\\.json"}]}}})
        write_pack(world, "gems2", {"pack.mcmeta": MCMETA, "data/minecraft/recipe/bone_meal.json": recipe(*DIAMOND)})
        command_feedback("f1_cut", "datapack enable file/cut last", lambda ls: any("file/cut" in l for l in ls))
        soft("f1_filter_hides_lower_pack", craft("f1_cut_on") == ("minecraft:air", 0))
        command_feedback("f1_gems2", "datapack enable file/gems2 last", lambda ls: any("gems2" in l for l in ls))
        soft("f1_pack_above_filter_wins", craft("f1_gems2_on_top") == DIAMOND)
        command_feedback("f1_gems2_off", "datapack disable file/gems2", lambda ls: any("gems2" in l for l in ls))
        soft("f1_back_to_filtered", craft("f1_gems2_off") == ("minecraft:air", 0))
        command_feedback("f1_cut_off", "datapack disable file/cut", lambda ls: any("file/cut" in l for l in ls))
        soft("f1_disable_filter_restores_lower", craft("f1_cut_off") == EMERALD)
        command_feedback("f1_cut_first", "datapack enable file/cut first", lambda ls: any("file/cut" in l for l in ls))
        soft("f1_filter_below_hides_nothing_above", craft("f1_cut_first") == EMERALD)
        command_feedback("f1_cut_off2", "datapack disable file/cut", lambda ls: any("file/cut" in l for l in ls))
    step("f1_filter_layers", f1_filter_layers)

    def f2_filter_patterns() -> None:
        cases = {
            "partial": ({"path": "bone_meal"}, ("minecraft:air", 0)),  # vanilla asPredicate(): find, any namespace
            "anchored_miss": ({"namespace": "^minecraft$", "path": "^bone_meal$"}, EMERALD),
            "namespace_other": ({"namespace": "^breaker$"}, EMERALD),
        }
        for name, (entry, want) in cases.items():
            write_pack(world, f"pat_{name}", {"pack.mcmeta": {**MCMETA, "filter": {"block": [entry]}}})
            lines = command_feedback(f"f2_{name}_on", f"datapack enable file/pat_{name} last",
                                     lambda ls, name=name: any(f"pat_{name}" in l for l in ls))
            got = craft(f"f2_{name}")
            soft(f"f2_{name}", got == want, {"entry": entry, "feedback": lines, "craft": got})
            command_feedback(f"f2_{name}_off", f"datapack disable file/pat_{name}",
                             lambda ls, name=name: any(f"pat_{name}" in l for l in ls))
        for name, entry in {"invalid_regex": {"path": "recipe/[bone"},
                            "java_lookahead": {"path": "recipe/(?!stick).*"},
                            "not_string": {"path": 5}}.items():
            write_pack(world, f"pat_{name}", {"pack.mcmeta": {**MCMETA, "filter": {"block": [entry]}}})
            lines = command_feedback(f"f2_{name}_on", f"datapack enable file/pat_{name}",
                                     lambda ls, name=name: any(f"pat_{name}" in l for l in ls))
            listed = command_feedback(f"f2_{name}_list", "datapack list enabled")
            report.setdefault("f2_refusals", {})[name] = {"enable": lines, "listed": listed}
            soft(f"f2_{name}_refused_naming_pack", any(f"pat_{name}" in l and "retained" in l for l in lines)
                 and not any(f"pat_{name}" in l for l in listed), {"enable": lines, "listed": listed})
            shutil.rmtree(world / "datapacks" / f"pat_{name}")
        soft("f2_still_emerald", craft("f2_end") == EMERALD)
    step("f2_filter_patterns", f2_filter_patterns)

    def s1_new_broken_dir_pack() -> None:
        write_pack(world, "newbroken", {"pack.mcmeta": MCMETA,
                                        "data/minecraft/recipe/bone_meal.json": '{"type": "minecraft:crafting_shapeless", '})
        lines = command_feedback("s1_reload_with_new_broken", "reload")
        report["s1_reload"] = lines
        started, log = reboot("s1")
        soft("s1_boots_skipping_new_broken_pack", started and "file/newbroken" in log and "kipping" in log,
             report["boot_logs"]["s1"])
        if started:
            soft("s1_gems_still_enabled", craft("s1_after") == EMERALD)
        shutil.rmtree(world / "datapacks" / "newbroken")
        if not started:
            reboot("s1_recover")
    step("s1_new_broken_dir_pack", s1_new_broken_dir_pack)

    def s2_scan_level_breakage() -> None:
        """Packs that break the folder scan itself: a corrupt zip, a malformed pack.mcmeta, a stray file."""
        root = world / "datapacks"
        cases = {
            "corrupt_zip": lambda: (root / "corrupt.zip").write_bytes(b"PK\x03\x04this is not a zip archive"),
            "bad_mcmeta": lambda: write_pack(world, "badmeta", {"pack.mcmeta": '{"pack": {"description": '}),
            "stray_file": lambda: (root / "README.txt").write_text("notes about my packs\n"),
        }
        cleanup = {"corrupt_zip": lambda: (root / "corrupt.zip").unlink(),
                   "bad_mcmeta": lambda: shutil.rmtree(root / "badmeta"),
                   "stray_file": lambda: (root / "README.txt").unlink()}
        for name, make in cases.items():
            make()
            lines = command_feedback(f"s2_{name}_reload", "reload")
            listed = command_feedback(f"s2_{name}_list", "datapack list")
            soft(f"s2_{name}_reload_succeeds", any("Reloading!" in l for l in lines), lines)
            soft(f"s2_{name}_list_succeeds", any("data pack(s) enabled" in l for l in listed), listed)
            started, _ = reboot(f"s2_{name}")
            soft(f"s2_{name}_server_starts", started,
                 {"reload": lines, "list": listed, "boot": report["boot_logs"][f"s2_{name}"],
                  "error": report.get("boot_errors", {}).get(f"s2_{name}")})
            cleanup[name]()
            if not started:
                reboot(f"s2_{name}_recover")
    step("s2_scan_level_breakage", s2_scan_level_breakage)

    def s3_selected_pack_breaks() -> None:
        """An enabled pack's file is broken on disk: /reload refuses and keeps the old data; then a restart."""
        write_pack(world, "gems", {"data/minecraft/recipe/bone_meal.json": '{"type": "minecraft:crafting_shapeless", '})
        lines = command_feedback("s3_reload", "reload")
        soft("s3_live_refusal_names_pack_and_file", any("file/gems" in l and "bone_meal.json" in l for l in lines), lines)
        soft("s3_old_recipe_kept_live", craft("s3_live") == EMERALD)
        started, _ = reboot("s3")
        report["s3_started"] = started
        soft("s3_boot_outcome_recorded", True, {"started": started, "boot": report["boot_logs"]["s3"],
                                                "error": report.get("boot_errors", {}).get("s3")})
        write_pack(world, "gems", {"data/minecraft/recipe/bone_meal.json": recipe(*EMERALD)})
        if not started:
            started, _ = reboot("s3_recover")
        if started:
            soft("s3_recovered_after_fix", craft("s3_fixed") == EMERALD)
    step("s3_selected_pack_breaks", s3_selected_pack_breaks)

    def s4_mixed_new_packs() -> None:
        """A broken new pack on disk vs an unrelated edit: /reload, then a start with good and bad new packs."""
        import zipfile
        root = world / "datapacks"
        write_pack(world, "gems", {"data/minecraft/recipe/bone_meal.json": recipe(*EMERALD)})
        write_pack(world, "nb2", {"pack.mcmeta": MCMETA,
                                  "data/minecraft/recipe/stick.json": '{"type": "minecraft:crafting_shaped", '})
        lines = command_feedback("s4_reload", "reload")
        got = craft("s4_after_reload")
        soft("s4_reload_applies_unrelated_edit_despite_new_broken_pack", got == EMERALD, {"reload": lines, "craft": got})
        write_pack(world, "goodnew", {"pack.mcmeta": MCMETA,
                                      "data/goodnew/function/hi.mcfunction": 'tellraw @s {"text":"GoodNewHi"}\n'})
        write_pack(world, "badfilter", {"pack.mcmeta": {**MCMETA, "filter": {"block": [{"path": "recipe/[x"}]}}})
        with zipfile.ZipFile(root / "badzip.zip", "w") as archive:
            archive.writestr("pack.mcmeta", json.dumps(MCMETA))
            archive.writestr("data/minecraft/recipe/torch.json", '{"type": ')
        started, log = reboot("s4")
        skipped = [l for l in log.splitlines() if "kipping new datapack" in l]
        soft("s4_start_skips_each_bad_new_pack", started and all(any(p in l for l in skipped)
             for p in ("file/nb2", "file/badfilter", "file/badzip.zip")), {"skipped": skipped,
                                                                            "error": report.get("boot_errors", {}).get("s4")})
        if started:
            lines = command_feedback("s4_good_function", "function goodnew:hi")
            soft("s4_good_new_pack_enabled", any("GoodNewHi" in l for l in lines), lines)
            soft("s4_edit_applied_at_start", craft("s4_after_start") == EMERALD)
        for name in ("nb2", "badfilter", "goodnew"):
            shutil.rmtree(root / name)
        (root / "badzip.zip").unlink()
        if not started:
            reboot("s4_recover")
    step("s4_mixed_new_packs", s4_mixed_new_packs)

    def zip_pack(path: Path, files: dict[str, Any]) -> None:
        import zipfile
        tmp = path.with_suffix(".tmp")
        with zipfile.ZipFile(tmp, "w") as archive:
            for relative, content in files.items():
                archive.writestr(relative, content if isinstance(content, str) else json.dumps(content))
        os.replace(tmp, path)

    def fn_pack(text: str) -> dict[str, Any]:
        return {"pack.mcmeta": MCMETA, f"data/flip/function/hi.mcfunction": f'tellraw @s {{"text":"{text}"}}\n'}

    def says(label: str, function: str, text: str) -> bool:
        lines = command_feedback(label, f"function {function}")
        return any(text in l for l in lines)

    def t1_break_fix_cycles() -> None:
        """A new pack goes broken -> fixed -> broken again between /reload calls; restarts in between."""
        write_pack(world, "flip", {**fn_pack("FlipV1"), "data/minecraft/recipe/stick.json": '{"type": '})
        write_pack(world, "gems", {"data/minecraft/recipe/bone_meal.json": recipe(*DIAMOND)})
        lines = command_feedback("t1_reload_new_broken", "reload", lambda ls: any("Reloading!" in l or "retained" in l for l in ls))
        soft("t1_new_broken_skipped_rest_applies", any("Reloading!" in l for l in lines)
             and any("file/flip" in l for l in lines) and craft("t1_gems_after_reload") == DIAMOND, lines)
        soft("t1_skipped_pack_not_enabled", not says("t1_fn_while_broken", "flip:hi", "FlipV1"))
        lines = command_feedback("t1_reload_again_still_broken", "reload", lambda ls: any("Reloading!" in l or "retained" in l for l in ls))
        report["t1_second_reload_same_broken"] = lines
        soft("t1_second_reload_still_names_skipped_pack", any("file/flip" in l for l in lines), lines)
        write_pack(world, "flip", {"data/minecraft/recipe/stick.json": {
            "type": "minecraft:crafting_shapeless", "ingredients": ["minecraft:bone"], "result": {"id": "minecraft:stick", "count": 9}}})
        (world / "datapacks" / "flip" / "data" / "minecraft" / "recipe" / "stick.json").unlink()
        lines = command_feedback("t1_reload_fixed", "reload", lambda ls: any("Reloading!" in l or "retained" in l for l in ls))
        soft("t1_fixed_pack_enabled_on_reload", says("t1_fn_fixed", "flip:hi", "FlipV1"), lines)
        started, _ = reboot("t1_after_fix")
        soft("t1_restart_keeps_fixed_pack", started and says("t1_fn_after_restart", "flip:hi", "FlipV1"))
        write_pack(world, "flip", {"pack.mcmeta": MCMETA, "data/flip/function/hi.mcfunction": 'tellraw @s {"text":"FlipV2"}\n',
                                   "data/minecraft/recipe/stick.json": '{"type": '})
        lines = command_feedback("t1_reload_saved_broken", "reload", lambda ls: any("Reloading!" in l or "retained" in l for l in ls))
        soft("t1_saved_broken_refused_naming_pack_and_file", any("file/flip" in l and "stick.json" in l and "retained" in l
                                                                  for l in lines), lines)
        soft("t1_saved_broken_keeps_old_function", says("t1_fn_after_refusal", "flip:hi", "FlipV1"))
        started, _ = reboot("t1_saved_broken_restart")
        report["t1_saved_broken_restart_started"] = started
        soft("t1_saved_broken_restart_outcome_recorded", True,
             {"started": started, "boot": report["boot_logs"]["t1_saved_broken_restart"]})
        shutil.rmtree(world / "datapacks" / "flip")
        if not started:
            started, _ = reboot("t1_recover")
        soft("t1_recovers_after_removing_pack", started and craft("t1_recovered") == DIAMOND)
    step("t1_break_fix_cycles", t1_break_fix_cycles)

    def t2_saved_pack_deleted_vs_corrupted() -> None:
        root = world / "datapacks"
        backup = world.parent / "gems-backup"
        shutil.copytree(root / "gems", backup, dirs_exist_ok=True)
        (root / "gems" / "pack.mcmeta").write_text('{"pack": {"description": ')
        lines = command_feedback("t2_reload_corrupted", "reload", lambda ls: any("Reloading!" in l or "retained" in l for l in ls))
        soft("t2_corrupted_saved_pack_refused_naming_it", any("file/gems" in l and "retained" in l for l in lines), lines)
        soft("t2_corrupted_keeps_old_recipe", craft("t2_corrupted_live") == DIAMOND)
        listed = command_feedback("t2_list_corrupted", "datapack list")
        report["t2_list_corrupted"] = listed
        soft("t2_list_works_with_corrupted_saved_pack", any("data pack(s) enabled" in l for l in listed), listed)
        started, _ = reboot("t2_corrupted_restart")
        report["t2_corrupted_restart_started"] = started
        soft("t2_corrupted_restart_outcome_recorded", True,
             {"started": started, "boot": report["boot_logs"]["t2_corrupted_restart"]})
        shutil.rmtree(root / "gems")
        shutil.copytree(backup, root / "gems")
        if not started:
            started, _ = reboot("t2_corrupted_recover")
        soft("t2_recovers_after_fixing_mcmeta", started and craft("t2_fixed") == DIAMOND)
        shutil.rmtree(root / "gems")
        lines = command_feedback("t2_reload_deleted", "reload", lambda ls: any("Reloading!" in l or "retained" in l for l in ls))
        got = craft("t2_deleted_live")
        soft("t2_deleted_saved_pack_reload_falls_back_to_vanilla", got == VANILLA, {"reload": lines, "craft": got})
        started, _ = reboot("t2_deleted_restart")
        soft("t2_restart_after_delete", started and craft("t2_deleted_restart_craft") == VANILLA,
             report["boot_logs"]["t2_deleted_restart"])
        shutil.copytree(backup, root / "gems")
        lines = command_feedback("t2_reload_restored", "reload", lambda ls: any("Reloading!" in l or "retained" in l for l in ls))
        listed = command_feedback("t2_list_restored", "datapack list enabled")
        report["t2_restored"] = {"reload": lines, "listed": listed, "craft": craft("t2_restored_craft")}
        if not any("gems" in l for l in listed):
            command_feedback("t2_reenable", "datapack enable file/gems", lambda ls: any("file/gems" in l for l in ls))
    step("t2_saved_pack_deleted_vs_corrupted", t2_saved_pack_deleted_vs_corrupted)

    def t3_zip_replaced_mid_reload() -> None:
        root = world / "datapacks"
        big = {f"data/zipfn/function/f{i}.mcfunction": "say filler\n" * 40 for i in range(3000)}
        zip_pack(root / "zipfn.zip", {"pack.mcmeta": MCMETA, "data/zipfn/function/hi.mcfunction": 'tellraw @s {"text":"ZipV1"}\n', **big})
        command_feedback("t3_reload_v1", "reload", lambda ls: any("Reloading!" in l or "retained" in l for l in ls))
        soft("t3_zip_v1_enabled", says("t3_fn_v1", "zipfn:hi", "ZipV1"))
        # An operator copies a new zip over the old one while a reload runs: the reload sees a half-written file.
        good = (root / "zipfn.zip").read_bytes()
        marker = place_marker("t3_race")
        probe.fixture("reload")
        with open(root / "zipfn.zip", "r+b") as handle:
            handle.truncate(len(good) // 3)
        try:
            probe.wait(lambda s: any("Reloading!" in l or "retained" in l for l in after(marker, s)), "t3_race_done",
                       timeout_seconds=30)
        except Exception as error:
            report["t3_race_wait"] = str(error)
        lines = after(marker, probe.observe())
        report["t3_race"] = lines
        alive = says("t3_fn_after_race", "zipfn:hi", "ZipV1")
        soft("t3_race_keeps_or_refuses_cleanly", bool(lines) and (alive or any("retained" in l for l in lines)),
             {"race": lines, "function_still_v1": alive})
        lines = command_feedback("t3_reload_truncated", "reload", lambda ls: any("Reloading!" in l or "retained" in l for l in ls))
        soft("t3_truncated_saved_zip_refused_naming_it", any("zipfn.zip" in l and "retained" in l for l in lines), lines)
        zip_pack(root / "zipfn.zip", {"pack.mcmeta": MCMETA, "data/zipfn/function/hi.mcfunction": 'tellraw @s {"text":"ZipV2"}\n'})
        lines = command_feedback("t3_reload_v2", "reload", lambda ls: any("Reloading!" in l or "retained" in l for l in ls))
        soft("t3_zip_v2_after_full_copy", says("t3_fn_v2", "zipfn:hi", "ZipV2"), lines)
        started, _ = reboot("t3_restart")
        soft("t3_restart_with_v2", started and says("t3_fn_v2_restart", "zipfn:hi", "ZipV2"), report["boot_logs"]["t3_restart"])
    step("t3_zip_replaced_mid_reload", t3_zip_replaced_mid_reload)

    def t4_many_broken_entries() -> None:
        root = world / "datapacks"
        made: list[Path] = []
        for i in range(15):
            (root / f"notes{i}.txt").write_text("x\n"); made.append(root / f"notes{i}.txt")
            (root / f"emptydir{i}").mkdir(exist_ok=True); made.append(root / f"emptydir{i}")
        for i in range(10):
            (root / f"bad{i}.zip").write_bytes(b"PK\x03\x04junk"); made.append(root / f"bad{i}.zip")
            write_pack(world, f"badrec{i}", {"pack.mcmeta": MCMETA, "data/minecraft/recipe/stick.json": '{"type": '})
            made.append(root / f"badrec{i}")
        lines = command_feedback("t4_reload", "reload", lambda ls: any("Reloading!" in l or "retained" in l for l in ls))
        fence("t4_after_reload")
        state = probe.observe()
        report["t4_chat_tail"] = state.get("recent_chat", [])[:25]
        got = craft("t4_craft")
        soft("t4_reload_applies_with_50_bad_entries", got == DIAMOND,
             {"craft": got, "note": "Reloading! may scroll out of the 20-line MCP chat log", "tail": lines[-3:]})
        listed = command_feedback("t4_list", "datapack list")
        soft("t4_list_works", any("data pack(s) enabled" in l for l in listed), listed)
        started, log = reboot("t4_restart")
        report["t4_restart_warnings"] = sum(1 for l in log.splitlines() if "WARN" in l and ("kipping" in l or "non-pack" in l))
        soft("t4_restart_with_50_bad_entries", started and craft("t4_restart_craft") == DIAMOND,
             {"warnings": report["t4_restart_warnings"], "error": report.get("boot_errors", {}).get("t4_restart")})
        for path in made:
            shutil.rmtree(path) if path.is_dir() else path.unlink()
        if not started:
            reboot("t4_recover")
    step("t4_many_broken_entries", t4_many_broken_entries)

    def t5_enable_disable_skipped() -> None:
        root = world / "datapacks"
        write_pack(world, "skipme", {"pack.mcmeta": MCMETA, "data/minecraft/recipe/stick.json": '{"type": '})
        (root / "skipzip.zip").write_bytes(b"PK\x03\x04junk")
        command_feedback("t5_reload", "reload", lambda ls: any("Reloading!" in l or "retained" in l for l in ls))
        for pack in ("file/skipme", "file/skipzip.zip"):
            lines = command_feedback(f"t5_enable_{pack}", f"datapack enable {pack}")
            soft(f"t5_enable_{pack}_refused_naming_it", any(pack in l for l in lines)
                 and not any("Enabling data pack" in l for l in lines), lines)
            lines = command_feedback(f"t5_disable_{pack}", f"datapack disable {pack}")
            report.setdefault("t5_disable", {})[pack] = lines
        listed = command_feedback("t5_list", "datapack list")
        report["t5_list"] = listed
        soft("t5_selection_unchanged", craft("t5_craft") == DIAMOND, listed)
        started, _ = reboot("t5_restart")
        soft("t5_restart_with_skipped_packs", started and craft("t5_restart_craft") == DIAMOND,
             report["boot_logs"]["t5_restart"])
        shutil.rmtree(root / "skipme"); (root / "skipzip.zip").unlink()
        if not started:
            reboot("t5_recover")
    step("t5_enable_disable_skipped", t5_enable_disable_skipped)

    failed = sorted([name for name, row in report["attacks"].items() if not row["ok"]]
                    + [name for name in report["errors"] if not name.endswith("_trace")])
    report["failed"] = failed
    if failed:
        raise RuntimeError(f"break-b10-reload failures: {failed}")
