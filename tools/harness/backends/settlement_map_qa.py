"""Civil settlement UI through the real vanilla/native GUI.

The scene-shot lifecycle owns the server, observer, private Xvfb and pin. Fixture
commands are recorded separately; navigation never substitutes a menu command
for a failed click. Java's observer has no widget geometry, so dialog labels are
located in the captured frame using the running vanilla version's bitmap font.
Map targets are located by their rendered palette pixels, not server payloads.
"""
from __future__ import annotations

import argparse
import ast
import io
import json
import math
import os
import re
import subprocess
import sys
import time
import uuid
import zipfile
from pathlib import Path
from typing import Any, Callable

from PIL import Image, ImageChops

from .. import runtime
from ..profiles import ProfileContext
from . import scene_shot
from .autocraft_qa import answer_after_marker, parse_json_line
from .probe_steps import ProbeSteps
from .native_settlement_storage import Input


def _label_bitmap(label: str, client_jar: Path) -> Image.Image:
    """Rasterize only the vanilla bitmap providers used by our button labels."""
    glyphs: dict[str, tuple[Image.Image, int, int]] = {}
    with zipfile.ZipFile(client_jar) as jar:
        def providers(name: str) -> None:
            namespace, path = name.split(":", 1)
            definition = json.loads(jar.read(f"assets/{namespace}/font/{path}.json"))
            for provider in definition["providers"]:
                if provider["type"] == "reference":
                    providers(provider["id"])
                elif provider["type"] == "space":
                    for char, advance in provider["advances"].items():
                        glyphs.setdefault(char, (Image.new("L", (1, 1)), advance, 0))
                elif provider["type"] == "bitmap":
                    namespace, path = provider["file"].split(":", 1)
                    atlas = Image.open(io.BytesIO(jar.read(f"assets/{namespace}/textures/{path}"))).convert("RGBA")
                    rows = provider["chars"]
                    cell_width, cell_height = atlas.width // len(rows[0]), atlas.height // len(rows)
                    height = provider.get("height", 8)
                    for y, row in enumerate(rows):
                        for x, char in enumerate(row):
                            if char in glyphs or char == "\0":
                                continue
                            alpha = atlas.crop((x * cell_width, y * cell_height,
                                                (x + 1) * cell_width, (y + 1) * cell_height)).getchannel("A")
                            box = alpha.getbbox()
                            if box:
                                advance = int(box[2] * height / cell_height + 0.5) + 1
                                mask = alpha.resize((round(cell_width * height / cell_height), height),
                                                    Image.Resampling.NEAREST).point(lambda value: 255 if value else 0)
                                glyphs[char] = mask, advance, provider["ascent"]
        providers("minecraft:default")
    if any(char not in glyphs for char in label):
        raise ValueError(f"button label has no vanilla bitmap glyph: {label!r}")
    image = Image.new("L", (sum(glyphs[char][1] for char in label) + 2, 32))
    x = 1
    for char in label:
        glyph, advance, ascent = glyphs[char]
        image.paste(glyph, (x, 16 - ascent))
        x += advance
    box = image.getbbox()
    if box is None:
        raise ValueError("empty button label")
    return image.crop(box)


