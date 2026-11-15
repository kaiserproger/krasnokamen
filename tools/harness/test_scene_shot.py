import json
import math
import os
import socket
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness import scenarios
from tools.harness.backends import scene_shot


class DebugOverlayAndPixelSteps(unittest.TestCase):
    """F3 through the dedicated tool, the generic pixel window, and the drawn-index bound."""

    def test_toggle_debug_overlay_is_a_validated_step_and_calls_the_client_tool(self):
        steps = [{"toggle_debug_overlay": True}, {"toggle_debug_overlay": False}]
        scene_shot.validate_steps(steps, "native")
        self.assertEqual([call for call in scene_shot.scene_mcp_calls(steps)],
                         [("minecraft_toggle_debug_overlay", {"enabled": True}),
                          ("minecraft_toggle_debug_overlay", {"enabled": False})])
        for bad in ({"toggle_debug_overlay": "on"}, {"toggle_debug_overlay": 1}, {"toggle_debug_overlay": None}):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                scene_shot.validate_steps([bad], "native")

    def test_pixel_window_counts_only_the_color_box_inside_the_window(self):
        from PIL import Image

        frame = Image.new("RGB", scene_shot.PIXEL_FRAME, (10, 20, 30))
        for y in range(100, 110):
            for x in range(50, 60):
                frame.putpixel((x, y), (200, 40, 40))
        count = scene_shot.pixel_window_count(frame, [40, 90, 70, 120], {"min": [180, 30, 30], "max": [255, 60, 60]})
        self.assertEqual(count, 100)
        # The same window outside the drawn rectangle sees none of those pixels.
        self.assertEqual(scene_shot.pixel_window_count(frame, [0, 0, 30, 30], {"min": [180, 30, 30], "max": [255, 60, 60]}), 0)

    def test_pixel_window_refuses_a_frame_that_is_not_the_matched_size(self):
        from PIL import Image

        with self.assertRaisesRegex(ValueError, "pixel_window requires"):
            scene_shot.pixel_window_count(Image.new("RGB", (640, 360)), [0, 0, 10, 10],
                                          {"min": [0, 0, 0], "max": [255, 255, 255]})

    def test_pixel_window_step_validation(self):
        good = {"pixel_window": {"name": "leg", "shot": "front", "window": [10, 20, 30, 40],
                                 "color": {"min": [1, 2, 3], "max": [4, 5, 6]}, "min_pixels": 1}}
        scene_shot.validate_steps([good], "native")
        broken = [
            {"pixel_window": {**good["pixel_window"], "name": "bad name"}},
            {"pixel_window": {**good["pixel_window"], "window": [30, 20, 10, 40]}},
            {"pixel_window": {**good["pixel_window"], "window": [10, 20, 3000, 40]}},
            {"pixel_window": {**good["pixel_window"], "color": {"min": [9, 2, 3], "max": [4, 5, 6]}}},
            {"pixel_window": {**good["pixel_window"], "min_pixels": 0}},
            {"pixel_window": {**good["pixel_window"], "shot": ""}},
            {"pixel_window": {k: v for k, v in good["pixel_window"].items() if k != "min_pixels"}},
        ]
        for bad in broken:
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                scene_shot.validate_steps([bad], "native")

    def test_render_metrics_and_the_drawn_index_bound(self):
        metrics = scene_shot.render_metrics({"render": {"draw_calls": 12, "index_count": 4000,
                                                        "entity_submitted_indices": 900}})
        self.assertEqual(metrics["index_count"], 4000)
        with self.assertRaises(RuntimeError):
            scene_shot.render_metrics({"render": {"draw_calls": 1}})
        with self.assertRaises(RuntimeError):
            scene_shot.render_metrics({})
        first = {"index_count": 4000}
        # Re-submitted per frame: bounded.
        self.assertTrue(scene_shot.bounded_draw({"index_count": 4200}, [first], max_index_count=10000, max_growth=1.5))
        # Accumulating with camera movement: grows past both the absolute and the relative bound.
        self.assertFalse(scene_shot.bounded_draw({"index_count": 7000}, [first], max_index_count=10000, max_growth=1.5))
        self.assertFalse(scene_shot.bounded_draw({"index_count": 20000}, [first], max_index_count=10000, max_growth=10))


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

    def test_server_identity_is_recorded_at_launch(self):
        import hashlib
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = root / 'artifact'
            artifact.mkdir()
            binary = root / 'server'
            binary.write_bytes(b'scene-server')
            binary.chmod(0o755)
            (root / 'playable.toml').write_text(
                (scene_shot.runtime.REPO_ROOT / 'playable.toml').read_text())
            ctx = scene_shot.ProfileContext(artifact, client='native', windowed=True)
            (root / 'template').mkdir()
            env = {'KRASNOKAMEN_SERVER_BINARY': str(binary), 'XDG_DATA_HOME': str(root / 'xdg')}
            from tools.harness import preflight
            with patch.dict('os.environ', env), patch.object(scene_shot.runtime, 'REPO_ROOT', root), \
                    patch.object(scene_shot, 'prepare_operator_fixture'), \
                    patch.object(preflight, 'client_template', return_value=root / 'template'), \
                    patch.object(scene_shot.subprocess, 'Popen', side_effect=RuntimeError('launch stopped')):
                with self.assertRaisesRegex(RuntimeError, 'launch stopped'):
                    scene_shot.run(ctx, scenario={'steps': []})
            self.assertEqual(ctx.commands[0][0], str(binary))
            self.assertEqual(ctx.client_evidence['server_binary_path'], str(binary))
            self.assertEqual(ctx.client_evidence['server_binary_sha256'],
                             hashlib.sha256(b'scene-server').hexdigest())

    def test_lod_option_only_changes_generated_config(self):
        from tools.harness import preflight
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'playable.toml').write_text(
                (scene_shot.runtime.REPO_ROOT / 'playable.toml').read_text())
            (root / 'template').mkdir()
            binary = root / 'server'
            binary.write_bytes(b'server')
            binary.chmod(0o755)
            env = {'KRASNOKAMEN_SERVER_BINARY': str(binary), 'XDG_DATA_HOME': str(root / 'xdg')}
            configs = []
            for enabled, share in ((None, None), (False, None), (True, None), (True, False), (True, True)):
                artifact = root / f'{enabled}-{share}'
                artifact.mkdir()
                ctx = scene_shot.ProfileContext(artifact, client='native', windowed=True)
                scenario = {'steps': []}
                if enabled is not None:
                    scenario['lod_enabled'] = enabled
                if share is not None:
                    scenario['lod_share_terrain_keys'] = share
                with patch.dict('os.environ', env), patch.object(scene_shot.runtime, 'REPO_ROOT', root), \
                        patch.object(scene_shot, 'prepare_operator_fixture'), \
                        patch.object(preflight, 'client_template', return_value=root / 'template'), \
                        patch.object(scene_shot.subprocess, 'Popen', side_effect=RuntimeError('launch stopped')):
                    with self.assertRaisesRegex(RuntimeError, 'launch stopped'):
                        scene_shot.run(ctx, scenario=scenario)
                configs.append((artifact / 'scene-shot.toml').read_text())
            import tomllib
            baseline, disabled, enabled, unshared, shared = map(tomllib.loads, configs)
            self.assertNotIn('lod', baseline)
            self.assertEqual(disabled.pop('lod'), {'enabled': False})
            self.assertEqual(enabled.pop('lod'), {'enabled': True})
            self.assertEqual(unshared.pop('lod'), {'enabled': True, 'share_terrain_keys': False})
            self.assertEqual(shared.pop('lod'), {'enabled': True, 'share_terrain_keys': True})
            # Private ports, worlds and plugin directories differ between runs.
            for config in (baseline, disabled, enabled, unshared, shared):
                del config['network']['port'], config['data']['world_dir'], config['plugins']['directory']
            for config in (disabled, enabled, unshared, shared):
                self.assertEqual(baseline, config)
            ctx = scene_shot.ProfileContext(root, client='native', windowed=True)
            with self.assertRaisesRegex(ValueError, 'lod_enabled'):
                scene_shot.validate_scenario(ctx, {'steps': [], 'lod_enabled': 1})
            for scenario in ({'steps': [], 'lod_share_terrain_keys': True},
                             {'steps': [], 'lod_enabled': False, 'lod_share_terrain_keys': True},
                             {'steps': [], 'lod_enabled': True, 'lod_share_terrain_keys': 1}):
                with self.assertRaisesRegex(ValueError, 'lod_share_terrain_keys'):
                    scene_shot.validate_scenario(ctx, scenario)

    def test_scenario_username(self):
        self.assertEqual(scene_shot.scene_username({}), "SceneShot")
        self.assertEqual(scene_shot.scene_username({"username": "Other_Player"}), "Other_Player")
        for name in ("", "bad name", "a" * 17, 42):
            with self.subTest(name=name), self.assertRaises(ValueError):
                scene_shot.scene_username({"username": name})

    def test_latency_proxy_delays_both_directions(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        def echo():
            with listener:
                connection, _ = listener.accept()
                with connection:
                    connection.sendall(connection.recv(32))
        thread = threading.Thread(target=echo)
        thread.start()
        proxy = scene_shot.LatencyProxy(listener.getsockname()[1], 75)
        try:
            port = proxy.start()
            with socket.create_connection(("127.0.0.1", port), timeout=5) as client:
                started = time.monotonic()
                client.sendall(b"ping")
                self.assertEqual(client.recv(4), b"ping")
                self.assertGreaterEqual(time.monotonic() - started, 0.14)
        finally:
            proxy.close()
            thread.join()

    def test_proxy_eof_is_a_clean_half_close(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        def echo():
            with listener:
                connection, _ = listener.accept()
                with connection:
                    connection.sendall(connection.recv(32))
                    connection.recv(32)
        thread = threading.Thread(target=echo)
        thread.start()
        proxy = scene_shot.LatencyProxy(listener.getsockname()[1], 0)
        try:
            port = proxy.start()
            with socket.create_connection(("127.0.0.1", port), timeout=5) as client:
                client.sendall(b"ping")
                self.assertEqual(client.recv(4), b"ping")
                proxy.send_eof()
                self.assertEqual(client.recv(1), b"")
        finally:
            proxy.close()
            thread.join()

    def test_proxy_eof_closes_every_isolated_connection(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        def echo(connection):
            with connection:
                connection.sendall(connection.recv(32))
                connection.recv(32)
        def serve():
            with listener:
                workers = [threading.Thread(target=echo, args=(listener.accept()[0],)) for _ in range(2)]
                for worker in workers:
                    worker.start()
                for worker in workers:
                    worker.join()
        thread = threading.Thread(target=serve)
        thread.start()
        proxy = scene_shot.LatencyProxy(listener.getsockname()[1], 0)
        try:
            port = proxy.start()
            with socket.create_connection(("127.0.0.1", port), timeout=5) as first, \
                    socket.create_connection(("127.0.0.1", port), timeout=5) as second:
                for client in (first, second):
                    client.sendall(b"ping")
                for client in (first, second):
                    self.assertEqual(client.recv(4), b"ping")
                proxy.send_eof()
                for client in (first, second):
                    self.assertEqual(client.recv(1), b"")
        finally:
            proxy.close()
            thread.join()

    def test_tool_wall_visibility_requires_close_target_and_visible_tool(self):
        from PIL import Image, ImageDraw

        class Probe:
            def __init__(self):
                self.report = {}
                self.distance = 0.5
            def observe(self):
                return {"target": {"block_id": "minecraft:obsidian",
                                   "location": {"x": 0, "y": 0, "z": self.distance}},
                        "selected_item": {"item_id": "minecraft:diamond_pickaxe"},
                        "player": {"camera_position": [0, 0, 0]}}
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "screenshots").mkdir()
            image = Image.new("RGB", (1280, 720), (10, 10, 20))
            ImageDraw.Draw(image).rectangle((1000, 400, 1040, 440), fill=(5, 120, 120))
            path = root / "screenshots" / "tool.png"
            image.save(path)
            probe = Probe()
            scene_shot.validate_steps([{"tool_wall_visible": "tool"}], "native")
            scene_shot.execute(probe, root, [{"tool_wall_visible": "tool"}])
            probe.distance = 1.0
            with self.assertRaisesRegex(AssertionError, "tool_wall_fixture"):
                scene_shot.execute(probe, root, [{"tool_wall_visible": "tool"}])
            probe.distance = 0.5
            Image.new("RGB", (1280, 720), (10, 10, 20)).save(path)
            with self.assertRaisesRegex(AssertionError, "tool_wall_visible"):
                scene_shot.execute(probe, root, [{"tool_wall_visible": "tool"}])
            Image.new("RGB", (1280, 720), (5, 120, 120)).save(path)
            with self.assertRaisesRegex(AssertionError, "tool_wall_close_background"):
                scene_shot.execute(probe, root, [{"tool_wall_visible": "tool"}])

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

    def test_require_entities_rejects_empty_or_invalid_type_lists(self):
        for types in ([], ["zombie"], [""], [None]):
            step = {"require_entities": {"name": "subjects", "args": {}, "types": types}}
            for client in ("native", "java"):
                with self.subTest(types=types, client=client), self.assertRaises(ValueError):
                    scene_shot.validate_steps([step], client)

    def test_walk_phase_rejects_idle_missing_and_wrong_phase(self):
        spec = {"name": "walk", "entity_type": "minecraft:zombie", "phase": 0.0,
                "tolerance": 0.25, "min_speed": 0.1, "max_ticks": 30}
        step = {"walk_capture": spec}
        scene_shot.validate_steps([step], "native")
        scene_shot.validate_steps([step], "java")
        entity = {"entity_type": "minecraft:zombie", "walk_position": 0.1, "walk_speed": 0.2}
        self.assertIs(scene_shot.matching_walk({"entities": [entity]}, spec), entity)
        for values in ({"walk_speed": 0.0}, {"walk_position": None}, {"walk_position": 1.0},
                       {"walk_position": math.tau + 0.1}):
            self.assertIsNone(scene_shot.matching_walk({"entities": [{**entity, **values}]}, spec))
        self.assertIsNone(scene_shot.matching_walk({"entities": [entity, entity]}, spec))

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

    def test_steps_dispatch_and_retain_evidence(self):
        class Probe:
            def __init__(self):
                self.client = object()
                self.calls = []
            def call(self, name, args=None):
                self.calls.append((name, args))
                return {"in_play": True}
            def observe(self):
                return {"in_play": True, "position": [1, 2, 3]}
            def wait(self, predicate, label, *, wait_for_play=False):
                assert wait_for_play
                assert predicate({"player": {"x": 1, "z": 3}})
            def remaining(self):
                return 50
        probe = Probe()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(scene_shot.native_client, "capture", return_value={"retained_path": str(root / "a.png")}):
                shots = scene_shot.execute(probe, root, [{"wait_play": True}, {"cmd": "time set 18000"}, {"tp": [1, 2, 3, 90, 0]}, {"wait_ticks": 5}, {"observe": "state"}, {"shot": "a"}])
            self.assertEqual(shots, [str(root / "a.png")])
            self.assertEqual(json.loads((root / "state.json").read_text())["position"], [1, 2, 3])
            self.assertIn(("minecraft_send_chat", {"message": "tp @s 1 2 3 90 0", "command": True}), probe.calls)
            self.assertIn(("minecraft_wait_ticks", {"ticks": 5}), probe.calls)

    def test_assert_chat_requires_fence_and_rejects_unexpected_feedback(self):
        class Probe:
            def __init__(self, lines):
                self.lines = lines
            def observe(self):
                return {"recent_chat": self.lines}
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)
        step = {"assert_chat": {"name": "disabled", "contains": ["fence"],
                                "excludes": ["Set time"]}}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            scene_shot.execute(Probe(["fence"]), root, [step])
            self.assertEqual(json.loads((root / "disabled.json").read_text())["recent_chat"], ["fence"])
            with self.assertRaisesRegex(AssertionError, "disabled_excludes_0"):
                scene_shot.execute(Probe(["fence", "Set time"]), root, [step])
            with self.assertRaisesRegex(AssertionError, "disabled_contains_0"):
                scene_shot.execute(Probe([]), root, [step])
            with self.assertRaisesRegex(RuntimeError, "omitted recent_chat"):
                scene_shot.execute(Probe(None), root, [step])
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([step], "java")
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([{"assert_chat": {"name": "x", "contains": [], "excludes": []}}], "native")

    def test_upgrade_certified_upgrades_the_one_certified_id_and_checks_the_reply(self):
        class Probe:
            def __init__(self, reply, walls=("17179869187", "17179869187")):
                self.reply = reply
                self.walls = walls
                self.sent = []
                self.upgraded = False
            def call(self, name, args=None):
                if name == "minecraft_send_chat":
                    self.sent.append(args["message"])
                    self.upgraded = True
                return {"in_play": True}
            def observe(self):
                # The client reports its last 20 lines newest first; entries may be error objects.
                lines = [f"Certified wall #{wall} (level 1)." for wall in self.walls]
                lines.insert(1, "Certified tower #17179869186 (level 1).")
                lines.append({"error": "unrenderable"})
                if self.upgraded:
                    lines.insert(0, self.reply)
                return {"recent_chat": lines}
            def wait(self, predicate, label, *, wait_for_play=False, timeout_seconds=None):
                assert predicate(self.observe())
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)
            def remaining(self):
                return 50
        step = {"upgrade_certified": {"kind": "wall", "level": 2}}
        ok = Probe("Upgraded building #17179869187 to level 2.")
        scene_shot.execute(ok, Path("."), [step])
        self.assertEqual(ok.sent, ["settlement building upgrade 17179869187"])
        stuck = Probe("Upgraded building #17179869187 to level 1.")
        with self.assertRaisesRegex(AssertionError, "wall_upgraded_to_level_2"):
            scene_shot.execute(stuck, Path("."), [step])
        ambiguous = Probe("", walls=("17179869187", "17179869188"))
        with self.assertRaisesRegex(AssertionError, "wall_one_certified_building"):
            scene_shot.execute(ambiguous, Path("."), [step])
        self.assertEqual(ambiguous.sent, [])
        for bad in ({"kind": "wall"}, {"kind": "wall", "level": 1}, {"kind": "Wall", "level": 2}):
            with self.assertRaises(ValueError):
                scene_shot.validate_steps([{"upgrade_certified": bad}], "native")
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([step], "java")

    def test_named_settlement_scenes_are_registered_and_reject_overrides(self):
        for name, command in (("native-settlement-radius-growth", "settlement bind radiusqa"),
                              ("native-settlement-buildings-upgrade", "settlement building certify tower")):
            class Context:
                client = "native"
                windowed = True
                platform = None
                scenario_id = name
                extra_args = []
                timeout_seconds = 180
            with patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": ""}):
                scenario = scene_shot.validate_scenario(Context())[0]
                self.assertTrue(any(step.get("cmd", "").startswith(command) for step in scenario["steps"]))
                self.assertTrue(any("assert_chat" in step for step in scenario["steps"]))
                with self.assertRaisesRegex(ValueError, "without overrides"):
                    scene_shot.validate_scenario(Context(), {"steps": []})
            self.assertEqual(scenarios.SCENARIOS[name].profile, "scene-shot")

    def test_every_registered_scene_runs_its_own_fixture_without_an_override(self):
        for name, entry in scenarios.SCENARIOS.items():
            if entry.profile != "scene-shot":
                continue
            fixture = [source for source in entry.sources if source.endswith("_scene.json")]
            self.assertEqual(len(fixture), 1, name)

            class Context:
                client = "native"
                windowed = True
                platform = None
                scenario_id = entry.scenario_id
                extra_args = []
                timeout_seconds = 600
            with self.subTest(name), patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": ""}):
                scenario = scene_shot.validate_scenario(Context())[0]
                expected = json.loads((scene_shot.runtime.REPO_ROOT / fixture[0]).read_text())
                self.assertEqual(scenario["steps"], expected["steps"])

    def test_named_feedback_scene_rejects_overrides(self):
        class Context:
            client = "native"
            windowed = True
            platform = None
            scenario_id = "native-command-feedback"
            extra_args = []
            timeout_seconds = 180
        with patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": ""}):
            scenario = scene_shot.validate_scenario(Context())[0]
            self.assertTrue(any("assert_chat" in step for step in scenario["steps"]))
            with self.assertRaisesRegex(ValueError, "without overrides"):
                scene_shot.validate_scenario(Context(), {"steps": []})
            with patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": "/tmp/other.json"}):
                with self.assertRaisesRegex(ValueError, "without overrides"):
                    scene_shot.validate_scenario(Context())

    def test_named_low_roof_scene_rejects_overrides(self):
        class Context:
            client = "native"
            windowed = True
            platform = None
            scenario_id = "native-low-roof-house"
            extra_args = []
            timeout_seconds = 180
        with patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": ""}):
            scenario = scene_shot.validate_scenario(Context())[0]
            self.assertTrue(any("assert_chat" in step for step in scenario["steps"]))
            with self.assertRaisesRegex(ValueError, "without overrides"):
                scene_shot.validate_scenario(Context(), {"steps": []})

    def test_named_falling_scene_rejects_overrides(self):
        class Context:
            client = "native"
            windowed = True
            platform = None
            scenario_id = "native-falling-block-prediction"
            extra_args = []
            timeout_seconds = 180
        with patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": ""}):
            scenario = scene_shot.validate_scenario(Context())[0]
            self.assertTrue(any("falling_block_capture" in step for step in scenario["steps"]))
            with self.assertRaisesRegex(ValueError, "without overrides"):
                scene_shot.validate_scenario(Context(), {"steps": []})

    def test_falling_frame_packet_fence_and_sand_pixels(self):
        from PIL import Image, ImageDraw
        from tools.harness.backends import falling_block_oracle

        first = {"observed_tick": 10, "sample_tick": 10, "received_position": [1, 2, 3], "position": [1, 2, 3]}
        second = {**first, "sample_tick": 11, "position": [1, 1.96, 3]}
        self.assertTrue(falling_block_oracle.same_packet(first, second))
        self.assertFalse(falling_block_oracle.same_packet(first, {**second, "observed_tick": 11}))
        self.assertFalse(falling_block_oracle.same_packet(first, {**second, "received_position": [1, 1, 3]}))
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "frame.png"
            image = Image.new("RGB", (1280, 720), "black")
            image.save(path)
            self.assertIsNone(falling_block_oracle.sand_bounds(path))
            ImageDraw.Draw(image).rectangle((600, 300, 630, 330), fill=(210, 190, 120))
            image.save(path)
            self.assertEqual(falling_block_oracle.sand_bounds(path), (600, 300, 630, 330))

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

    def test_named_frozen_age_scene_rejects_overrides(self):
        class Context:
            client = "native"
            windowed = True
            platform = None
            scenario_id = "native-frozen-game-age"
            extra_args = []
            timeout_seconds = 180
        with patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": ""}):
            scenario = scene_shot.validate_scenario(Context())[0]
            self.assertTrue(any("assert_same_game_time" in step for step in scenario["steps"]))
            with self.assertRaisesRegex(ValueError, "without overrides"):
                scene_shot.validate_scenario(Context(), {"steps": []})

    def test_frozen_age_assertion_rejects_advance_and_missing_clock(self):
        class Probe:
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)
        step = {"assert_same_game_time": {"before": "a", "after": "b", "day_delta": 1000}}
        scene_shot.validate_steps([step], "native")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.json").write_text(json.dumps({"time": {"game_time": 42, "clock_time": 5}}))
            (root / "b.json").write_text(json.dumps({"time": {"game_time": 42, "clock_time": 1005}}))
            scene_shot.execute(Probe(), root, [step])
            (root / "b.json").write_text(json.dumps({"time": {"game_time": 43, "clock_time": 1005}}))
            with self.assertRaisesRegex(AssertionError, "frozen_game_time"):
                scene_shot.execute(Probe(), root, [step])
            (root / "b.json").write_text(json.dumps({"time": {"game_time": 42, "clock_time": 5}}))
            with self.assertRaisesRegex(AssertionError, "received_fresh_day_time"):
                scene_shot.execute(Probe(), root, [step])
            (root / "b.json").write_text("{}")
            with self.assertRaisesRegex(RuntimeError, "omitted game_time"):
                scene_shot.execute(Probe(), root, [step])
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([step], "java")

    def test_wait_respawn_rejects_dead_or_open_screen(self):
        class Probe:
            def wait(self, predicate, label, *, wait_for_play=False):
                assert label == "respawn_alive_screen_closed"
                assert wait_for_play
                assert not predicate({"player": {"health": 0}, "screen": {"open": True}})
                assert not predicate({"player": {"health": 20}, "screen": {"open": True}})
                assert not predicate({"player": None, "screen": {"open": False}})
                assert predicate({"player": {"health": 20}, "screen": {"open": False}})
        scene_shot.execute(Probe(), Path("/tmp"), [{"wait_respawn": True}])
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([{"wait_respawn": False}], "native")

    def test_pause_dispatches_native_menu_and_rejects_other_inputs(self):
        class Probe:
            def __init__(self):
                self.calls = []
            def call(self, name, args):
                self.calls.append((name, args))
        probe = Probe()
        scene_shot.execute(probe, Path("/tmp"), [{"pause": True}])
        self.assertEqual(probe.calls, [("minecraft_open_pause_menu", {})])
        self.assertEqual(list(scene_shot.scene_mcp_calls([{"pause": True}])),
                         [("minecraft_open_pause_menu", {})])
        for client, value in [("java", True), ("native", False)]:
            with self.assertRaises(ValueError):
                scene_shot.validate_steps([{"pause": value}], client)

    def test_use_phases_checks_state_and_releases_on_failure(self):
        class Probe:
            def __init__(self, states):
                self.states = iter(states)
                self.calls = []
            def observe(self):
                return {"using": next(self.states)}
            def call(self, name, args):
                self.calls.append((name, args))
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)
        step = {"use_phases": {"name": "shield", "ticks": 10}}
        with tempfile.TemporaryDirectory() as tmp, \
                patch.object(scene_shot, "capture_native", side_effect=lambda p, a, n: n):
            root = Path(tmp)
            probe = Probe([None, {"hand": 0}, {"hand": 0}, None])
            self.assertEqual(scene_shot.execute(probe, root, [step]),
                             ["shield-start", "shield-hold", "shield-release"])
            failed = Probe([None, None])
            with self.assertRaisesRegex(AssertionError, "shield_started"):
                scene_shot.execute(failed, root, [step])
            self.assertEqual(failed.calls[-1],
                             ("minecraft_press_inputs", {"keys": ["use"], "ticks": 1, "hold": False}))

    def test_resize_requires_owned_window_and_matching_frame_size(self):
        class Probe:
            def __init__(self):
                self.client = type("Client", (), {"p4_window": {"display": ":99", "window_id": 1}})()
            def call(self, name, args):
                assert (name, args) == ("minecraft_wait_ticks", {"ticks": 5})
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)
        step = {"resize": {"name": "small", "size": [960, 540]}}
        with patch.object(scene_shot, "resize_window") as resize, \
                patch.object(scene_shot, "capture_native", return_value="frame.png"), \
                patch.object(scene_shot, "png_size", return_value=(1280, 720)):
            with self.assertRaisesRegex(AssertionError, "small_dimensions"):
                scene_shot.execute(Probe(), Path("/tmp"), [step])
            resize.assert_called_once_with({"display": ":99", "window_id": 1}, (960, 540))
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([{"resize": {"name": "bad", "size": [0, 540]}}], "native")

    def test_disconnect_wait_rejects_live_or_wrong_eof_state(self):
        class Probe:
            def __init__(self, state):
                self.state = state
            def call(self, name, args=None):
                assert name == "minecraft_observe"
                return self.state
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)
        step = {"wait_disconnect": {"name": "eof", "eof": True}}
        with tempfile.TemporaryDirectory() as tmp:
            state = {"in_play": False, "using": None, "loading_readiness": {"phase": "error"},
                     "disconnect_reason": "not EOF"}
            with self.assertRaisesRegex(AssertionError, "eof_eof"):
                scene_shot.execute(Probe(state), Path(tmp), [step])
            state["in_play"] = True
            with self.assertRaisesRegex(AssertionError, "eof_left_play"):
                scene_shot.execute(Probe(state), Path(tmp), [step])
            state["in_play"] = False
            state["disconnect_reason"] = "server closed connection"
            scene_shot.execute(Probe(state), Path(tmp), [step])
            self.assertEqual(json.loads((Path(tmp) / "eof.json").read_text())["disconnect_reason"],
                             "server closed connection")

    def test_look_is_validated_and_dispatched_to_real_client(self):
        step = {"look": {"yaw_deg": 45, "pitch_deg": -60}}
        self.assertEqual(list(scene_shot.scene_mcp_calls([step])),
                         [("minecraft_look", step["look"])])
        class Probe:
            def __init__(self):
                self.calls = []
            def call(self, name, args):
                self.calls.append((name, args))
        probe = Probe()
        scene_shot.execute(probe, Path("/tmp"), [step])
        self.assertEqual(probe.calls, [("minecraft_look", step["look"])])

    def test_hotbar_is_dispatched_to_real_client(self):
        step = {"hotbar": {"slot": 1}}
        self.assertEqual(list(scene_shot.scene_mcp_calls([step])),
                         [("minecraft_set_hotbar_slot", step["hotbar"])])
        class Probe:
            def __init__(self):
                self.calls = []
            def call(self, name, args):
                self.calls.append((name, args))
        probe = Probe()
        scene_shot.execute(probe, Path("/tmp"), [step])
        self.assertEqual(probe.calls, [("minecraft_set_hotbar_slot", step["hotbar"])])

    def test_mob_motion_gate_rejects_stationary_entities(self):
        class Probe:
            def __init__(self):
                self.checks = {}
            def check(self, name, value):
                self.checks[name] = value
        probe = Probe()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name, tick in zip(("a", "b", "c"), (100, 120, 140)):
                (root / f"{name}.json").write_text(json.dumps({
                    "game_time": tick,
                    "snapshot": {"entities": [
                        {"entity_type": "minecraft:zombie", "entity_uuid": "z", "x": 0, "z": 0},
                        {"entity_type": "minecraft:spider", "entity_uuid": "s", "x": 0, "z": 0},
                    ]},
                }))
            scene_shot.execute(probe, root, [{
                "mob_motion": {"samples": ["a", "b", "c"], "name": "motion",
                               "types": ["minecraft:zombie", "minecraft:spider"]},
            }])
        self.assertEqual(probe.checks, {"mob_moved_zombie": False, "mob_moved_spider": False})

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

    def test_mob_motion_gate_requires_the_kinds_it_measures(self):
        step = {"mob_motion": {"samples": ["a", "b", "c"], "name": "motion"}}
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([step], "native")
        for bad in ([], ["zombie"], [1], "minecraft:zombie"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                scene_shot.validate_steps([{"mob_motion": {**step["mob_motion"], "types": bad}}], "native")
        scene_shot.validate_steps(
            [{"mob_motion": {**step["mob_motion"], "types": ["minecraft:zombie"]}}], "native")

    def test_entity_snapshot_requires_same_thread_game_time(self):
        class Probe:
            def __init__(self, game_time):
                self.game_time = game_time
            def call(self, name, args):
                assert name == "minecraft_list_entities"
                return {"game_time": self.game_time, "entities": [{"x": 1}]}
        step = {"timed_entities": {"name": "mob", "args": {"radius": 24}}}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            scene_shot.execute(Probe(100), root, [step])
            receipt = json.loads((root / "mob.json").read_text())
            self.assertEqual(receipt["game_time"], 100)
            self.assertEqual(receipt["snapshot"]["entities"][0]["x"], 1)
            with self.assertRaisesRegex(RuntimeError, "omitted same-thread game_time"):
                scene_shot.execute(Probe(None), root, [step])

    def test_wait_terrain_fails_closed_on_stale_receipt_and_invalid_input(self):
        class Probe:
            report = {}
            def remaining(self):
                return 30
            def call(self, _name, _args):
                raise AssertionError("invalid input reached the tool")
        valid = {"wait_terrain": {"name": "terrain", "args": {"sections": [[1, 2, 3]]}}}
        class Stale(Probe):
            def call(self, _name, _args):
                return {"status": "ready", "connection_epoch": "1", "dimension_epoch": "1",
                        "resource_epoch": "1", "frame_id": "1", "sections": [
                            {"section": [1, 2, 3], "requested_revision": "8",
                             "mesh_revision": "7", "selected_for_draw": True}]}
        with self.assertRaisesRegex(RuntimeError, "invalid minecraft_wait_for_terrain receipt"):
            scene_shot.execute(Stale(), Path("/tmp"), [valid])
        for args in ({"sections": []}, {"sections": [[1, 2, 3], [1, 2, 3]]},
                     {"sections": [[True, 2, 3]]}, {"sections": [[1, 2, 3]], "timeout_seconds": 121}):
            with self.subTest(args=args), self.assertRaises(ValueError):
                scene_shot.execute(Probe(), Path("/tmp"), [{"wait_terrain": {"name": "terrain", "args": args}}])
        with self.assertRaisesRegex(ValueError, "requires --client native"):
            scene_shot.execute(Probe(), Path("/tmp"), [valid], "java")

    def test_unknown_or_unsafe_step_fails(self):
        for step in ({"shot": "../escape"}, {"unknown": True}, {"wait_play": False}, {"tp": [1, 2]}):
            with self.subTest(step=step), self.assertRaises(ValueError):
                scene_shot.execute(None, Path('/tmp'), [step])

    def test_wait_play_spans_mcp_wait_boundaries_and_detects_disconnect(self):
        class Probe:
            def __init__(self, results):
                self.results = iter(results)
                self.calls = 0
            def remaining(self):
                return 120
            def call(self, name, args):
                self.calls += 1
                return next(self.results)
        probe = Probe([{"in_play": False}, {"in_play": True}])
        scene_shot.wait_play(probe)
        self.assertEqual(probe.calls, 2)
        with self.assertRaisesRegex(RuntimeError, "kicked"):
            scene_shot.wait_play(Probe([{"in_play": False, "disconnect_reason": "kicked"}]))

    def test_long_waits_are_split_for_lease_renewal(self):
        class Probe:
            def __init__(self):
                self.ticks = []
            def call(self, name, args):
                self.ticks.append((name, args["ticks"]))
        probe = Probe()
        scene_shot.execute(probe, Path("/tmp"), [{"wait_ticks": 250}])
        self.assertEqual(probe.ticks, [("minecraft_wait_ticks", 100),
                                      ("minecraft_wait_ticks", 100),
                                      ("minecraft_wait_ticks", 50)])

    def test_screenshot_holds_and_releases_tab_even_on_capture_failure(self):
        events = []
        class Probe:
            client = object()
            def call(self, name, args):
                events.append(args["hold"])
            def wait(self, predicate, label):
                self.assert_visible = predicate({"ui_nodes": [{"id": "tab/player/1"}]})
                assert self.assert_visible
        probe = Probe()
        with tempfile.TemporaryDirectory() as tmp, patch.object(scene_shot.native_client, "capture", side_effect=RuntimeError("capture failed")):
            with self.assertRaisesRegex(RuntimeError, "capture failed"):
                scene_shot.execute(probe, Path(tmp), [{"shot": "tab", "hold_keys": ["key.keyboard.tab"]}])
        self.assertEqual(events, [True, False])

    def test_java_shot_uses_minecraft_screenshot(self):
        class Probe:
            client = object()
        with tempfile.TemporaryDirectory() as tmp, patch.object(scene_shot.java_p5, "capture", return_value="/x/a.png") as java_capture, \
                patch.object(scene_shot, "png_size", return_value=scene_shot.WINDOW), \
                patch.object(scene_shot.native_client, "capture") as native_capture:
            probe = Probe()
            self.assertEqual(scene_shot.execute(probe, Path(tmp), [{"shot": "a"}], "java"), ["/x/a.png"])
            java_capture.assert_called_once_with(probe, "a")
            native_capture.assert_not_called()

    def test_java_shot_with_wrong_window_size_fails(self):
        class Probe:
            client = object()
        with tempfile.TemporaryDirectory() as tmp, patch.object(scene_shot.java_p5, "capture", return_value="/x/a.png"), \
                patch.object(scene_shot, "png_size", return_value=(854, 480)):
            with self.assertRaisesRegex(RuntimeError, "expected"):
                scene_shot.execute(Probe(), Path(tmp), [{"shot": "a"}], "java")

    def test_java_rejects_native_only_held_screenshot(self):
        with self.assertRaisesRegex(ValueError, "unsupported for --client java"):
            scene_shot.execute(None, Path("/tmp"), [{"shot": "tab", "hold_keys": ["key.keyboard.tab"]}], "java")

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
                        with image.open("xb") as file:
                            file.write(b"image")
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

    def test_clock_capture_contract_and_tick_subscription(self):
        step = {"dropped_clock_capture": "ctx-day"}
        scene_shot.validate_steps([step], "native")
        self.assertEqual(list(scene_shot.scene_mcp_calls([step])),
                         [("minecraft_wait_ticks", {"ticks": 4})])
        for client, value in (("java", "ctx-day"), ("native", "held-day"), ("native", False)):
            with self.subTest(client=client, value=value), self.assertRaises(ValueError):
                scene_shot.validate_steps([{"dropped_clock_capture": value}], client)

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


class AssertEntitiesTests(unittest.TestCase):
    @staticmethod
    def villager(x, y, z, profession=0, height=1.95):
        return {"entity_type": "minecraft:villager", "x": x, "y": y, "z": z,
                "bounding_box": {"min_y": y, "max_y": y + height},
                "received_metadata": {"19": {"value": ["plains", profession, 1]}}}

    def test_filters_box_pose_and_profession(self):
        rows = [self.villager(1, 70, 1), self.villager(50, 70, 1),
                self.villager(2, 70, 2, profession=9), self.villager(3, 70, 3, height=0.2),
                {"entity_type": "minecraft:zombie", "x": 1, "y": 70, "z": 1}]
        spec = {"entity_type": "minecraft:villager"}
        self.assertEqual(len(scene_shot.matching_entities(rows, spec)), 4)
        self.assertEqual(len(scene_shot.matching_entities(rows, {**spec, "box": [0, 60, 0, 10, 80, 10]})), 3)
        self.assertEqual(len(scene_shot.matching_entities(rows, {**spec, "max_height": 0.5})), 1)
        self.assertEqual(len(scene_shot.matching_entities(rows, {**spec, "profession": "librarian"})), 1)
        self.assertEqual(len(scene_shot.matching_entities(rows, {**spec, "profession": "none"})), 3)

    def test_baby_filter_matches_only_received_flag(self):
        adult = {**self.villager(1, 70, 1), "received_baby": False}
        baby = {**self.villager(2, 70, 2), "received_baby": True}
        unknown = {**self.villager(3, 70, 3), "received_baby": None}
        silent = self.villager(4, 70, 4)
        rows = [adult, baby, unknown, silent]
        spec = {"entity_type": "minecraft:villager"}
        self.assertEqual(scene_shot.matching_entities(rows, {**spec, "baby": True}), [baby])
        self.assertEqual(scene_shot.matching_entities(rows, {**spec, "baby": False}), [adult])
        self.assertEqual(len(scene_shot.matching_entities(rows, spec)), 4)

    def test_baby_step_validation(self):
        step = {"name": "n", "args": {"radius": 8}, "entity_type": "minecraft:villager", "min": 1, "baby": True}
        scene_shot.validate_steps([{"assert_entities": step}], "native")
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([{"assert_entities": {**step, "baby": 1}}], "native")

    def test_breeding_scene_judges_a_birth_in_game_ticks(self):
        scene = json.loads((scene_shot.runtime.REPO_ROOT / "tools/harness/village_breeding_scene.json").read_text())
        steps = scene["steps"]
        scene_shot.validate_steps(steps, "native")
        ticks = [step["wait_ticks"] for step in steps if "wait_ticks" in step]
        self.assertEqual(sum(ticks[1:]), 40 + 3000)  # after the 100-tick join wait: settle, then 6 * 500
        self.assertFalse([step for step in steps if "sleep" in step or "wait_seconds" in step])
        asserts = {step["assert_entities"]["name"]: step["assert_entities"] for step in steps if "assert_entities" in step}
        self.assertEqual(asserts["breeding_start_no_baby"]["max"], 0)
        self.assertTrue(asserts["breeding_start_no_baby"]["baby"])
        born = asserts["breeding_baby_born"]
        self.assertEqual((born["baby"], born["min"], born["entity_type"]), (True, 1, "minecraft:villager"))
        # The newborn is asserted after every tick wait, never before.
        order = [next(iter(step)) for step in steps]
        self.assertGreater(steps.index({"assert_entities": born}), max(i for i, kind in enumerate(order) if kind == "wait_ticks"))

    def test_step_validation_and_village_scenes(self):
        good = {"assert_entities": {"name": "n", "args": {"radius": 8}, "entity_type": "minecraft:villager",
                                    "min": 1, "box": [0, 0, 0, 1, 1, 1], "profession": "librarian"}}
        scene_shot.validate_steps([good], "native")
        for bad in ({**good["assert_entities"], "profession": "wizard"},
                    {**good["assert_entities"], "box": [0, 0, 0]},
                    {k: v for k, v in good["assert_entities"].items() if k != "min"}):
            with self.assertRaises(ValueError):
                scene_shot.validate_steps([{"assert_entities": bad}], "native")
        for name in ("idle_stroll", "night_door", "late_job", "distant_job",
                     "deep_water", "mob_doors", "after_hours_job", "breeding"):
            scene = json.loads((scene_shot.runtime.REPO_ROOT / "tools/harness" / f"village_{name}_scene.json").read_text())
            scene_shot.validate_steps(scene["steps"], "native")


class VillagerClaimGateTests(unittest.TestCase):
    def test_sleeping_observer_requires_received_pose(self):
        self.assertTrue(scene_shot.received_sleeping({"6": {"kind": "pose", "value": 2}}))
        self.assertFalse(scene_shot.received_sleeping({"6": {"kind": "pose", "value": 0}}))
        for metadata in ({}, {"6": {"kind": "int", "value": 2}}, {"6": {"kind": "pose", "value": True}}):
            with self.subTest(metadata=metadata), self.assertRaises(RuntimeError):
                scene_shot.received_sleeping(metadata)

    def test_player_sleeping_asserts_received_metadata_not_predicted_pose(self):
        class Probe:
            def __init__(self, pose):
                self.pose = pose

            def observe(self):
                return {"player": {"pose": "standing", "received_metadata": {
                    "6": {"kind": "pose", "value": self.pose}}}}

            def check(self, name, condition):
                if not condition:
                    raise AssertionError(name)

        step = {"assert_player_sleeping": {"name": "not_asleep", "sleeping": False}}
        with tempfile.TemporaryDirectory() as tmp:
            scene_shot.execute(Probe(0), Path(tmp), [step])
            with self.assertRaisesRegex(AssertionError, "not_asleep"):
                scene_shot.execute(Probe(2), Path(tmp), [step])
        for bad in ({"name": "x", "sleeping": 0}, {"name": "x"}):
            with self.assertRaises(ValueError):
                scene_shot.validate_steps([{"assert_player_sleeping": bad}], "native")
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([step], "java")

    def test_entity_sleeping_filter_uses_received_pose(self):
        awake = {**AssertEntitiesTests.villager(1, 70, 1),
                 "received_metadata": {"6": {"kind": "pose", "value": 0}}}
        asleep = {**awake, "received_metadata": {"6": {"kind": "pose", "value": 2}}}
        spec = {"entity_type": "minecraft:villager"}
        self.assertEqual(scene_shot.matching_entities([awake, asleep], {**spec, "sleeping": True}), [asleep])
        self.assertEqual(scene_shot.matching_entities([awake, asleep], {**spec, "sleeping": False}), [awake])

    def test_registered_claim_gates_have_native_valid_fixtures(self):
        for name in ("moved-workstation", "occupied-bed", "open-air-home"):
            entry = scenarios.SCENARIOS["native-village-" + name]
            self.assertEqual(entry.evidence, "graphical")
            path = scene_shot.runtime.REPO_ROOT / "tools/harness" / ("village_" + name.replace("-", "_") + "_scene.json")
            scene_shot.validate_steps(json.loads(path.read_text())["steps"], "native")
