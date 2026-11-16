"""Read a generated unopened barrel, then prove its server-sent menu loot."""
from __future__ import annotations

import json
from pathlib import Path

from .atm_carry import digest
from .quirk_parity.anvil import read_region, read_world

AIR = {"minecraft:air", "minecraft:cave_air", "minecraft:void_air"}


def container_candidate(site: dict, pack: Path, probe: dict) -> dict:
    namespace, name = probe["table"].split(":", 1)
    table = (pack / "data" / namespace / "loot_table" / (name + ".json")).resolve()
    table.relative_to(pack.resolve())
    definition = json.loads(table.read_text())
    allowed = set()
    for pool in definition["pools"]:
        for entry in pool["entries"]:
            if entry["type"] != "minecraft:item":
                raise ValueError("container proof requires explicit item entries, not an inferred allowlist")
            allowed.add(entry["name"])
    if not allowed:
        raise ValueError("container proof has no allowed loot items")
    lo, hi = site["bounds"][:3], site["bounds"][3:]
    dimension = Path(site["dimension_dir"])
    expanded = [v - 2 for v in lo] + [v + 2 for v in hi]
    blocks = read_world(dimension, [expanded])[0]["blocks"]
    def block(pos):
        return blocks.get(",".join(map(str, pos)), "minecraft:air").split("[")[0]
    candidates = []
    for region in sorted((dimension / "region").glob("r.*.*.mca")):
        for _, _, chunk in read_region(region):
            for entity in chunk.get("block_entities", []):
                if entity.get("id") != probe["block"] or entity.get("LootTable") != probe["table"]:
                    continue
                pos = [entity[axis] for axis in ("x", "y", "z")]
                if not all(a <= v <= b for a, v, b in zip(lo, pos, hi)):
                    continue
                if entity.get("Items") or block(pos) != probe["block"]:
                    continue
                x, y, z = pos
                for dx, dz in ((0, -1), (0, 1), (-1, 0), (1, 0)):
                    feet, head, floor = [x+dx, y, z+dz], [x+dx, y+1, z+dz], [x+dx, y-1, z+dz]
                    if block(feet) in AIR and block(head) in AIR and block(floor) not in AIR | {"minecraft:water", "minecraft:lava"}:
                        candidates.append({"position": pos, "stand": [feet[0]+0.5, y, feet[2]+0.5],
                                           "saved_block_entity": entity, "table": probe["table"],
                                           "allowed_items": sorted(allowed), "table_sha256": digest(table)})
                        break
    if not candidates:
        raise ValueError(f"{site['id']}: no reachable generated unopened {probe['block']} with LootTable {probe['table']}")
    return min(candidates, key=lambda row: sum((a-b)**2 for a, b in zip(row["position"], site["anchor"])))


def rolled_slots(screen: dict, allowed: list[str]) -> list[dict]:
    if screen.get("screen") != "container":
        raise ValueError(f"generated barrel did not open a container menu: {screen}")
    slots = [slot for slot in screen.get("slots", []) if 300 <= slot.get("slot", -1) < 327]
    if len(slots) != 27 or len({slot["slot"] for slot in slots}) != 27:
        raise ValueError("generated barrel did not send its complete 27-slot inventory")
    occupied = [slot for slot in slots if slot.get("item") is not None]
    if not occupied:
        raise ValueError("generated barrel loot is empty")
    if any(slot["item"] not in allowed or not isinstance(slot.get("count"), int) or slot["count"] <= 0 for slot in occupied):
        raise ValueError(f"generated barrel contains items outside its loot table: {occupied}")
    return slots


def open_generated_container(client, site: dict, pack: Path, probe: dict, picture: Path) -> dict:
    evidence = container_candidate(site, pack, probe)
    client.teleport(evidence["stand"])
    if client.call("state").get("gamemode") != "creative":
        client.command("gamemode creative @s", "game mode")
    observed = client.call("block", *evidence["position"])
    if observed.get("loaded") is not True or observed.get("block", "").split("[")[0] != probe["block"]:
        raise ValueError(f"client did not receive the generated barrel: {observed}")
    client.call("interact", *evidence["position"])
    client.call("wait-for", "screen", "container", "--timeout", client.args.timeout)
    screen = client.call("screen")
    slots = rolled_slots(screen, evidence["allowed_items"])
    capture = client.call("screenshot", picture)
    if picture.read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("container screenshot did not produce a PNG")
    client.call("close")
    client.call("wait-for", "screen", "playing", "--timeout", client.args.timeout)
    client.call("interact", *evidence["position"])
    client.call("wait-for", "screen", "container", "--timeout", client.args.timeout)
    reopened = client.call("screen")
    if rolled_slots(reopened, evidence["allowed_items"]) != slots:
        raise ValueError("generated barrel changed on second open without any inventory input")
    client.call("close")
    client.call("wait-for", "screen", "playing", "--timeout", client.args.timeout)
    client.command("gamemode spectator @s", "game mode")
    evidence.update(client_block=observed, screen=screen, reopened_screen=reopened,
                    screenshot={"path": str(picture), "sha256": digest(picture), "capture": capture})
    return evidence
