"""Local ATM structure-data conversion; never downloads or publishes pack bytes.

Run with --help. Horizon owns the 1.21.1 conversion, not upgrade-datapack
(which alone handles 26.2). The server adapter preserves source worldgen's
26.1.2-compatible grammar and custom semantics, separately from the 26.3 carry.
Neither output is a claim that the server implements those semantics.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tomllib
import zipfile

HORIZON_COMMIT = "0a4dc7ca2923da8f3629c57706e44ad4bc59915f"
LICENCES = {
    "MIT": "MIT", "Apache-2.0": "Apache-2.0", "BSD-2-Clause": "BSD-2-Clause",
    "BSD-3-Clause": "BSD-3-Clause", "CC0-1.0": "CC0-1.0", "CC-BY-4.0": "CC-BY-4.0",
    "LGPLv3": "LGPL-3.0", "GNU Lesser General Public License v3.0": "LGPL-3.0",
    "LGPL-3.0": "LGPL-3.0", "LGPL-3.0-only": "LGPL-3.0-only",
    "LGPL-3.0-or-later": "LGPL-3.0-or-later", "GPL-3.0": "GPL-3.0",
    "GPL-3.0-only": "GPL-3.0-only", "GPL-3.0-or-later": "GPL-3.0-or-later",
    "MPL-2.0": "MPL-2.0",
}
IDENTIFIER = re.compile(r"[a-z0-9_.-]+:[a-z0-9_./-]+\Z")


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def inventory(path: Path) -> dict[str, str]:
    return {p.relative_to(path).as_posix(): digest(p) for p in sorted(path.rglob("*")) if p.is_file()}


def tree_hash(files: dict[str, str]) -> str:
    return hashlib.sha256(json.dumps(files, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def write_json(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def inspect_jar(jar: Path, spec: dict) -> tuple[dict, dict[str, bytes]]:
    if digest(jar) != spec["sha256"]:
        raise ValueError(f"{jar}: SHA-256 differs from pinned input")
    with zipfile.ZipFile(jar) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)) or len(names) > 65536:
            raise ValueError(f"{jar}: duplicate entries or more than 65536 entries")
        if sum(info.file_size for info in archive.infolist()) > 512 * 1024 * 1024:
            raise ValueError(f"{jar}: unpacked input exceeds 512 MiB")
        for name in names:
            p = PurePosixPath(name)
            if p.is_absolute() or ".." in p.parts or "\\" in name:
                raise ValueError(f"{jar}: unsafe archive entry {name}")
        entry = spec["licence_entry"]
        raw = archive.read(entry)
        if hashlib.sha256(raw).hexdigest() != spec["licence_sha256"]:
            raise ValueError(f"{jar}: licence evidence changed: {entry}")
        meta = tomllib.loads(raw.decode("utf-8"))
        licence = meta.get("license")
        if licence not in LICENCES or LICENCES[licence] != spec["licence"]:
            raise ValueError(f"{jar}: unapproved or mismatched jar licence {licence!r}")
        mod = next((m for m in meta.get("mods", []) if m.get("modId") == spec["id"]), None)
        if mod is None or mod.get("version") != spec["version"]:
            raise ValueError(f"{jar}: mod id/version differs from census selection")
        pack = json.loads(archive.read("pack.mcmeta"))["pack"]
        if pack.get("pack_format") != 48:
            raise ValueError(f"{jar}: expected 1.21.1 format 48, got {pack.get('pack_format')!r}")
        data = {n: archive.read(n) for n in names if n.startswith("data/") and not n.endswith("/")}
        for name, value in data.items():
            if name.endswith(".json"):
                try:
                    json.loads(value.decode("utf-8-sig"))
                except (ValueError, UnicodeError) as error:
                    raise ValueError(f"{jar}:{name}: invalid source JSON: {error}") from error
        evidence = {"metadata": meta, "licence_entry": entry, "licence_sha256": spec["licence_sha256"],
                    "licence": spec["licence"], "jar_sha256": spec["sha256"],
                    "jar_sha1": hashlib.sha1(jar.read_bytes()).hexdigest(), "source_pack": pack,
                    "source_data": {n: hashlib.sha256(v).hexdigest() for n, v in sorted(data.items())},
                    "classes_not_executed_or_copied": sum(n.endswith(".class") for n in names)}
        return evidence, data


def conversion_errors(report: dict, original: dict[str, bytes], carried: Path) -> list[str]:
    errors = []
    for key in ("outside", "unreadable", "blocks_unknown", "items_unknown", "differences",
                "recipes_kept", "recipes_unregistered", "recipes_off"):
        if report.get(key):
            errors.append(f"Horizon {key}: {report[key]}")
    for key in ("unsupported", "rules_lost", "processors_dropped", "modifiers_left"):
        value = report.get("worldgen_types", {}).get(key)
        if value:
            errors.append(f"Horizon worldgen {key}: {value}")
    for name in original:
        if not (carried / name).is_file():
            errors.append(f"source data missing after carry: {name}")
    for path in sorted((carried / "data").rglob("*.json")):
        try:
            json.loads(path.read_text())
        except ValueError as error:
            errors.append(f"invalid converted JSON {path}: {error}")
    if report.get("blocks") or report.get("items"):
        errors.append("requires custom block/item registration; not a vanilla-client structure pack")
    return errors


def convert(spec: dict, binary: Path, output: Path) -> dict:
    jar = Path(spec["jar"]).resolve()
    evidence, source = inspect_jar(jar, spec)
    work = output / spec["id"]
    work.mkdir()
    cmd = [str(binary), "check", str(jar), str(work / "horizon")]
    with (work / "horizon.log").open("w") as log:
        result = subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT, timeout=180)
    if result.returncode:
        raise ValueError(f"Horizon carry failed ({result.returncode}); see {work / 'horizon.log'}")
    report = json.loads((work / "horizon/carry-report.json").read_text())
    carried = work / "horizon/carry"
    evidence.update(command=cmd, carry_report=report, source_url=spec["source_url"],
                    version=spec["version"], id=spec["id"], status="unproven")
    errors = conversion_errors(report, source, carried)
    if errors:
        evidence.update(status="rejected", errors=errors)
        write_json(work / "conversion.json", evidence)
        raise ValueError(f"{spec['id']}: conversion loss/unsupported format; see {work / 'conversion.json'}")
    intermediate = work / "intermediate"
    shutil.copytree(carried / "data", intermediate / "data")
    metadata = {"pack": {"description": f"Local adapted {spec['id']} data, EXPERIMENTAL",
                         "pack_format": 48}}
    write_json(intermediate / "pack.mcmeta", metadata)
    canonical = work / "datapack-26.3"
    upgrade_cmd = [str(binary), "upgrade-datapack", str(intermediate), str(canonical)]
    upgrade = subprocess.run(upgrade_cmd, text=True, capture_output=True, timeout=180)
    upgrade_log = upgrade.stdout + upgrade.stderr
    (work / "upgrade.log").write_text(upgrade_log)
    evidence.update(upgrade_command=upgrade_cmd, upgrade_output=upgrade_log)
    if upgrade.returncode or "not faithful" in upgrade_log or "left as it is" in upgrade_log:
        evidence.update(status="rejected", errors=["26.3 upgrade failed or reported unsupported/lossy data"])
        write_json(work / "conversion.json", evidence)
        raise ValueError(f"{spec['id']}: 26.3 upgrade not faithful; see {work / 'upgrade.log'}")
    if json.loads((canonical / "pack.mcmeta").read_text())["pack"]["pack_format"] != 121:
        raise ValueError(f"{spec['id']}: converter did not produce format 121")
    server = work / "server-adapted"
    shutil.copytree(canonical, server)
    preserved = []
    for name, value in source.items():
        parts = PurePosixPath(name).parts
        # Keep all original worldgen semantics, including custom library data/tags/NBT.
        # Only gameplay loot/recipes/advancements take Horizon's 26.3 conversion.
        if parts[2] not in ("loot_table", "recipe", "advancement"):
            path = server / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(value)
            preserved.append(name)
    requirements = set()
    def walk(value):
        if isinstance(value, dict):
            for key, child in value.items():
                if key.startswith("horizon:") or (key in ("type", "processor_type", "element_type")
                    and isinstance(child, str) and not child.startswith("minecraft:")):
                    requirements.add(f"{key}={child}" if isinstance(child, str) else key)
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)
    for path in (canonical / "data").rglob("*.json"):
        walk(json.loads(path.read_text()))
    for structure in spec["structures"]:
        if not IDENTIFIER.fullmatch(structure):
            raise ValueError(f"invalid structure identifier: {structure}")
        ns, name = structure.split(":")
        definition = server / f"data/{ns}/worldgen/structure/{name}.json"
        if not definition.is_file():
            raise ValueError(f"selected structure absent from jar: {structure}")
    canonical_files, server_files = inventory(canonical), inventory(server)
    evidence.update(canonical_format="26.3 Horizon carry (extensions required, not vanilla-only)",
                    server_format="26.3 gameplay; original 1.21.1 worldgen for internal 26.1.2 codec",
                    preserved_source_worldgen=preserved, canonical_requirements=sorted(requirements),
                    canonical_files=canonical_files, server_files=server_files,
                    canonical_sha256=tree_hash(canonical_files), server_sha256=tree_hash(server_files),
                    original_data_count=len(source), converted_data_count=len(canonical_files) - 1,
                    fidelity="No oracle comparison performed; conversion is not generation acceptance",
                    assets="Not installed: server-side data only; original assets remain in Horizon carry",
                    structures=spec["structures"], server_pack=str(server), canonical_pack=str(canonical))
    write_json(work / "conversion.json", evidence)
    return evidence


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", type=Path, required=True, help="pinned local jar/licence manifest")
    parser.add_argument("--horizon", type=Path, required=True, help="own checkout's horizon-port-pack executable")
    parser.add_argument("--horizon-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="new local directory beneath targets/")
    args = parser.parse_args()
    output = args.output.resolve()
    if "targets" not in output.parts:
        parser.error("outputs must stay local beneath targets/")
    output.mkdir(parents=True, exist_ok=False)
    result = {"status": "failed", "packs": []}
    try:
        commit = subprocess.check_output(["git", "-C", str(args.horizon_root), "rev-parse", "HEAD"], text=True).strip()
        if commit != HORIZON_COMMIT:
            raise ValueError(f"Horizon checkout must be {HORIZON_COMMIT}, got {commit}")
        result.update(horizon_commit=commit, horizon_binary_sha256=digest(args.horizon),
                      inputs_sha256=digest(args.inputs))
        specs = json.loads(args.inputs.read_text())["packs"]
        ids = [spec["id"] for spec in specs]
        if not specs or len(set(ids)) != len(ids) or any(not re.fullmatch(r"[a-z0-9_-]+", name) for name in ids):
            raise ValueError("pack selection must contain unique safe ids")
        for spec in specs:
            result["packs"].append(convert(spec, args.horizon.resolve(), output))
        result["status"] = "converted-unproven"
    except (ValueError, OSError, KeyError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
        result["error"] = str(error)
    write_json(output / "result.json", result)
    print(json.dumps({"status": result["status"], "receipt": str(output / "result.json"), "error": result.get("error")}))
    return 0 if result["status"] == "converted-unproven" else 1


if __name__ == "__main__":
    raise SystemExit(main())
