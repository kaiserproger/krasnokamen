import io
import json
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import Mock, call, patch

from PIL import Image

from tools.harness.backends import settlement_map_qa as gate


class Probe:
    def __init__(self):
        self.state = {"screen": {"open": True, "class": gate.DIALOG, "title": "Settlement mapqa"}}
        self.fixtures = []

    def fixture(self, command):
        self.fixtures.append(command)
        raise AssertionError("navigation must not send a fixture command")

    def wait(self, predicate, label, **_):
        if not predicate(self.state):
            raise AssertionError(label)
        return self.state


class Gui:
    def __init__(self, probe):
        self.probe = probe
        self.buttons = []

    def button(self, label):
        self.buttons.append(label)
        self.probe.state = {
            "screen": {"open": True, "class": gate.CONTAINER if label == "Settlement" else gate.DIALOG,
                       "title": "Settlement" if label == "Settlement" else "Help"},
            "container": {"slot_count": 90},
        }


class SettlementMapQaTests(unittest.TestCase):
    def test_native_shortage_text_uses_rendered_nodes_without_a_java_font_jar(self):
        gui = object.__new__(gate.Gui)
        gui.client, gui.scale, gui.size = "native", 3, (1280, 720)
        gui.jar = Path("/nonexistent-civil-native-font.jar")
        gui.probe = Mock()
        node = {"role": "text", "label": "Cobblestone ×30", "bounds": [339, 282, 600, 27]}
        gui.probe.observe.return_value = {"ui_nodes": [node]}
        self.assertTrue(gui.text_visible("Cobblestone ×30", "native-material-detail.png"))
        self.assertFalse(gui.text_visible("Cobblestone ×31", "native-material-detail.png"))
        for bounds in ([1270, 282, 600, 27], [339, 710, 600, 27],
                       [339, 282, 0, 27], [-1, 282, 600, 27]):
            node["bounds"] = bounds
            self.assertFalse(gui.text_visible("Cobblestone ×30", "native-material-detail.png"))
        node["bounds"], node["role"] = [339, 282, 600, 27], "button"
        self.assertFalse(gui.text_visible("Cobblestone ×30", "native-material-detail.png"))

    def test_world_progress_waits_for_placed_blocks_not_thirty_client_tick_samples(self):
        probe, gui = Mock(), Mock()
        probe.report = {}
        probe.check.side_effect = lambda label, passed: self.assertTrue(passed, label)
        state = {"container": {"slots": [{"slot": 18, "name": "35/155"}]}}
        before = {(0, 201, 0): "minecraft:grass_block", (0, 202, 0): "minecraft:air"}
        expected = {(0, 201, 0): "minecraft:cobblestone", (0, 202, 0): "minecraft:oak_planks"}
        foundation_only = {**before, (0, 201, 0): "minecraft:cobblestone"}
        with patch.object(gate, "_world", side_effect=[foundation_only] * 34 + [expected]) as world, \
                patch.object(gate, "_menu", return_value=state):
            progress = gate._wait_for_world_progress(probe, gui, state, before, expected,
                                                    [0, 201, 0, 0, 202, 0], "Projects", "Construction")
        self.assertEqual(progress, (35, 155))
        self.assertEqual(world.call_count, 35)
        self.assertEqual(len(probe.report["world_progress_samples"]), 35)
        self.assertEqual(probe.report["world_progress_samples"][-1]["changed_matching_cells"], 1)
        probe.call.assert_called_with("minecraft_wait_ticks", {"ticks": 40})

    def test_shift_stays_down_until_the_received_container_change_and_releases_on_error(self):
        gui = object.__new__(gate.Gui)
        gui.client, gui.scale, gui.size = "java", 3, (1280, 720)
        gui.probe, gui.input = Mock(), Mock()
        gui.probe.report = {}
        container = {"menu_class": "net.minecraft.world.inventory.ChestMenu", "slot_count": 90,
                     "slots": [{"slot": 81, "item_id": "minecraft:cobblestone", "count": 1}]}
        gui.probe.observe.return_value = {"container": container}
        events = []

        def key(name, down):
            self.assertEqual(name, "Shift_L")
            events.append(("shift", down))

        def click(point, **_options):
            self.assertTrue(events, "modifier must be physically held before the mouse event")
            self.assertEqual(events[-1], ("shift", True))
            events.append(("click", point))

        def wait(predicate, _label, **_options):
            self.assertEqual(events[-1][0], "click")
            self.assertFalse(predicate({"container": container}))
            self.assertTrue(predicate({"container": {**container, "slots": []}}))
            events.append("received")

        gui.input._key.side_effect = key
        gui.input.click.side_effect = click
        gui.probe.wait.side_effect = wait
        gui.slot(81, shift=True)
        self.assertEqual(events, [("shift", True), ("click", (423, 642)), "received", ("shift", False)])
        gui.probe.wait.side_effect = RuntimeError("observer failed")
        with self.assertRaisesRegex(RuntimeError, "observer failed"):
            gui.slot(81, shift=True)
        self.assertEqual(events[-1], ("shift", False))

    def test_bell_click_waits_for_the_exact_received_crosshair_target(self):
        probe, gui = Mock(), Mock()
        probe.report = {}
        probe.observe.return_value = {"player": {
            "x": 127.94738365508307, "y": 200.9375, "z": -2.150989250297034}}
        gui.size, gui.serial = (1280, 720), 0
        events = []
        probe.call.side_effect = lambda name, arguments: events.append((name, arguments))
        target = {"type": "block", "x": 128, "y": 201, "z": 0, "block_id": "minecraft:bell"}

        def wait(predicate, label, **_options):
            if label.startswith("GUI bell target"):
                self.assertFalse(predicate({"target": {**target, "z": -2}}))
                self.assertFalse(predicate({"target": {"type": "miss"}}))
                # Java 26.1.2 received no target right after a screenshot.
                self.assertFalse(predicate({"target": None}))
                state = {"target": target}
            else:
                state = {"screen": {"open": True, "class": gate.DIALOG, "title": "Settlement mapqa"}}
            self.assertTrue(predicate(state))
            events.append(label)
            return state

        probe.wait.side_effect = wait
        gui.capture.side_effect = lambda *_args: events.append("frame") or "bell.png"
        gui.input.click.side_effect = lambda *args, **kwargs: events.append(("click", args, kwargs))
        gate._enter_hub(probe, gui, [128, 201, 0], "Settlement mapqa")
        self.assertEqual(events, [
            ("minecraft_look", {"yaw_deg": -12, "pitch_deg": 21}), "GUI bell target", "frame",
            "GUI bell target before click", ("click", ((640, 360),), {"button": 3}),
            "GUI dialog Settlement mapqa"])
        self.assertEqual(probe.report["gui_clicks"][0]["target"], target)
        probe.fixture.assert_not_called()

    def test_native_bell_press_survives_until_the_received_dialog_and_releases_on_error(self):
        for refuse in (False, True):
            with self.subTest(refuse=refuse):
                probe, gui = Mock(), Mock()
                probe.report = {}
                probe.observe.return_value = {"player": {"x": 128.5, "y": 201, "z": -2}}
                gui.client, gui.size, gui.serial = "native", (1280, 720), 0
                held = []
                gui.input._button.side_effect = lambda number, down: (
                    held.append(number) if down else held.remove(number))

                def wait(_predicate, label, **_options):
                    if label.startswith("GUI bell target"):
                        return {"target": {"block_id": "minecraft:bell", "x": 128, "y": 201, "z": 0}}
                    self.assertEqual(held, [3], "world use must span a client tick")
                    if refuse:
                        raise TimeoutError("dialog refused")
                    return {"screen": {"open": True}}

                probe.wait.side_effect = wait
                if refuse:
                    with self.assertRaisesRegex(TimeoutError, "dialog refused"):
                        gate._enter_hub(probe, gui, [128, 201, 0], "Settlement mapqa")
                else:
                    gate._enter_hub(probe, gui, [128, 201, 0], "Settlement mapqa")
                self.assertEqual(held, [])
                gui.input._button.assert_has_calls([call(3, True), call(3, False)])
                gui.input.click.assert_not_called()
                probe.fixture.assert_not_called()

    def test_reentering_the_hub_retains_each_bell_frame_under_its_own_name(self):
        probe, gui = Mock(), Mock()
        probe.report = {}
        probe.observe.return_value = {"player": {"x": 128.5, "y": 201, "z": -2}}
        gui.client, gui.size, gui.serial = "java", (1280, 720), 0
        probe.wait.side_effect = lambda _predicate, label, **_options: (
            {"target": {"block_id": "minecraft:bell", "x": 128, "y": 201, "z": 0}}
            if label.startswith("GUI bell target") else {"screen": {"open": True}})
        retained = []

        def capture(_probe, _artifact, name):
            # Native client_capture refuses an existing name; Java would overwrite the evidence.
            if name in retained:
                raise RuntimeError("artifact_create_failed: File exists (os error 17)")
            retained.append(name)
            return f"{name}.png"

        gui.capture.side_effect = capture
        for _ in range(3):
            gate._enter_hub(probe, gui, [128, 201, 0], "Settlement mapqa")
        self.assertEqual(len(set(retained)), 3)
        self.assertEqual([click["frame"] for click in probe.report["gui_clicks"]],
                         [f"{name}.png" for name in retained])

    def test_dashboard_is_answered_by_a_real_gui_action_not_a_menu_command(self):
        probe = Probe()
        gui = Gui(probe)
        gate._enter_dashboard(probe, gui, "Settlement", "Settlement")
        self.assertEqual(gui.buttons, ["Settlement"])
        self.assertEqual(probe.fixtures, [])

    def test_wrong_click_negative_control_fails_the_same_gui_oracle(self):
        probe = Probe()
        gui = Gui(probe)
        with self.assertRaisesRegex(AssertionError, "GUI menu Settlement"):
            gate._enter_dashboard(probe, gui, "Settlement", "Settlement", wrong_click=True)
        self.assertEqual(gui.buttons, ["Help"])
        self.assertEqual(probe.fixtures, [])

    def test_menu_oracle_refuses_a_dialog_with_the_expected_title(self):
        probe = Probe()
        probe.state["screen"]["title"] = "Settlement"
        probe.state["container"] = {"slot_count": 90}
        with self.assertRaises(AssertionError):
            gate._menu(probe, "Settlement")

    def test_pixels_count_every_map_palette_colour_within_tolerance(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "shot.png"
            image = Image.new("RGB", (4, 2))
            image.putpixel((0, 0), gate.PALETTE["territory"])
            image.putpixel((1, 0), (0x4C + 12, 0x7A - 12, 0x34))
            image.putpixel((2, 0), (0x4C + 13, 0x7A, 0x34))
            image.putpixel((3, 1), gate.PALETTE["storage"])
            image.save(path)
            self.assertEqual(gate._pixels(str(path)), {"territory": 2, "you": 0, "storage": 1, "house": 0})

    def test_cell_calibration_excludes_baseline_world_pixels(self):
        with tempfile.TemporaryDirectory() as tmp:
            baseline, shot = Path(tmp) / "main.png", Path(tmp) / "map.png"
            before = Image.new("RGB", (20, 8))
            before.paste(gate.PALETTE["you"], (14, 0, 18, 5))
            before.save(baseline)
            after = before.copy()
            after.paste(gate.PALETTE["you"], (1, 1, 4, 4))
            after.paste(gate.PALETTE["you"], (7, 1, 10, 4))
            after.putpixel((4, 4), gate.PALETTE["you"])
            after.save(shot)
            self.assertEqual(gate._largest_patch(str(shot), gate.PALETTE["you"], str(baseline)), 9)
            Image.new("RGB", (8, 8)).save(baseline)
            with self.assertRaises(ValueError):
                gate._largest_patch(str(shot), gate.PALETTE["you"], str(baseline))

    def test_map_click_point_is_a_real_pixel_of_the_largest_feature(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "map.png"
            image = Image.new("RGB", (20, 20))
            image.paste(gate.PALETTE["house"], (3, 7, 9, 13))
            image.putpixel((18, 18), gate.PALETTE["house"])
            image.save(path)
            x, y = gate._palette_point(str(path), gate.PALETTE["house"])
            self.assertTrue(3 <= x < 9 and 7 <= y < 13)
            with self.assertRaises(RuntimeError):
                gate._palette_point(str(path), gate.PALETTE["storage"])

    def test_vanilla_slot_geometry_includes_player_inventory_and_hotbar(self):
        self.assertEqual(gate._slot_point((1280, 720), 3, 6, 0), (423, 105))
        self.assertEqual(gate._slot_point((1280, 720), 3, 6, 54), (423, 468))
        self.assertEqual(gate._slot_point((1280, 720), 3, 6, 81), (423, 642))
        self.assertEqual(gate._slot_point((1920, 1080), 2, 6, 0), (816, 370))
        for rows, slot in ((0, 0), (7, 0), (6, -1), (6, 90)):
            with self.assertRaises(ValueError):
                gate._slot_point((1280, 720), 3, rows, slot)

    def test_exact_font_mask_does_not_accept_a_wrong_label(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            jar = root / "client.jar"
            atlas = Image.new("RGBA", (16, 8))
            atlas.paste((255, 255, 255, 255), (0, 0, 3, 7))
            atlas.paste((255, 255, 255, 255), (8, 0, 13, 5))
            data = io.BytesIO()
            atlas.save(data, format="PNG")
            with zipfile.ZipFile(jar, "w") as archive:
                archive.writestr("assets/minecraft/font/default.json", json.dumps({"providers": [
                    {"type": "space", "advances": {" ": 4}},
                    {"type": "bitmap", "file": "minecraft:font/test.png", "ascent": 7, "chars": ["AB"]},
                ]}))
                archive.writestr("assets/minecraft/textures/font/test.png", data.getvalue())
            mask = gate._label_bitmap("A B", jar)
            frame = Image.new("RGB", (120, 80))
            frame.paste((255, 255, 255), (14, 21), mask.resize((mask.width * 2, mask.height * 2), Image.Resampling.NEAREST))
            path = root / "frame.png"
            frame.save(path)
            self.assertTrue(gate._label_points(str(path), mask, 2))
            self.assertFalse(gate._label_points(str(path), gate._label_bitmap("B A", jar), 2))
            with self.assertRaises(ValueError):
                gate._label_bitmap("C", jar)

    def test_shortage_and_stock_readbacks_do_not_use_capped_icon_counts(self):
        state = {"container": {"slots": [
            {"slot": 27, "name": "Булыжник ×157", "count": 99, "item_id": "minecraft:cobblestone"},
            {"slot": 28, "name": "Oak Log ×0", "count": 0, "item_id": "minecraft:oak_log"},
            {"slot": 54, "name": "Cobblestone", "count": 8, "item_id": "minecraft:cobblestone"},
        ]}}
        self.assertEqual(gate._needs(state), {"minecraft:cobblestone": 157})
        self.assertEqual(gate._stored(state, "minecraft:cobblestone"), 157)
        self.assertEqual(gate._held(state, "minecraft:cobblestone"), 8)

    def test_progress_needs_independent_above_ground_placements_and_complete_is_not_constant(self):
        expected = {(0, 201, 0): "minecraft:oak_planks", (0, 202, 0): "minecraft:oak_log",
                    (1, 202, 0): "minecraft:oak_log"}
        before = {at: "minecraft:air" for at in expected}
        ground_only = {**before, (0, 201, 0): "minecraft:oak_planks"}
        partial = {**ground_only, (0, 202, 0): "minecraft:oak_log"}
        self.assertFalse(gate._progress_matches_world((1, 3), before, ground_only, expected, 201))
        self.assertFalse(gate._progress_matches_world((100, 100), before, partial, expected, 201))
        self.assertTrue(gate._progress_matches_world((2, 3), before, partial, expected, 201))
        self.assertTrue(gate._progress_matches_world((3, 3), before, expected, expected, 201))
        self.assertFalse(gate._progress_matches_world((4, 3), before, expected, expected, 201))
        self.assertFalse(gate._progress_matches_world(None, before, expected, expected, 201))

    def test_progress_uses_the_actual_fraction_not_other_icons(self):
        self.assertEqual(gate._progress({"container": {"slots": [
            {"slot": 4, "name": "100/100"}, {"slot": 18, "name": "unavailable"},
            {"slot": 19, "name": "7/103"},
        ]}}), (7, 103))

    def test_java_world_scan_uses_only_shared_bounded_scan_arguments(self):
        probe = Mock()
        rows = [{"x": x, "y": 201, "z": z, "block_id": "minecraft:air"}
                for x in range(-4, 5) for z in range(-4, 5)]

        def scan(name, arguments):
            self.assertEqual(name, "minecraft_scan_blocks")
            self.assertEqual(set(arguments), {
                "min_x", "min_y", "min_z", "max_x", "max_y", "max_z", "max_blocks"})
            return {"blocks": rows}

        probe.call.side_effect = scan
        self.assertEqual(len(gate._world(probe, [0, 201, 0, 0, 201, 0])), 81)

    def test_complete_world_scan_splits_byte_bounded_boxes_and_missing_cells_are_not_air(self):
        probe = Mock()
        rows = [{"x": x, "y": 201, "z": z, "block_id": "minecraft:air"}
                for x in range(-4, 5) for z in range(-4, 5)]

        def scan(name, arguments):
            visible = [row for row in rows if all(
                arguments[f"min_{axis}"] <= row[axis] <= arguments[f"max_{axis}"]
                for axis in ("x", "y", "z"))]
            return {"blocks": visible[:20], "next_offset": 20 if len(visible) > 20 else None}

        probe.call.side_effect = scan
        self.assertEqual(len(gate._world(probe, [0, 201, 0, 0, 201, 0])), 81)
        probe.call.side_effect = None
        probe.call.return_value = {"blocks": rows[:-1]}
        with self.assertRaisesRegex(RuntimeError, "incomplete"):
            gate._world(probe, [0, 201, 0, 0, 201, 0])
        probe.call.return_value = {"blocks": [], "next_offset": 0}
        with self.assertRaisesRegex(RuntimeError, "did not advance"):
            gate._world(probe, [0, 201, 0, 0, 201, 0])

    def test_native_house_fixture_wait_uses_scenario_deadline_not_sixty_client_waits(self):
        probe = Mock()
        probe.report = {}
        probe.observe.side_effect = [
            {"player": {"x": 128.5, "z": 0.5}},
            *[{"recent_chat": ["Buildings: storage only."]} for _ in range(60)],
            {"recent_chat": ["Buildings: #8589934594 house level 1 [112, 200, -20]-[118, 204, -14]."]},
        ]
        villagers = iter(f"00000000-0000-0000-0000-{index:012d}" for index in range(6))

        def call(name, _arguments):
            if name == "minecraft_list_entities":
                return {"entities": [{"entity_type": "minecraft:villager", "entity_uuid": next(villagers)}]}
            return {}

        def wait(predicate, label, **_options):
            if label == "fixture_bell_approach_received":
                self.assertFalse(predicate({"player": {"x": 128.5, "y": 201.0, "z": 0.5}}))
            state = {"recent_chat": ["Founded mapqa", "Certified storage #8589934593",
                                     "anchor=[128, 201, 0]"],
                     "player": {"x": 128.5, "y": 201.0,
                                "z": -2.0 if label == "fixture_bell_approach_received" else 0.5}}
            self.assertTrue(predicate(state))
            return state

        probe.call.side_effect = call
        probe.wait.side_effect = wait
        self.assertEqual(gate._setup(probe), ([128, 201, 0], 2, [112, 200, -20, 118, 204, -14]))
        self.assertEqual(probe.observe.call_count, 62)
        commands = [call.args[0] for call in probe.fixture.call_args_list]
        self.assertIn("gamemode creative @s", commands)
        self.assertIn("gamemode survival @s", commands)
        self.assertLess(commands.index("gamemode creative @s"), commands.index("tp @s 128.5 201 -2.0"))
        self.assertLess(commands.index("gamemode survival @s"), commands.index("settlement create mapqa 64"))
        labels = [call.args[1] for call in probe.wait.call_args_list]
        self.assertIn("fixture_founder_at_anchor", labels)
        self.assertIn("chat anchor=[128, 201, 0]", labels)
        self.assertEqual(labels[-1], "fixture_bell_approach_received")

    def test_house_world_oracle_uses_the_whole_blueprint_rotation_anchor(self):
        blueprint = {"palette": {"c": "cobblestone", "p": "oak_planks"},
                     "levels": [{"layers": [["cp"]]}, {"layers": [["cppp"]]}]}
        self.assertEqual(gate._house_cells(blueprint, 1, [10, 201, 20], 2, "minecraft:white_bed"),
                         {(13, 201, 20): "minecraft:cobblestone",
                          (12, 201, 20): "minecraft:oak_planks"})


# Assertions of the recorded real-client runs (their scene-shot/scenario.json, or
# details.scenario.assertions of the passing native run `134750-v4iye6sx`); only the
# GUI-check names matter here, the fixture and screenshot-size checks are kept as recorded.
RECORDED_WRONG_CLICK = {  # 140953-1275_cmr: Help clicked instead of Settlement, fails on the menu
    "in_play": True, "fixture_founder_at_anchor": True, "chat Founded mapqa": True,
    "chat anchor=[128, 201, 0]": True, "chat Certified storage #": True,
    "fixture_villager_0_received": True, "fixture_bell_approach_received": True,
    "GUI bell target": True, "GUI dialog Settlement mapqa": True, "civil-hub_dimensions": True,
    "GUI menu Settlement": False,
}
RECORDED_PASS = {  # 134750-v4iye6sx
    "in_play": True, "GUI bell target": True, "GUI closed": True,
    "GUI dialog Order construction L2 #2": True, "GUI dialog Projects": True,
    "GUI dialog Settlement mapqa": True, "GUI dialog waiting for materials": True,
    "GUI menu #2 ": True, "GUI menu Construction": True, "GUI menu Household": True,
    "GUI menu Projects": True, "GUI menu Resident ·": True, "GUI menu Residents": True,
    "GUI menu Resources": True, "GUI menu Settlement": True, "GUI menu Storage": True,
    "GUI shift slot/81 applied": True, "GUI_native_progress_advanced": True,
    "GUI_progress_matches_independent_placed_blocks": True, "GUI_storage_transfer_conserves_one": True,
    "GUI_supply_deposit": True, "chat Founded mapqa": True, "civil-hub_dimensions": True,
    "dashboard_map_GUI": True, "exact_shortage_rendered": True, "map_cell_GUI": True,
    "map_territory_rendered": True, "map_viewer_rendered": True, "upgrade_native_shortage_icons": True,
}


def passing_run(language):
    """The assertions a passing run of `language` records, written out from the page titles."""
    ru = language == "ru_ru"
    page = lambda key: gate.PAGES[key][ru]
    names = ["in_play", "GUI bell target", "GUI bell target before click", f"GUI dialog {page('hub')}",
             "dashboard_map_GUI", "map_viewer_rendered", "map_territory_rendered", "map_cell_GUI",
             "GUI menu #2 ", f"GUI dialog {page('order')} #2", f"GUI dialog {page('projects')}",
             "upgrade_native_shortage_icons", f"GUI dialog {page('waiting')}", "exact_shortage_rendered",
             "GUI shift slot/81 applied", "GUI_storage_transfer_conserves_one",
             "GUI_native_progress_advanced", "GUI_progress_matches_independent_placed_blocks"]
    names += [f"GUI menu {page(key)}" for key in ("settlement", "projects", "construction", "storage",
                                                  "residents", "resident", "household", "resources")]
    return dict.fromkeys(names, True)


class SettlementMapVerdictTests(unittest.TestCase):
    def test_verdict_names_are_the_ledger_registration(self):
        from tools.harness import regressions
        self.assertEqual(set(gate.VERDICTS), regressions._BOOLEAN_PROFILE_VERDICTS["settlement-map-qa"])
        self.assertEqual(set(gate.verdicts_from_assertions({}, "en_us")), set(gate.VERDICTS))

    def test_every_page_text_the_script_waits_for_is_in_the_page_table(self):
        import inspect
        import re
        source = inspect.getsource(gate._script) + inspect.getsource(gate._enter_dashboard)
        pairs = set(re.findall(r"""label\(\s*["']([^"']+)["'],\s*["']([^"']+)["']\s*\)""", source))
        self.assertGreater(len(pairs), 10)
        self.assertEqual({pair for pair in pairs if pair not in gate.PAGES.values()}, set())
        self.assertIn(f'f"Поселение {{NAME}}" if ru else f"Settlement {{NAME}}"', source)
        self.assertEqual(gate.PAGES["hub"], (f"Settlement {gate.NAME}", f"Поселение {gate.NAME}"))

    def test_a_passing_run_meets_every_verdict_in_both_languages(self):
        for language in ("en_us", "ru_ru"):
            verdicts = gate.verdicts_from_assertions(passing_run(language), language)
            expected = {name: True for name in gate.VERDICTS}
            if language == "en_us":
                expected["localized_text"] = None  # no Russian text to judge
            self.assertEqual(verdicts, expected, language)
        recorded = gate.verdicts_from_assertions(RECORDED_PASS, "en_us")
        self.assertEqual({name: value for name, value in recorded.items() if name != "localized_text"},
                         {name: True for name in gate.VERDICTS if name != "localized_text"})

    def test_recorded_wrong_click_run_fails_the_dashboard_verdict_and_everything_after_it(self):
        verdicts = gate.verdicts_from_assertions(RECORDED_WRONG_CLICK, "en_us")
        self.assertIs(verdicts["bell_opens_hub"], True)
        self.assertIs(verdicts["dashboard"], False)
        for stage in gate.STAGES[2:]:
            self.assertIs(verdicts[stage], False, stage)
        self.assertIs(verdicts["all"], False)
        self.assertIsNone(verdicts["localized_text"])

    def test_ledger_reads_the_recorded_wrong_click_run_as_red_and_a_pass_as_met(self):
        from tools.harness import ledger, regressions
        failed = {"status": "failed", "details": {"verdicts": gate.verdicts_from_assertions(RECORDED_WRONG_CLICK, "en_us")}}
        for check in ("dashboard", "storage_transfer", "all"):
            self.assertTrue(ledger._failing_verdict(failed, "settlement-map-qa", check), check)
        self.assertFalse(ledger._failing_verdict(failed, "settlement-map-qa", "bell_opens_hub"))
        passed = {"status": "passed", "details": {"verdicts": gate.verdicts_from_assertions(RECORDED_PASS, "en_us")}}
        for check in ("bell_opens_hub", "dashboard", "storage_transfer", "all"):
            regressions._profile_verdict(passed, "settlement-map-qa", check)
        with self.assertRaises(regressions.VerdictNotMet):
            regressions._profile_verdict(passed, "settlement-map-qa", "localized_text")

    def test_each_stage_goes_false_when_one_of_its_checks_fails(self):
        failing = {
            "bell_opens_hub": "GUI dialog Settlement mapqa", "dashboard": "GUI menu Settlement",
            "map": "map_territory_rendered", "building_order": "GUI dialog Projects",
            "shortage_icons": "exact_shortage_rendered", "storage_transfer": "GUI_storage_transfer_conserves_one",
            "project_progress": "GUI_native_progress_advanced", "residents": "GUI menu Household",
            "resources": "GUI menu Resources",
        }
        self.assertEqual(set(failing), set(gate.STAGES))
        for stage, name in failing.items():
            run = passing_run("en_us")
            run[name] = False
            verdicts = gate.verdicts_from_assertions(run, "en_us")
            self.assertIs(verdicts[stage], False, stage)
            self.assertIs(verdicts["all"], False, stage)

    def test_a_russian_title_that_never_arrives_fails_the_localized_text_verdict(self):
        run = passing_run("ru_ru")
        run["GUI menu Жители"] = False
        verdicts = gate.verdicts_from_assertions(run, "ru_ru")
        self.assertIs(verdicts["localized_text"], False)
        self.assertIs(verdicts["residents"], False)
        self.assertIs(verdicts["storage_transfer"], True)
        # the English run of the same scene says nothing about Russian text
        self.assertIsNone(gate.verdicts_from_assertions(passing_run("en_us"), "en_us")["localized_text"])

    def test_a_run_that_stops_without_a_failed_check_leaves_unreached_verdicts_unknown(self):
        reached = {name: True for name in ("GUI bell target", "GUI dialog Settlement mapqa", "GUI menu Settlement")}
        verdicts = gate.verdicts_from_assertions(reached, "en_us")
        self.assertIs(verdicts["bell_opens_hub"], True)
        self.assertIs(verdicts["dashboard"], True)
        for name in ("map", "storage_transfer", "resources", "all", "localized_text"):
            self.assertIsNone(verdicts[name], name)
        self.assertEqual(set(gate.verdicts_from_assertions({"in_play": True}, "ru_ru").values()), {None})
        # a lost connection is not a verdict about the GUI
        self.assertEqual(set(gate.verdicts_from_assertions({"in_play": False}, "en_us").values()), {None})

    def test_a_missing_button_is_a_failed_check_of_its_stage(self):
        gui = object.__new__(gate.Gui)
        gui.client, gui.scale, gui.size, gui.serial = "native", 3, (1280, 720), 0
        gui.artifact = Path("/nonexistent")
        gui.capture = Mock(return_value="frame.png")
        gui.probe = Mock()
        gui.probe.report = {"assertions": {}}
        gui.probe.observe.return_value = {"ui_nodes": [], "screen": {}}
        with patch.object(gate.scene_shot, "png_size", return_value=(1280, 720)):
            with self.assertRaisesRegex(RuntimeError, "visible GUI button 'Settlement' not found"):
                gui.button("Settlement")
        self.assertEqual(gui.probe.report["assertions"], {"GUI button Settlement": False})
        verdicts = gate.verdicts_from_assertions({"GUI bell target": True, "GUI dialog Settlement mapqa": True,
                                                  **gui.probe.report["assertions"]}, "en_us")
        self.assertIs(verdicts["bell_opens_hub"], True)
        self.assertIs(verdicts["dashboard"], False)

    def test_run_publishes_the_verdicts_when_the_walk_fails(self):
        from types import SimpleNamespace
        ctx = SimpleNamespace(client="native", extra_args=[], client_evidence={})
        probe = SimpleNamespace(report={"assertions": dict(RECORDED_WRONG_CLICK)})

        def walk(*_):
            raise RuntimeError("scene-shot assertion failed: GUI menu Settlement")

        def scene(ctx_, scenario, script):
            return script(probe, Path("/nonexistent"), [], "native")

        with patch.object(gate, "_script", walk), patch.object(gate.scene_shot, "run", scene):
            with self.assertRaisesRegex(RuntimeError, "GUI menu Settlement"):
                gate.run(ctx)
        self.assertIs(ctx.client_evidence["verdicts"]["dashboard"], False)
        self.assertIs(ctx.client_evidence["verdicts"]["bell_opens_hub"], True)
        self.assertEqual(probe.report["verdicts"], ctx.client_evidence["verdicts"])

    def test_ledger_refs_run_the_russian_check_on_java_and_the_rest_on_native(self):
        from tools.harness import regressions
        russian = regressions._profile_command("settlement-map-qa", "localized_text")
        self.assertEqual(russian[-8:], ["--client", "java", "--windowed", "--timeout-seconds", "3600",
                                        "--", "--language", "ru_ru"])
        for verdict in ("dashboard", "storage_transfer", "all"):
            command = regressions._profile_command("settlement-map-qa", verdict)
            self.assertEqual(command[-5:], ["--client", "native", "--windowed", "--timeout-seconds", "3600"])
        with self.assertRaises(ValueError):
            regressions._profile_command("settlement-map-qa", "production_order")

    def test_the_native_lane_refuses_russian_because_its_client_cannot_ask_for_it(self):
        from types import SimpleNamespace
        ctx = SimpleNamespace(client="native", extra_args=["--", "--language", "ru_ru"], client_evidence={})
        with self.assertRaisesRegex(ValueError, "require --client java"):
            gate.run(ctx)


if __name__ == "__main__":
    unittest.main()
