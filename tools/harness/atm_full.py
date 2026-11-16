"""Full selected MVS inventory and real-client generated-start campaign."""
from __future__ import annotations

import gzip
import json
import os
from pathlib import Path
import shutil
import subprocess
from types import SimpleNamespace

from . import runtime
from .atm_carry import digest, inventory, tree_hash, write_json
from .atm_evidence import generated_starts
from .atm_horizon import session
from .atm_loot import stand_positions, visible_face
from .backends.atm_structures import SEED, USERNAME, locate_coordinates
from .backends.chisel import rewrite_config
from .backends.gameplay import prepare_operator_fixture
from .profiles import ProfileContext
from .quirk_parity.anvil import _chunks_of, _regions, parse_nbt, read_world

OVERWORLD = "moogs_structures:moogs_structures_generic_jigsaw_structure"
NETHER = "moogs_structures:moogs_structures_generic_nether_jigsaw_structure"
DIMENSIONS = {OVERWORLD: "minecraft:overworld", NETHER: "minecraft:the_nether"}
# Existing crystal fixture had 437 eligible cells; eight is conservative,
# not a calibration of the other 129 definitions.
MIN_DIFFERENTIAL_BLOCKS = 8


def resource(pack: Path, identifier: str, kind: str, suffix: str) -> Path:
    namespace, name = identifier.split(":", 1)
    path = (pack / "data" / namespace / kind / (name + suffix)).resolve()
    if not path.is_relative_to(pack.resolve()):
        raise ValueError(f"unsafe resource identifier {identifier}")
    return path


def selected_template(element: dict) -> str:
    """Resolve the pack's inclusive version ranges for the server's 26.3 release."""
    fallback = element["location"]
    def version(raw):
        parts = raw.split(".")
        if not 1 <= len(parts) <= 3 or any(not part.isdecimal() for part in parts):
            raise ValueError(f"invalid template version: {raw}")
        return tuple(map(int, parts)) + (0,) * (3 - len(parts))
    chosen = []
    for key, value in element.get("locations", {}).items():
        ends = key.split("-")
        if len(ends) not in (1, 2) or not isinstance(value, str):
            raise ValueError(f"invalid template version range: {key}")
        low, high = version(ends[0]), version(ends[-1])
        if low > high:
            raise ValueError(f"inverted template version range: {key}")
        if low <= (26, 3, 0) <= high:
            chosen.append(value)
    if len(chosen) > 1:
        raise ValueError("ambiguous template version ranges")
    return chosen[0] if chosen else fallback


def selected_definitions(pack: Path) -> list[dict]:
    root = pack / "data/mvs/worldgen/structure"
    rows = []
    for definition_path in sorted(root.rglob("*.json")):
        identifier = "mvs:" + definition_path.relative_to(root).with_suffix("").as_posix()
        definition = json.loads(definition_path.read_text())
        typ = definition.get("type")
        if typ not in DIMENSIONS:
            raise ValueError(f"{identifier}: unsupported structure type {typ}")
        start_pool = definition["start_pool"]
        pool_path = resource(pack, start_pool, "worldgen/template_pool", ".json")
        pool = json.loads(pool_path.read_text())
        templates = set()

        def collect(value):
            if isinstance(value, dict):
                if isinstance(value.get("location"), str):
                    templates.add(selected_template(value))
                for child in value.values():
                    collect(child)
            elif isinstance(value, list):
                for child in value:
                    collect(child)

        collect(pool["elements"])
        if not templates:
            raise ValueError(f"{identifier}: start pool has no template")
        for template in templates:
            resource(pack, template, "structure", ".nbt").stat()
        rows.append({"id": identifier, "dimension": DIMENSIONS[typ], "start_pool": start_pool,
                     "templates": sorted(templates), "definition_sha256": digest(definition_path),
                     "start_pool_sha256": digest(pool_path), "status": "unrun",
                     "reason": "not yet attempted"})
    counts = {dim: sum(row["dimension"] == dim for row in rows) for dim in DIMENSIONS.values()}
    if counts != {"minecraft:overworld": 127, "minecraft:the_nether": 3}:
        raise ValueError(f"full MVS selection must contain 127 Overworld and 3 Nether definitions: {counts}")
    return rows


