"""The blocks of the settlement quick-wins walk (see `settlement_quickwins_qa`).

Each block opens its own screens from the bell and judges the criteria named in its table row;
a block never relies on the screen the previous one left open.
"""
from __future__ import annotations

import ast
import math
import re
import subprocess
import sys
from typing import Any

from .. import runtime
from . import scene_shot
from .settlement_quickwins_qa import (
    CRITERIA, NAME, RADIUS, StepFailed, Walk, blob, greeting_verdict, guide_verdict, matching,
    refusal_verdict, shortage_verdict, signature, view, UUID,
)

BUILDING_LIST = re.compile(
    r"#(\d+)\s+(\w+)\s+level\s+(\d+)\s+\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]-\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]",
    re.IGNORECASE)
OLD_REVIEW = re.compile(r"До приёма|Если занято|нехватка|Этот клик не ставит", re.IGNORECASE)
MANAGE = r"^(Управление|Поселение)$"  # the hub's way into the dashboard: new name, or the old one on main
BLUEPRINT = re.compile(r"\bL1$")


# ----------------------------------------------------------------------------- block runner


def run_block(w: Walk, name: str, criteria: tuple[str, ...], block: Any) -> None:
    """Run one block. Strict: an unreachable step fails the first criterion the block still owes.
    Survey: record the error, mark what is left as not reached, continue from a closed screen."""
    w.blocks[name] = "running"
    try:
        block(w)
        w.blocks[name] = "done"
    except StepFailed as error:
        w.blocks[name] = f"stopped: {error}"
        owed = [cid for cid in criteria if cid not in w.results]
        if w.strict:
            if owed:
                w.judge(owed[0], False, not_reached=True, error=str(error))  # raises: names the criterion
            raise
        for cid in owed:
            w.judge(cid, False, not_reached=True, error=str(error))
        try:
            w.close_all()
        except StepFailed:
            pass


# ----------------------------------------------------------------------------- setup


def setup(w: Walk) -> None:
    """Privileged prerequisites: a flat pad and a survival player; nothing here is judged."""
    from .buildings_qa import _chunked_fills
    probe = w.probe
    probe.report["setup_not_asserted"] = [
        "creative-only private pad, then survival before any settlement action",
        "bell at the anchor, teleports between the floor corners, house blocks, villagers, chest stock",
        "/settlement create, the dialogs and the typed `/settlement menu certify` are the player's own input",
    ]
    first = probe.observe(wait_for_play=True)
    w.evidence["join_chat"] = [str(line) for line in first.get("recent_chat", [])]
    for command in ("gamemode creative @s", "gamerule advance_time false", "gamerule spawn_mobs false",
                    "time set 6000", "weather clear", "clear @s"):
        w.fixture(command)
    me = probe.observe(wait_for_play=True)["player"]
    x, z = (math.floor(float(me[axis])) for axis in ("x", "z"))
    y = 201
    w.fixture(f"tp @s {x + .5} {y} {z - 2.0}")  # no wait: the pad below does not exist yet
    for command in _chunked_fills(x - 64, y - 2, z - 64, x + 64, y - 2, z + 64, "minecraft:dirt"):
        w.fixture(command)
    for command in _chunked_fills(x - 64, y - 1, z - 64, x + 64, y - 1, z + 64, "minecraft:grass_block"):
        w.fixture(command)
    for command in _chunked_fills(x - 64, y, z - 64, x + 64, y + 24, z + 64, "minecraft:air"):
        w.fixture(command)
    w.tp(x + .5, y, z + .5)
    w.fixture("gamemode survival @s")
    # The player's own command: found the settlement at the anchor.
    probe.report.setdefault("player_commands", []).append(f"settlement create {NAME} {RADIUS}")
    probe.call("minecraft_send_chat", {"message": f"settlement create {NAME} {RADIUS}", "command": True})
    probe.wait(lambda state: any(f"Founded {NAME}" in str(line) or f"Основан" in str(line)
                                 for line in state.get("recent_chat", [])),
               "QW settlement founded", timeout_seconds=60)
    w.evidence["pad"] = {"anchor": [x, y, z], "ground_y": y - 1, "stand": [x + .5, y, z - 2.0]}


