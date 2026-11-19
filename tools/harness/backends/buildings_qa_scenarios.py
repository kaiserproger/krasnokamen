"""Buildings-qa profile entry: the select of blueprints, the independent-parts split and
the run that founds, orders and inspects one settlement.

The client/report seams live on ``buildings_qa`` (the tests patch them there), so this
module reaches them as ``_qa.<name>`` and never binds them.
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
from ..profiles import ProfileContext, SplitPart, run_split, split_part_label
from . import native_client
from .gameplay import peak_rss_kib, prepare_operator_fixture
from .probe_steps import ProbeSteps
from . import buildings_qa as _qa
from .buildings_qa_consts import BLUEPRINTS, PERIMETER, SEED, USERNAME
from .buildings_qa_flows import _scenario_sequential
from .buildings_qa_probe import _matrix_19


def selected_targets(value: str | None) -> tuple[str, ...]:
    """`BUILDINGS_QA_SELECT`: ALL, one member, or a comma list of members.

    A split part names its own group here, so a part is exactly the old
    per-blueprint work and never the whole gate.
    """
    select = "ALL" if value is None else value.strip()
    if select == "ALL":
        return BLUEPRINTS
    chosen = tuple(part.strip() for part in select.split(",") if part.strip())
    unknown = [part for part in chosen if part not in (*BLUEPRINTS, PERIMETER)]
    if not chosen or unknown:
        raise ValueError(
            f"BUILDINGS_QA_SELECT must be ALL or a comma list of {BLUEPRINTS}, {PERIMETER}; "
            f"got {select!r}")
    if len(set(chosen)) != len(chosen):
        raise ValueError(f"BUILDINGS_QA_SELECT repeats a blueprint: {select!r}")
    if PERIMETER in chosen and len(chosen) > 1:
        raise ValueError("BUILDINGS_QA_SELECT cannot mix the perimeter with blueprints")
    return chosen


def split_groups(targets: tuple[str, ...], parts: int) -> list[tuple[str, ...]]:
    """Contiguous groups whose union is exactly `targets`, one child per group."""
    count = max(1, min(parts, len(targets)))
    size = math.ceil(len(targets) / count)
    return [targets[index:index + size] for index in range(0, len(targets), size)]


def merge_split_parts(blocks: list[dict[str, Any]],
                      targets: tuple[str, ...]) -> dict[str, Any]:
    """One gate verdict from independent parts; nothing may be missing or owned twice."""
    blueprints: dict[str, Any] = {}
    evidence: dict[str, Any] = {}
    repro: dict[str, Any] = {}
    behaviour: dict[str, Any] = {}
    owners: dict[str, str] = {}
    for block in blocks:
        detail = (block.get("details") or {}).get("buildings_acceptance") or {}
        for bp, sub in (detail.get("blueprints") or {}).items():
            if bp in owners:
                raise RuntimeError(
                    f"blueprint {bp} reported by parts {owners[bp]} and {block.get('label')}")
            owners[bp] = str(block.get("label"))
            blueprints[bp] = sub
        evidence.update(detail.get("evidence") or {})
        repro.update(detail.get("repro") or {})
        behaviour.update(detail.get("behaviour") or {})
    missing = [bp for bp in targets if bp not in blueprints]
    if missing:
        raise RuntimeError("split parts did not cover: " + ", ".join(missing))
    detail = {"mode": "split-independent-parts", "blueprints": blueprints,
              "behaviour": behaviour, "evidence": evidence, "repro": repro,
              "parts": [{"label": block.get("label"), "status": block.get("status"),
                         "artifact": block.get("artifact"), "log": block.get("log")}
                        for block in blocks]}
    detail["matrix_19"] = _matrix_19(detail)
    return {"scenario": {"buildings_acceptance": detail}}


def validate_args(ctx: ProfileContext) -> tuple[float, tuple]:
    if ctx.client != "native":
        raise ValueError("buildings-qa requires --client native")
    if ctx.extra_args or ctx.platform is not None:
        raise ValueError("buildings-qa accepts only --client, --windowed, --parts and --timeout-seconds")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 600.0
    if not 0 < timeout <= 3600:
        raise RuntimeError("buildings-qa --timeout-seconds must be in (0, 3600]")
    return timeout, selected_targets(os.environ.get("BUILDINGS_QA_SELECT"))


def run(ctx: ProfileContext) -> dict[str, Any]:
    timeout, targets = validate_args(ctx)
    if ctx.parts > 1 and split_part_label() is None:
        # Each part is its own server, client and scene slot on the same pinned
        # binary; the union must still cover exactly the selected blueprints.
        groups = split_groups(targets, ctx.parts)
        blocks = _qa.run_split(ctx, "buildings-qa", [
            SplitPart(f"part{index}", {"BUILDINGS_QA_SELECT": ",".join(group)})
            for index, group in enumerate(groups, start=1)])
        return merge_split_parts(blocks, targets)
    _qa._load_blueprints()
    artifact = ctx.artifact_dir
    override = os.environ.get("KRASNOKAMEN_SERVER_BINARY")
    if not override:
        raise RuntimeError("buildings-qa requires KRASNOKAMEN_SERVER_BINARY (a krasnokamen built from this tree)")
    binary = Path(override).expanduser().resolve()
    if not binary.is_file():
        raise RuntimeError(f"missing executable server binary: {binary}")
    server_port = runtime.reserve_port()
    address = f"127.0.0.1:{server_port}"
    config = artifact / "buildings-qa.toml"
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    replacements = (
        ("port = 25565", f"port = {server_port}"),
        ('world_dir = ".analysis/test-world-v12"',
         f'world_dir = "{(artifact / "world").relative_to(runtime.REPO_ROOT).as_posix()}"'),
        ('directory = "plugins"', f"directory = {json.dumps(str(artifact / 'plugins'))}"),
        ("seed = 0", f"seed = {SEED}"),
    )
    for old_tok, new_tok in replacements:
        if source.count(old_tok) != 1:
            raise RuntimeError(f"expected one buildings-qa config token: {old_tok}")
        source = source.replace(old_tok, new_tok, 1)
    config.write_text(source)
    keep = os.environ.get("BUILDINGS_QA_KEEP_WORLD")
    if keep:
        import shutil as _shutil
        _shutil.rmtree(artifact / "world", ignore_errors=True)
        _shutil.copytree(keep, artifact / "world")
        evidence_keep = {"resumed_world_from": keep}
    else:
        evidence_keep = {}
    (artifact / "plugins").mkdir(parents=True, exist_ok=True)
    prepare_operator_fixture(ctx, [USERNAME])
    evidence = ctx.client_evidence
    with binary.open("rb") as server_executable:
        server_hash = hashlib.file_digest(server_executable, "sha256").hexdigest()
    evidence.update(server_address=address, server_binary_path=str(binary),
                    server_binary_sha256=server_hash, world_dir=str(artifact / "world"),
                    seed=SEED, select=list(targets), **evidence_keep)
    server = client_process = xvfb = mcp = None
    server_log = None
    server_log_path = artifact / "server.log"
    try:
        server_log = server_log_path.open("wb")
        env = os.environ.copy()
        env["KRASNOKAMEN_HARNESS_LOG_STDOUT"] = "1"
        server = runtime.start_server([str(binary), "--config", str(config), "--no-console"],
                                  cwd=runtime.REPO_ROOT, stdout=server_log,
                                  stderr=subprocess.STDOUT, start_new_session=True, env=env)
        ctx.logs.append(str(server_log_path))
        runtime.wait_port(server_port, timeout, server, log_path=server_log_path)
        display = None
        if ctx.windowed:
            xvfb, display, _ = runtime.start_xvfb(artifact)
            ctx.logs.append(str(artifact / "xvfb.log"))
        client_process, _client_log, mcp = native_client.start(
            ctx, native_client.resolve_binary(ctx), address, USERNAME, timeout, display=display)
        mcp.request_timeout_seconds = 130.0
        with ProbeSteps(mcp, artifact / "buildings-qa", timeout, "buildings-qa") as probe:
            probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
            if ctx.windowed:
                native_client.capture(mcp, artifact, "buildings-qa-spawn")
            detail: dict[str, Any] = {"behaviour": {}, "evidence": {}, "repro": {}}
            probe.report["buildings_acceptance"] = detail
            detail["mode"] = "sequential-founded-resumed"
            probe.server_proc = server  # type: ignore[attr-defined]
            if targets == (PERIMETER,):
                # The settlement's own wall, gates and well (settlements/layout.rs), not a blueprint.
                from . import perimeter_qa
                perimeter_qa.scenario(probe, detail, ctx, artifact)
            else:
                _scenario_sequential(probe, detail, ctx, artifact, targets,
                                     address, config, server_log_path)
            server = getattr(probe, "server_proc", server)
            detail["matrix_19"] = _matrix_19(detail)
            if ctx.windowed:
                native_client.capture(mcp, artifact, "buildings-qa-final")
        if ctx.windowed:
            evidence["capture"] = native_client.capture(mcp, artifact)
        if client_process is not None:
            rss = peak_rss_kib(client_process.pid)
            if rss is not None:
                evidence["peak_client_rss_kib"] = rss
        return {"scenario": probe.report}
    finally:
        if mcp is not None:
            try:
                mcp.close()
            except Exception:
                pass
        runtime.stop_process(client_process)
        runtime.stop_process(server, interrupt=True)
        runtime.stop_process(xvfb)
        if server_log is not None:
            server_log.close()
