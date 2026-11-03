"""Craft-drag scenario leg for the canonical native/Xvfb harness.

Call run(probe, artifact, owned_window) INSIDE the existing windowed native
lifecycle after joining an isolated operator-fixture world. This module never
starts a server, opens another client, takes a scene semaphore, or runs itself.
The caller owns admission, process lifetime, receipts, and negative-control run.
"""
from __future__ import annotations

import ctypes as c
import ctypes.util
from pathlib import Path
from typing import Any

from tools.harness.backends import native_client
from tools.harness.backends.native_v2 import _echo
from tools.harness.backends.probe_steps import ProbeSteps, container_slot


class Pointer:
    """XTest input directed solely to the harness-owned Xvfb window."""
    def __init__(self, owned: dict[str, Any]) -> None:
        self.x = c.CDLL(ctypes.util.find_library("X11"))
        self.xt = c.CDLL(ctypes.util.find_library("Xtst"))
        self.x.XOpenDisplay.restype = c.c_void_p
        self.x.XOpenDisplay.argtypes = [c.c_char_p]
        self.x.XWarpPointer.argtypes = [c.c_void_p, c.c_ulong, c.c_ulong, c.c_int, c.c_int,
                                       c.c_uint, c.c_uint, c.c_int, c.c_int]
        self.x.XSync.argtypes = [c.c_void_p, c.c_int]
        self.x.XCloseDisplay.argtypes = [c.c_void_p]
        self.xt.XTestFakeButtonEvent.argtypes = [c.c_void_p, c.c_uint, c.c_int, c.c_ulong]
        self.display = self.x.XOpenDisplay(owned["display"].encode())
        if not self.display:
            raise RuntimeError("cannot open owned Xvfb display")
        self.window = owned["window_id"]
        self.down = False

    def move(self, x: int, y: int) -> None:
        self.x.XWarpPointer(self.display, 0, self.window, 0, 0, 0, 0, x, y)
        self.x.XSync(self.display, 0)

    def button(self, down: bool) -> None:
        if not self.xt.XTestFakeButtonEvent(self.display, 1, int(down), 0):
            raise RuntimeError("owned Xvfb refused mouse button")
        self.down = down
        self.x.XSync(self.display, 0)

    def close(self) -> None:
        try:
            if self.down:
                self.button(False)
        finally:
            self.x.XCloseDisplay(self.display)


def center(state: dict[str, Any], slot: int) -> tuple[int, int]:
    node = next(node for node in state["ui_nodes"] if node["id"] == f"slot/{slot}")
    x, y, w, h = node["bounds"]
    return round(x + w / 2), round(y + h / 2)


def count(state: dict[str, Any], item: str) -> int:
    # Container slots include player inventory; never double-count inventory rows.
    return sum(int(row["count"]) for row in state["container"]["slots"]
               if row.get("item_id") == item) + (
        int(state["container"]["carried"].get("count", 0))
        if state["container"]["carried"].get("item_id") == item else 0
    )


