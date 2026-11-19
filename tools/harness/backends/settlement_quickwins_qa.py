"""Owner-path acceptance of the settlement quick wins (owner D63, 2026-11-19).

Written from the decision text (`docs/settlements/OWNER_DECISIONS.md`, «Management screens and the
settlement map — owner D63») and section 5 of `receipts/SettlementUX/REPORT.md`, not from the
implementation. `receipts/SettlementQuickWinsAccept/CRITERIA.md` lists every criterion below with
its source line and whether a real client or a server read decides it.

One Russian-locale native client walks the owner's path on a fresh pad:
`/settlement create` -> the starter chest -> hub -> "next step" -> dashboard -> Storage -> an empty
Buildings list -> order a House (empty storage) -> Russian outcome with the storage link -> Back
-> stock the chest and bind residents -> the active project's icon and status -> certify a small
house by marking its two floor corners while standing on its floor -> Buildings list and map ->
`/settlement guide`. Every GUI step is physical XTest input into the harness-owned Xvfb window;
fixtures (pad, bell, house blocks, teleports, villagers, chest stock) go through the privileged
chat path and are listed in `fixture_commands`. A step records a screenshot and the structured
state it was judged from (`details` of the receipt: `scenario.quickwins`).

Modes. Default (strict): the run stops at the first unmet criterion, the failing assertion is
`QW <id>` and the receipt names it. `--survey`: every block runs, unmet criteria are recorded and
the next block starts from a closed screen; the run fails at the end when any criterion failed.
Survey exists to exercise the whole script against a tree that lacks the changes (the negative
control) so the steps after the first unmet criterion are proven to run; it is never acceptance.
"""
from __future__ import annotations

import ast
import hashlib
import json
import math
import re
import subprocess
import sys
import uuid
from pathlib import Path
from typing import Any, Callable

from .. import runtime
from . import scene_shot
from .native_settlement_storage import plain, since
from .probe_steps import ProbeSteps

NAME = "qw"
RADIUS = 48
DIALOG_MARK = ".screens.dialog."
CONTAINER = "net.minecraft.client.gui.screens.inventory.ContainerScreen"
STAGE = 90  # chest menu slot_count: 54 icons + 36 inventory