def find_chest(w: Walk) -> list[int] | None:
    """The chest block next to the anchor, by block scan (the bell is set after, so it cannot hide it)."""
    x, y, z = w.evidence["pad"]["anchor"]
    for _ in range(15):
        page = w.probe.call("minecraft_scan_blocks", {
            "min_x": x - 4, "min_y": y - 3, "min_z": z - 4, "max_x": x + 4, "max_y": y + 3, "max_z": z + 4,
            "max_blocks": 4096})
        found = sorted((abs(row["x"] - x) + abs(row["y"] - y) + abs(row["z"] - z), [row["x"], row["y"], row["z"]])
                       for row in page["blocks"] if row["block_id"] == "minecraft:chest")
        if found:
            return found[0][1]
        w.probe.call("minecraft_wait_ticks", {"ticks": 20})
    return None


def block_create(w: Walk) -> None:
    x, y, z = w.evidence["pad"]["anchor"]
    chest = find_chest(w)
    w.evidence["starter_chest"] = chest
    w.judge("C1", chest is not None, chest=chest, anchor=[x, y, z])
    if chest is None:
        if w.strict:
            return
        # Survey on a tree without the chest: a fixture chest keeps the later blocks walkable.
        chest = [x + 9, y, z + 9]
        w.fixture(f"setblock {chest[0]} {chest[1]} {chest[2]} minecraft:chest")
        w.fixture(f"settlement building certify storage {x + 8} {y - 1} {z + 8} {x + 10} {y + 2} {z + 10}")
        w.evidence["starter_chest"] = chest
        w.evidence["survey_fixture_chest"] = True
    w.fixture(f"setblock {x} {y} {z} minecraft:bell")
    for dx, dz in ((0, -2), (0, 3), (-2, 0), (3, 0)):
        between = (x + (dx > 0) - (dx < 0), z + (dz > 0) - (dz < 0))  # the cell the crosshair ray crosses
        if (chest[0], chest[2]) not in (between, (x + dx, z + dz)):
            w.evidence["pad"]["stand"] = [x + dx + .5, y, z + dz + .5]
            break
    w.tp(*w.evidence["pad"]["stand"])


# ----------------------------------------------------------------------------- hub and dashboard


def block_hub(w: Walk) -> None:
    shown = w.hub()
    w.shot("hub")
    w.judge("Q11a", bool(matching(r"^Управление$", shown["buttons"])) and not matching(r"^Поселение$", shown["buttons"]),
            buttons=shown["buttons"])
    w.judge("Q10a", not matching(r"Соседи", shown["buttons"]), buttons=shown["buttons"])
    landed = w.click_button(r"^Следующий шаг$", "hub Next step")
    w.shot("next-step")
    text = blob(landed)
    w.judge("Q9", not (landed["kind"] == "menu" and landed["title"].startswith("Постройки"))
            and bool(re.search(r"склад|заказ|строит|строй|материал|сертифиц", text, re.IGNORECASE)),
            screen={"kind": landed["kind"], "title": landed["title"], "text": text[:600]})


def open_dashboard(w: Walk) -> dict[str, Any]:
    w.hub()
    return w.click_button(MANAGE, "hub Manage", lambda s: s["kind"] == "menu")


def block_dashboard(w: Walk) -> None:
    dash = open_dashboard(w)
    w.shot("dashboard")
    names = [name for name in dash["slots"].values() if name]
    w.judge("Q11b", bool(matching(r"^Строительство$", names)) and not matching(r"^Стройка$", names), icons=names)
    w.judge("Q10b", not matching(r"Соседи|Чек-?лист|До удела", names), icons=names)
    back_check(w, "Q6a", "slot", r"^Назад$", lambda s: s["kind"] == "dialog" and s["title"] == f"Поселение {NAME}")