def _label_points(path: str, bitmap: Image.Image, scale: int) -> list[tuple[int, int]]:
    """Find exact bright text masks; do not OCR a similar-looking label."""
    with Image.open(path) as source:
        source = source.convert("RGB")
        bright = Image.new("L", source.size)
        bright.putdata([255 if min(pixel) >= 200 and max(pixel) - min(pixel) <= 8 else 0
                        for pixel in source.getdata()])
    needle = bitmap.resize((bitmap.width * scale, bitmap.height * scale), Image.Resampling.NEAREST)
    width, height = needle.size
    row = needle.tobytes()[:width]
    pixels = bright.tobytes()
    points = []
    for y in range(bright.height - height + 1):
        line = pixels[y * bright.width:(y + 1) * bright.width]
        x = line.find(row)
        while x >= 0 and x + width <= bright.width:
            crop = bright.crop((x, y, x + width, y + height))
            if ImageChops.difference(crop, needle).getbbox() is None:
                points.append((x + width // 2, y + height // 2))
            x = line.find(row, x + 1)
    return points


def _slot_point(size: tuple[int, int], scale: int, rows: int, slot: int) -> tuple[int, int]:
    """Vanilla ChestMenu/ContainerScreen geometry, in owned-window pixels."""
    if rows not in range(1, 7) or not 0 <= slot < rows * 9 + 36:
        raise ValueError(f"not a visible chest slot: rows={rows}, slot={slot}")
    width, height = (math.ceil(side / scale) for side in size)
    left, top = (width - 176) // 2, (height - (114 + rows * 18)) // 2
    chest = rows * 9
    if slot < chest:
        x, y = 16 + slot % 9 * 18, 26 + slot // 9 * 18
    else:
        index = slot - chest
        x = 16 + index % 9 * 18
        y = rows * 18 + (39 + index // 9 * 18 if index < 27 else 97)
    return (left + x) * scale, (top + y) * scale


class Gui:
    """Input into the lifecycle-owned window, with received-state postconditions."""

    def __init__(self, probe: ProbeSteps, artifact: Path, client: str, scale: int = 3,
                 size: tuple[int, int] = (1280, 720)):
        self.probe, self.artifact, self.client, self.scale, self.size = probe, artifact, client, scale, size
        owned = getattr(probe.client, "p4_window", None)
        if not owned:
            raise RuntimeError("civil GUI requires the lifecycle-owned Xvfb window")
        self.input = Input(owned)
        self.capture = scene_shot.capture_java if client == "java" else scene_shot.capture_native
        home = Path(os.environ.get("GRADLE_USER_HOME", Path.home() / ".gradle"))
        version = os.environ.get("KRASNOKAMEN_JAVA_VERSION", "26.1.2")
        metadata = Path(os.environ.get("KRASNOKAMEN_JAVA_VERSION_METADATA",
                                      home / "caches/fabric-loom" / version / "mojang_minecraft_info.json"))
        self.jar = Path(os.environ.get("KRASNOKAMEN_CLIENT_JAR", metadata.parent / "minecraft-client.jar"))
        self.labels: dict[str, Image.Image] = {}
        self.serial = 0

    def button(self, label: str) -> None:
        state = self.probe.observe()
        self.serial += 1
        frame = self.capture(self.probe, self.artifact, f"civil-click-{self.serial:03d}")
        if scene_shot.png_size(frame) != self.size:
            raise RuntimeError(f"GUI frame {frame} has wrong window size; expected {self.size}")
        if self.client == "native":
            found = [node for node in state.get("ui_nodes", [])
                     if node.get("role") == "button" and node.get("label") == label and node.get("enabled")]
            points = [(round(node["bounds"][0] + node["bounds"][2] / 2),
                       round(node["bounds"][1] + node["bounds"][3] / 2)) for node in found]
        else:
            if label not in self.labels:
                self.labels[label] = _label_bitmap(label, self.jar)
            points = _label_points(frame, self.labels[label], self.scale)
        if not points:
            # A button the page does not offer is a failed GUI check, like a title that never
            # arrives; the verdict mapping reads it from the assertions.
            self.probe.report["assertions"][f"GUI button {label}"] = False
            with runtime.phase("assertion"):
                raise RuntimeError(f"visible GUI button {label!r} not found; frame={frame}")
        # A dialog title/body may repeat the action label. Actions are below the
        # body; duplicated Back/exit actions have the same destination.
        point = max(points, key=lambda point: point[1])
        self.probe.report.setdefault("gui_clicks", []).append(
            {"label": label, "point": point, "frame": frame, "screen": state.get("screen")})
        self.input.click(point)

    def text_visible(self, label: str, frame: str) -> bool:
        if self.client == "native":
            for node in self.probe.observe().get("ui_nodes", []):
                if node.get("role") == "text" and node.get("label") == label:
                    x, y, width, height = node["bounds"]
                    if (x >= 0 and y >= 0 and width > 0 and height > 0
                            and x + width <= self.size[0] and y + height <= self.size[1]):
                        return True
            return False
        return bool(_label_points(frame, _label_bitmap(label, self.jar), self.scale))

    def slot(self, slot: int, *, shift: bool = False) -> None:
        state = self.probe.observe()
        container = state.get("container") or {}
        if container.get("menu_class") != "net.minecraft.world.inventory.ChestMenu":
            raise RuntimeError(f"not a visible ChestMenu: {state.get('screen')}")
        if self.client == "native":
            node = next((node for node in state.get("ui_nodes", []) if node.get("id") == f"slot/{slot}"), None)
            if node is None:
                raise RuntimeError(f"visible slot/{slot} not observed")
            x, y, width, height = node["bounds"]
            point = round(x + width / 2), round(y + height / 2)
        else:
            rows = (container["slot_count"] - 36) // 9
            point = _slot_point(self.size, self.scale, rows, slot)
        self.probe.report.setdefault("gui_clicks", []).append(
            {"slot": slot, "shift": shift, "point": point, "screen": state.get("screen")})
        if not shift:
            self.input.click(point)
            return
        # SDL may pump the whole X event batch before invoking the mouse callback.
        # Keep the physical modifier down until the received container acknowledges it.
        self.input._key("Shift_L", True)
        try:
            self.input.click(point)
            self.probe.wait(lambda received: received.get("container") != container,
                            f"GUI shift slot/{slot} applied", timeout_seconds=30)
        finally:
            self.input._key("Shift_L", False)

    def escape(self) -> None:
        self.input.tap("Escape")
        self.probe.wait(lambda state: not (state.get("screen") or {}).get("open"), "GUI closed")

    def close(self) -> None:
        self.input.close()

DIALOG = "net.minecraft.client.gui.screens.dialog.MultiButtonDialogScreen"
CONTAINER = "net.minecraft.client.gui.screens.inventory.ContainerScreen"
NAME = "mapqa"
RADIUS = 64
PALETTE = {
    "territory": (0x4C, 0x7A, 0x34),
    "you": (0xFF, 0x55, 0xFF),
    "storage": (0x9C, 0x6B, 0x30),
    "house": (0xE3, 0xC0, 0x8D),
}
TOLERANCE = 12
# Smallest plausible cell: one 8x8 glyph at GUI scale 1. The real cell size is
# measured from the viewer marker, which is exactly one cell (`_largest_patch`).
MIN_CELL = 8 * 8

# (en_us, ru_ru) text of every page the script waits for. `_script` passes the same pairs to its
# `label(...)`; test_settlement_map_qa checks that every pair there is listed here.
PAGES = {
    "hub": (f"Settlement {NAME}", f"Поселение {NAME}"),
    "settlement": ("Settlement", "Поселение"),
    "help": ("Help", "Помощь"),
    "map": ("Map ·", "Карта ·"),
    "house": ("House", "Дом"),
    "order": ("Order construction L2", "Заказать стройку L2"),
    "projects": ("Projects", "Стройка"),
    "status": ("Status", "Статус"),
    "construction": ("Construction", "Строительство"),
    "waiting": ("waiting for materials", "не хватает материалов"),
    "storage": ("Storage", "Склад"),
    "residents": ("Residents", "Жители"),
    "resident": ("Resident ·", "Житель ·"),
    "household": ("Household", "Домохозяйство"),
    "resources": ("Resources", "Ресурсы"),
    "orders": ("Production orders", "Заказы производства"),
    "new_order": ("New order", "Новый заказ"),
    "new_order_form": ("New production order", "Новый заказ производства"),
    "back": ("Back", "Назад"),
    "choose_station": ("Choose a workstation", "Выбор рабочего места"),
    "workstation": ("Cartographer workshop: crafting_table at", "Мастерская: Cartographer: crafting_table в"),
}

# The verdicts a receipt publishes in `details.verdicts` (registered as
# `regressions._BOOLEAN_PROFILE_VERDICTS["settlement-map-qa"]`): one per GUI stage the run
# walks, `localized_text` (every Russian title and text check, ru_ru runs only) and `all`.
STAGES = ("bell_opens_hub", "dashboard", "map", "building_order", "shortage_icons",
          "storage_transfer", "project_progress", "residents", "resources", "production_order")
VERDICTS = (*STAGES, "localized_text", "all")


def _stage_checks(language: str) -> dict[str, tuple[list[str], list[str]]]:
    """stage -> (required assertion patterns, extra patterns whose failure also fails the stage).

    Patterns are regexes that must match a whole assertion name of the run's report. The names
    are the labels `probe.wait`/`probe.check` record in `_script`, with the page text of the
    run's language; `Gui.button` records `GUI button <label>` only when the button is missing."""
    ru = language == "ru_ru"
    page = lambda key: PAGES[key][ru]
    menu = lambda key: re.escape(f"GUI menu {page(key)}")
    dialog = lambda key: re.escape(f"GUI dialog {page(key)}")
    button = lambda key: re.escape(f"GUI button {page(key)}")
    order = re.escape(page("order"))
    return {
        "bell_opens_hub": (["GUI bell target", dialog("hub")], ["GUI bell target before click"]),
        "dashboard": ([menu("settlement")], [button("settlement"), button("help")]),
        "map": (["dashboard_map_GUI", "map_viewer_rendered", "map_territory_rendered", "map_cell_GUI",
                 r"GUI menu \#\d+ "], [rf"GUI button {re.escape(page('house'))} \#\d+ L1"]),
        "building_order": ([rf"GUI dialog {order} \#\d+", dialog("projects")],
                           [rf"GUI button {order} \#\d+", button("status")]),
        "shortage_icons": ([menu("construction"), "upgrade_native_shortage_icons", dialog("waiting"),
                            "exact_shortage_rendered"], []),
        "storage_transfer": ([menu("storage"), r"GUI shift slot/\d+ applied",
                              "GUI_storage_transfer_conserves_one"], [button("storage")]),
        "project_progress": ([menu("projects"), menu("construction"), "GUI_native_progress_advanced",
                              "GUI_progress_matches_independent_placed_blocks"], []),
        "residents": ([menu("residents"), menu("resident"), menu("household")], []),
        "resources": ([menu("resources")], []),
        # Dashboard -> Production orders -> New order -> workstation -> recipe -> form -> confirm, then
        # the order is read back from the server and a resident has crafted for it.
        "production_order": ([menu("orders"), "production_workshop_certified", "production_new_order_entry",
                              menu("new_order_form"), "production_recipe_entry", dialog("new_order_form"),
                              dialog("orders"), "production_order_listed", "production_order_exists",
                              "production_order_resident_crafted"],
                             [button("new_order"), button("back")]),
        # The Russian run's page titles and its rendered shortage text: the checks the stages above
        # make with `label(en, ru)` text.
        "localized_text": ([dialog("hub"), menu("settlement"), "dashboard_map_GUI", menu("projects"),
                            menu("construction"), menu("storage"), menu("residents"), menu("resident"),
                            menu("household"), menu("resources"), rf"GUI dialog {order} \#\d+",
                            dialog("projects"), dialog("waiting"), "exact_shortage_rendered"], []),
    }


def verdicts_from_assertions(assertions: dict[str, Any], language: str) -> dict[str, bool | None]:
    """Map a run's recorded assertions to the published verdicts.

    A verdict is True when every assertion it requires was recorded True, and False when one of
    its assertions was recorded False. Once the run has failed a check, a verdict that never got
    its turn is False as well: the GUI path to it is broken, so it is not met. A run that
    recorded no failed check (a lost client or a timeout outside the checks) leaves unreached
    verdicts None, which the ledger reads as "not evidence" and never as red. `localized_text`
    is None on an en_us run (it has no Russian text to judge); `all` is True when every other
    verdict that applies is True."""
    checks = _stage_checks(language)

    def hits(pattern: str) -> list[str]:
        return [name for name in assertions if re.fullmatch(pattern, name)]

    verdicts: dict[str, bool | None] = {}
    for stage, (required, extra) in checks.items():
        if any(assertions[name] is False for pattern in (*required, *extra) for name in hits(pattern)):
            verdicts[stage] = False
        elif all(hits(pattern) and all(assertions[name] is True for name in hits(pattern))
                 for pattern in required):
            verdicts[stage] = True
        else:
            verdicts[stage] = None
    if language != "ru_ru":
        verdicts["localized_text"] = None
    applicable = [stage for stage in checks if stage != "localized_text" or language == "ru_ru"]
    failed = any(verdicts[stage] is False for stage in applicable)
    for stage in applicable:
        if verdicts[stage] is None and failed:
            verdicts[stage] = False
    verdicts["all"] = (True if all(verdicts[stage] is True for stage in applicable)
                       else False if failed else None)
    return {name: verdicts[name] for name in VERDICTS}


def run(ctx: ProfileContext) -> dict[str, Any]:
    parser = argparse.ArgumentParser(prog="settlement-map-qa")
    parser.add_argument("--gui-scale", type=int, choices=(2, 3), default=3)
    parser.add_argument("--language", choices=("en_us", "ru_ru"), default="en_us")
    parser.add_argument("--size", choices=("1280x720", "1920x1080"), default="1280x720")
    parser.add_argument("--wrong-click", action="store_true",
                        help="negative control: click Help instead of Settlement")
    extra = ctx.extra_args[1:] if ctx.extra_args[:1] == ["--"] else ctx.extra_args
    args = parser.parse_args(extra)
    if ctx.client != "java" and (args.gui_scale != 3 or args.language != "en_us" or args.size != "1280x720"):
        # The native client sends ClientInformation language "en_us" fixed (krasnokamen-client
        # src/protocol/network.rs `client_information`) and has no language or size option.
        raise ValueError("civil display variants require --client java")
    settings = {"gui_scale": args.gui_scale, "language": args.language,
                "screen_size": list(map(int, args.size.split("x")))} if ctx.client == "java" else {}
    def script(probe, artifact, steps, client):
        # Published whether the walk passes or stops: a receipt that failed on a GUI check says
        # which stage it was (`ledger._failing_verdict`). Before the script starts there are none.
        try:
            return _script(probe, artifact, steps, client, args)
        finally:
            published = verdicts_from_assertions(probe.report["assertions"], args.language)
            probe.report["verdicts"] = published
            ctx.client_evidence["verdicts"] = published
    return scene_shot.run(ctx, scenario={"steps": [], "java_plain": ctx.client == "java",
                                         "client_settings": settings}, script=script)


def _near(pixel: tuple[int, ...], color: tuple[int, int, int]) -> bool:
    return all(abs(a - b) <= TOLERANCE for a, b in zip(pixel, color))


def _pixels(path: str) -> dict[str, int]:
    counts = dict.fromkeys(PALETTE, 0)
    with Image.open(path) as image:
        for pixel in image.convert("RGB").getdata():
            for name, color in PALETTE.items():
                if _near(pixel, color):
                    counts[name] += 1
    return counts


def _largest_patch(path: str, color: tuple[int, int, int], baseline: str) -> int:
    """Pixel area of the largest 4-connected patch of `color` that the baseline shot does not
    have at the same pixels. Every map glyph is its own patch, so for the viewer colour this is
    one cell: the "You" legend swatch is a second patch, not added to it."""
    with Image.open(path) as image, Image.open(baseline) as before:
        if image.size != before.size:
            raise ValueError(f"{path} is {image.size}, baseline {baseline} is {before.size}")
        width = image.width
        pixels = image.convert("RGB").getdata()
        old = before.convert("RGB").getdata()
        left = {index for index, (pixel, prior) in enumerate(zip(pixels, old))
                if _near(pixel, color) and not _near(prior, color)}
    largest = 0
    while left:
        stack = [left.pop()]
        area = 0
        while stack:
            index = stack.pop()
            area += 1
            column = index % width
            for neighbour in (index - width, index + width,
                              index - 1 if column > 0 else -1,
                              index + 1 if column + 1 < width else -1):
                if neighbour in left:
                    left.remove(neighbour)
                    stack.append(neighbour)
        largest = max(largest, area)
    return largest


def _command(probe: ProbeSteps, command: str, expect: str) -> None:
    probe.fixture(command)
    probe.wait(lambda state: any(expect in str(line) for line in state.get("recent_chat", [])),
               f"chat {expect}", timeout_seconds=30)


def _enter_hub(probe: ProbeSteps, gui: Gui, bell: list[int], title: str) -> None:
    # Native villagers can push the standing survival player while a menu is open.
    player = probe.observe()["player"]
    dx, dz = bell[0] + .5 - player["x"], bell[2] + .5 - player["z"]
    dy = bell[1] + .5 - (player["y"] + 1.62)
    probe.call("minecraft_look", {
        "yaw_deg": round(math.degrees(math.atan2(-dx, dz))),
        "pitch_deg": round(-math.degrees(math.atan2(dy, math.hypot(dx, dz)))),
    })

    def on_bell(state: dict[str, Any]) -> bool:
        target = state.get("target") or {}
        return target.get("block_id") == "minecraft:bell" and [target.get(axis) for axis in ("x", "y", "z")] == bell

    probe.wait(on_bell, "GUI bell target", timeout_seconds=30)
    # The path re-enters the hub; the native client refuses to overwrite a retained frame.
    gui.serial += 1
    frame = gui.capture(probe, gui.artifact, f"civil-bell-target-{gui.serial:03d}")
    # Java 26.1.2 observed no crosshair target right after a screenshot, and a click
    # in that window used nothing. Press only on a target received after the frame.
    state = probe.wait(on_bell, "GUI bell target before click", timeout_seconds=30)
    point = (gui.size[0] // 2, gui.size[1] // 2)
    probe.report.setdefault("gui_clicks", []).append(
        {"bell_pixel": point, "target": state["target"], "time": state.get("time"), "frame": frame})
    if gui.client == "native":
        # World use is sampled by the Native client's tick, unlike GUI press edges.
        gui.input.move(*point)
        gui.input._button(3, True)
        try:
            _dialog(probe, title)
        finally:
            gui.input._button(3, False)
    else:
        gui.input.click(point, button=3)
        _dialog(probe, title)


def _dialog(probe: ProbeSteps, title: str) -> dict[str, Any]:
    return probe.wait(lambda state: bool((state.get("screen") or {}).get("open"))
                      and ".screens.dialog." in state["screen"].get("class", "")
                      and state["screen"].get("title") == title,
                      f"GUI dialog {title}", timeout_seconds=30)


def _menu(probe: ProbeSteps, title: str) -> dict[str, Any]:
    return probe.wait(lambda state: (state.get("screen") or {}).get("class") == CONTAINER
                      and str(state["screen"].get("title", "")).startswith(title)
                      and (state.get("container") or {}).get("slot_count") == 90,
                      f"GUI menu {title}", timeout_seconds=30)


def _enter_dashboard(probe: ProbeSteps, gui: Gui, label: str, title: str,
                     wrong_click: bool = False, help_label: str = "Help") -> dict[str, Any]:
    gui.button(help_label if wrong_click else label)
    return _menu(probe, title)


def _palette_point(path: str, color: tuple[int, int, int]) -> tuple[int, int]:
    """A real pixel in the largest rendered map-colour component."""
    with Image.open(path) as source:
        width = source.width
        left = {index for index, pixel in enumerate(source.convert("RGB").getdata()) if _near(pixel, color)}
    best: list[int] = []
    while left:
        todo = [left.pop()]
        component = []
        while todo:
            index = todo.pop()
            component.append(index)
            x = index % width
            for neighbour in (index - width, index + width,
                              index - 1 if x else -1, index + 1 if x + 1 < width else -1):
                if neighbour in left:
                    left.remove(neighbour)
                    todo.append(neighbour)
        if len(component) > len(best):
            best = component
    if not best:
        raise RuntimeError(f"map colour {color} not rendered: {path}")
    mean_x = sum(index % width for index in best) / len(best)
    mean_y = sum(index // width for index in best) / len(best)
    index = min(best, key=lambda index: (index % width - mean_x) ** 2 + (index // width - mean_y) ** 2)
    return index % width, index // width


def _needs(state: dict[str, Any]) -> dict[str, int]:
    result = {}
    for row in (state.get("container") or {}).get("slots", []):
        if 27 <= row["slot"] < 45:
            match = re.search(r" ×(\d+)$", str(row.get("name", "")))
            item = row.get("item_id")
            if match and int(match[1]) > 0 and isinstance(item, str) and re.fullmatch(r"[a-z0-9_.-]+:[a-z0-9_./-]+", item):
                result[item] = int(match[1])
    return result


def _held(state: dict[str, Any], item: str) -> int:
    return sum(int(row["count"]) for row in state["container"]["slots"]
               if row["slot"] >= 54 and row.get("item_id") == item)


def _stored(state: dict[str, Any], item: str) -> int:
    for row in state["container"]["slots"]:
        if row["slot"] < 45 and row.get("item_id") == item:
            count = re.search(r"×(\d+)$", str(row.get("name", "")))
            if count:
                return int(count[1])
    return 0


def _world(probe: ProbeSteps, bounds: list[int]) -> dict[tuple[int, int, int], str]:
    """Observe every cell using the bounds API shared by vanilla Java and native."""
    minimum = [bounds[0] - 4, bounds[1], bounds[2] - 4]
    maximum = [bounds[3] + 4, bounds[4], bounds[5] + 4]
    blocks = {}

    def scan(lo: list[int], hi: list[int]) -> None:
        arguments = {f"{edge}_{axis}": position[index]
                     for edge, position in (("min", lo), ("max", hi))
                     for index, axis in enumerate(("x", "y", "z"))}
        page = probe.call("minecraft_scan_blocks", {**arguments, "max_blocks": 4096})
        following = page.get("next_offset")
        if following is not None:
            if type(following) is not int or following <= 0:
                raise RuntimeError("civil upgrade scan pagination did not advance")
            # Native responses are byte-bounded. Java has no offset argument:
            # split the spatial box instead, retaining the same complete oracle.
            axis = max(range(3), key=lambda axis: hi[axis] - lo[axis])
            if lo[axis] == hi[axis]:
                raise RuntimeError("civil upgrade scan truncated a single cell")
            middle = (lo[axis] + hi[axis]) // 2
            lower, upper = hi.copy(), lo.copy()
            lower[axis], upper[axis] = middle, middle + 1
            scan(lo, lower)
            scan(upper, hi)
        else:
            blocks.update(((row["x"], row["y"], row["z"]), row["block_id"])
                          for row in page["blocks"])

    scan(minimum, maximum)
    volume = math.prod(hi - lo + 1 for lo, hi in zip(minimum, maximum))
    if len(blocks) != volume:
        raise RuntimeError(f"civil upgrade world scan incomplete: {len(blocks)}/{volume}")
    return blocks


def _house_cells(blueprint: dict[str, Any], level: int, origin: list[int], turns: int,
                 bed: str) -> dict[tuple[int, int, int], str]:
    from .village_autonomy_qa import _rotate_blueprint_xz
    layers = blueprint["levels"][level - 1]["layers"]
    width = max(len(candidate["layers"][0][0]) for candidate in blueprint["levels"])
    depth = max(len(candidate["layers"][0]) for candidate in blueprint["levels"])
    cells = {}
    for y, layer in enumerate(layers):
        for z, row in enumerate(layer):
            for x, symbol in enumerate(row):
                if symbol == ".":
                    continue
                block = blueprint["palette"][symbol].split("[", 1)[0]
                block = block if ":" in block else f"minecraft:{block}"
                if block.endswith("_bed"):
                    block = bed
                rx, rz = _rotate_blueprint_xz(x, z, width, depth, turns)
                cells[(origin[0] + rx, origin[1] + y, origin[2] + rz)] = block
    return cells


def _progress_matches_world(progress: tuple[int, int] | None,
                            before: dict[tuple[int, int, int], str],
                            after: dict[tuple[int, int, int], str],
                            expected: dict[tuple[int, int, int], str], ground: int) -> bool:
    if progress is None or not 0 < progress[0] <= progress[1]:
        return False
    placed = any(at[1] > ground and before.get(at) != block and after.get(at) == block
                 for at, block in expected.items())
    complete = all(after.get(at) == block for at, block in expected.items())
    return placed and (progress[0] < progress[1] or complete)


def _progress(state: dict[str, Any]) -> tuple[int, int] | None:
    for row in state["container"]["slots"]:
        if 18 <= row["slot"] < 27:
            match = re.fullmatch(r"(\d+)/(\d+)", row.get("name", ""))
            if match:
                return int(match[1]), int(match[2])
    return None


def _wait_for_world_progress(probe: ProbeSteps, gui: Gui, state: dict[str, Any],
                             before: dict[tuple[int, int, int], str],
                             expected: dict[tuple[int, int, int], str], scan_bounds: list[int],
                             projects_title: str, construction_title: str) -> tuple[int, int]:
    progress = None
    world_matches = False
    # Client ticks do not count native worker work; the scenario deadline bounds
    # ProbeSteps.call, just as for the authored L1 prerequisite below.
    while True:
        progress = _progress(state)
        if progress and progress[0] > 0:
            after = _world(probe, scan_bounds)
            world_matches = _progress_matches_world(progress, before, after, expected, scan_bounds[1])
            probe.report.setdefault("world_progress_samples", []).append({
                "counter": list(progress),
                "changed_matching_cells": sum(before.get(at) != block and after.get(at) == block
                                              for at, block in expected.items() if at[1] > scan_bounds[1]),
                "complete": all(after.get(at) == block for at, block in expected.items()),
            })
            if world_matches or progress[0] >= progress[1]:
                break
        probe.call("minecraft_wait_ticks", {"ticks": 40})
        gui.slot(45)
        _menu(probe, projects_title)
        gui.slot(52)
        state = _menu(probe, construction_title)
    probe.check("GUI_native_progress_advanced", bool(progress and progress[0] > 0))
    probe.check("GUI_progress_matches_independent_placed_blocks", world_matches)
    return progress


def _setup(probe: ProbeSteps) -> tuple[list[int], int, list[int]]:
    """Privileged prerequisites, explicitly not civil GUI behaviour evidence."""
    from .buildings_qa import _chunked_fills

    probe.report["setup_not_asserted"] = [
        "creative-only private pad preparation, then survival before settlement/player actions",
        "founded owned settlement, bell, certified physical funding chest",
        "summoned/bound residents and command-requested/funded authored House L1",
        "cleared fixture storage and inventory; later give commands supply test items",
        "building-list commands below wait only for the authored L1 prerequisite",
    ]
    for command in ("gamemode creative @s", "gamerule advance_time false",
                    "gamerule spawn_mobs false", "time set 6000", "weather clear", "clear @s"):
        probe.fixture(command)
    me = probe.observe(wait_for_play=True)["player"]
    x, z = (math.floor(float(me[axis])) for axis in ("x", "z"))
    y = 201
    probe.fixture(f"tp @s {x + .5} {y} {z - 2.0}")
    for command in _chunked_fills(x - 64, y - 2, z - 64, x + 64, y - 2, z + 64, "minecraft:dirt"):
        probe.fixture(command)
    for command in _chunked_fills(x - 64, y - 1, z - 64, x + 64, y - 1, z + 64, "minecraft:grass_block"):
        probe.fixture(command)
    for command in _chunked_fills(x - 64, y, z - 64, x + 64, y + 24, z + 64, "minecraft:air"):
        probe.fixture(command)
    probe.fixture(f"tp @s {x + .5} {y} {z + .5}")
    probe.wait(lambda state: all(state.get("player", {}).get(axis) == expected
                                for axis, expected in (("x", x + .5), ("y", y), ("z", z + .5))),
               "fixture_founder_at_anchor", timeout_seconds=30)
    probe.fixture("gamemode survival @s")
    _command(probe, f"settlement create {NAME} {RADIUS}", f"Founded {NAME}")
    _command(probe, f"settlement info {NAME}", f"anchor=[{x}, {y}, {z}]")
    probe.fixture(f"setblock {x} {y} {z} minecraft:bell")
    chest = [x + 20, y, z + 20]
    probe.fixture(f"setblock {chest[0]} {chest[1]} {chest[2]} minecraft:chest")
    _command(probe, f"settlement building certify storage {x + 19} {y - 1} {z + 19} {x + 21} {y + 2} {z + 21}",
             "Certified storage #")
    # Empty funding queues the owner request before residents can autonomously
    # select another project; all fixture stock then serves that native request.
    probe.fixture("settlement build house")
    taken: set[str] = set()
    candidates: list[dict[str, Any]] = []

    def arrived(_state: dict[str, Any]) -> bool:
        nonlocal candidates
        entities = probe.call("minecraft_list_entities", {"radius": 64})
        candidates = [entity for entity in entities.get("entities", entities.get("visible_entities", []))
                      if "villager" in str(entity.get("entity_type", entity.get("type", "")))
                      and entity.get("entity_uuid") not in taken]
        return bool(candidates)

    for index in range(6):
        probe.fixture(f"summon minecraft:villager {x + 5.5 + index} {y} {z + 5.5}")
        probe.wait(arrived, f"fixture_villager_{index}_received", timeout_seconds=30)
        worker = str(candidates[0]["entity_uuid"])
        taken.add(worker)
        probe.fixture(f"settlement bind {NAME} {worker}")
    materials = subprocess.run(
        [sys.executable, str(scene_shot.runtime.REPO_ROOT / "tools/blueprints.py"), "materials", "house"],
        check=True, capture_output=True, text=True, cwd=scene_shot.runtime.REPO_ROOT,
    )
    base = next(ast.literal_eval(line.split("base", 1)[1].strip())
                for line in materials.stdout.splitlines() if "L1:" in line and "base" in line)
    slot = 0
    for item, count in sorted(base.items()):
        remaining = math.ceil(count * 1.5) + 1
        while remaining:
            amount = min(64, remaining)
            if slot >= 27:
                raise RuntimeError("House L1 fixture exceeds physical funding chest")
            probe.fixture(f"item replace block {' '.join(map(str, chest))} container.{slot} with minecraft:{item} {amount}")
            remaining -= amount
            slot += 1
    building = None
    # wait_ticks counts client ticks, not completed native construction work.
    # ProbeSteps.call enforces the scenario deadline while certification is absent.
    while True:
        probe.call("minecraft_wait_ticks", {"ticks": 100})
        probe.fixture("settlement building list")
        probe.call("minecraft_wait_ticks", {"ticks": 5})
        chat = "\n".join(map(str, probe.observe().get("recent_chat", [])))
        match = re.search(
            r"#(\d+)\s+house\s+level\s+1\s+\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]"
            r"-\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]", chat, re.IGNORECASE)
        if match:
            building = int(match[1])
            bounds = list(map(int, match.groups()[1:]))
            break
    for slot in range(27):
        probe.fixture(f"item replace block {' '.join(map(str, chest))} container.{slot} with minecraft:air")
    probe.fixture("clear @s")
    probe.fixture(f"tp @s {x + .5} {y} {z - 2.0}")
    # A late teleport packet also restores its camera angles. Receive it before
    # aiming at the bell, rather than letting it overwrite the GUI approach look.
    probe.wait(lambda state: all(state.get("player", {}).get(axis) == expected
                                for axis, expected in (("x", x + .5), ("y", y), ("z", z - 2.0))),
               "fixture_bell_approach_received", timeout_seconds=30)
    probe.report["fixture"] = {"bell": [x, y, z], "funding_chest": chest,
                               "authored_house": building, "house_bounds": bounds,
                               "residents": sorted(taken)}
    return [x, y, z], building & 0xFFFF_FFFF, bounds


# The Production orders stage: a certified workshop east of the bell holding a crafting table, the
# stored hay one `open_hay` order needs, and the form's initial craft count (ui.rs `recipe_review`).
WORKSHOP_KIND = "cartographer"
PRODUCTION_RECIPE = "open_hay"
PRODUCTION_COUNT = 8
CRAFT_TIMEOUT = 900.0


def _workshop(probe: ProbeSteps, bell: list[int]) -> tuple[int, list[int]]:
    """Privileged fixture, not GUI evidence: a stone-floored cartographer workshop (its profession
    table and a crafting table) certified over a cleared box. Returns (building seq, table)."""
    x, y, z = bell
    low, high = (x + 28, y - 1, z - 14), (x + 32, y + 3, z - 10)
    table = [x + 31, y, z - 11]
    probe.fixture(f"fill {low[0]} {low[1]} {low[2]} {high[0]} {high[1]} {high[2]} minecraft:air")
    probe.fixture(f"fill {low[0]} {low[1]} {low[2]} {high[0]} {low[1]} {high[2]} minecraft:stone")
    probe.fixture(f"setblock {x + 29} {y} {z - 11} minecraft:cartography_table")
    probe.fixture(f"setblock {table[0]} {table[1]} {table[2]} minecraft:crafting_table")
    _command(probe, f"settlement building certify {WORKSHOP_KIND} {low[0]} {low[1]} {low[2]} "
                    f"{high[0]} {high[1]} {high[2]}", f"Certified {WORKSHOP_KIND} #")
    chat = "\n".join(map(str, probe.observe().get("recent_chat", [])))
    match = re.search(rf"Certified {WORKSHOP_KIND} #(\d+)", chat)
    probe.check("production_workshop_certified", bool(match))
    return int(match[1]) & 0xFFFF_FFFF, table


def _readback(probe: ProbeSteps, command: str, needle: str, label: str) -> str:
    """Send an owner command and return the first chat line with `needle` that is newer than a
    marker sent just before it (`recent_chat` is newest first; an answer can trail later chat)."""
    marker = f"mapqa {uuid.uuid4().hex[:8]}"
    probe.call("minecraft_send_chat", {"message": marker, "command": False})
    probe.wait(lambda state: any(marker in str(line) for line in state.get("recent_chat", [])),
               f"{label}_marker", timeout_seconds=30)
    probe.call("minecraft_send_chat", {"message": command, "command": True})
    state = probe.wait(lambda state: answer_after_marker(
        [str(line) for line in state.get("recent_chat", [])], marker, needle) is not None,
        label, timeout_seconds=30)
    return str(answer_after_marker([str(line) for line in state["recent_chat"]], marker, needle))


def _is_dialog(state: dict[str, Any], title: str) -> bool:
    screen = state.get("screen") or {}
    return (bool(screen.get("open")) and ".screens.dialog." in str(screen.get("class", ""))
            and screen.get("title") == title)


def _is_menu(state: dict[str, Any], title: str) -> bool:
    screen = state.get("screen") or {}
    return bool(screen.get("open")) and screen.get("class") == CONTAINER and screen.get("title") == title


def _production(probe: ProbeSteps, gui: Gui, label: Callable[[str, str], str],
                shot: Callable[[str], str], bell: list[int], residents: list[str],
                chest: list[int], hub: Callable[[], None], dashboard: Callable[[], None]) -> None:
    """Create a production order through the dialogs a player uses and judge it in server truth.

    Dashboard -> Production orders -> New order -> (workstation) -> recipe -> form -> New order.
    The server then lists the order and `settlement production <name> job <id>` shows one of the
    bound residents as its worker; the order is done only when that resident has crafted for it."""
    seq, table = _workshop(probe, bell)
    gui.escape()
    hub()
    dashboard()
    orders, form = label("Production orders", "Заказы производства"), label("New production order", "Новый заказ производства")
    gui.slot(28)
    state = _menu(probe, orders)
    shot("civil-production-empty")
    probe.check("production_list_empty_before_order",
                not any(row["slot"] < 45 for row in state["container"]["slots"]))
    entry = next((row for row in state["container"]["slots"]
                  if 45 <= row["slot"] < 54 and row.get("name") == label("New order", "Новый заказ")), None)
    probe.check("production_new_order_entry", entry is not None)
    gui.slot(entry["slot"])
    # One crafting table in a certified workshop opens the recipe page; several ask for the workstation.
    station = (f"#{seq} " + label("Cartographer workshop: crafting_table at",
                                  "Мастерская: Cartographer: crafting_table в")
               + f" {table[0]} {table[1]} {table[2]}")
    chooser = label("Choose a workstation", "Выбор рабочего места")
    state = probe.wait(lambda received: _is_menu(received, form) or _is_dialog(received, chooser),
                       "production_workstation_offered", timeout_seconds=30)
    if state["screen"].get("class") != CONTAINER:
        probe.report["production_workstation_choice"] = True
        gui.button(station)
    state = _menu(probe, form)
    shot("civil-production-recipes")
    recipe = next((row for row in state["container"]["slots"]
                   if row["slot"] < 45 and row.get("name") == PRODUCTION_RECIPE), None)
    probe.check("production_recipe_entry", recipe is not None)
    gui.slot(recipe["slot"])
    _dialog(probe, form)
    shot("civil-production-form")
    # The order needs `open_hay` input in real storage: one hay block per craft.
    probe.fixture(f"item replace block {chest[0]} {chest[1]} {chest[2]} container.26 "
                  f"with minecraft:hay_block {PRODUCTION_COUNT}")
    gui.button(label("New order", "Новый заказ"))
    _dialog(probe, orders)
    shot("civil-production-receipt")
    gui.button(label("Back", "Назад"))
    state = _menu(probe, orders)
    shot("civil-production-listed")
    listed = next((str(row.get("name", "")) for row in state["container"]["slots"] if row["slot"] == 0), "")
    job = listed.split(" ", 1)[0]
    probe.report["production_order_listed"] = listed
    probe.check("production_order_listed", re.fullmatch(rf"{re.escape(job)} · \d+/{PRODUCTION_COUNT}", listed) is not None
                and job.startswith("ui-"))
    gui.escape()

    def read() -> dict[str, Any]:
        return parse_json_line(_readback(probe, f"settlement production {NAME} job {job}", f'"id":"{job}"',
                                         "production_job_readback"))

    admitted = read()
    probe.report["production_order"] = {"job": job, "admitted": admitted}
    probe.check("production_order_exists",
                str(admitted["recipe"]).endswith(f":{PRODUCTION_RECIPE}") and admitted["count"] == PRODUCTION_COUNT
                and admitted["worker"] in residents and not admitted["cancelled"])
    deadline = time.monotonic() + CRAFT_TIMEOUT
    current = admitted
    while not current["receipts"] and time.monotonic() < deadline:
        probe.call("minecraft_wait_ticks", {"ticks": 100})
        current = read()
    probe.report["production_order"]["crafted"] = current
    probe.check("production_order_resident_crafted", bool(current["receipts"]) and current["done"] >= 1)


def _script(probe: ProbeSteps, artifact: Path, _steps: list[dict[str, Any]], client: str,
            args: argparse.Namespace) -> list[str]:
    size = tuple(map(int, args.size.split("x")))
    scene_shot.resize_window(probe.client.p4_window, size)
    gui = Gui(probe, artifact, client, args.gui_scale, size)
    shots: list[str] = []
    ru = args.language == "ru_ru"
    label = lambda en, russian: russian if ru else en
    probe.report["civil_display"] = {"size": size, "gui_scale": args.gui_scale, "language": args.language}
    probe.report["negative_control"] = args.wrong_click

    def shot(name: str) -> str:
        probe.call("minecraft_wait_ticks", {"ticks": 2})
        path = gui.capture(probe, artifact, name)
        probe.check(f"{name}_dimensions", scene_shot.png_size(path) == size)
        shots.append(path)
        return path

    def hub() -> None:
        _enter_hub(probe, gui, _bell, f"Поселение {NAME}" if ru else f"Settlement {NAME}")

    def dashboard() -> None:
        _enter_dashboard(probe, gui, label("Settlement", "Поселение"), label("Settlement", "Поселение"),
                         args.wrong_click, label("Help", "Помощь"))

    def project() -> dict[str, Any]:
        gui.slot(30)
        _menu(probe, label("Projects", "Стройка"))
        gui.slot(52)
        return _menu(probe, label("Construction", "Строительство"))

    try:
        _bell, seq, bounds = _setup(probe)
        from .buildings_qa import _load_blueprints
        from .village_autonomy_qa import _blueprint_placements
        blueprint = _load_blueprints()["house"]
        scan_bounds = list(bounds)
        scan_bounds[4] = bounds[1] + max(len(level["layers"]) for level in blueprint["levels"][:2]) - 1
        before_world = _world(probe, scan_bounds)
        beds = {block for at, block in before_world.items()
                if bounds[0] <= at[0] <= bounds[3] and bounds[2] <= at[2] <= bounds[5]
                and block.endswith("_bed")}
        if len(beds) != 1:
            raise RuntimeError(f"authored House fixture has no unique native bed variant: {beds}")
        record = {"id": seq, "level": 1, "min": bounds[:3], "max": bounds[3:]}
        placements = [placement for placement in _blueprint_placements(record, blueprint)
                      if all(before_world.get(at) == block for at, block in
                             _house_cells(blueprint, 1, placement["origin"],
                                          placement["rotation"], next(iter(beds))).items())]
        if len(placements) != 1:
            raise RuntimeError(f"authored House L1 placement is not uniquely observed: {len(placements)}")
        placement = placements[0]
        expected = _house_cells(blueprint, 2, placement["origin"], placement["rotation"], next(iter(beds)))
        probe.report["world_oracle"] = {"origin": placement["origin"], "rotation": placement["rotation"],
                                         "bed": next(iter(beds)), "expected_cells": len(expected),
                                         "scan_bounds": scan_bounds}
        hub()
        baseline = shot("civil-hub")
        dashboard()
        shot("civil-dashboard")
        gui.slot(10)
        state = probe.wait(lambda state: (state.get("screen") or {}).get("class") == DIALOG
                           and str(state["screen"].get("title", "")).startswith(label("Map ·", "Карта ·")),
                           "dashboard_map_GUI", timeout_seconds=30)
        map_title = state["screen"]["title"]
        map_frame = shot("civil-map")
        cell = _largest_patch(map_frame, PALETTE["you"], baseline)
        probe.check("map_viewer_rendered", cell >= MIN_CELL)
        probe.check("map_territory_rendered", _pixels(map_frame)["territory"] >= cell * 30)
        point = _palette_point(map_frame, PALETTE["house"])
        gui.input.move(*point)
        shot("civil-map-house-hover")
        probe.report.setdefault("gui_clicks", []).append({"map_house_pixel": point, "frame": map_frame})
        gui.input.click(point)
        selected = probe.wait(lambda state: (state.get("screen") or {}).get("open")
                              and state["screen"].get("title") != map_title, "map_cell_GUI", timeout_seconds=30)
        if selected["screen"]["class"] == DIALOG:
            shot("civil-map-overlaps")
            gui.button(f"{label('House', 'Дом')} #{seq} L1")
        _menu(probe, f"#{seq} ")
        shot("civil-building")
        gui.slot(52)
        review = f"{label('Order construction L2', 'Заказать стройку L2')} #{seq}"
        _dialog(probe, review)
        shot("civil-upgrade-review")
        gui.button(review)
        _dialog(probe, label("Projects", "Стройка"))
        shot("civil-request-receipt")
        gui.button(label("Status", "Статус"))
        state = _menu(probe, label("Construction", "Строительство"))
        # Existing native street work can finish before the queued upgrade
        # becomes the live shortage. Refresh the real card, never a command.
        for _ in range(30):
            needs = _needs(state)
            if any(count > 1 for count in needs.values()):
                break
            probe.call("minecraft_wait_ticks", {"ticks": 100})
            gui.slot(45)
            _menu(probe, label("Projects", "Стройка"))
            gui.slot(52)
            state = _menu(probe, label("Construction", "Строительство"))
        probe.check("upgrade_native_shortage_icons", any(count > 1 for count in needs.values()))
        probe.report["native_shortage"] = needs
        shot("civil-shortage")
        item = next(item for item, count in needs.items() if count > 1)
        material = next(row for row in state["container"]["slots"]
                        if 27 <= row["slot"] < 45 and row.get("item_id") == item)
        gui.slot(material["slot"])
        _dialog(probe, label("waiting for materials", "не хватает материалов"))
        detail = shot("civil-material-detail")
        name = str(material["name"]).rsplit(" ×", 1)[0]
        probe.check("exact_shortage_rendered", gui.text_visible(f"{name} ×{needs[item]}", detail))
        gui.button(label("Storage", "Склад"))
        state = _menu(probe, label("Storage", "Склад"))
        shot("civil-storage-empty")
        probe.fixture(f"give @s {item} 1")
        state = probe.wait(lambda state: _held(state, item) == 1, "fixture_transfer_item_received")
        before = _stored(state, item)
        own = next(row["slot"] for row in state["container"]["slots"]
                   if row["slot"] >= 54 and row.get("item_id") == item)
        gui.slot(own, shift=True)
        state = probe.wait(lambda state: _held(state, item) == 0 and _stored(state, item) == before + 1,
                           "GUI_storage_transfer_conserves_one", timeout_seconds=30)
        shot("civil-storage-transfer")
        for material, count in needs.items():
            remaining = count - int(material == item)
            while remaining:
                amount = min(64, remaining)
                probe.fixture(f"give @s {material} {amount}")
                state = probe.wait(lambda state: _held(state, material) >= amount, "fixture_supply_received")
                own = next(row["slot"] for row in state["container"]["slots"]
                           if row["slot"] >= 54 and row.get("item_id") == material)
                held = _held(state, material)
                gui.slot(own, shift=True)
                probe.wait(lambda state: _held(state, material) < held, "GUI_supply_deposit", timeout_seconds=30)
                remaining -= amount
        gui.escape()
        hub()
        dashboard()
        state = project()
        progress = _wait_for_world_progress(probe, gui, state, before_world, expected, scan_bounds,
                                           label("Projects", "Стройка"), label("Construction", "Строительство"))
        probe.report["native_progress"] = list(progress)
        shot("civil-project-progress")
        gui.slot(45)
        _menu(probe, label("Projects", "Стройка"))
        gui.slot(45)
        _menu(probe, label("Settlement", "Поселение"))
        gui.slot(14)
        _menu(probe, label("Residents", "Жители"))
        shot("civil-census")
        gui.slot(9)
        _menu(probe, label("Resident ·", "Житель ·"))
        shot("civil-resident")
        gui.slot(16)
        _menu(probe, label("Household", "Домохозяйство"))
        shot("civil-household")
        gui.escape()
        hub()
        dashboard()
        gui.slot(16)
        _menu(probe, label("Resources", "Ресурсы"))
        shot("civil-resources")
        fixture = probe.report["fixture"]
        _production(probe, gui, label, shot, _bell, fixture["residents"], fixture["funding_chest"], hub, dashboard)
        return shots
    finally:
        gui.close()