# id -> (what is observed, source). The text of CRITERIA.md follows this table.
CRITERIA: dict[str, tuple[str, str]] = {
    "C1": ("after /settlement create a chest block stands next to the anchor", "OWNER_DECISIONS D63 «starter storage chest»; REPORT §6.4"),
    "C2": ("the starter chest is empty (server `data get block`)", "OWNER_DECISIONS D63 (only the chest block is free)"),
    "C3": ("the Storage page counts exactly one container with nothing in it", "OWNER_DECISIONS D63 «registered the same way»; REPORT §1.1"),
    "C4": ("a Shift-click deposit lands in that chest (Storage count and server `data get block`)", "OWNER_DECISIONS D63 «works as storage»"),
    "C5a": ("a second CONCURRENT settlement founded by the same player gets its own empty starter chest", "Main reading of D63, 2026-11-20 (open to owner correction): chest per settlement owned, not once per player"),
    "C5b": ("dissolving a settlement and founding again gets no new chest (the player's chest count does not grow)", "Main reading of D63, 2026-11-20 (open to owner correction)"),
    "Q1": ("a refused certification shows EVERY missing requirement in the visible text in Russian with its count (`кровати 0/1, крыша 0/1`), no `footprint/beds/roof/...`; the what-to-do hint may live in the tooltip (not observable: human review)", "REPORT §5.1"),
    "Q2": ("workshop names in the Russian UI have no raw `{profession:?}` (no `Мастерская: Cartographer`)", "REPORT §5.2"),
    "Q3": ("the order outcome is Russian, names the missing materials and links to Storage", "REPORT §5.3"),
    "Q1b": ("after a long refusal the certify form's Сертифицировать / Угол 1 / Угол 2 / Назад lie fully inside 1280x720 without scrolling", "Main, 2026-11-20 (certification refusal text bounded); R:350"),
    "Q6e": ("the map dialog's Назад lies fully inside 1280x720 without scrolling", "Main, 2026-11-20 (map body is the grid only); R:355"),
    "Q4": ("the materials estimate lists item names, not `minecraft:` ids, and what the storage holds", "REPORT §5.4"),
    "Q5": ("the order review is two plain sentences, not the four fragments", "REPORT §5.5"),
    "Q6a": ("Dashboard -> Back returns to the hub it came from", "REPORT §5.6, §1.5"),
    "Q6b": ("hub -> Dashboard -> Storage -> Back returns to the Dashboard", "REPORT §5.6, §1.5"),
    "Q6c": ("order outcome -> Storage link -> Back returns to the outcome", "REPORT §5.6, §1.5; brief"),
    "Q6d": ("building card -> Map -> Back returns to the building card", "REPORT §5.6, §1.4"),
    "Q7": ("a click on the active project's icon opens its card/status, not the map", "REPORT §5.7"),
    "Q7b": ("a resident's Home/Work icons are captioned «на карте»", "REPORT §5.7"),
    "Q8a": ("an empty Buildings list says what to do (Строительство / Сертифицировать)", "REPORT §5.8"),
    "Q8b": ("an empty Storage page says to bring materials", "REPORT §5.8"),
    "Q9": ("hub «Следующий шаг» in an empty settlement does not open the empty Buildings list", "REPORT §5.9"),
    "Q10a": ("the hub has no «Соседи и война» button", "REPORT §5.10"),
    "Q10b": ("the dashboard has no «Соседи и война» and no «Чек-лист» icon", "REPORT §5.10"),
    "Q11a": ("the hub's management button is «Управление», not «Поселение»", "REPORT §5.11"),
    "Q11b": ("the dashboard names the construction icon «Строительство», no «Стройка»", "REPORT §5.11"),
    "Q12": ("standing on the floor, Corner 1 + Corner 2 + Certify certify a valid house", "REPORT §5.12; OWNER_DECISIONS D63 «the corner taken from the floor»"),
    "Q13": ("after Corner 1 the player is told the mark, how to continue, and offered a close-and-walk button", "REPORT §5.13"),
    "Q14": ("the success screen says what to do next, not an empty Buildings/Back", "REPORT §5.14"),
    "Q15a": ("/settlement guide is a short numbered Russian list in task order", "REPORT §5.15"),
    "Q15b": ("the join greeting is one line in the client language", "REPORT §5.15"),
    "Q16": ("docs/settlements/menus.md and the guide's quick start match the chest/storage-first UI", "REPORT §5.16"),
    "Q17": ("the project status names the builder, no uuid and no `Native supply cause`", "REPORT §5.17"),
    "PL": ("the order outcome's omitted-kind counter agrees with its number when 4, 5, 6 and 8 kinds are missing: «и ещё 1 вид / 2 вида / 3 вида / 5 видов»", "QwStage1Fixes 75f9625f0 (Russian one/few/many); Main 2026-11-20"),
}
VERDICTS = tuple(CRITERIA) + ("all",)


class StepFailed(RuntimeError):
    """A GUI step could not be walked (a button or screen the criterion needs is missing)."""


# ----------------------------------------------------------------------------- pure helpers


def on_window(node: dict[str, Any], size: tuple[int, int] = (1280, 720)) -> bool:
    """A widget a click can reach: its whole box lies inside the window (no scrolling in this scenario)."""
    x, y, width, height = node.get("bounds", (-1, -1, 0, 0))
    return (x >= 0 and y >= 0 and width > 0 and height > 0 and x + width <= size[0] + 1 and y + height <= size[1] + 1
            and not node.get("clipped"))