def back_check(w: Walk, cid: str, how: str, pattern: str, expected: Any, origin: dict[str, Any] | None = None) -> dict[str, Any]:
    """Press Back and judge where it lands (`expected`); the landing screen is the evidence."""
    before = signature(w.screen())
    (w.click_slot if how == "slot" else w.click_button)(pattern, f"{cid} Back", wait=False)
    try:
        landed = w.settle(f"{cid} Back target", before=before, timeout=20)
    except StepFailed:
        landed = w.screen()
    w.shot(f"{cid}-back")
    w.judge(cid, bool(expected(landed)), landed={"kind": landed["kind"], "title": landed["title"],
                                                  "buttons": landed["buttons"][:8]},
            expected_origin=origin and {"kind": origin["kind"], "title": origin["title"]})
    return landed


def to_storage(w: Walk) -> tuple[dict[str, Any], dict[str, Any]]:
    """Dashboard -> Storage by the icons a player clicks. Returns (screen it was opened from, Storage)."""
    current = open_dashboard(w)
    for _ in range(3):
        previous = current
        current = w.click_slot(r"^Склад$", "Storage icon", lambda s: s["kind"] in ("menu", "dialog"))
        if current["kind"] == "menu" and re.match(r"Склад\s*·", current["title"]):
            return previous, current
    raise StepFailed(f"Storage page not reached; last screen {current['title']!r}")


def block_storage(w: Walk) -> None:
    chest = w.evidence["starter_chest"]
    origin, storage = to_storage(w)
    w.shot("storage-empty")
    info = " ".join(storage["slots"].values())
    match = re.search(r"Хранилищ:\s*(\d+),\s*видов:\s*(\d+),\s*предметов:\s*(\d+)", info)
    counts = tuple(map(int, match.groups())) if match else None
    w.judge("C3", counts == (1, 0, 0), counts=counts, info=info[:300])
    w.judge("Q8b", bool(re.search(r"принес\w*\s+материал", " ".join([blob(storage), *w.lore(storage)]), re.IGNORECASE)))
    held_before = w.chest_items(chest)
    w.judge("C2", not re.search(r"minecraft:\w+", held_before), chest=chest, readback=held_before[:300])
    item = "minecraft:cobblestone"
    w.fixture(f"give @s {item} 1")
    state = w.probe.wait(lambda s: _held(s, item) == 1, "QW deposit item received", timeout_seconds=30)
    own = next(row["slot"] for row in state["container"]["slots"]
               if row["slot"] >= 54 and row.get("item_id") == item)
    w.gui.slot(own, shift=True)
    state = w.probe.wait(lambda s: _held(s, item) == 0 and _stored(s, item) >= 1,
                         "QW deposit stored", timeout_seconds=30)
    w.shot("storage-deposit")
    held_after = w.chest_items(chest)
    w.judge("C4", item in held_after, stored_icon=_stored(state, item), readback=held_after[:300])
    back_check(w, "Q6b", "slot", r"^Назад$",
               lambda s: s["kind"] == origin["kind"] and s["title"] == origin["title"], origin)


def _held(state: dict[str, Any], item: str) -> int:
    return sum(int(row["count"]) for row in state["container"]["slots"]
               if row["slot"] >= 54 and row.get("item_id") == item)


def _stored(state: dict[str, Any], item: str) -> int:
    for row in state["container"]["slots"]:
        if row["slot"] < 54 and row.get("item_id") == item:
            count = re.search(r"×(\d+)$", str(row.get("name") or ""))
            return int(count[1]) if count else 0
    return 0


