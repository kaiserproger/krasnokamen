"""Reproducible release workload measurements, not capacity/parity acceptance.

Reuses the canonical public-command fixture, process lifecycle and exact tick
stream. Preparation and login are outside the warmup/measurement windows.
"""
from __future__ import annotations

import argparse
import asyncio
import copy
import json
import os
from pathlib import Path
import platform
import secrets
import shlex
import signal
import statistics
import subprocess
import sys
import time
from typing import Any

from tools.coreperf import compare
from tools.harness.backends import catalog_load as load

PROFILES = {"idle": (0, 0, 0, 8), "explore-3": (3, 0, 0, 8), "explore-20": (20, 0, 0, 8),
            "native-100": (100, 100000, 0, 6), "villager-heavy": (100, 100000, 20000, 6)}
SCALE_PROFILES = {
    f"scale-{label}-{mix}": (count, mix)
    for label, count in (("100k", 100_000), ("500k", 500_000), ("1m", 1_000_000))
    for mix in ("all-active", "mostly-resting", "sparse-active")
}

PIN_ROOT = Path("/run/media/kaiserroman/games/solaris-work/pins")


def pinned_binary(binary: Path, kind: str) -> dict[str, Any]:
    """Verify pin provenance, retaining the immutable-store rule on the owner host."""
    binary = binary.resolve()
    owner_host = (PIN_ROOT.parent / "solaris/tools/dev/mem-run.sh").is_file()
    if owner_host and binary.parent.parent != PIN_ROOT.resolve():
        raise ValueError(f"{kind} binary is not in the pin-release store")
    manifest = json.loads((binary.parent / "manifest.json").read_text())
    recorded_binary = manifest.get("binary")
    path_matches = isinstance(recorded_binary, str) and (
        recorded_binary == str(binary) if owner_host
        else Path(recorded_binary).name == binary.name)
    if (manifest.get("repo") != "core" or manifest.get("kind") != kind
            or not path_matches or not manifest.get("head")
            or not manifest.get("diff_sha256")
            or not manifest.get("command") or not manifest.get("target")
            or manifest.get("binary_sha256") != load.scale.binary_sha256(binary)):
        raise ValueError(f"{kind} pin-release manifest does not match binary")
    return manifest


def validate_cohort(before: dict[str, Any], after: dict[str, Any],
                    expected: dict[str, int]) -> None:
    for cohort in (before, after):
        identities = cohort.get("identities")
        if not isinstance(identities, dict) or cohort.get("counts") != expected:
            raise ValueError("missing saved identities or wrong fixture population")
        counts = dict.fromkeys(expected, 0)
        for row in identities.values():
            kind = row.get("kind")
            if kind not in counts:
                raise ValueError("unexpected saved fixture identity")
            counts[kind] += 1
        if counts != expected:
            raise ValueError("missing saved identities or wrong fixture population")
    if before["identities"].keys() != after["identities"].keys():
        raise ValueError("persistent fixture cohort changed identity")
    if any(before["identities"][key]["kind"] != after["identities"][key]["kind"]
           for key in before["identities"]):
        raise ValueError("persistent fixture identity changed kind")


def validate_prepared(recipe: dict[str, Any], prepared: dict[str, Any]) -> None:
    if prepared.get("recipe_sha256") != load.digest(recipe):
        raise ValueError("prepared fixture recipe differs from requested recipe")
    cohort = prepared.get("cohort")
    if not isinstance(cohort, dict):
        raise ValueError("prepared fixture lacks saved identities")
    validate_cohort(cohort, cohort, recipe["expected"])


