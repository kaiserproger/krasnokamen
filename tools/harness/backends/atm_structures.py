"""Fresh-world ATM structure proof: generated starts, exported blocks, real window.

No fixture places blocks or structures. /locate only chooses chunks to visit;
the authoritative assertion reads persisted starts from the stopped world's
export. The same native world is then reopened for client block reads/photos.
"""
from __future__ import annotations

import json
from contextlib import ExitStack
import math
import os
from pathlib import Path
import re
import shutil
import subprocess

from .. import runtime
from ..atm_carry import digest, inventory, tree_hash, write_json
from ..atm_evidence import prove_site
from . import native_client
from .chisel import rewrite_config
from .gameplay import prepare_operator_fixture, require_tools
from .probe_steps import ProbeSteps

USERNAME = "AtmCarry"
SEED = 20261115
TOOLS = frozenset({"minecraft_observe", "minecraft_wait_for_play", "minecraft_wait_for_state_change",
                   "minecraft_send_chat", "minecraft_wait_ticks", "minecraft_look",
                   "minecraft_read_block", "minecraft_wait_for_block_state", "client_capture"})


def locate_coordinates(lines: list[str], identifier: str) -> list[int]:
    for line in lines:
        if identifier not in line:
            continue
        match = re.search(r"\[(-?\d+),\s*(?:-?\d+|~),\s*(-?\d+)\]", line)
        if match:
            return [int(match[1]), int(match[2])]
    raise ValueError(f"{identifier}: locate did not return coordinates: {lines}")


def chat(probe: ProbeSteps, command: str, label: str) -> list[str]:
    fence = f"ATMCARRY_{label}"
    probe.fixture(f"tellraw @s {json.dumps(fence)}")
    probe.wait(lambda s: any(fence in str(line) for line in s.get("recent_chat", [])), label + "_fence")
    probe.fixture(command)
    def newer(state):
        lines = [str(line) for line in state.get("recent_chat", [])]
        boundary = next((i for i, line in enumerate(lines) if fence in line), None)
        return lines[:boundary] if boundary is not None else []
    state = probe.wait(lambda s: bool(newer(s)), label + "_reply")
    return newer(state)


def spectator(probe):
    # An already-spectator reconnect emits no gamemode-change reply.
    probe.fixture("gamemode spectator @s")
    lines = chat(probe, 'execute if entity @s[gamemode=spectator] run tellraw @s "ATMCARRY_SPECTATOR_READY"', "spectator")
    probe.check("spectator", any("ATMCARRY_SPECTATOR_READY" in line for line in lines))


def teleport(probe: ProbeSteps, xyz: list[float], label: str) -> None:
    probe.fixture("tp @s " + " ".join(str(v) for v in xyz))
    probe.wait(lambda s: all(abs(float(s.get("player", {}).get(axis, 1e10)) - value) < 2
                             for axis, value in zip(("x", "y", "z"), xyz)), label)


