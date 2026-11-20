"""Stage 2 of the D63 settlement walk: certification by standing inside (owner D63, 2026-11-19, answers ~21:10).

Written from the decision text (`docs/settlements/OWNER_DECISIONS.md`, «Management screens and the settlement map —
owner D63»: «the player stands inside and presses «Проверить здесь»; the server finds the walls itself and shows a
Russian ✔/✘ checklist. No two-corner marking») and from Main's stage-2 statement of the rules (kinds, requirements,
the operator-only coordinate command), not from the implementation.

The player of `--scenario stage2` is NOT an operator: every privileged fixture (blocks, teleports, game rules) goes
through the server console (`settlement_quickwins_console`), the owner's own actions are physical GUI input and typed
chat. `--scenario stage2op` is the same player written into ops.json at permission level 2 before the server starts
and judges only the operator branch of the coordinate command.
"""
from __future__ import annotations

import functools
import re
from typing import Any, Callable

from .settlement_quickwins_qa import CRITERIA, StepFailed, Walk, matching, on_window
from . import settlement_quickwins_steps as steps

# Stage-1 criteria that were about marking two floor corners: replaced by the stage-2 criteria below.
OBSOLETE = ("Q1", "Q1b", "Q12", "Q13", "Q14")

NEW: dict[str, tuple[str, str]] = {
    "R0": ("the Buildings page's «Сертифицировать постройку» icon opens a form with a building-type choice and «Проверить здесь»",
           "OWNER_DECISIONS D63 «the player stands inside and presses «Проверить здесь»»"),
    "EH": ("a non-operator owner standing inside a closed House room: Check here shows only ✔, Certify registers a House", "D63; Main stage-2 rules"),
    "ES": ("standing inside a closed Storage room: all ✔, Certify registers a Storage", "D63; Main stage-2 rules"),
    "EW": ("standing inside a closed Workshop (Картограф) room: all ✔, Certify registers a Workshop", "D63; Main stage-2 rules"),
    "EM": ("standing inside a closed Mine room: all ✔, Certify registers a Mine", "D63; Main stage-2 rules"),
    "EL": ("standing inside a closed Lumber camp room: all ✔, Certify registers a Lumber camp", "D63; Main stage-2 rules"),
    "EB": ("standing inside a closed Barracks room (two beds): all ✔, Certify registers a Barracks", "D63; Main stage-2 rules"),
    "ET": ("standing inside a closed Tower room (height 6): all ✔, Certify registers a Tower", "D63; Main stage-2 rules"),
    "OS": ("open Storage: standing within four blocks of a chest, all ✔, Certify registers a Storage there", "Main stage-2 rules"),
    "OW": ("open Workshop (Библиотекарь): standing near its workstation, all ✔, Certify registers a Workshop", "Main stage-2 rules"),
    "OM": ("open Mine: standing near a crafting table, all ✔, Certify registers a Mine", "Main stage-2 rules"),
    "OL": ("open Lumber camp: standing near a barrel, all ✔, Certify registers a Lumber camp", "Main stage-2 rules"),
    "OA": ("a straight continuous wall five long and three high: standing beside it, all ✔, Certify registers a Wall", "Main stage-2 rules"),
    "OG": ("a gate (door in a collision frame): standing in front of it, all ✔, Certify registers a Gate", "Main stage-2 rules"),
    "FS": ("a floor of upper slabs counts as a floor (Storage room)", "Main stage-2 rules «upper slabs count»"),
    "FC": ("a carpet on the floor does not hide the floor below (Storage room, player standing on the carpet)", "Main stage-2 rules «carpet in the feet cell keeps the floor below»"),
    "RF1": ("a House without roof and bed: the checklist shows ✘ крыша 0/1 and ✘ кровати 0/1 in Russian with a repair hint each; no Certify; after the repair all ✔", "D63 Russian ✔/✘ checklist; Main"),
    "RF2": ("a Barracks without a door: ✘ дверь 0/1 with a repair hint; after placing the door all ✔", "D63; Main"),
    "RF3": ("a Tower one layer too low: ✘ высота 5/6 with a repair hint; after raising it all ✔", "D63; Main"),
    "RF4": ("a wall four long: ✘ длина стены 4/5 with a repair hint; after extending it all ✔", "D63; Main"),
    "RF5": ("a gate whose frame corner is missing: ✘ рама ворот 0/1 with a repair hint; after closing it all ✔", "D63; Main"),
    "RF6": ("a House room with an opening in its wall is refused in Russian with what to do; no checklist, no Certify", "Main «open wall = refusal»"),
    "RV": ("moving away after a passing check: the stale Certify registers nothing and a new Check here no longer passes", "Main «revalidation of the current position»"),
    "NO": ("a non-operator typing the coordinate form of /settlement building certify is refused and registers nothing", "Main «operator level 2 only»"),
    "S1": ("the success dialog says what to do next and offers residents/jobs, the next certificate and Back", "REPORT §5.14 (stage-1 Q14, kept)"),
    "N1": ("no corner or height control appears on any certification dialog (the only input is the building type)", "OWNER_DECISIONS D63 «No two-corner marking»"),
    "V1": ("on every certification dialog the building-type input and every button lie inside 1280x720 without scrolling", "Main «buttons visible at 720p»"),
}
STAGE2 = {**{cid: value for cid, value in CRITERIA.items() if cid not in OBSOLETE}, **NEW}
OP = {
    "OP1": ("a player at permission level 2 (ops.json, before the server started) registers a Storage with the coordinate form of /settlement building certify", "Main «operator level 2+»"),
    "OP2": ("the same level-2 player cannot use /op (the fixture is level 2, not 4)", "Main; advisor: console `op` always grants 4"),
}