def java_nodes(state: dict[str, Any]) -> list[dict[str, Any]]:
    """The vanilla Java client's dialog widgets (the observer's `screen.widgets`, GUI-scaled boxes) as the
    native client's `ui_nodes`: physical-pixel `bounds`, `role` button/text/input, `dialog/*` ids. A widget
    outside the scroll area that holds it is `clipped` (a click cannot reach it without scrolling)."""
    screen = state.get("screen") or {}
    scale = int(screen.get("gui_scale") or 3)
    nodes: list[dict[str, Any]] = []
    counts = {"title": 0, "body": 0, "action": 0, "input": 0}
    for widget in screen.get("widgets", []):
        simple = str(widget["class"]).rsplit(".", 1)[-1]
        text = str(widget.get("text") or "")
        if not widget.get("visible") or "ScrollableLayout" in simple or "Scroll" in simple and not text:
            continue
        is_input = bool(widget.get("edit_box")) or "Cycle" in simple or "Slider" in simple
        is_button = "Button" in simple and not is_input
        if is_button and "Sprite" in simple:  # the dialog's warning icon, not an action
            continue
        if not (is_input or is_button) and (not text or text == "warning"):
            continue
        if is_input:
            role, kind = "input", "input"
        elif is_button:
            role, kind = "button", "action"
        else:
            role, kind = "text", "title" if text == screen.get("title") and not counts["title"] else "body"
        x, y, width, height = (int(widget[key]) for key in ("x", "y", "width", "height"))
        clip = widget.get("clip")
        clipped = bool(clip) and not (x >= clip["x"] and y >= clip["y"] and x + width <= clip["x"] + clip["width"]
                                      and y + height <= clip["y"] + clip["height"])
        nodes.append({"id": f"dialog/{kind}/{counts[kind]}", "role": role, "label": text,
                      "enabled": bool(widget.get("active", True)), "clipped": clipped,
                      "bounds": (x * scale, y * scale, width * scale, height * scale)})
        counts[kind] += 1
    return nodes


def view(state: dict[str, Any]) -> dict[str, Any]:
    """The open screen of an observation: kind, title, dialog body/buttons or chest-icon names."""
    screen = state.get("screen") or {}
    nodes = state.get("ui_nodes") or []
    cls = str(screen.get("class", ""))
    kind = "none"
    if screen.get("open"):
        kind = "dialog" if DIALOG_MARK in cls else "menu" if cls == CONTAINER else "other"
    body = [str(node["label"]) for node in nodes
            if str(node.get("id", "")).startswith("dialog/body") and node.get("label")]
    buttons = [str(node["label"]) for node in nodes
               if node.get("role") == "button" and node.get("enabled")
               and str(node.get("id", "")).startswith("dialog/action") and on_window(node)]
    container = state.get("container") or {}
    slots = {int(row["slot"]): str(row.get("name") or "") for row in container.get("slots", [])
             if int(row["slot"]) < 54}
    items = {int(row["slot"]): str(row.get("item_id") or "") for row in container.get("slots", [])
             if int(row["slot"]) < 54}
    ready = (kind == "dialog" and bool(body or buttons)) or (
        kind == "menu" and container.get("slot_count") == STAGE and bool(slots))
    return {"kind": kind, "title": str(screen.get("title") or ""), "body": body, "buttons": buttons,
            "slots": slots, "items": items, "ready": ready}


def signature(shown: dict[str, Any]) -> tuple[Any, ...]:
    return (shown["kind"], shown["title"], tuple(shown["body"]), tuple(shown["buttons"]),
            tuple(sorted(shown["slots"].items())))


def blob(shown: dict[str, Any], extra: list[str] | None = None) -> str:
    """All text of a screen: title, dialog body, button labels, chest icon names and extra lore."""
    return "\n".join([shown["title"], *shown["body"], *shown["buttons"],
                      *(name for name in shown["slots"].values() if name), *(extra or [])])


def matching(pattern: str, labels: list[str]) -> list[str]:
    return [label for label in labels if re.search(pattern, label, re.IGNORECASE)]


LATIN_ALLOWED = re.compile(r"/\w+(?:\s+[a-z<>\[\]|-]+)*|\b(?:Shift|Ctrl|Esc|OK)\b", re.IGNORECASE)


def cyrillic_only(text: str, allowed: str = "") -> bool:
    """Russian prose: at least five Cyrillic letters and no Latin word once `allowed`, typed commands
    (`/settlement create`) and key names (Shift) are removed."""
    rest = LATIN_ALLOWED.sub("", text.replace(allowed, "") if allowed else text)
    return len(re.findall(r"[А-Яа-яЁё]", rest)) >= 5 and not re.search(r"[A-Za-z]{2,}", rest)


JARGON = re.compile(r"\b(footprint|height|beds?|roof|enclosure|entrance|workstations?|containers?|"
                    r"gate frame|continuous wall|wall (length|height))\b", re.IGNORECASE)
UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", re.IGNORECASE)
STEP_LINE = re.compile(r"^\s*(?:\d+[.)]|[•*-])\s*\S", re.MULTILINE)