def session(ctx, binary: Path, config: Path, port: int, label: str, timeout: float, action):
    server = client = xvfb = mcp = server_log = xvfb_log = client_log = None
    artifact = ctx.artifact_dir / label
    artifact.mkdir()
    try:
        log_path = artifact / "server.log"
        server_log = log_path.open("wb")
        command = [str(binary), "--config", str(config), "--no-console"]
        index = len(ctx.commands)
        ctx.commands.append(command)
        ctx.cwds.append(str(runtime.REPO_ROOT))
        ctx.exits.append(None)
        ctx.logs.append(str(log_path))
        server = runtime.start_server(command, cwd=runtime.REPO_ROOT, stdout=server_log,
                                      stderr=subprocess.STDOUT, start_new_session=True)
        runtime.wait_port(port, timeout, server, log_path=log_path)
        xvfb, display, xvfb_log = runtime.start_xvfb(artifact)
        ctx.logs.append(str(artifact / "xvfb.log"))
        # Native start owns command/client pin evidence and writes into this phase's directory.
        previous = ctx.artifact_dir
        ctx.artifact_dir = artifact
        try:
            client, client_log, mcp = native_client.start(ctx, native_client.resolve_binary(ctx),
                f"127.0.0.1:{port}", USERNAME, timeout, display=display)
        finally:
            ctx.artifact_dir = previous
        mcp.p4_window = ctx.client_evidence.get("window_focus")
        with ProbeSteps(mcp, artifact, timeout, "atm-" + label) as probe:
            require_tools(mcp.list_tools(), TOOLS, "atm-structures")
            probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
            spectator(probe)
            result = action(probe, artifact)
            chat(probe, "save-all flush", "save")
            probe.wait(lambda s: any("Saved" in str(line) for line in s.get("recent_chat", [])), "save_acknowledged")
        return result
    finally:
        # Every child is reaped even if another child's shutdown or MCP close fails.
        with ExitStack() as cleanup:
            for handle in (client_log, xvfb_log, server_log):
                if handle is not None:
                    cleanup.callback(handle.close)
            cleanup.callback(runtime.stop_process, xvfb)
            cleanup.callback(runtime.stop_process, server, interrupt=True)
            cleanup.callback(runtime.stop_process, client)
            if mcp is not None:
                cleanup.callback(mcp.close)
        if server is not None:
            ctx.exits[index] = server.returncode
        if client is not None:
            ctx.exits[ctx.client_evidence["command_index"]] = client.returncode


