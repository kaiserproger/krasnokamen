"""Horizon offscreen graphical ATM gate, or explicitly labelled vanilla calibration.

Uses the canonical harness lifecycle and evidence readers; no release builds,
block placement, downloaded bytes or live player homes. See ATM.md.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack, contextmanager
import json
import math
import os
from pathlib import Path
import shutil
import subprocess

from . import runtime
from .atm_carry import digest, inventory, tree_hash, write_json
from .atm_evidence import prove_site
from .atm_loot import open_generated_container
from .backends.atm_structures import SEED, USERNAME, locate_coordinates
from .backends.chisel import rewrite_config
from .backends.gameplay import prepare_operator_fixture
from .profiles import ProfileContext


class Horizon:
    def __init__(self, args, directory, env, receipt):
        self.args, self.directory, self.env, self.receipt = args, directory, env, receipt
        self.socket = directory / "ctl.sock"

    def call(self, *arguments):
        command = [str(self.args.ctl), "--socket", str(self.socket), *map(str, arguments)]
        result = subprocess.run(command, cwd=self.directory, env=self.env, text=True,
                                capture_output=True, timeout=self.args.timeout + 10)
        row = {"command": command, "exit": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
        self.receipt["commands"].append(row)
        write_json(self.args.output / "result.json", self.receipt)
        if result.returncode:
            raise ValueError(f"Horizon {' '.join(map(str, arguments))}: {result.stdout} {result.stderr}")
        response = json.loads(result.stdout)
        if response.get("ok") is False or response.get("error"):
            raise ValueError(f"Horizon command failed: {response}")
        return response

    def command(self, command, response):
        since = self.call("events")["last"]
        self.call("command", command)
        self.call("wait-for", "chat", response, "--since", since, "--timeout", self.args.timeout)
        return self.call("messages", 50)

    def teleport(self, xyz):
        self.command("tp @s " + " ".join(map(str, xyz)), "Teleported")
        # Horizon's wait-ready only counts local generation tasks, not remote chunks.
        # Advance a bounded simulation interval, then require destination data explicitly.
        self.call("wait", 100)
        column = self.call("block", int(xyz[0]), 64, int(xyz[2]))
        if column.get("loaded") is not True:
            raise ValueError(f"destination chunk not received after 100 client ticks: {column}")
        state = self.call("state")
        player = state.get("player") or {}
        position = player.get("position")
        if not isinstance(position, list) or len(position) != 3:
            raise ValueError(f"Horizon state has no player position: {state}")
        if any(abs(a-b) > 2 for a,b in zip(position, xyz)):
            raise ValueError(f"teleport not observed: expected {xyz}, got {position}")
        return state


@contextmanager
def session(args, config, port, label, receipt):
    directory = args.output / label
    directory.mkdir()
    env = os.environ.copy()
    for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_RUNTIME_DIR"):
        home = directory / key.lower()
        home.mkdir(mode=0o700)
        env[key] = str(home)
    env.update(HORIZON_NO_AUDIO="1", HORIZON_LAUNCHER_NO_SELF_UPDATE="1", ALSOFT_DRIVERS="null",
               SDL_VIDEODRIVER="x11", PULSE_SERVER="unix:/nonexistent", PIPEWIRE_REMOTE="/nonexistent")
    env.pop("WAYLAND_DISPLAY", None)
    with ExitStack() as cleanup:
        server_log = cleanup.enter_context((directory / "server.log").open("wb"))
        command = [str(args.server), "--config", str(config), "--no-console"]
        server = runtime.start_server(command, cwd=runtime.REPO_ROOT, stdout=server_log,
                                      stderr=subprocess.STDOUT, start_new_session=True)
        cleanup.callback(runtime.stop_process, server, interrupt=True)
        receipt["commands"].append({"command": command, "log": str(directory / "server.log")})
        runtime.wait_port(port, args.timeout, server, log_path=directory / "server.log")
        xvfb, display, display_log = runtime.start_xvfb(directory)
        cleanup.callback(display_log.close)
        cleanup.callback(runtime.stop_process, xvfb)
        env["DISPLAY"] = display
        horizon = Horizon(args, directory, env, receipt)
        command = [str(args.client), "--control-socket", str(horizon.socket), "--offscreen",
                   "--resources", str(args.resources), "--game-dir", str(directory), "--render-distance", "4"]
        client_log = cleanup.enter_context((directory / "client.log").open("wb"))
        client = subprocess.Popen(command, cwd=directory, env=env, stdout=client_log,
                                  stderr=subprocess.STDOUT, start_new_session=True)
        cleanup.callback(runtime.stop_process, client)
        receipt["commands"].append({"command": command, "log": str(directory / "client.log")})
        runtime._wait_for_file(horizon.socket, args.timeout, client)
        horizon.call("hello")
        horizon.call("connect", f"127.0.0.1:{port}", USERNAME)
        horizon.call("wait-for", "loaded", "--timeout", args.timeout)
        if horizon.call("state").get("gamemode") != "spectator":
            horizon.command("gamemode spectator @s", "game mode")
        yield horizon
        horizon.command("save-all flush", "Saved")
        horizon.call("disconnect")
        horizon.call("quit")
        client.wait(timeout=30)
        receipt.setdefault("client_exits", []).append(client.returncode)
        if client.returncode:
            raise ValueError(f"Horizon exited {client.returncode}")


def run(args, receipt):
    vanilla = args.vanilla_root is not None
    generation_only = False
    loot_probes = {}
    if vanilla:
        targets = [(args.vanilla_root.resolve(), args.structure)]
        manifest_hash = None
        receipt.update(scope="vanilla pipeline calibration, NOT ATM acceptance", packs=[])
    else:
        manifest = json.loads(args.manifest.read_text())
        if manifest.get("status") != "converted-unproven" or not manifest.get("packs"):
            raise ValueError("a converted-unproven manifest with selected packs is required")
        manifest_hash = digest(args.manifest)
        generation_only = manifest.get("generation_only", False) or any(p.get("generation_only", False) for p in manifest["packs"])
        targets = []
        for pack in manifest["packs"]:
            source = Path(pack["server_pack"])
            if inventory(source) != pack["server_files"] or tree_hash(pack["server_files"]) != pack["server_sha256"]:
                raise ValueError(f"converted pack changed: {source}")
            targets.extend((source, identifier) for identifier in pack["structures"])
            if pack.get("container_loot_probe"):
                if generation_only:
                    raise ValueError("generation-only fixture cannot require a loot proof")
                loot_probes[source] = pack["container_loot_probe"]
        receipt.update(scope="adapted ATM structure data, EXPERIMENTAL", packs=[p["id"] for p in manifest["packs"]])
        shutil.copyfile(args.manifest, args.output / "conversion.json")
    if not 1 <= len(targets) <= 12:
        raise ValueError("select 1–12 structures")
    world, plugins = args.output / "world", args.output / "plugins"
    world.mkdir(); plugins.mkdir()
    if not vanilla and not args.without_packs:
        for pack in manifest["packs"]:
            shutil.copytree(pack["server_pack"], world / "datapacks" / pack["id"])
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    token = 'vanilla_data_dir = "data/vanilla"'
    if source.count(token) != 1:
        raise ValueError("playable.toml vanilla_data_dir token changed")
    source = source.replace(token, "vanilla_data_dir = " + json.dumps(str(args.cache)))
    port = runtime.reserve_port()
    config = args.output / "server.toml"
    config.write_text(rewrite_config(source, port=port, world=str(world), plugins=str(plugins), seed=SEED))
    prepare_operator_fixture(ProfileContext(args.output), [USERNAME])
    supplied = None
    if args.without_packs:
        supplied = json.loads(args.candidates.read_text())
        if (supplied.get("seed"), supplied.get("manifest_sha256"), supplied.get("server_sha256")) != (SEED, manifest_hash, digest(args.server)):
            raise ValueError("negative control candidates differ from seed/manifest/server pin")
    candidates = []
    with session(args, config, port, "discovery", receipt) as client:
        for _pack, identifier in targets:
            if supplied is None:
                messages = client.command(f"locate structure {identifier}", identifier)
                # messages' text is retained verbatim as well as the parsed coordinates.
                xz = locate_coordinates([json.dumps(messages)], identifier)
            else:
                xz = next(c["xz"] for c in supplied["candidates"] if c["id"] == identifier)
            client.teleport([xz[0], 160, xz[1]])
            candidates.append({"id": identifier, "xz": xz})
    write_json(args.output / "candidates.json", {"seed": SEED, "manifest_sha256": manifest_hash,
               "server_sha256": digest(args.server), "candidates": candidates})
    export = args.output / "export"
    command = [str(args.server), "--config", str(config), "world", "export-anvil", "--source", str(world),
               "--destination", str(export), "--receipt", str(args.output / "export.json")]
    with (args.output / "export.log").open("wb") as log:
        result = subprocess.run(command, cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT, timeout=args.timeout)
    receipt["commands"].append({"command": command, "exit": result.returncode, "log": str(args.output / "export.log")})
    if result.returncode:
        raise ValueError("native export failed; see export.log")
    sites = [prove_site(export, pack, identifier, candidate["xz"])
             for (pack, identifier), candidate in zip(targets, candidates)]
    write_json(args.output / "generated-sites.json", sites)
    if args.without_packs:
        raise ValueError("pack-absent control unexpectedly generated selected starts")
    if generation_only:
        receipt.update(status="generated-only", sites=sites, seed=SEED,
                       scope="PARTIAL generation fixture only; no footage, loot or compatibility claim")
        return
    with session(args, config, port, "photographs", receipt) as client:
        for index, (site, (pack, _identifier)) in enumerate(zip(sites, targets)):
            center = site["anchor"]
            camera = [center[0], center[1]+16, center[2]-24]
            state = client.teleport(camera)
            samples = []
            for sample in site["samples"]:
                observed = client.call("block", *sample["position"])
                if observed.get("block", "").split("[")[0] != sample["state"].split("[")[0]:
                    raise ValueError(f"client block mismatch at {sample['position']}: {observed}, expected {sample['state']}")
                samples.append(observed)
            dx,dy,dz = center[0]-camera[0],center[1]-camera[1]-1.62,center[2]-camera[2]
            client.call("look", math.degrees(math.atan2(-dx,dz)), -math.degrees(math.atan2(dy,math.hypot(dx,dz))))
            picture = args.output / "photographs" / f"structure-{index}.png"
            capture = client.call("screenshot", picture)
            if picture.read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
                raise ValueError("Horizon screenshot did not produce a PNG")
            site.update(camera=camera, client_state=state, client_blocks=samples,
                        screenshot={"path": str(picture), "sha256": digest(picture), "capture": capture})
            if pack in loot_probes:
                site["container_loot"] = open_generated_container(
                    client, site, pack, loot_probes[pack],
                    args.output / "photographs" / f"container-{index}.png")
            write_json(args.output / "photographs" / f"structure-{index}-coordinates.json", site)
    receipt.update(status="passed", sites=sites, seed=SEED,
                   limits="Selected generated starts and sampled blocks only; PNGs still require visual inspection")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, default=os.environ.get("KRASNOKAMEN_SERVER_BINARY"))
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--ctl", type=Path, required=True)
    parser.add_argument("--resources", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    selection = parser.add_mutually_exclusive_group(required=True)
    selection.add_argument("--manifest", type=Path)
    selection.add_argument("--vanilla-root", type=Path)
    parser.add_argument("--structure", default="minecraft:village_plains")
    parser.add_argument("--without-packs", action="store_true")
    parser.add_argument("--candidates", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=300)
    args = parser.parse_args()
    if args.server is None or not 0 < args.timeout <= 3600:
        parser.error("server pin and timeout in (0,3600] required")
    if args.without_packs and (args.candidates is None or args.vanilla_root is not None):
        parser.error("pack-absent control requires --manifest and --candidates")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    receipt = {"status": "failed", "commands": [], "client": "Horizon offscreen (GPU, not headless)",
               "server_sha256": digest(args.server), "client_sha256": digest(args.client), "ctl_sha256": digest(args.ctl)}
    try:
        run(args, receipt)
    except (ValueError, OSError, KeyError, StopIteration, subprocess.SubprocessError, RuntimeError) as error:
        receipt["error"] = str(error)
    finally:
        write_json(args.output / "result.json", receipt)
    print(json.dumps({"status": receipt["status"], "error": receipt.get("error"), "receipt": str(args.output / "result.json")}))
    return 0 if receipt["status"] in ("passed", "generated-only") else 1


if __name__ == "__main__":
    raise SystemExit(main())
