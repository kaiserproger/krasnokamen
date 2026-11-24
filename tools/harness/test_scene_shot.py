import json
import math
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness import scenarios
from tools.harness.backends import scene_shot


class SceneShotTests(unittest.TestCase):
    def test_gait_fixture_keeps_previous_subject_outside_spider_view(self):
        fixture = json.loads(Path(__file__).with_name("gait_phase_scene.json").read_text())
        camera = None
        subjects = {}
        for step in fixture["steps"]:
            if "tp" in step:
                camera = step["tp"]
            command = step.get("cmd", "").split()
            if command[:1] == ["summon"]:
                subjects[command[1]] = tuple(map(float, command[2:5]))
            if step.get("walk_capture", {}).get("entity_type") == "minecraft:spider":
                spider = subjects["minecraft:spider"]
                zombie = subjects["minecraft:zombie"]
                self.assertIsNotNone(camera)
                self.assertAlmostEqual(camera[0], spider[0])
                self.assertEqual(camera[3], 0)  # Looking down +Z along the spider's approach.
                # Include a two-block body margin around the dying zombie's lane.
                lateral = abs(zombie[0] - camera[0]) - 2
                forward = zombie[2] - camera[2]
                self.assertGreater(lateral, forward * math.tan(math.radians(35)))
                return
        self.fail("gait fixture has no spider capture")

    def test_gait_spider_oracle_rejects_death_tinted_occlusion(self):
        from PIL import Image, ImageDraw
        class Probe:
            def __init__(self):
                self.report = {}
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            image = root / "spider.png"
            (root / "spider-phase-walk.json").write_text(json.dumps({"attempts": [{"image": str(image)}]}))
            frame = Image.new("RGB", scene_shot.WINDOW, (50, 90, 50))
            draw = ImageDraw.Draw(frame)
            draw.rectangle((580, 420, 700, 465), fill=(30, 25, 25))
            frame.save(image)
            scene_shot.check_gait_spider(Probe(), root)
            draw.rectangle((650, 410, 750, 475), fill=(130, 40, 80))
            frame.save(image)
            with self.assertRaisesRegex(AssertionError, "spider_unobstructed"):
                scene_shot.check_gait_spider(Probe(), root)
            Image.new("RGB", scene_shot.WINDOW, (50, 90, 50)).save(image)
            with self.assertRaisesRegex(AssertionError, "spider_body_visible"):
                scene_shot.check_gait_spider(Probe(), root)

    def test_java_chat_hidden_is_opt_in(self):
        self.assertIn("chatVisibility:0\n", scene_shot.java_options(8, 70))
        self.assertIn("chatVisibility:2\n", scene_shot.java_options(8, 70, hide_chat=True))

    def test_java_bootstrap_retries_only_early_client_thread_timeout(self):
        from types import SimpleNamespace
        process = SimpleNamespace(poll=lambda: None)
        timeout = RuntimeError('minecraft_observe failed: {"code": "tool-execution-failed", "message": "TimeoutException"}')
        with patch.object(scene_shot.runtime, "wait_client_ready_for_connect",
                          side_effect=[timeout, {"current_screen": "TitleScreen"}]) as ready:
            self.assertEqual(scene_shot.wait_java_bootstrap(object(), process, 30)["current_screen"], "TitleScreen")
            self.assertEqual(ready.call_count, 2)
        with patch.object(scene_shot.runtime, "wait_client_ready_for_connect",
                          side_effect=RuntimeError("unrelated protocol failure")) as ready:
            with self.assertRaisesRegex(RuntimeError, "unrelated protocol failure"):
                scene_shot.wait_java_bootstrap(object(), process, 30)
            self.assertEqual(ready.call_count, 1)

    def test_console_and_operator_level_options_are_validated(self):
        with tempfile.TemporaryDirectory() as tmp:
            ctx = scene_shot.ProfileContext(Path(tmp), client='native', windowed=True)
            for scenario, message in (({'steps': [], 'server_console': 1}, 'server_console'),
                                      ({'steps': [], 'operator_level': 5}, 'operator_level'),
                                      ({'steps': [], 'operator_level': 0}, 'operator_level')):
                with self.assertRaisesRegex(ValueError, message):
                    scene_shot.validate_scenario(ctx, scenario)
            self.assertEqual(scene_shot.validate_scenario(
                ctx, {'steps': [], 'server_console': True, 'operator': False, 'operator_level': 2})[0]['operator_level'], 2)

    def test_operator_fixture_writes_the_requested_level(self):
        with tempfile.TemporaryDirectory() as tmp:
            ctx = scene_shot.ProfileContext(Path(tmp), client='native', windowed=True)
            scene_shot.prepare_operator_fixture(ctx, ['SceneShot'], 2)
            entries = json.loads((ctx.artifact_dir / 'ops.json').read_text())
            self.assertEqual([(entry['name'], entry['level']) for entry in entries], [('SceneShot', 2)])
            scene_shot.prepare_operator_fixture(ctx, ['SceneShot'])
            self.assertEqual(json.loads((ctx.artifact_dir / 'ops.json').read_text())[0]['level'], 4)

    def test_require_entities_fails_before_shot_when_summon_did_not_create_subject(self):
        class Probe:
            def __init__(self, entities):
                self.entities = entities
                self.calls = []
            def call(self, name, args):
                self.calls.append((name, args))
                if name == "minecraft_wait_for_play":
                    return {"in_play": True}
                if name == "minecraft_list_entities":
                    return {"entities": self.entities}
                return {"status": "ok"}
            def remaining(self):
                return 10
            def check(self, name, condition):
                if not condition:
                    raise AssertionError(name)

        step = {"require_entities": {"name": "subjects", "args": {"radius": 16},
                                     "types": ["minecraft:zombie", "minecraft:slime"]}}
        scenario = [{"cmd": "summon minecraft:invalid"}, step, {"shot": "frame"}]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for client in ("native", "java"):
                with self.subTest(client=client), patch.object(scene_shot, f"capture_{client}", return_value="frame") as capture:
                    probe = Probe([{"entity_type": "minecraft:zombie"}])
                    with self.assertRaisesRegex(AssertionError, "subjects_minecraft:slime"):
                        scene_shot.execute(probe, root, scenario, client)
                    capture.assert_not_called()
                    self.assertEqual(json.loads((root / "subjects.json").read_text())["entities"], probe.entities)
                    probe.entities.append({"entity_type": "minecraft:slime"})
                    self.assertEqual(scene_shot.execute(probe, root, [step, {"shot": "frame"}], client), ["frame"])
                    self.assertIn(("minecraft_list_entities", {"radius": 16}),
                                  list(scene_shot.scene_mcp_calls([step])))
                    probe.entities = None
                    with self.assertRaisesRegex(RuntimeError, "entity snapshot omitted entities"):
                        scene_shot.execute(probe, root, [step], client)

    def test_entity_count_requires_complete_snapshot(self):
        class Probe:
            def __init__(self, result):
                self.result = result
                self.checks = {}
            def call(self, name, args):
                return self.result
            def check(self, name, condition):
                self.checks[name] = bool(condition)
                if not condition:
                    raise AssertionError(name)

        golem = {"entity_type": "minecraft:iron_golem", "x": 0, "y": 0, "z": 0}
        zombie = {"entity_type": "minecraft:zombie", "x": 0, "y": 0, "z": 0}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for entities, minimum, maximum in (([zombie], 0, 0), ([golem], 1, 1)):
                step = {"assert_entities": {"name": "golems", "args": {"limit": 1},
                                           "entity_type": "minecraft:iron_golem",
                                           "min": minimum, "max": maximum}}
                for status in (True, None, 0):
                    result = {"entities": entities, "game_time": 1201}
                    if status is not None:
                        result["truncated"] = status
                    with self.subTest(entities=entities, status=status):
                        with self.assertRaisesRegex(RuntimeError, "truncated"):
                            scene_shot.execute(Probe(result), root, [step])
                probe = Probe({"entities": entities, "game_time": 1201, "truncated": False})
                scene_shot.execute(probe, root, [step])
                self.assertTrue(probe.checks["golems_min"] and probe.checks["golems_max"])

    def test_assert_block_compares_the_client_block_read(self):
        class Probe:
            def __init__(self, result):
                self.result = result
                self.checks = {}
            def call(self, name, args):
                self.asserted = (name, args)
                return self.result
            def check(self, name, condition):
                self.checks[name] = bool(condition)
                if not condition:
                    raise AssertionError(name)

        step = {"assert_block": {"name": "kelp_stands", "args": {"x": 1, "y": 2, "z": 3},
                                 "expect": "minecraft:kelp"}}
        self.assertIn(("minecraft_read_block", {"x": 1, "y": 2, "z": 3}),
                      list(scene_shot.scene_mcp_calls([step])))
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            probe = Probe({"x": 1, "y": 2, "z": 3, "block_id": "minecraft:kelp", "is_air": False})
            scene_shot.execute(probe, root, [step])
            self.assertEqual(probe.asserted[0], "minecraft_read_block")
            self.assertTrue(probe.checks["kelp_stands"])
            washed = Probe({"x": 1, "y": 2, "z": 3, "block_id": "minecraft:water", "is_air": False})
            with self.assertRaisesRegex(AssertionError, "kelp_stands"):
                scene_shot.execute(washed, root, [step])
            air = {"assert_block": {"name": "headroom", "args": {"x": 4, "y": 5, "z": 6},
                                    "expect": "minecraft:air"}}
            cleared = Probe({"x": 4, "y": 5, "z": 6, "block_id": "minecraft:air", "is_air": True})
            scene_shot.execute(cleared, root, [air])
            self.assertTrue(cleared.checks["headroom"])

    def test_assert_screen_joins_title_dialog_and_slot_text(self):
        class Probe:
            def __init__(self, state):
                self.state = state
                self.checks = {}
            def observe(self):
                return self.state
            def check(self, name, condition):
                self.checks[name] = bool(condition)
                if not condition:
                    raise AssertionError(name)

        step = {"assert_screen": {"name": "project_label", "contains": ["Wall section L1"],
                                  "excludes": ["Unknown blueprint"]}}
        self.assertIn(("minecraft_observe", {}), list(scene_shot.scene_mcp_calls([step])))
        state = {"screen": {"title": "Projects", "class": "x.dialog.DialogScreen"},
                 "ui_nodes": [{"id": "dialog/body/0", "label": "Wall section L1 · Prepared 0/24"}],
                 "container": {"slots": [{"slot": 0, "name": "Wall section L1"}]}}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            probe = Probe(state)
            scene_shot.execute(probe, root, [step])
            self.assertTrue(probe.checks["project_label_contains_0"])
            self.assertTrue(probe.checks["project_label_excludes_0"])
            unknown = Probe({"screen": {"title": "Projects"},
                             "ui_nodes": [{"id": "dialog/body/0", "label": "Unknown blueprint L1"}]})
            with self.assertRaisesRegex(AssertionError, "project_label_contains_0"):
                scene_shot.execute(unknown, root, [step])

    def test_new_assertion_steps_are_native_only_and_strict(self):
        for step in ({"assert_block": {"name": "b", "args": {"x": 1, "y": 2}, "expect": "minecraft:air"}},
                     {"assert_block": {"name": "b", "args": {"x": 1, "y": 2, "z": 3}, "expect": "air"}},
                     {"assert_screen": {"name": "s", "contains": [], "excludes": []}},
                     {"assert_screen": {"name": "s", "contains": ["x"]}}):
            with self.subTest(step=step):
                with self.assertRaises(ValueError):
                    scene_shot.validate_steps([step], "native")
                with self.assertRaises(ValueError):
                    scene_shot.validate_steps([{"assert_block": {"name": "b", "args": {"x": 1, "y": 2, "z": 3},
                                                                 "expect": "minecraft:air"}}], "java")

    def test_require_entities_rejects_empty_or_invalid_type_lists(self):
        for types in ([], ["zombie"], [""], [None]):
            step = {"require_entities": {"name": "subjects", "args": {}, "types": types}}
            for client in ("native", "java"):
                with self.subTest(types=types, client=client), self.assertRaises(ValueError):
                    scene_shot.validate_steps([step], client)

    def test_walk_capture_checks_render_frame_not_later_observation(self):
        spec = {"name": "walk", "entity_type": "minecraft:zombie", "phase": 1.5,
                "tolerance": 0.25, "min_speed": 0.1, "max_ticks": 30}
        entity = {"entity_type": "minecraft:zombie", "entity_uuid": "mob",
                  "walk_position": 1.4, "walk_speed": 0.2}

        class Probe:
            def __init__(self):
                self.calls = 0
            def call(self, name, args):
                if name == "minecraft_wait_ticks":
                    raise AssertionError("capture should start at bracket entry without waiting")
                self.calls += 1
                return {"entities": [entity if self.calls == 1 else {**entity, "walk_position": 1.6}]}
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = str(root / "walk.png")
            for client in ("java", "native"):
                frame_path = Path(image + ".gait.json") if client == "java" else Path(image).with_suffix(".json")
                def write_frame(position):
                    items = [{**entity, "walk_position": position}]
                    frame_path.write_text(json.dumps(items if client == "java" else {"gait_frame": items}))
                with patch.object(scene_shot, f"capture_{client}", return_value=image):
                    write_frame(1.5)
                    self.assertEqual(scene_shot.execute(Probe(), root, [{"walk_capture": spec}], client), [image])
                    evidence = json.loads((root / "walk-walk.json").read_text())
                    self.assertEqual(evidence["after"]["entities"][0]["walk_position"], 1.6)
                    # A frame that lags below the bracket is resampled instead of
                    # waited out: the next rendered frame inside it is accepted.
                    frames = iter((1.2, 1.5))
                    def capture_next(*_args):
                        write_frame(next(frames))
                        return image
                    with patch.object(scene_shot, f"capture_{client}", side_effect=capture_next):
                        self.assertEqual(scene_shot.execute(Probe(), root, [{"walk_capture": spec}], client), [image])
                    evidence = json.loads((root / "walk-walk.json").read_text())
                    self.assertEqual([attempt["frame"]["entities"][0]["walk_position"]
                                      for attempt in evidence["attempts"]], [1.2, 1.5])
                    # A frame already past the bracket is a terminal miss: resampling
                    # cannot land inside it, so no tick is waited either.
                    write_frame(2.5)
                    with self.assertRaisesRegex(RuntimeError, "walk phase not reached"):
                        scene_shot.execute(Probe(), root, [{"walk_capture": spec}], client)

    def test_walk_capture_retries_interpolated_frame_without_widening_phase(self):
        spec = {"name": "walk", "entity_type": "minecraft:spider", "phase": 1.5,
                "tolerance": 0.45, "min_speed": 0.1, "max_ticks": 3}
        entity = {"entity_type": "minecraft:spider", "entity_uuid": "mob",
                  "walk_position": 1.467, "walk_speed": 0.6}

        class Probe:
            def __init__(self):
                self.tick = 0
            def call(self, name, args):
                if name == "minecraft_wait_ticks":
                    raise AssertionError("waiting can skip the interpolated phase")
                return {"entities": [{**entity, "walk_position": (1.467, 2.05)[min(self.tick, 1)]}]}
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = str(root / "walk.png")
            positions = iter((0.9, 1.6))
            def capture(*args):
                frame = {**entity, "walk_position": next(positions)}
                probe.tick += 1
                Path(image).with_suffix(".json").write_text(json.dumps({"gait_frame": [frame]}))
                return image
            probe = Probe()
            with patch.object(scene_shot, "capture_native", side_effect=capture):
                self.assertEqual(scene_shot.execute(probe, root, [{"walk_capture": spec}], "native"), [image])
            evidence = json.loads((root / "walk-walk.json").read_text())
            self.assertEqual([a["frame"]["entities"][0]["walk_position"] for a in evidence["attempts"]], [0.9, 1.6])
            self.assertEqual(evidence["spec"]["tolerance"], 0.45)

    def test_every_registered_scene_runs_its_own_fixture_without_an_override(self):
        for name, entry in scenarios.SCENARIOS.items():
            if entry.profile != "scene-shot":
                continue
            fixture = [source for source in entry.sources if source.endswith("_scene.json")]
            self.assertEqual(len(fixture), 1, name)

            class Context:
                client = "java" if name.startswith("java-") else "native"
                windowed = True
                platform = None
                scenario_id = entry.scenario_id
                extra_args = []
                timeout_seconds = 600
            with self.subTest(name), patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": ""}):
                scenario = scene_shot.validate_scenario(Context())[0]
                expected = json.loads((scene_shot.runtime.REPO_ROOT / fixture[0]).read_text())
                self.assertEqual(scenario["steps"], expected["steps"])

    def test_named_java_chat_scene_preserves_fixture_and_rejects_overrides(self):
        class Context:
            client = "java"
            windowed = True
            platform = None
            scenario_id = "java-matrix-system-chat"
            extra_args = []
            timeout_seconds = 120
        ctx = Context()
        with patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": ""}):
            self.assertEqual(scene_shot.validate_scenario(ctx)[0],
                             {"steps": [], "operator": True, "server_view_distance": 2, "java_plain": True})
            with self.assertRaisesRegex(ValueError, "without overrides"):
                scene_shot.validate_scenario(ctx, {"steps": []})
            with patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": "/tmp/other.json"}):
                with self.assertRaisesRegex(ValueError, "without overrides"):
                    scene_shot.validate_scenario(ctx)
            with patch.object(ctx, "extra_args", ["/tmp/other.json"]):
                with self.assertRaisesRegex(ValueError, "without overrides"):
                    scene_shot.validate_scenario(ctx)
            with patch.object(ctx, "client", "native"):
                with self.assertRaisesRegex(ValueError, "registered java fixture"):
                    scene_shot.validate_scenario(ctx)

    def test_named_death_tilt_scene_gates_its_own_shots(self):
        class Context:
            client = "native"
            windowed = True
            platform = None
            scenario_id = "native-death-tilt"
            extra_args = []
            timeout_seconds = 300
        with patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": ""}):
            scenario = scene_shot.validate_scenario(Context())[0]
            gate = [step["death_tilt"] for step in scenario["steps"] if "death_tilt" in step]
            shots = [step["shot"] for step in scenario["steps"] if "shot" in step]
            self.assertEqual(len(gate), 1)
            self.assertEqual(gate[0]["empty"], "empty")
            self.assertEqual(gate[0]["shots"], [name for name in shots if name not in ("empty", "alive")])
            from tools.harness.backends import death_tilt_oracle
            self.assertEqual(sorted(death_tilt_oracle.JAVA), sorted(gate[0]["shots"]))
            with self.assertRaisesRegex(ValueError, "without overrides"):
                scene_shot.validate_scenario(Context(), {"steps": []})

    def test_mob_motion_gate_ignores_a_kind_the_window_does_not_drive(self):
        """ISSUE-1-19: the spider window left the zombie standing and the gate
        reported its standstill as a client defect."""
        class Probe:
            def __init__(self):
                self.checks = {}
            def check(self, name, value):
                self.checks[name] = value
        probe = Probe()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name, tick, spider in zip(("a", "b", "c"), (100, 120, 140), (0., 1., 2.)):
                (root / f"{name}.json").write_text(json.dumps({
                    "game_time": tick,
                    "snapshot": {"entities": [
                        {"entity_type": "minecraft:zombie", "entity_uuid": "z", "x": 4., "z": 4.},
                        {"entity_type": "minecraft:spider", "entity_uuid": "s", "x": spider, "z": 0.},
                    ]},
                }))
            scene_shot.execute(probe, root, [{
                "mob_motion": {"samples": ["a", "b", "c"], "name": "spider-walk",
                               "types": ["minecraft:spider"]},
            }])
        self.assertEqual(probe.checks, {"mob_moved_spider": True})

    def test_mob_motion_gate_bounds_each_segment_speed(self):
        """ISSUE-1-19: displacement alone passed a trajectory whose pace was
        outside the row's range; every segment's rate must sit inside it."""
        class Probe:
            def __init__(self):
                self.checks = {}
            def check(self, name, value):
                self.checks[name] = value
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            # 1 block per 20 client ticks: 0.05 blocks/tick in both segments.
            for name, tick, x in zip(("a", "b", "c"), (100, 120, 140), (0., 1., 2.)):
                (root / f"{name}.json").write_text(json.dumps({
                    "game_time": tick,
                    "snapshot": {"entities": [
                        {"entity_type": "minecraft:zombie", "entity_uuid": "z", "x": x, "z": 0},
                    ]},
                }))
            step = {"mob_motion": {"samples": ["a", "b", "c"], "name": "motion",
                                   "types": [{"id": "minecraft:zombie",
                                              "min_speed": 0.04, "max_speed": 0.06}]}}
            probe = Probe()
            scene_shot.execute(probe, root, [step])
            self.assertEqual(probe.checks, {"mob_moved_zombie": True, "mob_speed_zombie": True})
            probe = Probe()
            scene_shot.execute(probe, root, [{"mob_motion": {
                "samples": ["a", "b", "c"], "name": "motion",
                "types": [{"id": "minecraft:zombie", "min_speed": 0.06, "max_speed": 0.09}],
            }}])
            self.assertEqual(probe.checks, {"mob_moved_zombie": True, "mob_speed_zombie": False})

    def test_mob_motion_gate_checks_the_teleport_step_not_only_the_rate(self):
        """The hostile-motion fixture teleports a fixed number of blocks per segment; the
        client-tick rate divides by round-trip ticks, so the step is the measurement the
        fixture controls and the rate is only a coarse band."""
        class Probe:
            def __init__(self):
                self.checks = {}
                self.report = {}
            def check(self, name, value):
                self.checks[name] = value
        steps = [{"mob_motion": {"samples": ["a", "b", "c"], "name": "motion", "types": [
            {"id": "minecraft:zombie", "min_speed": 0.02, "max_speed": 0.12,
             "min_blocks": 1.5, "max_blocks": 2.5}]}}]
        for xs, step_ok in (((0., 2., 4.), True), ((0., 1., 3.), False)):
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                for name, tick, x in zip(("a", "b", "c"), (100, 125, 150), xs):
                    (root / f"{name}.json").write_text(json.dumps({
                        "game_time": tick,
                        "snapshot": {"entities": [
                            {"entity_type": "minecraft:zombie", "entity_uuid": "z", "x": x, "z": 0},
                        ]},
                    }))
                probe = Probe()
                scene_shot.execute(probe, root, steps)
                with self.subTest(xs=xs):
                    self.assertEqual(probe.checks, {"mob_moved_zombie": True,
                                                    "mob_speed_zombie": True,
                                                    "mob_step_zombie": step_ok})
        step = steps[0]["mob_motion"]
        scene_shot.validate_steps([{"mob_motion": step}], "native")
        for bad in ({"min_blocks": 1.5}, {"max_blocks": 2.5},
                    {"min_blocks": 2.5, "max_blocks": 1.5},
                    {"min_blocks": -1, "max_blocks": 2.5}):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                scene_shot.validate_steps([{"mob_motion": {
                    **step, "types": [{**step["types"][0], **bad}]}}], "native")

    def test_mob_motion_gate_requires_the_kinds_it_measures(self):
        step = {"mob_motion": {"samples": ["a", "b", "c"], "name": "motion"}}
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([step], "native")
        for bad in ([], ["zombie"], [1], "minecraft:zombie"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                scene_shot.validate_steps([{"mob_motion": {**step["mob_motion"], "types": bad}}], "native")
        scene_shot.validate_steps(
            [{"mob_motion": {**step["mob_motion"], "types": ["minecraft:zombie"]}}], "native")

    def test_java_options_pin_window_fov_gui_and_distance(self):
        options = dict(line.split(":", 1) for line in scene_shot.java_options(32, 70).splitlines())
        self.assertEqual((options["overrideWidth"], options["overrideHeight"]), ("1280", "720"))
        self.assertEqual((options["fov"], options["guiScale"], options["renderDistance"]), ("0.0", "3", "32"))
        self.assertEqual(options["graphicsPreset"], '"custom"')
        self.assertEqual(scene_shot.java_options(8, 110).count("fov:1.0"), 1)

    def test_java_display_settings_are_explicit_and_fail_closed(self):
        for scale in (2, 3):
            for language in ("en_us", "ru_ru"):
                options = dict(line.split(":", 1) for line in scene_shot.java_options(
                    8, 70, gui_scale=scale, language=language).splitlines())
                self.assertEqual(options["guiScale"], str(scale))
                self.assertEqual(options["lang"], language)
                ctx = scene_shot.ProfileContext(Path("/tmp/scene-display"), client="java", windowed=True)
                settings = {"gui_scale": scale, "language": language}
                validated = scene_shot.validate_scenario(ctx, {"steps": [], "client_settings": settings})
                self.assertEqual(validated[7], settings)
        for settings in ({"gui_scale": True}, {"gui_scale": 0}, {"gui_scale": 4},
                         {"gui_scale": 2.0}, {"language": "ru_ru\noperator:true"},
                         {"language": None}, {"language": "unknown"}):
            with self.subTest(settings=settings), self.assertRaises(ValueError):
                ctx = scene_shot.ProfileContext(Path("/tmp/scene-display"), client="java", windowed=True)
                scene_shot.validate_scenario(ctx, {"steps": [], "client_settings": settings})
        with self.assertRaisesRegex(ValueError, "Java-only"):
            ctx = scene_shot.ProfileContext(Path("/tmp/scene-display"), client="native", windowed=True)
            scene_shot.validate_scenario(ctx, {"steps": [], "client_settings": {"gui_scale": 2}})

    def test_java_screen_size_reaches_options_and_capture_expectation(self):
        options = dict(line.split(":", 1) for line in scene_shot.java_options(
            8, 70, gui_scale=3, language="ru_ru", screen_size=(1920, 1080)).splitlines())
        self.assertEqual((options["overrideWidth"], options["overrideHeight"]), ("1920", "1080"))
        ctx = scene_shot.ProfileContext(Path("/tmp/scene-display"), client="java", windowed=True)
        settings = {"screen_size": [1920, 1080]}
        self.assertEqual(scene_shot.validate_scenario(
            ctx, {"steps": [], "client_settings": settings})[7], settings)
        class Client:
            scene_window_size = (1920, 1080)
        class Probe:
            client = Client()
        with patch.object(scene_shot.java_p5, "capture", return_value="/capture.png"), \
                patch.object(scene_shot, "png_size", return_value=(1920, 1080)):
            self.assertEqual(scene_shot.capture_java(Probe(), Path("/tmp"), "fit"), "/capture.png")
        with self.assertRaisesRegex(ValueError, "clamping"):
            scene_shot.java_options(8, 70, gui_scale=3, screen_size=(640, 480))

    def test_private_xvfb_size_is_validated_and_preserved_by_fallback(self):
        runtime = scene_shot.runtime
        for size, expected in (((1280, 720), "1280x720x24"), ((1920, 1080), "1920x1080x24")):
            with self.subTest(size=size), tempfile.TemporaryDirectory() as tmp, \
                    patch.object(runtime.subprocess, "Popen", side_effect=FileNotFoundError) as start, \
                    patch.object(runtime, "_start_xvfb_scan", return_value=(None, ":99", None)) as fallback:
                runtime.start_xvfb(Path(tmp), size)
                self.assertIn(expected, start.call_args.args[0])
                self.assertEqual(fallback.call_args.args[2], expected)
                fallback.call_args.args[1].close()
        for size in (None, [1280], [True, 720], [1280, 720.0], [0, 720], [3841, 720], [1280, 2161]):
            with self.subTest(size=size), tempfile.TemporaryDirectory() as tmp, \
                    patch.object(runtime.subprocess, "Popen") as start:
                with self.assertRaises(ValueError):
                    runtime.start_xvfb(Path(tmp) / "must-not-exist", size)
                start.assert_not_called()
                self.assertFalse((Path(tmp) / "must-not-exist").exists())

    def test_java_effective_render_distance_is_checked(self):
        with tempfile.TemporaryDirectory() as tmp:
            options = Path(tmp) / "options.txt"
            options.write_text("fov:0.0\nrenderDistance:32\n")
            scene_shot.check_java_render_distance(options, 32)
            options.write_text("renderDistance:16\n")
            with self.assertRaisesRegex(RuntimeError, "renderDistance.*16.*32"):
                scene_shot.check_java_render_distance(options, 32)
            options.write_text("fov:0.0\n")
            with self.assertRaisesRegex(RuntimeError, "renderDistance"):
                scene_shot.check_java_render_distance(options, 32)


class PlayerHeadTests(unittest.TestCase):
    class Probe:
        def __init__(self):
            self.report = {}
        def check(self, name, value):
            if not value:
                raise AssertionError(name)

    def store_frames(self, root, *, hat=True, placed=True):
        from PIL import Image, ImageDraw

        shots = root / "screenshots"
        shots.mkdir(exist_ok=True)
        frame = {"gpu_completion": True, "camera_position": [1000.5, 161.62, 1001.5],
                 "yaw": 0, "pitch": 0}
        for name in ("head-empty", "head-block", "head-worn", "head-none"):
            image = Image.new("RGB", scene_shot.WINDOW, (70, 70, 70))
            draw = ImageDraw.Draw(image)
            if name == "head-block" and placed:
                draw.rectangle((594, 537, 686, 640), fill=(0, 200, 200))
                if hat:
                    draw.rectangle((594, 537, 686, 560), fill=(200, 0, 200))
            if name == "head-worn":
                draw.rectangle((610, 306, 674, 370), fill=(200, 0, 200))
            image.save(shots / f"{name}.png")
            (shots / f"{name}.json").write_text(json.dumps(frame))

    def test_placed_head_roi_covers_fixture_and_requires_hat_pixels(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.store_frames(root)
            probe = self.Probe()
            scene_shot.check_player_head(probe, root)
            self.assertGreater(probe.report["placed_head_changed_pixels"], 9000)
            self.assertEqual(probe.report["placed_head_hat_layer_pixels"], 93 * 24)
            self.store_frames(root, hat=False)
            with self.assertRaisesRegex(AssertionError, "placed_head_hat_layer_pixels"):
                scene_shot.check_player_head(self.Probe(), root)
            self.store_frames(root, placed=False)
            with self.assertRaisesRegex(AssertionError, "placed_head_renders"):
                scene_shot.check_player_head(self.Probe(), root)

    def test_worn_head_rejects_extra_back_patch_even_with_correct_head(self):
        from PIL import Image, ImageDraw

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.store_frames(root)
            path = root / "screenshots/head-worn.png"
            with Image.open(path) as frame:
                image = frame.convert("RGB")
            # HeadBreaker's exact counterexample, leaving the valid head intact.
            ImageDraw.Draw(image).rectangle((605, 430, 675, 500), fill=(240, 0, 240))
            image.save(path)
            probe = self.Probe()
            with self.assertRaisesRegex(AssertionError, "worn_head_back_unchanged"):
                scene_shot.check_player_head(probe, root)
            self.assertEqual(probe.report["worn_head_back_changed_pixels"], 71 * 71)
            self.assertGreater(probe.report["worn_head_hat_layer_pixels"], 100)

    def test_worn_head_allows_small_pose_raster_changes_on_back(self):
        from PIL import Image, ImageDraw

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.store_frames(root)
            path = root / "screenshots/head-worn.png"
            with Image.open(path) as frame:
                image = frame.convert("RGB")
            ImageDraw.Draw(image).rectangle((605, 430, 624, 449), fill=(110, 100, 90))
            image.save(path)
            probe = self.Probe()
            scene_shot.check_player_head(probe, root)
            self.assertEqual(probe.report["worn_head_back_changed_pixels"], 400)

    def test_worn_head_ignores_idle_torso_motion_without_extra_hat_pixels(self):
        from PIL import Image, ImageDraw

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.store_frames(root)
            path = root / "screenshots/head-worn.png"
            with Image.open(path) as frame:
                image = frame.convert("RGB")
            # Pose changes are not attachments: even a large clothing change
            # must not be confused with the fixture's contrasting skull hat.
            ImageDraw.Draw(image).rectangle((600, 400, 679, 509), fill=(150, 90, 40))
            image.save(path)
            probe = self.Probe()
            scene_shot.check_player_head(probe, root)
            self.assertEqual(probe.report["worn_head_back_changed_pixels"], 8800)
            self.assertEqual(probe.report["worn_head_added_back_hat_pixels"], 0)

    def test_worn_head_rejects_back_attachment_despite_unrelated_head_roi_changes(self):
        from PIL import Image, ImageDraw

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.store_frames(root)
            path = root / "screenshots/head-worn.png"
            with Image.open(path) as frame:
                image = frame.convert("RGB")
            draw = ImageDraw.Draw(image)
            # Avatar pose/sky changes used to satisfy the changed-pixel check
            # even when the magenta equipped head was down on the torso.
            draw.rectangle((600, 300, 680, 380), fill=(110, 100, 90))
            draw.rectangle((600, 420, 680, 506), fill=(200, 0, 200))
            image.save(path)
            with self.assertRaisesRegex(AssertionError, "worn_head_renders"):
                scene_shot.check_player_head(self.Probe(), root)

    def test_worn_head_rejects_adjacent_skull_clipping_the_old_broad_roi(self):
        from PIL import Image, ImageDraw

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.store_frames(root)
            path = root / "screenshots/head-worn.png"
            with Image.open(path) as frame:
                image = frame.convert("RGB")
            draw = ImageDraw.Draw(image)
            draw.rectangle((600, 300, 680, 380), fill=(110, 100, 90))
            # Real rejected candidate: skull left of the bare head. Its edge
            # supplied 743 magenta pixels to the old x580-700/y280-400 box.
            draw.rectangle((486, 369, 597, 484), fill=(200, 0, 200))
            image.save(path)
            with self.assertRaisesRegex(AssertionError, "worn_head_renders"):
                scene_shot.check_player_head(self.Probe(), root)

    def test_authored_skin_has_opaque_contrasting_hat_and_base_window(self):
        from PIL import Image

        with tempfile.TemporaryDirectory() as directory:
            pack = scene_shot.player_head_skin_pack(Path(directory))
            self.assertEqual(json.loads((pack / "pack.mcmeta").read_text())["pack"]["pack_format"], 84)
            for model in ("slim", "wide"):
                texture = pack / f"assets/minecraft/textures/entity/player/{model}/steve.png"
                self.assertTrue(texture.is_file(), f"{model} default skin has no authored hat fixture")
                with Image.open(texture) as skin:
                    self.assertEqual(skin.size, (64, 64))
                    self.assertEqual(skin.getpixel((8, 8)), (0, 220, 220, 255))
                    self.assertEqual(skin.getpixel((40, 8)), (240, 0, 240, 255))
                    self.assertEqual(skin.getpixel((43, 11))[3], 0)
            scene = json.loads((scene_shot.runtime.REPO_ROOT / "tools/harness/player_head_scene.json").read_text())
            heads = [step["cmd"] for step in scene["steps"] if "profile" in step.get("cmd", "")]
            self.assertEqual(len(heads), 4)
            self.assertTrue(all("HarnessHat" in command for command in heads))


class PlayerHeadRegistryTests(unittest.TestCase):
    """PLAYER-HEAD-FAILED-PROFILE-REGISTRY-EXHAUSTION: a full registry must not deny new demand."""

    class Probe:
        def __init__(self):
            self.report = {}

        def check(self, name, value):
            if not value:
                raise AssertionError(name)

    def store_frames(self, root, *, valid_differs=True):
        from PIL import Image, ImageDraw

        shots = root / "screenshots"
        shots.mkdir(exist_ok=True)
        frame = {"gpu_completion": True, "camera_position": [1000.5, 161.62, 1001.5],
                 "yaw": 0, "pitch": 0}
        for name in ("empty", "flood-steve", "flood-valid"):
            image = Image.new("RGB", scene_shot.WINDOW, (70, 70, 70))
            if name != "empty":
                draw = ImageDraw.Draw(image)
                draw.rectangle((610, 290, 680, 370), fill=(160, 120, 90))
                if name == "flood-valid" and valid_differs:
                    draw.rectangle((610, 290, 680, 370), fill=(210, 180, 170))
            image.save(shots / f"{name}.png")
            (shots / f"{name}.json").write_text(json.dumps(frame))

    def test_valid_head_after_flood_requires_a_published_skin(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.store_frames(root)
            probe = self.Probe()
            scene_shot.check_player_head_registry(probe, root)
            self.assertGreater(probe.report["flood_profile_changed_pixels"], 2000)
            self.store_frames(root, valid_differs=False)
            probe = self.Probe()
            with self.assertRaisesRegex(AssertionError, "valid_head_after_flood"):
                scene_shot.check_player_head_registry(probe, root)
            self.assertEqual(probe.report["flood_profile_changed_pixels"], 0)

    def test_scene_is_the_generated_261_property_flood(self):
        from tools.harness import gen_player_head_registry_scene as gen

        scene = json.loads(Path(__file__).with_name("player_head_registry_scene.json").read_text())
        self.assertEqual(scene["steps"], gen.steps())
        self.assertEqual(scene["seed"], 100)
        malformed = [step["cmd"] for step in scene["steps"]
                     if "cmd" in step and 'value:"bad0' in step["cmd"]]
        self.assertEqual(len(malformed), 261)
        self.assertEqual(len(set(malformed)), 261)
        valid = [step["cmd"] for step in scene["steps"]
                 if "cmd" in step and step["cmd"].startswith("give") and "eyJ0ZXh0" in step["cmd"]]
        self.assertEqual(len(valid), 1)
        import base64
        value = valid[0].rsplit('value:"', 1)[1].split('"')[0]
        decoded = json.loads(base64.b64decode(value))
        self.assertTrue(decoded["textures"]["SKIN"]["url"].startswith(
            "http://textures.minecraft.net/texture/"))
        order = [index for index, step in enumerate(scene["steps"])
                 if "shot" in step and step["shot"] in ("flood-steve", "flood-valid")]
        self.assertEqual([scene["steps"][index]["shot"] for index in order],
                         ["flood-steve", "flood-valid"])


class ElytraTests(unittest.TestCase):
    class Probe:
        def __init__(self):
            self.report = {}
        def check(self, name, value):
            if not value:
                raise AssertionError(name)

    def store_frames(self, root, *, wings=True):
        from PIL import Image, ImageDraw

        shots = root / "screenshots"
        shots.mkdir(exist_ok=True)
        frame = {"gpu_completion": True, "camera_position": [1000.5, 161.62, 1005.5], "yaw": 0, "pitch": 0}
        for name in ("back-plain", "back-elytra", "back-none", "back-chestplate"):
            image = Image.new("RGB", scene_shot.WINDOW, (70, 70, 70))
            draw = ImageDraw.Draw(image)
            if name == "back-elytra" and wings:
                draw.rectangle((600, 330, 680, 430), fill=(200, 200, 210))
            if name == "back-chestplate":
                draw.rectangle((610, 340, 670, 420), fill=(90, 90, 100))
            image.save(shots / f"{name}.png")
            (shots / f"{name}.json").write_text(json.dumps(frame))

    def test_worn_elytra_layer_is_required_on_the_back_roi(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.store_frames(root)
            probe = self.Probe()
            scene_shot.check_elytra(probe, root)
            self.assertGreater(probe.report["elytra_changed_pixels"]["elytra"], 4000)
            self.assertEqual(probe.report["elytra_changed_pixels"]["idle"], 0)
            self.store_frames(root, wings=False)
            with self.assertRaisesRegex(AssertionError, "worn_elytra_drawn"):
                scene_shot.check_elytra(self.Probe(), root)

    def test_placed_head_scenario_needs_a_visible_default_skin(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            PlayerHeadTests().store_frames(root)
            scene_shot.check_placed_head(PlayerHeadTests.Probe(), root)
            PlayerHeadTests().store_frames(root, placed=False)
            with self.assertRaisesRegex(AssertionError, "placed_head_renders"):
                scene_shot.check_placed_head(PlayerHeadTests.Probe(), root)


class ToastTests(unittest.TestCase):
    class Probe:
        def __init__(self):
            self.report = {}
        def check(self, name, value):
            if not value:
                raise AssertionError(name)

    def store_frames(self, root, *, toast=True):
        from PIL import Image, ImageDraw

        shots = root / "screenshots"
        shots.mkdir(exist_ok=True)
        frame = {"gpu_completion": True, "camera_position": [1000.5, 161.62, 1000.5], "yaw": 0, "pitch": 0}
        for name in ("toast-early", "toast-late"):
            image = Image.new("RGB", scene_shot.WINDOW, (120, 170, 240))
            draw = ImageDraw.Draw(image)
            if name == "toast-early" and toast:
                # The panel rectangle at GUI scale 3 on 1280x720: x 621..1280, y 0..168,
                # with the yellow title ink and the white message lines inside it.
                draw.rectangle((621, 0, 1279, 167), fill=(20, 30, 50))
                draw.rectangle((640, 8, 1180, 26), fill=(255, 255, 0))
                draw.rectangle((640, 34, 1180, 52), fill=(240, 240, 240))
            image.save(shots / f"{name}.png")
            (shots / f"{name}.json").write_text(json.dumps(frame))

    def test_join_toast_panel_is_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.store_frames(root)
            probe = self.Probe()
            scene_shot.check_toasts(probe, root)
            self.assertGreater(probe.report["toast_pixels"]["early_panel"], 1500)
            self.assertLess(probe.report["toast_pixels"]["late_panel"], 50)
            self.assertGreater(probe.report["toast_pixels"]["early_title"], 800)
            self.assertLess(probe.report["toast_pixels"]["late_title"], 50)
            self.store_frames(root, toast=False)
            with self.assertRaisesRegex(AssertionError, "toast_panel_appeared"):
                scene_shot.check_toasts(self.Probe(), root)


class WalkCaptureArtifactsTests(unittest.TestCase):
    def test_each_retry_owns_an_artifact_and_misses_are_bounded(self):
        spec = {"name": "walk", "entity_type": "minecraft:zombie", "phase": 1.5,
                "tolerance": 0.25, "min_speed": 0.1, "max_ticks": 3}
        entity = {"entity_type": "minecraft:zombie", "entity_uuid": "mob",
                  "walk_position": 1.4, "walk_speed": 0.2}

        class Probe:
            def call(self, name, args):
                return {"entities": [entity]}
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)

        for client in ("native", "java"):
            for positions, succeeds in (((1.0, 1.1, 1.5), True), ((1.0, 1.1, 1.2), False)):
                with self.subTest(client=client, succeeds=succeeds), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    samples = iter(positions)
                    names = []
                    def capture(probe, artifact, name):
                        names.append(name)
                        image = artifact / f"{name}.png"
                        # The real MCP artifact store uses exclusive creation.
                        try:
                            with image.open("xb") as file:
                                file.write(b"image")
                        except FileExistsError as error:
                            self.fail(f"capture artifact reused: {image.name}: {error}")
                        frame = [{**entity, "walk_position": next(samples)}]
                        sidecar = Path(str(image) + ".gait.json") if client == "java" else image.with_suffix(".json")
                        sidecar.write_text(json.dumps(frame if client == "java" else {"gait_frame": frame}))
                        return str(image)
                    with patch.object(scene_shot, f"capture_{client}", side_effect=capture):
                        if succeeds:
                            result = scene_shot.execute(Probe(), root, [{"walk_capture": spec}], client)
                            self.assertEqual(result, [str(root / f"{names[-1]}.png")])
                        else:
                            with self.assertRaisesRegex(RuntimeError, "walk phase not reached"):
                                scene_shot.execute(Probe(), root, [{"walk_capture": spec}], client)
                    self.assertEqual(len(names), spec["max_ticks"])
                    self.assertEqual(len(set(names)), len(names))
                    evidence = json.loads((root / "walk-walk.json").read_text())
                    self.assertEqual(len(evidence["attempts"]), 3)
                    self.assertEqual([Path(a["image"]).stem for a in evidence["attempts"]], names)


class DroppedClockTests(unittest.TestCase):
    class Probe:
        def __init__(self):
            self.report = {}
        def check(self, name, value):
            if not value:
                raise AssertionError(name)

    def frames(self, clock):
        from PIL import Image, ImageDraw

        images = {phase: Image.new("RGB", scene_shot.WINDOW, (130, 130, 130))
                  for phase in ("ctx-day", "ctx-night", "compass-day", "held-day", "held-night")}
        for phase, image in images.items():
            draw = ImageDraw.Draw(image)
            if phase == "ctx-day":
                for box in ((385, 480, 410, 505), (948, 480, 982, 515)):
                    draw.rectangle(box, fill=(10, 10, 200))
                draw.rectangle((338, 645, 390, 650), fill="black")
            elif phase == "ctx-night":
                draw.rectangle((355, 642, 360, 695), fill="black")
            else:
                draw.rectangle((430, 490, 445, 493), fill=(200, 10, 10))
                draw.rectangle((1007, 482, 1010, 499), fill=(200, 10, 10))
            if clock and phase in ("ctx-day", "ctx-night", "held-day", "held-night"):
                x, y = (355, 300) if phase.startswith("ctx-") else (600, 330)
                draw.ellipse((x, y, x + 60, y + 55), fill=(180, 150, 20))
                draw.ellipse((x + 5, y + 5, x + 55, y + 50), fill=(
                    (10, 10, 200) if phase.endswith("day") or clock == "static" else (45, 45, 45)))
        return {phase: ({"camera_position": [1000.5, 161.62, 1001.5], "yaw": 0, "pitch": 0,
                         "environment": {"time_of_day": "18000" if phase.endswith("night") else "6000"}}, image)
                for phase, image in images.items()}

    @staticmethod
    def store_capture(artifact, name, frame):
        metadata, image = frame
        target = artifact / "screenshots" / f"{name}.png"
        target.parent.mkdir(exist_ok=True)
        image.save(target)
        target.with_suffix(".json").write_text(json.dumps({**metadata, "gpu_completion": True}))
        return str(target)

    def test_clock_scene_samples_past_edge_on_without_changing_oracle(self):
        scene = json.loads((scene_shot.runtime.REPO_ROOT / "tools/harness/dynamic_context_scene.json").read_text())
        visible, edge = self.frames("changed"), self.frames(None)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            probe = self.Probe()
            captures = []
            def capture(probe, artifact, name):
                captures.append(name)
                phase = name.split("-attempt-")[0]
                frames = visible if name.endswith("-attempt-2") else edge
                return self.store_capture(artifact, name, frames[phase])
            with patch.object(scene_shot, "capture_native", side_effect=capture), \
                    patch.object(probe, "call", create=True) as advance:
                for phase in ("ctx-day", "ctx-night"):
                    step = next(step for step in scene["steps"]
                                if step.get("dropped_clock_capture", step.get("shot")) == phase)
                    scene_shot.execute(probe, root, [step], "native")
                for phase in ("compass-day", "held-day", "held-night"):
                    self.store_capture(root, phase, visible[phase])
                scene_shot.check_dynamic_context(probe, root)
            self.assertEqual(len(captures), 4)
            self.assertEqual([call.args for call in advance.call_args_list],
                             [("minecraft_wait_ticks", {"ticks": 4})] * 2)
            for phase in ("ctx-day", "ctx-night"):
                evidence = json.loads((root / f"{phase}-clock-capture.json").read_text())
                self.assertEqual([a["sampled_cells"] for a in evidence["attempts"]], [0, 64])
                for suffix in (".png", ".json"):
                    self.assertEqual((root / f"screenshots/{phase}{suffix}").read_bytes(),
                                     (root / f"screenshots/{phase}-attempt-2{suffix}").read_bytes())

    def test_clock_capture_is_bounded_and_does_not_accept_hud_only(self):
        frame = self.frames(None)["ctx-day"]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            probe = self.Probe()
            with patch.object(scene_shot, "capture_native",
                              side_effect=lambda probe, artifact, name: self.store_capture(artifact, name, frame)), \
                    patch.object(probe, "call", create=True) as advance:
                with self.assertRaisesRegex(AssertionError, "ctx-day_clock_face_captured"):
                    scene_shot.execute(probe, root, [{"dropped_clock_capture": "ctx-day"}], "native")
            evidence = json.loads((root / "ctx-day-clock-capture.json").read_text())
            self.assertEqual(len(evidence["attempts"]), 32)
            self.assertIsNone(evidence["selected"])
            self.assertEqual(advance.call_count, 31)
            self.assertFalse((root / "screenshots/ctx-day.png").exists())

    def test_clock_capture_selects_static_face_but_animation_oracle_rejects_it(self):
        frames = self.frames("static")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            probe = self.Probe()
            def capture(probe, artifact, name):
                return self.store_capture(artifact, name, frames[name.split("-attempt-")[0]])
            with patch.object(scene_shot, "capture_native", side_effect=capture), \
                    patch.object(probe, "call", create=True) as advance:
                scene_shot.execute(probe, root, [{"dropped_clock_capture": phase}
                                                for phase in ("ctx-day", "ctx-night")], "native")
                advance.assert_not_called()
            for phase in ("compass-day", "held-day", "held-night"):
                self.store_capture(root, phase, frames[phase])
            with self.assertRaisesRegex(AssertionError, "dropped_clock_face_changed"):
                scene_shot.check_dynamic_context(probe, root)

    def test_static_clock_bob_and_clipped_bezel_do_not_count_as_animation(self):
        from PIL import Image, ImageDraw

        frames = []
        for offset in (0, 50):
            image = Image.new("RGB", scene_shot.WINDOW, (130, 130, 130))
            draw = ImageDraw.Draw(image)
            draw.rectangle((340, 280 + offset, 420, 410 + offset), fill=(180, 150, 20))
            draw.rectangle((350, 290 + offset, 410, 330 + offset), fill=(10, 10, 200))
            draw.rectangle((350, 331 + offset, 410, 340 + offset), fill=(45, 45, 45))
            frames.append(image)
        probe = self.Probe()
        with self.assertRaisesRegex(AssertionError, "dropped_clock_face_changed"):
            scene_shot.check_clock_face(probe, *frames, "dropped_clock", (300, 260, 460, 420))
        self.assertGreaterEqual(probe.report["dropped_clock_face_changed_cells"], 4)
        self.assertAlmostEqual(*probe.report["dropped_clock_face_blue_fractions"])

    def test_only_clock_face_change_passes_not_hud_or_static_clock(self):
        with patch.object(scene_shot, "_load_shots", return_value=self.frames(None)):
            with self.assertRaisesRegex(AssertionError, "dropped_clock_face_day"):
                scene_shot.check_dynamic_context(self.Probe(), Path("/unused"))
        with patch.object(scene_shot, "_load_shots", return_value=self.frames("static")):
            with self.assertRaisesRegex(AssertionError, "dropped_clock_face_changed"):
                scene_shot.check_dynamic_context(self.Probe(), Path("/unused"))
        with patch.object(scene_shot, "_load_shots", return_value=self.frames("changed")):
            probe = self.Probe()
            scene_shot.check_dynamic_context(probe, Path("/unused"))
            self.assertGreaterEqual(probe.report["dropped_clock_face_changed_cells"], 4)
            self.assertGreaterEqual(probe.report["held_clock_face_changed_cells"], 4)

    @unittest.skipUnless(os.environ.get("KRASNOKAMEN_CLOCK_TEST_FRAMES"), "requires retained original scene frames")
    def test_stored_original_frames_reject_hud_contamination(self):
        from PIL import Image

        root = Path(os.environ["KRASNOKAMEN_CLOCK_TEST_FRAMES"])
        with Image.open(root / "ctx-day.png") as day, Image.open(root / "ctx-night.png") as night:
            with self.assertRaisesRegex(AssertionError, "dropped_clock_face_day"):
                scene_shot.check_clock_face(self.Probe(), day.convert("RGB"), night.convert("RGB"),
                                            "dropped_clock", (300, 260, 460, 420))

    def test_fixture_lifts_and_lights_dropped_clock_away_from_hud(self):
        scene = json.loads((scene_shot.runtime.REPO_ROOT / "tools/harness/dynamic_context_scene.json").read_text())
        commands = [step["cmd"] for step in scene["steps"] if "cmd" in step]
        self.assertTrue(any(command.startswith("summon minecraft:item 1001.5 161.4 1003.5 ")
                            and "minecraft:clock" in command for command in commands))
        self.assertIn("setblock 1001 162 1003 minecraft:sea_lantern", commands)
        self.assertIn('setblock 997 160 1005 minecraft:oak_shelf[facing=north]{Items:[{Slot:0b,id:"minecraft:clock",count:1},{Slot:1b,id:"minecraft:compass",count:1}]}', commands)
        self.assertTrue(any(step.get("tp") == [1000.5, 162, 1001.5, 10, 37] for step in scene["steps"]))
        self.assertTrue(all({"shot": name} in scene["steps"] for name in ("held-day", "held-night")))


