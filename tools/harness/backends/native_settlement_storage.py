"""Settlement storage and icon menus on the canonical native/Xvfb harness (issue #1, item 7).

Scenario leg of `native-v2-settlement-storage`: `native_v2.run` calls `run(steps,
origin)` inside `client_probes`' windowed lifecycle, after the declared obsidian
pad fixture. This module never starts a server or client, never takes the scene
semaphore and never builds. The lifecycle owns every process, the operator file
and the windowed non-operator peer `StorageGuest` (`steps.client.storage_guest`).

Evidence classes:
- fixture, recorded in `fixture_commands` and never counted as behaviour:
  settlement create, setblock, certify, give/clear, summon/bind, tp, time and
  gamerules, and stocking the chests through the vanilla chest menu;
- owner behaviour: physical XTest pointer and keyboard input into the owned Xvfb
  window (left, right and Shift click, Escape, wheel) on dialog buttons and menu
  slots located by the client's own `ui_nodes` bounds; dialogs are entered by
  the typed `/settlement` commands. Assertions read only received server state;
- guest behaviour: the peer's window shares the owned Xvfb; X input focus moves
  to it for each guest leg (an unfocused window refuses input tools) and back to
  the owner afterwards. Its clicks go through the MCP container/dialog executor
  (declared; not physical input);
- readbacks: physical chests through the vanilla chest menu, villager metadata
  received by the client, and lore through `minecraft_read_item_components`.

Scenario (counts are asserted against received state after every move):
1. storage: 70 cobblestone in two chests and 5 bread show one icon per kind.
   Shift withdraws a stack, a left click takes the rest onto the cursor, a grid
   click stores it, Shift on the own stack stores 64, a right click takes half
   the bread and Escape returns it to the inventory. The real chests,
   inventory and cursor always hold exactly 70 + 5; the bread is stored back.
2. negative controls: a full inventory refuses Shift-withdraw with a chat line
   and moves nothing; the non-owner guest sees a view-only menu and its
   withdraw, Shift and assignment clicks are refused with the owner/operator
   message while nothing moves.
3. Assign crafts: resident -> farmer workshop assigns the first resident; the
   second is refused with "No free work place".
4. building page (Buildings -> Icons -> workshop): the worker fills place 1 and
   the icon lists next-level needs; Upgrade is refused while a workstation is
   missing; after a second composter (fixture) it upgrades to L2 and the free
   place hires the second resident.
5. map click, only when the VillageMap Map button and dialog link nodes exist:
   hovering and clicking the workshop cell opens the same page. Otherwise the
   leg is recorded in `report["unsupported"]`, never as a pass.
6. vanilla job acquisition: at day time 2000 both residents walk to their
   composters and their received villager metadata turns to farmer.
Final: the physical chests hold exactly 70 cobblestone and 5 bread, and no
item entity was dropped.
"""
from __future__ import annotations

import ctypes as c
import json
import os
import re
import time
import uuid
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Callable, Iterator

from .. import runtime
from . import native_client, native_focus
from .gameplay import require_tools
from .native_craft_drag import Pointer
from .native_v2 import _block, _open
from .probe_steps import ProbeSteps, item_count

NAME = "storeqa"
GUEST = "StorageGuest"
DIALOG = "net.minecraft.client.gui.screens.dialog.MultiButtonDialogScreen"
CONTAINER = "net.minecraft.client.gui.screens.inventory.ContainerScreen"
COBBLE, BREAD, DIRT = "minecraft:cobblestone", "minecraft:bread", "minecraft:dirt"
# Server icon menu layout (crates/mc-core/src/settlements/ui_menus.rs).
ICONS, PAGE, MENU_SLOTS = 54, 45, 90
BACK, INFO = 45, 49
BUILDING_ICON, BUILDING_UPGRADE, BUILDING_PLACES = 4, 6, 18
DENIED = "Only the owner or an operator can change this settlement."
GUEST_TOOLS = frozenset({
    "minecraft_observe", "minecraft_wait_for_state_change", "minecraft_send_chat",
    "minecraft_click_container_slot", "minecraft_quick_move_container_slot",
    "minecraft_click_screen_button", "minecraft_close_screen", "minecraft_read_item_components",
})
UI_WAIT = 60.0