def guide_verdict(text: str) -> tuple[bool, dict[str, Any]]:
    """`/settlement guide`: 5..7 short numbered Russian steps whose headings (the words before the first
    colon or question mark) run storage -> order/construction -> certification -> residents, no old wall of text."""
    lines = [line for line in text.splitlines() if STEP_LINE.match(line)]
    headings = [re.split(r"[:?]", line, maxsplit=1)[0] for line in lines]
    topics = (r"склад", r"заказ|строительств|стройк", r"сертифиц|построили сам", r"жител")
    where = [next((index for index, heading in enumerate(headings) if re.search(topic, heading, re.IGNORECASE)), -1)
             for topic in topics]
    facts = {
        "steps": len(lines),
        "chars": len(text),
        "cyrillic_only": cyrillic_only(text, "/settlement guide /settlement"),
        "task_step_order": where,
        "old_text": bool(re.search(r"Settlements /|composter=|Villagers work without|SETTLEMENTS_GUIDE", text)),
    }
    ok = (5 <= len(lines) <= 7 and len(text) <= 1100 and facts["cyrillic_only"]
          and all(index >= 0 for index in where) and where == sorted(set(where))
          and not facts["old_text"])
    return ok, facts


def greeting_verdict(lines: list[str]) -> tuple[bool, dict[str, Any]]:
    """The join greeting: exactly one chat line that names `/settlement guide`, in Russian only."""
    found = [line for line in lines if "/settlement guide" in line]
    facts: dict[str, Any] = {"lines": found}
    ok = len(found) == 1 and cyrillic_only(found[0], "/settlement guide")
    return ok, facts


ESTIMATE_LINE = re.compile(r"^\s*(.+?) ×(\d+) · на складе (\d+)\s*$", re.MULTILINE)
LISTED_ITEM = re.compile(r"([^\n,:;×.]+?)\s×(\d+)")
MORE_KINDS = re.compile(r"и ещё (\d+) (вид|вида|видов)\b")


def kind_word(n: int) -> str:
    """Russian one/few/many of `вид` for the count `n` (11..14 are always many)."""
    if 11 <= n % 100 <= 14:
        return "видов"
    return "вид" if n % 10 == 1 else "вида" if 2 <= n % 10 <= 4 else "видов"


def expected_shortage(estimate_text: str) -> dict[str, int]:
    """name -> still missing, from the materials estimate pages (need minus what the storage holds)."""
    missing = {}
    for name, need, have in ESTIMATE_LINE.findall(estimate_text):
        if int(need) > int(have):
            missing[name.strip()] = int(need) - int(have)
    return missing


def shortage_verdict(text: str, buttons: list[str], expected: dict[str, int]) -> tuple[bool, dict[str, Any]]:
    """The order outcome: Russian, lists 1..3 of the really missing items with their exact missing counts,
    says `и ещё N видов` for the rest (N = kinds not listed) and links to Storage.
    `expected` is name -> missing count taken from the materials estimate the player saw."""
    body = re.sub(r"Дом ур\. \d+|#\d+", "", text)  # order id / level are not materials
    listed = [(name.strip(), int(count)) for name, count in LISTED_ITEM.findall(body)]
    more = MORE_KINDS.search(text)
    rest = len(expected) - len(listed)
    facts = {
        "missing": bool(re.search(r"не хватает|нужно ещё|недостаёт", text, re.IGNORECASE)),
        "listed": listed,
        "expected_kinds": len(expected),
        "raw_ids": bool(re.search(r"minecraft:|not enough|queued|\bx\d+\b", text, re.IGNORECASE)),
        "storage_link": bool(matching(r"склад", buttons)),
        "russian": cyrillic_only(text),  # item names included: a client that lost its translations shows English
        "russian_ignoring_latin": cyrillic_only(re.sub(r"[A-Za-z]+\d*", "", text) or text),
        "items_exact": bool(listed) and len(listed) <= 3 and all(expected.get(name) == count for name, count in listed)
                       and len({name for name, _ in listed}) == len(listed),
        "more_rule": (more is None and rest == 0) or (more is not None and rest > 0 and int(more.group(1)) == rest
                                                       and more.group(2) == kind_word(rest)),
        "more_word": more.group(2) if more else None,
    }
    ok = (facts["missing"] and facts["items_exact"] and facts["more_rule"] and not facts["raw_ids"]
          and facts["storage_link"] and facts["russian"])
    return ok, facts