def run(probe: ProbeSteps, artifact: Path, owned_window: dict[str, Any]) -> None:
    """Exercise an already opened player InventoryMenu with declared fixture items.

    Prerequisite: isolated operator world; player inventory open; exactly one
    oak log and one diamond, both in player slots, empty crafting grid/cursor.
    The owning harness must record clear/give fixtures separately via ProbeSteps.
    """
    from PIL import Image, ImageChops

    initial = probe.observe()
    probe.check("inventory_menu", initial["container"]["container_id"] == 0
                and initial["container"]["slot_count"] == 46
                and initial["screen"]["open"])
    probe.check("empty_grid_cursor", all(container_slot(initial, i)["count"] == 0 for i in range(5))
                and initial["container"]["carried"]["count"] == 0)
    probe.check("fixture_totals", count(initial, "minecraft:oak_log") == 1
                and count(initial, "minecraft:diamond") == 1)
    sources = [row for row in initial["container"]["slots"]
               if 9 <= row["slot"] <= 44 and row.get("item_id") == "minecraft:oak_log"
               and row.get("count") == 1]
    diamonds = [row for row in initial["container"]["slots"]
                if 9 <= row["slot"] <= 44 and row.get("item_id") == "minecraft:diamond"
                and row.get("count") == 1]
    probe.check("unique_fixture_sources", len(sources) == 1 and len(diamonds) == 1)
    source, diamond = sources[0]["slot"], diamonds[0]["slot"]
    pointer = Pointer(owned_window)
    try:
        # Identical off-menu cursor position isolates held-cursor pixels; screenshots
        # require independent visual inspection as well as this bounded delta.
        pointer.move(25, 25)
        before = native_client.capture(probe.client, artifact, "craft-drag-empty-cursor")
        pointer.move(*center(initial, source))
        pointer.button(True)
        picked = probe.wait(lambda s: s["container"]["carried"].get("item_id") == "minecraft:oak_log"
                            and s["container"]["carried"].get("count") == 1
                            and container_slot(s, source)["count"] == 0,
                            "drag_pickup_server_image")
        probe.check("pickup_server_origin", picked["container"]["carried"].get("origin") == "server_observed")
        pointer.move(25, 25)
        held = native_client.capture(probe.client, artifact, "craft-drag-held-cursor")
        probe.check("held_frame_fresh", held["frame_id"] != before["frame_id"])
        with Image.open(before["retained_path"]) as a, Image.open(held["retained_path"]) as b:
            probe.check("cursor_frames_same_size", a.size == b.size)
            # Covers the 16 GUI-pixel icon at supported GUI scales, clipped to window.
            box = (0, 0, min(80, a.width), min(80, a.height))
            diff = ImageChops.difference(a.convert("RGB").crop(box), b.convert("RGB").crop(box))
            changed = sum(1 for pixel in diff.getdata() if max(pixel) > 8)
        probe.report["cursor_region_changed_pixels"] = changed
        pointer.move(*center(picked, 1))
        pointer.button(False)
        moved = _echo(probe, "craft_drag_ordered_server_echo")
        after_release = native_client.capture(probe.client, artifact, "craft-drag-direct-release")
        probe.check("craft_drag_slot_cursor_and_result_received",
                    container_slot(moved, 1).get("item_id") == "minecraft:oak_log"
                    and container_slot(moved, 1).get("count") == 1
                    and moved["container"]["carried"].get("count") == 0
                    and container_slot(moved, 0).get("item_id") == "minecraft:oak_planks"
                    and container_slot(moved, 0).get("count") == 4)
        probe.check("held_cursor_region_changed", changed >= 8)
        probe.check("log_conserved_after_drag", count(moved, "minecraft:oak_log") == 1)
        probe.check("diamond_conserved_after_drag", count(moved, "minecraft:diamond") == 1)
        pointer.move(25, 25)
        after = native_client.capture(probe.client, artifact, "craft-drag-grid-and-result")
        probe.check("post_drop_frame_fresh", after["frame_id"] != held["frame_id"])
        with Image.open(held["retained_path"]) as a, Image.open(after["retained_path"]) as b:
            box = (0, 0, min(80, a.width), min(80, a.height))
            diff = ImageChops.difference(a.convert("RGB").crop(box), b.convert("RGB").crop(box))
            cleared = sum(1 for pixel in diff.getdata() if max(pixel) > 8)
        probe.report["cursor_region_cleared_pixels"] = cleared
        probe.check("empty_cursor_region_changed_again", cleared >= 8)

        # A diamond cannot be inserted into the nonmatching virtual output.
        pointer.move(*center(moved, diamond))
        pointer.button(True)
        carried = probe.wait(lambda s: s["container"]["carried"].get("item_id") == "minecraft:diamond"
                             and s["container"]["carried"].get("count") == 1,
                             "negative_pickup_server_image")
        pointer.move(*center(carried, 0))
        pointer.button(False)
        refused = _echo(probe, "negative_output_click_ordered_server_echo")
        probe.check("output_reject_keeps_cursor", refused["container"]["carried"].get("item_id") == "minecraft:diamond"
                    and refused["container"]["carried"].get("count") == 1)
        probe.check("output_reject_preserves_grid_and_result", container_slot(refused, 1).get("item_id") == "minecraft:oak_log"
                    and container_slot(refused, 1).get("count") == 1
                    and container_slot(refused, 0).get("item_id") == "minecraft:oak_planks"
                    and container_slot(refused, 0).get("count") == 4)
        probe.check("negative_conservation", count(refused, "minecraft:diamond") == 1
                    and count(refused, "minecraft:oak_log") == 1)
        native_client.capture(probe.client, artifact, "craft-drag-output-rejected")
        probe.report["craft_drag_scope"] = {
            "input": "owned Xvfb pointer down/move/up", "fixture": "operator-granted log and diamond",
            "frames": [before["frame_id"], held["frame_id"], after_release["frame_id"], after["frame_id"]],
            "visual_review_required": True,
            "limits": "does not prove generic in-flight reconciliation, all recipes or vanilla parity",
        }
    finally:
        pointer.close()