class Input(Pointer):
    """Owned-Xvfb XTest input: any mouse button, wheel, Shift and Escape."""

    def __init__(self, owned: dict[str, Any]) -> None:
        super().__init__(owned)
        self.x.XStringToKeysym.restype = c.c_ulong
        self.x.XStringToKeysym.argtypes = [c.c_char_p]
        self.x.XKeysymToKeycode.restype = c.c_ubyte
        self.x.XKeysymToKeycode.argtypes = [c.c_void_p, c.c_ulong]
        self.xt.XTestFakeKeyEvent.argtypes = [c.c_void_p, c.c_uint, c.c_int, c.c_ulong]
        self.keys: list[int] = []
        self.buttons: list[int] = []

    def _key(self, name: str, down: bool) -> None:
        code = self.x.XKeysymToKeycode(self.display, self.x.XStringToKeysym(name.encode()))
        if not code:
            raise RuntimeError(f"owned Xvfb has no keycode for {name}")
        if not self.xt.XTestFakeKeyEvent(self.display, code, int(down), 0):
            raise RuntimeError(f"owned Xvfb refused key {name}")
        if down:
            self.keys.append(code)
        elif code in self.keys:
            self.keys.remove(code)
        self.x.XSync(self.display, 0)

    def _button(self, number: int, down: bool) -> None:
        if not self.xt.XTestFakeButtonEvent(self.display, number, int(down), 0):
            raise RuntimeError(f"owned Xvfb refused mouse button {number}")
        if down:
            self.buttons.append(number)
        elif number in self.buttons:
            self.buttons.remove(number)
        self.x.XSync(self.display, 0)

    def tap(self, name: str) -> None:
        self._key(name, True)
        self._key(name, False)

    def click(self, point: tuple[int, int], button: int = 1, shift: bool = False) -> None:
        """Press and release at one point, never crossing another slot (no drag)."""
        self.move(*point)
        if shift:
            self._key("Shift_L", True)
        try:
            self._button(button, True)
            self._button(button, False)
        finally:
            if shift:
                self._key("Shift_L", False)

    def wheel(self, point: tuple[int, int], down: bool = True, notches: int = 3) -> None:
        self.move(*point)
        for _ in range(notches):
            self._button(5 if down else 4, True)
            self._button(5 if down else 4, False)

    def close(self) -> None:
        try:
            for number in list(self.buttons):
                self._button(number, False)
            for code in list(self.keys):
                self.xt.XTestFakeKeyEvent(self.display, code, 0, 0)
            self.keys.clear()
        finally:
            super().close()


# ------------------------------------------------------------- pure helpers


def plain(value: Any) -> str:
    """Plain text of a JSON text component (string, list or object with extra)."""
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "".join(plain(part) for part in value)
    if isinstance(value, dict):
        return plain(value.get("text", "")) + plain(value.get("extra", []))
    return ""


def lore(components: dict[str, Any]) -> list[str]:
    return [plain(line) for line in components.get("minecraft:lore", [])]


def since(lines: list[str], marker: str) -> list[str]:
    """Chat lines received after `marker` (`recent_chat` lists the newest first)."""
    for index, line in enumerate(lines):
        if marker in line:
            return lines[:index]
    return []


def icon_total(row: dict[str, Any]) -> int:
    """Stored total from a storage icon name `<item> ×N` (the count is capped at 99)."""
    match = re.search(r"×(\d+)$", str(row.get("name") or ""))
    if not match:
        raise RuntimeError(f"storage icon without a ×total name: {row}")
    return int(match.group(1))


def villager_profession(row: dict[str, Any]) -> int | None:
    """Profession protocol id from a received `villager` metadata entry."""
    for entry in (row.get("received_metadata") or {}).values():
        if isinstance(entry, dict) and entry.get("kind") == "villager":
            return int(entry["value"][1])
    return None


def short_id(full: int) -> int:
    """`#N` in names and dialogs: the building id without its settlement half."""
    return full & 0xFFFF_FFFF


def hover_target(labels: dict[str, str], short: int) -> str | None:
    """The link node whose click opens building `#short`.

    A cell's click opens the first feature its hover lists (`ui_map.rs`), after
    the viewer's own "You are here" line; later features are only listed.
    """
    for node, label in labels.items():
        lines = [line for line in label.splitlines() if not line.startswith("You are here")]
        if lines and re.search(rf"#{short}\b", lines[0]):
            return node
    return None


def farmer_id() -> int:
    """26.1.2 protocol id of minecraft:farmer from the local vanilla reports."""
    root = Path(os.environ.get("KRASNOKAMEN_SERVER_VANILLA_DATA_DIR", "data/vanilla"))
    if not root.is_absolute():
        root = runtime.REPO_ROOT / root
    registries = json.loads((root / "reports" / "registries.json").read_text())
    return int(registries["minecraft:villager_profession"]["entries"]["minecraft:farmer"]["protocol_id"])


# ------------------------------------------------------------ state readers


