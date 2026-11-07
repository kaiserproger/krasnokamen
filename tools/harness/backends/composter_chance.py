"""Native graphical hopper-composter chance gate (Java 26.3 static oracle).

Sixteen independently reset level-1 composters receive one wheat seed each.
The player loads a locked hopper through its menu, then unlocks it. The hopper
menu and client block state independently witness the debit and resulting level.
For 30% success, sixteen successes have probability 0.3**16 under the oracle.
This is a statistical parity check, not proof of the full distribution.
"""
from __future__ import annotations

import hashlib
import json
import math
import os
import subprocess
from pathlib import Path
from typing import Any

from .. import runtime
from ..profiles import ProfileContext
from . import native_client
from .chisel import _pad_commands, rewrite_config
from .gameplay import prepare_operator_fixture, require_tools
from .probe_steps import ProbeSteps, item_count

USERNAME = "CompostGate"
SEED = 81
SAMPLES = 16
TOOLS = frozenset({"minecraft_observe", "minecraft_wait_for_play", "minecraft_wait_for_state_change",
                   "minecraft_wait_for_block_state", "minecraft_read_block", "minecraft_send_chat",
                   "minecraft_set_hotbar_slot", "minecraft_use_item_on", "minecraft_quick_move_container_slot",
                   "minecraft_close_screen"})


def validate_args(ctx: ProfileContext) -> float:
    if ctx.client != "native" or not ctx.windowed or ctx.extra_args or ctx.platform is not None:
        raise ValueError("composter-chance requires --client native --windowed")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 900.0
    if not 0 < timeout <= 3600:
        raise ValueError("composter-chance timeout must be in (0, 3600]")
    return timeout


def run(ctx: ProfileContext) -> dict[str, Any]:
    timeout = validate_args(ctx)
    override = os.environ.get("SOLARIS_SERVER_BINARY")
    if not override or not os.environ.get("SOLARIS_CLIENT_BINARY"):
        raise RuntimeError("composter-chance requires prebuilt SOLARIS_SERVER_BINARY and SOLARIS_CLIENT_BINARY")
    binary = Path(override).expanduser().resolve()
    if not binary.is_file():
        raise RuntimeError(f"missing server binary: {binary}")
    artifact = ctx.artifact_dir
    port = runtime.reserve_port()
    address = f"127.0.0.1:{port}"
    plugins = artifact / "plugins"
    plugins.mkdir()
    config = artifact / "composter.toml"
    vanilla = os.environ.get("SOLARIS_CLIENT_DATA_DIR")
    if not vanilla or not Path(vanilla).is_absolute() or not Path(vanilla).is_dir():
        raise RuntimeError("composter-chance requires an absolute existing SOLARIS_CLIENT_DATA_DIR")
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    token = 'vanilla_data_dir = "data/vanilla"'
    if source.count(token) != 1:
        raise RuntimeError("expected one vanilla_data_dir config token")
    source = source.replace(token, f"vanilla_data_dir = {json.dumps(vanilla)}")
    config.write_text(rewrite_config(source, port=port,
        world=(artifact / "world").relative_to(runtime.REPO_ROOT).as_posix(),
        plugins=str(plugins), seed=SEED))
    prepare_operator_fixture(ctx, [USERNAME])
    with binary.open("rb") as handle:
        digest = hashlib.file_digest(handle, "sha256").hexdigest()
    ctx.client_evidence.update(server_address=address, server_binary_path=str(binary),
        server_binary_sha256=digest, world_dir=str(artifact / "world"), seed=SEED,
        vanilla_oracle="Java 26.3 mapped bytecode and compostable/low.json")
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
        with ProbeSteps(mcp, artifact / "composter-chance", timeout, "native-composter-chance") as probe:
            require_tools(mcp.list_tools(), TOOLS, "composter-chance")
            scenario(probe, artifact)
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


