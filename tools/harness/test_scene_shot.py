import json
import math
import socket
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness.backends import scene_shot


class SceneShotTests(unittest.TestCase):
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
            env = {'SOLARIS_SERVER_BINARY': str(binary), 'XDG_DATA_HOME': str(root / 'xdg')}
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
            env = {'SOLARIS_SERVER_BINARY': str(binary), 'XDG_DATA_HOME': str(root / 'xdg')}
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
                    # A frame that lagged one tick step below the bracket still
                    # matches the triggering observation and is accepted.
                    write_frame(1.2)
                    self.assertEqual(scene_shot.execute(Probe(), root, [{"walk_capture": spec}], client), [image])
                    if client == "java":
                        # Java render-state frames carry no entity uuid; a single
                        # typed frame entity is matched by type.
                        items = [{key: value for key, value in entity.items() if key != "entity_uuid"}
                                 | {"walk_position": 1.2}]
                        frame_path.write_text(json.dumps(items))
                        self.assertEqual(scene_shot.execute(Probe(), root, [{"walk_capture": spec}], client), [image])
                    # A frame outside the bracket and further from the observation
                    # than one tick of walk advance is rejected.
                    write_frame(2.5)
                    with self.assertRaisesRegex(AssertionError, "walk_render_phase"):
                        scene_shot.execute(Probe(), root, [{"walk_capture": spec}], client)

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
                "mob_motion": {"samples": ["a", "b", "c"], "name": "motion"},
            }])
        self.assertEqual(probe.checks, {"mob_moved_zombie": False, "mob_moved_spider": False})

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
        for name in ("idle_stroll", "night_door", "late_job", "distant_job", "breeding"):
            scene = json.loads((scene_shot.runtime.REPO_ROOT / "tools/harness" / f"village_{name}_scene.json").read_text())
            scene_shot.validate_steps(scene["steps"], "native")