def refusal_verdict(text: str) -> tuple[bool, dict[str, Any]]:
    """A refused certification of a house without bed and roof: the VISIBLE text names every missing
    requirement in Russian with its count (`кровати 0/1, крыша 0/1`) and has no jargon.
    `hint` (a verb saying what to do) is recorded but not required: the compact design keeps the
    explanation in the hover/tooltip, which the client's UI nodes do not expose (human review)."""
    facts = {
        "bed": bool(re.search(r"кроват\w*\W{0,3}\d+\s*/\s*\d+", text, re.IGNORECASE)),
        "roof": bool(re.search(r"(крыш|потол)\w*\W{0,3}\d+\s*/\s*\d+", text, re.IGNORECASE)),
        "jargon": sorted({match.group(0).lower() for match in JARGON.finditer(text)}),
        "hint": bool(re.search(r"постав|добав|постро|нужн|положит|достро", text, re.IGNORECASE)),
    }
    return facts["bed"] and facts["roof"] and not facts["jargon"], facts


def corner_text(position: list[int]) -> str:
    return " ".join(map(str, position))


# ----------------------------------------------------------------------------- the walk


class Walk:
    """One real-client session: physical input, fixtures through chat, judged criteria."""

    def __init__(self, probe: ProbeSteps, artifact: Path, gui: Any, args: Any) -> None:
        self.probe, self.artifact, self.gui = probe, artifact, gui
        self.strict = not args.survey
        self.survey = args.survey
        self.shots: list[str] = []
        self.stuck = 0
        self.results: dict[str, dict[str, Any]] = {}
        self.blocks: dict[str, str] = {}
        self.fixture_log = probe.report["fixture_commands"]
        probe.report["quickwins"] = {
            "mode": "survey" if args.survey else "strict", "language": args.language,
            "criteria": self.results, "blocks": self.blocks, "evidence": {},
        }
        self.evidence = probe.report["quickwins"]["evidence"]

    # ---- recording
    def judge(self, cid: str, ok: bool, **facts: Any) -> bool:
        if self.evidence.get("survey_fixture_chest") and cid in ("C2", "C3", "C4", "Q8b", "Q8a", "Q9"):
            facts["fixture_chest"] = True  # survey on a tree without the starter chest: not the tree's chest
        self.results[cid] = {"ok": bool(ok), "what": CRITERIA[cid][0], "source": CRITERIA[cid][1], **facts}
        if self.strict:
            self.probe.check(f"QW {cid}", bool(ok))
        else:
            self.probe.report["assertions"][f"QW {cid}"] = bool(ok)
        return bool(ok)

    def shot(self, name: str) -> str:
        self.probe.call("minecraft_wait_ticks", {"ticks": 2})
        path = self.gui.capture(self.probe, self.artifact, f"qw-{name}")
        self.shots.append(path)
        return path

    # ---- screens
    def screen(self) -> dict[str, Any]:
        return view(self.probe.observe())

    def settle(self, label: str, accept: Callable[[dict[str, Any]], bool] | None = None,
               before: tuple[Any, ...] | None = None, timeout: float = 45.0) -> dict[str, Any]:
        """Wait for a ready screen that is not the one the click left (and passes `accept`)."""
        def ready(state: dict[str, Any]) -> bool:
            shown = view(state)
            return (shown["ready"] and (before is None or signature(shown) != before)
                    and (accept is None or accept(shown)))
        try:
            shown = view(self.probe.wait(ready, f"QW screen {label}", timeout_seconds=timeout))
            for _ in range(20 if self.evidence.get("client") in ("java", "horizon") else 0):
                # Java refreshes the chest page (item updates) before it opens the screen a click leads to:
                # a screen counts as reached once it stays the same for 8 ticks (a recorded Java difference).
                self.probe.call("minecraft_wait_ticks", {"ticks": 8})
                again = self.screen()
                if again["ready"] and signature(again) == signature(shown):
                    break
                self.evidence["java_settle_refreshes"] = self.evidence.get("java_settle_refreshes", 0) + 1
                shown = again if again["ready"] else shown
            return shown
        except TimeoutError as error:
            self.probe.remaining()  # the scenario's own deadline is a failure of the run, not a step
            raise StepFailed(f"screen not reached: {label} ({error})") from error

    @staticmethod
    def button_nodes(state: dict[str, Any], pattern: str) -> list[dict[str, Any]]:
        return [node for node in state.get("ui_nodes", []) if node.get("role") == "button"
                and node.get("enabled") and str(node.get("id", "")).startswith("dialog/action")
                and on_window(node) and re.search(pattern, str(node.get("label", "")), re.IGNORECASE)]

    def click_button(self, pattern: str, label: str, accept: Callable[[dict[str, Any]], bool] | None = None,
                     wait: bool = True, scroll: bool = False) -> dict[str, Any]:
        """Press a dialog button. `scroll=True` (never the default) scrolls a too-tall dialog to its action
        and records that it had to: used only where a screen is judged elsewhere on that count."""
        state = self.probe.observe()
        shown = view(state)
        nodes = self.button_nodes(state, pattern)
        scrolls = 0
        while scroll and self.survey and not nodes and shown["kind"] == "dialog" and scrolls < 4:
            self.gui.input.wheel((640, 360), down=True, notches=3)
            self.probe.call("minecraft_wait_ticks", {"ticks": 4})
            scrolls += 1
            state = self.probe.observe()
            nodes = self.button_nodes(state, pattern)
        if scrolls:
            self.evidence.setdefault("scrolled_to_button", []).append({"label": label, "scrolls": scrolls, "found": bool(nodes)})
            shown = view(state)  # what the press leaves is judged against the scrolled screen
        if not nodes:
            self.probe.report["assertions"][f"QW nav {label}"] = False
            self.stuck += 1
            self.shot(f"stuck-{self.stuck}")
            raise StepFailed(f"no button /{pattern}/ for {label}; buttons={shown['buttons']}")
        # An exact label beats a partial one; among equals the lowest on screen (actions sit below the body).
        exact = [node for node in nodes if re.fullmatch(pattern, str(node["label"]), re.IGNORECASE)]
        node = max(exact or nodes, key=lambda candidate: candidate["bounds"][1])
        x, y, width, height = node["bounds"]
        point = (round(x + width / 2), round(y + height / 2))
        self.probe.report.setdefault("gui_clicks", []).append({"label": node["label"], "for": label, "point": point})
        self.gui.input.click(point)
        return self.settle(label, accept, signature(shown)) if wait else shown

    def click_slot(self, pattern: str, label: str, accept: Callable[[dict[str, Any]], bool] | None = None,
                   wait: bool = True) -> dict[str, Any]:
        shown = self.screen()
        found = [slot for slot, name in sorted(shown["slots"].items()) if re.search(pattern, name, re.IGNORECASE)]
        if not found:
            self.probe.report["assertions"][f"QW nav {label}"] = False
            raise StepFailed(f"no chest icon /{pattern}/ for {label}; icons={sorted(shown['slots'].values())}")
        self.gui.slot(found[0])
        self.probe.report.setdefault("gui_clicks", []).append({"slot": found[0], "for": label})
        return self.settle(label, accept, signature(shown)) if wait else shown

    def lore(self, shown: dict[str, Any]) -> list[str]:
        """Lore lines of every icon on a chest page (`minecraft_read_item_components`)."""
        lines = []
        for slot in sorted(shown["slots"]):
            components = self.probe.call("minecraft_read_item_components",
                                         {"source": "container", "slot": slot}).get("components", {})
            lines.extend(plain(line) for line in components.get("minecraft:lore", []))
        return lines

    def wait_closed(self) -> None:
        self.probe.wait(lambda state: view(state)["kind"] == "none", "QW screen closed", timeout_seconds=30)

    def close_all(self) -> None:
        for _ in range(4):
            if view(self.probe.observe())["kind"] == "none":
                return
            self.gui.input.tap("Escape")
            self.probe.call("minecraft_wait_ticks", {"ticks": 4})
        if view(self.probe.observe())["kind"] != "none":
            raise StepFailed("could not close the open screens with Escape")

    def hub(self) -> dict[str, Any]:
        """Close everything and open the hub with a physical right-click on the bell."""
        from .settlement_map_qa import _enter_hub
        self.close_all()
        pad = self.evidence["pad"]
        stand = pad["stand"]
        player = self.probe.observe()["player"]
        if abs(player["x"] - stand[0]) > .3 or abs(player["z"] - stand[2]) > .3 or abs(player["y"] - stand[1]) > .3:
            self.tp(*stand, tol=.7)
        title = f"Поселение {NAME}"
        for attempt in range(3):
            try:
                _enter_hub(self.probe, self.gui, pad["anchor"], title)
                return self.screen()
            except TimeoutError:
                # A wandering villager can stand in the crosshair ray and take the right-click (seen: a
                # trade screen opened instead of the hub). Close whatever opened and click the bell again.
                self.probe.report["assertions"].pop(f"GUI dialog {title}", None)
                self.evidence.setdefault("hub_retries", []).append(
                    {"attempt": attempt, "screen": self.screen()["title"]})
                self.close_all()
                self.tp(*stand, tol=.7)
        raise StepFailed("the bell did not open the hub in three tries")

    # ---- fixtures and server reads
    def fixture(self, command: str) -> None:
        self.probe.fixture(command)

    def tp(self, x: float, y: float, z: float, tol: float = .05) -> None:
        """Teleport and wait until the client has the player there (`tol` blocks in x and z; y stays exact:
        villagers can push a standing player sideways, never up)."""
        self.fixture(f"tp @s {x} {y} {z}")
        self.probe.wait(lambda state: abs(state["player"]["x"] - x) <= tol and abs(state["player"]["y"] - y) < .05
                        and abs(state["player"]["z"] - z) <= tol, f"QW player at {x} {y} {z}", timeout_seconds=30)

    def chat(self, message: str, command: bool = True, settle_ticks: int = 20) -> list[str]:
        """Send `message` after a fresh chat marker; return the lines that arrived after it, oldest first."""
        marker = f"qw {uuid.uuid4().hex[:8]}"
        self.probe.call("minecraft_send_chat", {"message": marker, "command": False})
        self.probe.wait(lambda state: any(marker in str(line) for line in state.get("recent_chat", [])),
                        f"QW marker {marker}", timeout_seconds=30)
        self.probe.call("minecraft_send_chat", {"message": message, "command": command})
        self.probe.call("minecraft_wait_ticks", {"ticks": settle_ticks})
        lines = since([str(line) for line in self.probe.observe().get("recent_chat", [])], marker)
        return list(reversed(lines))

    def look_at(self, at: list[int]) -> None:
        """Aim the camera at a block centre (for a screenshot of a chest in the world)."""
        player = self.probe.observe()["player"]
        dx, dy, dz = at[0] + .5 - player["x"], at[1] + .5 - (player["y"] + 1.62), at[2] + .5 - player["z"]
        self.probe.call("minecraft_look", {"yaw_deg": round(math.degrees(math.atan2(-dx, dz))),
                                           "pitch_deg": round(-math.degrees(math.atan2(dy, math.hypot(dx, dz))))})
        self.probe.call("minecraft_wait_ticks", {"ticks": 6})

    def chest_items(self, at: list[int]) -> str:
        return "\n".join(self.chat(f"data get block {at[0]} {at[1]} {at[2]} Items"))