def nodes(state: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {node["id"]: node for node in state.get("ui_nodes", []) if isinstance(node, dict) and "id" in node}


def center(node: dict[str, Any]) -> tuple[int, int]:
    x, y, w, h = node["bounds"]
    return round(x + w / 2), round(y + h / 2)


def screen(state: dict[str, Any]) -> tuple[str, str]:
    current = state.get("screen") or {}
    return str(current.get("class") or ""), str(current.get("title") or "")


def slot_row(state: dict[str, Any], slot: int) -> dict[str, Any]:
    return next((row for row in state["container"]["slots"] if row.get("slot") == slot),
                {"slot": slot, "item_id": "minecraft:air", "count": 0})


def icon(state: dict[str, Any], item: str) -> dict[str, Any] | None:
    """The storage icon of `item`; only the 9x6 icon menu has icon slots."""
    if state["container"]["slot_count"] != MENU_SLOTS:
        return None
    return next((row for row in state["container"]["slots"]
                 if row.get("slot", ICONS) < PAGE and row.get("item_id") == item), None)


def own(state: dict[str, Any], item: str) -> int:
    """The player's own items while the 9x6 icon menu is open (icons excluded)."""
    if state["container"]["slot_count"] != MENU_SLOTS:
        return 0
    return sum(int(row["count"]) for row in state["container"]["slots"]
               if row.get("slot", 0) >= ICONS and row.get("item_id") == item)


def carried(state: dict[str, Any], item: str) -> int:
    cursor = state["container"]["carried"]
    return int(cursor.get("count") or 0) if cursor.get("item_id") == item else 0


def stocked(state: dict[str, Any], item: str) -> int:
    row = icon(state, item)
    return 0 if row is None else icon_total(row)


def in_menu(state: dict[str, Any], title: str) -> bool:
    return (screen(state) == (CONTAINER, title)
            and state["container"]["menu_class"] == "net.minecraft.world.inventory.ChestMenu"
            and state["container"]["slot_count"] == MENU_SLOTS)


class Leg:
    """One owner session: physical input, ordered chat markers and receipts."""

    def __init__(self, steps: ProbeSteps, owned: dict[str, Any]) -> None:
        self.steps = steps
        self.owned = owned
        self.input = Input(owned)
        self.artifact = steps.artifact_dir.parent
        self.detail: dict[str, Any] = {}
        steps.report["storage_qa"] = self.detail
        steps.report["unsupported"] = []

    def shot(self, name: str) -> None:
        native_client.capture(self.steps.client, self.artifact, f"settlement-storage-{name}")

    def wait(self, predicate: Callable[[dict[str, Any]], bool], label: str,
             seconds: float = UI_WAIT) -> dict[str, Any]:
        return self.steps.wait(predicate, label, timeout_seconds=seconds)

    def mark(self, label: str) -> str:
        marker = f"storage-qa {label} {uuid.uuid4().hex[:8]}"
        self.steps.call("minecraft_send_chat", {"message": marker, "command": False})
        self.wait(lambda s: any(marker in line for line in s["recent_chat"]), f"{label}_marker")
        return marker

    def said(self, marker: str, text: str, label: str) -> dict[str, Any]:
        return self.wait(lambda s: any(text in line for line in since(s["recent_chat"], marker)), label)

    def command(self, text: str, expect: str, label: str) -> str:
        """A fixture command whose reply line must contain `expect`."""
        marker = self.mark(label)
        self.steps.fixture(text)
        state = self.said(marker, expect, label)
        return next(line for line in since(state["recent_chat"], marker) if expect in line)

    def node(self, state: dict[str, Any], node_id: str) -> dict[str, Any]:
        found = nodes(state).get(node_id)
        if found is None:
            raise RuntimeError(f"ui node {node_id} is not visible on {screen(state)}")
        return found

    def slot(self, slot: int, label: str, button: int = 1, shift: bool = False) -> None:
        state = self.steps.observe()
        self.steps.report.setdefault("physical_clicks", []).append(
            {"label": label, "slot": slot, "button": button, "shift": shift, "screen": screen(state)})
        self.input.click(center(self.node(state, f"slot/{slot}")), button, shift)

    def menu(self, title: str, label: str) -> dict[str, Any]:
        return self.wait(lambda s: in_menu(s, title), label)

    def reveal(self, label: str) -> dict[str, Any] | None:
        """Wheel-scroll the open dialog until button `label` is listed, else None.

        The client lists a dialog control only while it lies entirely inside the
        scroll viewport (`ui/dialog.rs`), so a listed node is safe to click.
        """
        state = self.steps.observe()
        for down in [True] * 5 + [False] * 10 + [None]:
            found = [node for key, node in nodes(state).items()
                     if key.startswith("dialog/action/") and node.get("label") == label and node.get("enabled")]
            if found:
                return found[0]
            area = [node for key, node in nodes(state).items()
                    if key.startswith(("dialog/action/", "dialog/body/", "dialog/title"))]
            if down is None or not area:
                return None
            self.input.wheel(center(area[0]), down=down, notches=1)
            try:
                self.steps.call("minecraft_wait_for_state_change", {
                    "observed_version": state["state_version"], "timeout_seconds": 5.0})
            except Exception:
                pass  # the body may already be at that scroll end
            state = self.steps.observe()
        return None

    def button(self, label: str, name: str) -> None:
        """Physically click a dialog button revealed by the wheel."""
        found = self.reveal(label)
        if found is None:
            state = self.steps.observe()
            raise RuntimeError(f"dialog button {label!r} not reachable on {screen(state)}")
        self.steps.report.setdefault("physical_clicks", []).append(
            {"label": name, "button_label": label, "bounds": found.get("bounds")})
        self.input.click(center(found))

    def dialog(self, title: str, label: str) -> dict[str, Any]:
        return self.wait(lambda s: screen(s)[0] == DIALOG and screen(s)[1].startswith(title), label)

    def main(self, label: str) -> dict[str, Any]:
        self.steps.call("minecraft_send_chat", {"message": "settlement", "command": True})
        return self.dialog(f"Settlement {NAME}", label)

    def escape(self, label: str) -> dict[str, Any]:
        self.input.tap("Escape")
        return self.wait(lambda s: not (s.get("screen") or {}).get("open")
                         and s["container"]["container_id"] == 0, label)

    def components(self, slot: int) -> dict[str, Any]:
        return self.steps.call("minecraft_read_item_components",
                               {"source": "container", "slot": slot})["components"]

    def tp(self, at: tuple[float, float, float], label: str) -> None:
        self.steps.fixture(f"tp @s {at[0]:.1f} {at[1]:.1f} {at[2]:.1f}")
        self.wait(lambda s: abs(float(s["player"]["x"]) - at[0]) < 0.6
                  and abs(float(s["player"]["z"]) - at[2]) < 0.6, label, 30.0)

    def chest(self, pos: tuple[int, int, int], label: str) -> dict[str, int]:
        """Totals of one physical chest read through the vanilla chest menu."""
        self.tp((pos[0] + 0.5, pos[1], pos[2] - 1.5), f"{label}_at_chest")
        state = _open(self.steps, dict(zip("xyz", pos)), "ChestMenu", 63)
        totals: dict[str, int] = {}
        for row in state["container"]["slots"]:
            if row.get("slot", 99) < 27 and int(row.get("count") or 0) > 0:
                totals[row["item_id"]] = totals.get(row["item_id"], 0) + int(row["count"])
        self.steps.call("minecraft_close_screen")
        self.wait(lambda s: s["container"]["container_id"] == 0, f"{label}_chest_closed")
        return totals


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    owned = getattr(steps.client, "p4_window", None)
    guest = getattr(steps.client, "storage_guest", None)
    if not owned:
        raise RuntimeError("settlement storage requires the harness-owned windowed Xvfb client")
    if guest is None:
        raise RuntimeError("settlement storage requires the lifecycle's non-operator StorageGuest peer")
    require_tools(guest[0].list_tools(), set(GUEST_TOOLS), "native-v2-settlement-storage guest")
    leg = Leg(steps, owned)
    try:
        _scenario(leg, guest, origin)
    finally:
        leg.input.close()
    steps.report["limits"] = [
        "owner clicks are physical XTest input into the owned window; the windowed guest uses the MCP executor",
        "dialogs are opened by typed /settlement commands; fixtures are recorded separately",
        "map-cell click runs only with the VillageMap dialog link nodes, otherwise it is listed as unsupported",
        "visual review of the retained screenshots is still required; no vanilla Java client parity claim",
        "storage conservation closes before residents exist; residents may later use storage legitimately",
        "an obsidian town keeps planner street work (Builder conversion) out of the assignment legs",
    ]


def _scenario(leg: Leg, guest: Any, origin: tuple[int, int, int]) -> None:
    steps, detail = leg.steps, leg.detail
    ox, y0, oz = origin
    chest_a, chest_b = (ox - 4, y0, oz + 3), (ox - 2, y0, oz + 3)
    station_1, station_2 = (ox + 3, y0, oz + 3), (ox + 4, y0, oz + 3)
    home = (ox + 0.5, float(y0), oz + 0.5)
    for rule in ("gamerule advance_time false", "gamerule spawn_mobs false",
                 "gamerule random_tick_speed 0", "time set 10000"):
        # Day time 10000 is outside the 2000..9000 window in which residents
        # claim job sites on their own: only the menu assigns them.
        steps.fixture(rule)
    # No soil around the town: free street work (planner) needs grass/dirt road
    # columns and may make an unemployed resident a Builder, which this gate's
    # assignment legs must not race against.
    steps.fixture(f"fill {ox - 16} {y0 - 1} {oz - 16} {ox + 16} {y0 - 1} {oz + 16} minecraft:obsidian")
    steps.fixture(f"fill {ox - 16} {y0} {oz - 16} {ox + 16} {y0 + 5} {oz + 16} minecraft:air")
    _block(steps, {"x": ox + 16, "y": y0 - 1, "z": oz + 16}, "minecraft:obsidian")
    _block(steps, {"x": ox - 16, "y": y0, "z": oz - 16}, "minecraft:air")
    leg.command(f"settlement create {NAME} 24", f"Founded {NAME}", "founded")
    for pos in (chest_a, chest_b):
        steps.fixture(f"setblock {pos[0]} {pos[1]} {pos[2]} minecraft:chest[facing=south,type=single,waterlogged=false]")
    storage = leg.command(f"settlement building certify storage {ox - 5} {y0 - 1} {oz + 1} {ox - 1} {y0 + 3} {oz + 5}",
                          "Certified storage", "storage_certified")
    detail["storage_certificate"] = storage

    # Fixture stock: 64 cobblestone in A, 6 cobblestone and 5 bread in B.
    steps.fixture(f"give @s {COBBLE} 70")
    steps.fixture(f"give @s {BREAD} 5")
    leg.wait(lambda s: item_count(s, COBBLE) == 70 and item_count(s, BREAD) == 5, "fixture_items_given")
    for pos, moves in ((chest_a, [(COBBLE, 64)]), (chest_b, [(COBBLE, 6), (BREAD, 5)])):
        leg.tp((pos[0] + 0.5, pos[1], pos[2] - 1.5), f"stock_at_{pos[0]}")
        state = _open(steps, dict(zip("xyz", pos)), "ChestMenu", 63)
        for item, count in moves:
            source = next(row["slot"] for row in state["container"]["slots"]
                          if row.get("slot", 0) >= 27 and row.get("item_id") == item and row.get("count") == count)
            steps.call("minecraft_quick_move_container_slot", {"slot": source, "timeout_seconds": steps.remaining()})
            state = steps.observe()
        steps.call("minecraft_close_screen")
        leg.wait(lambda s: s["container"]["container_id"] == 0, f"stock_closed_{pos[0]}")
    leg.wait(lambda s: item_count(s, COBBLE) == 0 and item_count(s, BREAD) == 0, "fixture_stock_left_inventory")
    chests = [leg.chest(chest_a, "before_a"), leg.chest(chest_b, "before_b")]
    detail["chests_before"] = chests
    steps.check("chests_hold_fixture_stock", chests == [{COBBLE: 64}, {COBBLE: 6, BREAD: 5}])
    leg.tp(home, "home_for_storage")

    _storage(leg, chest_a, chest_b, home)
    _full_inventory(leg)
    _guest_storage(leg, guest, origin)

    # Conservation closes before residents exist: from here on villagers may
    # legitimately use settlement storage (eat, deposit), which is not menu custody.
    final = [leg.chest(chest_a, "final_a"), leg.chest(chest_b, "final_b")]
    detail["chests_final"] = final
    total = lambda item: sum(chest.get(item, 0) for chest in final)
    state = steps.observe()
    steps.check("final_conservation_cobblestone", total(COBBLE) == 70 and item_count(state, COBBLE) == 0)
    steps.check("final_conservation_bread", total(BREAD) == 5 and item_count(state, BREAD) == 0)
    dropped = [row for row in steps.call("minecraft_list_entities", {"radius": 32, "limit": 256})["entities"]
               if row.get("entity_type") == "minecraft:item"]
    detail["dropped_items"] = dropped
    steps.check("no_item_entities_dropped", not dropped)
    leg.tp(home, "home_for_residents")

    short, residents = _assign(leg, origin, station_1)
    _guest_assign(leg, guest)
    _building(leg, short, residents, station_2)
    _map(leg, short)
    _jobs(leg, residents, (station_1, station_2))
    leg.shot("final")


def _storage(leg: Leg, chest_a: tuple[int, int, int], chest_b: tuple[int, int, int],
             home: tuple[float, float, float]) -> None:
    steps, detail = leg.steps, leg.detail
    title = f"Storage · {NAME}"

    def conserved(state: dict[str, Any], label: str) -> None:
        for item, total in ((COBBLE, 70), (BREAD, 5)):
            here = stocked(state, item) + own(state, item) + carried(state, item)
            steps.check(f"{label}_{item.split(':')[1]}_conserved", here == total)

    leg.main("main_dialog_storage")
    leg.button("Storage", "main_storage")
    state = leg.menu(title, "storage_menu_open")
    cobble, bread = icon(state, COBBLE), icon(state, BREAD)
    steps.check("storage_icons_one_per_kind", cobble is not None and bread is not None
                and icon_total(cobble) == 70 and cobble["count"] == 70
                and icon_total(bread) == 5 and bread["count"] == 5)
    detail["storage_info"] = slot_row(state, INFO).get("name")
    steps.check("storage_info_counts_both_chests", detail["storage_info"] == "2 containers, 2 kinds, 75 items")
    detail["cobblestone_lore"] = lore(leg.components(cobble["slot"]))
    steps.check("storage_icon_lore", "In storage: 70" in detail["cobblestone_lore"]
                and "Left: take a stack, right: half" in detail["cobblestone_lore"])
    conserved(state, "opened")
    leg.shot("storage-open")

    leg.slot(cobble["slot"], "shift_withdraw_stack", shift=True)
    state = leg.wait(lambda s: own(s, COBBLE) == 64 and stocked(s, COBBLE) == 6
                     and carried(s, COBBLE) == 0, "shift_withdraw_64")
    steps.check("shift_withdraw_lands_in_hotbar_end", slot_row(state, MENU_SLOTS - 1).get("count") == 64)
    conserved(state, "shift_withdraw")

    leg.slot(icon(state, COBBLE)["slot"], "take_rest_to_cursor")
    state = leg.wait(lambda s: carried(s, COBBLE) == 6 and icon(s, COBBLE) is None, "rest_on_cursor")
    conserved(state, "cursor")
    leg.shot("cursor-stack")
    empty = next(slot for slot in range(PAGE) if slot_row(state, slot)["count"] == 0)
    leg.slot(empty, "store_cursor_on_grid")
    state = leg.wait(lambda s: carried(s, COBBLE) == 0 and stocked(s, COBBLE) == 6, "cursor_stored")
    conserved(state, "cursor_stored")

    stack = next(row["slot"] for row in state["container"]["slots"]
                 if row.get("slot", 0) >= ICONS and row.get("item_id") == COBBLE and row.get("count") == 64)
    leg.slot(stack, "shift_store_own_stack", shift=True)
    state = leg.wait(lambda s: own(s, COBBLE) == 0 and stocked(s, COBBLE) == 70, "own_stack_stored")
    conserved(state, "own_stored")

    leg.slot(icon(state, BREAD)["slot"], "right_take_half", button=3)
    state = leg.wait(lambda s: carried(s, BREAD) == 3 and stocked(s, BREAD) == 2, "half_bread_on_cursor")
    conserved(state, "half")
    leg.escape("escape_closes_storage")
    leg.wait(lambda s: item_count(s, BREAD) == 3, "escape_returns_cursor_to_inventory")
    chests = [leg.chest(chest_a, "mid_a"), leg.chest(chest_b, "mid_b")]
    detail["chests_after_moves"] = chests
    steps.check("physical_chests_match_menu",
                sum(chest.get(COBBLE, 0) for chest in chests) == 70
                and sum(chest.get(BREAD, 0) for chest in chests) == 2)
    leg.tp(home, "home_after_readback")

    steps.call("minecraft_send_chat", {"message": "settlement menu storage", "command": True})
    state = leg.menu(title, "storage_reopened")
    bread_slot = next(row["slot"] for row in state["container"]["slots"]
                      if row.get("slot", 0) >= ICONS and row.get("item_id") == BREAD)
    leg.slot(bread_slot, "shift_store_bread", shift=True)
    state = leg.wait(lambda s: own(s, BREAD) == 0 and stocked(s, BREAD) == 5, "bread_stored_back")
    conserved(state, "restored")
    leg.escape("storage_closed")


def _full_inventory(leg: Leg) -> None:
    steps = leg.steps
    steps.fixture(f"give @s {DIRT} 2304")
    leg.wait(lambda s: item_count(s, DIRT) == 2304, "inventory_filled")
    steps.call("minecraft_send_chat", {"message": "settlement menu storage", "command": True})
    state = leg.menu(f"Storage · {NAME}", "full_storage_open")
    marker = leg.mark("full_inventory")
    leg.slot(icon(state, COBBLE)["slot"], "shift_withdraw_full_inventory", shift=True)
    state = leg.said(marker, "Your inventory is full.", "full_inventory_refused")
    steps.check("full_inventory_moves_nothing", stocked(state, COBBLE) == 70 and own(state, DIRT) == 2304
                and own(state, COBBLE) == 0 and carried(state, COBBLE) == 0)
    leg.escape("full_storage_closed")
    steps.fixture(f"clear @s {DIRT}")
    leg.wait(lambda s: item_count(s, DIRT) == 0, "inventory_cleared")


def _focus(steps: ProbeSteps, display: str, pid: int, label: str, artifact: Path) -> None:
    """Give one client's window X input focus and wait until that client reports it.

    The capture doubles as the focus readback (`window.focused`) and as that
    client's view at the hand-over; until focus lands it is refused.
    """
    native_focus.acquire(display, pid, steps.remaining())

    def focused(_: dict[str, Any]) -> bool:
        try:
            metadata = native_client.capture(steps.client, artifact, f"settlement-storage-{label}")
        except Exception as error:
            if "window_not_focused" in str(error):
                return False
            raise
        return metadata["window"]["focused"] is True

    steps.wait(focused, label, timeout_seconds=UI_WAIT)


@contextmanager
def _guest(leg: Leg, guest: tuple[Any, int], name: str) -> Iterator[ProbeSteps]:
    """One guest leg with X focus on the guest window, handed back to the owner after."""
    client, pid = guest
    remaining = max(1.0, leg.steps.deadline - time.monotonic())
    with ProbeSteps(client, leg.steps.artifact_dir / name, remaining, name) as peer:
        _focus(peer, leg.owned["display"], pid, f"{name}-focused", leg.artifact)
        yield peer
    _focus(leg.steps, leg.owned["display"], leg.owned["pid"], f"{name}-owner-refocused", leg.artifact)


def _guest_storage(leg: Leg, guest: Any, origin: tuple[int, int, int]) -> None:
    steps = leg.steps
    ox, y0, oz = origin
    steps.fixture(f"tp {GUEST} {ox + 0.5:.1f} {y0:.1f} {oz - 3.5:.1f}")
    with _guest(leg, guest, "guest-storage") as peer:
        peer.wait(lambda s: abs(float(s["player"]["z"]) - (oz - 3.5)) < 0.6, "guest_inside_settlement",
                  timeout_seconds=30)
        peer.call("minecraft_send_chat", {"message": "settlement menu storage", "command": True})
        state = peer.wait(lambda s: in_menu(s, f"Storage · {NAME}"), "guest_storage_open", timeout_seconds=UI_WAIT)
        info = lore(peer.call("minecraft_read_item_components",
                              {"source": "container", "slot": INFO})["components"])
        peer.check("guest_menu_is_view_only", "View only." in info)
        cobble = icon(state, COBBLE)["slot"]
        for tool, args, label in (
                ("minecraft_click_container_slot", {"button": "primary"}, "guest_withdraw_denied"),
                ("minecraft_quick_move_container_slot", {}, "guest_shift_denied")):
            before = peer.observe()["recent_chat"]
            peer.call(tool, {"slot": cobble, "timeout_seconds": peer.remaining(), **args})
            state = peer.wait(lambda s: sum(DENIED in line for line in s["recent_chat"])
                              > sum(DENIED in line for line in before), label, timeout_seconds=UI_WAIT)
            peer.check(f"{label}_moves_nothing", stocked(state, COBBLE) == 70
                       and own(state, COBBLE) == 0 and carried(state, COBBLE) == 0)
        peer.call("minecraft_close_screen")
        peer.wait(lambda s: s["container"]["container_id"] == 0, "guest_storage_closed", timeout_seconds=UI_WAIT)
    steps.call("minecraft_send_chat", {"message": "settlement menu storage", "command": True})
    state = leg.menu(f"Storage · {NAME}", "owner_storage_after_guest")
    steps.check("guest_changed_no_stock", stocked(state, COBBLE) == 70 and stocked(state, BREAD) == 5)
    leg.escape("owner_storage_after_guest_closed")


def _assign(leg: Leg, origin: tuple[int, int, int],
            station: tuple[int, int, int]) -> tuple[int, list[str]]:
    steps, detail = leg.steps, leg.detail
    ox, y0, oz = origin
    steps.fixture(f"setblock {station[0]} {station[1]} {station[2]} minecraft:composter")
    certified = leg.command(f"settlement building certify farmer {ox + 1} {y0 - 1} {oz + 1} {ox + 5} {y0 + 3} {oz + 5}",
                            "Certified farmer", "farmer_certified")
    short = short_id(int(re.search(r"#(\d+)", certified).group(1)))
    detail["workshop"] = {"certificate": certified, "short": short}
    known = {str(row.get("entity_uuid")) for row in _villagers(steps)}
    for x in (ox + 0.5, ox + 2.5):
        steps.fixture(f"summon minecraft:villager {x:.1f} {y0:.1f} {oz - 8.5:.1f}")
    state = leg.wait(lambda s: len({str(row.get("entity_uuid")) for row in _villagers(steps)} - known) == 2,
                     "two_villagers_summoned")
    fresh = sorted({str(row.get("entity_uuid")) for row in _villagers(steps)} - known)
    for resident in fresh:
        leg.command(f"settlement bind {NAME} {resident}", f"Bound {resident}", f"bound_{resident[:8]}")
    workshop = f"#{short} Farmer workshop L1"

    leg.main("main_dialog_assign")
    leg.button("Assign crafts", "main_assign_crafts")
    state = leg.menu(f"Residents · {NAME}", "residents_menu_open")
    order: list[str] = []
    for slot in range(2):
        text = lore(leg.components(slot))
        match = next((line[3:] for line in text if line.startswith("id ")), None)
        order.append(next(resident for resident in fresh if match and resident.replace("-", "").startswith(match)))
        steps.check(f"resident_{slot}_unemployed", "Unemployed" in text and "No workplace" in text)
    steps.check("residents_are_the_bound_villagers", sorted(order) == fresh)
    detail["residents"] = order
    leg.shot("residents")

    leg.slot(0, "pick_first_resident")
    state = leg.wait(lambda s: screen(s)[0] == CONTAINER and screen(s)[1].startswith("Workplace for "),
                     "workplaces_for_first")
    steps.check("workshop_offered", slot_row(state, 0).get("name") == workshop
                and "Click: assign here" in lore(leg.components(0)))
    marker = leg.mark("assign_first")
    leg.slot(0, "assign_first")
    leg.said(marker, f"is assigned to {workshop}", "first_assigned")
    leg.menu(f"Residents · {NAME}", "back_to_residents")

    leg.slot(1, "pick_second_resident")
    state = leg.wait(lambda s: screen(s)[1].startswith("Workplace for "), "workplaces_for_second")
    steps.check("second_sees_full_workshop",
                "No free work place with a workstation here." in lore(leg.components(0)))
    marker = leg.mark("assign_second")
    leg.slot(0, "assign_second_refused")
    leg.said(marker, "No free work place with a workstation here.", "second_refused_full")
    leg.slot(BACK, "back_from_workplaces")
    leg.menu(f"Residents · {NAME}", "residents_again")
    text = lore(leg.components(0))
    steps.check("first_resident_works_at_workshop", f"Works at {workshop}" in text)
    steps.check("second_resident_still_free", "No workplace" in lore(leg.components(1)))
    leg.slot(BACK, "back_to_main_dialog")
    leg.dialog(f"Settlement {NAME}", "back_closed_menu_opened_main")
    leg.input.tap("Escape")
    leg.wait(lambda s: not (s.get("screen") or {}).get("open"), "main_dialog_closed")
    return short, order


def _guest_assign(leg: Leg, guest: Any) -> None:
    with _guest(leg, guest, "guest-assign") as peer:
        peer.call("minecraft_send_chat", {"message": "settlement", "command": True})
        state = peer.wait(lambda s: screen(s)[0] == DIALOG and screen(s)[1].startswith(f"Settlement {NAME}"),
                          "guest_main_dialog", timeout_seconds=UI_WAIT)
        peer.call("minecraft_click_screen_button", {
            "expected_screen_class": DIALOG, "expected_title": screen(state)[1], "button_label": "Assign crafts"})
        peer.wait(lambda s: in_menu(s, f"Residents · {NAME}"), "guest_residents_open", timeout_seconds=UI_WAIT)
        before = peer.observe()["recent_chat"]
        peer.call("minecraft_click_container_slot", {"slot": 1, "button": "primary",
                                                     "timeout_seconds": peer.remaining()})
        peer.wait(lambda s: sum(DENIED in line for line in s["recent_chat"])
                  > sum(DENIED in line for line in before), "guest_assign_denied", timeout_seconds=UI_WAIT)
        state = peer.observe()
        peer.check("guest_stays_on_residents", in_menu(state, f"Residents · {NAME}"))
        peer.call("minecraft_close_screen")
        peer.wait(lambda s: s["container"]["container_id"] == 0, "guest_residents_closed", timeout_seconds=UI_WAIT)


def _building(leg: Leg, short: int, residents: list[str], station: tuple[int, int, int]) -> None:
    steps, detail = leg.steps, leg.detail
    first, second = (resident.replace("-", "")[:8] for resident in residents)
    leg.main("main_dialog_buildings")
    leg.button("Buildings", "main_buildings")
    leg.dialog(f"Buildings - {NAME}", "buildings_dialog")
    leg.button("Icons", "buildings_icons")
    state = leg.menu(f"Buildings · {NAME}", "buildings_menu_open")
    workshop = next(row for row in state["container"]["slots"]
                    if row.get("slot", PAGE) < PAGE and str(row.get("name", "")).startswith(f"#{short} Farmer workshop"))
    leg.slot(workshop["slot"], "open_workshop_page")
    page = f"#{short} Farmer workshop L1"
    state = leg.menu(page, "workshop_page_open")
    needs = lore(leg.components(BUILDING_ICON))
    detail["workshop_l1_lore"] = needs
    steps.check("next_level_needs_listed", "Level 2 needs:" in needs and "- workstations 1/2" in needs)
    steps.check("first_worker_in_place_one", f"id {first}" in lore(leg.components(BUILDING_PLACES)))
    steps.check("upgrade_button_offered", slot_row(state, BUILDING_UPGRADE).get("name") == "Upgrade to level 2")
    leg.shot("workshop-l1")

    marker = leg.mark("upgrade_refused")
    leg.slot(BUILDING_UPGRADE, "upgrade_while_missing")
    leg.said(marker, f"Cannot upgrade #{short}. Missing: workstations 1/2.", "upgrade_refused")
    steps.check("refused_upgrade_keeps_level", in_menu(steps.observe(), page))

    steps.fixture(f"setblock {station[0]} {station[1]} {station[2]} minecraft:composter")
    steps.call("minecraft_wait_for_block_state", {
        "x": station[0], "y": station[1], "z": station[2],
        "block_id": "minecraft:composter", "timeout_seconds": steps.remaining(),
    })
    leg.slot(BUILDING_ICON, "refresh_workshop")
    leg.wait(lambda s: in_menu(s, page) and "Ready for level 2" in lore(leg.components(BUILDING_ICON)),
             "ready_for_level_2")
    marker = leg.mark("upgrade")
    leg.slot(BUILDING_UPGRADE, "upgrade_to_2")
    leg.said(marker, f"Building #{short} upgraded to level 2.", "upgraded")
    page = f"#{short} Farmer workshop L2"
    state = leg.menu(page, "workshop_l2_page")
    steps.check("free_place_after_upgrade", slot_row(state, BUILDING_PLACES + 1).get("name") == "Free work place")

    leg.slot(BUILDING_PLACES + 1, "open_free_place")
    state = leg.menu(f"Hire for {page}", "hire_menu_open")
    candidate = lore(leg.components(0))
    steps.check("hire_lists_second_first", f"id {second}" in candidate and "Click: assign here" in candidate)
    leg.shot("hire")
    marker = leg.mark("hire")
    leg.slot(0, "hire_second")
    leg.said(marker, f"is assigned to {page}", "second_hired")
    state = leg.menu(page, "back_on_workshop_page")
    steps.check("both_places_filled", f"id {first}" in lore(leg.components(BUILDING_PLACES))
                and f"id {second}" in lore(leg.components(BUILDING_PLACES + 1)))
    leg.shot("workshop-l2")
    leg.escape("workshop_page_closed")


def _map(leg: Leg, short: int) -> None:
    steps = leg.steps
    leg.main("main_dialog_map")
    found = leg.reveal("Map")
    if found is None:
        steps.report["unsupported"].append(
            "map-cell click: the main dialog has no Map button (VillageMap patch not integrated)")
        leg.input.tap("Escape")
        leg.wait(lambda s: not (s.get("screen") or {}).get("open"), "map_dialog_absent_closed")
        return
    steps.report.setdefault("physical_clicks", []).append(
        {"label": "main_map", "button_label": "Map", "bounds": found.get("bounds")})
    leg.input.click(center(found))
    state = leg.dialog(f"Map - {NAME}", "map_dialog_open")
    links = {key: node for key, node in nodes(state).items()
             if key.startswith("dialog/body/") and node.get("role") == "link"}
    if not links:
        steps.report["unsupported"].append(
            "map-cell click: the native client exposes no dialog-body link nodes (VillageMap client patch absent)")
        leg.input.tap("Escape")
        leg.wait(lambda s: not (s.get("screen") or {}).get("open"), "map_dialog_closed")
        return
    hovers: dict[str, str] = {}
    for key, node in links.items():
        # Leave every cell first so the next tooltip is this link's, not a stale one.
        leg.input.move(5, 5)
        leg.wait(lambda s: "component/hover/text" not in nodes(s), f"unhover_before_{key}")
        leg.input.move(*center(node))
        hovered = leg.wait(lambda s: "component/hover/text" in nodes(s), f"hover_{key}")
        hovers[key] = str(nodes(hovered)["component/hover/text"].get("label", ""))
        if hover_target({key: hovers[key]}, short):
            break
    leg.detail["map_hovers"] = hovers
    target = hover_target(hovers, short)
    steps.check("map_cell_names_workshop", target is not None)
    leg.input.click(center(links[target]))
    leg.menu(f"#{short} Farmer workshop L2", "map_click_opens_building_page")
    leg.shot("map-building-page")
    leg.escape("map_building_page_closed")


def _villagers(steps: ProbeSteps) -> list[dict[str, Any]]:
    rows = steps.call("minecraft_list_entities", {"radius": 48, "limit": 256})["entities"]
    return [row for row in rows if row.get("entity_type") == "minecraft:villager"]


def _jobs(leg: Leg, residents: list[str], stations: tuple[tuple[int, int, int], ...]) -> None:
    steps = leg.steps
    farmer = farmer_id()
    steps.fixture("time set 2000")

    def professions() -> dict[str, int | None]:
        return {str(row["entity_uuid"]): villager_profession(row)
                for row in _villagers(steps) if str(row.get("entity_uuid")) in residents}

    seen: dict[str, int | None] = {}

    def farmers(_: dict[str, Any]) -> bool:
        seen.clear()
        seen.update(professions())
        return len(seen) == 2 and all(value == farmer for value in seen.values())

    leg.wait(farmers, "both_residents_became_farmers", 300.0)
    leg.detail["professions"] = dict(seen)
    for pos in stations:
        _block(steps, dict(zip("xyz", pos)), "minecraft:composter")
    steps.check("composters_kept", True)
    leg.main("main_dialog_jobs")
    leg.button("Assign crafts", "main_assign_after_work")
    leg.menu(f"Residents · {NAME}", "residents_after_work")
    for slot in range(2):
        steps.check(f"resident_{slot}_shown_as_farmer", "Farmer" in lore(leg.components(slot)))
    leg.shot("farmers")
    leg.escape("residents_after_work_closed")
