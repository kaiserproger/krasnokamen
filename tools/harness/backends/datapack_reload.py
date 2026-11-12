"""Native graphical datapack hot-reload gate (no restart, one live server).

A player puts one bone in the 2x2 crafting grid of the inventory screen and the
output slot shows the vanilla bone meal recipe. While the server runs, the
harness writes a directory pack that replaces that recipe; `/datapack enable`
installs it; the same craft then shows the pack's result in the client's own
output slot. `/datapack disable` brings the vanilla result back. The recipe book
the client holds is read around each reload.
"""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import zipfile
from pathlib import Path
from typing import Any

from .. import runtime
from ..profiles import ProfileContext
from . import native_client
from .chisel import rewrite_config
from .gameplay import prepare_operator_fixture, require_tools
from .probe_steps import ProbeSteps, container_slot, item_count

USERNAME = "ReloadGate"
SEED = 82
PACK = "gems"
PACK_ID = f"file/{PACK}"
RESULT_SLOT = 0
GRID_SLOT = 1
BONE = "minecraft:bone"
VANILLA_RESULT = ("minecraft:bone_meal", 3)
PACK_RESULT = ("minecraft:diamond", 2)
PACK_FILES = {
    "pack.mcmeta": json.dumps({"pack": {"description": "Reload gate", "min_format": 121, "max_format": 121}}),
    "data/minecraft/recipe/bone_meal.json": json.dumps({
        "type": "minecraft:crafting_shapeless",
        "ingredients": [BONE],
        "result": {"id": PACK_RESULT[0], "count": PACK_RESULT[1]},
    }),
    "data/test/predicate/dry.json": json.dumps({"condition": "minecraft:weather_check", "raining": False}),
    "data/test/function/hello.mcfunction": "say hello from the pack\n",
}
ZIP_PACK = "zipgems"
ZIP_ID = f"file/{ZIP_PACK}.zip"
TOOLS = frozenset({"minecraft_observe", "minecraft_wait_for_play", "minecraft_wait_for_state_change",
                   "minecraft_send_chat", "minecraft_wait_for_inventory", "minecraft_open_inventory",
                   "minecraft_click_container_slot", "minecraft_close_screen",
                   "minecraft_read_recipe_book"})


def validate_args(ctx: ProfileContext) -> float:
    if ctx.client != "native" or not ctx.windowed or ctx.extra_args or ctx.platform is not None:
        raise ValueError("datapack-reload requires --client native --windowed")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 600.0
    if not 0 < timeout <= 3600:
        raise ValueError("datapack-reload timeout must be in (0, 3600]")
    return timeout


def write_pack(world: Path) -> Path:
    """Create the pack folder inside the running server's world, like an operator would."""
    root = world / "datapacks" / PACK
    for relative, text in PACK_FILES.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text + "\n")
    return root


