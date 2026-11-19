"""Live acceptance of `horizon:lod`: a real Horizon client draws far terrain from our server.

Horizon (MIT, pin bd431b38; built binaries and prepared resources named by
`--horizon-client`, `--horizon-ctl`, `--horizon-resources` or the
`KRASNOKAMEN_HORIZON_*` variables) runs offscreen under the harness's own Xvfb,
never on the owner's desktop (owner D56). It joins our server with its render
distance at 4 and LOD Distance at 64, twice on the same pregenerated stored
world, each time with its own clean game folder (Horizon keeps every chunk it
visits under `server-terrain/` and may read BlueMap, so nothing may carry over;
BlueMap detection is switched off):

* OFF control: no `[horizon_lod]` section. The client must report
  `lod.source == "server-terrain"` (it only has what the server sent), which is
  the baseline for the screenshot and the tile counters;
* ON: `[horizon_lod] enabled = true`. Pass needs ALL of: the client accepted
  the Offer (`lod.source == "server"`, effective distance beyond four times the
  view distance), `lod.resident_quads` at least `--min-quad-ratio` times the OFF
  value, no "far terrain from the server" decode warning in the client log, and
  the ON screenshot differing from the OFF one (same camera 100 blocks above the
  spawn column, frozen noon, same seed) over at least `--min-changed-fraction` of
  its pixels, i.e. terrain drawn beyond the chunks the server actually sent.
  `lod.sampled_target_tiles` is recorded but proves nothing alone: Horizon
  reports every target tile sampled in OFF too.

`--only off` records the control session alone and always ends `failed`
(`off_only_is_a_control_not_acceptance`): its receipt is evidence, never a pass.
Expected evidence of a pass is written down in docs/development/horizon-lod-acceptance.md.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import time
from contextlib import ExitStack
from pathlib import Path
from typing import Any

from .. import runtime
from ..profiles import ProfileContext
from .gameplay import prepare_operator_fixture
from .horizon_lod import Server, server_config

USERNAME = "LodLive"
ENV_NAMES = {"client": "KRASNOKAMEN_HORIZON_CLIENT", "ctl": "KRASNOKAMEN_HORIZON_CTL",
             "resources": "KRASNOKAMEN_HORIZON_RESOURCES"}
SETTLE_SAMPLES = 6  # consecutive unchanged status samples, 20 game ticks apart


def validate_args(ctx: ProfileContext) -> tuple[argparse.Namespace, float]:
    parser = argparse.ArgumentParser(prog="horizon-lod-live")
    for key, env in ENV_NAMES.items():
        parser.add_argument(f"--horizon-{key}", type=Path,
                            default=Path(os.environ[env]) if os.environ.get(env) else None)
    parser.add_argument("--only", choices=("off", "pair"), default="pair")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--view-distance", type=int, default=4)
    parser.add_argument("--lod-distance", type=int, default=64)
    parser.add_argument("--pregenerate-radius", type=int, default=400)
    parser.add_argument("--min-quad-ratio", type=float, default=5.0)
    parser.add_argument("--min-changed-fraction", type=float, default=0.30)
    try:
        args = parser.parse_args(ctx.extra_args)
    except SystemExit as error:
        raise ValueError("invalid horizon-lod-live arguments") from error
    for key, env in ENV_NAMES.items():
        path = getattr(args, f"horizon_{key}")
        if path is None or not path.exists():
            raise ValueError(f"--horizon-{key} (or {env}) must name an existing Horizon "
                             f"{'resources folder' if key == 'resources' else 'executable'}")
    if not 1 <= args.view_distance <= 8 or not 64 <= args.lod_distance <= 1024:
        raise ValueError("--view-distance 1..8 and --lod-distance 64..1024 (Horizon's slider range)")
    if ctx.platform is not None or ctx.plugins_dir is not None:
        raise ValueError("horizon-lod-live runs the bare server without Loader/plugins")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 1800.0
    if not timeout > 0:
        raise ValueError("timeout must be positive")
    return args, timeout


class Ctl:
    """`horizon-ctl` against one running Horizon; every reply is one JSON line."""

    def __init__(self, ctl: Path, socket: Path, cwd: Path, env: dict[str, str], timeout: float,
                 commands: list):
        self.ctl, self.socket, self.cwd, self.env, self.timeout = ctl, socket, cwd, env, timeout
        self.commands = commands

    def __call__(self, *arguments: Any) -> dict[str, Any]:
        command = [str(self.ctl), "--socket", str(self.socket), *map(str, arguments)]
        result = subprocess.run(command, cwd=self.cwd, env=self.env, text=True, capture_output=True,
                                timeout=self.timeout + 30)
        self.commands.append({"command": command, "exit": result.returncode,
                              "stdout": result.stdout[:4000], "stderr": result.stderr[-2000:]})
        if result.returncode:
            raise RuntimeError(f"horizon-ctl {' '.join(map(str, arguments))}: "
                               f"{result.stdout.strip()} {result.stderr.strip()[-500:]}")
        response = json.loads(result.stdout)
        if response.get("ok") is False or response.get("error"):
            raise RuntimeError(f"horizon-ctl {' '.join(map(str, arguments))}: {response}")
        return response


def lod_sample(status: dict[str, Any]) -> dict[str, Any]:
    lod = status.get("lod") or {}
    keys = ("source", "distance", "effective_distance", "terrain_distance", "target_tiles",
            "sampled_target_tiles", "active_tiles", "cached_tiles", "resident_quads", "kept_tiles")
    return {key: lod.get(key) for key in keys} | {
        "render_distance": status.get("render_distance"), "chunks": status.get("chunks"),
        "pending_chunks": status.get("pending_chunks")}


def changed_fraction(first: Path, second: Path, threshold: int = 48) -> float:
    """Share of pixels in the horizon band (28%..62% of the height, full width: below the
    toasts, above the chat) whose largest channel difference exceeds `threshold`. From the
    camera 120 blocks up the sent chunks fill only the bottom of the frame, so everything
    changed in the band is terrain past them; toasts and chat lines are outside it."""
    from PIL import Image, ImageChops

    a, b = Image.open(first).convert("RGB"), Image.open(second).convert("RGB")
    if a.size != b.size:
        raise ValueError(f"screenshots differ in size: {a.size} vs {b.size}")
    box = (0, int(a.size[1] * 0.28), a.size[0], int(a.size[1] * 0.62))
    difference = ImageChops.difference(a.crop(box), b.crop(box))
    channels = difference.split()
    peak = ImageChops.lighter(ImageChops.lighter(channels[0], channels[1]), channels[2])
    histogram = peak.histogram()
    return sum(histogram[threshold + 1:]) / (difference.size[0] * difference.size[1])


def play(args: argparse.Namespace, ctx: ProfileContext, binary: Path, artifact: Path, world: Path,
         mode: str, timeout: float) -> dict[str, Any]:
    """One Horizon session against one server run; returns its evidence."""
    directory = artifact / mode
    directory.mkdir(parents=True)
    game = directory / "game"
    game.mkdir()
    # A clean game folder per run: no server-terrain, no BlueMap, no earlier settings.
    (game / "horizon-graphics.txt").write_text(
        f"lodDistance:{args.lod_distance}\nblueMapAutoDetect:false\n")
    env = os.environ.copy()
    for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME",
                "XDG_RUNTIME_DIR"):
        home = directory / key.lower()
        home.mkdir(mode=0o700)
        env[key] = str(home)
    env.update(HORIZON_NO_AUDIO="1", HORIZON_LAUNCHER_NO_SELF_UPDATE="1", ALSOFT_DRIVERS="null",
               SDL_VIDEODRIVER="x11", PULSE_SERVER="unix:/nonexistent", PIPEWIRE_REMOTE="/nonexistent",
               MIMALLOC_VERBOSE="0", MIMALLOC_SHOW_STATS="0")
    env.pop("WAYLAND_DISPLAY", None)
    horizon_lod = (f"\n[horizon_lod]\nenabled = true\ndistance_chunks = {args.lod_distance}\n"
                   if mode == "on" else None)
    server = Server(ctx, binary, artifact, f"server-{mode}",
                    server_config(port=(port := runtime.reserve_port()), world=world, seed=args.seed,
                                  view_distance=args.view_distance, horizon_lod=horizon_lod),
                    port=port)
    commands: list[dict[str, Any]] = []
    evidence: dict[str, Any] = {"mode": mode, "game_dir": str(game), "server_log": str(server.log_path),
                                "commands": commands, "samples": []}
    import tempfile

    with ExitStack() as cleanup:
        socket_dir = Path(cleanup.enter_context(tempfile.TemporaryDirectory(prefix="hzlod-ctl-", dir="/tmp")))
        socket = socket_dir / "ctl.sock"
        server.start()
        cleanup.callback(server.stop)
        server.wait_ready(timeout)
        xvfb, display, display_log = runtime.start_xvfb(directory)
        cleanup.callback(display_log.close)
        cleanup.callback(runtime.stop_process, xvfb)
        env["DISPLAY"] = display
        client_log = cleanup.enter_context((directory / "client.log").open("wb"))
        command = [str(args.horizon_client), "--control-socket", str(socket), "--offscreen",
                   "--resources", str(args.horizon_resources), "--game-dir", str(game),
                   "--render-distance", str(args.view_distance)]
        client = subprocess.Popen(command, cwd=directory, env=env, stdout=client_log,
                                  stderr=subprocess.STDOUT, start_new_session=True)
        cleanup.callback(runtime.stop_process, client)
        commands.append({"command": command, "log": str(directory / "client.log")})
        runtime._wait_for_file(socket, timeout, client)
        ctl = Ctl(args.horizon_ctl, socket, directory, env, timeout, commands)
        ctl("hello")
        ctl("connect", f"127.0.0.1:{port}", USERNAME)
        ctl("wait-for", "loaded", "--timeout", timeout)
        # The same camera in both runs (the world remembers the player between them, so the pose is
        # absolute): spectator, noon, clear sky, no animated clouds, 120 blocks up over the spawn
        # column, looking down at the horizon.
        ctl("options-set", "renderClouds", "false")
        for line in ("gamemode spectator @s", "time set 6000", "weather clear"):
            ctl("command", line)
        x, _, z = ctl("state")["player"]["position"]
        ctl("command", f"tp @s {x:.1f} 120.0 {z:.1f}")
        ctl("wait", 20)
        evidence["camera"] = {"column": [x, z], "to": ctl("state")["player"]["position"]}
        ctl("look", 0, 20)
        # Far terrain arrives as the client asks for tiles: wait on game ticks until its counters settle.
        last, steady = None, 0
        deadline = time.monotonic() + timeout
        while steady < SETTLE_SAMPLES:
            if time.monotonic() > deadline:
                raise TimeoutError(f"Horizon's LOD counters never settled: {evidence['samples'][-3:]}")
            ctl("wait", 20)
            sample = lod_sample(ctl("status"))
            evidence["samples"].append(sample)
            key = (sample["sampled_target_tiles"], sample["target_tiles"], sample["pending_chunks"],
                   sample["resident_quads"])
            steady = steady + 1 if key == last else 0
            last = key
        picture = directory / "far-terrain.png"
        ctl("screenshot", picture)
        evidence["screenshot"] = str(picture)
        evidence["final"] = lod_sample(ctl("status"))
        evidence["state"] = ctl("state").get("player")
        ctl("disconnect")
        ctl("quit")
        client.wait(timeout=60)
        evidence["client_exit"] = client.returncode
    client_text = (directory / "client.log").read_text(errors="replace")
    evidence["decode_warnings"] = [line for line in client_text.splitlines()
                                   if "far terrain from the server" in line]
    evidence["server_exit"] = server.process.returncode
    return evidence


def judge(args: argparse.Namespace, sessions: dict[str, dict[str, Any]], checks) -> None:
    """Every acceptance check over the sessions. Fails closed: `--only off` is a control run and
    never an acceptance result, and a pair without a completed ON session cannot pass."""
    off = sessions.get("off")
    if off is not None:
        final = off["final"]
        checks("off_control_is_clean", final["source"] == "server-terrain" and not off["decode_warnings"],
               {"source": final["source"], "sampled_target_tiles": final["sampled_target_tiles"],
                "resident_quads": final["resident_quads"]})
    on = sessions.get("on")
    if args.only == "off":
        checks("off_only_is_a_control_not_acceptance", False,
               "the control evidence is saved in this receipt; only the pair run can accept horizon:lod")
    if on is not None and off is not None:
        final, base = on["final"], off["final"]
        checks("on_client_accepted_the_offer", final["source"] == "server",
               {"source": final["source"], "rule": "lod.source is 'server' only when the Offer "
                "arrived and the client uses the server as its far-terrain source"})
        checks("on_effective_distance_reaches_beyond_the_sent_chunks",
               (final["effective_distance"] or 0) > args.view_distance * 4,
               {"effective_distance": final["effective_distance"], "view_distance": args.view_distance})
        checks("on_tiles_were_sampled", (final["sampled_target_tiles"] or 0) > 0,
               {"target_tiles": final["target_tiles"], "sampled_target_tiles": final["sampled_target_tiles"],
                "off_sampled_target_tiles": base["sampled_target_tiles"],
                "note": "informational against OFF: Horizon reports every target tile sampled in OFF too "
                        "(2400 of 2400 in the recorded control), so this counter alone proves nothing"})
        checks("on_resident_far_geometry_exceeds_off",
               (final["resident_quads"] or 0) >= args.min_quad_ratio * (base["resident_quads"] or 0),
               {"on": final["resident_quads"], "off": base["resident_quads"], "required_ratio": args.min_quad_ratio})
        checks("on_client_found_no_reply_it_refused", not on["decode_warnings"], on["decode_warnings"])
        try:
            fraction = changed_fraction(Path(off["screenshot"]), Path(on["screenshot"]))
            checks("on_screenshot_shows_far_terrain_off_does_not", fraction >= args.min_changed_fraction,
                   {"changed_fraction": round(fraction, 4), "required": args.min_changed_fraction,
                    "off": off["screenshot"], "on": on["screenshot"]})
        except Exception as error:  # noqa: BLE001
            checks("on_screenshot_shows_far_terrain_off_does_not", False, f"{type(error).__name__}: {error}")
        for mode, session in sessions.items():
            checks(f"{mode}_exits_clean", session["client_exit"] == 0 and session["server_exit"] == 0,
                   {"client": session["client_exit"], "server": session["server_exit"]})
    elif args.only == "pair":
        checks("on_session", False, "the ON session did not complete")


def run(ctx: ProfileContext) -> dict[str, Any]:
    args, timeout = validate_args(ctx)
    from ..profiles import _server_binary
    from .horizon_lod import Checks

    artifact = ctx.artifact_dir
    artifact.mkdir(parents=True, exist_ok=True)
    binary = _server_binary(ctx, "server-build.log")
    world = artifact / "world"
    world.mkdir(parents=True, exist_ok=True)
    checks = Checks()
    report: dict[str, Any] = {
        "scope": "real Horizon client, offscreen under the harness Xvfb: far terrain beyond the sent chunks over horizon:lod",
        "horizon_client": str(args.horizon_client), "seed": args.seed, "view_distance": args.view_distance,
        "lod_distance": args.lod_distance, "pregenerate_radius": args.pregenerate_radius,
        "checks": checks.rows,
    }
    ctx.client_evidence["horizon_lod_live"] = report
    prepare_operator_fixture(ctx, [USERNAME])

    with runtime.phase("server_start"):
        pregen = Server(ctx, binary, artifact, "pregenerate",
                        server_config(port=0, world=world, seed=args.seed, view_distance=args.view_distance,
                                      horizon_lod=None), port=0)
        argv = [*pregen.argv, "world", "pregenerate", "--radius", str(args.pregenerate_radius)]
        ctx.commands.append(argv)
        ctx.cwds.append(str(runtime.REPO_ROOT))
        ctx.exits.append(None)
        ctx.logs.append(str(pregen.log_path))
        with pregen.log_path.open("wb") as log:
            result = subprocess.run(argv, cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT,
                                    timeout=timeout)
        ctx.exits[-1] = result.returncode
        if result.returncode or "pregenerate complete" not in pregen.text():
            raise RuntimeError(f"pregeneration failed; see {pregen.log_path}")

    sessions: dict[str, dict[str, Any]] = {}
    for mode in (("off",) if args.only == "off" else ("off", "on")):
        try:
            sessions[mode] = play(args, ctx, binary, artifact, world, mode, timeout)
        except Exception as error:  # noqa: BLE001 - the failure is the evidence
            checks(f"{mode}_session", False, f"{type(error).__name__}: {error}")
            break
    report["sessions"] = sessions

    judge(args, sessions, checks)
    failed = checks.failed()
    report["failed_checks"] = failed
    if failed:
        raise RuntimeError("horizon-lod-live failed: " + ", ".join(failed))
    return report