# Sites lie inside the settlement (radius 48 from the bell): the farthest corner of a 5x5 site is at ~47 blocks.
G_OFFSETS_X = (-32, -22, -12, 12, 22, 32)
G_OFFSETS_Z = (-26, -13, 13, 26)
LIST_ROW = re.compile(r"#(\d+)\s+(.+?)\s+level\s+(\d+)\s+\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]-\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]",
                      re.IGNORECASE)
CHECK_LINE = re.compile(r"^\s*([✔✘])\s*(.+?)\s+(\d+)\s*/\s*(\d+)\s*(?:[—–-]+\s*(.+?))?\s*$")
JARGON = re.compile(r"\b(footprint|beds?|roof|enclosure|entrance|workstations?|containers?|gate frame|continuous wall)\b",
                    re.IGNORECASE)
KIND_LABEL = {
    "house": "Жильё: Дом", "storage": "Склад: Склад", "cartographer": "Производство: Мастерская: Картограф",
    "librarian": "Производство: Мастерская: Библиотекарь", "mine": "Производство: Шахта",
    "lumber": "Производство: Лесопилка", "barracks": "Военное: Казарма", "tower": "Укрепления: Башня",
    "wall": "Укрепления: Секция стены", "gate": "Укрепления: Ворота",
}
KIND_TOKEN = {
    "house": ("house",), "storage": ("storage",), "cartographer": ("workshop", "cartographer"),
    "librarian": ("workshop", "librarian"), "mine": ("mine",), "lumber": ("lumber",), "barracks": ("barracks",),
    "tower": ("tower",), "wall": ("wall",), "gate": ("gate",),
}


# ----------------------------------------------------------------------------- pure helpers


def checklist(text: str) -> list[dict[str, Any]]:
    """The ✔/✘ lines of a dialog body: mark, label, found, required, repair hint (None when absent)."""
    rows = []
    for line in text.splitlines():
        match = CHECK_LINE.match(line)
        if match:
            rows.append({"ok": match[1] == "✔", "label": match[2].strip(), "found": int(match[3]),
                         "required": int(match[4]), "hint": (match[5] or "").strip() or None})
    return rows


def pass_verdict(text: str, buttons: list[str], expect: dict[str, int]) -> tuple[bool, dict[str, Any]]:
    """Every line ✔ with found >= required, the kind's own counters present with their required numbers
    (`expect`: Russian label stem -> required), Certify offered, Russian only."""
    rows = checklist(text)
    present = {stem: [row for row in rows if re.search(stem, row["label"], re.IGNORECASE)] for stem in expect}
    facts = {
        "rows": rows,
        "all_ticked": bool(rows) and all(row["ok"] and row["found"] >= row["required"] for row in rows),
        "expected_counters": {stem: [(row["found"], row["required"]) for row in found] for stem, found in present.items()},
        "counters_ok": all(any(row["required"] == need for row in present[stem]) for stem, need in expect.items()),
        "certify_offered": bool(matching(r"^Сертифицировать$", buttons)),
        "jargon": sorted({m.group(0).lower() for m in JARGON.finditer(text)}),
    }
    return (facts["all_ticked"] and facts["counters_ok"] and facts["certify_offered"] and not facts["jargon"]), facts


def fail_verdict(text: str, buttons: list[str], failing: list[tuple[str, int, int]]) -> tuple[bool, dict[str, Any]]:
    """Each expected failure is a ✘ line with exactly its found/required numbers and a repair hint (a verb phrase
    after the counter); the dialog offers no Certify; no jargon."""
    rows = checklist(text)
    seen = {}
    for stem, found, required in failing:
        hit = [row for row in rows if re.search(stem, row["label"], re.IGNORECASE) and not row["ok"]]
        seen[stem] = {"rows": hit, "numbers_ok": any((row["found"], row["required"]) == (found, required) for row in hit),
                      "hint_ok": any(row["hint"] and len(row["hint"]) >= 8 for row in hit)}
    facts = {
        "rows": rows, "expected": seen,
        "no_certify": not matching(r"^Сертифицировать$", buttons),
        "every_cross_has_hint": all(row["hint"] and len(row["hint"]) >= 8 for row in rows if not row["ok"]),
        "jargon": sorted({m.group(0).lower() for m in JARGON.finditer(text)}),
    }
    ok = (all(item["numbers_ok"] and item["hint_ok"] for item in seen.values()) and facts["no_certify"]
          and facts["every_cross_has_hint"] and not facts["jargon"])
    return ok, facts


def refusal_verdict(text: str, buttons: list[str]) -> tuple[bool, dict[str, Any]]:
    """A discovery refusal: Russian sentence that says what to do (a verb), no checklist rows, no Certify."""
    facts = {
        "russian": len(re.findall(r"[А-Яа-яЁё]", text)) >= 20 and not re.search(r"[A-Za-z]{3,}", text),
        "action": bool(re.search(r"встаньте|закройте|отделите|подойдите|загрузите|выберите", text, re.IGNORECASE)),
        "no_rows": not checklist(text),
        "no_certify": not matching(r"^Сертифицировать$", buttons),
    }
    return all(facts.values()), facts