def fixture(mobs: int, villagers: int) -> dict[str, Any]:
    """PGOBolt's shared public-command pens; no private world seeding."""
    value = compare.fixture(mobs, 0)
    value["regions"].pop()
    regions: list[dict[str, Any]] = []
    for region in value["regions"]:
        if (regions and not regions[-1].get("paired")
                and regions[-1]["anchor"][1] == region["anchor"][1]):
            previous = regions[-1]
            previous["anchor"][0] = (previous["anchor"][0] + region["anchor"][0]) // 2
            previous["commands"].extend(region["commands"])
            previous["paired"] = True
        else:
            regions.append(region)
    value["regions"] = regions
    for group, offset in enumerate(range(0, villagers, 1024)):
        x, z = -800 - (group % 5) * 96, (group // 5) * 96
        commands = [
            f"fill {x-8} 62 {z-8} {x+72} 62 {z+72} minecraft:dirt",
            f"fill {x-8} 63 {z-8} {x+72} 63 {z+72} minecraft:grass_block",
            f"fill {x-9} 64 {z-9} {x+73} 70 {z-9} minecraft:stone",
            f"fill {x-9} 64 {z+73} {x+73} 70 {z+73} minecraft:stone",
            f"fill {x-9} 64 {z-8} {x-9} 70 {z+72} minecraft:stone",
            f"fill {x+73} 64 {z-8} {x+73} 70 {z+72} minecraft:stone",
            f"fill {x-9} 71 {z-9} {x+73} 71 {z+73} minecraft:stone",
        ]
        for dx in (0, 40):
            for dz in (0, 40):
                commands.append(f"fill {x+dx-8} 64 {z+dz-8} {x+dx+32} 70 {z+dz+32} minecraft:air")
        for index in range(min(1024, villagers-offset)):
            px, pz = x+(index % 32)*2, z+(index//32)*2
            commands.extend([
                f"setblock {px} 64 {pz} minecraft:composter",
                f"setblock {px} 64 {pz+1} minecraft:red_bed[facing=east,part=foot]",
                f"setblock {px+1} 64 {pz+1} minecraft:red_bed[facing=east,part=head]",
                f"summon minecraft:villager {px+1.5} 64 {pz+.5} {{CustomName:'{compare.COHORT_NAME}',PersistenceRequired:1b}}",
            ])
        value["regions"].append({"name": f"villagers-{group}", "anchor": [x+32, z+32],
                                 "view_distance": 8, "commands": commands})
    value["expected"]["minecraft:villager"] = villagers
    # Keep this identity byte-for-byte compatible with the shared PGO cache.
    value["variant"] = "pgo-internal-100k-mobs-20k-villagers-100-wire-players"
    value["official_performance_evidence"] = False
    if not mobs and not villagers:
        value["regions"] = [{"name": "spawn", "anchor": [32, 32], "view_distance": 8,
            "commands": ["gamerule doMobSpawning false", "gamerule doWeatherCycle false",
                         "gamerule randomTickSpeed 3", "gamerule doDaylightCycle true",
                         "weather clear", "time set 2000", "difficulty easy"]}]
        value["variant"] = "profiles-natural-terrain-no-summoned-cohort"
    return value


def scale_fixture(residents: int, mix: str, *, excluded_kinds: tuple[str, ...] = (),
                  active_override: int | None = None) -> dict[str, Any]:
    """Separate public-command pens, with cold regions kept resident by adjacent tickets."""
    if residents < 0 or mix not in ("all-active", "mostly-resting", "sparse-active"):
        raise ValueError("invalid scale population or activity mix")
    kinds = compare.MOB_KINDS
    eligible = tuple(kind for kind in kinds if kind not in excluded_kinds)
    if len(eligible) < 2:
        raise ValueError("scale fixture requires at least two species")
    counts = {kind: 0 for kind in kinds}
    if mix == "mostly-resting":
        primary = next((kind for kind in ("cow", "sheep", "pig") if kind in eligible), eligible[0])
        secondary = ("zombie" if "zombie" in eligible and primary != "zombie" else
                     next(kind for kind in eligible if kind != primary))
        counts[primary] = residents - residents // 10
        counts[secondary] = residents // 10
    else:
        for index, kind in enumerate(eligible):
            counts[kind] = residents // len(eligible) + (index < residents % len(eligible))
    active_total = (residents // 10 if mix == "sparse-active" else residents)
    if active_override is not None:
        active_total = active_override
    if active_total < 0 or active_total > residents:
        raise ValueError("active population exceeds residents")
    active_left = active_total
    portions: dict[str, int] = {}
    for kind, count in counts.items():
        portions[kind] = min(count, active_left)
        active_left -= portions[kind]
    regions = []
    coverage = []
    for active in (True, False):
        for kind, total in counts.items():
            count = portions[kind] if active else total - portions[kind]
            for offset in range(0, count, 1024):
                n = min(1024, count - offset)
                group = len(regions)
                x, z = group % 32 * 256, group // 32 * 256
                roof = 67 if kind == "spider" else 71
                commands = [
                    f"fill {x-1} 62 {z-1} {x+64} 62 {z+64} minecraft:dirt",
                    f"fill {x-1} 63 {z-1} {x+64} 63 {z+64} minecraft:grass_block",
                    f"fill {x} 64 {z} {x+63} 70 {z+63} minecraft:air",
                    f"fill {x-1} 64 {z-1} {x+64} {roof-1} {z-1} minecraft:stone",
                    f"fill {x-1} 64 {z+64} {x+64} {roof-1} {z+64} minecraft:stone",
                    f"fill {x-1} 64 {z} {x-1} {roof-1} {z+63} minecraft:stone",
                    f"fill {x+64} 64 {z} {x+64} {roof-1} {z+63} minecraft:stone",
                    f"fill {x-1} {roof} {z-1} {x+64} {roof} {z+64} minecraft:stone",
                ]
                if active:
                    # Keep the normal moving observer on its roof ticket; no
                    # cadence or view-distance reduction is used for coverage.
                    commands += [
                        f"fill {x-1} {roof+1} {z-1} {x+64} {roof+2} {z-1} minecraft:stone",
                        f"fill {x-1} {roof+1} {z+64} {x+64} {roof+2} {z+64} minecraft:stone",
                        f"fill {x-1} {roof+1} {z} {x-1} {roof+2} {z+63} minecraft:stone",
                        f"fill {x+64} {roof+1} {z} {x+64} {roof+2} {z+63} minecraft:stone",
                    ]
                if not regions:
                    commands[:0] = [
                        "gamerule doMobSpawning false", "gamerule doWeatherCycle false",
                        "gamerule randomTickSpeed 3", "gamerule doDaylightCycle true",
                        "weather clear", "time set 2000", "difficulty easy",
                        f"setworldspawn {x+32} 64 {z+32}",
                        *[f"kill @e[type=minecraft:{name}]" for name in (*kinds, "villager")],
                    ]
                if not active:
                    # Ticket centres lie outside the pen; their level-33 halo
                    # retains all four columns without ticking the pen itself.
                    commands += [f"forceload add {x-16} {z+16}", f"forceload add {x+64} {z+16}"]
                tag = f" {{CustomName:'{compare.COHORT_NAME}',PersistenceRequired:1b}}"
                commands.extend(
                    f"summon minecraft:{kind} {x+(i % 32)*2+1.0} 64 {z+(i//32)*2+1.0}{tag}"
                    for i in range(n))
                regions.append({"name": f"{'active' if active else 'cold'}-{kind}-{offset//1024}",
                                "anchor": [x+32, z+32], "view_distance": 8, "commands": commands})
                coverage.append({"chunks": [[x//16+dx, z//16+dz] for dx in range(4) for dz in range(4)],
                                 "active": active, "resident": n, "kind": f"minecraft:{kind}"})
    return {"schema_version": 1, "kind": "native-v2-public-commands", "seed": 42,
            "variant": f"noncanonical-scale-{residents}-{mix}",
            "expected": {f"minecraft:{kind}": count for kind, count in counts.items()},
            "regions": regions, "coverage": coverage,
            "population": {"resident_requested": residents, "active_requested": active_total,
                           "cold_requested": residents-active_total},
            "official_performance_evidence": False}


def add_dispenser_bank(recipe: dict[str, Any], count: int, capacity: int, item: str) -> tuple[int, str, str]:
    """Build an ordinary powered-dispenser bank in a separate active region."""
    if count < 0 or count > capacity or capacity > 1024:
        raise ValueError("ammunition count exceeds dispenser bank capacity")
    group = len(recipe["regions"])
    x, z = group % 32 * 256, group // 32 * 256
    commands = [
        f"fill {x-1} 62 {z-1} {x+80} 62 {z+64} minecraft:dirt",
        f"fill {x-1} 63 {z-1} {x+80} 63 {z+64} minecraft:grass_block",
        f"fill {x} 64 {z} {x+63} 70 {z+63} minecraft:air",
        f"fill {x+65} 64 {z} {x+79} 70 {z+63} minecraft:air",
        f"fill {x+64} 64 {z-1} {x+80} 66 {z-1} minecraft:stone",
        f"fill {x+64} 64 {z+64} {x+80} 66 {z+64} minecraft:stone",
        f"fill {x+64} 64 {z} {x+64} 66 {z+63} minecraft:stone",
        f"fill {x+80} 64 {z} {x+80} 66 {z+63} minecraft:stone",
    ]
    for i in range(capacity):
        px, pz = x + (i % 32) * 2, z + (i // 32) * 2
        commands.append(f"setblock {px} 66 {pz} minecraft:dispenser[facing=up,triggered=false]")
        recipe.setdefault("expected_blocks", []).append({"position": [px, 66, pz], "block": "minecraft:dispenser",
            "item": f"minecraft:{item}" if i < count else "minecraft:air", "count": 64 if i < count else 0})
        if i < count:
            commands.append(f"item replace block {px} 66 {pz} container.0 with minecraft:{item} 64")
    recipe["regions"].append({"name": f"{item}-bank", "anchor": [x+72, z+32],
                              "view_distance": 8, "commands": commands})
    recipe["coverage"].append({"chunks": [[x//16+dx, z//16+dz] for dx in range(4) for dz in range(4)],
                               "active": True, "resident": 0, "kind": None})
    return (group, f"fill {x} 65 {z} {x+63} 65 {z+63} minecraft:redstone_block",
            f"fill {x} 65 {z} {x+63} 65 {z+63} minecraft:air")


def add_projectile_fixture(recipe: dict[str, Any], count: int, *, smoke: bool) -> None:
    """Pulse an identical dispenser bank; only the loaded-ammunition count varies."""
    capacity = max(count, 4) if smoke else 1000
    group, on, off = add_dispenser_bank(recipe, count, capacity, "arrow")
    recipe["stress"] = {"kind": "projectiles", "regions": [group], "requested_count": count,
                        "dispenser_capacity": capacity, "pulse_interval_ms": 3000,
                        "commands": [on, off]}


def add_xp_fixture(recipe: dict[str, Any], count: int, *, smoke: bool,
                   players: int = 1, locality: str = "compact") -> None:
    """One active bank region, with independently counted observer anchors."""
    if (count < 0 or count > 100000 or players not in (0, 1, 100, 1000)
            or (players == 0 and count != 0) or locality not in ("compact", "spread")):
        raise ValueError("invalid XP workload")
    capacity = (max(count, 4) if smoke else 100000) if count else 0
    group, _, _ = add_dispenser_bank(recipe, 0, 0, "experience_bottle")
    region = recipe["regions"][group]
    region["name"] = "xp-observer-bank" if count else "xp-observer"
    recipe["variant"] = f"noncanonical-xp-{recipe['population']['resident_requested']}-cold-background"
    x, z = group % 32 * 256, group // 32 * 256
    if players:
        region["commands"].append(f"setworldspawn {x+32} 64 {z+32}")
    else:
        recipe["coverage"][group]["active"] = False
        region["commands"] += [f"setworldspawn {x+128} 64 {z+128}",
                               f"forceload add {x-16} {z+16}", f"forceload add {x+64} {z+16}"]
    on, off = [], []
    width = 63
    for start in range(0, capacity, width * width):
        size = min(width * width, capacity - start)
        y = 66 + (start // (width * width)) * 3
        rows, remainder = divmod(size, width)
        if rows:
            region["commands"].append(f"fill {x} {y} {z} {x+width-1} {y} {z+rows-1} minecraft:dispenser[facing=up,triggered=false]")
        if remainder:
            region["commands"].append(f"fill {x} {y} {z+rows} {x+remainder-1} {y} {z+rows} minecraft:dispenser[facing=up,triggered=false]")
        recipe.setdefault("expected_blocks", []).extend(
            {"position": [x+i%width, y, z+i//width], "block": "minecraft:dispenser",
             "item": "minecraft:experience_bottle" if start+i < count else "minecraft:air",
             "count": 64 if start+i < count else 0} for i in range(size))
        for i in range(min(size, max(0, count-start))):
            region["commands"].append(f"item replace block {x+i%width} {y} {z+i//width} container.0 with minecraft:experience_bottle 64")
        on.append(f"fill {x} {y-1} {z} {x+width-1} {y-1} {z+width-1} minecraft:redstone_block")
        off.append(f"fill {x} {y-1} {z} {x+width-1} {y-1} {z+width-1} minecraft:air")
    anchors = ([[x+32, z+32]] * players if locality == "compact" else
               [[x+8+(i%4)*16, z+8+((i//4)%4)*16] for i in range(players)])
    recipe["stress"] = {"kind": "xp-orbs", "regions": [group] if count else [],
                        "observer_region": group, "requested_count": count,
                        "dispenser_capacity": capacity, "players": players,
                        "locality": locality, "anchors": anchors,
                        "pulse_interval_ms": max(1, 6000 // max(1, len(on)+len(off))),
                        "commands": on + off}




def add_brewing_fixture(recipe: dict[str, Any], containers: int, *, stands: int = 5) -> None:
    """Place non-brewing containers and a fixed set of genuinely brewing stands."""
    if containers < 1 or stands < 1:
        raise ValueError("container and brewing-stand counts must be positive")
    groups, refresh, discovery = [], [], {}
    for start in range(0, containers, 1000):
        group, _, _ = add_dispenser_bank(recipe, 0, 0, "arrow")
        groups.append(group)
        region = recipe["regions"][-1]
        region["name"] = "brewing-containers"
        x, z = group % 32 * 256, group // 32 * 256
        discovery[str(group)] = min(1000, containers - start) + (stands if start == 0 else 0)
        for i in range(min(1000, containers - start)):
            px, pz = x + (i % 32) * 2, z + (i // 32) * 2
            region["commands"].append(f"setblock {px} 64 {pz} minecraft:chest")
            recipe.setdefault("expected_blocks", []).append({"position": [px, 64, pz], "block": "minecraft:chest"})
        if start == 0:
            for i in range(stands):
                px, pz = x + (i % 32) * 2, z + (i // 32) * 2
                recipe.setdefault("expected_blocks", []).append({"position": [px, 68, pz], "block": "minecraft:brewing_stand"})
                region["commands"] += [
                    f"setblock {px} 68 {pz} minecraft:brewing_stand",
                    f"item replace block {px} 68 {pz} container.3 with minecraft:nether_wart 64",
                    f"item replace block {px} 68 {pz} container.4 with minecraft:blaze_powder 64",
                ]
                refresh.append(
                    f'item replace block {px} 68 {pz} container.0 with '
                    'minecraft:potion[potion_contents={potion:"minecraft:water"}]')
            region["commands"] += refresh
    recipe["stress"] = {"kind": "brewing-containers", "regions": groups,
                        "requested_containers": containers, "requested_stands": stands,
                        "discovery_per_region": discovery,
                        "pulse_interval_ms": max(1, 10000 // stands), "commands": refresh}


def add_productive_fixture(recipe: dict[str, Any], workers: int, *, smoke: bool) -> None:
    """One unbound farmer per walled crop cell; stock and harvest stay native."""
    capacity = max(workers, 1) if smoke else 10000
    if workers < 0 or workers > capacity:
        raise ValueError("productive worker count exceeds fixture capacity")
    groups, crops, refresh = [], [], []
    for start in range(0, capacity, 36):
        group, _, _ = add_dispenser_bank(recipe, 0, 0, "arrow")
        groups.append(group)
        region = recipe["regions"][-1]
        region["name"] = "productive-farmers"
        x, z = group % 32 * 256, group // 32 * 256
        n = min(36, capacity - start)
        residents = min(n, max(0, workers - start))
        recipe["coverage"][-1]["resident"] = residents
        recipe["coverage"][-1]["kind"] = "minecraft:villager" if residents else None
        for i in range(n):
            px, pz = x + (i % 6) * 10 + 1, z + (i // 6) * 10 + 1
            region["commands"] += [
                f"fill {px-1} 64 {pz-1} {px+3} 66 {pz-1} minecraft:stone",
                f"fill {px-1} 64 {pz+3} {px+3} 66 {pz+3} minecraft:stone",
                f"fill {px-1} 64 {pz} {px-1} 66 {pz+2} minecraft:stone",
                f"fill {px+3} 64 {pz} {px+3} 66 {pz+2} minecraft:stone",
                f"setblock {px+1} 64 {pz} minecraft:composter",
                f"setblock {px+1} 63 {pz+2} minecraft:farmland[moisture=7]",
                f"setblock {px+1} 64 {pz+2} minecraft:wheat[age=7]",
            ]
            crops.append([px+1, 64, pz+2])
            if i < residents:
                region["commands"].append(
                    f"summon minecraft:villager {px+.5} 64 {pz+1.5} "
                    f"{{CustomName:'{compare.COHORT_NAME}',PersistenceRequired:1b}}")
        refresh.append(f"fill {x} 64 {z} {x+63} 64 {z+63} minecraft:wheat[age=7] replace minecraft:wheat")
    recipe["regions"][-1]["commands"].append("time set 2000")
    old_residents = recipe["population"]["resident_requested"]
    recipe["expected"]["minecraft:villager"] = workers
    recipe["population"]["resident_requested"] += workers
    recipe["population"]["active_requested"] += workers
    recipe["variant"] = recipe["variant"].replace(
        f"noncanonical-scale-{old_residents}-",
        f"noncanonical-scale-{recipe['population']['resident_requested']}-", 1)
    recipe["stress"] = {"kind": "productive-npcs", "regions": groups,
                        "requested_workers": workers, "worker_cells": crops[:workers],
                        "pulse_interval_ms": max(1, 15000 // len(groups)), "commands": refresh}


def add_damage_fixture(recipe: dict[str, Any], victims: int, *, smoke: bool) -> None:
    """Generate real projectile damage against isolated, ordinary living targets."""
    capacity = max(victims, 1) if smoke else 100000
    if victims < 1 or victims > capacity or recipe["expected"]["minecraft:cow"]:
        raise ValueError("damage fixture requires exclusive cow targets within capacity")
    groups = []
    for start in range(0, capacity, 256):
        group, _, _ = add_dispenser_bank(recipe, 0, 0, "arrow")
        groups.append(group)
        region = recipe["regions"][-1]
        region["name"] = "projectile-damage-targets"
        x, z = group % 32 * 256, group // 32 * 256
        n = min(256, capacity - start)
        residents = min(n, max(0, victims - start))
        recipe["coverage"][-1]["resident"] = residents
        recipe["coverage"][-1]["kind"] = "minecraft:cow" if residents else None
        if group == 0:
            region["commands"][:0] = [
                "gamerule doMobSpawning false", "gamerule doWeatherCycle false",
                "gamerule randomTickSpeed 3", "gamerule doDaylightCycle true",
                "weather clear", "time set 2000", "difficulty easy", f"setworldspawn {x+32} 64 {z+32}",
                *[f"kill @e[type=minecraft:{kind}]" for kind in (*compare.MOB_KINDS, "villager")],
            ]
        for i in range(n):
            px, pz = x + (i % 16) * 4 + 1, z + (i // 16) * 4 + 1
            recipe.setdefault("expected_blocks", []).append({"position": [px, 63, pz], "block": "minecraft:dispenser",
                "item": "minecraft:arrow" if i < residents else "minecraft:air", "count": 64 if i < residents else 0})
            region["commands"] += [
                f"fill {px-1} 64 {pz-1} {px+1} 66 {pz-1} minecraft:stone",
                f"fill {px-1} 64 {pz+1} {px+1} 66 {pz+1} minecraft:stone",
                f"fill {px-1} 64 {pz} {px-1} 66 {pz} minecraft:stone",
                f"fill {px+1} 64 {pz} {px+1} 66 {pz} minecraft:stone",
                f"setblock {px} 63 {pz} minecraft:dispenser[facing=up,triggered=false]",
            ]
            if i < residents:
                region["commands"] += [
                    f"item replace block {px} 63 {pz} container.0 with minecraft:arrow 64",
                    f"summon minecraft:cow {px+.5} 64 {pz+.5} "
                    f"{{CustomName:'{compare.COHORT_NAME}',PersistenceRequired:1b,Health:100f}}",
                ]
    recipe["regions"][0]["commands"][:0] = [
        "gamerule minecraft:max_command_forks 200001",
        "gamerule minecraft:max_command_sequence_length 400000",
    ]
    old_residents = recipe["population"]["resident_requested"]
    recipe["expected"]["minecraft:cow"] = victims
    recipe["population"]["resident_requested"] += victims
    recipe["population"]["active_requested"] += victims
    recipe["variant"] = recipe["variant"].replace(
        f"noncanonical-scale-{old_residents}-",
        f"noncanonical-scale-{recipe['population']['resident_requested']}-", 1)
    selector = f"@e[type=minecraft:cow,nbt={{CustomName:'{compare.COHORT_NAME}'}}]"
    recipe["stress"] = {"kind": "damage", "regions": groups, "requested_victims": victims,
                        "target_initial_health": 100, "dispenser_capacity": capacity,
                        "pulse_interval_ms": 3000,
                        "commands": [f"execute as {selector} at @s run setblock ~1 ~-1 ~ minecraft:redstone_block",
                                     f"execute as {selector} at @s run setblock ~1 ~-1 ~ minecraft:grass_block"]}


def add_reactive_fixture(recipe: dict[str, Any], pairs: int) -> None:
    """Confined golem/zombie pairs trigger native, guaranteed thorns reactions."""
    if pairs < 1 or recipe["expected"]["minecraft:zombie"]:
        raise ValueError("reactive fixture requires exclusive zombie targets")
    groups = []
    for start in range(0, pairs, 64):
        group, _, _ = add_dispenser_bank(recipe, 0, 0, "arrow")
        groups.append(group)
        region = recipe["regions"][group]
        region["name"] = "reactive-pairs"
        x, z = group % 32 * 256, group // 32 * 256
        n = min(64, pairs-start)
        recipe["coverage"][group].update(resident=2*n, kind="minecraft:zombie")
        for i in range(n):
            px, pz = x+(i%8)*8+2, z+(i//8)*8+2
            region["commands"] += [
                f"fill {px-2} 64 {pz-2} {px+2} 67 {pz-2} minecraft:stone",
                f"fill {px-2} 64 {pz+2} {px+2} 67 {pz+2} minecraft:stone",
                f"fill {px-2} 64 {pz-1} {px-2} 67 {pz+1} minecraft:stone",
                f"fill {px+2} 64 {pz-1} {px+2} 67 {pz+1} minecraft:stone",
                f"fill {px-2} 68 {pz-2} {px+2} 68 {pz+2} minecraft:stone",
                f"summon minecraft:zombie {px-.5} 64 {pz+.5} "
                f"{{CustomName:'{compare.COHORT_NAME}',PersistenceRequired:1b,Health:100000f,"
                "equipment:{chest:{id:'minecraft:diamond_chestplate',components:{'minecraft:enchantments':{'minecraft:thorns':10}}}}}",
                f"summon minecraft:iron_golem {px+1.0} 64 {pz+.5} "
                f"{{CustomName:'{compare.COHORT_NAME}',PersistenceRequired:1b,Health:100000f}}",
            ]
    old = recipe["population"]["resident_requested"]
    recipe["expected"]["minecraft:zombie"] = pairs
    recipe["expected"]["minecraft:iron_golem"] = pairs
    recipe["population"]["resident_requested"] += 2*pairs
    recipe["population"]["active_requested"] += 2*pairs
    recipe["variant"] = recipe["variant"].replace(f"noncanonical-scale-{old}-",
        f"noncanonical-scale-{recipe['population']['resident_requested']}-", 1)
    recipe["stress"] = {"kind": "reactive-damage", "regions": groups,
                        "requested_pairs": pairs, "commands": []}


def add_natural_fixture(recipe: dict[str, Any], count: int, kind: str) -> None:
    """Confined exposed zombies or initially sheared sheep, summoned by operators."""
    if count < 0 or kind not in ("daylight-zombies", "wool-sheep"):
        raise ValueError("invalid natural cohort")
    animal = "zombie" if kind == "daylight-zombies" else "sheep"
    if recipe["expected"][f"minecraft:{animal}"]:
        raise ValueError("natural cohort species must be exclusive")
    groups = []
    for start in range(0, count, 256):
        group, _, _ = add_dispenser_bank(recipe, 0, 0, "arrow")
        groups.append(group)
        region = recipe["regions"][group]
        region["name"] = kind
        x, z = group % 32 * 256, group // 32 * 256
        n = min(256, count-start)
        recipe["coverage"][group].update(resident=n, kind=f"minecraft:{animal}")
        for i in range(n):
            px, pz = x + (i%16)*4 + 1, z + (i//16)*4 + 1
            region["commands"] += [
                f"fill {px-1} 64 {pz-1} {px+1} 66 {pz-1} minecraft:stone",
                f"fill {px-1} 64 {pz+1} {px+1} 66 {pz+1} minecraft:stone",
                f"fill {px-1} 64 {pz} {px-1} 66 {pz} minecraft:stone",
                f"fill {px+1} 64 {pz} {px+1} 66 {pz} minecraft:stone",
                f"fill {px} 65 {pz} {px} 319 {pz} minecraft:air",
            ]
            if kind == "wool-sheep":
                region["commands"].append(f"setblock {px} 63 {pz} minecraft:stone")
                extra = ",Sheared:1b"
            else:
                extra = ",Health:100000f,equipment:{}"
            region["commands"].append(
                f"summon minecraft:{animal} {px+.5} 64 {pz+.5} "
                f"{{CustomName:'{compare.COHORT_NAME}',PersistenceRequired:1b{extra}}}")
    recipe["regions"][-1]["commands"] += ["gamerule doDaylightCycle false", "time set 2000"]
    old_residents = recipe["population"]["resident_requested"]
    recipe["expected"][f"minecraft:{animal}"] = count
    recipe["population"]["resident_requested"] += count
    recipe["population"]["active_requested"] += count
    recipe["variant"] = recipe["variant"].replace(f"noncanonical-scale-{old_residents}-",
        f"noncanonical-scale-{recipe['population']['resident_requested']}-", 1)
    commands = []
    if count and kind == "wool-sheep":
        recipe["regions"][0]["commands"][:0] = [
            "gamerule minecraft:max_command_forks 200001",
            "gamerule minecraft:max_command_sequence_length 400000"]
        commands = [f"execute as @e[type=minecraft:sheep,nbt={{CustomName:'{compare.COHORT_NAME}'}}] at @s run setblock ~ ~-1 ~ minecraft:grass_block"]
    recipe["stress"] = {"kind": kind, "regions": groups, "requested_count": count,
                        "pulse_interval_ms": 3000, "commands": commands}


def validate_spawn_rules(snapshot: dict[str, Any], expected: dict[str, bool]) -> dict[str, bool]:
    checkpoint = snapshot.get("globals", {}).get("checkpoint", {})
    rules = checkpoint.get("payload", {}).get("gamerules") if isinstance(checkpoint, dict) else None
    if not isinstance(rules, dict):
        raise ValueError("missing saved spawn-rule state")
    observed = {}
    for key, wanted in expected.items():
        value = rules.get(key)
        if value not in ("true", "false") or (value == "true") != wanted:
            raise ValueError(f"saved spawn-rule {key} differs from request")
        observed[key] = value == "true"
    return observed


def validate_reactive_equipment(snapshot: dict[str, Any], expected: int) -> int:
    rows = snapshot.get("entities", {}).get("minecraft:zombie", {}).get("samples", [])
    rows = [row for row in rows if row.get("active") and row.get("payload", {}).get("CustomName") == compare.COHORT_NAME]
    if len(rows) != expected:
        raise ValueError("saved reactive zombie count differs from request")
    for row in rows:
        chest = row["payload"].get("equipment", {}).get("chest", {})
        levels = chest.get("components", {}).get("minecraft:enchantments", {})
        if chest.get("id") != "minecraft:diamond_chestplate" or levels.get("minecraft:thorns") != 10:
            raise ValueError("saved reactive target lacks requested thorns equipment")
    return len(rows)


def validate_reactive_samples(stress: dict[str, Any], samples: list[dict[str, Any]]) -> int:
    if not samples or any(type(row.get("damage_reactive")) is not int
                          or row["damage_reactive"] < 0 for row in samples):
        raise ValueError("missing reactive source-tick samples")
    total = sum(row["damage_reactive"] for row in samples)
    if total < stress["requested_pairs"]:
        raise ValueError("accepted reactive damage pieces below requested pairs")
    return total


def validate_natural_snapshot(stress: dict[str, Any], snapshot: dict[str, Any], *, before: bool) -> dict[str, int]:
    animal = "zombie" if stress["kind"] == "daylight-zombies" else "sheep"
    rows = snapshot.get("entities", {}).get(f"minecraft:{animal}", {}).get("samples", [])
    rows = [row for row in rows if row.get("active") and row.get("payload", {}).get("CustomName") == compare.COHORT_NAME]
    if len(rows) != stress["requested_count"]:
        raise ValueError("saved natural cohort count differs from request")
    key = "Fire" if animal == "zombie" else "Sheared"
    positive = sum(type(row["payload"].get(key)) is int and row["payload"][key] > 0 for row in rows)
    if (animal == "sheep" and before or animal == "zombie" and not before) and positive != len(rows):
        raise ValueError(f"saved {animal} {key} state differs from requested cohort")
    return {"saved_count": len(rows), "positive_state": positive}


def add_churn_fixture(recipe: dict[str, Any], count: int, percent: int) -> None:
    """Alternate native cows and pigs in confined cells via public commands."""
    if (count < 0 or percent not in (0, 1, 10) or recipe["expected"]["minecraft:cow"]
            or recipe["expected"]["minecraft:pig"]):
        raise ValueError("invalid churn cohort")
    groups = []
    for start in range(0, count, 256):
        group, _, _ = add_dispenser_bank(recipe, 0, 0, "arrow")
        groups.append(group)
        region = recipe["regions"][group]
        region["name"] = "churn-cells"
        x, z = group % 32 * 256, group // 32 * 256
        n = min(256, count - start)
        recipe["coverage"][group].update(resident=n, kind="minecraft:cow")
        for i in range(n):
            px, pz = x + (i % 16) * 4 + 1, z + (i // 16) * 4 + 1
            region["commands"] += [
                f"fill {px-1} 64 {pz-1} {px+1} 66 {pz-1} minecraft:stone",
                f"fill {px-1} 64 {pz+1} {px+1} 66 {pz+1} minecraft:stone",
                f"fill {px-1} 64 {pz} {px-1} 66 {pz} minecraft:stone",
                f"fill {px+1} 64 {pz} {px+1} 66 {pz} minecraft:stone",
                f"summon minecraft:cow {px+.5} 64 {pz+.5} {{CustomName:'{compare.COHORT_NAME}',PersistenceRequired:1b}}",
            ]
    recipe["regions"][0]["commands"][:0] = [
        "gamerule minecraft:max_command_forks 200001",
        "gamerule minecraft:max_command_sequence_length 400000"]
    recipe["expected"]["minecraft:cow"] = count
    old_residents = recipe["population"]["resident_requested"]
    recipe["population"]["resident_requested"] += count
    recipe["population"]["active_requested"] += count
    recipe["variant"] = recipe["variant"].replace(f"noncanonical-scale-{old_residents}-",
        f"noncanonical-scale-{recipe['population']['resident_requested']}-", 1)
    marker = compare.COHORT_NAME
    commands = [] if not count else [
        f"execute as @e[type=minecraft:cow,nbt={{CustomName:'{marker}'}}] at @s run summon minecraft:pig ~ ~ ~ {{CustomName:'{marker}',PersistenceRequired:1b}}",
        f"kill @e[type=minecraft:cow,nbt={{CustomName:'{marker}'}}]",
        f"execute as @e[type=minecraft:pig,nbt={{CustomName:'{marker}'}}] at @s run summon minecraft:cow ~ ~ ~ {{CustomName:'{marker}',PersistenceRequired:1b}}",
        f"kill @e[type=minecraft:pig,nbt={{CustomName:'{marker}'}}]",
    ]
    recipe["stress"] = {"kind": "churn", "regions": groups, "requested_count": count,
                        "requested_percent": percent, "pulse_interval_ms": 500, "commands": commands}


def add_map_frame_fixture(recipe: dict[str, Any], frames: int, *, smoke: bool) -> None:
    """Place real filled maps in frames through the normal protocol player."""
    capacity = max(frames, 1) if smoke else 100000
    if frames < 0 or frames > capacity:
        raise ValueError("map-frame count exceeds fixture capacity")
    groups = []
    for start in range(0, capacity, 1000):
        group, _, _ = add_dispenser_bank(recipe, 0, 0, "arrow")
        groups.append(group)
        region = recipe["regions"][-1]
        region["name"] = "map-frames"
        x, z = group % 32 * 256, group // 32 * 256
        residents = min(1000, max(0, frames - start))
        recipe["coverage"][-1]["resident"] = residents
        recipe["coverage"][-1]["kind"] = "minecraft:item_frame" if residents else None
        if group == 0:
            region["commands"][:0] = [
                "gamerule doMobSpawning false", "gamerule doWeatherCycle false",
                "gamerule randomTickSpeed 3", "gamerule doDaylightCycle true",
                "weather clear", "time set 2000", "difficulty easy", f"setworldspawn {x+32} 64 {z+32}",
                *[f"kill @e[type=minecraft:{kind}]" for kind in (*compare.MOB_KINDS, "villager")],
            ]
        if residents:
            positions = [[x + (i % 32) * 2, 65, z + (i // 32) * 2] for i in range(residents)]
            region["commands"] += [
                f"summon minecraft:item_frame {px+.5} {py} {pz+.5} "
                f"{{CustomName:'{compare.COHORT_NAME}'}}" for px, py, pz in positions]
            region["frame_positions"] = positions
    old_residents = recipe["population"]["resident_requested"]
    recipe["expected"]["minecraft:item_frame"] = frames
    recipe["population"]["resident_requested"] += frames
    recipe["population"]["active_requested"] += frames
    recipe["variant"] = recipe["variant"].replace(
        f"noncanonical-scale-{old_residents}-",
        f"noncanonical-scale-{recipe['population']['resident_requested']}-", 1)
    recipe["stress"] = {"kind": "map-frames", "regions": groups, "requested_frames": frames}


def validate_map_frames(snapshot: dict[str, Any], expected: int) -> dict[str, int]:
    """Check frame custody against records decoded by the native recovery owner."""
    frames = snapshot.get("entities", {}).get("minecraft:item_frame", {}).get("samples", [])
    rows = [row for row in frames if row.get("active")
            and row.get("payload", {}).get("CustomName") == compare.COHORT_NAME]
    globals_ = snapshot.get("globals", {})
    if len(rows) != expected or not isinstance(globals_, dict):
        raise ValueError("saved map-frame identities or map records differ from request")
    identities: dict[str, int] = {}
    counts: dict[int, int] = {}
    for row in rows:
        item = row["payload"].get("Item", {})
        components = item.get("components", {}) if isinstance(item, dict) else {}
        map_id = components.get("minecraft:map_id") if isinstance(components, dict) else None
        record = globals_.get(f"map/{map_id}")
        identity = row.get("uuid")
        if (not isinstance(identity, str) or not identity or identity in identities
                or not isinstance(item, dict) or item.get("id") != "minecraft:filled_map"
                or item.get("count", item.get("Count")) != 1
                or type(map_id) is not int or map_id < 0
                or not isinstance(record, dict) or record.get("version") != 1
                or not isinstance(record.get("payload"), dict) or not record["payload"]):
            raise ValueError("saved frame is missing its identity, filled map or map record")
        identities[identity] = map_id
        counts[map_id] = counts.get(map_id, 0) + 1
        if counts[map_id] > 256:
            raise ValueError("saved map is assigned to more than 256 frames")
    return identities


def validate_churn_cohort(recipe: dict[str, Any], before: dict[str, Any], snapshot: dict[str, Any]) -> tuple[dict[str, Any], dict[str, int]]:
    count = recipe["stress"]["requested_count"]
    rows = compare.entities(snapshot)
    churn = {key: row for key, row in rows.items() if row["custom_name"] == compare.COHORT_NAME
             and row["kind"] in ("minecraft:cow", "minecraft:pig")}
    if len(churn) != count:
        raise ValueError("saved churn count differs from requested cohort")
    counts = dict(recipe["expected"])
    counts["minecraft:cow"] = sum(row["kind"] == "minecraft:cow" for row in churn.values())
    counts["minecraft:pig"] = sum(row["kind"] == "minecraft:pig" for row in churn.values())
    after = compare.cohort(snapshot, counts)
    old_churn = {key for key, row in before["identities"].items()
                 if row["kind"] in ("minecraft:cow", "minecraft:pig")}
    if len(old_churn) != count or (count and old_churn.intersection(churn)):
        raise ValueError("saved churn identities did not all turn over")
    stable_expected = dict(counts, **{"minecraft:cow": 0, "minecraft:pig": 0})
    def stable(cohort):
        return {"counts": stable_expected, "identities": {key: row for key, row in cohort["identities"].items()
                if row["kind"] not in ("minecraft:cow", "minecraft:pig")}}
    validate_cohort(stable(before), stable(after), stable_expected)
    return after, {"before": len(old_churn), "after": len(churn), "replaced_min": count}


def fixture_block_spec(recipe: dict[str, Any]) -> dict[str, Any]:
    rows = recipe.get("expected_blocks", [])
    positions = [tuple(row["position"]) for row in rows]
    if len(set(positions)) != len(positions):
        raise ValueError("fixture block positions are duplicated")
    containers = [row for row in rows if row["block"] == "minecraft:dispenser"]
    return {"entities": [], "blocks": [
        {"name": f"fixture-{i}", "position": row["position"]} for i, row in enumerate(rows)],
        "containers": [{"name": f"fixture-container-{i}", "position": row["position"], "slots": 9}
                       for i, row in enumerate(containers)]}


def validate_fixture_blocks(recipe: dict[str, Any], snapshot: dict[str, Any]) -> dict[str, Any]:
    expected = recipe.get("expected_blocks", [])
    blocks = snapshot.get("blocks")
    if not isinstance(blocks, dict) or len(blocks) != len(expected):
        raise ValueError("saved fixture block count differs from recipe")
    counts: dict[str, int] = {}
    for i, row in enumerate(expected):
        observed = blocks.get(f"fixture-{i}")
        if (not isinstance(observed, dict) or observed.get("position") != row["position"]
                or observed.get("final", {}).get("name") != row["block"]):
            raise ValueError(f"saved fixture block {i} differs from recipe")
        counts[row["block"]] = counts.get(row["block"], 0) + 1
    expected_containers = [row for row in expected if row["block"] == "minecraft:dispenser"]
    containers = snapshot.get("containers", {})
    if not isinstance(containers, dict) or len(containers) != len(expected_containers):
        raise ValueError("saved fixture container count differs from recipe")
    loaded_sources: dict[str, int] = {}
    for i, row in enumerate(expected_containers):
        observed = containers.get(f"fixture-container-{i}")
        slots = observed.get("final") if isinstance(observed, dict) else None
        if (not isinstance(observed, dict) or observed.get("position") != row["position"]
                or not isinstance(slots, list) or len(slots) != 9
                or slots[0].get("name") != row["item"] or slots[0].get("count") != row["count"]
                or any(slot.get("name") != "minecraft:air" or slot.get("count") != 0 for slot in slots[1:])):
            raise ValueError(f"saved fixture ammunition {i} differs from recipe")
        if slots[0]["count"]:
            item = slots[0]["name"]
            loaded_sources[item] = loaded_sources.get(item, 0) + 1
    return {"blocks": counts, "loaded_sources": loaded_sources}


def validate_scale_sample(recipe: dict[str, Any], sample: dict[str, Any],
                          allowed_cells: set[tuple[int, int, int]] | None = None) -> dict[str, Any]:
    """Fail closed on partial region coverage, stale source ticks or cohort mismatch."""
    data = sample.get("coreperf")
    if (type(sample.get("tick")) is not int or not isinstance(data, dict)
            or data.get("tick") != sample["tick"]):
        raise ValueError("missing or mismatched source-tick region telemetry")
    resident_rows = data.get("resident_entities")
    if type(resident_rows) is not int or resident_rows < recipe["population"]["resident_requested"]:
        raise ValueError("live resident rows are below the requested fixture population")
    rows = data.get("regions")
    if not isinstance(rows, list):
        raise ValueError("missing region coverage")
    by_region: dict[int, dict[str, Any]] = {}
    for row in rows:
        index = row.get("region")
        if not isinstance(index, int) or index < 0 or index >= len(recipe["coverage"]) or index in by_region:
            raise ValueError("duplicate or invalid region coverage")
        by_region[index] = row
    totals = {"tick": sample["tick"], "resident_rows": resident_rows, "active": 0, "admitted": 0, "full": 0,
              "resting": 0, "active_chunks": 0}
    stress = recipe.get("stress", {})
    stress_regions = set(stress.get("regions", []))
    if stress_regions or stress.get("kind") == "xp-orbs":
        totals["projectiles"] = 0
    if stress.get("kind") == "xp-orbs":
        xp = data.get("xp_orbs_processed")
        regions = data.get("xp_orbs_by_region")
        if (type(xp) is not int or xp < 0 or not isinstance(regions, dict)
                or any(type(value) is not int or value < 0 for value in regions.values())):
            raise ValueError("missing XP-orb owner telemetry")
        if not stress_regions and xp:
            raise ValueError("zero XP control observed active orbs")
        totals["xp_orbs"] = sum(regions.get(str(i), 0) for i in stress_regions)
        totals["xp_orbs_global"] = xp
    if stress.get("kind") in ("damage", "reactive-damage"):
        victims = data.get("damage_victims_by_region")
        reactive = data.get("damage_reactive_by_region")
        if not isinstance(victims, dict) or not isinstance(reactive, dict):
            raise ValueError("missing accepted damage owner telemetry")
        if any(type(value) is not int or value < 0 for value in (*victims.values(), *reactive.values())):
            raise ValueError("invalid accepted damage counters")
        totals["damage_victims"] = sum(victims.get(str(i), 0) for i in stress_regions)
        totals["damage_reactive"] = sum(reactive.get(str(i), 0) for i in stress_regions)
    if stress.get("kind") == "brewing-containers":
        discovery = data.get("brewing_discovery_by_region")
        active = data.get("brewing_active_by_region")
        progressing = data.get("brewing_progressing_by_region")
        if not all(isinstance(value, dict) for value in (discovery, active, progressing)):
            raise ValueError("missing brewing owner telemetry")
        if any(type(value) is not int or value < 0 for counters in (discovery, active, progressing)
               for value in counters.values()):
            raise ValueError("invalid brewing owner counters")
        for region, required in stress["discovery_per_region"].items():
            if type(discovery.get(region)) is not int or discovery[region] < required:
                raise ValueError(f"brewing discovery region {region} below requested containers")
        totals["brewing_discovered"] = sum(discovery.get(str(i), 0) for i in stress_regions)
        totals["brewing_active"] = sum(active.get(str(i), 0) for i in stress_regions)
        totals["brewing_progressing"] = sum(progressing.get(str(i), 0) for i in stress_regions)
        if totals["brewing_active"] < stress["requested_stands"]:
            raise ValueError("brewing owner did not visit every requested active stand")
    if stress.get("kind") == "productive-npcs":
        brain_candidates = data.get("brain_candidates_by_region")
        brain_admitted = data.get("brain_admitted_by_region")
        if not isinstance(brain_candidates, dict) or not isinstance(brain_admitted, dict):
            raise ValueError("missing villager brain candidate/admission telemetry")
        totals["brain_candidates"] = 0
        totals["brain_admitted"] = 0
        cells = data.get("productive_harvested_cells")
        if not isinstance(cells, list) or any(
                not isinstance(cell, list) or len(cell) != 3 or any(type(v) is not int for v in cell)
                for cell in cells):
            raise ValueError("missing productive farmer work telemetry")
        allowed = allowed_cells if allowed_cells is not None else {tuple(cell) for cell in stress["worker_cells"]}
        if any(tuple(cell) not in allowed for cell in cells):
            raise ValueError("productive farmer work outside declared worker cells")
        totals["harvested_cells"] = cells
    for index, expected in enumerate(recipe["coverage"]):
        row = by_region.get(index)
        actual = {}
        for key in ("active_chunks", "motion_admitted", "motion_full", "motion_resting"):
            value = 0 if row is None else row.get(key)
            if type(value) is not int or value < 0:
                raise ValueError("invalid or missing region counter")
            actual[key] = value
        row = row or {}
        required = expected["resident"] if expected["active"] else 0
        if expected.get("kind") == "minecraft:villager":
            candidate = brain_candidates.get(str(index), 0)
            admitted = brain_admitted.get(str(index), 0)
            if (type(candidate) is not int or type(admitted) is not int
                    or candidate != required or admitted < 0 or admitted > candidate):
                raise ValueError(f"region {index} villager brain candidate/admission differs")
            if actual["motion_admitted"] != 0:
                raise ValueError("villager brain rows were incorrectly counted as motion admissions")
            totals["brain_candidates"] += candidate
            totals["brain_admitted"] += admitted
        else:
            candidate = admitted = actual["motion_admitted"]
            if (stress.get("kind") == "churn" and index in stress_regions):
                if not required <= candidate <= 2 * required:
                    raise ValueError(f"region {index} churn cohort outside expected transition bounds")
            elif candidate != required:
                raise ValueError(f"region {index} admitted {candidate}, expected {required}")
        if actual["active_chunks"] != (len(expected["chunks"]) if expected["active"] else 0):
            raise ValueError(f"region {index} active chunk coverage differs from recipe")
        if actual["motion_full"] + actual["motion_resting"] != actual["motion_admitted"]:
            raise ValueError(f"region {index} motion full/resting partition differs from admissions")
        totals["active"] += candidate
        totals["admitted"] += admitted
        totals["full"] += actual["motion_full"]
        totals["resting"] += actual["motion_resting"]
        totals["active_chunks"] += actual["active_chunks"]
        if index in stress_regions:
            projectiles = row.get("projectile_candidates")
            if type(projectiles) is not int or projectiles < 0:
                raise ValueError("missing projectile owner candidates")
            totals["projectiles"] += projectiles
    expected_active = recipe["population"]["active_requested"]
    churn_extra = stress.get("requested_count", 0) if stress.get("kind") == "churn" else 0
    if not expected_active <= totals["active"] <= expected_active + churn_extra:
        raise ValueError("observed active cohort differs from advertised population")
    return totals


def exact_cohort_tick(samples: list[dict[str, Any]], field: str, requested: int) -> int:
    if not samples or any(type(row.get("tick")) is not int or type(row.get(field)) is not int
                          or row[field] < 0 for row in samples):
        raise ValueError(f"missing {field} source-tick samples")
    if requested == 0:
        if any(row[field] != 0 for row in samples):
            raise ValueError(f"{field} zero control observed a nonzero cohort")
        return samples[0]["tick"]
    for row in samples:
        if row[field] == requested:
            return row["tick"]
    raise ValueError(f"no source tick observed exactly {requested} {field}")


def validate_xp_observers(stress: dict[str, Any], bots: dict[str, Any]) -> dict[str, Any]:
    players = bots.get("mechanics", {}).get("players")
    anchors = stress["anchors"]
    if not isinstance(players, list) or len(players) != stress["players"] or len(anchors) != stress["players"]:
        raise ValueError("missing XP observer locality evidence")
    centers = []
    for player, anchor in zip(players, anchors):
        center = [anchor[0] // 16, anchor[1] // 16]
        if (player.get("initial_chunk_center") != center
                or player.get("final_chunk_center") != center
                or player.get("chunk_center_transitions") != 0):
            raise ValueError("XP observer left requested chunk or locality evidence is missing")
        centers.append(center)
    return {"players": len(players), "locality": stress["locality"], "centers": centers,
            "scope": "received chunk-center evidence, not exact player positions"}


def visibility_summary(samples: list[dict[str, Any]], rows: list[dict[str, Any]],
                       start: int, end: int) -> dict[str, Any]:
    ticks = {sample["tick"] for sample in samples if start <= sample["wall_start_unix_ns"] < end}
    observations = []
    for row in rows:
        visibility = row["stats"].get("coreperf_visibility")
        if not isinstance(visibility, dict) or visibility.get("source_tick") not in ticks:
            continue
        pairs = visibility.get("tracked_entity_viewer_pairs")
        if not isinstance(pairs, int) or pairs < 0:
            raise ValueError("invalid edge visibility observation")
        observations.append((visibility["source_tick"], pairs))
    if not observations:
        raise ValueError("no edge visibility observation matches a measured source tick")
    return {"tracked_entity_viewer_pairs_min": min(pairs for _, pairs in observations),
            "tracked_entity_viewer_pairs_max": max(pairs for _, pairs in observations),
            "source_ticks": [tick for tick, _ in observations],
            "scope": "edge planned tracked entity-viewer pairs; includes nonfixture entities, not distinct or delivered entities"}


def schedule(args: argparse.Namespace, recipe: dict[str, Any] | None = None) -> dict[str, Any]:
    active = args.profile != "idle"
    plan = {"windows": [{"name": name, "duration_ms": round(seconds * 1000),
                         "movement_hz": 20 if active else 0,
                         "edit_hz": 2 if args.mobs else 0,
                         "chat_hz": 1 if args.mobs else 0}
                        for name, seconds in (("warmup", args.warmup_seconds),
                                              ("measurement", args.measure_seconds))]}
    if recipe and recipe.get("stress", {}).get("kind") == "xp-orbs":
        for window in plan["windows"]:
            window["edit_hz"] = 0
            window["chat_hz"] = 0
    if recipe and recipe.get("stress", {}).get("commands"):
        for window in plan["windows"]:
            window["command_interval_ms"] = recipe["stress"]["pulse_interval_ms"]
            window["commands"] = recipe["stress"]["commands"]
    return plan


def host_receipt(out: Path) -> dict[str, Any]:
    def capture(command: list[str]) -> str:
        return subprocess.check_output(command, text=True).strip()
    return {"cpu": json.loads(capture(["lscpu", "--json"])),
            "memory": Path("/proc/meminfo").read_text(), "os": platform.freedesktop_os_release(),
            "kernel": platform.uname()._asdict(),
            "disks": json.loads(capture(["lsblk", "--json", "-o", "NAME,MODEL,SIZE,ROTA,FSTYPE,MOUNTPOINTS"])),
            "artifact_filesystem": capture(["findmnt", "-T", str(out), "-o", "SOURCE,FSTYPE,OPTIONS"]),
            "cpu_affinity": sorted(os.sched_getaffinity(0)),
            "loadavg": Path("/proc/loadavg").read_text().strip(),
            "cgroup": Path("/proc/self/cgroup").read_text()}


def summarize(samples: list[dict[str, Any]], processes: list[dict[str, Any]],
              start: int, end: int) -> dict[str, Any]:
    ticks = [s for s in samples if start <= s["wall_start_unix_ns"] < end]
    rows = [r for r in processes if start <= r["captured_unix_ns"] < end]
    if not ticks or not rows:
        raise ValueError("measurement lacks exact ticks or process samples")
    seconds = (end - start) / 1e9
    tick = load.tick_metrics(ticks)
    return {"duration_seconds": seconds, "ticks": len(ticks), "tps": len(ticks) / seconds,
            "mspt": {key: tick[key + "_us"] / 1000 for key in ("p50", "p95", "p99", "max")},
            "rss": {name: {"peak_lifetime_bytes": max(r["processes"][name]["peak_rss_bytes"] for r in rows),
                           "peak_sampled_bytes": max(r["processes"][name]["rss_bytes"] for r in rows),
                           "steady_median_bytes": statistics.median(r["processes"][name]["rss_bytes"] for r in rows)}
                    for name in rows[0]["processes"]},
            "chunk_generation_per_second": None,
            "chunk_generation_scope": "not instrumented; resident chunk deltas include reloads/unloads",
            "population_observed": {"first": rows[0]["stats"]["entities"], "last": rows[-1]["stats"]["entities"]},
            "resident_chunks_observed": {"first": rows[0]["stats"]["chunks"], "last": rows[-1]["stats"]["chunks"]}}


def validate_metrics(metric: dict[str, Any], initial: dict[str, Any], cursor: int) -> int:
    if (metric.get("schema_version") != 1 or metric.get("overrun") or
            metric.get("instance_started_unix_ns") != initial.get("instance_started_unix_ns") or
            metric.get("source_skipped_ticks_total") != initial.get("source_skipped_ticks_total")):
        raise ValueError("exact tick stream overwritten, reset or lost source samples")
    if not isinstance(metric.get("samples"), list):
        raise ValueError("exact source tick samples missing")
    for row in metric["samples"]:
        if row["tick"] != cursor + 1:
            raise ValueError("noncontiguous exact tick stream")
        if (not isinstance(row.get("wall_start_unix_ns"), int) or row["wall_start_unix_ns"] <= 0
                or not isinstance(row.get("wall_us"), int) or row["wall_us"] < 0):
            raise ValueError("exact tick lacks a valid wall start/duration")
        cursor = row["tick"]
    return cursor


async def initial_metrics(session: Any, url: str) -> dict[str, Any]:
    initial = await session.http(url + "/load-metrics?wait=true&min_samples=1", 12)
    if initial.get("schema_version") != 1 or initial.get("latest_tick") is None:
        raise ValueError("exact source tick telemetry unavailable")
    return initial


class ColdVisibilityUnavailable(ValueError):
    """The bounded cold capture could not establish the requested proof."""

    def __init__(self, proof: dict[str, Any]):
        self.proof = proof
        pairs, limit = proof["tracked_entity_viewer_pairs"], proof["pair_limit"]
        super().__init__(f"cold visibility proof unavailable: requires {pairs} pair visits, limit {limit}; "
                         f"up to {pairs} distinct-ID hash entries, allocation size implementation-dependent")


async def cold_visibility_proof(session: Any, url: str, viewers: int, after_tick: int,
                                timeout: float) -> dict[str, Any]:
    """Request a bounded one-shot tracked-ID union before measured workload begins."""
    proof = await session.http(url + "/coreperf-visibility-proof", timeout,
                               bearer_token=session.args.metrics_bearer_token)
    if not isinstance(proof, dict):
        raise ValueError("cold visibility proof response is not an object")
    tick, count = proof.get("source_tick"), proof.get("viewers")
    pairs, unique, limit = (proof.get("tracked_entity_viewer_pairs"),
                            proof.get("unique_tracked_entities"), proof.get("pair_limit"))
    if type(tick) is not int or tick <= after_tick or count != viewers:
        raise ValueError("cold visibility proof has wrong source tick or incomplete viewers")
    if type(pairs) is not int or type(limit) is not int or pairs < 0 or limit < 0:
        raise ValueError("cold visibility proof has invalid pair bound")
    if pairs > limit:
        raise ColdVisibilityUnavailable(proof)
    if type(unique) is not int or not 0 <= unique <= pairs:
        raise ValueError("cold visibility proof unique count invalid")
    return proof


async def measure(args: argparse.Namespace, directory: Path, world: Path,
                  recipe: dict[str, Any], prepared: dict[str, Any]) -> dict[str, Any]:
    directory.mkdir()
    if "coverage" in recipe:
        if getattr(args, "remote_host", None):
            raise ValueError("scale telemetry currently requires a local server")
        region_file = directory / "coreperf-regions.json"
        load.scale.dump(region_file, {"marker": compare.COHORT_NAME,
                                      "regions": [row["chunks"] for row in recipe["coverage"]],
                                      "productive_crops": recipe.get("stress", {}).get("worker_cells", [])})
        args = copy.copy(args)
        args.metrics_bearer_token = secrets.token_urlsafe(32)
        args.server_wrapper = " ".join(filter(None, [
            args.server_wrapper,
            "env", shlex.quote(f"SOLARIS_COREPERF_REGIONS={region_file}")]))
    config, url, game = load.config(args, directory, world, args.clients, operators=True)
    # Idle measures simulation, not the default pause-when-empty policy.
    config.write_text(config.read_text().replace("[server]\n", "[server]\npause_when_empty_seconds = -1\n", 1))
    if getattr(args, "remote_host", None):
        from tools.bench.remote_session import RemoteSession  # imported lazily: only vps runs need it
        session = RemoteSession(args, directory, config, world)
    else:
        session = compare.CoreSession(args, directory, config)
    plan = schedule(args, recipe)
    load.scale.dump(directory / "schedule.json", plan)
    result: dict[str, Any] = {"status": "failed", "schedule": plan, "config": str(config),
                              "timing_scope": "exact active-iteration wall_us, excluding pacing sleep"}
    samples: list[dict[str, Any]] = []
    process_rows: list[dict[str, Any]] = []
    coverage_samples: list[dict[str, Any]] = []
    allowed_cells = {tuple(cell) for cell in recipe.get("stress", {}).get("worker_cells", [])}
    try:
        await session.start_server()
        if hasattr(session, "wait_ready"):
            await session.wait_ready(url, args.readiness_seconds)
        else:
            await load.wait_dashboard(session, url, args.readiness_seconds)
        if args.clients:
            command = ["taskset", "-c", args.bot_cpus,
                       *load.bot_command(args, directory, game, args.clients, {"group_size": 1}),
                       "--open-loop", "--schedule-file", str(directory / "schedule.json")]
            if args.mobs:
                anchors = (recipe["stress"]["anchors"] if recipe.get("stress", {}).get("kind") == "xp-orbs" else
                           [region["anchor"] for index, region in enumerate(recipe["regions"])
                            if "coverage" not in recipe or recipe["coverage"][index]["active"]])
                load.scale.dump(directory / "anchors.json", [anchors[n % len(anchors)] for n in range(args.clients)])
                command += ["--anchors", str(directory / "anchors.json")]
                if recipe.get("stress", {}).get("kind") == "xp-orbs":
                    load.scale.dump(directory / "xp-observers.json", {"schema_version": 1,
                                    "entities": [], "blocks": [], "containers": [], "actions": []})
                    command += ["--anchor-chunk-walk", "--no-edits",
                                "--mechanic-spec", str(directory / "xp-observers.json")]
            else:
                command += ["--explore", "--no-edits"]
            if hasattr(session, "bot_command"):  # remote server: bots dial the VPS, not loopback
                command = session.bot_command(command)
            result["bot_command"] = command
            await session.spawn("bots", command, events=True)
            result["ready"] = await session.ready(args.clients, args.readiness_seconds)
        initial = await initial_metrics(session, url)
        cursor = initial["latest_tick"]
        proof = (await cold_visibility_proof(session, url, args.clients, cursor, args.readiness_seconds)
                 if "coverage" in recipe else None)
        epoch = time.time_ns()
        if args.clients:
            stdin = session.processes["bots"].stdin
            assert stdin
            stdin.write(b'{"event":"run"}\n')
            await stdin.drain()
            running = await load._event(session, "running", args.readiness_seconds)
            epoch = running["open_loop"]["epoch_unix_ns"]
        # Tick timestamps are on the server's clock; a remote session reports its offset from ours.
        offset = getattr(session, "clock_offset_ns", 0)
        start = epoch + offset + round(args.warmup_seconds * 1e9)
        end = start + round(args.measure_seconds * 1e9)
        result.update(epoch_unix_ns=epoch, start_unix_ns=start, end_unix_ns=end)
        deadline = time.monotonic() + args.warmup_seconds + args.measure_seconds + args.readiness_seconds
        previous_wall_start = 0
        with (directory / "samples.jsonl").open("w", buffering=1) as raw:
            while True:
                session.alive()
                if time.monotonic() >= deadline:
                    raise ValueError("measurement completion deadline expired")
                metric = await session.http(url + f"/load-metrics?after_tick={cursor}&wait=true&min_samples=20", 12)
                cursor = validate_metrics(metric, initial, cursor)
                for tick in metric["samples"]:
                    if tick["wall_start_unix_ns"] <= previous_wall_start:
                        raise ValueError("source wall clock regressed")
                    previous_wall_start = tick["wall_start_unix_ns"]
                    if "coverage" in recipe and start <= tick["wall_start_unix_ns"] < end:
                        coverage_samples.append(validate_scale_sample(recipe, tick, allowed_cells))
                stats = await session.http(url + "/stats", 2)
                row = {"captured_unix_ns": time.time_ns() + offset, "processes": session.process_samples(),
                       "stats": stats, "bots": session.latest_bot}
                raw.write(json.dumps({"metrics": metric, **row}) + "\n")
                samples.extend(metric["samples"])
                process_rows.append(row)
                if stats["players"]["count"] != args.clients:
                    raise ValueError("live player count differs from requested profile")
                if any(p["rss_bytes"] > args.max_rss_mb * 2**20 for p in row["processes"].values()):
                    raise ValueError("process RSS exceeded configured memory guard")
                if metric["samples"] and metric["samples"][-1]["wall_start_unix_ns"] >= end:
                    break
        result["summary"] = summarize(samples, process_rows, start, end)
        if "coverage" in recipe:
            if not coverage_samples:
                raise ValueError("measurement lacks source-tick region coverage")
            if (recipe["variant"].endswith("-mostly-resting")
                    and sum(row["resting"] for row in coverage_samples)
                    <= sum(row["full"] for row in coverage_samples)):
                raise ValueError("mostly-resting recipe did not observe a majority of resting motion rows")
            visible = visibility_summary(samples, process_rows, start, end)
            if proof is None or not any(sample["tick"] == proof["source_tick"]
                                        and sample["wall_start_unix_ns"] < epoch + offset for sample in samples):
                raise ValueError("cold visibility proof lacks its exact pre-warmup core source-tick sample")
            result["population"] = {**recipe["population"],
                                    "resident_all_entities_min": min(row["resident_rows"] for row in coverage_samples),
                                    "resident_observed_saved": sum(prepared["cohort"]["counts"].values()),
                                    "active_observed_min": min(row["active"] for row in coverage_samples),
                                    "admitted_observed_min": min(row["admitted"] for row in coverage_samples),
                                    "motion_full_min": min(row["full"] for row in coverage_samples),
                                    "motion_resting_min": min(row["resting"] for row in coverage_samples),
                                    "active_chunks_min": min(row["active_chunks"] for row in coverage_samples),
                                    "visible_unique_entities": proof["unique_tracked_entities"],
                                    "visible_unique_scope": "bounded pre-warmup edge tracked-ID union, not measured-window visibility or delivery",
                                    "visible_unique_proof": proof,
                                    "visibility": visible,
                                    "source_tick_samples": len(coverage_samples)}
            if recipe.get("stress", {}).get("kind") == "projectiles":
                peak = max(row["projectiles"] for row in coverage_samples)
                requested = recipe["stress"]["requested_count"]
                matching_tick = exact_cohort_tick(coverage_samples, "projectiles", requested)
                result["stress"] = {**recipe["stress"], "projectile_candidates_peak": peak,
                                    "requested_cohort_tick": matching_tick,
                                    "scope": "exact source-tick flight candidates, not sustained concurrency"}
            elif recipe.get("stress", {}).get("kind") == "xp-orbs":
                peak = max(row["xp_orbs"] for row in coverage_samples)
                projectile_peak = max(row["projectiles"] for row in coverage_samples)
                requested = recipe["stress"]["requested_count"]
                matching_tick = exact_cohort_tick(coverage_samples, "xp_orbs", requested)
                result["stress"] = {**recipe["stress"], "xp_orbs_processed_peak": peak,
                                    "requested_cohort_tick": matching_tick,
                                    "all_active_orbs_peak": max(row["xp_orbs_global"] for row in coverage_samples),
                                    "projectile_candidates_peak": projectile_peak,
                                    "scope": "source-tick active owner bodies, not sustained concurrency"}
            elif recipe.get("stress", {}).get("kind") == "brewing-containers":
                progressing = max(row["brewing_progressing"] for row in coverage_samples)
                if progressing < recipe["stress"]["requested_stands"]:
                    raise ValueError("brewing owner did not advance every requested stand in one tick")
                result["stress"] = {**recipe["stress"],
                                    "discovered_min": min(row["brewing_discovered"] for row in coverage_samples),
                                    "active_stands_min": min(row["brewing_active"] for row in coverage_samples),
                                    "progressing_stands_peak": progressing}
            elif recipe.get("stress", {}).get("kind") == "productive-npcs":
                cells = {tuple(cell) for row in coverage_samples for cell in row["harvested_cells"]}
                requested = recipe["stress"]["requested_workers"]
                if len(cells) != requested:
                    raise ValueError(f"observed {len(cells)} productive worker cells, expected {requested}")
                result["stress"] = {**recipe["stress"], "productive_worker_cells_observed": len(cells),
                                    "brain_candidates_min": min(row["brain_candidates"] for row in coverage_samples),
                                    "brain_admitted_min": min(row["brain_admitted"] for row in coverage_samples),
                                    "scope": "distinct isolated farmer crop cells changed in villager brain"}
            elif recipe.get("stress", {}).get("kind") == "reactive-damage":
                reactive = validate_reactive_samples(recipe["stress"], coverage_samples)
                result["stress"] = {**recipe["stress"], "accepted_reactive_pieces_total": reactive,
                                    "scope": "accepted marked reactive pieces across source ticks"}
            elif recipe.get("stress", {}).get("kind") == "damage":
                peak = max(row["damage_victims"] for row in coverage_samples)
                if peak < recipe["stress"]["requested_victims"]:
                    raise ValueError(f"accepted distinct damage victims peak {peak} below requested")
                result["stress"] = {**recipe["stress"], "accepted_distinct_victims_peak": peak,
                                    "accepted_reactive_pieces_total": sum(row["damage_reactive"] for row in coverage_samples),
                                    "scope": "accepted grouped victims in one source tick"}
    finally:
        await session.stop()
        result["shutdown"] = session.shutdown
        load.scale.dump(directory / "result.json", result)
    compare.clean_shutdown(session)
    if not args.clients and recipe.get("stress", {}).get("kind") == "xp-orbs":
        result["stress"]["observer_evidence"] = {"players": 0, "locality": recipe["stress"]["locality"],
                                                  "scope": "zero live players in sampled dashboard batches"}
    if args.clients:
        bots = json.loads((directory / "bots-report.json").read_text())
        result["bots"] = bots
        if bots.get("success") is not True or not bots.get("open_loop", {}).get("schedule_complete"):
            raise ValueError("bot schedule failed or incomplete")
        if bots.get("movements_sent", 0) == 0:
            raise ValueError("requested active profile sent no movement")
        if recipe.get("stress", {}).get("kind") == "xp-orbs":
            result["stress"]["observer_evidence"] = validate_xp_observers(recipe["stress"], bots)
        if args.profile in ("explore-3", "explore-20") and bots.get("chunk_boundaries_crossed", 0) == 0:
            raise ValueError("exploration did not cross a chunk boundary")
        if recipe.get("stress", {}).get("commands"):
            windows = bots.get("open_loop", {}).get("windows", [])
            if (not windows or any(row.get("command", {}).get("writes_completed") !=
                                   row.get("command", {}).get("offered") for row in windows)):
                raise ValueError("fixture trigger commands were dropped")
            measurement = next((row for row in windows if row.get("name") == "measurement"), {})
            if measurement.get("command", {}).get("writes_completed", 0) == 0:
                raise ValueError("fixture measurement sent no trigger command")
    snapshot = await compare.snapshot(args, directory, world, config)
    result["entity_counts_after"] = {kind: group["active"] for kind, group in snapshot["entities"].items()}
    if recipe.get("spawn_rules"):
        result["spawn_rules_after"] = validate_spawn_rules(snapshot, recipe["spawn_rules"])
    if recipe.get("stress", {}).get("kind") == "churn":
        after, churn = validate_churn_cohort(recipe, prepared["cohort"], snapshot)
        result["stress"] = {**recipe["stress"], "saved_turnover": churn,
                            "scope": "distinct identities replaced between saved snapshots; not a per-tick rate"}
    else:
        after = compare.cohort(snapshot, recipe["expected"])
        validate_cohort(prepared["cohort"], after, recipe["expected"])
    if recipe.get("stress", {}).get("kind") in ("daylight-zombies", "wool-sheep"):
        result["stress"] = {**recipe["stress"], "saved_state_after":
                            validate_natural_snapshot(recipe["stress"], snapshot, before=False)}
    if recipe.get("stress", {}).get("kind") == "map-frames":
        maps_after = validate_map_frames(snapshot, recipe["stress"]["requested_frames"])
        if maps_after != validate_map_frames(prepared["snapshot"], recipe["stress"]["requested_frames"]):
            raise ValueError("saved frame map identities changed during measurement")
        result["stress"] = {**recipe["stress"], "saved_filled_map_frames": len(maps_after)}
    result["cohort_counts"] = after["counts"]
    result["status"] = "smoke" if args.smoke else "measured"
    result["publication_evidence"] = not args.smoke and "coverage" not in recipe
    load.scale.dump(directory / "result.json", result)
    return result


async def run(args: argparse.Namespace) -> int:
    args.out.mkdir(parents=True, exist_ok=False)
    result: dict[str, Any] = {"status": "failed", "profile": args.profile,
                             "scope": "stated workload measurement; not parity/capacity acceptance",
                             "command": [sys.executable, "-m", "tools.coreperf.profiles", *sys.argv[1:]],
                             "conditions": {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()}}
    task = asyncio.current_task()
    assert task
    asyncio.get_running_loop().add_signal_handler(signal.SIGTERM, task.cancel)
    try:
        if args.productive_npcs is not None:
            raise ValueError("productive-npcs unavailable until CivSim 3/3 and brain owner callsites are integrated")
        result["host"] = host_receipt(args.out)
        server_pin = pinned_binary(args.server_bin, "server")
        bots_pin = pinned_binary(args.bots_bin, "bots")
        if args.build_receipt.resolve() != args.server_bin.resolve().parent / "manifest.json":
            raise ValueError("build receipt must be the server pin-release manifest")
        if (server_pin["head"], server_pin["diff_sha256"]) != (bots_pin["head"], bots_pin["diff_sha256"]):
            raise ValueError("server and bots pins identify different source trees")
        hashes = {"server_sha256": server_pin["binary_sha256"],
                  "bots_sha256": bots_pin["binary_sha256"]}
        result["build"] = {"server": server_pin, "bots": bots_pin}
        result["registry"] = load.tree_hash(args.vanilla_data_dir)
        base_residents = (args.mobs - (args.productive_npcs or 0)
                          - (args.damage_victims or 0) - (args.map_frames or 0)
                          - (args.mobs * args.churn_percent // 100 if args.churn_percent is not None else 0)
                          - (args.daylight_zombies or 0) - (args.wool_sheep or 0)
                          - 2 * (args.reactive_pairs or 0))
        excluded = (("cow", "pig") if args.churn_percent is not None else
                    ("sheep", "zombie") if (args.daylight_zombies is not None or args.wool_sheep is not None
                                            or args.reactive_pairs is not None) else
                    ("cow",) if args.damage_victims is not None else ())
        recipe = (scale_fixture(base_residents, SCALE_PROFILES[args.profile][1],
                                excluded_kinds=excluded,
                                active_override=0 if args.xp_orbs is not None else None)
                  if args.profile in SCALE_PROFILES else fixture(args.mobs, args.villagers))
        if args.projectiles is not None:
            add_projectile_fixture(recipe, args.projectiles, smoke=args.smoke)
        if args.xp_orbs is not None:
            add_xp_fixture(recipe, args.xp_orbs, smoke=args.smoke,
                           players=args.xp_players if args.xp_players is not None else 1,
                           locality=args.xp_locality or "compact")
        if args.containers is not None:
            add_brewing_fixture(recipe, args.containers)
        if args.productive_npcs is not None:
            add_productive_fixture(recipe, args.productive_npcs, smoke=args.smoke)
        if args.damage_victims is not None:
            add_damage_fixture(recipe, args.damage_victims, smoke=args.smoke)
        if args.map_frames is not None:
            add_map_frame_fixture(recipe, args.map_frames, smoke=args.smoke)
        if args.churn_percent is not None:
            add_churn_fixture(recipe, args.mobs * args.churn_percent // 100, args.churn_percent)
        if args.daylight_zombies is not None:
            add_natural_fixture(recipe, args.daylight_zombies, "daylight-zombies")
        if args.wool_sheep is not None:
            add_natural_fixture(recipe, args.wool_sheep, "wool-sheep")
        if args.reactive_pairs is not None:
            add_reactive_fixture(recipe, args.reactive_pairs)
        spawn_rules = {name: value == "on" for name, value in
                       (("spawn_mobs", args.spawn_mobs), ("spawn_monsters", args.spawn_monsters))
                       if value is not None}
        if spawn_rules:
            recipe["spawn_rules"] = spawn_rules
            recipe["regions"][-1]["commands"].extend(
                f"gamerule minecraft:{name} {str(value).lower()}" for name, value in spawn_rules.items())
        if "coverage" in recipe:
            args.clients = (recipe["stress"]["players"] if args.xp_orbs is not None
                            else sum(row["active"] for row in recipe["coverage"]))
            result["conditions"]["clients"] = args.clients
            result["noncanonical"] = True
            result["variant"] = recipe["variant"]
            result["recipe_sha256"] = load.digest(recipe)
            result["population"] = recipe["population"]
        max_commands = max((len(region["commands"]) for region in recipe["regions"]), default=0)
        args.readiness_seconds = max(args.readiness_seconds, (max_commands + 9) // 10 + 120)
        result["conditions"]["readiness_seconds"] = args.readiness_seconds
        load.scale.dump(args.out / "fixture.json", recipe)
        key = compare.fixture_cache_key(args, recipe, result["registry"]["sha256"])
        world, prepared = await compare.prepare(args, args.out / "prepare", recipe, key, hashes)
        validate_prepared(recipe, prepared)
        if recipe.get("spawn_rules"):
            result["spawn_rules_before"] = validate_spawn_rules(prepared["snapshot"], recipe["spawn_rules"])
        if recipe.get("stress", {}).get("kind") == "reactive-damage":
            result["reactive_equipment_before"] = validate_reactive_equipment(prepared["snapshot"], args.reactive_pairs)
        if recipe.get("stress", {}).get("kind") in ("daylight-zombies", "wool-sheep"):
            result["natural_cohort_before"] = validate_natural_snapshot(
                recipe["stress"], prepared["snapshot"], before=True)
        if recipe.get("expected_blocks"):
            block_snapshot = await load.native_snapshot(args, args.out / "fixture-blocks", world,
                                                        args.out / "prepare" / "server.toml",
                                                        fixture_block_spec(recipe))
            result["fixture_blocks_before"] = validate_fixture_blocks(recipe, block_snapshot)
        if args.map_frames is not None:
            result["saved_filled_map_frames_before"] = len(validate_map_frames(prepared["snapshot"], args.map_frames))
        result["fixture_cache"] = prepared["fixture_cache"]
        result["entity_counts_before"] = {kind: group["active"] for kind, group in prepared["snapshot"]["entities"].items()}
        if args.prepare_only:
            result["status"] = "prepared"
        else:
            result["measurement"] = await measure(args, args.out / "run", world, recipe, prepared)
            result["status"] = "smoke" if args.smoke else "measured"
            result["publication_evidence"] = not args.smoke and "coverage" not in recipe
    except ColdVisibilityUnavailable as error:
        result["status"] = "blocked"
        result["error"] = f"{type(error).__name__}: {error}"
        result["cold_visibility_proof"] = error.proof
    except (Exception, asyncio.CancelledError) as error:
        result["error"] = f"{type(error).__name__}: {error}"
    finally:
        result["logs"] = [str(path) for path in sorted(args.out.rglob("*.log"))]
        result["commands"] = [json.loads(path.read_text()) for path in sorted(args.out.rglob("server-command.json"))]
        result["exits"] = {}
        for phase, filename in (("prepare", "receipt.json"), ("run", "result.json")):
            receipt_path = args.out / phase / filename
            if receipt_path.is_file():
                receipt = json.loads(receipt_path.read_text())
                if phase == "prepare" and receipt.get("fixture_cache", {}).get("status") == "hit":
                    continue  # Cached preparation commands/exits belong to the producer, not this run.
                result["exits"][phase] = receipt.get("shutdown", {})
                for row in receipt.get("regions", []):
                    result["commands"].append(row["bot_command"])
                    result["exits"][f"prepare/{row['region']}"] = row.get("shutdown", {})
                if receipt.get("bot_command"):
                    result["commands"].append(receipt["bot_command"])
        load.scale.dump(args.out / "result.json", result)
    print(json.dumps({"status": result["status"], "receipt": str(args.out / "result.json"),
                      "error": result.get("error")}), flush=True)
    return int(result["status"] not in ("measured", "prepared", "smoke"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", choices=(*PROFILES, *SCALE_PROFILES))
    parser.add_argument("--server-bin", type=Path, required=True)
    parser.add_argument("--bots-bin", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--fixture-cache", type=Path, default=load.ROOT / ".analysis/profile-fixtures")
    parser.add_argument("--vanilla-data-dir", type=Path, default=load.ROOT / "data/vanilla")
    parser.add_argument("--server-cpus", default="0-7")
    parser.add_argument("--bot-cpus", default="8-11")
    parser.add_argument("--server-wrapper", default="",
                        help="command prefix placed before the pinned server (e.g. a systemd-run memory scope; must exec in place)")
    remote = parser.add_argument_group("remote server (tools.bench vps)",
        "Server runs on a remote host in its own transient systemd unit; bots stay local. --server-cpus then names REMOTE CPUs.")
    remote.add_argument("--remote-host", help="ssh destination, e.g. root@203.0.113.7")
    remote.add_argument("--remote-ssh-option", action="append", default=[], help="extra ssh option, repeatable (e.g. -p)")
    remote.add_argument("--remote-dir", default="/var/tmp", help="parent of the per-run solaris-bench-* directory")
    remote.add_argument("--remote-data-dir", help="imported content cache on the remote host (dir with version.json)")
    remote.add_argument("--remote-game-port", type=int, default=25590)
    remote.add_argument("--remote-dash-port", type=int, default=25591)
    remote.add_argument("--remote-memory-max", help="MemoryMax of the remote server unit")
    remote.add_argument("--remote-tunnel-game", action="store_true", help="carry bot traffic through ssh (adds sshd CPU on the host)")
    remote.add_argument("--remote-bot-host", help="address bots dial (default: host part of --remote-host)")
    parser.add_argument("--warmup-seconds", type=int, default=30)
    parser.add_argument("--measure-seconds", type=int, default=120)
    parser.add_argument("--readiness-seconds", type=int, default=1800)
    parser.add_argument("--max-rss-mb", type=int, default=4096)
    parser.add_argument("--prepare-only", action="store_true",
                        help="populate/reuse the fixture cache without measuring; never a performance receipt")
    parser.add_argument("--smoke", action="store_true",
                        help="2s warmup + 5s measurement at full population; lifecycle only, never publication evidence")
    parser.add_argument("--smoke-residents", type=int,
                        help="smaller noncanonical resident cohort for --smoke only")
    stress = parser.add_mutually_exclusive_group()
    stress.add_argument("--projectiles", type=int,
                        help="noncanonical exact projectile cohort: 0, 100 or 1000 (smoke may use smaller counts)")
    stress.add_argument("--xp-orbs", type=int,
                        help="noncanonical exact active XP-orb cohort: 0, 10000 or 100000 (smoke may use smaller counts)")
    stress.add_argument("--containers", type=int,
                        help="brewing discovery load: 1000 or 100000 containers (smoke may use smaller counts)")
    stress.add_argument("--productive-npcs", type=int,
                        help="productive farmers: 0, 1000 or 10000 (smoke may use smaller counts)")
    stress.add_argument("--damage-victims", type=int,
                        help="distinct accepted projectile victims: 1, 1000, 10000 or 100000")
    stress.add_argument("--map-frames", type=int,
                        help="saved filled-map frames: 0, 10000 or 100000 (smoke may use smaller counts)")
    parser.add_argument("--xp-players", type=int, choices=(0, 1, 100, 1000))
    parser.add_argument("--xp-locality", choices=("compact", "spread"))
    parser.add_argument("--spawn-mobs", choices=("on", "off"))
    parser.add_argument("--spawn-monsters", choices=("on", "off"))
    stress.add_argument("--churn-percent", type=int, choices=(0, 1, 10))
    stress.add_argument("--daylight-zombies", type=int)
    stress.add_argument("--wool-sheep", type=int)
    stress.add_argument("--reactive-pairs", type=int)
    args = parser.parse_args()
    if args.xp_orbs is None and (args.xp_players is not None or args.xp_locality is not None):
        parser.error("XP player options require --xp-orbs")
    if args.profile in SCALE_PROFILES:
        if args.remote_host:
            parser.error("scale telemetry requires a local server")
        if args.smoke_residents is not None and not args.smoke:
            parser.error("--smoke-residents requires --smoke")
        args.clients, args.mobs, args.villagers, args.view_distance = (
            0, args.smoke_residents if args.smoke_residents is not None else SCALE_PROFILES[args.profile][0], 0, 6)
        if args.mobs < 10:
            parser.error("scale population must be at least 10")
        if args.projectiles is not None and (
                args.projectiles < 0 or args.projectiles > 1000
                or not args.smoke and args.projectiles not in (0, 100, 1000)):
            parser.error("projectiles must be 0, 100 or 1000 except in smoke")
        if args.xp_players == 0 and args.xp_orbs != 0:
            parser.error("zero XP observers require the zero-orb control")
        if args.xp_orbs is not None and (
                args.xp_orbs < 0 or args.xp_orbs > 100000
                or not args.smoke and args.xp_orbs not in (0, 10000, 100000)):
            parser.error("XP-orbs must be 0, 10000 or 100000 except in smoke")
        if args.containers is not None and (
                args.containers < 1 or args.containers > 100000
                or not args.smoke and args.containers not in (1000, 100000)):
            parser.error("containers must be 1000 or 100000 except in smoke")
        if args.productive_npcs is not None and (
                args.productive_npcs < 0 or args.productive_npcs > min(10000, args.mobs - 10)
                or not args.smoke and args.productive_npcs not in (0, 1000, 10000)):
            parser.error("productive-npcs must be 0, 1000 or 10000 and leave at least 10 mobs")
        if args.damage_victims is not None and (
                args.damage_victims < 1 or args.damage_victims > min(100000, args.mobs)
                or not args.smoke and args.damage_victims not in (1, 1000, 10000, 100000)):
            parser.error("damage-victims must be 1, 1000, 10000 or 100000 and not exceed residents")
        for count in (args.daylight_zombies, args.wool_sheep):
            if count is not None and not 0 <= count <= args.mobs - 10:
                parser.error("natural cohort count must leave at least 10 background residents")
        if args.reactive_pairs is not None and not 1 <= args.reactive_pairs <= (args.mobs - 10) // 2:
            parser.error("reactive pairs must leave at least 10 background residents")
        if args.churn_percent and args.mobs * args.churn_percent // 100 == 0:
            parser.error("churn percentage produces an empty cohort")
        if args.map_frames is not None and (
                args.map_frames < 0 or args.map_frames > min(100000, args.mobs)
                or not args.smoke and args.map_frames not in (0, 10000, 100000)):
            parser.error("map-frames must be 0, 10000 or 100000 and not exceed residents")
    else:
        if args.spawn_mobs is not None or args.spawn_monsters is not None:
            parser.error("spawn-rule controls require a scale profile")
        if args.smoke_residents is not None:
            parser.error("--smoke-residents requires a scale profile")
        if any(value is not None for value in (args.projectiles, args.xp_orbs, args.containers,
                                                args.productive_npcs, args.damage_victims, args.map_frames,
                                                args.churn_percent, args.daylight_zombies, args.wool_sheep,
                                                args.reactive_pairs)):
            parser.error("stress fixtures require a scale profile")
        args.clients, args.mobs, args.villagers, args.view_distance = PROFILES[args.profile]
    args.seed, args.prepare_concurrency = 42, 3
    if min(args.warmup_seconds, args.measure_seconds, args.readiness_seconds, args.max_rss_mb) <= 0:
        parser.error("durations and RSS guard must be positive")
    if args.smoke:
        args.warmup_seconds, args.measure_seconds = 2, 5
    if args.out is None:
        args.out = load.ROOT / ".analysis/validation" / f"{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}-profile-{args.profile}-{os.getpid()}"
    for name in ("server_bin", "bots_bin", "build_receipt", "out", "fixture_cache", "vanilla_data_dir"):
        setattr(args, name, getattr(args, name).resolve())
    return asyncio.run(run(args))


if __name__ == "__main__":
    raise SystemExit(main())