def restart_selection(rows: list[dict]) -> set[str]:
    overworld = sorted(row["id"] for row in rows if row["dimension"] == "minecraft:overworld")
    nether = {row["id"] for row in rows if row["dimension"] == "minecraft:the_nether"}
    return set(overworld[::10]) | nether


def locate_failure(messages: dict, identifier: str) -> str | None:
    """Only a server's explicit, ID-matched failure is a finding."""
    for item in messages.get("styled", []):
        key = item.get("translate")
        if key not in ("commands.locate.structure.invalid", "commands.locate.structure.not_found"):
            continue
        if not any(argument.get("text") == identifier
                   for argument in item.get("with", []) if isinstance(argument, dict)):
            continue
        return ("identifier not admitted by the server" if key.endswith(".invalid")
                else "server located no structure")
    return None


def dimension_teleport(client, dimension: str, xyz: list[float]):
    if client.call("state").get("gamemode") != "spectator":
        client.command("gamemode spectator @s", "game mode")
    client.command(f"execute in {dimension} run tp @s " + " ".join(map(str, xyz)), "Teleported")
    client.call("wait", 100)
    state = client.call("state")
    if state.get("dimension") != dimension:
        raise ValueError(f"dimension teleport failed: expected {dimension}, got {state.get('dimension')}")
    position = (state.get("player") or {}).get("position")
    if not isinstance(position, list) or len(position) != 3 or any(abs(a-b) > 2 for a, b in zip(position, xyz)):
        raise ValueError(f"teleport not observed: expected {xyz}, got {position}")
    if client.call("block", int(xyz[0]), int(xyz[1]), int(xyz[2])).get("loaded") is not True:
        raise ValueError("client did not receive destination chunk")
    return state


def template_palette(pack: Path, row: dict) -> set[str]:
    blocks = set()
    for identifier in row["templates"]:
        nbt = parse_nbt(gzip.decompress(resource(pack, identifier, "structure", ".nbt").read_bytes()))
        for palette in nbt.get("palettes", [nbt.get("palette", [])]):
            for block in palette:
                name = block["Name"]
                if name not in {"minecraft:air", "minecraft:cave_air", "minecraft:void_air", "minecraft:structure_void"}:
                    blocks.add(name)
    if not blocks:
        raise ValueError(f"{row['id']}: start templates contain no block palette")
    return blocks


def find_site(export: Path, row: dict) -> dict:
    candidate = row["xz"]
    starts = [site for site in generated_starts(export, row["id"])
              if abs(site["anchor"][0] - candidate[0]) <= 32 and abs(site["anchor"][2] - candidate[1]) <= 32]
    expected_suffix = ("dimensions/minecraft/the_nether" if row["dimension"] == "minecraft:the_nether"
                       else "dimensions/minecraft/overworld")
    starts = [site for site in starts if site["dimension_dir"].endswith(expected_suffix)]
    if not starts:
        raise ValueError(f"{row['id']}: no persisted generated start near locate coordinates in {row['dimension']}")
    return min(starts, key=lambda s: abs(s["anchor"][0]-candidate[0]) + abs(s["anchor"][2]-candidate[1]))


def expanded_bounds(bounds: list[int], margin: int) -> list[int]:
    return [value - margin for value in bounds[:3]] + [value + margin for value in bounds[3:]]


def require_chunks(dimension_dir: Path, bounds: list[int]) -> None:
    wanted = _chunks_of([bounds])
    present = {chunk for chunk, _ in _regions(dimension_dir / "region", wanted)}
    if missing := wanted - present:
        raise ValueError(f"missing exported chunks in {dimension_dir}: {sorted(missing)}")


def differential_candidates(blocks: dict, baseline: dict, natural: set[str], palette: set[str]) -> list[dict]:
    """Only count different palette types absent from the local pack-free vocabulary."""
    matches = []
    for xyz, state in sorted(blocks.items()):
        kind = state.split("[", 1)[0]
        prior = baseline.get(xyz, "minecraft:air").split("[", 1)[0]
        if kind in palette and kind not in natural and kind != prior:
            matches.append({"position": [int(v) for v in xyz.split(",")], "state": state})
    return matches