def corner_or_height_controls(nodes: list[dict[str, Any]]) -> list[str]:
    """Labels of corner/height controls among a dialog's inputs and buttons (D63: none)."""
    found = []
    for node in nodes:
        node_id = str(node.get("id", ""))
        if node_id.startswith(("dialog/input/", "dialog/action")) and not node_id.endswith("/label") \
                and re.search(r"Угол|угл|Высот|corner|height", str(node.get("label", "")), re.IGNORECASE):
            found.append(str(node.get("label")))
    return found


def list_buildings(w: Walk) -> list[dict[str, Any]]:
    rows = []
    for line in w.chat("settlement building list"):
        for match in LIST_ROW.finditer(line):  # one chat line can list several buildings
            rows.append({"id": int(match[1]), "kind": match[2].strip().lower(), "level": int(match[3]),
                         "min": [int(match[4]), int(match[5]), int(match[6])],
                         "max": [int(match[7]), int(match[8]), int(match[9])], "line": line})
    return rows


def contains(row: dict[str, Any], at: tuple[int, int, int]) -> bool:
    return all(row["min"][axis] <= at[axis] <= row["max"][axis] for axis in range(3))


# ----------------------------------------------------------------------------- dialog driving


BASE_BUTTONS = (r"^Проверить здесь$", r"^Назад$")


def note_dialog(w: Walk, label: str, want: tuple[str, ...] = BASE_BUTTONS) -> None:
    """Record the certification dialog as the player sees it: its inputs, buttons and where they lie. `want` are
    the buttons that must be reachable on this screen: a client lists only widgets it shows, so a button that fell
    outside the window is a MISSING entry, not an off-window one."""
    state = w.probe.observe()
    nodes = [node for node in state.get("ui_nodes", []) if str(node.get("id", "")).startswith("dialog/")]
    controls = [node for node in nodes if str(node.get("id", "")).startswith(("dialog/input/", "dialog/action"))
                and not str(node.get("id", "")).endswith("/label") and node.get("role") in ("button", "textbox", "slider")]
    w.evidence.setdefault("dialog_states", []).append({
        "label": label,
        "controls": [{"id": node["id"], "label": node.get("label"), "bounds": node.get("bounds"),
                      "inside_720p": on_window(node)} for node in controls],
        "corner_or_height": corner_or_height_controls(nodes),
        "inputs": [node.get("label") for node in controls if str(node["id"]).startswith("dialog/input/")],
        "missing": [pattern for pattern in want
                    if not any(re.search(pattern, str(node.get("label", ""))) and on_window(node) for node in controls)],
    })


def open_form(w: Walk) -> dict[str, Any]:
    """The player's typed `/settlement menu certify` (the form is also reachable from the Buildings page: R0)."""
    w.close_all()
    w.probe.report.setdefault("player_commands", []).append("settlement menu certify")
    w.probe.call("minecraft_send_chat", {"message": "settlement menu certify", "command": True})
    shown = w.settle("certify form", lambda s: s["kind"] == "dialog" and bool(matching(r"Проверить здесь", s["buttons"])))
    note_dialog(w, "form opened")
    return shown


def kind_node(state: dict[str, Any]) -> dict[str, Any] | None:
    return next((node for node in state.get("ui_nodes", []) if node.get("id") == "dialog/input/kind"), None)


def select_kind(w: Walk, key: str) -> str:
    """Press the building-type input until it reads the wanted kind; returns the label it ended on."""
    want = KIND_LABEL[key]
    for _ in range(24):
        state = w.probe.observe()
        node = kind_node(state)
        if node is None:
            raise StepFailed("the certification dialog has no building-type input")
        label = str(node.get("label", ""))
        if re.match(r"Тип постройки:\s*", label) and label.split(":", 1)[1].strip() == want:
            return label
        x, y, width, height = node["bounds"]
        w.gui.input.click((round(x + width / 2), round(y + height / 2)))
        w.probe.call("minecraft_wait_ticks", {"ticks": 4})
    raise StepFailed(f"the building-type input never read {want!r}")


def press_check(w: Walk, label: str) -> dict[str, Any]:
    shown = w.click_button(r"^Проверить здесь$", label, lambda s: s["kind"] == "dialog")
    note_dialog(w, label)
    return shown


# ----------------------------------------------------------------------------- fixtures


def ground(w: Walk) -> int:
    return int(w.evidence["pad"]["ground_y"])