def block_buildings_empty(w: Walk) -> None:
    open_dashboard(w)
    listing = w.click_slot(r"^Постройки$", "dashboard Buildings", lambda s: s["kind"] == "menu")
    w.shot("buildings-empty")
    text = " ".join([blob(listing), *w.lore(listing)])
    w.judge("Q8a", bool(re.search(r"зданий\s+пока\s+нет|пока\s+нет\s+зданий", text, re.IGNORECASE))
            and bool(re.search(r"Строительств|Стройк", text)) and bool(re.search(r"Сертифиц", text)),
            title=listing["title"], text=text[:500])


# ----------------------------------------------------------------------------- order


def open_blueprints(w: Walk) -> dict[str, Any]:
    open_dashboard(w)
    shown = w.click_slot(r"^(Строительство|Стройка)$", "dashboard Construction", lambda s: s["kind"] == "menu")
    if not matching(BLUEPRINT.pattern, list(shown["slots"].values())):
        shown = w.click_slot(r"^(Проекты|Заказать|Чертежи|Список)", "Construction -> blueprint list",
                             lambda s: s["kind"] == "menu")
    if not matching(BLUEPRINT.pattern, list(shown["slots"].values())):
        raise StepFailed(f"no blueprint list (icons ending in L1) behind Construction: {sorted(shown['slots'].values())}")
    return shown


def block_order(w: Walk) -> None:
    open_blueprints(w)
    review = w.click_slot(r"^Дом\b", "blueprint House", lambda s: s["kind"] == "dialog")
    w.shot("order-review")
    text = " ".join(review["body"])
    sentences = [part for part in re.split(r"[.!?]\s*", text.replace("\n", " ")) if part.strip()]
    w.judge("Q5", not OLD_REVIEW.search(text) and bool(re.search(r"строител", text, re.IGNORECASE))
            and bool(re.search(r"склад", text, re.IGNORECASE)) and 2 <= len(sentences) <= 4,
            body=text[:500], sentences=len(sentences))
    estimate = w.click_button(r"^Оценка материалов$", "review Materials estimate", lambda s: s["kind"] == "dialog")
    w.shot("order-estimate")
    text = " ".join(estimate["body"])
    w.judge("Q4", not re.search(r"minecraft:", text) and bool(re.search(r"на складе", text, re.IGNORECASE))
            and bool(re.search(r"[А-Яа-я]{3,}", text)), body=text[:500])
    w.click_button(r"^Назад$", "estimate Back", lambda s: s["kind"] == "dialog" and matching(r"^Заказ", s["buttons"]))
    outcome = w.click_button(r"^Заказ", "review Order", lambda s: s["kind"] == "dialog")
    w.shot("order-outcome")
    outcome = {**outcome, "buttons": w.reveal("order outcome")["buttons"]}
    w.shot("order-outcome-actions")
    text = " ".join(outcome["body"])
    ok, facts = shortage_verdict(text, outcome["buttons"])
    w.judge("Q3", ok, body=text[:600], buttons=outcome["buttons"], **facts)
    w.click_button(r"склад", "outcome Storage link", lambda s: s["kind"] == "menu")
    w.shot("outcome-storage")
    back_check(w, "Q6c", "slot", r"^Назад$",
               lambda s: s["kind"] == "dialog" and s["body"] == outcome["body"], outcome)


# ----------------------------------------------------------------------------- project


