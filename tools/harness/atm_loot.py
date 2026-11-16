"""Read a generated unopened barrel, then prove its server-sent menu loot."""
from __future__ import annotations

import json
import math
from pathlib import Path

from .atm_carry import digest
from .quirk_parity.anvil import read_region, read_world

AIR = {"minecraft:air", "minecraft:cave_air", "minecraft:void_air"}
PASSABLE = AIR | {"minecraft:water"}
FACES = (("down", (0, -1, 0)), ("up", (0, 1, 0)), ("north", (0, 0, -1)),
         ("south", (0, 0, 1)), ("west", (-1, 0, 0)), ("east", (1, 0, 0)))


def stand_positions(pos, block):
    """Potential feet cells, checked again against the real client after teleport."""
    x, y, z = pos
    positions = []
    for sx in range(x - 5, x + 6):
        for sy in range(y - 6, y + 5):
            for sz in range(z - 5, z + 6):
                if (block((sx, sy, sz)) not in PASSABLE or
                    block((sx, sy + 1, sz)) not in PASSABLE or
                    block((sx, sy - 1, sz)) in PASSABLE | {"minecraft:lava"}):
                    continue
                eye = (sx + 0.5, sy + 1.62, sz + 0.5)
                nearest = [max(p, min(e, p + 1)) for e, p in zip(eye, pos)]
                if math.dist(eye, nearest) <= 4.5:
                    positions.append([sx + 0.5, sy, sz + 0.5])
    return sorted(positions, key=lambda p: (sum((a - (b + 0.5)) ** 2 for a, b in zip(p, pos)), p))


def visible_face(client, pos):
    """Use Horizon's actual clip ray and enforce survival eye-to-hit reach."""
    centre = [value + 0.5 for value in pos]
    for name, face in FACES:
        for inset in (0.45, 0.3, 0.0):
            client.call("look-at", *(c + d * inset for c, d in zip(centre, face)))
            target = client.call("looking-at")
            hit = target.get("block") or {}
            if (target.get("kind") == "block" and hit.get("pos") == pos and hit.get("face") == name and
                isinstance(hit.get("distance"), (int, float)) and hit["distance"] <= 4.5):
                return hit
    return None


def select_reachable(client, candidates):
    attempts = []
    client.receipt["container_selection"] = {
        "candidate_order": [row["position"] for row in candidates], "attempts": attempts,
    }
    for evidence in candidates:
        for stand in evidence["stands"]:
            if client.call("state").get("gamemode") != "spectator":
                client.command("gamemode spectator @s", "game mode")
            client.teleport(stand)
            client.command("gamemode survival @s", "game mode")
            client.call("wait", 10)
            player = client.call("state").get("player") or {}
            actual = player.get("position")
            attempt = {"position": evidence["position"], "stand": stand, "player": player}
            attempts.append(attempt)
            if (not player.get("on_ground") or not isinstance(actual, list) or len(actual) != 3 or
                any(abs(a - b) > 0.05 for a, b in zip(actual, stand))):
                continue
            hit = visible_face(client, evidence["position"])
            if hit is not None:
                attempt["hit"] = hit
                return {**evidence, "stand": stand, "target_hit": hit, "selection_attempts": attempts}
    raise ValueError("no reachable generated unopened container with clear survival-reach clip ray")


def container_candidates(site: dict, pack: Path, probe: dict) -> list[dict]:
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
    expanded = [v - 6 for v in lo] + [v + 6 for v in hi]
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
                stands = stand_positions(pos, block)
                if stands:
                    candidates.append({"position": pos, "stands": stands,
                                       "saved_block_entity": entity, "table": probe["table"],
                                       "allowed_items": sorted(allowed), "table_sha256": digest(table)})
    if not candidates:
        raise ValueError(f"{site['id']}: no reachable generated unopened {probe['block']} with LootTable {probe['table']}")
    return sorted(candidates, key=lambda row: (sum((a-b)**2 for a, b in zip(row["position"], site["anchor"])),
                                                row["position"]))


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
    evidence = select_reachable(client, container_candidates(site, pack, probe))
    observed = client.call("block", *evidence["position"])
    if observed.get("loaded") is not True or observed.get("block", "").split("[")[0] != probe["block"]:
        raise ValueError(f"client did not receive the generated barrel: {observed}")
    client.call("interact", *evidence["position"], evidence["target_hit"]["face"])
    client.call("wait-for", "screen", "container", "--timeout", client.args.timeout)
    screen = client.call("screen")
    slots = rolled_slots(screen, evidence["allowed_items"])
    capture = client.call("screenshot", picture)
    if picture.read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("container screenshot did not produce a PNG")
    client.call("close")
    client.call("wait-for", "screen", "playing", "--timeout", client.args.timeout)
    client.call("interact", *evidence["position"], evidence["target_hit"]["face"])
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
