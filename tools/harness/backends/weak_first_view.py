"""Fresh dedicated-server startup to a real native client's presented view."""
from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import time

from tools.bench import common
from .. import runtime
from ..profiles import ProfileContext
from . import catalog_load, native_client
from .title_connect import _TitleProbeSteps


def validate_args(ctx: ProfileContext) -> argparse.Namespace:
    if ctx.client != "native" or not ctx.windowed or ctx.platform is not None:
        raise ValueError("weak-first-view requires --client native --windowed")
    parser = argparse.ArgumentParser(prog="weak-first-view")
    parser.add_argument("--server-cpus")
    parser.add_argument("--client-cpus")
    parser.add_argument("--memory", default="2G")
    try:
        args = parser.parse_args(ctx.extra_args)
    except SystemExit as error:
        raise ValueError("weak-first-view received invalid envelope arguments") from error
    common.parse_size(args.memory)
    allowed = set(os.sched_getaffinity(0))
    split = common.split_cpus(sorted(allowed), 1)
    args.server_cpus = args.server_cpus or split["server"]
    args.client_cpus = args.client_cpus or split["bots"]
    for value in (args.server_cpus, args.client_cpus):
        cpus = set(common.parse_cpu_list(value))
        if not cpus or not cpus <= allowed:
            raise ValueError(f"CPU set {value} is outside the measurement envelope")
    return args


def run(ctx: ProfileContext) -> dict:
    args = validate_args(ctx)
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 1200.0
    if not 0 < timeout < float("inf"):
        raise ValueError("timeout must be positive and finite")
    server = Path(os.environ["SOLARIS_SERVER_BINARY"]).resolve()
    binary = native_client.resolve_binary(ctx)
    artifact = ctx.artifact_dir
    world = artifact / "world"
    args.view_distance, args.seed = 8, 42
    args.vanilla_data_dir = (runtime.REPO_ROOT / "data/vanilla").resolve()
    config, _, port = catalog_load.config(args, artifact, world, 1, operators=False)
    command = [*common.scope_prefix(args.memory, "solaris-heavy.slice"), "--",
               "taskset", "-c", args.server_cpus, str(server), "--config", str(config), "--no-console"]
    server_index = len(ctx.commands)
    ctx.commands.append(command)
    ctx.exits.append(None)
    ctx.cwds.append(str(runtime.REPO_ROOT))
    with server.open("rb") as executable:
        ctx.client_evidence["server_binary_sha256"] = hashlib.file_digest(executable, "sha256").hexdigest()
    ctx.client_evidence["server_binary_path"] = str(server)
    root = artifact / "client-data" / "solaris-client"
    root.mkdir(parents=True)
    source = native_client._client_root()
    for name in ("facts", "mojang", "resources", "assets", "data"):
        path = source / name
        if path.exists():
            (root / name).symlink_to(path, target_is_directory=path.is_dir())
    for name in ("home", "cache", "config"):
        (artifact / name).mkdir()
    env = {"SOLARIS_CLIENT_ROOT": str(root), "XDG_DATA_HOME": str(root.parent), "ALSOFT_DRIVERS": "null",
           "HOME": str(artifact / "home"), "XDG_CACHE_HOME": str(artifact / "cache"),
           "XDG_CONFIG_HOME": str(artifact / "config"),
           "LIBGL_ALWAYS_SOFTWARE": "1", "VK_DRIVER_FILES": "/usr/share/vulkan/icd.d/lvp_icd.json",
           "VK_ICD_FILENAMES": "/usr/share/vulkan/icd.d/lvp_icd.json"}
    previous = {name: os.environ.get(name) for name in env}
    affinity = os.sched_getaffinity(0)
    process = client = xvfb = mcp = None
    report = {}
    try:
        with (artifact / "server.log").open("wb") as log:
            started = time.monotonic()
            process = runtime.start_server(command, cwd=runtime.REPO_ROOT, stdout=log,
                                           stderr=subprocess.STDOUT, start_new_session=True)
            runtime.wait_port(port, timeout, process, log_path=artifact / "server.log")
            report["server_listening_seconds"] = time.monotonic() - started
            os.environ.update(env)
            os.sched_setaffinity(0, common.parse_cpu_list(args.client_cpus))
            xvfb, display, _ = runtime.start_xvfb(artifact)
            client, _, mcp = native_client.start(ctx, binary, f"127.0.0.1:{port}", "WeakView",
                                                 timeout, display=display)
            os.sched_setaffinity(0, affinity)
            with _TitleProbeSteps(mcp, artifact / "first-view", timeout, "weak-first-view") as probe:
                probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
                ready = probe.wait(lambda state: state.get("in_play") is True
                                   and state["screen"].get("open") is False, "presented_world")
                probe.check("loading_overlay_cleared", ready["screen"].get("open") is False)
                report["first_view_seconds"] = time.monotonic() - started
                report["processes"] = {"server": catalog_load.scale.proc_sample(process.pid),
                                       "client": catalog_load.scale.proc_sample(client.pid)}
                report["envelope"] = {"server_cpus": args.server_cpus, "client_cpus": args.client_cpus,
                                      "server_memory_max": args.memory, "seed": 42, "view_distance": 8}
                report["view"] = ready
                native_client.capture(mcp, artifact, "first-usable-view")
                probe.call("minecraft_disconnect", {})
            report["scenario"] = probe.report
    finally:
        os.sched_setaffinity(0, affinity)
        if mcp is not None:
            mcp.close()
        runtime.stop_process(client)
        runtime.stop_process(process, timeout_seconds=180)
        runtime.stop_process(xvfb)
        if process is not None:
            ctx.exits[server_index] = process.returncode
        if world.exists():
            shutil.rmtree(world)
        for name, value in previous.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value
    if process is not None and process.returncode != 0:
        raise RuntimeError(f"first-view server exited with {process.returncode}; see {artifact / 'server.log'}")
    return report