def site(w: Walk, index: int) -> tuple[int, int]:
    ax, _, az = w.evidence["pad"]["anchor"]
    cols = len(G_OFFSETS_X)
    return ax + G_OFFSETS_X[index % cols], az + G_OFFSETS_Z[index // cols]


def room(w: Walk, sx: int, sz: int, *, roof: bool = True, door: bool = True, tall: bool = False,
         floor: str = "minecraft:stone", extra: tuple[str, ...] = ()) -> None:
    """A 5x5 room: floor on the ground layer, three (tall: five) wall layers, a roof layer, a closed oak door in the
    west wall, then `extra` blocks (beds, a chest, a workstation). Interior 3x3."""
    g = ground(w)
    top = g + (5 if tall else 3)
    x1, z1 = sx + 4, sz + 4
    commands = [
        f"fill {sx} {g} {sz} {x1} {top + 2} {z1} minecraft:air",
        f"fill {sx} {g} {sz} {x1} {g} {z1} {floor}",
        f"fill {sx} {g + 1} {sz} {x1} {top} {z1} minecraft:stone",
        f"fill {sx + 1} {g + 1} {sz + 1} {x1 - 1} {top} {z1 - 1} minecraft:air",
    ]
    if roof:
        commands.append(f"fill {sx} {top + 1} {sz} {x1} {top + 1} {z1} minecraft:stone")
    if door:
        commands += [f"setblock {sx} {g + 1} {sz + 2} minecraft:oak_door[half=lower,facing=west]",
                     f"setblock {sx} {g + 2} {sz + 2} minecraft:oak_door[half=upper,facing=west]"]
    for command in (*commands, *extra):
        w.fixture(command)


def bed(w: Walk, x: int, z: int) -> tuple[str, str]:
    g = ground(w)
    return (f"setblock {x} {g + 1} {z} minecraft:white_bed[facing=south,part=foot]",
            f"setblock {x} {g + 1} {z + 1} minecraft:white_bed[facing=south,part=head]")


def put(w: Walk, x: int, y: int, z: int, block: str) -> str:
    return f"setblock {x} {y} {z} {block}"


def stand_in(w: Walk, sx: int, sz: int, dy: float = 0.0, dz: int = 3) -> tuple[float, float, float]:
    return (sx + 3.5, ground(w) + 1 + dy, sz + dz + .5)


# ----------------------------------------------------------------------------- one certification case


class Case:
    """One kind: its fixture, where the owner stands, the counters its checklist must show, the optional
    broken start (`broken` expected ✘ lines, `repair` fixes it) and the point the registered building must contain."""

    def __init__(self, cid: str, key: str, index: int, expect: dict[str, int], build: Callable[[Walk, int, int], None],
                 stand: Callable[[Walk, int, int], tuple[float, float, float]], point: Callable[[Walk, int, int], tuple[int, int, int]],
                 *, broken: list[tuple[str, int, int]] | None = None, repair: Callable[[Walk, int, int], None] | None = None,
                 certify: bool = True) -> None:
        self.cid, self.key, self.index, self.expect = cid, key, index, expect
        self.build, self.stand, self.point = build, stand, point
        self.broken, self.repair, self.certify = broken, repair, certify


def success_dialog_ok(w: Walk, done: dict[str, Any]) -> tuple[bool, dict[str, Any]]:
    text = " ".join(done["body"])
    facts = {"body": text[:400], "buttons": done["buttons"], "title": done["title"]}
    ok = (bool(re.search(r"сертифицирован", text, re.IGNORECASE)) and bool(re.search(r"Дальше|Теперь|следующ", text, re.IGNORECASE))
          and bool(matching(r"жител", done["buttons"])) and bool(matching(r"Сертификат", done["buttons"]))
          and bool(matching(r"^Назад$", done["buttons"])))
    return ok, facts


def block_case(w: Walk, case: Case) -> None:
    sx, sz = site(w, case.index)
    case.build(w, sx, sz)
    x, y, z = case.stand(w, sx, sz)
    w.tp(x, y, z, tol=.7)
    open_form(w)
    label = select_kind(w, case.key)
    facts: dict[str, Any] = {"site": [sx, sz], "kind_input": label, "stand": [x, y, z]}
    shown = press_check(w, f"{case.cid} Check here")
    w.shot(f"{case.cid.lower()}-check")
    text = "\n".join(shown["body"])
    if case.broken is not None:
        ok, broken = fail_verdict(text, shown["buttons"], case.broken)
        facts["broken"] = broken
        facts["broken_body"] = text[:700]
        if not ok:
            w.judge(case.cid, False, **facts)
            return
        assert case.repair is not None
        case.repair(w, sx, sz)
        shown = press_check(w, f"{case.cid} Check here after repair")
        w.shot(f"{case.cid.lower()}-repaired")
        text = "\n".join(shown["body"])
    ok, passing = pass_verdict(text, shown["buttons"], case.expect)
    note_dialog(w, f"{case.cid} passing check", (r"^Сертифицировать$", *BASE_BUTTONS) if ok or passing["all_ticked"] else BASE_BUTTONS)
    facts["checked"] = passing
    facts["body"] = text[:700]
    if not ok or not case.certify:
        w.judge(case.cid, ok, **facts)
        return
    before = {row["id"] for row in list_buildings(w)}
    done = w.click_button(r"^Сертифицировать$", f"{case.cid} Certify", lambda s: s["kind"] == "dialog", scroll=True)
    note_dialog(w, f"{case.cid} certified", (r"^Назад$",))
    w.shot(f"{case.cid.lower()}-certified")
    rows = [row for row in list_buildings(w) if row["id"] not in before]
    point = case.point(w, sx, sz)
    mine = [row for row in rows if any(token in row["kind"] for token in KIND_TOKEN[case.key]) and contains(row, point)]
    facts.update(new_rows=[f"#{row['id']} {row['kind']} {row['min']}-{row['max']}" for row in rows], registered=bool(mine))
    done_ok, done_facts = success_dialog_ok(w, done)
    facts["success_dialog"] = done_facts
    if case.cid == "EH":
        w.judge("S1", done_ok, **done_facts)
    w.judge(case.cid, bool(mine) and len(rows) == 1, **facts)


def cases() -> list[Case]:
    def room_with(extra_fn: Callable[[Walk, int, int, int], tuple[str, ...]], **opts: Any) -> Callable[[Walk, int, int], None]:
        def build(w: Walk, sx: int, sz: int) -> None:
            room(w, sx, sz, extra=extra_fn(w, sx, sz, ground(w)), **opts)
        return build

    def inside(w: Walk, sx: int, sz: int) -> tuple[float, float, float]:
        return stand_in(w, sx, sz)

    def point_inside(w: Walk, sx: int, sz: int) -> tuple[int, int, int]:
        return (sx + 3, ground(w) + 1, sz + 3)

    def house_extras(w: Walk, sx: int, sz: int, g: int) -> tuple[str, ...]:
        return bed(w, sx + 1, sz + 1)

    def barracks_extras(w: Walk, sx: int, sz: int, g: int) -> tuple[str, ...]:
        return (*bed(w, sx + 1, sz + 1), *bed(w, sx + 2, sz + 1))

    def one(block: str, dx: int = 1, dz: int = 3) -> Callable[[Walk, int, int, int], tuple[str, ...]]:
        return lambda w, sx, sz, g: (put(w, sx + dx, g + 1, sz + dz, block),)

    def open_site(block: str) -> Callable[[Walk, int, int], None]:
        def build(w: Walk, sx: int, sz: int) -> None:
            g = ground(w)
            w.fixture(f"fill {sx} {g + 1} {sz} {sx + 4} {g + 4} {sz + 4} minecraft:air")
            w.fixture(put(w, sx + 2, g + 1, sz + 2, block))
        return build

    def near(w: Walk, sx: int, sz: int) -> tuple[float, float, float]:
        return (sx + 4.5, ground(w) + 1, sz + 2.5)

    def at_block(w: Walk, sx: int, sz: int) -> tuple[int, int, int]:
        return (sx + 2, ground(w) + 1, sz + 2)

    def wall_build(length: int) -> Callable[[Walk, int, int], None]:
        def build(w: Walk, sx: int, sz: int) -> None:
            g = ground(w)
            w.fixture(f"fill {sx - 1} {g + 1} {sz - 1} {sx + 7} {g + 5} {sz + 3} minecraft:air")
            w.fixture(f"fill {sx} {g + 1} {sz} {sx + length - 1} {g + 3} {sz} minecraft:stone")
        return build

    def wall_stand(w: Walk, sx: int, sz: int) -> tuple[float, float, float]:
        return (sx + 2.5, ground(w) + 1, sz + 1.5)

    def wall_point(w: Walk, sx: int, sz: int) -> tuple[int, int, int]:
        return (sx + 2, ground(w) + 1, sz)

    def gate_build(frame: bool) -> Callable[[Walk, int, int], None]:
        def build(w: Walk, sx: int, sz: int) -> None:
            g = ground(w)
            gx, gz = sx + 1, sz + 1
            w.fixture(f"fill {sx} {g + 1} {sz} {sx + 4} {g + 5} {sz + 4} minecraft:air")
            w.fixture(f"fill {gx - 1} {g + 1} {gz} {gx + 1} {g + 3} {gz + 2} minecraft:stone")
            w.fixture(f"fill {gx} {g + 1} {gz} {gx} {g + 2} {gz + 2} minecraft:air")
            w.fixture(f"setblock {gx} {g + 1} {gz + 1} minecraft:oak_door[half=lower,facing=south]")
            w.fixture(f"setblock {gx} {g + 2} {gz + 1} minecraft:oak_door[half=upper,facing=south]")
            if not frame:
                w.fixture(f"setblock {gx - 1} {g + 2} {gz} minecraft:air")
        return build

    def gate_stand(w: Walk, sx: int, sz: int) -> tuple[float, float, float]:
        return (sx + 1.5, ground(w) + 1, sz + .5)

    def gate_point(w: Walk, sx: int, sz: int) -> tuple[int, int, int]:
        return (sx + 1, ground(w) + 1, sz + 2)

    storage_expect = {"сундук|бочк": 1}
    station_expect = {"рабоч": 1}

    def top_slab_floor(w: Walk, sx: int, sz: int, g: int) -> tuple[str, ...]:
        return (f"fill {sx + 1} {g} {sz + 1} {sx + 3} {g} {sz + 3} minecraft:stone_slab[type=top]",
                put(w, sx + 1, g + 1, sz + 3, "minecraft:chest"))

    def carpet_floor(w: Walk, sx: int, sz: int, g: int) -> tuple[str, ...]:
        return (f"fill {sx + 1} {g + 1} {sz + 1} {sx + 3} {g + 1} {sz + 3} minecraft:white_carpet",
                put(w, sx + 1, g + 1, sz + 3, "minecraft:chest"))

    def repair_roof_and_bed(w: Walk, sx: int, sz: int) -> None:
        g = ground(w)
        w.fixture(f"fill {sx} {g + 4} {sz} {sx + 4} {g + 4} {sz + 4} minecraft:stone")
        for command in bed(w, sx + 1, sz + 1):
            w.fixture(command)

    def repair_door(w: Walk, sx: int, sz: int) -> None:
        g = ground(w)
        w.fixture(f"setblock {sx} {g + 1} {sz + 2} minecraft:oak_door[half=lower,facing=west]")
        w.fixture(f"setblock {sx} {g + 2} {sz + 2} minecraft:oak_door[half=upper,facing=west]")

    def raise_tower(w: Walk, sx: int, sz: int) -> None:
        g = ground(w)
        w.fixture(f"fill {sx} {g + 4} {sz} {sx + 4} {g + 5} {sz + 4} minecraft:stone")
        w.fixture(f"fill {sx + 1} {g + 4} {sz + 1} {sx + 3} {g + 5} {sz + 3} minecraft:air")
        w.fixture(f"fill {sx} {g + 6} {sz} {sx + 4} {g + 6} {sz + 4} minecraft:stone")

    def extend_wall(w: Walk, sx: int, sz: int) -> None:
        g = ground(w)
        w.fixture(f"fill {sx + 4} {g + 1} {sz} {sx + 4} {g + 3} {sz} minecraft:stone")

    def close_frame(w: Walk, sx: int, sz: int) -> None:
        w.fixture(f"setblock {sx} {ground(w) + 2} {sz + 1} minecraft:stone")

    return [
        Case("EH", "house", 0, {"кроват": 1, "крыш": 1, "двер": 1}, room_with(house_extras), inside, point_inside),
        Case("ES", "storage", 1, storage_expect, room_with(one("minecraft:chest")), inside, point_inside),
        Case("EW", "cartographer", 2, station_expect, room_with(one("minecraft:cartography_table")), inside, point_inside),
        Case("EM", "mine", 3, station_expect, room_with(one("minecraft:crafting_table")), inside, point_inside),
        Case("EL", "lumber", 4, station_expect, room_with(one("minecraft:crafting_table")), inside, point_inside),
        Case("EB", "barracks", 5, {"кроват": 2, "двер": 1}, room_with(barracks_extras), inside, point_inside),
        Case("ET", "tower", 6, {"высот": 6, "крыш": 1}, room_with(lambda w, sx, sz, g: (), tall=True), inside, point_inside),
        Case("OS", "storage", 7, storage_expect, open_site("minecraft:chest"), near, at_block),
        Case("OW", "librarian", 8, station_expect, open_site("minecraft:lectern"), near, at_block),
        Case("OM", "mine", 9, station_expect, open_site("minecraft:crafting_table"), near, at_block),
        Case("OL", "lumber", 10, station_expect, open_site("minecraft:barrel"), near, at_block),
        Case("OA", "wall", 11, {"длин": 5, "высот": 3}, wall_build(5), wall_stand, wall_point),
        Case("OG", "gate", 12, {"рам": 1, "двер": 1}, gate_build(True), gate_stand, gate_point),
        Case("FS", "storage", 13, storage_expect, room_with(top_slab_floor), inside, point_inside),
        Case("FC", "storage", 14, storage_expect, room_with(carpet_floor),
             lambda w, sx, sz: stand_in(w, sx, sz, dy=0.0625), point_inside),
        Case("RF1", "house", 15, {"кроват": 1, "крыш": 1, "двер": 1}, room_with(lambda w, sx, sz, g: (), roof=False),
             inside, point_inside, broken=[("кроват", 0, 1), ("крыш", 0, 1)], repair=repair_roof_and_bed, certify=False),
        Case("RF2", "barracks", 16, {"кроват": 2, "двер": 1}, room_with(barracks_extras, door=False), inside, point_inside,
             broken=[("двер", 0, 1)], repair=repair_door, certify=False),
        Case("RF3", "tower", 17, {"высот": 6, "крыш": 1}, room_with(lambda w, sx, sz, g: ()), inside, point_inside,
             broken=[("высот", 5, 6)], repair=raise_tower, certify=False),
        Case("RF4", "wall", 18, {"длин": 5, "высот": 3}, wall_build(4), wall_stand, wall_point,
             broken=[("длин", 4, 5)], repair=extend_wall, certify=False),
        Case("RF5", "gate", 19, {"рам": 1, "двер": 1}, gate_build(False), gate_stand, gate_point,
             broken=[("рам", 0, 1)], repair=close_frame, certify=False),
    ]


# ----------------------------------------------------------------------------- the other blocks


def block_route(w: Walk) -> None:
    """The owner's way: dashboard -> Buildings -> «Сертифицировать постройку»."""
    steps.open_dashboard(w)
    w.click_slot(r"^Постройки$", "dashboard Buildings", lambda s: s["kind"] == "menu")
    form = w.click_slot(r"^Сертифицировать", "Buildings Certify", lambda s: s["kind"] == "dialog")
    note_dialog(w, "form from the Buildings page")
    state = w.probe.observe()
    w.shot("route-form")
    w.judge("R0", kind_node(state) is not None and bool(matching(r"Проверить здесь", form["buttons"])),
            title=form["title"], buttons=form["buttons"], kind_input=(kind_node(state) or {}).get("label"))
    w.close_all()


def block_refusal(w: Walk) -> None:
    """RF6: a House room with an opening in the wall, the player standing in line with the opening."""
    sx, sz = site(w, 20)
    g = ground(w)
    room(w, sx, sz, extra=(f"setblock {sx + 4} {g + 1} {sz + 2} minecraft:air", f"setblock {sx + 4} {g + 2} {sz + 2} minecraft:air"))
    w.tp(sx + 3.5, g + 1, sz + 2.5, tol=.7)
    open_form(w)
    select_kind(w, "house")
    shown = press_check(w, "RF6 Check here")
    w.shot("rf6-refused")
    text = "\n".join(shown["body"])
    ok, facts = refusal_verdict(text, shown["buttons"])
    w.judge("RF6", ok, body=text[:500], **facts)


def block_revalidate(w: Walk) -> None:
    """RV: a passing check does not outlive the position it was made at."""
    ax, y, az = w.evidence["pad"]["anchor"]
    sx, sz = site(w, 21)
    g = ground(w)
    room(w, sx, sz, extra=(put(w, sx + 1, g + 1, sz + 3, "minecraft:chest"),))
    inside = stand_in(w, sx, sz)
    away = (ax + 0.5, y, az + 45.5)
    w.tp(*inside, tol=.7)
    open_form(w)
    select_kind(w, "storage")
    first = press_check(w, "RV Check here inside")
    ok_first, passing = pass_verdict("\n".join(first["body"]), first["buttons"], {"сундук|бочк": 1})
    note_dialog(w, "RV passing check", (r"^Сертифицировать$", *BASE_BUTTONS))
    before = list_buildings(w)
    w.tp(*away, tol=.7)
    stale = w.click_button(r"^Сертифицировать$", "RV stale Certify", lambda s: s["kind"] == "dialog")
    note_dialog(w, "RV stale certify")
    after = list_buildings(w)
    stale_text = "\n".join(stale["body"])
    stale_ok = (not re.search(r"сертифицирован", stale_text, re.IGNORECASE) and len(after) == len(before)
                and not any(contains(row, (sx + 3, g + 1, sz + 3)) and "storage" in row["kind"] for row in after))
    w.tp(*inside, tol=.7)
    press_check(w, "RV Check here inside again")
    w.tp(*away, tol=.7)
    moved = press_check(w, "RV Check here away")
    w.shot("rv-moved")
    moved_text = "\n".join(moved["body"])
    moved_ok = not checklist(moved_text) and not matching(r"^Сертифицировать$", moved["buttons"]) \
        and bool(re.search(r"[А-Яа-я]{4}", moved_text))
    w.judge("RV", ok_first and stale_ok and moved_ok, first_pass=ok_first, stale_text=stale_text[:400],
            stale_refused_and_nothing_registered=stale_ok, moved_text=moved_text[:400], moved_ok=moved_ok,
            first=passing)
    w.close_all()


def coordinate_command(w: Walk, index: int) -> tuple[str, tuple[int, int, int]]:
    sx, sz = site(w, index)
    g = ground(w)
    w.fixture(f"fill {sx} {g + 1} {sz} {sx + 4} {g + 5} {sz + 4} minecraft:air")
    w.fixture(put(w, sx + 2, g + 1, sz + 2, "minecraft:chest"))
    w.tp(sx + 4.5, g + 1, sz + 2.5, tol=.7)
    return (f"settlement building certify storage {sx + 1} {g} {sz + 1} {sx + 3} {g + 3} {sz + 3}",
            (sx + 2, g + 1, sz + 2))


def block_nonop(w: Walk) -> None:
    command, point = coordinate_command(w, 22)
    before = list_buildings(w)
    w.probe.report.setdefault("player_commands", []).append(command)
    lines = w.chat(command)
    after = list_buildings(w)
    registered = [row for row in after if row["id"] not in {item["id"] for item in before} and contains(row, point)]
    denied = any(re.search(r"unknown|incomplete|неизвест|permission|operator|оператор|прав", line, re.IGNORECASE)
                 for line in lines)
    w.judge("NO", not registered and denied, reply=lines[:6], registered=[f"#{row['id']} {row['kind']}" for row in registered],
            note="tab-completion denial has no native/Java tool; covered by mc-edge command_tree tests (33/33)")


def block_op(w: Walk) -> None:
    command, point = coordinate_command(w, 0)
    before = list_buildings(w)
    w.probe.report.setdefault("player_commands", []).append(command)
    lines = w.chat(command)
    after = list_buildings(w)
    registered = [row for row in after if row["id"] not in {item["id"] for item in before}
                  and "storage" in row["kind"] and contains(row, point)]
    w.judge("OP1", len(registered) == 1, reply=lines[:6], registered=[f"#{row['id']} {row['kind']}" for row in registered])
    name = w.console.player
    reply = w.chat(f"op {name}")
    levels = [(entry["name"], entry["level"]) for entry in w.evidence.get("operator_fixture") or []]
    denied = any(re.search(r"unknown|incomplete|неизвест", line, re.IGNORECASE) for line in reply)
    w.evidence["ops_json_levels"] = levels
    w.judge("OP2", denied and (name, 2) in levels, reply=reply[:4], ops_json=levels)


def block_dialogs(w: Walk) -> None:
    states = w.evidence.get("dialog_states", [])
    corners = [(state["label"], state["corner_or_height"]) for state in states if state["corner_or_height"]]
    # The forms carry exactly the building-type input; the success dialog carries none.
    many_inputs = [(state["label"], state["inputs"]) for state in states
                   if len(state["inputs"]) != (0 if state["label"].endswith("certified") else 1)]
    w.judge("N1", len(states) >= 20 and not corners and not many_inputs, states=len(states), corner_or_height=corners,
            inputs_not_exactly_one=many_inputs)
    hidden = [(state["label"], [c["label"] for c in state["controls"] if not c["inside_720p"]], state["missing"])
              for state in states if state["missing"] or any(not c["inside_720p"] for c in state["controls"])]
    w.judge("V1", len(states) >= 20 and not hidden, states=len(states), outside_720p_or_missing=hidden)


# ----------------------------------------------------------------------------- blueprint names

BLUEPRINTS = {
    "BL": ("the blueprint list (Construction page) shows only localized names: no raw ids (snake_case, `spanish_*`, "
           "`minecraft:`), no «Unknown»/«Неизвестный», every name in the client language, 26 or more distinct blueprints",
           "Main 2026-11-20 «no raw IDs in the blueprint list» (ru and en)"),
}
SNAKE = re.compile(r"\b[a-z]+_[a-z_]+\b|minecraft:")


def blueprint_names(language: str, names: list[str]) -> tuple[bool, dict[str, Any]]:
    """The icon names of the blueprint list (`<name> L1`): localized words only."""
    base = [re.sub(r"\s+L\d+$", "", name).strip() for name in names]
    word = r"[А-Яа-яЁё' -]+" if language == "ru_ru" else r"[A-Za-z' -]+"
    facts = {
        "language": language, "count": len(base), "distinct": len(set(base)),
        "raw_ids": sorted({name for name in names if SNAKE.search(name)}),
        "unknown": sorted({name for name in base if re.search(r"Unknown|Неизвестн", name, re.IGNORECASE)}),
        "not_in_language": sorted({name for name in base if not re.fullmatch(word, name)}),
        "names": base,
    }
    ok = (facts["distinct"] >= 26 and len(base) == facts["distinct"] and not facts["raw_ids"]
          and not facts["unknown"] and not facts["not_in_language"])
    return ok, facts


def blueprint_walk(w: Walk) -> None:
    """`--scenario blueprints`: the player's `/settlement menu projects` opens the blueprint list; every page is read."""
    names: list[str] = []
    w.close_all()
    w.probe.report.setdefault("player_commands", []).append("settlement menu projects")
    w.probe.call("minecraft_send_chat", {"message": "settlement menu projects", "command": True})
    shown = w.settle("blueprint list", lambda s: s["kind"] == "menu" and any(re.search(r"\bL1$", n) for n in s["slots"].values()))
    for _ in range(6):
        names += [name for name in shown["slots"].values() if re.search(r"\bL1$", name)]
        w.shot(f"blueprints-{len(names)}")
        if not matching(r"^(Next page|Следующая страница)$", list(shown["slots"].values())):
            break
        shown = w.click_slot(r"^(Next page|Следующая страница)$", "blueprint list next page",
                             lambda s: s["kind"] == "menu" and s["slots"] != shown["slots"])
    ok, facts = blueprint_names(w.evidence.get("language", w.probe.report["quickwins"]["language"]), names)
    w.judge("BL", ok, **facts)


# ----------------------------------------------------------------------------- suites


def tab(w: Walk, text: str) -> tuple[str, bool]:
    """Type `text` in the chat line of the Horizon client; return the chat line after Tab and whether the client
    showed suggestions or the argument usage for it before Tab (`chat_suggestions`)."""
    hz = w.probe.hz
    w.close_all()
    hz("key", key="t", action="tap")
    hz("type", text=text)
    hz("wait", ticks=6)
    shown = hz("screen")
    w.evidence.setdefault("tab_screens", []).append({"typed": text, "screen": shown})
    hz("key", key="tab", action="tap")
    hz("wait", ticks=2)
    got = str(hz("screen").get("chat_input", ""))
    hz("key", key="esc", action="tap")
    if hz("screen").get("screen") == "chat":  # Escape first dismisses the suggestions
        hz("key", key="esc", action="tap")
    return got, bool(shown.get("chat_suggestions"))


def block_completion_probe(w: Walk) -> None:
    """Horizon only, recorded and NOT judged: what the client shows for /settlement building certify <kind>
    followed by a space. The coordinates are integer arguments: they offer no Tab suggestion even to a level-2
    operator (`chat_suggestions` stays false), so completion cannot tell the two trees apart; the tree itself is
    covered by the mc-edge test `settlement_certification_bounds_are_only_advertised_to_level_two_operators`."""
    control_in = "/settlement building ce"
    control, control_shown = tab(w, control_in)
    coordinates_in = "/settlement building certify storage "
    after, shown = tab(w, coordinates_in)
    w.evidence["completion_probe"] = {"control": [control_in, control, control_shown],
                                      "coordinates": [coordinates_in, after, shown]}


def suite(scenario: str, client: str = "native") -> tuple[dict[str, tuple[str, str]], tuple[tuple[str, tuple[str, ...], Any], ...]]:
    if scenario == "stage2op":
        probe = (("completion-probe", (), block_completion_probe),) if client == "horizon" else ()
        return OP, (("operator", ("OP1", "OP2"), block_op), *probe)
    one_one = {name: (ids, block) for name, ids, block in steps.BLOCKS}
    blocks: list[tuple[str, tuple[str, ...], Any]] = []
    for name in ("create", "hub", "dashboard", "storage", "buildings-empty", "order", "plurals", "project", "resident"):
        ids, block = one_one[name]
        blocks.append((name, ids, block))
    blocks.append(("route", ("R0",), block_route))
    for case in cases():
        owed = (case.cid, "S1") if case.cid == "EH" else (case.cid,)
        blocks.append((f"case-{case.cid}", owed, functools.partial(block_case, case=case)))
    blocks.append(("refusal", ("RF6",), block_refusal))
    blocks.append(("revalidate", ("RV",), block_revalidate))
    blocks.append(("non-operator", ("NO",), block_nonop))
    if client == "horizon":
        blocks.append(("completion-probe", (), block_completion_probe))
    for name in ("cards", "guide", "docs", "second-founding"):
        ids, block = one_one[name]
        blocks.append((name, ids, block))
    blocks.append(("dialogs", ("N1", "V1"), block_dialogs))
    return STAGE2, tuple(blocks)