def run(ctx: ProfileContext) -> dict[str, Any]:
    timeout = validate_args(ctx)
    override = os.environ.get("SOLARIS_SERVER_BINARY")
    if not override or not os.environ.get("SOLARIS_CLIENT_BINARY"):
        raise RuntimeError("datapack-reload requires prebuilt SOLARIS_SERVER_BINARY and SOLARIS_CLIENT_BINARY")
    binary = Path(override).expanduser().resolve()
    if not binary.is_file():
        raise RuntimeError(f"missing server binary: {binary}")
    artifact = ctx.artifact_dir
    port = runtime.reserve_port()
    address = f"127.0.0.1:{port}"
    plugins = artifact / "plugins"
    plugins.mkdir()
    config = artifact / "datapack-reload.toml"
    # The server's own content cache, as client_probes uses it; the client's
    # SOLARIS_CLIENT_DATA_DIR is not a server content cache.
    vanilla = os.environ.get("SOLARIS_SERVER_VANILLA_DATA_DIR", "data/vanilla")
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    token = 'vanilla_data_dir = "data/vanilla"'
    if source.count(token) != 1:
        raise RuntimeError("expected one vanilla_data_dir config token")
    source = source.replace(token, f"vanilla_data_dir = {json.dumps(vanilla)}")
    world = artifact / "world"
    config.write_text(rewrite_config(source, port=port,
        world=world.relative_to(runtime.REPO_ROOT).as_posix() if world.is_relative_to(runtime.REPO_ROOT) else str(world),
        plugins=str(plugins), seed=SEED))
    prepare_operator_fixture(ctx, [USERNAME])
    with binary.open("rb") as handle:
        digest = hashlib.file_digest(handle, "sha256").hexdigest()
    ctx.client_evidence.update(server_address=address, server_binary_path=str(binary),
        server_binary_sha256=digest, world_dir=str(world), seed=SEED)
    server = client = xvfb = mcp = log = None
    try:
        log_path = artifact / "server.log"
        log = log_path.open("wb")
        ctx.logs.append(str(log_path))
        server = runtime.start_server([str(binary), "--config", str(config), "--no-console"],
            cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        runtime.wait_port(port, timeout, server, log_path=log_path)
        xvfb, display, _ = runtime.start_xvfb(artifact)
        ctx.logs.append(str(artifact / "xvfb.log"))
        client, _, mcp = native_client.start(ctx, native_client.resolve_binary(ctx), address,
                                              USERNAME, timeout, display=display)
        mcp.p4_window = ctx.client_evidence.get("window_focus")
        mcp.request_timeout_seconds = min(timeout + 10, 300.0)
        with ProbeSteps(mcp, artifact / "datapack-reload", timeout, "native-datapack-reload") as probe:
            require_tools(mcp.list_tools(), TOOLS, "datapack-reload")
            scenario(probe, artifact, world)
        return {"scenario": probe.report}
    finally:
        if mcp is not None:
            try:
                mcp.close()
            except Exception:
                pass
        runtime.stop_process(client)
        runtime.stop_process(server, interrupt=True)
        runtime.stop_process(xvfb)
        if log is not None:
            log.close()


def chat_says_after(recent_chat: list[Any], after: str, *markers: str) -> bool:
    """True when a line holding every marker is newer than the newest line holding `after`.

    The client's `recent_chat` is newest first (`.rev()` in the observation), so the lines newer than
    the fence are the ones before it in the list."""
    lines = [str(line) for line in recent_chat]
    fence = next((i for i, line in enumerate(lines) if after in line), None)
    return fence is not None and any(all(m in line for m in markers) for line in lines[:fence])


def scenario(probe: ProbeSteps, artifact: Path, world: Path) -> None:
    probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
    probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "player_present")
    probe.fixture(f"give @s {BONE} 1")
    probe.call("minecraft_wait_for_inventory", {"item_id": BONE, "count": 1, "timeout_seconds": probe.remaining()})
    probe.report["crafts"] = []

    def craft(label: str) -> tuple[str, int]:
        """Put the bone in the grid, read the client's output slot, take the bone back."""
        probe.call("minecraft_open_inventory", {})
        opened = probe.wait(lambda s: s["container"]["container_id"] == 0
            and any(row.get("item_id") == BONE for row in s["container"]["slots"]), f"{label}_inventory_open")
        source = next(int(row["slot"]) for row in opened["container"]["slots"]
                      if row.get("item_id") == BONE and int(row["slot"]) != GRID_SLOT)
        probe.call("minecraft_click_container_slot", {"slot": source, "button": "primary"})
        probe.call("minecraft_click_container_slot", {"slot": GRID_SLOT, "button": "primary"})
        shown = probe.wait(lambda s: container_slot(s, RESULT_SLOT)["item_id"] != "minecraft:air",
                           f"{label}_result_shown")
        result = container_slot(shown, RESULT_SLOT)
        probe.report["crafts"].append({"label": label, "result": result["item_id"], "count": int(result["count"])})
        probe.call("minecraft_click_container_slot", {"slot": GRID_SLOT, "button": "primary"})
        probe.call("minecraft_click_container_slot", {"slot": source, "button": "primary"})
        probe.wait(lambda s: container_slot(s, GRID_SLOT)["item_id"] == "minecraft:air"
                   and container_slot(s, RESULT_SLOT)["item_id"] == "minecraft:air", f"{label}_grid_cleared")
        probe.call("minecraft_close_screen")
        return result["item_id"], int(result["count"])

    def reported(state: dict[str, Any]) -> int:
        return sum(PACK_ID in str(line) for line in state.get("recent_chat", []))

    def barrier(label: str, command: str) -> None:
        """Run a pack command; the server's feedback line names the pack once it is done."""
        before = reported(probe.observe())
        probe.fixture(command)
        probe.wait(lambda s: reported(s) > before, label)

    def says(label: str, *markers: str) -> None:
        """Wait for a chat line the client received that holds every marker."""
        try:
            probe.wait(lambda s: any(all(m in str(line) for m in markers) for line in s.get("recent_chat", [])),
                       label, timeout_seconds=30)
        except TimeoutError:
            probe.report.setdefault("chat_at_timeout", {})[label] = probe.observe().get("recent_chat", [])
            raise

    def pack_barrier(label: str, command: str, pack: str) -> None:
        before = sum(pack in str(line) for line in probe.observe().get("recent_chat", []))
        probe.fixture(command)
        probe.wait(lambda s: sum(pack in str(line) for line in s.get("recent_chat", [])) > before, label)

    probe.report["recipe_book"] = {"before": probe.call("minecraft_read_recipe_book", {"limit": 8192})}
    probe.check("vanilla_recipe_before_reload", craft("vanilla") == VANILLA_RESULT)

    write_pack(world)
    # A pack dropped into the running server's folder is listed at once, without a reload.
    probe.fixture("datapack list available")
    says("list_discovers_new_pack", "available", PACK_ID)
    barrier("enable_reported", f"datapack enable {PACK_ID}")
    probe.report["recipe_book"]["enabled"] = probe.call("minecraft_read_recipe_book", {"limit": 8192})
    # The native MCP exposes only the book's display ids and counts, and a replaced recipe keeps its
    # display id (same set before and after the reload), so the craft output slot is the proof of
    # the recipe itself; the book reads are recorded for the report.
    probe.check("pack_recipe_after_hot_enable", craft("enabled") == PACK_RESULT)

    # The enabled pack's predicate and function work through the real command path.
    probe.fixture('tellraw @s[predicate=test:dry] {"text":"PredicateHit"}')
    says("selector_predicate_selects", "PredicateHit")
    probe.fixture("debug function test:hello")
    says("debug_function_writes_a_trace", "debug-trace")

    barrier("disable_reported", f"datapack disable {PACK_ID}")
    probe.report["recipe_book"]["disabled"] = probe.call("minecraft_read_recipe_book", {"limit": 8192})
    probe.check("vanilla_recipe_after_hot_disable", craft("disabled") == VANILLA_RESULT)

    # Vanilla pack, debug and data commands answer a real client.
    probe.fixture("datapack create madeit gate")
    says("create_reported", "Created new empty pack")  # pack jobs compile off-thread: one at a time
    probe.fixture("datapack list available")
    says("create_makes_a_listed_pack", "available", "file/madeit")
    probe.fixture("debug start")
    probe.fixture("debug stop")
    says("debug_profiler_reports", "tick profiling after")
    probe.fixture("data modify storage test:gate x set value 5")
    probe.fixture("data get storage test:gate x")
    says("data_storage_modified", "Storage data updated")
    probe.wait(lambda st: any(str(line) == "5" for line in st.get("recent_chat", [])), "data_storage_get_answers",
               timeout_seconds=30)

    # A ZIP pack enables live like a directory pack and changes the craft.
    zipped = world / "datapacks" / f"{ZIP_PACK}.zip"
    with zipfile.ZipFile(zipped, "w") as archive:
        for relative, text in PACK_FILES.items():
            archive.writestr(relative, text + "\n")
    pack_barrier("zip_enable_reported", f"datapack enable {ZIP_ID}", ZIP_ID)
    probe.check("zip_pack_recipe_after_hot_enable", craft("zip_enabled") == PACK_RESULT)
    pack_barrier("zip_disable_reported", f"datapack disable {ZIP_ID}", ZIP_ID)
    probe.check("vanilla_recipe_after_zip_disable", craft("zip_disabled") == VANILLA_RESULT)

    # Entries the server cannot read as packs never block a live /reload or /datapack list: a corrupt
    # ZIP and a pack without pack.mcmeta are skipped with a named error, a README is ignored.
    packs_dir = world / "datapacks"
    (packs_dir / "brokenzip.zip").write_bytes(b"PK not really a zip")
    (packs_dir / "nometa").mkdir()
    (packs_dir / "README.txt").write_text("not a pack\n")
    probe.fixture("datapack list available")
    says("list_survives_unreadable_entries", "available")
    probe.fixture("reload")
    says("reload_skips_unreadable_new_packs", "Skipping new datapack file/brokenzip.zip")
    says("reload_names_the_pack_without_mcmeta", "Skipping new datapack file/nometa")
    says("reload_still_completes", "Reloading!")  # vanilla text of commands.reload.success

    def says_after(label: str, after: str, *markers: str) -> None:
        """Wait for a chat line holding every marker that arrives after the line holding `after`."""
        try:
            probe.wait(lambda s: chat_says_after(s.get("recent_chat", []), after, *markers), label, timeout_seconds=30)
        except TimeoutError:
            probe.report.setdefault("chat_at_timeout", {})[label] = probe.observe().get("recent_chat", [])
            raise

    def fenced(marker: str) -> None:
        probe.fixture(f'tellraw @s {{"text":"{marker}"}}')
        says(f"fence_{marker}", marker)

    # A second /reload still names the pack it skips, and a long warning list is a few lines and a count
    # (the server log has every one).
    for index in range(4):
        (packs_dir / f"stray{index}.txt").write_text("x\n")
    fenced("Round2Begin")
    probe.fixture("reload")
    says_after("second_reload_still_names_the_skipped_pack", "Round2Begin", "Skipping new datapack file/brokenzip.zip")
    says_after("reload_counts_the_rest_instead_of_listing_them", "Round2Begin",
               "more datapack warning(s), see the server log")

    # Vanilla getPack: a pack that exists but is not selected is "not enabled", not unknown.
    fenced("Round3Begin")
    probe.fixture("datapack disable file/nometa")
    says_after("disable_of_a_skipped_pack_says_not_enabled", "Round3Begin", "is not enabled!")

    # A selected pack whose pack.mcmeta breaks keeps running, so the listing keeps it.
    pack_barrier("madeit_enable_reported", "datapack enable file/madeit", "file/madeit")
    (packs_dir / "madeit" / "pack.mcmeta").write_text('{"pack": {')
    fenced("Round4Begin")
    probe.fixture("datapack list enabled")
    says_after("list_keeps_a_selected_pack_that_no_longer_reads", "Round4Begin", "enabled", "file/madeit")