# ----------------------------------------------------------------------------- entry


MAP_LABELS = {"ru_ru": "Строительство", "en_us": "Construction"}


def map_label_walk(w: Walk, args: Any) -> None:
    """`--scenario maplabel` (Java): the map dialog's construction action shows its whole label at the GUI scale
    the owner plays with (a vanilla button scrolls a label wider than its slot minus 2 px per side, so the
    player sees a moving substring). Judged by the exact bitmap of the whole label in the rendered frame."""
    from .settlement_map_qa import _label_bitmap, _label_points
    want = MAP_LABELS[args.language]
    w.probe.call("minecraft_send_chat", {"message": "settlement menu map", "command": True})  # the player's own command
    w.settle("map dialog", lambda s: s["kind"] == "dialog" and bool(re.match(r"(Карта|Map)\b", s["title"])))
    w.probe.call("minecraft_wait_ticks", {"ticks": 20})
    state = w.probe.observe()
    nodes = [node for node in state.get("ui_nodes", []) if node.get("role") == "button" and node.get("label") == want]
    frame = w.shot(f"map-label-{args.language}-scale{args.gui_scale}")
    bitmap = _label_bitmap(want, w.gui.jar)
    points = _label_points(frame, bitmap, w.gui.scale)
    node = nodes[0] if nodes else None
    width_px = node["bounds"][2] // w.gui.scale if node else 0
    facts = {"label": want, "gui_scale": args.gui_scale, "language": args.language,
             "buttons": [n["label"] for n in state.get("ui_nodes", []) if n.get("role") == "button"],
             "slot_width_gui_px": width_px, "label_width_gui_px": bitmap.width, "whole_label_in_frame": bool(points),
             "node_clipped": bool(node and node.get("clipped")), "on_window": bool(node and on_window(node))}
    ok = bool(node) and facts["on_window"] and not facts["node_clipped"] and facts["whole_label_in_frame"]
    w.results["ML"] = {"ok": ok, "what": f"the map's «{want}» action shows its whole label at GUI scale {args.gui_scale}",
                       "source": "QwStage1Fixes 75f9625f0; breaker P2", **facts}
    w.probe.check("QW ML", ok)


