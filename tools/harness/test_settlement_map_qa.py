import tempfile
import unittest
from pathlib import Path

from PIL import Image

from tools.harness.backends import settlement_map_qa as gate


class Probe:
    """Records MCP calls and fixtures; the open dialog always has `title`."""

    def __init__(self, title: str) -> None:
        self.title = title
        self.calls: list[tuple[str, dict | None]] = []
        self.fixtures: list[str] = []

    def call(self, name, arguments=None):
        self.calls.append((name, arguments))
        return {}

    def fixture(self, command):
        self.fixtures.append(command)

    def wait(self, predicate, label, **_):
        state = {"screen": {"open": True, "class": gate.DIALOG, "title": self.title}}
        if not predicate(state):
            raise AssertionError(label)
        return state


class PointerProbe(Probe):
    """Map with two storage link runs at x 570..624 (window px); everything else is
    territory. The server answers a click with the Buildings page."""

    def __init__(self) -> None:
        super().__init__("Buildings - mapqa")
        self.report: dict = {}
        self.checks: dict[str, bool] = {}

    def observe(self, **_):
        return {"ui_nodes": [
            {"id": "dialog/body/0", "role": "text", "bounds": [300, 99, 600, 459]},
            {"id": "dialog/body/0/action/0", "role": "link", "bounds": [570, 369, 54, 27]},
            {"id": "dialog/body/0/action/1", "role": "link", "bounds": [570, 396, 54, 27]},
            {"id": "dialog/action/0", "role": "button", "bounds": [300, 600, 300, 60]},
        ]}

    def check(self, name, condition):
        self.checks[name] = condition
        if not condition:
            raise AssertionError(name)

    def call(self, name, arguments=None):
        self.calls.append((name, arguments))
        if name != "client_pointer":
            return {}
        storage = 570 <= arguments["x"] < 624
        return {"tooltip": "Storage #3 L1\n4 64 4 .. 6 67 6" if storage else "Territory, radius 16"}


class SettlementMapQaTests(unittest.TestCase):
    def test_pixels_count_palette_colours_within_tolerance(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "shot.png"
            image = Image.new("RGB", (4, 2))
            image.putpixel((0, 0), (0x4C, 0x7A, 0x34))
            image.putpixel((1, 0), (0x4C + 12, 0x7A - 12, 0x34))
            image.putpixel((2, 0), (0x4C + 13, 0x7A, 0x34))
            image.putpixel((3, 1), (0x9C, 0x6B, 0x30))
            image.save(path)
            self.assertEqual(gate._pixels(str(path)), {"territory": 2, "you": 0, "storage": 1})

    def test_cell_calibration_is_one_viewer_glyph_not_the_legend_swatch_or_world(self):
        magenta = gate.PALETTE["you"]

        def fill(image, left, top, width, height):
            for x in range(left, left + width):
                for y in range(top, top + height):
                    image.putpixel((x, y), magenta)

        with tempfile.TemporaryDirectory() as tmp:
            baseline, shot = Path(tmp) / "main.png", Path(tmp) / "map.png"
            before = Image.new("RGB", (20, 8))
            fill(before, 14, 0, 4, 5)  # magenta world pixels behind both dialogs
            before.save(baseline)
            after = before.copy()
            fill(after, 1, 1, 3, 3)  # viewer cell glyph
            fill(after, 7, 1, 3, 3)  # "You" legend swatch glyph
            after.putpixel((4, 4), magenta)  # touches the viewer only diagonally
            after.save(shot)
            delta = gate._pixels(str(shot))["you"] - gate._pixels(str(baseline))["you"]
            self.assertEqual(delta, 19, "the whole-screen delta counts both glyphs")
            self.assertEqual(gate._largest_patch(str(shot), magenta, str(baseline)), 9)
            Image.new("RGB", (8, 8)).save(baseline)
            with self.assertRaises(ValueError):
                gate._largest_patch(str(shot), magenta, str(baseline))

    def test_native_presses_the_button_and_java_sends_the_page_command(self):
        native = Probe("Map - mapqa")
        gate._go(native, "native", "Settlement mapqa", "Map", "map", "Map - mapqa")
        self.assertEqual(native.calls[0], ("minecraft_click_screen_button", {
            "expected_screen_class": gate.DIALOG, "expected_title": "Settlement mapqa",
            "button_label": "Map",
        }))
        self.assertEqual(native.fixtures, [])
        java = Probe("Map - mapqa")
        gate._go(java, "java", "Settlement mapqa", "Map", "map", "Map - mapqa")
        self.assertEqual(java.fixtures, ["settlement menu map"])
        self.assertNotIn("minecraft_click_screen_button", [name for name, _ in java.calls])

    def test_wrong_next_dialog_fails(self):
        with self.assertRaises(AssertionError):
            gate._go(Probe("Settlement mapqa"), "native", "Settlement mapqa", "Map", "map", "Map - mapqa")

    def test_native_cells_hover_territory_and_storage_then_click_the_storage_run(self):
        probe = PointerProbe()
        shots = []
        gate._cells(probe, "Map - mapqa", shots.append)
        pointer = [args for name, args in probe.calls if name == "client_pointer"]
        common = {"expected_screen_class": gate.DIALOG, "expected_title": "Map - mapqa"}
        self.assertEqual(pointer, [
            {**common, "action": "move", "x": 556.5, "y": 382.5},
            {**common, "action": "move", "x": 597.0, "y": 382.5},
            {**common, "action": "click", "x": 597.0, "y": 382.5},
        ])
        self.assertEqual(shots, ["map-hover-storage", "map-cell-click"])
        self.assertTrue(all(probe.checks.values()))
        self.assertEqual(probe.report["map_links"], 2)

    def test_a_click_is_answered_by_the_buildings_page_or_the_building_page(self):
        def state(title):
            return {"screen": {"open": True, "title": title}}
        self.assertTrue(gate._opened_building(state("Buildings - mapqa"), "3"))
        self.assertTrue(gate._opened_building(state("#3 Storage L1"), "3"))
        self.assertFalse(gate._opened_building(state("#31 Storage L1"), "3"))
        self.assertFalse(gate._opened_building(state("Map - mapqa"), "3"))
        self.assertFalse(gate._opened_building({"screen": {"open": False, "title": "Buildings - mapqa"}}, "3"))


if __name__ == "__main__":
    unittest.main()