def stock_and_staff(w: Walk) -> None:
    """Fixture: six bound villagers and the real materials of House L1 in the starter chest."""
    chest = w.evidence["starter_chest"]
    taken: set[str] = set()
    candidates: list[dict[str, Any]] = []
    x, y, z = w.evidence["pad"]["anchor"]

    def arrived(_state: dict[str, Any]) -> bool:
        nonlocal candidates
        entities = w.probe.call("minecraft_list_entities", {"radius": 64})
        candidates = [entity for entity in entities.get("entities", entities.get("visible_entities", []))
                      if "villager" in str(entity.get("entity_type", entity.get("type", "")))
                      and entity.get("entity_uuid") not in taken]
        return bool(candidates)

    for index in range(6):
        w.fixture(f"summon minecraft:villager {x + 5.5 + index} {y} {z + 5.5}")
        w.probe.wait(arrived, f"QW villager {index} received", timeout_seconds=30)
        worker = str(candidates[0]["entity_uuid"])
        taken.add(worker)
        w.fixture(f"settlement bind {NAME} {worker}")
    w.evidence["residents"] = sorted(taken)
    materials = subprocess.run(
        [sys.executable, str(scene_shot.runtime.REPO_ROOT / "tools/blueprints.py"), "materials", "house"],
        check=True, capture_output=True, text=True, cwd=scene_shot.runtime.REPO_ROOT)
    base = next(ast.literal_eval(line.split("base", 1)[1].strip())
                for line in materials.stdout.splitlines() if "L1:" in line and "base" in line)
    slot = 0
    for item, count in sorted(base.items()):
        remaining = math.ceil(count * 1.5) + 1
        while remaining:
            amount = min(64, remaining)
            if slot >= 27:
                raise RuntimeError("House L1 stock exceeds the starter chest")
            w.fixture(f"item replace block {chest[0]} {chest[1]} {chest[2]} container.{slot} with minecraft:{item} {amount}")
            remaining -= amount
            slot += 1


def open_status(w: Walk) -> dict[str, Any]:
    """Dashboard -> Construction -> the status page of the active/queued work."""
    open_blueprints(w)
    return w.click_slot(r"^Статус$", "blueprints Status", lambda s: s["kind"] == "menu")


def block_project(w: Walk) -> None:
    stock_and_staff(w)
    active: int | None = None
    status: dict[str, Any] = {}
    for _ in range(40):
        status = open_status(w)
        active = next((slot for slot in sorted(status["slots"]) if slot < 8 and status["slots"][slot]), None)
        if active is not None:
            break
        w.probe.call("minecraft_wait_ticks", {"ticks": 100})
    if active is None:
        raise StepFailed("no active project appeared after the funded order")
    w.shot("project-status")
    icon = status["slots"][active]
    before = signature(status)
    w.gui.slot(active)
    landed = w.settle("project icon target", before=before)
    w.shot("project-icon")
    w.judge("Q7", not (landed["kind"] == "dialog" and landed["title"].startswith("Карта")),
            icon=icon, landed={"kind": landed["kind"], "title": landed["title"], "text": blob(landed)[:400]})
    open_status(w)
    details = w.click_slot(r"^Статус$", "Status details", lambda s: s["kind"] == "dialog")
    for _ in range(5):
        if re.search(r"Строител", " ".join(details["body"])):
            break
        w.probe.call("minecraft_wait_ticks", {"ticks": 100})
        details = w.screen()
    w.shot("project-details")
    text = " ".join(details["body"])
    w.judge("Q17", bool(re.search(r"Строител", text)) and not UUID.search(text)
            and not re.search(r"Native supply cause|Причина снабжения недоступна", text), body=text[:600])


def block_resident(w: Walk) -> None:
    open_dashboard(w)
    census = w.click_slot(r"^Жители", "dashboard Residents", lambda s: s["kind"] == "menu")
    head = next((slot for slot, item in sorted(census["items"].items())
                 if slot >= 9 and item in ("minecraft:player_head", "minecraft:villager_spawn_egg")), None)
    if head is None:
        raise StepFailed(f"no resident icon on the census page: {census['items']}")
    w.gui.slot(head)
    card = w.settle("resident card", before=signature(census), accept=lambda s: s["kind"] == "menu")
    w.shot("resident")
    text = " ".join([blob(card), *w.lore(card)])
    w.judge("Q7b", bool(re.search(r"на карте", text, re.IGNORECASE)), title=card["title"], text=text[:500])


# ----------------------------------------------------------------------------- certification