def site_proof(export: Path, baseline_export: Path, pack: Path, row: dict, phase: str) -> dict:
    site = find_site(export, row)
    dimension_dir = Path(site["dimension_dir"])
    baseline_dir = baseline_export / dimension_dir.relative_to(export)
    bounds = site["bounds"]
    baseline_bounds = expanded_bounds(bounds, 16)
    require_chunks(dimension_dir, expanded_bounds(bounds, 6))
    require_chunks(baseline_dir, baseline_bounds)
    palette = template_palette(pack, row)
    audit = row.setdefault("palette_audit", {}).setdefault(phase, {"candidate": {}, "baseline": {}})
    blocks = read_world(dimension_dir, [bounds], palette_audit=audit["candidate"])[0]["blocks"]
    baseline = read_world(baseline_dir, [baseline_bounds], palette_audit=audit["baseline"])[0]["blocks"]
    natural = {state.split("[", 1)[0] for state in baseline.values()} | {"minecraft:air"}
    matches = differential_candidates(blocks, baseline, natural, palette)
    self_control = differential_candidates(baseline, baseline, natural, palette)
    row.setdefault("differential", {})[phase] = {
        "eligible_blocks": len(matches), "minimum": MIN_DIFFERENTIAL_BLOCKS,
        "natural_types": len(natural), "baseline_self_control_blocks": len(self_control),
        "baseline_dimension": str(baseline_dir),
    }
    if self_control:
        raise ValueError(f"{row['id']}: pack-absent self-control unexpectedly matched")
    if len(matches) < MIN_DIFFERENTIAL_BLOCKS:
        raise ValueError(f"{row['id']}: only {len(matches)} non-natural differential template blocks "
                         f"(minimum {MIN_DIFFERENTIAL_BLOCKS})")
    matches.sort(key=lambda block: sum((a-b)**2 for a, b in zip(block["position"], site["anchor"])))
    audit["candidate_approach"] = {}
    nearby = read_world(dimension_dir, [expanded_bounds(bounds, 6)],
                        palette_audit=audit["candidate_approach"])[0]["blocks"]
    def block(pos):
        return nearby.get(",".join(map(str, pos)), "minecraft:air").split("[")[0]
    samples = []
    for match in matches:
        stands = stand_positions(match["position"], block)
        if stands:
            samples.append({**match, "stands": stands})
        if len(samples) == 16:
            break
    if not samples:
        raise ValueError(f"{row['id']}: no standable survival approach to a differential block")
    site["samples"] = samples
    return site


def export_world(args, config: Path, world: Path, name: str, receipt: dict) -> Path:
    destination = args.output / name
    command = [str(args.server), "--config", str(config), "world", "export-anvil", "--source", str(world),
               "--destination", str(destination), "--receipt", str(args.output / (name + ".json"))]
    log_path = args.output / (name + ".log")
    with log_path.open("wb") as log:
        result = subprocess.run(command, cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT, timeout=args.timeout)
    receipt["commands"].append({"command": command, "exit": result.returncode, "log": str(log_path)})
    if result.returncode:
        raise ValueError(f"native export failed: {log_path}")
    return destination


def photograph(client, row: dict, site: dict, directory: Path) -> dict:
    attempts = []
    selected = None
    for sample in site["samples"]:
        for stand in sample["stands"]:
            dimension_teleport(client, row["dimension"], stand)
            client.command("gamemode survival @s", "game mode")
            client.call("wait", 10)
            state = client.call("state")
            player = state.get("player") or {}
            actual = player.get("position")
            attempt = {"target": sample["position"], "stand": stand, "player": player}
            attempts.append(attempt)
            if (not player.get("on_ground") or not isinstance(actual, list) or len(actual) != 3 or
                    any(abs(a - b) > 0.05 for a, b in zip(actual, stand))):
                continue
            hit = visible_face(client, sample["position"])
            if hit is not None:
                selected = {"state": state, "hit": hit, "sample": sample}
                break
        if selected is not None:
            break
    if selected is None:
        raise ValueError(f"{row['id']}: no visible template block within survival reach")
    observed = client.call("block", *selected["sample"]["position"])
    if not observed.get("loaded") or observed.get("block", "").split("[")[0] != selected["sample"]["state"].split("[")[0]:
        raise ValueError(f"client block mismatch: {selected['sample']}, {observed}")
    picture = directory / (row["id"].replace(":", "_").replace("/", "_") + ".png")
    capture = client.call("screenshot", picture)
    if picture.read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"invalid screenshot: {picture}")
    return {"state": selected["state"], "block": observed, "hit": selected["hit"],
            "attempts": attempts, "screenshot": str(picture),
            "screenshot_sha256": digest(picture), "capture": capture}