def run(ctx: Any, args: Any) -> dict[str, Any]:
    """`settlement-map-qa --scenario quickwins`: one ru_ru session (native or vanilla Java) on the lifecycle-owned window."""
    if ctx.client not in ("native", "java"):
        raise ValueError("--scenario quickwins requires --client native or --client java (a client that exposes dialog text)")
    maplabel = args.scenario == "maplabel"
    if maplabel and ctx.client != "java":
        raise ValueError("--scenario maplabel requires --client java (GUI scale and language are Java options)")
    if not maplabel and args.language != "ru_ru":
        raise ValueError("--scenario quickwins judges Russian text: pass -- --language ru_ru")
    java = ctx.client == "java"

    def script(probe: ProbeSteps, artifact: Path, _steps: list[dict[str, Any]], client: str) -> list[str]:
        from . import settlement_quickwins_steps as walk_steps
        from .settlement_map_qa import Gui
        size = (1280, 720)
        if java:
            # The Java observer lists the open screen's widgets; give them the native `ui_nodes` shape.
            call_tool = probe.client.call_tool

            def observed(name: str, arguments: dict[str, Any] | None = None) -> dict[str, Any]:
                result = call_tool(name, arguments)
                if name == "minecraft_observe" and isinstance(result, dict):
                    result["ui_nodes"] = java_nodes(result)
                return result
            probe.client.call_tool = observed
        else:
            scene_shot.resize_window(probe.client.p4_window, size)
        if java:
            # A Java client that could not load its language pack silently shows en_us: the run is not Russian evidence.
            broken = [log for log in ctx.logs if "Failed to load translations" in Path(log).read_text(errors="replace")]
            probe.report["translations_loaded"] = not broken
            probe.check("Java translations loaded (no 'Failed to load translations' in client.log)", not broken)
        gui = Gui(probe, artifact, client, args.gui_scale if maplabel else 3, size)
        walk = Walk(probe, artifact, gui, args)
        walk.evidence["client"] = ctx.client
        if maplabel:
            try:
                walk_steps.setup(walk)
                map_label_walk(walk, args)
            finally:
                gui.close()
                ctx.client_evidence["maplabel"] = walk.results.get("ML")
            return walk.shots
        try:
            walk_steps.setup(walk)
            for name, criteria, block in walk_steps.BLOCKS:
                walk_steps.run_block(walk, name, criteria, block)
            unmet = [cid for cid in CRITERIA if not walk.results.get(cid, {}).get("ok")]
            probe.check("QW all", not unmet)
        finally:
            gui.close()
            order = [cid for _, ids, _ in walk_steps.BLOCKS for cid in ids]
            first = next((cid for cid in order if not walk.results.get(cid, {}).get("ok")), None)
            ctx.client_evidence["quickwins"] = {
                "mode": "survey" if args.survey else "strict",
                "first_unmet": first,
                "criteria": {cid: walk.results.get(cid, {}).get("ok") for cid in CRITERIA},
                "blocks": dict(walk.blocks),
            }
        return walk.shots

    settings = {"gui_scale": args.gui_scale if maplabel else 3, "language": args.language, "screen_size": [1280, 720]} if java else {"language": args.language}
    return scene_shot.run(ctx, scenario={"steps": [], "java_plain": java, "client_settings": settings}, script=script)