def house_fixture(w: Walk, origin: list[int], *, complete: bool) -> None:
    """A 5x5x5 house: stone floor on the ground layer, plank walls two high with open doors as the two
    diagonal corner posts (a closed corner is solid, so nobody could stand on its floor), two solid roof
    layers, and a bed (only when `complete`: the first attempt lacks bed and roof)."""
    hx, ground, hz = origin
    x4, z4 = hx + 4, hz + 4
    fixtures = [
        f"fill {hx} {ground} {hz} {x4} {ground + 5} {z4} minecraft:air",
        f"fill {hx} {ground} {hz} {x4} {ground} {z4} minecraft:stone",
        f"fill {hx} {ground + 1} {hz} {x4} {ground + 2} {hz} minecraft:oak_planks",
        f"fill {hx} {ground + 1} {z4} {x4} {ground + 2} {z4} minecraft:oak_planks",
        f"fill {hx} {ground + 1} {hz} {hx} {ground + 2} {z4} minecraft:oak_planks",
        f"fill {x4} {ground + 1} {hz} {x4} {ground + 2} {z4} minecraft:oak_planks",
        f"fill {hx} {ground + 3} {hz} {x4} {ground + 4} {z4} minecraft:oak_planks",
    ]
    for corner_x, corner_z in ((hx, hz), (x4, z4)):
        fixtures += [
            f"setblock {corner_x} {ground + 1} {corner_z} minecraft:oak_door[half=lower,facing=south,hinge=left,open=true]",
            f"setblock {corner_x} {ground + 2} {corner_z} minecraft:oak_door[half=upper,facing=south,hinge=left,open=true]",
        ]
    for command in fixtures:
        w.fixture(command)
    if complete:
        complete_house(w, origin)
    else:
        for layer in (ground + 3, ground + 4):
            w.fixture(f"setblock {hx + 2} {layer} {hz + 2} minecraft:air")


def complete_house(w: Walk, origin: list[int]) -> None:
    hx, ground, hz = origin
    for layer in (ground + 3, ground + 4):
        w.fixture(f"setblock {hx + 2} {layer} {hz + 2} minecraft:oak_planks")
    w.fixture(f"setblock {hx + 2} {ground + 1} {hz + 1} minecraft:white_bed[facing=south,part=foot]")
    w.fixture(f"setblock {hx + 2} {ground + 1} {hz + 2} minecraft:white_bed[facing=south,part=head]")


def building_list(w: Walk) -> list[dict[str, Any]]:
    lines = w.chat("settlement building list")
    rows = []
    for line in lines:
        match = BUILDING_LIST.search(line)
        if match:
            rows.append({"id": int(match[1]), "kind": match[2].lower(), "level": int(match[3]),
                         "min": [int(match[4]), int(match[5]), int(match[6])],
                         "max": [int(match[7]), int(match[8]), int(match[9])]})
    return rows