def scenario(probe: ProbeSteps, artifact: Path) -> None:
    probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
    state = probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "player_present")
    ox, oy, oz = (math.floor(state["player"][axis]) for axis in ("x", "y", "z"))
    comp = {"x": ox + 2, "y": oy + 1, "z": oz}
    hopper = {"x": ox + 2, "y": oy + 2, "z": oz}
    power = {"x": ox + 3, "y": oy + 2, "z": oz}
    probe.report.update(layout={"composter": comp, "hopper": hopper, "power": power},
                        samples=[], oracle_success_probability=0.3,
                        oracle_all_success_probability=0.3 ** SAMPLES,
                        oracle_version="Java 26.3 mapped bytecode and compostable/low.json")

    def setblock(pos: dict[str, int], block: str) -> None:
        probe.fixture(f"setblock {pos['x']} {pos['y']} {pos['z']} {block}")

    def wait_block(pos: dict[str, int], block: str, properties: dict[str, str]) -> dict[str, Any]:
        return probe.call("minecraft_wait_for_block_state", {**pos, "block_id": block,
            "properties": properties, "timeout_seconds": probe.remaining()})["block"]

    def open_hopper(label: str) -> dict[str, Any]:
        probe.call("minecraft_set_hotbar_slot", {"slot": 8})
        probe.call("minecraft_use_item_on", {**hopper, "face": "north", "hand": "main_hand"})
        return probe.wait(lambda s: s["container"]["container_id"] != 0
            and str(s["container"]["menu_class"]).endswith("HopperMenu"), label)

    def close_hopper(label: str) -> None:
        probe.call("minecraft_close_screen")
        probe.wait(lambda s: s["container"]["container_id"] == 0, label)

    def hopper_seeds(s: dict[str, Any]) -> int:
        return sum(int(row.get("count") or 0) for row in s["container"]["slots"]
                   if 0 <= int(row.get("slot", -1)) < 5 and row.get("item_id") == "minecraft:wheat_seeds")

    for command in _pad_commands(ox, oy, oz):
        probe.fixture(command)
    probe.fixture("gamemode creative @s")
    setblock(power, "minecraft:redstone_block")
    setblock(hopper, "minecraft:hopper[facing=down]")
    wait_block(hopper, "minecraft:hopper", {"enabled": "false"})
    probe.call("minecraft_set_hotbar_slot", {"slot": 8})

    for i in range(SAMPLES):
        setblock(comp, "minecraft:composter[level=1]")
        wait_block(comp, "minecraft:composter", {"level": "1"})
        probe.fixture("give @s minecraft:wheat_seeds 1")
        probe.wait(lambda s: item_count(s, "minecraft:wheat_seeds") == 1, f"{i}_one_seed_granted")
        opened = open_hopper(f"{i}_hopper_opened")
        probe.check(f"{i}_hopper_initially_empty", hopper_seeds(opened) == 0)
        rows = [r for r in opened["container"]["slots"] if int(r.get("slot", -1)) >= 5
                and r.get("item_id") == "minecraft:wheat_seeds" and int(r.get("count") or 0) == 1]
        probe.check(f"{i}_one_seed_in_player_inventory", len(rows) == 1)
        probe.call("minecraft_quick_move_container_slot", {"slot": int(rows[0]["slot"])})
        loaded = probe.wait(lambda s: hopper_seeds(s) == 1, f"{i}_seed_loaded")
        probe.check(f"{i}_player_debited", item_count(loaded, "minecraft:wheat_seeds") == 0)
        close_hopper(f"{i}_hopper_closed_loaded")
        setblock(power, "minecraft:air")
        wait_block(hopper, "minecraft:hopper", {"enabled": "true"})
        open_hopper(f"{i}_hopper_opened_after_unlock")
        probe.wait(lambda s: hopper_seeds(s) == 0, f"{i}_hopper_debited")
        close_hopper(f"{i}_hopper_closed_debited")
        # Ordered chat response is a server/client packet barrier after observed debit.
        marker = f"compost_sample_{i}_ordered"
        probe.call("minecraft_send_chat", {"message": f"say {marker}", "command": True})
        probe.wait(lambda s: any(marker in str(line) for line in s.get("recent_chat", [])), f"{i}_barrier")
        block = probe.call("minecraft_read_block", comp)
        if isinstance(block.get("block"), dict):
            block = block["block"]
        probe.check(f"{i}_composter_still_present", block.get("block_id") == "minecraft:composter")
        level = str((block.get("properties") or {}).get("level"))
        probe.report["samples"].append({"index": i, "input": 1, "hopper_after": 0, "level": level})
        probe.check(f"{i}_level_valid", level in ("1", "2"))
        setblock(power, "minecraft:redstone_block")
        wait_block(hopper, "minecraft:hopper", {"enabled": "false"})
    native_client.capture(probe.client, artifact, "composter-chance-final")
    failures = sum(row["level"] == "1" for row in probe.report["samples"])
    probe.report["failed_rolls"] = failures
    probe.check("at_least_one_failed_roll_in_16_trials", failures > 0)