def visit_coverage(client, rows: list[dict]) -> None:
    """Generate a grid covering each expanded start; exports verify actual coverage."""
    visited = set()
    for row in rows:
        if row["status"] != "located":
            continue
        bounds = expanded_bounds(row["initial_site"]["bounds"], 16)
        for x in range(bounds[0] >> 4, (bounds[3] >> 4) + 1, 4):
            for z in range(bounds[2] >> 4, (bounds[5] >> 4) + 1, 4):
                key = (row["dimension"], x, z)
                if key not in visited:
                    dimension_teleport(client, row["dimension"], [x * 16 + 8.5, 100, z * 16 + 8.5])
                    visited.add(key)


def run(ctx: ProfileContext) -> dict:
    def env_path(name):
        value = os.environ.get(name)
        if not value:
            raise ValueError(f"{name} is required")
        return Path(value).resolve(strict=True)

    manifest_path = env_path("KRASNOKAMEN_ATM_MANIFEST")
    manifest = json.loads(manifest_path.read_text())
    packs = manifest.get("packs", [])
    if manifest.get("status") != "converted-unproven" or len(packs) != 1 or packs[0]["id"] != "mvs":
        raise ValueError("one full converted MVS pack is required")
    pack = Path(packs[0]["server_pack"]).resolve(strict=True)
    if inventory(pack) != packs[0]["server_files"] or tree_hash(packs[0]["server_files"]) != packs[0]["server_sha256"]:
        raise ValueError("converted MVS pack changed")
    rows = selected_definitions(pack)
    chosen = restart_selection(rows)
    subset = os.environ.get("KRASNOKAMEN_ATM_CONTROL_SUBSET", "")
    if subset not in ("", "1"):
        raise ValueError("KRASNOKAMEN_ATM_CONTROL_SUBSET must be absent or 1")
    active = [row for row in rows if not subset or row["id"] in chosen]
    if subset:
        for row in rows:
            if row["id"] not in chosen:
                row.update(status="control_not_selected", reason="outside placement-control subset")
    output = ctx.artifact_dir / "full-atm"
    output.mkdir()
    args = SimpleNamespace(output=output, server=env_path("KRASNOKAMEN_SERVER_BINARY"),
                           client=env_path("KRASNOKAMEN_BATTLE_HORIZON_CLIENT"),
                           ctl=env_path("KRASNOKAMEN_BATTLE_HORIZON_CTL"),
                           resources=env_path("KRASNOKAMEN_BATTLE_HORIZON_RESOURCES"),
                           cache=env_path("KRASNOKAMEN_ATM_CACHE"), timeout=ctx.timeout_seconds or 600)
    report = {"status": "failed", "commands": [], "seed": SEED, "definitions": rows,
              "restart_rule": "every tenth sorted Overworld ID starting at index 0 and all Nether IDs",
              "dimension_travel": "execute in <dimension> run tp @s", "manifest_sha256": digest(manifest_path),
              "server_sha256": digest(args.server), "client_sha256": digest(args.client), "ctl_sha256": digest(args.ctl)}
    report["control_subset"] = bool(subset)
    report["differential_minimum"] = MIN_DIFFERENTIAL_BLOCKS
    ctx.failure_scenario = report
    write_json(output / "definitions.json", rows)
    write_json(output / "result.json", report)
    world, plugins = output / "world", output / "plugins"
    world.mkdir(); plugins.mkdir()
    shutil.copytree(pack, world / "datapacks" / "mvs")
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    token = 'vanilla_data_dir = "data/vanilla"'
    if source.count(token) != 1:
        raise ValueError("playable.toml vanilla_data_dir token changed")
    source = source.replace(token, "vanilla_data_dir = " + json.dumps(str(args.cache)))
    port = runtime.reserve_port()
    config = output / "server.toml"
    config.write_text(rewrite_config(source, port=port, world=str(world), plugins=str(plugins), seed=SEED))
    prepare_operator_fixture(ProfileContext(output), [USERNAME])
    with session(args, config, port, "discovery", report) as client:
        for row in active:
            dimension_teleport(client, row["dimension"], [-255.5, 100, -255.5])
            messages = client.command(f"locate structure {row['id']}", row["id"])
            row["locate_messages"] = messages
            try:
                row["xz"] = locate_coordinates([json.dumps(messages)], row["id"])
            except ValueError:
                reason = locate_failure(messages, row["id"])
                if reason is None:
                    raise
                row.update(status="not_found", reason=reason)
                write_json(output / "result.json", report)
                continue
            dimension_teleport(client, row["dimension"], [row["xz"][0], 100, row["xz"][1]])
            row.update(status="located", reason=None)
            write_json(output / "result.json", report)
    discovery_export = export_world(args, config, world, "discovery-export", report)
    for row in active:
        if row["status"] == "located":
            try:
                row["initial_site"] = find_site(discovery_export, row)
            except ValueError as error:
                row.update(status="verification_failed", reason=str(error))
            write_json(output / "result.json", report)
    with session(args, config, port, "coverage", report) as client:
        visit_coverage(client, active)
    export = export_world(args, config, world, "before-export", report)
    baseline_world = output / "without-pack-world"
    baseline_world.mkdir()
    baseline_config = output / "without-pack-server.toml"
    baseline_config.write_text(rewrite_config(source, port=port, world=str(baseline_world),
                                               plugins=str(plugins), seed=SEED))
    with session(args, baseline_config, port, "without-pack-coverage", report) as client:
        visit_coverage(client, active)
    baseline_export = export_world(args, baseline_config, baseline_world, "without-pack-export", report)
    for row in active:
        if row["status"] == "located":
            try:
                row["before"] = site_proof(export, baseline_export, pack, row, "before")
            except ValueError as error:
                row.update(status="verification_failed", reason=str(error))
            write_json(output / "result.json", report)
    (output / "photographs").mkdir()
    with session(args, config, port, "photograph-session", report) as client:
        for row in active:
            if row["status"] == "located":
                try:
                    row["visit"] = photograph(client, row, row["before"], output / "photographs")
                    row["status"] = "visited"
                except ValueError as error:
                    if not str(error).startswith((row["id"] + ": no visible", "client block mismatch:")):
                        raise
                    row.update(status="verification_failed", reason=str(error))
                write_json(output / "result.json", report)
    after = export_world(args, config, world, "after-export", report)
    for row in active:
        if row["status"] == "visited":
            try:
                proof = site_proof(after, baseline_export, pack, row, "after")
                if (proof["anchor"], proof["bounds"], proof["chunk"]) != (row["before"]["anchor"], row["before"]["bounds"], row["before"]["chunk"]):
                    raise ValueError(f"{row['id']}: start changed across restart")
                blocks = read_world(Path(proof["dimension_dir"]), [proof["bounds"]])[0]["blocks"]
                for sample in row["before"]["samples"]:
                    key = ",".join(map(str, sample["position"]))
                    if blocks.get(key, "minecraft:air") != sample["state"]:
                        raise ValueError(f"{row['id']}: persisted structure block changed at {key}")
                row.update(after=proof, status="verified")
            except ValueError as error:
                row.update(status="verification_failed", reason=str(error))
            write_json(output / "result.json", report)
    (output / "restart-photographs").mkdir()
    with session(args, config, port, "restart", report) as client:
        for row in active:
            if row["status"] == "verified" and row["id"] in chosen:
                try:
                    row["restart_visit"] = photograph(client, row, row["after"], output / "restart-photographs")
                except ValueError as error:
                    if not str(error).startswith((row["id"] + ": no visible", "client block mismatch:")):
                        raise
                    row.update(status="verification_failed", reason=str(error))
                write_json(output / "result.json", report)
    report["status"] = "passed" if not subset and all(row["status"] == "verified" for row in rows) else "findings"
    report["full_target_accepted"] = report["status"] == "passed"
    write_json(output / "result.json", report)
    if not report["full_target_accepted"]:
        raise RuntimeError("full ATM campaign has unverified definitions; see full-atm/result.json")
    return {"full_atm": report}