def block_certify(w: Walk) -> None:
    x, y, z = w.evidence["pad"]["anchor"]
    ground = y - 1
    origin = [x - 30, ground, z - 30]
    house_fixture(w, origin, complete=False)
    corner1 = [origin[0], origin[2]]
    corner2 = [origin[0] + 4, origin[2] + 4]
    w.evidence["house"] = {"origin": origin, "floor_y": ground, "corner1_xz": corner1, "corner2_xz": corner2}
    # Open the certify form by the chest screens a player uses, then stand on the floor of corner 1.
    open_dashboard(w)
    w.click_slot(r"^Постройки$", "dashboard Buildings", lambda s: s["kind"] == "menu")
    w.click_slot(r"^Сертифицировать", "Buildings Certify", lambda s: s["kind"] == "dialog")
    w.shot("certify-form")
    w.tp(corner1[0] + .5, ground + 1.0, corner1[1] + .5)
    marked = w.click_button(r"Угол\s*1", "form Corner 1", lambda s: s["kind"] == "dialog")
    w.shot("certify-corner1")
    text = blob(marked)
    walk = matching(r"закрыть и|идти|подойти|пройти", marked["buttons"])
    dialog_ok = (bool(re.search(r"-?\d+\s+-?\d+\s+-?\d+", text)) and bool(re.search(r"угл\w*\s*2", text, re.IGNORECASE))
                 and bool(walk))
    chat_ok = False
    chat_text = ""
    if walk:
        w.click_button(re.escape(walk[0]), "form Close and walk", wait=False)
        w.wait_closed()
        chat_text = "\n".join(str(line) for line in w.probe.observe().get("recent_chat", [])[:6])
        chat_ok = bool(re.search(r"Метк\w*\s*1", chat_text)) and bool(re.search(r"-?\d+\s+-?\d+\s+-?\d+", chat_text)) \
            and bool(re.search(r"угл\w*\s*2", chat_text, re.IGNORECASE)) and bool(re.search(r"сертификац", chat_text, re.IGNORECASE))
    w.judge("Q13", dialog_ok and chat_ok, dialog=text[:500], walk_buttons=walk, chat=chat_text[:500])
    if not walk:
        w.close_all()
    # The second corner: walk (teleport) onto its floor and reopen the form with the typed command.
    w.tp(corner2[0] + .5, ground + 1.0, corner2[1] + .5)
    w.probe.report.setdefault("player_commands", []).append("settlement menu certify")
    w.probe.call("minecraft_send_chat", {"message": "settlement menu certify", "command": True})
    w.settle("certify form again", accept=lambda s: s["kind"] == "dialog" and bool(matching(r"Угол\s*2", s["buttons"])))
    w.click_button(r"Угол\s*2", "form Corner 2", lambda s: s["kind"] == "dialog")
    refused = w.click_button(r"^Сертифицировать$", "form Certify (incomplete house)", lambda s: s["kind"] == "dialog")
    w.shot("certify-refused")
    text = " ".join(refused["body"])
    ok, facts = refusal_verdict(text)
    w.judge("Q1", ok, body=text[:600], **facts)
    complete_house(w, origin)
    done = w.click_button(r"^Сертифицировать$", "form Certify (valid house)", lambda s: s["kind"] == "dialog")
    w.shot("certify-done")
    text = " ".join(done["body"])
    rows = [row for row in building_list(w) if row["kind"] == "house"]
    exact = [row for row in rows if row["min"][1] == ground and row["min"][0] == origin[0]
             and row["min"][2] == origin[2] and row["max"][0] == origin[0] + 4 and row["max"][2] == origin[2] + 4]
    w.evidence["house"]["certified"] = exact
    w.judge("Q12", bool(exact) and bool(re.search(r"сертифицирован|засчитан", text, re.IGNORECASE))
            and not re.search(r"Не удалось", text), body=text[:500], server_rows=rows)
    w.judge("Q14", bool(re.search(r"Дальше|Теперь|следующ", text, re.IGNORECASE))
            and bool(matching(r"жител|улучш|назнач", done["buttons"])) and len(done["buttons"]) >= 2,
            body=text[:500], buttons=done["buttons"])


def block_cards(w: Walk) -> None:
    x, y, z = w.evidence["pad"]["anchor"]
    ground = y - 1
    low, high = (x + 12, ground, z - 20), (x + 16, ground + 3, z - 16)
    w.fixture(f"fill {low[0]} {ground} {low[2]} {high[0]} {ground + 5} {high[2]} minecraft:air")
    w.fixture(f"fill {low[0]} {ground} {low[2]} {high[0]} {ground} {high[2]} minecraft:stone")
    w.fixture(f"setblock {x + 14} {ground + 1} {z - 18} minecraft:cartography_table")
    w.fixture(f"settlement building certify cartographer {low[0]} {low[1]} {low[2]} {high[0]} {high[1]} {high[2]}")
    open_dashboard(w)
    listing = w.click_slot(r"^Постройки$", "dashboard Buildings", lambda s: s["kind"] == "menu")
    w.shot("buildings-list")
    workshops = matching(r"Мастерск", [name for name in listing["slots"].values() if name])
    w.judge("Q2", bool(workshops) and not any(re.search(r"Мастерская:?\s*[A-Za-z]", name) for name in workshops),
            names=workshops)
    card = w.click_slot(r"Дом", "Buildings house card", lambda s: s["kind"] == "menu")
    w.shot("building-card")
    mapped = w.click_slot(r"^Карта$", "card Map", lambda s: s["kind"] == "dialog")
    w.shot("building-map")
    back_check(w, "Q6d", "button", r"^Назад$", lambda s: s["kind"] == card["kind"] and s["title"] == card["title"], card)
    del mapped