def run(ctx) -> dict:
    if ctx.client != "native" or not ctx.windowed or ctx.extra_args or ctx.platform is not None:
        raise ValueError("atm-structures requires --client native --windowed and no extra arguments")
    timeout = ctx.timeout_seconds or 600.0
    if not 0 < timeout <= 3600:
        raise ValueError("atm-structures timeout must be in (0, 3600]")
    manifest_path = Path(os.environ["KRASNOKAMEN_ATM_MANIFEST"]).resolve()
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("status") != "converted-unproven" or not manifest.get("packs"):
        raise ValueError(f"{manifest_path}: successful local conversion receipt required")
    if manifest.get("generation_only") or any(p.get("generation_only") for p in manifest["packs"]):
        raise ValueError("generation-only fixtures cannot enter the native photograph gate; use atm_horizon")
    binary = Path(os.environ["KRASNOKAMEN_SERVER_BINARY"]).resolve()
    if not binary.is_file() or "slot-" in str(binary):
        raise ValueError("atm-structures needs Main's immutable release server pin")
    negative = os.environ.get("KRASNOKAMEN_ATM_WITHOUT_PACKS") == "1"
    artifact = ctx.artifact_dir
    world, plugins = artifact / "world", artifact / "plugins"
    world.mkdir()
    plugins.mkdir()
    manifest_hash = digest(manifest_path)
    targets = []
    for pack in manifest["packs"]:
        source = Path(pack["server_pack"])
        if inventory(source) != pack["server_files"] or tree_hash(pack["server_files"]) != pack["server_sha256"]:
            raise ValueError(f"{source}: converted pack changed after receipt")
        if not negative:
            shutil.copytree(source, world / "datapacks" / pack["id"])
        targets.extend((pack, name) for name in pack["structures"])
    if not targets or len(targets) > 12:
        raise ValueError("select 1–12 structures in the conversion inputs")
    port = runtime.reserve_port()
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    token = 'vanilla_data_dir = "data/vanilla"'
    if source.count(token) != 1:
        raise ValueError("playable.toml must have exactly one vanilla_data_dir")
    source = source.replace(token, "vanilla_data_dir = " + json.dumps(os.environ["KRASNOKAMEN_SERVER_VANILLA_DATA_DIR"]))
    config = artifact / "atm.toml"
    config.write_text(rewrite_config(source, port=port, world=str(world), plugins=str(plugins), seed=SEED))
    prepare_operator_fixture(ctx, [USERNAME])
    ctx.client_evidence.update(seed=SEED, server_binary_path=str(binary), server_binary_sha256=digest(binary),
                               manifest_sha256=manifest_hash, without_packs=negative, world_dir=str(world))
    shutil.copyfile(manifest_path, artifact / "conversion.json")
    supplied = None
    if negative:
        control = Path(os.environ["KRASNOKAMEN_ATM_CANDIDATES"])
        supplied = json.loads(control.read_text())
        if supplied.get("manifest_sha256") != manifest_hash or supplied.get("seed") != SEED:
            raise ValueError("negative control candidates must match this conversion and seed")
        if supplied.get("server_sha256") != digest(binary):
            raise ValueError("pack-absent control must use the identical server pin")

    def discover(probe, _artifact):
        candidates = []
        for index, (_pack, identifier) in enumerate(targets):
            if supplied is None:
                lines = chat(probe, f"locate structure {identifier}", f"locate_{index}")
                point = locate_coordinates(lines, identifier)
            else:
                entry = next((e for e in supplied["candidates"] if e["id"] == identifier), None)
                if entry is None:
                    raise ValueError(f"negative control missing candidate {identifier}")
                point = entry["xz"]
            teleport(probe, [point[0], 160, point[1]], f"visit_{index}")
            # Simulation ticks, not a wall-clock delay; persisted start/block proof follows export.
            probe.call("minecraft_wait_ticks", {"ticks": 100})
            candidates.append({"id": identifier, "xz": point})
        return candidates

    candidates = session(ctx, binary, config, port, "discovery", timeout, discover)
    write_json(artifact / "candidates.json", {"manifest_sha256": manifest_hash, "seed": SEED,
               "server_sha256": digest(binary), "candidates": candidates})
    export = artifact / "export"
    command = [str(binary), "--config", str(config), "world", "export-anvil", "--source", str(world),
               "--destination", str(export), "--receipt", str(artifact / "export.json")]
    ctx.commands.append(command)
    ctx.cwds.append(str(runtime.REPO_ROOT))
    ctx.exits.append(None)
    log_path = artifact / "export.log"
    ctx.logs.append(str(log_path))
    with log_path.open("wb") as log:
        result = subprocess.run(command, cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
    ctx.exits[-1] = result.returncode
    if result.returncode:
        raise ValueError(f"offline native export failed: {log_path}")
    sites = [prove_site(export, Path(pack["server_pack"]), identifier, candidate["xz"])
             for (pack, identifier), candidate in zip(targets, candidates)]
    write_json(artifact / "generated-sites.json", sites)
    if negative:
        raise ValueError("negative control unexpectedly generated selected pack starts; evidence is contaminated")

    def photograph(probe, phase_dir):
        for index, site in enumerate(sites):
            # The authoritative generated start anchors both sampling and the photograph.
            center = site["anchor"]
            camera = [center[0], center[1]+16, center[2]-24]
            teleport(probe, camera, f"photo_position_{index}")
            observed = []
            for sample in site["samples"]:
                sx, sy, sz = sample["position"]
                expected = sample["state"].split("[")[0]
                probe.call("minecraft_wait_for_block_state", {"x": sx, "y": sy, "z": sz,
                           "block_id": expected, "timeout_seconds": probe.remaining()})
                block = probe.call("minecraft_read_block", {"x": sx, "y": sy, "z": sz})
                probe.check(f"{index}_block_{sx}_{sy}_{sz}", block.get("block_id") == expected)
                observed.append(block)
            dx, dy, dz = center[0]-camera[0], center[1]-camera[1]-1.62, center[2]-camera[2]
            probe.call("minecraft_look", {"yaw_deg": math.degrees(math.atan2(-dx, dz)),
                                          "pitch_deg": -math.degrees(math.atan2(dy, math.hypot(dx, dz)))})
            probe.call("minecraft_wait_ticks", {"ticks": 2})
            site.update(client_blocks=observed, camera=camera, client_observation=probe.observe(),
                        screenshot=native_client.capture(probe.client, phase_dir, f"structure-{index}"))
            write_json(phase_dir / f"structure-{index}-coordinates.json", site)
        return sites

    photos = session(ctx, binary, config, port, "photographs", timeout, photograph)
    return {"structures": photos, "packs": [p["id"] for p in manifest["packs"]],
            "caption": f"adapted data from {len(manifest['packs'])} ATM10 mods, EXPERIMENTAL",
            "limits": "Selected structure starts and sampled blocks only; no NeoForge/modpack compatibility or full-mod fidelity claim. Screenshots require visual inspection."}