# ----------------------------------------------------------------------------- guide and docs


def block_guide(w: Walk) -> None:
    lines = w.chat("settlement guide", settle_ticks=40)
    text = "\n".join(lines)
    ok, facts = guide_verdict(text)
    w.judge("Q15a", ok, text=text[:1200], **facts)
    ok, facts = greeting_verdict(w.evidence["join_chat"])
    w.judge("Q15b", ok, **facts)


def block_docs(w: Walk) -> None:
    root = runtime.REPO_ROOT
    menus = (root / "docs/settlements/menus.md").read_text()
    guide = (root / "docs/SETTLEMENTS_GUIDE.md").read_text()
    quick = re.search(r"(?is)##\s*Quick start(.*?)(?=\n##\s|\Z)", guide)
    stale = [needle for needle in ("Assign crafts", "below the legend", "certify a storage building first")
             if needle in menus or needle in guide]
    w.judge("Q16", not stale and bool(quick) and bool(re.search(r"starter", quick.group(1), re.IGNORECASE)),
            stale=stale, quick_start_found=bool(quick))


def block_second_founding(w: Walk) -> None:
    """The same player founds a second settlement at the pad's edge (clear of the first): no second chest."""
    x, y, z = w.evidence["pad"]["anchor"]
    w.close_all()
    w.tp(x - 60 + .5, y, z + .5)
    w.probe.report["player_commands"].append("settlement create qw2 8")
    w.probe.call("minecraft_send_chat", {"message": "settlement create qw2 8", "command": True})
    w.probe.wait(lambda state: any("Founded qw2" in str(line) or "Основан" in str(line) and "qw2" in str(line)
                                   for line in state.get("recent_chat", [])), "QW second settlement founded", timeout_seconds=60)
    w.probe.call("minecraft_wait_ticks", {"ticks": 200})
    page = w.probe.call("minecraft_scan_blocks", {
        "min_x": x - 64, "min_y": y - 3, "min_z": z - 4, "max_x": x - 56, "max_y": y + 3, "max_z": z + 4, "max_blocks": 4096})
    chests = [[row["x"], row["y"], row["z"]] for row in page["blocks"] if row["block_id"] == "minecraft:chest"]
    w.judge("C5", not chests and bool(w.evidence.get("starter_chest")), second_anchor=[x - 60, y, z], chests=chests)


BLOCKS: tuple[tuple[str, tuple[str, ...], Any], ...] = (
    ("create", ("C1",), block_create),
    ("hub", ("Q11a", "Q10a", "Q9"), block_hub),
    ("dashboard", ("Q11b", "Q10b", "Q6a"), block_dashboard),
    ("storage", ("C3", "Q8b", "C2", "C4", "Q6b"), block_storage),
    ("buildings-empty", ("Q8a",), block_buildings_empty),
    ("order", ("Q5", "Q4", "Q3", "Q6c"), block_order),
    ("project", ("Q7", "Q17"), block_project),
    ("resident", ("Q7b",), block_resident),
    ("certify", ("Q13", "Q1", "Q12", "Q14"), block_certify),
    ("cards", ("Q2", "Q6d"), block_cards),
    ("guide", ("Q15a", "Q15b"), block_guide),
    ("docs", ("Q16",), block_docs),
    ("second-founding", ("C5",), block_second_founding),
)
assert {cid for _, ids, _ in BLOCKS for cid in ids} == set(CRITERIA), "every criterion belongs to one block"
